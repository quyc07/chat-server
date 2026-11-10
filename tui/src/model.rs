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
    #[error("invalid access token: {0}")]
    Token(String),
    #[error("invalid event-stream payload: {0}")]
    Stream(String),
    #[error("invalid message id in the send response: {0}")]
    SendId(String),
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

/// One historical message in the open conversation, oldest first.
///
/// The direct-message history endpoint sends this concept flat
/// (`mid`, `msg`, `time`, `from_uid`, `from_name`), unlike the conversation
/// list's nested enum. This module is the only place that knows either shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub mid: i64,
    pub from_uid: i32,
    pub from_name: String,
    pub body: String,
    /// Local wall-clock time, as the server sent it.
    pub time: NaiveDateTime,
}

impl Message {
    /// Decide how the bubble is presented. Rendering only paints this.
    ///
    /// Authorship comes from the immutable sender id, never from the display
    /// name: usernames are not unique on this server, so comparing names would
    /// label a same-named peer's messages as our own.
    pub fn view(&self, me_uid: Option<i32>, now: NaiveDateTime) -> MessageView {
        MessageView {
            sender: self.from_name.clone(),
            mine: me_uid.is_some_and(|uid| uid == self.from_uid),
            time_label: time_label(self.time, now),
            body: self.body.clone(),
            failed: false,
        }
    }
}

/// Everything the render layer needs to draw one message, decided here so it
/// can be asserted without a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageView {
    pub sender: String,
    /// True when the logged-in user sent it.
    pub mine: bool,
    pub time_label: String,
    pub body: String,
    /// True for an optimistic row whose send failed. Rendering only colours it.
    pub failed: bool,
}

/// The server's `UserHistoryMsg`, field-for-field.
#[derive(Debug, Deserialize)]
struct WireUserHistoryMsg {
    mid: i64,
    msg: String,
    time: String,
    from_uid: i32,
    from_name: String,
}

/// Translate the `GET /user/{uid}/history` body into domain messages.
///
/// The endpoint returns at most 1000 rows in chronological order and takes no
/// pagination parameters.
pub fn messages_from_json(body: &[u8]) -> Result<Vec<Message>, WireError> {
    let rows: Vec<WireUserHistoryMsg> =
        serde_json::from_slice(body).map_err(|e| WireError::Json(e.to_string()))?;
    rows.into_iter()
        .map(|row| {
            Ok(Message {
                mid: row.mid,
                from_uid: row.from_uid,
                from_name: row.from_name,
                body: row.msg,
                time: parse_wire_time(&row.time)?,
            })
        })
        .collect()
}

/// Translate the `PUT /user/{uid}/send` body into the new message id.
///
/// The server's handler is `Res<String>`, and the id is quoted on the wire
/// (`"42"`), not a JSON number. A bare numeric body is accepted as well: the same
/// handler renders as plain text when axum's default `String` conversion applies,
/// and both shapes carry one and the same id. This is the only place that knows.
pub fn sent_mid_from_json(body: &[u8]) -> Result<i64, WireError> {
    let raw = std::str::from_utf8(body)
        .map_err(|e| WireError::SendId(e.to_string()))?
        .trim();
    if raw.is_empty() {
        return Err(WireError::SendId("empty body".to_string()));
    }
    // `"42"` parses as a String; a bare `42` falls through to the text branch.
    let id = match serde_json::from_str::<String>(raw) {
        Ok(unquoted) => unquoted,
        Err(_) => raw.to_string(),
    };
    id.trim()
        .parse::<i64>()
        .map_err(|e| WireError::SendId(format!("{id:?}: {e}")))
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

/// The server never returns the caller's uid at login, but the JWT payload
/// carries it as `id` (`src/auth.rs::Token`). The payload is the middle segment
/// of the token and is base64url without padding; the signature is not verified
/// here because the server is the one that issued it to this process.
pub fn uid_from_token(token: &str) -> Result<i32, WireError> {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    #[derive(Deserialize)]
    struct Claims {
        id: i32,
    }

    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| WireError::Token("token has no payload segment".to_string()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| WireError::Token(e.to_string()))?;
    let claims: Claims =
        serde_json::from_slice(&bytes).map_err(|e| WireError::Token(e.to_string()))?;
    Ok(claims.id)
}

/// A chat message as it arrives on the event stream.
///
/// The push shape is not the history shape: the stream nests the text three
/// levels deep (`payload.detail.Normal.content.content`) and carries the target
/// instead of naming the conversation. It also has no sender name, only
/// `from_uid`, so the state machine attaches the name from the conversation row
/// (or asks `/user/detail/{uid}` for a peer it has never seen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChat {
    pub mid: i64,
    pub from_uid: i32,
    /// The user or group the sender addressed.
    pub target: ConversationId,
    pub body: String,
    /// Local wall-clock time, as the server sent it.
    pub time: NaiveDateTime,
}

impl StreamChat {
    /// True when this is the echo of a message this client sent.
    pub fn is_mine(&self, me: Option<i32>) -> bool {
        me.is_some_and(|uid| uid == self.from_uid)
    }

    /// The conversation the message belongs to, from this client's point of
    /// view. A message I sent belongs to whoever I addressed; an incoming group
    /// message belongs to the group, not to the sender's direct conversation.
    pub fn conversation(&self, me: Option<i32>) -> ConversationId {
        match self.target {
            ConversationId::Group(_) => self.target,
            ConversationId::User(target) if self.is_mine(me) => ConversationId::User(target),
            ConversationId::User(_) => ConversationId::User(self.from_uid),
        }
    }

    /// Turn the event into the one message type the rest of the client knows.
    pub fn into_message(self, from_name: String) -> Message {
        Message {
            mid: self.mid,
            from_uid: self.from_uid,
            from_name,
            body: self.body,
            time: self.time,
        }
    }
}

/// What one named event from the stream means to this client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Chat(StreamChat),
    /// Liveness only. Never a message.
    Heartbeat,
}

/// The server's `ChatMessage`, field-for-field (`src/message.rs`).
#[derive(Debug, Deserialize)]
struct WireChatMessage {
    mid: i64,
    payload: WireChatPayload,
}

#[derive(Debug, Deserialize)]
struct WireChatPayload {
    from_uid: i32,
    created_at: String,
    target: WireMessageTarget,
    detail: WireMessageDetail,
}

/// `MessageTarget` is externally tagged: `{"User":{"uid":2}}`.
#[derive(Debug, Deserialize)]
enum WireMessageTarget {
    User { uid: i32 },
    Group { gid: i32 },
}

/// `MessageDetail` is externally tagged too. `Replay` is never constructed by
/// the current server, but accepting it costs nothing and keeps an unknown
/// message from killing the stream.
#[derive(Debug, Deserialize)]
enum WireMessageDetail {
    Normal(WireMessageNormal),
    Replay { content: WireMessageContent },
}

#[derive(Debug, Deserialize)]
struct WireMessageNormal {
    content: WireMessageContent,
}

#[derive(Debug, Deserialize)]
struct WireMessageContent {
    content: String,
}

/// Translate one dispatched stream frame. Unknown event names return `Ok(None)`
/// so a server-side addition is ignored instead of treated as a failure.
pub fn stream_event(event: &str, data: &str) -> Result<Option<StreamEvent>, WireError> {
    match event {
        "Chat" => {
            let wire: WireChatMessage =
                serde_json::from_str(data).map_err(|e| WireError::Stream(format!("Chat: {e}")))?;
            Ok(Some(StreamEvent::Chat(wire.into_domain()?)))
        }
        "Heartbeat" => Ok(Some(StreamEvent::Heartbeat)),
        _ => Ok(None),
    }
}

impl WireChatMessage {
    fn into_domain(self) -> Result<StreamChat, WireError> {
        let WireChatPayload {
            from_uid,
            created_at,
            target,
            detail,
        } = self.payload;
        let body = match detail {
            WireMessageDetail::Normal(normal) => normal.content.content,
            WireMessageDetail::Replay { content } => content.content,
        };
        let target = match target {
            WireMessageTarget::User { uid } => ConversationId::User(uid),
            WireMessageTarget::Group { gid } => ConversationId::Group(gid),
        };
        Ok(StreamChat {
            mid: self.mid,
            from_uid,
            target,
            body,
            time: parse_wire_time(&created_at)?,
        })
    }
}

/// Translate `GET /user/detail/{uid}` (`src/user.rs::UserDetail`). Only id and
/// name matter here; the rest of the payload is deliberately ignored.
pub fn user_detail_from_json(body: &[u8]) -> Result<FoundUser, WireError> {
    let row: WireIdName =
        serde_json::from_slice(body).map_err(|e| WireError::FriendJson(e.to_string()))?;
    Ok(FoundUser {
        id: row.id,
        name: row.name,
    })
}
