use axum::{async_trait, Json, Router};
use axum::extract::{FromRequest, Request};
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use thiserror::Error;
use validator::{Validate, ValidationErrors};

pub struct UserApi;

impl UserApi {
    pub async fn route() -> Router {
        Router::new().route("/register", post(register))
    }
}

#[derive(Debug, Deserialize, Serialize, Validate)]
struct UserRegisterReq {
    #[validate(required)]
    name: Option<String>,
    #[validate(length(min = 10))]
    email: String,
    password: String,
}

// async fn register(Json(payload): Json<UserRegisterReq>) -> axum::response::Result<Json<UserRegisterReq>> {
//     payload.validate()?;
//     println!("{payload:#?}");
//     Ok(Json(payload))
// }

async fn register(ValidatedJson(payload): ValidatedJson<UserRegisterReq>) -> axum::response::Result<Json<UserRegisterReq>> {
    println!("{payload:#?}");
    Ok(Json(payload))
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