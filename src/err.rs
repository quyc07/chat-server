use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::DbErr;
use thiserror::Error;
use tracing::log::{error, warn};
use validator::ValidationErrors;

use crate::AppRes;
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
                (StatusCode::BAD_REQUEST, String::from(AppRes::fail_with_msg(message)))
            }
            ServerError::AxumJsonRejection(ref err) => {
                warn!("json parse err: {err}");
                (StatusCode::BAD_REQUEST, String::from(AppRes::fail_with_msg(self.to_string())))
            }
            ServerError::UserErr(err) => (StatusCode::OK, err.into()),
            ServerError::DbErr(err) => {
                error!("db error happened {err}");
                (StatusCode::INTERNAL_SERVER_ERROR, String::from(AppRes::fail()))
            }
        }.into_response()
    }
}
