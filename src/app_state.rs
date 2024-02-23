use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use sea_orm::{Database, DatabaseConnection};
use tokio::sync::broadcast;

use msg::MsgDb;

use crate::err::ServerError;
use crate::event::BroadcastEvent;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub msg_db: Arc<Mutex<MsgDb>>,
    pub event_sender: Arc<broadcast::Sender<Arc<BroadcastEvent>>>,
}

static ENVS: Lazy<HashMap<String, String>> = Lazy::new(|| {
    let string = fs::read_to_string(".env").unwrap();
    let env = string.lines();
    env.into_iter()
        .map(|line| line.split_once("=").map(|(k, v)| (k.to_string(), v.to_string())).unwrap())
        .collect()
});

impl AppState {
    pub async fn new() -> Result<AppState, ServerError> {
        let url = ENVS.get("DATABASE_URL")
            .ok_or(ServerError::CustomErr("fail to get database url from .env".to_string()))?;
        let db = Database::connect(url).await?;
        let msg_db = MsgDb::open(PathBuf::from("data/msgdb")).expect("fail to init msg db");
        let (sender, _) = broadcast::channel(128);
        Ok(AppState { db, msg_db: Arc::new(Mutex::new(msg_db)), event_sender: Arc::new(sender) })
    }
}