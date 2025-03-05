use config::Config;
use serde::Deserialize;
use std::env;

#[derive(Debug, Deserialize)]
struct Host {
    server: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Settings {
    debug: bool,
    host: Host,
}

impl Settings {
    pub fn new() -> Result<Self, config::ConfigError> {
        let run_mode = env::var("RUN_MODE").unwrap_or_else(|_| "dev".into());
        Config::builder()
            .add_source(config::File::with_name(&format!("config/{}", run_mode)).required(false))
            .add_source(config::Environment::with_prefix("APP"))
            .build()?
            .try_deserialize()
    }
}