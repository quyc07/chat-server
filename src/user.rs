use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Local, NaiveDateTime};
use itertools::Itertools;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;
use utoipa::{OpenApi, ToSchema};
use validator::Validate;

use entity::prelude::User;
use entity::user;

use crate::app_state::AppState;
use crate::auth::{AuthError, Token};
use crate::err::{ErrPrint, ServerError};
use crate::event::BroadcastEvent;
use crate::validate::ValidatedJson;
use crate::{auth, AppRes, Res};

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
            .route("/login", post(login))
            .route("/:uid/send", post(send))
            .route("/:uid/history", get(get_history_msg))
            .route("/history", get(history))
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
    #[serde(with = "my_date_format")]
    pub create_time: NaiveDateTime,
    #[serde(with = "my_opt_date_format")]
    pub update_time: Option<NaiveDateTime>,
    pub status: String,
}

/// 自定义 Option<DateTime> 序列化
mod my_opt_date_format {
    use chrono::NaiveDateTime;
    use serde::{self, Deserialize, Deserializer, Serializer};

    const FORMAT: &'static str = "%Y-%m-%d %H:%M:%S";

    pub type OK = ();

    pub fn serialize<S>(date: &Option<NaiveDateTime>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            None => serializer.serialize_none(),
            Some(t) => serializer.serialize_str(t.format(FORMAT).to_string().as_str()),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<NaiveDateTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match String::deserialize(deserializer) {
            Ok(s) => Ok(Some(
                NaiveDateTime::parse_from_str(&s, FORMAT).map_err(serde::de::Error::custom)?,
            )),
            Err(_) => Ok(None),
        }
    }
}

/// 自定义 DateTime 序列化
mod my_date_format {
    use chrono::NaiveDateTime;
    use serde::{self, Deserialize, Deserializer, Serializer};

    const FORMAT: &'static str = "%Y-%m-%d %H:%M:%S";

    // The signature of a serialize_with function must follow the pattern:
    //
    //    fn serialize<S>(&T, S) -> Result<S::Ok, S::Error>
    //    where
    //        S: Serializer
    //
    // although it may also be generic over the input types T.
    pub fn serialize<S>(date: &NaiveDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = format!("{}", date.format(FORMAT));
        serializer.serialize_str(&s)
    }

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<NaiveDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let dt = NaiveDateTime::parse_from_str(&s, FORMAT).map_err(serde::de::Error::custom)?;
        Ok(dt)
    }
}

impl From<user::Model> for UserRes {
    fn from(value: user::Model) -> Self {
        Self {
            id: value.id,
            name: value.name,
            email: value.email,
            phone: value.phone,
            password: value.password,
            create_time: value.create_time,
            update_time: value.update_time,
            status: value.status.into(),
        }
    }
}

async fn login(
    State(app_state): State<AppState>,
    ValidatedJson(req): ValidatedJson<UserLoginReq>,
) -> Res<UserLoginRes> {
    let user = find_by_name(&app_state, &req.name).await.unwrap().unwrap();
    if user.password != req.password {
        return Err(ServerError::from(AuthError::WrongCredentials));
    }
    // Create the authorization token
    let token = Token::from(user);
    let access_token = auth::gen_token(token).await?;

    // Send the authorized token
    Ok(AppRes::success(UserLoginRes {
        access_token,
        access_token_expires: auth::expire().await,
    }))
}

#[derive(Debug, Deserialize, Validate)]
struct UserLoginReq {
    #[validate(length(min = 1))]
    name: String,
    #[validate(length(min = 1))]
    password: String,
}

#[derive(Debug, Serialize)]
struct UserLoginRes {
    access_token: String,
    access_token_expires: DateTime<Local>,
}

// 按照参数定义的先后顺序进行解析，ValidatedJson会消耗掉Request，因此要放在最后面解析
async fn send(
    State(app_state): State<AppState>,
    uid: Path<i32>,
    token: Token,
    ValidatedJson(msg): ValidatedJson<SendMsgReq>,
) -> Res<i64> {
    let payload = ChatMessagePayload::new(token.id, uid.0, msg.msg);
    let mid = app_state.msg_db.lock().unwrap().messages().send_to_dm(
        token.id as i64,
        uid.0 as i64,
        &serde_json::to_vec(&payload)
            .map_err(|_e| ServerError::CustomErr("fail to transfer message to vec".to_string()))?,
    )?;
    let _ = app_state.event_sender.send(Arc::new(BroadcastEvent::Chat {
        targets: BTreeSet::from([token.id, uid.0]),
        message: ChatMessage::new(mid, payload),
    }));
    return Ok(AppRes::success(mid));
}

#[derive(Deserialize, Validate, Debug)]
struct SendMsgReq {
    #[validate(length(min = 1, code = "1", message = "msg is blank"))]
    msg: String,
}

async fn get_history_msg(
    State(app_state): State<AppState>,
    uid: Path<i32>,
    token: Token,
) -> Res<Vec<ChatMessagePayload>> {
    let msgs = app_state
        .msg_db
        .lock()
        .unwrap()
        .messages()
        .fetch_dm_messages_before(token.id as i64, uid.0 as i64, None, 1000)?;
    let msg = msgs
        .into_iter()
        .filter_map(|(_, msg)| serde_json::from_slice::<ChatMessagePayload>(&msg).ok())
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
    let messages = app_state
        .msg_db
        .lock()
        .unwrap()
        .messages()
        .fetch_user_messages_after(token.id as i64, params.after_mid, i32::MAX as usize)?;
    let chat_messages = messages
        .into_iter()
        .filter_map(|(id, data)| {
            Some(id).zip(serde_json::from_slice::<ChatMessagePayload>(&data).ok())
        })
        .map(|(id, payload)| ChatMessage::new(id, payload))
        .collect::<Vec<ChatMessage>>();
    let mut target_uid_2_msg = chat_messages.into_iter().into_group_map_by(|x| {
        if x.payload.from_uid == token.id {
            x.payload.to_uid
        } else {
            x.payload.from_uid
        }
    });
    target_uid_2_msg.iter_mut().for_each(|(_, v)| {
        v.sort_by(|msg1, msg2| msg2.payload.create_time.cmp(&msg1.payload.create_time))
    });
    Ok(AppRes::success(target_uid_2_msg))
}

/// Chat message
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ChatMessage {
    /// Message id
    pub mid: i64,
    pub payload: ChatMessagePayload,
}

impl ChatMessage {
    fn new(mid: i64, payload: ChatMessagePayload) -> Self {
        ChatMessage { mid, payload }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessagePayload {
    pub from_uid: i32,
    pub to_uid: i32,
    pub create_time: DateTime<Local>,
    pub msg: String,
}

impl ChatMessagePayload {
    fn new(from_uid: i32, to_uid: i32, msg: String) -> Self {
        Self {
            from_uid,
            to_uid,
            create_time: Local::now(),
            msg,
        }
    }
}

async fn find_by_name(app_state: &AppState, name: &str) -> Result<Option<user::Model>, DbErr> {
    User::find()
        .filter(user::Column::Name.eq(name))
        .one(&app_state.db)
        .await
}
