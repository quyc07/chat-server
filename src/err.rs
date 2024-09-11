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
use crate::friend::FriendErr;
use crate::group::GroupErr;
use crate::user::UserErr;
use crate::{friend, AppRes};

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
    GroupErr(#[from] GroupErr),
    #[error(transparent)]
    DbErr(#[from] DbErr),
    #[error(transparent)]
    AuthErr(#[from] AuthError),
    #[error(transparent)]
    MsgErr(#[from] msg::Error),
    #[error(transparent)]
    IoErr(#[from] std::io::Error),
    #[error(transparent)]
    ReqwestErr(#[from] reqwest::Error),
    #[error(transparent)]
    FriendErr(#[from] FriendErr),
}

const ERROR_MESSAGE: &str = "系统异常，请稍后再试";

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        match self {
            ServerError::ValidationError(_) => {
                let message = format!("Input validation error: [{self}]").replace('\n', ", ");
                (StatusCode::BAD_REQUEST, Json(message)).into_response()
            }
            ServerError::AxumJsonRejection(ref err) => {
                warn!("request json parse err: {err}");
                (StatusCode::BAD_REQUEST, Json(self.to_string())).into_response()
            }
            ServerError::DbErr(err) => {
                let report = eyre!("db error happened: {err}");
                error!(?report);
                (StatusCode::INTERNAL_SERVER_ERROR, ERROR_MESSAGE).into_response()
            }
            ServerError::UserErr(err) => {
                err.print();
                match err {
                    UserErr::UserNameExist(_) => {
                        (StatusCode::CONFLICT, err.to_string()).into_response()
                    }
                    UserErr::UserNotExist(_) => {
                        (StatusCode::NOT_FOUND, err.to_string()).into_response()
                    }
                    UserErr::UserWasFreeze(_) => {
                        (StatusCode::UNAUTHORIZED, err.to_string()).into_response()
                    }
                    UserErr::LoginUserWasFreeze => {
                        (StatusCode::UNAUTHORIZED, err.to_string()).into_response()
                    }
                    UserErr::UserNameNotExist(_) => {
                        (StatusCode::NOT_FOUND, err.to_string()).into_response()
                    }
                }
            }
            ServerError::GroupErr(err) => {
                err.print();
                match err {
                    GroupErr::GroupNotExist(_) => {
                        (StatusCode::NOT_FOUND, err.to_string()).into_response()
                    }
                    GroupErr::UserNotInGroup { .. } => {
                        (StatusCode::NOT_FOUND, err.to_string()).into_response()
                    }
                    GroupErr::UserHasBeenForbid => {
                        (StatusCode::NOT_MODIFIED, err.to_string()).into_response()
                    }
                    GroupErr::UserAlreadyInGroup => {
                        (StatusCode::NOT_MODIFIED, err.to_string()).into_response()
                    }
                    GroupErr::UserWasNotForbid => {
                        (StatusCode::NOT_MODIFIED, err.to_string()).into_response()
                    }
                    GroupErr::YouAreNotAdmin => {
                        (StatusCode::FORBIDDEN, err.to_string()).into_response()
                    }
                    GroupErr::YouAreForbid => (StatusCode::FORBIDDEN, err.to_string()).into_response(),
                }
            }
            ServerError::AuthErr(err) => {
                err.print();
                match err {
                    AuthError::UserNotExist => {
                        (StatusCode::NOT_FOUND, err.to_string()).into_response();
                    }
                    AuthError::WrongCredentials => {
                        (StatusCode::UNAUTHORIZED, err.to_string()).into_response();
                    }
                    AuthError::MissingCredentials => {
                        (StatusCode::UNAUTHORIZED, err.to_string()).into_response();
                    }
                    AuthError::TokenCreation => {
                        (StatusCode::UNAUTHORIZED, err.to_string()).into_response();
                    }
                    AuthError::InvalidToken => {
                        (StatusCode::UNAUTHORIZED, err.to_string()).into_response();
                    }
                    AuthError::NeedAdmin => {
                        (StatusCode::FORBIDDEN, err.to_string()).into_response();
                    }
                }
                (StatusCode::UNAUTHORIZED, err.to_string()).into_response()
            }
            ServerError::MsgErr(err) => {
                err.print();
                (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
            }
            ServerError::IoErr(err) => {
                err.print();
                (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
            }
            ServerError::CustomErr(err) => {
                err.print();
                (StatusCode::INTERNAL_SERVER_ERROR,).into_response()
            }
            ServerError::ReqwestErr(err) => {
                err.print();
                (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
            }
            ServerError::FriendErr(err) => {
                err.print();
                match err {
                    FriendErr::NotFriend(_) => {
                        (StatusCode::NOT_FOUND, err.to_string()).into_response()
                    }
                    FriendErr::CanNotReviewFriendRequest => {
                        (StatusCode::FORBIDDEN, err.to_string()).into_response()
                    }
                    FriendErr::AlreadyFriend => {
                        (StatusCode::NOT_MODIFIED, err.to_string()).into_response()
                    }
                    FriendErr::RequestWaiting => {
                        (StatusCode::NOT_MODIFIED, err.to_string()).into_response()
                    }
                }
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
