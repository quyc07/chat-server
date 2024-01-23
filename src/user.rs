use axum::{Json, Router};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use validator::Validate;

use crate::app_state::AppState;
use crate::entity::user;
use crate::entity::user::Model;
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

// Tell axum how `AppError` should be converted into a response.
//
// This is also a convenient place to log errors.
impl Into<(StatusCode, String)> for UserErr {
    fn into(self) -> (StatusCode, String) {
        // How we want errors responses to be serialized
        #[derive(Serialize)]
        struct ErrorResponse {
            message: String,
        }

        match self {
            UserErr::UserNameExist(name) => {
                // Because `TraceLayer` wraps each request in a span that contains the request
                // method, uri, etc we don't need to include those details here
                tracing::error!("error from user_name {name} exist");

                // Don't expose any details about the error to the client
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("用户名{name}已存在")
                )
            }
            _ => {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "系统异常，请稍后再试".to_string()
                )
            }
        }
    }
}

async fn all(State(app_state): State<AppState>) -> Json<Vec<user::Model>> {
    let result = user::Entity::find().all(&app_state.db().await).await;
    let model = result.unwrap();
    println!("{model:?}");
    Json(model)
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
        name: Set(req.name.unwrap()),
        email: Set(req.email),
        phone: Set(req.phone),
        create_time: Default::default(),
        update_time: Default::default(),
    };
    let model = user.insert(&app_state.db().await).await?;
    Ok(Json(model))
}

async fn find_by_name(app_state: &AppState, name: &str) -> Result<Option<Model>, DbErr> {
    user::Entity::find().filter(user::Column::Name.eq(name)).one(&app_state.db().await).await
}



