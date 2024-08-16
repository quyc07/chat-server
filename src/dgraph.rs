use crate::app_state::AppState;
use crate::err::ServerError;
use crate::validate::ValidatedJson;
use crate::{AppRes, Res};
use axum::extract::State;
use axum::routing::{get, patch, post};
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
struct UserDgraph {
    name: String,
    phone: String,
}

const DGRAPH_URL: &str = "http://localhost:8080";

//
// {
//   "set": [
//     {
//       "name": "Bob",
//       "phone": "12345678"
//     }
//   ]
// }
async fn set(Json(req): Json<UserDgraph>) -> Res<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/mutate", DGRAPH_URL);
    // let mut map = HashMap::new();
    // map.insert("set", vec![req]);
    let value = json!({
        "name":req.name,
        "phone":req.phone,
        "dgraph.type":"User"
    });
    let param = value.as_object().ok_or(ServerError::CustomErr(
        "fail to convert req to map".to_string(),
    ))?;

    match client.post(url).json(param).send().await {
        Ok(res) => match res.text().await {
            Ok(res) => {
                info!(res);
                Ok(AppRes::success(()))
            }
            Err(err) => Err(ServerError::CustomErr(err.to_string())),
        },
        Err(err) => Err(ServerError::CustomErr(err.to_string())),
    }
}

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
        Ok(res) => match res.json::<QueryRes<UserDgraph>>().await {
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
//{
//   "data": {
//     "users": [
//       {
//         "uid": "0x4e2c",
//         "name": "Bob1"
//       }
//     ]
//   },
//   "extensions": {
//     "server_latency": {
//       "parsing_ns": 55125,
//       "processing_ns": 971458,
//       "encoding_ns": 55625,
//       "assign_timestamp_ns": 677583,
//       "total_ns": 1887375
//     },
//     "txn": {
//       "start_ts": 20359
//     },
//     "metrics": {
//       "num_uids": {
//         "_total": 2,
//         "dgraph.type": 0,
//         "name": 1,
//         "uid": 1
//       }
//     }
//   }
// }
#[derive(Debug, Deserialize, Serialize)]
struct QueryRes<T> {
    data: HashMap<String, Vec<T>>,
    extensions: Extensions,
}

#[derive(Debug, Deserialize, Serialize)]
struct Extensions {
    server_latency: ServerLatency,
    txn: Txn,
    metrics: Metrics,
}
#[derive(Debug, Deserialize, Serialize)]
struct ServerLatency {
    parsing_ns: i32,
    processing_ns: i32,
    encoding_ns: i32,
    assign_timestamp_ns: i32,
    total_ns: i32,
}
#[derive(Debug, Deserialize, Serialize)]
struct Txn {
    start_ts: i32,
}
#[derive(Debug, Deserialize, Serialize)]
struct Metrics {
    num_uids: NumUids,
}
#[derive(Debug, Deserialize, Serialize)]
struct NumUids {
    _total: i32,
    name: i32,
    phone: i32,
    uid: i32,
}
