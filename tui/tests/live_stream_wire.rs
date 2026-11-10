//! Wire tests for the push stream and for the peer-name lookup.
//!
//! The stub server serves the exact bytes the real server writes (axum's SSE
//! encoder plus its keep-alive comment), and the test drives the same code path
//! the app uses: `open_stream` then `Response::chunk` then `sse::Parser`. That
//! is how the stream parsing is pinned without a live server.

use chat_tui::api::{ApiClient, ApiError};
use chat_tui::model::{self, ConversationId, FoundUser, StreamEvent};
use chat_tui::sse::{Frame, Parser};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `ChatMessage` from `src/message.rs`, wrapped in the server's SSE framing.
const CHAT_DATA: &str = r#"{"mid":42,"payload":{"from_uid":2,"created_at":"2026-11-11 09:00:00","target":{"User":{"uid":1}},"detail":{"Normal":{"content":{"content":"早上好"}}}}}"#;

fn stream_body() -> String {
    format!(
        ": keep-alive-text\n\n\
         event: Chat\ndata: {CHAT_DATA}\n\n\
         event: Heartbeat\ndata: {{\"time\":\"2026-11-11 09:00:05\"}}\n\n\
         : keep-alive-text\n\n"
    )
}

#[tokio::test]
async fn the_stream_is_opened_with_the_token_in_the_query_string_and_parses_live_frames() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/event/stream"))
        .and(query_param("token", "tok-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream_body()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let api = ApiClient::new(server.uri());
    let mut response = api.open_stream("tok-1").await.expect("stream opens");

    let mut parser = Parser::new();
    let mut frames: Vec<Frame> = Vec::new();
    while let Some(chunk) = response.chunk().await.expect("a readable chunk") {
        frames.extend(parser.push(&chunk).expect("valid event stream"));
    }

    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.event.as_str())
            .collect::<Vec<_>>(),
        vec!["Chat", "Heartbeat"],
        "keep-alive comments never become frames"
    );
    assert_eq!(frames[0].data, CHAT_DATA);

    let chat = match model::stream_event(&frames[0].event, &frames[0].data).expect("decodes") {
        Some(StreamEvent::Chat(chat)) => chat,
        _ => panic!("the Chat frame must decode"),
    };
    assert_eq!(chat.mid, 42);
    assert_eq!(chat.from_uid, 2);
    assert_eq!(chat.conversation(Some(1)), ConversationId::User(2));
    assert_eq!(chat.body, "早上好");

    assert_eq!(
        model::stream_event(&frames[1].event, &frames[1].data).expect("decodes"),
        Some(StreamEvent::Heartbeat)
    );
    server.verify().await;
}

#[tokio::test]
async fn a_rejected_stream_token_is_classified_as_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/event/stream"))
        .and(query_param("token", "expired"))
        .respond_with(
            ResponseTemplate::new(401).set_body_raw(r#""无效的Token""#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let api = ApiClient::new(server.uri());

    assert_eq!(
        api.open_stream("expired")
            .await
            .expect_err("token is rejected"),
        ApiError::Unauthorized("无效的Token".to_string())
    );
    server.verify().await;
}

#[tokio::test]
async fn a_non_success_stream_response_is_reported_with_its_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/event/stream"))
        .respond_with(ResponseTemplate::new(500).set_body_raw("boom", "text/plain"))
        .expect(1)
        .mount(&server)
        .await;

    let api = ApiClient::new(server.uri());

    assert_eq!(
        api.open_stream("tok-1").await.expect_err("stream fails"),
        ApiError::Status {
            status: 500,
            message: "boom".to_string(),
        }
    );
}

#[tokio::test]
async fn user_detail_returns_the_peer_name_with_the_bearer_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/detail/2"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            // `UserDetail` from src/user.rs; only id and name are used.
            r#"{"id":2,"name":"bob","email":null,"phone":null,"create_time":"2026-01-02 03:04:05","update_time":null,"status":"Normal","is_friend":true}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));

    assert_eq!(
        api.user_detail(2).await.expect("detail decodes"),
        FoundUser {
            id: 2,
            name: "bob".to_string(),
        }
    );
    server.verify().await;
}

#[tokio::test]
async fn a_401_on_user_detail_is_classified_so_the_wiring_can_reauthenticate() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/detail/2"))
        .respond_with(
            ResponseTemplate::new(401).set_body_raw(r#""无效的Token""#, "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("expired".to_string()));

    assert_eq!(
        api.user_detail(2).await.expect_err("token is rejected"),
        ApiError::Unauthorized("无效的Token".to_string())
    );
}
