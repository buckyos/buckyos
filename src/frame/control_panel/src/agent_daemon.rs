use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buckyos_api::*;
use buckyos_http_server::{server_err, ServerError, ServerErrorCode, ServerResult};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use http_body_util::combinators::BoxBody;
use hyper_util::rt::TokioIo;
use kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_tungstenite::{
    tungstenite::{
        protocol::{Role, WebSocketConfig},
        Message,
    },
    WebSocketStream,
};

use crate::{ControlPanelServer, RpcAuthPrincipal};

const RECORD_ROOT: &str = "services/control_panel/agent_executors";
type Socket = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn failure(message: impl ToString) -> RPCErrors {
    RPCErrors::ReasonError(message.to_string())
}
fn invalid(message: impl ToString) -> RPCErrors {
    RPCErrors::ParseRequestError(message.to_string())
}
fn forbidden() -> RPCErrors {
    RPCErrors::InvalidToken("executor access denied".into())
}
fn record_key(id: &str) -> Result<String, RPCErrors> {
    if id.len() != 36 || uuid::Uuid::parse_str(id).is_err() {
        return Err(invalid("invalid executor_id"));
    }
    Ok(format!("{RECORD_ROOT}/{id}"))
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    device: AgentDaemonDevice,
    credential_sha256: String,
}

impl Record {
    fn active(&self, now: u64) -> bool {
        self.device.enabled
            && !self.credential_sha256.is_empty()
            && self.device.expires_at_ms.is_none_or(|expiry| now < expiry)
    }
}

struct Connection {
    generation: u64,
    instance_id: String,
    resume_sha256: String,
    last_heartbeat: Instant,
    ready: bool,
    connected: bool,
    store_id: String,
    capabilities: Value,
    sender: mpsc::Sender<AgentDaemonServerMessage>,
    cancel: watch::Sender<bool>,
    config: ExecutorConfig,
    stream_slots: Arc<Semaphore>,
}

impl Connection {
    fn accepts_reconnect(&self, instance_id: &str, resume_token: Option<&str>) -> bool {
        self.last_heartbeat.elapsed() >= Duration::from_millis(AGENT_DAEMON_LEASE_MS)
            || (self.instance_id == instance_id
                && resume_token.is_some_and(|token| digest(token) == self.resume_sha256))
    }
    fn live(&self) -> bool {
        self.connected
            && self.ready
            && self.last_heartbeat.elapsed() < Duration::from_millis(AGENT_DAEMON_LEASE_MS)
    }
    fn target(&self, record: &Record, profiles: Vec<String>) -> AgentDaemonOnlineTarget {
        AgentDaemonOnlineTarget {
            executor_id: record.device.executor_id.clone(),
            name: record.device.name.clone(),
            generation: self.generation,
            store_id: self.store_id.clone(),
            profiles,
            capabilities: self.capabilities.clone(),
            expires_at_ms: record.device.expires_at_ms,
        }
    }
}

struct PendingStream {
    _slot: OwnedSemaphorePermit,
    executor_id: String,
    generation: u64,
    expires_at_ms: u64,
    session_expires_at_ms: u64,
    agent_sha256: String,
    daemon_sha256: String,
    agent: Option<Socket>,
    daemon: Option<Socket>,
    cancel: watch::Receiver<bool>,
}

#[derive(Default)]
pub(crate) struct AgentDaemonHub {
    connections: Mutex<HashMap<String, Connection>>,
    streams: Mutex<HashMap<String, PendingStream>>,
    mutations: Mutex<()>,
}

async fn config_client() -> Result<Arc<SystemConfigClient>, RPCErrors> {
    get_buckyos_api_runtime()?.get_system_config_client().await
}

async fn load_record(id: &str) -> Result<Record, RPCErrors> {
    let raw = config_client()
        .await?
        .get(&record_key(id)?)
        .await
        .map_err(failure)?;
    serde_json::from_str(&raw.value).map_err(failure)
}

async fn save_record(record: &Record) -> Result<(), RPCErrors> {
    config_client()
        .await?
        .set(
            &record_key(&record.device.executor_id)?,
            &serde_json::to_string(record).map_err(failure)?,
        )
        .await
        .map_err(failure)?;
    Ok(())
}

async fn records() -> Result<Vec<Record>, RPCErrors> {
    let client = config_client().await?;
    let mut result = Vec::new();
    for id in client.list(RECORD_ROOT).await.map_err(failure)? {
        result.push(load_record(&id).await?);
    }
    Ok(result)
}

async fn snapshot(
    record: &Record,
    previous: Option<&ExecutorConfig>,
) -> Result<ExecutorConfig, RPCErrors> {
    let client = config_client().await?;
    let owner = &record.device.owner_user_id;
    let entries = client
        .list(&format!("users/{owner}/agents"))
        .await
        .map_err(failure)?;
    let mut specs = Vec::new();
    for entry in entries {
        let agent_id = AgentId::parse(&entry).map_err(failure)?;
        let raw = client
            .get(&agent_spec_key(owner, &agent_id))
            .await
            .map_err(failure)?;
        let spec: AgentSpec = serde_json::from_str(&raw.value).map_err(failure)?;
        spec.validate().map_err(failure)?;
        if spec.agent_id != agent_id || spec.binding.target_app_instance_id.owner_user_id() != owner
        {
            return Err(failure("Agent binding identity mismatch"));
        }
        specs.push(spec);
    }
    let mut bindings = BTreeMap::new();
    for spec in &specs {
        let did = spec.agent_did.to_string();
        if !record
            .device
            .profiles
            .values()
            .any(|profile| profile.principals.contains(&did))
        {
            continue;
        }
        let instance = &spec.binding.target_app_instance_id;
        if instance.app_id().app_did().to_string() != did
            || specs
                .iter()
                .filter(|other| other.binding.target_app_instance_id == *instance)
                .count()
                != 1
        {
            continue;
        }
        let enabled = match client.get(&agent_settings_key(owner, &spec.agent_id)).await {
            Ok(raw) => {
                serde_json::from_str::<AgentSettings>(&raw.value)
                    .map_err(failure)?
                    .enabled
            }
            Err(SystemConfigError::KeyNotFound(_)) => true,
            Err(error) => return Err(failure(error)),
        };
        let installed = match client
            .get(&user_app_spec_key(owner, instance.app_id()))
            .await
        {
            Ok(raw) => {
                let app: AppServiceSpec = serde_json::from_str(&raw.value).map_err(failure)?;
                app.app_instance_id == *instance && app.is_installed()
            }
            Err(SystemConfigError::KeyNotFound(_)) => false,
            Err(error) => return Err(failure(error)),
        };
        bindings.insert(
            did,
            ExecutorPrincipalBinding {
                app_instance_id: instance.clone(),
                agent_generation: spec.generation,
                binding_generation: spec.binding.generation,
                enabled,
                app_installed: installed,
                permission_mode: spec.permission_mode,
            },
        );
    }
    let revision = executor_revision(&(record.device.clone(), &bindings));
    let generation = previous
        .map(|config| {
            if config.revision == revision {
                config.generation
            } else {
                config.generation.saturating_add(1)
            }
        })
        .unwrap_or(record.device.generation);
    let deadline = now_ms()
        .saturating_add(AGENT_DAEMON_LEASE_MS)
        .min(record.device.expires_at_ms.unwrap_or(u64::MAX));
    Ok(ExecutorConfig {
        enabled: record.active(now_ms()),
        revision,
        generation,
        auth_valid_until_ms: deadline,
        allowed_devices: Vec::new(),
        principal_bindings: bindings,
        profiles: record.device.profiles.clone(),
        limits: record.device.limits.clone(),
    })
}

fn authorized_profiles(config: &ExecutorConfig, app: &AppInstanceId) -> Vec<String> {
    let bound: Vec<_> = config
        .principal_bindings
        .iter()
        .filter(|(_, binding)| &binding.app_instance_id == app)
        .collect();
    if bound.len() != 1 {
        return Vec::new();
    }
    config
        .profiles
        .keys()
        .filter(|profile| {
            config
                .granted_ops(profile, bound[0].0)
                .contains(&ExecutorOp::OpenStream)
        })
        .cloned()
        .collect()
}

async fn verified_session(token: &str) -> Result<AgentDaemonSession, RPCErrors> {
    let parsed = get_buckyos_api_runtime()?
        .verify_trusted_session_token(token)
        .await?;
    let claims = validate_verify_hub_token_claims(&parsed, TokenUse::Session)?;
    if claims.principal_kind != TokenPrincipalKind::App {
        return Err(forbidden());
    }
    let AuthTarget::App { app_instance_id } = &claims.target else {
        return Err(forbidden());
    };
    let subject = parsed.sub.clone().ok_or_else(forbidden)?;
    if subject != app_instance_id.owner_user_id() {
        return Err(forbidden());
    }
    let expires_at_ms = parsed.exp.ok_or_else(forbidden)?.saturating_mul(1000);
    if expires_at_ms <= now_ms() {
        return Err(forbidden());
    }
    check_agent_runtime_session(config_client().await?.as_ref(), app_instance_id).await?;
    Ok(AgentDaemonSession {
        subject,
        principal_kind: claims.principal_kind,
        token_use: claims.token_use,
        app_instance_id: Some(app_instance_id.clone()),
        authorization_key: claims.target.authorization_key(),
        expires_at_ms,
    })
}

impl AgentDaemonHub {
    async fn disconnect(&self, id: &str, generation: u64) {
        if let Some(connection) = self.connections.lock().await.get_mut(id) {
            if connection.generation == generation {
                connection.ready = false;
                connection.connected = false;
                connection.cancel.send_replace(true);
            }
        }
        self.streams
            .lock()
            .await
            .retain(|_, stream| stream.executor_id != id || stream.generation != generation);
    }

    async fn revoke(&self, id: &str) {
        if let Some(connection) = self.connections.lock().await.remove(id) {
            connection.cancel.send_replace(true);
            let _ = connection
                .sender
                .try_send(AgentDaemonServerMessage::Closing {
                    reason: "revoked".into(),
                });
        }
        self.streams
            .lock()
            .await
            .retain(|_, stream| stream.executor_id != id);
    }
}

impl ControlPanelServer {
    fn executor_public_base(&self) -> Result<url::Url, RPCErrors> {
        #[cfg(test)]
        if let Ok(origin) = std::env::var("BUCKYOS_EXECUTOR_FIXTURE_ORIGIN") {
            let url = url::Url::parse(&origin).map_err(failure)?;
            if url.scheme() != "wss"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(invalid(
                    "fixture origin must be a WSS origin without credentials",
                ));
            }
            return Ok(url);
        }
        let runtime = get_buckyos_api_runtime()?;
        let host = Self::sso_zone_host(runtime)?;
        url::Url::parse(&format!("wss://{host}")).map_err(failure)
    }

    pub(super) async fn handle_agent_daemon_rpc(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let result = match req.method.as_str() {
            "executor.online.list" => {
                let token = Self::extract_rpc_session_token(&req).ok_or_else(forbidden)?;
                let session = verified_session(&token).await?;
                let app = session.app_instance_id.as_ref().ok_or_else(forbidden)?;
                let mut targets = Vec::new();
                for record in records().await? {
                    if record.device.owner_user_id != session.subject || !record.active(now_ms()) {
                        continue;
                    }
                    let config = snapshot(&record, None).await?;
                    let profiles = authorized_profiles(&config, app);
                    if profiles.is_empty() {
                        continue;
                    }
                    if let Some(connection) = self
                        .agent_daemon
                        .connections
                        .lock()
                        .await
                        .get(&record.device.executor_id)
                        .filter(|connection| connection.live())
                    {
                        targets.push(connection.target(&record, profiles));
                    }
                }
                json!({"executors": targets})
            }
            "executor.stream.open" => self.open_executor_stream(&req).await?,
            _ => {
                crate::user_mgr::require_control_panel_admin(principal)?;
                let _mutation = self.agent_daemon.mutations.lock().await;
                match req.method.as_str() {
                    "executor.device.list" => {
                        let mut devices = Vec::new();
                        for record in records()
                            .await?
                            .into_iter()
                            .filter(|record| record.device.owner_user_id == principal.owner_user_id)
                        {
                            let online = self
                                .agent_daemon
                                .connections
                                .lock()
                                .await
                                .get(&record.device.executor_id)
                                .filter(|connection| record.active(now_ms()) && connection.live())
                                .map(|connection| {
                                    connection.target(
                                        &record,
                                        record.device.profiles.keys().cloned().collect(),
                                    )
                                });
                            devices.push(json!({"device": record.device, "online": online}));
                        }
                        json!({"devices": devices})
                    }
                    "executor.device.create" => {
                        let name = Self::require_param_str(&req, "name")?.trim().to_string();
                        if name.is_empty() || name.len() > 128 {
                            return Err(invalid("device name must be 1..128 bytes"));
                        }
                        let profiles: BTreeMap<String, ExecutorProfile> = serde_json::from_value(
                            req.params
                                .get("profiles")
                                .cloned()
                                .ok_or_else(|| invalid("profiles are required"))?,
                        )
                        .map_err(invalid)?;
                        if profiles.is_empty() || profiles.len() > 32 {
                            return Err(invalid("1..32 profiles are required"));
                        }
                        for (id, profile) in &profiles {
                            profile.validate(id).map_err(invalid)?;
                            if profile.run_as.is_some() {
                                return Err(invalid(
                                    "lightweight executors run as the current OS user",
                                ));
                            }
                        }
                        let limits: ExecutorLimits = serde_json::from_value(
                            req.params.get("limits").cloned().unwrap_or(json!({})),
                        )
                        .map_err(invalid)?;
                        limits.validate().map_err(invalid)?;
                        let expires_at_ms = expiry(&req)?;
                        let credential = secret();
                        let record = Record {
                            device: AgentDaemonDevice {
                                executor_id: uuid::Uuid::new_v4().to_string(),
                                name,
                                owner_user_id: principal.owner_user_id.clone(),
                                enabled: true,
                                expires_at_ms,
                                generation: 1,
                                profiles,
                                limits,
                                created_at_ms: now_ms(),
                            },
                            credential_sha256: digest(&credential),
                        };
                        let config = snapshot(&record, None).await?;
                        if record
                            .device
                            .profiles
                            .values()
                            .flat_map(|profile| &profile.principals)
                            .any(|did| {
                                !config
                                    .principal_bindings
                                    .get(did)
                                    .is_some_and(|binding| binding.enabled && binding.app_installed)
                            })
                        {
                            return Err(invalid("all principals must identify enabled, installed Agents owned by the caller"));
                        }
                        let output = self.executor_issued(&record, &credential)?;
                        save_record(&record).await?;
                        output
                    }
                    "executor.device.issue"
                    | "executor.device.revoke"
                    | "executor.device.delete" => {
                        let id = Self::require_param_str(&req, "executor_id")?;
                        let mut record = load_record(&id).await?;
                        if record.device.owner_user_id != principal.owner_user_id {
                            return Err(forbidden());
                        }
                        if req.method == "executor.device.delete" {
                            config_client()
                                .await?
                                .delete(&record_key(&id)?)
                                .await
                                .map_err(failure)?;
                            self.agent_daemon.revoke(&id).await;
                            json!({})
                        } else {
                            record.device.generation += 1;
                            let credential = secret();
                            if req.method == "executor.device.issue" {
                                record.device.enabled = true;
                                record.device.expires_at_ms = expiry(&req)?;
                                record.credential_sha256 = digest(&credential);
                            } else {
                                record.device.enabled = false;
                                record.credential_sha256.clear();
                            }
                            let output = if record.device.enabled {
                                self.executor_issued(&record, &credential)?
                            } else {
                                json!({})
                            };
                            save_record(&record).await?;
                            self.agent_daemon.revoke(&id).await;
                            output
                        }
                    }
                    _ => return Err(invalid("unknown executor method")),
                }
            }
        };
        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }

    fn executor_issued(&self, record: &Record, credential: &str) -> Result<Value, RPCErrors> {
        let mut url = self.executor_public_base()?;
        url.set_path(AGENT_DAEMON_MANAGE_PATH);
        url.query_pairs_mut()
            .append_pair("executor_id", &record.device.executor_id);
        url.set_fragment(Some(credential));
        Ok(
            json!({"device": record.device, "access_url": url.as_str(), "start_command": format!("./start-buckyos-agent-daemon.sh '{}'", url.as_str())}),
        )
    }

    async fn open_executor_stream(&self, req: &RPCRequest) -> Result<Value, RPCErrors> {
        let token = Self::extract_rpc_session_token(req).ok_or_else(forbidden)?;
        let mut session = verified_session(&token).await?;
        let id = Self::require_param_str(req, "executor_id")?;
        let generation = req
            .params
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("generation required"))?;
        let profile = Self::require_param_str(req, "profile_id")?;
        let record = load_record(&id).await?;
        if !record.active(now_ms()) || record.device.owner_user_id != session.subject {
            return Err(forbidden());
        }
        let config = snapshot(&record, None).await?;
        if !authorized_profiles(
            &config,
            session.app_instance_id.as_ref().ok_or_else(forbidden)?,
        )
        .contains(&profile)
        {
            return Err(forbidden());
        }
        session.expires_at_ms = session
            .expires_at_ms
            .min(record.device.expires_at_ms.unwrap_or(u64::MAX));
        let expires_at_ms = now_ms()
            .saturating_add(AGENT_DAEMON_STREAM_TTL_MS)
            .min(session.expires_at_ms);
        let stream_id = uuid::Uuid::new_v4().to_string();
        let agent_ticket = secret();
        let daemon_ticket = secret();
        let connections = self.agent_daemon.connections.lock().await;
        let connection = connections
            .get(&id)
            .filter(|connection| connection.live() && connection.generation == generation)
            .ok_or_else(|| failure("executor is offline or generation is stale"))?;
        let slot = connection
            .stream_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| failure("executor stream limit reached"))?;
        let mut streams = self.agent_daemon.streams.lock().await;
        streams.retain(|_, stream| stream.expires_at_ms > now_ms());
        if streams
            .values()
            .filter(|stream| stream.executor_id == id)
            .count()
            >= record.device.limits.max_streams as usize
        {
            return Err(failure("too many pending streams"));
        }
        streams.insert(
            stream_id.clone(),
            PendingStream {
                _slot: slot,
                executor_id: id.clone(),
                generation,
                expires_at_ms,
                session_expires_at_ms: session.expires_at_ms,
                agent_sha256: digest(&agent_ticket),
                daemon_sha256: digest(&daemon_ticket),
                agent: None,
                daemon: None,
                cancel: connection.cancel.subscribe(),
            },
        );
        let message = AgentDaemonServerMessage::OpenStream {
            generation,
            stream_id: stream_id.clone(),
            ticket: daemon_ticket,
            expires_at_ms,
            profile_id: profile.clone(),
            session,
            session_token_sha256: digest(&token),
        };
        if connection.sender.try_send(message).is_err() {
            streams.remove(&stream_id);
            return Err(failure("executor management connection unavailable"));
        }
        let mut ws_url = self.executor_public_base()?;
        ws_url.set_path(&format!("{AGENT_DAEMON_ROUTE_PATH}/{stream_id}"));
        Ok(json!(AgentDaemonStreamRoute {
            executor_id: id,
            generation,
            profile_id: profile,
            stream_id,
            ws_url: ws_url.to_string(),
            ticket: agent_ticket,
            expires_at_ms
        }))
    }
}

fn expiry(req: &RPCRequest) -> Result<Option<u64>, RPCErrors> {
    match req.params.get("expires_at_ms") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let expiry = value
                .as_u64()
                .filter(|expiry| *expiry > now_ms())
                .ok_or_else(|| {
                    invalid("expires_at_ms must be a future Unix time in milliseconds")
                })?;
            Ok(Some(expiry))
        }
    }
}

async fn send_control(socket: &mut Socket, message: &AgentDaemonServerMessage) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

impl ControlPanelServer {
    pub(super) async fn serve_agent_daemon_ws(
        &self,
        mut req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let path = req.uri().path().to_string();
        let bearer = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(ToOwned::to_owned);
        let manage = path == AGENT_DAEMON_MANAGE_PATH;
        let reject = |status, message: &str| {
            let mut response =
                http::Response::new(Self::boxed_http_body(message.as_bytes().to_vec()));
            *response.status_mut() = status;
            Ok(response)
        };
        let (id, daemon_side) = if manage {
            let Some(id) =
                url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes())
                    .find(|(key, _)| key == "executor_id")
                    .map(|(_, value)| value.into_owned())
            else {
                return reject(http::StatusCode::FORBIDDEN, "executor credentials required");
            };
            let Ok(record) = load_record(&id).await else {
                return reject(http::StatusCode::FORBIDDEN, "executor credentials rejected");
            };
            if !record.active(now_ms())
                || bearer
                    .as_ref()
                    .is_none_or(|token| digest(token) != record.credential_sha256)
            {
                return reject(http::StatusCode::FORBIDDEN, "executor credentials rejected");
            }
            (id, true)
        } else if let Some(id) = path.strip_prefix(&format!("{AGENT_DAEMON_DATA_PATH}/")) {
            if bearer.is_none() {
                return reject(http::StatusCode::FORBIDDEN, "stream credentials required");
            }
            (id.to_string(), true)
        } else if let Some(id) = path.strip_prefix(&format!("{AGENT_DAEMON_ROUTE_PATH}/")) {
            (id.to_string(), false)
        } else {
            return reject(http::StatusCode::NOT_FOUND, "unknown executor endpoint");
        };
        let mut head = http::Request::builder()
            .method(req.method())
            .uri(req.uri())
            .version(req.version())
            .body(())
            .map_err(|_| server_err!(ServerErrorCode::BadRequest, "invalid websocket request"))?;
        *head.headers_mut() = req.headers().clone();
        let response = tokio_tungstenite::tungstenite::handshake::server::create_response(&head)
            .map_err(|_| server_err!(ServerErrorCode::BadRequest, "invalid websocket upgrade"))?;
        let upgrade = hyper::upgrade::on(&mut req);
        let server = self.clone();
        tokio::spawn(async move {
            let Ok(Ok(upgraded)) = tokio::time::timeout(Duration::from_secs(5), upgrade).await
            else {
                return;
            };
            let config = WebSocketConfig::default()
                .max_message_size(Some(2 * 1024 * 1024))
                .max_frame_size(Some(2 * 1024 * 1024));
            let socket = WebSocketStream::from_raw_socket(
                TokioIo::new(upgraded),
                Role::Server,
                Some(config),
            )
            .await;
            if manage {
                server
                    .manage_executor(socket, id, bearer.unwrap_or_default())
                    .await;
            } else {
                server
                    .accept_executor_stream(socket, id, daemon_side, bearer)
                    .await;
            }
        });
        let (parts, _) = response.into_parts();
        Ok(http::Response::from_parts(
            parts,
            Self::boxed_http_body(Vec::new()),
        ))
    }

    async fn manage_executor(&self, mut socket: Socket, id: String, credential: String) {
        let hello = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
        let Ok(Some(Ok(Message::Text(text)))) = hello else {
            return;
        };
        let Ok(AgentDaemonClientMessage::Hello {
            protocol_version,
            executor_id,
            instance_id,
            resume_token,
        }) = serde_json::from_str(&text)
        else {
            return;
        };
        if protocol_version != AGENT_DAEMON_PROTOCOL_VERSION
            || executor_id != id
            || instance_id.is_empty()
            || instance_id.len() > 128
        {
            return;
        }
        let registration = async {
            let _mutation = self.agent_daemon.mutations.lock().await;
            let record = load_record(&id).await?;
            if !record.active(now_ms()) || digest(&credential) != record.credential_sha256 {
                return Err(forbidden());
            }
            let previous = self
                .agent_daemon
                .connections
                .lock()
                .await
                .get(&id)
                .map(|connection| connection.config.clone());
            let config = snapshot(&record, previous.as_ref()).await?;
            let mut connections = self.agent_daemon.connections.lock().await;
            if let Some(old) = connections.get(&id) {
                if !old.accepts_reconnect(&instance_id, resume_token.as_deref()) {
                    return Err(failure("executor already has an active instance"));
                }
                old.cancel.send_replace(true);
            }
            let generation = (OsRng.next_u64() & ((1u64 << 53) - 1)).max(1);
            let resume_token = secret();
            let (sender, receiver) = mpsc::channel(32);
            let (cancel, cancel_rx) = watch::channel(false);
            connections.insert(
                id.clone(),
                Connection {
                    generation,
                    instance_id,
                    resume_sha256: digest(&resume_token),
                    last_heartbeat: Instant::now(),
                    ready: false,
                    connected: true,
                    store_id: String::new(),
                    capabilities: json!({}),
                    sender,
                    cancel,
                    stream_slots: Arc::new(Semaphore::new(config.limits.max_streams as usize)),
                    config: config.clone(),
                },
            );
            Ok((
                record,
                config,
                generation,
                resume_token,
                receiver,
                cancel_rx,
            ))
        }
        .await;
        let (record, config, generation, resume_token, mut messages, mut cancel) =
            match registration {
                Ok(registration) => registration,
                Err(error) => {
                    let _ = send_control(
                        &mut socket,
                        &AgentDaemonServerMessage::Closing {
                            reason: error.to_string(),
                        },
                    )
                    .await;
                    return;
                }
            };
        let welcome = AgentDaemonServerMessage::Welcome {
            protocol_version: AGENT_DAEMON_PROTOCOL_VERSION,
            executor_id: id.clone(),
            generation,
            resume_token,
            server_time_ms: now_ms(),
            expires_at_ms: record.device.expires_at_ms,
            heartbeat_interval_ms: AGENT_DAEMON_HEARTBEAT_MS,
            lease_timeout_ms: AGENT_DAEMON_LEASE_MS,
            config,
        };
        if send_control(&mut socket, &welcome).await.is_err() {
            self.agent_daemon.disconnect(&id, generation).await;
            return;
        }
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut refreshed = Instant::now();
        let mut close_reason = "disconnected";
        loop {
            tokio::select! {
                biased;
                _ = cancel.changed() => { close_reason = "revoked_or_replaced"; break; },
                _ = tick.tick() => {
                    let valid = self.agent_daemon.connections.lock().await.get(&id).is_some_and(|connection| connection.generation == generation && connection.last_heartbeat.elapsed() < Duration::from_millis(AGENT_DAEMON_LEASE_MS));
                    if !valid { close_reason = "heartbeat_timeout"; break; }
                    if record.device.expires_at_ms.is_some_and(|expiry| now_ms() >= expiry) { close_reason = "expired"; break; }
                    self.agent_daemon.streams.lock().await.retain(|_, stream| stream.expires_at_ms > now_ms());
                    if refreshed.elapsed() >= Duration::from_millis(AGENT_DAEMON_HEARTBEAT_MS) {
                        let refreshed_config = async {
                            let record = load_record(&id).await?;
                            if !record.active(now_ms()) || digest(&credential) != record.credential_sha256 { return Err(forbidden()); }
                            let previous = self.agent_daemon.connections.lock().await.get(&id).map(|connection| connection.config.clone());
                            let config = snapshot(&record, previous.as_ref()).await?;
                            Ok((record, config))
                        }.await;
                        let Ok((record, config)): Result<(Record, ExecutorConfig), RPCErrors> = refreshed_config else { close_reason = "authorization_unavailable"; break; };
                        if let Some(connection) = self.agent_daemon.connections.lock().await.get_mut(&id).filter(|connection| connection.generation == generation) { connection.config = config.clone(); }
                        let update = AgentDaemonServerMessage::Snapshot { generation, server_time_ms: now_ms(), expires_at_ms: record.device.expires_at_ms, config };
                        if send_control(&mut socket, &update).await.is_err() { break; }
                        refreshed = Instant::now();
                    }
                },
                outgoing = messages.recv() => {
                    let Some(outgoing) = outgoing else { break; };
                    let close = matches!(outgoing, AgentDaemonServerMessage::Closing { .. });
                    if send_control(&mut socket, &outgoing).await.is_err() || close { break; }
                },
                incoming = socket.next() => {
                    match incoming {
                        Some(Ok(Message::Text(text))) => {
                            let Ok(incoming) = serde_json::from_str::<AgentDaemonClientMessage>(&text) else { break; };
                            let mut connections = self.agent_daemon.connections.lock().await;
                            let Some(connection) = connections.get_mut(&id).filter(|connection| connection.generation == generation) else { break; };
                            match incoming {
                                AgentDaemonClientMessage::Ready { generation: received, store_id, capabilities } if received == generation && !store_id.is_empty() && store_id.len() <= 128 => {
                                    connection.store_id = store_id;
                                    connection.capabilities = capabilities;
                                    connection.ready = true;
                                    connection.last_heartbeat = Instant::now();
                                },
                                AgentDaemonClientMessage::Heartbeat { generation: received } if received == generation => connection.last_heartbeat = Instant::now(),
                                AgentDaemonClientMessage::Goodbye { generation: received } if received == generation => { close_reason = "stopped"; break; },
                                _ => break,
                            }
                        },
                        Some(Ok(Message::Ping(payload))) => { if socket.send(Message::Pong(payload)).await.is_err() { break; } },
                        Some(Ok(Message::Pong(_))) => {},
                        _ => break,
                    }
                },
            }
        }
        self.agent_daemon.disconnect(&id, generation).await;
        if close_reason == "stopped" {
            let mut connections = self.agent_daemon.connections.lock().await;
            if connections
                .get(&id)
                .is_some_and(|connection| connection.generation == generation)
            {
                connections.remove(&id);
            }
        }
        let _ = send_control(
            &mut socket,
            &AgentDaemonServerMessage::Closing {
                reason: close_reason.into(),
            },
        )
        .await;
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
    }

    async fn accept_executor_stream(
        &self,
        mut socket: Socket,
        id: String,
        daemon_side: bool,
        mut ticket: Option<String>,
    ) {
        let browser_auth = ticket.is_none() && !daemon_side;
        if browser_auth {
            let Ok(Some(Ok(Message::Text(text)))) =
                tokio::time::timeout(Duration::from_secs(5), socket.next()).await
            else {
                return;
            };
            ticket = serde_json::from_str::<Value>(&text).ok().and_then(|value| {
                value
                    .get("ticket")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        }
        let Some(ticket) = ticket else {
            return;
        };
        let connections = self.agent_daemon.connections.lock().await;
        let mut streams = self.agent_daemon.streams.lock().await;
        let Some(stream) = streams.get_mut(&id) else {
            return;
        };
        let expected = if daemon_side {
            &stream.daemon_sha256
        } else {
            &stream.agent_sha256
        };
        if expected.is_empty()
            || digest(&ticket) != *expected
            || stream.expires_at_ms <= now_ms()
            || *stream.cancel.borrow()
            || !connections
                .get(&stream.executor_id)
                .is_some_and(|connection| {
                    connection.live() && connection.generation == stream.generation
                })
        {
            return;
        }
        if browser_auth
            && socket
                .send(Message::Text("{\"ready\":true}".into()))
                .await
                .is_err()
        {
            return;
        }
        if daemon_side {
            stream.daemon_sha256.clear();
            stream.daemon = Some(socket);
        } else {
            stream.agent_sha256.clear();
            stream.agent = Some(socket);
        }
        if stream.agent.is_some() && stream.daemon.is_some() {
            let stream = streams.remove(&id).unwrap();
            tokio::spawn(relay_stream(stream));
        }
    }
}

async fn relay_stream(mut stream: PendingStream) {
    let mut agent = stream.agent.take().unwrap();
    let mut daemon = stream.daemon.take().unwrap();
    let deadline = tokio::time::sleep(Duration::from_millis(
        stream.session_expires_at_ms.saturating_sub(now_ms()),
    ));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = stream.cancel.changed() => break,
            _ = &mut deadline => break,
            message = agent.next() => match message {
                Some(Ok(Message::Binary(bytes))) => if tokio::time::timeout(Duration::from_secs(10), daemon.send(Message::Binary(bytes))).await.map(|result| result.is_err()).unwrap_or(true) { break; },
                Some(Ok(Message::Ping(bytes))) => if agent.send(Message::Pong(bytes)).await.is_err() { break; },
                Some(Ok(Message::Pong(_))) => {},
                _ => break,
            },
            message = daemon.next() => match message {
                Some(Ok(Message::Binary(bytes))) => if tokio::time::timeout(Duration::from_secs(10), agent.send(Message::Binary(bytes))).await.map(|result| result.is_err()).unwrap_or(true) { break; },
                Some(Ok(Message::Ping(bytes))) => if daemon.send(Message::Pong(bytes)).await.is_err() { break; },
                Some(Ok(Message::Pong(_))) => {},
                _ => break,
            },
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), agent.close(None)).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), daemon.close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    fn configuration() -> ExecutorConfig {
        ExecutorConfig {
            enabled: true,
            revision: "sha256:test".into(),
            generation: 1,
            auth_valid_until_ms: now_ms() + AGENT_DAEMON_LEASE_MS,
            allowed_devices: Vec::new(),
            principal_bindings: BTreeMap::from([(
                "did:bns:test.buckyos".into(),
                ExecutorPrincipalBinding {
                    app_instance_id: "test.buckyos@alice".parse().unwrap(),
                    agent_generation: 1,
                    binding_generation: 1,
                    enabled: true,
                    app_installed: true,
                    permission_mode: AgentPermissionMode::ReadWrite,
                },
            )]),
            profiles: BTreeMap::from([(
                "default".into(),
                ExecutorProfile {
                    run_as: None,
                    workdir_roots: vec!["/tmp".into()],
                    env: BTreeMap::new(),
                    principals: vec!["did:bns:test.buckyos".into()],
                    ops: ExecutorOp::ALL.to_vec(),
                },
            )]),
            limits: ExecutorLimits::default(),
        }
    }

    fn connection(generation: u64) -> Connection {
        let (sender, _) = mpsc::channel(4);
        let (cancel, _) = watch::channel(false);
        Connection {
            generation,
            instance_id: "instance".into(),
            resume_sha256: digest("resume"),
            last_heartbeat: Instant::now(),
            ready: true,
            connected: true,
            store_id: "store".into(),
            capabilities: json!({"ui":"unsupported"}),
            sender,
            cancel,
            config: configuration(),
            stream_slots: Arc::new(Semaphore::new(4)),
        }
    }

    #[test]
    fn reconnect_requires_server_secret_and_expired_lease_is_not_online() {
        let mut connection = connection(1);
        assert!(connection.live());
        assert!(!connection.accepts_reconnect("instance", None));
        assert!(!connection.accepts_reconnect("another", Some("resume")));
        assert!(!connection.accepts_reconnect("instance", Some("wrong")));
        assert!(connection.accepts_reconnect("instance", Some("resume")));
        connection.last_heartbeat -= Duration::from_millis(AGENT_DAEMON_LEASE_MS + 1);
        assert!(!connection.live());
        assert!(connection.accepts_reconnect("another", None));
    }

    #[test]
    fn authorized_directory_requires_exact_enabled_installed_app() {
        let mut config = configuration();
        let app = "test.buckyos@alice".parse().unwrap();
        assert_eq!(authorized_profiles(&config, &app), vec!["default"]);
        assert!(authorized_profiles(&config, &"test.buckyos@bob".parse().unwrap()).is_empty());
        config
            .principal_bindings
            .values_mut()
            .next()
            .unwrap()
            .enabled = false;
        assert!(authorized_profiles(&config, &app).is_empty());
        config
            .principal_bindings
            .values_mut()
            .next()
            .unwrap()
            .enabled = true;
        config
            .principal_bindings
            .values_mut()
            .next()
            .unwrap()
            .app_installed = false;
        assert!(authorized_profiles(&config, &app).is_empty());
    }

    #[test]
    fn expiry_and_profile_read_only_boundaries() {
        let mut config = configuration();
        config
            .principal_bindings
            .values_mut()
            .next()
            .unwrap()
            .permission_mode = AgentPermissionMode::ReadOnly;
        let ops = config.granted_ops("default", "did:bns:test.buckyos");
        assert!(ops.contains(&ExecutorOp::Read));
        assert!(!ops.contains(&ExecutorOp::Exec));
        assert!(!ops.contains(&ExecutorOp::Write));
        let request = RPCRequest::new(
            "executor.device.create",
            json!({"expires_at_ms": now_ms() - 1}),
        );
        assert!(expiry(&request).is_err());
        assert!(record_key("../../nodes/config").is_err());
    }

    #[tokio::test]
    async fn stale_close_cannot_disconnect_replacement_and_revoke_cancels_streams() {
        let hub = AgentDaemonHub::default();
        let conn = connection(2);
        let cancellation = conn.cancel.subscribe();
        hub.connections.lock().await.insert("device".into(), conn);
        hub.disconnect("device", 1).await;
        assert!(hub.connections.lock().await["device"].live());
        assert!(!*cancellation.borrow());
        hub.revoke("device").await;
        assert!(*cancellation.borrow());
        assert!(!hub.connections.lock().await.contains_key("device"));
    }

    async fn http_server() -> (
        ControlPanelServer,
        std::net::SocketAddr,
        tokio::task::JoinHandle<()>,
    ) {
        let server = ControlPanelServer::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let service = Arc::new(server.clone());
        let task = tokio::spawn(async move {
            while let Ok((stream, peer)) = listener.accept().await {
                let service = service.clone();
                tokio::spawn(async move {
                    let _ = buckyos_http_server::hyper_serve_http(
                        Box::new(stream),
                        service,
                        buckyos_http_server::StreamInfo::new(peer.to_string()),
                    )
                    .await;
                });
            }
        });
        (server, address, task)
    }

    async fn prepare_stream(server: &ControlPanelServer, id: &str, expires: u64) {
        let mut connections = server.agent_daemon.connections.lock().await;
        let conn = connections
            .entry("device".into())
            .or_insert_with(|| connection(7));
        server.agent_daemon.streams.lock().await.insert(
            id.into(),
            PendingStream {
                _slot: conn.stream_slots.clone().try_acquire_owned().unwrap(),
                executor_id: "device".into(),
                generation: 7,
                expires_at_ms: expires,
                session_expires_at_ms: now_ms() + 60_000,
                agent_sha256: digest("agent-ticket"),
                daemon_sha256: digest("daemon-ticket"),
                agent: None,
                daemon: None,
                cancel: conn.cancel.subscribe(),
            },
        );
    }

    #[tokio::test]
    async fn actual_http_upgrade_binary_bridge_browser_auth_and_revoke() {
        let (server, address, listener) = http_server().await;
        prepare_stream(&server, "stream", now_ms() + 10_000).await;
        let mut request = format!("ws://{address}{AGENT_DAEMON_DATA_PATH}/stream")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Authorization", "Bearer daemon-ticket".parse().unwrap());
        let (mut daemon, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let (mut agent, _) = tokio_tungstenite::connect_async(format!(
            "ws://{address}{AGENT_DAEMON_ROUTE_PATH}/stream"
        ))
        .await
        .unwrap();
        agent
            .send(Message::Text(
                json!({"ticket":"agent-ticket"}).to_string().into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            agent.next().await.unwrap().unwrap().to_text().unwrap(),
            "{\"ready\":true}"
        );
        let binary = Bytes::from_static(b"\0command-stream\xff");
        agent.send(Message::Binary(binary.clone())).await.unwrap();
        assert_eq!(daemon.next().await.unwrap().unwrap().into_data(), binary);
        daemon
            .send(Message::Binary(Bytes::from_static(b"result")))
            .await
            .unwrap();
        assert_eq!(
            agent.next().await.unwrap().unwrap().into_data(),
            Bytes::from_static(b"result")
        );
        assert!(!server
            .agent_daemon
            .streams
            .lock()
            .await
            .contains_key("stream"));
        let mut replay = format!("ws://{address}{AGENT_DAEMON_ROUTE_PATH}/stream")
            .into_client_request()
            .unwrap();
        replay
            .headers_mut()
            .insert("Authorization", "Bearer agent-ticket".parse().unwrap());
        let (mut replay, _) = tokio_tungstenite::connect_async(replay).await.unwrap();
        let rejected = tokio::time::timeout(Duration::from_secs(2), replay.next())
            .await
            .unwrap();
        assert!(matches!(
            rejected,
            Some(Ok(Message::Close(_))) | None | Some(Err(_))
        ));

        server.agent_daemon.revoke("device").await;
        let end = tokio::time::timeout(Duration::from_secs(2), agent.next())
            .await
            .unwrap();
        assert!(matches!(
            end,
            Some(Ok(Message::Close(_))) | None | Some(Err(_))
        ));
        listener.abort();
    }

    #[tokio::test]
    async fn wrong_ticket_stale_generation_and_expired_ticket_are_rejected() {
        let (server, address, listener) = http_server().await;
        let rejected =
            tokio_tungstenite::connect_async(format!("ws://{address}{AGENT_DAEMON_MANAGE_PATH}"))
                .await;
        assert!(
            matches!(rejected, Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status() == http::StatusCode::FORBIDDEN)
        );
        for (id, ticket, expiry, generation) in [
            ("wrong", "wrong-ticket", now_ms() + 10_000, 7),
            ("expired", "agent-ticket", now_ms() - 1, 7),
            ("stale", "agent-ticket", now_ms() + 10_000, 6),
        ] {
            prepare_stream(&server, id, expiry).await;
            server
                .agent_daemon
                .streams
                .lock()
                .await
                .get_mut(id)
                .unwrap()
                .generation = generation;
            let (mut socket, _) = tokio_tungstenite::connect_async(format!(
                "ws://{address}{AGENT_DAEMON_ROUTE_PATH}/{id}"
            ))
            .await
            .unwrap();
            socket
                .send(Message::Text(json!({"ticket":ticket}).to_string().into()))
                .await
                .unwrap();
            let end = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap();
            assert!(matches!(
                end,
                Some(Ok(Message::Close(_))) | None | Some(Err(_))
            ));
        }
        listener.abort();
    }
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires an activated local Zone; runs a separate control-panel listener until Ctrl-C"]
    async fn manual_zone_access_fixture() {
        let identity = load_local_node_identity_config(
            &buckyos_kit::get_buckyos_system_etc_dir().join("node_identity.json"),
        )
        .unwrap();
        let (_, device) = load_local_device_config(&identity, true).unwrap();
        let private_key = load_local_device_private_key(&identity.device_did).unwrap();
        let (_, mut assertion) = generate_service_login_assertion(
            &device.name,
            CONTROL_PANEL_SERVICE_NAME,
            &device.name,
            &private_key,
        )
        .unwrap();
        assertion.aud = Some(SYSTEM_CONFIG_BOOTSTRAP_AUDIENCE.to_string());
        assertion.token = None;
        let bootstrap = assertion
            .generate_jwt(Some(device.name), &private_key)
            .unwrap();
        unsafe {
            std::env::set_var(
                get_service_session_token_env_key(CONTROL_PANEL_SERVICE_NAME),
                bootstrap,
            );
        }
        let mut runtime = init_buckyos_api_runtime(
            CONTROL_PANEL_SERVICE_NAME,
            None,
            BuckyOSRuntimeType::KernelService,
        )
        .await
        .unwrap();
        runtime.login().await.unwrap();
        runtime.renew_token_from_verify_hub().await.unwrap();
        set_buckyos_api_runtime(runtime).unwrap();
        let server = Arc::new(ControlPanelServer::new());
        let address = std::env::var("BUCKYOS_EXECUTOR_FIXTURE_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:14020".into());
        let listener = tokio::net::TcpListener::bind(&address).await.unwrap();
        eprintln!(
            "Agent daemon control-panel fixture listening at {}",
            listener.local_addr().unwrap()
        );
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                accepted = listener.accept() => {
                    let (stream, peer) = accepted.unwrap();
                    let server = server.clone();
                    tokio::spawn(async move {
                        let _ = buckyos_http_server::hyper_serve_http(Box::new(stream), server, buckyos_http_server::StreamInfo::new(peer.to_string())).await;
                    });
                }
            }
        }
    }
}
