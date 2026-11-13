//! Wire tests for `PUT /user/{peer}/send`.
//!
//! The response body is the server's `Res<String>` (`src/user.rs::send`), and
//! axum renders a `String` response as plain text: the new message id arrives as
//! `42`, never as JSON `"42"` or as a JSON number. The 404 is the server's
//! `FriendErr::NotFriend` text, and a 401 is the same read-body classification as
//! every other authenticated call.

use chat_tui::api::{ApiClient, ApiError};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn send_puts_the_message_and_decodes_the_bare_text_id() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/user/2/send"))
        .and(header("authorization", "Bearer tok-1"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({ "msg": "你好，世界" })))
        // `Res<String>` rendered by axum: the id is plain text, not JSON.
        .respond_with(ResponseTemplate::new(200).set_body_raw("42", "text/plain; charset=utf-8"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));

    let mid = api
        .send_message(2, "你好，世界")
        .await
        .expect("send succeeds");

    assert_eq!(mid, 42, "the bare text body holds the numeric id");
    server.verify().await;
}

#[tokio::test]
async fn a_quoted_json_id_is_tolerated_even_though_this_server_never_sends_it() {
    let server = MockServer::start().await;
    // Defensive tolerance only: `Res<String>` is plain text on this server, so a
    // quoted body is not a fixture of real server output. It is accepted because
    // it carries the same id and a proxy should not turn a valid send into a
    // decode failure.
    Mock::given(method("PUT"))
        .and(path("/user/2/send"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#""43""#, "application/json"))
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
            // `ServerError::AuthErr(InvalidToken)` renders as `(StatusCode, String)`,
            // which axum sends as plain text.
            ResponseTemplate::new(401).set_body_raw("无效的Token", "text/plain; charset=utf-8"),
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
