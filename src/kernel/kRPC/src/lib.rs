#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
mod session_token;
mod protocol;
pub use session_token::*;
pub use protocol::*;

use reqwest::{Client, ClientBuilder};
use std::time::{Duration,SystemTime, UNIX_EPOCH};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RPCErrors {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("Unknown method: {0}")]
    UnknownMethod(String),
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    #[error("Parse Request Error: {0}")]
    ParseRequestError(String),
    #[error("Parse Response Error: {0}")]
    ParserResponseError(String),
    #[error("Token expired:{0}")]
    TokenExpired(String),
    #[error("No permission:{0}")]
    NoPermission(String),
    #[error("Invalid password")]
    InvalidPassword,
    #[error("User Not Found:{0}")]
    UserNotFound(String),
}
pub type Result<T> = std::result::Result<T, RPCErrors>;
pub struct kRPC {
    client: Client,
    server_url: String,
    protcol_type:RPCProtoclType,
    seq:RwLock<u64>,
    session_token:RwLock<Option<String>>,
    init_token:Option<String>,
}

impl kRPC {
    pub fn new(url: &str,token:Option<String>) -> Self {
        let start = SystemTime::now();
        let since_the_epoch = start.duration_since(UNIX_EPOCH)
            .expect("Time went backwards");
        let timestamp_millis = since_the_epoch.as_secs() as u64 * 1_000_000;

        let client = ClientBuilder::new()
            //.https_only(true)
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .pool_max_idle_per_host(10)
            .timeout(Duration::from_secs(15))
            .build()
            .expect("Failed to build reqwest client");

        kRPC {
            client,
            server_url: url.to_string(),
            protcol_type:RPCProtoclType::HttpPostJson,
            seq:RwLock::new(timestamp_millis),
            session_token:RwLock::new(token.clone()),
            init_token:token.clone(),
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        //retry 2 times here.
        self._call(method, params).await
    }

    pub async fn _call(&self, method: &str, params: Value) -> Result<Value> {
        let request_body:Value;
        let current_seq:u64;
        {
            let mut seq = self.seq.write().await;
            *seq += 1;
            current_seq = *seq;

            let session_token = self.session_token.read().await;
            
            if session_token.is_some() {
                request_body = json!({
                    "method": method,
                    "params": params,
                    "sys": [*seq, session_token.as_ref().unwrap()]
                });
            } else {
                request_body = json!({
                    "method": method,
                    "params": params,
                    "sys": [*seq]
                });
            }
        }

        let response = self.client
            .post(&self.server_url)
            .json(&request_body)
            .send()
            .await.map_err(|err| RPCErrors::ReasonError(format!("{}",err)))?;

        if response.status().is_success() {
            let rpc_response: Value = response.json().await.map_err(|err| RPCErrors::ReasonError(format!("{}",err)))?;
            let sys_vec = rpc_response.get("sys");
            if sys_vec.is_some() {
                let sys = sys_vec.unwrap().as_array()
                    .ok_or(RPCErrors::ParserResponseError("sys is not array".to_string()))?;
                
                if sys.len() > 1 {
                    let seq = sys[0].as_u64()
                        .ok_or(RPCErrors::ParserResponseError("sys[0] is not u64".to_string()))?;
                    if seq != current_seq {
                        return Err(RPCErrors::ParserResponseError(format!("seq not match: {}!={}",seq,current_seq)));
                    }
                }
                if sys.len() > 2 {
                    let token = sys[1].as_str()
                        .ok_or(RPCErrors::ParserResponseError("sys[1] is not string".to_string()))?;
                    self.session_token.write().await.replace(token.to_string());    
                }
            }

            if rpc_response.get("error").is_some() {
                Err(RPCErrors::ReasonError(format!("rpc call error: {}", rpc_response.get("error").unwrap())))
            } else {
                Ok(rpc_response.get("result").unwrap().clone())
            }
        } else {
            Err(RPCErrors::ReasonError(format!("rpc call error: {}", response.status())))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_encode_decode() {
        let req = RPCRequest {
            method: "add".to_string(),
            params: json!({"a":1,"b":2}),
            seq: 100,
            token: Some("$dsdsd".to_string()),
            trace_id: Some("$trace_id".to_string()),
        };
        let encoded = serde_json::to_string(&req).unwrap();
        println!("req encoded:{}",encoded);

        let decoded: RPCRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(req, decoded);

        let resp = RPCResponse {
            result: RPCResult::Success(json!(100)),
            seq:100,
            token: Some("$3232323".to_string()),
            trace_id: Some("$trace_id".to_string()),
        };
        let encoded = serde_json::to_string(&resp).unwrap();
        println!("resp encoded:{}",encoded);
        let decoded: RPCResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(resp, decoded);


        let resp = RPCResponse {
            result: RPCResult::Failed("game over".to_string()),
            seq:100,
            token: None,
            trace_id: None,
        };
        let encoded = serde_json::to_string(&resp).unwrap();
        println!("resp encoded:{}",encoded);
        let decoded: RPCResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(resp, decoded);
    }
}