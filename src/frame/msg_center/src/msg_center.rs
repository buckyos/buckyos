use crate::contact_mgr::ContactMgr;
use async_trait::async_trait;
use buckyos_api::{
    AccessDecision, AccessGroupLevel, AccountBinding, BoxKind, Contact, ContactPatch, ContactQuery,
    DeliveryInfo, DeliveryReportResult, DispatchResult, GrantTemporaryAccessResult,
    ImportContactEntry, ImportReport, IngressContext, MsgCenterHandler, MsgObject, MsgReceiptObj,
    MsgRecord, MsgRecordPage, MsgRecordWithObject, MsgState, PostSendDelivery, PostSendResult,
    ReadReceiptState, RouteInfo, SendContext, SetGroupSubscribersResult,
};
use kRPC::{RPCContext, RPCErrors};
use name_lib::DID;
use ndn_lib::ObjId;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

const DEFAULT_PEEK_LIMIT: usize = 20;
const MAX_PEEK_LIMIT: usize = 200;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;
const DEFAULT_READ_RECEIPT_LIMIT: usize = 100;
const MAX_READ_RECEIPT_LIMIT: usize = 1000;
const MAX_DELIVERY_RETRY: u32 = 5;
const DEFAULT_FALLBACK_TUNNEL_SUBJECT: &str = "msg-center-default-tunnel";

#[derive(Debug, Default)]
struct MessageCenterState {
    messages: HashMap<String, MsgObject>,
    records: HashMap<String, MsgRecord>,
    box_index: HashMap<String, HashSet<String>>,
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
}

impl MessageCenter {
    pub fn new(contact_mgr: ContactMgr) -> Self {
        Self {
            state: Arc::new(RwLock::new(MessageCenterState::default())),
            contact_mgr,
        }
    }

    pub fn try_new() -> std::result::Result<Self, RPCErrors> {
        Ok(Self::new(ContactMgr::new()?))
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

    fn box_index_key(owner: &DID, box_kind: &BoxKind) -> String {
        format!("{}|{}", owner.to_string(), Self::box_kind_name(box_kind))
    }

    fn state_matches(state_filter: Option<&[MsgState]>, state: &MsgState) -> bool {
        match state_filter {
            None => true,
            Some(filters) if filters.is_empty() => true,
            Some(filters) => filters.iter().any(|item| item == state),
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

    fn logical_sender(msg: &MsgObject) -> DID {
        msg.source.clone().unwrap_or_else(|| msg.from.clone())
    }

    fn outbound_author(msg: &MsgObject) -> DID {
        msg.source.clone().unwrap_or_else(|| msg.from.clone())
    }

    fn is_group_message(msg: &MsgObject) -> bool {
        msg.source.is_some()
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

    fn ensure_message(state: &mut MessageCenterState, msg: MsgObject) -> MsgObject {
        let msg_key = msg.id.to_string();
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

    fn create_or_get_record(
        state: &mut MessageCenterState,
        owner: DID,
        box_kind: BoxKind,
        msg: &MsgObject,
        initial_state: MsgState,
        route: Option<RouteInfo>,
        delivery: Option<DeliveryInfo>,
        tags: Vec<String>,
        variant: &str,
    ) -> MsgRecord {
        let record_id = Self::build_record_id(&owner, &box_kind, &msg.id, variant);
        if let Some(existing) = state.records.get(&record_id) {
            return existing.clone();
        }

        let now_ms = Self::now_ms();
        let record = MsgRecord {
            record_id: record_id.clone(),
            owner: owner.clone(),
            box_kind: box_kind.clone(),
            msg_id: msg.id.clone(),
            state: initial_state,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            route,
            delivery,
            thread_key: msg.thread_key.clone(),
            sort_key: if msg.created_at_ms > 0 {
                msg.created_at_ms
            } else {
                now_ms
            },
            tags,
        };

        state.records.insert(record_id.clone(), record.clone());
        state
            .box_index
            .entry(Self::box_index_key(&owner, &box_kind))
            .or_default()
            .insert(record_id);

        record
    }

    fn collect_box_records(
        state: &MessageCenterState,
        owner: &DID,
        box_kind: &BoxKind,
        state_filter: Option<&[MsgState]>,
    ) -> Vec<MsgRecord> {
        let mut records = Vec::new();
        let index_key = Self::box_index_key(owner, box_kind);
        if let Some(record_ids) = state.box_index.get(&index_key) {
            records.reserve(record_ids.len());
            for record_id in record_ids {
                if let Some(record) = state.records.get(record_id) {
                    if Self::state_matches(state_filter, &record.state) {
                        records.push(record.clone());
                    }
                }
            }
        }
        records
    }

    fn sort_records(records: &mut [MsgRecord], descending: bool) {
        if descending {
            records.sort_by(|left, right| {
                right
                    .sort_key
                    .cmp(&left.sort_key)
                    .then_with(|| right.record_id.cmp(&left.record_id))
            });
        } else {
            records.sort_by(|left, right| {
                left.sort_key
                    .cmp(&right.sort_key)
                    .then_with(|| left.record_id.cmp(&right.record_id))
            });
        }
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

    fn build_record_with_object(
        state: &MessageCenterState,
        record: MsgRecord,
    ) -> std::result::Result<MsgRecordWithObject, RPCErrors> {
        let msg_key = record.msg_id.to_string();
        let msg = state.messages.get(&msg_key).ok_or_else(|| {
            RPCErrors::ReasonError(format!(
                "message object {} not found for record {}",
                msg_key, record.record_id
            ))
        })?;
        Ok(MsgRecordWithObject {
            record,
            msg: msg.clone(),
        })
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

    fn is_contact_blocked(
        &self,
        did: &DID,
        owner: Option<DID>,
    ) -> std::result::Result<bool, RPCErrors> {
        let contact = self.contact_mgr.get_contact(did.clone(), owner)?;
        Ok(contact
            .map(|item| item.access_level == AccessGroupLevel::Block)
            .unwrap_or(false))
    }

    fn decide_inbox_kind(
        &self,
        sender: &DID,
        target: &DID,
        context_id: Option<String>,
    ) -> std::result::Result<Option<BoxKind>, RPCErrors> {
        let decision: AccessDecision = self.contact_mgr.check_access_permission(
            sender.clone(),
            context_id,
            Some(target.clone()),
        )?;
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

    fn build_delivery_plan(&self, target_did: DID, send_ctx: Option<&SendContext>) -> DeliveryPlan {
        let preferred_tunnel = send_ctx.and_then(|ctx| ctx.preferred_tunnel.clone());
        let preferred_binding: Option<AccountBinding> = self
            .contact_mgr
            .get_preferred_binding(target_did.clone(), None)
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
            targets.push(msg.from.clone());
        }
        targets
    }

    fn dispatch_internal(
        &self,
        msg: MsgObject,
        ingress_ctx: Option<IngressContext>,
        idempotency_key: Option<String>,
    ) -> std::result::Result<DispatchResult, RPCErrors> {
        self.with_state_write(|state| {
            if let Some(key) = idempotency_key.as_ref() {
                if let Some(cached) = state.dispatch_idempotency.get(key) {
                    return Ok(cached.clone());
                }
            }

            let stored_msg = Self::ensure_message(state, msg);
            let sender = Self::logical_sender(&stored_msg);
            if self.is_contact_blocked(&sender, None)? {
                let result = DispatchResult {
                    ok: false,
                    msg_id: stored_msg.id.clone(),
                    delivered_recipients: Vec::new(),
                    dropped_recipients: Vec::new(),
                    delivered_group: None,
                    delivered_agents: Vec::new(),
                    reason: Some("blocked".to_string()),
                };
                if let Some(key) = idempotency_key {
                    state.dispatch_idempotency.insert(key, result.clone());
                }
                return Ok(result);
            }

            let context_id = ingress_ctx.as_ref().and_then(|ctx| ctx.context_id.clone());
            let ingress_route = Self::route_from_ingress(ingress_ctx.as_ref());
            let mut result = DispatchResult {
                ok: true,
                msg_id: stored_msg.id.clone(),
                delivered_recipients: Vec::new(),
                dropped_recipients: Vec::new(),
                delivered_group: None,
                delivered_agents: Vec::new(),
                reason: None,
            };

            if Self::is_group_message(&stored_msg) {
                let group_id = stored_msg.from.clone();
                Self::create_or_get_record(
                    state,
                    group_id.clone(),
                    BoxKind::GroupInbox,
                    &stored_msg,
                    MsgState::Unread,
                    ingress_route.clone(),
                    None,
                    Vec::new(),
                    "group-inbox",
                );

                let readers =
                    self.contact_mgr
                        .get_group_subscribers(group_id.clone(), None, None, None)?;
                let readers = Self::dedupe_dids(readers);
                for agent_did in readers.iter() {
                    let tag = format!("group:{}", group_id.to_string());
                    Self::create_or_get_record(
                        state,
                        agent_did.clone(),
                        BoxKind::Inbox,
                        &stored_msg,
                        MsgState::Unread,
                        ingress_route.clone(),
                        None,
                        vec![tag],
                        &format!("group-agent-{}", group_id.to_string()),
                    );
                }

                result.delivered_group = Some(group_id);
                result.delivered_agents = readers;
            } else {
                let recipients = Self::dedupe_dids(stored_msg.to.clone());
                for recipient in recipients {
                    let decision =
                        self.decide_inbox_kind(&sender, &recipient, context_id.clone())?;
                    match decision {
                        Some(box_kind) => {
                            Self::create_or_get_record(
                                state,
                                recipient.clone(),
                                box_kind,
                                &stored_msg,
                                MsgState::Unread,
                                ingress_route.clone(),
                                None,
                                Vec::new(),
                                "inbox",
                            );
                            result.delivered_recipients.push(recipient);
                        }
                        None => {
                            result.dropped_recipients.push(recipient);
                        }
                    }
                }
            }

            if let Some(key) = idempotency_key {
                state.dispatch_idempotency.insert(key, result.clone());
            }
            Ok(result)
        })
    }

    fn post_send_internal(
        &self,
        msg: MsgObject,
        send_ctx: Option<SendContext>,
        idempotency_key: Option<String>,
    ) -> std::result::Result<PostSendResult, RPCErrors> {
        self.with_state_write(|state| {
            if let Some(key) = idempotency_key.as_ref() {
                if let Some(cached) = state.post_send_idempotency.get(key) {
                    return Ok(cached.clone());
                }
            }

            let stored_msg = Self::ensure_message(state, msg);
            let author = Self::outbound_author(&stored_msg);
            if self.is_contact_blocked(&author, None)? {
                let result = PostSendResult {
                    ok: false,
                    msg_id: stored_msg.id.clone(),
                    deliveries: Vec::new(),
                    reason: Some("blocked_author".to_string()),
                };
                if let Some(key) = idempotency_key {
                    state.post_send_idempotency.insert(key, result.clone());
                }
                return Ok(result);
            }

            Self::create_or_get_record(
                state,
                author,
                BoxKind::Outbox,
                &stored_msg,
                MsgState::Sent,
                None,
                None,
                Vec::new(),
                "owner-outbox",
            );

            let delivery_targets = Self::list_delivery_targets(&stored_msg);
            let mut deliveries = Vec::with_capacity(delivery_targets.len());
            for target in delivery_targets {
                let plan = self.build_delivery_plan(target.clone(), send_ctx.as_ref());
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
                let record = Self::create_or_get_record(
                    state,
                    plan.tunnel_did.clone(),
                    BoxKind::TunnelOutbox,
                    &stored_msg,
                    MsgState::Wait,
                    Some(plan.route.clone()),
                    Some(DeliveryInfo::default()),
                    Vec::new(),
                    &variant,
                );

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
                msg_id: stored_msg.id.clone(),
                deliveries,
                reason: None,
            };
            if let Some(key) = idempotency_key {
                state.post_send_idempotency.insert(key, result.clone());
            }
            Ok(result)
        })
    }

    fn get_next_internal(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        lock_on_take: Option<bool>,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        self.with_state_write(|state| {
            let default_filter = match box_kind {
                BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => {
                    Some(vec![MsgState::Unread])
                }
                BoxKind::TunnelOutbox => Some(vec![MsgState::Wait]),
                BoxKind::Outbox => None,
            };
            let effective_filter = state_filter.or(default_filter);
            let state_filter_ref = effective_filter.as_deref();
            let mut records = Self::collect_box_records(state, &owner, &box_kind, state_filter_ref);
            Self::sort_records(&mut records, false);

            let should_lock = lock_on_take.unwrap_or(true);
            for mut record in records {
                if should_lock {
                    if let Some(next_state) = Self::next_state_on_take(&box_kind, &record.state) {
                        if let Some(record_entry) = state.records.get_mut(&record.record_id) {
                            record_entry.state = next_state;
                            record_entry.updated_at_ms = Self::now_ms();
                            record = record_entry.clone();
                        }
                    }
                }

                let with_object = Self::build_record_with_object(state, record)?;
                return Ok(Some(with_object));
            }

            Ok(None)
        })
    }

    fn peek_box_internal(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
    ) -> std::result::Result<Vec<MsgRecordWithObject>, RPCErrors> {
        self.with_state_read(|state| {
            let limit = Self::clamp_limit(limit, DEFAULT_PEEK_LIMIT, MAX_PEEK_LIMIT);
            let state_filter_ref = state_filter.as_deref();
            let mut records = Self::collect_box_records(state, &owner, &box_kind, state_filter_ref);
            Self::sort_records(&mut records, true);

            let mut result = Vec::new();
            for record in records.into_iter().take(limit) {
                result.push(Self::build_record_with_object(state, record)?);
            }
            Ok(result)
        })
    }

    fn list_box_by_time_internal(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
    ) -> std::result::Result<MsgRecordPage, RPCErrors> {
        self.with_state_read(|state| {
            let descending = descending.unwrap_or(true);
            let limit = Self::clamp_limit(limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT);
            let state_filter_ref = state_filter.as_deref();

            let mut records = Self::collect_box_records(state, &owner, &box_kind, state_filter_ref);
            Self::sort_records(&mut records, descending);
            let records = Self::filter_after_cursor(
                records,
                cursor_sort_key,
                cursor_record_id.as_deref(),
                descending,
            );

            let mut items = Vec::new();
            let mut iter = records.into_iter();
            for record in iter.by_ref().take(limit) {
                items.push(Self::build_record_with_object(state, record)?);
            }

            let next_item = iter.next();
            let (next_cursor_sort_key, next_cursor_record_id) = if next_item.is_some() {
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
        })
    }

    fn update_record_state_internal(
        &self,
        record_id: String,
        new_state: MsgState,
        reason: Option<String>,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        self.with_state_write(|state| {
            let record = state
                .records
                .get_mut(&record_id)
                .ok_or_else(|| RPCErrors::ReasonError(format!("record {} not found", record_id)))?;

            if !Self::is_valid_transition(&record.box_kind, &record.state, &new_state) {
                return Err(RPCErrors::ReasonError(format!(
                    "invalid state transition {:?} -> {:?} for {:?}",
                    record.state, new_state, record.box_kind
                )));
            }

            record.state = new_state.clone();
            record.updated_at_ms = Self::now_ms();
            if let Some(reason) = reason {
                let trimmed = reason.trim();
                if !trimmed.is_empty() {
                    let mut delivery = record.delivery.clone().unwrap_or_default();
                    delivery.last_error = Some(trimmed.to_string());
                    record.delivery = Some(delivery);
                }
            }
            Ok(record.clone())
        })
    }

    fn report_delivery_internal(
        &self,
        record_id: String,
        result_payload: DeliveryReportResult,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        self.with_state_write(|state| {
            let record = state
                .records
                .get_mut(&record_id)
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
            Ok(record.clone())
        })
    }

    fn set_read_state_internal(
        &self,
        group_id: DID,
        msg_id: ObjId,
        reader_did: DID,
        status: ReadReceiptState,
        reason: Option<String>,
        at_ms: Option<u64>,
    ) -> std::result::Result<MsgReceiptObj, RPCErrors> {
        self.with_state_write(|state| {
            let msg_key = msg_id.to_string();
            if !state.messages.contains_key(&msg_key) {
                return Err(RPCErrors::ReasonError(format!(
                    "message {} not found",
                    msg_id.to_string()
                )));
            }

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

    fn get_record_internal(
        &self,
        record_id: String,
        _with_object: Option<bool>,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        self.with_state_read(|state| {
            let Some(record) = state.records.get(&record_id).cloned() else {
                return Ok(None);
            };
            Ok(Some(Self::build_record_with_object(state, record)?))
        })
    }

    fn get_message_internal(
        &self,
        msg_id: ObjId,
    ) -> std::result::Result<Option<MsgObject>, RPCErrors> {
        self.with_state_read(|state| Ok(state.messages.get(&msg_id.to_string()).cloned()))
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
    }

    async fn handle_post_send(
        &self,
        msg: MsgObject,
        send_ctx: Option<SendContext>,
        idempotency_key: Option<String>,
        _ctx: RPCContext,
    ) -> std::result::Result<PostSendResult, RPCErrors> {
        self.post_send_internal(msg, send_ctx, idempotency_key)
    }

    async fn handle_get_next(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        lock_on_take: Option<bool>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        self.get_next_internal(owner, box_kind, state_filter, lock_on_take)
    }

    async fn handle_peek_box(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<MsgRecordWithObject>, RPCErrors> {
        self.peek_box_internal(owner, box_kind, state_filter, limit)
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
        )
    }

    async fn handle_update_record_state(
        &self,
        record_id: String,
        new_state: MsgState,
        reason: Option<String>,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        self.update_record_state_internal(record_id, new_state, reason)
    }

    async fn handle_report_delivery(
        &self,
        record_id: String,
        result_payload: DeliveryReportResult,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        self.report_delivery_internal(record_id, result_payload)
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
        self.get_record_internal(record_id, with_object)
    }

    async fn handle_get_message(
        &self,
        msg_id: ObjId,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<MsgObject>, RPCErrors> {
        self.get_message_internal(msg_id)
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
    }

    async fn handle_get_preferred_binding(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<AccountBinding, RPCErrors> {
        self.contact_mgr
            .get_preferred_binding(did, contact_mgr_owner)
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
    }

    async fn handle_get_contact(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<Contact>, RPCErrors> {
        self.contact_mgr.get_contact(did, contact_mgr_owner)
    }

    async fn handle_list_contacts(
        &self,
        query: ContactQuery,
        contact_mgr_owner: Option<DID>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<Contact>, RPCErrors> {
        self.contact_mgr.list_contacts(query, contact_mgr_owner)
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
    }
}
