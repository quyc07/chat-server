use crate::app_state::AppState;
use crate::auth::AuthError;
use crate::err::{ErrPrint, ServerError};
use crate::validate::ValidatedJson;
use crate::{AppRes, Res};
use axum::extract::State;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use itertools::Itertools;
use moka::ops::compute::Op;
use reqwest::{Client, Error, Response};
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
struct UidData<T> {
    user: Vec<T>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DgraphRes<T> {
    data: T,
    extensions: Extensions,
}

pub struct FriendShip {
    pub uid_1: String,
    pub uid_2: String,
}

impl ErrPrint for Error {}

/// 建立好友关系
pub async fn set_friend_ship(friend_ship: FriendShip) -> Result<(), ServerError> {
    let client = Client::new();
    let url = format!("{DGRAPH_URL}/mutate");
    // 开启事务
    let txn = do_set_friend_ship(
        SetFriendShip::new(friend_ship.uid_1.clone(), friend_ship.uid_2.clone()),
        client.clone(),
        url.clone(),
    )
    .await?;
    // 加入事务
    let url = format!("{url}?startTs={}", txn.start_ts);
    let txn = do_set_friend_ship(
        SetFriendShip::new(friend_ship.uid_2, friend_ship.uid_1),
        client.clone(),
        url,
    )
    .await?;
    // 提交事务
    commit(txn).await?;
    Ok(())
}

/// 提交dgraph的事务
async fn commit(txn: Txn) -> Result<(), ServerError> {
    let client = Client::new();
    let url = format!("{DGRAPH_URL}/commit?startTs={}", txn.start_ts);
    let keys = txn
        .keys
        .ok_or(ServerError::CustomErr("未找到事务".to_string()))?;
    client
        .post(url)
        .json(&json!({
            "keys":keys,
            "preds":txn.preds,
        }))
        .send()
        .await?;
    Ok(())
}

async fn do_set_friend_ship(
    set_friend_ship: SetFriendShip,
    client: Client,
    url: String,
) -> Result<Txn, ServerError> {
    let res = client
        .post(url)
        .json(&set_friend_ship)
        .send()
        .await?
        .json::<DgraphRes<MutateData<HashMap<String, String>>>>()
        .await?;
    Ok(res.extensions.txn)
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

/// 查询用户好友关系
/// {
///   user(func: uid("0x4e37")) {
///     uid
///     name
///     friend {
///       uid,
///       name
///     }
///   }
/// }
pub(crate) async fn is_friend(dgraph_uid: Option<String>, friend_id: i32) -> Result<bool, Error> {
    match dgraph_uid {
        None => Ok(false),
        Some(dgraph_uid) => Ok(match get_friends(dgraph_uid.as_str()).await? {
            None => false,
            Some(friendRes) => friendRes
                .friends
                .unwrap_or(vec![])
                .iter()
                .find(|&friend| {
                    friend
                        .user_id
                        .map(|user_id| user_id == friend_id)
                        .or(Some(false))
                        .unwrap()
                })
                .is_some(),
        }),
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct Friend {
    uid: String,
    user_id: Option<i32>,
    name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct FriendRes {
    uid: String,
    user_id: Option<i32>,
    name: String,
    friends: Option<Vec<Friend>>,
}

pub(crate) async fn get_friends(dgraph_uid: &str) -> Result<Option<FriendRes>, Error> {
    let client = reqwest::Client::new();
    let url = format!("{}/query", DGRAPH_URL);
    let value = "
    {
        user(func: uid("
        .to_string()
        + "\""
        + dgraph_uid
        + "\""
        + ")) {
            uid
            name
            user_id
            friend {
                uid,
                name,
                user_id
            }
        }
    }";
    let res = client
        .post(url)
        .body(value)
        .header("Content-type", "application/dql")
        .send()
        .await?;
    let res = res.json::<DgraphRes<UidData<FriendRes>>>().await?;
    Ok(res.data.user.first().map(|t| t.clone()))
}

#[cfg(test)]
mod test {
    use crate::dgraph::{DgraphRes, FriendRes, UidData};
    use serde_json::json;

    #[test]
    fn test() {
        let value = json!({
          "data": {
            "user": [
              {
                "uid": "0x4e42",
                "name": "andy"
              }
            ]
          },
          "extensions": {
            "server_latency": {
              "parsing_ns": 107000,
              "processing_ns": 877959,
              "encoding_ns": 77250,
              "assign_timestamp_ns": 661417,
              "total_ns": 1801625
            },
            "txn": {
              "start_ts": 21044
            },
            "metrics": {
              "num_uids": {
                "": 1,
                "_total": 5,
                "friend": 1,
                "name": 1,
                "uid": 1,
                "user_id": 1
              }
            }
          }
        });
        let result =
            serde_json::from_slice::<DgraphRes<UidData<FriendRes>>>(value.to_string().as_ref());
        println!("{:?}", result)
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Txn {
    pub start_ts: i64,
    pub commit_ts: Option<i64>,
    pub keys: Option<Vec<String>>,
    pub preds: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ServerLatency {
    pub parsing_ns: i64,
    pub processing_ns: i64,
    pub assign_timestamp_ns: Option<i64>,
    pub total_ns: i64,
}

#[derive(Serialize, Deserialize, Debug)]
struct Extensions {
    pub server_latency: ServerLatency,
    pub txn: Txn,
}
