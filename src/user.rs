use axum::{async_trait, Json, Router};
use axum::extract::{FromRequest, Request, State};
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use sea_orm::{ActiveModelTrait, DbErr, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use thiserror::Error;
use validator::{Validate, ValidationErrors};

use crate::app_state::AppState;
use crate::entity::user;

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

// Create our own JSON extractor by wrapping `axum::Json`. This makes it easy to override the
// rejection and provide our own which formats errors to match our application.
//
// `axum::Json` responds with plain text if the input is invalid.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(UserErr))]
struct AppJson<T>(T);

impl<T> IntoResponse for AppJson<T>
    where
        Json<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        Json(self.0).into_response()
    }
}

// The kinds of errors we can hit in our application.
enum UserErr {
    // The request body contained invalid JSON
    JsonRejection(JsonRejection),
    UserNameExist(String),
    DbErr(DbErr),
}

// Tell axum how `AppError` should be converted into a response.
//
// This is also a convenient place to log errors.
impl IntoResponse for UserErr {
    fn into_response(self) -> Response {
        // How we want errors responses to be serialized
        #[derive(Serialize)]
        struct ErrorResponse {
            message: String,
        }

        let (status, message) = match self {
            UserErr::JsonRejection(rejection) => {
                // This error is caused by bad user input so don't log it
                (rejection.status(), rejection.body_text())
            }
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
        };

        (status, AppJson(ErrorResponse { message })).into_response()
    }
}

impl From<JsonRejection> for UserErr {
    fn from(rejection: JsonRejection) -> Self {
        Self::JsonRejection(rejection)
    }
}

impl From<DbErr> for UserErr {
    fn from(value: DbErr) -> Self {
        UserErr::DbErr(value)
    }
}

async fn all(State(app_state): State<AppState>) -> Json<Vec<user::Model>> {
    let result = user::Entity::find().all(&app_state.db().await).await;
    let model = result.unwrap();
    println!("{model:?}");
    Json(model)
}

async fn register(ValidatedJson(req): ValidatedJson<UserRegisterReq>, State(app_state): State<AppState>) -> Result<Json<user::Model>, UserErr> {
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

#[derive(Debug, Clone, Copy, Default)]
pub struct ValidatedJson<T>(pub T);

#[async_trait]
impl<T, S> FromRequest<S> for ValidatedJson<T>
    where
        T: DeserializeOwned + Validate,
        S: Send + Sync,
        Json<T>: FromRequest<S, Rejection=JsonRejection>,
{
    type Rejection = ServerError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state).await?;
        value.validate()?;
        Ok(ValidatedJson(value))
    }
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    ValidationError(#[from] ValidationErrors),

    #[error(transparent)]
    AxumJsonRejection(#[from] JsonRejection),
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        match self {
            ServerError::ValidationError(_) => {
                let message = format!("Input validation error: [{self}]").replace('\n', ", ");
                (StatusCode::BAD_REQUEST, message)
            }
            ServerError::AxumJsonRejection(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        }
            .into_response()
    }
}