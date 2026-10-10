mod contact_mgr;
mod cyfs_dispatch;
mod group_http;
mod group_publication;
mod group_service;
mod group_store;
mod group_sync;
mod group_types;
mod message_hub;
mod msg_box_db;
mod msg_center;
mod msg_tunnel;
mod object_access;
mod owner_session;
mod owner_session_db;
#[cfg(test)]
mod test_group_service;
#[cfg(test)]
mod test_msg_center;
mod tg_tunnel;
mod zone_agent;

use ::kRPC::*;
use anyhow::{Context, Result};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AccountBinding,
    BuckyOSRuntimeType, DeliveryRecordWithObject, DeliveryReportResult, DeliveryState,
    MsgCenterClient, MsgCenterServerHandler, SystemConfigClient, UserContactSettings,
    UserPrivateProfile, UserSettings, UserState, MSG_CENTER_SERVICE_NAME, MSG_CENTER_SERVICE_PORT,
};
use buckyos_http_server::Runner;
use buckyos_http_server::{
    serve_http_by_rpc_handler, HttpServer, ServerError, ServerResult, StreamInfo,
};
use buckyos_kit::{get_buckyos_service_data_dir, init_logging};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use name_lib::DID;
use ndn_lib::{MsgContent, MsgContentFormat, MsgObject};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::contact_mgr::ZoneUserContactSeed;
use crate::message_hub::MessageHubExecutor;
use crate::msg_center::MessageCenter;
use crate::msg_tunnel::{DeliveryExecutor, DeliveryExecutorMgr, ExecutorInstanceState};
use crate::tg_tunnel::{GrammersTgGatewayConfig, TgBotBinding, TgTunnel, TgTunnelConfig};
use crate::zone_agent::{load_zone_agents, ZoneAgent, TELEGRAM_PLATFORM};

const MSG_CENTER_HTTP_PATH: &str = "/kapi/msg-center";
const MSG_CENTER_DEFAULT_TG_TUNNEL_DID: &str = "did:bns:msg-center-default-tunnel";
const PROFILE_SYSTEM_CONTACT_KEY: &str = "system_contact";
const TG_BINDING_BOT_TOKEN_KEY: &str = "bot_token";
const METHOD_RELOAD_SETTINGS: &str = "reload_settings";
const METHOD_SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
const METHOD_REALOAD_SETTINGS: &str = "reaload_settings";
const METHOD_SERVICE_REALOAD_SETTINGS: &str = "service.reaload_settings";
const DELIVERY_PUMP_IDLE_SLEEP_MS: u64 = 300;
const ZONE_USER_SYNC_INTERVAL_SECS: u64 = 30;
const DELIVERY_PUMP_ERROR_SLEEP_MS: u64 = 1_000;
const DELIVERY_PUMP_RETRY_AFTER_MS: u64 = 2_000;

#[derive(Debug, Clone, Default)]
struct MsgCenterSettings {
    telegram_tunnel: TelegramTunnelSettings,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawMsgCenterSettings {
    #[serde(default)]
    telegram_tunnel: Option<TelegramTunnelSettings>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TelegramGatewayMode {
    DryRun,
    Grammers,
    BotApi,
}

impl Default for TelegramGatewayMode {
    fn default() -> Self {
        Self::DryRun
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
struct TelegramGatewaySettings {
    #[serde(default)]
    mode: TelegramGatewayMode,
    #[serde(default)]
    api_id: Option<i32>,
    #[serde(default)]
    api_hash: Option<String>,
    #[serde(default)]
    session_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct TelegramBindingSettings {
    owner_did: String,
    bot_token: String,
    #[serde(default)]
    bot_account_id: Option<String>,
    #[serde(default)]
    extra: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct TelegramTunnelSettings {
    #[serde(default = "default_tg_tunnel_enabled")]
    enabled: bool,
    /// The tunnel instance's own DID: DELIVERY_QUEUE owner / delivery executor.
    #[serde(default = "default_tg_transport_did")]
    transport_did: String,
    /// Stable tunnel instance id embedded in shadow endpoint DIDs
    /// (e.g. `tg-main-tunnel`). Locally unique, never reused.
    #[serde(default = "default_tg_tunnel_instance_id")]
    tunnel_instance_id: String,
    #[serde(default = "default_true")]
    supports_ingress: bool,
    #[serde(default = "default_true")]
    supports_egress: bool,
    #[serde(default)]
    gateway: TelegramGatewaySettings,
    #[serde(default)]
    bindings: Vec<TelegramBindingSettings>,
}

impl Default for TelegramTunnelSettings {
    fn default() -> Self {
        Self {
            enabled: default_tg_tunnel_enabled(),
            transport_did: default_tg_transport_did(),
            tunnel_instance_id: default_tg_tunnel_instance_id(),
            supports_ingress: default_true(),
            supports_egress: default_true(),
            gateway: TelegramGatewaySettings::default(),
            bindings: vec![],
        }
    }
}

struct MsgCenterHttpServer {
    rpc_handler: MsgCenterServerHandler<MessageCenter>,
    zone_sync: Arc<ZoneSync>,
}

impl MsgCenterHttpServer {
    fn new(center: MessageCenter, zone_sync: Arc<ZoneSync>) -> Self {
        Self {
            rpc_handler: MsgCenterServerHandler::new(center),
            zone_sync,
        }
    }

    async fn handle_reload_settings(&self) -> std::result::Result<serde_json::Value, RPCErrors> {
        let settings = load_msg_center_settings().await.map_err(|err| {
            RPCErrors::ReasonError(format!("load msg-center settings failed: {}", err))
        })?;
        let dispatch_settings = cyfs_dispatch::CyfsDispatchSettings::parse(&settings)
            .map_err(|e| RPCErrors::ParseRequestError(e.to_string()))?;
        *self.rpc_handler.0.cyfs_dispatch.write().unwrap() = dispatch_settings;
        self.zone_sync.sync(&settings, true).await.map_err(|err| {
            RPCErrors::ReasonError(format!("reload msg-center settings failed: {}", err))
        })
    }
}

#[async_trait::async_trait]
impl RPCHandler for MsgCenterHttpServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        if req.method == METHOD_RELOAD_SETTINGS
            || req.method == METHOD_SERVICE_RELOAD_SETTINGS
            || req.method == METHOD_REALOAD_SETTINGS
            || req.method == METHOD_SERVICE_REALOAD_SETTINGS
        {
            let result = self.handle_reload_settings().await?;
            return Ok(RPCResponse {
                result: RPCResult::Success(result),
                seq: req.seq,
                trace_id: req.trace_id,
            });
        }
        if req.token.is_none() {
            return Err(RPCErrors::NoPermission("authentication-required".into()));
        }
        if req.method.starts_with("group.") {
            let ctx = RPCContext::from_request(&req, ip_from);
            let value = self
                .rpc_handler
                .0
                .group_rpc(&req.method, req.params.clone(), ctx)
                .await?;
            return Ok(RPCResponse {
                result: RPCResult::Success(value),
                seq: req.seq,
                trace_id: req.trace_id,
            });
        }
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait::async_trait]
impl HttpServer for MsgCenterHttpServer {
    async fn serve_request(
        &self,
        mut req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if let Some(response) = group_http::serve(&self.rpc_handler.0, &mut req).await {
            return Ok(response);
        }
        if let Some(response) = object_access::serve(&self.rpc_handler.0, &req).await {
            return Ok(response);
        }
        if *req.method() == Method::POST
            && (req.uri().path() == MSG_CENTER_HTTP_PATH
                || req
                    .uri()
                    .path()
                    .starts_with(&format!("{MSG_CENTER_HTTP_PATH}/")))
        {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Ok(cyfs_dispatch::serve(&self.rpc_handler.0, req).await)
    }

    fn id(&self) -> String {
        MSG_CENTER_SERVICE_NAME.to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

fn default_true() -> bool {
    true
}

fn default_tg_tunnel_enabled() -> bool {
    true
}

fn default_tg_transport_did() -> String {
    MSG_CENTER_DEFAULT_TG_TUNNEL_DID.to_string()
}

fn default_tg_tunnel_instance_id() -> String {
    "tg-main-tunnel".to_string()
}

fn default_tg_session_dir() -> PathBuf {
    get_buckyos_service_data_dir(MSG_CENTER_SERVICE_NAME).join("tg_sessions")
}

fn parse_msg_center_settings(settings: &Value) -> Result<MsgCenterSettings> {
    if settings.is_null() {
        return Ok(MsgCenterSettings::default());
    }
    let raw = serde_json::from_value::<RawMsgCenterSettings>(settings.clone())
        .map_err(|err| anyhow::anyhow!("parse msg-center settings failed: {}", err))?;

    let telegram_tunnel = raw.telegram_tunnel.unwrap_or_default();
    Ok(MsgCenterSettings { telegram_tunnel })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// One round of the delivery pump: for every running egress-capable executor,
/// take the next due DeliveryRecord from its DELIVERY_QUEUE (WAIT → SENDING),
/// execute it, and report the result back (SENT / WAIT-with-backoff / DEAD).
/// The periodic poll doubles as the SENDING lease sweep — real-time
/// notifications are only an accelerator, never the source of truth.
async fn pump_delivery_queue_once(
    msg_center: &MsgCenterClient,
    executor_mgr: &DeliveryExecutorMgr,
) -> Result<bool> {
    let instances = executor_mgr
        .list_instances()
        .map_err(|err| anyhow::anyhow!("list delivery executors failed: {}", err))?;
    let mut worked = false;

    for instance in instances {
        if instance.state != ExecutorInstanceState::Running || !instance.supports_egress {
            continue;
        }

        let transport_did = instance.transport_did.clone();
        let maybe_record = msg_center
            .get_next_delivery(transport_did.clone(), Some(true), Some(true))
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "take next delivery failed for {}: {}",
                    transport_did.to_string(),
                    err
                )
            })?;
        let Some(record) = maybe_record else {
            continue;
        };
        let delivery_id = record.record.delivery_id.clone();
        let failure_notice = build_delivery_failure_notice(&record);
        worked = true;

        let report = match executor_mgr.execute_via(&transport_did, record).await {
            Ok(report) => report,
            Err(err) => {
                // Executor-level failure (not running, transport error). Feed
                // it into the same state machine as a retryable failure.
                let reason = err.to_string();
                warn!(
                    "msg-center delivery execute failed: executor={} delivery_id={} err={}",
                    transport_did.to_string(),
                    delivery_id,
                    reason
                );
                DeliveryReportResult {
                    ok: false,
                    error_message: Some(reason),
                    retry_after_ms: Some(DELIVERY_PUMP_RETRY_AFTER_MS),
                    retryable: Some(true),
                    ..Default::default()
                }
            }
        };

        let report_ok = report.ok;
        match msg_center
            .report_delivery(delivery_id.clone(), report)
            .await
        {
            Err(err) => {
                // The SENDING lease sweep will reclaim this row if the report is
                // lost for good.
                warn!(
                    "msg-center report_delivery failed: executor={} delivery_id={} err={}",
                    transport_did.to_string(),
                    delivery_id,
                    err
                );
            }
            Ok(_) if report_ok => {
                info!(
                    "msg-center delivery done: executor={} delivery_id={}",
                    transport_did.to_string(),
                    delivery_id
                );
            }
            Ok(record) => {
                warn!(
                    "msg-center delivery reported failure: executor={} delivery_id={}",
                    transport_did.to_string(),
                    delivery_id
                );
                if record.state == DeliveryState::Dead {
                    if let Some(notice) = failure_notice {
                        let idempotency_key = Some(format!("delivery-failure:{delivery_id}"));
                        match msg_center.post_send(notice, idempotency_key).await {
                            Ok(result) if result.ok => {}
                            Ok(result) => warn!(
                                "msg-center delivery failure notice rejected: delivery_id={} reason={:?}",
                                delivery_id, result.reason
                            ),
                            Err(err) => warn!(
                                "msg-center delivery failure notice failed: delivery_id={} err={}",
                                delivery_id, err
                            ),
                        }
                    }
                }
            }
        }
    }

    Ok(worked)
}

fn build_delivery_failure_notice(record: &DeliveryRecordWithObject) -> Option<MsgObject> {
    let mut msg = record.msg.clone()?;
    if msg
        .meta
        .get("delivery_failure_fallback")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let text = msg
        .meta
        .remove("delivery_failure_notice")?
        .as_str()?
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }
    msg.to = vec![record.record.envelope.target_did.clone()];
    msg.created_at_ms = now_ms();
    msg.content = MsgContent {
        format: Some(MsgContentFormat::TextPlain),
        content: text,
        ..Default::default()
    };
    msg.meta
        .insert("delivery_failure_fallback".to_string(), Value::Bool(true));
    Some(msg)
}

fn start_delivery_pump(center: MessageCenter, executor_mgr: Arc<DeliveryExecutorMgr>) {
    tokio::spawn(async move {
        let msg_center = MsgCenterClient::new_in_process(Box::new(center));
        info!("msg-center delivery pump started");

        loop {
            match pump_delivery_queue_once(&msg_center, executor_mgr.as_ref()).await {
                Ok(true) => {}
                Ok(false) => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        DELIVERY_PUMP_IDLE_SLEEP_MS,
                    ))
                    .await;
                }
                Err(err) => {
                    warn!("msg-center delivery pump error: {}", err);
                    tokio::time::sleep(std::time::Duration::from_millis(
                        DELIVERY_PUMP_ERROR_SLEEP_MS,
                    ))
                    .await;
                }
            }
        }
    });
}

fn profile_system_contact(profile: &UserPrivateProfile) -> Option<UserContactSettings> {
    profile
        .private_extra
        .get(PROFILE_SYSTEM_CONTACT_KEY)
        .and_then(|value| serde_json::from_value::<UserContactSettings>(value.clone()).ok())
}

fn build_zone_user_seed(
    username: &str,
    settings: UserSettings,
    profile: Option<UserPrivateProfile>,
) -> Option<ZoneUserContactSeed> {
    if !matches!(settings.state, UserState::Active) {
        return None;
    }

    let mut contact_did = profile
        .as_ref()
        .map(|profile| profile.did.clone())
        .unwrap_or_else(|| DID::new("bns", username));
    let mut note = None;
    let mut groups = vec!["zone_user".to_string()];
    let mut tags = vec!["zone_user".to_string()];
    let mut bindings = Vec::new();

    if let Some(contact_cfg) = profile.as_ref().and_then(profile_system_contact) {
        if let Some(raw_did) = contact_cfg.did.as_ref() {
            let did = raw_did.trim();
            if !did.is_empty() {
                match DID::from_str(did) {
                    Ok(parsed) => {
                        contact_did = parsed;
                    }
                    Err(error) => {
                        warn!(
                            "invalid user contact.did, fallback to {}: did={}, error={}",
                            contact_did.to_string(),
                            did,
                            error
                        );
                    }
                }
            }
        }

        note = contact_cfg.note;
        groups.extend(contact_cfg.groups);
        tags.extend(contact_cfg.tags);

        for binding in contact_cfg.bindings {
            let platform = binding.platform.trim().to_string();
            let account_id = binding.account_id.trim();
            let account_id = if platform.eq_ignore_ascii_case(TELEGRAM_PLATFORM) {
                account_id
                    .strip_prefix("user:")
                    .unwrap_or(account_id)
                    .trim()
            } else {
                account_id
            }
            .to_string();
            if platform.is_empty() || account_id.is_empty() {
                warn!(
                    "skip invalid user tunnel binding for {}: platform='{}', account_id='{}'",
                    username, binding.platform, binding.account_id
                );
                continue;
            }

            let display_id = binding
                .display_id
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| account_id.clone());
            let tunnel_instance_id = binding
                .tunnel_instance_id
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("{}-default-tunnel", platform.to_ascii_lowercase()));

            bindings.push(AccountBinding {
                platform,
                account_id,
                display_id,
                tunnel_instance_id,
                account_type: String::new(),
                endpoint_did: None,
                last_active_at: now_ms(),
                meta: binding.meta,
            });
        }
    }

    let name = profile
        .as_ref()
        .and_then(|profile| profile.display_name.as_ref().or(profile.name.as_ref()))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    Some(ZoneUserContactSeed {
        did: contact_did,
        name: if name.is_empty() {
            username.to_string()
        } else {
            name
        },
        note,
        bindings,
        groups,
        tags,
    })
}

async fn load_zone_user_profile(
    system_config_client: &SystemConfigClient,
    username: &str,
) -> Option<UserPrivateProfile> {
    let profile_path = format!("users/{}/profile", username);
    match system_config_client.get(&profile_path).await {
        Ok(profile_val) => match serde_json::from_str::<UserPrivateProfile>(&profile_val.value) {
            Ok(profile) => Some(profile),
            Err(error) => {
                warn!(
                    "parse user profile failed while syncing zone users: username={}, error={}",
                    username, error
                );
                None
            }
        },
        Err(error) => {
            warn!(
                "load user profile failed while syncing zone users: username={}, error={}",
                username, error
            );
            None
        }
    }
}

async fn load_zone_user_contact_seeds() -> Result<Vec<ZoneUserContactSeed>> {
    let runtime = get_buckyos_api_runtime()?;
    let control_panel_client = runtime
        .get_control_panel_client()
        .await
        .map_err(|error| anyhow::anyhow!("get control panel client failed: {}", error))?;

    let users = control_panel_client
        .get_user_list()
        .await
        .map_err(|error| anyhow::anyhow!("list users failed: {}", error))?;
    let mut contacts = Vec::new();
    let system_config_client = runtime.get_system_config_client().await?;

    for username in users {
        let settings = match control_panel_client
            .get_user_settings_by_username(username.as_str())
            .await
        {
            Ok(settings) => settings,
            Err(error) => {
                warn!(
                    "load user settings failed while syncing zone users: username={}, error={}",
                    username, error
                );
                continue;
            }
        };
        let profile = load_zone_user_profile(&system_config_client, username.as_str()).await;

        if let Some(seed) = build_zone_user_seed(username.as_str(), settings, profile) {
            contacts.push(seed);
        }
    }

    Ok(contacts)
}

async fn load_msg_center_settings() -> Result<Value> {
    Ok(get_buckyos_api_runtime()?.get_my_settings().await?)
}

async fn load_system_zone_agents() -> Result<Vec<ZoneAgent>> {
    let client = get_buckyos_api_runtime()?
        .get_system_config_client()
        .await?;
    load_zone_agents(client.as_ref()).await
}

/// The Telegram bindings to run: msg-center settings plus each Agent's own
/// bot from `settings.msg_tunnels` (`owner_did` = Agent DID). An Agent's own
/// bot replaces a settings binding of the same Agent.
fn with_agent_telegram_bindings(
    mut tunnel: TelegramTunnelSettings,
    agents: &[ZoneAgent],
) -> TelegramTunnelSettings {
    let agent_bindings: Vec<TelegramBindingSettings> = agents
        .iter()
        .filter_map(|agent| {
            let (bot_token, bot_account_id) = agent.telegram_tunnel()?;
            Some(TelegramBindingSettings {
                owner_did: agent.did().to_string(),
                bot_token: bot_token.to_string(),
                bot_account_id: bot_account_id.map(str::to_string),
                extra: HashMap::new(),
            })
        })
        .collect();
    tunnel.bindings.retain(|binding| {
        !agent_bindings
            .iter()
            .any(|agent| agent.owner_did == binding.owner_did.trim())
    });
    tunnel.bindings.extend(agent_bindings);
    tunnel
        .bindings
        .sort_by(|left, right| left.owner_did.cmp(&right.owner_did));
    tunnel
}

/// What msg-center follows from system-config: the Telegram bot bindings and
/// the zone user / Agent contacts. Users, Agents and their settings change
/// without notifying msg-center, so they are re-read every
/// `ZONE_USER_SYNC_INTERVAL_SECS`; `reload_settings` runs a round at once.
/// Rounds run one at a time and only rebuild what changed.
struct ZoneSync {
    center: MessageCenter,
    executor_mgr: Arc<DeliveryExecutorMgr>,
    applied: tokio::sync::Mutex<AppliedZoneSync>,
}

#[derive(Default)]
struct AppliedZoneSync {
    tunnel: Option<(TelegramTunnelSettings, Value)>,
    contacts: Option<String>,
}

impl ZoneSync {
    fn new(center: MessageCenter, executor_mgr: Arc<DeliveryExecutorMgr>) -> Self {
        Self {
            center,
            executor_mgr,
            applied: tokio::sync::Mutex::new(AppliedZoneSync::default()),
        }
    }

    async fn sync(&self, raw_settings: &Value, force: bool) -> Result<Value> {
        let agents = load_system_zone_agents().await?;
        self.apply(raw_settings, &agents, force).await
    }

    /// `force` rebuilds the tunnel executors and rewrites contacts even when
    /// nothing changed. Tunnel configuration errors are returned; contact
    /// sync failures are retried by the next round.
    async fn apply(
        &self,
        raw_settings: &Value,
        agents: &[ZoneAgent],
        force: bool,
    ) -> Result<Value> {
        let settings = parse_msg_center_settings(raw_settings)?;
        let tunnel = with_agent_telegram_bindings(settings.telegram_tunnel, agents);
        let mut applied = self.applied.lock().await;
        let unchanged = applied
            .tunnel
            .as_ref()
            .is_some_and(|(current, _)| *current == tunnel);
        if force || !unchanged {
            match apply_tg_tunnel_settings(&self.center, self.executor_mgr.as_ref(), &tunnel).await
            {
                Ok(result) => applied.tunnel = Some((tunnel, result)),
                Err(error) => {
                    applied.tunnel = Some((
                        tunnel,
                        serde_json::json!({"ok": false, "error": error.to_string()}),
                    ));
                    return Err(error);
                }
            }
        }
        let result = applied
            .tunnel
            .as_ref()
            .map(|(_, result)| result.clone())
            .unwrap_or_default();
        if let Err(error) = self.sync_contacts(&mut applied, agents, force).await {
            warn!("zone contact sync failed: {}", error);
        }
        Ok(result)
    }

    async fn sync_contacts(
        &self,
        applied: &mut AppliedZoneSync,
        agents: &[ZoneAgent],
        force: bool,
    ) -> Result<()> {
        let users = load_zone_user_contact_seeds().await?;
        let signature = zone_contact_seed_signature(&users, agents);
        if !force && applied.contacts.as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        sync_zone_agent_contacts(&self.center, &users, agents).await?;
        sync_zone_user_contacts(&self.center, users).await?;
        applied.contacts = Some(signature);
        Ok(())
    }
}

/// What a contact sync would write, without the per-load binding timestamps.
fn zone_contact_seed_signature(seeds: &[ZoneUserContactSeed], agents: &[ZoneAgent]) -> String {
    let mut parts: Vec<String> = seeds
        .iter()
        .map(|seed| {
            let mut bindings: Vec<String> = seed
                .bindings
                .iter()
                .map(|binding| {
                    let mut meta: Vec<_> = binding.meta.iter().collect();
                    meta.sort();
                    format!(
                        "{}|{}|{}|{}|{:?}",
                        binding.platform,
                        binding.account_id,
                        binding.display_id,
                        binding.tunnel_instance_id,
                        meta
                    )
                })
                .collect();
            bindings.sort();
            format!(
                "{}|{}|{:?}|{:?}|{:?}|{:?}",
                seed.did.to_string(),
                seed.name,
                seed.note,
                seed.groups,
                seed.tags,
                bindings
            )
        })
        .collect();
    parts.extend(agents.iter().map(|agent| {
        format!(
            "agent|{}|{}|{}",
            agent.did().to_string(),
            agent.owner().to_string(),
            agent.display_name()
        )
    }));
    parts.sort();
    parts.join("\n")
}

fn start_zone_sync(sync: Arc<ZoneSync>) {
    tokio::spawn(async move {
        let period = std::time::Duration::from_secs(ZONE_USER_SYNC_INTERVAL_SECS);
        loop {
            tokio::time::sleep(period).await;
            let settings = match load_msg_center_settings().await {
                Ok(settings) => settings,
                Err(error) => {
                    warn!("load msg-center settings for zone sync failed: {}", error);
                    continue;
                }
            };
            if let Err(error) = sync.sync(&settings, false).await {
                warn!("periodic zone sync failed: {}", error);
            }
        }
    });
}

async fn sync_zone_agent_contacts(
    center: &MessageCenter,
    users: &[ZoneUserContactSeed],
    agents: &[ZoneAgent],
) -> Result<()> {
    for agent in agents {
        let owner_contact = users
            .iter()
            .find(|user| &user.did == agent.owner())
            .cloned()
            .unwrap_or_else(|| ZoneUserContactSeed {
                did: agent.owner().clone(),
                name: agent.owner().id.clone(),
                note: None,
                bindings: vec![],
                groups: vec![],
                tags: vec![],
            });
        center.register_local_recipients([agent.did().clone()]);
        center
            .upsert_zone_user_contacts(vec![owner_contact], Some(agent.did().clone()))
            .await?;
        center
            .contact_mgr
            .upsert_zone_agent_contacts(
                vec![ZoneUserContactSeed {
                    did: agent.did().clone(),
                    name: agent.display_name(),
                    note: None,
                    bindings: vec![],
                    groups: vec![],
                    tags: vec!["agent".to_string()],
                }],
                agent.owner().clone(),
            )
            .await?;
    }
    Ok(())
}

async fn sync_zone_user_contacts(
    center: &MessageCenter,
    contacts: Vec<ZoneUserContactSeed>,
) -> Result<()> {
    let mut owner_scopes: Vec<Option<DID>> = vec![None];
    for owner in contacts.iter().map(|contact| contact.did.clone()) {
        if !owner_scopes.contains(&Some(owner.clone())) {
            owner_scopes.push(Some(owner));
        }
    }

    for owner in owner_scopes {
        let updated = center
            .upsert_zone_user_contacts(contacts.clone(), owner.clone())
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "zone user sync failed for owner_scope={}: {}",
                    owner
                        .as_ref()
                        .map(|did| did.to_string())
                        .unwrap_or_else(|| "__system__".to_string()),
                    error
                )
            })?;

        info!(
            "zone user sync applied: owner_scope={}, contacts={}",
            owner
                .as_ref()
                .map(|did| did.to_string())
                .unwrap_or_else(|| "__system__".to_string()),
            updated
        );
    }

    Ok(())
}

fn resolve_tg_transport_did(settings: &TelegramTunnelSettings) -> Result<DID> {
    DID::from_str(settings.transport_did.trim()).map_err(|e| {
        anyhow::anyhow!(
            "invalid telegram tunnel transport_did {}, err={}",
            settings.transport_did,
            e
        )
    })
}

fn resolve_bot_account_id(owner_did: &DID, input: Option<&str>) -> String {
    if let Some(raw) = input {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    format!("telegram-bot@{}", owner_did.to_string())
}

fn build_tg_tunnel(cfg: TgTunnelConfig, settings: &TelegramTunnelSettings) -> Result<TgTunnel> {
    match settings.gateway.mode {
        TelegramGatewayMode::DryRun => {
            info!("telegram tunnel initialized with dry-run gateway");
            Ok(TgTunnel::new(cfg))
        }
        TelegramGatewayMode::Grammers => {
            let api_id = settings.gateway.api_id.ok_or_else(|| {
                anyhow::anyhow!("telegram gateway mode=grammers requires gateway.api_id")
            })?;
            if api_id <= 0 {
                return Err(anyhow::anyhow!(
                    "telegram gateway api_id must be > 0, got {}",
                    api_id
                ));
            }
            let api_hash = settings
                .gateway
                .api_hash
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("telegram gateway mode=grammers requires gateway.api_hash")
                })?;
            let session_dir = settings
                .gateway
                .session_dir
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(default_tg_session_dir);

            let gateway_cfg = GrammersTgGatewayConfig {
                api_id,
                api_hash,
                session_dir,
                transport_did: Some(cfg.transport_did.clone()),
                tunnel_instance_id: Some(cfg.tunnel_instance_id.clone()),
            };
            info!(
                "telegram tunnel initialized with grammers gateway (session_dir={})",
                gateway_cfg.session_dir.display()
            );
            Ok(TgTunnel::with_grammers_gateway(cfg, gateway_cfg))
        }
        TelegramGatewayMode::BotApi => {
            info!("telegram tunnel initialized with bot-api gateway");
            Ok(TgTunnel::with_bot_api_gateway(cfg))
        }
    }
}

fn bind_tg_tunnel_bots(tg_tunnel: &TgTunnel, settings: &TelegramTunnelSettings) -> Result<()> {
    for binding in settings.bindings.iter() {
        let owner_did = DID::from_str(binding.owner_did.trim()).map_err(|err| {
            anyhow::anyhow!(
                "invalid telegram binding owner_did {}, err={}",
                binding.owner_did,
                err
            )
        })?;
        let bot_token = binding.bot_token.trim();
        if bot_token.is_empty() {
            return Err(anyhow::anyhow!(
                "telegram binding bot_token is empty for owner {}",
                owner_did.to_string()
            ));
        }

        let mut extra = binding.extra.clone();
        extra.insert(TG_BINDING_BOT_TOKEN_KEY.to_string(), bot_token.to_string());
        tg_tunnel
            .bind_bot(TgBotBinding {
                owner_did: owner_did.clone(),
                bot_account_id: resolve_bot_account_id(
                    &owner_did,
                    binding.bot_account_id.as_deref(),
                ),
                bot_token_env_key: None,
                extra,
            })
            .with_context(|| {
                format!(
                    "bind telegram bot for owner {} failed",
                    owner_did.to_string()
                )
            })?;
    }
    Ok(())
}

/// Stop and unregister every registered *tunnel* executor (the message hub is
/// kept — it is zone infrastructure, not settings-driven), and drop the
/// matching tunnel routes so a reload can re-register them.
async fn clear_tunnel_instances(
    center: &MessageCenter,
    executor_mgr: &DeliveryExecutorMgr,
) -> Result<()> {
    let hub_did = center.message_hub_did();
    let instances = executor_mgr
        .list_instances()
        .map_err(|err| anyhow::anyhow!("list delivery executors failed: {}", err))?;
    for instance in instances.iter() {
        if Some(&instance.transport_did) == hub_did.as_ref() {
            continue;
        }
        if let Err(err) = executor_mgr.stop_instance(&instance.transport_did).await {
            warn!(
                "stop executor {} failed during reload: {}",
                instance.transport_did.to_string(),
                err
            );
        }
    }

    let instances = executor_mgr
        .list_instances()
        .map_err(|err| anyhow::anyhow!("list delivery executors failed: {}", err))?;
    for instance in instances.iter() {
        if Some(&instance.transport_did) == hub_did.as_ref() {
            continue;
        }
        if let Err(err) = executor_mgr.unregister(&instance.transport_did) {
            warn!(
                "unregister executor {} failed during reload: {}",
                instance.transport_did.to_string(),
                err
            );
        }
    }
    let remaining = executor_mgr
        .list_instances()
        .map_err(|err| anyhow::anyhow!("list delivery executors failed: {}", err))?
        .into_iter()
        .filter(|instance| Some(&instance.transport_did) != hub_did.as_ref())
        .count();
    if remaining != 0 {
        return Err(anyhow::anyhow!(
            "clear tunnel instances incomplete, {} instance(s) still registered",
            remaining
        ));
    }
    center.clear_tunnel_registry();
    Ok(())
}

async fn apply_tg_tunnel_settings(
    center: &MessageCenter,
    executor_mgr: &DeliveryExecutorMgr,
    tunnel: &TelegramTunnelSettings,
) -> Result<serde_json::Value> {
    if !tunnel.enabled {
        info!("telegram tunnel is disabled by settings");
        clear_tunnel_instances(center, executor_mgr).await?;
        return Ok(serde_json::json!({
            "ok": true,
            "tunnel_enabled": false,
            "tunnel_started": false,
            "bindings": 0
        }));
    }

    let transport_did = resolve_tg_transport_did(tunnel)?;
    let tunnel_instance_id = tunnel.tunnel_instance_id.trim().to_string();
    let tunnel_instance_id = if tunnel_instance_id.is_empty() {
        default_tg_tunnel_instance_id()
    } else {
        tunnel_instance_id
    };
    let mut cfg = TgTunnelConfig::new(transport_did.clone());
    cfg.tunnel_instance_id = tunnel_instance_id.clone();
    cfg.supports_ingress = tunnel.supports_ingress;
    cfg.supports_egress = tunnel.supports_egress;

    // Rebuild the executor set + tunnel route registry from settings.
    clear_tunnel_instances(center, executor_mgr).await?;
    let tg_tunnel = Arc::new(build_tg_tunnel(cfg, tunnel)?);

    tg_tunnel
        .bind_msg_center_handler(Arc::new(center.clone()))
        .context("bind msg_center handler to telegram tunnel failed")?;
    bind_tg_tunnel_bots(tg_tunnel.as_ref(), tunnel)?;
    // Bot owners (agents) receive replies natively via the message hub.
    let binding_owner_dids = tunnel
        .bindings
        .iter()
        .filter_map(|binding| DID::from_str(binding.owner_did.trim()).ok());
    center.register_local_recipients(binding_owner_dids);
    info!(
        "telegram tunnel {} (instance '{}') loaded {} binding(s)",
        transport_did.to_string(),
        tunnel_instance_id,
        tunnel.bindings.len()
    );

    executor_mgr
        .register(tg_tunnel.clone())
        .map_err(|e| anyhow::anyhow!("register telegram tunnel failed: {}", e))?;
    // Route registration comes after the executor exists so post_send can
    // never plan onto a tunnel with no consumer. A duplicate instance id is a
    // hard error (never silently overwritten).
    center.register_tunnel(
        tunnel_instance_id.clone(),
        transport_did.clone(),
        "telegram".to_string(),
    )?;
    center.declare_edit_capability(&transport_did, tg_tunnel.edit_capability());

    let mut started = false;
    let mut start_error: Option<String> = None;
    match executor_mgr.start_instance(&transport_did).await {
        Ok(_) => {
            started = true;
            info!(
                "telegram tunnel {} started (ingress={}, egress={})",
                transport_did.to_string(),
                tg_tunnel.supports_ingress(),
                tg_tunnel.supports_egress()
            );
        }
        Err(err) => {
            warn!(
                "telegram tunnel {} start failed, continue without tg tunnel: {}",
                transport_did.to_string(),
                err
            );
            start_error = Some(err.to_string());
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "tunnel_enabled": true,
        "transport_did": transport_did.to_string(),
        "tunnel_instance_id": tunnel_instance_id,
        "tunnel_started": started,
        "bindings": tunnel.bindings.len(),
        "start_error": start_error
    }))
}

/// Derive the MessageHub transport DID from the zone DID
/// (`did:web:example.com` → `did:web:msg-hub.example.com`).
fn resolve_message_hub_did() -> DID {
    match get_buckyos_api_runtime() {
        Ok(runtime) if runtime.zone_id != DID::undefined() => DID::new(
            runtime.zone_id.method.as_str(),
            format!("msg-hub.{}", runtime.zone_id.id).as_str(),
        ),
        _ => DID::new("bns", "msg-hub"),
    }
}

/// Register the MessageHub executor: the native delivery path for shareable
/// DID targets. Must succeed — without it `post_send` cannot plan any
/// shareable-DID delivery.
async fn register_message_hub(
    center: &MessageCenter,
    executor_mgr: &DeliveryExecutorMgr,
) -> Result<DID> {
    let hub_did = resolve_message_hub_did();
    center.set_message_hub_did(hub_did.clone());
    let hub = Arc::new(MessageHubExecutor::new(hub_did.clone(), center.clone()));
    center.declare_edit_capability(&hub_did, hub.edit_capability());
    executor_mgr
        .register(hub)
        .map_err(|err| anyhow::anyhow!("register message hub executor failed: {}", err))?;
    executor_mgr
        .start_instance(&hub_did)
        .await
        .map_err(|err| anyhow::anyhow!("start message hub executor failed: {}", err))?;
    info!("message hub executor started: {}", hub_did.to_string());
    Ok(hub_did)
}

pub async fn start_msg_center_service() -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        MSG_CENTER_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    let login_result = runtime.login().await;
    if login_result.is_err() {
        error!(
            "msg-center service login to system failed! err:{:?}",
            login_result
        );
        return Err(anyhow::anyhow!(
            "msg-center service login to system failed! err:{:?}",
            login_result
        ));
    }
    runtime.set_main_service_port(MSG_CENTER_SERVICE_PORT).await;

    let settings = match runtime.get_my_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(
                "load msg-center settings failed, fallback to empty settings, err={}",
                err
            );
            serde_json::json!({})
        }
    };

    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register msg-center runtime failed: {}", err))?;

    let center = MessageCenter::open_from_service_spec()
        .await
        .map_err(|err| anyhow::anyhow!("create message center failed: {:?}", err))?;
    *center.cyfs_dispatch.write().unwrap() = cyfs_dispatch::CyfsDispatchSettings::parse(&settings)?;
    center.start_idempotency_sweep();
    center.start_group_sync();

    let executor_mgr = Arc::new(DeliveryExecutorMgr::new());
    register_message_hub(&center, executor_mgr.as_ref()).await?;
    // Tunnel assembly must fail startup on configuration errors — most
    // importantly a duplicate tunnel_instance_id must never be silently
    // overwritten (shadow endpoint DID stability depends on it).
    // Agent bots that cannot be scanned yet join on the next sync round.
    let agents = load_system_zone_agents().await.unwrap_or_else(|error| {
        warn!("zone agent scan failed during startup: {}", error);
        vec![]
    });
    let zone_sync = Arc::new(ZoneSync::new(center.clone(), executor_mgr.clone()));
    let tunnel_result = zone_sync
        .apply(&settings, &agents, true)
        .await
        .map_err(|err| anyhow::anyhow!("assemble tunnel registry failed: {}", err))?;
    info!("msg-center settings initialized: {}", tunnel_result);
    start_zone_sync(zone_sync.clone());
    start_delivery_pump(center.clone(), executor_mgr);
    let server = Arc::new(MsgCenterHttpServer::new(center, zone_sync));

    let runner = Runner::new(MSG_CENTER_SERVICE_PORT);
    if let Err(err) = runner.add_http_server(MSG_CENTER_HTTP_PATH.to_string(), server.clone()) {
        error!("failed to add msg-center http server: {:?}", err);
        return Err(anyhow::anyhow!(
            "failed to add msg-center http server: {:?}",
            err
        ));
    }
    runner
        .add_http_server("/".into(), server)
        .map_err(|e| anyhow::anyhow!("register CYFS adapter: {e}"))?;
    if let Err(err) = runner.run().await {
        error!("msg-center runner exited with error: {:?}", err);
        return Err(anyhow::anyhow!(
            "msg-center runner exited with error: {:?}",
            err
        ));
    }

    info!(
        "msg-center service started at port {}",
        MSG_CENTER_SERVICE_PORT
    );
    Ok(())
}

#[tokio::main]
async fn main() {
    init_logging("msg_center", true);
    if let Err(err) = start_msg_center_service().await {
        error!("msg-center service start failed: {:?}", err);
    }
}

#[cfg(test)]
mod zone_contact_tests {
    use super::*;
    use crate::msg_box_db::MsgBoxDbMgr;
    use crate::msg_tunnel::ExecutorInstanceState;
    use crate::zone_agent::test_support::{agent_document, put_agent};
    use buckyos_api::{AgentId, AgentProfile, RdbBackend};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn zone_agent(did: &str, owner: &DID, settings: Value, profile: Value) -> ZoneAgent {
        let did = DID::new("web", did);
        ZoneAgent {
            agent_id: AgentId::from_agent_did(&did).unwrap(),
            doc: agent_document(&did, owner),
            settings: serde_json::from_value(settings).unwrap(),
            profile: serde_json::from_value(profile).unwrap(),
        }
    }

    fn active_user() -> UserSettings {
        serde_json::from_value(json!({
            "user_id": "alice", "type": "user", "password": "", "state": "active",
            "res_pool_id": "default"
        }))
        .unwrap()
    }

    async fn new_center() -> (MessageCenter, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("msg-center.db");
        let conn = format!(
            "sqlite:///{}?mode=rwc",
            db_path.to_string_lossy().replace('\\', "/")
        );
        let cfg = buckyos_api::msg_center_default_rdb_instance_config();
        let schema = cfg.schema.get(&RdbBackend::Sqlite).cloned();
        let db = MsgBoxDbMgr::open(&conn, RdbBackend::Sqlite, schema.as_deref())
            .await
            .unwrap();
        (MessageCenter::open_with_db(db).await.unwrap(), tmp)
    }

    #[test]
    fn zone_user_contact_uses_the_profile_did() {
        let profile: UserPrivateProfile = serde_json::from_value(json!({
            "did": "did:web:alice.test.buckyos.io",
            "display_name": "Alice"
        }))
        .unwrap();
        let seed = build_zone_user_seed("alice", active_user(), Some(profile.clone())).unwrap();
        assert_eq!(seed.did, profile.did);
        assert_eq!(seed.name, "Alice");
    }

    #[test]
    fn zone_user_telegram_accounts_are_bare_ids() {
        let profile: UserPrivateProfile = serde_json::from_value(json!({
            "did": "did:web:alice.test.buckyos.io",
            "private_extra": {"system_contact": {"bindings": [
                {"platform": "telegram", "account_id": "user:10001"},
                {"platform": "telegram", "account_id": "20002"},
                {"platform": "email", "account_id": "user:alice@example.com"}
            ]}}
        }))
        .unwrap();
        let seed = build_zone_user_seed("alice", active_user(), Some(profile)).unwrap();
        let accounts: Vec<_> = seed
            .bindings
            .iter()
            .map(|binding| (binding.platform.as_str(), binding.account_id.as_str()))
            .collect();
        assert_eq!(
            accounts,
            vec![
                ("telegram", "10001"),
                ("telegram", "20002"),
                ("email", "user:alice@example.com")
            ]
        );
    }

    #[test]
    fn agent_changes_trigger_contact_sync() {
        let alice = DID::new("bns", "alice");
        let agent = zone_agent("xiaobai.test.buckyos.io", &alice, json!({}), json!({}));
        let original = zone_contact_seed_signature(&[], &[agent.clone()]);
        assert_ne!(original, zone_contact_seed_signature(&[], &[]));
        let mut renamed_agent = agent.clone();
        renamed_agent.profile.display_name = Some("My assistant".into());
        let renamed = zone_contact_seed_signature(&[], &[renamed_agent.clone()]);
        assert_ne!(original, renamed);
        renamed_agent.doc.owner = DID::new("bns", "bob");
        assert_ne!(renamed, zone_contact_seed_signature(&[], &[renamed_agent]));
    }

    #[tokio::test]
    async fn agent_contact_is_named_by_profile_or_agent_name() {
        let (center, _tmp) = new_center().await;
        let alice = DID::new("web", "alice.test.buckyos.io");
        let named = zone_agent(
            "xiaobai.test.buckyos.io",
            &alice,
            json!({}),
            json!({"display_name": " 小白 "}),
        );
        let unnamed = zone_agent(
            "xiaohei.test.buckyos.io",
            &alice,
            json!({}),
            json!({"display_name": "  "}),
        );
        sync_zone_agent_contacts(&center, &[], &[named.clone(), unnamed.clone()])
            .await
            .unwrap();
        for (agent, name) in [(&named, "小白"), (&unnamed, "xiaohei")] {
            let contact = center
                .contact_mgr
                .get_contact(agent.did().clone(), Some(alice.clone()))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(contact.name, name);
        }
        assert_eq!(
            AgentProfile::default().resolved_display_name(&unnamed.agent_id),
            "xiaohei"
        );
    }

    #[tokio::test]
    async fn telegram_bindings_merge_settings_with_agents_that_have_a_spec() {
        let alice = DID::new("web", "alice.test.buckyos.io");
        let ready = agent_document(&DID::new("web", "xiaobai.test.buckyos.io"), &alice);
        let creating = agent_document(&DID::new("web", "xiaohei.test.buckyos.io"), &alice);
        let tunnel = |token: &str| json!({"msg_tunnels": [{"platform": "telegram", "bot_token": token, "bot_account_id": "@bot"}]});
        let mut config = BTreeMap::new();
        put_agent(
            &mut config,
            "alice",
            &ready,
            true,
            tunnel("1:ready"),
            json!({}),
        );
        put_agent(
            &mut config,
            "alice",
            &creating,
            false,
            tunnel("2:creating"),
            json!({}),
        );
        let agents = crate::zone_agent::load_zone_agents(&config).await.unwrap();

        let settings = parse_msg_center_settings(&json!({"telegram_tunnel": {
            "gateway": {"mode": "bot_api"},
            "bindings": [
                {"owner_did": ready.id.to_string(), "bot_token": "9:replaced"},
                {"owner_did": "did:web:ops.test.buckyos.io", "bot_token": "3:ops"}
            ]
        }}))
        .unwrap();
        let merged = with_agent_telegram_bindings(settings.telegram_tunnel, &agents);
        let bindings: Vec<_> = merged
            .bindings
            .iter()
            .map(|binding| {
                (
                    binding.owner_did.as_str(),
                    binding.bot_token.as_str(),
                    binding.bot_account_id.as_deref(),
                )
            })
            .collect();
        let ready_did = ready.id.to_string();
        assert_eq!(
            bindings,
            vec![
                ("did:web:ops.test.buckyos.io", "3:ops", None),
                (ready_did.as_str(), "1:ready", Some("@bot")),
            ]
        );
        assert_eq!(merged.gateway.mode, TelegramGatewayMode::BotApi);
    }

    #[tokio::test]
    async fn bot_api_without_bindings_runs_and_rebuilds_only_on_change() {
        let (center, _tmp) = new_center().await;
        let executor_mgr = Arc::new(DeliveryExecutorMgr::new());
        let sync = ZoneSync::new(center, executor_mgr.clone());
        let activated = json!({"telegram_tunnel": {
            "enabled": true,
            "gateway": {"mode": "bot_api"},
            "bindings": []
        }});
        let result = sync.apply(&activated, &[], true).await.unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["tunnel_started"], true);
        assert_eq!(result["bindings"], 0);
        let transport = DID::from_str(MSG_CENTER_DEFAULT_TG_TUNNEL_DID).unwrap();
        let state = || {
            executor_mgr
                .list_instances()
                .unwrap()
                .into_iter()
                .find(|instance| instance.transport_did == transport)
                .map(|instance| instance.state)
        };
        assert_eq!(state(), Some(ExecutorInstanceState::Running));

        let dry_run = json!({"telegram_tunnel": {"gateway": {"mode": "dry_run"}}});
        sync.apply(&dry_run, &[], false).await.unwrap();
        executor_mgr.stop_instance(&transport).await.unwrap();
        sync.apply(&dry_run, &[], false).await.unwrap();
        assert_eq!(state(), Some(ExecutorInstanceState::Stopped));

        let agent = zone_agent(
            "xiaobai.test.buckyos.io",
            &DID::new("web", "alice.test.buckyos.io"),
            json!({"msg_tunnels": [{"platform": "telegram", "bot_token": "1:a"}]}),
            json!({}),
        );
        let result = sync.apply(&dry_run, &[agent], false).await.unwrap();
        assert_eq!(result["bindings"], 1);
        assert_eq!(state(), Some(ExecutorInstanceState::Running));
    }
}

#[cfg(test)]
mod delivery_failure_tests {
    use super::*;
    use buckyos_api::{DeliveryEnvelope, DeliveryRecord, TransportKind};
    use ndn_lib::{MsgObjKind, NamedObject};

    fn record_with_notice() -> DeliveryRecordWithObject {
        let transport_did = DID::new("bns", "telegram");
        let target_did = DID::new("msgtunnel", "42.user.telegram");
        let mut msg = MsgObject {
            from: DID::new("web", "xiaobai.test.buckyos.io"),
            to: vec![target_did.clone()],
            kind: MsgObjKind::Chat,
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: "result".to_string(),
                ..Default::default()
            },
            created_at_ms: 1,
            ..Default::default()
        };
        msg.meta.insert(
            "delivery_failure_notice".to_string(),
            Value::String("delivery failed".to_string()),
        );
        let msg_id = msg.gen_obj_id().0;
        DeliveryRecordWithObject {
            record: DeliveryRecord {
                delivery_id: "delivery-1".to_string(),
                envelope: DeliveryEnvelope {
                    msg_id,
                    target_did,
                    transport_did,
                    transport: TransportKind::Tunnel {
                        platform: "telegram".to_string(),
                        tunnel_instance_id: "telegram".to_string(),
                    },
                    address: None,
                },
                state: DeliveryState::Sending,
                attempts: 4,
                next_retry_at_ms: None,
                external_msg_id: None,
                delivered_at_ms: None,
                last_error: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
            msg: Some(msg),
        }
    }

    #[test]
    fn builds_plain_text_failure_notice() {
        let record = record_with_notice();
        let notice = build_delivery_failure_notice(&record).unwrap();
        assert_eq!(notice.to, vec![record.record.envelope.target_did]);
        assert_eq!(notice.content.content, "delivery failed");
        assert!(notice.content.refs.is_empty());
        assert_eq!(
            notice.meta.get("delivery_failure_fallback"),
            Some(&Value::Bool(true))
        );
        assert!(!notice.meta.contains_key("delivery_failure_notice"));
    }

    #[test]
    fn does_not_recurse_for_failure_notice() {
        let mut record = record_with_notice();
        record
            .msg
            .as_mut()
            .unwrap()
            .meta
            .insert("delivery_failure_fallback".to_string(), Value::Bool(true));
        assert!(build_delivery_failure_notice(&record).is_none());
    }
}
