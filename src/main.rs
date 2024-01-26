use axum::Router;
use axum::routing::get;
use color_eyre::eyre::eyre;
use tokio::net::TcpListener;
use tracing::instrument;
use tracing::{error, info};
use tracing_appender::rolling;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use chat_server::app_state::AppState;
use chat_server::user::UserApi;

#[tokio::main]
async fn main() {
    log_init_non_block().await;
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

async fn log_init() {
    // 输出到文件
    let debug_file = rolling::daily("./logs", "debug");
    let warn_file = rolling::daily("./logs", "warn");
    let all_logs = debug_file
        .and(warn_file.with_max_level(tracing::Level::WARN));
    tracing_subscriber::fmt()
        .with_writer(all_logs)
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .init();

    // // 输出到控制台
    // tracing_subscriber::fmt().pretty().with_writer(std::io::stdout).init();
    // // 输出到文件
    // let debug_file = rolling::daily("./logs", "debug");
    // let warn_file = rolling::daily("./logs", "warn");
    // let (non_blocking_debug, _guard) = tracing_appender::non_blocking(debug_file);
    // let (non_blocking_warn, _guard) = tracing_appender::non_blocking(warn_file);
    // let all_logs = non_blocking_debug.with_max_level(tracing::Level::TRACE).and(non_blocking_warn.with_max_level(tracing::Level::WARN));
    // tracing_subscriber::fmt()
    //     .with_writer(all_logs)
    //     .with_ansi(false)
    //     .init();
    // let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(tracing::Level::INFO.as_str()));
    // // Registry::default()
    // //     .with(env_filter)
    // //     .with(console_layer)
    // //     .init()
}

async fn log_init_non_block() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // 输出到控制台中
    let formatting_layer = tracing_subscriber::fmt::layer().pretty().with_writer(std::io::stderr);

    // 输出到文件中
    let file_appender = rolling::never("logs", "app.log");
    // let (non_blocking_appender, _guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(file_appender);

    // 注册
    tracing_subscriber::Registry::default()
        .with(env_filter)
        .with(formatting_layer)
        .with(file_layer)
        .init();
}
