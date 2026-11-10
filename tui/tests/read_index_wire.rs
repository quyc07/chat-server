//! Wire tests for read-position reporting (`PUT /ri`).
//!
//! Two kinds of assertions, like the other `*_wire.rs` files:
//! 1. request shape: method, path, auth header, and the externally tagged JSON
//!    body, copied from the server's `UpdateReadIndex` (`src/read_index.rs`);
//! 2. classification of the failures the state machine turns into status-line
//!    text.
//!
//! The success response is the shape the real handler produces: `Res<()>` is
//! `Result<(), ServerError>`, and `()` renders as `200` with an empty body.

use chat_tui::api::{ApiClient, ApiError};
use chat_tui::model::{ConversationId, ReadPosition};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn a_direct_conversation_is_reported_with_the_peer_and_the_mid() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/ri"))
        .and(header("authorization", "Bearer tok-1"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "User": { "target_uid": 2, "mid": 42 },
        })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    api.report_read(ReadPosition {
        conversation: ConversationId::User(2),
        mid: 42,
    })
    .await
    .expect("the server accepted the read position");

    server.verify().await;
}

#[tokio::test]
async fn a_group_conversation_is_reported_with_the_group_id_and_the_mid() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/ri"))
        .and(header("authorization", "Bearer tok-1"))
        .and(body_json(json!({
            "Group": { "target_gid": 8, "mid": 42 },
        })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    api.report_read(ReadPosition {
        conversation: ConversationId::Group(8),
        mid: 42,
    })
    .await
    .expect("the server accepted the read position");

    server.verify().await;
}

#[tokio::test]
async fn a_rejected_report_is_classified_for_the_status_line() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/ri"))
        .respond_with(
            ResponseTemplate::new(500)
                // `(StatusCode, ERROR_MESSAGE).into_response()` in `src/err.rs`
                // is plain text.
                .set_body_raw("系统异常，请稍后再试", "text/plain; charset=utf-8"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let error = api
        .report_read(ReadPosition {
            conversation: ConversationId::User(2),
            mid: 42,
        })
        .await
        .expect_err("the server failed");

    assert_eq!(
        error,
        ApiError::Status {
            status: 500,
            message: "系统异常，请稍后再试".to_string(),
        }
    );
    assert_eq!(error.message(), "系统异常，请稍后再试");
}

#[tokio::test]
async fn a_401_on_the_report_is_an_expired_session() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/ri"))
        .respond_with(
            ResponseTemplate::new(401).set_body_raw(r#""无效的Token""#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("expired-token".to_string()));

    assert_eq!(
        api.report_read(ReadPosition {
            conversation: ConversationId::User(2),
            mid: 42,
        })
        .await
        .expect_err("the token is rejected"),
        ApiError::Unauthorized("无效的Token".to_string())
    );
}
