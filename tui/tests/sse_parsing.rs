//! Event-stream parsing without a server.
//!
//! The fixture below is the exact byte shape the server writes: axum's SSE
//! encoder emits `event: <name>` plus `data: <json>` followed by a blank line,
//! and `KeepAlive::text("keep-alive-text")` emits the comment line
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

/// `ChatMessage` from `src/message.rs`: `mid` at the top, then
/// `payload.from_uid`, `payload.created_at`, an externally tagged target, and
/// the text nested three levels down under `detail.Normal`.
const CHAT_DATA: &str = r#"{"mid":42,"payload":{"from_uid":2,"created_at":"2026-11-11 09:00:00","target":{"User":{"uid":1}},"detail":{"Normal":{"content":{"content":"早上好"}}}}}"#;

const ECHO_DATA: &str = r#"{"mid":43,"payload":{"from_uid":1,"created_at":"2026-11-11 09:01:00","target":{"User":{"uid":2}},"detail":{"Normal":{"content":{"content":"早"}}}}}"#;

const GROUP_DATA: &str = r#"{"mid":44,"payload":{"from_uid":3,"created_at":"2026-11-11 09:02:00","target":{"Group":{"gid":7}},"detail":{"Normal":{"content":{"content":"大家好"}}}}}"#;

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

/// A real server stream: a keep-alive, a chat, a heartbeat, another keep-alive.
fn stream_bytes() -> Vec<u8> {
    format!(
        ": keep-alive-text\n\n\
         event: Chat\ndata: {CHAT_DATA}\n\n\
         event: Heartbeat\ndata: {{\"time\":\"2026-11-11 09:00:05\"}}\n\n\
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
                data: r#"{"time":"2026-11-11 09:00:05"}"#.to_string(),
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

#[test]
fn a_chat_frame_decodes_the_three_level_payload_into_one_stream_chat() {
    let event = model::stream_event("Chat", CHAT_DATA)
        .expect("a real Chat payload decodes")
        .expect("Chat is a known event");

    let StreamEvent::Chat(chat) = event else {
        panic!("a Chat event must decode to a chat");
    };
    assert_eq!(chat.mid, 42);
    assert_eq!(chat.from_uid, 2);
    assert_eq!(chat.target, ConversationId::User(1));
    assert_eq!(chat.body, "早上好");
    assert_eq!(chat.time, at("2026-11-11 09:00:00"));
}

#[test]
fn an_incoming_message_belongs_to_the_sender_but_my_echo_belongs_to_the_recipient() {
    let incoming = match model::stream_event("Chat", CHAT_DATA).expect("decodes") {
        Some(StreamEvent::Chat(chat)) => chat,
        _ => panic!("expected a chat"),
    };
    assert_eq!(
        incoming.conversation(Some(1)),
        ConversationId::User(2),
        "a message addressed to me is the sender's conversation"
    );
    assert!(!incoming.is_mine(Some(1)));

    let echo = match model::stream_event("Chat", ECHO_DATA).expect("decodes") {
        Some(StreamEvent::Chat(chat)) => chat,
        _ => panic!("expected a chat"),
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
    let chat = match model::stream_event("Chat", GROUP_DATA).expect("decodes") {
        Some(StreamEvent::Chat(chat)) => chat,
        _ => panic!("expected a chat"),
    };

    assert_eq!(chat.target, ConversationId::Group(7));
    assert_eq!(chat.conversation(Some(1)), ConversationId::Group(7));
}

#[test]
fn a_heartbeat_is_liveness_only_and_an_unknown_event_is_ignored() {
    assert_eq!(
        model::stream_event("Heartbeat", r#"{"time":"2026-11-11 09:00:05"}"#).expect("known"),
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
