use axum::{Json, Router};
use axum::extract::State;
use axum::routing::{get, post};
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use validator::Validate;

use crate::app_state::AppState;
use crate::AppRes;
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


// The kinds of errors we can hit in our application.
#[derive(Debug, Error)]
pub enum UserErr {
    #[error("the name {0} was exist")]
    UserNameExist(String),
}

impl Into<String> for UserErr {
    fn into(self) -> String {
        match self {
            UserErr::UserNameExist(name) => {
                // Because `TraceLayer` wraps each request in a span that contains the request
                // method, uri, etc we don't need to include those details here
                tracing::error!("error from user_name {name} exist");

                // Don't expose any details about the error to the client
                AppRes::<()>::fail_with_msg(format!("用户名{name}已存在")).into()
            }
        }
    }
}

async fn all(State(app_state): State<AppState>) -> AppRes<Vec<user::Model>> {
    let result = user::Entity::find().all(&app_state.db().await).await;
    let model = result.unwrap();
    println!("{model:?}");
    AppRes::success(model)
}

async fn register(State(app_state): State<AppState>, ValidatedJson(req): ValidatedJson<UserRegisterReq>) -> Result<Json<user::Model>, ServerError> {
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
    Ok(Json(model))
}

async fn find_by_name(app_state: &AppState, name: &str) -> Result<Option<user::Model>, DbErr> {
    user::Entity::find().filter(user::Column::Name.eq(name)).one(&app_state.db().await).await
}



