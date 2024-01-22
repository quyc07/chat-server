use axum::extract::FromRequest;
use axum::Json;
use axum::response::{IntoResponse, Response};

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

impl<T> IntoResponse for AppJson<T>
    where
        Json<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        Json(self.0).into_response()
    }
}