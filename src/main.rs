use std::time::Duration;

use axum::routing::get;
use axum::Router;
use moka::future::Cache;
use tokio::net::TcpListener;
use tracing::{error, info};
use utoipa::OpenApi;
use utoipa_swagger_ui::{SwaggerUi, Url};

use chat_server::app_state::AppState;
use chat_server::auth::TokenApi;
use chat_server::event::EventApi;
use chat_server::friend::FriendApi;
use chat_server::group::GroupApi;
use chat_server::open_api::swagger_ui;
use chat_server::read_index::ReadIndexApi;
use chat_server::user::UserApi;
use chat_server::{log, Api};
use migration::{Migrator, MigratorTrait};

#[tokio::main]
async fn main() {
    log::log_init_multi().await;
    color_eyre::install().unwrap();
    info!("chat server start begin!");
    let app_state = AppState::new().await.unwrap();
    // 数据初始化
    // Migrator::up(&app_state.db, None)
    //     .await
    //     .expect("fail to apply migrations");
    let app = Router::new()
        .merge(swagger_ui().await)
        .route("/", get(|| async { "Hello, World!" }))
        .nest("/user", UserApi::route(app_state.clone()))
        .nest("/group", GroupApi::route(app_state.clone()))
        .nest("/token", TokenApi::route(app_state.clone()))
        .nest("/event", EventApi::route(app_state.clone()))
        .nest("/friend", FriendApi::route(app_state.clone()))
        .nest("/ri", ReadIndexApi::route(app_state.clone()));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("chat server started!");
    axum::serve(listener, app).await.unwrap();
}
