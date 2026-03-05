#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
mod protocol;
mod session_token;
mod example_krpc_client;

pub use protocol::*;
pub use session_token::*;

use reqwest::{Client, ClientBuilder};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;

//TODO:整体设计基本与jsonrpc2.0一致，要考虑是否完全兼容

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
    #[error("Key not exist: {0}")]
    KeyNotExist(String),
    #[error("Service not valid: {0}")]
    ServiceNotValid(String),
}
pub type Result<T> = std::result::Result<T, RPCErrors>;
const DEFAULT_KRPC_TIMEOUT_SECS: u64 = 15;

pub struct kRPC {
    client: Client,
    server_url: String,
    protcol_type: RPCProtoclType,
    seq: RwLock<u64>,
    session_token: RwLock<Option<String>>,
    trace_id: RwLock<Option<String>>,
}

impl kRPC {
    pub fn new(url: &str, token: Option<String>) -> Self {
        Self::new_with_timeout_secs(url, token, DEFAULT_KRPC_TIMEOUT_SECS)
    }

    pub fn new_with_timeout_secs(url: &str, token: Option<String>, timeout_secs: u64) -> Self {
        let timeout_secs = timeout_secs.max(1);
        Self::new_with_timeout(url, token, Duration::from_secs(timeout_secs))
    }

    pub fn new_with_timeout(url: &str, token: Option<String>, timeout: Duration) -> Self {
        let start = SystemTime::now();
        let since_the_epoch = start
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");
        let timestamp_millis = since_the_epoch.as_secs() as u64 * 1_000_000;

        let client = ClientBuilder::new()
            //.https_only(true)
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .pool_max_idle_per_host(10)
            .timeout(timeout)
            .build()
            .expect("Failed to build reqwest client");

        kRPC {
            client,
            server_url: url.to_string(),
            protcol_type: RPCProtoclType::HttpPostJson,
            seq: RwLock::new(timestamp_millis),
            session_token: RwLock::new(token.clone()),
            trace_id: RwLock::new(None),
        }
    }

    pub async fn reset_session_token(&self) {
        let mut session_token = self.session_token.write().await;
        *session_token = None;
    }

    pub async fn get_session_token(&self) -> Option<String> {
        let session_token = self.session_token.read().await;
        session_token.clone()
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        //TODO: do auto-retry by config
        self._call(method, params).await
    }

    pub async fn set_context(&self, context: RPCContext) {
        let mut session_token = self.session_token.write().await;
        *session_token = context.token.clone();
        drop(session_token);
        
        let mut trace_id = self.trace_id.write().await;
        *trace_id = context.trace_id.clone();
        drop(trace_id);
    }

    async fn _call(&self, method: &str, params: Value) -> Result<Value> {
        let request_body: Value;
        let current_seq: u64;
        let current_trace_id: Option<String>;
        let current_session_token: Option<String>;
        {
            let mut seq = self.seq.write().await;
            *seq += 1;
            current_seq = *seq;
            drop(seq);

            let session_token = self.session_token.read().await;
            current_session_token = session_token.clone();
            let trace_id = self.trace_id.read().await;
            current_trace_id = trace_id.clone();

            let mut request = RPCRequest::new(method, params);
            request.seq = current_seq;
            request.token = current_session_token;
            request.trace_id = current_trace_id;
            request_body =
                serde_json::to_value(&request).map_err(|err| {
                    RPCErrors::ParseRequestError(format!("serialize request failed: {}", err))
                })?;
        }

        let response = self
            .client
            .post(&self.server_url)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| RPCErrors::ReasonError(format!("{}", err)))?;

        if response.status().is_success() {
            let rpc_response: RPCResponse = response
                .json()
                .await
                .map_err(|err| RPCErrors::ParserResponseError(format!("{}", err)))?;

            if rpc_response.seq != current_seq {
                return Err(RPCErrors::ParserResponseError(format!(
                    "seq not match: {}!={}",
                    rpc_response.seq, current_seq
                )));
            }

            match rpc_response.result {
                RPCResult::Success(value) => Ok(value),
                RPCResult::Failed(err) => Err(RPCErrors::ReasonError(format!(
                    "rpc call error: {}",
                    err
                ))),
            }
        } else {
            Err(RPCErrors::ReasonError(format!(
                "rpc call error: {}",
                response.status()
            )))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;
    #[test]
    fn test_encode_decode() {
        let req = RPCRequest {
            method: "add".to_string(),
            params: json!({"a":1,"b":2}),
            seq: 100,
            token: None,
            trace_id: Some("$trace_id".to_string()),
        };
        let encoded = serde_json::to_string(&req).unwrap();
        println!("req encoded:{}", encoded);

        let decoded: RPCRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(req, decoded);

        let resp = RPCResponse {
            result: RPCResult::Success(json!(100)),
            seq: 100,
            trace_id: Some("$trace_id".to_string()),
        };
        let encoded = serde_json::to_string(&resp).unwrap();
        println!("resp encoded:{}", encoded);
        let decoded: RPCResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(resp, decoded);

        let resp = RPCResponse {
            result: RPCResult::Failed("game over".to_string()),
            seq: 100,
            trace_id: None,
        };
        let encoded = serde_json::to_string(&resp).unwrap();
        println!("resp encoded:{}", encoded);
        let decoded: RPCResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(resp, decoded);
    }
}
