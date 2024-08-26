#![allow(dead_code)]
mod session_token;

use reqwest::{Client, ClientBuilder};
use std::time::{Duration,SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serde::ser::SerializeStruct;
use thiserror::Error;
pub enum RPCProtoclType {
    HttpPostJson,
}


#[derive(Debug, Deserialize,Serialize,PartialEq)]
pub struct RPCRequest {
    pub method: String,
    pub params: Value,
    //0: seq,1:token(option)
    pub sys:  Option<Vec<Value>>,
}

#[derive(Debug, PartialEq)]
pub enum RPCResult {
    Success(Value),
    Failed(String),
}



#[derive(Debug,PartialEq)]
pub struct RPCResponse {
    pub result: RPCResult,
    pub sys:  Option<Vec<Value>>,
}

impl Serialize for RPCResponse {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.result {
            RPCResult::Success(value) => {
                let mut state = serializer.serialize_struct("RPCResponse", 1)?;
                state.serialize_field("result", &value)?;
                if self.sys.is_some() {
                    state.serialize_field("sys", &self.sys)?;
                }
                
                state.end()
            }
            RPCResult::Failed(err) => {
                let mut state = serializer.serialize_struct("RPCResponse", 1)?;
                state.serialize_field("error", &err)?;
                if self.sys.is_some() {
                    state.serialize_field("sys", &self.sys)?;
                }
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RPCResponse {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        //let sys = v.get("sys").map(|v| v.clone());
        if let Some(value) = v.get("error") {
            return Ok(
                RPCResponse {
                    result: RPCResult::Failed(value.to_string()),
                    sys: None,
                }
            ) 
        } else {
            if let Some(value ) = v.get("result") {
                return Ok(
                    RPCResponse {
                        result: RPCResult::Success(value.clone()),
                        sys: None,
                    }
                )
            } else {
                return Ok(RPCResponse {
                    result : RPCResult::Success(Value::Null),
                    sys: None,
                });
            }
        }
        
    }
}

#[derive(Error, Debug)]
pub enum RPCErrors {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("Unknown method: {0}")]
    UnknownMethod(String),

}
pub type Result<T> = std::result::Result<T, RPCErrors>;
pub struct kRPC {
    client: Client,
    server_url: String,
    protcol_type:RPCProtoclType,
    seq:u64,
    session_token:Option<String>,
}

impl kRPC {
    pub fn new(url: &str) -> Self {
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
            seq:timestamp_millis,
            session_token:None,
        }
    }

    pub fn set_session_token(&mut self, token: &str) {
        self.session_token = Some(token.to_string());
    }

    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let request_body:Value;
        self.seq += 1;
        
        if self.session_token.is_some() {
            request_body = json!({
                "method": method,
                "params": params,
                "sys": [self.seq, self.session_token.clone().unwrap()]
            });
        } else {
            request_body = json!({
                "method": method,
                "params": params,
                "sys": [self.seq]
            });
        }
        
        let response = self.client
            .post(&self.server_url)
            .json(&request_body)
            .send()
            .await.map_err(|err| RPCErrors::ReasonError(format!("{}",err)))?;


        if response.status().is_success() {
            let rpc_response: Value = response.json().await.map_err(|err| RPCErrors::ReasonError(format!("{}",err)))?;
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


mod test {
    use super::*;
    #[test]
    fn test_encode_decode() {
        let req = RPCRequest {
            method: "add".to_string(),
            params: json!({"a":1,"b":2}),
            sys: Some(vec![json!(1023),json!("$token_abcdefg")]),
        };
        let encoded = serde_json::to_string(&req).unwrap();
        println!("encoded:{}",encoded);

        let decoded: RPCRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(req, decoded);
    }
}