use std::collections::BTreeSet;
use std::convert::Infallible;
use std::path::PathBuf;
use std::time::Duration;

use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::Event;
use axum::Router;
use axum::routing::get;
use axum_extra::{headers, TypedHeader};
use chrono::{DateTime, Local};
use futures::Stream;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;
use tower_http::services::ServeDir;

use crate::app_state::AppState;
use crate::auth::Token;
use crate::user::ChatMessage;

pub struct EventApi;

impl EventApi {
    pub async fn route(app_state: AppState) -> Router {
        let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let static_files_service = ServeDir::new(assets_dir).append_index_html_on_directories(true);
        // build our application with a route
        Router::new()
            .fallback_service(static_files_service)
            .route("/stream", get(event_handler))
            .with_state(app_state)
    }
}

async fn event_handler(
    State(app_state): State<AppState>,
    token: Token,
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
    tokio::spawn(event_loop(app_state, tx_msg, token.id));
    let receiver_stream = tokio_stream::wrappers::UnboundedReceiverStream::from(rx_msg);
    Sse::new(receiver_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(5))
            .text("keep-alive-text"),
    )
}

async fn event_loop(app_state: AppState, tx_msg: UnboundedSender<Result<Event, Infallible>>, current_uid: i32) {
    let mut heartbeat = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(15),
        Duration::from_secs(15),
    );
    let mut receiver = app_state.event_sender.subscribe();
    loop {
        tokio::select! {
            res = receiver.recv() =>{
                match res {
                    Ok(event) => {
                        match &*event{
                            BroadcastEvent::Chat{ targets,message } => {
                                if !targets.contains(&current_uid){
                                    continue;
                                }
                                let event = Event::default().json_data(Message::ChatMessage(message.clone())).expect("fail to transfer event to json");
                                if tx_msg.send(Ok(event)).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            _ = heartbeat.tick() =>{
                let event = Event::default().json_data(Message::Heartbeat(HeartbeatMessage{time:Local::now()})).expect("fail to transfer event to json");
                if tx_msg.send(Ok(event)).is_err() {
                    break;
                }
            }

        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum Message {
    ChatMessage(ChatMessage),
    Heartbeat(HeartbeatMessage),
}

#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatMessage {
    time: DateTime<Local>,
}

#[derive(Debug, Clone)]
pub enum BroadcastEvent {
    /// Chat message
    Chat {
        targets: BTreeSet<i32>,
        message: ChatMessage,
    },
}