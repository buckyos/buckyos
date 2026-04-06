use crate::{ControlPanelServer, GATEWAY_ETC_DIR, RpcAuthPrincipal};
use buckyos_api::{get_buckyos_api_runtime, LoginByPasswordResponse, UserType};
use buckyos_kit::buckyos_get_unix_timestamp;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken, RPCSessionTokenType};
use name_lib::{load_private_key, DID};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

use cyfs_gateway_lib::server::ServerError;

const CONTROL_PANEL_AUTH_APPID: &str = "control-panel";
const CONTROL_PANEL_SSO_TOKEN_EXPIRE_SECONDS: u64 = 15 * 60;

#[derive(Serialize)]
struct AuthLoginResponse {
    #[serde(flatten)]
    login_result: LoginByPasswordResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    sso_token: Option<String>,
}

impl ControlPanelServer {
    pub(super) async fn handle_auth_login(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let username = Self::require_param_str(&req, "username")?;
        let password = Self::require_param_str(&req, "password")?;
        let redirect_url = Self::param_str(&req, "redirect_url");
        let appid = Self::param_str(&req, "appid")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CONTROL_PANEL_AUTH_APPID.to_string());
        let login_nonce = req
            .params
            .get("login_nonce")
            .and_then(|value| value.as_u64())
            .or(Some(req.seq));

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let login_result = verify_hub_client
            .login_by_password(username, password, appid, login_nonce)
            .await?;
        let sso_token = Self::resolve_sso_target_appid(
            redirect_url.as_deref(),
            runtime.zone_id.to_host_name().as_str(),
        )?
        .map(|target_appid| {
            let issuer = Self::resolve_local_device_name(runtime)?;
            Self::issue_gateway_sso_token(
                issuer.as_str(),
                login_result.user_info.user_id.as_str(),
                target_appid.as_str(),
            )
        })
        .transpose()?;
        let response = AuthLoginResponse {
            login_result,
            sso_token,
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(response)),
            req.seq,
        ))
    }

    pub(super) async fn handle_auth_issue_sso_token(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let redirect_url = Self::require_param_str(&req, "redirect_url")?;
        let session_token = Self::extract_rpc_session_token(&req)
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing session_token".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let verified = verify_hub_client
            .verify_token(session_token.as_str(), Some(CONTROL_PANEL_AUTH_APPID))
            .await?;
        if !verified {
            return Err(RPCErrors::InvalidToken(
                "Invalid control-panel session token".to_string(),
            ));
        }

        let target_appid = Self::resolve_sso_target_appid(
            Some(redirect_url.as_str()),
            runtime.zone_id.to_host_name().as_str(),
        )?
        .ok_or_else(|| RPCErrors::ParseRequestError("Missing redirect_url".to_string()))?;
        let session_token = RPCSessionToken::from_string(session_token.as_str())?;
        let user_id = session_token
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?;
        let issuer = Self::resolve_local_device_name(runtime)?;
        let sso_token = Self::issue_gateway_sso_token(
            issuer.as_str(),
            user_id.as_str(),
            target_appid.as_str(),
        )?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "sso_token": sso_token })),
            req.seq,
        ))
    }

    pub(super) async fn handle_auth_refresh(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let refresh_token = Self::require_param_str(&req, "refresh_token")?;

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let token_pair = verify_hub_client
            .refresh_token(refresh_token.as_str())
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!(token_pair)),
            req.seq,
        ))
    }

    pub(super) async fn handle_auth_verify(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let session_token = Self::extract_rpc_session_token(&req)
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing session_token".to_string()))?;
        let appid = Self::param_str(&req, "appid");

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let verified = verify_hub_client
            .verify_token(session_token.as_str(), appid.as_deref())
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!(verified)),
            req.seq,
        ))
    }

    pub(super) async fn handle_auth_logout(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true })),
            req.seq,
        ))
    }

    pub(super) fn normalize_session_token(token: Option<String>) -> Option<String> {
        token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn is_public_rpc_method(method: &str) -> bool {
        matches!(
            method,
            "auth.login" | "auth.refresh" | "auth.verify" | "auth.logout" | "auth.issue_sso_token"
        )
    }

    fn resolve_local_device_name(
        runtime: &buckyos_api::BuckyOSRuntime,
    ) -> Result<String, RPCErrors> {
        runtime
            .device_config
            .as_ref()
            .map(|value| value.name.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::ReasonError("missing local device name".to_string()))
    }

    fn resolve_sso_target_appid(
        redirect_url: Option<&str>,
        zone_host: &str,
    ) -> Result<Option<String>, RPCErrors> {
        let redirect_url = match redirect_url.map(|value| value.trim()) {
            Some(value) if !value.is_empty() => value,
            _ => return Ok(None),
        };

        let zone_host = zone_host.trim().trim_matches('.').to_ascii_lowercase();
        if zone_host.is_empty() {
            return Err(RPCErrors::ReasonError("missing zone host".to_string()));
        }

        let url = url::Url::parse(redirect_url).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid redirect_url: {}", error))
        })?;
        let host = url
            .host_str()
            .map(|value| value.trim().trim_matches('.').to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::ParseRequestError("redirect_url missing host".to_string()))?;

        let app_key = if host == zone_host {
            "_".to_string()
        } else {
            let suffix = format!(".{}", zone_host);
            let prefix = host.strip_suffix(suffix.as_str()).ok_or_else(|| {
                RPCErrors::ParseRequestError(
                    "redirect_url host is outside current zone".to_string(),
                )
            })?;
            prefix
                .split(['.', '-'])
                .next()
                .map(|value| value.trim().to_string())
                .filter(|value| {
                    !value.is_empty()
                        && value
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                })
                .ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                        "redirect_url host does not resolve to an app".to_string(),
                    )
                })?
        };

        Self::lookup_gateway_appid(app_key.as_str()).map(Some)
    }

    fn lookup_gateway_appid(app_key: &str) -> Result<String, RPCErrors> {
        let gateway_info_path = Path::new(GATEWAY_ETC_DIR).join("node_gateway_info.json");
        let content = std::fs::read_to_string(gateway_info_path.as_path()).map_err(|error| {
            RPCErrors::ReasonError(format!("read node_gateway_info.json failed: {}", error))
        })?;
        let value: Value = serde_json::from_str(content.as_str()).map_err(|error| {
            RPCErrors::ReasonError(format!("parse node_gateway_info.json failed: {}", error))
        })?;
        let app_info = value
            .get("app_info")
            .and_then(|value| value.get(app_key))
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(format!(
                    "redirect_url app '{}' is not present in gateway info",
                    app_key
                ))
            })?;

        app_info
            .get("app_id")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(format!(
                    "redirect_url app '{}' does not have a routable app_id",
                    app_key
                ))
            })
    }

    fn issue_gateway_sso_token(
        issuer: &str,
        user_id: &str,
        appid: &str,
    ) -> Result<String, RPCErrors> {
        let key_path = Path::new(GATEWAY_ETC_DIR).join("node_private_key.pem");
        let private_key = load_private_key(key_path.as_path()).map_err(|error| {
            RPCErrors::ReasonError(format!("load node private key failed: {}", error))
        })?;
        let session_token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(buckyos_get_unix_timestamp() + CONTROL_PANEL_SSO_TOKEN_EXPIRE_SECONDS),
            iss: Some(issuer.to_string()),
            jti: Some(Uuid::new_v4().to_string()),
            session: None,
            sub: Some(user_id.to_string()),
            appid: Some(appid.to_string()),
            extra: HashMap::new(),
        };

        session_token.generate_jwt(None, &private_key)
    }

    pub(super) fn extract_rpc_session_token(req: &RPCRequest) -> Option<String> {
        Self::normalize_session_token(req.token.clone())
            .or_else(|| Self::normalize_session_token(Self::param_str(req, "session_token")))
    }

    pub(super) fn extract_http_session_token(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Option<String> {
        if let Some(value) = req.headers().get("X-Auth") {
            if let Ok(token) = value.to_str() {
                if let Some(token) = Self::normalize_session_token(Some(token.to_string())) {
                    return Some(token);
                }
            }
        }

        if let Some(value) = req.headers().get(http::header::AUTHORIZATION) {
            if let Ok(raw) = value.to_str() {
                if let Some(token) = raw.strip_prefix("Bearer ") {
                    if let Some(token) = Self::normalize_session_token(Some(token.to_string())) {
                        return Some(token);
                    }
                }
            }
        }

        if let Some(query) = req.uri().query() {
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                if key == "auth" || key == "session_token" {
                    if let Some(token) = Self::normalize_session_token(Some(value.to_string())) {
                        return Some(token);
                    }
                }
            }
        }

        if let Some(cookie_header) = req.headers().get("Cookie") {
            if let Ok(raw_cookie) = cookie_header.to_str() {
                for piece in raw_cookie.split(';') {
                    let segment = piece.trim();
                    for key in ["auth=", "control-panel_token=", "control_panel_token="] {
                        if let Some(token) = segment.strip_prefix(key) {
                            if let Some(token) =
                                Self::normalize_session_token(Some(token.to_string()))
                            {
                                return Some(token);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    pub(super) async fn authenticate_session_token_for_method(
        &self,
        method: &str,
        token: Option<String>,
    ) -> Result<Option<RpcAuthPrincipal>, RPCErrors> {
        if Self::is_public_rpc_method(method) {
            return Ok(None);
        }

        let token = Self::normalize_session_token(token)
            .ok_or_else(|| RPCErrors::InvalidToken("missing session token".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let parsed = runtime.verify_trusted_session_token(&token).await?;
        let username = parsed
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?;
        let owner_did = DID::new("bns", &username).to_string();

        Ok(Some(RpcAuthPrincipal {
            username,
            user_type: UserType::Root,
            owner_did,
        }))
    }

    pub(super) async fn authenticate_rpc_request(
        &self,
        req: &RPCRequest,
    ) -> Result<Option<RpcAuthPrincipal>, RPCErrors> {
        self.authenticate_session_token_for_method(
            req.method.as_str(),
            Self::extract_rpc_session_token(req),
        )
        .await
    }
}
