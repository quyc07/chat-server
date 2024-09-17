use std::collections::{HashMap, HashSet};

use crate::datetime::datetime_format;
use axum::extract::{Path, State};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Local};
use futures::{FutureExt, StreamExt, TryStreamExt};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, IntoActiveModel, ModelTrait, QueryFilter, TransactionTrait};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_stream::StreamExt as OtherStreamExt;
use utoipa::{OpenApi, ToSchema};
use validator::Validate;

use entity::group::Model;
use entity::prelude::{Group, UserGroupRel};
use entity::{group, user_group_rel};

use crate::app_state::AppState;
use crate::auth::Token;
use crate::err::{ErrPrint, ServerError};
use crate::message::{
    HistoryMsgGroup, HistoryMsgReq, HistoryReq, MessageTarget, MessageTargetGroup, SendMsgReq,
};
use crate::read_index::UpdateReadIndex;
use crate::user::UserErr;
use crate::validate::ValidatedJson;
use crate::{message, middleware, read_index, user, Api, Res};

#[derive(OpenApi)]
#[openapi(
    paths(
        all, create, add
    ),
    components(
        schemas(GroupRes, CreateReq)
    ),
    tags(
        (name = "group", description = "Group API")
    )
)]
pub struct GroupApi;

impl Api for GroupApi {
    fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/", post(create))
            .route("/:gid/:uid", put(add).delete(remove))
            .route("/:gid", delete(delete_group))
            .route("/:gid/send", put(send))
            .route("/:gid/admin/:uid", patch(admin))
            .route("/:gid/forbid/:uid", put(forbid).delete(un_forbid))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_user_status,
            ))
            .route("/:gid", get(detail))
            .route("/", get(mine))
            .route("/all", get(all))
            .route("/:gid/history", get(history))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_login,
            ))
            .with_state(app_state.clone())
    }
}

#[derive(Debug, Error, ToSchema)]
pub enum GroupErr {
    /// Group not exist
    #[error("群（ID={0}）不存在存在")]
    GroupNotExist(i32),
    /// User not exist in group
    #[error("用户（ID={uid}）不在群（ID={gid}）内")]
    UserNotInGroup { uid: i32, gid: i32 },
    /// 通用异常
    #[error("用户已经被禁言")]
    UserHasBeenForbid,
    #[error("用户已经在群内")]
    UserAlreadyInGroup,
    #[error("用户未被禁言")]
    UserWasNotForbid,
    #[error("您不是群管理员，不能设置群主！")]
    YouAreNotAdmin,
    #[error("您已被禁言，无权发言")]
    YouAreForbid,
}

impl ErrPrint for GroupErr {}

#[derive(Serialize, ToSchema)]
struct GroupRes {
    pub id: i32,
    pub name: String,
}

impl From<Model> for GroupRes {
    fn from(value: Model) -> Self {
        Self {
            id: value.id,
            name: value.name,
        }
    }
}

#[utoipa::path(
    get,
    path = "/group/all",
    responses(
    (status = 200, description = "Get all groups", body = [AllRes]),
    )
)]
async fn all(State(app_state): State<AppState>) -> Res<Json<Vec<GroupRes>>> {
    let groups = Group::find().all(&app_state.db).await?;
    Ok(Json(groups.into_iter().map(GroupRes::from).collect()))
}

#[utoipa::path(
    get,
    path = "/group/mine",
    responses(
    (status = 200, description = "Get all groups", body = [AllRes]),
    )
)]
async fn mine(State(app_state): State<AppState>, token: Token) -> Res<Json<Vec<GroupRes>>> {
    let ugrs = UserGroupRel::find()
        .filter(user_group_rel::Column::UserId.eq(token.id))
        .all(&app_state.db)
        .await?;
    let gids = ugrs.iter().map(|x| x.group_id).collect::<HashSet<i32>>();
    let groups = Group::find()
        .filter(group::Column::Id.is_in(gids))
        .all(&app_state.db)
        .await?;
    Ok(Json(groups.into_iter().map(GroupRes::from).collect()))
}

/// 采用stream操作可减少内存分配
async fn mine_stream(State(app_state): State<AppState>, token: Token) -> Res<Vec<GroupRes>> {
    let mut ugr_stream = UserGroupRel::find()
        .filter(user_group_rel::Column::UserId.eq(token.id))
        .stream(&app_state.db)
        .await?;
    let mut groups = Vec::new();
    while let Some(ugr) = TryStreamExt::try_next(&mut ugr_stream).await? {
        let option = Group::find()
            .filter(group::Column::Id.eq(ugr.group_id))
            .one(&app_state.db)
            .await?;
        if let Some(g) = option {
            groups.push(g);
        }
    }
    Ok(groups.into_iter().map(GroupRes::from).collect())
}

#[derive(Deserialize, Validate, ToSchema)]
struct CreateReq {
    #[validate(length(min = 1, message = "Group name must be at least one letter"))]
    name: String,
}

#[utoipa::path(
    post,
    path = "/group/create",
    request_body = CreateReq,
    responses(
        (status = 200, description = "Create new group", body = [i32])
    )
)]

async fn create(
    State(app_state): State<AppState>,
    token: Token,
    ValidatedJson(req): ValidatedJson<CreateReq>,
) -> Res<String> {
    let group = group::ActiveModel {
        id: Default::default(),
        name: Set(req.name),
        admin: Set(token.id),
        c_time: Default::default(),
        u_time: Default::default(),
    };
    let group = group.insert(&app_state.db).await?;
    add_to_group(&app_state, group.id, token.id).await?;
    Ok(group.id.to_string())
}

#[derive(Deserialize, ToSchema)]
struct AddReq {
    gid: i32,
    uid: i32,
}

#[derive(Deserialize, ToSchema)]
struct RemoveReq {
    gid: i32,
    uid: i32,
}

#[utoipa::path(
    put,
    path = "/:gid/add/:uid",
    responses(
        (status = 200, description = "Add user to group", body = [()]),
    )
)]
async fn add(State(app_state): State<AppState>, Path(req): Path<AddReq>, _: Token) -> Res<()> {
    if !exist(req.gid, &app_state).await? {
        return Err(GroupErr::GroupNotExist(req.gid).into());
    }
    if !user::exist(req.uid, &app_state).await? {
        return Err(UserErr::UserNotExist(req.uid).into());
    }
    if check_group_status(req.gid, req.uid, &app_state)
        .await?
        .in_group
    {
        return Err(GroupErr::UserAlreadyInGroup.into());
    }
    add_to_group(&app_state, req.gid, req.uid).await?;
    Ok(())
}

async fn add_to_group(app_state: &AppState, gid: i32, uid: i32) -> Result<(), ServerError> {
    let rel = user_group_rel::ActiveModel {
        id: Default::default(),
        group_id: Set(gid),
        user_id: Set(uid),
        c_time: Default::default(),
        forbid: Default::default(),
    };
    rel.insert(&app_state.db).await?;
    Ok(())
}

async fn exist(p0: i32, app_state: &AppState) -> Result<bool, DbErr> {
    Group::find()
        .filter(group::Column::Id.eq(p0))
        .one(&app_state.db)
        .await
        .map(|t| t.is_some())
}

struct CheckStatus {
    in_group: bool,
    forbid: bool,
}

async fn check_group_status(
    gid: i32,
    uid: i32,
    app_state: &AppState,
) -> Result<CheckStatus, DbErr> {
    UserGroupRel::find()
        .filter(user_group_rel::Column::GroupId.eq(gid))
        .filter(user_group_rel::Column::UserId.eq(uid))
        .one(&app_state.db)
        .await
        .map(|t| CheckStatus {
            in_group: t.is_some(),
            forbid: t.map(|x| x.forbid).unwrap_or(true),
        })
}

async fn remove(
    State(app_state): State<AppState>,
    Path(req): Path<RemoveReq>,
    _: Token,
) -> Res<()> {
    if !exist(req.gid, &app_state).await? {
        return Err(GroupErr::GroupNotExist(req.gid).into());
    }
    if !user::exist(req.uid, &app_state).await? {
        return Err(UserErr::UserNotExist(req.uid).into());
    }
    if !check_group_status(req.gid, req.uid, &app_state)
        .await?
        .in_group
    {
        return Err(GroupErr::UserNotInGroup {
            uid: req.uid,
            gid: req.gid,
        }
        .into());
    }
    UserGroupRel::delete_many()
        .filter(user_group_rel::Column::GroupId.eq(req.gid))
        .filter(user_group_rel::Column::UserId.eq(req.uid))
        .exec(&app_state.db)
        .await?;
    Ok(())
}

async fn delete_group(
    State(app_state): State<AppState>,
    Path(gid): Path<i32>,
    _: Token,
) -> Res<()> {
    if !exist(gid, &app_state).await? {
        return Err(GroupErr::GroupNotExist(gid).into());
    }
    // 开启事务
    let x = app_state.db.begin().await?;
    if let Some(group) = Group::find_by_id(gid).one(&app_state.db).await? {
        group.delete(&x).await?;
    }
    // return Err(CustomErr("error happened here".to_string()));
    UserGroupRel::delete_many()
        .filter(user_group_rel::Column::GroupId.eq(gid))
        .exec(&x)
        .await?;
    // 提交事务
    x.commit().await?;
    Ok(())
}

#[derive(Serialize)]
struct DetailRes {
    group_id: i32,
    name: String,
    users: Vec<User>,
}

#[derive(Serialize)]
struct User {
    id: i32,
    name: String,
    admin: bool,
    forbid: bool,
}

async fn detail(
    State(app_state): State<AppState>,
    Path(gid): Path<i32>,
    token: Token,
) -> Res<Json<DetailRes>> {
    match Group::find_by_id(gid).one(&app_state.db).await? {
        None => Err(GroupErr::GroupNotExist(gid).into()),
        Some(group) => {
            let rels = get_rels(&app_state, gid).await?;
            // 显示值定数据类型，下面的contians()方法才能通过编译，否则程序无法推断出uids的类型，也就无法使用contains()方法
            let uids: Vec<i32> = rels.iter().map(|x| x.user_id).collect();
            // if !rels.iter().any(|x| x.user_id == token.id) {
            if !uids.contains(&token.id) {
                return Err(GroupErr::UserNotInGroup { uid: token.id, gid }.into());
            }
            let uid_2_forbid: HashMap<i32, bool> =
                rels.iter().map(|x| (x.user_id, x.forbid)).collect();
            let users = user::get_by_ids(uids, &app_state).await?;
            Ok(Json(DetailRes {
                group_id: gid,
                name: group.name,
                users: users
                    .into_iter()
                    .map(|u| User {
                        id: u.id,
                        name: u.name,
                        admin: u.id == group.admin,
                        forbid: *uid_2_forbid.get(&u.id).unwrap_or(&false),
                    })
                    .collect(),
            }))
        }
    }
}

pub(crate) async fn get_uids(app_state: &AppState, gid: i32) -> Result<Vec<i32>, DbErr> {
    Ok(get_rels(&app_state, gid)
        .await?
        .into_iter()
        .map(|ugr| ugr.user_id)
        .collect::<Vec<i32>>())
}

async fn get_rels(app_state: &AppState, gid: i32) -> Result<Vec<user_group_rel::Model>, DbErr> {
    UserGroupRel::find()
        .filter(user_group_rel::Column::GroupId.eq(gid))
        .all(&app_state.db)
        .await
}

async fn admin(
    State(app_state): State<AppState>,
    Path((gid, uid)): Path<(i32, i32)>,
    token: Token,
) -> Res<()> {
    match Group::find_by_id(gid).one(&app_state.db).await? {
        None => Err(GroupErr::GroupNotExist(gid).into()),
        Some(group) => {
            if group.admin != token.id {
                return Err(GroupErr::YouAreNotAdmin.into());
            }
            let uids = get_uids(&app_state, gid).await?;
            if !uids.contains(&uid) {
                return Err(GroupErr::UserNotInGroup { uid: token.id, gid }.into());
            }
            let mut group = group.into_active_model();
            group.admin = Set(uid);
            group.update(&app_state.db).await?;
            Ok(())
        }
    }
}

async fn forbid(
    State(app_state): State<AppState>,
    Path((gid, uid)): Path<(i32, i32)>,
    token: Token,
) -> Res<()> {
    match Group::find_by_id(gid).one(&app_state.db).await? {
        None => Err(GroupErr::GroupNotExist(gid).into()),
        Some(group) => {
            if group.admin != token.id {
                return Err(GroupErr::YouAreNotAdmin.into());
            }
            match UserGroupRel::find()
                .filter(user_group_rel::Column::GroupId.eq(gid))
                .filter(user_group_rel::Column::UserId.eq(uid))
                .one(&app_state.db)
                .await?
            {
                None => Err(GroupErr::UserNotInGroup { uid: token.id, gid }.into()),
                Some(ugr) => {
                    if ugr.forbid == true {
                        return Err(GroupErr::UserHasBeenForbid.into());
                    }
                    let mut model = ugr.into_active_model();
                    model.forbid = Set(true.into());
                    model.update(&app_state.db).await?;
                    Ok(())
                }
            }
        }
    }
}

async fn un_forbid(
    State(app_state): State<AppState>,
    Path((gid, uid)): Path<(i32, i32)>,
    token: Token,
) -> Res<()> {
    match Group::find_by_id(gid).one(&app_state.db).await? {
        None => Err(GroupErr::GroupNotExist(gid).into()),
        Some(group) => {
            if group.admin != token.id {
                return Err(GroupErr::YouAreNotAdmin.into());
            }
            match UserGroupRel::find()
                .filter(user_group_rel::Column::GroupId.eq(gid))
                .filter(user_group_rel::Column::UserId.eq(uid))
                .one(&app_state.db)
                .await?
            {
                None => Err(GroupErr::UserNotInGroup { uid: token.id, gid }.into()),
                Some(ugr) => {
                    if ugr.forbid == false {
                        return Err(GroupErr::UserWasNotForbid.into());
                    }
                    let mut model = ugr.into_active_model();
                    model.forbid = Set(false.into());
                    model.update(&app_state.db).await?;
                    Ok(())
                }
            }
        }
    }
}

pub(crate) async fn get_user_by_gid(
    app_state: AppState,
    gid: i32,
) -> Result<Vec<entity::user::Model>, DbErr> {
    let uids = get_uids(&app_state, gid).await?;
    user::get_by_ids(uids, &app_state).await
}


async fn send(
    State(app_state): State<AppState>,
    Path(gid): Path<i32>,
    token: Token,
    ValidatedJson(msg): ValidatedJson<SendMsgReq>,
) -> Res<String> {
    let s = check_group_status(gid, token.id, &app_state).await?;
    if !s.in_group {
        return Err(GroupErr::UserNotInGroup { uid: token.id, gid }.into());
    };
    if s.forbid {
        return Err(GroupErr::YouAreForbid.into());
    }
    let payload = msg.build_payload(token.id, MessageTarget::Group(MessageTargetGroup { gid }));
    let mid = message::send_msg(payload, &app_state).await?;
    // 设置当前用户的read_index
    read_index::set_read_index(
        &app_state,
        token.id,
        UpdateReadIndex::Group {
            target_gid: gid,
            mid,
        },
    )
    .await?;
    Ok(mid.to_string())
}

pub(crate) async fn get_by_gids(gids: Vec<i32>, app_state: &AppState) -> Result<Vec<Model>, DbErr> {
    Group::find()
        .filter(group::Column::Id.is_in(gids))
        .all(&app_state.db)
        .await
}

#[derive(Serialize)]
struct GroupHistoryMsg {
    mid: i64,
    msg: String,
    #[serde(with = "datetime_format")]
    time: DateTime<Local>,
    from_uid: i32,
    name_of_from_uid: String,
}

pub(crate) async fn history(
    State(app_state): State<AppState>,
    token: Token,
    Path(gid): Path<i32>,
) -> Res<Json<Vec<GroupHistoryMsg>>> {
    if !check_group_status(gid, token.id, &app_state)
        .await?
        .in_group
    {
        return Err(GroupErr::UserNotInGroup { uid: token.id, gid }.into());
    }
    let mut history_msg = message::get_history_msg(
        &app_state,
        HistoryMsgReq::Group(HistoryMsgGroup {
            gid,
            history: HistoryReq {
                before: None,
                limit: 1000,
            },
        }),
    );
    history_msg.sort_by(|m1, m2| m2.payload.created_at.cmp(&m1.payload.created_at));
    let from_uids = history_msg
        .iter()
        .map(|x| x.payload.from_uid)
        .collect::<Vec<i32>>();
    let from_uid_2_name = user::get_by_ids(from_uids, &app_state)
        .await?
        .iter()
        .map(|x| (x.id, x.name.clone()))
        .collect::<HashMap<i32, String>>();
    Ok(Json(history_msg
        .into_iter()
        .map(|x| GroupHistoryMsg {
            mid: x.mid,
            msg: x.payload.detail.get_content(),
            time: x.payload.created_at,
            from_uid: x.payload.from_uid,
            name_of_from_uid: from_uid_2_name
                .get(&x.payload.from_uid)
                .unwrap_or(&"未知用户".to_string())
                .to_string(),
        })
        .collect()))
}
