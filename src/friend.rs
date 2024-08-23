use crate::app_state::AppState;
use crate::auth::Token;
use crate::datetime::datetime_format;
use crate::{datetime, dgraph, user, AppRes, Res};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Local};
use entity::friend_request;
use entity::prelude::FriendRequest;
use entity::sea_orm_active_enums::FriendRequestStatus;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub struct FriendApi;

impl FriendApi {
    pub fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/req/:uid", post(request))
            .route("/req", get(req_list))
            .with_state(app_state)
    }
}

#[derive(Deserialize)]
struct Request {
    reason: Option<String>,
}

async fn request(
    State(app_state): State<AppState>,
    Path(friend_id): Path<i32>,
    token: Token,
    Json(Request { reason }): Json<Request>,
) -> Res<()> {
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

async fn review() -> Res<()> {
    todo!("待实现审核好友申请")
}
