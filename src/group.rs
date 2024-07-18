use crate::app_state::AppState;
use crate::auth::Token;
use crate::user::UserApi;
use crate::validate::ValidatedJson;
use crate::{AppRes, Res};
use axum::extract::State;
use axum::routing::{delete, get, post, put};
use axum::Router;
use entity::group;
use entity::group::Model;
use entity::prelude::{Group, User};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::openapi::SchemaFormat;
use utoipa::{OpenApi, ToSchema};
use validator::Validate;

#[derive(OpenApi)]
#[openapi(
    paths(
        all,create
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

#[derive(Deserialize, Validate, ToSchema)]
struct CreateReq {
    #[validate(length(min = 1, message = "Group name must be at least one letter"))]
    name: String,
    #[validate(range(min = 1, message = "Admin id must larger or equal than 1"))]
    admin: i32,
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
    _: Token,
    ValidatedJson(req): ValidatedJson<CreateReq>,
) -> Res<i32> {
    let group = group::ActiveModel {
        id: Default::default(),
        name: Set(req.name),
        admin: Set(req.admin),
        c_time: Default::default(),
        u_time: Default::default(),
    };
    let group = group.insert(&app_state.db).await?;
    Ok(AppRes::success(group.id))
}
