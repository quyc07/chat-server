use axum::extract::{Path, State};
use axum::Router;
use axum::routing::{delete, get, post, put};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, ModelTrait, QueryFilter, TransactionTrait,
};
use sea_orm::ActiveValue::Set;
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use validator::Validate;

use entity::{group, user_group_rel};
use entity::group::Model;
use entity::prelude::{Group, UserGroupRel};

use crate::{AppRes, Res, user};
use crate::app_state::AppState;
use crate::auth::Token;
use crate::validate::ValidatedJson;

#[derive(OpenApi)]
#[openapi(
    paths(
        all,create,add
    ),
    components(
        schemas(AllRes,CreateReq)
    ),
    tags(
        (name = "group", description = "Group API")
    )
)]
pub struct GroupApi;

impl GroupApi {
    pub fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/all", get(all))
            .route("/create", post(create))
            .route("/:gid/add/:uid", put(add))
            .route("/:gid/remove/:uid", delete(remove))
            .route("/delete/:gid", delete(delete_group))
            .with_state(app_state)
    }
}

#[derive(Serialize, ToSchema)]
struct AllRes {
    pub id: i32,
    pub name: String,
}

impl From<Model> for AllRes {
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
async fn all(State(app_state): State<AppState>, _: Token) -> Res<Vec<AllRes>> {
    let groups = Group::find().all(&app_state.db).await?;
    Ok(AppRes::success(
        groups.into_iter().map(AllRes::from).collect(),
    ))
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
) -> Res<i32> {
    let group = group::ActiveModel {
        id: Default::default(),
        name: Set(req.name),
        admin: Set(token.id),
        c_time: Default::default(),
        u_time: Default::default(),
    };
    let group = group.insert(&app_state.db).await?;
    Ok(AppRes::success(group.id))
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
        return Ok(AppRes::fail_with_msg(format!("群（id={}）不存在", req.gid)));
    }
    if !user::exist(req.uid, &app_state).await? {
        return Ok(AppRes::fail_with_msg(format!(
            "用户（id={}）不存在",
            req.uid
        )));
    }
    if is_in_group(req.gid, req.uid, &app_state).await? {
        return Ok(AppRes::success_with_msg(
            "用户已在群内，无需再次添加".to_string(),
        ));
    }
    let rel = user_group_rel::ActiveModel {
        id: Default::default(),
        group_id: Set(req.gid),
        user_id: Set(req.uid),
        c_time: Default::default(),
        can_replay: Default::default(),
    };
    rel.insert(&app_state.db).await?;
    Ok(AppRes::success(()))
}

async fn exist(p0: i32, app_state: &AppState) -> Result<bool, DbErr> {
    Group::find()
        .filter(group::Column::Id.eq(p0))
        .one(&app_state.db)
        .await
        .map(|t| t.is_some())
}

async fn is_in_group(gid: i32, uid: i32, app_state: &AppState) -> Result<bool, DbErr> {
    UserGroupRel::find()
        .filter(user_group_rel::Column::GroupId.eq(gid))
        .filter(user_group_rel::Column::UserId.eq(uid))
        .one(&app_state.db)
        .await
        .map(|t| t.is_some())
}

async fn remove(
    State(app_state): State<AppState>,
    Path(req): Path<RemoveReq>,
    _: Token,
) -> Res<()> {
    if !exist(req.gid, &app_state).await? {
        return Ok(AppRes::fail_with_msg(format!("群（id={}）不存在", req.gid)));
    }
    if !user::exist(req.uid, &app_state).await? {
        return Ok(AppRes::fail_with_msg(format!(
            "用户（id={}）不存在",
            req.uid
        )));
    }
    if !is_in_group(req.gid, req.uid, &app_state).await? {
        return Ok(AppRes::fail_with_msg("用户不在群内，无需移出".to_string()));
    }
    UserGroupRel::delete_many()
        .filter(user_group_rel::Column::GroupId.eq(req.gid))
        .filter(user_group_rel::Column::UserId.eq(req.uid))
        .exec(&app_state.db)
        .await?;
    return Ok(AppRes::success(()));
}

async fn delete_group(
    State(app_state): State<AppState>,
    Path(gid): Path<i32>,
    _: Token,
) -> Res<()> {
    if !exist(gid, &app_state).await? {
        return Ok(AppRes::fail_with_msg(format!("群（ID={}）不存在", gid)));
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
    return Ok(AppRes::success(()));
}
