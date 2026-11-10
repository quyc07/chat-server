//! Behavioural tests for read-position reporting ("已读位点").
//!
//! These use the pure state-machine seam: the server's conversation list, the
//! open conversation's history, and stream messages go in; the read-position
//! report the wiring must send comes out. Unread counts are never computed here
//! — the list refresh stays the server's authority — so the assertions are about
//! which report is emitted and when, not about local arithmetic.
//!
//! No terminal, no server, no sleeping: the debounce is advanced with explicit
//! [`Event::Tick`]s.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chat_tui::model::{self, Conversation, ConversationId, FoundUser, Message, StreamChat, Unread};
use chat_tui::state::{Effect, Event, Key, Machine, REFRESH_DEBOUNCE};
use chrono::NaiveDateTime;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
}

/// A JWT-shaped token with `{"id": uid}` in its payload, like the server's.
fn token_for(uid: i32) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(format!(
        r#"{{"id":{uid},"name":"alice","email":null,"phone":null,"exp":9999999999}}"#
    ));
    format!("{header}.{payload}.c2lnbmF0dXJl")
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

/// Alice (uid `uid`) logged in with a token that carries her uid.
fn logged_in(uid: i32) -> Machine {
    let mut machine = Machine::new();
    type_str(&mut machine, "alice");
    machine.handle(Event::Key(Key::Tab));
    type_str(&mut machine, "s3cret");
    machine.handle(Event::Key(Key::Enter));
    machine.handle(Event::LoginSucceeded {
        token: token_for(uid),
    });
    machine
}

/// Bob's conversation is the newest and therefore the one the client opens;
/// carol's is only in the list.
fn with_two_conversations(machine: &mut Machine) -> Vec<Effect> {
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![
            conversation(2, "bob", 21, "2026-11-11 11:00:00", Unread::Count(3)),
            conversation(3, "carol", 20, "2026-11-11 10:00:00", Unread::None),
        ],
    })
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
fn opening_a_conversation_reports_its_last_message_as_read() {
    let mut machine = logged_in(1);

    let effects = with_two_conversations(&mut machine);

    assert_eq!(
        effects,
        vec![
            Effect::ReportRead {
                conversation: ConversationId::User(2),
                mid: 21,
            },
            Effect::LoadMessages { peer: 2 },
        ],
        "opening bob's conversation reports bob's newest mid, then loads history"
    );
}

#[test]
fn a_message_arriving_in_the_open_conversation_is_reported_as_read() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(21, 2, "bob", "早", "2026-11-11 11:00:00")],
    });

    let effects = machine.handle(Event::Chat(chat(
        50,
        2,
        ConversationId::User(1),
        "晚上好",
        "2026-11-11 12:00:00",
    )));

    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 50,
        }],
        "the user is looking at bob's conversation, so the new message is read"
    );
    assert_eq!(
        machine.state().conversations[0].unread,
        Unread::Count(3),
        "the row keeps the server's count until the authoritative refresh says otherwise"
    );
}

#[test]
fn history_newer_than_the_list_row_advances_the_read_position() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    // The list row said 21, but the history already has 25: the server's two
    // views can disagree, and the position the user has actually seen is 25.
    let effects = machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(21, 2, "bob", "早", "2026-11-11 11:00:00"),
            message(25, 2, "bob", "刚才", "2026-11-11 11:30:00"),
        ],
    });

    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 25,
        }],
        "the newest message the pane shows is the position that is read"
    );
}

#[test]
fn a_read_position_is_only_reported_when_it_advances() {
    let mut machine = logged_in(1);
    let effects = with_two_conversations(&mut machine);
    assert_eq!(
        effects.first(),
        Some(&Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 21,
        }),
        "opening reported the row's newest mid"
    );

    // The history's newest mid is the one the open already reported.
    let effects = machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(21, 2, "bob", "早", "2026-11-11 11:00:00")],
    });
    assert!(
        effects.is_empty(),
        "the same position is not reported a second time"
    );

    // A newer position is reported...
    let effects = machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(21, 2, "bob", "早", "2026-11-11 11:00:00"),
            message(25, 2, "bob", "刚才", "2026-11-11 11:30:00"),
        ],
    });
    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 25,
        }]
    );

    // ...and reloading the same history stays silent.
    let effects = machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(25, 2, "bob", "刚才", "2026-11-11 11:30:00")],
    });
    assert!(
        effects.is_empty(),
        "a reload of the same history never re-reports"
    );

    // A stream message advances it again.
    let effects = machine.handle(Event::Chat(chat(
        30,
        2,
        ConversationId::User(1),
        "在吗",
        "2026-11-11 12:00:00",
    )));
    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 30,
        }]
    );

    // A redelivery of the same message (reconnect catch-up) does not.
    let effects = machine.handle(Event::Chat(chat(
        30,
        2,
        ConversationId::User(1),
        "在吗",
        "2026-11-11 12:00:00",
    )));
    assert!(effects.is_empty(), "a redelivered mid is not re-reported");
}

#[test]
fn a_successful_report_refreshes_the_list_so_the_badge_disappears() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    let now = at("2026-11-11 12:00:00");
    assert_eq!(
        row(&machine, 2).row_view(now).unread_badge,
        Some("3".to_string()),
        "the server's unread count is what the list shows"
    );

    assert!(
        machine.handle(Event::ReadReported).is_empty(),
        "a successful report changes nothing locally; the list is the server's"
    );
    assert_eq!(
        row(&machine, 2).row_view(now).unread_badge,
        Some("3".to_string()),
        "no local unread arithmetic: the stale count waits for the refresh"
    );
    assert_eq!(
        machine.handle(Event::Tick(REFRESH_DEBOUNCE)),
        vec![Effect::RefreshConversations],
        "the report schedules the authoritative list refresh"
    );

    machine.handle(Event::ConversationsRefreshed {
        items: vec![
            conversation(2, "bob", 21, "2026-11-11 11:00:00", Unread::None),
            conversation(3, "carol", 20, "2026-11-11 10:00:00", Unread::None),
        ],
    });

    assert_eq!(
        row(&machine, 2).row_view(now).unread_badge,
        None,
        "after the report lands, the refresh shows the conversation as read"
    );
}

#[test]
fn a_failed_report_only_reaches_the_status_line_and_is_retried() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(21, 2, "bob", "早", "2026-11-11 11:00:00")],
    });
    type_str(&mut machine, "还没发出去的草稿");
    let effects = machine.handle(Event::Chat(chat(
        50,
        2,
        ConversationId::User(1),
        "晚上好",
        "2026-11-11 12:00:00",
    )));
    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 50,
        }]
    );

    let effects = machine.handle(Event::ReadReportFailed {
        conversation: ConversationId::User(2),
        mid: 50,
        message: "服务器返回 500".to_string(),
    });

    assert!(
        effects.is_empty(),
        "a failed report never blocks on a retry request"
    );
    let state = machine.state();
    assert_eq!(
        state.status.as_deref(),
        Some("已读状态同步失败：服务器返回 500"),
        "the status line carries the reason, and no raw conversation id"
    );
    assert_eq!(
        state.input, "还没发出去的草稿",
        "the draft is untouched: the user keeps typing"
    );
    assert_eq!(state.active, Some(ConversationId::User(2)));
    assert_eq!(
        state.messages.len(),
        2,
        "the pane is untouched: bob's earlier message and the new one"
    );
    assert!(!state.quit, "a read report never ends the session");

    // The position was not accepted, so the next event that shows it retries.
    let effects = machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(50, 2, "bob", "晚上好", "2026-11-11 12:00:00")],
    });
    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 50,
        }],
        "a rejected position is retried instead of silently staying unread"
    );
}

#[test]
fn opening_a_conversation_with_no_messages_reports_nothing() {
    let mut machine = logged_in(1);

    // `/open <name>` can put a conversation with no history in the list. Message
    // ids start at 1 (the server's sequence), so 0 means "no position yet", and
    // reporting it would make the server upsert a row for the peer.
    let effects = machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(
            4,
            "dave",
            0,
            "2026-11-11 11:00:00",
            Unread::None,
        )],
    });

    assert_eq!(
        effects,
        vec![Effect::LoadMessages { peer: 4 }],
        "nothing to report until a message exists"
    );
    assert_eq!(machine.state().active, Some(ConversationId::User(4)));
}

#[test]
fn a_new_login_does_not_inherit_the_previous_sessions_read_positions() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    machine.handle(Event::TokenExpired);
    assert_eq!(
        machine.state().screen,
        chat_tui::state::Screen::Login,
        "an expired token sends the user back to the login screen"
    );

    // The name is kept and the password has to be typed again.
    type_str(&mut machine, "s3cret");
    assert_eq!(
        machine.handle(Event::Key(Key::Enter)),
        vec![Effect::Login {
            name: "alice".to_string(),
            password: "s3cret".to_string(),
        }]
    );
    machine.handle(Event::LoginSucceeded {
        token: token_for(1),
    });

    let effects = with_two_conversations(&mut machine);

    assert_eq!(
        effects.first(),
        Some(&Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 21,
        }),
        "positions reported by the previous session are not assumed to still be read"
    );
}

/// `/open <name>` reaches the pane through its own path (`open_peer`), not
/// through the list selection, so it must report the position too.
#[test]
fn a_conversation_opened_with_the_open_command_is_reported_as_read() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    assert_eq!(
        machine.state().active,
        Some(ConversationId::User(2)),
        "bob's conversation is the open one"
    );

    type_str(&mut machine, "/open carol");
    assert_eq!(
        machine.handle(Event::Key(Key::Enter)),
        vec![Effect::LookupOpen {
            query: "carol".to_string(),
        }]
    );
    let effects = machine.handle(Event::OpenUsersFound {
        query: "carol".to_string(),
        users: vec![FoundUser {
            id: 3,
            name: "carol".to_string(),
        }],
    });

    assert_eq!(
        effects,
        vec![
            Effect::ReportRead {
                conversation: ConversationId::User(3),
                mid: 20,
            },
            Effect::LoadMessages { peer: 3 },
        ],
        "opening carol with /open marks her conversation read, like a list switch"
    );

    let effects = machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(3),
        items: vec![message(45, 3, "carol", "刚发的", "2026-11-11 12:00:00")],
    });
    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(3),
            mid: 45,
        }],
        "and the history the pane shows advances the position"
    );
}

/// `/open` can reach a peer the server has never recorded. There is no position
/// to report — the send endpoint writes the read index for the first message.
#[test]
fn a_conversation_opened_with_the_open_command_and_no_history_reports_nothing() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    type_str(&mut machine, "/open 张三");
    machine.handle(Event::Key(Key::Enter));

    let effects = machine.handle(Event::OpenUsersFound {
        query: "张三".to_string(),
        users: vec![FoundUser {
            id: 9,
            name: "张三".to_string(),
        }],
    });
    assert_eq!(
        effects,
        vec![Effect::LoadMessages { peer: 9 }],
        "no row and no history means no position to report"
    );

    let effects = machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(9),
        items: vec![],
    });
    assert!(effects.is_empty(), "an empty history still has no position");
}

/// A reconnect redraws the pane from history rather than from the stream. The
/// messages that arrived during the outage are on screen, so they are read.
#[test]
fn messages_caught_up_after_a_reconnect_are_reported_as_read() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(21, 2, "bob", "早", "2026-11-11 11:00:00")],
    });

    let effects = machine.handle(Event::CatchUpLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(21, 2, "bob", "早", "2026-11-11 11:00:00"),
            message(60, 2, "bob", "断线期间发的", "2026-11-11 12:00:00"),
        ],
    });

    assert_eq!(
        effects,
        vec![Effect::ReportRead {
            conversation: ConversationId::User(2),
            mid: 60,
        }],
        "the missed message is in the pane, so it is read"
    );

    let effects = machine.handle(Event::CatchUpLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(60, 2, "bob", "断线期间发的", "2026-11-11 12:00:00")],
    });
    assert!(
        effects.is_empty(),
        "a catch-up that adds nothing new does not re-report"
    );
}
