use crate::app_state::AppState;
use crate::auth::Token;
use crate::err::ServerError;
use crate::{group, message, middleware, Api, Res};
use axum::extract::State;
use axum::routing::put;
use axum::{Json, Router};
use entity::read_index;
use entity::read_index::{ActiveModel, Model};
use sea_orm::ActiveValue::Set;
use sea_orm::{sea_query, DbErr, EntityTrait, NotSet};
use serde::{Deserialize, Serialize};

pub struct ReadIndexApi;

impl Api for ReadIndexApi {
    fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/", put(read_index))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_login,
            ))
            .with_state(app_state.clone())
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) enum UpdateReadIndex {
    User { target_uid: i32, mid: i64 },
    Group { target_gid: i32, mid: i64 },
}

async fn read_index(
    State(app_state): State<AppState>,
    token: Token,
    Json(read_index): Json<UpdateReadIndex>,
) -> Res<()> {
    set_read_index(&app_state, token.id, read_index).await?;
    Ok(())
}

pub(crate) async fn set_read_index(
    app_state: &AppState,
    uid: i32,
    read_index: UpdateReadIndex,
) -> Result<(), ServerError> {
    Ok(match read_index {
        UpdateReadIndex::User { target_uid, mid } => {
            let active_model = ActiveModel {
                id: Set(Default::default()),
                uid: Set(uid),
                target_uid: Set(Some(target_uid)),
                target_gid: NotSet,
                mid: Set(Some(mid)),
                latest_mid: Set(mid),
                uid_of_latest_msg: Set(uid),
            };
            let result = read_index::Entity::insert(active_model)
                .on_conflict(
                    sea_query::OnConflict::columns([
                        read_index::Column::Uid,
                        read_index::Column::TargetUid,
                    ])
                    .update_columns(vec![
                        read_index::Column::Mid,
                        read_index::Column::LatestMid,
                        read_index::Column::UidOfLatestMsg,
                    ])
                    .to_owned(),
                )
                .exec(&app_state.db)
                .await;
            if let Err(DbErr::RecordNotInserted) = result {
                // do nothing
            }
            let active_model = ActiveModel {
                id: Set(Default::default()),
                uid: Set(target_uid),
                target_uid: Set(Some(uid)),
                target_gid: Default::default(),
                mid: Set(None),
                latest_mid: Set(mid),
                uid_of_latest_msg: Set(uid),
            };
            read_index::Entity::insert(active_model)
                .on_conflict(
                    sea_query::OnConflict::columns([
                        read_index::Column::Uid,
                        read_index::Column::TargetUid,
                    ])
                    .update_columns(vec![
                        read_index::Column::LatestMid,
                        read_index::Column::UidOfLatestMsg,
                    ])
                    .to_owned(),
                )
                .exec(&app_state.db)
                .await?;
        }
        UpdateReadIndex::Group { target_gid, mid } => {
            let active_model = ActiveModel {
                id: Set(Default::default()),
                uid: Set(uid),
                target_uid: NotSet,
                target_gid: Set(Some(target_gid)),
                mid: Set(Some(mid)),
                latest_mid: Set(mid),
                uid_of_latest_msg: Set(uid),
            };
            read_index::Entity::insert(active_model)
                .on_conflict(
                    sea_query::OnConflict::columns([
                        read_index::Column::Uid,
                        read_index::Column::TargetGid,
                    ])
                    .update_columns(vec![
                        read_index::Column::Mid,
                        read_index::Column::LatestMid,
                        read_index::Column::UidOfLatestMsg,
                    ])
                    .to_owned(),
                )
                .exec(&app_state.db)
                .await?;
            let ris = group::get_uids(app_state, target_gid)
                .await?
                .into_iter()
                .map(|rest_uid_of_group| {
                    return ActiveModel {
                        id: Set(Default::default()),
                        uid: Set(rest_uid_of_group),
                        target_uid: NotSet,
                        target_gid: Set(Some(target_gid)),
                        mid: Set(None),
                        latest_mid: Set(mid),
                        uid_of_latest_msg: Set(uid),
                    };
                })
                .collect::<Vec<ActiveModel>>();
            read_index::Entity::insert_many(ris)
                .on_conflict(
                    sea_query::OnConflict::columns([
                        read_index::Column::Uid,
                        read_index::Column::TargetGid,
                    ])
                    .update_columns(vec![
                        read_index::Column::LatestMid,
                        read_index::Column::UidOfLatestMsg,
                    ])
                    .to_owned(),
                )
                .exec(&app_state.db)
                .await?;
        }
    })
}

pub(crate) fn count_unread_msg(ri: &Model, app_state: &AppState) -> Option<String> {
    match (ri.target_uid, ri.target_gid) {
        (Some(target_uid), None) => {
            message::count_dm_unread(ri.uid, target_uid, ri.mid, app_state).map(|c| c.to_string())
        }
        (None, Some(target_gid)) => {
            message::count_group_unread(target_gid, ri.mid, app_state).map(|c| c.to_string())
        }
        _ => None,
    }
}
