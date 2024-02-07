use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use color_eyre::eyre::eyre;
use sea_orm::DbErr;
use thiserror::Error;
use tracing::{error, warn};
use validator::ValidationErrors;
use msg::Error;

use crate::AppRes;
use crate::auth::AuthError;
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
    #[error(transparent)]
    AuthErr(#[from] AuthError),
    #[error(transparent)]
    MsgErr(#[from] msg::Error),
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        match self {
            ServerError::ValidationError(_) => {
                let message = format!("Input validation error: [{self}]").replace('\n', ", ");
                (StatusCode::BAD_REQUEST, String::from(AppRes::fail_with_msg(message)))
            }
            ServerError::AxumJsonRejection(ref err) => {
                warn!("request json parse err: {err}");
                (StatusCode::BAD_REQUEST, String::from(AppRes::fail_with_msg(self.to_string())))
            }
            ServerError::DbErr(err) => {
                let report = eyre!("db error happened: {err}");
                error!(?report);
                (StatusCode::INTERNAL_SERVER_ERROR, String::from(AppRes::fail()))
            }
            ServerError::UserErr(err) => {
                err.print();
                (StatusCode::OK, err.into())
            }
            ServerError::AuthErr(err) => {
                err.print();
                (StatusCode::OK, err.into())
            }
            ServerError::MsgErr(err) => {
                err.print();
                (StatusCode::OK, MsgErrorWrapper(err).into())
            }
        }.into_response()
    }
}

pub trait ErrPrint: std::fmt::Display {
    fn print(&self) {
        let report = eyre!(self.to_string());
        error!(?report);
    }
}

impl ErrPrint for msg::Error {}

struct MsgErrorWrapper(msg::Error);

impl From<MsgErrorWrapper> for String {
    fn from(value: MsgErrorWrapper) -> Self {
        value.0.to_string()
    }
}