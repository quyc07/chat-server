use crate::app_state::AppState;
use crate::auth::Token;
use crate::err::ServerError;
use crate::{auth, user};
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

// 状态检查
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

// 校验token有效期
pub(crate) async fn check_login(token: Token, request: Request, next: Next) -> Response {
    if let Err(err) = auth::check_token_expire(token).await {
        return ServerError::from(err).into_response();
    }
    let response = next.run(request).await;
    response
}
