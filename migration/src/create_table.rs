use futures::StreamExt;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        // 读取 2024-09-17.sql 文件内容
        let sqls = vec![
            include_str!("./2024-09-17.sql"),
            include_str!("./2025-03-20.sql"),
        ];
        futures::stream::iter(sqls)
            .for_each(|sql| async {
                db.execute_unprepared(sql)
                    .await
                    .expect("fail to execute sql");
            })
            .await;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Replace the sample below with your own migration scripts
        todo!();
    }
}
