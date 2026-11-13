//! Wire tests for the HTTP seam.
//!
//! Two kinds of assertions:
//! 1. request shape: method, path, headers, and JSON body;
//! 2. real server JSON decodes into domain types. The fixtures below are copied
//!    from the server's own types (`src/user.rs::ChatVo`, `src/auth.rs::UserLoginRes`),
//!    including the externally tagged `{"User": ...}` / `{"Group": ...}` rows and
//!    the `%Y-%m-%d %H:%M:%S` timestamps.
//!
//! The history fixture was additionally checked against a running server:
//! `curl -X POST /user/history -d '{"page":1,"limit":50}'` with a Bearer token
//! returned exactly this shape (`unread` is `"all"`, a decimal string, or `null`).

use chat_tui::api::{ApiClient, ApiError};
use chat_tui::model::{self, ConversationId, Unread};
use chrono::NaiveDateTime;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A real `Vec<ChatVo>` body: one direct conversation never read (`"all"`), one
/// with three unread, and one group row (`unread: null`).
const HISTORY_JSON: &str = r#"[
  {"User":{"uid":2,"user_name":"alice","mid":42,"msg":"第一行\n第二行","msg_time":"2026-11-10 23:59:00","unread":"all"}},
  {"User":{"uid":3,"user_name":"bob","mid":41,"msg":"hi there","msg_time":"2026-11-10 22:00:00","unread":"3"}},
  {"Group":{"gid":7,"group_name":"群聊","uid":3,"user_name":"bob","mid":40,"msg":"group msg","msg_time":"2026-11-10 21:00:00","unread":null}}
]"#;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, model::WIRE_TIME_FORMAT).expect("test timestamp")
}

#[tokio::test]
async fn login_posts_credentials_and_decodes_the_server_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token/login"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "name": "alice",
            "password": "s3cret",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            // `UserLoginRes` from src/auth.rs.
            "access_token": "jwt-token-value",
            "access_token_expires": "2026-11-11 01:22:23",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    let token = api.login("alice", "s3cret").await.expect("login succeeds");

    assert_eq!(token, "jwt-token-value");
    assert_eq!(
        api.token(),
        Some("jwt-token-value"),
        "token is kept in memory"
    );
    server.verify().await;
}

#[tokio::test]
async fn login_failure_exposes_the_server_text_without_outer_json_quotes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token/login"))
        .respond_with(
            ResponseTemplate::new(401)
                // `ServerError::AuthErr` renders as `(StatusCode, err.to_string())`,
                // which axum sends as plain text, not a quoted JSON string.
                .set_body_raw("用户名或密码错误", "text/plain; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    let error = api
        .login("alice", "wrong")
        .await
        .expect_err("credentials are rejected");

    assert_eq!(
        error,
        ApiError::Unauthorized("用户名或密码错误".to_string())
    );
    assert_eq!(error.message(), "用户名或密码错误");
    assert!(!error.message().contains('"'), "quotes are stripped");
    assert_eq!(api.token(), None);
}

#[tokio::test]
async fn register_posts_the_new_account_and_decodes_the_assigned_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/register"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "name": "newbie",
            "password": "pw",
        })))
        .respond_with(
            // `register` returns `Res<String>`; axum renders `String` as a plain
            // text body, so the real server sends `7`, not JSON `"7"`.
            ResponseTemplate::new(200).set_body_raw("7", "text/plain; charset=utf-8"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    let id = api
        .register("newbie", "pw")
        .await
        .expect("register succeeds");

    assert_eq!(id, "7");
    assert_eq!(api.token(), None, "register does not log in server-side");
    server.verify().await;
}

#[tokio::test]
async fn history_sends_the_bearer_header_and_page_body_and_decodes_real_server_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/history"))
        .and(header("authorization", "Bearer tok-1"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({ "page": 1, "limit": 50 })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(HISTORY_JSON, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let conversations = api.history(1, 50).await.expect("history decodes");

    assert_eq!(conversations.len(), 3);

    let all_unread = &conversations[0];
    assert_eq!(all_unread.id, ConversationId::User(2));
    assert_eq!(all_unread.title, "alice");
    assert_eq!(all_unread.mid, 42);
    assert_eq!(all_unread.preview, "第一行", "excerpt is one line");
    assert_eq!(all_unread.time, at("2026-11-10 23:59:00"));
    assert_eq!(all_unread.unread, Unread::All);
    assert_eq!(all_unread.last_sender, None);

    let counted = &conversations[1];
    assert_eq!(counted.id, ConversationId::User(3));
    assert_eq!(counted.unread, Unread::Count(3));

    let group = &conversations[2];
    assert_eq!(group.id, ConversationId::Group(7));
    assert_eq!(group.title, "群聊");
    assert_eq!(group.unread, Unread::None);
    assert_eq!(group.last_sender.as_deref(), Some("bob"));

    server.verify().await;
}

#[tokio::test]
async fn logout_sends_delete_with_the_bearer_token_and_clears_it() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/token/logout"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    api.logout().await.expect("logout succeeds");

    assert_eq!(api.token(), None, "the local token is cleared");
    server.verify().await;
}

#[tokio::test]
async fn a_401_on_history_is_classified_so_the_wiring_can_reauthenticate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/history"))
        .respond_with(
            ResponseTemplate::new(401).set_body_raw(r#""无效的Token""#, "application/json"),
        )
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("expired-token".to_string()));

    assert_eq!(
        api.history(1, 50).await.expect_err("token is rejected"),
        ApiError::Unauthorized("无效的Token".to_string())
    );
}

#[tokio::test]
async fn a_generic_server_error_shows_the_server_own_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/history"))
        .respond_with(ResponseTemplate::new(500).set_body_raw("系统异常，请稍后再试", "text/plain"))
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok".to_string()));
    let error = api.history(1, 50).await.expect_err("server failed");

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
async fn an_unlistening_server_is_reported_as_unreachable() {
    // Reserve an ephemeral port, then free it so nothing is listening there.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("probe address").port();
    drop(probe);

    let mut api = ApiClient::new(format!("http://127.0.0.1:{port}"));
    let error = api
        .login("alice", "s3cret")
        .await
        .expect_err("nothing is listening");

    assert!(
        matches!(error, ApiError::Unreachable(_)),
        "expected Unreachable, got {error:?}"
    );
    assert!(
        error.message().starts_with("无法连接服务器："),
        "message was {:?}",
        error.message()
    );
}

#[test]
fn debug_output_never_exposes_the_token() {
    let mut api = ApiClient::new("http://example.test");
    api.set_token(Some("super-secret-token".to_string()));

    let debug = format!("{api:?}");

    assert!(
        !debug.contains("super-secret-token"),
        "Debug output leaked the token: {debug}"
    );
}

#[test]
fn base_url_trailing_slashes_do_not_produce_double_slashes() {
    let api = ApiClient::new("http://example.test/");
    assert_eq!(api.base_url(), "http://example.test");
}
