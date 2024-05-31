use std::string::ToString;

use axum::extract::FromRequest;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Local};
use serde::Serialize;

use crate::err::ServerError;

pub mod app_state;
pub mod auth;
pub mod err;
pub mod event;
pub mod log;
pub mod user;
pub mod validate;

// Create our own JSON extractor by wrapping `axum::Json`. This makes it easy to override the
// rejection and provide our own which formats errors to match our application.
//
// `axum::Json` responds with plain text if the input is invalid.
#[derive(FromRequest)]
#[from_request(via(axum::Json))]
pub struct AppJson<T>(pub T);

#[derive(Serialize)]
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
mod test{
    use chrono::{DateTime, Local, TimeZone, Utc};

    #[test]
    fn test_date() {
        println!("{}", serde_json::to_string(&Local::now()).unwrap());
        println!("{}", Local::now().format("%Y-%m-%d %H:%M:%S"));
        let date_time: DateTime<Utc> = Utc.with_ymd_and_hms(2017, 04, 02, 12, 50, 32).unwrap();
        let formatted = format!("{}", date_time.format("%d/%m/%Y %H:%M"));
        let formatted1 = format!("{}", Local::now().format("%d/%m/%Y %H:%M"));
        assert_eq!(formatted, "02/04/2017 12:50");
        println!("{}", formatted1);
    }

}

