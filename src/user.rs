use std::collections::HashMap;
use std::option::Option;

use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::Router;
use chrono::{DateTime, Local};
use itertools::Itertools;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DbErr, EntityTrait, IntoActiveModel, QueryFilter,
    QuerySelect,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;
use utoipa::{OpenApi, ToSchema};
use validator::Validate;

use crate::app_state::AppState;
use crate::auth::Token;
use crate::datetime::datetime_format;
use crate::datetime::opt_datetime_format;
use crate::err::{ErrPrint, ServerError};
use crate::friend::FriendRegister;
use crate::message::{
    ChatMessage, HistoryMsgReq, HistoryMsgUser, HistoryReq, MessageTarget, MessageTargetUser,
    SendMsgReq,
};
use crate::read_index;
use crate::read_index::UpdateReadIndex;
use crate::validate::ValidatedJson;
use crate::{auth, datetime, friend, group, message, AppRes, Res};
use entity::prelude::User;
use entity::user;

#[derive(OpenApi)]
#[openapi(
    paths(
        register
    ),
    components(
        schemas(UserRegisterReq, UserRes)
    ),
    tags(
        (name = "user", description = "USER API")
    )
)]
pub struct UserApi;

impl UserApi {
    pub fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/register", post(register))
            .route("/all", get(all))
            .route("/:uid/send", post(send))
            .route("/:uid/history", get(user_history))
            .route("/history/:limit", get(history))
            .route("/password", patch(password))
            .with_state(app_state)
    }
}

/// Register New User
#[derive(Debug, Deserialize, Validate, ToSchema)]
struct UserRegisterReq {
    /// name
    #[validate(length(min = 1))]
    name: String,
    /// email
    #[validate(email)]
    email: String,
    /// password
    #[validate(length(min = 1))]
    password: String,
    /// phone
    phone: Option<String>,
}

/// User error
#[derive(Debug, Error, ToSchema)]
pub enum UserErr {
    /// UserName already exists
    #[error("用户名 {0} 已存在")]
    UserNameExist(String),
    /// User not exist
    #[error("用户{0}不存在")]
    UserNotExist(i32),
}

impl ErrPrint for UserErr {}

async fn all(State(app_state): State<AppState>, _: Token) -> Res<Vec<UserRes>> {
    let result = User::find().all(&app_state.db).await;
    let model = result?;
    Ok(AppRes::success(
        model.into_iter().map(UserRes::from).collect(),
    ))
}

/// Register User.
///
/// Register User and return the User.
#[utoipa::path(
    post,
    path = "/user/register",
    request_body = UserRegisterReq,
    responses(
        (status = 200, description = "Register User and return the User successfully", body = [UserRes]),
        (status = 409, description = "UserName already exists", body = [ServerError])
    )
)]
async fn register(
    State(app_state): State<AppState>,
    ValidatedJson(req): ValidatedJson<UserRegisterReq>,
) -> Res<UserRes> {
    let name = req.name.as_str();
    if find_by_name(&app_state, name).await?.is_some() {
        return Err(ServerError::from(UserErr::UserNameExist(name.to_string())));
    }
    // save db
    let mut user = user::ActiveModel {
        id: Default::default(),
        name: Set(req.name.clone()),
        password: Set(req.password),
        email: Set(req.email),
        phone: Set(req.phone.clone()),
        create_time: Default::default(),
        update_time: Default::default(),
        status: ActiveValue::NotSet,
        dgraph_uid: Default::default(),
    };
    let user = user.insert(&app_state.db).await?;
    // save dgraph, get dgraph_uid
    let dgraph_uid = friend::register(FriendRegister {
        user_id: user.id,
        name: req.name,
        phone: req.phone,
    })
    .await?;
    let mut user = user.into_active_model();
    user.dgraph_uid = Set(dgraph_uid);
    let user = user.update(&app_state.db).await?;
    Ok(AppRes::success(UserRes::from(user)))
}

/// The new user.
#[derive(Serialize, Deserialize, ToSchema)]
struct UserRes {
    pub id: i32,
    #[schema(example = "User Name")]
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub password: String,
    #[serde(with = "datetime_format")]
    pub create_time: DateTime<Local>,
    #[serde(with = "opt_datetime_format")]
    pub update_time: Option<DateTime<Local>>,
    pub status: String,
    pub dgraph_uid: String,
}

impl From<user::Model> for UserRes {
    fn from(value: user::Model) -> Self {
        Self {
            id: value.id,
            name: value.name,
            email: value.email,
            phone: value.phone,
            password: value.password,
            create_time: datetime::native_datetime_2_datetime(value.create_time),
            update_time: value
                .update_time
                .map(|t| datetime::native_datetime_2_datetime(t)),
            status: value.status.into(),
            dgraph_uid: value.dgraph_uid,
        }
    }
}

// 按照参数定义的先后顺序进行解析，ValidatedJson会消耗掉Request，因此要放在最后面解析
async fn send(
    State(app_state): State<AppState>,
    Path(uid): Path<i32>,
    token: Token,
    ValidatedJson(msg): ValidatedJson<SendMsgReq>,
) -> Res<i64> {
    // 判断是否是好友
    if !friend::is_friend(token.dgraph_uid, uid).await {
        return Err(ServerError::from(friend::FriendErr::NotFriend(uid)));
    }
    let payload = msg.build_payload(token.id, MessageTarget::User(MessageTargetUser { uid }));
    let mid = message::send_msg(payload, &app_state).await?;
    // 设置read_index
    read_index::set_read_index(
        &app_state,
        token.id,
        UpdateReadIndex::User {
            uid,
            mid,
            uid_of_msg: token.id,
        },
    )
    .await?;
    Ok(AppRes::success(mid))
}

async fn user_history(
    State(app_state): State<AppState>,
    Path(uid): Path<i32>,
    token: Token,
) -> Res<Vec<ChatMessage>> {
    if !friend::is_friend(token.dgraph_uid, uid).await {
        return Err(ServerError::from(friend::FriendErr::NotFriend(uid)));
    }
    let mut history_msg = message::get_history_msg(
        &app_state,
        HistoryMsgReq::User(HistoryMsgUser {
            from_id: token.id,
            to_id: uid,
            history: HistoryReq {
                before: None,
                limit: 1000,
            },
        }),
    );
    history_msg.sort_by(|m1, m2| m2.payload.created_at.cmp(&m1.payload.created_at));
    Ok(AppRes::success(history_msg))
}

#[derive(Hash, Clone, PartialEq, Eq)]
enum ChatTarget {
    User,
    Group,
}

#[derive(Debug, Serialize, Hash, Eq, PartialEq)]
enum ChatListVo {
    User {
        uid: i32,
        user_name: String,
        mid: i64,
        msg: String,
        #[serde(with = "datetime_format")]
        msg_time: DateTime<Local>,
    },
    Group {
        gid: i32,
        group_name: String,
        uid: i32,
        user_name: String,
        mid: i64,
        msg: String,
        #[serde(with = "datetime_format")]
        msg_time: DateTime<Local>,
    },
}

impl ChatListVo {
    fn get_msg_time(&self) -> &DateTime<Local> {
        match self {
            ChatListVo::User { msg_time, .. } => msg_time,
            ChatListVo::Group { msg_time, .. } => msg_time,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatList {
    chat_list: Vec<ChatListVo>,
}
/// 查询用户最近聊天列表
async fn history(
    State(app_state): State<AppState>,
    Path(limit): Path<u64>,
    token: Token,
) -> Res<ChatList> {
    let ris = entity::read_index::Entity::find()
        .filter(entity::read_index::Column::Uid.eq(token.id))
        .limit(limit)
        .all(&app_state.db)
        .await?;
    let map = ris
        .into_iter()
        .filter_map(|x| match (x.target_uid, x.target_gid) {
            (Some(_), None) => Some((ChatTarget::User, x)),
            (None, Some(_)) => Some((ChatTarget::Group, x)),
            _ => None,
        })
        .into_iter()
        .into_group_map_by(|(t, m)| t.clone())
        .into_iter()
        .map(|(target, x)| {
            (
                target,
                x.into_iter()
                    .map(|(_, g)| g)
                    .collect::<Vec<entity::read_index::Model>>(),
            )
        })
        .collect::<HashMap<ChatTarget, Vec<entity::read_index::Model>>>();

    let chat_of_user = match map.get(&ChatTarget::User) {
        Some(ri_of_users) => {
            let (uids, mids) = ri_of_users
                .iter()
                .map(|x| (x.target_uid.unwrap(), x.mid))
                .collect::<(Vec<i32>, Vec<i64>)>();
            let uid_2_name = get_by_ids(uids, &app_state)
                .await?
                .into_iter()
                .map(|x| (x.id, x.name))
                .collect::<HashMap<i32, String>>();
            let mid_2_msg = message::get_by_mids(mids, &app_state)
                .into_iter()
                .map(|x| (x.mid, x))
                .collect::<HashMap<i64, ChatMessage>>();
            ri_of_users
                .into_iter()
                .map(|x| ChatListVo::User {
                    uid: x.target_uid.unwrap(),
                    user_name: uid_2_name
                        .get(&x.target_uid.unwrap())
                        .unwrap_or(&String::from("未知用户"))
                        .to_string(),
                    mid: x.mid,
                    msg: mid_2_msg
                        .get(&x.mid)
                        .map(|x| x.payload.detail.get_content())
                        .unwrap_or(String::from("")),
                    msg_time: mid_2_msg
                        .get(&x.mid)
                        .map(|x| x.payload.created_at)
                        .unwrap_or(Local::now()),
                })
                .collect()
        }
        None => vec![],
    };
    let chat_of_group = match map.get(&ChatTarget::Group) {
        None => vec![],
        Some(ris_of_group) => {
            let (uids, mids) = ris_of_group
                .iter()
                .map(|x| ((x.target_gid.unwrap(), x.uid_of_msg), x.mid))
                .collect::<(Vec<(i32, i32)>, Vec<i64>)>();
            let gids = ris_of_group
                .iter()
                .map(|x| x.target_gid.unwrap())
                .collect::<Vec<i32>>();
            let uids = uids.into_iter().fold(vec![], |mut x, (u1, u2)| {
                x.into_iter().chain(vec![u1, u2]).collect()
            });
            let uid_2_name = get_by_ids(uids, &app_state)
                .await?
                .into_iter()
                .map(|x| (x.id, x.name))
                .collect::<HashMap<i32, String>>();
            let mid_2_msg = message::get_by_mids(mids, &app_state)
                .into_iter()
                .map(|x| (x.mid, x))
                .collect::<HashMap<i64, ChatMessage>>();
            let gid_2_name = group::get_by_gids(gids, &app_state)
                .await?
                .into_iter()
                .map(|x| (x.id, x.name))
                .collect::<HashMap<i32, String>>();
            ris_of_group
                .into_iter()
                .map(|x| ChatListVo::Group {
                    gid: x.target_gid.unwrap(),
                    group_name: gid_2_name
                        .get(&x.target_gid.unwrap())
                        .unwrap_or(&String::from("未知群聊"))
                        .to_string(),
                    uid: x.uid_of_msg,
                    user_name: uid_2_name
                        .get(&x.uid_of_msg)
                        .unwrap_or(&String::from("未知用户"))
                        .to_string(),
                    mid: x.mid,
                    msg: mid_2_msg
                        .get(&x.mid)
                        .map(|x| x.payload.detail.get_content())
                        .unwrap_or(String::from("")),
                    msg_time: mid_2_msg
                        .get(&x.mid)
                        .map(|x| x.payload.created_at)
                        .unwrap_or(Local::now()),
                })
                .collect()
        }
    };
    let mut vec = chat_of_user
        .into_iter()
        .chain(chat_of_group)
        .collect::<Vec<ChatListVo>>();
    vec.sort_by(|x1, x2| x2.get_msg_time().cmp(&x1.get_msg_time()));
    Ok(AppRes::success(ChatList { chat_list: vec }))
}

pub async fn find_by_name(app_state: &AppState, name: &str) -> Result<Option<user::Model>, DbErr> {
    User::find()
        .filter(user::Column::Name.eq(name))
        .one(&app_state.db)
        .await
}

pub async fn exist(uid: i32, app_state: &AppState) -> Result<bool, DbErr> {
    User::find()
        .filter(user::Column::Id.eq(uid))
        .one(&app_state.db)
        .await
        .map(|t| t.is_some())
}

pub async fn get_by_ids(uids: Vec<i32>, app_state: &AppState) -> Result<Vec<user::Model>, DbErr> {
    User::find()
        .filter(user::Column::Id.is_in(uids))
        .all(&app_state.db)
        .await
}

pub async fn get_by_id(uid: i32, app_state: &AppState) -> Result<Option<user::Model>, DbErr> {
    User::find()
        .filter(user::Column::Id.eq(uid))
        .one(&app_state.db)
        .await
}

#[derive(Deserialize, ToSchema, Validate)]
struct PasswordReq {
    #[validate(length(min = 1, message = "password is blank"))]
    password: String,
}
async fn password(
    State(app_state): State<AppState>,
    token: Token,
    ValidatedJson(req): ValidatedJson<PasswordReq>,
) -> Res<()> {
    match User::find_by_id(token.id).one(&app_state.db).await? {
        None => Err(ServerError::from(UserErr::UserNotExist(token.id))),
        Some(user) => {
            // 修改密码
            let mut user = user.into_active_model();
            user.password = Set(req.password);
            user.update(&app_state.db).await?;
            // 删除登陆状态
            auth::delete_login_status(token.id).await;
            Ok(AppRes::success(()))
        }
    }
}
