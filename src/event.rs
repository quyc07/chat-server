use std::convert::Infallible;
use std::path::PathBuf;
use std::time::Duration;
use axum::response::Sse;
use axum::response::sse::Event;
use axum::Router;
use axum::routing::{get};
use axum_extra::{headers, TypedHeader};
use futures::{Stream, stream};
use tokio_stream::StreamExt;
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
    TypedHeader(user_agent): TypedHeader<headers::UserAgent>,
) -> Sse<impl Stream<Item=Result<Event, Infallible>>> {
    println!("`{}` connected", user_agent.as_str());

    // A `Stream` that repeats an event every second
    //
    // You can also create streams from tokio channels using the wrappers in
    // https://docs.rs/tokio-stream
    let stream = stream::repeat_with(|| Event::default().data("hi!"))
        .map(Ok)
        .throttle(Duration::from_secs(1));

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(1))
            .text("keep-alive-text"),
    )
}