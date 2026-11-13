//! Wire tests for the direct-message history endpoint.
//!
//! The fixture is copied from the server's own `UserHistoryMsg` in
//! `src/user.rs`: a flat array of `{mid, msg, time, from_uid, from_name}`, with
//! `%Y-%m-%d %H:%M:%S` local timestamps. This is a different shape from the
//! conversation list's `{"User": ...}` rows.
//!
//! The 404 body is the server's `FriendErr::NotFriend` text, rendered by
//! `ServerError::into_response` as a bare string (`用户{uid}不是您的好友`).

use chat_tui::api::{ApiClient, ApiError};
use chat_tui::model;
use chrono::NaiveDateTime;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
}

/// A real `Vec<UserHistoryMsg>` body, oldest first, including CJK and embedded
/// newlines (the server stores the raw message text).
const MESSAGES_JSON: &str = r#"[
  {"mid":41,"msg":"早上好","time":"2026-11-10 09:00:00","from_uid":2,"from_name":"bob"},
  {"mid":42,"msg":"第一行\n第二行：这是很长的一条中文消息，用来验证换行不会截断。","time":"2026-11-10 09:01:00","from_uid":1,"from_name":"alice"}
]"#;

#[tokio::test]
async fn messages_gets_the_peer_history_with_the_bearer_header_and_decodes_flat_rows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/2/history"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(MESSAGES_JSON, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let messages = api.messages(2).await.expect("history decodes");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].mid, 41);
    assert_eq!(messages[0].from_uid, 2);
    assert_eq!(messages[0].from_name, "bob");
    assert_eq!(messages[0].body, "早上好");
    assert_eq!(messages[0].time, at("2026-11-10 09:00:00"));

    assert_eq!(messages[1].mid, 42);
    assert_eq!(messages[1].from_uid, 1);
    assert_eq!(
        messages[1].body, "第一行\n第二行：这是很长的一条中文消息，用来验证换行不会截断。",
        "the flat `msg` field is the whole body, newlines included"
    );
    assert_eq!(messages[1].time, at("2026-11-10 09:01:00"));
    server.verify().await;
}

#[tokio::test]
async fn a_non_friend_404_is_reported_with_the_server_own_reason() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/2/history"))
        .respond_with(
            // `(StatusCode::NOT_FOUND, err.to_string())`: a bare text body.
            ResponseTemplate::new(404).set_body_raw("用户2不是您的好友", "text/plain"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let error = api.messages(2).await.expect_err("not a friend");

    assert_eq!(
        error,
        ApiError::Status {
            status: 404,
            message: "用户2不是您的好友".to_string(),
        }
    );
    assert_eq!(error.message(), "用户2不是您的好友");
}

#[tokio::test]
async fn a_401_on_message_history_is_classified_so_the_wiring_can_reauthenticate() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/2/history"))
        .respond_with(
            ResponseTemplate::new(401).set_body_raw(r#""无效的Token""#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("expired-token".to_string()));

    assert_eq!(
        api.messages(2).await.expect_err("token is rejected"),
        ApiError::Unauthorized("无效的Token".to_string())
    );
}

#[tokio::test]
async fn a_max_length_history_decodes_every_row() {
    let server = MockServer::start().await;
    // The endpoint returns at most 1000 rows and takes no pagination parameters.
    let rows: Vec<String> = (1..=1000)
        .map(|i| {
            format!(
                r#"{{"mid":{i},"msg":"消息 {i}","time":"2026-11-01 00:00:00","from_uid":2,"from_name":"bob"}}"#
            )
        })
        .collect();
    let body = format!("[{}]", rows.join(","));
    Mock::given(method("GET"))
        .and(path("/user/2/history"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let messages = api.messages(2).await.expect("1000 rows decode");

    assert_eq!(messages.len(), 1000);
    assert_eq!(messages.last().map(|message| message.mid), Some(1000));
    server.verify().await;
}
