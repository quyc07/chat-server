//! The push-stream handshake.
//!
//! `POST /token/login` then `GET /event/stream?token=…` is the only way a
//! client receives live messages, so the handshake must not depend on headers
//! the HTTP spec does not require.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chat_server::app_state::AppState;
use migration::{Migrator, MigratorTrait};
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::{NotSet, Set};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

/// Boot the real router against a throwaway data directory.
async fn boot() -> (TempDir, Router, AppState) {
    let data_dir = tempfile::tempdir().unwrap();
    let app_state = AppState::new(data_dir.path()).await.unwrap();
    Migrator::up(&app_state.db, None).await.unwrap();
    let app = chat_server::router(app_state.clone()).await;
    (data_dir, app, app_state)
}

async fn create_user(app_state: &AppState, name: &str) -> i32 {
    entity::user::ActiveModel {
        id: Default::default(),
        name: Set(name.to_owned()),
        password: Set("password".to_owned()),
        email: NotSet,
        phone: NotSet,
        create_time: Default::default(),
        update_time: NotSet,
        status: Default::default(),
        role: Default::default(),
    }
    .insert(&app_state.db)
    .await
    .unwrap()
    .id
}

async fn login(app: &Router, name: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/token/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"name":"{name}","password":"password"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    body["access_token"].as_str().unwrap().to_owned()
}

/// `User-Agent` is not a required header in HTTP. A stream handshake that
/// rejects requests without one shuts out every client whose HTTP library sends
/// no default — `reqwest`, for instance. The symptom is silent: the client sees
/// a failed connect, treats it as a dead link, and redials forever, so the user
/// never receives a live message and only sees it after a re-login refetches
/// history.
#[tokio::test]
async fn the_push_stream_accepts_a_client_that_sends_no_user_agent() {
    let (_data_dir, app, app_state) = boot().await;
    create_user(&app_state, "alice").await;
    let token = login(&app, "alice").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/event/stream?token={token}"))
                // Deliberately no `User-Agent`.
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a stream handshake must not require a header HTTP does not require"
    );
}
