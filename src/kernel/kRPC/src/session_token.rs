
use std::collections::hash_map::HashMap;
use serde::{Serialize,Deserialize};
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use log::*;
use serde_json::json;
use crate::{Result,RPCErrors};

#[derive(Clone, Debug, Serialize, Deserialize,PartialEq)]
pub enum RPCSessionTokenType {
    Normal,
    JWT,
}

impl Default for RPCSessionTokenType {
    fn default() -> Self {
        RPCSessionTokenType::JWT
    }
}

#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct RPCSessionToken {
    #[serde(skip_serializing,skip_deserializing)]
    pub token_type : RPCSessionTokenType,
    #[serde(skip_serializing,skip_deserializing)]
    pub token: Option<String>,

    pub appid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    pub userid: Option<String>,
}

impl RPCSessionToken {
    pub fn from_string(token: &str) -> Result<Self> {
        let have_dot = token.find('.');
        if have_dot.is_none() {
            return Ok(RPCSessionToken {
                token_type : RPCSessionTokenType::Normal,
                nonce: None,
                appid: None,
                userid: None,   
                token: Some(token.to_string()),
                iss: None,
                exp: None,
            });
        } else {
            return Ok(RPCSessionToken {
                token_type : RPCSessionTokenType::JWT,
                nonce: None,
                appid: None,
                userid: None,
                token: Some(token.to_string()),
                iss: None,
                exp: None,
            });
           
        }
    }

    pub fn get_values(&self) -> Result<(String,String)> {
        if self.userid.is_none() || self.appid.is_none() {
            return Err(RPCErrors::InvalidToken("Invalid token: userid or appid is none".to_string()));
        }
        Ok((self.userid.as_ref().unwrap().to_string(),self.appid.as_ref().unwrap().to_string()))
    }

    pub fn to_string(&self) -> String {
        match self.token_type {
            RPCSessionTokenType::Normal => {
                return self.token.as_ref().unwrap().to_string();
            }
            RPCSessionTokenType::JWT => {
                if self.token.is_none() {
                    //let jwt_token 
                    return "".to_string();
                } else {
                    return self.token.as_ref().unwrap().to_string();
                }
            }
        }
    }

    pub fn generate_jwt(&self,kid:Option<String>,private_key:&EncodingKey) -> Result<String> {
        let mut header = Header::new(Algorithm::EdDSA);        
        header.kid = kid;
        header.typ = None;
        let payload = serde_json::to_value(self).map_err(|op| RPCErrors::ReasonError(format!("encode to JSON error:{}",op)))?;
        //info!("header: {:?}",header);
        //info!("payload: {:?}",payload);
        let token = encode(&header, &payload, private_key)
            .map_err(|op| RPCErrors::ReasonError(format!("JWT encode error:{}",op)))?;
        Ok(token)
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

    pub fn do_self_verify(&mut self,trust_keys:&HashMap<String,DecodingKey>) -> Result<()> {
        if !self.is_self_verify() {
            return Err(RPCErrors::InvalidToken("Not a self verify token".to_string()));
        }
        if self.token.is_none() {
            return Err(RPCErrors::InvalidToken("Token is empty".to_string()));
        }

        let token_str = self.token.as_ref().unwrap();
        let header: jsonwebtoken::Header = jsonwebtoken::decode_header(token_str).map_err(|error| {
            RPCErrors::ReasonError(format!("JWT decode header error : {}",error))
        })?;

        let kid:String;
        if header.kid.is_none() {
            kid = "{verify_hub}".to_string();
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
        info!("decoded token: {:?}",decoded_json);

        let userid = decoded_json.get("userid")
            .ok_or(RPCErrors::InvalidToken("Missing userid".to_string()))?;
        let userid = userid.as_str().ok_or(RPCErrors::ReasonError("Invalid userid".to_string()))?;
        let appid = decoded_json.get("appid");
        if appid.is_some() {
            let appid = appid.unwrap();
            if appid.is_null() {
                self.appid = None;
            } else {
                let appid = appid.as_str().ok_or(RPCErrors::ReasonError("Invalid appid".to_string()))?;
                self.appid = Some(appid.to_string());
            }
        }

        let iss = decoded_json.get("iss");
        if iss.is_some() {
            let iss = iss.unwrap();
            if iss.is_null() {
                self.iss = None;
            } else {
                self.iss = Some(iss.as_str().unwrap().to_string());
            }
        }

        let exp = decoded_json.get("exp");
        if exp.is_some() {
            let exp = exp.unwrap();
            if exp.is_null() {
                self.exp = None;
            } else {
                let exp = exp.as_u64().ok_or(RPCErrors::ReasonError("Invalid expire time".to_string()))?;
                self.exp = Some(exp);
            }
        }

        self.userid = Some(userid.to_string());
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


pub async fn requst_verify_session_token(_token: &str) -> bool {
    unimplemented!();
}