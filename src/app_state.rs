use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

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

static ENVS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let string = fs::read_to_string(".env").unwrap();
    let env = string.lines();
    env.into_iter()
        .map(|line| {
            line.split_once("=")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .unwrap()
        })
        .collect()
});

impl AppState {
    /// Open the application state using `data_dir` as the root for both the
    /// SQLite database and the sled message store.
    pub async fn new(data_dir: impl AsRef<Path>) -> Result<AppState, ServerError> {
        let data_dir = data_dir.as_ref();
        let db_dir = data_dir.join("db");
        if !db_dir.exists() {
            fs::create_dir_all(&db_dir)?;
        }
        let msg_db = MsgDb::open(data_dir.join("msgdb"))?;
        let db = Database::connect(format!(
            "sqlite://{}/chat.sqlite?mode=rwc",
            db_dir.display()
        ))
        .await?;

        let (sender, _) = broadcast::channel(128);
        Ok(AppState {
            db,
            msg_db: Arc::new(Mutex::new(msg_db)),
            event_sender: Arc::new(sender),
        })
    }
}
