//! Wire tests for the push-link supervisor: the part that redials a dead stream.
//!
//! The stub server writes the same bytes as the real one, and every connection
//! it serves ends exactly the way the server's does when its broadcast receiver
//! falls behind: a keep-alive comment, then a clean end of response. No error
//! event is sent, so a clean end must itself be read as "disconnected".
//!
//! The redial wait is injected instead of slept through, and liveness timing is
//! covered separately with an injected clock (`link_liveness.rs`). Nothing here
//! waits on a real deadline.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chat_tui::api::ApiClient;
use chat_tui::link;
use chat_tui::state::Event;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// The server's SSE `data:` line: `src/event.rs` serialises its externally
/// tagged `Message` enum, so the message is wrapped in `{"ChatMessage": ...}`.
const CHAT_DATA: &str = r#"{"ChatMessage":{"mid":42,"payload":{"from_uid":2,"created_at":"2026-11-11 09:00:00","target":{"User":{"uid":1}},"detail":{"Normal":{"content":{"content":"早上好"}}}}}}"#;

fn stream_body(extra: &str) -> String {
    format!(": keep-alive-text\n\n{extra}")
}

/// Collect `count` events, failing if the supervisor goes quiet.
async fn take_events(rx: &mut mpsc::UnboundedReceiver<Event>, count: usize) -> Vec<Event> {
    let mut events = Vec::new();
    while events.len() < count {
        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("the supervisor must keep producing link events")
            .expect("the event channel stays open");
        events.push(event);
    }
    events
}

fn is_up(event: &Event) -> bool {
    matches!(event, Event::LinkUp)
}

fn is_down(event: &Event) -> bool {
    matches!(event, Event::LinkDown)
}

#[tokio::test]
async fn a_cleanly_closed_stream_is_redialed_and_reports_the_down_up_cycle() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let served = calls.clone();
    Mock::given(method("GET"))
        .and(path("/event/stream"))
        .and(query_param("token", "tok-1"))
        .respond_with(move |_request: &Request| {
            let call = served.fetch_add(1, Ordering::SeqCst);
            let extra = if call == 0 {
                // First connection: keep-alive then a clean end, the server's
                // silent death. Nothing is delivered on it.
                String::new()
            } else {
                // The redialed connection carries a real message.
                format!("event: Chat\ndata: {CHAT_DATA}\n\n")
            };
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream_body(&extra))
        })
        .mount(&server)
        .await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (token_tx, token_rx) = watch::channel(Some("tok-1".to_string()));
    let api = ApiClient::new(server.uri());
    // The redial delay is injected: zero here, one second in the real client.
    link::spawn_with_delay(tx, api, token_rx, Duration::ZERO);

    let events = take_events(&mut rx, 4).await;

    assert!(is_up(&events[0]), "the first connect reports the link up");
    assert!(
        is_down(&events[1]),
        "a clean end of response must count as a disconnect, got {:?}",
        events[1]
    );
    assert!(
        is_up(&events[2]),
        "the client redialed on its own, got {:?}",
        events[2]
    );
    match &events[3] {
        Event::MessageReceived(message) => {
            assert_eq!(message.mid, 42);
            assert_eq!(message.body, "早上好");
        }
        other => panic!("the redialed stream must deliver messages, got {other:?}"),
    }

    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one redial, no user action in between"
    );

    // Stop the supervisor so the test does not leave it redialing.
    let _ = token_tx.send(None);
}

#[tokio::test]
async fn replacing_the_token_redials_immediately_instead_of_waiting_out_the_delay() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/event/stream"))
        .and(query_param("token", "old-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream_body("")),
        )
        .mount(&server)
        .await;
    let new_token_calls = Arc::new(AtomicUsize::new(0));
    let served = new_token_calls.clone();
    Mock::given(method("GET"))
        .and(path("/event/stream"))
        .and(query_param("token", "new-token"))
        .respond_with(move |_request: &Request| {
            served.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream_body("event: Heartbeat\ndata: {}\n\n"))
        })
        .mount(&server)
        .await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (token_tx, token_rx) = watch::channel(Some("old-token".to_string()));
    let api = ApiClient::new(server.uri());
    // A minute between redials: only a token change can explain a fast reconnection.
    link::spawn_with_delay(tx, api, token_rx, Duration::from_secs(60));

    // Up, then down from the clean end of the first body.
    let first = take_events(&mut rx, 2).await;
    assert!(is_up(&first[0]) && is_down(&first[1]));

    token_tx
        .send(Some("new-token".to_string()))
        .expect("token sink");

    let after = take_events(&mut rx, 1).await;
    assert!(
        is_up(&after[0]),
        "a new token replaces the stream without the user waiting, got {:?}",
        after[0]
    );
    assert_eq!(
        new_token_calls.load(Ordering::SeqCst),
        1,
        "the redial used the token the session now holds"
    );

    let _ = token_tx.send(None);
}
