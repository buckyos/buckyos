
use std::collections::hash_map::HashMap;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use crate::{Result,RPCErrors};

pub enum RPCSessionTokenType {
    Normal,
    JWT,
}

pub struct RPCSessionToken {
    pub token_type : RPCSessionTokenType,
    pub userid: Option<String>,
    pub appid: Option<String>,
    pub token: Option<String>,
    pub exp: Option<u64>,
}

impl RPCSessionToken {
    pub fn from_string(token: &str) -> Result<Self> {
        let have_dot = token.find('.');
        if have_dot.is_none() {
            return Ok(RPCSessionToken {
                token_type : RPCSessionTokenType::Normal,
                appid: None,
                userid: None,
                token: Some(token.to_string()),
                exp: None,
            });
        } else {
            return Ok(RPCSessionToken {
                token_type : RPCSessionTokenType::JWT,
                appid: None,
                userid: None,
                token: Some(token.to_string()),
                exp: None,
            });
        }
    }

    pub fn to_string(&self) -> String {
        match self.token_type {
            RPCSessionTokenType::Normal => {
                return self.token.as_ref().unwrap().to_string();
            }
            RPCSessionTokenType::JWT => {
                return self.token.as_ref().unwrap().to_string();
            }
        }
    }

    pub fn is_self_verify(&self) -> bool {
        match self.token_type {
            RPCSessionTokenType::Normal => {
                return false;
            }
            RPCSessionTokenType::JWT => {
                return true;
            }
        }
    }

    pub fn do_self_verify(&mut self,trust_keys:HashMap<String,DecodingKey>) -> Result<()> {
        if !self.is_self_verify() {
            return Err(RPCErrors::InvalidToken("Not a self verify token".to_string()));
        }
        if self.token.is_none() {
            return Err(RPCErrors::InvalidToken("Token is empty".to_string()));
        }

        let token_str = self.token.as_ref().unwrap();
        let header: jsonwebtoken::Header = jsonwebtoken::decode_header(token_str).map_err(|error| {
            RPCErrors::ReasonError("JWT decode header error".to_string())
        })?;

        let kid:String;
        if header.kid.is_none() {
            kid = "{owner}".to_string();
        } else {
            kid = header.kid.unwrap();
        }    
        let public_key = trust_keys.get(kid.as_str())
            .ok_or(RPCErrors::InvalidToken("No trust key".to_string()))?;
        let validation = Validation::new(header.alg);
        let decoded_token = decode::<serde_json::Value>(token_str, &public_key, &validation).map_err(
            |error| RPCErrors::InvalidToken(format!("JWT decode error:{}",error))
        )?;

        let decoded_json = decoded_token.claims.as_object()
            .ok_or(RPCErrors::InvalidToken("Invalid token".to_string()))?;

        let userid = decoded_json.get("userid")
            .ok_or(RPCErrors::InvalidToken("Missing userid".to_string()))?;
        let userid = userid.as_str().ok_or(RPCErrors::ReasonError("Invalid userid".to_string()))?;
        let appid = decoded_json.get("appid")
            .ok_or(RPCErrors::InvalidToken("Missing appid".to_string()))?;
        let appid = appid.as_str().ok_or(RPCErrors::ReasonError("Invalid appid".to_string()))?;
        let exp = decoded_json.get("exp");
        if exp.is_some() {
            let exp = exp.unwrap().as_u64().ok_or(RPCErrors::ReasonError("Invalid expire time".to_string()))?;
            self.exp = Some(exp);
        }

        self.userid = Some(userid.to_string());
        self.appid = Some(appid.to_string());
       

        Ok(())
    }
}

//store verified session tokens
pub struct SessionTokenManager {
    cache_tokens:HashMap<String, RPCSessionToken>,
}

impl SessionTokenManager {
    pub fn new() -> Self {
        SessionTokenManager {
            cache_tokens:HashMap::new(),
        }
    }
}

pub async fn request_session_token() -> String {
    unimplemented!();
}


pub async fn requst_verify_session_token(token: &str) -> bool {
    unimplemented!();
}