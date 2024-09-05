//! `SeaORM` Entity, @generated by sea-orm-codegen 1.0.0

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "status")]
pub enum UserStatus {
    #[sea_orm(string_value = "NORMAL")]
    Normal,
    #[sea_orm(string_value = "FREEZE")]
    Freeze,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "status")]
pub enum FriendRequestStatus {
    #[sea_orm(string_value = "WAIT")]
    WAIT,
    #[sea_orm(string_value = "APPROVE")]
    APPROVE,
    #[sea_orm(string_value = "REJECT")]
    REJECT,
}

impl From<UserStatus> for String {
    fn from(value: UserStatus) -> Self {
        match value {
            UserStatus::Normal => "Normal",
            UserStatus::Freeze => "Freeze",
        }
        .to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "role")]
pub enum Role {
    #[sea_orm(string_value = "User")]
    User,
    #[sea_orm(string_value = "Admin")]
    Admin,
}
