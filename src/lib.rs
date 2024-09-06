use std::string::ToString;

use crate::app_state::AppState;
use crate::err::ServerError;
use axum::extract::FromRequest;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde::Serialize;
use utoipa::ToSchema;

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
pub mod admin;

pub trait Api {
    fn route(app_state: AppState) -> Router;
}
#[deprecated]
pub struct AppRouter(Vec<EveryRouter>);
#[deprecated]
enum EveryRouter {
    NeedLogin(Router, AppState),
    CheckStatus(Router, AppState),
    NotNeedLogin(Router, AppState),
}

impl AppRouter {
    pub fn route(&self) -> Router {
        self.0
            .iter()
            .map(|x| match x {
                EveryRouter::NeedLogin(router, app_state) => {
                    router
                        .clone()
                        .route_layer(axum::middleware::from_fn_with_state(
                            app_state.clone(),
                            middleware::check_user_status,
                        ))
                }
                EveryRouter::CheckStatus(router, app_state) => router
                    .clone()
                    .route_layer(axum::middleware::from_fn_with_state(
                        app_state.clone(),
                        middleware::check_user_status,
                    ))
                    .route_layer(axum::middleware::from_fn_with_state(
                        app_state.clone(),
                        middleware::check_user_status,
                    )),
                EveryRouter::NotNeedLogin(router, _) => router.clone(),
            })
            .fold(Router::new(), |acc, x| acc.merge(x))
    }
}

#[deprecated]
pub struct CheckRouter {
    need_login: Option<Router>,
    not_need_login: Option<Router>,
    app_state: AppState,
}

impl CheckRouter {
    pub fn route(&self) -> Router {
        let need_login = match self.need_login.clone() {
            None => None,
            Some(router) => Some(router.route_layer(axum::middleware::from_fn_with_state(
                self.app_state.clone(),
                middleware::check_user_status,
            ))),
        };
        match (need_login, self.not_need_login.clone()) {
            (Some(need_login_router), Some(not_need_login_router)) => {
                need_login_router.merge(not_need_login_router)
            }
            (Some(need_login_router), None) => need_login_router,
            (None, Some(not_need_login_router)) => not_need_login_router,
            (None, None) => panic!("need_login_router and not_need_login_router is None"),
        }
    }
}

// Create our own JSON extractor by wrapping `axum::Json`. This makes it easy to override the
// rejection and provide our own which formats errors to match our application.
//
// `axum::Json` responds with plain text if the input is invalid.
#[derive(FromRequest)]
#[from_request(via(axum::Json))]
pub struct AppJson<T>(pub T);

#[derive(Serialize,ToSchema)]
pub struct AppRes<T: Serialize> {
    code: i8,
    msg: String,
    data: T,
}

const FAIL_MESSAGE: &str = "系统异常请稍后再试";
const SUCCESS_MESSAGE: &str = "操作成功";
const FAIL_CODE: i8 = 1;
const SUCCESS_CODE: i8 = 0;

type Res<T> = Result<AppRes<T>, ServerError>;

impl<T: Serialize> AppRes<T> {
    pub fn success(data: T) -> AppRes<T> {
        AppRes {
            code: SUCCESS_CODE,
            msg: SUCCESS_MESSAGE.to_string(),
            data,
        }
    }
}

impl AppRes<()> {
    pub fn success_with_msg(msg: String) -> AppRes<()> {
        AppRes {
            code: SUCCESS_CODE,
            msg,
            data: (),
        }
    }
    pub fn fail_with_msg(msg: String) -> AppRes<()> {
        AppRes {
            code: FAIL_CODE,
            msg,
            data: (),
        }
    }
    pub fn fail() -> AppRes<()> {
        AppRes {
            code: FAIL_CODE,
            msg: FAIL_MESSAGE.to_string(),
            data: (),
        }
    }
}

impl<T: Serialize> IntoResponse for AppRes<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, String::from(self)).into_response()
    }
}

impl<T: Serialize> From<AppRes<T>> for String {
    fn from(value: AppRes<T>) -> Self {
        serde_json::to_string(&value).unwrap()
    }
}

#[cfg(test)]
mod test {
    use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, TimeZone, Utc};

    #[test]
    fn test_date() {
        println!("{}", serde_json::to_string(&Local::now()).unwrap());
        println!("{}", Local::now().format("%Y-%m-%d %H:%M:%S"));
        let date_time: DateTime<Utc> = Utc.with_ymd_and_hms(2017, 04, 02, 12, 50, 32).unwrap();
        let formatted = format!("{}", date_time.format("%d/%m/%Y %H:%M"));
        let local = format!("{}", Local::now().format("%d/%m/%Y %H:%M"));
        let time = Utc::now();
        assert_eq!(formatted, "02/04/2017 12:50");
        println!("{}", time);
        println!("{}", local);
        let timestamp_millis = NaiveDateTime::from_timestamp_opt(Local::now().timestamp(), 0);
        println!("{}", timestamp_millis.unwrap());
        let offset = Local.offset_from_utc_datetime(&timestamp_millis.unwrap());
        println!("{}", offset);
        println!(
            "{}",
            "2024-07-16 10:00:00Z".parse::<DateTime<Local>>().unwrap()
        );
        println!("{}", FixedOffset::east_opt(8 * 3600).unwrap());
    }
}
