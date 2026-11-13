//! Behavioural tests for sending a message from the message box.
//!
//! Same seam as the rest of the client: events in, state and effects out. The
//! HTTP call is not part of these tests; the wiring layer turns the effect into a
//! [`Event::MessageSent`] or [`Event::MessageSendFailed`].

use chat_tui::model::{self, Conversation, ConversationId, Message, StreamMessage, Unread};
use chat_tui::state::{Delivery, Effect, Event, Key, Machine, PendingMessage, Screen};
use chrono::NaiveDateTime;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
}

fn now() -> NaiveDateTime {
    at("2026-11-11 12:00:00")
}

fn conversation(uid: i32, name: &str, mid: i64, time: &str) -> Conversation {
    Conversation {
        id: ConversationId::User(uid),
        title: name.to_string(),
        mid,
        preview: format!("last message {mid}"),
        time: at(time),
        unread: Unread::None,
        last_sender: None,
    }
}

fn message(mid: i64, from_uid: i32, from_name: &str, body: &str, time: &str) -> Message {
    Message {
        mid,
        from_uid,
        from_name: from_name.to_string(),
        body: body.to_string(),
        time: at(time),
    }
}

fn stream_message(
    mid: i64,
    from_uid: i32,
    target: ConversationId,
    body: &str,
    time: &str,
) -> StreamMessage {
    StreamMessage {
        mid,
        from_uid,
        target,
        body: body.to_string(),
        time: at(time),
    }
}

/// A JWT-shaped token whose payload is `{"id": uid}`: login now refuses a token
/// it cannot attribute to a user.
fn token_for(uid: i32) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let header = URL_SAFE_NO_PAD.encode(r#"{{"alg":"HS256","typ":"JWT"}}"#);
    let payload = URL_SAFE_NO_PAD.encode(format!(
        r#"{{"id":{uid},"name":"alice","email":null,"phone":null,"exp":9999999999}}"#
    ));
    format!("{header}.{payload}.c2lnbmF0dXJl")
}

fn type_str(machine: &mut Machine, text: &str) {
    for c in text.chars() {
        machine.handle(Event::Key(Key::Char(c)));
    }
}

fn logged_in() -> Machine {
    let mut machine = Machine::new();
    type_str(&mut machine, "alice");
    machine.handle(Event::Key(Key::Tab));
    type_str(&mut machine, "s3cret");
    machine.handle(Event::Key(Key::Enter));
    machine.handle(Event::LoginSucceeded {
        token: token_for(1),
    });
    machine
}

/// Alice's list with bob's conversation open and its history loaded.
fn open_bob() -> Machine {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![
            conversation(2, "bob", 21, "2026-11-11 11:00:00"),
            conversation(3, "carol", 20, "2026-11-11 10:00:00"),
        ],
    });
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(21, 2, "bob", "早上好", "2026-11-11 11:00:00")],
    });
    machine
}

fn local_id_of(effects: &[Effect]) -> u64 {
    match effects.first() {
        Some(Effect::SendMessage { local_id, .. }) => *local_id,
        other => panic!("expected a send effect, got {other:?}"),
    }
}

fn pending_rows(machine: &Machine) -> &[PendingMessage] {
    &machine.state().pending
}

#[test]
fn pressing_enter_shows_the_message_as_pending_before_the_server_replies() {
    let mut machine = open_bob();
    type_str(&mut machine, "你好，世界");
    assert_eq!(machine.state().input, "你好，世界");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::SendMessage {
            peer: 2,
            local_id: 0,
            text: "你好，世界".to_string(),
        }]
    );
    assert_eq!(
        machine.state().input,
        "",
        "the box empties as soon as the send starts"
    );

    let views = machine.state().message_views(now());
    assert_eq!(views.len(), 2, "the typed row is already in the stream");
    let last = views.last().unwrap();
    assert_eq!(last.body, "你好，世界");
    assert!(last.mine, "one's own message belongs on one's own side");
    assert_eq!(last.time_label, "发送中…");
    assert!(!last.failed);

    let rows = pending_rows(&machine);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].conversation, ConversationId::User(2));
    assert_eq!(rows[0].delivery, Delivery::Sending);
    assert_eq!(machine.state().screen, Screen::Conversations);
}

#[test]
fn a_confirmed_send_takes_the_server_id_without_a_duplicate_row() {
    let mut machine = open_bob();
    type_str(&mut machine, "hello");
    let local_id = local_id_of(&machine.handle(Event::Key(Key::Enter)));

    machine.handle(Event::MessageSent {
        conversation: ConversationId::User(2),
        local_id,
        mid: 42,
        time: at("2026-11-11 11:59:30"),
    });

    let views = machine.state().message_views(now());
    assert_eq!(
        views.iter().filter(|view| view.body == "hello").count(),
        1,
        "the optimistic row and the confirmed one are the same row"
    );
    let last = views.last().unwrap();
    assert_eq!(
        last.time_label, "11:59",
        "the confirmed row gets a real time"
    );
    assert!(!last.failed);

    assert!(
        pending_rows(&machine).is_empty(),
        "a confirmed row belongs to the message history, not to the optimistic list"
    );
    assert_eq!(
        machine.state().messages.last().map(|message| message.mid),
        Some(42),
        "the server's id moves the row into the confirmed history"
    );
}

/// The row the send endpoint confirms must be ordered by its server id, not
/// appended after whatever arrived while the reply was in flight.
#[test]
fn a_confirmed_send_renders_above_a_message_that_arrived_later() {
    let mut machine = open_bob();
    type_str(&mut machine, "hello");
    let local_id = local_id_of(&machine.handle(Event::Key(Key::Enter)));

    // The send reply wins the race and gives the row mid 42.
    machine.handle(Event::MessageSent {
        conversation: ConversationId::User(2),
        local_id,
        mid: 42,
        time: at("2026-11-11 11:59:30"),
    });
    // A later message gets the next id, 43, and arrives after the reply.
    machine.handle(Event::MessageReceived(stream_message(
        43,
        2,
        ConversationId::User(1),
        "刚刚",
        "2026-11-11 12:00:00",
    )));

    let bodies: Vec<String> = machine
        .state()
        .message_views(now())
        .into_iter()
        .map(|view| view.body)
        .collect();
    assert_eq!(
        bodies,
        vec![
            "早上好".to_string(),
            "hello".to_string(),
            "刚刚".to_string()
        ],
        "message ids are the render order: 21, 42, 43"
    );
}

#[test]
fn a_failed_send_keeps_the_text_and_explains_the_error_on_the_status_line() {
    let mut machine = open_bob();
    type_str(&mut machine, "今晚见");
    let local_id = local_id_of(&machine.handle(Event::Key(Key::Enter)));

    machine.handle(Event::MessageSendFailed {
        conversation: ConversationId::User(2),
        local_id,
        message: "用户2不是您的好友".to_string(),
    });

    assert_eq!(
        machine.state().input,
        "今晚见",
        "the user must not retype the message"
    );
    assert_eq!(
        machine.state().status.as_deref(),
        Some("发送失败：用户2不是您的好友"),
        "the server's own reason reaches the status line"
    );

    let views = machine.state().message_views(now());
    let last = views.last().unwrap();
    assert_eq!(last.body, "今晚见");
    assert!(last.failed, "the attempt is marked failed");
    assert_eq!(last.time_label, "发送失败");
    assert_eq!(
        pending_rows(&machine)[0].delivery,
        Delivery::Failed {
            message: "用户2不是您的好友".to_string(),
        }
    );
}

#[test]
fn a_failure_does_not_overwrite_text_typed_after_the_send() {
    let mut machine = open_bob();
    type_str(&mut machine, "first");
    let local_id = local_id_of(&machine.handle(Event::Key(Key::Enter)));
    type_str(&mut machine, "second");

    machine.handle(Event::MessageSendFailed {
        conversation: ConversationId::User(2),
        local_id,
        message: "网络错误".to_string(),
    });

    assert_eq!(machine.state().input, "second");
    assert_eq!(
        machine.state().status.as_deref(),
        Some("发送失败：网络错误")
    );
}

#[test]
fn a_failure_for_a_closed_conversation_does_not_fill_another_peers_box() {
    let mut machine = open_bob();
    type_str(&mut machine, "for bob");
    let local_id = local_id_of(&machine.handle(Event::Key(Key::Enter)));
    // The user moved on to carol while bob's request was still in flight.
    machine.handle(Event::Key(Key::Down));
    assert_eq!(machine.state().active, Some(ConversationId::User(3)));

    machine.handle(Event::MessageSendFailed {
        conversation: ConversationId::User(2),
        local_id,
        message: "用户2不是您的好友".to_string(),
    });

    assert_eq!(
        machine.state().input,
        "",
        "bob's text must not become a draft for carol"
    );
    assert_eq!(
        pending_rows(&machine)[0].delivery,
        Delivery::Failed {
            message: "用户2不是您的好友".to_string(),
        },
        "the failed attempt is still recorded on bob's row"
    );
}

#[test]
fn an_empty_or_blank_box_does_not_send() {
    let mut machine = open_bob();

    assert!(
        machine.handle(Event::Key(Key::Enter)).is_empty(),
        "an empty box emits no request"
    );
    assert!(pending_rows(&machine).is_empty());

    type_str(&mut machine, "   ");
    assert!(
        machine.handle(Event::Key(Key::Enter)).is_empty(),
        "whitespace-only is empty too"
    );
    assert!(pending_rows(&machine).is_empty());
    assert_eq!(machine.state().input, "   ", "the box is left alone");
}

#[test]
fn backspace_and_cjk_edit_the_message_text() {
    let mut machine = open_bob();
    type_str(&mut machine, "你好a");
    machine.handle(Event::Key(Key::Backspace));

    assert_eq!(
        machine.state().input,
        "你好",
        "backspace removes one character, not one byte"
    );

    let effects = machine.handle(Event::Key(Key::Enter));
    assert_eq!(
        effects,
        vec![Effect::SendMessage {
            peer: 2,
            local_id: 0,
            text: "你好".to_string(),
        }]
    );
}

#[test]
fn q_is_message_text_on_the_conversations_screen() {
    let mut machine = open_bob();

    machine.handle(Event::Key(Key::Char('q')));

    assert!(!machine.state().quit, "the message box owns printable keys");
    assert_eq!(machine.state().input, "q");

    let effects = machine.handle(Event::Key(Key::Esc));
    assert_eq!(effects.len(), 2, "Esc still logs out and quits");
    assert!(machine.state().quit);
}

#[test]
fn enter_no_longer_reloads_the_conversation_and_ctrl_r_does() {
    let mut machine = open_bob();
    type_str(&mut machine, "hi");

    let effects = machine.handle(Event::Key(Key::Enter));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadMessages { .. })),
        "Enter is reserved for send"
    );

    // Ctrl-R carries the login screen's ToggleMode key; here it retries the load.
    let effects = machine.handle(Event::Key(Key::ToggleMode));
    assert_eq!(effects, vec![Effect::LoadMessages { peer: 2 }]);
}

#[test]
fn pending_rows_follow_their_own_conversation() {
    let mut machine = open_bob();
    type_str(&mut machine, "for bob");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::Key(Key::Down));
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(3),
        items: vec![message(20, 3, "carol", "hi alice", "2026-11-11 10:00:00")],
    });
    let carol_views = machine.state().message_views(now());
    assert!(
        carol_views.iter().all(|view| view.body != "for bob"),
        "bob's optimistic row must not appear in carol's pane"
    );

    machine.handle(Event::Key(Key::Up));
    let bob_views = machine.state().message_views(now());
    assert_eq!(
        bob_views
            .iter()
            .filter(|view| view.body == "for bob")
            .count(),
        1,
        "the row reappears when bob is open again"
    );
    assert_eq!(pending_rows(&machine).len(), 1);
}

#[test]
fn a_history_reload_that_contains_the_sent_message_drops_the_optimistic_row() {
    let mut machine = open_bob();
    type_str(&mut machine, "hello");
    let local_id = local_id_of(&machine.handle(Event::Key(Key::Enter)));
    machine.handle(Event::MessageSent {
        conversation: ConversationId::User(2),
        local_id,
        mid: 42,
        time: at("2026-11-11 11:59:30"),
    });

    // The server's own copy of the same message now arrives via history.
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(42, 1, "alice", "hello", "2026-11-11 11:59:30")],
    });

    let views = machine.state().message_views(now());
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].body, "hello");
    assert_eq!(views[0].time_label, "11:59");
    assert!(
        machine.state().pending.is_empty(),
        "the confirmed history row is authoritative"
    );
}

#[test]
fn ending_the_session_clears_unsent_and_pending_state() {
    let mut machine = open_bob();
    type_str(&mut machine, "draft");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::TokenExpired);

    assert!(machine.state().pending.is_empty());
    assert_eq!(machine.state().input, "");
    assert_eq!(machine.state().screen, Screen::Login);
}
