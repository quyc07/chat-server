use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use sea_orm::{Database, DatabaseConnection, DbErr};
use msg::MsgDb;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub msg_db: Arc<Mutex<MsgDb>>,
}

impl AppState {
    pub async fn new() -> Result<AppState, DbErr> {
        let db: DatabaseConnection = Database::connect("mysql://root:Aa123456@localhost/chat").await?;
        let msg_db = MsgDb::open(PathBuf::from("data/msgdb")).expect("fail to init msg db");
        Ok(AppState { db, msg_db: Arc::new(Mutex::new(msg_db)) })
    }
}