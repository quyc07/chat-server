use tokio::net::TcpListener;
use tracing::info;

use chat_server::app_state::AppState;
use chat_server::log;
use migration::{Migrator, MigratorTrait};

#[tokio::main]
async fn main() {
    log::log_init_multi().await;
    color_eyre::install().unwrap();
    info!("chat server start begin!");
    let app_state = AppState::new("data").await.unwrap();
    // 数据初始化
    Migrator::up(&app_state.db, None)
        .await
        .expect("fail to apply migrations");
    let app = chat_server::router(app_state).await;

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("chat server started!");
    axum::serve(listener, app).await.unwrap();
}
