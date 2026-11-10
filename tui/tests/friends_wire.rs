//! Wire tests for the friends HTTP calls.
//!
//! The fixtures are copied from the server's own response types:
//! - `GET /user/find/{name}` -> `src/user.rs::FindFriendRes` (`id`, `name`),
//! - `GET /friend/req` -> `src/friend.rs::FriendReqVo`, including the
//!   `%Y-%m-%d %H:%M:%S` `create_time` and the uppercase
//!   `WAIT` / `APPROVE` / `REJECT` status strings from
//!   `entity::sea_orm_active_enums::FriendRequestStatus`,
//! - `GET /friend` -> `src/friend.rs::Friend` (`id`, `name`).
//!
//! The unusual status codes are part of the contract: a repeated friend request
//! comes back as **201** with the reason in the body, and a forbidden review as
//! **403**. These tests pin the meaning, not just "2xx vs not".

use chat_tui::api::ApiClient;
use chat_tui::model::{FoundUser, Friend, FriendDecision, FriendRequestStatus, SendOutcome};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIND_JSON: &str = r#"[
  {"id":2,"name":"alice"},
  {"id":5,"name":"alice-bot"}
]"#;

const REQUESTS_JSON: &str = r#"[
  {"id":11,"request_id":3,"request_name":"bob","create_time":"2026-11-11 10:00:00","reason":null,"status":"WAIT"},
  {"id":12,"request_id":4,"request_name":"carol","create_time":"2026-11-10 09:00:00","reason":"一起玩","status":"APPROVE"}
]"#;

const FRIENDS_JSON: &str = r#"[
  {"id":2,"name":"alice"},
  {"id":3,"name":"bob"}
]"#;

#[tokio::test]
async fn find_users_gets_by_name_with_bearer_and_decodes_pairs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/find/ali"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(FIND_JSON, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let users = api.find_users("ali").await.expect("search decodes");

    assert_eq!(
        users,
        vec![
            FoundUser {
                id: 2,
                name: "alice".to_string()
            },
            FoundUser {
                id: 5,
                name: "alice-bot".to_string()
            },
        ]
    );
    server.verify().await;
}

#[tokio::test]
async fn find_users_percent_encodes_a_name_with_spaces() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/find/%E7%99%BD%20%E8%8F%9C"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let users = api.find_users("白 菜").await.expect("search decodes");

    assert!(users.is_empty());
    server.verify().await;
}

#[tokio::test]
async fn pending_requests_get_decodes_real_server_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/friend/req"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(REQUESTS_JSON, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let requests = api.friend_requests().await.expect("requests decode");

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].id, 11);
    assert_eq!(requests[0].request_id, 3);
    assert_eq!(requests[0].request_name, "bob");
    assert_eq!(
        requests[0].create_time,
        chrono::NaiveDateTime::parse_from_str(
            "2026-11-11 10:00:00",
            chat_tui::model::WIRE_TIME_FORMAT
        )
        .expect("timestamp")
    );
    assert_eq!(requests[0].reason, None);
    assert_eq!(requests[0].status, FriendRequestStatus::Wait);
    assert_eq!(requests[1].reason.as_deref(), Some("一起玩"));
    assert_eq!(requests[1].status, FriendRequestStatus::Approve);

    server.verify().await;
}

#[tokio::test]
async fn friend_list_gets_and_decodes_id_name_pairs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/friend"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(FRIENDS_JSON, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let friends = api.friends().await.expect("friends decode");

    assert_eq!(
        friends,
        vec![
            Friend {
                id: 2,
                name: "alice".to_string()
            },
            Friend {
                id: 3,
                name: "bob".to_string()
            },
        ]
    );
    server.verify().await;
}

#[tokio::test]
async fn send_friend_request_posts_the_target_and_reads_plain_success_as_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/friend/req/7"))
        .and(header("authorization", "Bearer tok-1"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({ "reason": null })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let outcome = api
        .send_friend_request(7, None)
        .await
        .expect("request sent");

    assert_eq!(outcome, SendOutcome::Sent);
    server.verify().await;
}

#[tokio::test]
async fn an_already_friend_201_is_classified_by_meaning_not_as_success_or_crash() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/friend/req/7"))
        .respond_with(
            // `FriendErr::AlreadyFriend` is rendered as
            // `(StatusCode::CREATED, err.to_string())`: a plain text 201.
            ResponseTemplate::new(201)
                .set_body_raw("已经是好友，无需再次申请", "text/plain; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let outcome = api.send_friend_request(7, None).await.expect("not a crash");

    assert_eq!(outcome, SendOutcome::AlreadyFriend);
}

#[tokio::test]
async fn a_request_already_waiting_201_is_classified_separately() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/friend/req/7"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_raw("请求等待中，请勿再次发起", "text/plain; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let outcome = api.send_friend_request(7, None).await.expect("not a crash");

    assert_eq!(outcome, SendOutcome::AlreadyWaiting);
}

#[tokio::test]
async fn a_forbidden_friend_request_is_a_readable_outcome_not_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/friend/req/7"))
        .respond_with(
            ResponseTemplate::new(403).set_body_raw("用户已被冻结", "text/plain; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let outcome = api.send_friend_request(7, None).await.expect("not a crash");

    assert_eq!(outcome, SendOutcome::Forbidden("用户已被冻结".to_string()));
}

#[tokio::test]
async fn review_posts_the_request_id_and_uppercase_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/friend/req"))
        .and(header("authorization", "Bearer tok-1"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({ "id": 11, "status": "APPROVE" })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    api.review_friend_request(11, FriendDecision::Approve)
        .await
        .expect("review applied");

    server.verify().await;
}

#[tokio::test]
async fn a_forbidden_review_is_classified_by_meaning() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/friend/req"))
        .respond_with(ResponseTemplate::new(403).set_body_raw(
            "您不是该好友请求的目标对象，无权批准",
            "text/plain; charset=utf-8",
        ))
        .mount(&server)
        .await;

    let mut api = ApiClient::new(server.uri());
    api.set_token(Some("tok-1".to_string()));
    let outcome = api
        .review_friend_request(11, FriendDecision::Reject)
        .await
        .expect("not a crash");

    assert_eq!(
        outcome,
        chat_tui::model::ReviewOutcome::Forbidden(
            "您不是该好友请求的目标对象，无权批准".to_string()
        )
    );
}
