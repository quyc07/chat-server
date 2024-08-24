use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Add;
use std::sync::LazyLock;
use std::time::Duration;

use axum::extract::{FromRequest, FromRequestParts, State};
use axum::http::request::Parts;
use axum::routing::{delete, post};
use axum::Router;
use axum::{async_trait, RequestPartsExt};
use axum_extra::headers::authorization::Bearer;
use axum_extra::headers::Authorization;
use axum_extra::TypedHeader;
use chrono::{DateTime, Local};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, TokenData, Validation};
use moka::future::Cache;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;
use validator::Validate;

use crate::app_state::AppState;
use crate::err::{ErrPrint, ServerError};
use crate::validate::ValidatedJson;
use crate::{user, AppRes, Res};

const KEYS: LazyLock<Keys, fn() -> Keys> = LazyLock::new(|| {
    let secret = std::env::var("JWT_SECRET").unwrap_or("abc".to_string());
    Keys::new(secret.as_bytes())
});

/// 当前已登陆用户集合，替换成moka 缓存
// static LOGIN_USER: Lazy<Arc<Mutex<HashMap<i32, Token>>>> =
//     Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

// const 修饰的变量是只读的，运行时无法修改，因此该缓存只能使用static修饰
static LOGIN_USER: LazyLock<Cache<i32, Token>> = LazyLock::new(|| {
    Cache::builder()
        // 空闲时间与jwt过期时间保持一致
        .time_to_idle(Duration::from_secs(SECOND_TO_EXPIRED))
        .build()
});

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Token {
    pub id: i32,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub dgraph_uid: String,
    // 失效时间，timestamp
    exp: i64,
}

impl From<entity::user::Model> for Token {
    fn from(value: entity::user::Model) -> Self {
        Token {
            id: value.id,
            name: value.name,
            email: value.email,
            phone: value.phone,
            dgraph_uid: value.dgraph_uid,
            exp: expire_timestamp(),
        }
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for Token
where
    S: Send + Sync,
{
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.extract::<TypedHeader<Authorization<Bearer>>>().await {
            Ok(TypedHeader(Authorization(bearer))) => {
                let token_data = parse_token(bearer.token()).await?;
                // 判断是否是已登陆用户，LOGIN_USER的内存过期时间与token的expire时间一致，因此只需判断是否存在即可
                match LOGIN_USER.get(&token_data.claims.id).await {
                    None => Err(ServerError::from(AuthError::InvalidToken)),
                    Some(_) => Ok(token_data.claims),
                }
            }
            Err(_) => {
                let query = parts.uri.query().unwrap_or_default();
                let value: HashMap<String, String> =
                    serde_html_form::from_str(query).map_err(|_| AuthError::InvalidToken)?;
                let token = value.get("token").ok_or(AuthError::InvalidToken)?.as_str();
                let token_data = parse_token(token).await?;
                // 判断是否是已登陆用户，LOGIN_USER的内存过期时间与token的expire时间一致，因此只需判断是否存在即可
                match LOGIN_USER.get(&token_data.claims.id).await {
                    None => Err(ServerError::from(AuthError::InvalidToken)),
                    Some(_) => Ok(token_data.claims),
                }
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("用户名或密码错误")]
    WrongCredentials,
    #[error("登录参数丢失")]
    MissingCredentials,
    #[error("Token创建失败")]
    TokenCreation,
    #[error("无效的Token")]
    InvalidToken,
}

impl ErrPrint for AuthError {}

impl From<AuthError> for String {
    fn from(err: AuthError) -> Self {
        AppRes::fail_with_msg(err.to_string()).into()
    }
}

pub struct TokenApi;

impl TokenApi {
    pub fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/renew", post(renew))
            .route("/login", post(login))
            .route("/logout", delete(logout))
            .with_state(app_state)
    }
}

#[derive(Debug, Deserialize, Validate)]
struct UserLoginReq {
    #[validate(length(min = 1))]
    name: String,
    #[validate(length(min = 1))]
    password: String,
}

#[derive(Debug, Serialize)]
struct UserLoginRes {
    access_token: String,
    access_token_expires: DateTime<Local>,
}

async fn login(
    State(app_state): State<AppState>,
    ValidatedJson(req): ValidatedJson<UserLoginReq>,
) -> Res<UserLoginRes> {
    let user = user::find_by_name(&app_state, &req.name).await?.unwrap();
    if user.password != req.password {
        return Err(ServerError::from(AuthError::WrongCredentials));
    }
    // Create the authorization token
    let token = Token::from(user);
    let access_token = gen_token(&token).await?;
    // 保存已登陆用户
    LOGIN_USER.insert(token.id, token).await;
    // Send the authorized token
    Ok(AppRes::success(UserLoginRes {
        access_token,
        access_token_expires: expire().await,
    }))
}

async fn logout(token: Token) -> Res<()> {
    // 删除已登陆用户
    LOGIN_USER.remove(&token.id).await;
    Ok(AppRes::success(()))
}

async fn renew(token: Token) -> Res<String> {
    let token = Token {
        exp: expire_timestamp(),
        ..token
    };
    let access_token = gen_token(&token).await?;
    // 刷新已登陆用户token
    LOGIN_USER.insert(token.id, token).await;
    Ok(AppRes::success(access_token))
}

const SECOND_TO_EXPIRED: u64 = 60 * 5;

fn expire_timestamp() -> i64 {
    Local::now()
        .add(Duration::from_secs(SECOND_TO_EXPIRED))
        .timestamp()
}

pub async fn expire() -> DateTime<Local> {
    Local::now().add(Duration::from_secs(SECOND_TO_EXPIRED))
}

pub async fn gen_token(token: &Token) -> Result<String, AuthError> {
    encode(&Header::default(), token, &KEYS.encoding).map_err(|_| AuthError::TokenCreation)
}

pub async fn parse_token(token: &str) -> Result<TokenData<Token>, AuthError> {
    let mut validation = Validation::default();
    // 修改leeway=0，让exp校验使用绝对时间，参考Validation.leeway的使用
    validation.leeway = 0;
    decode(token, &KEYS.decoding, &validation).map_err(|_| AuthError::InvalidToken)
}

pub struct Keys {
    pub(crate) encoding: EncodingKey,
    pub(crate) decoding: DecodingKey,
}

impl Keys {
    fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }
}

#[cfg(test)]
mod test {
    use std::ops::Add;
    use std::thread::sleep;
    use std::time::Duration;

    use chrono::{DateTime, Local};
    use hmac::{Hmac, Mac};
    use jsonwebtoken::{decode, encode, Header, Validation};
    use jwt::{SignWithKey, VerifyWithKey};
    use serde::{Deserialize, Serialize};
    use sha2::Sha256;

    use crate::auth::{AuthError, Token, KEYS};

    #[test]
    fn test_token() {
        let token = Token {
            id: 0,
            name: "name".to_string(),
            email: "email".to_string(),
            phone: None,
            dgraph_uid: Default::default(),
            exp: Local::now().add(Duration::from_secs(3)).timestamp(),
        };

        let encode_token = encode(&Header::default(), &token, &KEYS.encoding)
            .map_err(|_| AuthError::TokenCreation)
            .unwrap();
        println!("{encode_token}");
        sleep(Duration::from_secs(2));
        let mut validation = Validation::default();
        // 修改leeway=0，让exp校验使用绝对时间，参考Validation.leeway的使用
        validation.leeway = 0;
        let token_data = decode::<Token>(&encode_token, &KEYS.decoding, &validation)
            .map_err(|_| AuthError::InvalidToken)
            .unwrap();
        println!("{:?}", token_data.claims)
    }

    #[derive(Serialize, Deserialize, Debug)]
    enum TokenType {
        Token,
        Refresh,
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct TokenWithData<T> {
        data: T,
        expired_at: DateTime<Local>,
        token_type: TokenType,
    }

    #[test]
    fn test_token_custom_expire() {
        let token_with_data = TokenWithData {
            data: String::from("abc"),
            expired_at: Local::now() + Duration::from_secs(100),
            token_type: TokenType::Token,
        };

        let encode_token = token_with_data
            .sign_with_key(&create_hmac_key("123"))
            .unwrap();
        println!("{}", encode_token);
        let decode_token: TokenWithData<String> = encode_token
            .as_str()
            .verify_with_key(&create_hmac_key("123"))
            .unwrap();
        // let decode_token =
        //     VerifyWithKey::<Token>::verify_with_key(&*encode_token, &create_hmac_key("123")).unwrap();
        if decode_token.expired_at < Local::now() {
            println!("expired exp={}", decode_token.expired_at);
        }
        println!("{:?}", decode_token);
    }

    fn create_hmac_key(server_key: &str) -> Hmac<Sha256> {
        Hmac::<Sha256>::new_from_slice(server_key.as_bytes()).expect("invalid server key")
    }

    #[test]
    fn test_date() {
        let time = Local::now();
        println!("{}", time);
        println!("{:?}", time.timezone());
        let time = Local::now();
        println!("{}", time);
        println!("{:?}", time.timezone());
    }
}

pub(crate) async fn delete_login_status(user_id: i32) {
    LOGIN_USER.remove(&user_id).await;
}
