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

use crate::model::{self, Conversation, LoginWire};

/// How long a request may take before the UI reports the server unreachable.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The server could not be reached at all (connect error, timeout, DNS...).
    Unreachable(String),
    /// 401: credentials were rejected or the token is no longer valid.
    Unauthorized(String),
    /// Any other non-success status. `message` is the server's own text.
    Status { status: u16, message: String },
    /// The body was not the JSON the server documents.
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
        Self {
            http,
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

/// The server's error bodies are either a JSON string (`Json(err.to_string())`,
/// e.g. `"用户名或密码错误"`) or a bare text body for generic 500s. Strip the
/// JSON string wrapper when present, otherwise pass the text through.
fn server_message(bytes: &[u8]) -> String {
    let text = std::str::from_utf8(bytes).unwrap_or_default().trim();
    match serde_json::from_str::<String>(text) {
        Ok(unquoted) => unquoted,
        Err(_) => text.to_string(),
    }
}
