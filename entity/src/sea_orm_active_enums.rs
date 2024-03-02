//! `SeaORM` Entity. Generated by sea-orm-codegen 0.12.11

use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "status")]
pub enum Status {
    #[sea_orm(string_value = "NORMAL")]
    Normal,
    #[sea_orm(string_value = "FREEZE")]
    Freeze,
}

impl From<Status> for String {
    fn from(value: Status) -> Self {
        match value {
            Status::Normal => "Normal",
            Status::Freeze => "Freeze",
        }.to_string()
    }
}