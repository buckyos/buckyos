mod contact_mgr;
mod msg_center;
mod msg_tunnel;
#[cfg(test)]
mod test_msg_center;
mod tg_tunnel;

use ::kRPC::*;
use anyhow::{Context, Result};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AccountBinding,
    BuckyOSRuntimeType, MsgCenterServerHandler, UserSettings, UserState, MSG_CENTER_SERVICE_NAME,
    MSG_CENTER_SERVICE_PORT,
};
use buckyos_kit::{get_buckyos_service_data_dir, init_logging};
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use name_lib::DID;
use serde::Deserialize;
use serde_json::Value;
use server_runner::Runner;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::contact_mgr::ZoneUserContactSeed;
use crate::msg_center::MessageCenter;
use crate::msg_tunnel::{MsgTunnel, MsgTunnelInstanceMgr};
use crate::tg_tunnel::{GrammersTgGatewayConfig, TgBotBinding, TgTunnel, TgTunnelConfig};

const MSG_CENTER_HTTP_PATH: &str = "/kapi/msg-center";
const MSG_CENTER_DEFAULT_TG_TUNNEL_DID: &str = "did:bns:msg-center-default-tunnel";
const TG_BINDING_BOT_TOKEN_KEY: &str = "bot_token";
const METHOD_RELOAD_SETTINGS: &str = "reload_settings";
const METHOD_SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
const METHOD_REALOAD_SETTINGS: &str = "reaload_settings";
const METHOD_SERVICE_REALOAD_SETTINGS: &str = "service.reaload_settings";

#[derive(Debug, Clone, Default)]
struct MsgCenterSettings {
    telegram_tunnel: TelegramTunnelSettings,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LegacyMsgTunnelsSettings {
    #[serde(default)]
    telegram_tunnel: Option<TelegramTunnelSettings>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawMsgCenterSettings {
    #[serde(default)]
    telegram_tunnel: Option<TelegramTunnelSettings>,
    #[serde(default)]
    msg_tunnels: LegacyMsgTunnelsSettings,
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
    default_chat_id: Option<String>,
    #[serde(default)]
    extra: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramTunnelSettings {
    #[serde(default = "default_tg_tunnel_enabled")]
    enabled: bool,
    #[serde(default = "default_tg_tunnel_did")]
    tunnel_did: String,
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
            tunnel_did: default_tg_tunnel_did(),
            supports_ingress: default_true(),
            supports_egress: default_true(),
            gateway: TelegramGatewaySettings::default(),
            bindings: vec![],
        }
    }
}

struct MsgCenterHttpServer {
    rpc_handler: MsgCenterServerHandler<MessageCenter>,
    tunnel_mgr: Arc<MsgTunnelInstanceMgr>,
}

impl MsgCenterHttpServer {
    fn new(center: MessageCenter, tunnel_mgr: Arc<MsgTunnelInstanceMgr>) -> Self {
        Self {
            rpc_handler: MsgCenterServerHandler::new(center),
            tunnel_mgr,
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
            apply_tg_tunnel_settings(&self.rpc_handler.0, self.tunnel_mgr.as_ref(), &settings)
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

fn default_tg_tunnel_did() -> String {
    MSG_CENTER_DEFAULT_TG_TUNNEL_DID.to_string()
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

    if raw.telegram_tunnel.is_none() && raw.msg_tunnels.telegram_tunnel.is_some() {
        info!("msg-center settings uses legacy key: msg_tunnels.telegram_tunnel");
    }

    let telegram_tunnel = raw
        .telegram_tunnel
        .or(raw.msg_tunnels.telegram_tunnel)
        .unwrap_or_default();
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

fn build_zone_user_seed(username: &str, settings: UserSettings) -> Option<ZoneUserContactSeed> {
    if !matches!(settings.state, UserState::Active) {
        return None;
    }

    let mut contact_did = DID::new("bns", username);
    let mut note = None;
    let mut groups = vec!["zone_user".to_string()];
    let mut tags = vec!["zone_user".to_string()];
    let mut bindings = Vec::new();

    if let Some(contact_cfg) = settings.contact {
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
            let tunnel_id = binding
                .tunnel_id
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("{}-default", platform.to_ascii_lowercase()));

            bindings.push(AccountBinding {
                platform,
                account_id,
                display_id,
                tunnel_id,
                last_active_at: now_ms(),
                meta: binding.meta,
            });
        }
    }

    let name = settings.show_name.trim().to_string();
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

        if let Some(seed) = build_zone_user_seed(username.as_str(), settings) {
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

fn resolve_tg_tunnel_did(settings: &TelegramTunnelSettings) -> Result<DID> {
    DID::from_str(settings.tunnel_did.trim()).map_err(|e| {
        anyhow::anyhow!(
            "invalid telegram tunnel did {}, err={}",
            settings.tunnel_did,
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
                tunnel_did: Some(cfg.tunnel_did.clone()),
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
                default_chat_id: binding.default_chat_id.clone(),
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

async fn clear_tunnel_instances(tunnel_mgr: &MsgTunnelInstanceMgr) -> Result<()> {
    let instances = tunnel_mgr
        .list_instances()
        .map_err(|err| anyhow::anyhow!("list tunnel instances failed: {}", err))?;
    for instance in instances.iter() {
        if let Err(err) = tunnel_mgr.stop_instance(&instance.tunnel_did).await {
            warn!(
                "stop tunnel {} failed during reload: {}",
                instance.tunnel_did.to_string(),
                err
            );
        }
    }

    let instances = tunnel_mgr
        .list_instances()
        .map_err(|err| anyhow::anyhow!("list tunnel instances failed: {}", err))?;
    for instance in instances.iter() {
        if let Err(err) = tunnel_mgr.unregister(&instance.tunnel_did) {
            warn!(
                "unregister tunnel {} failed during reload: {}",
                instance.tunnel_did.to_string(),
                err
            );
        }
    }
    let remaining = tunnel_mgr
        .list_instances()
        .map_err(|err| anyhow::anyhow!("list tunnel instances failed: {}", err))?;
    if !remaining.is_empty() {
        return Err(anyhow::anyhow!(
            "clear tunnel instances incomplete, {} instance(s) still registered",
            remaining.len()
        ));
    }
    Ok(())
}

async fn apply_tg_tunnel_settings(
    center: &MessageCenter,
    tunnel_mgr: &MsgTunnelInstanceMgr,
    raw_settings: &Value,
) -> Result<serde_json::Value> {
    let settings = parse_msg_center_settings(raw_settings)?;
    if !settings.telegram_tunnel.enabled {
        info!("telegram tunnel is disabled by settings");
        clear_tunnel_instances(tunnel_mgr).await?;
        return Ok(serde_json::json!({
            "ok": true,
            "tunnel_enabled": false,
            "tunnel_started": false,
            "bindings": 0
        }));
    }

    let tunnel_did = resolve_tg_tunnel_did(&settings.telegram_tunnel)?;
    let mut cfg = TgTunnelConfig::new(tunnel_did.clone());
    cfg.supports_ingress = settings.telegram_tunnel.supports_ingress;
    cfg.supports_egress = settings.telegram_tunnel.supports_egress;
    let tg_tunnel = Arc::new(build_tg_tunnel(cfg, &settings.telegram_tunnel)?);

    tg_tunnel
        .bind_msg_center_handler(Arc::new(center.clone()))
        .context("bind msg_center handler to telegram tunnel failed")?;
    bind_tg_tunnel_bots(tg_tunnel.as_ref(), &settings.telegram_tunnel)?;
    info!(
        "telegram tunnel {} loaded {} binding(s)",
        tunnel_did.to_string(),
        settings.telegram_tunnel.bindings.len()
    );

    clear_tunnel_instances(tunnel_mgr).await?;
    tunnel_mgr
        .register(tg_tunnel.clone())
        .map_err(|e| anyhow::anyhow!("register telegram tunnel failed: {}", e))?;

    let mut started = false;
    let mut start_error: Option<String> = None;
    match tunnel_mgr.start_instance(&tunnel_did).await {
        Ok(_) => {
            started = true;
            info!(
                "telegram tunnel {} started (ingress={}, egress={})",
                tunnel_did.to_string(),
                tg_tunnel.supports_ingress(),
                tg_tunnel.supports_egress()
            );
        }
        Err(err) => {
            warn!(
                "telegram tunnel {} start failed, continue without tg tunnel: {}",
                tunnel_did.to_string(),
                err
            );
            start_error = Some(err.to_string());
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "tunnel_enabled": true,
        "tunnel_did": tunnel_did.to_string(),
        "tunnel_started": started,
        "bindings": settings.telegram_tunnel.bindings.len(),
        "start_error": start_error
    }))
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

    set_buckyos_api_runtime(runtime);

    let center = MessageCenter::try_new()
        .map_err(|err| anyhow::anyhow!("create message center failed: {:?}", err))?;

    let tunnel_mgr = Arc::new(MsgTunnelInstanceMgr::new());
    match apply_tg_tunnel_settings(&center, tunnel_mgr.as_ref(), &settings).await {
        Ok(result) => {
            info!("msg-center settings initialized: {}", result);
        }
        Err(err) => {
            warn!(
                "msg-center settings apply failed during startup, continue without tunnel: {}",
                err
            );
        }
    }
    if let Err(error) = sync_zone_user_contacts_once(&center, &settings).await {
        warn!("zone-user sync failed during startup: {}", error);
    }
    let server = MsgCenterHttpServer::new(center, tunnel_mgr);

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

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    if let Err(err) = rt.block_on(async {
        init_logging("msg_center", true);
        start_msg_center_service().await
    }) {
        error!("msg-center service start failed: {:?}", err);
    }
}
