use axum::extract::State;
use axum::Router;
use axum::routing::{get, post};
use chrono::{DateTime, Local};
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;
use validator::{Validate, ValidateArgs};

use entity::prelude::User;
use entity::sea_orm_active_enums::Status;
use entity::user;

use crate::{AppRes, auth, Res};
use crate::app_state::AppState;
use crate::auth::{AuthError, Token};
use crate::err::{ErrPrint, ServerError};
use crate::validate::ValidatedJson;

pub struct UserApi;

impl UserApi {
    pub async fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/register", post(register))
            .route("/all", get(all))
            .route("/login", post(login))
            .with_state(app_state)
    }
}

#[derive(Debug, Deserialize, Validate)]
struct UserRegisterReq {
    #[validate(length(min = 1))]
    name: String,
    #[validate(email)]
    email: String,
    #[validate(length(min = 1))]
    password: String,
    phone: Option<String>,
}


#[derive(Debug, Error)]
pub enum UserErr {
    #[error("用户名 {0} 已存在")]
    UserNameExist(String),
}

impl ErrPrint for UserErr {}

impl Into<String> for UserErr {
    fn into(self) -> String {
        AppRes::<()>::fail_with_msg(self.to_string()).into()
    }
}

async fn all(State(app_state): State<AppState>, _: Token) -> Res<Vec<user::Model>> {
    let result = User::find().all(&app_state.db().await).await;
    let model = result.unwrap();
    Ok(AppRes::success(model))
}

async fn register(State(app_state): State<AppState>, ValidatedJson(req): ValidatedJson<UserRegisterReq>) -> Res<user::Model> {
    let name = req.name.as_str();
    if find_by_name(&app_state, name).await?.is_some() {
        return Err(ServerError::from(UserErr::UserNameExist(name.to_string())));
    }

    let user = user::ActiveModel {
        id: Default::default(),
        name: Set(req.name),
        password: Set(req.password),
        email: Set(req.email),
        phone: Set(req.phone),
        create_time: Default::default(),
        update_time: Default::default(),
        status: ActiveValue::NotSet,
    };
    let model = user.insert(&app_state.db().await).await?;
    Ok(AppRes::success(model))
}

async fn login(State(app_state): State<AppState>, ValidatedJson(req): ValidatedJson<UserLoginReq>) -> Res<UserLoginRes> {
    let user = find_by_name(&app_state, &req.name).await.unwrap().unwrap();
    if user.password != req.password {
        return Err(ServerError::from(AuthError::WrongCredentials));
    }
    // Create the authorization token
    let token = Token::from(user);
    let access_token = auth::gen_token(token).await?;

    // Send the authorized token
    Ok(AppRes::success(UserLoginRes { access_token, access_token_expires: auth::expire().await }))
}

#[derive(Debug, Deserialize, Validate)]
struct UserLoginReq {
    #[validate(length(min = 1))]
    name: String,
    #[validate(length(min = 1))]
    password: String,
}

#[derive(Debug, Serialize)]
struct UserLoginRes {
    access_token: String,
    access_token_expires: DateTime<Local>,
}


async fn find_by_name(app_state: &AppState, name: &str) -> Result<Option<user::Model>, DbErr> {
    User::find().filter(user::Column::Name.eq(name)).one(&app_state.db().await).await
}



