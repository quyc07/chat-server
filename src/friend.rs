mod dgraph;

use crate::app_state::AppState;
use crate::auth::Token;
use crate::datetime::datetime_format;
use crate::err::{ErrPrint, ServerError};
use crate::friend::dgraph::{FriendVo, Location, Point};
use crate::{datetime, middleware, user, Api, AppRes, Res};
use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Local};
use entity::friend_request;
use entity::prelude::FriendRequest;
use entity::sea_orm_active_enums::FriendRequestStatus;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use utoipa::ToSchema;

pub struct FriendApi;

impl Api for FriendApi {
    fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/loc/:radius", patch(set_loc).get(nearby))
            .route("/req/:uid", post(request))
            .route("/req", post(review))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_user_status,
            ))
            .route("/", get(list))
            .route("/req", get(req_list))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_login,
            ))
            .with_state(app_state.clone())
    }
}

#[derive(Debug, Error, ToSchema)]
pub(crate) enum FriendErr {
    #[error("用户{0}不是您的好友")]
    NotFriend(i32),
    #[error("您不是该好友请求的目标对象，无权批准")]
    CanNotReviewFriendRequest,
}

impl ErrPrint for FriendErr {}

#[derive(Deserialize)]
struct Request {
    reason: Option<String>,
}

/// 发送好友请求
async fn request(
    State(app_state): State<AppState>,
    Path(friend_id): Path<i32>,
    token: Token,
    Json(Request { reason }): Json<Request>,
) -> Res<()> {
    user::check_status(friend_id, token.id, &app_state).await?;
    // 1. 若两者已是好友，则直接返回
    if dgraph::is_friend(token.dgraph_uid, friend_id).await? {
        return Ok(AppRes::success_with_msg(
            "已经是好友，无需再次申请".to_string(),
        ));
    }
    // 2. 查看是否已有请求记录
    match FriendRequest::find()
        .filter(friend_request::Column::RequestId.eq(token.id))
        .filter(friend_request::Column::TargetId.eq(friend_id))
        .one(&app_state.db)
        .await?
    {
        // 3. 存在
        Some(req) => match req.status {
            // 3.1 若状态是已通过，则直接返回
            FriendRequestStatus::APPROVE => Ok(AppRes::success_with_msg(
                "已经是好友，无需再次申请".to_string(),
            )),
            // 3.2 若状态是等待，则直接返回
            FriendRequestStatus::WAIT => Ok(AppRes::success_with_msg(
                "请求等待中，请勿再次发起".to_string(),
            )),
            // 3.3 若状态是拒绝，则修改状态是等待
            FriendRequestStatus::REJECT => {
                let mut req = req.into_active_model();
                req.status = Set(FriendRequestStatus::WAIT);
                req.reason = Set(reason);
                req.update(&app_state.db).await?;
                Ok(AppRes::success(()))
            }
        },
        // 4. 若不存在，则创建请求记录
        None => {
            friend_request::ActiveModel {
                id: Default::default(),
                request_id: Set(token.id),
                target_id: Set(friend_id),
                reason: Set(reason),
                status: Default::default(),
                create_time: Default::default(),
                modify_time: Default::default(),
            }
            .insert(&app_state.db)
            .await?;
            Ok(AppRes::success(()))
        }
    }
}

#[derive(Debug, Serialize)]
struct FriendReqVo {
    id: i32,
    request_id: i32,
    request_name: String,
    #[serde(with = "datetime_format")]
    create_time: DateTime<Local>,
    reason: Option<String>,
    status: FriendRequestStatus,
}

async fn req_list(State(app_state): State<AppState>, token: Token) -> Res<Vec<FriendReqVo>> {
    let reqs = FriendRequest::find()
        .filter(friend_request::Column::TargetId.eq(token.id))
        .all(&app_state.db)
        .await?;
    let id_2_name = user::get_by_ids(reqs.iter().map(|x| x.request_id).collect(), &app_state)
        .await?
        .iter()
        .map(|user| (user.id, user.name.clone()))
        .collect::<HashMap<i32, String>>();
    Ok(AppRes::success(
        reqs.iter()
            .map(|req| FriendReqVo {
                id: req.id,
                request_id: req.request_id,
                request_name: id_2_name
                    .get(&req.request_id)
                    .unwrap_or(&"未知用户".to_string())
                    .to_string(),
                create_time: datetime::native_datetime_2_datetime(req.create_time),
                reason: req.reason.clone(),
                status: req.status.clone(),
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct ReviewReq {
    id: i32,
    status: FriendRequestStatus,
}

async fn review(
    State(app_state): State<AppState>,
    token: Token,
    Json(req): Json<ReviewReq>,
) -> Res<()> {
    // 1. 更新db状态
    match FriendRequest::find_by_id(req.id).one(&app_state.db).await? {
        None => Ok(AppRes::success(())),
        Some(fr) => {
            if fr.target_id != token.id {
                return Err(ServerError::from(FriendErr::CanNotReviewFriendRequest));
            }
            let mut fr = fr.into_active_model();
            fr.status = Set(req.status);
            let fr = fr.update(&app_state.db).await?;
            // 2. 建立dgraph好友关系
            let request_user = user::get_by_id(fr.request_id, &app_state)
                .await?
                .ok_or(user::UserErr::UserNotExist(fr.request_id))?;
            let target_user = user::get_by_id(fr.target_id, &app_state)
                .await?
                .ok_or(user::UserErr::UserNotExist(fr.target_id))?;
            Ok(AppRes::success(
                dgraph::set_friend_ship(request_user.dgraph_uid, target_user.dgraph_uid).await?,
            ))
        }
    }
}

#[derive(Serialize)]
struct Friend {
    id: i32,
    name: String,
}

/// 好友列表
async fn list(token: Token) -> Res<Vec<Friend>> {
    match dgraph::get_friends(token.dgraph_uid.as_str()).await? {
        None => Ok(AppRes::success(vec![])),
        Some(res) => match res.friend {
            None => Ok(AppRes::success(vec![])),
            Some(friends) => Ok(AppRes::success(
                friends
                    .iter()
                    .map(|friend| Friend {
                        id: friend.user_id,
                        name: friend.name.clone(),
                    })
                    .collect(),
            )),
        },
    }
}

pub(crate) struct FriendRegister {
    pub(crate) user_id: i32,
    pub(crate) name: String,
    pub(crate) phone: Option<String>,
}

pub(crate) async fn register(fr: FriendRegister) -> Result<String, ServerError> {
    dgraph::register(fr).await
}

pub(crate) async fn is_friend(object_graph_id: String, user_id: i32) -> bool {
    dgraph::is_friend(object_graph_id, user_id)
        .await
        .unwrap_or(false)
}

#[derive(Deserialize, ToSchema)]
struct Loc {
    // #[validate(length(min = 1))]
    longitude: f64,
    // #[validate(length(min = 1))]
    latitude: f64,
}
async fn set_loc(token: Token, Json(loc): Json<Loc>) -> Res<()> {
    dgraph::set_loc(
        token.dgraph_uid,
        Location::Point(Point {
            long: loc.longitude,
            lat: loc.latitude,
        }),
    )
    .await?;
    Ok(AppRes::success(()))
}

async fn nearby(token: Token, Path(radius): Path<i32>) -> Res<Vec<FriendVo>> {
    if let Some(friends) = dgraph::get_friends(token.dgraph_uid.as_str()).await? {
        if let Some(loc) = friends.loc {
            match loc.r#type.as_str() {
                "Point" => {
                    return Ok(AppRes::success(
                        dgraph::nearby(
                            Location::Point(Point {
                                long: loc.coordinates[0],
                                lat: loc.coordinates[1],
                            }),
                            radius,
                        )
                        .await?,
                    ))
                }
                "Polygon" => {
                    todo!("待实现区域");
                }
                "MultiPolygon" => {}
                _ => {}
            }
        }
    }
    Ok(AppRes::success(vec![]))
}
