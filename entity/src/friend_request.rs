//! `SeaORM` Entity, @generated by sea-orm-codegen 1.0.0

use super::sea_orm_active_enums::FriendRequestStatus;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "friend_request")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub request_id: i32,
    pub target_id: i32,
    pub reason: Option<String>,
    pub status: FriendRequestStatus,
    pub create_time: DateTime,
    pub modify_time: Option<DateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
