use crate::app_state::AppState;
use crate::auth::AuthError;
use crate::err::{ErrPrint, ServerError};
use crate::validate::ValidatedJson;
use crate::{AppRes, Res};
use axum::extract::State;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use reqwest::{Error, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::string::ToString;
use tracing::info;
use utoipa::ToSchema;
use validator::Validate;

pub struct DgraphApi;

impl DgraphApi {
    pub fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/user", post(set))
            .route("/user/all", get(all))
            .with_state(app_state)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UserDgraph {
    pub(crate) name: String,
    pub(crate) phone: Option<String>,
}

const DGRAPH_URL: &str = "http://localhost:8080";

/// mutate request:
/// {
///   "set": [
///     {
///       "name": "Bob2",
///       "phone": "123456781",
///   	  "dgraph.type":"User",
///       "uid":"_:user"
/// 	}
///   ]
/// }
/// response:
/// {
///   "data": {
///     "code": "Success",
///     "message": "Done",
///     "queries": null,
///     "uids": {
///       "user": "0x4e2d"
///     }
///   },
///   "extensions": {
///     "server_latency": {
///       "parsing_ns": 74250,
///       "processing_ns": 1840625,
///       "assign_timestamp_ns": 783792,
///       "total_ns": 2947625
///     },
///     "txn": {
///       "start_ts": 20387,
///       "commit_ts": 20388,
///       "preds": [
///         "1-0-dgraph.type",
///         "1-0-name",
///         "1-0-phone"
///       ]
///     }
///   }
/// }
///
async fn set(Json(req): Json<UserDgraph>) -> Res<String> {
    let client = reqwest::Client::new();
    // 直接提交事务 参考：https://dgraph.io/docs/dql/clients/raw-http/#committing-the-transaction
    let url = format!("{}/mutate?commitNow=true", DGRAPH_URL);
    let value = json!({
        "set":[
            {
                "name":req.name,
                "phone":req.phone,
                "dgraph.type":"User",
                "uid":"_:uid"
            }
        ]
    });
    match client.post(url).json(&value).send().await {
        Ok(res) => match res
            .json::<DgraphRes<MutateData<HashMap<String, String>>>>()
            .await
        {
            Ok(res) => match res.data.uids.get("uid") {
                None => Err(ServerError::CustomErr("fail to set user".to_string())),
                Some(uid) => Ok(AppRes::success(uid.clone())),
            },
            Err(err) => Err(ServerError::CustomErr(err.to_string())),
        },
        Err(err) => Err(ServerError::CustomErr(err.to_string())),
    }
}

pub async fn register(ud: UserDgraph) -> Result<String, ServerError> {
    let client = reqwest::Client::new();
    // 直接提交事务 参考：https://dgraph.io/docs/dql/clients/raw-http/#committing-the-transaction
    let url = format!("{}/mutate?commitNow=true", DGRAPH_URL);
    let value = json!({
        "set":[
            {
                "name":ud.name,
                "phone":ud.phone,
                "dgraph.type":"User",
                "uid":"_:uid"
            }
        ]
    });
    match client.post(url).json(&value).send().await {
        Ok(res) => match res
            .json::<DgraphRes<MutateData<HashMap<String, String>>>>()
            .await
        {
            Ok(res) => match res.data.uids.get("uid") {
                None => Err(ServerError::CustomErr("fail to set user".to_string())),
                Some(uid) => Ok(uid.clone()),
            },
            Err(err) => Err(ServerError::from(err)),
        },
        Err(err) => Err(ServerError::from(err)),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Uid {
    uid: String,
}

/// query request:
/// {
///     users(func: type(User)) {
///         uid
///         name
///         phone
///     }
/// }
/// response:
/// {
///   "data": {
///     "users": [
///       {
///         "uid": "0x4e2c",
///         "name": "Bob1"
///       }
///     ]
///   },
///   "extensions": {
///     "server_latency": {
///       "parsing_ns": 55125,
///       "processing_ns": 971458,
///       "encoding_ns": 55625,
///       "assign_timestamp_ns": 677583,
///       "total_ns": 1887375
///     },
///     "txn": {
///       "start_ts": 20359
///     },
///     "metrics": {
///       "num_uids": {
///         "_total": 2,
///         "dgraph.type": 0,
///         "name": 1,
///         "uid": 1
///       }
///     }
///   }
/// }
async fn all() -> Res<Vec<UserDgraph>> {
    let client = reqwest::Client::new();
    let url = format!("{}/query", DGRAPH_URL);
    let body = "
        {
    users(func: type(User)) {
        uid
        name
        phone
    }
    }";
    match client
        .post(url)
        .header("Content-type", "application/dql")
        .body(body)
        .send()
        .await
    {
        Ok(res) => match res
            .json::<DgraphRes<HashMap<String, Vec<UserDgraph>>>>()
            .await
        {
            Ok(res) => {
                info!("{}", format!("{:?}", res));
                match res.data.get("users") {
                    None => Ok(AppRes::success(vec![])),
                    Some(users) => Ok(AppRes::success(users.clone())),
                }
            }
            Err(err) => Err(ServerError::CustomErr(err.to_string())),
        },
        Err(err) => Err(ServerError::CustomErr(err.to_string())),
    }
}

/// {
///     "code": "Success",
///     "message": "Done",
///     "queries": null,
///     "uids": {
///       "user": "0x4e2d"
///     }
/// }
#[derive(Debug, Deserialize, Serialize)]
struct MutateData<T> {
    uids: T,
}

#[derive(Debug, Deserialize, Serialize)]
struct DgraphRes<T> {
    data: T,
}

struct FriendShip {
    uid_1: String,
    uid_2: String,
}

impl ErrPrint for Error {}

/// 建立好友关系
pub async fn set_friend_ship(friend_ship: FriendShip) -> Result<(), ServerError> {
    let client = reqwest::Client::new();
    let url = format!("{}/mutate?commitNow=true", DGRAPH_URL);
    let set_friend_ship = SetFriendShip::new(friend_ship.uid_1, friend_ship.uid_2);
    let result = client.post(url).json(&set_friend_ship).send().await;
    match result {
        Ok(res) => match res.text().await {
            Ok(_) => Ok(()),
            Err(err) => Err(ServerError::from(err)),
        },
        Err(err) => Err(ServerError::from(err)),
    }
}

#[derive(Serialize, Deserialize)]
struct Subject {
    pub uid: String,
}

#[derive(Serialize, Deserialize)]
struct Object {
    pub uid: String,
    pub friend: Vec<Subject>,
}

#[derive(Serialize, Deserialize)]
struct SetFriendShip {
    pub set: Vec<Object>,
}

impl SetFriendShip {
    fn new(object_id: String, subject_id: String) -> Self {
        SetFriendShip {
            set: vec![Object {
                uid: object_id,
                friend: vec![Subject { uid: subject_id }],
            }],
        }
    }
}

pub(crate) fn is_friend(uid: i32) -> bool {
    return false
}
