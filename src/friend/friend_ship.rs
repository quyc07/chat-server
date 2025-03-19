use crate::app_state::AppState;
use crate::user::{get_by_id, UserErr};
use crate::Res;
use entity::friend_ship;
use entity::prelude::FriendShip;
use futures::{stream, StreamExt};
use migration::Condition;
use sea_orm::QueryFilter;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};

pub async fn set_friend_ship(app_state: &AppState, uid_1: i32, uid_2: i32) -> Res<()> {
    // Implementation goes here
    let friend_ship = entity::friend_ship::ActiveModel {
        id: Default::default(),
        user_id_1: Set(uid_1),
        user_id_2: Set(uid_2),
        c_time: Default::default(),
    };
    friend_ship.insert(&app_state.db).await?;
    Ok(())
}

pub async fn is_friend(app_state: &AppState, uid: i32, friend_id: i32) -> Res<bool> {
    let exist = FriendShip::find()
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(friend_ship::Column::UserId1.eq(uid))
                        .add(friend_ship::Column::UserId2.eq(friend_id)),
                )
                .add(
                    Condition::all()
                        .add(friend_ship::Column::UserId2.eq(uid))
                        .add(friend_ship::Column::UserId1.eq(friend_id)),
                ),
        )
        .one(&app_state.db)
        .await?;
    Ok(exist.is_some())
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct FriendVo {
    pub user_id: i32,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct GetFriendRes {
    pub user_id: i32,
    pub name: String,
    pub loc: Option<Loc>,
    pub friend: Option<Vec<FriendVo>>,
}

pub async fn get_friends(app_state: &AppState, uid: i32) -> Res<Option<GetFriendRes>> {
    match get_by_id(uid, app_state).await? {
        None => Err(UserErr::UserNotExist(uid).into()),
        Some(user) => {
            let friend_ships = FriendShip::find()
                .filter(
                    Condition::any()
                        .add(friend_ship::Column::UserId1.eq(uid))
                        .add(friend_ship::Column::UserId2.eq(uid)),
                )
                .all(&app_state.db)
                .await?;
            let friends = stream::iter(friend_ships)
                .filter_map(|fs| async move {
                    let user_id = if fs.user_id_1 == uid {
                        fs.user_id_2
                    } else {
                        fs.user_id_1
                    };
                    match get_by_id(user_id, app_state).await {
                        Ok(Some(user)) => Some(FriendVo {
                            user_id,
                            name: user.name.clone(),
                        }),
                        _ => None,
                    }
                })
                .collect::<Vec<FriendVo>>()
                .await;
            Ok(Some(GetFriendRes {
                user_id: uid,
                name: user.name.clone(),
                loc: None,
                friend: Some(friends),
            }))
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Loc {
    #[serde(rename = "type")]
    pub r#type: String,
    pub coordinates: Vec<f64>,
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

pub(crate) async fn set_loc(app_state: &AppState, uid: i32, loc: Location) -> Res<()> {
    todo!()
}

pub(crate) async fn nearby(app_state: &AppState, loc: Location, radius: i32) -> Res<Vec<FriendVo>> {
    todo!()
}
