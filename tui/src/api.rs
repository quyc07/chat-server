//! HTTP wire client.
//!
//! The client speaks HTTP and JSON only. This module is the single place that
//! knows the server's routes, headers, request bodies, and status codes.
//!
//! Auth: normal requests use `Authorization: Bearer`. The token lives in memory
//! only; nothing here touches the filesystem.

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde_json::json;

use crate::model::{
    self, Conversation, FoundUser, Friend, FriendDecision, FriendRequest, LoginWire, Message,
    ReadPosition, ReviewOutcome, SendOutcome,
};

/// How long a request may take before the UI reports the server unreachable.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// The server could not be reached at all (connect error, timeout, DNS...).
    #[error("无法连接服务器：{0}")]
    Unreachable(String),
    /// 401: credentials were rejected or the token is no longer valid.
    #[error("{0}")]
    Unauthorized(String),
    /// Any other non-success status. `message` is the server's own text.
    #[error("服务器返回 {status}：{message}")]
    Status { status: u16, message: String },
    /// The body was not the JSON the server documents.
    #[error("无法解析服务器响应：{0}")]
    Decode(String),
}

impl ApiError {
    /// Text suitable for the status line, with no protocol jargon and no outer
    /// JSON quotes (the server's error bodies are bare JSON strings).
    pub fn message(&self) -> String {
        match self {
            ApiError::Unreachable(reason) => format!("无法连接服务器：{reason}"),
            ApiError::Unauthorized(message) => message.clone(),
            ApiError::Status { status, message } => {
                if message.is_empty() {
                    format!("服务器返回 {status}")
                } else {
                    message.clone()
                }
            }
            ApiError::Decode(reason) => format!("无法解析服务器响应：{reason}"),
        }
    }
}

/// A logged-in HTTP client. Cloning is cheap (the underlying `reqwest::Client`
/// is shared) and gives a task a consistent token snapshot.
#[derive(Clone)]
pub struct ApiClient {
    http: Client,
    /// A second client without a total-request timeout, used only for the event
    /// stream: the shared client's 10-second timeout covers the response body,
    /// which would tear down a long-lived connection mid-stream.
    stream_http: Client,
    base_url: String,
    token: Option<String>,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is a credential; a stray debug print must not leak it.
        f.debug_struct("ApiClient")
            .field("base_url", &self.base_url)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .finish()
    }
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest client configuration is valid");
        let stream_http = Client::builder()
            .connect_timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest client configuration is valid");
        Self {
            http,
            stream_http,
            base_url,
            token: None,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn set_token(&mut self, token: Option<String>) {
        self.token = token;
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// `POST /token/login`, returns the access token and installs it.
    pub async fn login(&mut self, name: &str, password: &str) -> Result<String, ApiError> {
        let response = self
            .http
            .post(self.url("/token/login"))
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({ "name": name, "password": password }))
            .send()
            .await
            .map_err(map_send_error)?;
        let body = read_body(response).await?;
        let login: LoginWire =
            model::login_from_json(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        self.token = Some(login.access_token.clone());
        Ok(login.access_token)
    }

    /// `POST /user/register`, returns the id the server assigned.
    pub async fn register(&mut self, name: &str, password: &str) -> Result<String, ApiError> {
        let response = self
            .http
            .post(self.url("/user/register"))
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({ "name": name, "password": password }))
            .send()
            .await
            .map_err(map_send_error)?;
        let body = read_body(response).await?;
        // `register` returns `Res<String>`, and axum renders `String` as a plain
        // text body: the response is `7`, not JSON `"7"`. Verified against the
        // running server, not assumed.
        std::str::from_utf8(&body)
            .map(|text| text.trim().to_string())
            .map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `DELETE /token/logout`, authenticated with the current token.
    pub async fn logout(&mut self) -> Result<(), ApiError> {
        let token = self
            .token
            .clone()
            .ok_or_else(|| ApiError::Unauthorized("登录状态已丢失，请重新登录".to_string()))?;
        self.logout_with(&token).await?;
        self.token = None;
        Ok(())
    }

    /// Logout with an explicit token. The wiring layer uses this after the state
    /// machine has already cleared its local token on quit.
    pub async fn logout_with(&self, token: &str) -> Result<(), ApiError> {
        let response = self
            .http
            .delete(self.url("/token/logout"))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
            .map_err(map_send_error)?;
        read_body(response).await?;
        Ok(())
    }

    /// `POST /user/history` with `{page, limit}`, authenticated with a Bearer
    /// header. The server already returns recency order; the state machine sorts
    /// each page again as a defensive measure.
    pub async fn history(&self, page: u64, limit: u64) -> Result<Vec<Conversation>, ApiError> {
        let mut request = self
            .http
            .post(self.url("/user/history"))
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({ "page": page, "limit": limit }));
        if let Some(token) = &self.token {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = request.send().await.map_err(map_send_error)?;
        let body = read_body(response).await?;
        model::conversations_from_json(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `GET /user/{peer}/history`: the direct-message history with one peer, at
    /// most 1000 rows, oldest first. The endpoint has no pagination parameters and
    /// requires friendship; a non-friend peer comes back as a 404 carrying the
    /// server's own explanation.
    pub async fn messages(&self, peer: i32) -> Result<Vec<Message>, ApiError> {
        let mut request = self.http.get(self.url(&format!("/user/{peer}/history")));
        if let Some(token) = &self.token {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = request.send().await.map_err(map_send_error)?;
        let body = read_body(response).await?;
        model::messages_from_json(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `PUT /user/{peer}/send` with `{msg}`: send one direct message and return
    /// the new message id. The server renders `Res<String>` as plain text, so
    /// the body is `42` (not JSON `"42"`); the model layer is the only place that
    /// parses it. A non-friend peer is a 404 with the server's own reason.
    pub async fn send_message(&self, peer: i32, msg: &str) -> Result<i64, ApiError> {
        let response = self
            .authorized(
                self.http
                    .put(self.url(&format!("/user/{peer}/send")))
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({ "msg": msg })),
            )
            .send()
            .await
            .map_err(map_send_error)?;
        let body = read_body(response).await?;
        model::sent_mid_from_body(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `PUT /ri` with `{"User":{"target_uid":..,"mid":..}}` or the group shape:
    /// the read position the server derives unread counts from. Success carries
    /// no body; any failure is best effort (the state machine only surfaces it).
    pub async fn report_read(&self, position: ReadPosition) -> Result<(), ApiError> {
        let response = self
            .authorized(
                self.http
                    .put(self.url("/ri"))
                    .header(CONTENT_TYPE, "application/json")
                    .json(&model::read_index_body(position)),
            )
            .send()
            .await
            .map_err(map_send_error)?;
        read_body(response).await?;
        Ok(())
    }

    /// `GET /user/detail/{uid}`, the display name of one peer. Used when a
    /// stream message names a conversation the list has never seen.
    pub async fn user_detail(&self, uid: i32) -> Result<FoundUser, ApiError> {
        let response = self
            .authorized(self.http.get(self.url(&format!("/user/detail/{uid}"))))
            .send()
            .await
            .map_err(map_send_error)?;
        let body = read_body(response).await?;
        model::user_detail_from_json(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `GET /event/stream?token=...`: the server-sent event connection.
    ///
    /// The token goes in the query because this client cannot put an
    /// `Authorization` header on an event source. The response is returned
    /// unread; the caller drives its chunks through [`crate::sse::Parser`].
    pub async fn open_stream(&self, token: &str) -> Result<reqwest::Response, ApiError> {
        let mut url = reqwest::Url::parse(&self.url("/event/stream"))
            .map_err(|e| ApiError::Unreachable(e.to_string()))?;
        // A JWT is URL-safe already, but let the URL builder do the encoding so
        // any future token shape cannot break the query string.
        url.query_pairs_mut().append_pair("token", token);
        let response = self
            .stream_http
            .get(url)
            .send()
            .await
            .map_err(map_send_error)?;
        if response.status().is_success() {
            return Ok(response);
        }
        // Only failures are read to completion; a live stream is not.
        let (status, body) = read_raw(response).await?;
        let message = server_message(&body);
        if status == StatusCode::UNAUTHORIZED {
            Err(ApiError::Unauthorized(message))
        } else {
            Err(ApiError::Status {
                status: status.as_u16(),
                message,
            })
        }
    }

    /// `GET /user/find/{name}`, a fuzzy match by name returning id+name pairs.
    /// The name is percent-encoded because it is a path segment and may contain
    /// spaces or non-ASCII characters.
    pub async fn find_users(&self, name: &str) -> Result<Vec<FoundUser>, ApiError> {
        let mut url = reqwest::Url::parse(&self.url("/user/find"))
            .map_err(|e| ApiError::Unreachable(e.to_string()))?;
        url.path_segments_mut()
            .map_err(|_| ApiError::Unreachable("无法构造搜索地址".to_string()))?
            .push(name);
        let response = self
            .authorized(self.http.get(url))
            .send()
            .await
            .map_err(map_send_error)?;
        let body = read_body(response).await?;
        model::found_users_from_json(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `GET /friend/req`, every inbound request; the state machine keeps the
    /// pending ones.
    pub async fn friend_requests(&self) -> Result<Vec<FriendRequest>, ApiError> {
        let response = self
            .authorized(self.http.get(self.url("/friend/req")))
            .send()
            .await
            .map_err(map_send_error)?;
        let body = read_body(response).await?;
        model::friend_requests_from_json(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `GET /friend`, the friend list.
    pub async fn friends(&self) -> Result<Vec<Friend>, ApiError> {
        let response = self
            .authorized(self.http.get(self.url("/friend")))
            .send()
            .await
            .map_err(map_send_error)?;
        let body = read_body(response).await?;
        model::friends_from_json(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// `POST /friend/req/{uid}`. 200 means the request was created; 201 carries
    /// "already a friend" or "already waiting" in its body and 403 is a refusal.
    /// All three are outcomes the status line explains, never crashes.
    pub async fn send_friend_request(
        &self,
        uid: i32,
        reason: Option<&str>,
    ) -> Result<SendOutcome, ApiError> {
        let response = self
            .authorized(
                self.http
                    .post(self.url(&format!("/friend/req/{uid}")))
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({ "reason": reason })),
            )
            .send()
            .await
            .map_err(map_send_error)?;
        let (status, body) = read_raw(response).await?;
        let message = server_message(&body);

        if status == StatusCode::FORBIDDEN {
            return Ok(SendOutcome::Forbidden(message));
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized(message));
        }
        if status.is_success() {
            if status == StatusCode::CREATED {
                return Ok(if message.contains("已经是好友") {
                    SendOutcome::AlreadyFriend
                } else if message.contains("等待") {
                    SendOutcome::AlreadyWaiting
                } else {
                    // An unknown 201 still means the request exists; do not turn
                    // it into a failure.
                    SendOutcome::Sent
                });
            }
            return Ok(SendOutcome::Sent);
        }
        Err(ApiError::Status {
            status: status.as_u16(),
            message,
        })
    }

    /// `POST /friend/req` with `{id, status}`. 403 means the caller is not the
    /// request's target and is surfaced as a readable outcome, not an error.
    pub async fn review_friend_request(
        &self,
        id: i32,
        decision: FriendDecision,
    ) -> Result<ReviewOutcome, ApiError> {
        let status = match decision {
            FriendDecision::Approve => "APPROVE",
            FriendDecision::Reject => "REJECT",
        };
        let response = self
            .authorized(
                self.http
                    .post(self.url("/friend/req"))
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({ "id": id, "status": status })),
            )
            .send()
            .await
            .map_err(map_send_error)?;
        let (status, body) = read_raw(response).await?;
        let message = server_message(&body);

        if status == StatusCode::FORBIDDEN {
            return Ok(ReviewOutcome::Forbidden(message));
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized(message));
        }
        if status.is_success() {
            return Ok(ReviewOutcome::Reviewed);
        }
        Err(ApiError::Status {
            status: status.as_u16(),
            message,
        })
    }

    /// Attach the in-memory token when there is one. New calls share this
    /// helper; the existing history call keeps its inline header.
    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(token) => request.header(AUTHORIZATION, format!("Bearer {token}")),
            None => request,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

fn map_send_error(error: reqwest::Error) -> ApiError {
    ApiError::Unreachable(error.to_string())
}

async fn read_body(response: reqwest::Response) -> Result<Vec<u8>, ApiError> {
    let status = response.status();
    let bytes = response.bytes().await.map_err(map_send_error)?;
    if status.is_success() {
        return Ok(bytes.to_vec());
    }
    let message = server_message(&bytes);
    if status == StatusCode::UNAUTHORIZED {
        Err(ApiError::Unauthorized(message))
    } else {
        Err(ApiError::Status {
            status: status.as_u16(),
            message,
        })
    }
}

/// Read a response without judging it: the friends endpoints attach meaning to
/// 201 and 403, so the caller needs both the status and the body.
async fn read_raw(response: reqwest::Response) -> Result<(StatusCode, Vec<u8>), ApiError> {
    let status = response.status();
    let bytes = response.bytes().await.map_err(map_send_error)?;
    Ok((status, bytes.to_vec()))
}

/// The server's error bodies are mostly bare text (`(StatusCode, err.to_string())`
/// in `src/err.rs`), but validator and JSON-rejection failures use `Json(String)`,
/// which is a quoted JSON string. Strip the wrapper when present, otherwise pass
/// the text through.
fn server_message(bytes: &[u8]) -> String {
    let text = std::str::from_utf8(bytes).unwrap_or_default().trim();
    match serde_json::from_str::<String>(text) {
        Ok(unquoted) => unquoted,
        Err(_) => text.to_string(),
    }
}
