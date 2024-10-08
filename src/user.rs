use std::collections::HashMap;
use std::option::Option;

use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Local};
use itertools::Itertools;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, IntoActiveModel, QueryFilter, QuerySelect};
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
use crate::friend::{FriendErr, FriendRegister};
use crate::message::{
    ChatMessage, HistoryMsgReq, HistoryMsgUser, HistoryReq, MessageTarget, MessageTargetUser,
    SendMsgReq,
};
use crate::read_index::UpdateReadIndex;
use crate::validate::ValidatedJson;
use crate::{auth, datetime, friend, group, message, middleware, Res};
use crate::{read_index, Api};
use entity::prelude::User;
use entity::sea_orm_active_enums::UserStatus;
use entity::user;

#[derive(OpenApi)]
#[openapi(
    paths(
        register,send,user_history,password,detail,history
    ),
    components(
        schemas(UserRegisterReq,SendMsgReq,UserHistoryMsg,PasswordReq,
        UserDetail,ChatVo,UserErr,friend::FriendErr)
    ),
    tags(
        (name = "user", description = "USER API")
    )
)]
pub struct UserApi;

impl Api for UserApi {
    fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/:uid/send", post(send))
            .route("/password", patch(password))
            .route("/:name", get(detail))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_user_status,
            ))
            .route("/:uid/history", get(user_history))
            .route("/history/:limit", get(history))
            .route("/find/:name", get(find_friend))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_login,
            ))
            .route("/register", post(register))
            .with_state(app_state.clone())
    }
}

#[derive(Serialize)]
struct FindFriendRes {
    id: i32,
    name: String,
}

async fn find_friend(
    State(app_state): State<AppState>,
    Path(name): Path<String>,
) -> Res<Json<Vec<FindFriendRes>>> {
    let result = User::find()
        .filter(user::Column::Name.like(format!("%{name}%")))
        .all(&app_state.db)
        .await?;
    Ok(Json(
        result
            .into_iter()
            .map(|model| FindFriendRes {
                id: model.id,
                name: model.name,
            })
            .collect(),
    ))
}

/// Register New User
#[derive(Debug, Deserialize, Validate, ToSchema)]
struct UserRegisterReq {
    /// name
    #[validate(length(min = 1))]
    name: String,
    /// email
    #[validate(email)]
    email: Option<String>,
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
    /// UserName not exist
    #[error("用户{0}不存在")]
    UserNameNotExist(String),
    /// Login user was Freeze
    #[error("您的账号已冻结，请先申请解冻")]
    LoginUserWasFreeze,
    /// User was Freeze
    #[error("对方的账号异常，请谨慎操作")]
    UserWasFreeze(String),
}

impl ErrPrint for UserErr {}

/// Register User.
///
/// Register User and return the User.
#[utoipa::path(
    post,
    path = "/user/register",
    request_body = UserRegisterReq,
    responses(
        (status = 200, description = "Register User and return the User successfully", body = AppRes<i32> ),
        (status = 409, description = "UserName already exists", body = UserErr)
    )
)]
async fn register(
    State(app_state): State<AppState>,
    ValidatedJson(req): ValidatedJson<UserRegisterReq>,
) -> Res<String> {
    let name = req.name.as_str();
    if find_by_name(&app_state, name).await?.is_some() {
        return Err(UserErr::UserNameExist(name.to_string()).into());
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
        status: Default::default(),
        dgraph_uid: Default::default(),
        role: Default::default(),
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
    Ok(user.id.to_string())
}

/// The User Detail.
#[derive(Serialize, Deserialize, ToSchema)]
struct UserDetail {
    /// User id
    pub id: i32,
    /// User name
    #[schema(example = "User Name")]
    pub name: String,
    /// User email
    pub email: Option<String>,
    /// User phone
    pub phone: Option<String>,
    /// Create time
    #[serde(with = "datetime_format")]
    pub create_time: DateTime<Local>,
    /// Update time
    #[serde(with = "opt_datetime_format")]
    pub update_time: Option<DateTime<Local>>,
    /// User status
    pub status: String,
    /// dgraph uid
    pub dgraph_uid: String,
    /// Is friend
    pub is_friend: bool,
}

impl From<user::Model> for UserDetail {
    fn from(value: user::Model) -> Self {
        Self {
            id: value.id,
            name: value.name,
            email: value.email,
            phone: value.phone,
            create_time: datetime::native_datetime_2_datetime(value.create_time),
            update_time: value
                .update_time
                .map(|t| datetime::native_datetime_2_datetime(t)),
            status: value.status.into(),
            dgraph_uid: value.dgraph_uid,
            is_friend: false,
        }
    }
}

#[utoipa::path(
    post,
    path = "/{uid}/send",
    params(
        ("uid" = i32, Path, description = "id of friend")
    ),
    request_body = SendMsgReq,
    responses(
        (status = 200, description = "Send message to user successfully"),
        (status = 401, description = "Friend was freeze", body = FriendErr),
    ),
)]
/// 向好友发送消息

async fn send(
    State(app_state): State<AppState>,
    Path(uid): Path<i32>,
    token: Token,
    // 按照参数定义的先后顺序进行解析，ValidatedJson会消耗掉Request，因此要放在最后面解析
    ValidatedJson(msg): ValidatedJson<SendMsgReq>,
) -> Res<String> {
    // 校验好友状态
    check_status(uid, token.id, &app_state).await?;
    // 判断是否是好友
    if !friend::is_friend(token.dgraph_uid, uid).await {
        return Err(FriendErr::NotFriend(uid).into());
    }
    let payload = msg.build_payload(token.id, MessageTarget::User(MessageTargetUser { uid }));
    let mid = message::send_msg(payload, &app_state).await?;
    // 设置read_index
    read_index::set_read_index(
        &app_state,
        token.id,
        UpdateReadIndex::User {
            target_uid: uid,
            mid,
        },
    )
    .await?;
    Ok(mid.to_string())
}

/// 历史聊天记录
#[derive(Serialize, ToSchema)]
struct UserHistoryMsg {
    /// 消息id
    mid: i64,
    /// 消息内容
    msg: String,
    /// 消息发送时间
    #[serde(with = "datetime_format")]
    time: DateTime<Local>,
    /// 消息发送者id
    from_uid: i32,
}

#[utoipa::path(
    get,
    path = "/{uid}/history",
    params(
        ("uid" = i32, Path, description = "id of friend")
    ),
    responses(
        (status = 200, description = "Get history message successfully", body = [UserHistoryMsg]),
        (status = 401, description = "Target user is not friend of you", body = FriendErr),
    ),
)]
/// 查询与好友的聊天记录
async fn user_history(
    State(app_state): State<AppState>,
    Path(uid): Path<i32>,
    token: Token,
) -> Res<Json<Vec<UserHistoryMsg>>> {
    if !friend::is_friend(token.dgraph_uid, uid).await {
        return Err(FriendErr::NotFriend(uid).into());
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
    Ok(Json(
        history_msg
            .into_iter()
            .map(|x| UserHistoryMsg {
                mid: x.mid,
                msg: x.payload.detail.get_content(),
                time: x.payload.created_at,
                from_uid: x.payload.from_uid,
            })
            .sorted_by(|x1, x2| x1.time.cmp(&x2.time))
            .collect(),
    ))
}

#[derive(Hash, Clone, PartialEq, Eq)]
enum ChatTarget {
    User,
    Group,
}

/// 聊天记录
#[derive(Debug, Serialize, Hash, Eq, PartialEq, ToSchema)]
enum ChatVo {
    /// UserChat
    User {
        /// id of friend
        uid: i32,
        /// name of friend
        user_name: String,
        /// message id
        mid: i64,
        /// message content
        msg: String,
        /// message time
        #[serde(with = "datetime_format")]
        msg_time: DateTime<Local>,
        /// unread message count
        unread: Option<String>,
    },
    /// GroupChat
    Group {
        /// id of group
        gid: i32,
        /// name of group
        group_name: String,
        /// id of friend
        uid: i32,
        /// name of friend
        user_name: String,
        /// message id
        mid: i64,
        /// message content
        msg: String,
        /// message time
        #[serde(with = "datetime_format")]
        msg_time: DateTime<Local>,
        /// unread message count
        unread: Option<String>,
    },
}

impl ChatVo {
    fn get_msg_time(&self) -> &DateTime<Local> {
        match self {
            ChatVo::User { msg_time, .. } => msg_time,
            ChatVo::Group { msg_time, .. } => msg_time,
        }
    }
}

#[utoipa::path(
    get,
    path = "/history",
    params(
        ("limit" = u64, Path, description = "limit of chat list")
    ),
    responses(
        (status = 200, description = "Get chat list successfully", body = ChatList),
    ),
)]
/// 查询用户最近聊天列表
async fn history(
    State(app_state): State<AppState>,
    Path(limit): Path<u64>,
    token: Token,
) -> Res<Json<Vec<ChatVo>>> {
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
                .map(|x| (x.target_uid.unwrap(), x.latest_mid))
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
                .map(|x| ChatVo::User {
                    uid: x.target_uid.unwrap(),
                    user_name: uid_2_name
                        .get(&x.target_uid.unwrap())
                        .unwrap_or(&String::from("未知用户"))
                        .to_string(),
                    mid: x.latest_mid,
                    msg: mid_2_msg
                        .get(&x.latest_mid)
                        .map(|x| x.payload.detail.get_content())
                        .unwrap_or(String::from("未知消息")),
                    msg_time: mid_2_msg
                        .get(&x.latest_mid)
                        .map(|x| x.payload.created_at)
                        .unwrap_or(Local::now()),
                    unread: read_index::count_unread_msg(x, &app_state),
                })
                .collect()
        }
        None => vec![],
    };
    let chat_of_group = match map.get(&ChatTarget::Group) {
        None => vec![],
        Some(ris_of_group) => {
            let ((gids, uids), mids) = ris_of_group
                .iter()
                .map(|x| ((x.target_gid.unwrap(), x.uid_of_latest_msg), x.latest_mid))
                .collect::<((Vec<i32>, Vec<i32>), Vec<i64>)>();
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
                .map(|x| ChatVo::Group {
                    gid: x.target_gid.unwrap(),
                    group_name: gid_2_name
                        .get(&x.target_gid.unwrap())
                        .unwrap_or(&String::from("未知群聊"))
                        .to_string(),
                    uid: x.uid_of_latest_msg,
                    user_name: uid_2_name
                        .get(&x.uid_of_latest_msg)
                        .unwrap_or(&String::from("未知用户"))
                        .to_string(),
                    mid: x.latest_mid,
                    msg: mid_2_msg
                        .get(&x.latest_mid)
                        .map(|x| x.payload.detail.get_content())
                        .unwrap_or(String::from("未知消息")),
                    msg_time: mid_2_msg
                        .get(&x.latest_mid)
                        .map(|x| x.payload.created_at)
                        .unwrap_or(Local::now()),
                    unread: read_index::count_unread_msg(x, &app_state),
                })
                .collect()
        }
    };
    let mut history = chat_of_user
        .into_iter()
        .chain(chat_of_group)
        .collect::<Vec<ChatVo>>();
    history.sort_by(|x1, x2| x2.get_msg_time().cmp(&x1.get_msg_time()));
    Ok(Json(history))
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

/// 修改密码
#[derive(Deserialize, ToSchema, Validate)]
struct PasswordReq {
    /// 新密码
    #[validate(length(min = 1, message = "password is blank"))]
    password: String,
}

#[utoipa::path(
    post,
    path = "/password",
    request_body(content = PasswordReq, description = "修改密码", content_type = "application/json"),
    responses(
        (status = 404, description = "用户不存在", content_type = "application/json", body = UserErr)
    ),
)]
/// 修改密码
async fn password(
    State(app_state): State<AppState>,
    token: Token,
    ValidatedJson(req): ValidatedJson<PasswordReq>,
) -> Res<()> {
    match User::find_by_id(token.id).one(&app_state.db).await? {
        None => Err(UserErr::UserNotExist(token.id).into()),
        Some(user) => {
            // 修改密码
            let mut user = user.into_active_model();
            user.password = Set(req.password);
            user.update(&app_state.db).await?;
            // 删除登陆状态
            auth::delete_login_status(token.id).await;
            Ok(())
        }
    }
}

// 判断用户状态是否是冻结状态，如果是冻结状态，则抛出用户状态异常的error
pub(crate) async fn check_status(
    uid: i32,
    login_uid: i32,
    app_state: &AppState,
) -> Result<(), ServerError> {
    match User::find_by_id(uid).one(&app_state.db).await? {
        None => Err(UserErr::UserNotExist(uid).into()),
        Some(user) => match user.status {
            UserStatus::Freeze if login_uid != uid => Err(UserErr::UserWasFreeze(user.name).into()),
            UserStatus::Freeze => Err(UserErr::LoginUserWasFreeze.into()),
            UserStatus::Normal => Ok(()),
        },
    }
}

#[utoipa::path(
    get,
    path = "/{name}",
    params(
        ("name" = String, Path, description = "用户名")
    ),
    responses(
        (status = 200, description = "查询成功", body = UserDetail),
        (status = 404, description = "用户不存在", body = UserErr),
    ),
)]
/// 根据用户名查询用户详情，并判断是否是自己好友
async fn detail(
    State(app_state): State<AppState>,
    Path(name): Path<String>,
    token: Token,
) -> Res<Json<UserDetail>> {
    match User::find()
        .filter(user::Column::Name.eq(name.clone()))
        .one(&app_state.db)
        .await?
    {
        None => Err(UserErr::UserNameNotExist(name).into()),
        Some(user) => {
            let mut detail = UserDetail::from(user);
            detail.is_friend = friend::is_friend(detail.dgraph_uid.clone(), token.id).await;
            Ok(Json(detail))
        }
    }
}
