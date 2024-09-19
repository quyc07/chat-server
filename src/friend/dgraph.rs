use crate::err::{ErrPrint, ServerError};
use crate::friend::FriendRegister;
use reqwest::Client;
use reqwest::Error;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fmt::{Display, Formatter};
use std::string::ToString;
use std::sync::LazyLock;

static DGRAPH_URL: DgraphUrl = DgraphUrl(LazyLock::new(|| {
    env::var("DGRAPH_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}));

struct DgraphUrl(LazyLock<String>);

impl Display for DgraphUrl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.to_string())
    }
}

pub async fn register(fr: FriendRegister) -> Result<String, ServerError> {
    let client = reqwest::Client::new();
    // 直接提交事务 参考：https://dgraph.io/docs/dql/clients/raw-http/#committing-the-transaction
    let url = format!("{DGRAPH_URL}/mutate?commitNow=true");
    let value = json!({
        "set":[
            {
                "name":fr.name,
                "user_id":fr.user_id,
                "phone":fr.phone,
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
            Err(err) => Err(err.into()),
        },
        Err(err) => Err(err.into()),
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
struct UserData<T> {
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
pub(crate) struct FriendVo {
    pub uid: String,
    pub user_id: i32,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct GetFriendRes {
    pub uid: String,
    pub user_id: i32,
    pub name: String,
    pub loc: Option<Loc>,
    pub friend: Option<Vec<FriendVo>>,
}

pub async fn get_friends(dgraph_uid: &str) -> Result<Option<GetFriendRes>, Error> {
    let client = Client::new();
    let url = format!("{DGRAPH_URL}/query");
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
            loc
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
    let res = res.json::<DgraphRes<UserData<GetFriendRes>>>().await?;
    Ok(res.data.user.first().map(|t| t.clone()))
}

#[cfg(test)]
mod test {
    use crate::friend::dgraph::{DgraphRes, GetFriendRes, UserData};
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
            serde_json::from_slice::<DgraphRes<UserData<GetFriendRes>>>(value.to_string().as_ref());
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Loc {
    #[serde(rename = "type")]
    pub r#type: String,
    pub coordinates: Vec<f64>,
}

#[derive(Serialize, Deserialize)]
struct SetLoc {
    pub uid: String,
    pub loc: Loc,
}

#[derive(Serialize, Deserialize)]
struct Mutate<T> {
    pub set: Vec<T>,
}
pub(crate) struct Point {
    pub long: f64,
    pub lat: f64,
}
pub(crate) enum Location {
    Point(Point),
    Polygon(Vec<Point>),
    MultiPolygon(Vec<Vec<Point>>),
}
pub(crate) async fn set_loc(uid: String, loc: Location) -> Result<(), ServerError> {
    let client = Client::new();
    let url = format!("{DGRAPH_URL}/mutate?commitNow=true");
    client
        .post(url)
        .json(&Mutate {
            set: vec![SetLoc {
                uid,
                loc: match loc {
                    Location::Point(Point { long, lat }) => Loc {
                        r#type: "Point".to_string(),
                        coordinates: vec![long, lat],
                    },
                    Location::Polygon(_) => todo!("待实现区域设置"),
                    Location::MultiPolygon(_) => todo!(),
                },
            }],
        })
        .send()
        .await?
        .json::<DgraphRes<MutateData<HashMap<String, String>>>>()
        .await?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct NearByData<T> {
    nearby: Vec<T>,
}
pub(crate) async fn nearby(loc: Location, radius: i32) -> Result<Vec<FriendVo>, ServerError> {
    let client = Client::new();
    let url = format!("{DGRAPH_URL}/query");
    let body = match loc {
        Location::Point(Point { long, lat }) => {
            "
   {
       nearby(func: near(loc, "
                .to_string()
                + &format!("[{long},{lat}]")
                + ", "
                + radius.to_string().as_str()
                + ") ) {
           uid,
           name,
           user_id
       }
   }"
        }
        Location::Polygon(_) => {
            todo!()
        }
        Location::MultiPolygon(_) => {
            todo!()
        }
    };
    let res = client
        .post(url)
        .body(body)
        .header("Content-type", "application/dql")
        .send()
        .await?;
    let res = res.json::<DgraphRes<NearByData<FriendVo>>>().await?;
    Ok(res.data.nearby)
}
