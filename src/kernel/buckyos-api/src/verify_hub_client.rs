use ::kRPC::*;
use async_trait::async_trait;
use jsonwebtoken::EncodingKey;
use name_lib::DID;
use rand::random;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{AccessDenial, AppDoc, AppType, AuthMethod, AuthTarget, SelectorType, UserInfo};

pub const VERIFY_HUB_UNIQUE_ID: &str = "verify-hub";
pub const VERIFY_HUB_SERVICE_NAME: &str = "verify-hub";
pub const VERIFY_HUB_TOKEN_EXPIRE_TIME: u64 = 60 * 10; //10 minutes
pub const VERIFY_HUB_SERVICE_PORT: u16 = 3210;

pub fn generate_service_login_assertion(
    subject_id: &str,
    appid: &str,
    device_name: &str,
    device_private_key: &EncodingKey,
) -> Result<(String, RPCSessionToken)> {
    let subject_id = subject_id.trim();
    let appid = appid.trim();
    let device_name = device_name.trim();
    if subject_id.is_empty() || appid.is_empty() || device_name.is_empty() {
        return Err(RPCErrors::ReasonError(
            "subject_id, appid and device_name are required".to_string(),
        ));
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut login_token = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        token: None,
        aud: None,
        exp: Some(now + VERIFY_HUB_TOKEN_EXPIRE_TIME * 2),
        iss: Some(device_name.to_string()),
        jti: Some(random::<u64>().to_string()),
        sub: Some(subject_id.to_string()),
        appid: Some(appid.to_string()),
        sudo: false,
        extra: HashMap::new(),
    };
    let jwt = login_token.generate_jwt(Some(device_name.to_string()), device_private_key)?;
    login_token.token = Some(jwt.clone());
    Ok((jwt, login_token))
}

pub fn generate_user_login_assertion(
    user_id: &str,
    target_appid: &str,
    user_private_key: &EncodingKey,
) -> Result<(String, RPCSessionToken)> {
    let user_id = user_id.trim();
    let target_appid = target_appid.trim();
    if user_id.is_empty() || target_appid.is_empty() {
        return Err(RPCErrors::ReasonError(
            "user_id and target_appid are required".to_string(),
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut assertion = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        token: None,
        aud: None,
        exp: Some(now + VERIFY_HUB_TOKEN_EXPIRE_TIME * 2),
        iss: Some(user_id.to_string()),
        jti: Some(random::<u64>().to_string()),
        sub: Some(user_id.to_string()),
        appid: Some(target_appid.to_string()),
        sudo: false,
        extra: HashMap::new(),
    };
    let jwt = assertion.generate_jwt(None, user_private_key)?;
    assertion.token = Some(jwt.clone());
    Ok((jwt, assertion))
}

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
#[serde(deny_unknown_fields)]
pub struct LoginByJwtParams {
    pub target: AuthTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginByJwtRequest {
    #[serde(rename = "type")]
    login_type: String,
    pub jwt: String,
    #[serde(flatten)]
    pub login_params: LoginByJwtParams,
}

impl LoginByJwtRequest {
    pub fn new(jwt: String, target: AuthTarget) -> Self {
        Self {
            login_type: "jwt".to_string(),
            jwt,
            login_params: LoginByJwtParams { target },
        }
    }

    pub fn to_json(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to serialize LoginByJwtRequest: {error}"))
        })
    }

    pub fn from_json(value: Value) -> Result<Self> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse LoginByJwtRequest: {error}"))
        })?;
        if request.login_type != "jwt" {
            return Err(RPCErrors::ParseRequestError(
                "login type must be jwt".to_string(),
            ));
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginByPasswordRequest {
    #[serde(rename = "type")]
    login_type: String,
    pub username: String,
    pub password: String,
    pub target: AuthTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl LoginByPasswordRequest {
    pub fn new(
        username: String,
        password: String,
        target: AuthTarget,
        login_nonce: Option<u64>,
    ) -> Self {
        Self {
            login_type: "password".to_string(),
            username,
            password,
            target,
            login_nonce,
            source_url: None,
        }
    }

    pub fn to_json(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to serialize LoginByPasswordRequest: {error}"
            ))
        })
    }

    pub fn from_json(value: Value) -> Result<Self> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse LoginByPasswordRequest: {error}"))
        })?;
        if request.login_type != "password" {
            return Err(RPCErrors::ParseRequestError(
                "login type must be password".to_string(),
            ));
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginByPasswordResponse {
    pub user_info: UserInfo,
    pub session_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SudoByPasswordRequest {
    pub username: String,
    pub password: String,
    pub target: AuthTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_nonce: Option<u64>,
}

impl SudoByPasswordRequest {
    pub fn new(
        username: String,
        password: String,
        target: AuthTarget,
        aud: Option<String>,
        login_nonce: Option<u64>,
    ) -> Self {
        Self {
            username,
            password,
            target,
            aud,
            login_nonce,
        }
    }

    pub fn to_json(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to serialize SudoByPasswordRequest: {error}"
            ))
        })
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse SudoByPasswordRequest: {error}"))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SudoByPasswordResponse {
    pub session_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyTokenRequest {
    pub session_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_target: Option<AuthTarget>,
}

impl VerifyTokenRequest {
    pub fn new(session_token: String, expected_target: Option<AuthTarget>) -> Self {
        Self {
            session_token,
            expected_target,
        }
    }

    pub fn to_json(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to serialize VerifyTokenRequest: {}", error))
        })
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse VerifyTokenRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
            RPCErrors::ParseRequestError(format!("Failed to parse RefreshTokenRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegateUserSessionRequest {
    pub source_session_token: String,
    pub target: AuthTarget,
}

impl DelegateUserSessionRequest {
    pub fn new(source_session_token: String, target: AuthTarget) -> Self {
        Self {
            source_session_token,
            target,
        }
    }

    pub fn to_json(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to serialize DelegateUserSessionRequest: {error}"
            ))
        })
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse DelegateUserSessionRequest: {error}"
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DelegateUserSessionResult {
    Issued(TokenPair),
    SourceInactive,
    AccessDenied,
}

// ─── hybrid SSO (doc/arch/verify-hub/混合SSO技术契约.md §5, §9) ─────────────

/// `login_by_exclusive_invite`: Control Panel only; the browser never calls it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusiveInviteLoginRequest {
    pub invite_id: String,
    pub secret: String,
    pub target: AuthTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExclusiveInviteLoginResult {
    Issued {
        user_id: String,
        token_pair: TokenPair,
    },
    Denied {
        denial: AccessDenial,
    },
}

/// `revoke_families`: end the subject's families matching every given filter,
/// except the one `except_session_token` belongs to. Control Panel only.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RevokeFamiliesRequest {
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<AuthMethod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_invite_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub except_session_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeFamiliesResult {
    pub revoked: u32,
}

/// Public profile a canonical zone shares with the target zone (Q-05).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FederationProfile {
    pub show_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

/// Canonical side, after consent: Control Panel only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFederationCodeRequest {
    pub user_id: String,
    pub did: String,
    pub client_zone: String,
    pub redirect_uri: String,
    pub nonce: String,
    pub code_challenge: String,
    pub profile: FederationProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueFederationCodeResult {
    pub code: String,
    pub expires_in: u64,
}

/// Canonical side, public over HTTPS: the code is the credential and PKCE
/// binds it to the target zone's pending login.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExchangeFederationCodeRequest {
    pub code: String,
    pub code_verifier: String,
    pub client_zone: String,
    pub redirect_uri: String,
}

pub const FEDERATION_PURPOSE_LOGIN: &str = "login";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FederationAssertion {
    pub did: String,
    pub client_zone: String,
    pub nonce: String,
    pub purpose: String,
    pub issued_at: u64,
    pub profile: FederationProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExchangeFederationCodeResult {
    Ok { assertion: FederationAssertion },
    InvalidCode,
}

/// Target side: Control Panel calls it from `/sso/did_callback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginByRemoteVerifyHubRequest {
    pub did: String,
    pub code: String,
    pub code_verifier: String,
    pub nonce: String,
    pub redirect_uri: String,
    pub target: AuthTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginByRemoteVerifyHubResult {
    Issued {
        /// The local account the DID maps to; `None` for an External session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_id: Option<String>,
        token_pair: TokenPair,
        profile: FederationProfile,
    },
    Denied {
        denial: AccessDenial,
    },
    /// Resolution of the canonical zone or the exchange call failed; retryable.
    CanonicalUnavailable,
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

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => client.set_context(context).await,
        }
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

    pub async fn logout(&self, refresh_jwt: &str) -> Result<bool> {
        match self {
            Self::InProcess(handler) => handler.handle_logout(refresh_jwt).await,
            Self::KRPC(client) => {
                let params = RefreshTokenRequest::new(refresh_jwt.to_string()).to_json()?;
                let result = client.call("logout", params).await?;
                let ok: bool = serde_json::from_value(result)
                    .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(ok)
            }
        }
    }

    pub async fn login_by_jwt(&self, jwt: &str, target: AuthTarget) -> Result<TokenPair> {
        match self {
            Self::InProcess(handler) => handler.handle_login_by_jwt(jwt, &target).await,
            Self::KRPC(client) => {
                client.reset_session_token().await;
                let params = LoginByJwtRequest::new(jwt.to_string(), target).to_json()?;
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
        target: AuthTarget,
        login_nonce: Option<u64>,
    ) -> Result<LoginByPasswordResponse> {
        match self {
            Self::InProcess(handler) => {
                let login_nonce = login_nonce.unwrap_or_else(current_login_nonce_millis);
                handler
                    .handle_login_by_password(&username, &password, &target, login_nonce)
                    .await
            }
            Self::KRPC(client) => {
                client.reset_session_token().await;
                let params = LoginByPasswordRequest::new(username, password, target, login_nonce)
                    .to_json()?;
                let result = client.call("login_by_password", params).await?;
                let login_by_password_response: LoginByPasswordResponse =
                    serde_json::from_value(result)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(login_by_password_response)
            }
        }
    }

    pub async fn sudo_by_password(
        &self,
        username: String,
        password: String,
        target: AuthTarget,
        aud: Option<String>,
        login_nonce: Option<u64>,
    ) -> Result<SudoByPasswordResponse> {
        match self {
            Self::InProcess(handler) => {
                let login_nonce = login_nonce.unwrap_or_else(current_login_nonce_millis);
                handler
                    .handle_sudo_by_password(&username, &password, &target, aud, login_nonce)
                    .await
            }
            Self::KRPC(client) => {
                client.reset_session_token().await;
                let login_nonce = login_nonce.or_else(|| Some(current_login_nonce_millis()));
                let params =
                    SudoByPasswordRequest::new(username, password, target, aud, login_nonce)
                        .to_json()?;
                let result = client.call("sudo_by_password", params).await?;
                let sudo_by_password_response: SudoByPasswordResponse =
                    serde_json::from_value(result)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(sudo_by_password_response)
            }
        }
    }

    pub async fn delegate_user_session(
        &self,
        source_session_token: &str,
        target: AuthTarget,
    ) -> Result<DelegateUserSessionResult> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_delegate_user_session(None, source_session_token, &target)
                    .await
            }
            Self::KRPC(client) => {
                let params =
                    DelegateUserSessionRequest::new(source_session_token.to_string(), target)
                        .to_json()?;
                let result = client.call("delegate_user_session", params).await?;
                serde_json::from_value(result)
                    .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))
            }
        }
    }

    async fn call_typed<R: serde::de::DeserializeOwned>(
        client: &kRPC,
        method: &str,
        params: impl Serialize,
    ) -> Result<R> {
        let params = serde_json::to_value(params)
            .map_err(|e| RPCErrors::ReasonError(format!("serialize {method}: {e}")))?;
        let result = client.call(method, params).await?;
        serde_json::from_value(result).map_err(|e| RPCErrors::ParserResponseError(e.to_string()))
    }

    pub async fn login_by_exclusive_invite(
        &self,
        request: ExclusiveInviteLoginRequest,
    ) -> Result<ExclusiveInviteLoginResult> {
        match self {
            Self::InProcess(handler) => handler.handle_login_by_exclusive_invite(None, request).await,
            Self::KRPC(client) => Self::call_typed(client, "login_by_exclusive_invite", request).await,
        }
    }

    pub async fn revoke_families(&self, request: RevokeFamiliesRequest) -> Result<RevokeFamiliesResult> {
        match self {
            Self::InProcess(handler) => handler.handle_revoke_families(None, request).await,
            Self::KRPC(client) => Self::call_typed(client, "revoke_families", request).await,
        }
    }

    pub async fn issue_federation_code(
        &self,
        request: IssueFederationCodeRequest,
    ) -> Result<IssueFederationCodeResult> {
        match self {
            Self::InProcess(handler) => handler.handle_issue_federation_code(None, request).await,
            Self::KRPC(client) => Self::call_typed(client, "issue_federation_code", request).await,
        }
    }

    pub async fn exchange_federation_code(
        &self,
        request: ExchangeFederationCodeRequest,
    ) -> Result<ExchangeFederationCodeResult> {
        match self {
            Self::InProcess(handler) => handler.handle_exchange_federation_code(request).await,
            Self::KRPC(client) => {
                client.reset_session_token().await;
                Self::call_typed(client, "exchange_federation_code", request).await
            }
        }
    }

    pub async fn login_by_remote_verify_hub(
        &self,
        request: LoginByRemoteVerifyHubRequest,
    ) -> Result<LoginByRemoteVerifyHubResult> {
        match self {
            Self::InProcess(handler) => handler.handle_login_by_remote_verify_hub(None, request).await,
            Self::KRPC(client) => Self::call_typed(client, "login_by_remote_verify_hub", request).await,
        }
    }

    pub async fn verify_token(
        &self,
        session_token: &str,
        expected_target: Option<AuthTarget>,
    ) -> Result<bool> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_verify_token(session_token, expected_target)
                    .await
            }
            Self::KRPC(client) => {
                let params = VerifyTokenRequest::new(session_token.to_string(), expected_target)
                    .to_json()?;
                let result = client.call("verify_token", params).await?;
                let value: bool = serde_json::from_value(result)
                    .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
                Ok(value)
            }
        }
    }
}

#[async_trait]
pub trait VerifyHubApiHandler: Send + Sync {
    async fn handle_login_by_jwt(&self, jwt: &str, target: &AuthTarget) -> Result<TokenPair>;

    async fn handle_verify_token(
        &self,
        session_token: &str,
        expected_target: Option<AuthTarget>,
    ) -> Result<bool>;

    async fn handle_refresh_token(&self, refresh_jwt: &str) -> Result<TokenPair>;

    async fn handle_logout(&self, refresh_jwt: &str) -> Result<bool>;

    async fn handle_login_by_password(
        &self,
        username: &str,
        password: &str,
        target: &AuthTarget,
        login_nonce: u64,
    ) -> Result<LoginByPasswordResponse>;

    async fn handle_sudo_by_password(
        &self,
        username: &str,
        password: &str,
        target: &AuthTarget,
        aud: Option<String>,
        login_nonce: u64,
    ) -> Result<SudoByPasswordResponse>;

    async fn handle_delegate_user_session(
        &self,
        caller_token: Option<&str>,
        source_session_token: &str,
        target: &AuthTarget,
    ) -> Result<DelegateUserSessionResult>;

    async fn handle_login_by_exclusive_invite(
        &self,
        _caller_token: Option<&str>,
        _request: ExclusiveInviteLoginRequest,
    ) -> Result<ExclusiveInviteLoginResult> {
        Err(RPCErrors::UnknownMethod("login_by_exclusive_invite".to_string()))
    }

    async fn handle_revoke_families(
        &self,
        _caller_token: Option<&str>,
        _request: RevokeFamiliesRequest,
    ) -> Result<RevokeFamiliesResult> {
        Err(RPCErrors::UnknownMethod("revoke_families".to_string()))
    }

    async fn handle_issue_federation_code(
        &self,
        _caller_token: Option<&str>,
        _request: IssueFederationCodeRequest,
    ) -> Result<IssueFederationCodeResult> {
        Err(RPCErrors::UnknownMethod("issue_federation_code".to_string()))
    }

    async fn handle_exchange_federation_code(
        &self,
        _request: ExchangeFederationCodeRequest,
    ) -> Result<ExchangeFederationCodeResult> {
        Err(RPCErrors::UnknownMethod("exchange_federation_code".to_string()))
    }

    async fn handle_login_by_remote_verify_hub(
        &self,
        _caller_token: Option<&str>,
        _request: LoginByRemoteVerifyHubRequest,
    ) -> Result<LoginByRemoteVerifyHubResult> {
        Err(RPCErrors::UnknownMethod("login_by_remote_verify_hub".to_string()))
    }
}

fn parse_request<T: serde::de::DeserializeOwned>(method: &str, params: Value) -> Result<T> {
    serde_json::from_value(params)
        .map_err(|error| RPCErrors::ParseRequestError(format!("invalid {method} request: {error}")))
}

fn success_value(value: impl Serialize) -> Result<RPCResult> {
    Ok(RPCResult::Success(
        serde_json::to_value(value).map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
    ))
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
    async fn handle_rpc_call(&self, req: RPCRequest, _ip_from: IpAddr) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();

        let result = match req.method.as_str() {
            "login_by_jwt" => {
                let request = LoginByJwtRequest::from_json(req.params)?;
                let token_pair = self
                    .0
                    .handle_login_by_jwt(&request.jwt, &request.login_params.target)
                    .await?;
                RPCResult::Success(
                    serde_json::to_value(token_pair)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
            }
            "login_by_password" => {
                let request = LoginByPasswordRequest::from_json(req.params)?;
                let fallback_nonce = if req.seq > 10_000_000_000_000 {
                    req.seq / 1000
                } else {
                    req.seq
                };
                let login_nonce = request.login_nonce.unwrap_or(fallback_nonce);
                let result = self
                    .0
                    .handle_login_by_password(
                        &request.username,
                        &request.password,
                        &request.target,
                        login_nonce,
                    )
                    .await?;
                RPCResult::Success(
                    serde_json::to_value(result)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
            }
            "sudo_by_password" => {
                let request = SudoByPasswordRequest::from_json(req.params)?;
                let fallback_nonce = if req.seq > 10_000_000_000_000 {
                    req.seq / 1000
                } else {
                    req.seq
                };
                let login_nonce = request.login_nonce.unwrap_or(fallback_nonce);
                let result = self
                    .0
                    .handle_sudo_by_password(
                        &request.username,
                        &request.password,
                        &request.target,
                        request.aud,
                        login_nonce,
                    )
                    .await?;
                RPCResult::Success(
                    serde_json::to_value(result)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
            }
            "refresh_token" => {
                let refresh_jwt = RefreshTokenRequest::from_json(req.params)?;
                let token_pair = self
                    .0
                    .handle_refresh_token(&refresh_jwt.refresh_token)
                    .await?;
                RPCResult::Success(
                    serde_json::to_value(token_pair)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
            }
            "logout" => {
                let refresh_jwt = RefreshTokenRequest::from_json(req.params)?;
                let ok = self.0.handle_logout(&refresh_jwt.refresh_token).await?;
                RPCResult::Success(
                    serde_json::to_value(ok)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
            }
            "delegate_user_session" => {
                let request = DelegateUserSessionRequest::from_json(req.params)?;
                let result = self
                    .0
                    .handle_delegate_user_session(
                        req.token.as_deref(),
                        &request.source_session_token,
                        &request.target,
                    )
                    .await?;
                RPCResult::Success(
                    serde_json::to_value(result)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
            }
            "login_by_exclusive_invite" => success_value(
                self.0
                    .handle_login_by_exclusive_invite(
                        req.token.as_deref(),
                        parse_request("login_by_exclusive_invite", req.params)?,
                    )
                    .await?,
            )?,
            "revoke_families" => success_value(
                self.0
                    .handle_revoke_families(
                        req.token.as_deref(),
                        parse_request("revoke_families", req.params)?,
                    )
                    .await?,
            )?,
            "issue_federation_code" => success_value(
                self.0
                    .handle_issue_federation_code(
                        req.token.as_deref(),
                        parse_request("issue_federation_code", req.params)?,
                    )
                    .await?,
            )?,
            "exchange_federation_code" => success_value(
                self.0
                    .handle_exchange_federation_code(parse_request(
                        "exchange_federation_code",
                        req.params,
                    )?)
                    .await?,
            )?,
            "login_by_remote_verify_hub" => success_value(
                self.0
                    .handle_login_by_remote_verify_hub(
                        req.token.as_deref(),
                        parse_request("login_by_remote_verify_hub", req.params)?,
                    )
                    .await?,
            )?,
            "verify_token" => {
                let verify_req = VerifyTokenRequest::from_json(req.params)?;
                let value = self
                    .0
                    .handle_verify_token(&verify_req.session_token, verify_req.expected_target)
                    .await?;
                RPCResult::Success(
                    serde_json::to_value(value)
                        .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?,
                )
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
    use crate::{UserState, UserType};
    use serde_json::json;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, Mutex};

    #[test]
    fn service_login_jwt_uses_owner_as_subject_and_device_as_issuer() {
        let private_key = EncodingKey::from_ed_pem(
            b"-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        let (jwt, token) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();

        assert_eq!(token.sub.as_deref(), Some("alice"));
        assert_eq!(token.appid.as_deref(), Some("control-panel"));
        assert_eq!(token.iss.as_deref(), Some("ood1"));
        assert!(token.jti.is_some());

        let parsed = RPCSessionToken::from_string(&jwt).unwrap();
        assert_eq!(parsed.sub.as_deref(), Some("alice"));
        assert_eq!(parsed.appid.as_deref(), Some("control-panel"));
        assert_eq!(parsed.iss.as_deref(), Some("ood1"));
    }

    #[test]
    fn system_service_login_jwt_uses_device_as_subject_and_issuer() {
        let private_key = EncodingKey::from_ed_pem(
            b"-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        let (jwt, token) =
            generate_service_login_assertion("ood1", "node-daemon", "ood1", &private_key).unwrap();

        assert_eq!(token.sub.as_deref(), Some("ood1"));
        assert_eq!(token.appid.as_deref(), Some("node-daemon"));
        assert_eq!(token.iss.as_deref(), Some("ood1"));

        let parsed = RPCSessionToken::from_string(&jwt).unwrap();
        assert_eq!(parsed.sub.as_deref(), Some("ood1"));
        assert_eq!(parsed.appid.as_deref(), Some("node-daemon"));
        assert_eq!(parsed.iss.as_deref(), Some("ood1"));
    }

    #[test]
    fn typed_requests_round_trip_and_reject_unknown_or_mixed_targets() {
        let app_target = AuthTarget::app("filebrowser@alice".parse().unwrap());
        let password = LoginByPasswordRequest::new(
            "alice".to_string(),
            "digest".to_string(),
            app_target.clone(),
            Some(42),
        );
        let decoded = LoginByPasswordRequest::from_json(password.to_json().unwrap()).unwrap();
        assert_eq!(decoded.target, app_target);

        assert!(LoginByPasswordRequest::from_json(json!({
            "type": "password",
            "username": "alice",
            "password": "digest",
            "target": {"kind": "system", "service_id": "control-panel"},
            "appid": "control-panel"
        }))
        .is_err());
        assert!(LoginByPasswordRequest::from_json(json!({
            "type": "password",
            "username": "alice",
            "password": "digest",
            "target": {
                "kind": "app",
                "app_instance_id": "filebrowser@alice",
                "service_id": "control-panel"
            }
        }))
        .is_err());

        let jwt = LoginByJwtRequest::new(
            "assertion".to_string(),
            AuthTarget::system("kernel".parse().unwrap()),
        );
        assert_eq!(
            LoginByJwtRequest::from_json(jwt.to_json().unwrap())
                .unwrap()
                .jwt,
            "assertion"
        );
        assert!(LoginByJwtRequest::from_json(json!({
            "type": "jwt",
            "jwt": "assertion",
            "target": {"kind": "system", "service_id": "kernel"},
            "unexpected": true
        }))
        .is_err());

        assert!(VerifyTokenRequest::from_json(json!({
            "session_token": "session",
            "expected_target": {"kind": "app", "app_instance_id": "filebrowser@alice"},
            "expected_appid": "filebrowser"
        }))
        .is_err());
    }

    #[test]
    fn delegate_result_status_is_the_wire_discriminant() {
        for (result, expected) in [
            (
                DelegateUserSessionResult::SourceInactive,
                json!({"status": "source_inactive"}),
            ),
            (
                DelegateUserSessionResult::AccessDenied,
                json!({"status": "access_denied"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&result).unwrap(), expected);
        }
        let issued: DelegateUserSessionResult = serde_json::from_value(json!({
            "status": "issued",
            "session_token": "s",
            "refresh_token": "r"
        }))
        .unwrap();
        assert!(matches!(
            issued,
            DelegateUserSessionResult::Issued(TokenPair { ref session_token, ref refresh_token })
                if session_token == "s" && refresh_token == "r"
        ));
        assert!(serde_json::from_value::<DelegateUserSessionResult>(json!({
            "status": "revoked"
        }))
        .is_err());
    }

    #[derive(Default, Debug)]
    struct MockCalls {
        login_jwt: Option<(String, AuthTarget)>,
        login_password: Option<(String, String, AuthTarget, u64)>,
        sudo_password: Option<(String, String, AuthTarget, Option<String>, u64)>,
        verify_token: Option<(String, Option<AuthTarget>)>,
        refresh_token: Option<String>,
        logout: Option<String>,
        delegate: Option<(Option<String>, String, AuthTarget)>,
    }

    #[derive(Clone)]
    struct MockVerifyHub {
        calls: Arc<Mutex<MockCalls>>,
    }

    #[async_trait]
    impl VerifyHubApiHandler for MockVerifyHub {
        async fn handle_login_by_jwt(&self, jwt: &str, target: &AuthTarget) -> Result<TokenPair> {
            let mut calls = self.calls.lock().unwrap();
            calls.login_jwt = Some((jwt.to_string(), target.clone()));
            Ok(TokenPair {
                session_token: "session-1".to_string(),
                refresh_token: "refresh-1".to_string(),
            })
        }

        async fn handle_login_by_password(
            &self,
            username: &str,
            password: &str,
            target: &AuthTarget,
            login_nonce: u64,
        ) -> Result<LoginByPasswordResponse> {
            let mut calls = self.calls.lock().unwrap();
            calls.login_password = Some((
                username.to_string(),
                password.to_string(),
                target.clone(),
                login_nonce,
            ));
            Ok(LoginByPasswordResponse {
                user_info: UserInfo {
                    show_name: "mock".to_string(),
                    user_id: "mock_id".to_string(),
                    user_type: UserType::Admin,
                    state: UserState::Active,
                    groups: Vec::new(),
                },
                session_token: "session-1".to_string(),
                refresh_token: "refresh-1".to_string(),
            })
        }

        async fn handle_sudo_by_password(
            &self,
            username: &str,
            password: &str,
            target: &AuthTarget,
            aud: Option<String>,
            login_nonce: u64,
        ) -> Result<SudoByPasswordResponse> {
            let mut calls = self.calls.lock().unwrap();
            calls.sudo_password = Some((
                username.to_string(),
                password.to_string(),
                target.clone(),
                aud,
                login_nonce,
            ));
            Ok(SudoByPasswordResponse {
                session_token: "sudo-session-1".to_string(),
            })
        }

        async fn handle_refresh_token(&self, refresh_jwt: &str) -> Result<TokenPair> {
            let mut calls = self.calls.lock().unwrap();
            calls.refresh_token = Some(refresh_jwt.to_string());
            Ok(TokenPair {
                session_token: "session-2".to_string(),
                refresh_token: "refresh-2".to_string(),
            })
        }

        async fn handle_logout(&self, refresh_jwt: &str) -> Result<bool> {
            let mut calls = self.calls.lock().unwrap();
            calls.logout = Some(refresh_jwt.to_string());
            Ok(true)
        }

        async fn handle_verify_token(
            &self,
            session_token: &str,
            expected_target: Option<AuthTarget>,
        ) -> Result<bool> {
            let mut calls = self.calls.lock().unwrap();
            calls.verify_token = Some((session_token.to_string(), expected_target));
            Ok(true)
        }

        async fn handle_delegate_user_session(
            &self,
            caller_token: Option<&str>,
            source_session_token: &str,
            target: &AuthTarget,
        ) -> Result<DelegateUserSessionResult> {
            let mut calls = self.calls.lock().unwrap();
            calls.delegate = Some((
                caller_token.map(str::to_string),
                source_session_token.to_string(),
                target.clone(),
            ));
            Ok(DelegateUserSessionResult::Issued(TokenPair {
                session_token: "delegated-session".to_string(),
                refresh_token: "delegated-refresh".to_string(),
            }))
        }
    }

    #[test]
    fn test_generate_verify_hub_service_doc() {
        use super::generate_verify_hub_service_doc;
        let doc = generate_verify_hub_service_doc();
        assert_eq!(doc.doc_type.to_string(), "app");
        doc.validate().unwrap();
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

        let kernel_target = AuthTarget::system("kernel".parse().unwrap());
        let control_panel_target = AuthTarget::system("control-panel".parse().unwrap());
        let token_pair = client
            .login_by_jwt("jwt-1", kernel_target.clone())
            .await
            .unwrap();
        assert_eq!(token_pair.session_token, "session-1");
        assert_eq!(token_pair.refresh_token, "refresh-1");

        let verify_result = client
            .verify_token("session-1", Some(kernel_target.clone()))
            .await
            .unwrap();
        assert!(verify_result);

        let sudo_result = client
            .sudo_by_password(
                "alice".to_string(),
                "password-hash".to_string(),
                control_panel_target.clone(),
                Some("system-config".to_string()),
                Some(123),
            )
            .await
            .unwrap();
        assert_eq!(sudo_result.session_token, "sudo-session-1");

        let logout_result = client.logout("refresh-1").await.unwrap();
        assert!(logout_result);

        let calls = calls.lock().unwrap();
        let (jwt, params) = calls.login_jwt.clone().unwrap();
        assert_eq!(jwt, "jwt-1");
        assert_eq!(params, kernel_target);
        let (session_token, expected_target) = calls.verify_token.clone().unwrap();
        assert_eq!(session_token, "session-1");
        assert_eq!(
            expected_target,
            Some(AuthTarget::system("kernel".parse().unwrap()))
        );
        let (username, password, target, aud, login_nonce) = calls.sudo_password.clone().unwrap();
        assert_eq!(username, "alice");
        assert_eq!(password, "password-hash");
        assert_eq!(target, control_panel_target);
        assert_eq!(aud, Some("system-config".to_string()));
        assert_eq!(login_nonce, 123);
        assert_eq!(calls.logout.as_deref(), Some("refresh-1"));
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
            params: json!({
                "type": "jwt",
                "jwt": "jwt-2",
                "target": {"kind": "system", "service_id": "kernel"}
            }),
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
            params: json!({
                "session_token": "session-1",
                "expected_target": {"kind": "system", "service_id": "kernel"}
            }),
            seq: 8,
            token: None,
            trace_id: None,
        };
        let verify_resp = rpc_handler.handle_rpc_call(verify_req, ip).await.unwrap();
        match verify_resp.result {
            RPCResult::Success(value) => {
                let value: bool = serde_json::from_value(value).unwrap();
                assert!(value);
            }
            _ => panic!("Expected success response"),
        }

        let sudo_req = RPCRequest {
            method: "sudo_by_password".to_string(),
            params: json!({
                "username": "alice",
                "password": "password-hash",
                "target": {"kind": "system", "service_id": "control-panel"},
                "aud": "system-config",
                "login_nonce": 123
            }),
            seq: 10,
            token: None,
            trace_id: None,
        };
        let sudo_resp = rpc_handler.handle_rpc_call(sudo_req, ip).await.unwrap();
        match sudo_resp.result {
            RPCResult::Success(value) => {
                let value: SudoByPasswordResponse = serde_json::from_value(value).unwrap();
                assert_eq!(value.session_token, "sudo-session-1");
            }
            _ => panic!("Expected success response"),
        }

        let logout_req = RPCRequest {
            method: "logout".to_string(),
            params: json!({"refresh_token": "refresh-2"}),
            seq: 9,
            token: None,
            trace_id: None,
        };
        let logout_resp = rpc_handler.handle_rpc_call(logout_req, ip).await.unwrap();
        match logout_resp.result {
            RPCResult::Success(value) => {
                let value: bool = serde_json::from_value(value).unwrap();
                assert!(value);
            }
            _ => panic!("Expected success response"),
        }

        let delegate_req = RPCRequest {
            method: "delegate_user_session".to_string(),
            params: json!({
                "source_session_token": "desktop-session",
                "target": {"kind": "app", "app_instance_id": "notes@bob"}
            }),
            seq: 11,
            token: Some("control-panel-service".to_string()),
            trace_id: None,
        };
        let delegate_resp = rpc_handler.handle_rpc_call(delegate_req, ip).await.unwrap();
        match delegate_resp.result {
            RPCResult::Success(value) => {
                assert_eq!(
                    value,
                    json!({
                        "status": "issued",
                        "session_token": "delegated-session",
                        "refresh_token": "delegated-refresh"
                    })
                );
            }
            _ => panic!("Expected success response"),
        }
        let unknown_field_req = RPCRequest {
            method: "delegate_user_session".to_string(),
            params: json!({
                "source_session_token": "desktop-session",
                "target": {"kind": "app", "app_instance_id": "notes@bob"},
                "username": "alice"
            }),
            seq: 12,
            token: Some("control-panel-service".to_string()),
            trace_id: None,
        };
        assert!(matches!(
            rpc_handler.handle_rpc_call(unknown_field_req, ip).await,
            Err(RPCErrors::ParseRequestError(_))
        ));

        let calls = calls.lock().unwrap();
        assert_eq!(
            calls.delegate.clone().unwrap(),
            (
                Some("control-panel-service".to_string()),
                "desktop-session".to_string(),
                AuthTarget::app("notes@bob".parse().unwrap()),
            )
        );
        let (jwt, params) = calls.login_jwt.clone().unwrap();
        assert_eq!(jwt, "jwt-2");
        assert_eq!(params, AuthTarget::system("kernel".parse().unwrap()));
        let (session_token, expected_target) = calls.verify_token.clone().unwrap();
        assert_eq!(session_token, "session-1");
        assert_eq!(
            expected_target,
            Some(AuthTarget::system("kernel".parse().unwrap()))
        );
        let (username, password, target, aud, login_nonce) = calls.sudo_password.clone().unwrap();
        assert_eq!(username, "alice");
        assert_eq!(password, "password-hash");
        assert_eq!(target, AuthTarget::system("control-panel".parse().unwrap()));
        assert_eq!(aud, Some("system-config".to_string()));
        assert_eq!(login_nonce, 123);
        assert_eq!(calls.logout.as_deref(), Some("refresh-2"));
    }
}
