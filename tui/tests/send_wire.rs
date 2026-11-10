//! Wire tests for `PUT /user/{peer}/send`.
//!
//! The response body is the server's `Res<String>`: the new message id quoted as
//! a JSON string (`"42"`), not a JSON number. The 404 is the server's
//! `FriendErr::NotFriend` text, and a 401 is the same read-body classification as
//! every other authenticated call.

use chat_tui::api::{ApiClient, ApiError};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn send_puts_the_message_and_decodes_the_quoted_json_id() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/user/2/send"))
        .and(header("authorization", "Bearer tok-1"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({ "msg": "你好，世界" })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#""42""#, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));

    let mid = api
        .send_message(2, "你好，世界")
        .await
        .expect("send succeeds");

    assert_eq!(mid, 42, "the quoted string holds the numeric id");
    server.verify().await;
}

#[tokio::test]
async fn a_plain_numeric_body_also_decodes_to_the_same_id() {
    let server = MockServer::start().await;
    // Axum renders a bare `String` handler result as text; the id still travels
    // in the body, so the client accepts both shapes.
    Mock::given(method("PUT"))
        .and(path("/user/2/send"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("43", "text/plain"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));

    assert_eq!(api.send_message(2, "hi").await.expect("send succeeds"), 43);
    server.verify().await;
}

#[tokio::test]
async fn a_non_friend_404_keeps_the_server_reason_for_the_status_line() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/user/2/send"))
        .respond_with(ResponseTemplate::new(404).set_body_raw("用户2不是您的好友", "text/plain"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let error = api.send_message(2, "hi").await.expect_err("not a friend");

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
async fn a_401_on_send_is_classified_so_the_wiring_can_reauthenticate() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/user/2/send"))
        .respond_with(
            ResponseTemplate::new(401).set_body_raw(r#""无效的Token""#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("expired-token".to_string()));

    assert_eq!(
        api.send_message(2, "hi")
            .await
            .expect_err("token is rejected"),
        ApiError::Unauthorized("无效的Token".to_string())
    );
}

#[tokio::test]
async fn an_unparseable_send_body_is_a_decode_error_not_a_wrong_id() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/user/2/send"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("not-an-id", "text/plain"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));

    let error = api.send_message(2, "hi").await.expect_err("no id");
    assert!(
        matches!(error, ApiError::Decode(_)),
        "got {error:?}, expected a decode error"
    );
    assert!(error.message().contains("无法解析服务器响应"));
}
