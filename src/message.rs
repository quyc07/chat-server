use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::app_state::AppState;
use crate::err::ServerError;
use crate::event::BroadcastEvent;
use crate::group;

// #[derive(Error, ToSchema)]
// pub enum MsgErr {
//     #[error("用户名 {0} 已存在")]
//     FailToSendDM(ChatMessagePayload),
// }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessagePayload {
    /// Sender id
    pub from_uid: i32,

    /// The create time of the message.
    pub created_at: DateTime<Local>,

    /// Message target
    pub target: MessageTarget,

    /// Message detail
    pub detail: MessageDetail,
}

#[derive(Deserialize, Validate, Debug)]
pub struct SendMsgReq {
    #[validate(length(min = 1, code = "1", message = "msg is blank"))]
    pub msg: String,
}

impl SendMsgReq {
    pub fn build_payload(self, from_id: i32, message_target: MessageTarget) -> ChatMessagePayload {
        ChatMessagePayload {
            from_uid: from_id,
            created_at: Local::now(),
            target: message_target,
            detail: MessageDetail::Normal(MessageNormal {
                content: MessageContent { content: self.msg },
            }),
        }
    }
}

impl ChatMessagePayload {}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum MessageTarget {
    User(MessageTargetUser),
    Group(MessageTargetGroup),
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct MessageTargetUser {
    pub uid: i32,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct MessageTargetGroup {
    pub gid: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MessageDetail {
    Normal(MessageNormal),
    Replay(MessageReplay),
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
    app_state: AppState,
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
