//! `SeaORM` Entity, @generated by sea-orm-codegen 1.0.0

use super::sea_orm_active_enums::{Role, UserStatus};
use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize)]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub password: String,
    pub create_time: DateTime,
    pub update_time: Option<DateTime>,
    pub status: UserStatus,
    pub dgraph_uid: String,
    pub role: Role,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
