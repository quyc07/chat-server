use crate::app_state::AppState;
use crate::auth::Token;
use crate::user;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

pub(crate) async fn check_user_status(
    State(state): State<AppState>,
    token: Token,
    request: Request,
    next: Next,
) -> Response {
    if let Err(err) = user::check_status(token.id, &state).await {
        return err.into_response();
    }
    let response = next.run(request).await;
    response
}
