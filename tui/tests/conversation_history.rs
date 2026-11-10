//! Behavioural tests for opening a conversation and showing its history.
//!
//! These go through the pure state-machine seam: events in, state and effects out.
//! No terminal, no server, no sleeping.

use chat_tui::model::{self, Conversation, ConversationId, Message, Unread};
use chat_tui::state::{Effect, Event, Key, Machine, Screen};
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

fn type_str(machine: &mut Machine, text: &str) {
    for c in text.chars() {
        machine.handle(Event::Key(Key::Char(c)));
    }
}

/// A JWT-shaped access token whose payload is `{"id":1,...}`: the state machine
/// reads the caller's uid out of it, and `message_views` needs that uid to tell
/// alice's own rows from bob's. `c2ln` is the (unchecked) signature segment.
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

/// Alice's list with two direct conversations: bob is the freshest, carol older.
fn two_conversations(machine: &mut Machine) -> Vec<Effect> {
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![
            conversation(2, "bob", 21, "2026-11-11 11:00:00", Unread::Count(1)),
            conversation(3, "carol", 20, "2026-11-11 10:00:00", Unread::None),
        ],
    })
}

#[test]
fn the_first_conversation_opens_as_soon_as_the_list_arrives() {
    let mut machine = logged_in();

    let effects = two_conversations(&mut machine);

    assert_eq!(
        effects,
        vec![
            // Opening is reading: the position goes to the server first.
            Effect::ReportRead {
                conversation: ConversationId::User(2),
                mid: 21,
            },
            Effect::LoadMessages { peer: 2 },
        ],
        "the highlighted row's history is requested without another keypress"
    );
    assert_eq!(machine.state().active, Some(ConversationId::User(2)));
    assert!(machine.state().messages_loading);
    assert!(machine.state().messages.is_empty());
    assert_eq!(machine.state().screen, Screen::Conversations);
}

#[test]
fn moving_to_another_row_switches_the_pane_and_drops_the_previous_history() {
    let mut machine = logged_in();
    two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(1, 2, "bob", "早上好", "2026-11-11 09:00:00"),
            message(2, 1, "alice", "早", "2026-11-11 09:01:00"),
        ],
    });
    assert_eq!(machine.state().messages.len(), 2);

    let effects = machine.handle(Event::Key(Key::Down));

    assert_eq!(
        effects,
        vec![
            Effect::ReportRead {
                conversation: ConversationId::User(3),
                mid: 20,
            },
            Effect::LoadMessages { peer: 3 },
        ]
    );
    assert_eq!(machine.state().active, Some(ConversationId::User(3)));
    assert_eq!(
        machine.state().selected_conversation().map(|row| row.id),
        Some(ConversationId::User(3)),
        "the highlight and the pane agree"
    );
    assert!(
        machine.state().messages.is_empty(),
        "no message from bob may survive into carol's pane"
    );
    assert!(machine.state().messages_loading);
}

#[test]
fn a_history_response_for_a_conversation_the_user_already_left_is_ignored() {
    let mut machine = logged_in();
    two_conversations(&mut machine);
    // bob's request is still in flight when the user moves to carol.
    machine.handle(Event::Key(Key::Down));

    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(1, 2, "bob", "迟到的响应", "2026-11-11 09:00:00")],
    });

    assert_eq!(machine.state().active, Some(ConversationId::User(3)));
    assert!(
        machine.state().messages.is_empty(),
        "a late response must not repaint the pane the user left"
    );
    assert!(
        machine.state().messages_loading,
        "carol's request is still the one in flight"
    );
}

#[test]
fn reopening_a_conversation_replaces_the_whole_message_stream() {
    let mut machine = logged_in();
    two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(1, 2, "bob", "bob only", "2026-11-11 09:00:00")],
    });

    machine.handle(Event::Key(Key::Down));
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(3),
        items: vec![
            message(3, 3, "carol", "carol one", "2026-11-11 10:00:00"),
            message(4, 1, "alice", "carol two", "2026-11-11 10:01:00"),
        ],
    });
    assert_eq!(machine.state().messages.len(), 2);

    machine.handle(Event::Key(Key::Up));
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(1, 2, "bob", "bob only", "2026-11-11 09:00:00")],
    });

    let bodies: Vec<_> = machine
        .state()
        .messages
        .iter()
        .map(|m| m.body.as_str())
        .collect();
    assert_eq!(bodies, vec!["bob only"], "carol's rows are gone");
}

#[test]
fn each_message_carries_its_side_a_sender_and_a_time_label() {
    let mut machine = logged_in();
    two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(1, 2, "bob", "你好，世界", "2026-11-11 09:00:00"),
            message(2, 1, "alice", "hello\nworld", "2026-11-11 09:01:00"),
        ],
    });

    let views = machine.state().message_views(at("2026-11-11 12:00:00"));

    assert_eq!(views.len(), 2);
    assert!(!views[0].mine, "bob's words are the peer's side");
    assert_eq!(views[0].sender, "bob");
    assert_eq!(views[0].body, "你好，世界");
    assert_eq!(views[0].time_label, "09:00");
    assert!(views[1].mine, "alice's own words are her side");
    assert_eq!(views[1].sender, "alice");
    assert_eq!(
        views[1].body, "hello\nworld",
        "newlines survive for wrapping"
    );
    assert_eq!(views[1].time_label, "09:01");
}

#[test]
fn a_non_friend_404_is_a_readable_status_line_not_a_crash() {
    let mut machine = logged_in();
    two_conversations(&mut machine);

    machine.handle(Event::MessagesFailed {
        conversation: ConversationId::User(2),
        message: "用户2不是您的好友".to_string(),
    });

    let state = machine.state();
    assert_eq!(
        state.status.as_deref(),
        Some("无法加载聊天记录：用户2不是您的好友")
    );
    assert!(!state.messages_loading);
    assert!(state.messages.is_empty());
    assert_eq!(state.conversations.len(), 2, "the list is untouched");
    assert_eq!(state.screen, Screen::Conversations);
    assert!(!state.quit);
}

#[test]
fn selecting_a_group_row_explains_that_group_history_is_not_implemented() {
    let mut machine = logged_in();
    let effects = machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![Conversation {
            id: ConversationId::Group(7),
            title: "群聊".to_string(),
            mid: 30,
            preview: "hi".to_string(),
            time: at("2026-11-11 11:00:00"),
            unread: Unread::None,
            last_sender: Some("bob".to_string()),
        }],
    });

    assert!(
        effects.is_empty(),
        "a direct-message history request for a group id cannot succeed"
    );
    assert_eq!(machine.state().active, Some(ConversationId::Group(7)));
    assert!(machine.state().messages.is_empty());
    assert_eq!(machine.state().status.as_deref(), Some("群聊功能暂未实现"));
    assert!(!machine.state().messages_loading);
}

#[test]
fn a_thousand_messages_are_all_kept_and_the_pane_scrolls_by_pages() {
    let mut machine = logged_in();
    two_conversations(&mut machine);
    let base = at("2026-11-01 00:00:00");
    let items: Vec<_> = (1..=1000)
        .map(|i| {
            let time = base + chrono::Duration::seconds(i);
            message(
                i,
                2,
                "bob",
                &format!("消息 {i}"),
                &time.format(model::WIRE_TIME_FORMAT).to_string(),
            )
        })
        .collect();
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items,
    });

    assert_eq!(
        machine.state().messages.len(),
        1000,
        "the backend ceiling renders in full"
    );
    assert_eq!(machine.state().messages.last().map(|m| m.mid), Some(1000));

    assert!(machine.handle(Event::Key(Key::PageUp)).is_empty());
    assert_eq!(machine.state().message_scroll_up, 10);
    machine.handle(Event::Key(Key::PageUp));
    assert_eq!(machine.state().message_scroll_up, 20);
    machine.handle(Event::Key(Key::PageDown));
    assert_eq!(machine.state().message_scroll_up, 10);
    machine.handle(Event::Key(Key::PageDown));
    machine.handle(Event::Key(Key::PageDown));
    assert_eq!(
        machine.state().message_scroll_up,
        0,
        "scrolling stops at the newest message"
    );

    machine.handle(Event::Key(Key::PageUp));
    machine.handle(Event::Key(Key::Down));
    assert_eq!(
        machine.state().message_scroll_up,
        0,
        "switching conversations returns to the bottom"
    );
}

#[test]
fn a_list_refresh_keeps_the_open_conversation_and_does_not_reload_it() {
    let mut machine = logged_in();
    two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(1, 2, "bob", "keep me", "2026-11-11 09:00:00")],
    });

    // A later refresh reorders the rows (carol is now freshest).
    let effects = machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![
            conversation(3, "carol", 25, "2026-11-11 12:00:00", Unread::None),
            conversation(2, "bob", 21, "2026-11-11 11:00:00", Unread::Count(1)),
        ],
    });

    assert!(effects.is_empty(), "the pane keeps what it already has");
    assert_eq!(machine.state().active, Some(ConversationId::User(2)));
    assert_eq!(
        machine.state().selected_conversation().map(|row| row.id),
        Some(ConversationId::User(2)),
        "the highlight follows the open conversation through a reorder"
    );
    assert_eq!(machine.state().messages.len(), 1);
}

#[test]
fn ending_the_session_clears_the_history_pane() {
    let mut machine = logged_in();
    two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(1, 2, "bob", "private", "2026-11-11 09:00:00")],
    });

    machine.handle(Event::TokenExpired);

    let state = machine.state();
    assert!(
        state.messages.is_empty(),
        "history is not left for the next login"
    );
    assert_eq!(state.active, None);
    assert!(!state.messages_loading);
    assert_eq!(state.screen, Screen::Login);
}
