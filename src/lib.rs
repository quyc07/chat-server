use crate::app_state::AppState;
use crate::err::ServerError;
use axum::Router;
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

type Res<T> = Result<T, ServerError>;

#[derive(Deserialize, Validate)]
pub(crate) struct PageReq {
    #[validate(range(min = 1, message = "page should be larger than 0"))]
    pub(crate) page: u64,
    #[validate(range(min = 1, max = 10, message = "limit should be between 1 and 10"))]
    pub(crate) limit: u64,
}

impl PageReq {
    pub(crate) fn offset(&self) -> u64 {
        (self.page - 1) * self.limit
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
