mod contact_mgr;
mod group_mgr;
mod message_hub;
mod msg_box_db;
mod msg_center;
mod msg_tunnel;
#[cfg(test)]
mod test_msg_center;
mod tg_tunnel;

use ::kRPC::*;
use anyhow::{Context, Result};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AccountBinding,
    BuckyOSRuntimeType, DeliveryReportResult, MsgCenterClient, MsgCenterServerHandler,
    SystemConfigClient, UserContactSettings, UserPrivateProfile, UserSettings, UserState,
    MSG_CENTER_SERVICE_NAME, MSG_CENTER_SERVICE_PORT,
};
use buckyos_http_server::Runner;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::{get_buckyos_service_data_dir, init_logging};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use name_lib::DID;
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

const MSG_CENTER_HTTP_PATH: &str = "/kapi/msg-center";
const MSG_CENTER_DEFAULT_TG_TUNNEL_DID: &str = "did:bns:msg-center-default-tunnel";
const PROFILE_SYSTEM_CONTACT_KEY: &str = "system_contact";
const TG_BINDING_BOT_TOKEN_KEY: &str = "bot_token";
const METHOD_RELOAD_SETTINGS: &str = "reload_settings";
const METHOD_SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
const METHOD_REALOAD_SETTINGS: &str = "reaload_settings";
const METHOD_SERVICE_REALOAD_SETTINGS: &str = "service.reaload_settings";
const DELIVERY_PUMP_IDLE_SLEEP_MS: u64 = 300;
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

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize, Default)]
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

#[derive(Debug, Clone, Deserialize)]
struct TelegramBindingSettings {
    owner_did: String,
    bot_token: String,
    #[serde(default)]
    bot_account_id: Option<String>,
    #[serde(default)]
    extra: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
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
    executor_mgr: Arc<DeliveryExecutorMgr>,
}

impl MsgCenterHttpServer {
    fn new(center: MessageCenter, executor_mgr: Arc<DeliveryExecutorMgr>) -> Self {
        Self {
            rpc_handler: MsgCenterServerHandler::new(center),
            executor_mgr,
        }
    }

    async fn handle_reload_settings(&self) -> std::result::Result<serde_json::Value, RPCErrors> {
        let runtime = get_buckyos_api_runtime()
            .map_err(|err| RPCErrors::ReasonError(format!("get runtime failed: {}", err)))?;
        let settings = match runtime.get_my_settings().await {
            Ok(settings) => settings,
            Err(err) => {
                warn!(
                    "load msg-center settings failed during reload, use empty settings: {}",
                    err
                );
                serde_json::json!({})
            }
        };

        let tunnel_result =
            apply_tg_tunnel_settings(&self.rpc_handler.0, self.executor_mgr.as_ref(), &settings)
                .await
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("reload msg-center settings failed: {}", err))
                })?;

        if let Err(error) = sync_zone_user_contacts_once(&self.rpc_handler.0, &settings).await {
            warn!("reload settings zone-user sync failed: {}", error);
        }

        Ok(tunnel_result)
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
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait::async_trait]
impl HttpServer for MsgCenterHttpServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
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

fn collect_sync_owner_dids(raw_settings: &Value) -> Vec<DID> {
    let mut owners = Vec::new();
    let settings = match parse_msg_center_settings(raw_settings) {
        Ok(settings) => settings,
        Err(error) => {
            warn!(
                "parse msg-center settings failed while collecting sync owners: {}",
                error
            );
            return owners;
        }
    };

    for binding in settings.telegram_tunnel.bindings {
        let owner_raw = binding.owner_did.trim();
        if owner_raw.is_empty() {
            continue;
        }
        match DID::from_str(owner_raw) {
            Ok(owner) => {
                if !owners.iter().any(|existing| existing == &owner) {
                    owners.push(owner);
                }
            }
            Err(error) => {
                warn!(
                    "skip invalid telegram binding owner_did while collecting sync owners: owner_did={}, error={}",
                    owner_raw,
                    error
                );
            }
        }
    }

    owners
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
        if let Err(err) = msg_center
            .report_delivery(delivery_id.clone(), report)
            .await
        {
            // The SENDING lease sweep will reclaim this row if the report is
            // lost for good.
            warn!(
                "msg-center report_delivery failed: executor={} delivery_id={} err={}",
                transport_did.to_string(),
                delivery_id,
                err
            );
        } else if report_ok {
            info!(
                "msg-center delivery done: executor={} delivery_id={}",
                transport_did.to_string(),
                delivery_id
            );
        } else {
            warn!(
                "msg-center delivery reported failure: executor={} delivery_id={}",
                transport_did.to_string(),
                delivery_id
            );
        }
    }

    Ok(worked)
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

    let mut contact_did = DID::new("bns", username);
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
                            "invalid user contact.did, fallback to did:bns:{}: did={}, error={}",
                            username, did, error
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
            let account_id = binding.account_id.trim().to_string();
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

async fn sync_zone_user_contacts_once(center: &MessageCenter, raw_settings: &Value) -> Result<()> {
    let contacts = load_zone_user_contact_seeds().await?;
    let mut owner_scopes: Vec<Option<DID>> = vec![None];
    for owner in collect_sync_owner_dids(raw_settings) {
        owner_scopes.push(Some(owner));
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
            "zone user sync applied on settings reload/startup: owner_scope={}, contacts={}",
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
    raw_settings: &Value,
) -> Result<serde_json::Value> {
    let settings = parse_msg_center_settings(raw_settings)?;
    if !settings.telegram_tunnel.enabled {
        info!("telegram tunnel is disabled by settings");
        clear_tunnel_instances(center, executor_mgr).await?;
        return Ok(serde_json::json!({
            "ok": true,
            "tunnel_enabled": false,
            "tunnel_started": false,
            "bindings": 0
        }));
    }

    let transport_did = resolve_tg_transport_did(&settings.telegram_tunnel)?;
    let tunnel_instance_id = settings
        .telegram_tunnel
        .tunnel_instance_id
        .trim()
        .to_string();
    let tunnel_instance_id = if tunnel_instance_id.is_empty() {
        default_tg_tunnel_instance_id()
    } else {
        tunnel_instance_id
    };
    let mut cfg = TgTunnelConfig::new(transport_did.clone());
    cfg.tunnel_instance_id = tunnel_instance_id.clone();
    cfg.supports_ingress = settings.telegram_tunnel.supports_ingress;
    cfg.supports_egress = settings.telegram_tunnel.supports_egress;

    // Rebuild the executor set + tunnel route registry from settings.
    clear_tunnel_instances(center, executor_mgr).await?;
    let tg_tunnel = Arc::new(build_tg_tunnel(cfg, &settings.telegram_tunnel)?);

    tg_tunnel
        .bind_msg_center_handler(Arc::new(center.clone()))
        .context("bind msg_center handler to telegram tunnel failed")?;
    bind_tg_tunnel_bots(tg_tunnel.as_ref(), &settings.telegram_tunnel)?;
    // Bot owners (agents) receive replies natively via the message hub.
    let binding_owner_dids = settings
        .telegram_tunnel
        .bindings
        .iter()
        .filter_map(|binding| DID::from_str(binding.owner_did.trim()).ok());
    center.register_local_recipients(binding_owner_dids);
    info!(
        "telegram tunnel {} (instance '{}') loaded {} binding(s)",
        transport_did.to_string(),
        tunnel_instance_id,
        settings.telegram_tunnel.bindings.len()
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
        "bindings": settings.telegram_tunnel.bindings.len(),
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
    center.start_idempotency_sweep();

    let executor_mgr = Arc::new(DeliveryExecutorMgr::new());
    register_message_hub(&center, executor_mgr.as_ref()).await?;
    // Tunnel assembly must fail startup on configuration errors — most
    // importantly a duplicate tunnel_instance_id must never be silently
    // overwritten (shadow endpoint DID stability depends on it).
    let tunnel_result = apply_tg_tunnel_settings(&center, executor_mgr.as_ref(), &settings)
        .await
        .map_err(|err| anyhow::anyhow!("assemble tunnel registry failed: {}", err))?;
    info!("msg-center settings initialized: {}", tunnel_result);
    if let Err(error) = sync_zone_user_contacts_once(&center, &settings).await {
        warn!("zone-user sync failed during startup: {}", error);
    }
    start_delivery_pump(center.clone(), executor_mgr.clone());
    let server = MsgCenterHttpServer::new(center, executor_mgr);

    let runner = Runner::new(MSG_CENTER_SERVICE_PORT);
    if let Err(err) = runner.add_http_server(MSG_CENTER_HTTP_PATH.to_string(), Arc::new(server)) {
        error!("failed to add msg-center http server: {:?}", err);
        return Err(anyhow::anyhow!(
            "failed to add msg-center http server: {:?}",
            err
        ));
    }
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
