use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::Router;
use axum::routing::{get, patch, post};
use chrono::{DateTime, Local, Offset};
use itertools::Itertools;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DbErr, EntityTrait, IntoActiveModel, QueryFilter,
};
use sea_orm::ActiveValue::Set;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;
use utoipa::{OpenApi, ToSchema};
use validator::Validate;

use entity::prelude::User;
use entity::user;

use crate::{AppRes, auth, message, Res};
use crate::app_state::AppState;
use crate::auth::Token;
use crate::err::{ErrPrint, ServerError};
use crate::format::datetime_format;
use crate::format::opt_datetime_format;
use crate::message::{
    ChatMessage, ChatMessagePayload, MessageTarget, MessageTargetUser, SendMsgReq,
};
use crate::validate::ValidatedJson;

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
            .route("/:uid/history", get(get_history_msg))
            .route("/history", get(history))
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
    let model = result.unwrap();
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

    let user = user::ActiveModel {
        id: Default::default(),
        name: Set(req.name),
        password: Set(req.password),
        email: Set(req.email),
        phone: Set(req.phone),
        create_time: Default::default(),
        update_time: Default::default(),
        status: ActiveValue::NotSet,
    };
    let model = user.insert(&app_state.db).await?;
    Ok(AppRes::success(UserRes::from(model)))
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
}

impl From<user::Model> for UserRes {
    fn from(value: user::Model) -> Self {
        Self {
            id: value.id,
            name: value.name,
            email: value.email,
            phone: value.phone,
            password: value.password,
            create_time: DateTime::<Local>::from_naive_utc_and_offset(
                value.create_time,
                Local::now().offset().fix(),
            ),
            update_time: value.update_time.map(|t| {
                DateTime::<Local>::from_naive_utc_and_offset(t, Local::now().offset().fix())
            }),
            status: value.status.into(),
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
    let payload = msg.build_payload(token.id, MessageTarget::User(MessageTargetUser { uid }));
    let mid = message::send_msg(payload, app_state).await?;
    return Ok(AppRes::success(mid));
}

async fn get_history_msg(
    State(app_state): State<AppState>,
    uid: Path<i32>,
    token: Token,
) -> Res<Vec<ChatMessage>> {
    let msgs = app_state
        .msg_db
        .lock()
        .unwrap()
        .messages()
        .fetch_dm_messages_before(token.id as i64, uid.0 as i64, None, 1000)?;
    let msg = msgs
        .into_iter()
        .filter_map(|(mid, msg)| {
            serde_json::from_slice::<ChatMessagePayload>(&msg)
                .ok()
                .map(|c| ChatMessage::new(mid, c))
        })
        .collect();

    Ok(AppRes::success(msg))
}

#[derive(Debug, Deserialize)]
struct Params {
    after_mid: Option<i64>,
}

async fn history(
    State(app_state): State<AppState>,
    Query(params): Query<Params>,
    token: Token,
) -> Res<HashMap<i32, Vec<ChatMessage>>> {
    // let messages = app_state
    //     .msg_db
    //     .lock()
    //     .unwrap()
    //     .messages()
    //     .fetch_user_messages_after(token.id as i64, params.after_mid, i32::MAX as usize)?;
    // let chat_messages = messages
    //     .into_iter()
    //     .filter_map(|(id, data)| {
    //         Some(id).zip(serde_json::from_slice::<ChatMessagePayload>(&data).ok())
    //     })
    //     .map(|(id, payload)| ChatMessage::new(id, payload))
    //     .collect::<Vec<ChatMessage>>();
    // let mut target_uid_2_msg = chat_messages.into_iter().into_group_map_by(|x| {
    //     if x.payload.from_uid == token.id {
    //         x.payload.to_uid
    //     } else {
    //         x.payload.from_uid
    //     }
    // });
    // target_uid_2_msg.iter_mut().for_each(|(_, v)| {
    //     v.sort_by(|msg1, msg2| msg2.payload.create_time.cmp(&msg1.payload.create_time))
    // });
    // Ok(AppRes::success(target_uid_2_msg))
    todo!("查询历史消息");
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
