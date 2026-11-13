//! Behavioural tests for the pure state machine seam.
//!
//! Everything here is synchronous and has no I/O: events go in, state and
//! effects come out. These are the behaviours the user chose to have covered.

use std::time::Duration;

use chat_tui::model::{self, Conversation, ConversationId, Unread};
use chat_tui::state::{
    Effect, Event, Key, LayoutMode, LoginMode, Machine, PAGE_LIMIT, RELOGIN_AFTER, Screen,
};
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

fn fill_credentials(machine: &mut Machine, name: &str, password: &str) {
    type_str(machine, name);
    machine.handle(Event::Key(Key::Tab));
    type_str(machine, password);
}

fn logged_in() -> Machine {
    let mut machine = Machine::new();
    fill_credentials(&mut machine, "alice", "s3cret");
    assert_eq!(
        machine.handle(Event::Key(Key::Enter)),
        vec![Effect::Login {
            name: "alice".to_string(),
            password: "s3cret".to_string(),
        }]
    );
    let effects = machine.handle(Event::LoginSucceeded {
        token: token_for(1),
    });
    assert_eq!(effects, vec![Effect::LoadHistory { page: 1 }]);
    machine
}

fn one_row_list() -> Machine {
    let mut machine = logged_in();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(
            7,
            "bob",
            11,
            "2026-11-11 10:00:00",
            Unread::Count(2),
        )],
    });
    machine
}

#[test]
fn starts_on_the_login_screen_with_no_session() {
    let machine = Machine::new();
    let state = machine.state();

    assert_eq!(state.screen, Screen::Login);
    assert_eq!(state.form.mode, LoginMode::Login);
    assert!(state.status.is_none());
    assert!(state.token().is_none());
    assert!(!state.is_logged_in());
}

#[test]
fn typing_credentials_and_pressing_enter_emits_a_login_effect() {
    let mut machine = Machine::new();
    fill_credentials(&mut machine, "alice", "s3cret");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::Login {
            name: "alice".to_string(),
            password: "s3cret".to_string(),
        }]
    );
    assert!(machine.state().form.busy, "a request is on the wire");
}

#[test]
fn empty_credentials_are_rejected_locally_without_a_request() {
    let mut machine = Machine::new();

    let effects = machine.handle(Event::Key(Key::Enter));

    assert!(effects.is_empty());
    assert_eq!(
        machine.state().status.as_deref(),
        Some("请输入用户名和密码")
    );
    assert!(!machine.state().form.busy);
}

#[test]
fn backspace_edits_the_focused_field() {
    let mut machine = Machine::new();
    type_str(&mut machine, "aliX");
    machine.handle(Event::Key(Key::Backspace));
    machine.handle(Event::Key(Key::Tab));
    type_str(&mut machine, "pw");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::Login {
            name: "ali".to_string(),
            password: "pw".to_string(),
        }]
    );
}

#[test]
fn a_typed_q_on_the_login_screen_is_text_not_quit() {
    let mut machine = Machine::new();

    machine.handle(Event::Key(Key::Char('q')));

    assert!(!machine.state().quit);
    assert_eq!(machine.state().form.name, "q");
}

#[test]
fn login_failure_keeps_the_login_screen_and_shows_the_server_text() {
    let mut machine = Machine::new();
    fill_credentials(&mut machine, "alice", "wrong");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::LoginFailed("用户名或密码错误".to_string()));

    assert_eq!(machine.state().screen, Screen::Login);
    assert_eq!(machine.state().status.as_deref(), Some("用户名或密码错误"));
    assert!(!machine.state().form.busy, "the form can be retried");
    assert!(machine.state().token().is_none());

    let retry = machine.handle(Event::Key(Key::Enter));
    assert_eq!(retry.len(), 1, "retrying the same credentials is allowed");
}

#[test]
fn login_success_opens_the_list_and_requests_the_first_page() {
    let mut machine = Machine::new();
    fill_credentials(&mut machine, "alice", "s3cret");
    machine.handle(Event::Key(Key::Enter));

    let effects = machine.handle(Event::LoginSucceeded {
        token: token_for(1),
    });

    assert_eq!(effects, vec![Effect::LoadHistory { page: 1 }]);
    assert_eq!(machine.state().screen, Screen::Conversations);
    assert_eq!(machine.state().token(), Some(token_for(1).as_str()));
    assert_eq!(machine.state().user_name(), Some("alice"));
    assert!(machine.state().loading);
    assert!(machine.state().status.is_none());
}

#[test]
fn register_mode_is_a_toggle_and_register_success_logs_in_with_same_credentials() {
    let mut machine = Machine::new();
    machine.handle(Event::Key(Key::ToggleMode));
    assert_eq!(machine.state().form.mode, LoginMode::Register);

    fill_credentials(&mut machine, "newbie", "pw");
    let effects = machine.handle(Event::Key(Key::Enter));
    assert_eq!(
        effects,
        vec![Effect::Register {
            name: "newbie".to_string(),
            password: "pw".to_string(),
        }]
    );

    let effects = machine.handle(Event::RegisterSucceeded);
    assert_eq!(
        effects,
        vec![Effect::Login {
            name: "newbie".to_string(),
            password: "pw".to_string(),
        }],
        "register does not create a session server-side, so log in right after"
    );
    assert_eq!(machine.state().screen, Screen::Login);
    assert_eq!(
        machine.state().status.as_deref(),
        Some("注册成功，正在登录…")
    );

    machine.handle(Event::LoginSucceeded {
        token: token_for(2),
    });
    assert_eq!(machine.state().screen, Screen::Conversations);
    assert_eq!(machine.state().token(), Some(token_for(2).as_str()));
}

#[test]
fn register_success_switches_the_form_to_login_mode() {
    let mut machine = Machine::new();
    machine.handle(Event::Key(Key::ToggleMode));
    fill_credentials(&mut machine, "newbie", "pw");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::RegisterSucceeded);

    assert_eq!(
        machine.state().form.mode,
        LoginMode::Login,
        "the account now exists; a retry must log in, not register again"
    );
}

#[test]
fn the_mode_toggle_is_ignored_while_a_request_is_in_flight() {
    let mut machine = Machine::new();
    fill_credentials(&mut machine, "alice", "pw");
    machine.handle(Event::Key(Key::Enter));
    assert!(machine.state().form.busy);

    machine.handle(Event::Key(Key::ToggleMode));

    assert_eq!(
        machine.state().form.mode,
        LoginMode::Login,
        "the in-flight response must match the mode it was sent with"
    );
}

#[test]
fn register_failure_stays_on_the_form_and_shows_the_server_text() {
    let mut machine = Machine::new();
    machine.handle(Event::Key(Key::ToggleMode));
    fill_credentials(&mut machine, "taken", "pw");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::RegisterFailed("用户名 taken 已存在".to_string()));

    assert_eq!(machine.state().screen, Screen::Login);
    assert_eq!(machine.state().form.mode, LoginMode::Register);
    assert_eq!(
        machine.state().status.as_deref(),
        Some("用户名 taken 已存在")
    );
    assert!(!machine.state().form.busy);
}

#[test]
fn conversation_list_is_ordered_by_recency_with_mid_as_tie_break() {
    let mut machine = logged_in();

    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![
            conversation(1, "oldest", 10, "2026-11-11 09:00:00", Unread::None),
            conversation(2, "newest", 30, "2026-11-11 11:00:00", Unread::None),
            conversation(3, "middle", 20, "2026-11-11 10:00:00", Unread::None),
            conversation(
                4,
                "same-second-high-mid",
                42,
                "2026-11-11 11:00:00",
                Unread::None,
            ),
            conversation(
                5,
                "same-second-low-mid",
                41,
                "2026-11-11 11:00:00",
                Unread::None,
            ),
        ],
    });

    let titles: Vec<_> = machine
        .state()
        .conversations
        .iter()
        .map(|c| c.title.as_str())
        .collect();
    assert_eq!(
        titles,
        vec![
            "same-second-high-mid",
            "same-second-low-mid",
            "newest",
            "middle",
            "oldest"
        ]
    );
    assert_eq!(machine.state().selected, 0);
}

#[test]
fn unread_badges_distinguish_all_unread_from_a_number() {
    let now = at("2026-11-11 12:00:00");
    let all = conversation(1, "a", 1, "2026-11-11 11:00:00", Unread::All).row_view(now);
    let some = conversation(2, "b", 2, "2026-11-11 11:00:00", Unread::Count(3)).row_view(now);
    let none = conversation(3, "c", 3, "2026-11-11 11:00:00", Unread::None).row_view(now);

    assert_eq!(all.unread_badge.as_deref(), Some("●"));
    assert_eq!(some.unread_badge.as_deref(), Some("3"));
    assert_eq!(none.unread_badge, None);
    assert_ne!(
        all.unread_badge.as_deref(),
        Some("1"),
        "\"all unread\" must never look like a count"
    );
}

#[test]
fn rows_carry_name_excerpt_and_a_time_label() {
    let now = at("2026-11-11 12:00:00");
    let today = conversation(1, "bob", 1, "2026-11-11 09:30:00", Unread::None).row_view(now);
    let earlier = conversation(2, "carol", 2, "2026-11-09 18:05:00", Unread::None).row_view(now);

    assert_eq!(today.title, "bob");
    assert_eq!(today.preview, "last message 1");
    assert_eq!(today.time_label, "09:30");
    assert_eq!(earlier.time_label, "11-09 18:05");
}

#[test]
fn scrolling_past_the_last_row_loads_the_next_page_then_merges_without_duplicates() {
    let mut machine = logged_in();
    let base = at("2026-11-01 00:00:00");
    let first_page: Vec<_> = (0..PAGE_LIMIT)
        .map(|i| {
            let time = base + chrono::Duration::seconds(i as i64);
            conversation(
                i as i32,
                &format!("peer-{i}"),
                i as i64,
                &time.format(model::WIRE_TIME_FORMAT).to_string(),
                Unread::None,
            )
        })
        .collect();
    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: first_page,
    });
    assert!(machine.state().has_more, "a full page may have a successor");
    assert_eq!(machine.state().conversations.len(), PAGE_LIMIT as usize);

    for _ in 0..(PAGE_LIMIT - 1) {
        machine.handle(Event::Key(Key::Down));
    }
    assert_eq!(
        machine.state().selected,
        PAGE_LIMIT as usize - 1,
        "selection reached the bottom row"
    );

    let effects = machine.handle(Event::Key(Key::Down));
    assert_eq!(effects, vec![Effect::LoadHistory { page: 2 }]);
    assert!(machine.state().loading);

    // Page two overlaps row 49 (now fresher) and adds a new, older row.
    let overlap = conversation(
        49,
        "peer-49-fresh",
        149,
        "2026-11-11 12:00:00",
        Unread::Count(1),
    );
    let older = conversation(99, "peer-99", 1, "2026-10-01 00:00:00", Unread::None);
    machine.handle(Event::ConversationsLoaded {
        page: 2,
        items: vec![overlap, older],
    });

    let rows = &machine.state().conversations;
    assert_eq!(
        rows.len(),
        PAGE_LIMIT as usize + 1,
        "one duplicate collapsed"
    );
    assert_eq!(
        rows[0].title, "peer-49-fresh",
        "refreshed row is still first"
    );
    assert_eq!(rows[0].mid, 149);
    assert_eq!(rows.last().unwrap().title, "peer-99");
    let mut ids: Vec<_> = rows.iter().map(|row| row.id).collect();
    ids.sort_by_key(|id| format!("{id:?}"));
    ids.dedup();
    assert_eq!(ids.len(), rows.len());
    assert!(!machine.state().has_more);
    assert!(!machine.state().loading);
}

#[test]
fn a_connection_error_keeps_the_loaded_list_and_does_not_quit() {
    let mut machine = one_row_list();

    machine.handle(Event::ConversationsFailed(
        "无法连接服务器：tcp connect error".to_string(),
    ));

    assert_eq!(machine.state().screen, Screen::Conversations);
    assert_eq!(
        machine.state().status.as_deref(),
        Some("无法连接服务器：tcp connect error")
    );
    assert_eq!(machine.state().conversations.len(), 1);
    assert!(!machine.state().quit);
    assert!(!machine.state().loading);
}

#[test]
fn debug_output_never_exposes_the_password() {
    let machine = logged_in();

    let debug = format!("{:?}", machine.state());

    assert!(
        !debug.contains("s3cret"),
        "Debug output leaked the password: {debug}"
    );
}

#[test]
fn the_token_is_replaced_proactively_at_fifty_minutes() {
    let mut machine = logged_in();

    assert!(
        machine
            .handle(Event::Tick(RELOGIN_AFTER - Duration::from_secs(1)))
            .is_empty()
    );

    let effects = machine.handle(Event::Tick(Duration::from_secs(1)));
    assert_eq!(
        effects,
        vec![Effect::Relogin {
            name: "alice".to_string(),
            password: "s3cret".to_string(),
        }],
        "the password stays in memory so re-login needs no prompt"
    );

    assert!(
        machine
            .handle(Event::Tick(Duration::from_secs(10)))
            .is_empty(),
        "one re-login is in flight; do not queue another"
    );

    machine.handle(Event::ReloginSucceeded {
        token: token_for(2),
    });
    assert_eq!(machine.state().token(), Some(token_for(2).as_str()));
    assert!(machine.state().status.is_none());

    assert!(
        machine
            .handle(Event::Tick(RELOGIN_AFTER - Duration::from_secs(1)))
            .is_empty()
    );
    assert_eq!(machine.handle(Event::Tick(Duration::from_secs(1))).len(), 1);
}

#[test]
fn a_failed_relogin_sends_the_user_back_to_the_login_page() {
    let mut machine = logged_in();
    machine.handle(Event::Tick(RELOGIN_AFTER));

    machine.handle(Event::ReloginFailed("无效的Token".to_string()));

    let state = machine.state();
    assert_eq!(state.screen, Screen::Login);
    assert!(state.token().is_none(), "the local token is cleared");
    assert!(!state.is_logged_in());
    assert!(state.conversations.is_empty(), "private rows are dropped");
    assert_eq!(state.form.name, "alice", "name is kept for convenience");
    assert!(
        state.form.password.is_empty(),
        "password must be re-entered"
    );
    assert!(
        state
            .status
            .as_deref()
            .is_some_and(|s| s.contains("自动续登失败") && s.contains("无效的Token")),
        "status was {:?}",
        state.status
    );
}

#[test]
fn a_rejected_token_sends_the_user_back_to_the_login_page() {
    let mut machine = one_row_list();

    machine.handle(Event::TokenExpired);

    let state = machine.state();
    assert_eq!(state.screen, Screen::Login);
    assert!(state.token().is_none());
    assert!(state.conversations.is_empty());
    assert_eq!(state.form.name, "alice");
    assert_eq!(state.status.as_deref(), Some("登录已过期，请重新登录"));
}

#[test]
fn stale_conversation_results_after_logout_are_ignored() {
    let mut machine = one_row_list();
    machine.handle(Event::TokenExpired);

    machine.handle(Event::ConversationsLoaded {
        page: 1,
        items: vec![conversation(
            7,
            "bob",
            11,
            "2026-11-11 10:00:00",
            Unread::Count(1),
        )],
    });
    machine.handle(Event::ConversationsFailed("later failure".to_string()));

    assert!(
        machine.state().conversations.is_empty(),
        "a response that arrives after the session ended must not refill the list"
    );
    assert_eq!(
        machine.state().status.as_deref(),
        Some("登录已过期，请重新登录")
    );
}

#[test]
fn quitting_after_login_asks_for_a_server_logout_and_clears_the_token() {
    let mut machine = logged_in();

    // `q` is message text now that the message box owns printable keys; Esc is
    // the quit key on the conversations screen.
    let effects = machine.handle(Event::Key(Key::Esc));

    assert_eq!(
        effects,
        vec![
            Effect::Logout {
                token: token_for(1)
            },
            Effect::Quit
        ]
    );
    assert!(machine.state().quit);
    assert!(machine.state().token().is_none());
}

#[test]
fn quitting_from_the_login_screen_has_nothing_to_log_out() {
    let mut machine = Machine::new();

    let effects = machine.handle(Event::Key(Key::Quit));

    assert_eq!(effects, vec![Effect::Quit]);
    assert!(machine.state().quit);
}

#[test]
fn resizing_degrades_the_layout_without_losing_state() {
    let mut machine = one_row_list();

    machine.handle(Event::Resize {
        width: 20,
        height: 5,
    });
    assert_eq!(machine.state().layout_mode(), LayoutMode::Minimal);
    assert_eq!(machine.state().screen, Screen::Conversations);
    assert_eq!(machine.state().conversations.len(), 1);
    assert_eq!(machine.state().token(), Some(token_for(1).as_str()));

    machine.handle(Event::Resize {
        width: 60,
        height: 20,
    });
    assert_eq!(machine.state().layout_mode(), LayoutMode::Compact);

    machine.handle(Event::Resize {
        width: 120,
        height: 40,
    });
    assert_eq!(machine.state().layout_mode(), LayoutMode::Full);

    machine.handle(Event::Resize {
        width: 0,
        height: 0,
    });
    assert_eq!(machine.state().layout_mode(), LayoutMode::Minimal);
    assert!(!machine.state().quit);
}

#[test]
fn a_token_without_a_uid_fails_the_login_instead_of_inverting_authorship() {
    let mut machine = Machine::new();
    fill_credentials(&mut machine, "alice", "s3cret");
    machine.handle(Event::Key(Key::Enter));

    let effects = machine.handle(Event::LoginSucceeded {
        token: "not-a-jwt".to_string(),
    });

    assert!(
        effects.is_empty(),
        "no list request may go out for a session we cannot attribute"
    );
    let state = machine.state();
    assert_eq!(state.screen, Screen::Login, "the user stays on login");
    assert!(!state.is_logged_in(), "no session is created");
    assert_eq!(state.user_uid(), None);
    assert!(!state.form.busy, "the form can be retried");
    assert!(
        state
            .status
            .as_deref()
            .is_some_and(|status| status.contains("登录令牌")),
        "the status line says why the login was refused: {:?}",
        state.status
    );
}

#[test]
fn a_relogin_token_without_a_uid_ends_the_session_instead_of_guessing() {
    let mut machine = logged_in();
    assert_eq!(
        machine.handle(Event::Tick(RELOGIN_AFTER)),
        vec![Effect::Relogin {
            name: "alice".to_string(),
            password: "s3cret".to_string(),
        }]
    );

    machine.handle(Event::ReloginSucceeded {
        token: "header.eyJpZCI6.bad".to_string(),
    });

    let state = machine.state();
    assert_eq!(state.screen, Screen::Login);
    assert!(state.token().is_none(), "the unreadable token is dropped");
    assert!(!state.is_logged_in());
    assert!(
        state
            .status
            .as_deref()
            .is_some_and(|status| status.contains("自动续登失败")),
        "the status line reports the failed renewal: {:?}",
        state.status
    );
    assert!(state.form.password.is_empty(), "the password is re-entered");
}
