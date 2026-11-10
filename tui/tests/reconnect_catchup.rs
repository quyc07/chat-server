//! Behavioural tests for surviving a dead push link and catching back up.
//!
//! These use the pure state-machine seam: link events and HTTP results go in,
//! state and effects come out. No terminal, no server, no sleeping — reconnect
//! timing is the watchdog's job and is tested separately in `link_liveness.rs`.

use chat_tui::model::{self, Conversation, ConversationId, Message, StreamChat, Unread};
use chat_tui::state::{CONNECTION_LOST_STATUS, Connection, Effect, Event, Key, Machine, Screen};
use chrono::NaiveDateTime;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
}

fn conversation(uid: i32, name: &str, mid: i64, time: &str, unread: Unread) -> Conversation {
    Conversation {
        id: ConversationId::User(uid),
        title: name.to_string(),
        mid,
        preview: format!("last message {mid}"),
        time: at(time),
        unread,
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

fn chat(mid: i64, from_uid: i32, target: ConversationId, body: &str, time: &str) -> StreamChat {
    StreamChat {
        mid,
        from_uid,
        target,
        body: body.to_string(),
        time: at(time),
    }
}

fn type_str(machine: &mut Machine, text: &str) {
    for c in text.chars() {
        machine.handle(Event::Key(Key::Char(c)));
    }
}

/// A JWT-shaped access token whose payload is `{"id":1,...}`: the state machine
/// reads the caller's uid out of it.
const ALICE_TOKEN: &str = concat!(
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
    ".",
    "eyJpZCI6MSwibmFtZSI6ImFsaWNlIiwiZW1haWwiOm51bGwsInBob25lIjpudWxsLCJleHAiOjk5OTk5OTk5OTl9",
    ".",
    "c2lnbmF0dXJl"
);

fn logged_in() -> Machine {
    let mut machine = Machine::new();
    type_str(&mut machine, "alice");
    machine.handle(Event::Key(Key::Tab));
    type_str(&mut machine, "s3cret");
    machine.handle(Event::Key(Key::Enter));
    machine.handle(Event::LoginSucceeded {
        token: ALICE_TOKEN.to_string(),
    });
    machine
}

/// Alice has bob's conversation open, with two messages already on screen.
fn with_open_conversation() -> Machine {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![
            conversation(2, "bob", 11, "2026-11-11 11:00:00", Unread::None),
            conversation(3, "carol", 10, "2026-11-11 10:00:00", Unread::Count(1)),
        ],
    });
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(10, 2, "bob", "在吗", "2026-11-11 10:30:00"),
            message(11, 2, "bob", "早", "2026-11-11 10:31:00"),
        ],
    });
    machine
}

fn mids(machine: &Machine) -> Vec<i64> {
    machine
        .state()
        .messages
        .iter()
        .map(|message| message.mid)
        .collect()
}

fn row(machine: &Machine, uid: i32) -> &Conversation {
    machine
        .state()
        .conversations
        .iter()
        .find(|row| row.id == ConversationId::User(uid))
        .expect("conversation row")
}

#[test]
fn a_dead_link_puts_the_reconnect_notice_in_the_status_line() {
    let mut machine = with_open_conversation();
    assert_eq!(machine.state().connection, Connection::Idle);
    assert_eq!(machine.state().status_line(), None);

    let effects = machine.handle(Event::LinkDown);

    assert!(effects.is_empty(), "reconnecting needs no user action");
    assert_eq!(machine.state().connection, Connection::Down);
    assert_eq!(machine.state().screen, Screen::Conversations);
    assert_eq!(
        machine.state().status_line().as_deref(),
        Some(CONNECTION_LOST_STATUS)
    );
    assert_eq!(CONNECTION_LOST_STATUS, "连接断开，正在重连");
}

#[test]
fn the_first_connect_is_not_a_reconnect_but_a_redial_after_a_drop_is() {
    let mut machine = with_open_conversation();

    // The stream's first successful connect after login has nothing to catch up:
    // the list and the open history were just loaded.
    assert_eq!(machine.handle(Event::LinkUp), Vec::new());
    assert_eq!(machine.state().connection, Connection::Live);

    machine.handle(Event::LinkDown);
    let effects = machine.handle(Event::LinkUp);

    assert_eq!(
        effects,
        vec![
            Effect::CatchUpMessages { peer: 2 },
            Effect::RefreshConversations,
        ],
        "the open conversation is refetched and the unread counts are re-pulled"
    );
    assert_eq!(machine.state().connection, Connection::Live);
    assert_eq!(
        machine.state().status_line(),
        None,
        "the reconnect notice disappears as soon as bytes are flowing again"
    );
}

#[test]
fn a_reconnect_with_no_open_conversation_still_refreshes_the_list() {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: Vec::new(),
    });
    assert_eq!(machine.state().active, None);

    machine.handle(Event::LinkDown);
    let effects = machine.handle(Event::LinkUp);

    assert_eq!(effects, vec![Effect::RefreshConversations]);
}

#[test]
fn catching_up_merges_by_message_id_and_keeps_each_message_exactly_once() {
    let mut machine = with_open_conversation();
    machine.handle(Event::LinkDown);
    machine.handle(Event::LinkUp);

    // The redialed stream is live while the history request is still in flight,
    // so the pane already holds message 12 when the snapshot lands.
    machine.handle(Event::Chat(chat(
        12,
        2,
        ConversationId::User(1),
        "刚回来",
        "2026-11-11 12:00:00",
    )));
    assert_eq!(mids(&machine), vec![10, 11, 12]);

    // The snapshot contains what was missed offline (13) and everything the pane
    // already had, including 12.
    machine.handle(Event::CatchUpLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(10, 2, "bob", "在吗", "2026-11-11 10:30:00"),
            message(11, 2, "bob", "早", "2026-11-11 10:31:00"),
            message(12, 2, "bob", "刚回来", "2026-11-11 12:00:00"),
            message(13, 2, "bob", "断线的时候发的", "2026-11-11 12:05:00"),
        ],
    });

    assert_eq!(
        mids(&machine),
        vec![10, 11, 12, 13],
        "no duplicate for 12 and no loss of 13"
    );
    assert_eq!(machine.state().messages.len(), 4);
}

#[test]
fn a_catch_up_that_lands_after_the_user_switched_conversations_is_ignored() {
    let mut machine = with_open_conversation();
    machine.handle(Event::LinkDown);
    assert_eq!(
        machine.handle(Event::LinkUp),
        vec![
            Effect::CatchUpMessages { peer: 2 },
            Effect::RefreshConversations,
        ]
    );

    // The user moved to carol before bob's catch-up answered.
    machine.handle(Event::Key(Key::Down));
    assert_eq!(machine.state().active, Some(ConversationId::User(3)));

    machine.handle(Event::CatchUpLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(14, 2, "bob", "迟到的补拉", "2026-11-11 12:10:00")],
    });

    assert!(
        machine
            .state()
            .messages
            .iter()
            .all(|message| message.mid != 14),
        "bob's history must not be merged into carol's pane"
    );
    assert!(machine.state().pending.is_empty());
}

#[test]
fn a_catch_up_that_includes_an_optimistic_send_leaves_it_once_as_confirmed_history() {
    let mut machine = with_open_conversation();

    // Typed while the link is down: the send path is independent of the push
    // connection and the row is optimistic until the HTTP reply lands.
    machine.handle(Event::LinkDown);
    type_str(&mut machine, "我先说");
    let effects = machine.handle(Event::Key(Key::Enter));
    assert_eq!(
        effects,
        vec![Effect::SendMessage {
            peer: 2,
            local_id: 0,
            text: "我先说".to_string(),
        }],
        "sending keeps working while the push connection is down"
    );
    assert_eq!(machine.state().connection, Connection::Down);
    assert_eq!(
        machine.state().status_line().as_deref(),
        Some(CONNECTION_LOST_STATUS),
        "sending must not hide the reconnect notice"
    );
    machine.handle(Event::MessageSent {
        conversation: ConversationId::User(2),
        local_id: 0,
        mid: 12,
        time: at("2026-11-11 12:00:00"),
    });
    assert_eq!(machine.state().pending.len(), 1);

    machine.handle(Event::LinkUp);
    machine.handle(Event::CatchUpLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(10, 2, "bob", "在吗", "2026-11-11 10:30:00"),
            message(11, 2, "bob", "早", "2026-11-11 10:31:00"),
            message(12, 1, "alice", "我先说", "2026-11-11 12:00:00"),
        ],
    });

    assert!(
        machine.state().pending.is_empty(),
        "the confirmed optimistic row is now history and must not be drawn twice"
    );
    assert_eq!(mids(&machine), vec![10, 11, 12]);
}

#[test]
fn the_list_refresh_after_a_reconnect_carries_the_servers_unread_counts() {
    let mut machine = with_open_conversation();
    machine.handle(Event::LinkDown);
    assert_eq!(
        machine.handle(Event::LinkUp),
        vec![
            Effect::CatchUpMessages { peer: 2 },
            Effect::RefreshConversations,
        ]
    );

    // While offline bob sent two more messages; the server is the authority.
    machine.handle(Event::ConversationsRefreshed {
        items: vec![
            conversation(2, "bob", 30, "2026-11-11 13:00:00", Unread::Count(2)),
            conversation(3, "carol", 10, "2026-11-11 10:00:00", Unread::Count(1)),
        ],
    });

    assert_eq!(row(&machine, 2).mid, 30);
    assert_eq!(row(&machine, 2).unread, Unread::Count(2));
    assert_eq!(machine.state().active, Some(ConversationId::User(2)));
}

#[test]
fn a_failed_catch_up_is_reported_without_emptying_the_pane() {
    let mut machine = with_open_conversation();
    machine.handle(Event::LinkDown);
    machine.handle(Event::LinkUp);

    machine.handle(Event::CatchUpFailed {
        conversation: ConversationId::User(2),
        message: "连接被重置".to_string(),
    });

    assert_eq!(mids(&machine), vec![10, 11]);
    let status = machine.state().status_line().unwrap_or_default();
    assert!(
        status.contains("连接被重置"),
        "the failure shares the status line with the reconnect notice, got {status:?}"
    );
}

#[test]
fn ending_the_session_forgets_the_link_state() {
    let mut machine = with_open_conversation();
    machine.handle(Event::LinkDown);
    assert_eq!(machine.state().connection, Connection::Down);

    machine.handle(Event::TokenExpired);

    assert_eq!(machine.state().screen, Screen::Login);
    assert_eq!(machine.state().connection, Connection::Idle);
    assert_eq!(
        machine.state().status_line(),
        Some("登录已过期，请重新登录".to_string())
    );
}
