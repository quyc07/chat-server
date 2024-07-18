use crate::app_state::AppState;
use crate::auth::Token;
use crate::user::UserApi;
use crate::{AppRes, Res};
use axum::extract::State;
use axum::routing::{delete, get, post, put};
use axum::Router;
use entity::group;
use entity::group::Model;
use entity::prelude::{Group, User};
use sea_orm::{EntityTrait, QueryFilter};
use serde::Serialize;
use utoipa::openapi::SchemaFormat;
use utoipa::{OpenApi, ToSchema};

#[derive(OpenApi)]
#[openapi(
    paths(
        all
    ),
    components(
        schemas( AllRes)
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
            // .route("/create", post(create))
            // .route("/:gid/add/:uid/", put(add))
            // .route("/:gid/remove/:gid", delete(remove))
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
