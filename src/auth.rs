use std::fmt::Display;
use std::ops::Add;
use std::time::Duration;

use axum::{async_trait, RequestPartsExt};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::Router;
use axum::routing::post;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use axum_extra::TypedHeader;
use chrono::{DateTime, Local};
use jsonwebtoken::{decode, DecodingKey, encode, EncodingKey, Header, TokenData, Validation};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use entity::user;

use crate::{AppRes, Res};
use crate::app_state::AppState;
use crate::err::{ErrPrint, ServerError};

pub static KEYS: Lazy<Keys> = Lazy::new(|| {
    let secret = std::env::var("JWT_SECRET").unwrap_or("abc".to_string());
    Keys::new(secret.as_bytes())
});

#[derive(Debug, Serialize, Deserialize)]
pub struct Token {
    pub id: i32,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    // 失效时间，timestamp
    exp: i64,
}

impl From<user::Model> for Token {
    fn from(value: user::Model) -> Self {
        Token {
            id: value.id,
            name: value.name,
            email: value.email,
            phone: value.phone,
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
        // Extract the token from the authorization header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::InvalidToken)?;
        // Decode the user data
        let token_data = parse_token(bearer.token()).await?;

        Ok(token_data.claims)
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
    pub async fn route(app_state: AppState) -> Router {
        Router::new()
            .route("/renew", post(renew))
            .with_state(app_state)
    }
}


async fn renew(token: Token) -> Res<String> {
    let token = Token {
        exp: expire_timestamp(),
        ..token
    };
    Ok(AppRes::success(gen_token(token).await?))
}

const SECOND_TO_EXPIRED: u64 = 60 * 5;

fn expire_timestamp() -> i64 {
    Local::now().add(Duration::from_secs(SECOND_TO_EXPIRED)).timestamp()
}

pub async fn expire() -> DateTime<Local> {
    Local::now().add(Duration::from_secs(SECOND_TO_EXPIRED))
}

pub async fn gen_token(token: Token) -> Result<String, AuthError> {
    encode(&Header::default(), &token, &KEYS.encoding).map_err(|_| AuthError::TokenCreation)
}

pub async fn parse_token(token: &str) -> Result<TokenData<Token>, AuthError> {
    decode(token, &KEYS.decoding, &Validation::default()).map_err(|_| AuthError::InvalidToken)
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
    use std::time::Duration;

    use chrono::{DateTime, Local};
    use hmac::{Hmac, Mac};
    use jsonwebtoken::{decode, encode, Header, Validation};
    use jwt::{SignWithKey, VerifyWithKey};
    use serde::{Deserialize, Serialize};
    use sha2::Sha256;

    use crate::auth::{AuthError, KEYS, Keys, Token};

    #[test]
    fn test_token() {
        let keys = Keys::new("abc".as_bytes());
        let token = Token {
            id: 0,
            name: "name".to_string(),
            email: "email".to_string(),
            phone: None,
            exp: Local::now().add(Duration::from_secs(100)).timestamp(),
        };

        let encode_token = encode(&Header::default(), &token, &KEYS.encoding)
            .map_err(|_| AuthError::TokenCreation).unwrap();
        println!("{encode_token}");
        let token_data = decode::<Token>(&encode_token, &KEYS.decoding, &Validation::default())
            .map_err(|_| AuthError::InvalidToken).unwrap();
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
        let keys = Keys::new("123".as_bytes());
        let token_with_data = TokenWithData {
            data: String::from("abc"),
            expired_at: Local::now() + Duration::from_secs(100),
            token_type: TokenType::Token,
        };

        let encode_token = token_with_data.sign_with_key(&create_hmac_key("123")).unwrap();
        println!("{}", encode_token);
        let decode_token: TokenWithData<String> = encode_token.as_str().verify_with_key(&create_hmac_key("123")).unwrap();
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

