use axum::extract::State;
use axum::Router;
use axum::routing::{get, post};
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use validator::Validate;

use crate::{AppRes, Res};
use crate::app_state::AppState;
use crate::entity::prelude::User;
use crate::entity::user;
use crate::err::ServerError;
use crate::validate::ValidatedJson;

pub struct UserApi;

impl UserApi {
    pub async fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/register", post(register))
            .route("/all", get(all))
            .with_state(app_state)
    }
}

#[derive(Debug, Deserialize, Serialize, Validate)]
struct UserRegisterReq {
    #[validate(required)]
    name: Option<String>,
    #[validate(length(min = 10))]
    email: String,
    password: String,
    phone: Option<String>,
}


#[derive(Debug, Error)]
pub enum UserErr {
    #[error("用户名 {0} 已存在")]
    UserNameExist(String),
}

impl Into<String> for UserErr {
    fn into(self) -> String {
        match self {
            UserErr::UserNameExist(_) => {
                AppRes::<()>::fail_with_msg(self.to_string()).into()
            }
        }
    }
}

async fn all(State(app_state): State<AppState>) -> Res<Vec<user::Model>> {
    let result = User::find().all(&app_state.db().await).await;
    let model = result.unwrap();
    Ok(AppRes::success(model))
}

async fn register(State(app_state): State<AppState>, ValidatedJson(req): ValidatedJson<UserRegisterReq>) -> Res<user::Model> {
    if req.name.is_some() {
        let name = req.name.as_ref().unwrap().as_str();
        let result = find_by_name(&app_state, name).await;
        if result.unwrap().is_some() {
            return Err(ServerError::from(UserErr::UserNameExist(name.to_string())));
        }
    }

    let user = user::ActiveModel {
        id: Default::default(),
        name: Set(req.name),
        password: Set(req.password),
        email: Set(req.email),
        phone: Set(req.phone),
        create_time: Default::default(),
        update_time: Default::default(),
    };
    let model = user.insert(&app_state.db().await).await?;
    Ok(AppRes::success(model))
}

async fn find_by_name(app_state: &AppState, name: &str) -> Result<Option<user::Model>, DbErr> {
    User::find().filter(user::Column::Name.eq(name)).one(&app_state.db().await).await
}



