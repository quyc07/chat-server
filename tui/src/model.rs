//! Conversation domain model and the one place wire JSON is translated.
//!
//! The server's conversation list is an externally tagged enum:
//! `{"User": {...}}` or `{"Group": {...}}`. Every timestamp on the wire is a
//! local-time string without an offset, formatted `%Y-%m-%d %H:%M:%S`.
//!
//! Nothing outside this module knows those shapes.

use chrono::{NaiveDate, NaiveDateTime};
use serde::Deserialize;

pub const WIRE_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// How many characters of the last message the list keeps before eliding.
pub const PREVIEW_CHARS: usize = 120;

/// Stable identity of a conversation, used for de-duplicating merged pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationId {
    User(i32),
    Group(i32),
}

/// Unread state of one conversation, translated from the server's
/// `Option<String>` (`null`, `"all"`, or a decimal count).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unread {
    None,
    Count(u32),
    All,
}

impl Unread {
    /// The badge shown in the conversation list.
    ///
    /// `All` deliberately renders as a shape, never as a digit, so "never read"
    /// is visually distinct from "one unread".
    pub fn badge(self) -> Option<String> {
        match self {
            Unread::None => None,
            Unread::Count(0) => None,
            Unread::Count(n) => Some(n.to_string()),
            Unread::All => Some("●".to_string()),
        }
    }
}

/// One row of the conversation list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub id: ConversationId,
    /// Peer name for a direct conversation, group name for a group one.
    pub title: String,
    /// Last message id. Message ids are a global monotonic sequence.
    pub mid: i64,
    /// Single-line last-message excerpt.
    pub preview: String,
    /// Local wall-clock time of the last message, as the server sent it.
    pub time: NaiveDateTime,
    pub unread: Unread,
    /// For a group conversation, who sent the last message. `None` otherwise.
    pub last_sender: Option<String>,
}

impl Conversation {
    pub fn id(&self) -> ConversationId {
        self.id
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("invalid conversation-list JSON: {0}")]
    Json(String),
    #[error("invalid friend JSON: {0}")]
    FriendJson(String),
    #[error("invalid server timestamp {0:?}: {1}")]
    Time(String, String),
}

/// The server's `ChatVo`, field-for-field. The `DateTime<Local>` fields on the
/// server use its `%Y-%m-%d %H:%M:%S` serializer, so they arrive as strings.
#[derive(Debug, Deserialize)]
enum WireChatVo {
    User {
        uid: i32,
        user_name: String,
        mid: i64,
        msg: String,
        msg_time: String,
        #[serde(default)]
        unread: Option<String>,
    },
    Group {
        gid: i32,
        group_name: String,
        #[allow(dead_code)] // present on the wire; the UI does not need it
        uid: i32,
        user_name: String,
        mid: i64,
        msg: String,
        msg_time: String,
        #[serde(default)]
        unread: Option<String>,
    },
}

impl WireChatVo {
    fn into_domain(self) -> Result<Conversation, WireError> {
        match self {
            WireChatVo::User {
                uid,
                user_name,
                mid,
                msg,
                msg_time,
                unread,
            } => Ok(Conversation {
                id: ConversationId::User(uid),
                title: user_name,
                mid,
                preview: excerpt(&msg),
                time: parse_wire_time(&msg_time)?,
                unread: parse_unread(unread.as_deref()),
                last_sender: None,
            }),
            WireChatVo::Group {
                gid,
                group_name,
                uid: _,
                user_name,
                mid,
                msg,
                msg_time,
                unread,
            } => Ok(Conversation {
                id: ConversationId::Group(gid),
                title: group_name,
                mid,
                preview: excerpt(&msg),
                time: parse_wire_time(&msg_time)?,
                unread: parse_unread(unread.as_deref()),
                last_sender: Some(user_name),
            }),
        }
    }
}

/// Translate the `POST /user/history` body into domain conversations.
///
/// The rows are returned in the server's order; ordering by recency is the state
/// machine's job (it also re-sorts each page defensively).
pub fn conversations_from_json(body: &[u8]) -> Result<Vec<Conversation>, WireError> {
    let rows: Vec<WireChatVo> =
        serde_json::from_slice(body).map_err(|e| WireError::Json(e.to_string()))?;
    rows.into_iter().map(WireChatVo::into_domain).collect()
}

/// The server's login response. It sends more fields (`access_token_expires`);
/// unknown fields are ignored on purpose.
#[derive(Debug, Deserialize)]
pub struct LoginWire {
    pub access_token: String,
}

pub fn login_from_json(body: &[u8]) -> Result<LoginWire, serde_json::Error> {
    serde_json::from_slice(body)
}

fn parse_unread(raw: Option<&str>) -> Unread {
    match raw {
        None => Unread::None,
        Some("all") => Unread::All,
        Some(text) => text
            .parse::<u32>()
            .map(Unread::Count)
            .unwrap_or(Unread::None),
    }
}

fn parse_wire_time(raw: &str) -> Result<NaiveDateTime, WireError> {
    NaiveDateTime::parse_from_str(raw, WIRE_TIME_FORMAT)
        .map_err(|e| WireError::Time(raw.to_string(), e.to_string()))
}

/// First line, at most [`PREVIEW_CHARS`] characters, with an ellipsis when cut.
pub fn excerpt(msg: &str) -> String {
    let first_line = msg.lines().next().unwrap_or("");
    let mut chars = first_line.chars();
    let head: String = chars.by_ref().take(PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Output for a list timestamp: same-day messages show the clock time, older
/// ones also show the date. No time-zone conversion: the server sends local time.
pub fn time_label(time: NaiveDateTime, now: NaiveDateTime) -> String {
    if time.date() == now.date() {
        time.format("%H:%M").to_string()
    } else {
        time.format("%m-%d %H:%M").to_string()
    }
}

/// A date at midnight, handy for tests and fixtures.
pub fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
}

/// One user row from `GET /user/find/{name}`: the server returns id and name
/// only, which is all a friend request needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundUser {
    pub id: i32,
    pub name: String,
}

/// One row of `GET /friend`: id and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friend {
    pub id: i32,
    pub name: String,
}

/// Incoming friend-request state, translated from the server's uppercase
/// `WAIT` / `APPROVE` / `REJECT` strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendRequestStatus {
    Wait,
    Approve,
    Reject,
}

/// One incoming friend request from `GET /friend/req`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendRequest {
    pub id: i32,
    pub request_id: i32,
    pub request_name: String,
    pub create_time: NaiveDateTime,
    pub reason: Option<String>,
    pub status: FriendRequestStatus,
}

/// What the user decided about an incoming request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendDecision {
    Approve,
    Reject,
}

/// Meaning of `POST /friend/req/{uid}`.
///
/// The server reports "already a friend" and "request already waiting" as **201**
/// with the reason in the body, so this is deliberately not a `Result<(), _>`:
/// those are outcomes, not transport failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    AlreadyFriend,
    AlreadyWaiting,
    /// The server refused with 403 and a readable reason.
    Forbidden(String),
}

/// Meaning of `POST /friend/req` (approve/reject).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewOutcome {
    Reviewed,
    /// 403: this user is not the request's target.
    Forbidden(String),
}

#[derive(Debug, Deserialize)]
struct WireIdName {
    id: i32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct WireFriendRequest {
    id: i32,
    request_id: i32,
    request_name: String,
    create_time: String,
    #[serde(default)]
    reason: Option<String>,
    status: String,
}

/// Translate the `GET /user/find/{name}` body into id/name pairs.
pub fn found_users_from_json(body: &[u8]) -> Result<Vec<FoundUser>, WireError> {
    let rows: Vec<WireIdName> =
        serde_json::from_slice(body).map_err(|e| WireError::FriendJson(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|row| FoundUser {
            id: row.id,
            name: row.name,
        })
        .collect())
}

/// Translate the `GET /friend` body into domain friends.
pub fn friends_from_json(body: &[u8]) -> Result<Vec<Friend>, WireError> {
    let rows: Vec<WireIdName> =
        serde_json::from_slice(body).map_err(|e| WireError::FriendJson(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|row| Friend {
            id: row.id,
            name: row.name,
        })
        .collect())
}

/// Translate the `GET /friend/req` body. The server sends every inbound request,
/// whatever its status; keeping the status lets the state machine decide what is
/// still pending.
pub fn friend_requests_from_json(body: &[u8]) -> Result<Vec<FriendRequest>, WireError> {
    let rows: Vec<WireFriendRequest> =
        serde_json::from_slice(body).map_err(|e| WireError::FriendJson(e.to_string()))?;
    rows.into_iter()
        .map(|row| {
            Ok(FriendRequest {
                id: row.id,
                request_id: row.request_id,
                request_name: row.request_name,
                create_time: parse_wire_time(&row.create_time)?,
                reason: row.reason,
                status: parse_request_status(&row.status),
            })
        })
        .collect()
}

/// Unknown upper-case values (a server-side addition we have not seen yet) are
/// treated as pending: showing a request is safer than hiding it.
fn parse_request_status(raw: &str) -> FriendRequestStatus {
    match raw {
        "APPROVE" => FriendRequestStatus::Approve,
        "REJECT" => FriendRequestStatus::Reject,
        _ => FriendRequestStatus::Wait,
    }
}
