use crate::datetime::datetime_format;
use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::app_state::AppState;
use crate::auth::Token;
use crate::message::ChatMessage;
use crate::{Api, middleware};
use axum::Router;
use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::Event;
use axum::routing::get;
use axum_extra::{TypedHeader, headers};
use chrono::{DateTime, Local};
use futures::Stream;
use serde::Serialize;
use tokio::sync::broadcast::Receiver;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;
use tower_http::services::ServeDir;

pub struct EventApi;

impl Api for EventApi {
    fn route(app_state: AppState) -> Router {
        let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let static_files_service = ServeDir::new(assets_dir).append_index_html_on_directories(true);
        // build our application with a route
        Router::new()
            .fallback_service(static_files_service)
            .route("/stream", get(event_handler))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_login,
            ))
            .with_state(app_state.clone())
    }
}

async fn event_handler(
    State(app_state): State<AppState>,
    token: Token, // sse无法通过header传递，需要通过query传递，需提供一个从query解析的QueryToken同该接口使用
    // 可选：User-Agent 不是 HTTP 必需头，缺失时不该拒绝连接。
    // 声明成必需提取器时，任何不自带 UA 的客户端（例如 reqwest 默认）都会收到 400，
    // 于是永远接不上推送，只能在重新登录拉历史时才看到消息。
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let user_agent = user_agent
        .map(|user_agent| user_agent.0.as_str().to_owned())
        .unwrap_or_else(|| "<未提供 User-Agent>".to_owned());
    println!("`{user_agent}` connected");

    // You can also create streams from tokio channels using the wrappers in
    // https://docs.rs/tokio-stream
    let (tx_msg, rx_msg) = mpsc::unbounded_channel();
    tokio::spawn(event_loop(
        tx_msg,
        token.id,
        app_state.event_sender.subscribe(),
    )); // 临时使用1
    let receiver_stream = tokio_stream::wrappers::UnboundedReceiverStream::from(rx_msg);
    Sse::new(receiver_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(5))
            .text("keep-alive-text"),
    )
}

async fn event_loop(
    tx_msg: UnboundedSender<Result<Event, Infallible>>,
    current_uid: i32,
    mut receiver: Receiver<Arc<BroadcastEvent>>,
) {
    let mut heartbeat = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(5),
        Duration::from_secs(60),
    );
    loop {
        tokio::select! {
            res = receiver.recv() =>{
                match res {
                    Ok(event) => {
                        match &*event{
                            BroadcastEvent::Chat{ targets,message } => {
                                if !targets.contains(&current_uid) && message.payload.from_uid != current_uid{
                                    continue;
                                }
                                let chat = Message::ChatMessage(message.clone());
                                let event = Event::default().event(chat.to_string()).json_data(chat).expect("fail to transfer event to json");
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
                let heartbeat = Message::Heartbeat(HeartbeatMessage{time:Local::now()});
                let event = Event::default().event(heartbeat.to_string()).json_data(heartbeat).expect("fail to transfer event to json");
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

// 也可以使用strum库来实现
impl Display for Message {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Message::ChatMessage(_) => "Chat",
                Message::Heartbeat(_) => "Heartbeat",
            }
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatMessage {
    #[serde(with = "datetime_format")]
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
