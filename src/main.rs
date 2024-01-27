use std::fmt::Debug;

use axum::Router;
use axum::routing::get;
use tokio::net::TcpListener;
use tracing::info;

use chat_server::app_state::AppState;
use chat_server::log;
use chat_server::user::UserApi;

#[tokio::main]
async fn main() {
    log::log_init_multi().await;
    color_eyre::install().unwrap();
    info!("chat server start begin!");
    let app_state = AppState::new().await.unwrap();
    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .nest("/user", UserApi::route(app_state).await)
        ;

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("chat server started!");
    axum::serve(listener, app).await.unwrap();
}


