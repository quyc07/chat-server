//! Behavioural tests for live messages arriving through the push stream.
//!
//! These use the pure state-machine seam: a [`StreamChat`] event goes in, the
//! conversation list and the open pane come out. No terminal, no server, no
//! sleeping: the debounce is advanced with explicit [`Event::Tick`]s.

use std::time::Duration;

use chat_tui::model::{self, Conversation, ConversationId, Message, StreamChat, Unread};
use chat_tui::state::{Effect, Event, Key, Machine, REFRESH_DEBOUNCE, Screen};
use chrono::NaiveDateTime;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

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

/// Bob's conversation is open (the first row); carol's is only in the list.
fn with_two_conversations(machine: &mut Machine) {
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![
            conversation(2, "bob", 21, "2026-11-11 11:00:00", Unread::None),
            conversation(3, "carol", 20, "2026-11-11 10:00:00", Unread::None),
        ],
    });
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
fn a_message_for_the_open_conversation_appears_without_an_unread_bump() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    assert_eq!(machine.state().active, Some(ConversationId::User(2)));

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
        "no request is needed to display it, but the open conversation stays read"
    );
    let state = machine.state();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].mid, 50);
    assert_eq!(state.messages[0].from_uid, 2);
    assert_eq!(
        state.messages[0].from_name, "bob",
        "the name comes from the open conversation's title"
    );
    assert_eq!(state.messages[0].body, "晚上好");
    assert_eq!(state.messages[0].time, at("2026-11-11 12:00:00"));
    assert_eq!(
        row(&machine, 2).unread,
        Unread::None,
        "the user is looking at the conversation, so it is read"
    );
    assert_eq!(
        row(&machine, 2).mid,
        50,
        "the row summary follows the message"
    );
}

#[test]
fn an_echo_of_a_message_already_in_the_pane_is_deduped_by_mid() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![message(
            42,
            1,
            "alice",
            "已经发过的消息",
            "2026-11-11 09:00:00",
        )],
    });

    // The server echoes the sender's own message back on the stream.
    machine.handle(Event::Chat(chat(
        42,
        1,
        ConversationId::User(2),
        "已经发过的消息",
        "2026-11-11 09:00:00",
    )));

    assert_eq!(
        machine.state().messages.len(),
        1,
        "the echoed mid is already in the pane; it must not be appended twice"
    );
}

#[test]
fn a_message_for_another_conversation_bumps_its_unread_and_moves_it_to_the_top() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    let effects = machine.handle(Event::Chat(chat(
        60,
        3,
        ConversationId::User(1),
        "在吗",
        "2026-11-11 12:00:00",
    )));

    assert!(
        effects.is_empty(),
        "carol's row already exists, so no name lookup is needed"
    );
    let state = machine.state();
    assert_eq!(
        state.conversations[0].id,
        ConversationId::User(3),
        "the conversation with the newest message sorts first"
    );
    assert_eq!(state.conversations[0].unread, Unread::Count(1));
    assert_eq!(state.conversations[0].preview, "在吗");
    assert_eq!(state.conversations[0].mid, 60);
    assert_eq!(
        state.active,
        Some(ConversationId::User(2)),
        "receiving elsewhere does not switch the open conversation"
    );
    assert_eq!(
        state.selected_conversation().map(|row| row.id),
        Some(ConversationId::User(2)),
        "the highlight follows the open conversation, not the new first row"
    );
    assert!(state.messages.is_empty(), "bob's pane is untouched");
}

#[test]
fn incoming_messages_debounce_the_conversation_list_refresh() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    machine.handle(Event::Chat(chat(
        60,
        3,
        ConversationId::User(1),
        "第一条",
        "2026-11-11 12:00:00",
    )));
    assert!(
        machine
            .handle(Event::Tick(Duration::from_secs(1)))
            .is_empty(),
        "one second is inside the debounce window"
    );
    machine.handle(Event::Chat(chat(
        61,
        3,
        ConversationId::User(1),
        "第二条",
        "2026-11-11 12:00:01",
    )));
    assert!(
        machine
            .handle(Event::Tick(Duration::from_secs(1)))
            .is_empty(),
        "a second message restarts the window"
    );
    assert_eq!(
        machine.handle(Event::Tick(Duration::from_secs(1))),
        vec![Effect::RefreshConversations],
        "one refresh after the burst, not one per message"
    );
    assert!(
        machine
            .handle(Event::Tick(Duration::from_secs(1)))
            .is_empty(),
        "the debounce fires once"
    );
    assert_eq!(row(&machine, 3).unread, Unread::Count(2));
}

#[test]
fn the_debounce_fires_exactly_at_the_window_boundary() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::Chat(chat(
        60,
        3,
        ConversationId::User(1),
        "在吗",
        "2026-11-11 12:00:00",
    )));

    assert!(machine.handle(Event::Tick(REFRESH_DEBOUNCE / 2)).is_empty());
    assert_eq!(
        machine.handle(Event::Tick(REFRESH_DEBOUNCE / 2)),
        vec![Effect::RefreshConversations]
    );
}

#[test]
fn a_message_from_a_conversation_never_seen_adds_a_row_and_asks_for_the_name() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    let effects = machine.handle(Event::Chat(chat(
        70,
        4,
        ConversationId::User(1),
        "第一次说话",
        "2026-11-11 12:00:00",
    )));

    assert_eq!(
        effects,
        vec![Effect::LoadPeer { uid: 4 }],
        "the peer's display name has to come from /user/detail/{{uid}}"
    );
    let state = machine.state();
    assert_eq!(state.conversations[0].id, ConversationId::User(4));
    assert_eq!(
        state.conversations[0].title, "用户 4",
        "a placeholder until the name arrives"
    );
    assert_eq!(state.conversations[0].unread, Unread::Count(1));
    assert_eq!(state.conversations[0].preview, "第一次说话");

    // A second message from the same unknown peer reuses the row.
    let effects = machine.handle(Event::Chat(chat(
        71,
        4,
        ConversationId::User(1),
        "还在吗",
        "2026-11-11 12:00:02",
    )));
    assert!(
        effects.is_empty(),
        "the row exists now, so no second lookup"
    );
    assert_eq!(row(&machine, 4).unread, Unread::Count(2));

    machine.handle(Event::PeerLoaded {
        uid: 4,
        name: "dave".to_string(),
    });
    assert_eq!(row(&machine, 4).title, "dave");
}

#[test]
fn a_message_from_an_unknown_group_gets_a_placeholder_without_a_user_lookup() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    let effects = machine.handle(Event::Chat(chat(
        75,
        4,
        ConversationId::Group(8),
        "群里有人说话",
        "2026-11-11 12:00:00",
    )));

    assert!(
        effects.is_empty(),
        "there is no group-name endpoint in this phase, so nothing to look up"
    );
    let state = machine.state();
    assert_eq!(state.conversations[0].id, ConversationId::Group(8));
    assert_eq!(state.conversations[0].title, "群聊 #8");
    assert_eq!(state.conversations[0].unread, Unread::Count(1));
}

#[test]
fn my_own_echo_in_another_conversation_does_not_raise_its_unread_count() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    let effects = machine.handle(Event::Chat(chat(
        80,
        1,
        ConversationId::User(3),
        "我发给 carol 的",
        "2026-11-11 12:00:00",
    )));

    assert!(effects.is_empty());
    assert_eq!(
        row(&machine, 3).unread,
        Unread::None,
        "I sent it; it is not unread for me"
    );
    assert_eq!(row(&machine, 3).preview, "我发给 carol 的");
    assert_eq!(
        machine.state().conversations[0].id,
        ConversationId::User(3),
        "my own message still makes the conversation the most recent"
    );
}

#[test]
fn the_server_refresh_merges_rows_without_switching_the_open_conversation() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);

    machine.handle(Event::ConversationsRefreshed {
        items: vec![
            conversation(3, "carol", 90, "2026-11-11 13:00:00", Unread::Count(5)),
            conversation(5, "erin", 85, "2026-11-11 12:30:00", Unread::None),
        ],
    });

    let state = machine.state();
    assert_eq!(
        state
            .conversations
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![
            ConversationId::User(3),
            ConversationId::User(5),
            ConversationId::User(2),
        ],
        "server rows are merged and the list is sorted by recency"
    );
    assert_eq!(
        row(&machine, 3).unread,
        Unread::Count(5),
        "the server is authoritative"
    );
    assert_eq!(
        state.active,
        Some(ConversationId::User(2)),
        "a refresh never switches the open conversation"
    );
    assert_eq!(
        state.selected_conversation().map(|row| row.id),
        Some(ConversationId::User(2)),
        "the highlight follows the open conversation to its new index"
    );
    assert_eq!(
        row(&machine, 2).mid,
        21,
        "rows missing from page one stay as they were"
    );
}

#[test]
fn authorship_comes_from_the_uid_so_two_users_with_the_same_name_are_distinct() {
    let mut machine = logged_in(1);
    // The peer is also called alice: usernames are not unique in this database.
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(
            2,
            "alice",
            21,
            "2026-11-11 11:00:00",
            Unread::None,
        )],
    });
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![
            message(1, 2, "alice", "另一个 alice 说的", "2026-11-11 09:00:00"),
            message(2, 1, "alice", "我说的", "2026-11-11 09:01:00"),
        ],
    });

    assert_eq!(machine.state().user_uid(), Some(1));
    let views = machine.state().message_views(at("2026-11-11 12:00:00"));
    assert!(!views[0].mine, "a same-named peer's message is not mine");
    assert!(views[1].mine, "my own message is mine");
}

#[test]
fn the_uid_is_read_at_login_and_again_after_a_relogin() {
    let mut machine = logged_in(7);
    assert_eq!(machine.state().user_uid(), Some(7));

    machine.handle(Event::ReloginSucceeded {
        token: token_for(9),
    });

    assert_eq!(
        machine.state().user_uid(),
        Some(9),
        "a replacement token is decoded again instead of trusting the old uid"
    );
}

#[test]
fn stream_traffic_after_the_session_ended_is_ignored() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::TokenExpired);
    assert_eq!(machine.state().screen, Screen::Login);

    let effects = machine.handle(Event::Chat(chat(
        99,
        2,
        ConversationId::User(1),
        "迟到的推送",
        "2026-11-11 12:00:00",
    )));

    assert!(effects.is_empty());
    assert!(machine.state().conversations.is_empty());
    assert!(machine.state().messages.is_empty());
}

/// The wiring only knows the `local_id` echoed back by the send effect.
fn local_id_of(effects: &[Effect]) -> u64 {
    match effects.first() {
        Some(Effect::SendMessage { local_id, .. }) => *local_id,
        other => panic!("expected a send effect, got {other:?}"),
    }
}

#[test]
fn an_echo_that_beats_the_send_response_replaces_the_optimistic_row() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![],
    });
    type_str(&mut machine, "你好");
    machine.handle(Event::Key(Key::Enter));
    assert_eq!(
        machine
            .state()
            .message_views(at("2026-11-11 12:00:00"))
            .len(),
        1,
        "the optimistic row is drawn before the server answers"
    );

    // The push echo wins the race: the pending row has no mid yet, so the echo
    // consumes it by conversation and body instead of appending a second row.
    machine.handle(Event::Chat(chat(
        50,
        1,
        ConversationId::User(2),
        "你好",
        "2026-11-11 12:00:00",
    )));

    let views = machine.state().message_views(at("2026-11-11 12:00:00"));
    assert_eq!(views.len(), 1, "one message, not two");
    assert_eq!(views[0].body, "你好");
    assert!(views[0].mine);
    assert_eq!(machine.state().messages[0].mid, 50);

    // The late send result has no row left to confirm; it must not resurrect it.
    machine.handle(Event::MessageSent {
        conversation: ConversationId::User(2),
        local_id: 0,
        mid: 50,
        time: at("2026-11-11 12:00:00"),
    });
    assert_eq!(
        machine
            .state()
            .message_views(at("2026-11-11 12:00:00"))
            .len(),
        1
    );
}

#[test]
fn an_echo_of_an_already_confirmed_send_is_not_appended_again() {
    let mut machine = logged_in(1);
    with_two_conversations(&mut machine);
    machine.handle(Event::MessagesLoaded {
        conversation: ConversationId::User(2),
        items: vec![],
    });
    type_str(&mut machine, "你好");
    let effects = machine.handle(Event::Key(Key::Enter));
    let local_id = local_id_of(&effects);

    // The HTTP result wins the race: the optimistic row now carries the mid.
    machine.handle(Event::MessageSent {
        conversation: ConversationId::User(2),
        local_id,
        mid: 50,
        time: at("2026-11-11 12:00:00"),
    });
    assert_eq!(
        machine
            .state()
            .message_views(at("2026-11-11 12:00:00"))
            .len(),
        1
    );

    machine.handle(Event::Chat(chat(
        50,
        1,
        ConversationId::User(2),
        "你好",
        "2026-11-11 12:00:00",
    )));

    assert_eq!(
        machine
            .state()
            .message_views(at("2026-11-11 12:00:00"))
            .len(),
        1,
        "the confirmed mid is already in the pane"
    );
    assert_eq!(
        row(&machine, 2).mid,
        50,
        "the list summary still catches up with my own echo"
    );
    assert_eq!(
        machine.handle(Event::Tick(REFRESH_DEBOUNCE)),
        vec![Effect::RefreshConversations],
        "and the authoritative list refresh is scheduled"
    );
}
