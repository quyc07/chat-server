//! Behavioural tests for friends, at the pure state-machine seam.
//!
//! Events in, state and effects out: no I/O, no terminal. These cover the cold
//! start path the server enforces — search a user, ask to be friends, accept or
//! refuse inbound asks, and see the contact list — plus the classification of
//! the server's 201/403 "not really a failure" replies.

use std::time::Duration;

use chat_tui::model::{
    self, FoundUser, Friend, FriendDecision, FriendRequest, FriendRequestStatus, ReviewOutcome,
    SendOutcome,
};
use chat_tui::state::{Effect, Event, FriendsFocus, Key, Machine, Notice, Screen};
use chrono::NaiveDateTime;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
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
    machine
}

fn opened_friends() -> Machine {
    let mut machine = logged_in();
    let effects = machine.handle(Event::Key(Key::Friends));
    assert_eq!(
        effects,
        vec![Effect::LoadFriendRequests, Effect::LoadFriends]
    );
    machine
}

fn pending_request(id: i32, request_id: i32, name: &str, time: &str) -> FriendRequest {
    FriendRequest {
        id,
        request_id,
        request_name: name.to_string(),
        create_time: at(time),
        reason: None,
        status: FriendRequestStatus::Wait,
    }
}

fn found(id: i32, name: &str) -> FoundUser {
    FoundUser {
        id,
        name: name.to_string(),
    }
}

#[test]
fn opening_friends_loads_requests_and_friends_and_focuses_search() {
    let mut machine = logged_in();

    let effects = machine.handle(Event::Key(Key::Friends));

    assert_eq!(
        effects,
        vec![Effect::LoadFriendRequests, Effect::LoadFriends]
    );
    assert_eq!(machine.state().screen, Screen::Friends);
    assert_eq!(machine.state().friends.focus, FriendsFocus::Search);
    assert!(machine.state().friends.loading_requests);
    assert!(machine.state().friends.loading_friends);
    assert!(machine.state().is_logged_in());
}

#[test]
fn escape_returns_to_conversations_without_logging_out() {
    let mut machine = opened_friends();

    assert!(machine.handle(Event::Key(Key::Esc)).is_empty());

    assert_eq!(machine.state().screen, Screen::Conversations);
    assert_eq!(machine.state().token(), Some(token_for(1).as_str()));
    assert!(!machine.state().quit);
}

#[test]
fn a_typed_q_in_the_friends_search_box_is_text_not_quit() {
    let mut machine = opened_friends();

    machine.handle(Event::Key(Key::Char('q')));

    assert!(!machine.state().quit);
    assert_eq!(machine.state().friends.query, "q");
}

#[test]
fn typing_a_name_and_pressing_enter_searches_for_the_trimmed_query() {
    let mut machine = opened_friends();
    type_str(&mut machine, "  ali  ");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::FindUsers {
            query: "ali".to_string()
        }]
    );
    assert_eq!(machine.state().friends.query, "ali");
    assert!(machine.state().friends.searching);
    assert!(machine.state().friends.notice.is_none());
}

#[test]
fn an_empty_search_is_rejected_locally_without_a_request() {
    let mut machine = opened_friends();
    type_str(&mut machine, "   ");

    let effects = machine.handle(Event::Key(Key::Enter));

    assert!(effects.is_empty());
    assert!(!machine.state().friends.searching);
    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info("请输入要搜索的用户名".to_string()))
    );
}

#[test]
fn search_results_replace_the_previous_page_and_ignore_a_stale_reply() {
    let mut machine = opened_friends();
    type_str(&mut machine, "ali");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::UsersFound {
        query: "ali".to_string(),
        users: vec![found(2, "alice"), found(5, "alice-bot")],
    });
    assert_eq!(machine.state().friends.results.len(), 2);
    assert!(!machine.state().friends.searching);

    // The user edited the query and searched again; the first reply is stale.
    machine.handle(Event::Key(Key::Backspace));
    type_str(&mut machine, "b");
    machine.handle(Event::Key(Key::Enter));
    machine.handle(Event::UsersFound {
        query: "ali".to_string(),
        users: vec![found(9, "old-result")],
    });
    assert_eq!(
        machine.state().friends.results.len(),
        2,
        "a reply for an older query must not replace the current results"
    );

    machine.handle(Event::UsersFound {
        query: "alb".to_string(),
        users: vec![found(3, "bob")],
    });
    assert_eq!(machine.state().friends.results, vec![found(3, "bob")]);
    assert_eq!(machine.state().friends.selected_result, 0);
}

#[test]
fn an_empty_search_result_is_announced_as_info_not_as_a_failure() {
    let mut machine = opened_friends();
    type_str(&mut machine, "nobody");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::UsersFound {
        query: "nobody".to_string(),
        users: vec![],
    });

    assert!(matches!(
        machine.state().friends.notice,
        Some(Notice::Info(ref text)) if text.contains("没有找到")
    ));
}

#[test]
fn a_search_transport_failure_is_an_error_notice() {
    let mut machine = opened_friends();
    type_str(&mut machine, "ali");
    machine.handle(Event::Key(Key::Enter));

    machine.handle(Event::UserSearchFailed(
        "无法连接服务器：tcp connect error".to_string(),
    ));

    assert!(!machine.state().friends.searching);
    assert!(matches!(
        machine.state().friends.notice,
        Some(Notice::Error(ref text)) if text.contains("无法连接服务器")
    ));
}

#[test]
fn tab_cycles_focus_through_search_results_requests_and_friends() {
    let mut machine = opened_friends();
    let seen: Vec<_> = (0..4)
        .map(|_| {
            let focus = machine.state().friends.focus;
            machine.handle(Event::Key(Key::Tab));
            focus
        })
        .collect();

    assert_eq!(
        seen,
        vec![
            FriendsFocus::Search,
            FriendsFocus::Results,
            FriendsFocus::Requests,
            FriendsFocus::Friends,
        ]
    );
    assert_eq!(machine.state().friends.focus, FriendsFocus::Search);
}

#[test]
fn enter_on_a_search_result_sends_a_friend_request_to_that_user() {
    let mut machine = opened_friends();
    type_str(&mut machine, "ali");
    machine.handle(Event::Key(Key::Enter));
    machine.handle(Event::UsersFound {
        query: "ali".to_string(),
        users: vec![found(2, "alice"), found(5, "alice-bot")],
    });

    machine.handle(Event::Key(Key::Tab));
    assert_eq!(machine.state().friends.focus, FriendsFocus::Results);
    machine.handle(Event::Key(Key::Down));
    let effects = machine.handle(Event::Key(Key::Enter));

    assert_eq!(
        effects,
        vec![Effect::SendFriendRequest {
            target: found(5, "alice-bot")
        }]
    );
}

#[test]
fn already_a_friend_is_a_readable_info_notice_and_refreshes_the_friend_list() {
    let mut machine = opened_friends();

    let effects = machine.handle(Event::FriendRequestSent {
        target: "alice-bot".to_string(),
        outcome: SendOutcome::AlreadyFriend,
    });

    assert_eq!(effects, vec![Effect::LoadFriends]);
    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info("alice-bot 已经是你的好友".to_string()))
    );
    assert_eq!(machine.state().screen, Screen::Friends);
    assert!(!machine.state().quit);
}

#[test]
fn a_request_already_waiting_is_its_own_readable_notice() {
    let mut machine = opened_friends();

    let effects = machine.handle(Event::FriendRequestSent {
        target: "carol".to_string(),
        outcome: SendOutcome::AlreadyWaiting,
    });

    assert!(effects.is_empty());
    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info(
            "已向 carol 发送过好友请求，正在等待对方处理".to_string()
        ))
    );
}

#[test]
fn a_sent_request_reports_success_as_info() {
    let mut machine = opened_friends();

    machine.handle(Event::FriendRequestSent {
        target: "bob".to_string(),
        outcome: SendOutcome::Sent,
    });

    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info("已向 bob 发送好友请求".to_string()))
    );
}

#[test]
fn a_forbidden_friend_request_is_shown_as_readable_text_not_a_failure() {
    let mut machine = opened_friends();

    machine.handle(Event::FriendRequestSent {
        target: "bob".to_string(),
        outcome: SendOutcome::Forbidden("用户已被冻结".to_string()),
    });

    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info("用户已被冻结".to_string()))
    );
}

#[test]
fn a_failed_send_is_an_error_notice_that_keeps_the_screen() {
    let mut machine = opened_friends();

    machine.handle(Event::FriendRequestSendFailed {
        target: "bob".to_string(),
        message: "系统异常，请稍后再试".to_string(),
    });

    assert_eq!(machine.state().screen, Screen::Friends);
    assert!(matches!(
        machine.state().friends.notice,
        Some(Notice::Error(ref text))
            if text.contains("bob") && text.contains("系统异常")
    ));
}

#[test]
fn only_waiting_requests_are_pending_newest_first() {
    let mut machine = opened_friends();

    machine.handle(Event::FriendRequestsLoaded(vec![
        pending_request(11, 3, "bob", "2026-11-11 09:00:00"),
        FriendRequest {
            status: FriendRequestStatus::Approve,
            ..pending_request(12, 4, "approved", "2026-11-11 11:00:00")
        },
        FriendRequest {
            status: FriendRequestStatus::Reject,
            ..pending_request(13, 5, "rejected", "2026-11-11 10:00:00")
        },
        pending_request(14, 6, "carol", "2026-11-11 12:00:00"),
    ]));

    let requests = &machine.state().friends.requests;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request_name, "carol");
    assert_eq!(requests[1].request_name, "bob");
    assert!(!machine.state().friends.loading_requests);
}

#[test]
fn approving_the_selected_request_emits_a_review_and_then_reloads_friends() {
    let mut machine = opened_friends();
    machine.handle(Event::FriendRequestsLoaded(vec![pending_request(
        11,
        3,
        "bob",
        "2026-11-11 09:00:00",
    )]));
    machine.handle(Event::Key(Key::Tab));
    machine.handle(Event::Key(Key::Tab));
    assert_eq!(machine.state().friends.focus, FriendsFocus::Requests);

    let effects = machine.handle(Event::Key(Key::Enter));
    let request = pending_request(11, 3, "bob", "2026-11-11 09:00:00");
    assert_eq!(
        effects,
        vec![Effect::ReviewFriendRequest {
            request,
            decision: FriendDecision::Approve,
        }]
    );

    let effects = machine.handle(Event::FriendRequestReviewed {
        request_id: 11,
        decision: FriendDecision::Approve,
        outcome: ReviewOutcome::Reviewed,
    });

    assert_eq!(effects, vec![Effect::LoadFriends]);
    assert!(machine.state().friends.requests.is_empty());
    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info("已通过 bob 的好友请求".to_string()))
    );
}

#[test]
fn r_rejects_the_selected_request_without_reloading_the_friend_list() {
    let mut machine = opened_friends();
    machine.handle(Event::FriendRequestsLoaded(vec![pending_request(
        11,
        3,
        "bob",
        "2026-11-11 09:00:00",
    )]));
    machine.handle(Event::Key(Key::Tab));
    machine.handle(Event::Key(Key::Tab));

    let effects = machine.handle(Event::Key(Key::Char('r')));
    assert_eq!(
        effects,
        vec![Effect::ReviewFriendRequest {
            request: pending_request(11, 3, "bob", "2026-11-11 09:00:00"),
            decision: FriendDecision::Reject,
        }]
    );

    let effects = machine.handle(Event::FriendRequestReviewed {
        request_id: 11,
        decision: FriendDecision::Reject,
        outcome: ReviewOutcome::Reviewed,
    });

    assert!(effects.is_empty());
    assert!(machine.state().friends.requests.is_empty());
    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info("已拒绝 bob 的好友请求".to_string()))
    );
}

#[test]
fn a_forbidden_review_is_informational_and_keeps_the_request_pending() {
    let mut machine = opened_friends();
    machine.handle(Event::FriendRequestsLoaded(vec![pending_request(
        11,
        3,
        "bob",
        "2026-11-11 09:00:00",
    )]));

    let effects = machine.handle(Event::FriendRequestReviewed {
        request_id: 11,
        decision: FriendDecision::Approve,
        outcome: ReviewOutcome::Forbidden("您不是该好友请求的目标对象，无权批准".to_string()),
    });

    assert!(effects.is_empty());
    assert_eq!(
        machine.state().friends.notice,
        Some(Notice::Info(
            "您不是该好友请求的目标对象，无权批准".to_string()
        ))
    );
    assert_eq!(
        machine.state().friends.requests.len(),
        1,
        "a refused review must not drop the pending request"
    );
}

#[test]
fn a_review_transport_failure_is_an_error_and_keeps_the_request() {
    let mut machine = opened_friends();
    machine.handle(Event::FriendRequestsLoaded(vec![pending_request(
        11,
        3,
        "bob",
        "2026-11-11 09:00:00",
    )]));

    machine.handle(Event::FriendRequestReviewFailed(
        "服务器返回 500".to_string(),
    ));

    assert!(matches!(
        machine.state().friends.notice,
        Some(Notice::Error(ref text)) if text.contains("处理好友请求失败")
    ));
    assert_eq!(machine.state().friends.requests.len(), 1);
}

#[test]
fn the_friend_list_is_replaced_and_keeps_a_valid_selection() {
    let mut machine = opened_friends();
    machine.handle(Event::FriendsLoaded(vec![
        Friend {
            id: 2,
            name: "alice".to_string(),
        },
        Friend {
            id: 3,
            name: "bob".to_string(),
        },
    ]));

    assert_eq!(machine.state().friends.friends.len(), 2);
    assert!(!machine.state().friends.loading_friends);

    machine.handle(Event::Key(Key::Tab));
    machine.handle(Event::Key(Key::Tab));
    machine.handle(Event::Key(Key::Tab));
    assert_eq!(machine.state().friends.focus, FriendsFocus::Friends);
    machine.handle(Event::Key(Key::Down));

    machine.handle(Event::FriendsLoaded(vec![Friend {
        id: 4,
        name: "carol".to_string(),
    }]));

    assert_eq!(machine.state().friends.friends.len(), 1);
    assert_eq!(machine.state().friends.selected_friend, 0, "clamped");
}

#[test]
fn a_friend_reply_after_the_token_expired_is_ignored() {
    let mut machine = opened_friends();

    machine.handle(Event::TokenExpired);
    machine.handle(Event::FriendsLoaded(vec![Friend {
        id: 2,
        name: "alice".to_string(),
    }]));
    machine.handle(Event::FriendRequestsLoaded(vec![pending_request(
        11,
        3,
        "bob",
        "2026-11-11 09:00:00",
    )]));

    let state = machine.state();
    assert_eq!(state.screen, Screen::Login);
    assert!(!state.is_logged_in());
    assert!(state.friends.friends.is_empty(), "private rows are dropped");
    assert!(state.friends.requests.is_empty());
}

#[test]
fn friend_events_only_move_state_on_the_friends_screen() {
    let mut machine = logged_in(); // conversations, not friends

    machine.handle(Event::FriendsLoaded(vec![Friend {
        id: 2,
        name: "alice".to_string(),
    }]));
    machine.handle(Event::FriendRequestsLoaded(vec![pending_request(
        11,
        3,
        "bob",
        "2026-11-11 09:00:00",
    )]));

    assert_eq!(machine.state().screen, Screen::Conversations);
    assert!(machine.state().friends.friends.is_empty());
    assert!(machine.state().friends.requests.is_empty());
}

#[test]
fn the_friends_screen_still_relogs_in_at_fifty_minutes() {
    let mut machine = opened_friends();

    let effects = machine.handle(Event::Tick(Duration::from_secs(50 * 60)));

    assert_eq!(
        effects,
        vec![Effect::Relogin {
            name: "alice".to_string(),
            password: "s3cret".to_string(),
        }],
        "the token clock keeps running on the friends screen"
    );
}
