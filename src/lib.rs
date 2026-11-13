use crate::admin::AdminApi;
use crate::app_state::AppState;
use crate::auth::TokenApi;
use crate::err::ServerError;
use crate::event::EventApi;
use crate::friend::FriendApi;
use crate::group::GroupApi;
use crate::open_api::swagger_ui;
use crate::read_index::ReadIndexApi;
use crate::user::UserApi;
use axum::Router;
use axum::routing::get;
use serde::Deserialize;
use validator::Validate;

pub mod admin;
pub mod app_state;
pub mod auth;
pub mod datetime;
pub mod err;
pub mod event;
pub mod friend;
pub mod group;
pub mod log;
pub mod message;
pub mod middleware;
pub mod open_api;
pub mod read_index;
pub mod user;
pub mod validate;

pub trait Api {
    fn route(app_state: AppState) -> Router;
}

/// Build the full HTTP router for `app_state`.
pub async fn router(app_state: AppState) -> Router {
    Router::new()
        .merge(swagger_ui().await)
        .route("/", get(|| async { "Hello, World!" }))
        .nest("/admin", AdminApi::route(app_state.clone()))
        .nest("/user", UserApi::route(app_state.clone()))
        .nest("/group", GroupApi::route(app_state.clone()))
        .nest("/token", TokenApi::route(app_state.clone()))
        .nest("/event", EventApi::route(app_state.clone()))
        .nest("/friend", FriendApi::route(app_state.clone()))
        .nest("/ri", ReadIndexApi::route(app_state))
}

type Res<T> = Result<T, ServerError>;

#[derive(Deserialize, Validate)]
pub(crate) struct PageReq {
    #[validate(range(min = 1, message = "page should be larger than 0"))]
    pub(crate) page: u64,
    #[validate(range(min = 1, max = 50, message = "limit should be between 1 and 50"))]
    pub(crate) limit: u64,
}

impl PageReq {
    pub(crate) fn offset(&self) -> u64 {
        (self.page - 1) * self.limit
    }
}

#[cfg(test)]
mod test {
    use chrono::{DateTime, FixedOffset, Local, TimeZone, Utc};

    #[test]
    fn test_date() {
        println!("{}", serde_json::to_string(&Local::now()).unwrap());
        println!("{}", Local::now().format("%Y-%m-%d %H:%M:%S"));
        let date_time: DateTime<Utc> = Utc.with_ymd_and_hms(2017, 4, 2, 12, 50, 32).unwrap();
        let formatted = format!("{}", date_time.format("%d/%m/%Y %H:%M"));
        let local = format!("{}", Local::now().format("%d/%m/%Y %H:%M"));
        let time = Utc::now();
        assert_eq!(formatted, "02/04/2017 12:50");
        println!("{}", time);
        println!("{}", local);
        let timestamp = DateTime::from_timestamp(Local::now().timestamp(), 0).unwrap();
        println!("{}", timestamp);
        let offset = Local.offset_from_utc_datetime(&timestamp.naive_utc());
        println!("{}", offset);
        println!(
            "{}",
            "2024-07-16 10:00:00Z".parse::<DateTime<Local>>().unwrap()
        );
        println!("{}", FixedOffset::east_opt(8 * 3600).unwrap());
    }
}
