use crate::{gateway_etc_dir, ControlPanelServer, PrincipalSession, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use buckyos_api::{
    get_buckyos_api_runtime, is_system_login_target, token_auth_method, token_session_ceiling,
    validate_verify_hub_token_claims, zone_document_hostname, AccessDenial, AppId, AppInstanceId,
    AuthTarget, BuckyOSRuntime, ControlPanelClient, CurrentIdentity, DelegateUserSessionResult,
    DenyReason, LoginByPasswordResponse, SessionCeiling, SessionScope, SystemServiceId, TokenPair,
    TokenPrincipalKind, TokenUse, UserSettings, UserState, UserType,
    CONTROL_PANEL_SERVICE_UNIQUE_ID,
};
use buckyos_http_server::{server_err, ServerError, ServerErrorCode, ServerResult, StreamInfo};
use buckyos_kit::buckyos_get_unix_timestamp;
use bytes::Bytes;
use http::header::{HOST, LOCATION, SET_COOKIE};
use http_body_util::combinators::BoxBody;
use log::{info, warn};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

const GATEWAY_SSO_SESSION_COOKIE: &str = "buckyos_session_token";
const GATEWAY_SSO_REFRESH_COOKIE: &str = "buckyos_refresh_token";
const PENDING_SSO_LOGIN_TTL_SECS: u64 = 60;
const MAX_SAFE_JSON_INTEGER_U64: u64 = (1u64 << 53) - 1;
pub(super) const PREPARE_APP_LAUNCH_METHOD: &str = "auth.prepare_app_launch";

#[derive(Serialize)]
struct AuthLoginResponse {
    #[serde(flatten)]
    login_result: LoginByPasswordResponse,
    sso_nonce: u64,
    identity: CurrentIdentity,
}

/// Control Panel methods a restricted Limited session (scope `account_self`)
/// reaches; each acts on the caller's own account only. `desktop.api` stands
/// for the desktop state, Desktop folder and wallpaper HTTP API.
const ACCOUNT_SELF_METHODS: [&str; 12] = [
    "auth.invite.set_password",
    PREPARE_APP_LAUNCH_METHOD,
    "ui.locale.get",
    "user.get",
    "user.profile.get",
    "user.profile.set",
    "user.update",
    "user.change_password",
    "zone.presentation.get",
    "apps.list",
    "apps.availability.check",
    "desktop.api",
];

/// Methods every verified user session reaches, External included.
const ANY_SESSION_METHODS: [&str; 2] = ["auth.current_identity", "user.invite.accept"];

/// Expected outcomes of `auth.prepare_app_launch`. kRPC errors reach the browser as plain
/// text, so everything the desktop branches on travels as this status instead.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AppLaunchResult {
    Ready { callback_url: String },
    SourceSessionInactive,
    AccessDenied,
    InvalidTarget,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct PrepareAppLaunchParams {
    app_instance_id: String,
    redirect_url: String,
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
pub(crate) struct ResolvedSsoAuthTarget {
    pub(crate) auth_target: AuthTarget,
    pub(crate) canonical_origin: String,
    pub(crate) canonical_redirect_url: String,
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
                Self::sso_zone_host(runtime)?.as_str(),
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
        let sso_nonce = match resolved_target.as_ref() {
            Some(resolved) => {
                let nonce = self
                    .store_new_pending_sso_login(
                        login_result.session_token.clone(),
                        login_result.refresh_token.clone(),
                        resolved,
                    )
                    .await;
                info!(
                    "prepared pending sso login pid={} username='{}' principal_kind='user' target='{}' origin='{}'",
                    std::process::id(),
                    username,
                    auth_target.canonical_key(),
                    resolved.canonical_origin
                );
                nonce
            }
            None => 0,
        };
        let identity = self
            .current_identity_for_token(&login_result.session_token)
            .await?;
        let response = AuthLoginResponse {
            login_result,
            sso_nonce,
            identity,
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(response)),
            req.seq,
        ))
    }

    /// Desktop app launch: exchanges the caller's Control Panel user session for a session of
    /// the target App instance and parks it behind a one-time `/sso_callback` on the App host.
    pub(super) async fn handle_auth_prepare_app_launch(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let source_token = Self::extract_rpc_session_token(&req)
            .ok_or_else(|| RPCErrors::InvalidToken("missing session token".to_string()))?;
        if !principal.is_control_panel_session || RPCSessionToken::from_string(&source_token)?.sudo
        {
            return Err(RPCErrors::NoPermission(
                "app launch requires a Control Panel user session".to_string(),
            ));
        }
        let params: PrepareAppLaunchParams = Self::typed_params(&req)?;
        let result = self
            .prepare_app_launch(principal.username.as_str(), &source_token, &params)
            .await?;
        info!(
            "prepare_app_launch user='{}' app_instance_id='{}' status={}",
            principal.username,
            params.app_instance_id,
            Self::app_launch_status_name(&result)
        );
        Ok(Self::app_launch_response(result, req.seq))
    }

    /// The dispatcher's authentication rejected the caller's token outright: for the launch
    /// method that is the expected "sign in again" outcome, not an error.
    pub(super) fn app_launch_source_inactive_response(seq: u64) -> RPCResponse {
        Self::app_launch_response(AppLaunchResult::SourceSessionInactive, seq)
    }

    fn app_launch_response(result: AppLaunchResult, seq: u64) -> RPCResponse {
        RPCResponse::new(RPCResult::Success(json!(result)), seq)
    }

    fn app_launch_status_name(result: &AppLaunchResult) -> &'static str {
        match result {
            AppLaunchResult::Ready { .. } => "ready",
            AppLaunchResult::SourceSessionInactive => "source_session_inactive",
            AppLaunchResult::AccessDenied => "access_denied",
            AppLaunchResult::InvalidTarget => "invalid_target",
        }
    }

    async fn prepare_app_launch(
        &self,
        user_id: &str,
        source_token: &str,
        params: &PrepareAppLaunchParams,
    ) -> Result<AppLaunchResult, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let resolved = Self::resolve_sso_auth_target(
            params.redirect_url.as_str(),
            Self::sso_zone_host(runtime)?.as_str(),
            !runtime.force_https,
            runtime.node_gateway_port,
        );
        let Some(resolved) =
            Self::resolve_app_launch_target(params.app_instance_id.as_str(), resolved)?
        else {
            return Ok(AppLaunchResult::InvalidTarget);
        };
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let token_pair = match verify_hub_client
            .delegate_user_session(source_token, resolved.auth_target.clone())
            .await?
        {
            DelegateUserSessionResult::Issued(token_pair) => token_pair,
            DelegateUserSessionResult::SourceInactive => {
                return Ok(AppLaunchResult::SourceSessionInactive);
            }
            DelegateUserSessionResult::AccessDenied => return Ok(AppLaunchResult::AccessDenied),
        };
        if let Err(error) =
            Self::validate_delegated_token_pair(&token_pair, user_id, &resolved.auth_target)
        {
            if let Err(logout_error) = verify_hub_client.logout(&token_pair.refresh_token).await {
                warn!("failed to revoke rejected delegated session: {logout_error}");
            }
            return Err(error);
        }
        let nonce = self
            .store_new_pending_sso_login(
                token_pair.session_token,
                token_pair.refresh_token,
                &resolved,
            )
            .await;
        Ok(AppLaunchResult::Ready {
            callback_url: Self::sso_callback_url(&resolved, nonce)?,
        })
    }

    /// `None` when the URL is not this Zone's Gateway route of exactly the requested App
    /// instance; route lookups that fail for other reasons stay errors.
    fn resolve_app_launch_target(
        app_instance_id: &str,
        resolved: Result<ResolvedSsoAuthTarget, RPCErrors>,
    ) -> Result<Option<ResolvedSsoAuthTarget>, RPCErrors> {
        let resolved = match resolved {
            Ok(resolved) => resolved,
            Err(RPCErrors::ParseRequestError(_) | RPCErrors::NoPermission(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let Ok(app_instance_id) = app_instance_id.parse::<AppInstanceId>() else {
            return Ok(None);
        };
        Ok((resolved.auth_target == AuthTarget::app(app_instance_id)).then_some(resolved))
    }

    fn validate_delegated_token_pair(
        token_pair: &TokenPair,
        user_id: &str,
        target: &AuthTarget,
    ) -> Result<(), RPCErrors> {
        Self::validate_sso_token_pair(
            &token_pair.session_token,
            &token_pair.refresh_token,
            target,
        )?;
        let now = buckyos_get_unix_timestamp();
        for (token, token_use) in [
            (&token_pair.session_token, TokenUse::Session),
            (&token_pair.refresh_token, TokenUse::Refresh),
        ] {
            let parsed = RPCSessionToken::from_string(token)?;
            let claims = validate_verify_hub_token_claims(&parsed, token_use)?;
            if claims.principal_kind != TokenPrincipalKind::User
                || parsed.sub.as_deref() != Some(user_id)
                || !parsed.exp.map(|exp| exp > now).unwrap_or(false)
            {
                return Err(RPCErrors::InvalidToken(
                    "delegated token pair does not belong to the launching user".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn sso_callback_url(
        resolved: &ResolvedSsoAuthTarget,
        nonce: u64,
    ) -> Result<String, RPCErrors> {
        let mut url =
            url::Url::parse(format!("{}/sso_callback", resolved.canonical_origin).as_str())
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("invalid callback origin: {error}"))
                })?;
        url.query_pairs_mut()
            .append_pair("nonce", nonce.to_string().as_str())
            .append_pair("redirect_url", resolved.canonical_redirect_url.as_str());
        Ok(url.to_string())
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
            "received sso_callback pid={} host='{}'",
            std::process::id(),
            req.headers()
                .get(HOST)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
        );

        let pending = match self.take_pending_sso_login(nonce).await {
            PendingSsoLookupResult::Found(pending) => pending,
            PendingSsoLookupResult::Expired {
                created_at,
                age_secs,
            } => {
                return Err(server_err!(
                    ServerErrorCode::BadRequest,
                    "SSO login callback expired: created_at={} age_secs={} ttl_secs={}",
                    created_at,
                    age_secs,
                    PENDING_SSO_LOGIN_TTL_SECS
                ));
            }
            PendingSsoLookupResult::Missing => {
                return Err(server_err!(
                    ServerErrorCode::BadRequest,
                    "SSO login callback nonce is unknown"
                ));
            }
        };
        let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
        let validation = (|| -> Result<(), RPCErrors> {
            let zone_host = Self::sso_zone_host(runtime)?;
            let callback_target = Self::resolve_sso_auth_target(
                redirect_url.as_str(),
                zone_host.as_str(),
                !runtime.force_https,
                runtime.node_gateway_port,
            )?;
            let request_origin =
                Self::request_origin(&req, !runtime.force_https, runtime.node_gateway_port)?;
            let request_target = Self::resolve_sso_auth_target(
                format!("{request_origin}/").as_str(),
                zone_host.as_str(),
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
            "sso_callback delivering target='{}' origin='{}'",
            pending.auth_target.canonical_key(),
            pending.canonical_origin
        );
        let cookies = Self::build_sso_cookie_headers(
            &req,
            pending.session_token.as_str(),
            pending.refresh_token.as_str(),
        )?;
        let mut response = http::Response::builder()
            .status(http::StatusCode::FOUND)
            .header(http::header::CACHE_CONTROL, "no-store")
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
                Self::sso_zone_host(runtime)?.as_str(),
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
                // The rotated pair goes out whatever happens next: losing it would sign
                // the browser out once the refresh grace period ends.
                let cookies = Self::build_sso_cookie_headers(
                    &req,
                    token_pair.session_token.as_str(),
                    token_pair.refresh_token.as_str(),
                )?;
                let identity = self
                    .current_identity_for_token(token_pair.session_token.as_str())
                    .await;
                let (status, body) =
                    Self::sso_refresh_body(token_pair.session_token.as_str(), identity);
                let mut response = http::Response::builder()
                    .status(status)
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(http::header::CACHE_CONTROL, "no-store");
                for cookie in cookies {
                    response = response.header(SET_COOKIE, cookie);
                }
                let body = serde_json::to_vec(&body).map_err(|error| {
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

    /// `/sso_refresh` body: the new session and identity, or 503 when the identity
    /// cannot be built (the error keeps its D-4 `code` / `data`).
    fn sso_refresh_body(
        session_token: &str,
        identity: Result<CurrentIdentity, RPCErrors>,
    ) -> (http::StatusCode, Value) {
        match identity {
            Ok(identity) => (
                http::StatusCode::OK,
                json!({ "session_token": session_token, "identity": identity }),
            ),
            Err(error) => {
                warn!("sso_refresh rotated the session but has no identity: {error}");
                let mut body = json!({ "error": error.to_string() });
                if let Some(detail) = error.detail() {
                    body["code"] = json!(detail.code);
                    if let Some(data) = detail.data.as_ref() {
                        body["data"] = data.clone();
                    }
                }
                (http::StatusCode::SERVICE_UNAVAILABLE, body)
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

    pub(crate) async fn store_new_pending_sso_login(
        &self,
        session_token: String,
        refresh_token: String,
        resolved: &ResolvedSsoAuthTarget,
    ) -> u64 {
        // The browser reads the nonce through JSON as a JS number, so keep it within
        // Number.MAX_SAFE_INTEGER to avoid precision loss in the callback URL.
        // Use OS-backed randomness so the nonce is not guessable within the short callback TTL.
        let nonce = (OsRng.next_u64() & MAX_SAFE_JSON_INTEGER_U64).max(1);
        self.store_pending_sso_login(
            nonce,
            PendingSsoLogin {
                session_token,
                refresh_token,
                auth_target: resolved.auth_target.clone(),
                canonical_origin: resolved.canonical_origin.clone(),
                canonical_redirect_url: resolved.canonical_redirect_url.clone(),
                created_at: buckyos_get_unix_timestamp(),
            },
        )
        .await;
        nonce
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
            "store pending sso login pid={} created_at={} now={} ttl_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
            std::process::id(),
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
                    "pending sso login expired pid={} created_at={} now={} age_secs={} ttl_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
                    std::process::id(),
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
                    "take pending sso login hit pid={} created_at={} now={} age_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
                    std::process::id(),
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
                    "take pending sso login miss pid={} now={} ttl_secs={} cache_before={} evicted={} cache_after={} cache=[{}]",
                    std::process::id(),
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
            .values()
            .map(|value| {
                format!(
                    "{}:{}s",
                    value.auth_target.canonical_key(),
                    now.saturating_sub(value.created_at)
                )
            })
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

    pub(crate) fn http_query_param(
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

    /// The Gateway SSO session cookie (host-only, renewed by `/sso_refresh`): what a browser page signed in
    /// at the Zone root carries on plain loads it cannot add headers to, such as an image.
    pub(super) fn extract_http_sso_session_cookie(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Option<String> {
        Self::extract_http_cookie(req, GATEWAY_SSO_SESSION_COOKIE)
    }

    /// Subjects of the SSO session and refresh cookies the browser already holds,
    /// read without verification or expiry checks: they only ever refuse a delivery.
    pub(crate) fn sso_cookie_subjects(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Vec<String> {
        [GATEWAY_SSO_SESSION_COOKIE, GATEWAY_SSO_REFRESH_COOKIE]
            .into_iter()
            .filter_map(|name| Self::extract_http_cookie(req, name))
            .filter_map(|token| RPCSessionToken::from_string(&token).ok()?.sub)
            .map(|subject| subject.trim().to_string())
            .filter(|subject| !subject.is_empty())
            .collect()
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

    pub(crate) fn request_origin(
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

    pub(crate) fn build_sso_cookie_headers(
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

    pub(crate) fn validate_sso_token_pair(
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

    pub(crate) fn rpc_to_server_error(error: RPCErrors) -> ServerError {
        server_err!(ServerErrorCode::BadRequest, "{}", error)
    }

    fn is_public_rpc_method(method: &str) -> bool {
        matches!(
            method,
            "auth.login"
                | "auth.sso_config"
                | "auth.entry.describe"
                | "auth.register"
                | "auth.invite.describe"
                | "auth.invite.login"
                | "auth.invite.request_resend"
                | "auth.did.begin"
                | "user.invite.get"
                | "zone.presentation.get"
        )
    }

    /// The origin of this Zone's root as the browser reached it: the request's own
    /// origin when its host is the Zone host (http and the gateway port only where
    /// the runtime allows them), otherwise `https://<zone_host>`.
    pub(crate) fn zone_root_origin(req: &RPCRequest, zone_host: &str) -> String {
        Self::param_str(req, crate::REQ_ORIGIN_PARAM)
            .and_then(|origin| url::Url::parse(&origin).ok())
            .filter(|url| {
                url.host_str()
                    .map(|host| host.trim_end_matches('.').eq_ignore_ascii_case(zone_host))
                    .unwrap_or(false)
            })
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_else(|| format!("https://{zone_host}"))
    }

    /// A restricted session acts on its own account only.
    pub(crate) fn require_restricted_self_target(
        principal: &RpcAuthPrincipal,
        req: &RPCRequest,
    ) -> Result<(), RPCErrors> {
        if !principal.is_restricted() {
            return Ok(());
        }
        match Self::param_str(req, "user_id") {
            Some(user_id) if user_id != principal.username => {
                Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)
                    .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, req.method.as_str())
                    .into_rpc_error())
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn sso_zone_host(runtime: &BuckyOSRuntime) -> Result<String, RPCErrors> {
        let zone_config = runtime
            .get_zone_config()
            .ok_or_else(|| RPCErrors::ReasonError("SSO requires a Zone config".to_string()))?;
        let zone_document = zone_config.zone_document().map_err(|error| {
            RPCErrors::ReasonError(format!("load SSO Zone document failed: {error}"))
        })?;
        if zone_document
            .hostname
            .trim()
            .trim_end_matches('.')
            .is_empty()
        {
            return Err(RPCErrors::ReasonError(
                "SSO requires a published Zone hostname".to_string(),
            ));
        }
        Ok(zone_document_hostname(&zone_document).to_ascii_lowercase())
    }

    pub(crate) fn resolve_sso_auth_target(
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
        let (parsed, ceiling) = verify_session_any_ceiling(runtime, &token).await?;
        let claims = validate_verify_hub_token_claims(&parsed, TokenUse::Session)?;
        let is_external = buckyos_api::token_is_external(&parsed);
        let is_control_panel_session = is_control_panel_user_session(&parsed)?;
        restricted_method_access(method, &ceiling, is_external, is_control_panel_session).map_err(
            |denial| {
                denial
                    .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, method)
                    .into_rpc_error()
            },
        )?;
        let session = PrincipalSession {
            ceiling,
            auth_method: token_auth_method(&parsed),
            is_external,
            is_sudo: parsed.sudo,
        };
        let is_user_session = claims.principal_kind == TokenPrincipalKind::User;
        let is_device_session = claims.principal_kind == TokenPrincipalKind::Device;
        let authenticated_app_id = claims.target.canonical_key();
        let username = parsed
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?;
        if is_external {
            // No account record: the DID is the principal. Only methods that need no
            // account reach here, so the type is the least privileged placeholder.
            return Ok(Some(RpcAuthPrincipal {
                owner_user_id: username.clone(),
                owner_did: username.clone(),
                username,
                authenticated_app_id,
                user_type: UserType::Limited,
                is_user_session,
                is_control_panel_session,
                agent_permission_mode: None,
                session,
            }));
        }
        let system_config_client = runtime.get_system_config_client().await?;
        if is_device_session {
            let device = ControlPanelClient::from_shared(system_config_client)
                .get_device_config(username.as_str())
                .await
                .map_err(|error| match error {
                    // Only a device that really is not registered invalidates the
                    // session; a failed system_config read is retryable and must not
                    // look like a bad token to the caller.
                    RPCErrors::KeyNotExist(path) => RPCErrors::InvalidToken(format!(
                        "failed to load device identity: device doc {} not found",
                        path
                    )),
                    other => {
                        RPCErrors::ReasonError(format!("failed to load device identity: {}", other))
                    }
                })?;
            return Ok(Some(RpcAuthPrincipal {
                username,
                owner_user_id: device.owner.id.clone(),
                authenticated_app_id,
                user_type: UserType::Root,
                owner_did: device.owner.to_string(),
                is_user_session: false,
                is_control_panel_session: false,
                agent_permission_mode: None,
                session,
            }));
        }
        let agent_permission_mode = if claims.principal_kind == TokenPrincipalKind::App {
            match &claims.target {
                AuthTarget::App { app_instance_id } => {
                    buckyos_api::load_agent_permission_mode(
                        system_config_client.as_ref(),
                        app_instance_id,
                    )
                    .await?
                }
                AuthTarget::System { .. } => None,
            }
        } else {
            None
        };
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
        let owner_did = match ControlPanelClient::from_shared(system_config_client)
            .get_user_did(&username)
            .await
        {
            Ok(did) => did.to_string(),
            Err(error) => {
                warn!("failed to load the DID document of {username}: {error}");
                String::new()
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
            agent_permission_mode,
            session,
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

/// Verify a verify-hub session whatever its ceiling; the caller decides what a
/// restricted one opens. The unverified ceiling only picks the scope the shared
/// check is asked for: the signature covers the same claims, and the ceiling
/// returned is read again from the verified token.
async fn verify_session_any_ceiling(
    runtime: &BuckyOSRuntime,
    token: &str,
) -> Result<(RPCSessionToken, SessionCeiling), RPCErrors> {
    let claimed = token_session_ceiling(&RPCSessionToken::from_string(token)?)?;
    let verified = match claimed.scopes.iter().next() {
        Some(scope) if claimed.restricted => {
            runtime.verify_session_token_opened(token, *scope).await?
        }
        _ => runtime.verify_trusted_session_token(token).await?,
    };
    let ceiling = token_session_ceiling(&verified)?;
    Ok((verified, ceiling))
}

/// D-4 default deny for restricted sessions (SSO-04): a Limited session reaches
/// its own-account methods, an External one its identity and invitation acceptance, and
/// either only as a Control Panel (Zone root) session.
fn restricted_method_access(
    method: &str,
    ceiling: &SessionCeiling,
    is_external: bool,
    is_control_panel_session: bool,
) -> Result<(), AccessDenial> {
    if !ceiling.restricted && !is_external {
        return Ok(());
    }
    if !is_control_panel_session {
        return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed));
    }
    if ANY_SESSION_METHODS.contains(&method) {
        return Ok(());
    }
    if is_external || !ACCOUNT_SELF_METHODS.contains(&method) {
        return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed));
    }
    if !ceiling.allows(SessionScope::AccountSelf) {
        return Err(AccessDenial::new(DenyReason::SessionCeilingExceeded));
    }
    Ok(())
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

    #[test]
    fn restricted_sessions_reach_only_their_opened_methods() {
        let limited = SessionCeiling::for_user_type(UserType::Limited);
        let external = SessionCeiling::external();
        let deny = |method: &str, ceiling: &SessionCeiling, is_external: bool, cp: bool| {
            restricted_method_access(method, ceiling, is_external, cp)
                .err()
                .map(|denial| denial.reason)
        };
        for method in [
            "auth.current_identity",
            "auth.invite.set_password",
            PREPARE_APP_LAUNCH_METHOD,
            "ui.locale.get",
            "user.get",
            "user.profile.get",
            "user.profile.set",
            "user.update",
            "user.change_password",
            "apps.list",
            "apps.availability.check",
            "desktop.api",
        ] {
            assert_eq!(deny(method, &limited, false, true), None, "{method}");
        }
        for method in [
            "user.list",
            "user.create",
            "user.update_contact",
            "user.exclusive_invite.create",
            "sso.settings.get",
            "agent.create",
            "apps.details",
            "system.overview",
            "auth.federation.describe",
            "future.method",
        ] {
            assert_eq!(
                deny(method, &limited, false, true),
                Some(DenyReason::SessionKindNotAllowed),
                "{method}"
            );
        }
        // A Limited session that lost `account_self` exceeds its ceiling.
        let narrowed = SessionCeiling::restricted([SessionScope::MsgGroup]);
        assert_eq!(
            deny("user.profile.get", &narrowed, false, true),
            Some(DenyReason::SessionCeilingExceeded)
        );
        // External: current identity and authenticated invitation acceptance.
        assert_eq!(deny("auth.current_identity", &external, true, true), None);
        assert_eq!(deny("user.invite.accept", &external, true, true), None);
        assert!(!ControlPanelServer::is_public_rpc_method(
            "user.invite.accept"
        ));
        for method in [
            "user.get",
            PREPARE_APP_LAUNCH_METHOD,
            "desktop.api",
            "apps.list",
        ] {
            assert_eq!(
                deny(method, &external, true, true),
                Some(DenyReason::SessionKindNotAllowed),
                "{method}"
            );
        }
        // A restricted session that targets anything but the Zone root is refused.
        assert_eq!(
            deny("auth.current_identity", &limited, false, false),
            Some(DenyReason::SessionKindNotAllowed)
        );
        // Unrestricted sessions keep the methods' own checks.
        let open = SessionCeiling::unrestricted();
        assert_eq!(deny("user.create", &open, false, false), None);
    }

    #[test]
    fn restricted_callers_cannot_name_another_account() {
        let mut principal = RpcAuthPrincipal {
            username: "carol".to_string(),
            owner_user_id: "carol".to_string(),
            authenticated_app_id: "control-panel".to_string(),
            user_type: UserType::Limited,
            owner_did: String::new(),
            is_user_session: true,
            is_control_panel_session: true,
            agent_permission_mode: None,
            session: PrincipalSession {
                ceiling: SessionCeiling::for_user_type(UserType::Limited),
                ..Default::default()
            },
        };
        let other = RPCRequest::new("user.profile.get", json!({"user_id": "alice"}));
        let own = RPCRequest::new("user.profile.get", json!({"user_id": "carol"}));
        let implicit = RPCRequest::new("user.profile.get", json!({}));
        let error =
            ControlPanelServer::require_restricted_self_target(&principal, &other).unwrap_err();
        assert_eq!(
            AccessDenial::from_rpc_error(&error).map(|denial| denial.reason),
            Some(DenyReason::SessionKindNotAllowed)
        );
        assert!(ControlPanelServer::require_restricted_self_target(&principal, &own).is_ok());
        assert!(ControlPanelServer::require_restricted_self_target(&principal, &implicit).is_ok());
        principal.session = PrincipalSession::default();
        principal.user_type = UserType::Admin;
        assert!(ControlPanelServer::require_restricted_self_target(&principal, &other).is_ok());
    }

    #[test]
    fn refresh_keeps_its_body_shape_and_reports_a_missing_identity() {
        let identity = CurrentIdentity::Anonymous;
        let (status, body) = ControlPanelServer::sso_refresh_body("session", Ok(identity.clone()));
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(
            body,
            json!({"session_token": "session", "identity": identity})
        );
        let (status, body) = ControlPanelServer::sso_refresh_body(
            "session",
            Err(AccessDenial::new(DenyReason::AccountState).into_rpc_error()),
        );
        assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "access_denied");
        assert_eq!(body["data"]["reason"], "account_state");
        assert!(body.get("session_token").is_none());
    }

    #[test]
    fn held_session_subjects_are_read_without_verification() {
        let jwt = |sub: &str| {
            let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
            let payload = URL_SAFE_NO_PAD.encode(json!({ "sub": sub, "exp": 1 }).to_string());
            format!("{header}.{payload}.signature")
        };
        let req = request(
            Some(&format!(
                "buckyos_session_token={}; buckyos_refresh_token={}; other=x",
                jwt("alice"),
                jwt("mallory")
            )),
            true,
        );
        assert_eq!(
            ControlPanelServer::sso_cookie_subjects(&req),
            vec!["alice".to_string(), "mallory".to_string()]
        );
        let req = request(Some("buckyos_session_token=garbage"), true);
        assert!(ControlPanelServer::sso_cookie_subjects(&req).is_empty());
        assert!(ControlPanelServer::sso_cookie_subjects(&request(None, true)).is_empty());
    }

    #[test]
    fn zone_root_origin_only_trusts_the_zone_host() {
        let req = |origin: Option<&str>| {
            let mut params = json!({});
            if let Some(origin) = origin {
                params[crate::REQ_ORIGIN_PARAM] = json!(origin);
            }
            RPCRequest::new("auth.did.begin", params)
        };
        assert_eq!(
            ControlPanelServer::zone_root_origin(
                &req(Some("http://test.buckyos.io:3180")),
                "test.buckyos.io"
            ),
            "http://test.buckyos.io:3180"
        );
        for origin in [
            None,
            Some("https://evil.example"),
            Some("https://files.test.buckyos.io"),
        ] {
            assert_eq!(
                ControlPanelServer::zone_root_origin(&req(origin), "test.buckyos.io"),
                "https://test.buckyos.io"
            );
        }
    }

    use buckyos_api::{BuckyOSRuntimeType, ZoneConfig};
    use buckyos_kit::BuckyOSMachineConfig;
    use name_lib::{ZoneDocument, DID};

    fn sso_runtime(hostname: &str) -> BuckyOSRuntime {
        let mut runtime = BuckyOSRuntime::new(
            CONTROL_PANEL_SERVICE_UNIQUE_ID,
            None,
            BuckyOSRuntimeType::KernelService,
        );
        let machine_config = BuckyOSMachineConfig::default();
        runtime.web3_bridges = machine_config.web3_bridge;
        runtime.force_https = machine_config.force_https;
        runtime.zone_id = DID::new("bns", "wugren021");
        let public_key = serde_json::from_value(json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        }))
        .unwrap();
        let mut document = ZoneDocument::new(
            runtime.zone_id.clone(),
            DID::new("bns", "wugren021"),
            public_key,
        );
        document.hostname = hostname.to_string();
        runtime.zone_config = Some(ZoneConfig::from_zone_document(&document, None).unwrap());
        runtime
    }

    fn resolve_sso_test_target(
        runtime: &BuckyOSRuntime,
        redirect_url: &str,
    ) -> Result<ResolvedSsoAuthTarget, RPCErrors> {
        let zone_host = ControlPanelServer::sso_zone_host(runtime)?;
        let (url, app_key, is_root) = ControlPanelServer::parse_sso_redirect_url(
            redirect_url,
            &zone_host,
            !runtime.force_https,
            runtime.node_gateway_port,
        )?;
        ControlPanelServer::resolve_sso_auth_target_from_gateway_info(
            &url,
            &app_key,
            is_root,
            &gateway_info(),
        )
    }

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

    fn delegated_pair(user: &str, target: &AuthTarget) -> TokenPair {
        let token = |token_use| {
            let mut token = RPCSessionToken {
                token_type: ::kRPC::RPCSessionTokenType::Normal,
                token: None,
                aud: None,
                exp: Some(buckyos_get_unix_timestamp() + 600),
                iss: Some(buckyos_api::VERIFY_HUB_TOKEN_ISSUER.to_string()),
                jti: Some("1".to_string()),
                sub: Some(user.to_string()),
                appid: None,
                sudo: false,
                extra: std::collections::HashMap::new(),
            };
            buckyos_api::bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
            buckyos_api::bind_token_target(&mut token, target, token_use).unwrap();
            token
        };
        TokenPair {
            session_token: serde_json::to_string(&token(TokenUse::Session)).unwrap(),
            refresh_token: serde_json::to_string(&token(TokenUse::Refresh)).unwrap(),
        }
    }

    #[test]
    fn app_launch_requires_the_exact_instance_route_of_this_zone() {
        let runtime = sso_runtime("wugren021.web3.buckyos.ai");
        let launch = |instance: &str, url: &str| {
            ControlPanelServer::resolve_app_launch_target(
                instance,
                resolve_sso_test_target(&runtime, url),
            )
        };
        let resolved = launch(
            "filebrowser@alice",
            "https://files.wugren021.web3.buckyos.ai/doc/1?x=1#bfp=frame",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            resolved.auth_target,
            AuthTarget::app("filebrowser@alice".parse().unwrap())
        );
        for (instance, url) in [
            (
                "filebrowser@bob",
                "https://files.wugren021.web3.buckyos.ai/",
            ),
            ("filebrowser@alice", "https://wugren021.web3.buckyos.ai/"),
            (
                "filebrowser@alice",
                "https://sys.wugren021.web3.buckyos.ai/",
            ),
            (
                "filebrowser@alice",
                "https://unknown.wugren021.web3.buckyos.ai/",
            ),
            ("filebrowser@alice", "https://files.outside.example/"),
            (
                "filebrowser@alice",
                "http://files.wugren021.web3.buckyos.ai/",
            ),
            (
                "filebrowser@alice",
                "https://user:pw@files.wugren021.web3.buckyos.ai/",
            ),
            (
                "filebrowser@alice",
                "https://files.wugren021.web3.buckyos.ai:8443/",
            ),
            (
                "Filebrowser@alice",
                "https://files.wugren021.web3.buckyos.ai/",
            ),
        ] {
            assert!(
                launch(instance, url).unwrap().is_none(),
                "{instance} {url} must be an invalid target"
            );
        }
        assert!(matches!(
            ControlPanelServer::resolve_app_launch_target(
                "filebrowser@alice",
                Err(RPCErrors::ReasonError(
                    "gateway info unreadable".to_string()
                )),
            ),
            Err(RPCErrors::ReasonError(_))
        ));
    }

    #[test]
    fn callback_url_carries_the_full_canonical_redirect() {
        let runtime = sso_runtime("wugren021.web3.buckyos.ai");
        let resolved = resolve_sso_test_target(
            &runtime,
            "https://files.wugren021.web3.buckyos.ai/doc/a b?q=1&r=%2F#bfp=frame-nonce",
        )
        .unwrap();
        let callback_url = ControlPanelServer::sso_callback_url(&resolved, 4242).unwrap();
        let parsed = url::Url::parse(&callback_url).unwrap();
        assert_eq!(
            parsed.origin().ascii_serialization(),
            resolved.canonical_origin
        );
        assert_eq!(parsed.path(), "/sso_callback");
        assert!(parsed.fragment().is_none());

        let req = http::Request::builder()
            .uri(format!("{}?{}", parsed.path(), parsed.query().unwrap()))
            .header(HOST, "files.wugren021.web3.buckyos.ai")
            .body(ControlPanelServer::boxed_http_body(Vec::new()))
            .unwrap();
        assert_eq!(
            ControlPanelServer::http_query_param(&req, "nonce").as_deref(),
            Some("4242")
        );
        let redirect_url = ControlPanelServer::http_query_param(&req, "redirect_url").unwrap();
        assert_eq!(redirect_url, resolved.canonical_redirect_url);
        assert!(redirect_url.ends_with("#bfp=frame-nonce"));
        let callback = resolve_sso_test_target(&runtime, &redirect_url).unwrap();
        let expected = pending(
            resolved.auth_target.clone(),
            &resolved.canonical_origin,
            &resolved.canonical_redirect_url,
        );
        ControlPanelServer::validate_sso_callback_binding(&callback, &callback, &expected).unwrap();
    }

    #[test]
    fn app_launch_result_status_is_the_wire_discriminant() {
        for (result, expected) in [
            (
                AppLaunchResult::Ready {
                    callback_url: "https://files.example.test/sso_callback?nonce=1".to_string(),
                },
                json!({"status": "ready", "callback_url": "https://files.example.test/sso_callback?nonce=1"}),
            ),
            (
                AppLaunchResult::SourceSessionInactive,
                json!({"status": "source_session_inactive"}),
            ),
            (
                AppLaunchResult::AccessDenied,
                json!({"status": "access_denied"}),
            ),
            (
                AppLaunchResult::InvalidTarget,
                json!({"status": "invalid_target"}),
            ),
        ] {
            assert_eq!(json!(result), expected);
        }
        let response = ControlPanelServer::app_launch_source_inactive_response(9);
        assert_eq!(response.seq, 9);
        assert!(matches!(
            response.result,
            RPCResult::Success(value) if value == json!({"status": "source_session_inactive"})
        ));
    }

    #[test]
    fn app_launch_params_do_not_accept_caller_supplied_identity() {
        assert!(serde_json::from_value::<PrepareAppLaunchParams>(json!({
            "app_instance_id": "filebrowser@alice",
            "redirect_url": "https://files.example.test/"
        }))
        .is_ok());
        for (key, value) in [
            ("username", json!("bob")),
            ("source_appid", json!("filebrowser")),
            (
                "target",
                json!({"kind": "system", "service_id": "control-panel"}),
            ),
        ] {
            let mut params = json!({
                "app_instance_id": "filebrowser@alice",
                "redirect_url": "https://files.example.test/"
            });
            params[key] = value;
            assert!(serde_json::from_value::<PrepareAppLaunchParams>(params).is_err());
        }
    }

    #[test]
    fn delegated_pair_must_be_the_launching_users_session_of_the_target() {
        let target = AuthTarget::app("filebrowser@bob".parse().unwrap());
        let valid = delegated_pair("alice", &target);
        ControlPanelServer::validate_delegated_token_pair(&valid, "alice", &target).unwrap();

        assert!(ControlPanelServer::validate_delegated_token_pair(&valid, "bob", &target).is_err());
        assert!(ControlPanelServer::validate_delegated_token_pair(
            &valid,
            "alice",
            &AuthTarget::app("filebrowser@alice".parse().unwrap()),
        )
        .is_err());
        let swapped = TokenPair {
            session_token: valid.refresh_token.clone(),
            refresh_token: valid.session_token.clone(),
        };
        assert!(
            ControlPanelServer::validate_delegated_token_pair(&swapped, "alice", &target).is_err()
        );

        let mutate = |field: &str, f: &dyn Fn(&mut RPCSessionToken)| {
            let mut session = RPCSessionToken::from_string(&valid.session_token).unwrap();
            f(&mut session);
            let pair = TokenPair {
                session_token: serde_json::to_string(&session).unwrap(),
                refresh_token: valid.refresh_token.clone(),
            };
            assert!(
                ControlPanelServer::validate_delegated_token_pair(&pair, "alice", &target).is_err(),
                "{field}"
            );
        };
        mutate("sudo", &|token| token.sudo = true);
        mutate("expired", &|token| token.exp = Some(1));
        mutate("app principal", &|token| {
            buckyos_api::bind_token_principal_kind(token, TokenPrincipalKind::App)
        });
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
    fn published_hostname_drives_login_callback_and_refresh_routes() {
        for hostname in ["wugren021.web3.buckyos.ai", "zone.custom.example"] {
            let mut runtime = sso_runtime(hostname);
            for bridge in ["bns.buckyos.ai", "web3.buckyos.ai"] {
                runtime
                    .web3_bridges
                    .insert("bns".to_string(), bridge.to_string());
                assert_eq!(
                    ControlPanelServer::sso_zone_host(&runtime).unwrap(),
                    hostname
                );
                for (host, target) in [
                    (
                        hostname.to_string(),
                        AuthTarget::system("control-panel".parse().unwrap()),
                    ),
                    (
                        format!("files.{hostname}"),
                        AuthTarget::app("filebrowser@alice".parse().unwrap()),
                    ),
                    (
                        format!("files-{hostname}"),
                        AuthTarget::app("filebrowser@alice".parse().unwrap()),
                    ),
                ] {
                    let redirect_url = format!("https://{host}/folder?tab=recent#files");
                    let login = resolve_sso_test_target(&runtime, &redirect_url).unwrap();
                    assert_eq!(login.auth_target, target);
                    let expected = pending(target.clone(), &login.canonical_origin, &redirect_url);
                    let callback = resolve_sso_test_target(&runtime, &redirect_url).unwrap();
                    let mut req = request(None, true);
                    req.headers_mut().insert(HOST, host.parse().unwrap());
                    let origin = ControlPanelServer::request_origin(
                        &req,
                        !runtime.force_https,
                        runtime.node_gateway_port,
                    )
                    .unwrap();
                    let route = resolve_sso_test_target(&runtime, &format!("{origin}/")).unwrap();
                    ControlPanelServer::validate_sso_callback_binding(&callback, &route, &expected)
                        .unwrap();
                    ControlPanelServer::validate_sso_refresh_route(&route, &target).unwrap();
                    assert!(ControlPanelServer::validate_sso_refresh_route(
                        &route,
                        &AuthTarget::app("filebrowser@bob".parse().unwrap()),
                    )
                    .is_err());
                }
            }
        }
    }

    #[test]
    fn published_hostname_does_not_authorize_other_hosts_or_unknown_apps() {
        let runtime = sso_runtime("wugren021.web3.buckyos.ai");
        for redirect_url in [
            "https://outside.example/",
            "https://wugren021.bns.buckyos.ai/",
            "https://wugren021.web3.buckyos.ai.evil.example/",
            "https://evilwugren021.web3.buckyos.ai/",
            "https://unknown.wugren021.web3.buckyos.ai/",
            "https://unknown-wugren021.web3.buckyos.ai/",
        ] {
            assert!(
                resolve_sso_test_target(&runtime, redirect_url).is_err(),
                "{redirect_url}"
            );
        }
        let custom = sso_runtime("zone.custom.example");
        assert!(resolve_sso_test_target(&custom, "https://wugren021.web3.buckyos.ai/").is_err());
    }

    #[test]
    fn sso_requires_a_readable_zone_document_with_a_published_hostname() {
        let mut runtime = sso_runtime("wugren021.web3.buckyos.ai");
        runtime.zone_config = None;
        assert!(matches!(
            ControlPanelServer::sso_zone_host(&runtime),
            Err(RPCErrors::ReasonError(_))
        ));
        runtime.zone_config = Some(ZoneConfig::new("invalid document".to_string()));
        assert!(matches!(
            ControlPanelServer::sso_zone_host(&runtime),
            Err(RPCErrors::ReasonError(_))
        ));
        for hostname in ["", "   ", "."] {
            assert!(matches!(
                ControlPanelServer::sso_zone_host(&sso_runtime(hostname)),
                Err(RPCErrors::ReasonError(_))
            ));
        }
        assert_eq!(
            ControlPanelServer::sso_zone_host(&sso_runtime(" WUGREN021.web3.buckyos.ai. "))
                .unwrap(),
            "wugren021.web3.buckyos.ai"
        );
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
