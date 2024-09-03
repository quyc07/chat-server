use crate::app_state::AppState;
use crate::auth::Token;
use crate::err::ServerError;
use crate::{middleware, Api, AppRes, Res};
use axum::extract::State;
use axum::routing::put;
use axum::{Json, Router};
use entity::read_index;
use sea_orm::ActiveValue::Set;
use sea_orm::{sea_query, DbErr, EntityTrait};
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
    User { uid: i32, mid: i64, uid_of_msg: i32 },
    Group { gid: i32, mid: i64, uid_of_msg: i32 },
}

async fn read_index(
    State(app_state): State<AppState>,
    token: Token,
    Json(read_index): Json<UpdateReadIndex>,
) -> Res<()> {
    set_read_index(&app_state, token.id, read_index).await?;
    Ok(AppRes::success(()))
}

pub(crate) async fn set_read_index(
    app_state: &AppState,
    uid: i32,
    read_index: UpdateReadIndex,
) -> Result<(), ServerError> {
    Ok(match read_index {
        UpdateReadIndex::User {
            uid,
            mid,
            uid_of_msg,
        } => {
            let active_model = read_index::ActiveModel {
                id: Set(Default::default()),
                uid: Set(uid),
                target_uid: Set(Some(uid)),
                target_gid: Default::default(),
                mid: Set(mid),
                uid_of_msg: Set(uid_of_msg),
            };
            let result = read_index::Entity::insert(active_model)
                .on_conflict(
                    sea_query::OnConflict::columns([
                        read_index::Column::Uid,
                        read_index::Column::TargetUid,
                    ])
                    .update_column(read_index::Column::Mid)
                    .to_owned(),
                )
                .exec(&app_state.db)
                .await;
            if let Err(DbErr::RecordNotInserted) = result {
                // do nothing
            }
        }
        UpdateReadIndex::Group {
            gid,
            mid,
            uid_of_msg,
        } => {
            let active_model = read_index::ActiveModel {
                id: Set(Default::default()),
                uid: Set(uid),
                target_uid: Default::default(),
                target_gid: Set(Some(gid)),
                mid: Set(mid),
                uid_of_msg: Set(uid_of_msg),
            };
            let result = read_index::Entity::insert(active_model)
                .on_conflict(
                    sea_query::OnConflict::columns([
                        read_index::Column::Uid,
                        read_index::Column::TargetGid,
                    ])
                    .update_column(read_index::Column::Mid)
                    .to_owned(),
                )
                .exec(&app_state.db)
                .await;
            if let Err(DbErr::RecordNotInserted) = result {
                // do nothing
            }
        }
    })
}
