use axum::extract::FromRequest;
use axum::http::StatusCode;
use axum::Json;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

pub mod user;
pub mod entity;
pub mod app_state;
pub mod err;
pub mod validate;

// Create our own JSON extractor by wrapping `axum::Json`. This makes it easy to override the
// rejection and provide our own which formats errors to match our application.
//
// `axum::Json` responds with plain text if the input is invalid.
#[derive(FromRequest)]
#[from_request(via(axum::Json))]
pub struct AppJson<T>(pub T);

pub struct AppRes<T> {
    code: i8,
    msg: String,
    data: T,
}

impl<T: Deserialize> IntoResponse for AppRes<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, self).into_response()
    }
}

// impl<T> From<AppRes<T>> for (StatusCode, Json<T>) {
//     fn from(value: AppRes<T>) -> Self {
//         (StatusCode::OK,Json(value))
//     }
// }