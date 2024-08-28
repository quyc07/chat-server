use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "
create table read_index
(
    id         bigint auto_increment,
    uid        int not null,
    target_uid int null,
    target_gid int null,
    mid        int not null,
    constraint read_index_pk
        primary key (id),
    constraint read_index_group_id_fk
        foreign key (target_gid) references `group` (id)
            on delete cascade,
    constraint read_index_user_id_fk
        foreign key (target_uid) references user (id)
            on delete cascade,
    constraint read_index_user_id_fk_2
        foreign key (uid) references user (id)
            on delete cascade
)
    comment '消息读取进度';

create unique index read_index_uid_target_gid_uindex
    on read_index (uid, target_gid);

create unique index read_index_uid_target_uid_uindex
    on read_index (uid, target_uid);
        ",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Replace the sample below with your own migration scripts
        todo!();
    }
}
