//! Behavioural tests for the `/open <name>` cold-start path.
//!
//! Same seam as the rest of the client: events in, state and effects out. No
//! I/O and no terminal, so every assertion is about what the user can observe:
//! which conversation is open, what the list contains, and what the status line
//! says.

use chat_tui::model::{self, Conversation, ConversationId, FoundUser, Unread};
use chat_tui::state::{Effect, Event, Key, Machine, REFRESH_DEBOUNCE};
use chrono::NaiveDateTime;
use std::time::Duration;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
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

fn found(id: i32, name: &str) -> FoundUser {
    FoundUser {
        id,
        name: name.to_string(),
    }
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
    assert_eq!(
        machine.handle(Event::Key(Key::Enter)),
        vec![Effect::Login {
            name: "alice".to_string(),
            password: "s3cret".to_string(),
        }]
    );
    machine.handle(Event::LoginSucceeded {
        token: "tok-1".to_string(),
    });
    machine
}

/// Several matches must be shown, never silently resolved to the first row. The
/// chooser owns the keyboard, so arrows move inside it and Enter opens the
/// highlighted user instead of sending the message box's (empty) text.
#[test]
fn several_matches_are_offered_and_the_highlighted_one_is_opened() {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(2, "bob", 21, "2026-11-11 11:00:00")],
    });
    type_str(&mut machine, "/open al");
    machine.handle(Event::Key(Key::Enter));

    let effects = machine.handle(Event::OpenUsersFound {
        query: "al".to_string(),
        users: vec![found(3, "alice"), found(5, "alice-bot"), found(6, "al")],
    });

    assert!(
        effects.is_empty(),
        "a chooser is not a send and not a history load yet"
    );
    assert_eq!(machine.state().open.choices.len(), 3);
    assert_eq!(machine.state().open.selected, 0);
    assert_eq!(
        machine.state().active,
        Some(ConversationId::User(2)),
        "nothing opened until the user chooses"
    );

    machine.handle(Event::Key(Key::Down));
    machine.handle(Event::Key(Key::Down));
    assert_eq!(machine.state().open.selected, 2);

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(effects, vec![Effect::LoadMessages { peer: 6 }]);
    assert_eq!(machine.state().active, Some(ConversationId::User(6)));
    assert!(
        machine.state().open.choices.is_empty(),
        "the chooser closed"
    );
}

/// Esc cancels the chooser rather than quitting, and ordinary typing works
/// again immediately afterwards.
#[test]
fn cancelling_the_chooser_returns_to_ordinary_typing() {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(2, "bob", 21, "2026-11-11 11:00:00")],
    });
    type_str(&mut machine, "/open al");
    machine.handle(Event::Key(Key::Enter));
    machine.handle(Event::OpenUsersFound {
        query: "al".to_string(),
        users: vec![found(3, "alice"), found(5, "alice-bot")],
    });

    let effects = machine.handle(Event::Key(Key::Esc));

    assert!(effects.is_empty());
    assert!(!machine.state().quit, "Esc cancels the chooser here");
    assert!(machine.state().open.choices.is_empty());
    assert_eq!(
        machine.state().active,
        Some(ConversationId::User(2)),
        "the previously open conversation is untouched"
    );

    type_str(&mut machine, "q/还是消息");
    let effects = machine.handle(Event::Key(Key::Enter));
    assert_eq!(
        effects,
        vec![Effect::SendMessage {
            peer: 2,
            local_id: 0,
            text: "q/还是消息".to_string(),
        }],
        "after cancelling, Enter is a send again"
    );
}

/// A name that matches nobody is stated plainly and changes nothing.
#[test]
fn no_match_is_reported_and_nothing_opens() {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(2, "bob", 21, "2026-11-11 11:00:00")],
    });
    type_str(&mut machine, "/open 不存在");
    machine.handle(Event::Key(Key::Enter));

    let effects = machine.handle(Event::OpenUsersFound {
        query: "不存在".to_string(),
        users: vec![],
    });

    assert!(effects.is_empty());
    assert_eq!(
        machine.state().status.as_deref(),
        Some("找不到匹配“不存在”的用户")
    );
    assert!(machine.state().open.choices.is_empty());
    assert!(!machine.state().quit);
    assert_eq!(
        machine.state().active,
        Some(ConversationId::User(2)),
        "the previous conversation stays open"
    );
}

/// An answer to an older lookup must never open someone: the second lookup's
/// query is the one in flight.
#[test]
fn a_reply_to_a_superseded_lookup_is_ignored() {
    let mut machine = logged_in();
    type_str(&mut machine, "/open aa");
    machine.handle(Event::Key(Key::Enter));
    type_str(&mut machine, "/open bb");
    machine.handle(Event::Key(Key::Enter));

    let stale = machine.handle(Event::OpenUsersFound {
        query: "aa".to_string(),
        users: vec![found(9, "aa-user")],
    });

    assert!(stale.is_empty());
    assert_eq!(
        machine.state().active,
        None,
        "the stale reply opened nobody"
    );

    let fresh = machine.handle(Event::OpenUsersFound {
        query: "bb".to_string(),
        users: vec![found(10, "bb-user")],
    });
    assert_eq!(fresh, vec![Effect::LoadMessages { peer: 10 }]);
    assert_eq!(machine.state().active, Some(ConversationId::User(10)));
}

/// Only the exact command word is intercepted; `/openfoo` stays a message.
#[test]
fn a_message_that_merely_starts_with_the_command_word_is_still_sent() {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(2, "bob", 21, "2026-11-11 11:00:00")],
    });
    type_str(&mut machine, "/openfoo");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::SendMessage {
            peer: 2,
            local_id: 0,
            text: "/openfoo".to_string(),
        }]
    );
}

/// `/open` with no name explains the syntax instead of searching for nothing.
#[test]
fn open_without_a_name_shows_usage() {
    let mut machine = logged_in();
    type_str(&mut machine, "/open");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert!(effects.is_empty());
    assert_eq!(
        machine.state().status.as_deref(),
        Some("用法：/open <名字>")
    );
    assert!(!machine.state().open.searching);
}

/// The message box command is recognised on submit, not while typing, so `/`,
/// `q`, and CJK text still enter the box like any other message.
#[test]
fn slash_open_with_a_name_asks_the_server_for_matching_users() {
    let mut machine = logged_in();
    type_str(&mut machine, "/open 张");
    assert_eq!(
        machine.state().input,
        "/open 张",
        "the command is ordinary text until Enter"
    );

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::LookupOpen {
            query: "张".to_string()
        }]
    );
    assert_eq!(machine.state().input, "", "the command is consumed");
    assert!(machine.state().open.searching);
}

/// A single match opens at once. The conversation is not in the list yet — the
/// server has never recorded it — so it is active purely by peer id, shows the
/// name the lookup returned, and loads an empty history it can be typed into.
#[test]
fn a_single_match_opens_an_empty_conversation_outside_the_list() {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(2, "bob", 21, "2026-11-11 11:00:00")],
    });
    type_str(&mut machine, "/open 张");
    machine.handle(Event::Key(Key::Enter));

    let effects = machine.handle(Event::OpenUsersFound {
        query: "张".to_string(),
        users: vec![found(9, "张三")],
    });

    assert_eq!(effects, vec![Effect::LoadMessages { peer: 9 }]);
    assert_eq!(machine.state().active, Some(ConversationId::User(9)));
    assert_eq!(
        machine.state().active_title().as_deref(),
        Some("张三"),
        "the pane can name a peer the list has never seen"
    );
    assert!(machine.state().messages.is_empty());
    assert!(machine.state().messages_loading);
    assert_eq!(
        machine.state().conversations.len(),
        1,
        "opening does not fabricate a list row; the server owns that"
    );
    assert_eq!(
        machine.state().highlighted_index(),
        None,
        "no row means no list highlight, not a stranger's row"
    );
}

/// The server records the conversation only when the first message is sent.
/// After that send is accepted, the client asks the server for the list again,
/// and the conversation appears there on its own — no local fabrication of the
/// row and no second command from the user.
#[test]
fn the_first_message_makes_a_new_conversation_appear_in_the_list() {
    let mut machine = logged_in();
    type_str(&mut machine, "/open 张三");
    machine.handle(Event::Key(Key::Enter));
    machine.handle(Event::OpenUsersFound {
        query: "张三".to_string(),
        users: vec![found(9, "张三")],
    });

    type_str(&mut machine, "你好");
    let effects = machine.handle(Event::Key(Key::Enter));
    let local_id = match effects.as_slice() {
        [
            Effect::SendMessage {
                peer: 9, local_id, ..
            },
        ] => *local_id,
        other => panic!("expected one send effect, got {other:?}"),
    };
    assert!(
        machine.state().conversations.is_empty(),
        "the list knows nothing about this conversation yet"
    );

    machine.handle(Event::MessageSent {
        conversation: ConversationId::User(9),
        local_id,
        mid: 100,
        time: at("2026-11-11 12:00:00"),
    });

    assert!(
        machine
            .handle(Event::Tick(REFRESH_DEBOUNCE - Duration::from_millis(1)))
            .is_empty(),
        "the refresh is debounced"
    );
    let effects = machine.handle(Event::Tick(Duration::from_millis(1)));
    assert_eq!(
        effects,
        vec![Effect::RefreshConversations],
        "the server's list is asked again after the first send"
    );

    machine.handle(Event::ConversationsRefreshed {
        items: vec![conversation(9, "张三", 100, "2026-11-11 12:00:00")],
    });

    let rows = &machine.state().conversations;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, ConversationId::User(9));
    assert_eq!(rows[0].title, "张三");
    assert_eq!(
        machine.state().highlighted_index(),
        Some(0),
        "the now-recorded conversation is the highlighted one"
    );
    assert_eq!(machine.state().active, Some(ConversationId::User(9)));
}

/// A message that merely starts with a slash, and one that contains `q`, must
/// still be sent as text: the command is only the exact `/open` word.
#[test]
fn ordinary_text_with_q_a_slash_and_chinese_is_still_a_message() {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(2, "bob", 21, "2026-11-11 11:00:00")],
    });
    type_str(&mut machine, "q/你好/世界");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::SendMessage {
            peer: 2,
            local_id: 0,
            text: "q/你好/世界".to_string(),
        }]
    );
    assert!(!machine.state().quit, "typing q must never quit the client");
}
