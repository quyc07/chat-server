//! `SeaORM` Entity, @generated by sea-orm-codegen 1.0.0

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "read_index")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub uid: i32,
    pub target_uid: Option<i32>,
    pub target_gid: Option<i32>,
    pub mid: Option<i64>,
    pub latest_mid: i64,
    pub uid_of_latest_msg: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::group::Entity",
        from = "Column::TargetGid",
        to = "super::group::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Group,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::TargetUid",
        to = "super::user::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    User2,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::Uid",
        to = "super::user::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    User1,
}

impl Related<super::group::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Group.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
