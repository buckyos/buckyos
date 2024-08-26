
use std::collections::hash_map::HashMap;

pub struct RPCSessionToken {
    userid: Option<String>,
    appid: Option<String>,
    token: Option<String>,
    expire_time: Option<u64>,
}

impl RPCSessionToken {
    pub fn from_string(token: &str) -> Self {
        let parts: Vec<&str> = token.split('_').collect();
        let appid = parts[1].to_string();
        let userid = parts[2].to_string();
        let expire_time = parts[3].parse::<u64>().unwrap();
        
        RPCSessionToken {
            appid: Some(appid),
            userid: Some(userid),
            token: None,
            expire_time: Some(expire_time),
        }
    }

    pub fn is_self_verify(&self) -> bool {
        unimplemented!();
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



    pub fn add_token(&mut self, token: &str) {
        let session_token = RPCSessionToken::from_string(token);
        self.cache_tokens.insert(token.to_string(), session_token);
    }

    pub fn verify_token(&self, token: &str) -> bool {
        if self.cache_tokens.contains_key(token) {
            let session_token = self.cache_tokens.get(token).unwrap();
            if session_token.is_self_verify() {
                return true;
            }
        }
        false
    }
}

pub async fn request_session_token() -> String {
    unimplemented!();
}


pub async fn requst_verify_session_token(token: &str) -> bool {
    unimplemented!();
}