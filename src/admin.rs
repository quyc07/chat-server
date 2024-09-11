use crate::app_state::AppState;
use crate::auth::Token;
use crate::{middleware, Api, Res};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use entity::prelude::User;
use entity::user::Model;
use sea_orm::EntityTrait;

pub struct AdminApi;

impl Api for AdminApi {
    fn route(app_state: AppState) -> Router {
        Router::new()
            .nest("/user", Router::new().route("/", get(all)))
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                middleware::check_admin,
            ))
            .with_state(app_state.clone())
    }
}

async fn all(State(app_state): State<AppState>, _: Token) -> Res<Json<Vec<Model>>> {
    Ok(Json(User::find().all(&app_state.db).await?))
}
