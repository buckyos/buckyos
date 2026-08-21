use crate::{gateway_etc_dir, ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use buckyos_api::{
    get_buckyos_api_runtime, ControlPanelClient, LoginByPasswordResponse, UserInfo,
    UserPrivateProfile, UserSettings, UserState, UserType,
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

const CONTROL_PANEL_AUTH_APPID: &str = "control-panel";
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
    pub created_at: u64,
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
        let requested_appid =
            Self::param_str(&req, "appid").unwrap_or(CONTROL_PANEL_AUTH_APPID.to_string());
        let requested_app_instance_id = Self::param_str(&req, "app_instance_id");
        let redirect_url = Self::param_str(&req, "redirect_url");
        let login_nonce = req
            .params
            .get("login_nonce")
            .and_then(|value| value.as_u64())
            .or(Some(req.seq));

        let runtime: &buckyos_api::BuckyOSRuntime = get_buckyos_api_runtime()?;
        let appid = match redirect_url.as_deref() {
            Some(redirect_url) => {
                let redirect_appid = Self::resolve_sso_target_appid(
                    Some(redirect_url),
                    runtime.zone_id.to_host_name().as_str(),
                )?
                .unwrap_or_else(|| requested_appid.clone());

                if redirect_appid != requested_appid {
                    warn!(
                        "auth.login appid '{}' adjusted to '{}' from redirect_url '{}'",
                        requested_appid, redirect_appid, redirect_url
                    );
                }

                redirect_appid
            }
            None => requested_appid.clone(),
        };
        let app_instance_id = match redirect_url.as_deref() {
            Some(redirect_url) => Self::resolve_sso_target_app_instance_id(
                Some(redirect_url),
                runtime.zone_id.to_host_name().as_str(),
            )?
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(
                    "redirect_url does not resolve to an app instance".to_string(),
                )
            })?,
            None => requested_app_instance_id.unwrap_or_else(|| {
                format!("{}@{}", requested_appid, buckyos_api::SYSTEM_APP_OWNER_ID)
            }),
        };
        let (instance_app_id, _) = buckyos_api::parse_app_instance_id(&app_instance_id)?;
        if instance_app_id != appid {
            return Err(RPCErrors::ParseRequestError(format!(
                "appid `{appid}` does not match app_instance_id `{app_instance_id}`"
            )));
        }
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let login_result = verify_hub_client
            .login_by_password(
                username.clone(),
                password,
                appid.clone(),
                app_instance_id.clone(),
                login_nonce,
            )
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
                    created_at: buckyos_get_unix_timestamp(),
                },
            )
            .await;
            info!(
                "prepared pending sso login pid={} username='{}' requested_appid='{}' resolved_appid='{}' app_instance_id='{}' login_nonce={:?} req_seq={} sso_nonce={} redirect_url='{}'",
                std::process::id(),
                username,
                requested_appid,
                appid,
                app_instance_id,
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

        let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
        Self::resolve_sso_target_appid(
            Some(redirect_url.as_str()),
            runtime.zone_id.to_host_name().as_str(),
        )
        .map_err(Self::rpc_to_server_error)?
        .ok_or_else(|| {
            server_err!(
                ServerErrorCode::BadRequest,
                "redirect_url is not a valid in-zone target"
            )
        })?;
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
            .header(LOCATION, redirect_url.as_str());
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
        match self.refresh_auth_tokens(refresh_token.as_str()).await {
            Ok(token_pair) => {
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
            Err(error) => {
                let mut response = http::Response::builder()
                    .status(Self::auth_error_status(&error))
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(http::header::CACHE_CONTROL, "no-store");
                for cookie in Self::build_clear_auth_cookie_headers(&req) {
                    response = response.header(SET_COOKIE, cookie);
                }
                let body = serde_json::to_vec(&json!({ "error": error.to_string() })).map_err(
                    |encode_error| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "Failed to serialize sso refresh error: {}",
                            encode_error
                        )
                    },
                )?;
                response
                    .body(Self::boxed_http_body(body))
                    .map_err(|build_error| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "Failed to build sso refresh error response: {}",
                            build_error
                        )
                    })
            }
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
            Some(prefix.trim().to_string())
                .filter(|value| {
                    !value.is_empty()
                        && value
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
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
        let gateway_info_path = gateway_etc_dir().join("node_gateway_info.json");
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
            .or_else(|| app_info.get("service_id"))
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

    fn resolve_sso_target_app_instance_id(
        redirect_url: Option<&str>,
        zone_host: &str,
    ) -> Result<Option<String>, RPCErrors> {
        let redirect_url = match redirect_url.map(str::trim) {
            Some(value) if !value.is_empty() => value,
            _ => return Ok(None),
        };
        let zone_host = zone_host.trim().trim_matches('.').to_ascii_lowercase();
        let url = url::Url::parse(redirect_url).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid redirect_url: {error}"))
        })?;
        let host = url
            .host_str()
            .map(|value| value.trim().trim_matches('.').to_ascii_lowercase())
            .ok_or_else(|| RPCErrors::ParseRequestError("redirect_url missing host".to_string()))?;
        let app_key = if host == zone_host {
            "_".to_string()
        } else {
            let suffix = format!(".{zone_host}");
            host.strip_suffix(&suffix)
                .map(str::to_string)
                .filter(|value| {
                    !value.is_empty()
                        && value
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
                })
                .ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                        "redirect_url host is outside current zone".to_string(),
                    )
                })?
        };
        let gateway_info_path = gateway_etc_dir().join("node_gateway_info.json");
        let content = std::fs::read_to_string(&gateway_info_path).map_err(|error| {
            RPCErrors::ReasonError(format!("read node_gateway_info.json failed: {error}"))
        })?;
        let value: Value = serde_json::from_str(&content).map_err(|error| {
            RPCErrors::ReasonError(format!("parse node_gateway_info.json failed: {error}"))
        })?;
        let app_info = value
            .get("app_info")
            .and_then(|value| value.get(&app_key))
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(format!(
                    "redirect_url app `{app_key}` is not present in gateway info"
                ))
            })?;
        if let Some(instance_id) = app_info.get("app_instance_id").and_then(Value::as_str) {
            return Ok(Some(instance_id.to_string()));
        }
        let service_id = app_info
            .get("service_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(format!(
                    "redirect_url app `{app_key}` has no app_instance_id"
                ))
            })?;
        Ok(Some(format!(
            "{}@{}",
            service_id,
            buckyos_api::SYSTEM_APP_OWNER_ID
        )))
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
        let principal_kind = token_principal_kind(&parsed);
        let is_user_session = principal_kind == Some(buckyos_api::TOKEN_PRINCIPAL_KIND_USER);
        let is_device_session = principal_kind == Some(buckyos_api::TOKEN_PRINCIPAL_KIND_DEVICE);
        let is_control_panel_session = parsed.appid.as_deref() == Some(CONTROL_PANEL_AUTH_APPID)
            && parsed
                .extra
                .get(buckyos_api::APP_INSTANCE_ID_CLAIM)
                .and_then(Value::as_str)
                == Some("control-panel@system");
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
            username,
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

fn token_principal_kind(token: &RPCSessionToken) -> Option<&str> {
    token
        .extra
        .get(buckyos_api::TOKEN_PRINCIPAL_KIND_CLAIM)
        .and_then(Value::as_str)
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
            token_principal_kind(&token),
            Some(buckyos_api::TOKEN_PRINCIPAL_KIND_DEVICE)
        );
        assert_ne!(
            token_principal_kind(&token),
            Some(buckyos_api::TOKEN_PRINCIPAL_KIND_USER)
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
