use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{AppDoc, AppType, SelectorType, UserInfo};

pub const VERIFY_HUB_UNIQUE_ID: &str = "verify-hub";
pub const VERIFY_HUB_SERVICE_NAME: &str = "verify-hub";
pub const VERIFY_HUB_TOKEN_EXPIRE_TIME: u64 = 60*10;//10 minutes
pub const VERIFY_HUB_SERVICE_PORT: u16 = 3210;

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
struct VerifyHubSettings {
    trust_keys: Vec<String>,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenPair {
    pub session_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginByJwtRequest {
    pub jwt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_params: Option<Value>,
}

impl LoginByJwtRequest {
    pub fn new(jwt: String, login_params: Option<Value>) -> Self {
        Self { jwt, login_params }
    }

    pub fn to_json(&self) -> Result<Value> {
        let mut params = Map::new();
        params.insert("type".to_string(), Value::String("jwt".to_string()));
        params.insert("jwt".to_string(), Value::String(self.jwt.clone()));

        if let Some(extra) = &self.login_params {
            if let Some(extra_obj) = extra.as_object() {
                for (key, value) in extra_obj {
                    params.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(Value::Object(params))
    }

    pub fn from_json(value: Value) -> Result<(String, Option<Value>)> {
        let mut params = value.as_object().cloned().ok_or_else(|| {
            RPCErrors::ParseRequestError("Expected object params for login".to_string())
        })?;

        let login_type = params
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("jwt");
        if login_type != "jwt" {
            return Err(RPCErrors::ParseRequestError(
                "login type must be jwt".to_string(),
            ));
        }

        let jwt = params
            .get("jwt")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing jwt".to_string()))?
            .to_string();

        params.remove("type");
        params.remove("jwt");

        let login_params = if params.is_empty() {
            None
        } else {
            Some(Value::Object(params))
        };

        Ok((jwt, login_params))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginByPasswordRequest {
    pub username: String,
    pub password: String,
    pub appid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl LoginByPasswordRequest {
    pub fn new(username: String, password: String, appid: String) -> Self {
        Self {
            username,
            password,
            appid,
            source_url: None,
        }
    }

    pub fn to_json(&self) -> Result<Value> {
        let mut params = Map::new();
        params.insert("type".to_string(), Value::String("password".to_string()));
        params.insert("username".to_string(), Value::String(self.username.clone()));
        params.insert("password".to_string(), Value::String(self.password.clone()));
        params.insert("appid".to_string(), Value::String(self.appid.clone()));
        Ok(Value::Object(params))
    }

    pub fn from_json(value: Value) -> Result<(String, String, String)> {
        let params = value.as_object().cloned().ok_or_else(|| {
            RPCErrors::ParseRequestError("Expected object params for login".to_string())
        })?;

        let login_type = params
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("jwt");
        if login_type != "password" {
            return Err(RPCErrors::ParseRequestError(
                "login type must be password".to_string(),
            ));
        }

        let username = params
            .get("username")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing username".to_string()))?
            .to_string();
        let password = params
            .get("password")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing password".to_string()))?
            .to_string();
        let appid = params
            .get("appid")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing appid".to_string()))?
            .to_string();

        Ok((username, password, appid))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginByPasswordResponse {
    pub user_info:UserInfo,
    pub session_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyTokenRequest {
    pub session_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appid: Option<String>,
}

impl VerifyTokenRequest {
    pub fn new(session_token: String, appid: Option<String>) -> Self {
        Self { session_token, appid }
    }

    pub fn to_json(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to serialize VerifyTokenRequest: {}",
                error
            ))
        })
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse VerifyTokenRequest: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

impl RefreshTokenRequest {
    pub fn new(refresh_token: String) -> Self {
        Self { refresh_token }
    }
    pub fn to_json(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to serialize RefreshTokenRequest: {}",
                error
            ))
        })
    }
    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse RefreshTokenRequest: {}",
                error
            ))
        })
    }
}

pub enum VerifyHubClient {
    InProcess(Box<dyn VerifyHubApiHandler>),
    KRPC(Box<kRPC>),
}

impl VerifyHubClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn VerifyHubApiHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn refresh_token(&self, refresh_jwt: &str) -> Result<TokenPair> {
        match self {
            Self::InProcess(handler) => handler.handle_refresh_token(refresh_jwt).await,
            Self::KRPC(client) => {
                let params = RefreshTokenRequest::new(refresh_jwt.to_string()).to_json()?;
                let result = client.call("refresh_token", params).await?;
                let token_pair: TokenPair = serde_json::from_value(result)
                    .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(token_pair)
            }
        }
    }

    pub async fn login_by_jwt(&self, jwt: &str, login_params: Option<Value>) -> Result<TokenPair> {
        match self {
            Self::InProcess(handler) => handler.handle_login_by_jwt(jwt, login_params).await,
            Self::KRPC(client) => {
                client.reset_session_token().await;
                let params = LoginByJwtRequest::new(jwt.to_string(), login_params).to_json()?;
                let result = client.call("login_by_jwt", params).await?;
                let token_pair: TokenPair = serde_json::from_value(result)
                    .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(token_pair)
            }
        }
    }

    pub async fn login_by_password(
        &self,
        username: String,
        password: String,
        appid: String,
        login_nonce: Option<u64>,
    ) -> Result<LoginByPasswordResponse> {
        match self {
            Self::InProcess(handler) => {
                let login_nonce = login_nonce.unwrap_or_else(current_login_nonce_millis);
                handler
                    .handle_login_by_password(&username, &password, &appid, login_nonce)
                    .await
            }
            Self::KRPC(client) => {
                client.reset_session_token().await;
                let params = LoginByPasswordRequest::new(username, password, appid).to_json()?;
                let result = client.call("login_by_password", params).await?;
                let login_by_password_response: LoginByPasswordResponse = serde_json::from_value(result)
                    .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(login_by_password_response)
            }
        }
    }

    pub async fn verify_token(&self, session_token: &str, appid: Option<&str>) -> Result<bool> {
        let appid = appid.map(|value| value.to_string());
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_verify_token(session_token, appid)
                    .await
            }
            Self::KRPC(client) => {
                let params = VerifyTokenRequest::new(session_token.to_string(), appid).to_json()?;
                let result = client.call("verify_token", params).await?;
                let value: bool = serde_json::from_value(result)
                    .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(value)
            }
        }
    }

    // Backward-compatible convenience: exchange JWT for a single session token.
    // pub async fn login_by_jwt_session_token(
    //     &self,
    //     jwt: String,
    //     login_params: Option<Value>,
    // ) -> Result<RPCSessionToken> {
    //     let token_pair = self.login_by_jwt(jwt, login_params).await?;
    //     RPCSessionToken::from_string(token_pair.session_token.as_str())
    // }
}

#[async_trait]
pub trait VerifyHubApiHandler: Send + Sync {
    async fn handle_login_by_jwt(
        &self,
        jwt: &str,
        login_params: Option<Value>,
    ) -> Result<TokenPair>;

    async fn handle_verify_token(
        &self,
        session_token: &str,
        appid: Option<String>,
    ) -> Result<bool>;


    async fn handle_refresh_token(
        &self,
        refresh_jwt: &str,
    ) -> Result<TokenPair>;

    async fn handle_login_by_password(
        &self,
        username: &str,
        password: &str,
        appid: &str,
        login_nonce: u64,
    ) -> Result<LoginByPasswordResponse>;
}

/// Adapter that exposes a VerifyHubHandler as a RPCHandler.
pub struct VerifyHubRpcHandler<T: VerifyHubApiHandler>(pub T);

impl<T: VerifyHubApiHandler> VerifyHubRpcHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: VerifyHubApiHandler> RPCHandler for VerifyHubRpcHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();

        let result = match req.method.as_str() {
            "login_by_jwt" => {
                let (jwt, login_params) = LoginByJwtRequest::from_json(req.params)?;
                let token_pair = self.0.handle_login_by_jwt(&jwt, login_params).await?;
                RPCResult::Success(
                    serde_json::to_value(token_pair)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
            }
            "login_by_password" => {
                let (username, password, appid) = LoginByPasswordRequest::from_json(req.params)?;
                let result = self
                    .0
                    .handle_login_by_password(&username, &password, &appid, req.seq)
                    .await?;
                RPCResult::Success(serde_json::to_value(result).map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?)
            }
            "refresh_token" => {
                let refresh_jwt = RefreshTokenRequest::from_json(req.params)?;
                let token_pair = self.0.handle_refresh_token(&refresh_jwt.refresh_token).await?;
                RPCResult::Success(serde_json::to_value(token_pair).map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?)
            }
            "verify_token" => {
                let verify_req = VerifyTokenRequest::from_json(req.params)?;
                let value = self
                    .0
                    .handle_verify_token(&verify_req.session_token, verify_req.appid)
                    .await?;
                RPCResult::Success(serde_json::to_value(value).map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?)
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

fn current_login_nonce_millis() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as u64
}

pub fn generate_verify_hub_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        VERIFY_HUB_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Verify Hub")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserType, UserState};
    use serde_json::json;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, Mutex};

    #[derive(Default, Debug)]
    struct MockCalls {
        login_jwt: Option<(String, Option<Value>)>,
        login_password: Option<(String, String, String, u64)>,
        verify_token: Option<(String, Option<String>)>,
        refresh_token: Option<String>,
    }

    #[derive(Clone)]
    struct MockVerifyHub {
        calls: Arc<Mutex<MockCalls>>,
    }

    #[async_trait]
    impl VerifyHubApiHandler for MockVerifyHub {
        async fn handle_login_by_jwt(
            &self,
            jwt: &str,
            login_params: Option<Value>,
        ) -> Result<TokenPair> {
            let mut calls = self.calls.lock().unwrap();
            calls.login_jwt = Some((jwt.to_string(), login_params));
            Ok(TokenPair {
                session_token: "session-1".to_string(),
                refresh_token: "refresh-1".to_string(),
            })
        }

        async fn handle_login_by_password(
            &self,
            username: &str,
            password: &str,
            appid: &str,
            login_nonce: u64,
        ) -> Result<LoginByPasswordResponse> {
            let mut calls = self.calls.lock().unwrap();
            calls.login_password = Some((
                username.to_string(),
                password.to_string(),
                appid.to_string(),
                login_nonce,
            ));
            Ok(LoginByPasswordResponse {
                user_info: UserInfo {
                    show_name: "mock".to_string(),
                    user_id: "mock_id".to_string(),
                    user_type: UserType::Admin,
                    state: UserState::Active,
                },
                session_token: "session-1".to_string(),
                refresh_token: "refresh-1".to_string(),
            })
        }

        async fn handle_refresh_token(
            &self,
            refresh_jwt: &str,
        ) -> Result<TokenPair> {
            let mut calls = self.calls.lock().unwrap();
            calls.refresh_token = Some(refresh_jwt.to_string());
            Ok(TokenPair {
                session_token: "session-2".to_string(),
                refresh_token: "refresh-2".to_string(),
            })
        }

        async fn handle_verify_token(
            &self,
            session_token: &str,
            appid: Option<String>,
        ) -> Result<bool> {
            let mut calls = self.calls.lock().unwrap();
            calls.verify_token = Some((session_token.to_string(), appid.clone()));
            Ok(true)
        }
    }

    #[test]
    fn test_generate_verify_hub_service_doc() {
        use super::generate_verify_hub_service_doc;
        let doc = generate_verify_hub_service_doc();
        let pkg_id = doc.get_package_id();
        let pkg_did = pkg_id.to_did();
        println!("pkg_id: {}", pkg_did.to_raw_host_name());
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }

    #[tokio::test]
    async fn test_in_process_client_with_mock() {
        let calls = Arc::new(Mutex::new(MockCalls::default()));
        let handler = MockVerifyHub {
            calls: calls.clone(),
        };
        let client = VerifyHubClient::new_in_process(Box::new(handler));

        let login_params = json!({"client": "mock"});
        let token_pair = client
            .login_by_jwt("jwt-1", Some(login_params.clone()))
            .await
            .unwrap();
        assert_eq!(token_pair.session_token, "session-1");
        assert_eq!(token_pair.refresh_token, "refresh-1");

        let verify_result = client.verify_token("session-1", Some("kernel")).await.unwrap();
        assert_eq!(verify_result,true);

        let calls = calls.lock().unwrap();
        let (jwt, params) = calls.login_jwt.clone().unwrap();
        assert_eq!(jwt, "jwt-1");
        assert_eq!(params, Some(login_params));
        let (session_token, appid) = calls.verify_token.clone().unwrap();
        assert_eq!(session_token, "session-1");
        assert_eq!(appid, Some("kernel".to_string()));
    }

    #[tokio::test]
    async fn test_rpc_handler_adapter_with_mock() {
        let calls = Arc::new(Mutex::new(MockCalls::default()));
        let handler = MockVerifyHub {
            calls: calls.clone(),
        };
        let rpc_handler = VerifyHubRpcHandler::new(handler);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        let login_req = RPCRequest {
            method: "login_by_jwt".to_string(),
            params: json!({"type": "jwt", "jwt": "jwt-2", "extra": "value"}),
            seq: 7,
            token: None,
            trace_id: None,
        };
        let login_resp = rpc_handler.handle_rpc_call(login_req, ip).await.unwrap();
        match login_resp.result {
            RPCResult::Success(value) => {
                let token_pair: TokenPair = serde_json::from_value(value).unwrap();
                assert_eq!(token_pair.session_token, "session-1");
                assert_eq!(token_pair.refresh_token, "refresh-1");
            }
            _ => panic!("Expected success response"),
        }

        let verify_req = RPCRequest {
            method: "verify_token".to_string(),
            params: json!({"session_token": "session-1", "appid": "kernel"}),
            seq: 8,
            token: None,
            trace_id: None,
        };
        let verify_resp = rpc_handler.handle_rpc_call(verify_req, ip).await.unwrap();
        match verify_resp.result {
            RPCResult::Success(value) => {
                let value: bool = serde_json::from_value(value).unwrap();
                assert_eq!(value, true);
            }
            _ => panic!("Expected success response"),
        }

        let calls = calls.lock().unwrap();
        let (jwt, params) = calls.login_jwt.clone().unwrap();
        assert_eq!(jwt, "jwt-2");
        assert_eq!(params, Some(json!({"extra": "value"})));
        let (session_token, appid) = calls.verify_token.clone().unwrap();
        assert_eq!(session_token, "session-1");
        assert_eq!(appid, Some("kernel".to_string()));
    }
}
