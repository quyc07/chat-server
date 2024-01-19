use sea_orm::{Database, DatabaseConnection, DbErr};

#[derive(Clone)]
pub struct AppState {
    db: DatabaseConnection,
}

impl AppState {
    pub async fn new() -> Result<AppState, DbErr> {
        let db: DatabaseConnection = Database::connect("mysql://root:Aa123456@localhost/chat").await?;
        Ok(AppState { db })
    }

    pub async fn db(&self) -> DatabaseConnection {
        self.db.clone()
    }
}