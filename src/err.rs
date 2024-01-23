use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::DbErr;
use thiserror::Error;
use validator::ValidationErrors;

use crate::user::UserErr;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    ValidationError(#[from] ValidationErrors),
    #[error(transparent)]
    AxumJsonRejection(#[from] JsonRejection),
    #[error(transparent)]
    UserErr(#[from] UserErr),
    #[error(transparent)]
    DbErr(#[from] DbErr),
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        match self {
            ServerError::ValidationError(_) => {
                let message = format!("Input validation error: [{self}]").replace('\n', ", ");
                (StatusCode::BAD_REQUEST, message)
            }
            ServerError::AxumJsonRejection(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ServerError::UserErr(err) => err.into(),
            ServerError::DbErr(err) => {
                println!("{err}");
                tracing::error!("db err {err}");
                (StatusCode::INTERNAL_SERVER_ERROR, "系统异常请稍后再试".to_string())
            }
        }
            .into_response()
    }
}
