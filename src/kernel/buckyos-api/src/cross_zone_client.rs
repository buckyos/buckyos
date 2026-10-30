//! Calls into another Zone's system services (doc/arch/verify-hub/跨Zone登录断言契约.md §7):
//! log in at the Target verify-hub with a signed assertion, keep the Target's token pair per
//! (Zone, service, principal), and carry requests over RTCP (the local gateway's SOCKS entry)
//! or, when RTCP does not connect, plain HTTPS.

use crate::{
    cross_zone_auth::{sign_cross_zone_assertion, zone_hostname, CustodyResolver, DeviceSigner, ResolveFailure},
    AuthTarget, CrossZoneLoginError, CrossZoneLoginRequest, NameServiceResolver, RefreshTokenRequest, SessionScope,
    SystemServiceId, TokenPair, CROSS_ZONE_LOGIN_METHOD,
};
use ::kRPC::{RPCErrorDetail, RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_system_etc_dir};
use name_lib::{decode_jwt_claim_without_verify, DID};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Credentials of the gateway's loopback SOCKS entry, written by node_daemon.
pub const CROSS_ZONE_SOCKS_FILE: &str = "cross_zone_socks.json";
pub const CROSS_ZONE_SOCKS_ADDR: &str = "127.0.0.1:3186";
/// After RTCP failed, HTTPS carries the Zone's traffic and RTCP is tried again this often.
const RTCP_REPROBE_INTERVAL: Duration = Duration::from_secs(300);
/// An access token this close to expiry is refreshed before use.
const REFRESH_MARGIN_SECS: u64 = 60;
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);
const ZONE_HOST_TTL: Duration = Duration::from_secs(600);
const VERIFY_HUB_PATH: &str = "/kapi/verify-hub";

#[derive(Debug, Clone, Deserialize)]
pub struct SocksCredentials {
    pub username: String,
    pub password: String,
}

impl SocksCredentials {
    /// `$BUCKYOS_ROOT/etc/cross_zone_socks.json`; `None` when the node has no RTCP entry.
    pub fn load_default() -> Option<Self> {
        let path = get_buckyos_system_etc_dir().join(CROSS_ZONE_SOCKS_FILE);
        let raw = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&raw)
            .map_err(|error| log::warn!("{} is not valid: {error}", path.display()))
            .ok()
    }
}

/// Whom a cross-Zone call speaks for: a user or Agent DID this Zone hosts, and the App acting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Principal {
    pub did: DID,
    pub app_id: Option<String>,
}

impl Principal {
    pub fn new(did: DID) -> Self {
        Self { did, app_id: None }
    }
}

#[derive(Debug)]
pub enum CrossZoneError {
    /// Nothing reached the Target (or it answered 5xx): retry later, the same request is safe
    /// to send again only if it is idempotent.
    Transport(String),
    /// The Target refused the login or the token. Not retried over another transport.
    Auth { code: Option<CrossZoneLoginError>, detail: String },
    /// The Target Zone has no usable document: no route exists.
    Unroutable(String),
    /// A kRPC call reached the service and it answered with an error.
    Target(RPCErrors),
}

impl CrossZoneError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::Auth { code, .. } => code.is_some_and(CrossZoneLoginError::is_retryable),
            Self::Unroutable(_) | Self::Target(_) => false,
        }
    }
}

impl std::fmt::Display for CrossZoneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(detail) => write!(f, "cross-zone transport: {detail}"),
            Self::Auth { detail, .. } => write!(f, "cross-zone authentication: {detail}"),
            Self::Unroutable(detail) => write!(f, "cross-zone route: {detail}"),
            Self::Target(error) => write!(f, "cross-zone call: {error}"),
        }
    }
}

impl std::error::Error for CrossZoneError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    /// Nothing known yet: RTCP first.
    Probing,
    RtcpOk,
    /// RTCP did not connect; HTTPS until the next probe.
    HttpsFallback,
    /// Neither transport connected on the last attempt.
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Rtcp,
    Https,
}

struct ZoneLink {
    state: TransportState,
    rtcp_failed_at: Option<Instant>,
}

#[derive(Clone)]
struct CachedToken {
    session: String,
    session_exp: u64,
    refresh: String,
    refresh_exp: u64,
}

impl CachedToken {
    fn from_pair(pair: TokenPair) -> Self {
        let exp = |jwt: &str| {
            decode_jwt_claim_without_verify(jwt)
                .ok()
                .and_then(|claims| claims.get("exp").and_then(Value::as_u64))
                .unwrap_or(0)
        };
        Self {
            session_exp: exp(&pair.session_token),
            refresh_exp: exp(&pair.refresh_token),
            session: pair.session_token,
            refresh: pair.refresh_token,
        }
    }
}

/// (target zone host, service, principal DID, app): never shared between Zones or services.
type TokenKey = (String, String, String, Option<String>);

/// One HTTP request to a Target service; rebuilt for each transport attempt.
#[derive(Clone)]
pub struct CrossZoneRequest {
    pub method: http::Method,
    pub path_and_query: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<bytes::Bytes>,
    pub timeout: Duration,
}

impl CrossZoneRequest {
    pub fn new(method: http::Method, path_and_query: impl Into<String>) -> Self {
        Self {
            method,
            path_and_query: path_and_query.into(),
            headers: Vec::new(),
            body: None,
            timeout: Duration::from_secs(20),
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, body: impl Into<bytes::Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sending it twice does no harm, so a timeout may move on to the next transport.
    fn replayable(&self) -> bool {
        matches!(self.method, http::Method::GET | http::Method::HEAD)
    }
}

pub struct CrossZoneClient {
    signer: DeviceSigner,
    resolver: Arc<dyn CustodyResolver>,
    rtcp: Option<reqwest::Client>,
    https: reqwest::Client,
    /// Zone host → origin for the HTTPS transport (`https://<host>` otherwise).
    origins: HashMap<String, String>,
    links: Mutex<HashMap<String, ZoneLink>>,
    hosts: Mutex<HashMap<String, (String, Instant)>>,
    tokens: Mutex<HashMap<TokenKey, CachedToken>>,
    logins: Mutex<HashMap<TokenKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl CrossZoneClient {
    /// `socks`: the local gateway's RTCP entry, `None` to use HTTPS only.
    pub fn new(signer: DeviceSigner, socks: Option<(String, SocksCredentials)>) -> Result<Self, RPCErrors> {
        let build = |proxy: Option<reqwest::Proxy>| {
            let builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
            match proxy {
                Some(proxy) => builder.proxy(proxy),
                None => builder.no_proxy(),
            }
            .build()
            .map_err(|error| RPCErrors::ReasonError(format!("cross-zone http client: {error}")))
        };
        let rtcp = match socks {
            Some((addr, credentials)) => {
                let proxy = reqwest::Proxy::all(format!("socks5h://{addr}"))
                    .map_err(|error| RPCErrors::ReasonError(format!("cross-zone socks entry: {error}")))?
                    .basic_auth(&credentials.username, &credentials.password);
                Some(build(Some(proxy))?)
            }
            None => None,
        };
        Ok(Self {
            signer,
            resolver: Arc::new(NameServiceResolver),
            rtcp,
            https: build(None)?,
            origins: HashMap::new(),
            links: Mutex::default(),
            hosts: Mutex::default(),
            tokens: Mutex::default(),
            logins: Mutex::default(),
        })
    }

    /// The client of a system service: this node's device key, and the gateway's RTCP entry
    /// when node_daemon set one up.
    pub fn from_runtime(runtime: &crate::BuckyOSRuntime) -> Result<Self, RPCErrors> {
        let signer = runtime
            .device_signer()
            .ok_or_else(|| RPCErrors::ReasonError("device key not loaded".to_string()))?;
        let socks = SocksCredentials::load_default().map(|credentials| (CROSS_ZONE_SOCKS_ADDR.to_string(), credentials));
        if socks.is_none() {
            log::warn!("no cross-zone RTCP entry on this node; other Zones are reached over HTTPS only");
        }
        Self::new(signer, socks)
    }

    pub fn with_resolver(mut self, resolver: Arc<dyn CustodyResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Declared origins for the HTTPS transport (Zones not served at `https://<host>`).
    pub fn with_origins(mut self, origins: HashMap<String, String>) -> Self {
        self.origins = origins
            .into_iter()
            .map(|(host, origin)| (host.trim_end_matches('.').to_ascii_lowercase(), origin))
            .collect();
        self
    }

    pub fn signer(&self) -> &DeviceSigner {
        &self.signer
    }

    pub fn transport_state(&self, zone_host: &str) -> TransportState {
        self.links
            .lock()
            .unwrap()
            .get(zone_host)
            .map_or(TransportState::Probing, |link| link.state)
    }

    /// Hostname the Target Zone serves under (its ZoneDocument), cached for a while.
    pub async fn zone_host(&self, zone: &DID) -> Result<String, CrossZoneError> {
        let key = zone.to_string();
        if let Some((host, at)) = self.hosts.lock().unwrap().get(&key) {
            if at.elapsed() < ZONE_HOST_TTL {
                return Ok(host.clone());
            }
        }
        let host = match zone_hostname(self.resolver.as_ref(), zone).await {
            Ok(host) => host,
            Err(ResolveFailure::NotFound(detail)) => return Err(CrossZoneError::Unroutable(detail)),
            Err(error) => return Err(CrossZoneError::Transport(format!("resolve Zone {key}: {error}"))),
        };
        self.hosts.lock().unwrap().insert(key, (host.clone(), Instant::now()));
        Ok(host)
    }

    /// An HTTP request to `service` of `zone` as `principal`, with the Target's Bearer token.
    /// Any answer but 401 is returned as is; a 401 logs in again once.
    pub async fn request(
        &self,
        zone: &DID,
        service: &str,
        principal: &Principal,
        request: CrossZoneRequest,
    ) -> Result<reqwest::Response, CrossZoneError> {
        let host = self.zone_host(zone).await?;
        self.request_to(zone, &host, service, principal, request).await
    }

    /// [`Self::request`] to a Zone known only by its hostname (the assertion is addressed to
    /// `did:web:<host>`, which the Target accepts for its own hostname).
    pub async fn request_host(
        &self,
        host: &str,
        service: &str,
        principal: &Principal,
        request: CrossZoneRequest,
    ) -> Result<reqwest::Response, CrossZoneError> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.request_to(&DID::new("web", &host), &host, service, principal, request).await
    }

    /// [`Self::request`] when the Target Zone's hostname is already known.
    pub async fn request_to(
        &self,
        zone: &DID,
        host: &str,
        service: &str,
        principal: &Principal,
        request: CrossZoneRequest,
    ) -> Result<reqwest::Response, CrossZoneError> {
        let host = host.to_string();
        let key = token_key(&host, service, principal);
        let mut token = self.bearer(zone, &host, service, principal, &key).await?;
        for attempt in 0..2 {
            let authorized = request.clone().header("authorization", format!("Bearer {token}"));
            let response = self.send(&host, || Ok(authorized.clone())).await?;
            if response.status() != reqwest::StatusCode::UNAUTHORIZED {
                return Ok(response);
            }
            self.tokens.lock().unwrap().remove(&key);
            if attempt == 0 {
                token = self.bearer(zone, &host, service, principal, &key).await?;
            }
        }
        Err(CrossZoneError::Auth { code: None, detail: format!("{service} of {host} refused a fresh token") })
    }

    /// A kRPC call to `/kapi/<service>` of `zone` as `principal`.
    pub async fn call(
        &self,
        zone: &DID,
        service: &str,
        principal: &Principal,
        method: &str,
        params: Value,
    ) -> Result<Value, CrossZoneError> {
        let host = self.zone_host(zone).await?;
        let key = token_key(&host, service, principal);
        let token = self.bearer(zone, &host, service, principal, &key).await?;
        let result = self.krpc(&host, &format!("/kapi/{service}"), method, params.clone(), Some(token)).await;
        match result {
            Err(CrossZoneError::Target(RPCErrors::InvalidToken(_) | RPCErrors::TokenExpired(_))) => {
                self.tokens.lock().unwrap().remove(&key);
                let token = self.bearer(zone, &host, service, principal, &key).await?;
                self.krpc(&host, &format!("/kapi/{service}"), method, params, Some(token)).await
            }
            other => other,
        }
    }

    /// The Target token for `key`: cached, refreshed, or a fresh login (one login at a time).
    async fn bearer(
        &self,
        zone: &DID,
        host: &str,
        service: &str,
        principal: &Principal,
        key: &TokenKey,
    ) -> Result<String, CrossZoneError> {
        let gate = self.logins.lock().unwrap().entry(key.clone()).or_default().clone();
        let _held = gate.lock().await;
        let now = buckyos_get_unix_timestamp();
        let cached = self.tokens.lock().unwrap().get(key).cloned();
        if let Some(cached) = &cached {
            if cached.session_exp > now + REFRESH_MARGIN_SECS {
                return Ok(cached.session.clone());
            }
        }
        if let Some(cached) = cached.filter(|cached| cached.refresh_exp > now + REFRESH_MARGIN_SECS) {
            let params = serde_json::to_value(RefreshTokenRequest::new(cached.refresh)).unwrap_or_default();
            match self.krpc(host, VERIFY_HUB_PATH, "refresh_token", params, None).await {
                Ok(value) => {
                    if let Ok(pair) = serde_json::from_value::<TokenPair>(value) {
                        let token = CachedToken::from_pair(pair);
                        self.tokens.lock().unwrap().insert(key.clone(), token.clone());
                        return Ok(token.session);
                    }
                }
                // Not reached: the family is still good, try again later.
                Err(error @ CrossZoneError::Transport(_)) => return Err(error),
                Err(error) => log::info!("refresh at {host} refused, logging in again: {error}"),
            }
        }
        let target = AuthTarget::system(
            SystemServiceId::parse(service).map_err(|error| CrossZoneError::Unroutable(error.to_string()))?,
        );
        let scopes: Vec<SessionScope> = Vec::new();
        let value = self
            .send_login(host, || {
                let assertion =
                    sign_cross_zone_assertion(&self.signer, &principal.did, zone, &target, principal.app_id.as_deref())?;
                serde_json::to_value(CrossZoneLoginRequest { assertion, scopes: scopes.clone() })
                    .map_err(|error| RPCErrors::ReasonError(error.to_string()))
            })
            .await?;
        let pair: TokenPair = serde_json::from_value(value)
            .map_err(|error| CrossZoneError::Auth { code: None, detail: format!("login answer: {error}") })?;
        let token = CachedToken::from_pair(pair);
        self.tokens.lock().unwrap().insert(key.clone(), token.clone());
        log::info!(
            "logged in at {host} for {service} as {}",
            principal.did.to_string()
        );
        Ok(token.session)
    }

    /// Login with a fresh assertion per transport attempt (each assertion is single-use).
    async fn send_login(
        &self,
        host: &str,
        params: impl Fn() -> Result<Value, RPCErrors>,
    ) -> Result<Value, CrossZoneError> {
        let response = self
            .send(host, || {
                let params = params().map_err(|error| CrossZoneError::Auth { code: None, detail: error.to_string() })?;
                Ok(krpc_request(CROSS_ZONE_LOGIN_METHOD, params, None).timeout(LOGIN_TIMEOUT))
            })
            .await?;
        match krpc_answer(response).await {
            Err(CrossZoneError::Target(error)) => Err(CrossZoneError::Auth {
                code: CrossZoneLoginError::from_rpc_error(&error),
                detail: error.to_string(),
            }),
            other => other,
        }
    }

    async fn krpc(
        &self,
        host: &str,
        path: &str,
        method: &str,
        params: Value,
        token: Option<String>,
    ) -> Result<Value, CrossZoneError> {
        let mut request = krpc_request(method, params, token);
        request.path_and_query = path.to_string();
        let response = self.send(host, || Ok(request.clone())).await?;
        krpc_answer(response).await
    }

    /// Send over RTCP first unless it failed recently, then HTTPS. Only a failure to connect
    /// (or of a replayable request) moves on to the next transport; an answer never does.
    async fn send(
        &self,
        host: &str,
        build: impl Fn() -> Result<CrossZoneRequest, CrossZoneError>,
    ) -> Result<reqwest::Response, CrossZoneError> {
        let order = self.transport_order(host);
        let mut last_error = String::new();
        for (index, transport) in order.iter().enumerate() {
            let request = build()?;
            let replayable = request.replayable();
            let (client, base) = match transport {
                Transport::Rtcp => (self.rtcp.as_ref().expect("rtcp transport configured"), format!("http://{host}")),
                Transport::Https => (
                    &self.https,
                    self.origins.get(host).cloned().unwrap_or_else(|| format!("https://{host}")),
                ),
            };
            let url = format!("{}{}", base.trim_end_matches('/'), request.path_and_query);
            let mut builder = client.request(request.method.clone(), &url).header("host", host).timeout(request.timeout);
            for (name, value) in &request.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            match builder.send().await {
                Ok(response) => {
                    self.record(host, *transport, true);
                    return Ok(response);
                }
                Err(error) => {
                    log::info!("cross-zone {transport:?} to {host} failed: {error}");
                    self.record(host, *transport, false);
                    last_error = format!("{transport:?} {url}: {error}");
                    // A request that may have arrived is not sent again (the caller retries
                    // by its own idempotency key).
                    if !(error.is_connect() || replayable) || index + 1 == order.len() {
                        break;
                    }
                }
            }
        }
        Err(CrossZoneError::Transport(last_error))
    }

    fn transport_order(&self, host: &str) -> Vec<Transport> {
        if self.rtcp.is_none() {
            return vec![Transport::Https];
        }
        let links = self.links.lock().unwrap();
        match links.get(host) {
            Some(ZoneLink { state: TransportState::HttpsFallback, rtcp_failed_at: Some(at) })
                if at.elapsed() < RTCP_REPROBE_INTERVAL =>
            {
                vec![Transport::Https]
            }
            _ => vec![Transport::Rtcp, Transport::Https],
        }
    }

    fn record(&self, host: &str, transport: Transport, connected: bool) {
        let mut links = self.links.lock().unwrap();
        let link = links
            .entry(host.to_string())
            .or_insert(ZoneLink { state: TransportState::Probing, rtcp_failed_at: None });
        link.state = match (transport, connected) {
            (Transport::Rtcp, true) => {
                link.rtcp_failed_at = None;
                TransportState::RtcpOk
            }
            (Transport::Rtcp, false) => {
                link.rtcp_failed_at = Some(Instant::now());
                TransportState::HttpsFallback
            }
            (Transport::Https, true) => TransportState::HttpsFallback,
            (Transport::Https, false) => TransportState::Unreachable,
        };
    }
}

fn token_key(host: &str, service: &str, principal: &Principal) -> TokenKey {
    (host.to_string(), service.to_string(), principal.did.to_string(), principal.app_id.clone())
}

fn krpc_request(method: &str, params: Value, token: Option<String>) -> CrossZoneRequest {
    let request = RPCRequest {
        method: method.to_string(),
        params,
        seq: buckyos_get_unix_timestamp(),
        token,
        trace_id: None,
    };
    CrossZoneRequest::new(http::Method::POST, VERIFY_HUB_PATH)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&request).unwrap_or_default())
}

/// A kRPC answer: 5xx is a transport failure (the service is not there now), any other
/// non-200 status a refusal, and a kRPC error keeps its detail code.
async fn krpc_answer(response: reqwest::Response) -> Result<Value, CrossZoneError> {
    let status = response.status();
    if status.is_server_error() {
        return Err(CrossZoneError::Transport(format!("HTTP {status}")));
    }
    if !status.is_success() {
        return Err(CrossZoneError::Target(RPCErrors::ReasonError(format!("HTTP {status}"))));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| CrossZoneError::Transport(format!("kRPC answer: {error}")))?;
    let detail = RPCErrorDetail::from_response_value(&value);
    let response: RPCResponse = serde_json::from_value(value)
        .map_err(|error| CrossZoneError::Target(RPCErrors::ParserResponseError(error.to_string())))?;
    match response.result {
        RPCResult::Success(value) => Ok(value),
        RPCResult::Failed(error) => Err(CrossZoneError::Target(match detail {
            Some(detail) => RPCErrors::Detailed(detail),
            None => RPCErrors::ReasonError(error),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_zone_auth::tests::{fixture, signer, DEVICE_KEY};
    use crate::StaticCustodyResolver;
    use name_lib::DidDocType;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A Target Zone in one listener: verify-hub logins and refreshes, and a service that
    /// accepts only the latest session token.
    struct FakeTarget {
        logins: AtomicUsize,
        refreshes: AtomicUsize,
        served: AtomicUsize,
        session_ttl: u64,
        refuse_login: Option<CrossZoneLoginError>,
        current: Mutex<String>,
    }

    fn jwt_with_exp(exp: u64, tag: &str) -> String {
        use base64::Engine;
        let b64 = |value: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string());
        format!("{}.{}.sig", b64(json!({"alg": "EdDSA"})), b64(json!({"exp": exp, "tag": tag})))
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<(String, String)> {
        let mut data = Vec::new();
        loop {
            let mut chunk = [0u8; 8192];
            let size = socket.read(&mut chunk).await.ok()?;
            if size == 0 {
                return None;
            }
            data.extend_from_slice(&chunk[..size]);
            if let Some(split) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..split]).to_string();
                let length = head
                    .lines()
                    .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                if data.len() >= split + 4 + length {
                    return Some((head, String::from_utf8_lossy(&data[split + 4..split + 4 + length]).to_string()));
                }
            }
        }
    }

    impl FakeTarget {
        fn answer(&self, head: &str, body: &str) -> (u16, String) {
            let path = head.split_whitespace().nth(1).unwrap_or("");
            let now = buckyos_get_unix_timestamp();
            let pair = |tag: &str| {
                let session = jwt_with_exp(now + self.session_ttl, tag);
                *self.current.lock().unwrap() = session.clone();
                json!({"result": {"session_token": session, "refresh_token": jwt_with_exp(now + 3600, tag)}, "sys": [0]})
            };
            if path == VERIFY_HUB_PATH {
                let request: Value = serde_json::from_str(body).unwrap();
                let seq = request["sys"][0].clone();
                let mut answer = match request["method"].as_str() {
                    Some(CROSS_ZONE_LOGIN_METHOD) => {
                        self.logins.fetch_add(1, Ordering::SeqCst);
                        assert!(request["params"]["assertion"].as_str().is_some());
                        match self.refuse_login {
                            Some(code) => json!({"error": code.as_str(), "code": code.as_str(), "message": code.as_str(), "sys": [0]}),
                            None => pair("login"),
                        }
                    }
                    Some("refresh_token") => {
                        self.refreshes.fetch_add(1, Ordering::SeqCst);
                        pair("refresh")
                    }
                    _ => json!({"error": "unknown", "sys": [0]}),
                };
                answer["sys"] = json!([seq]);
                return (200, answer.to_string());
            }
            let bearer = head
                .lines()
                .find_map(|line| line.strip_prefix("authorization: Bearer ").or_else(|| line.strip_prefix("Authorization: Bearer ")))
                .unwrap_or("");
            if bearer != *self.current.lock().unwrap() {
                return (401, "{}".into());
            }
            self.served.fetch_add(1, Ordering::SeqCst);
            (200, json!({"ok": true}).to_string())
        }
    }

    async fn serve(target: Arc<FakeTarget>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let target = target.clone();
                tokio::spawn(async move {
                    while let Some((head, body)) = read_request(&mut socket).await {
                        let (status, answer) = target.answer(&head, &body);
                        let response = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{answer}",
                            answer.len()
                        );
                        if socket.write_all(response.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        origin
    }

    fn target(session_ttl: u64, refuse_login: Option<CrossZoneLoginError>) -> Arc<FakeTarget> {
        Arc::new(FakeTarget {
            logins: AtomicUsize::new(0),
            refreshes: AtomicUsize::new(0),
            served: AtomicUsize::new(0),
            session_ttl,
            refuse_login,
            current: Mutex::new(String::new()),
        })
    }

    fn resolver() -> Arc<StaticCustodyResolver> {
        Arc::new(fixture().with_doc(
            &DID::from_str("did:web:target.example").unwrap(),
            Some(DidDocType::Zone),
            json!({"id": "did:web:target.example", "hostname": "target.example", "owner": "did:web:target.example",
                "oods": [], "boot_jwt": "", "exp": 4102444800u64, "iat": 1,
                "verificationMethod": [], "authentication": []}),
        ))
    }

    async fn closed_port() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        drop(listener);
        addr
    }

    fn client(origin: &str, socks: Option<String>) -> CrossZoneClient {
        CrossZoneClient::new(
            signer("did:web:ood1.alice.example", DEVICE_KEY),
            socks.map(|addr| (addr, SocksCredentials { username: "u".into(), password: "p".into() })),
        )
        .unwrap()
        .with_resolver(resolver())
        .with_origins(HashMap::from([("target.example".to_string(), origin.to_string())]))
    }

    fn get() -> CrossZoneRequest {
        CrossZoneRequest::new(http::Method::GET, "/kapi/msg-center/x")
    }

    #[tokio::test]
    async fn logs_in_once_reuses_and_refreshes_the_target_token() {
        let zone = DID::from_str("did:web:target.example").unwrap();
        let bob = Principal::new(DID::from_str("did:web:bob.example").unwrap());
        // Access tokens expire within the refresh margin: every use refreshes.
        let fake = target(30, None);
        let origin = serve(fake.clone()).await;
        let client = client(&origin, None);
        for _ in 0..3 {
            let response = client.request(&zone, "msg-center", &bob, get()).await.unwrap();
            assert_eq!(response.status(), 200);
        }
        assert_eq!(fake.logins.load(Ordering::SeqCst), 1);
        assert_eq!(fake.refreshes.load(Ordering::SeqCst), 2);
        assert_eq!(fake.served.load(Ordering::SeqCst), 3);

        // A long-lived token is reused as is; a token the service no longer takes logs in again.
        let fake = target(3600, None);
        let origin = serve(fake.clone()).await;
        let client = super::tests::client(&origin, None);
        client.request(&zone, "msg-center", &bob, get()).await.unwrap();
        client.request(&zone, "msg-center", &bob, get()).await.unwrap();
        assert_eq!(fake.logins.load(Ordering::SeqCst), 1);
        *fake.current.lock().unwrap() = "rotated".into();
        let response = client.request(&zone, "msg-center", &bob, get()).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(fake.logins.load(Ordering::SeqCst), 2);
        // Another service or principal never reuses the token.
        client.request(&zone, "homestation", &bob, get()).await.unwrap();
        let carol = Principal::new(DID::from_str("did:web:carol.example").unwrap());
        client.request(&zone, "msg-center", &carol, get()).await.unwrap();
        assert_eq!(fake.logins.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn refused_logins_fail_without_switching_transport() {
        let zone = DID::from_str("did:web:target.example").unwrap();
        let bob = Principal::new(DID::from_str("did:web:bob.example").unwrap());
        let fake = target(3600, Some(CrossZoneLoginError::CustodyRejected));
        let origin = serve(fake.clone()).await;
        let error = client(&origin, None).request(&zone, "msg-center", &bob, get()).await.unwrap_err();
        assert!(matches!(error, CrossZoneError::Auth { code: Some(CrossZoneLoginError::CustodyRejected), .. }), "{error}");
        assert!(!error.is_retryable());
        assert_eq!(fake.logins.load(Ordering::SeqCst), 1);
    }

    /// A SOCKS5 entry that takes the credentials, then refuses every target the way the gateway
    /// does when RTCP cannot reach (or admit) it: `reply` is the SOCKS reply code.
    async fn refusing_socks(reply: u8) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0u8; 512];
                    let _ = socket.read(&mut buf).await; // greeting
                    socket.write_all(&[5, 2]).await.ok(); // username/password
                    let _ = socket.read(&mut buf).await; // credentials
                    socket.write_all(&[1, 0]).await.ok();
                    let _ = socket.read(&mut buf).await; // CONNECT
                    socket.write_all(&[5, reply, 0, 1, 0, 0, 0, 0, 0, 0]).await.ok();
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn a_refusing_rtcp_entry_is_a_transport_failure_too() {
        let zone = DID::from_str("did:web:target.example").unwrap();
        let bob = Principal::new(DID::from_str("did:web:bob.example").unwrap());
        for reply in [1u8, 4] {
            let fake = target(3600, None);
            let origin = serve(fake.clone()).await;
            let client = client(&origin, Some(refusing_socks(reply).await));
            let response = client.request(&zone, "msg-center", &bob, get()).await.unwrap();
            assert_eq!(response.status(), 200, "reply {reply}");
            assert_eq!(client.transport_state("target.example"), TransportState::HttpsFallback);
            assert_eq!(fake.logins.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn rtcp_failure_falls_back_to_https_and_probes_again_later() {
        let zone = DID::from_str("did:web:target.example").unwrap();
        let bob = Principal::new(DID::from_str("did:web:bob.example").unwrap());
        let fake = target(3600, None);
        let origin = serve(fake.clone()).await;
        let client = client(&origin, Some(closed_port().await));
        assert_eq!(client.transport_state("target.example"), TransportState::Probing);
        client.request(&zone, "msg-center", &bob, get()).await.unwrap();
        assert_eq!(client.transport_state("target.example"), TransportState::HttpsFallback);
        assert_eq!(client.transport_order("target.example"), vec![Transport::Https]);
        client.links.lock().unwrap().get_mut("target.example").unwrap().rtcp_failed_at =
            Some(Instant::now() - RTCP_REPROBE_INTERVAL);
        assert_eq!(client.transport_order("target.example"), vec![Transport::Rtcp, Transport::Https]);

        // Both down: a retryable transport error.
        let down = super::tests::client(&format!("http://{}", closed_port().await), Some(closed_port().await));
        let error = down.request(&zone, "msg-center", &bob, get()).await.unwrap_err();
        assert!(matches!(error, CrossZoneError::Transport(_)) && error.is_retryable(), "{error}");
        assert_eq!(down.transport_state("target.example"), TransportState::Unreachable);
        // A Zone without a document has no route.
        let error = down
            .request(&DID::from_str("did:web:nowhere.example").unwrap(), "msg-center", &bob, get())
            .await
            .unwrap_err();
        assert!(matches!(error, CrossZoneError::Unroutable(_)), "{error}");
    }
}
