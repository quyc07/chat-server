use tracing_appender::rolling;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

async fn log_file() {
    // 输出到文件
    let debug_file = rolling::daily("./logs", "debug");
    let warn_file = rolling::daily("./logs", "warn");
    let all_logs = debug_file.and(warn_file.with_max_level(tracing::Level::WARN));
    tracing_subscriber::fmt()
        .with_writer(all_logs)
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .init();
}

pub async fn log_init_multi() {
    let file_appender = rolling::hourly("logs", "info.log");
    // 不生效，不知道为什么
    // let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(tracing::Level::INFO.as_str())),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file_appender)
                .with_ansi(false)
                .pretty(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stdout)
                .with_line_number(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .pretty(),
        )
        .init()
}
