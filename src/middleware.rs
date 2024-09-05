use crate::app_state::AppState;
use crate::auth::{AuthError, Token};
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
    if let Err(err) = user::check_status(token.id, token.id, &state).await {
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

pub(crate) async fn check_admin(token: Token, request: Request, next: Next) -> Response {
    match auth::check_admin(token).await {
        Err(err) => ServerError::from(err).into_response(),
        Ok(true) => next.run(request).await,
        _ => ServerError::from(AuthError::NeedAdmin).into_response(),
    }
}
