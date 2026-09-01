use crate::{gateway_etc_dir, ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use buckyos_api::{
    get_buckyos_api_runtime, is_system_login_target, validate_verify_hub_token_claims, AppId,
    AppInstanceId, AuthTarget, ControlPanelClient, LoginByPasswordResponse, SystemServiceId,
    TokenPrincipalKind, TokenUse, UserInfo, UserPrivateProfile, UserSettings, UserState, UserType,
    CONTROL_PANEL_SERVICE_UNIQUE_ID,
};
use buckyos_http_server::{server_err, ServerError, ServerErrorCode, ServerResult, StreamInfo};
use buckyos_kit::buckyos_get_unix_timestamp;
use bytes::Bytes;
use http::header::{HOST, LOCATION, SET_COOKIE};
use http_body_util::combinators::BoxBody;
use log::{info, warn};
use name_lib::DID;
use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

const GATEWAY_SSO_SESSION_COOKIE: &str = "buckyos_session_token";
const GATEWAY_SSO_REFRESH_COOKIE: &str = "buckyos_refresh_token";
const PENDING_SSO_LOGIN_TTL_SECS: u64 = 60;
const MAX_SAFE_JSON_INTEGER_U64: u64 = (1u64 << 53) - 1;

#[derive(Serialize)]
struct AuthLoginResponse {
    #[serde(flatten)]
    login_result: LoginByPasswordResponse,
    sso_nonce: u64,
}

#[derive(Clone, Debug)]
pub(super) struct PendingSsoLogin {
    pub session_token: String,
    pub refresh_token: String,
    pub auth_target: AuthTarget,
    pub canonical_origin: String,
    pub canonical_redirect_url: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedSsoAuthTarget {
    auth_target: AuthTarget,
    canonical_origin: String,
    canonical_redirect_url: String,
}

#[derive(Debug)]
enum PendingSsoLookupResult {
    Found(PendingSsoLogin),
    Expired { created_at: u64, age_secs: u64 },
    Missing,
}

impl ControlPanelServer {
    //这个不是kapi,只能在当前域名内调用
    pub(super) async fn handle_auth_login(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let username = Self::require_param_str(&req, "username")?;
        let password = Self::require_param_str(&req, "password")?;
        let requested_appid = Self::param_str(&req, "appid");
        let redirect_url = Self::param_str(&req, "redirect_url");
        let login_nonce = req
            .params
            .get("login_nonce")
            .and_then(|value| value.as_u64())
            .or(Some(req.seq));

        let runtime: &buckyos_api::BuckyOSRuntime = get_buckyos_api_runtime()?;
        let resolved_target = match redirect_url.as_deref() {
            Some(redirect_url) => Some(Self::resolve_sso_auth_target(
                redirect_url,
                runtime.zone_id.to_host_name().as_str(),
                !runtime.force_https,
                runtime.node_gateway_port,
            )?),
            None => None,
        };
        let auth_target = match resolved_target.as_ref() {
            Some(resolved) => resolved.auth_target.clone(),
            None => serde_json::from_value::<AuthTarget>(
                req.params.get("target").cloned().ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                        "direct login requires a structured target".to_string(),
                    )
                })?,
            )
            .map_err(|error| {
                RPCErrors::ParseRequestError(format!("invalid login target: {error}"))
            })?,
        };
        if let Some(requested_appid) = requested_appid.as_deref() {
            if requested_appid != auth_target.appid_claim() {
                return Err(RPCErrors::ParseRequestError(format!(
                    "appid `{requested_appid}` does not match redirect target `{}`",
                    auth_target.appid_claim()
                )));
            }
        }
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let login_result = verify_hub_client
            .login_by_password(username.clone(), password, auth_target.clone(), login_nonce)
            .await?;
        let sso_nonce = if redirect_url.is_some() {
            // Frontend reads this field through JSON as a JS number, so keep it within
            // Number.MAX_SAFE_INTEGER to avoid precision loss in the callback URL.
            // Use OS-backed randomness so the nonce is not guessable within the short callback TTL.
            let nonce = (OsRng.next_u64() & MAX_SAFE_JSON_INTEGER_U64).max(1);
            self.store_pending_sso_login(
                nonce,
                PendingSsoLogin {
                    session_token: login_result.session_token.clone(),
                    refresh_token: login_result.refresh_token.clone(),
                    auth_target: auth_target.clone(),
                    canonical_origin: resolved_target
                        .as_ref()
                        .expect("redirect target was resolved")
                        .canonical_origin
                        .clone(),
                    canonical_redirect_url: resolved_target
                        .as_ref()
                        .expect("redirect target was resolved")
                        .canonical_redirect_url
                        .clone(),
                    created_at: buckyos_get_unix_timestamp(),
                },
            )
            .await;
            info!(
                "prepared pending sso login pid={} username='{}' principal_kind='user' target='{}' login_nonce={:?} req_seq={} sso_nonce={} redirect_url='{}'",
                std::process::id(),
                username,
                auth_target.canonical_key(),
                login_nonce,
                req.seq,
                nonce,
                redirect_url.as_deref().unwrap_or("")
            );
            nonce
        } else {
            0
        };
        let response = AuthLoginResponse {
            login_result,
            sso_nonce,
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(response)),
            req.seq,
        ))
    }

    //handle sso_callback(callbback_nonce) ，特殊的get方法
    // 如果有nonce,说明是登录成功的返回，把RefrechTokens写入HttpOnly Cookie中
    pub(super) async fn serve_sso_callback(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        _info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        //uri like : /sso_callback?nonce=1234567890&redirect_url=https://example.com
        // get nonce from query
        // load refresh_token by nonce from memory
        // set HttpOnly Cookie with refresh_token
        // redirect to redirect_url
        let redirect_url = Self::http_query_param(&req, "redirect_url").ok_or_else(|| {
            server_err!(
                ServerErrorCode::BadRequest,
                "Missing redirect_url in sso callback"
            )
        })?;
        let nonce = Self::http_query_param(&req, "nonce")
            .and_then(|value| value.trim().parse::<u64>().ok())
            .ok_or_else(|| {
                server_err!(ServerErrorCode::BadRequest, "Missing nonce in sso callback")
            })?;
        info!(
            "received sso_callback pid={} host='{}' nonce={} redirect_url='{}'",
            std::process::id(),
            req.headers()
                .get(HOST)
                .and_then(|value| value.to_str().ok())
                .unwrap_or(""),
            nonce,
            redirect_url
        );

        let pending = match self.take_pending_sso_login(nonce).await {
            PendingSsoLookupResult::Found(pending) => pending,
            PendingSsoLookupResult::Expired {
                created_at,
                age_secs,
            } => {
                return Err(server_err!(
                    ServerErrorCode::BadRequest,
                    "SSO login callback expired: nonce={} created_at={} age_secs={} ttl_secs={}",
                    nonce,
                    created_at,
                    age_secs,
                    PENDING_SSO_LOGIN_TTL_SECS
                ));
            }
            PendingSsoLookupResult::Missing => {
                return Err(server_err!(
                    ServerErrorCode::BadRequest,
                    "SSO login callback nonce is unknown: nonce={}",
                    nonce
                ));
            }
        };
        let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
        let validation = (|| -> Result<(), RPCErrors> {
            let callback_target = Self::resolve_sso_auth_target(
                redirect_url.as_str(),
                runtime.zone_id.to_host_name().as_str(),
                !runtime.force_https,
                runtime.node_gateway_port,
            )?;
            let request_origin =
                Self::request_origin(&req, !runtime.force_https, runtime.node_gateway_port)?;
            let request_target = Self::resolve_sso_auth_target(
                format!("{request_origin}/").as_str(),
                runtime.zone_id.to_host_name().as_str(),
                !runtime.force_https,
                runtime.node_gateway_port,
            )?;
            Self::validate_sso_callback_binding(&callback_target, &request_target, &pending)?;
            let session = RPCSessionToken::from_string(&pending.session_token)?;
            let session_claims = validate_verify_hub_token_claims(&session, TokenUse::Session)?;
            let refresh = RPCSessionToken::from_string(&pending.refresh_token)?;
            let refresh_claims = validate_verify_hub_token_claims(&refresh, TokenUse::Refresh)?;
            if session.sudo
                || session_claims.target != pending.auth_target
                || refresh_claims.target != pending.auth_target
            {
                return Err(RPCErrors::InvalidToken(
                    "pending SSO token pair does not match its delivery target".to_string(),
                ));
            }
            Ok(())
        })();
        if let Err(error) = validation {
            self.revoke_pending_sso_login(&pending).await;
            return Err(Self::rpc_to_server_error(error));
        }
        info!(
            "sso_callback nonce: {},redirect_url: {}",
            nonce, redirect_url
        );
        let cookies = Self::build_sso_cookie_headers(
            &req,
            pending.session_token.as_str(),
            pending.refresh_token.as_str(),
        )?;
        let mut response = http::Response::builder()
            .status(http::StatusCode::FOUND)
            .header(LOCATION, pending.canonical_redirect_url.as_str());
        for cookie in cookies {
            response = response.header(SET_COOKIE, cookie);
        }

        response
            .body(Self::boxed_http_body(Vec::new()))
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build sso callback redirect: {}",
                    error
                )
            })
    }

    //handle sso_refresh
    // cooke中必然有refresh_token, 刷新refresh_token+返回access_token+用户信息
    pub(super) async fn serve_sso_refresh(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        _info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        //uri like : /sso_refresh
        // get refresh_token from HttpOnly Cookie
        // if need, refresh refresh_token by verify_hub
        // generate new access_token+user_info
        // set http only cookie with new refresh_token
        // return access_token+user_info
        let refresh_token = match Self::extract_http_cookie(&req, GATEWAY_SSO_REFRESH_COOKIE) {
            Some(value) => value,
            None => {
                warn!("sso_refresh missing refresh token cookie");
                return Self::build_http_json_response(
                    http::StatusCode::UNAUTHORIZED,
                    json!({ "error": "missing refresh token cookie" }),
                );
            }
        };
        info!("sso_refresh received refresh token cookie");
        let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
        let expected_target = match (|| -> Result<AuthTarget, RPCErrors> {
            let parsed = RPCSessionToken::from_string(&refresh_token)?;
            let claims = validate_verify_hub_token_claims(&parsed, TokenUse::Refresh)?;
            let request_origin =
                Self::request_origin(&req, !runtime.force_https, runtime.node_gateway_port)?;
            let current_route = Self::resolve_sso_auth_target(
                format!("{request_origin}/").as_str(),
                runtime.zone_id.to_host_name().as_str(),
                !runtime.force_https,
                runtime.node_gateway_port,
            )?;
            Self::validate_sso_refresh_route(&current_route, &claims.target)?;
            Ok(claims.target)
        })() {
            Ok(target) => target,
            Err(error) => return Self::build_sso_auth_error_response(&req, &error),
        };
        match self.refresh_auth_tokens(refresh_token.as_str()).await {
            Ok(token_pair) => {
                if let Err(error) = Self::validate_sso_token_pair(
                    &token_pair.session_token,
                    &token_pair.refresh_token,
                    &expected_target,
                ) {
                    return Self::build_sso_auth_error_response(&req, &error);
                }
                let user_info = self
                    .lookup_user_info_by_session_token(token_pair.session_token.as_str())
                    .await
                    .map_err(Self::rpc_to_server_error)?;
                let mut response = http::Response::builder()
                    .status(http::StatusCode::OK)
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(http::header::CACHE_CONTROL, "no-store");
                for cookie in Self::build_sso_cookie_headers(
                    &req,
                    token_pair.session_token.as_str(),
                    token_pair.refresh_token.as_str(),
                )? {
                    response = response.header(SET_COOKIE, cookie);
                }

                let body = serde_json::to_vec(&json!({
                    "session_token": token_pair.session_token,
                    "user_info": user_info,
                }))
                .map_err(|error| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "Failed to serialize sso refresh response: {}",
                        error
                    )
                })?;
                response.body(Self::boxed_http_body(body)).map_err(|error| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "Failed to build sso refresh response: {}",
                        error
                    )
                })
            }
            Err(error) => Self::build_sso_auth_error_response(&req, &error),
        }
    }

    //handle sso_logout 这个是标准的krpc/post方法
    pub(super) async fn serve_sso_logout(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        _info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        //uri like : /sso_logout
        // get refresh_token from HttpOnly Cookie
        // logout refresh_token by verify_hub
        // clear HttpOnly Cookie
        // return ok
        if let Some(refresh_token) = Self::extract_http_cookie(&req, GATEWAY_SSO_REFRESH_COOKIE) {
            info!("sso_logout received refresh token cookie");
            let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
            let verify_hub_client = runtime
                .get_verify_hub_client()
                .await
                .map_err(Self::rpc_to_server_error)?;
            let _ = verify_hub_client.logout(refresh_token.as_str()).await;
        }

        let mut response = http::Response::builder()
            .status(http::StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::CACHE_CONTROL, "no-store");
        for cookie in Self::build_clear_auth_cookie_headers(&req) {
            response = response.header(SET_COOKIE, cookie);
        }
        let body = serde_json::to_vec(&json!({ "ok": true })).map_err(|error| {
            server_err!(
                ServerErrorCode::InvalidData,
                "Failed to serialize sso logout response: {}",
                error
            )
        })?;
        response.body(Self::boxed_http_body(body)).map_err(|error| {
            server_err!(
                ServerErrorCode::InvalidData,
                "Failed to build sso logout response: {}",
                error
            )
        })
    }

    pub(super) fn normalize_session_token(token: Option<String>) -> Option<String> {
        token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    async fn refresh_auth_tokens(
        &self,
        refresh_token: &str,
    ) -> Result<buckyos_api::TokenPair, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        verify_hub_client.refresh_token(refresh_token).await
    }

    async fn revoke_pending_sso_login(&self, pending: &PendingSsoLogin) {
        let result = async {
            let runtime = get_buckyos_api_runtime()?;
            let client = runtime.get_verify_hub_client().await?;
            client.logout(&pending.refresh_token).await
        }
        .await;
        if let Err(error) = result {
            warn!("failed to revoke rejected pending SSO login: {error}");
        }
    }

    async fn lookup_user_info_by_session_token(
        &self,
        session_token: &str,
    ) -> Result<UserInfo, RPCErrors> {
        let parsed = RPCSessionToken::from_string(session_token)?;
        let username = parsed
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?;
        let runtime = get_buckyos_api_runtime()?;
        let system_config_client = runtime.get_system_config_client().await?;
        let user_info_path = format!("users/{}/settings", username);
        let user_info = system_config_client
            .get(user_info_path.as_str())
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let user_settings: UserSettings = serde_json::from_str(user_info.value.as_str())
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        if !matches!(user_settings.state, UserState::Active) {
            return Err(RPCErrors::InvalidToken(format!(
                "user '{}' is not active",
                username
            )));
        }
        let mut user_info = user_settings.to_user_info();
        let profile_path = format!("users/{}/profile", username);
        if let Ok(profile_val) = system_config_client.get(&profile_path).await {
            if let Ok(profile) = serde_json::from_str::<UserPrivateProfile>(&profile_val.value) {
                if let Some(show_name) = profile.display_name.or(profile.name) {
                    user_info.show_name = show_name;
                }
            }
        }
        Ok(user_info)
    }

    async fn store_pending_sso_login(&self, nonce: u64, pending: PendingSsoLogin) {
        let now = buckyos_get_unix_timestamp();
        let mut cache = self.pending_sso_logins.lock().await;
        let before_len = cache.len();
        cache.retain(|_, value| now.saturating_sub(value.created_at) <= PENDING_SSO_LOGIN_TTL_SECS);
        let evicted = before_len.saturating_sub(cache.len());
        let created_at = pending.created_at;
        cache.insert(nonce, pending);
        info!(
            "store pending sso login pid={} nonce={} created_at={} now={} ttl_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
            std::process::id(),
            nonce,
            created_at,
            now,
            PENDING_SSO_LOGIN_TTL_SECS,
            before_len,
            evicted,
            cache.len(),
            Self::summarize_pending_sso_logins(&cache, now)
        );
    }

    async fn take_pending_sso_login(&self, nonce: u64) -> PendingSsoLookupResult {
        let now = buckyos_get_unix_timestamp();
        let mut cache = self.pending_sso_logins.lock().await;
        let before_len = cache.len();
        let target_state = cache
            .get(&nonce)
            .map(|value| (value.created_at, now.saturating_sub(value.created_at)));
        cache.retain(|_, value| now.saturating_sub(value.created_at) <= PENDING_SSO_LOGIN_TTL_SECS);
        let evicted = before_len.saturating_sub(cache.len());
        if let Some((created_at, age_secs)) = target_state {
            if age_secs > PENDING_SSO_LOGIN_TTL_SECS {
                warn!(
                    "pending sso login expired pid={} nonce={} created_at={} now={} age_secs={} ttl_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
                    std::process::id(),
                    nonce,
                    created_at,
                    now,
                    age_secs,
                    PENDING_SSO_LOGIN_TTL_SECS,
                    before_len,
                    evicted,
                    cache.len(),
                    Self::summarize_pending_sso_logins(&cache, now)
                );
                return PendingSsoLookupResult::Expired {
                    created_at,
                    age_secs,
                };
            }
        }

        match cache.remove(&nonce) {
            Some(pending) => {
                info!(
                    "take pending sso login hit pid={} nonce={} created_at={} now={} age_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
                    std::process::id(),
                    nonce,
                    pending.created_at,
                    now,
                    now.saturating_sub(pending.created_at),
                    before_len,
                    evicted,
                    cache.len(),
                    Self::summarize_pending_sso_logins(&cache, now)
                );
                PendingSsoLookupResult::Found(pending)
            }
            None => {
                warn!(
                    "take pending sso login miss pid={} nonce={} now={} ttl_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
                    std::process::id(),
                    nonce,
                    now,
                    PENDING_SSO_LOGIN_TTL_SECS,
                    before_len,
                    evicted,
                    cache.len(),
                    Self::summarize_pending_sso_logins(&cache, now)
                );
                PendingSsoLookupResult::Missing
            }
        }
    }

    fn summarize_pending_sso_logins(cache: &HashMap<u64, PendingSsoLogin>, now: u64) -> String {
        let mut entries = cache
            .iter()
            .map(|(nonce, value)| format!("{}:{}s", nonce, now.saturating_sub(value.created_at)))
            .collect::<Vec<_>>();
        entries.sort();
        if entries.len() > 8 {
            entries.truncate(8);
            entries.push("...".to_string());
        }
        if entries.is_empty() {
            "empty".to_string()
        } else {
            entries.join(", ")
        }
    }

    fn http_query_param(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        key: &str,
    ) -> Option<String> {
        let query = req.uri().query()?;
        url::form_urlencoded::parse(query.as_bytes())
            .find_map(|(param_key, value)| {
                if param_key == key {
                    Some(value.into_owned())
                } else {
                    None
                }
            })
            .and_then(|value| Self::normalize_session_token(Some(value)))
    }

    fn extract_http_cookie(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        cookie_name: &str,
    ) -> Option<String> {
        req.headers()
            .get(http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|raw| {
                raw.split(';').find_map(|segment| {
                    let mut parts = segment.trim().splitn(2, '=');
                    let key = parts.next()?.trim();
                    let value = parts.next()?.trim();
                    if key != cookie_name || value.is_empty() {
                        return None;
                    }
                    Some(value.to_string())
                })
            })
            .and_then(|value| Self::normalize_session_token(Some(value)))
    }

    fn request_is_secure(req: &http::Request<BoxBody<Bytes, ServerError>>) -> bool {
        if let Some(value) = req.headers().get("X-Forwarded-Proto") {
            if let Ok(proto) = value.to_str() {
                return proto
                    .split(',')
                    .next()
                    .map(|value| value.trim().eq_ignore_ascii_case("https"))
                    .unwrap_or(false);
            }
        }

        if let Some(value) = req.headers().get("Forwarded") {
            if let Ok(forwarded) = value.to_str() {
                for item in forwarded.split(';').flat_map(|segment| segment.split(',')) {
                    let item = item.trim();
                    if let Some(proto) = item.strip_prefix("proto=") {
                        return proto.trim().eq_ignore_ascii_case("https");
                    }
                }
            }
        }

        false
    }

    fn request_origin(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        allow_http: bool,
        gateway_port: u16,
    ) -> Result<String, RPCErrors> {
        let scheme = if Self::request_is_secure(req) {
            "https"
        } else if allow_http {
            "http"
        } else {
            return Err(RPCErrors::ParseRequestError(
                "callback request is not HTTPS".to_string(),
            ));
        };
        let authority = req
            .headers()
            .get(HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::ParseRequestError("request is missing Host".to_string()))?;
        let url =
            url::Url::parse(format!("{scheme}://{authority}/").as_str()).map_err(|error| {
                RPCErrors::ParseRequestError(format!("invalid request origin: {error}"))
            })?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RPCErrors::ParseRequestError(
                "request origin cannot contain credentials".to_string(),
            ));
        }
        if let Some(port) = url.port() {
            if !(allow_http && port == gateway_port) {
                return Err(RPCErrors::ParseRequestError(format!(
                    "request origin port {port} is not exposed by the gateway"
                )));
            }
        }
        Ok(url.origin().ascii_serialization())
    }

    fn token_max_age(token: &str) -> Option<u64> {
        let parsed = RPCSessionToken::from_string(token).ok()?;
        let exp = parsed.exp?;
        Some(exp.saturating_sub(buckyos_get_unix_timestamp()))
    }

    fn build_host_cookie_header(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        name: &str,
        value: Option<&str>,
        http_only: bool,
        max_age: Option<u64>,
    ) -> String {
        let mut parts = vec![match value {
            Some(value) => format!("{}={}", name, value),
            None => format!("{}=", name),
        }];
        parts.push("Path=/".to_string());
        parts.push("SameSite=Lax".to_string());
        if let Some(max_age) = max_age {
            parts.push(format!("Max-Age={}", max_age));
        } else {
            parts.push("Max-Age=0".to_string());
            parts.push("Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string());
        }
        if http_only {
            parts.push("HttpOnly".to_string());
        }
        if Self::request_is_secure(req) {
            parts.push("Secure".to_string());
        }
        parts.join("; ")
    }

    fn build_refresh_cookie_header(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        value: Option<&str>,
        max_age: Option<u64>,
    ) -> String {
        Self::build_host_cookie_header(req, GATEWAY_SSO_REFRESH_COOKIE, value, true, max_age)
    }

    fn build_session_cookie_headers(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        session_token: &str,
    ) -> ServerResult<Vec<String>> {
        let max_age = Self::token_max_age(session_token).ok_or_else(|| {
            server_err!(
                ServerErrorCode::BadRequest,
                "session token is missing expiration"
            )
        })?;
        Ok(vec![Self::build_host_cookie_header(
            req,
            GATEWAY_SSO_SESSION_COOKIE,
            Some(session_token),
            false,
            Some(max_age),
        )])
    }

    fn build_sso_cookie_headers(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        session_token: &str,
        refresh_token: &str,
    ) -> ServerResult<Vec<String>> {
        let mut headers = Self::build_session_cookie_headers(req, session_token)?;
        let refresh_max_age = Self::token_max_age(refresh_token).ok_or_else(|| {
            server_err!(
                ServerErrorCode::BadRequest,
                "refresh token is missing expiration"
            )
        })?;
        headers.push(Self::build_refresh_cookie_header(
            req,
            Some(refresh_token),
            Some(refresh_max_age),
        ));
        Ok(headers)
    }

    fn build_clear_auth_cookie_headers(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Vec<String> {
        vec![
            Self::build_host_cookie_header(req, GATEWAY_SSO_SESSION_COOKIE, None, false, None),
            Self::build_refresh_cookie_header(req, None, None),
        ]
    }

    fn auth_error_status(error: &RPCErrors) -> http::StatusCode {
        match error {
            RPCErrors::InvalidToken(_)
            | RPCErrors::InvalidPassword
            | RPCErrors::NoPermission(_) => http::StatusCode::UNAUTHORIZED,
            RPCErrors::ParseRequestError(_) => http::StatusCode::BAD_REQUEST,
            _ => http::StatusCode::BAD_REQUEST,
        }
    }

    fn validate_sso_token_pair(
        session_token: &str,
        refresh_token: &str,
        expected_target: &AuthTarget,
    ) -> Result<(), RPCErrors> {
        let session = RPCSessionToken::from_string(session_token)?;
        let session_claims = validate_verify_hub_token_claims(&session, TokenUse::Session)?;
        let refresh = RPCSessionToken::from_string(refresh_token)?;
        let refresh_claims = validate_verify_hub_token_claims(&refresh, TokenUse::Refresh)?;
        if session.sudo
            || session_claims.target != *expected_target
            || refresh_claims.target != *expected_target
        {
            return Err(RPCErrors::InvalidToken(
                "SSO token pair target or use mismatch".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_sso_callback_binding(
        callback_target: &ResolvedSsoAuthTarget,
        request_target: &ResolvedSsoAuthTarget,
        pending: &PendingSsoLogin,
    ) -> Result<(), RPCErrors> {
        if callback_target.auth_target != pending.auth_target
            || request_target.auth_target != pending.auth_target
            || callback_target.canonical_origin != pending.canonical_origin
            || request_target.canonical_origin != pending.canonical_origin
            || callback_target.canonical_redirect_url != pending.canonical_redirect_url
        {
            return Err(RPCErrors::InvalidToken(
                "SSO callback target or canonical origin does not match pending login".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_sso_refresh_route(
        current_route: &ResolvedSsoAuthTarget,
        token_target: &AuthTarget,
    ) -> Result<(), RPCErrors> {
        if current_route.auth_target != *token_target {
            return Err(RPCErrors::InvalidToken(
                "refresh token target does not match the current Gateway route".to_string(),
            ));
        }
        Ok(())
    }

    fn build_sso_auth_error_response(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        error: &RPCErrors,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let mut response = http::Response::builder()
            .status(Self::auth_error_status(error))
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::CACHE_CONTROL, "no-store");
        for cookie in Self::build_clear_auth_cookie_headers(req) {
            response = response.header(SET_COOKIE, cookie);
        }
        let body =
            serde_json::to_vec(&json!({ "error": error.to_string() })).map_err(|encode_error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to serialize sso auth error: {}",
                    encode_error
                )
            })?;
        response
            .body(Self::boxed_http_body(body))
            .map_err(|build_error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build sso auth error response: {}",
                    build_error
                )
            })
    }

    fn rpc_to_server_error(error: RPCErrors) -> ServerError {
        server_err!(ServerErrorCode::BadRequest, "{}", error)
    }

    fn is_public_rpc_method(method: &str) -> bool {
        matches!(
            method,
            "auth.login"
                | "auth.refresh"
                | "auth.verify"
                | "auth.logout"
                | "auth.issue_sso_token"
                | "user.invite.get"
                | "user.invite.accept"
        )
    }

    fn resolve_sso_auth_target(
        redirect_url: &str,
        zone_host: &str,
        allow_http: bool,
        gateway_port: u16,
    ) -> Result<ResolvedSsoAuthTarget, RPCErrors> {
        let redirect_url = redirect_url.trim();
        if redirect_url.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "redirect_url is required".to_string(),
            ));
        }
        let zone_host = zone_host.trim().trim_matches('.').to_ascii_lowercase();
        if zone_host.is_empty() {
            return Err(RPCErrors::ReasonError("missing zone host".to_string()));
        }

        let (url, app_key, is_zone_root) =
            Self::parse_sso_redirect_url(redirect_url, &zone_host, allow_http, gateway_port)?;
        let gateway_info_path = gateway_etc_dir().join("node_gateway_info.json");
        let content = std::fs::read_to_string(gateway_info_path.as_path()).map_err(|error| {
            RPCErrors::ReasonError(format!("read node_gateway_info.json failed: {}", error))
        })?;
        let value: Value = serde_json::from_str(content.as_str()).map_err(|error| {
            RPCErrors::ReasonError(format!("parse node_gateway_info.json failed: {}", error))
        })?;
        Self::resolve_sso_auth_target_from_gateway_info(&url, &app_key, is_zone_root, &value)
    }

    fn parse_sso_redirect_url(
        redirect_url: &str,
        zone_host: &str,
        allow_http: bool,
        gateway_port: u16,
    ) -> Result<(url::Url, String, bool), RPCErrors> {
        let url = url::Url::parse(redirect_url).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid redirect_url: {error}"))
        })?;
        match url.scheme() {
            "https" => {}
            "http" if allow_http => {}
            "http" => {
                return Err(RPCErrors::ParseRequestError(
                    "http redirect_url is disabled".to_string(),
                ));
            }
            _ => {
                return Err(RPCErrors::ParseRequestError(
                    "redirect_url scheme must be https".to_string(),
                ));
            }
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RPCErrors::ParseRequestError(
                "redirect_url cannot contain credentials".to_string(),
            ));
        }
        if let Some(port) = url.port() {
            if !(allow_http && port == gateway_port) {
                return Err(RPCErrors::ParseRequestError(format!(
                    "redirect_url port {port} is not exposed by the gateway"
                )));
            }
        }
        let host = url
            .host_str()
            .map(|value| value.trim().trim_matches('.').to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::ParseRequestError("redirect_url missing host".to_string()))?;
        if host == zone_host {
            return Ok((url, "_".to_string(), true));
        }
        let dot_suffix = format!(".{zone_host}");
        let dash_suffix = format!("-{zone_host}");
        let prefix = host
            .strip_suffix(&dot_suffix)
            .or_else(|| host.strip_suffix(&dash_suffix))
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(
                    "redirect_url host is outside current zone".to_string(),
                )
            })?;
        let app_key = prefix
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-'))
            .then(|| prefix.to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(
                    "redirect_url host does not resolve to an app".to_string(),
                )
            })?;
        Ok((url, app_key, false))
    }

    fn resolve_sso_auth_target_from_gateway_info(
        url: &url::Url,
        app_key: &str,
        is_zone_root: bool,
        gateway_info: &Value,
    ) -> Result<ResolvedSsoAuthTarget, RPCErrors> {
        let app_info = gateway_info
            .get("app_info")
            .and_then(|value| value.get(app_key))
            .and_then(Value::as_object)
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(format!(
                    "redirect_url app '{}' is not present in gateway info",
                    app_key
                ))
            })?;

        let field = |name: &str| -> Result<Option<&str>, RPCErrors> {
            match app_info.get(name) {
                None => Ok(None),
                Some(Value::String(value)) if !value.is_empty() && value.trim() == value => {
                    Ok(Some(value.as_str()))
                }
                Some(_) => Err(RPCErrors::ParseRequestError(format!(
                    "gateway app_info['{app_key}'].{name} must be a non-empty canonical string"
                ))),
            }
        };
        let app_id = field("app_id")?;
        let app_instance_id = field("app_instance_id")?;
        let app_owner_user_id = field("app_owner_user_id")?;
        let service_id = field("service_id")?;
        let has_app_fields =
            app_id.is_some() || app_instance_id.is_some() || app_owner_user_id.is_some();
        let auth_target = match (has_app_fields, service_id) {
            (true, None) => {
                let app_id = AppId::parse(app_id.ok_or_else(|| {
                    RPCErrors::ParseRequestError("App route is missing app_id".to_string())
                })?)
                .map_err(RPCErrors::ParseRequestError)?;
                let app_instance_id = app_instance_id
                    .ok_or_else(|| {
                        RPCErrors::ParseRequestError(
                            "App route is missing app_instance_id".to_string(),
                        )
                    })?
                    .parse::<AppInstanceId>()
                    .map_err(RPCErrors::ParseRequestError)?;
                let owner_user_id = app_owner_user_id.ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                        "App route is missing app_owner_user_id".to_string(),
                    )
                })?;
                if app_instance_id.app_id() != &app_id
                    || app_instance_id.owner_user_id() != owner_user_id
                {
                    return Err(RPCErrors::ParseRequestError(
                        "App route identity fields do not match app_instance_id".to_string(),
                    ));
                }
                AuthTarget::app(app_instance_id)
            }
            (false, Some(service_id)) => {
                let service_id =
                    SystemServiceId::parse(service_id).map_err(RPCErrors::ParseRequestError)?;
                if !is_system_login_target(service_id.as_str()) {
                    return Err(RPCErrors::NoPermission(format!(
                        "system service '{}' does not allow interactive user login",
                        service_id
                    )));
                }
                if service_id.as_str() == CONTROL_PANEL_SERVICE_UNIQUE_ID && !is_zone_root {
                    return Err(RPCErrors::ParseRequestError(
                        "control-panel system tokens can only be delivered to the Zone root origin"
                            .to_string(),
                    ));
                }
                AuthTarget::system(service_id)
            }
            _ => {
                return Err(RPCErrors::ParseRequestError(format!(
                    "gateway app_info['{app_key}'] mixes or omits App/System identity fields"
                )));
            }
        };

        Ok(ResolvedSsoAuthTarget {
            auth_target,
            canonical_origin: url.origin().ascii_serialization(),
            canonical_redirect_url: url.to_string(),
        })
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
        let claims = validate_verify_hub_token_claims(&parsed, TokenUse::Session)?;
        let is_user_session = claims.principal_kind == TokenPrincipalKind::User;
        let is_device_session = claims.principal_kind == TokenPrincipalKind::Device;
        let is_control_panel_session = is_control_panel_user_session(&parsed)?;
        let authenticated_app_id = claims.target.canonical_key();
        let username = parsed
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?;
        let runtime = get_buckyos_api_runtime()?;
        let system_config_client = runtime.get_system_config_client().await?;
        if is_device_session {
            let device = ControlPanelClient::from_shared(system_config_client)
                .get_device_config(username.as_str())
                .await
                .map_err(|error| {
                    RPCErrors::InvalidToken(format!("failed to load device identity: {}", error))
                })?;
            return Ok(Some(RpcAuthPrincipal {
                username,
                owner_user_id: device.owner.id.clone(),
                authenticated_app_id,
                user_type: UserType::Root,
                owner_did: device.owner.to_string(),
                is_user_session: false,
                is_control_panel_session: false,
            }));
        }
        let settings_path = format!("users/{}/settings", username);
        let settings_val = system_config_client
            .get(&settings_path)
            .await
            .map_err(|error| {
                RPCErrors::InvalidToken(format!("failed to load user settings: {}", error))
            })?;
        let user_settings: UserSettings = serde_json::from_str(settings_val.value.as_str())
            .map_err(|error| {
                RPCErrors::InvalidToken(format!("failed to parse user settings: {}", error))
            })?;
        if !matches!(user_settings.state, UserState::Active) {
            return Err(RPCErrors::InvalidToken(format!(
                "user '{}' is not active",
                username
            )));
        }
        let profile_path = format!("users/{}/profile", username);
        let owner_did = match system_config_client.get(&profile_path).await {
            Ok(profile_val) => serde_json::from_str::<UserPrivateProfile>(&profile_val.value)
                .map(|profile| profile.did.to_string())
                .unwrap_or_else(|error| {
                    warn!("failed to parse user profile for principal did: {}", error);
                    DID::new("bns", &username).to_string()
                }),
            Err(error) => {
                warn!("failed to load user profile for principal did: {}", error);
                DID::new("bns", &username).to_string()
            }
        };

        Ok(Some(RpcAuthPrincipal {
            owner_user_id: username.clone(),
            username,
            authenticated_app_id,
            user_type: user_settings.user_type,
            owner_did,
            is_user_session,
            is_control_panel_session,
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

fn is_control_panel_user_session(token: &RPCSessionToken) -> Result<bool, RPCErrors> {
    let claims = validate_verify_hub_token_claims(token, TokenUse::Session)?;
    Ok(claims.principal_kind == TokenPrincipalKind::User
        && matches!(
            claims.target,
            AuthTarget::System { service_id }
                if service_id.as_str() == CONTROL_PANEL_SERVICE_UNIQUE_ID
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    fn request(cookie: Option<&str>, secure: bool) -> http::Request<BoxBody<Bytes, ServerError>> {
        let mut builder = http::Request::builder()
            .uri("/sso_refresh")
            .header(HOST, "files.example.test");
        if let Some(cookie) = cookie {
            builder = builder.header(http::header::COOKIE, cookie);
        }
        if secure {
            builder = builder.header("X-Forwarded-Proto", "https");
        }
        builder
            .body(ControlPanelServer::boxed_http_body(Vec::new()))
            .unwrap()
    }

    fn token_with_exp(exp: u64) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(json!({ "exp": exp }).to_string());
        format!("{}.{}.signature", header, payload)
    }

    fn gateway_info() -> Value {
        json!({
            "app_info": {
                "_": {"service_id": "control-panel"},
                "sys": {"service_id": "control-panel"},
                "files": {
                    "app_id": "filebrowser",
                    "app_instance_id": "filebrowser@alice",
                    "app_owner_user_id": "alice"
                }
            }
        })
    }

    fn resolved(target: AuthTarget, origin: &str, redirect_url: &str) -> ResolvedSsoAuthTarget {
        ResolvedSsoAuthTarget {
            auth_target: target,
            canonical_origin: origin.to_string(),
            canonical_redirect_url: redirect_url.to_string(),
        }
    }

    fn pending(target: AuthTarget, origin: &str, redirect_url: &str) -> PendingSsoLogin {
        PendingSsoLogin {
            session_token: "session".to_string(),
            refresh_token: "refresh".to_string(),
            auth_target: target,
            canonical_origin: origin.to_string(),
            canonical_redirect_url: redirect_url.to_string(),
            created_at: 1,
        }
    }

    fn user_token(target: &AuthTarget) -> RPCSessionToken {
        let mut token = RPCSessionToken {
            token_type: ::kRPC::RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(u64::MAX),
            iss: Some(buckyos_api::VERIFY_HUB_TOKEN_ISSUER.to_string()),
            jti: Some("1".to_string()),
            sub: Some("alice".to_string()),
            appid: None,
            sudo: false,
            extra: std::collections::HashMap::new(),
        };
        buckyos_api::bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
        buckyos_api::bind_token_target(&mut token, target, TokenUse::Session).unwrap();
        token
    }

    #[test]
    fn zone_root_resolves_to_control_panel_system_target() {
        let (url, app_key, is_root) = ControlPanelServer::parse_sso_redirect_url(
            "https://example.test/desktop?tab=apps#installed",
            "example.test",
            false,
            3180,
        )
        .unwrap();
        let resolved = ControlPanelServer::resolve_sso_auth_target_from_gateway_info(
            &url,
            &app_key,
            is_root,
            &gateway_info(),
        )
        .unwrap();
        assert_eq!(
            resolved.auth_target,
            AuthTarget::system("control-panel".parse().unwrap())
        );
        assert_eq!(resolved.canonical_origin, "https://example.test");
    }

    #[test]
    fn app_route_resolves_to_exact_instance() {
        let (url, app_key, is_root) = ControlPanelServer::parse_sso_redirect_url(
            "https://files.example.test/path",
            "example.test",
            false,
            3180,
        )
        .unwrap();
        let resolved = ControlPanelServer::resolve_sso_auth_target_from_gateway_info(
            &url,
            &app_key,
            is_root,
            &gateway_info(),
        )
        .unwrap();
        assert_eq!(
            resolved.auth_target,
            AuthTarget::app("filebrowser@alice".parse().unwrap())
        );
    }

    #[test]
    fn redirect_url_validation_fails_closed() {
        for redirect_url in [
            "http://example.test/",
            "https://user:password@example.test/",
            "https://example.test:8443/",
            "https://outside.test/",
        ] {
            assert!(ControlPanelServer::parse_sso_redirect_url(
                redirect_url,
                "example.test",
                false,
                3180,
            )
            .is_err());
        }
        assert!(ControlPanelServer::parse_sso_redirect_url(
            "http://files.example.test:3180/",
            "example.test",
            true,
            3180,
        )
        .is_ok());
    }

    #[test]
    fn gateway_route_identity_invariants_are_strict() {
        let url = url::Url::parse("https://files.example.test/").unwrap();
        for entry in [
            json!({"service_id": "control-panel", "app_id": "filebrowser", "app_instance_id": "filebrowser@alice", "app_owner_user_id": "alice"}),
            json!({"app_id": "filebrowser", "app_instance_id": "filebrowser@alice"}),
            json!({"app_id": "filebrowser", "app_instance_id": "filebrowser@bob", "app_owner_user_id": "alice"}),
            json!({}),
        ] {
            let info = json!({"app_info": {"files": entry}});
            assert!(
                ControlPanelServer::resolve_sso_auth_target_from_gateway_info(
                    &url, "files", false, &info,
                )
                .is_err()
            );
        }
        let sys_url = url::Url::parse("https://sys.example.test/").unwrap();
        assert!(
            ControlPanelServer::resolve_sso_auth_target_from_gateway_info(
                &sys_url,
                "sys",
                false,
                &gateway_info(),
            )
            .is_err()
        );
    }

    #[test]
    fn callback_binding_requires_exact_target_origin_and_redirect() {
        let app_a = AuthTarget::app("filebrowser@alice".parse().unwrap());
        let app_b = AuthTarget::app("filebrowser@bob".parse().unwrap());
        let expected = pending(
            app_a.clone(),
            "https://files.example.test",
            "https://files.example.test/folder?tab=recent",
        );
        let callback = resolved(
            app_a.clone(),
            "https://files.example.test",
            "https://files.example.test/folder?tab=recent",
        );
        let request = resolved(
            app_a.clone(),
            "https://files.example.test",
            "https://files.example.test/",
        );
        assert!(
            ControlPanelServer::validate_sso_callback_binding(&callback, &request, &expected)
                .is_ok()
        );

        for (changed_callback, changed_request) in [
            (
                resolved(
                    app_b,
                    "https://files.example.test",
                    "https://files.example.test/folder?tab=recent",
                ),
                request.clone(),
            ),
            (
                resolved(
                    app_a.clone(),
                    "https://files-alt.example.test",
                    "https://files-alt.example.test/folder?tab=recent",
                ),
                request.clone(),
            ),
            (
                callback.clone(),
                resolved(
                    app_a.clone(),
                    "https://files-alt.example.test",
                    "https://files-alt.example.test/",
                ),
            ),
            (
                resolved(
                    app_a,
                    "https://files.example.test",
                    "https://files.example.test/other",
                ),
                request.clone(),
            ),
        ] {
            assert!(ControlPanelServer::validate_sso_callback_binding(
                &changed_callback,
                &changed_request,
                &expected,
            )
            .is_err());
        }
    }

    #[test]
    fn refresh_route_requires_exact_target_kind_and_app_instance() {
        let app_alice = AuthTarget::app("control-panel@alice".parse().unwrap());
        let route = resolved(
            app_alice.clone(),
            "https://app.example.test",
            "https://app.example.test/",
        );
        assert!(ControlPanelServer::validate_sso_refresh_route(&route, &app_alice).is_ok());
        assert!(ControlPanelServer::validate_sso_refresh_route(
            &route,
            &AuthTarget::app("control-panel@bob".parse().unwrap()),
        )
        .is_err());
        assert!(ControlPanelServer::validate_sso_refresh_route(
            &route,
            &AuthTarget::system("control-panel".parse().unwrap()),
        )
        .is_err());
    }

    #[test]
    fn control_panel_session_requires_user_system_target() {
        let system = user_token(&AuthTarget::system("control-panel".parse().unwrap()));
        assert!(is_control_panel_user_session(&system).unwrap());

        let app = user_token(&AuthTarget::app("control-panel@alice".parse().unwrap()));
        assert!(!is_control_panel_user_session(&app).unwrap());

        let mut legacy = system;
        legacy.extra.remove(buckyos_api::TOKEN_TARGET_KIND_CLAIM);
        legacy.extra.insert(
            buckyos_api::APP_INSTANCE_ID_CLAIM.to_string(),
            Value::String("control-panel@system".to_string()),
        );
        assert!(is_control_panel_user_session(&legacy).is_err());
    }

    #[test]
    fn device_principal_kind_is_distinct_from_user() {
        let mut extra = std::collections::HashMap::new();
        extra.insert(
            buckyos_api::TOKEN_PRINCIPAL_KIND_CLAIM.to_string(),
            Value::String(buckyos_api::TOKEN_PRINCIPAL_KIND_DEVICE.to_string()),
        );
        let token = RPCSessionToken {
            token_type: ::kRPC::RPCSessionTokenType::Normal,
            token: None,
            aud: None,
            exp: None,
            iss: Some("verify-hub".to_string()),
            jti: Some("1".to_string()),
            sub: Some("ood1".to_string()),
            appid: Some("buckycli".to_string()),
            sudo: false,
            extra,
        };

        assert_eq!(
            buckyos_api::token_principal_kind(&token).unwrap(),
            TokenPrincipalKind::Device
        );
        assert_ne!(
            buckyos_api::token_principal_kind(&token).unwrap(),
            TokenPrincipalKind::User
        );
    }

    #[test]
    fn refresh_cookie_is_distinct_and_host_only() {
        let req = request(None, true);
        let header = ControlPanelServer::build_refresh_cookie_header(
            &req,
            Some("current-refresh-token"),
            Some(300),
        );

        assert_eq!(
            header,
            "buckyos_refresh_token=current-refresh-token; Path=/; SameSite=Lax; Max-Age=300; HttpOnly; Secure"
        );
        assert!(!header.contains(GATEWAY_SSO_SESSION_COOKIE));
        assert!(!header.contains("Domain="));
    }

    #[test]
    fn refresh_cookie_extraction_ignores_duplicate_session_cookies() {
        let req = request(
            Some(
                "buckyos_session_token=stale-parent-refresh; buckyos_session_token=current-app-session; buckyos_refresh_token=current-app-refresh",
            ),
            true,
        );

        assert_eq!(
            ControlPanelServer::extract_http_cookie(&req, GATEWAY_SSO_REFRESH_COOKIE).as_deref(),
            Some("current-app-refresh")
        );
    }

    #[test]
    fn sso_cookie_headers_include_gateway_session_and_isolated_refresh() {
        let req = request(None, true);
        let exp = buckyos_get_unix_timestamp() + 300;
        let session_token = token_with_exp(exp);
        let refresh_token = token_with_exp(exp);
        let headers = ControlPanelServer::build_sso_cookie_headers(
            &req,
            session_token.as_str(),
            refresh_token.as_str(),
        )
        .unwrap();

        assert_eq!(headers.len(), 2);
        assert!(
            headers[0].starts_with(format!("buckyos_session_token={};", session_token).as_str())
        );
        assert!(
            headers[1].starts_with(format!("buckyos_refresh_token={};", refresh_token).as_str())
        );
        assert!(headers[1].contains("HttpOnly"));
        assert!(headers.iter().all(|header| !header.contains("Domain=")));
    }

    #[test]
    fn clear_auth_cookies_clears_both_host_only_cookies() {
        let req = request(None, true);
        let headers = ControlPanelServer::build_clear_auth_cookie_headers(&req);

        assert_eq!(headers.len(), 2);
        assert_eq!(
            headers[0],
            "buckyos_session_token=; Path=/; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Secure"
        );
        assert_eq!(
            headers[1],
            "buckyos_refresh_token=; Path=/; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; Secure"
        );
        assert!(headers.iter().all(|header| !header.contains("Domain=")));
    }
}
