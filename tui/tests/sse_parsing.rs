//! Event-stream parsing without a server.
//!
//! The fixtures are the exact byte shape the server writes: axum's SSE encoder
//! emits `event: <name>` plus `data: <json>` followed by a blank line, and
//! `KeepAlive::text("keep-alive-text")` emits the comment line
//! `: keep-alive-text`. Comments carry no event and must never surface.
//!
//! Two layers are covered here: the transport parser (`chat_tui::sse`) and the
//! wire-to-domain translation (`chat_tui::model::stream_event`). The live HTTP
//! path around them is pinned in `live_stream_wire.rs` against a stub server.

use chat_tui::model::{self, ConversationId, StreamEvent};
use chat_tui::sse::{Frame, Parser};
use chrono::NaiveDateTime;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// The `data:` line for an incoming message, byte-for-byte as the server writes
/// it. `src/event.rs::event_handler` serialises its `Message::ChatMessage(..)`
/// through `json_data`, and that enum is externally tagged, so the message is
/// wrapped in `{"ChatMessage": ...}`; the repo's own `assets/index.html` reads
/// `data.ChatMessage.payload` for the same reason. Inside the wrapper the shape
/// is the server's `message::ChatMessage` (`mid` + `payload`), with
/// `payload.target` and `payload.detail` externally tagged and the text nested
/// under `detail.Normal.content.content`.
///
/// This is a hand-copied-but-verified fixture (it also matches a capture from
/// the running server). Generating it from `chat_server::event::Message` would
/// need a dev-dependency on the server crate, which would drag that whole crate
/// into `cargo clippy -p chat-tui --all-targets -- -D warnings` and turn its
/// pre-existing warnings into failures. The regression test below still fails
/// for the unwrapped `{mid, payload}` shape this replaced.
const CHAT_DATA: &str = r#"{"ChatMessage":{"mid":42,"payload":{"from_uid":2,"created_at":"2026-11-11 09:00:00","target":{"User":{"uid":1}},"detail":{"Normal":{"content":{"content":"早上好"}}}}}}"#;

/// My own echo, in the same envelope.
const ECHO_DATA: &str = r#"{"ChatMessage":{"mid":43,"payload":{"from_uid":1,"created_at":"2026-11-11 09:01:00","target":{"User":{"uid":2}},"detail":{"Normal":{"content":{"content":"早"}}}}}}"#;

/// A group-targeted message, in the same envelope.
const GROUP_DATA: &str = r#"{"ChatMessage":{"mid":44,"payload":{"from_uid":3,"created_at":"2026-11-11 09:02:00","target":{"Group":{"gid":7}},"detail":{"Normal":{"content":{"content":"大家好"}}}}}}"#;

/// Exact `data:` bytes captured from the running server for one real message
/// (bob -> alice, `mid` 1): the regression test below feeds these unmodified.
const CAPTURED_CHAT_DATA: &str = r#"{"ChatMessage":{"mid":1,"payload":{"from_uid":2,"created_at":"2026-11-11 01:28:19","target":{"User":{"uid":1}},"detail":{"Normal":{"content":{"content":"hello from bob"}}}}}}"#;

/// The server's `Message::Heartbeat(HeartbeatMessage { time })` is wrapped the
/// same way (`src/event.rs`). The client treats a heartbeat as liveness only and
/// never reads `time`, so only the envelope tag matters here.
const HEARTBEAT_DATA: &str = r#"{"Heartbeat":{"time":"2026-11-11 09:00:05"}}"#;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
}

fn token_for(uid: i32) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(format!(
        r#"{{"id":{uid},"name":"alice","email":null,"phone":null,"exp":9999999999}}"#
    ));
    format!("{header}.{payload}.c2lnbmF0dXJl")
}

/// A real server stream: a keep-alive, a message, a heartbeat, another keep-alive.
fn stream_bytes() -> Vec<u8> {
    format!(
        ": keep-alive-text\n\n\
         event: Chat\ndata: {CHAT_DATA}\n\n\
         event: Heartbeat\ndata: {HEARTBEAT_DATA}\n\n\
         : keep-alive-text\n\n"
    )
    .into_bytes()
}

#[test]
fn the_parser_frames_named_events_and_never_turns_keep_alive_comments_into_events() {
    let mut parser = Parser::new();

    let frames = parser.push(&stream_bytes()).expect("valid UTF-8 stream");

    assert_eq!(
        frames,
        vec![
            Frame {
                event: "Chat".to_string(),
                data: CHAT_DATA.to_string(),
            },
            Frame {
                event: "Heartbeat".to_string(),
                data: HEARTBEAT_DATA.to_string(),
            },
        ],
        "the two keep-alive comments produce no frames at all"
    );
}

#[test]
fn a_stream_split_anywhere_still_parses_the_same_frames() {
    // Byte-at-a-time splits lines, frames, and multi-byte UTF-8 characters.
    let mut parser = Parser::new();
    let mut frames = Vec::new();
    for byte in stream_bytes() {
        frames.extend(
            parser
                .push(&[byte])
                .expect("a partial chunk is not a frame"),
        );
    }

    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].event, "Chat");
    assert_eq!(frames[1].event, "Heartbeat");
}

#[test]
fn a_line_that_keeps_growing_without_a_newline_does_not_grow_without_bound() {
    let mut parser = Parser::new();
    let mut framed = false;
    // A chunk larger than the cap with no newline must be refused, not buffered.
    for _ in 0..3 {
        if parser.push(&vec![b'x'; 512 * 1024]).is_err() {
            framed = true;
            break;
        }
    }
    assert!(framed, "the parser reports the oversized line");
}

/// The regression this whole envelope handling exists for: the server's data is
/// `{"ChatMessage": {mid, payload}}`, and a client that parses the bare
/// `{mid, payload}` drops every live message. The input is the capture above.
#[test]
fn a_captured_chat_frame_decodes_through_its_enum_envelope() {
    let data = CAPTURED_CHAT_DATA;
    assert!(
        data.starts_with(r#"{"ChatMessage":{"mid":1"#),
        "the server wraps the message in its externally tagged enum: {data}"
    );

    let event = model::stream_event("Chat", data)
        .expect("the captured Chat frame decodes")
        .expect("Chat is a known event");

    let StreamEvent::MessageReceived(message) = event else {
        panic!("a Chat event must decode to a message");
    };
    assert_eq!(message.mid, 1);
    assert_eq!(message.from_uid, 2);
    assert_eq!(message.target, ConversationId::User(1));
    assert_eq!(message.body, "hello from bob");
    assert_eq!(message.time, at("2026-11-11 01:28:19"));
}

#[test]
fn an_incoming_message_belongs_to_the_sender_but_my_echo_belongs_to_the_recipient() {
    let incoming = match model::stream_event("Chat", CHAT_DATA).expect("decodes") {
        Some(StreamEvent::MessageReceived(message)) => message,
        _ => panic!("expected a message"),
    };
    assert_eq!(
        incoming.conversation(Some(1)),
        ConversationId::User(2),
        "a message addressed to me is the sender's conversation"
    );
    assert!(!incoming.is_mine(Some(1)));

    let echo = match model::stream_event("Chat", ECHO_DATA).expect("decodes") {
        Some(StreamEvent::MessageReceived(message)) => message,
        _ => panic!("expected a message"),
    };
    assert_eq!(
        echo.conversation(Some(1)),
        ConversationId::User(2),
        "my own echoed message is my recipient's conversation"
    );
    assert!(echo.is_mine(Some(1)));
}

#[test]
fn a_group_message_belongs_to_the_group_even_when_someone_else_sent_it() {
    let message = match model::stream_event("Chat", GROUP_DATA).expect("decodes") {
        Some(StreamEvent::MessageReceived(message)) => message,
        _ => panic!("expected a message"),
    };

    assert_eq!(message.target, ConversationId::Group(7));
    assert_eq!(message.conversation(Some(1)), ConversationId::Group(7));
}

#[test]
fn a_heartbeat_is_liveness_only_and_an_unknown_event_is_ignored() {
    assert_eq!(
        model::stream_event("Heartbeat", HEARTBEAT_DATA).expect("known"),
        Some(StreamEvent::Heartbeat)
    );
    assert_eq!(
        model::stream_event("Presence", r#"{"uid":2}"#).expect("unknown names are not errors"),
        None,
        "a server-side addition must not break the stream"
    );
}

#[test]
fn a_malformed_chat_payload_is_an_error_the_stream_can_skip() {
    let error = model::stream_event("Chat", "{").expect_err("truncated JSON");
    assert!(
        error.to_string().contains("Chat"),
        "the error names the event: {error}"
    );
}

#[test]
fn the_uid_comes_out_of_the_jwt_payload_not_the_login_response() {
    assert_eq!(
        model::uid_from_token(&token_for(7)).expect("valid token"),
        7
    );
    assert_eq!(
        model::uid_from_token(&token_for(1)).expect("valid token"),
        1
    );

    assert!(
        model::uid_from_token("not-a-jwt").is_err(),
        "a token without a payload segment has no uid"
    );
    assert!(
        model::uid_from_token("header.eyJpZCI6.bad-signature").is_err(),
        "a payload without an id has no uid"
    );
    assert!(
        model::uid_from_token("a.%%%.c").is_err(),
        "invalid base64url is an error, not a panic"
    );
}
