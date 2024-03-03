use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use color_eyre::eyre::eyre;
use sea_orm::DbErr;
use thiserror::Error;
use tracing::{error, warn};
use utoipa::ToSchema;
use validator::ValidationErrors;

use crate::auth::AuthError;
use crate::user::UserErr;
use crate::AppRes;

#[derive(Debug, Error, ToSchema)]
pub enum ServerError {
    #[error("err: {0}")]
    CustomErr(String),
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
    #[error(transparent)]
    IoErr(#[from] std::io::Error),
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        match self {
            ServerError::ValidationError(_) => {
                let message = format!("Input validation error: [{self}]").replace('\n', ", ");
                (
                    StatusCode::BAD_REQUEST,
                    Json(AppRes::fail_with_msg(message)),
                )
            }
            ServerError::AxumJsonRejection(ref err) => {
                warn!("request json parse err: {err}");
                (
                    StatusCode::BAD_REQUEST,
                    Json(AppRes::fail_with_msg(self.to_string())),
                )
            }
            ServerError::DbErr(err) => {
                let report = eyre!("db error happened: {err}");
                error!(?report);
                (StatusCode::INTERNAL_SERVER_ERROR, Json(AppRes::fail()))
            }
            ServerError::UserErr(err) => {
                err.print();
                match err {
                    UserErr::UserNameExist(_) => (
                        StatusCode::CONFLICT,
                        Json(AppRes::fail_with_msg(err.to_string())),
                    ),
                }
            }
            ServerError::AuthErr(err) => {
                err.print();
                (StatusCode::OK, Json(AppRes::fail_with_msg(err.to_string())))
            }
            ServerError::MsgErr(err) => {
                err.print();
                (StatusCode::OK, Json(AppRes::fail_with_msg(err.to_string())))
            }
            ServerError::IoErr(err) => {
                err.print();
                (StatusCode::OK, Json(AppRes::fail_with_msg(err.to_string())))
            }
            ServerError::CustomErr(err) => {
                err.print();
                (StatusCode::OK, Json(AppRes::fail_with_msg(err.to_string())))
            }
        }
        .into_response()
    }
}

pub trait ErrPrint: std::fmt::Display {
    fn print(&self) {
        let report = eyre!(self.to_string());
        error!(?report);
    }
}

impl ErrPrint for String {}

impl ErrPrint for msg::Error {}

impl ErrPrint for std::io::Error {}

// impl ErrPrint for CustomErr{}

// #[derive(Debug, Error)]
// struct CustomErr {
//     msg: String,
// }
