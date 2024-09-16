use crate::app_state::AppState;
use crate::datetime::datetime_format;
use crate::err::ServerError;
use crate::event::BroadcastEvent;
use crate::group;
use chrono::{DateTime, Local};
use futures::{FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use utoipa::ToSchema;
use validator::Validate;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessagePayload {
    /// Sender id
    pub from_uid: i32,

    #[serde(with = "datetime_format")]
    /// The create time of the message.
    pub created_at: DateTime<Local>,

    /// Message target
    pub target: MessageTarget,

    /// Message detail
    pub detail: MessageDetail,
}

/// Send message request
#[derive(Deserialize, Validate, Debug, ToSchema)]
pub struct SendMsgReq {
    /// Message content
    #[validate(length(min = 1, code = "1", message = "msg is blank"))]
    pub msg: String,
}

impl SendMsgReq {
    pub fn build_payload(self, from_uid: i32, message_target: MessageTarget) -> ChatMessagePayload {
        ChatMessagePayload {
            from_uid,
            created_at: Local::now(),
            target: message_target,
            detail: MessageDetail::Normal(MessageNormal {
                content: MessageContent { content: self.msg },
            }),
        }
    }
}

impl ChatMessagePayload {}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum MessageTarget {
    User(MessageTargetUser),
    Group(MessageTargetGroup),
}

impl From<MessageTarget> for String {
    fn from(value: MessageTarget) -> Self {
        match value {
            MessageTarget::User(MessageTargetUser { uid }) => format!("MessageTargetUser:{uid}"),
            MessageTarget::Group(MessageTargetGroup { gid }) => {
                format!("MessageTargetGroup:{gid}")
            }
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub struct MessageTargetUser {
    pub uid: i32,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub struct MessageTargetGroup {
    pub gid: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MessageDetail {
    Normal(MessageNormal),
    Replay(MessageReplay),
}

impl MessageDetail {
    pub fn get_content(&self) -> String {
        match self {
            MessageDetail::Normal(msg) => msg.content.content.clone(),
            MessageDetail::Replay(msg) => msg.content.content.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageNormal {
    pub content: MessageContent,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageReplay {
    pub mid: i64,
    pub content: MessageContent,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageContent {
    /// Extended attributes
    // pub properties: Option<HashMap<String, Value>>,
    /// Content type
    // pub content_type: String,
    /// Content
    pub(crate) content: String,
}

/// Chat message
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ChatMessage {
    /// Message id
    pub mid: i64,
    pub payload: ChatMessagePayload,
}

impl ChatMessage {
    pub fn new(mid: i64, payload: ChatMessagePayload) -> Self {
        ChatMessage { mid, payload }
    }
}

pub(crate) async fn send_msg(
    payload: ChatMessagePayload,
    app_state: &AppState,
) -> Result<i64, ServerError> {
    let from_uid = payload.from_uid;
    let msg = serde_json::to_vec(&payload)
        .map_err(|_| ServerError::CustomErr("fail to serialize msg".to_string()))?;
    let mid = match payload.target {
        MessageTarget::User(MessageTargetUser { uid }) => {
            let mid = app_state.msg_db.lock().unwrap().messages().send_to_dm(
                from_uid as i64,
                uid as i64,
                &msg,
            )?;
            let _ = app_state.event_sender.send(Arc::new(BroadcastEvent::Chat {
                targets: BTreeSet::from([from_uid, uid]),
                message: ChatMessage::new(mid, payload),
            }));
            mid
        }
        MessageTarget::Group(MessageTargetGroup { gid }) => {
            let uids = group::get_uids(&app_state, gid).await?;
            let mid = app_state.msg_db.lock().unwrap().messages().send_to_group(
                gid as i64,
                uids.iter().map(|&x| i64::from(x)).collect::<Vec<i64>>(),
                &msg,
            )?;
            let _ = app_state.event_sender.send(Arc::new(BroadcastEvent::Chat {
                targets: uids.into_iter().collect(),
                message: ChatMessage::new(mid, payload),
            }));
            mid
        }
    };
    Ok(mid)
}

pub enum HistoryMsgReq {
    User(HistoryMsgUser),
    Group(HistoryMsgGroup),
}

pub struct HistoryReq {
    pub(crate) before: Option<i64>,
    pub(crate) limit: usize,
}

pub struct HistoryMsgUser {
    pub(crate) from_id: i32,
    pub(crate) to_id: i32,
    pub(crate) history: HistoryReq,
}

pub struct HistoryMsgGroup {
    pub(crate) gid: i32,
    pub(crate) history: HistoryReq,
}

pub(crate) fn get_history_msg(
    app_state: &AppState,
    history_msg_req: HistoryMsgReq,
) -> Vec<ChatMessage> {
    match history_msg_req {
        HistoryMsgReq::User(HistoryMsgUser {
            from_id,
            to_id,
            history: HistoryReq { before, limit },
        }) => {
            let result = app_state
                .msg_db
                .lock()
                .unwrap()
                .messages()
                .fetch_dm_messages_before(from_id as i64, to_id as i64, before, limit)
                .ok();
            match result {
                Some(msgs) => build_chat_messages(msgs),
                None => vec![],
            }
        }
        HistoryMsgReq::Group(HistoryMsgGroup {
            gid,
            history: HistoryReq { before, limit },
        }) => {
            let result = app_state
                .msg_db
                .lock()
                .unwrap()
                .messages()
                .fetch_group_messages_before(gid as i64, before, limit)
                .ok();
            match result {
                Some(msgs) => build_chat_messages(msgs),
                None => vec![],
            }
        }
    }
}

fn build_chat_messages(msgs: Vec<(i64, Vec<u8>)>) -> Vec<ChatMessage> {
    msgs.into_iter()
        .filter_map(|(mid, msg)| build_chat_message(mid, msg))
        .collect()
}

fn build_chat_message(mid: i64, msg: Vec<u8>) -> Option<ChatMessage> {
    serde_json::from_slice::<ChatMessagePayload>(&msg)
        .ok()
        .map(|c| ChatMessage::new(mid, c))
}

pub(crate) fn get_by_mids(mids: Vec<i64>, app_state: &AppState) -> Vec<ChatMessage> {
    mids.into_iter()
        .filter_map(|mid| {
            app_state
                .msg_db
                .lock()
                .unwrap()
                .messages()
                .get(mid)
                .ok()
                .flatten()
                .map(|msg| (mid, msg))
        })
        .filter_map(|(mid, msg)| build_chat_message(mid, msg))
        .collect()
}

/// 查询群未读消息数量
pub(crate) fn count_group_unread(
    gid: i32,
    mid: Option<i64>,
    app_state: &AppState,
) -> Option<UnRead> {
    match mid {
        None => Some(UnRead::ALL),
        Some(mid) => {
            match app_state
                .msg_db
                .lock()
                .unwrap()
                .messages()
                .count_group_messages_after(gid as i64, mid)
            {
                Ok(count) if count > 0 => Some(UnRead::Part(count)),
                _ => None,
            }
        }
    }
}

pub(crate) enum UnRead {
    ALL,
    Part(usize),
}

// 为UnRead实现Display
impl fmt::Display for UnRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnRead::ALL => write!(f, "all"),
            UnRead::Part(count) => write!(f, "{}", count),
        }
    }
}

/// 查询单聊未读消息数量
pub(crate) fn count_dm_unread(
    from_uid: i32,
    to_uid: i32,
    mid: Option<i64>,
    app_state: &AppState,
) -> Option<UnRead> {
    match mid {
        None => Some(UnRead::ALL),
        Some(mid) => match app_state
            .msg_db
            .lock()
            .unwrap()
            .messages()
            .count_dm_messages_after(from_uid as i64, to_uid as i64, mid)
        {
            Ok(count) if count > 0 => Some(UnRead::Part(count)),
            _ => None,
        },
    }
}
