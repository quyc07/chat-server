use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chat_server::app_state::AppState;
use entity::read_index;
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::{NotSet, Set};
use serde_json::{Value, json};
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

/// Write a message payload in the wire shape the server reads back from sled.
fn append_dm(app_state: &AppState, from: i32, to: i32, content: &str, created_at: &str) -> i64 {
    let payload = json!({
        "from_uid": from,
        "created_at": created_at,
        "target": { "User": { "uid": to } },
        "detail": { "Normal": { "content": { "content": content } } },
    });
    app_state
        .msg_db
        .lock()
        .unwrap()
        .messages()
        .send_to_dm(from as i64, to as i64, payload.to_string().as_bytes())
        .unwrap()
}

async fn seed_read_index(
    app_state: &AppState,
    uid: i32,
    target_uid: i32,
    mid: Option<i64>,
    latest_mid: i64,
) {
    read_index::ActiveModel {
        id: Default::default(),
        uid: Set(uid),
        target_uid: Set(Some(target_uid)),
        target_gid: Set(None),
        mid: Set(mid),
        latest_mid: Set(latest_mid),
        uid_of_latest_msg: Set(target_uid),
    }
    .insert(&app_state.db)
    .await
    .unwrap();
}

fn json_request(method: Method, uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn login(app: &Router, name: &str) -> String {
    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/token/login",
            None,
            json!({ "name": name, "password": "password" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice::<Value>(&body).unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Fetch one conversation-list page and decode the body.
async fn conversation_page(
    app: &Router,
    token: &str,
    page: u64,
    limit: u64,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/user/history",
            Some(token),
            json!({ "page": page, "limit": limit }),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn user_names(rows: &[Value]) -> Vec<&str> {
    rows.iter()
        .map(|row| row["User"]["user_name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn conversation_list_is_ordered_by_recency_across_pages() {
    let (_data_dir, app, app_state) = boot().await;
    let me = create_user(&app_state, "me").await;
    let alice = create_user(&app_state, "alice").await;
    let bob = create_user(&app_state, "bob").await;
    let carol = create_user(&app_state, "carol").await;

    let alice_mid = append_dm(&app_state, alice, me, "oldest", "2024-01-01 10:00:00");
    let bob_mid = append_dm(&app_state, bob, me, "middle", "2024-01-01 11:00:00");
    let carol_mid = append_dm(&app_state, carol, me, "newest", "2024-01-01 12:00:00");
    assert!(alice_mid < bob_mid && bob_mid < carol_mid);

    // Seeded in an order that has nothing to do with recency.
    seed_read_index(&app_state, me, alice, None, alice_mid).await;
    seed_read_index(&app_state, me, carol, Some(carol_mid), carol_mid).await;
    seed_read_index(&app_state, me, bob, Some(bob_mid), bob_mid).await;

    let token = login(&app, "me").await;

    let (status, page1) = conversation_page(&app, &token, 1, 2).await;
    assert_eq!(status, StatusCode::OK, "body: {page1}");
    let page1 = page1.as_array().unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(user_names(page1), vec!["carol", "bob"]);

    // The fields every conversation row already carried must not regress.
    let newest = &page1[0]["User"];
    assert_eq!(newest["uid"].as_i64(), Some(carol as i64));
    assert_eq!(newest["mid"].as_i64(), Some(carol_mid));
    assert_eq!(newest["msg"].as_str(), Some("newest"));
    assert_eq!(newest["msg_time"].as_str(), Some("2024-01-01 12:00:00"));
    assert_eq!(newest["unread"], Value::Null);

    // Same underlying data, same order.
    let (status, repeated) = conversation_page(&app, &token, 1, 2).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        user_names(repeated.as_array().unwrap()),
        vec!["carol", "bob"]
    );

    // Paging continues in recency order instead of restarting a local sort.
    let (status, page2) = conversation_page(&app, &token, 2, 2).await;
    assert_eq!(status, StatusCode::OK);
    let page2 = page2.as_array().unwrap();
    assert_eq!(user_names(page2), vec!["alice"]);
    assert_eq!(page2[0]["User"]["mid"].as_i64(), Some(alice_mid));
    assert_eq!(page2[0]["User"]["msg"].as_str(), Some("oldest"));
    assert_eq!(
        page2[0]["User"]["msg_time"].as_str(),
        Some("2024-01-01 10:00:00")
    );
    assert_eq!(page2[0]["User"]["unread"].as_str(), Some("all"));
}

#[tokio::test]
async fn conversation_list_accepts_a_page_size_up_to_50() {
    let (_data_dir, app, app_state) = boot().await;
    let me = create_user(&app_state, "me").await;
    let mut peers = Vec::new();
    for index in 0..12 {
        peers.push(create_user(&app_state, &format!("peer{index}")).await);
    }
    for (index, peer) in peers.iter().enumerate() {
        let latest_mid = index as i64 + 1;
        seed_read_index(&app_state, me, *peer, Some(latest_mid), latest_mid).await;
    }

    let token = login(&app, "me").await;

    let (status, rows) = conversation_page(&app, &token, 1, 50).await;
    assert_eq!(status, StatusCode::OK, "body: {rows}");
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 12);
    let mids = rows
        .iter()
        .map(|row| row["User"]["mid"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(mids, (1..=12).rev().collect::<Vec<_>>());

    // The upper bound still exists, it is just larger.
    let (status, _) = conversation_page(&app, &token, 1, 51).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
