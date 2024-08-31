use crate::err::{ErrPrint, ServerError};
use crate::friend::FriendRegister;
use reqwest::Client;
use reqwest::Error;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::string::ToString;

const DGRAPH_URL: &str = "http://localhost:8080";

pub async fn register(ud: FriendRegister) -> Result<String, ServerError> {
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
    code: String,
    message: String,
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

impl ErrPrint for Error {}

/// 建立好友关系
pub async fn set_friend_ship(uid_1: String, uid_2: String) -> Result<(), ServerError> {
    let client = Client::new();
    let url = format!("{DGRAPH_URL}/mutate");
    // 开启事务
    let txn = do_set_friend_ship(
        SetFriendShip::new(uid_1.clone(), uid_2.clone()),
        client.clone(),
        url.clone(),
    )
    .await?;
    // 加入事务
    let url = format!("{url}?startTs={}", txn.start_ts);
    let txn = do_set_friend_ship(SetFriendShip::new(uid_2, uid_1), client.clone(), url).await?;
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
    let preds = txn
        .preds
        .ok_or(ServerError::CustomErr("未找到事务".to_string()))?;
    client
        .post(url)
        .json(&json!({
            "keys":keys,
            "preds":preds,
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
pub async fn is_friend(dgraph_uid: String, friend_id: i32) -> Result<bool, Error> {
    Ok(match get_friends(dgraph_uid.as_str()).await? {
        None => false,
        Some(friend_res) => friend_res
            .friend
            .unwrap_or(vec![])
            .iter()
            .find(|&friend| friend.user_id == friend_id)
            .is_some(),
    })
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Friend {
    pub uid: String,
    pub user_id: i32,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FriendRes {
    pub uid: String,
    pub user_id: i32,
    pub name: String,
    pub friend: Option<Vec<Friend>>,
}

pub async fn get_friends(dgraph_uid: &str) -> Result<Option<FriendRes>, Error> {
    let client = Client::new();
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
    use crate::friend::dgraph::{DgraphRes, FriendRes, UidData};
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
    pub preds: Option<Vec<String>>,
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
