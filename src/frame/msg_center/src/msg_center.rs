use crate::contact_mgr::{ContactMgr, ZoneUserContactSeed};
use crate::group_mgr::GroupMgr;
use crate::msg_box_db::MsgBoxDbMgr;
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, AccessDecision, AccessGroupLevel, AccountBinding, Contact,
    ContactPatch, ContactQuery, DeliveryEnvelope, DeliveryError, DeliveryRecord,
    DeliveryRecordWithObject, DeliveryReportResult, DeliverySnapshot, DeliveryState,
    DispatchResult, GrantTemporaryAccessResult, GroupAccessDecision, GroupApproveMemberReq,
    GroupCheckAccessReq, GroupCreateReq, GroupCreateSubgroupReq, GroupDoc, GroupExpandMembersReq,
    GroupExpansionSnapshot, GroupGetDocReq, GroupInviteMemberReq, GroupListByMemberReq,
    GroupListMembersReq, GroupListParentsReq, GroupListSubgroupsReq, GroupMemberRecord,
    GroupRejectMemberReq, GroupRemoveMemberReq, GroupRequestJoinReq, GroupSubgroup,
    GroupSubmitMemberProofReq, GroupSummary, GroupUpdateAttributionPolicyReq,
    GroupUpdateCollectionPolicyReq, GroupUpdateMemberRoleReq, GroupUpdateProfileReq,
    GroupUpdateSubgroupReq, ImportContactEntry, ImportReport, IngressContext, KEventClient,
    MailboxKind, MailboxRecord, MailboxRecordPage, MailboxRecordWithObject, MsgCenterHandler,
    MsgReceiptObj, PostSendDelivery, PostSendResult, ReadReceiptState, RecipientState,
    SessionDeliveryOverall, SessionDeliveryTarget, SessionDeliveryView, SessionMessageDirection,
    SessionMessageItem, SessionMessagePage, SessionSummary, SessionSummaryPage,
    SetGroupSubscribersResult, TransportKind, UiSessionStateEntry,
};
use kRPC::{RPCContext, RPCErrors};
use log::{info, warn};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, NamedObject, ObjId};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

const DEFAULT_PEEK_LIMIT: usize = 20;
const MAX_PEEK_LIMIT: usize = 200;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;
const DEFAULT_SESSION_LIST_LIMIT: usize = 50;
const MAX_SESSION_LIST_LIMIT: usize = 200;
const DEFAULT_READ_RECEIPT_LIMIT: usize = 100;
const MAX_READ_RECEIPT_LIMIT: usize = 1000;
const MAX_DELIVERY_RETRY: u32 = 5;
/// SENDING rows older than this are reclaimed by the sweep (executor crash).
const DELIVERY_SENDING_LEASE_MS: u64 = 60_000;
const DELIVERY_RETRY_BASE_MS: u64 = 2_000;
const DELIVERY_RETRY_MAX_MS: u64 = 300_000;
const MSG_CENTER_BOX_CHANGED_EVENT_NAME: &str = "changed";

#[derive(Debug, Default)]
struct MessageCenterState {
    messages: HashMap<String, MsgObject>,
    receipts: HashMap<String, MsgReceiptObj>,
    dispatch_idempotency: HashMap<String, DispatchResult>,
    post_send_idempotency: HashMap<String, PostSendResult>,
}

/// Registry entry for a registered message tunnel: maps the stable
/// `tunnel_instance_id` (embedded in shadow endpoint DIDs) to the tunnel's
/// `transport_did` (the DELIVERY_QUEUE owner) and its platform.
#[derive(Clone, Debug)]
struct TunnelRegistryEntry {
    transport_did: DID,
    platform: String,
}

#[derive(Clone, Debug)]
pub struct MessageCenter {
    state: Arc<RwLock<MessageCenterState>>,
    contact_mgr: ContactMgr,
    group_mgr: GroupMgr,
    msg_box_db: MsgBoxDbMgr,
    /// tunnel_instance_id -> (transport_did, platform).
    tunnel_registry: Arc<RwLock<HashMap<String, TunnelRegistryEntry>>>,
    /// DIDs hosted by this zone (zone users / agents / hosted groups): the
    /// message hub delivers to them natively via local dispatch.
    local_recipients: Arc<RwLock<HashSet<String>>>,
    /// Transport DID of the MessageHub executor. Unset means shareable-DID
    /// targets cannot be planned (post_send fails them with a clear reason
    /// instead of parking records in a queue nobody consumes).
    message_hub_did: Arc<OnceLock<DID>>,
}

impl MessageCenter {
    /// Resolve the msg-center rdb instance from the service spec and build a
    /// MessageCenter. Both `ContactMgr` and the msg-box share the same pool.
    pub async fn open_from_service_spec() -> std::result::Result<Self, RPCErrors> {
        let msg_box_db = MsgBoxDbMgr::open_from_service_spec().await?;
        Self::open_with_db(msg_box_db).await
    }

    /// Build a MessageCenter that reuses an already-opened `MsgBoxDbMgr`.
    pub async fn open_with_db(msg_box_db: MsgBoxDbMgr) -> std::result::Result<Self, RPCErrors> {
        let contact_mgr = ContactMgr::new_with_msg_box(msg_box_db.clone()).await?;
        let group_mgr = GroupMgr::new_with_msg_box(msg_box_db.clone());
        Ok(Self {
            state: Arc::new(RwLock::new(MessageCenterState::default())),
            contact_mgr,
            group_mgr,
            msg_box_db,
            tunnel_registry: Arc::new(RwLock::new(HashMap::new())),
            local_recipients: Arc::new(RwLock::new(HashSet::new())),
            message_hub_did: Arc::new(OnceLock::new()),
        })
    }

    /// Register a message-tunnel route so `post_send` can map a shadow
    /// endpoint DID's `tunnel_instance_id` back to the tunnel's transport DID.
    /// Duplicate instance ids are a configuration error and must fail loudly
    /// (never silently overwrite): shadow DID stability depends on it.
    pub fn register_tunnel(
        &self,
        tunnel_instance_id: String,
        transport_did: DID,
        platform: String,
    ) -> std::result::Result<(), RPCErrors> {
        let tunnel_instance_id = tunnel_instance_id.trim().to_string();
        if tunnel_instance_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "tunnel_instance_id cannot be empty".to_string(),
            ));
        }
        let mut registry = self.tunnel_registry.write().unwrap();
        if registry.contains_key(&tunnel_instance_id) {
            return Err(RPCErrors::ReasonError(format!(
                "tunnel_instance_id '{}' is already registered; duplicate tunnel instance ids are forbidden",
                tunnel_instance_id
            )));
        }
        registry.insert(
            tunnel_instance_id,
            TunnelRegistryEntry {
                transport_did,
                platform,
            },
        );
        Ok(())
    }

    /// Drop all tunnel routes (settings reload rebuilds the registry).
    pub fn clear_tunnel_registry(&self) {
        self.tunnel_registry.write().unwrap().clear();
    }

    fn lookup_tunnel_route(&self, tunnel_instance_id: &str) -> Option<TunnelRegistryEntry> {
        self.tunnel_registry
            .read()
            .unwrap()
            .get(tunnel_instance_id)
            .cloned()
    }

    /// Install the MessageHub transport DID (start-up wiring).
    pub fn set_message_hub_did(&self, transport_did: DID) {
        let _ = self.message_hub_did.set(transport_did);
    }

    pub fn message_hub_did(&self) -> Option<DID> {
        self.message_hub_did.get().cloned()
    }

    /// Mark DIDs as hosted by this zone (zone users, agents, hosted groups).
    pub fn register_local_recipients<I: IntoIterator<Item = DID>>(&self, dids: I) {
        let mut guard = self.local_recipients.write().unwrap();
        for did in dids {
            guard.insert(did.to_string());
        }
    }

    /// Is this DID hosted by this zone? True for explicitly registered
    /// recipients and for DIDs under the zone host (`jarvis.<zone>`,
    /// `telegram.<zone>` aliases, the zone DID itself).
    pub fn is_local_recipient(&self, did: &DID) -> bool {
        if self
            .local_recipients
            .read()
            .unwrap()
            .contains(&did.to_string())
        {
            return true;
        }
        let Ok(runtime) = get_buckyos_api_runtime() else {
            return false;
        };
        let zone = &runtime.zone_id;
        if zone == did {
            return true;
        }
        let zone_host = zone.to_host_name();
        if zone_host.is_empty() {
            return false;
        }
        let target_host = did.to_host_name();
        target_host == zone_host || target_host.ends_with(&format!(".{}", zone_host))
    }

    /// Read-only accessor for the group manager. Used by tests and by the
    /// in-process `MsgCenterClient` adapter so callers do not need to lift
    /// the GroupMgr through every API surface.
    #[allow(dead_code)]
    pub fn group_mgr(&self) -> &GroupMgr {
        &self.group_mgr
    }

    pub async fn upsert_zone_user_contacts(
        &self,
        contacts: Vec<ZoneUserContactSeed>,
        owner: Option<DID>,
    ) -> std::result::Result<usize, RPCErrors> {
        // Zone users are native local recipients of the message hub.
        self.register_local_recipients(contacts.iter().map(|seed| seed.did.clone()));
        self.contact_mgr
            .upsert_zone_user_contacts(contacts, owner)
            .await
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn with_state_read<T, F>(&self, f: F) -> std::result::Result<T, RPCErrors>
    where
        F: FnOnce(&MessageCenterState) -> std::result::Result<T, RPCErrors>,
    {
        let guard = self
            .state
            .read()
            .map_err(|_| RPCErrors::ReasonError("message center read lock poisoned".to_string()))?;
        f(&guard)
    }

    fn with_state_write<T, F>(&self, f: F) -> std::result::Result<T, RPCErrors>
    where
        F: FnOnce(&mut MessageCenterState) -> std::result::Result<T, RPCErrors>,
    {
        let mut guard = self.state.write().map_err(|_| {
            RPCErrors::ReasonError("message center write lock poisoned".to_string())
        })?;
        f(&mut guard)
    }

    fn sanitize_token(raw: &str) -> String {
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
            trimmed.chars().take(80).collect()
        }
    }

    fn box_kind_name(box_kind: &MailboxKind) -> &'static str {
        match box_kind {
            MailboxKind::Inbox => "INBOX",
            MailboxKind::Sent => "SENT",
            MailboxKind::GroupInbox => "GROUP_INBOX",
            MailboxKind::RequestBox => "REQUEST_BOX",
        }
    }

    fn box_id_prefix(box_kind: &MailboxKind) -> &'static str {
        match box_kind {
            MailboxKind::Inbox => "box_in",
            MailboxKind::Sent => "box_sent",
            MailboxKind::GroupInbox => "box_group_in",
            MailboxKind::RequestBox => "box_request",
        }
    }

    fn kevent_source_node() -> String {
        match get_buckyos_api_runtime() {
            Ok(runtime) => Self::sanitize_token(&runtime.get_full_appid()),
            Err(_) => "msg_center".to_string(),
        }
    }

    fn get_kevent_client() -> KEventClient {
        static KEVENT_CLIENT: OnceLock<KEventClient> = OnceLock::new();
        KEVENT_CLIENT
            .get_or_init(|| KEventClient::new_full(Self::kevent_source_node(), None))
            .clone()
    }

    fn build_box_id(owner: &DID, box_kind: &MailboxKind) -> String {
        let owner_token = owner.to_raw_host_name();
        format!(
            "/msg_center/{}/{}_{}",
            owner_token,
            Self::box_id_prefix(box_kind),
            owner_token
        )
    }

    fn build_box_changed_event_id(owner: &DID, box_kind: &MailboxKind) -> String {
        format!(
            "{}/{}",
            Self::build_box_id(owner, box_kind),
            MSG_CENTER_BOX_CHANGED_EVENT_NAME
        )
    }

    fn publish_event(event_id: String, payload: Value) {
        let client = Self::get_kevent_client();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(err) = client.pub_event(&event_id, payload).await {
                    warn!(
                        "publish msg_center changed event failed: event_id={}, err={:?}",
                        event_id, err
                    );
                }
            });
        } else {
            warn!(
                "skip msg_center changed event without tokio runtime: event_id={}",
                event_id
            );
        }
    }

    fn publish_box_changed_event(record: &MailboxRecord, operation: &str) {
        let box_id = Self::build_box_id(&record.owner, &record.box_kind);
        let event_id = Self::build_box_changed_event_id(&record.owner, &record.box_kind);
        let payload = json!({
            "operation": operation,
            "owner": record.owner.to_string(),
            "box_kind": Self::box_kind_name(&record.box_kind),
            "box_id": box_id,
            "record_id": record.record_id.clone(),
            "msg_id": record.msg_id.to_string(),
            "state": record.state,
            "session_id": record.session_id.clone(),
            "updated_at_ms": record.updated_at_ms,
        });
        Self::publish_event(event_id, payload);
    }

    fn publish_delivery_changed_event(record: &DeliveryRecord, operation: &str) {
        let executor_token = record.envelope.transport_did.to_raw_host_name();
        let event_id = format!(
            "/msg_center/{}/delivery_queue_{}/{}",
            executor_token, executor_token, MSG_CENTER_BOX_CHANGED_EVENT_NAME
        );
        let payload = json!({
            "operation": operation,
            "transport_did": record.envelope.transport_did.to_string(),
            "delivery_id": record.delivery_id.clone(),
            "msg_id": record.envelope.msg_id.to_string(),
            "target_did": record.envelope.target_did.to_string(),
            "state": record.state,
            "attempts": record.attempts,
            "updated_at_ms": record.updated_at_ms,
        });
        Self::publish_event(event_id, payload);
    }

    fn clamp_limit(limit: Option<usize>, default: usize, max: usize) -> usize {
        limit.unwrap_or(default).clamp(1, max)
    }

    fn clamp_offset(offset: Option<u64>) -> usize {
        let raw = offset.unwrap_or(0);
        if raw > usize::MAX as u64 {
            usize::MAX
        } else {
            raw as usize
        }
    }

    fn dedupe_dids(values: Vec<DID>) -> Vec<DID> {
        let mut result = Vec::with_capacity(values.len());
        let mut visited = HashSet::new();
        for did in values {
            let key = did.to_string();
            if visited.insert(key) {
                result.push(did);
            }
        }
        result
    }

    fn is_group_message(msg: &MsgObject) -> bool {
        msg.kind == MsgObjKind::GroupMsg
    }

    /// Group messages carry `from = actor, to = group`; a group message
    /// without a group target is malformed (the legacy `from` fallback for old
    /// persisted records was removed in beta2.2).
    fn group_did_from_message(msg: &MsgObject) -> std::result::Result<DID, RPCErrors> {
        msg.to.first().cloned().ok_or_else(|| {
            RPCErrors::ParseRequestError(
                "group message requires the group DID in msg.to (from=actor, to=group)".to_string(),
            )
        })
    }

    fn ingress_snapshot(ingress_ctx: Option<&IngressContext>) -> Option<IngressContext> {
        let ctx = ingress_ctx?;
        let is_empty = ctx.transport_did.is_none()
            && ctx.platform.is_none()
            && ctx.chat_id.is_none()
            && ctx.source_account_id.is_none()
            && ctx.context_id.is_none()
            && ctx.extra.is_none();
        if is_empty {
            None
        } else {
            Some(ctx.clone())
        }
    }

    fn normalize_non_empty(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }

    fn normalize_ui_session_arg(
        label: &str,
        value: &str,
    ) -> std::result::Result<String, RPCErrors> {
        Self::normalize_non_empty(Some(value))
            .ok_or_else(|| RPCErrors::ReasonError(format!("{} cannot be empty", label)))
    }

    fn extract_session_id_from_value(payload: &Value) -> Option<String> {
        for pointer in [
            "/session_id",
            "/thread_key",
            "/owner_session_id",
            "/payload/session_id",
            "/payload/thread_key",
            "/payload/owner_session_id",
            "/payload/payload/session_id",
            "/payload/payload/thread_key",
            "/msg_payload/session_id",
            "/msg_payload/thread_key",
            "/msg/thread_key",
            "/meta/thread_key",
            "/thread/correlation_id",
            "/thread/topic",
            "/content/machine/data/session_id",
            "/content/machine/data/owner_session_id",
        ] {
            if let Some(session_id) =
                Self::normalize_non_empty(payload.pointer(pointer).and_then(|value| value.as_str()))
            {
                return Some(session_id);
            }
        }
        None
    }

    fn extract_record_session_id(msg: &MsgObject) -> Option<String> {
        if let Some(session_id) = Self::normalize_non_empty(msg.thread.correlation_id.as_deref()) {
            return Some(session_id);
        }
        if let Some(session_id) = Self::normalize_non_empty(
            msg.meta
                .get("session_id")
                .and_then(|value| value.as_str())
                .or_else(|| {
                    msg.meta
                        .get("owner_session_id")
                        .and_then(|value| value.as_str())
                }),
        ) {
            return Some(session_id);
        }
        let Ok(payload) = serde_json::to_value(msg) else {
            return None;
        };
        Self::extract_session_id_from_value(&payload)
    }

    /// Local session projection key of one record (`Message Center.md` §5.4):
    /// the message's semantic hint (`thread.topic` / correlation id) wins;
    /// otherwise group messages key on the group DID and direct messages key
    /// on the peer DID, so both directions of a DM land in the same session.
    fn derive_session_id(box_kind: &MailboxKind, msg: &MsgObject) -> Option<String> {
        if let Some(topic) = Self::normalize_non_empty(msg.thread.topic.as_deref()) {
            return Some(topic);
        }
        if let Some(session_id) = Self::extract_record_session_id(msg) {
            return Some(session_id);
        }
        if Self::is_group_message(msg) {
            return msg.to.first().map(|group| group.to_string());
        }
        match box_kind {
            MailboxKind::Sent => msg.to.first().map(|peer| format!("dm:{}", peer.to_string())),
            _ => Some(format!("dm:{}", msg.from.to_string())),
        }
    }

    async fn store_message(
        msg_id: &ObjId,
        msg_json_str: &str,
    ) -> std::result::Result<(), RPCErrors> {
        let msg_id = msg_id.clone();
        let msg_json = msg_json_str.to_string();
        let runtime = match get_buckyos_api_runtime() {
            Ok(runtime) => runtime,
            Err(RPCErrors::ReasonError(reason))
                if reason.contains("BuckyOSRuntime is not initialized") =>
            {
                warn!(
                    "skip storing message {} to named_store because runtime is not initialized",
                    msg_id.to_string()
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let named_store = runtime.get_named_store().await?;
        named_store
            .put_object(&msg_id, &msg_json)
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "store message {} in named_store failed: {}",
                    msg_id.to_string(),
                    error
                ))
            })?;
        Ok(())
    }

    async fn load_message(msg_id: &ObjId) -> std::result::Result<MsgObject, RPCErrors> {
        let msg_id = msg_id.clone();
        let runtime = get_buckyos_api_runtime()?;
        let named_store = runtime.get_named_store().await?;
        let msg_json = named_store.get_object(&msg_id).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "load message {} from named_store failed: {}",
                msg_id.to_string(),
                error
            ))
        })?;

        serde_json::from_str::<MsgObject>(&msg_json).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "parse message {} from named_store failed: {}",
                msg_id.to_string(),
                error
            ))
        })
    }

    fn message_obj_id(msg: &MsgObject) -> ObjId {
        msg.gen_obj_id().0
    }

    fn ensure_message(state: &mut MessageCenterState, msg: MsgObject) -> MsgObject {
        let msg_key = Self::message_obj_id(&msg).to_string();
        if let Some(existing) = state.messages.get(&msg_key) {
            return existing.clone();
        }
        state.messages.insert(msg_key, msg.clone());
        msg
    }

    fn build_record_id(
        owner: &DID,
        box_kind: &MailboxKind,
        msg_id: &ObjId,
        variant: &str,
    ) -> String {
        format!(
            "{}|{}|{}|{}",
            owner.to_string(),
            Self::box_kind_name(box_kind),
            msg_id.to_string(),
            Self::sanitize_token(variant)
        )
    }

    /// Deterministic idempotency key of one delivery:
    /// hash(msg_id + target_did + transport_did).
    fn build_delivery_id(msg_id: &ObjId, target_did: &DID, transport_did: &DID) -> String {
        let mut hasher = Sha256::new();
        hasher.update(msg_id.to_string().as_bytes());
        hasher.update(b"|");
        hasher.update(target_did.to_string().as_bytes());
        hasher.update(b"|");
        hasher.update(transport_did.to_string().as_bytes());
        format!("dlv-{}", hex::encode(&hasher.finalize()[..16]))
    }

    async fn create_or_get_record(
        &self,
        owner: DID,
        box_kind: MailboxKind,
        msg: &MsgObject,
        initial_state: RecipientState,
        ingress: Option<IngressContext>,
        tags: Vec<String>,
        variant: &str,
    ) -> std::result::Result<MailboxRecord, RPCErrors> {
        let msg_id = Self::message_obj_id(msg);
        let record_id = Self::build_record_id(&owner, &box_kind, &msg_id, variant);
        let session_id = Self::derive_session_id(&box_kind, msg);
        if let Some(existing) = self.msg_box_db.get_record(&owner, &record_id).await? {
            let mut record_for_update = existing.clone();
            if record_for_update.msg_kind != msg.kind {
                record_for_update.msg_kind = msg.kind;
            }
            if record_for_update.session_id.is_none() {
                record_for_update.session_id = session_id;
            }
            self.msg_box_db
                .upsert_record_with_msg(&record_for_update, Some(msg))
                .await?;
            Self::publish_box_changed_event(&record_for_update, "upsert");
            return Ok(record_for_update);
        }

        let now_ms = Self::now_ms();
        let record_to = match box_kind {
            MailboxKind::Inbox | MailboxKind::GroupInbox | MailboxKind::RequestBox => owner.clone(),
            MailboxKind::Sent => msg.to.first().cloned().unwrap_or_else(|| owner.clone()),
        };
        let record = MailboxRecord {
            record_id: record_id.clone(),
            owner: owner.clone(),
            box_kind,
            msg_id: msg_id.clone(),
            msg_kind: msg.kind,
            state: initial_state,
            from: msg.from.clone(),
            from_name: None,
            to: record_to,
            session_id,
            sort_key: if msg.created_at_ms > 0 {
                msg.created_at_ms
            } else {
                now_ms
            },
            tags,
            ingress,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };

        self.msg_box_db
            .upsert_record_with_msg(&record, Some(msg))
            .await?;
        Self::publish_box_changed_event(&record, "upsert");
        Ok(record)
    }

    async fn load_box_records(
        &self,
        owner: &DID,
        box_kind: &MailboxKind,
        state_filter: Option<&[RecipientState]>,
        descending: bool,
    ) -> std::result::Result<Vec<MailboxRecord>, RPCErrors> {
        self.msg_box_db
            .list_records(owner, box_kind, state_filter, descending)
            .await
    }

    fn filter_after_cursor(
        records: Vec<MailboxRecord>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<&str>,
        descending: bool,
    ) -> Vec<MailboxRecord> {
        let Some(cursor_sort_key) = cursor_sort_key else {
            return records;
        };
        let cursor_record_id = cursor_record_id.unwrap_or("");

        records
            .into_iter()
            .filter(|record| {
                if descending {
                    record.sort_key < cursor_sort_key
                        || (record.sort_key == cursor_sort_key
                            && record.record_id.as_str() < cursor_record_id)
                } else {
                    record.sort_key > cursor_sort_key
                        || (record.sort_key == cursor_sort_key
                            && record.record_id.as_str() > cursor_record_id)
                }
            })
            .collect()
    }

    fn owner_from_record_id(record_id: &str) -> std::result::Result<DID, RPCErrors> {
        let owner = record_id.split('|').next().ok_or_else(|| {
            RPCErrors::ReasonError(format!("invalid record id '{}': missing owner", record_id))
        })?;
        DID::from_str(owner).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "invalid record id '{}': owner DID parse failed: {}",
                record_id, error
            ))
        })
    }

    async fn build_record_view(
        record: MailboxRecord,
        with_object: Option<bool>,
    ) -> std::result::Result<MailboxRecordWithObject, RPCErrors> {
        let mut result = MailboxRecordWithObject { record, msg: None };
        if with_object.unwrap_or(false) {
            result.msg = Some(Self::load_message(&result.record.msg_id).await?);
        }
        Ok(result)
    }

    fn next_state_on_take(
        box_kind: &MailboxKind,
        state: &RecipientState,
    ) -> Option<RecipientState> {
        match (box_kind, state) {
            (MailboxKind::Inbox, RecipientState::Unread)
            | (MailboxKind::GroupInbox, RecipientState::Unread)
            | (MailboxKind::RequestBox, RecipientState::Unread) => Some(RecipientState::Reading),
            _ => None,
        }
    }

    fn is_valid_transition(
        box_kind: &MailboxKind,
        current: &RecipientState,
        next: &RecipientState,
    ) -> bool {
        if current == next {
            return true;
        }
        if matches!(next, RecipientState::Deleted | RecipientState::Archived) {
            return true;
        }

        match box_kind {
            MailboxKind::Inbox | MailboxKind::GroupInbox | MailboxKind::RequestBox => {
                match current {
                    RecipientState::Unread => {
                        matches!(next, RecipientState::Reading | RecipientState::Read)
                    }
                    RecipientState::Reading => {
                        matches!(next, RecipientState::Unread | RecipientState::Read)
                    }
                    RecipientState::Read => matches!(next, RecipientState::Reading),
                    _ => false,
                }
            }
            // SENT records have no reading semantics: only archive / delete.
            MailboxKind::Sent => false,
        }
    }

    async fn is_contact_blocked(
        &self,
        did: &DID,
        owner: Option<DID>,
    ) -> std::result::Result<bool, RPCErrors> {
        let contact = self.contact_mgr.get_contact(did.clone(), owner).await?;
        Ok(contact
            .map(|item| item.access_level == AccessGroupLevel::Block)
            .unwrap_or(false))
    }

    async fn decide_inbox_kind(
        &self,
        sender: &DID,
        target: &DID,
        context_id: Option<String>,
    ) -> std::result::Result<Option<MailboxKind>, RPCErrors> {
        let decision: AccessDecision = self
            .contact_mgr
            .check_access_permission(sender.clone(), context_id, Some(target.clone()))
            .await?;
        if decision.allow_delivery {
            return Ok(Some(MailboxKind::Inbox));
        }

        let target_box = decision.target_box.to_ascii_uppercase();
        if target_box == "REQUEST_BOX" {
            Ok(Some(MailboxKind::RequestBox))
        } else {
            Ok(None)
        }
    }

    /// Resolve a *determined* target DID into a delivery envelope.
    ///
    /// `post_send` only accepts confirmed `MsgObject.to`; exactly two
    /// deterministic branches exist (`Message Center.md` §2.2):
    ///
    /// - local shadow endpoint DID (`did:msgtunnel:*`) → MessageTunnel: the
    ///   embedded `tunnel_instance_id` names a registered tunnel; the platform
    ///   address comes from the DID's embedded account, snapshot into the
    ///   envelope.
    /// - shareable DID (everything else) → MessageHub native delivery.
    ///
    /// Any resolution failure fails the whole `post_send`. There is no default
    /// tunnel, no default chat and no last-active fallback.
    fn build_delivery_envelope(
        &self,
        msg_id: &ObjId,
        target_did: DID,
    ) -> std::result::Result<DeliveryEnvelope, String> {
        if let Some((account_id, account_type, tunnel_instance_id)) =
            ContactMgr::parse_msgtunnel_did(&target_did)
        {
            let route = self.lookup_tunnel_route(&tunnel_instance_id).ok_or_else(|| {
                format!(
                    "unknown tunnel_instance_id '{}' for endpoint target {}; no tunnel registered",
                    tunnel_instance_id,
                    target_did.to_string()
                )
            })?;

            // The DID-embedded account is the delivery address. `chat_id` for
            // conversational platforms, `address` for mailbox-style platforms;
            // the executor consumes the snapshot and never guesses.
            let (chat_id, address) = if account_type == "addr" {
                (None, Some(account_id.clone()))
            } else {
                (Some(account_id.clone()), None)
            };
            let snapshot = DeliverySnapshot {
                platform: Some(route.platform.clone()),
                account_id: Some(account_id),
                account_type: Some(account_type),
                chat_id,
                address,
                ext_ids: HashMap::new(),
                extra: None,
            };
            return Ok(DeliveryEnvelope {
                msg_id: msg_id.clone(),
                target_did,
                transport_did: route.transport_did,
                transport: TransportKind::Tunnel {
                    platform: route.platform,
                    tunnel_instance_id,
                },
                address: Some(snapshot),
            });
        }

        // Shareable DID → MessageHub native delivery.
        let hub_did = self.message_hub_did().ok_or_else(|| {
            format!(
                "no message hub executor available for shareable target {}",
                target_did.to_string()
            )
        })?;
        Ok(DeliveryEnvelope {
            msg_id: msg_id.clone(),
            target_did,
            transport_did: hub_did,
            transport: TransportKind::Native,
            address: None,
        })
    }

    async fn dispatch_internal(
        &self,
        msg: MsgObject,
        ingress_ctx: Option<IngressContext>,
        idempotency_key: Option<String>,
    ) -> std::result::Result<DispatchResult, RPCErrors> {
        let ingress_contact_mgr_owner = ingress_ctx
            .as_ref()
            .and_then(|ctx| ctx.contact_mgr_owner.clone());

        enum DispatchPrepare {
            Done(DispatchResult),
            Ready {
                stored_msg: MsgObject,
                stored_msg_id: ObjId,
                stored_msg_json: String,
                sender: DID,
                context_id: Option<String>,
                ingress: Option<IngressContext>,
            },
        }

        let prepared = self.with_state_write(|state| {
            if let Some(key) = idempotency_key.as_ref() {
                if let Some(cached) = state.dispatch_idempotency.get(key) {
                    return Ok(DispatchPrepare::Done(cached.clone()));
                }
            }

            let stored_msg = Self::ensure_message(state, msg);
            let (stored_msg_id, stored_msg_json) = stored_msg.gen_obj_id();

            let sender = stored_msg.from.clone();
            let context_id = ingress_ctx.as_ref().and_then(|ctx| ctx.context_id.clone());

            Ok(DispatchPrepare::Ready {
                stored_msg,
                stored_msg_id,
                stored_msg_json,
                sender,
                context_id,
                ingress: Self::ingress_snapshot(ingress_ctx.as_ref()),
            })
        })?;

        let (stored_msg, stored_msg_id, stored_msg_json, sender, context_id, ingress) =
            match prepared {
                DispatchPrepare::Done(result) => return Ok(result),
                DispatchPrepare::Ready {
                    stored_msg,
                    stored_msg_id,
                    stored_msg_json,
                    sender,
                    context_id,
                    ingress,
                } => (
                    stored_msg,
                    stored_msg_id,
                    stored_msg_json,
                    sender,
                    context_id,
                    ingress,
                ),
            };

        Self::store_message(&stored_msg_id, &stored_msg_json).await?;

        // Re-check idempotency before doing any work; a concurrent caller may
        // have completed the same dispatch while we were awaiting store_message.
        if let Some(key) = idempotency_key.as_ref() {
            if let Some(cached) =
                self.with_state_read(|state| Ok(state.dispatch_idempotency.get(key).cloned()))?
            {
                return Ok(cached);
            }
        }

        let mut result = DispatchResult {
            ok: true,
            msg_id: stored_msg_id.clone(),
            delivered_recipients: Vec::new(),
            dropped_recipients: Vec::new(),
            delivered_group: None,
            delivered_agents: Vec::new(),
            reason: None,
        };

        if Self::is_group_message(&stored_msg) {
            if self
                .is_contact_blocked(&sender, ingress_contact_mgr_owner.clone())
                .await?
            {
                warn!(
                    "dispatch blocked by sender access policy: msg_id={}, sender={}, context_id={}, contact_mgr_owner={}",
                    stored_msg_id.to_string(),
                    sender.to_string(),
                    context_id.as_deref().unwrap_or("-"),
                    ingress_contact_mgr_owner
                        .as_ref()
                        .map(|did| did.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                );
                let blocked = DispatchResult {
                    ok: false,
                    msg_id: stored_msg_id.clone(),
                    delivered_recipients: Vec::new(),
                    dropped_recipients: Vec::new(),
                    delivered_group: None,
                    delivered_agents: Vec::new(),
                    reason: Some("blocked".to_string()),
                };
                if let Some(key) = idempotency_key.as_ref() {
                    self.with_state_write(|state| {
                        state
                            .dispatch_idempotency
                            .insert(key.clone(), blocked.clone());
                        Ok(())
                    })?;
                }
                return Ok(blocked);
            }

            let group_id = Self::group_did_from_message(&stored_msg)?;
            info!(
                "dispatch about to write inbox record: msg_id={}, sender={}, owner={}, box_kind=GROUP_INBOX, context_id={}",
                stored_msg_id.to_string(),
                sender.to_string(),
                group_id.to_string(),
                context_id.as_deref().unwrap_or("-"),
            );
            self.create_or_get_record(
                group_id.clone(),
                MailboxKind::GroupInbox,
                &stored_msg,
                RecipientState::Unread,
                ingress.clone(),
                Vec::new(),
                "group-inbox",
            )
            .await?;

            // Prefer the authoritative member list from GroupMgr when this
            // group is hosted locally; fall back to the ContactMgr
            // subscriber index for joined groups (whose member roster lives
            // on the remote host Zone).
            let owner_key_for_group = ingress_contact_mgr_owner
                .as_ref()
                .map(|did| did.to_string())
                .unwrap_or_else(|| "__system__".to_string());
            let readers = match self
                .group_mgr
                .active_singleton_members(&owner_key_for_group, &group_id)
                .await?
            {
                Some(members) => members,
                None => {
                    self.contact_mgr
                        .get_group_subscribers(
                            group_id.clone(),
                            None,
                            None,
                            ingress_contact_mgr_owner.clone(),
                        )
                        .await?
                }
            };
            let readers = Self::dedupe_dids(readers);
            for agent_did in readers.iter() {
                let tag = format!("group:{}", group_id.to_string());
                self.create_or_get_record(
                    agent_did.clone(),
                    MailboxKind::Inbox,
                    &stored_msg,
                    RecipientState::Unread,
                    ingress.clone(),
                    vec![tag],
                    &format!("group-agent-{}", group_id.to_string()),
                )
                .await?;
            }

            result.delivered_group = Some(group_id);
            result.delivered_agents = readers;
        } else {
            let recipients = Self::dedupe_dids(stored_msg.to.clone());
            if recipients.is_empty() {
                warn!(
                    "dispatch has no recipients, cannot write inbox: msg_id={}, sender={}, context_id={}",
                    stored_msg_id.to_string(),
                    sender.to_string(),
                    context_id.as_deref().unwrap_or("-"),
                );
            }
            for recipient in recipients {
                if self
                    .is_contact_blocked(&sender, Some(recipient.clone()))
                    .await?
                {
                    warn!(
                        "dispatch blocked by sender access policy: msg_id={}, sender={}, recipient={}, context_id={}",
                        stored_msg_id.to_string(),
                        sender.to_string(),
                        recipient.to_string(),
                        context_id.as_deref().unwrap_or("-"),
                    );
                    result.dropped_recipients.push(recipient);
                    continue;
                }

                let decision = match self
                    .decide_inbox_kind(&sender, &recipient, context_id.clone())
                    .await
                {
                    Ok(value) => value,
                    Err(error) => {
                        warn!(
                            "dispatch failed while deciding inbox: msg_id={}, sender={}, recipient={}, context_id={}, error={}",
                            stored_msg_id.to_string(),
                            sender.to_string(),
                            recipient.to_string(),
                            context_id.as_deref().unwrap_or("-"),
                            error,
                        );
                        return Err(error);
                    }
                };
                match decision {
                    Some(box_kind) => {
                        if box_kind == MailboxKind::RequestBox {
                            warn!(
                                "dispatch inbox not found, route to REQUEST_BOX: msg_id={}, sender={}, recipient={}, context_id={}",
                                stored_msg_id.to_string(),
                                sender.to_string(),
                                recipient.to_string(),
                                context_id.as_deref().unwrap_or("-"),
                            );
                        }
                        self.create_or_get_record(
                            recipient.clone(),
                            box_kind,
                            &stored_msg,
                            RecipientState::Unread,
                            ingress.clone(),
                            Vec::new(),
                            "inbox",
                        )
                        .await?;
                        result.delivered_recipients.push(recipient);
                    }
                    None => {
                        warn!(
                            "dispatch inbox not found, dropping recipient: msg_id={}, sender={}, recipient={}, context_id={}",
                            stored_msg_id.to_string(),
                            sender.to_string(),
                            recipient.to_string(),
                            context_id.as_deref().unwrap_or("-"),
                        );
                        result.dropped_recipients.push(recipient);
                    }
                }
            }
        }

        if let Some(key) = idempotency_key.as_ref() {
            self.with_state_write(|state| {
                state
                    .dispatch_idempotency
                    .insert(key.clone(), result.clone());
                Ok(())
            })?;
        }
        Ok(result)
    }

    async fn post_send_internal(
        &self,
        msg: MsgObject,
        idempotency_key: Option<String>,
    ) -> std::result::Result<PostSendResult, RPCErrors> {
        if msg.to.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "post_send requires at least one target in msg.to".to_string(),
            ));
        }

        enum PostSendPrepare {
            Done(PostSendResult),
            Ready {
                stored_msg: MsgObject,
                stored_msg_id: ObjId,
                stored_msg_json: String,
                author: DID,
                contact_mgr_owner: Option<DID>,
            },
        }

        let prepared = self.with_state_write(|state| {
            if let Some(key) = idempotency_key.as_ref() {
                if let Some(cached) = state.post_send_idempotency.get(key) {
                    return Ok(PostSendPrepare::Done(cached.clone()));
                }
            }

            let stored_msg = Self::ensure_message(state, msg);
            let (stored_msg_id, stored_msg_json) = stored_msg.gen_obj_id();
            let author = stored_msg.from.clone();
            // Owner scope for contact-manager lookups is the message author.
            let contact_mgr_owner = Some(author.clone());

            Ok(PostSendPrepare::Ready {
                stored_msg,
                stored_msg_id,
                stored_msg_json,
                author,
                contact_mgr_owner,
            })
        })?;

        let (stored_msg, stored_msg_id, stored_msg_json, author, contact_mgr_owner) = match prepared
        {
            PostSendPrepare::Done(result) => return Ok(result),
            PostSendPrepare::Ready {
                stored_msg,
                stored_msg_id,
                stored_msg_json,
                author,
                contact_mgr_owner,
            } => (
                stored_msg,
                stored_msg_id,
                stored_msg_json,
                author,
                contact_mgr_owner,
            ),
        };

        let cache_result = |result: PostSendResult| -> std::result::Result<PostSendResult, RPCErrors> {
            if let Some(key) = idempotency_key.as_ref() {
                self.with_state_write(|state| {
                    state
                        .post_send_idempotency
                        .insert(key.clone(), result.clone());
                    Ok(())
                })?;
            }
            Ok(result)
        };

        if self
            .is_contact_blocked(&author, contact_mgr_owner.clone())
            .await?
        {
            return cache_result(PostSendResult {
                ok: false,
                msg_id: stored_msg_id.clone(),
                deliveries: Vec::new(),
                reason: Some("blocked_author".to_string()),
            });
        }

        // Phase 1: resolve every target up front (pure, no writes). One
        // unroutable target fails the whole post_send with a clear reason —
        // the database keeps no partial state, never a silent fallback.
        let delivery_targets = Self::dedupe_dids(stored_msg.to.clone());
        let mut envelopes = Vec::with_capacity(delivery_targets.len());
        for target in delivery_targets {
            match self.build_delivery_envelope(&stored_msg_id, target) {
                Ok(envelope) => envelopes.push(envelope),
                Err(reason) => {
                    return cache_result(PostSendResult {
                        ok: false,
                        msg_id: stored_msg_id.clone(),
                        deliveries: Vec::new(),
                        reason: Some(reason),
                    });
                }
            }
        }

        // Phase 2: persist the message, the SENT mailbox record and one
        // delivery record per target. All ids are deterministic, so replays
        // converge onto the same rows.
        Self::store_message(&stored_msg_id, &stored_msg_json).await?;

        if let Some(key) = idempotency_key.as_ref() {
            if let Some(cached) =
                self.with_state_read(|state| Ok(state.post_send_idempotency.get(key).cloned()))?
            {
                return Ok(cached);
            }
        }

        self.create_or_get_record(
            author,
            MailboxKind::Sent,
            &stored_msg,
            RecipientState::Read,
            None,
            Vec::new(),
            "owner-sent",
        )
        .await?;

        let now_ms = Self::now_ms();
        let mut deliveries = Vec::with_capacity(envelopes.len());
        for envelope in envelopes {
            let delivery_id = Self::build_delivery_id(
                &envelope.msg_id,
                &envelope.target_did,
                &envelope.transport_did,
            );
            let record = DeliveryRecord {
                delivery_id: delivery_id.clone(),
                envelope: envelope.clone(),
                state: DeliveryState::Wait,
                attempts: 0,
                next_retry_at_ms: None,
                external_msg_id: None,
                delivered_at_ms: None,
                last_error: None,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            };
            self.msg_box_db.create_delivery_if_absent(&record).await?;
            Self::publish_delivery_changed_event(&record, "enqueue");

            deliveries.push(PostSendDelivery {
                delivery_id,
                transport_did: envelope.transport_did,
                target_did: envelope.target_did,
                transport: envelope.transport,
            });
        }

        cache_result(PostSendResult {
            ok: true,
            msg_id: stored_msg_id.clone(),
            deliveries,
            reason: None,
        })
    }

    async fn get_next_internal(
        &self,
        owner: DID,
        box_kind: MailboxKind,
        state_filter: Option<Vec<RecipientState>>,
        lock_on_take: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<Option<MailboxRecordWithObject>, RPCErrors> {
        let default_filter = match box_kind {
            MailboxKind::Inbox | MailboxKind::GroupInbox | MailboxKind::RequestBox => {
                Some(vec![RecipientState::Unread])
            }
            MailboxKind::Sent => None,
        };
        let effective_filter = state_filter.or(default_filter);
        let state_filter_ref = effective_filter.as_deref();
        let records = self
            .load_box_records(&owner, &box_kind, state_filter_ref, false)
            .await?;
        let mut selected = records.into_iter().next();
        if let Some(record) = selected.as_mut() {
            if lock_on_take.unwrap_or(true) {
                if let Some(next_state) = Self::next_state_on_take(&box_kind, &record.state) {
                    record.state = next_state;
                    record.updated_at_ms = Self::now_ms();
                    self.msg_box_db.upsert_record(record).await?;
                    Self::publish_box_changed_event(record, "take");
                }
            }
        }

        let Some(record) = selected else {
            return Ok(None);
        };
        let record = Self::build_record_view(record, with_object).await?;
        Ok(Some(record))
    }

    async fn get_next_delivery_internal(
        &self,
        transport_did: DID,
        lock_on_take: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<Option<DeliveryRecordWithObject>, RPCErrors> {
        let now_ms = Self::now_ms();
        // Lease sweep first: reclaim SENDING rows from crashed executors so a
        // lost notification never strands a delivery (the poll IS the sweep).
        let lease_deadline = now_ms.saturating_sub(DELIVERY_SENDING_LEASE_MS);
        let reclaimed = self
            .msg_box_db
            .reclaim_stale_sending(&transport_did, lease_deadline, now_ms)
            .await?;
        if reclaimed > 0 {
            warn!(
                "reclaimed {} stale SENDING delivery record(s) for {}",
                reclaimed,
                transport_did.to_string()
            );
        }

        let taken = self
            .msg_box_db
            .take_next_delivery(&transport_did, now_ms, lock_on_take.unwrap_or(true))
            .await?;
        let Some(record) = taken else {
            return Ok(None);
        };
        Self::publish_delivery_changed_event(&record, "take");

        let mut view = DeliveryRecordWithObject { record, msg: None };
        if with_object.unwrap_or(false) {
            view.msg = Some(Self::load_message(&view.record.envelope.msg_id).await?);
        }
        Ok(Some(view))
    }

    fn retry_backoff_ms(attempts: u32) -> u64 {
        let shift = attempts.min(16);
        (DELIVERY_RETRY_BASE_MS.saturating_mul(1u64 << shift)).min(DELIVERY_RETRY_MAX_MS)
    }

    async fn report_delivery_internal(
        &self,
        delivery_id: String,
        result_payload: DeliveryReportResult,
    ) -> std::result::Result<DeliveryRecord, RPCErrors> {
        let mut record = self
            .msg_box_db
            .get_delivery(&delivery_id)
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("delivery record {} not found", delivery_id))
            })?;

        let now_ms = Self::now_ms();
        record.attempts = record.attempts.saturating_add(1);

        if result_payload.ok {
            record.state = DeliveryState::Sent;
            record.external_msg_id = result_payload.external_msg_id.clone();
            record.delivered_at_ms = Some(result_payload.delivered_at_ms.unwrap_or(now_ms));
            record.next_retry_at_ms = None;
            record.last_error = None;
        } else {
            let retryable = result_payload.retryable.unwrap_or(true);
            let error = DeliveryError {
                error_code: result_payload.error_code.clone(),
                message: result_payload
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "delivery failed".to_string()),
                retryable,
                duplicate_risk: false,
            };
            record.last_error = Some(error);
            let too_many_attempts = record.attempts >= MAX_DELIVERY_RETRY;
            if retryable && !too_many_attempts {
                // FAILED → WAIT with backoff: the retry scheduler is the queue
                // itself (next_retry_at_ms gates take_next_delivery).
                record.state = DeliveryState::Wait;
                let backoff = result_payload
                    .retry_after_ms
                    .unwrap_or_else(|| Self::retry_backoff_ms(record.attempts));
                record.next_retry_at_ms = Some(now_ms.saturating_add(backoff));
            } else {
                record.state = DeliveryState::Dead;
                record.next_retry_at_ms = None;
            }
        }

        record.updated_at_ms = now_ms;
        self.msg_box_db.upsert_delivery(&record).await?;
        Self::publish_delivery_changed_event(&record, "delivery");
        Ok(record)
    }

    async fn update_record_state_internal(
        &self,
        record_id: String,
        new_state: RecipientState,
    ) -> std::result::Result<MailboxRecord, RPCErrors> {
        let owner = Self::owner_from_record_id(&record_id)?;
        let mut record = self
            .msg_box_db
            .get_record(&owner, &record_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("record {} not found", record_id)))?;

        if !Self::is_valid_transition(&record.box_kind, &record.state, &new_state) {
            return Err(RPCErrors::ReasonError(format!(
                "invalid state transition {:?} -> {:?} for {:?}",
                record.state, new_state, record.box_kind
            )));
        }

        record.state = new_state;
        record.updated_at_ms = Self::now_ms();
        self.msg_box_db.upsert_record(&record).await?;
        Self::publish_box_changed_event(&record, "state");
        Ok(record)
    }

    async fn update_record_session_internal(
        &self,
        record_id: String,
        session_id: String,
    ) -> std::result::Result<MailboxRecord, RPCErrors> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(RPCErrors::ReasonError(
                "session_id cannot be empty".to_string(),
            ));
        }

        let owner = Self::owner_from_record_id(&record_id)?;
        let mut record = self
            .msg_box_db
            .get_record(&owner, &record_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("record {} not found", record_id)))?;

        if record.session_id.as_deref() == Some(session_id) {
            return Ok(record);
        }

        record.session_id = Some(session_id.to_string());
        record.updated_at_ms = Self::now_ms();
        self.msg_box_db.upsert_record(&record).await?;
        Self::publish_box_changed_event(&record, "session");
        Ok(record)
    }

    /// Aggregate all delivery records of one outbound message into the session
    /// view (`Message Center.md` §5.3): all SENT → delivered; any WAIT/SENDING
    /// → sending; some DEAD/FAILED → partial_failed; all DEAD → failed.
    fn aggregate_delivery_view(deliveries: &[DeliveryRecord]) -> Option<SessionDeliveryView> {
        if deliveries.is_empty() {
            return None;
        }
        let mut pending = 0usize;
        let mut sent = 0usize;
        let mut dead_or_failed = 0usize;
        let mut per_target = Vec::with_capacity(deliveries.len());
        for record in deliveries {
            match record.state {
                DeliveryState::Wait | DeliveryState::Sending => pending += 1,
                DeliveryState::Sent => sent += 1,
                DeliveryState::Failed | DeliveryState::Dead => dead_or_failed += 1,
            }
            per_target.push(SessionDeliveryTarget {
                target_did: record.envelope.target_did.clone(),
                state: record.state,
                attempts: record.attempts,
                external_msg_id: record.external_msg_id.clone(),
                last_error: record.last_error.clone(),
            });
        }

        let overall = if pending > 0 {
            SessionDeliveryOverall::Sending
        } else if dead_or_failed == 0 {
            SessionDeliveryOverall::Delivered
        } else if sent > 0 {
            SessionDeliveryOverall::PartialFailed
        } else {
            SessionDeliveryOverall::Failed
        };

        Some(SessionDeliveryView {
            overall,
            per_target,
        })
    }

    async fn build_session_item(
        &self,
        record: MailboxRecord,
        with_object: bool,
    ) -> std::result::Result<SessionMessageItem, RPCErrors> {
        let direction = match record.box_kind {
            MailboxKind::Sent => SessionMessageDirection::Out,
            _ => SessionMessageDirection::In,
        };
        let (recipient_state, delivery) = match direction {
            SessionMessageDirection::In => (Some(record.state), None),
            SessionMessageDirection::Out => {
                let deliveries = self
                    .msg_box_db
                    .list_deliveries_for_msg(&record.msg_id)
                    .await?;
                (None, Self::aggregate_delivery_view(&deliveries))
            }
        };
        let msg = if with_object {
            Some(Self::load_message(&record.msg_id).await?)
        } else {
            None
        };
        Ok(SessionMessageItem {
            record_id: record.record_id,
            msg_id: record.msg_id,
            direction,
            box_kind: record.box_kind,
            sort_key: record.sort_key,
            from: record.from,
            to: record.to,
            recipient_state,
            delivery,
            msg,
        })
    }

    async fn list_sessions_internal(
        &self,
        owner: DID,
        limit: Option<usize>,
        cursor_updated_at_ms: Option<u64>,
        cursor_session_id: Option<String>,
        with_object: Option<bool>,
    ) -> std::result::Result<SessionSummaryPage, RPCErrors> {
        let limit = Self::clamp_limit(limit, DEFAULT_SESSION_LIST_LIMIT, MAX_SESSION_LIST_LIMIT);
        // Fetch one extra row to detect whether another page exists.
        let entries = self
            .msg_box_db
            .list_session_index(
                &owner,
                limit + 1,
                cursor_updated_at_ms,
                cursor_session_id.as_deref(),
            )
            .await?;
        let has_more = entries.len() > limit;
        let page_entries = entries.into_iter().take(limit).collect::<Vec<_>>();

        let mut items = Vec::with_capacity(page_entries.len());
        for entry in page_entries {
            let last_record = self
                .msg_box_db
                .list_session_records(&owner, &entry.session_id, 1, None, None, true)
                .await?
                .into_iter()
                .next();
            let last_record = match last_record {
                Some(record) => Some(
                    Self::build_record_view(record, Some(with_object.unwrap_or(false))).await?,
                ),
                None => None,
            };
            items.push(SessionSummary {
                session_id: entry.session_id,
                last_record,
                unread_count: entry.unread_count,
                updated_at_ms: entry.updated_at_ms,
            });
        }

        let (next_cursor_updated_at_ms, next_cursor_session_id) = if has_more {
            items
                .last()
                .map(|item| (Some(item.updated_at_ms), Some(item.session_id.clone())))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        Ok(SessionSummaryPage {
            items,
            next_cursor_updated_at_ms,
            next_cursor_session_id,
        })
    }

    async fn list_session_internal(
        &self,
        owner: DID,
        session_id: String,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<SessionMessagePage, RPCErrors> {
        let session_id = Self::normalize_ui_session_arg("session_id", &session_id)?;
        let descending = descending.unwrap_or(true);
        let limit = Self::clamp_limit(limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT);
        let records = self
            .msg_box_db
            .list_session_records(
                &owner,
                &session_id,
                limit + 1,
                cursor_sort_key,
                cursor_record_id.as_deref(),
                descending,
            )
            .await?;
        let has_more = records.len() > limit;
        let page_records = records.into_iter().take(limit).collect::<Vec<_>>();

        let mut items = Vec::with_capacity(page_records.len());
        for record in page_records {
            items.push(
                self.build_session_item(record, with_object.unwrap_or(false))
                    .await?,
            );
        }

        let (next_cursor_sort_key, next_cursor_record_id) = if has_more {
            items
                .last()
                .map(|item| (Some(item.sort_key), Some(item.record_id.clone())))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        Ok(SessionMessagePage {
            items,
            next_cursor_sort_key,
            next_cursor_record_id,
        })
    }

    async fn peek_box_internal(
        &self,
        owner: DID,
        box_kind: MailboxKind,
        state_filter: Option<Vec<RecipientState>>,
        limit: Option<usize>,
        with_object: Option<bool>,
    ) -> std::result::Result<Vec<MailboxRecordWithObject>, RPCErrors> {
        let limit = Self::clamp_limit(limit, DEFAULT_PEEK_LIMIT, MAX_PEEK_LIMIT);
        let state_filter_ref = state_filter.as_deref();
        let records = self
            .load_box_records(&owner, &box_kind, state_filter_ref, true)
            .await?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();

        let mut result = Vec::with_capacity(records.len());
        for record in records {
            result.push(Self::build_record_view(record, with_object).await?);
        }
        Ok(result)
    }

    async fn list_box_by_time_internal(
        &self,
        owner: DID,
        box_kind: MailboxKind,
        state_filter: Option<Vec<RecipientState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<MailboxRecordPage, RPCErrors> {
        let descending = descending.unwrap_or(true);
        let limit = Self::clamp_limit(limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT);
        let state_filter_ref = state_filter.as_deref();
        let records = self
            .load_box_records(&owner, &box_kind, state_filter_ref, descending)
            .await?;
        let records = Self::filter_after_cursor(
            records,
            cursor_sort_key,
            cursor_record_id.as_deref(),
            descending,
        );
        let has_more = records.len() > limit;
        let page_records = records.into_iter().take(limit).collect::<Vec<_>>();

        let mut items = Vec::with_capacity(page_records.len());
        for record in page_records {
            items.push(Self::build_record_view(record, with_object).await?);
        }

        let (next_cursor_sort_key, next_cursor_record_id) = if has_more {
            if let Some(last) = items.last() {
                (
                    Some(last.record.sort_key),
                    Some(last.record.record_id.clone()),
                )
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Ok(MailboxRecordPage {
            items,
            next_cursor_sort_key,
            next_cursor_record_id,
        })
    }

    async fn set_read_state_internal(
        &self,
        group_id: DID,
        msg_id: ObjId,
        reader_did: DID,
        status: ReadReceiptState,
        reason: Option<String>,
        at_ms: Option<u64>,
    ) -> std::result::Result<MsgReceiptObj, RPCErrors> {
        let msg_key = msg_id.to_string();
        let in_memory = self.with_state_read(|state| Ok(state.messages.contains_key(&msg_key)))?;
        if !in_memory && !self.msg_box_db.has_message(&group_id, &msg_id).await? {
            return Err(RPCErrors::ReasonError(format!(
                "message {} not found",
                msg_id.to_string()
            )));
        }

        self.with_state_write(|state| {
            let receipt = MsgReceiptObj {
                msg_id: msg_id.clone(),
                iss: reader_did.clone(),
                reader: reader_did.clone(),
                group_id: Some(group_id.clone()),
                at_ms: at_ms.unwrap_or_else(Self::now_ms),
                status,
                reason,
            };
            let receipt_id = format!(
                "{}|{}|{}",
                group_id.to_string(),
                reader_did.to_string(),
                msg_id.to_string()
            );
            state.receipts.insert(receipt_id, receipt.clone());
            Ok(receipt)
        })
    }

    fn list_read_receipts_internal(
        &self,
        msg_id: ObjId,
        group_id: Option<DID>,
        reader: Option<DID>,
        limit: Option<usize>,
        offset: Option<u64>,
    ) -> std::result::Result<Vec<MsgReceiptObj>, RPCErrors> {
        self.with_state_read(|state| {
            let limit =
                Self::clamp_limit(limit, DEFAULT_READ_RECEIPT_LIMIT, MAX_READ_RECEIPT_LIMIT);
            let offset = Self::clamp_offset(offset);
            let mut receipts: Vec<MsgReceiptObj> = state
                .receipts
                .values()
                .filter(|receipt| receipt.msg_id == msg_id)
                .filter(|receipt| match group_id.as_ref() {
                    Some(group_id) => receipt.group_id.as_ref() == Some(group_id),
                    None => true,
                })
                .filter(|receipt| match reader.as_ref() {
                    Some(reader) => &receipt.reader == reader,
                    None => true,
                })
                .cloned()
                .collect();

            receipts.sort_by(|left, right| {
                right
                    .at_ms
                    .cmp(&left.at_ms)
                    .then_with(|| left.reader.to_string().cmp(&right.reader.to_string()))
            });

            Ok(receipts.into_iter().skip(offset).take(limit).collect())
        })
    }

    async fn get_record_internal(
        &self,
        record_id: String,
        with_object: Option<bool>,
    ) -> std::result::Result<Option<MailboxRecordWithObject>, RPCErrors> {
        let owner = Self::owner_from_record_id(&record_id)?;
        let record = self.msg_box_db.get_record(&owner, &record_id).await?;
        let Some(record) = record else {
            return Ok(None);
        };
        let with_object_record = Self::build_record_view(record, with_object).await?;
        Ok(Some(with_object_record))
    }

    async fn get_message_internal(
        &self,
        msg_id: ObjId,
    ) -> std::result::Result<Option<MsgObject>, RPCErrors> {
        if let Some(msg) =
            self.with_state_read(|state| Ok(state.messages.get(&msg_id.to_string()).cloned()))?
        {
            return Ok(Some(msg));
        }

        let runtime = get_buckyos_api_runtime()?;
        let named_store = runtime.get_named_store().await?;
        match named_store.get_object(&msg_id).await {
            Ok(msg_json) => {
                let msg = serde_json::from_str::<MsgObject>(&msg_json).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "parse message {} from named_store failed: {}",
                        msg_id.to_string(),
                        error
                    ))
                })?;
                Ok(Some(msg))
            }
            Err(error) => {
                let error_text = error.to_string().to_ascii_lowercase();
                if error_text.contains("notfound") || error_text.contains("not found") {
                    Ok(None)
                } else {
                    Err(RPCErrors::ReasonError(format!(
                        "load message {} from named_store failed: {}",
                        msg_id.to_string(),
                        error
                    )))
                }
            }
        }
    }

    async fn update_ui_session_state_internal(
        &self,
        session_id: String,
        key: String,
        value: Value,
    ) -> std::result::Result<UiSessionStateEntry, RPCErrors> {
        let session_id = Self::normalize_ui_session_arg("session_id", &session_id)?;
        let key = Self::normalize_ui_session_arg("key", &key)?;
        self.msg_box_db
            .upsert_ui_session_state(&session_id, &key, &value, Self::now_ms())
            .await
    }

    async fn get_ui_session_state_internal(
        &self,
        session_id: String,
        key: String,
    ) -> std::result::Result<Option<UiSessionStateEntry>, RPCErrors> {
        let session_id = Self::normalize_ui_session_arg("session_id", &session_id)?;
        let key = Self::normalize_ui_session_arg("key", &key)?;
        self.msg_box_db
            .get_ui_session_state(&session_id, &key)
            .await
    }

    async fn list_ui_session_state_internal(
        &self,
        session_id: String,
    ) -> std::result::Result<Vec<UiSessionStateEntry>, RPCErrors> {
        let session_id = Self::normalize_ui_session_arg("session_id", &session_id)?;
        self.msg_box_db.list_ui_session_state(&session_id).await
    }
}

#[async_trait]
impl MsgCenterHandler for MessageCenter {
    async fn handle_dispatch(
        &self,
        msg: MsgObject,
        ingress_ctx: Option<IngressContext>,
        idempotency_key: Option<String>,
        _ctx: RPCContext,
    ) -> std::result::Result<DispatchResult, RPCErrors> {
        self.dispatch_internal(msg, ingress_ctx, idempotency_key)
            .await
    }

    async fn handle_post_send(
        &self,
        msg: MsgObject,
        idempotency_key: Option<String>,
        _ctx: RPCContext,
    ) -> std::result::Result<PostSendResult, RPCErrors> {
        self.post_send_internal(msg, idempotency_key).await
    }

    async fn handle_get_next(
        &self,
        owner: DID,
        box_kind: MailboxKind,
        state_filter: Option<Vec<RecipientState>>,
        lock_on_take: Option<bool>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<MailboxRecordWithObject>, RPCErrors> {
        self.get_next_internal(owner, box_kind, state_filter, lock_on_take, with_object)
            .await
    }

    async fn handle_get_next_delivery(
        &self,
        transport_did: DID,
        lock_on_take: Option<bool>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<DeliveryRecordWithObject>, RPCErrors> {
        self.get_next_delivery_internal(transport_did, lock_on_take, with_object)
            .await
    }

    async fn handle_peek_box(
        &self,
        owner: DID,
        box_kind: MailboxKind,
        state_filter: Option<Vec<RecipientState>>,
        limit: Option<usize>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<MailboxRecordWithObject>, RPCErrors> {
        self.peek_box_internal(owner, box_kind, state_filter, limit, with_object)
            .await
    }

    async fn handle_list_box_by_time(
        &self,
        owner: DID,
        box_kind: MailboxKind,
        state_filter: Option<Vec<RecipientState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<MailboxRecordPage, RPCErrors> {
        self.list_box_by_time_internal(
            owner,
            box_kind,
            state_filter,
            limit,
            cursor_sort_key,
            cursor_record_id,
            descending,
            with_object,
        )
        .await
    }

    async fn handle_list_sessions(
        &self,
        owner: DID,
        limit: Option<usize>,
        cursor_updated_at_ms: Option<u64>,
        cursor_session_id: Option<String>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<SessionSummaryPage, RPCErrors> {
        self.list_sessions_internal(
            owner,
            limit,
            cursor_updated_at_ms,
            cursor_session_id,
            with_object,
        )
        .await
    }

    async fn handle_list_session(
        &self,
        owner: DID,
        session_id: String,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<SessionMessagePage, RPCErrors> {
        self.list_session_internal(
            owner,
            session_id,
            limit,
            cursor_sort_key,
            cursor_record_id,
            descending,
            with_object,
        )
        .await
    }

    async fn handle_update_record_state(
        &self,
        record_id: String,
        new_state: RecipientState,
        _ctx: RPCContext,
    ) -> std::result::Result<MailboxRecord, RPCErrors> {
        self.update_record_state_internal(record_id, new_state)
            .await
    }

    async fn handle_update_record_session(
        &self,
        record_id: String,
        session_id: String,
        _ctx: RPCContext,
    ) -> std::result::Result<MailboxRecord, RPCErrors> {
        self.update_record_session_internal(record_id, session_id)
            .await
    }

    async fn handle_report_delivery(
        &self,
        delivery_id: String,
        result_payload: DeliveryReportResult,
        _ctx: RPCContext,
    ) -> std::result::Result<DeliveryRecord, RPCErrors> {
        self.report_delivery_internal(delivery_id, result_payload)
            .await
    }

    async fn handle_set_read_state(
        &self,
        group_id: DID,
        msg_id: ObjId,
        reader_did: DID,
        status: ReadReceiptState,
        reason: Option<String>,
        at_ms: Option<u64>,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgReceiptObj, RPCErrors> {
        self.set_read_state_internal(group_id, msg_id, reader_did, status, reason, at_ms)
            .await
    }

    async fn handle_list_read_receipts(
        &self,
        msg_id: ObjId,
        group_id: Option<DID>,
        reader: Option<DID>,
        limit: Option<usize>,
        offset: Option<u64>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<MsgReceiptObj>, RPCErrors> {
        self.list_read_receipts_internal(msg_id, group_id, reader, limit, offset)
    }

    async fn handle_get_record(
        &self,
        record_id: String,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<MailboxRecordWithObject>, RPCErrors> {
        self.get_record_internal(record_id, with_object).await
    }

    async fn handle_get_message(
        &self,
        msg_id: ObjId,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<MsgObject>, RPCErrors> {
        self.get_message_internal(msg_id).await
    }

    async fn handle_update_ui_session_state(
        &self,
        session_id: String,
        key: String,
        value: Value,
        _ctx: RPCContext,
    ) -> std::result::Result<UiSessionStateEntry, RPCErrors> {
        self.update_ui_session_state_internal(session_id, key, value)
            .await
    }

    async fn handle_get_ui_session_state(
        &self,
        session_id: String,
        key: String,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<UiSessionStateEntry>, RPCErrors> {
        self.get_ui_session_state_internal(session_id, key).await
    }

    async fn handle_list_ui_session_state(
        &self,
        session_id: String,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<UiSessionStateEntry>, RPCErrors> {
        self.list_ui_session_state_internal(session_id).await
    }

    async fn handle_resolve_did(
        &self,
        platform: String,
        account_id: String,
        profile_hint: Option<Value>,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<DID, RPCErrors> {
        self.contact_mgr
            .resolve_did(platform, account_id, profile_hint, contact_mgr_owner)
            .await
    }

    async fn handle_resolve_endpoint_did(
        &self,
        platform: String,
        account_id: String,
        account_type: String,
        tunnel_instance_id: String,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<DID, RPCErrors> {
        self.contact_mgr
            .resolve_endpoint_did(
                platform,
                account_id,
                account_type,
                tunnel_instance_id,
                contact_mgr_owner,
            )
            .await
    }

    async fn handle_resolve_target(
        &self,
        contact_did: DID,
        selector: String,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<DID, RPCErrors> {
        self.contact_mgr
            .resolve_target(contact_did, selector, contact_mgr_owner)
            .await
    }

    async fn handle_resolve_contact_for_endpoint(
        &self,
        endpoint_did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<DID>, RPCErrors> {
        self.contact_mgr
            .resolve_contact_for_endpoint(endpoint_did, contact_mgr_owner)
            .await
    }

    async fn handle_resolve_canonical_did(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<DID, RPCErrors> {
        self.contact_mgr
            .resolve_canonical_did(did, contact_mgr_owner)
            .await
    }

    async fn handle_list_alias_dids(
        &self,
        canonical_did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<DID>, RPCErrors> {
        self.contact_mgr
            .list_alias_dids(canonical_did, contact_mgr_owner)
            .await
    }

    async fn handle_get_preferred_binding(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<AccountBinding, RPCErrors> {
        self.contact_mgr
            .get_preferred_binding(did, contact_mgr_owner)
            .await
    }

    async fn handle_check_access_permission(
        &self,
        did: DID,
        context_id: Option<String>,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<AccessDecision, RPCErrors> {
        self.contact_mgr
            .check_access_permission(did, context_id, contact_mgr_owner)
            .await
    }

    async fn handle_grant_temporary_access(
        &self,
        dids: Vec<DID>,
        context_id: String,
        duration_secs: u64,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<GrantTemporaryAccessResult, RPCErrors> {
        self.contact_mgr
            .grant_temporary_access(dids, context_id, duration_secs, contact_mgr_owner)
            .await
    }

    async fn handle_block_contact(
        &self,
        did: DID,
        reason: Option<String>,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        self.contact_mgr
            .block_contact(did, reason, contact_mgr_owner)
            .await
    }

    async fn handle_import_contacts(
        &self,
        contacts: Vec<ImportContactEntry>,
        upgrade_to_friend: Option<bool>,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<ImportReport, RPCErrors> {
        self.contact_mgr
            .import_contacts(contacts, upgrade_to_friend, contact_mgr_owner)
            .await
    }

    async fn handle_merge_contacts(
        &self,
        target_did: DID,
        source_did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Contact, RPCErrors> {
        self.contact_mgr
            .merge_contacts(target_did, source_did, contact_mgr_owner)
            .await
    }

    async fn handle_update_contact(
        &self,
        did: DID,
        patch: ContactPatch,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Contact, RPCErrors> {
        self.contact_mgr
            .update_contact(did, patch, contact_mgr_owner)
            .await
    }

    async fn handle_get_contact(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<Contact>, RPCErrors> {
        self.contact_mgr.get_contact(did, contact_mgr_owner).await
    }

    async fn handle_list_contacts(
        &self,
        query: ContactQuery,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<Contact>, RPCErrors> {
        self.contact_mgr
            .list_contacts(query, contact_mgr_owner)
            .await
    }

    async fn handle_get_group_subscribers(
        &self,
        group_id: DID,
        limit: Option<usize>,
        offset: Option<u64>,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<DID>, RPCErrors> {
        self.contact_mgr
            .get_group_subscribers(group_id, limit, offset, contact_mgr_owner)
            .await
    }

    async fn handle_set_group_subscribers(
        &self,
        group_id: DID,
        subscribers: Vec<DID>,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<SetGroupSubscribersResult, RPCErrors> {
        self.contact_mgr
            .set_group_subscribers(group_id, subscribers, contact_mgr_owner)
            .await
    }

    // -------------------------------------------------------------------
    // Self-host group RPC bridge — see `GroupMgr` for behaviour notes.
    // -------------------------------------------------------------------

    async fn handle_group_create(
        &self,
        req: GroupCreateReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        self.group_mgr.create_group(req).await
    }

    async fn handle_group_get_doc(
        &self,
        req: GroupGetDocReq,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<GroupDoc>, RPCErrors> {
        self.group_mgr.get_group_doc(req).await
    }

    async fn handle_group_update_profile(
        &self,
        req: GroupUpdateProfileReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        self.group_mgr.update_group_profile(req).await
    }

    async fn handle_group_invite_member(
        &self,
        req: GroupInviteMemberReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        self.group_mgr.invite_member(req).await
    }

    async fn handle_group_submit_member_proof(
        &self,
        req: GroupSubmitMemberProofReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        self.group_mgr.submit_member_proof(req).await
    }

    async fn handle_group_request_join(
        &self,
        req: GroupRequestJoinReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        self.group_mgr.request_join(req).await
    }

    async fn handle_group_approve_member(
        &self,
        req: GroupApproveMemberReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        self.group_mgr.approve_member(req).await
    }

    async fn handle_group_reject_member(
        &self,
        req: GroupRejectMemberReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        self.group_mgr.reject_member(req).await
    }

    async fn handle_group_remove_member(
        &self,
        req: GroupRemoveMemberReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        self.group_mgr.remove_member(req).await
    }

    async fn handle_group_update_member_role(
        &self,
        req: GroupUpdateMemberRoleReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupMemberRecord, RPCErrors> {
        self.group_mgr.update_member_role(req).await
    }

    async fn handle_group_list_members(
        &self,
        req: GroupListMembersReq,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<GroupMemberRecord>, RPCErrors> {
        self.group_mgr.list_members(req).await
    }

    async fn handle_group_create_subgroup(
        &self,
        req: GroupCreateSubgroupReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupSubgroup, RPCErrors> {
        self.group_mgr.create_subgroup(req).await
    }

    async fn handle_group_update_subgroup(
        &self,
        req: GroupUpdateSubgroupReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupSubgroup, RPCErrors> {
        self.group_mgr.update_subgroup(req).await
    }

    async fn handle_group_list_subgroups(
        &self,
        req: GroupListSubgroupsReq,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<GroupSubgroup>, RPCErrors> {
        self.group_mgr.list_subgroups(req).await
    }

    async fn handle_group_update_collection_policy(
        &self,
        req: GroupUpdateCollectionPolicyReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        self.group_mgr.update_collection_policy(req).await
    }

    async fn handle_group_update_attribution_policy(
        &self,
        req: GroupUpdateAttributionPolicyReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupDoc, RPCErrors> {
        self.group_mgr.update_attribution_policy(req).await
    }

    async fn handle_group_expand_members(
        &self,
        req: GroupExpandMembersReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupExpansionSnapshot, RPCErrors> {
        self.group_mgr.expand_group_members(req).await
    }

    async fn handle_group_list_by_member(
        &self,
        req: GroupListByMemberReq,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<GroupSummary>, RPCErrors> {
        self.group_mgr.list_groups_by_member(req).await
    }

    async fn handle_group_list_parents(
        &self,
        req: GroupListParentsReq,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<GroupSummary>, RPCErrors> {
        self.group_mgr.list_parent_groups(req).await
    }

    async fn handle_group_check_access(
        &self,
        req: GroupCheckAccessReq,
        _ctx: RPCContext,
    ) -> std::result::Result<GroupAccessDecision, RPCErrors> {
        self.group_mgr.check_group_access(req).await
    }
}
