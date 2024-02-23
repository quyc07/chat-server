use std::convert::Infallible;
use std::path::PathBuf;
use std::time::Duration;

use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::Event;
use axum::Router;
use axum::routing::get;
use axum_extra::{headers, TypedHeader};
use futures::{SinkExt, Stream};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;
use tower_http::services::ServeDir;

use crate::app_state::AppState;

pub struct EventApi;

impl EventApi {
    pub async fn route(app_state: AppState) -> Router {
        let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let static_files_service = ServeDir::new(assets_dir).append_index_html_on_directories(true);
        // build our application with a route
        Router::new()
            .fallback_service(static_files_service)
            .route("/sse", get(sse_handler))
            .with_state(app_state)
    }
}

async fn sse_handler(
    State(app_state): State<AppState>,
    TypedHeader(user_agent): TypedHeader<headers::UserAgent>,
) -> Sse<impl Stream<Item=Result<Event, Infallible>>> {
    println!("`{}` connected", user_agent.as_str());

    // A `Stream` that repeats an event every second
    //
    // You can also create streams from tokio channels using the wrappers in
    // https://docs.rs/tokio-stream
    // let stream = stream::repeat_with(|| Event::default().data("hi!"))
    //     .map(Ok)
    //     .throttle(Duration::from_secs(1));
    let (tx_msg, rx_msg) = mpsc::unbounded_channel();
    tokio::spawn(event_loop(app_state, tx_msg));
    let receiver_stream = tokio_stream::wrappers::UnboundedReceiverStream::from(rx_msg);
    Sse::new(receiver_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(5))
            .text("keep-alive-text"),
    )
}

async fn event_loop(_app_state: AppState, mut tx_msg: UnboundedSender<Result<Event, Infallible>>) {
    loop {
        sleep(Duration::from_secs(1)).await;
        let event = MessageEvent { msg: "Hello World!".to_string() };
        let result = Event::default().json_data(event).expect("fail to transfer event to json");
        tx_msg.send(Ok(result)).expect("send failed");
    }
}

#[derive(Serialize)]
struct MessageEvent {
    msg: String,
}
