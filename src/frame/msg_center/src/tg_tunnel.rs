use crate::msg_tunnel::MsgTunnel;
use anyhow::{bail, Context, Result as AnyResult};
use async_trait::async_trait;
use buckyos_api::{
    DeliveryReportResult, IngressContext, MsgCenterHandler, MsgObject, MsgRecordWithObject,
    MSG_CENTER_SERVICE_NAME,
};
use buckyos_kit::get_buckyos_service_data_dir;
use grammers_client::session::defs::{PeerAuth, PeerId, PeerRef};
use grammers_client::session::storages::SqliteSession;
use grammers_client::session::updates::UpdatesLike;
use grammers_client::types::update::Message as TgMessage;
use grammers_client::types::{Peer as TgPeer, Update};
use grammers_client::{Client, UpdatesConfiguration};
use grammers_mtsender::{SenderPool, SenderPoolHandle};
use kRPC::RPCContext;
use log::{info, warn};
use name_lib::DID;
use ndn_lib::ObjId;
use reqwest::Client as HttpClient;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc::UnboundedReceiver, Mutex};
use tokio::task::JoinHandle;
use tokio::time::Duration;

const TELEGRAM_PLATFORM: &str = "telegram";
const TG_API_ID_ENV_KEY: &str = "BUCKYOS_TG_API_ID";
const TG_API_HASH_ENV_KEY: &str = "BUCKYOS_TG_API_HASH";
const TG_SESSION_DIR_ENV_KEY: &str = "BUCKYOS_TG_SESSION_DIR";
const TG_BINDING_EXTRA_BOT_TOKEN: &str = "bot_token";
const TG_BOT_API_ENDPOINT: &str = "https://api.telegram.org";

#[derive(Debug, Clone)]
pub struct TgTunnelConfig {
    pub tunnel_did: DID,
    pub name: String,
    pub supports_ingress: bool,
    pub supports_egress: bool,
}

impl TgTunnelConfig {
    pub fn new(tunnel_did: DID) -> Self {
        Self {
            name: format!("{}-tg-tunnel", tunnel_did.to_string()),
            tunnel_did,
            supports_ingress: true,
            supports_egress: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgBotBinding {
    pub owner_did: DID,
    pub bot_account_id: String,
    pub bot_token_env_key: Option<String>,
    pub default_chat_id: Option<String>,
    pub extra: HashMap<String, String>,
}

impl TgBotBinding {
    pub fn validate(&self) -> AnyResult<()> {
        if self.bot_account_id.trim().is_empty() {
            bail!("bot_account_id is required");
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TgEgressEnvelope {
    pub sender_did: DID,
    pub bot_account_id: String,
    pub chat_id: Option<String>,
    pub text: Option<String>,
    pub payload: Value,
    pub record_id: String,
}

struct TgIngressDispatch {
    msg: MsgObject,
    ingress_ctx: IngressContext,
    idempotency_key: String,
}

struct TgMessageConverter;

impl TgMessageConverter {
    fn chat_kind(chat: &TgPeer) -> &'static str {
        match chat {
            TgPeer::User(_) => "user",
            TgPeer::Group(_) => "group",
            TgPeer::Channel(_) => "channel",
        }
    }

    fn build_dispatch_key(bot_account_id: &str, chat_id: i64, message_id: i32) -> String {
        format!("tg:{}:{}:{}", bot_account_id, chat_id, message_id)
    }

    fn fnv1a64(raw: &[u8]) -> u64 {
        const OFFSET: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;

        let mut hash = OFFSET;
        for byte in raw {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(PRIME);
        }
        hash
    }

    fn build_msg_obj_id(idempotency_key: &str) -> AnyResult<ObjId> {
        let hash = Self::fnv1a64(idempotency_key.as_bytes());
        let raw = format!("chunk:{:016x}", hash);
        ObjId::new(&raw).with_context(|| format!("failed to build msg obj id from {}", raw))
    }

    fn extract_text_from_payload(payload: &Value) -> Option<String> {
        payload
            .get("msg_payload")
            .and_then(|msg_payload| msg_payload.get("text"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn msg_object_to_tg_content(msg: &MsgObject) -> (Option<String>, Value) {
        let text = msg
            .payload
            .get("text")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let payload = json!({
            "msg_payload": msg.payload,
            "msg_meta": msg.meta,
            "thread_key": msg.thread_key,
        });
        (text, payload)
    }

    fn tg_message_to_msg_object(
        owner_did: DID,
        sender_did: DID,
        sender_account_id: String,
        chat: &TgPeer,
        sender_chat: &TgPeer,
        bot_account_id: &str,
        tunnel_did: Option<DID>,
        message: &TgMessage,
    ) -> AnyResult<TgIngressDispatch> {
        let chat_id = chat.id().bot_api_dialog_id();
        let chat_type = Self::chat_kind(chat);
        let idempotency_key = Self::build_dispatch_key(bot_account_id, chat_id, message.id());
        let msg_obj_id = Self::build_msg_obj_id(&idempotency_key)?;
        let created_at_ms = message.date().timestamp_millis().max(0) as u64;
        let text = message.text().to_string();
        let payload_kind = if text.trim().is_empty() {
            "telegram_event"
        } else {
            "text"
        };

        let msg = MsgObject {
            id: msg_obj_id,
            from: sender_did,
            source: None,
            to: vec![owner_did.clone()],
            thread_key: Some(format!("tg:{}:{}", bot_account_id, chat_id)),
            payload: json!({
                "kind": payload_kind,
                "text": text,
                "telegram": {
                    "message_id": message.id(),
                    "chat_id": chat_id,
                    "chat_type": chat_type,
                    "chat_name": chat.name(),
                }
            }),
            meta: Some(json!({
                "telegram": {
                    "chat_dialog_id": chat_id,
                    "chat_username": chat.username(),
                    "sender_id": sender_chat.id().bot_api_dialog_id(),
                    "sender_username": sender_chat.username(),
                    "sender_name": sender_chat.name(),
                    "bot_account_id": bot_account_id,
                }
            })),
            created_at_ms,
        };

        let ingress_ctx = IngressContext {
            tunnel_did,
            platform: Some(TELEGRAM_PLATFORM.to_string()),
            chat_id: Some(chat_id.to_string()),
            source_account_id: Some(sender_account_id),
            context_id: Some(format!("tg:{}:{}", owner_did.to_string(), chat_id)),
            extra: Some(json!({
                "tg_message_id": message.id(),
                "chat_type": chat_type,
            })),
        };

        Ok(TgIngressDispatch {
            msg,
            ingress_ctx,
            idempotency_key,
        })
    }
}

#[async_trait]
pub trait TgGateway: Send + Sync {
    async fn start(&self, bindings: &[TgBotBinding]) -> AnyResult<()>;
    async fn stop(&self) -> AnyResult<()>;
    async fn send(&self, envelope: TgEgressEnvelope) -> AnyResult<DeliveryReportResult>;

    async fn set_dispatcher(&self, dispatcher: Option<Arc<dyn MsgCenterHandler>>) -> AnyResult<()> {
        let _ = dispatcher;
        Ok(())
    }
}

#[derive(Default)]
pub struct DryRunTgGateway {
    running: AtomicBool,
    seq: AtomicU64,
}

#[async_trait]
impl TgGateway for DryRunTgGateway {
    async fn start(&self, _bindings: &[TgBotBinding]) -> AnyResult<()> {
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&self) -> AnyResult<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send(&self, envelope: TgEgressEnvelope) -> AnyResult<DeliveryReportResult> {
        if !self.running.load(Ordering::SeqCst) {
            bail!("dry-run tg gateway is not running");
        }

        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let ext_id = format!(
            "dry-tg-{}-{}",
            envelope
                .chat_id
                .as_deref()
                .map(Self::sanitize)
                .unwrap_or_else(|| "unknown-chat".to_string()),
            seq
        );

        Ok(DeliveryReportResult {
            ok: true,
            external_msg_id: Some(ext_id),
            delivered_at_ms: Some(TgTunnel::now_ms()),
            ..Default::default()
        })
    }
}

impl DryRunTgGateway {
    fn sanitize(value: &str) -> String {
        let mut output = String::with_capacity(value.len());
        for ch in value.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                output.push(ch);
            }
        }

        if output.is_empty() {
            "unknown-chat".to_string()
        } else {
            output
        }
    }
}

#[derive(Debug, Clone)]
pub struct GrammersTgGatewayConfig {
    pub api_id: i32,
    pub api_hash: String,
    pub session_dir: PathBuf,
    pub tunnel_did: Option<DID>,
}

impl GrammersTgGatewayConfig {
    pub fn from_env() -> AnyResult<Self> {
        let api_id = std::env::var(TG_API_ID_ENV_KEY)
            .with_context(|| format!("{} is required", TG_API_ID_ENV_KEY))?
            .trim()
            .parse::<i32>()
            .context("failed to parse BUCKYOS_TG_API_ID as i32")?;
        if api_id <= 0 {
            bail!("BUCKYOS_TG_API_ID must be > 0");
        }

        let api_hash = std::env::var(TG_API_HASH_ENV_KEY)
            .with_context(|| format!("{} is required", TG_API_HASH_ENV_KEY))?;
        if api_hash.trim().is_empty() {
            bail!("BUCKYOS_TG_API_HASH cannot be empty");
        }

        let session_dir = std::env::var(TG_SESSION_DIR_ENV_KEY)
            .unwrap_or_else(|_| default_tg_session_dir().to_string_lossy().to_string());

        Ok(Self {
            api_id,
            api_hash,
            session_dir: PathBuf::from(session_dir),
            tunnel_did: None,
        })
    }
}

fn default_tg_session_dir() -> PathBuf {
    get_buckyos_service_data_dir(MSG_CENTER_SERVICE_NAME).join("tg_sessions")
}

fn resolve_binding_bot_token(binding: &TgBotBinding) -> AnyResult<String> {
    if let Some(token) = binding.extra.get(TG_BINDING_EXTRA_BOT_TOKEN) {
        if !token.trim().is_empty() {
            return Ok(token.to_string());
        }
    }

    let env_key = binding.bot_token_env_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "bot_token_env_key is required for {}",
            binding.owner_did.to_string()
        )
    })?;
    let token = std::env::var(env_key).with_context(|| {
        format!(
            "env {} is required for {}",
            env_key,
            binding.owner_did.to_string()
        )
    })?;
    if token.trim().is_empty() {
        bail!(
            "env {} is empty for {}",
            env_key,
            binding.owner_did.to_string()
        );
    }
    Ok(token)
}

struct GrammersTgRuntime {
    owner_did: DID,
    bot_account_id: String,
    client: Client,
    sender_pool_handle: SenderPoolHandle,
    sender_pool_task: JoinHandle<()>,
}

pub struct GrammersTgGateway {
    cfg: GrammersTgGatewayConfig,
    runtimes: Mutex<HashMap<String, GrammersTgRuntime>>,
    dispatcher: Arc<Mutex<Option<Arc<dyn MsgCenterHandler>>>>,
    ingress_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl GrammersTgGateway {
    pub fn new(cfg: GrammersTgGatewayConfig) -> Self {
        Self {
            cfg,
            runtimes: Mutex::new(HashMap::new()),
            dispatcher: Arc::new(Mutex::new(None)),
            ingress_tasks: Mutex::new(HashMap::new()),
        }
    }

    pub fn from_env() -> AnyResult<Self> {
        Ok(Self::new(GrammersTgGatewayConfig::from_env()?))
    }

    fn session_file_name(binding: &TgBotBinding) -> String {
        format!(
            "{}__{}.session",
            Self::sanitize(&binding.owner_did.to_string()),
            Self::sanitize(&binding.bot_account_id)
        )
    }

    fn sanitize(raw: &str) -> String {
        let mut output = String::with_capacity(raw.len());
        let mut prev_dash = false;
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() {
                output.push(ch.to_ascii_lowercase());
                prev_dash = false;
            } else if !prev_dash {
                output.push('-');
                prev_dash = true;
            }
        }

        let trimmed = output.trim_matches('-');
        if trimmed.is_empty() {
            "default".to_string()
        } else {
            trimmed.chars().take(96).collect()
        }
    }

    fn resolve_text(envelope: &TgEgressEnvelope) -> String {
        if let Some(text) = envelope.text.as_ref() {
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }

        if let Some(text) = TgMessageConverter::extract_text_from_payload(&envelope.payload) {
            return text;
        }

        "[unsupported message payload]".to_string()
    }

    fn username_from_chat_id(chat_id: &str) -> Option<String> {
        let trimmed = chat_id.trim();
        if trimmed.is_empty() {
            return None;
        }

        if let Some(name) = trimmed.strip_prefix('@') {
            return Self::sanitize_username(name);
        }

        if let Some(suffix) = trimmed
            .strip_prefix("https://t.me/")
            .or_else(|| trimmed.strip_prefix("http://t.me/"))
        {
            let candidate = suffix
                .split('/')
                .next()
                .unwrap_or_default()
                .split('?')
                .next()
                .unwrap_or_default()
                .split('#')
                .next()
                .unwrap_or_default();
            return Self::sanitize_username(candidate);
        }

        Self::sanitize_username(trimmed)
    }

    fn sanitize_username(raw: &str) -> Option<String> {
        let value = raw.trim();
        if value.len() < 4 || value.len() > 64 {
            return None;
        }
        if !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return None;
        }
        if !value
            .chars()
            .any(|ch| ch.is_ascii_alphabetic() || ch == '_')
        {
            return None;
        }
        Some(value.to_string())
    }

    fn chat_account_id(chat: &TgPeer) -> String {
        format!(
            "{}:{}",
            TgMessageConverter::chat_kind(chat),
            chat.id().bot_api_dialog_id()
        )
    }

    fn profile_hint_from_chat(
        chat: &TgPeer,
        bot_account_id: &str,
        tunnel_did: Option<&DID>,
    ) -> Value {
        let chat_id = chat.id().bot_api_dialog_id();
        let username = chat.username().map(|value| value.to_string());
        let display_id = username
            .as_ref()
            .map(|value| format!("@{}", value))
            .unwrap_or_else(|| chat_id.to_string());
        json!({
            "chat_type": TgMessageConverter::chat_kind(chat),
            "chat_id": chat_id,
            "name": chat.name(),
            "username": username,
            "display_id": display_id,
            "bot_account_id": bot_account_id,
            "tunnel_id": tunnel_did.map(|did| did.to_string()).unwrap_or_default(),
        })
    }

    async fn resolve_chat_did(
        dispatcher: &Arc<dyn MsgCenterHandler>,
        owner_scope: Option<DID>,
        chat: &TgPeer,
        bot_account_id: &str,
        tunnel_did: Option<&DID>,
    ) -> AnyResult<(DID, String)> {
        let account_id = Self::chat_account_id(chat);
        let did = dispatcher
            .handle_resolve_did(
                TELEGRAM_PLATFORM.to_string(),
                account_id.clone(),
                Some(Self::profile_hint_from_chat(
                    chat,
                    bot_account_id,
                    tunnel_did,
                )),
                owner_scope,
                RPCContext::default(),
            )
            .await?;
        Ok((did, account_id))
    }

    async fn dispatch_incoming_message(
        dispatcher: Arc<dyn MsgCenterHandler>,
        owner_did: DID,
        bot_account_id: String,
        tunnel_did: Option<DID>,
        message: TgMessage,
    ) -> AnyResult<()> {
        if message.outgoing() {
            return Ok(());
        }

        let chat = match message.peer() {
            Ok(chat) => chat.clone(),
            Err(peer_ref) => {
                warn!(
                    "skip telegram message with unresolved chat peer: owner={}, bot={}, chat_id={}",
                    owner_did.to_string(),
                    bot_account_id,
                    peer_ref.id.bot_api_dialog_id()
                );
                return Ok(());
            }
        };
        let sender_chat = message.sender().cloned().unwrap_or_else(|| chat.clone());
        let owner_scope = Some(owner_did.clone());

        let (sender_did, sender_account_id) = Self::resolve_chat_did(
            &dispatcher,
            owner_scope.clone(),
            &sender_chat,
            &bot_account_id,
            tunnel_did.as_ref(),
        )
        .await
        .map_err(|error| {
            warn!(
                "telegram ingress resolve sender did failed (gateway=grammers): owner={}, bot={}, message_id={}, error={}",
                owner_did.to_string(),
                bot_account_id,
                message.id(),
                error
            );
            error
        })?;
        let converted = TgMessageConverter::tg_message_to_msg_object(
            owner_did,
            sender_did,
            sender_account_id,
            &chat,
            &sender_chat,
            &bot_account_id,
            tunnel_did,
            &message,
        )
        .map_err(|error| {
            warn!(
                "telegram ingress convert message failed (gateway=grammers): owner={}, bot={}, message_id={}, error={}",
                owner_scope
                    .as_ref()
                    .map(|did| did.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                bot_account_id,
                message.id(),
                error
            );
            error
        })?;
        let owner_for_log = converted
            .msg
            .to
            .first()
            .map(|did| did.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        info!(
            "telegram ingress message received (gateway=grammers): owner={}, bot={}, chat_id={}, message_id={}",
            owner_for_log,
            bot_account_id,
            converted
                .ingress_ctx
                .chat_id
                .as_deref()
                .unwrap_or(""),
            message.id()
        );

        let dispatch_result = dispatcher
            .handle_dispatch(
                converted.msg,
                Some(converted.ingress_ctx),
                Some(converted.idempotency_key),
                RPCContext::default(),
            )
            .await
            .map_err(|error| {
                warn!(
                    "telegram ingress handle_dispatch failed (gateway=grammers): owner={}, bot={}, message_id={}, error={}",
                    owner_for_log,
                    bot_account_id,
                    message.id(),
                    error
                );
                error
            })?;

        if !dispatch_result.ok {
            warn!(
                "telegram ingress dispatch result not ok (gateway=grammers): owner={}, bot={}, message_id={}, reason={}",
                owner_for_log,
                bot_account_id,
                message.id(),
                dispatch_result.reason.as_deref().unwrap_or("-"),
            );
        }
        if !dispatch_result.dropped_recipients.is_empty() {
            warn!(
                "telegram ingress dropped recipients (gateway=grammers): owner={}, bot={}, message_id={}, dropped={}",
                owner_for_log,
                bot_account_id,
                message.id(),
                dispatch_result
                    .dropped_recipients
                    .iter()
                    .map(|did| did.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        let delivered_count = dispatch_result.delivered_recipients.len()
            + dispatch_result.delivered_agents.len()
            + usize::from(dispatch_result.delivered_group.is_some());
        if delivered_count == 0 {
            warn!(
                "telegram ingress dispatched but nothing delivered (gateway=grammers): owner={}, bot={}, message_id={}, reason={}",
                owner_for_log,
                bot_account_id,
                message.id(),
                dispatch_result.reason.as_deref().unwrap_or("-"),
            );
        }

        Ok(())
    }

    fn spawn_ingress_task(
        &self,
        owner_did: DID,
        bot_account_id: String,
        client: Client,
        updates_rx: UnboundedReceiver<UpdatesLike>,
    ) -> JoinHandle<()> {
        let dispatcher = self.dispatcher.clone();
        let tunnel_did = self.cfg.tunnel_did.clone();
        tokio::spawn(async move {
            let mut updates = client.stream_updates(
                updates_rx,
                UpdatesConfiguration {
                    catch_up: true,
                    ..Default::default()
                },
            );
            loop {
                match updates.next().await {
                    Ok(Update::NewMessage(message)) => {
                        info!("tg_tunnel get tg msg:{}", message.text());
                        let dispatcher = {
                            let guard = dispatcher.lock().await;
                            guard.clone()
                        };
                        let Some(dispatcher) = dispatcher else {
                            warn!(
                                "telegram ingress dispatcher missing, dropping message (gateway=grammers): owner={}, bot={}, message_id={}",
                                owner_did.to_string(),
                                bot_account_id,
                                message.id(),
                            );
                            continue;
                        };
                        if let Err(error) = Self::dispatch_incoming_message(
                            dispatcher,
                            owner_did.clone(),
                            bot_account_id.clone(),
                            tunnel_did.clone(),
                            message,
                        )
                        .await
                        {
                            warn!(
                                "telegram ingress dispatch failed, owner={}, bot={}, error={}",
                                owner_did.to_string(),
                                bot_account_id,
                                error
                            );
                        }
                    }
                    Ok(_) => {
                        warn!("tg_tunnel get unknwon uupdate");
                    }
                    Err(grammers_client::InvocationError::Dropped) => break,
                    Err(error) => {
                        warn!(
                            "telegram update loop failed, owner={}, bot={}, error={}",
                            owner_did.to_string(),
                            bot_account_id,
                            error
                        );
                        tokio::time::sleep(Duration::from_millis(800)).await;
                    }
                }
            }
        })
    }

    fn peer_ref_from_dialog_id(dialog_id: i64) -> AnyResult<PeerRef> {
        if (1..=0xFFFF_FFFFFF).contains(&dialog_id) {
            return Ok(PeerRef {
                id: PeerId::user(dialog_id),
                auth: PeerAuth::default(),
            });
        }

        if (-999_999_999_999..=-1).contains(&dialog_id) {
            return Ok(PeerRef {
                id: PeerId::chat(-dialog_id),
                auth: PeerAuth::default(),
            });
        }

        if (-1_997_852_516_352..=-1_000_000_000_001).contains(&dialog_id)
            || (-4_000_000_000_000..=-2_002_147_483_649).contains(&dialog_id)
        {
            let channel_id = -dialog_id - 1_000_000_000_000;
            return Ok(PeerRef {
                id: PeerId::channel(channel_id),
                auth: PeerAuth::default(),
            });
        }

        bail!("invalid telegram dialog id {}", dialog_id)
    }

    async fn resolve_chat_peer(client: &Client, chat_id: &str) -> AnyResult<PeerRef> {
        let trimmed = chat_id.trim();
        if trimmed.is_empty() {
            bail!("empty telegram chat_id");
        }

        if let Some(username) = Self::username_from_chat_id(trimmed) {
            let chat = client
                .resolve_username(&username)
                .await?
                .ok_or_else(|| anyhow::anyhow!("telegram username {} not found", username))?;
            return Ok(PeerRef::from(chat));
        }

        let dialog_id = trimmed
            .parse::<i64>()
            .with_context(|| format!("invalid telegram chat_id {}", chat_id))?;
        Self::peer_ref_from_dialog_id(dialog_id)
    }

    async fn start_binding(
        &self,
        binding: &TgBotBinding,
        token: &str,
    ) -> AnyResult<(GrammersTgRuntime, UnboundedReceiver<UpdatesLike>)> {
        std::fs::create_dir_all(&self.cfg.session_dir).with_context(|| {
            format!(
                "failed to create telegram session dir {}",
                self.cfg.session_dir.display()
            )
        })?;
        let session_path = self.cfg.session_dir.join(Self::session_file_name(binding));
        let session = Arc::new(SqliteSession::open(&session_path).with_context(|| {
            format!(
                "failed to open/create telegram session {}",
                session_path.display()
            )
        })?);
        let pool = SenderPool::new(session, self.cfg.api_id);
        let client = Client::new(&pool);
        let SenderPool {
            runner,
            updates,
            handle,
        } = pool;
        let sender_pool_task = tokio::spawn(runner.run());

        if !client.is_authorized().await? {
            client
                .bot_sign_in(token, &self.cfg.api_hash)
                .await
                .with_context(|| {
                    format!(
                        "telegram bot sign-in failed for owner {} (bot account {})",
                        binding.owner_did.to_string(),
                        binding.bot_account_id
                    )
                })?;
        }

        let me = client.get_me().await?;
        info!(
            "telegram tunnel bot ready: owner={}, bot_id={}, username={:?}",
            binding.owner_did.to_string(),
            me.bare_id(),
            me.username()
        );

        Ok((
            GrammersTgRuntime {
                owner_did: binding.owner_did.clone(),
                bot_account_id: binding.bot_account_id.clone(),
                client,
                sender_pool_handle: handle,
                sender_pool_task,
            },
            updates,
        ))
    }

    async fn stop_runtime(runtime: GrammersTgRuntime) -> AnyResult<()> {
        runtime.sender_pool_handle.quit();
        if let Err(error) = runtime.sender_pool_task.await {
            warn!(
                "telegram sender pool join failed, owner={}, bot={}, error={}",
                runtime.owner_did.to_string(),
                runtime.bot_account_id,
                error
            );
        }
        Ok(())
    }
}

#[async_trait]
impl TgGateway for GrammersTgGateway {
    async fn start(&self, bindings: &[TgBotBinding]) -> AnyResult<()> {
        {
            let guard = self.runtimes.lock().await;
            if !guard.is_empty() {
                return Ok(());
            }
        }

        if bindings.is_empty() {
            warn!("grammers tg gateway started with empty bindings");
            return Ok(());
        }

        let mut started = HashMap::new();
        let mut started_tasks: HashMap<String, JoinHandle<()>> = HashMap::new();
        for binding in bindings {
            binding.validate()?;
            let token = resolve_binding_bot_token(binding)?;
            let (runtime, updates_rx) = match self.start_binding(binding, &token).await {
                Ok(runtime) => runtime,
                Err(error) => {
                    for (_, task) in started_tasks.drain() {
                        task.abort();
                    }
                    for (_, runtime) in started.drain() {
                        let _ = Self::stop_runtime(runtime).await;
                    }
                    return Err(error);
                }
            };
            let owner_key = binding.owner_did.to_string();
            let task = self.spawn_ingress_task(
                binding.owner_did.clone(),
                binding.bot_account_id.clone(),
                runtime.client.clone(),
                updates_rx,
            );
            started.insert(owner_key.clone(), runtime);
            started_tasks.insert(owner_key, task);
        }

        let mut guard = self.runtimes.lock().await;
        if !guard.is_empty() {
            for (_, task) in started_tasks.drain() {
                task.abort();
            }
            for (_, runtime) in started.drain() {
                let _ = Self::stop_runtime(runtime).await;
            }
            return Ok(());
        }

        *guard = started;
        drop(guard);

        let mut task_guard = self.ingress_tasks.lock().await;
        *task_guard = started_tasks;
        Ok(())
    }

    async fn stop(&self) -> AnyResult<()> {
        {
            let mut dispatcher = self.dispatcher.lock().await;
            *dispatcher = None;
        }
        {
            let mut tasks_guard = self.ingress_tasks.lock().await;
            for (_, task) in tasks_guard.drain() {
                task.abort();
            }
        }

        let mut guard = self.runtimes.lock().await;
        let runtimes: Vec<_> = guard.drain().map(|(_, runtime)| runtime).collect();
        drop(guard);

        for runtime in runtimes {
            let _ = Self::stop_runtime(runtime).await;
        }
        Ok(())
    }

    async fn send(&self, envelope: TgEgressEnvelope) -> AnyResult<DeliveryReportResult> {
        let sender_key = envelope.sender_did.to_string();
        let (client, runtime_bot_account_id) = {
            let guard = self.runtimes.lock().await;
            let runtime = guard.get(&sender_key).ok_or_else(|| {
                anyhow::anyhow!(
                    "no running telegram runtime for sender {}",
                    envelope.sender_did.to_string()
                )
            })?;
            (runtime.client.clone(), runtime.bot_account_id.clone())
        };

        if runtime_bot_account_id != envelope.bot_account_id {
            bail!(
                "sender {} bound bot {} mismatches envelope bot {}",
                envelope.sender_did.to_string(),
                runtime_bot_account_id,
                envelope.bot_account_id
            );
        }

        let chat_id = envelope.chat_id.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "chat_id is required for telegram send ({})",
                envelope.record_id
            )
        })?;
        let chat = Self::resolve_chat_peer(&client, chat_id).await?;
        let text = Self::resolve_text(&envelope);
        info!(
            "telegram egress about to send (gateway=grammers): sender={}, bot={}, chat_id={}, record_id={}, text_len={}",
            envelope.sender_did.to_string(),
            envelope.bot_account_id,
            chat_id,
            envelope.record_id,
            text.len()
        );

        let sent = client.send_message(chat, text).await?;
        Ok(DeliveryReportResult {
            ok: true,
            external_msg_id: Some(sent.id().to_string()),
            delivered_at_ms: Some(TgTunnel::now_ms()),
            ..Default::default()
        })
    }

    async fn set_dispatcher(&self, dispatcher: Option<Arc<dyn MsgCenterHandler>>) -> AnyResult<()> {
        {
            let mut guard = self.dispatcher.lock().await;
            *guard = dispatcher;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct BotApiTgRuntime {
    owner_did: DID,
    bot_account_id: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct TgBotApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgBotApiUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TgBotApiMessage>,
}

#[derive(Debug, Deserialize)]
struct TgBotApiMessage {
    message_id: i64,
    date: i64,
    chat: TgBotApiChat,
    #[serde(default)]
    from: Option<TgBotApiUser>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    caption: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgBotApiChat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgBotApiUser {
    id: i64,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgBotApiMe {
    id: i64,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgBotApiSentMessage {
    message_id: i64,
}

pub struct BotApiTgGateway {
    http: HttpClient,
    runtimes: Mutex<HashMap<String, BotApiTgRuntime>>,
    dispatcher: Arc<Mutex<Option<Arc<dyn MsgCenterHandler>>>>,
    ingress_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    tunnel_did: Option<DID>,
    poll_timeout_secs: u64,
}

impl BotApiTgGateway {
    pub fn new(tunnel_did: Option<DID>) -> Self {
        Self {
            http: HttpClient::new(),
            runtimes: Mutex::new(HashMap::new()),
            dispatcher: Arc::new(Mutex::new(None)),
            ingress_tasks: Mutex::new(HashMap::new()),
            tunnel_did,
            poll_timeout_secs: 20,
        }
    }

    fn api_url(token: &str, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            TG_BOT_API_ENDPOINT,
            token.trim(),
            method.trim_matches('/')
        )
    }

    async fn call_api_with_client<T: DeserializeOwned>(
        http: &HttpClient,
        token: &str,
        method: &str,
        payload: Option<Value>,
    ) -> AnyResult<T> {
        let req = http.post(Self::api_url(token, method));
        let req = if let Some(body) = payload {
            req.json(&body)
        } else {
            req
        };
        let response = req
            .send()
            .await
            .with_context(|| format!("telegram bot api {} request failed", method))?;
        let status = response.status();
        let body: TgBotApiResponse<T> = response
            .json()
            .await
            .with_context(|| format!("telegram bot api {} parse response failed", method))?;

        if !status.is_success() || !body.ok {
            let desc = body
                .description
                .unwrap_or_else(|| "unknown telegram bot api error".to_string());
            bail!("telegram bot api {} failed: {}", method, desc);
        }

        body.result
            .ok_or_else(|| anyhow::anyhow!("telegram bot api {} missing result", method))
    }

    fn normalize_chat_kind(raw: &str) -> &str {
        match raw {
            "private" => "user",
            "group" => "group",
            "supergroup" => "channel",
            "channel" => "channel",
            _ => "user",
        }
    }

    fn join_name(first_name: Option<&str>, last_name: Option<&str>) -> Option<String> {
        let first = first_name.unwrap_or("").trim();
        let last = last_name.unwrap_or("").trim();
        let joined = if first.is_empty() {
            last.to_string()
        } else if last.is_empty() {
            first.to_string()
        } else {
            format!("{} {}", first, last)
        };
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    fn chat_name(chat: &TgBotApiChat) -> Option<String> {
        chat.title
            .clone()
            .or_else(|| Self::join_name(chat.first_name.as_deref(), chat.last_name.as_deref()))
    }

    fn user_name(user: &TgBotApiUser) -> Option<String> {
        Self::join_name(user.first_name.as_deref(), user.last_name.as_deref())
    }

    fn chat_account_id(chat_kind: &str, id: i64) -> String {
        format!("{}:{}", chat_kind, id)
    }

    fn profile_hint(
        chat_kind: &str,
        chat_id: i64,
        display_name: Option<&str>,
        username: Option<&str>,
        bot_account_id: &str,
        tunnel_did: Option<&DID>,
    ) -> Value {
        let display_id = username
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("@{}", value))
            .unwrap_or_else(|| chat_id.to_string());
        json!({
            "chat_type": chat_kind,
            "chat_id": chat_id,
            "name": display_name,
            "username": username,
            "display_id": display_id,
            "bot_account_id": bot_account_id,
            "tunnel_id": tunnel_did.map(|did| did.to_string()).unwrap_or_default(),
        })
    }

    fn text_from_message(message: &TgBotApiMessage) -> String {
        message
            .text
            .as_ref()
            .or(message.caption.as_ref())
            .map(|value| value.to_string())
            .unwrap_or_default()
    }

    async fn dispatch_incoming_message(
        dispatcher: Arc<dyn MsgCenterHandler>,
        owner_did: DID,
        bot_account_id: String,
        tunnel_did: Option<DID>,
        message: TgBotApiMessage,
    ) -> AnyResult<()> {
        let chat_kind = Self::normalize_chat_kind(message.chat.kind.as_str());
        let chat_id = message.chat.id;
        let sender_kind;
        let sender_chat_id;
        let sender_name;
        let sender_username;
        if let Some(user) = message.from.as_ref() {
            sender_kind = "user";
            sender_chat_id = user.id;
            sender_name = Self::user_name(user);
            sender_username = user.username.clone();
        } else {
            sender_kind = chat_kind;
            sender_chat_id = chat_id;
            sender_name = Self::chat_name(&message.chat);
            sender_username = message.chat.username.clone();
        }
        let sender_account_id = Self::chat_account_id(sender_kind, sender_chat_id);
        let sender_did = dispatcher
            .handle_resolve_did(
                TELEGRAM_PLATFORM.to_string(),
                sender_account_id.clone(),
                Some(Self::profile_hint(
                    sender_kind,
                    sender_chat_id,
                    sender_name.as_deref(),
                    sender_username.as_deref(),
                    &bot_account_id,
                    tunnel_did.as_ref(),
                )),
                Some(owner_did.clone()),
                RPCContext::default(),
            )
            .await
            .map_err(|error| {
                warn!(
                    "telegram ingress resolve sender did failed (gateway=bot_api): owner={}, bot={}, message_id={}, error={}",
                    owner_did.to_string(),
                    bot_account_id,
                    message.message_id,
                    error
                );
                error
            })?;

        let message_id_i32 = i32::try_from(message.message_id).unwrap_or(i32::MAX);
        let idempotency_key =
            TgMessageConverter::build_dispatch_key(&bot_account_id, chat_id, message_id_i32);
        let msg_obj_id = TgMessageConverter::build_msg_obj_id(&idempotency_key)?;
        let text = Self::text_from_message(&message);
        let payload_kind = if text.trim().is_empty() {
            "telegram_event"
        } else {
            "text"
        };
        let created_at_ms = (message.date.max(0) as u64).saturating_mul(1000);
        let msg = MsgObject {
            id: msg_obj_id,
            from: sender_did,
            source: None,
            to: vec![owner_did.clone()],
            thread_key: Some(format!("tg:{}:{}", bot_account_id, chat_id)),
            payload: json!({
                "kind": payload_kind,
                "text": text,
                "telegram": {
                    "message_id": message.message_id,
                    "chat_id": chat_id,
                    "chat_type": chat_kind,
                    "chat_name": Self::chat_name(&message.chat),
                }
            }),
            meta: Some(json!({
                "telegram": {
                    "chat_dialog_id": chat_id,
                    "chat_username": message.chat.username,
                    "sender_id": sender_chat_id,
                    "sender_username": sender_username,
                    "sender_name": sender_name,
                    "bot_account_id": bot_account_id,
                }
            })),
            created_at_ms,
        };

        let ingress_ctx = IngressContext {
            tunnel_did,
            platform: Some(TELEGRAM_PLATFORM.to_string()),
            chat_id: Some(chat_id.to_string()),
            source_account_id: Some(sender_account_id),
            context_id: Some(format!("tg:{}:{}", owner_did.to_string(), chat_id)),
            extra: Some(json!({
                "tg_message_id": message.message_id,
                "chat_type": chat_kind,
            })),
        };
        info!(
            "telegram ingress message received (gateway=bot_api): owner={}, bot={}, chat_id={}, message_id={}",
            owner_did.to_string(),
            bot_account_id,
            chat_id,
            message.message_id
        );

        let dispatch_result = dispatcher
            .handle_dispatch(
                msg,
                Some(ingress_ctx),
                Some(idempotency_key),
                RPCContext::default(),
            )
            .await
            .map_err(|error| {
                warn!(
                    "telegram ingress handle_dispatch failed (gateway=bot_api): owner={}, bot={}, message_id={}, error={}",
                    owner_did.to_string(),
                    bot_account_id,
                    message.message_id,
                    error
                );
                error
            })?;

        if !dispatch_result.ok {
            warn!(
                "telegram ingress dispatch result not ok (gateway=bot_api): owner={}, bot={}, message_id={}, reason={}",
                owner_did.to_string(),
                bot_account_id,
                message.message_id,
                dispatch_result.reason.as_deref().unwrap_or("-"),
            );
        }
        if !dispatch_result.dropped_recipients.is_empty() {
            warn!(
                "telegram ingress dropped recipients (gateway=bot_api): owner={}, bot={}, message_id={}, dropped={}",
                owner_did.to_string(),
                bot_account_id,
                message.message_id,
                dispatch_result
                    .dropped_recipients
                    .iter()
                    .map(|did| did.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        let delivered_count = dispatch_result.delivered_recipients.len()
            + dispatch_result.delivered_agents.len()
            + usize::from(dispatch_result.delivered_group.is_some());
        if delivered_count == 0 {
            warn!(
                "telegram ingress dispatched but nothing delivered (gateway=bot_api): owner={}, bot={}, message_id={}, reason={}",
                owner_did.to_string(),
                bot_account_id,
                message.message_id,
                dispatch_result.reason.as_deref().unwrap_or("-"),
            );
        }
        Ok(())
    }

    fn normalize_chat_id_for_send(chat_id: &str) -> Value {
        let trimmed = chat_id.trim();
        if let Ok(value) = trimmed.parse::<i64>() {
            return json!(value);
        }
        if trimmed.starts_with('@') {
            return json!(trimmed);
        }
        if let Some(username) = GrammersTgGateway::username_from_chat_id(trimmed) {
            return json!(format!("@{}", username));
        }
        json!(trimmed)
    }

    fn spawn_ingress_task(&self, runtime: BotApiTgRuntime) -> JoinHandle<()> {
        let http = self.http.clone();
        let dispatcher = self.dispatcher.clone();
        let tunnel_did = self.tunnel_did.clone();
        let poll_timeout_secs = self.poll_timeout_secs;
        tokio::spawn(async move {
            let mut offset = 0_i64;
            loop {
                let updates = Self::call_api_with_client::<Vec<TgBotApiUpdate>>(
                    &http,
                    &runtime.token,
                    "getUpdates",
                    Some(json!({
                        "offset": offset,
                        "timeout": poll_timeout_secs,
                        "allowed_updates": ["message"]
                    })),
                )
                .await;

                let updates = match updates {
                    Ok(updates) => updates,
                    Err(error) => {
                        warn!(
                            "telegram bot api updates failed, owner={}, bot={}, error={}",
                            runtime.owner_did.to_string(),
                            runtime.bot_account_id,
                            error
                        );
                        tokio::time::sleep(Duration::from_millis(1000)).await;
                        continue;
                    }
                };

                for update in updates {
                    offset = offset.max(update.update_id.saturating_add(1));
                    let Some(message) = update.message else {
                        continue;
                    };
                    let dispatcher = {
                        let guard = dispatcher.lock().await;
                        guard.clone()
                    };
                    let Some(dispatcher) = dispatcher else {
                        warn!(
                            "telegram ingress dispatcher missing, dropping message (gateway=bot_api): owner={}, bot={}, message_id={}",
                            runtime.owner_did.to_string(),
                            runtime.bot_account_id,
                            message.message_id,
                        );
                        continue;
                    };
                    if let Err(error) = Self::dispatch_incoming_message(
                        dispatcher,
                        runtime.owner_did.clone(),
                        runtime.bot_account_id.clone(),
                        tunnel_did.clone(),
                        message,
                    )
                    .await
                    {
                        warn!(
                            "telegram bot api ingress dispatch failed, owner={}, bot={}, error={}",
                            runtime.owner_did.to_string(),
                            runtime.bot_account_id,
                            error
                        );
                    }
                }
            }
        })
    }
}

#[async_trait]
impl TgGateway for BotApiTgGateway {
    async fn start(&self, bindings: &[TgBotBinding]) -> AnyResult<()> {
        {
            let guard = self.runtimes.lock().await;
            if !guard.is_empty() {
                return Ok(());
            }
        }
        if bindings.is_empty() {
            warn!("bot-api tg gateway started with empty bindings");
            return Ok(());
        }

        let mut started = HashMap::<String, BotApiTgRuntime>::new();
        let mut started_tasks = HashMap::<String, JoinHandle<()>>::new();
        for binding in bindings {
            binding.validate()?;
            let token = resolve_binding_bot_token(binding)?;
            let me = Self::call_api_with_client::<TgBotApiMe>(&self.http, &token, "getMe", None)
                .await
                .with_context(|| {
                    format!(
                        "telegram bot api getMe failed for owner {} (bot account {})",
                        binding.owner_did.to_string(),
                        binding.bot_account_id
                    )
                })?;
            info!(
                "telegram bot api ready: owner={}, bot_id={}, username={:?}",
                binding.owner_did.to_string(),
                me.id,
                me.username
            );

            let runtime = BotApiTgRuntime {
                owner_did: binding.owner_did.clone(),
                bot_account_id: binding.bot_account_id.clone(),
                token,
            };
            let owner_key = binding.owner_did.to_string();
            let task = self.spawn_ingress_task(runtime.clone());
            started.insert(owner_key.clone(), runtime);
            started_tasks.insert(owner_key, task);
        }

        let mut guard = self.runtimes.lock().await;
        if !guard.is_empty() {
            for (_, task) in started_tasks.drain() {
                task.abort();
            }
            return Ok(());
        }
        *guard = started;
        drop(guard);

        let mut tasks_guard = self.ingress_tasks.lock().await;
        *tasks_guard = started_tasks;
        Ok(())
    }

    async fn stop(&self) -> AnyResult<()> {
        {
            let mut dispatcher = self.dispatcher.lock().await;
            *dispatcher = None;
        }
        {
            let mut tasks_guard = self.ingress_tasks.lock().await;
            for (_, task) in tasks_guard.drain() {
                task.abort();
            }
        }
        let mut guard = self.runtimes.lock().await;
        guard.clear();
        Ok(())
    }

    async fn send(&self, envelope: TgEgressEnvelope) -> AnyResult<DeliveryReportResult> {
        let sender_key = envelope.sender_did.to_string();
        let runtime = {
            let guard = self.runtimes.lock().await;
            guard.get(&sender_key).cloned().ok_or_else(|| {
                anyhow::anyhow!("no running telegram runtime for sender {}", sender_key)
            })?
        };
        if runtime.bot_account_id != envelope.bot_account_id {
            bail!(
                "sender {} bound bot {} mismatches envelope bot {}",
                sender_key,
                runtime.bot_account_id,
                envelope.bot_account_id
            );
        }

        let chat_id = envelope.chat_id.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "chat_id is required for telegram send ({})",
                envelope.record_id
            )
        })?;
        let text = GrammersTgGateway::resolve_text(&envelope);
        info!(
            "telegram egress about to send (gateway=bot_api): sender={}, bot={}, chat_id={}, record_id={}, text_len={}",
            sender_key,
            envelope.bot_account_id,
            chat_id,
            envelope.record_id,
            text.len()
        );
        let sent = Self::call_api_with_client::<TgBotApiSentMessage>(
            &self.http,
            runtime.token.as_str(),
            "sendMessage",
            Some(json!({
                "chat_id": Self::normalize_chat_id_for_send(chat_id),
                "text": text,
            })),
        )
        .await?;

        Ok(DeliveryReportResult {
            ok: true,
            external_msg_id: Some(sent.message_id.to_string()),
            delivered_at_ms: Some(TgTunnel::now_ms()),
            ..Default::default()
        })
    }

    async fn set_dispatcher(&self, dispatcher: Option<Arc<dyn MsgCenterHandler>>) -> AnyResult<()> {
        let mut guard = self.dispatcher.lock().await;
        *guard = dispatcher;
        Ok(())
    }
}

pub struct TgTunnel {
    cfg: TgTunnelConfig,
    running: AtomicBool,
    bindings: Arc<RwLock<HashMap<String, TgBotBinding>>>,
    dispatcher: Arc<RwLock<Option<Arc<dyn MsgCenterHandler>>>>,
    gateway: Arc<dyn TgGateway>,
}

impl TgTunnel {
    pub fn new(cfg: TgTunnelConfig) -> Self {
        Self::with_gateway(cfg, Arc::new(DryRunTgGateway::default()))
    }

    pub fn with_grammers_gateway(
        cfg: TgTunnelConfig,
        grammers_cfg: GrammersTgGatewayConfig,
    ) -> Self {
        let mut grammers_cfg = grammers_cfg;
        if grammers_cfg.tunnel_did.is_none() {
            grammers_cfg.tunnel_did = Some(cfg.tunnel_did.clone());
        }
        Self::with_gateway(cfg, Arc::new(GrammersTgGateway::new(grammers_cfg)))
    }

    pub fn with_grammers_gateway_from_env(cfg: TgTunnelConfig) -> AnyResult<Self> {
        Ok(Self::with_grammers_gateway(
            cfg,
            GrammersTgGatewayConfig::from_env()?,
        ))
    }

    pub fn with_bot_api_gateway(cfg: TgTunnelConfig) -> Self {
        let tunnel_did = Some(cfg.tunnel_did.clone());
        Self::with_gateway(cfg, Arc::new(BotApiTgGateway::new(tunnel_did)))
    }

    pub fn with_gateway(cfg: TgTunnelConfig, gateway: Arc<dyn TgGateway>) -> Self {
        Self {
            cfg,
            running: AtomicBool::new(false),
            bindings: Arc::new(RwLock::new(HashMap::new())),
            dispatcher: Arc::new(RwLock::new(None)),
            gateway,
        }
    }

    pub fn bind_msg_center_handler(&self, handler: Arc<dyn MsgCenterHandler>) -> AnyResult<()> {
        let mut guard = self
            .dispatcher
            .write()
            .map_err(|_| anyhow::anyhow!("tg dispatcher lock poisoned"))?;
        *guard = Some(handler);
        Ok(())
    }

    pub fn clear_msg_center_handler(&self) -> AnyResult<()> {
        let mut guard = self
            .dispatcher
            .write()
            .map_err(|_| anyhow::anyhow!("tg dispatcher lock poisoned"))?;
        *guard = None;
        Ok(())
    }

    fn get_msg_center_handler(&self) -> AnyResult<Option<Arc<dyn MsgCenterHandler>>> {
        let guard = self
            .dispatcher
            .read()
            .map_err(|_| anyhow::anyhow!("tg dispatcher lock poisoned"))?;
        Ok(guard.clone())
    }

    pub fn bind_bot(&self, binding: TgBotBinding) -> AnyResult<()> {
        self.ensure_not_running("bind_bot")?;
        binding.validate()?;

        let mut guard = self
            .bindings
            .write()
            .map_err(|_| anyhow::anyhow!("tg binding lock poisoned"))?;
        guard.insert(binding.owner_did.to_string(), binding);
        Ok(())
    }

    pub fn bind_bot_simple(
        &self,
        owner_did: DID,
        bot_account_id: String,
        bot_token_env_key: Option<String>,
        default_chat_id: Option<String>,
    ) -> AnyResult<()> {
        self.bind_bot(TgBotBinding {
            owner_did,
            bot_account_id,
            bot_token_env_key,
            default_chat_id,
            extra: HashMap::new(),
        })
    }

    pub fn unbind_bot(&self, owner_did: &DID) -> AnyResult<Option<TgBotBinding>> {
        self.ensure_not_running("unbind_bot")?;

        let mut guard = self
            .bindings
            .write()
            .map_err(|_| anyhow::anyhow!("tg binding lock poisoned"))?;
        Ok(guard.remove(&owner_did.to_string()))
    }

    pub fn get_binding(&self, owner_did: &DID) -> AnyResult<Option<TgBotBinding>> {
        let guard = self
            .bindings
            .read()
            .map_err(|_| anyhow::anyhow!("tg binding lock poisoned"))?;
        Ok(guard.get(&owner_did.to_string()).cloned())
    }

    pub fn list_bindings(&self) -> AnyResult<Vec<TgBotBinding>> {
        let guard = self
            .bindings
            .read()
            .map_err(|_| anyhow::anyhow!("tg binding lock poisoned"))?;
        let mut result: Vec<_> = guard.values().cloned().collect();
        result.sort_by(|left, right| left.owner_did.to_string().cmp(&right.owner_did.to_string()));
        Ok(result)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn ensure_not_running(&self, op: &str) -> AnyResult<()> {
        if self.is_running() {
            bail!("cannot {} when tg tunnel is running", op);
        }
        Ok(())
    }

    fn resolve_sender_did(record: &MsgRecordWithObject) -> DID {
        record
            .msg
            .source
            .clone()
            .unwrap_or_else(|| record.msg.from.clone())
    }

    fn parse_chat_id_from_route_extra(extra: &Value) -> Option<String> {
        let raw = extra
            .pointer("/route/chat_id")
            .or_else(|| extra.get("chat_id"))
            .and_then(Self::json_value_to_chat_id)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_string())
    }

    fn parse_chat_id_from_account_id(account_id: &str) -> Option<String> {
        let trimmed = account_id.trim();
        if trimmed.is_empty() {
            return None;
        }

        if Self::is_telegram_chat_id(trimmed) {
            return Some(trimmed.to_string());
        }

        if let Some((_, tail)) = trimmed.split_once(':') {
            let candidate = tail.trim();
            if !candidate.is_empty() && Self::is_telegram_chat_id(candidate) {
                return Some(candidate.to_string());
            }
        }
        Self::parse_numeric_chat_id_tail(trimmed)
    }

    fn json_value_to_chat_id(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => Some(text.to_string()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        }
    }

    fn is_telegram_chat_id(value: &str) -> bool {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return false;
        }
        if GrammersTgGateway::username_from_chat_id(trimmed).is_some() {
            return true;
        }
        trimmed.parse::<i64>().is_ok()
    }

    fn parse_numeric_chat_id_tail(value: &str) -> Option<String> {
        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return None;
        }

        let mut end = bytes.len();
        while end > 0 && !bytes[end - 1].is_ascii_digit() {
            end = end.saturating_sub(1);
        }
        if end == 0 {
            return None;
        }

        let mut start = end;
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start = start.saturating_sub(1);
        }
        if start > 0 && bytes[start - 1] == b'-' {
            start = start.saturating_sub(1);
        }

        let candidate = value[start..end].trim();
        if candidate.is_empty() {
            return None;
        }
        if candidate.parse::<i64>().is_ok() {
            Some(candidate.to_string())
        } else {
            None
        }
    }

    fn build_egress_envelope(
        &self,
        record: &MsgRecordWithObject,
        binding: &TgBotBinding,
    ) -> TgEgressEnvelope {
        let sender_did = Self::resolve_sender_did(record);
        let route = record.record.route.as_ref();
        let chat_id = route
            .and_then(|route| route.chat_id.clone())
            .or_else(|| {
                route
                    .and_then(|route| route.extra.as_ref())
                    .and_then(Self::parse_chat_id_from_route_extra)
            })
            .or_else(|| {
                route
                    .and_then(|route| route.account_id.as_deref())
                    .and_then(Self::parse_chat_id_from_account_id)
            })
            .or_else(|| binding.default_chat_id.clone());

        let (text, payload) = TgMessageConverter::msg_object_to_tg_content(&record.msg);

        // TODO: 用 grammers 的消息模型做完整转换（媒体、回复链、实体、按钮等）
        TgEgressEnvelope {
            sender_did,
            bot_account_id: binding.bot_account_id.clone(),
            chat_id,
            text,
            payload,
            record_id: record.record.record_id.clone(),
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[async_trait]
impl MsgTunnel for TgTunnel {
    fn tunnel_did(&self) -> DID {
        self.cfg.tunnel_did.clone()
    }

    fn name(&self) -> &str {
        &self.cfg.name
    }

    fn platform(&self) -> &str {
        TELEGRAM_PLATFORM
    }

    fn supports_ingress(&self) -> bool {
        self.cfg.supports_ingress
    }

    fn supports_egress(&self) -> bool {
        self.cfg.supports_egress
    }

    async fn start(&self) -> AnyResult<()> {
        if self.is_running() {
            return Ok(());
        }

        let dispatcher = self.get_msg_center_handler()?;
        if self.cfg.supports_ingress && dispatcher.is_none() {
            bail!("msg_center handler is required before starting ingress-enabled tg tunnel");
        }
        self.gateway.set_dispatcher(dispatcher).await?;

        let bindings = self.list_bindings()?;
        self.gateway.start(&bindings).await?;
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&self) -> AnyResult<()> {
        if !self.is_running() {
            return Ok(());
        }

        self.gateway.set_dispatcher(None).await?;
        self.gateway.stop().await?;
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send_record(&self, record: MsgRecordWithObject) -> AnyResult<DeliveryReportResult> {
        if !self.cfg.supports_egress {
            bail!(
                "tg tunnel {} does not support egress",
                self.cfg.tunnel_did.to_string()
            );
        }

        if !self.is_running() {
            bail!(
                "tg tunnel {} is not running",
                self.cfg.tunnel_did.to_string()
            );
        }

        let sender_did = Self::resolve_sender_did(&record);
        let binding = self.get_binding(&sender_did)?.ok_or_else(|| {
            anyhow::anyhow!("missing tg bot binding for {}", sender_did.to_string())
        })?;

        let envelope = self.build_egress_envelope(&record, &binding);
        self.gateway.send(envelope).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{BoxKind, MsgObject, MsgRecord, MsgState, RouteInfo};
    use ndn_lib::ObjId;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    fn next_msg_id() -> ObjId {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        ObjId::new(&format!("chunk:{:016x}", seq)).unwrap()
    }

    fn new_tunnel() -> TgTunnel {
        let mut cfg = TgTunnelConfig::new(DID::new("bns", "tg-test"));
        cfg.supports_ingress = false;
        TgTunnel::new(cfg)
    }

    fn build_record(
        from: DID,
        source: Option<DID>,
        route_chat_id: Option<&str>,
    ) -> MsgRecordWithObject {
        let msg = MsgObject {
            id: next_msg_id(),
            from,
            source,
            to: vec![DID::new("bns", "receiver")],
            thread_key: Some("thread-a".to_string()),
            payload: json!({ "kind": "text", "text": "hello" }),
            meta: None,
            created_at_ms: 1,
        };

        let record = MsgRecord {
            record_id: format!("record-{}", msg.id.to_string()),
            owner: DID::new("bns", "tunnel-owner"),
            box_kind: BoxKind::TunnelOutbox,
            msg_id: msg.id.clone(),
            state: MsgState::Wait,
            created_at_ms: 1,
            updated_at_ms: 1,
            route: Some(RouteInfo {
                chat_id: route_chat_id.map(|value| value.to_string()),
                ..Default::default()
            }),
            delivery: None,
            thread_key: msg.thread_key.clone(),
            sort_key: 1,
            tags: Vec::new(),
        };

        MsgRecordWithObject { record, msg }
    }

    #[tokio::test]
    async fn binding_can_only_be_changed_before_start() {
        let tunnel = new_tunnel();
        let owner = DID::new("bns", "alice");

        tunnel
            .bind_bot_simple(
                owner.clone(),
                "@alice_bot".to_string(),
                Some("ALICE_BOT_TOKEN".to_string()),
                Some("10001".to_string()),
            )
            .unwrap();
        assert!(tunnel.get_binding(&owner).unwrap().is_some());

        tunnel.start().await.unwrap();
        let bind_err = tunnel
            .bind_bot_simple(DID::new("bns", "bob"), "@bob_bot".to_string(), None, None)
            .unwrap_err();
        assert!(bind_err
            .to_string()
            .contains("cannot bind_bot when tg tunnel is running"));

        tunnel.stop().await.unwrap();
        let removed = tunnel.unbind_bot(&owner).unwrap();
        assert!(removed.is_some());
    }

    #[tokio::test]
    async fn send_uses_source_did_for_group_message() {
        let tunnel = new_tunnel();
        let agent_did = DID::new("bns", "agent-a");

        tunnel
            .bind_bot_simple(
                agent_did.clone(),
                "@agent_bot".to_string(),
                Some("AGENT_BOT_TOKEN".to_string()),
                Some("fallback-chat".to_string()),
            )
            .unwrap();

        tunnel.start().await.unwrap();

        let record = build_record(
            DID::new("bns", "group-a"),
            Some(agent_did),
            Some("route-chat-1"),
        );
        let report = tunnel.send_record(record).await.unwrap();

        assert!(report.ok);
        assert!(report.external_msg_id.is_some());
        assert!(report.delivered_at_ms.is_some());

        tunnel.stop().await.unwrap();
    }

    #[tokio::test]
    async fn send_fails_when_sender_binding_is_missing() {
        let tunnel = new_tunnel();
        let sender = DID::new("bns", "no-binding");
        let record = build_record(sender, None, None);

        tunnel.start().await.unwrap();
        let err = tunnel.send_record(record).await.unwrap_err();
        assert!(err
            .to_string()
            .contains("missing tg bot binding for did:bns:no-binding"));
        tunnel.stop().await.unwrap();
    }

    // 测试方式：
    // 1. 填入 TG_TEST_BOT_TOKEN 常量
    // 2. 设置 BUCKYOS_TG_API_ID / BUCKYOS_TG_API_HASH（环境变量）
    // 3. 运行: cargo test -p msg_center tg_tunnel::tests::grammers_gateway_can_sign_in_with_constant_token -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires valid telegram credentials and network"]
    async fn grammers_gateway_can_sign_in_with_constant_token() {
        const TG_TEST_BOT_TOKEN: &str = "";
        if TG_TEST_BOT_TOKEN.trim().is_empty() {
            panic!("set TG_TEST_BOT_TOKEN constant before running this test");
        }

        let cfg = GrammersTgGatewayConfig::from_env().unwrap();
        let gateway = GrammersTgGateway::new(cfg);

        let mut extra = HashMap::new();
        extra.insert(
            TG_BINDING_EXTRA_BOT_TOKEN.to_string(),
            TG_TEST_BOT_TOKEN.to_string(),
        );
        let binding = TgBotBinding {
            owner_did: DID::new("bns", "tg-live-test-owner"),
            bot_account_id: "@tg_live_test_bot".to_string(),
            bot_token_env_key: None,
            default_chat_id: None,
            extra,
        };

        gateway.start(std::slice::from_ref(&binding)).await.unwrap();
        let send_err = gateway
            .send(TgEgressEnvelope {
                sender_did: binding.owner_did.clone(),
                bot_account_id: binding.bot_account_id.clone(),
                chat_id: None,
                text: Some("health-check".to_string()),
                payload: json!({}),
                record_id: "rt-live-check".to_string(),
            })
            .await
            .unwrap_err();
        assert!(send_err.to_string().contains("chat_id is required"));
        gateway.stop().await.unwrap();
    }
}
