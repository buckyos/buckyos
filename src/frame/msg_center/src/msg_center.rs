use crate::contact_mgr::{ContactMgr, ZoneUserContactSeed};
use crate::msg_box_db::MsgBoxDbMgr;
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, AccessDecision, AccessGroupLevel, AccountBinding, BoxKind, Contact,
    ContactPatch, ContactQuery, DeliveryInfo, DeliveryReportResult, DispatchResult,
    GrantTemporaryAccessResult, ImportContactEntry, ImportReport, IngressContext, KEventClient,
    MsgCenterHandler, MsgReceiptObj, MsgRecord, MsgRecordPage, MsgRecordWithObject, MsgState,
    PostSendDelivery, PostSendResult, ReadReceiptState, RouteInfo, SendContext,
    SetGroupSubscribersResult,
};
use kRPC::{RPCContext, RPCErrors};
use log::{info, warn};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, NamedObject, ObjId};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

const DEFAULT_PEEK_LIMIT: usize = 20;
const MAX_PEEK_LIMIT: usize = 200;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;
const DEFAULT_READ_RECEIPT_LIMIT: usize = 100;
const MAX_READ_RECEIPT_LIMIT: usize = 1000;
const MAX_DELIVERY_RETRY: u32 = 5;
const DEFAULT_FALLBACK_TUNNEL_SUBJECT: &str = "msg-center-default-tunnel";
const MSG_CENTER_BOX_CHANGED_EVENT_NAME: &str = "changed";

#[derive(Debug, Default)]
struct MessageCenterState {
    messages: HashMap<String, MsgObject>,
    receipts: HashMap<String, MsgReceiptObj>,
    dispatch_idempotency: HashMap<String, DispatchResult>,
    post_send_idempotency: HashMap<String, PostSendResult>,
}

#[derive(Clone, Debug)]
struct DeliveryPlan {
    tunnel_did: DID,
    route: RouteInfo,
    target_did: Option<DID>,
    mode: Option<String>,
    priority: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct MessageCenter {
    state: Arc<RwLock<MessageCenterState>>,
    contact_mgr: ContactMgr,
    msg_box_db: MsgBoxDbMgr,
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
        Ok(Self {
            state: Arc::new(RwLock::new(MessageCenterState::default())),
            contact_mgr,
            msg_box_db,
        })
    }

    pub async fn upsert_zone_user_contacts(
        &self,
        contacts: Vec<ZoneUserContactSeed>,
        owner: Option<DID>,
    ) -> std::result::Result<usize, RPCErrors> {
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

    fn box_kind_name(box_kind: &BoxKind) -> &'static str {
        match box_kind {
            BoxKind::Inbox => "INBOX",
            BoxKind::Outbox => "OUTBOX",
            BoxKind::GroupInbox => "GROUP_INBOX",
            BoxKind::TunnelOutbox => "TUNNEL_OUTBOX",
            BoxKind::RequestBox => "REQUEST_BOX",
        }
    }

    fn box_event_name(box_kind: &BoxKind) -> &'static str {
        match box_kind {
            BoxKind::Inbox => "in",
            BoxKind::Outbox => "out",
            BoxKind::GroupInbox => "group_in",
            BoxKind::TunnelOutbox => "tunnel_out",
            BoxKind::RequestBox => "request",
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

    fn build_box_changed_event_id(owner: &DID, box_kind: &BoxKind) -> String {
        let owner_token = owner.to_raw_host_name();
        format!(
            "/msg_center/{}/box/{}/{}",
            owner_token,
            Self::box_event_name(box_kind),
            MSG_CENTER_BOX_CHANGED_EVENT_NAME
        )
    }

    fn publish_box_changed_event(record: &MsgRecord, operation: &str) {
        let owner = match Self::owner_from_record_id(&record.record_id) {
            Ok(owner) => owner,
            Err(error) => {
                warn!(
                    "skip msg_center box changed event with invalid record_id: record_id={}, err={}",
                    record.record_id, error
                );
                return;
            }
        };
        let event_id = Self::build_box_changed_event_id(&owner, &record.box_kind);
        let payload = json!({
            "operation": operation,
            "owner": owner.to_string(),
            "box_kind": Self::box_kind_name(&record.box_kind),
            "box_name": Self::box_event_name(&record.box_kind),
            "record_id": record.record_id.clone(),
            "msg_id": record.msg_id.to_string(),
            "state": record.state.clone(),
            "updated_at_ms": record.updated_at_ms,
        });
        let client = Self::get_kevent_client();

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            info!(
                "msg_center.publish_box_changed_event_begin: operation={} event_id={} record_id={}",
                operation, event_id, record.record_id
            );
            handle.spawn(async move {
                if let Err(err) = client.pub_event(&event_id, payload).await {
                    warn!(
                        "publish msg_center box changed event failed: event_id={}, err={:?}",
                        event_id, err
                    );
                } else {
                    info!(
                        "msg_center.publish_box_changed_event_ok: event_id={}",
                        event_id
                    );
                }
            });
        } else {
            warn!(
                "skip msg_center box changed event without tokio runtime: event_id={}",
                event_id
            );
        }
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

    fn group_did_from_message(msg: &MsgObject) -> DID {
        // New MsgObject semantics: group message uses `to` as group DID.
        // Keep `from` fallback for backward compatibility with old persisted records.
        msg.to.first().cloned().unwrap_or_else(|| msg.from.clone())
    }

    fn route_from_ingress(ingress_ctx: Option<&IngressContext>) -> Option<RouteInfo> {
        let Some(ctx) = ingress_ctx else {
            return None;
        };

        let route = RouteInfo {
            tunnel_did: ctx.tunnel_did.clone(),
            platform: ctx.platform.clone(),
            account_id: ctx.source_account_id.clone(),
            address: None,
            chat_id: ctx.chat_id.clone(),
            target_did: None,
            mode: Some("ingress".to_string()),
            priority: None,
            ext_ids: HashMap::new(),
            extra: ctx.extra.clone(),
        };

        let is_empty = route.tunnel_did.is_none()
            && route.platform.is_none()
            && route.account_id.is_none()
            && route.address.is_none()
            && route.chat_id.is_none()
            && route.target_did.is_none()
            && route.mode.is_none()
            && route.priority.is_none()
            && route.ext_ids.is_empty()
            && route.extra.is_none();
        if is_empty {
            None
        } else {
            Some(route)
        }
    }

    fn normalize_non_empty(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
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

    fn parse_or_build_did(raw: &str, fallback_prefix: &str) -> DID {
        if let Ok(did) = DID::from_str(raw) {
            return did;
        }
        let subject = format!(
            "{}-{}",
            Self::sanitize_token(fallback_prefix),
            Self::sanitize_token(raw)
        );
        DID::new("bns", &subject)
    }

    fn default_tunnel_did() -> DID {
        DID::new("bns", DEFAULT_FALLBACK_TUNNEL_SUBJECT)
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

    fn build_record_id(owner: &DID, box_kind: &BoxKind, msg_id: &ObjId, variant: &str) -> String {
        format!(
            "{}|{}|{}|{}",
            owner.to_string(),
            Self::box_kind_name(box_kind),
            msg_id.to_string(),
            Self::sanitize_token(variant)
        )
    }

    async fn create_or_get_record(
        &self,
        owner: DID,
        box_kind: BoxKind,
        msg: &MsgObject,
        initial_state: MsgState,
        route: Option<RouteInfo>,
        delivery: Option<DeliveryInfo>,
        tags: Vec<String>,
        variant: &str,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        let msg_id = Self::message_obj_id(msg);
        let record_id = Self::build_record_id(&owner, &box_kind, &msg_id, variant);
        let ui_session_id = Self::normalize_non_empty(msg.thread.topic.as_deref())
            .or_else(|| Self::extract_record_session_id(msg));
        if let Some(existing) = self.msg_box_db.get_record(&owner, &record_id).await? {
            let mut record_for_update = existing.clone();
            if record_for_update.msg_kind != msg.kind {
                record_for_update.msg_kind = msg.kind;
            }
            if record_for_update.ui_session_id.is_none() {
                record_for_update.ui_session_id = ui_session_id;
            }
            self.msg_box_db
                .upsert_record_with_msg(&record_for_update, Some(msg))
                .await?;
            Self::publish_box_changed_event(&record_for_update, "upsert");
            return Ok(record_for_update);
        }

        let now_ms = Self::now_ms();
        let record_to = match box_kind {
            BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => owner.clone(),
            BoxKind::Outbox | BoxKind::TunnelOutbox => {
                msg.to.first().cloned().unwrap_or_else(|| owner.clone())
            }
        };
        let record = MsgRecord {
            record_id: record_id.clone(),
            box_kind: box_kind.clone(),
            msg_id: msg_id.clone(),
            msg_kind: msg.kind,
            state: initial_state,
            from: msg.from.clone(),
            from_name: None,
            to: record_to,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            route,
            delivery,
            ui_session_id,
            sort_key: if msg.created_at_ms > 0 {
                msg.created_at_ms
            } else {
                now_ms
            },
            tags,
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
        box_kind: &BoxKind,
        state_filter: Option<&[MsgState]>,
        descending: bool,
    ) -> std::result::Result<Vec<MsgRecord>, RPCErrors> {
        self.msg_box_db
            .list_records(owner, box_kind, state_filter, descending)
            .await
    }

    fn filter_after_cursor(
        records: Vec<MsgRecord>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<&str>,
        descending: bool,
    ) -> Vec<MsgRecord> {
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
        record: MsgRecord,
        with_object: Option<bool>,
    ) -> std::result::Result<MsgRecordWithObject, RPCErrors> {
        let mut result = MsgRecordWithObject { record, msg: None };
        if with_object.unwrap_or(false) {
            result.msg = Some(Self::load_message(&result.record.msg_id).await?);
        }
        Ok(result)
    }

    fn next_state_on_take(box_kind: &BoxKind, state: &MsgState) -> Option<MsgState> {
        match (box_kind, state) {
            (BoxKind::Inbox, MsgState::Unread)
            | (BoxKind::GroupInbox, MsgState::Unread)
            | (BoxKind::RequestBox, MsgState::Unread) => Some(MsgState::Reading),
            (BoxKind::TunnelOutbox, MsgState::Wait) => Some(MsgState::Sending),
            _ => None,
        }
    }

    fn is_valid_transition(box_kind: &BoxKind, current: &MsgState, next: &MsgState) -> bool {
        if current == next {
            return true;
        }
        if matches!(next, MsgState::Deleted | MsgState::Archived) {
            return true;
        }

        match box_kind {
            BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => match current {
                MsgState::Unread => matches!(next, MsgState::Reading | MsgState::Readed),
                MsgState::Reading => matches!(next, MsgState::Unread | MsgState::Readed),
                MsgState::Readed => matches!(next, MsgState::Reading),
                _ => false,
            },
            BoxKind::Outbox => match current {
                MsgState::Sent => false,
                _ => false,
            },
            BoxKind::TunnelOutbox => match current {
                MsgState::Wait => {
                    matches!(next, MsgState::Sending | MsgState::Failed | MsgState::Dead)
                }
                MsgState::Sending => {
                    matches!(
                        next,
                        MsgState::Sent | MsgState::Failed | MsgState::Wait | MsgState::Dead
                    )
                }
                MsgState::Failed => {
                    matches!(next, MsgState::Wait | MsgState::Dead | MsgState::Sending)
                }
                MsgState::Dead => matches!(next, MsgState::Wait),
                MsgState::Sent => false,
                _ => false,
            },
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
    ) -> std::result::Result<Option<BoxKind>, RPCErrors> {
        let decision: AccessDecision = self
            .contact_mgr
            .check_access_permission(sender.clone(), context_id, Some(target.clone()))
            .await?;
        if decision.allow_delivery {
            return Ok(Some(BoxKind::Inbox));
        }

        let target_box = decision.target_box.to_ascii_uppercase();
        if target_box == "REQUEST_BOX" {
            Ok(Some(BoxKind::RequestBox))
        } else {
            Ok(None)
        }
    }

    async fn build_delivery_plan(
        &self,
        target_did: DID,
        send_ctx: Option<&SendContext>,
        contact_mgr_owner: Option<DID>,
    ) -> DeliveryPlan {
        let preferred_tunnel = send_ctx.and_then(|ctx| ctx.preferred_tunnel.clone());
        let preferred_binding: Option<AccountBinding> = self
            .contact_mgr
            .get_preferred_binding(target_did.clone(), contact_mgr_owner)
            .await
            .ok();

        let mut route = RouteInfo::default();
        route.target_did = Some(target_did.clone());
        route.mode = Some("direct".to_string());
        route.priority = send_ctx.and_then(|ctx| ctx.priority);
        route.extra = send_ctx.and_then(|ctx| ctx.extra.clone());

        if let Some(binding) = preferred_binding {
            route.platform = Some(binding.platform.clone());
            route.account_id = Some(binding.account_id.clone());
            route.address = Some(binding.display_id.clone());
            route.tunnel_did = Some(Self::parse_or_build_did(&binding.tunnel_id, "tunnel"));
        }

        if let Some(tunnel_did) = preferred_tunnel {
            route.tunnel_did = Some(tunnel_did);
        }

        let tunnel_did = route
            .tunnel_did
            .clone()
            .unwrap_or_else(Self::default_tunnel_did);
        route.tunnel_did = Some(tunnel_did.clone());

        DeliveryPlan {
            tunnel_did,
            target_did: Some(target_did),
            mode: route.mode.clone(),
            priority: route.priority,
            route,
        }
    }

    fn list_delivery_targets(msg: &MsgObject) -> Vec<DID> {
        let mut targets = Self::dedupe_dids(msg.to.clone());
        if targets.is_empty() && Self::is_group_message(msg) {
            // Backward compatibility fallback for old group messages.
            targets.push(msg.from.clone());
        }
        targets
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
                ingress_route: Option<RouteInfo>,
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
                ingress_route: Self::route_from_ingress(ingress_ctx.as_ref()),
            })
        })?;

        let (stored_msg, stored_msg_id, stored_msg_json, sender, context_id, ingress_route) =
            match prepared {
                DispatchPrepare::Done(result) => return Ok(result),
                DispatchPrepare::Ready {
                    stored_msg,
                    stored_msg_id,
                    stored_msg_json,
                    sender,
                    context_id,
                    ingress_route,
                } => (
                    stored_msg,
                    stored_msg_id,
                    stored_msg_json,
                    sender,
                    context_id,
                    ingress_route,
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

            let group_id = Self::group_did_from_message(&stored_msg);
            info!(
                "dispatch about to write inbox record: msg_id={}, sender={}, owner={}, box_kind=GROUP_INBOX, context_id={}",
                stored_msg_id.to_string(),
                sender.to_string(),
                group_id.to_string(),
                context_id.as_deref().unwrap_or("-"),
            );
            self.create_or_get_record(
                group_id.clone(),
                BoxKind::GroupInbox,
                &stored_msg,
                MsgState::Unread,
                ingress_route.clone(),
                None,
                Vec::new(),
                "group-inbox",
            )
            .await?;

            let readers = self
                .contact_mgr
                .get_group_subscribers(
                    group_id.clone(),
                    None,
                    None,
                    ingress_contact_mgr_owner.clone(),
                )
                .await?;
            let readers = Self::dedupe_dids(readers);
            for agent_did in readers.iter() {
                let tag = format!("group:{}", group_id.to_string());
                info!(
                    "dispatch about to write inbox record: msg_id={}, sender={}, owner={}, box_kind=INBOX, context_id={}",
                    stored_msg_id.to_string(),
                    sender.to_string(),
                    agent_did.to_string(),
                    context_id.as_deref().unwrap_or("-"),
                );
                self.create_or_get_record(
                    agent_did.clone(),
                    BoxKind::Inbox,
                    &stored_msg,
                    MsgState::Unread,
                    ingress_route.clone(),
                    None,
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
                        "dispatch blocked by sender access policy: msg_id={}, sender={}, recipient={}, context_id={}, contact_mgr_owner={}",
                        stored_msg_id.to_string(),
                        sender.to_string(),
                        recipient.to_string(),
                        context_id.as_deref().unwrap_or("-"),
                        recipient.to_string(),
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
                        if box_kind == BoxKind::Inbox {
                            info!(
                                "dispatch about to write inbox record: msg_id={}, sender={}, owner={}, box_kind=INBOX, context_id={}",
                                stored_msg_id.to_string(),
                                sender.to_string(),
                                recipient.to_string(),
                                context_id.as_deref().unwrap_or("-"),
                            );
                        } else if box_kind == BoxKind::RequestBox {
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
                            MsgState::Unread,
                            ingress_route.clone(),
                            None,
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
        send_ctx: Option<SendContext>,
        idempotency_key: Option<String>,
    ) -> std::result::Result<PostSendResult, RPCErrors> {
        let send_contact_mgr_owner = send_ctx
            .as_ref()
            .and_then(|ctx| ctx.contact_mgr_owner.clone());

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
            let contact_mgr_owner = send_contact_mgr_owner
                .clone()
                .or_else(|| Some(author.clone()));

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

        if self
            .is_contact_blocked(&author, contact_mgr_owner.clone())
            .await?
        {
            let result = PostSendResult {
                ok: false,
                msg_id: stored_msg_id.clone(),
                deliveries: Vec::new(),
                reason: Some("blocked_author".to_string()),
            };
            if let Some(key) = idempotency_key.as_ref() {
                self.with_state_write(|state| {
                    state
                        .post_send_idempotency
                        .insert(key.clone(), result.clone());
                    Ok(())
                })?;
            }
            return Ok(result);
        }

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
            BoxKind::Outbox,
            &stored_msg,
            MsgState::Sent,
            None,
            None,
            Vec::new(),
            "owner-outbox",
        )
        .await?;

        let delivery_targets = Self::list_delivery_targets(&stored_msg);
        let mut deliveries = Vec::with_capacity(delivery_targets.len());
        for target in delivery_targets {
            let plan = self
                .build_delivery_plan(target.clone(), send_ctx.as_ref(), contact_mgr_owner.clone())
                .await;
            let variant = format!(
                "{}-{}-{}-{}",
                plan.tunnel_did.to_string(),
                plan.target_did
                    .as_ref()
                    .map(|did| did.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                plan.route
                    .account_id
                    .clone()
                    .unwrap_or_else(|| "none".to_string()),
                plan.route
                    .mode
                    .clone()
                    .unwrap_or_else(|| "direct".to_string())
            );
            let record = self
                .create_or_get_record(
                    plan.tunnel_did.clone(),
                    BoxKind::TunnelOutbox,
                    &stored_msg,
                    MsgState::Wait,
                    Some(plan.route.clone()),
                    Some(DeliveryInfo::default()),
                    Vec::new(),
                    &variant,
                )
                .await?;

            deliveries.push(PostSendDelivery {
                tunnel_did: plan.tunnel_did,
                record_id: record.record_id,
                target_did: plan.target_did,
                mode: plan.mode,
                priority: plan.priority,
            });
        }

        let result = PostSendResult {
            ok: true,
            msg_id: stored_msg_id.clone(),
            deliveries,
            reason: None,
        };
        if let Some(key) = idempotency_key.as_ref() {
            self.with_state_write(|state| {
                state
                    .post_send_idempotency
                    .insert(key.clone(), result.clone());
                Ok(())
            })?;
        }
        Ok(result)
    }

    async fn get_next_internal(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        lock_on_take: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        let default_filter = match box_kind {
            BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => {
                Some(vec![MsgState::Unread])
            }
            BoxKind::TunnelOutbox => Some(vec![MsgState::Wait]),
            BoxKind::Outbox => None,
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

    async fn peek_box_internal(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        with_object: Option<bool>,
    ) -> std::result::Result<Vec<MsgRecordWithObject>, RPCErrors> {
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
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<MsgRecordPage, RPCErrors> {
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

        Ok(MsgRecordPage {
            items,
            next_cursor_sort_key,
            next_cursor_record_id,
        })
    }

    async fn update_record_state_internal(
        &self,
        record_id: String,
        new_state: MsgState,
        reason: Option<String>,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
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
        if let Some(reason) = reason {
            let trimmed = reason.trim();
            if !trimmed.is_empty() {
                let mut delivery = record.delivery.clone().unwrap_or_default();
                delivery.last_error = Some(trimmed.to_string());
                record.delivery = Some(delivery);
            }
        }
        self.msg_box_db.upsert_record(&record).await?;
        Self::publish_box_changed_event(&record, "state");
        Ok(record)
    }

    async fn update_record_session_internal(
        &self,
        record_id: String,
        session_id: String,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
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

        if record.ui_session_id.as_deref() == Some(session_id) {
            return Ok(record);
        }

        record.ui_session_id = Some(session_id.to_string());
        record.updated_at_ms = Self::now_ms();
        self.msg_box_db.upsert_record(&record).await?;
        Self::publish_box_changed_event(&record, "session");
        Ok(record)
    }

    async fn report_delivery_internal(
        &self,
        record_id: String,
        result_payload: DeliveryReportResult,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        let owner = Self::owner_from_record_id(&record_id)?;
        let mut record = self
            .msg_box_db
            .get_record(&owner, &record_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("record {} not found", record_id)))?;
        if record.box_kind != BoxKind::TunnelOutbox {
            return Err(RPCErrors::ReasonError(format!(
                "record {} is not tunnel outbox",
                record_id
            )));
        }

        let now_ms = Self::now_ms();
        let mut delivery = record.delivery.clone().unwrap_or_default();
        delivery.attempts = delivery.attempts.saturating_add(1);
        delivery.external_msg_id = result_payload.external_msg_id.clone();
        delivery.delivered_at_ms = result_payload.delivered_at_ms;
        delivery.error_code = result_payload.error_code.clone();
        delivery.error_message = result_payload.error_message.clone();
        delivery.retry_after_ms = result_payload.retry_after_ms;
        if let Some(error_message) = result_payload.error_message.as_ref() {
            if !error_message.trim().is_empty() {
                delivery.last_error = Some(error_message.clone());
            }
        }
        if let Some(retry_after_ms) = result_payload.retry_after_ms {
            delivery.next_retry_at_ms = Some(now_ms.saturating_add(retry_after_ms));
        }

        if result_payload.ok {
            record.state = MsgState::Sent;
            if delivery.delivered_at_ms.is_none() {
                delivery.delivered_at_ms = Some(now_ms);
            }
        } else {
            let retryable = result_payload.retryable.unwrap_or(true);
            let too_many_attempts = delivery.attempts >= MAX_DELIVERY_RETRY;
            if retryable && !too_many_attempts {
                record.state = MsgState::Wait;
            } else {
                record.state = MsgState::Dead;
            }
        }

        record.updated_at_ms = now_ms;
        record.delivery = Some(delivery);
        self.msg_box_db.upsert_record(&record).await?;
        Self::publish_box_changed_event(&record, "delivery");
        Ok(record)
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
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
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
        send_ctx: Option<SendContext>,
        idempotency_key: Option<String>,
        _ctx: RPCContext,
    ) -> std::result::Result<PostSendResult, RPCErrors> {
        self.post_send_internal(msg, send_ctx, idempotency_key)
            .await
    }

    async fn handle_get_next(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        lock_on_take: Option<bool>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        self.get_next_internal(owner, box_kind, state_filter, lock_on_take, with_object)
            .await
    }

    async fn handle_peek_box(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<MsgRecordWithObject>, RPCErrors> {
        self.peek_box_internal(owner, box_kind, state_filter, limit, with_object)
            .await
    }

    async fn handle_list_box_by_time(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgRecordPage, RPCErrors> {
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

    async fn handle_update_record_state(
        &self,
        record_id: String,
        new_state: MsgState,
        reason: Option<String>,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        self.update_record_state_internal(record_id, new_state, reason)
            .await
    }

    async fn handle_update_record_session(
        &self,
        record_id: String,
        session_id: String,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        self.update_record_session_internal(record_id, session_id)
            .await
    }

    async fn handle_report_delivery(
        &self,
        record_id: String,
        result_payload: DeliveryReportResult,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        self.report_delivery_internal(record_id, result_payload)
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
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        self.get_record_internal(record_id, with_object).await
    }

    async fn handle_get_message(
        &self,
        msg_id: ObjId,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<MsgObject>, RPCErrors> {
        self.get_message_internal(msg_id).await
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
}
