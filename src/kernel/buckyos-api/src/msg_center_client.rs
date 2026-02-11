use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::ObjId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;

pub const MSG_CENTER_SERVICE_UNIQUE_ID: &str = "msg-center";
pub const MSG_CENTER_SERVICE_NAME: &str = "msg-center";
pub const MSG_CENTER_SERVICE_PORT: u16 = 4050;

const METHOD_MSG_DISPATCH: &str = "msg.dispatch";
const METHOD_MSG_POST_SEND: &str = "msg.post_send";
const METHOD_MSG_GET_NEXT: &str = "msg.get_next";
const METHOD_MSG_PEEK_BOX: &str = "msg.peek_box";
const METHOD_MSG_LIST_BOX_BY_TIME: &str = "msg.list_box_by_time";
const METHOD_MSG_UPDATE_RECORD_STATE: &str = "msg.update_record_state";
const METHOD_MSG_REPORT_DELIVERY: &str = "msg.report_delivery";
const METHOD_MSG_SET_READ_STATE: &str = "msg.set_read_state";
const METHOD_MSG_LIST_READ_RECEIPTS: &str = "msg.list_read_receipts";
const METHOD_MSG_GET_RECORD: &str = "msg.get_record";
const METHOD_MSG_GET_MESSAGE: &str = "msg.get_message";

const METHOD_CONTACT_RESOLVE_DID: &str = "contact.resolve_did";
const METHOD_CONTACT_GET_PREFERRED_BINDING: &str = "contact.get_preferred_binding";
const METHOD_CONTACT_CHECK_ACCESS_PERMISSION: &str = "contact.check_access_permission";
const METHOD_CONTACT_GRANT_TEMPORARY_ACCESS: &str = "contact.grant_temporary_access";
const METHOD_CONTACT_BLOCK_CONTACT: &str = "contact.block_contact";
const METHOD_CONTACT_IMPORT_CONTACTS: &str = "contact.import_contacts";
const METHOD_CONTACT_MERGE_CONTACTS: &str = "contact.merge_contacts";
const METHOD_CONTACT_UPDATE_CONTACT: &str = "contact.update_contact";
const METHOD_CONTACT_GET_CONTACT: &str = "contact.get_contact";
const METHOD_CONTACT_LIST_CONTACTS: &str = "contact.list_contacts";
const METHOD_CONTACT_GET_GROUP_SUBSCRIBERS: &str = "contact.get_group_subscribers";
const METHOD_CONTACT_SET_GROUP_SUBSCRIBERS: &str = "contact.set_group_subscribers";

fn parse_from_json<T: DeserializeOwned>(
    value: Value,
    type_name: &str,
) -> std::result::Result<T, RPCErrors> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParseRequestError(format!("Failed to parse {}: {}", type_name, error))
    })
}

fn serialize_to_json<T: Serialize>(
    value: &T,
    type_name: &str,
) -> std::result::Result<Value, RPCErrors> {
    serde_json::to_value(value).map_err(|error| {
        RPCErrors::ReasonError(format!("Failed to serialize {}: {}", type_name, error))
    })
}

fn parse_rpc_response<T: DeserializeOwned>(
    value: Value,
    type_name: &str,
) -> std::result::Result<T, RPCErrors> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParserResponseError(format!("Failed to parse {} response: {}", type_name, error))
    })
}

fn parse_optional_rpc_response<T: DeserializeOwned>(
    value: Value,
    type_name: &str,
) -> std::result::Result<Option<T>, RPCErrors> {
    if value.is_null() {
        return Ok(None);
    }

    let parsed: T = serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParserResponseError(format!(
            "Failed to parse optional {} response: {}",
            type_name, error
        ))
    })?;
    Ok(Some(parsed))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BoxKind {
    Inbox,
    Outbox,
    GroupInbox,
    TunnelOutbox,
    RequestBox,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MsgState {
    Unread,
    Reading,
    Readed,
    Wait,
    Sending,
    Sent,
    Failed,
    Dead,
    Deleted,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgObject {
    pub id: ObjId,
    pub from: DID,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<DID>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_key: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    pub created_at_ms: u64,
}

impl MsgObject {
    pub fn new(
        id: ObjId,
        from: DID,
        source: Option<DID>,
        to: Vec<DID>,
        payload: Value,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id,
            from,
            source,
            to,
            thread_key: None,
            payload,
            meta: None,
            created_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct IngressContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tunnel_did: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SendContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_tunnel: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RouteInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tunnel_did: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_did: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub ext_ids: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeliveryInfo {
    #[serde(default)]
    pub attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_retry_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_msg_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgRecord {
    pub record_id: String,
    pub owner: DID,
    pub box_kind: BoxKind,
    pub msg_id: ObjId,
    pub state: MsgState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_key: Option<String>,
    pub sort_key: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgRecordWithObject {
    pub record: MsgRecord,
    pub msg: MsgObject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MsgRecordPage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<MsgRecordWithObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor_sort_key: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor_record_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadReceiptState {
    Unread,
    Reading,
    Readed,
    Accepted,
    Rejected,
    Quarantined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MsgReceiptObj {
    pub msg_id: ObjId,
    pub iss: DID,
    pub reader: DID,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<DID>,
    pub at_ms: u64,
    pub status: ReadReceiptState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DispatchResult {
    pub ok: bool,
    pub msg_id: ObjId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivered_recipients: Vec<DID>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_recipients: Vec<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_group: Option<DID>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivered_agents: Vec<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostSendDelivery {
    pub tunnel_did: DID,
    pub record_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_did: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostSendResult {
    pub ok: bool,
    pub msg_id: ObjId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<PostSendDelivery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeliveryReportResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_msg_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountBinding {
    pub platform: String,
    pub account_id: String,
    pub display_id: String,
    pub tunnel_id: String,
    pub last_active_at: u64,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub meta: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContactSource {
    ManualImport,
    ManualCreate,
    AutoInferred,
    Shared,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessGroupLevel {
    Block,
    Stranger,
    Temporary,
    Friend,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemporaryGrant {
    pub context_id: String,
    pub granted_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contact {
    pub did: DID,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub source: ContactSource,
    pub is_verified: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<AccountBinding>,
    pub access_level: AccessGroupLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temp_grants: Vec<TemporaryGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessDecision {
    pub level: AccessGroupLevel,
    pub allow_delivery: bool,
    pub target_box: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary_expires_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportContactEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<AccountBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ImportReport {
    pub imported: u64,
    pub upgraded_shadow: u64,
    pub merged: u64,
    pub created: u64,
    pub skipped: u64,
    pub failed: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_dids: Vec<DID>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContactPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_level: Option<AccessGroupLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ContactSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemporaryGrantOutcome {
    pub did: DID,
    pub granted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GrantTemporaryAccessResult {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub updated: Vec<TemporaryGrantOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContactQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ContactSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_level: Option<AccessGroupLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetGroupSubscribersResult {
    pub group_id: DID,
    pub subscriber_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterDispatchReq {
    pub msg: MsgObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress_ctx: Option<IngressContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl MsgCenterDispatchReq {
    pub fn new(
        msg: MsgObject,
        ingress_ctx: Option<IngressContext>,
        idempotency_key: Option<String>,
    ) -> Self {
        Self {
            msg,
            ingress_ctx,
            idempotency_key,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterDispatchReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterPostSendReq {
    pub msg: MsgObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_ctx: Option<SendContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl MsgCenterPostSendReq {
    pub fn new(
        msg: MsgObject,
        send_ctx: Option<SendContext>,
        idempotency_key: Option<String>,
    ) -> Self {
        Self {
            msg,
            send_ctx,
            idempotency_key,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterPostSendReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterGetNextReq {
    pub owner: DID,
    pub box_kind: BoxKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_filter: Option<Vec<MsgState>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_on_take: Option<bool>,
}

impl MsgCenterGetNextReq {
    pub fn new(
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        lock_on_take: Option<bool>,
    ) -> Self {
        Self {
            owner,
            box_kind,
            state_filter,
            lock_on_take,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterGetNextReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterPeekBoxReq {
    pub owner: DID,
    pub box_kind: BoxKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_filter: Option<Vec<MsgState>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl MsgCenterPeekBoxReq {
    pub fn new(
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
    ) -> Self {
        Self {
            owner,
            box_kind,
            state_filter,
            limit,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterPeekBoxReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterListBoxByTimeReq {
    pub owner: DID,
    pub box_kind: BoxKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_filter: Option<Vec<MsgState>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_sort_key: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descending: Option<bool>,
}

impl MsgCenterListBoxByTimeReq {
    pub fn new(
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
    ) -> Self {
        Self {
            owner,
            box_kind,
            state_filter,
            limit,
            cursor_sort_key,
            cursor_record_id,
            descending,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterListBoxByTimeReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterUpdateRecordStateReq {
    pub record_id: String,
    pub new_state: MsgState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl MsgCenterUpdateRecordStateReq {
    pub fn new(record_id: String, new_state: MsgState, reason: Option<String>) -> Self {
        Self {
            record_id,
            new_state,
            reason,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterUpdateRecordStateReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterReportDeliveryReq {
    pub record_id: String,
    pub result: DeliveryReportResult,
}

impl MsgCenterReportDeliveryReq {
    pub fn new(record_id: String, result: DeliveryReportResult) -> Self {
        Self { record_id, result }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterReportDeliveryReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterSetReadStateReq {
    pub group_id: DID,
    pub msg_id: ObjId,
    pub reader_did: DID,
    pub status: ReadReceiptState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

impl MsgCenterSetReadStateReq {
    pub fn new(
        group_id: DID,
        msg_id: ObjId,
        reader_did: DID,
        status: ReadReceiptState,
        reason: Option<String>,
        at_ms: Option<u64>,
    ) -> Self {
        Self {
            group_id,
            msg_id,
            reader_did,
            status,
            reason,
            at_ms,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterSetReadStateReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterListReadReceiptsReq {
    pub msg_id: ObjId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reader: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

impl MsgCenterListReadReceiptsReq {
    pub fn new(
        msg_id: ObjId,
        group_id: Option<DID>,
        reader: Option<DID>,
        limit: Option<usize>,
        offset: Option<u64>,
    ) -> Self {
        Self {
            msg_id,
            group_id,
            reader,
            limit,
            offset,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterListReadReceiptsReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterGetRecordReq {
    pub record_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_object: Option<bool>,
}

impl MsgCenterGetRecordReq {
    pub fn new(record_id: String, with_object: Option<bool>) -> Self {
        Self {
            record_id,
            with_object,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterGetRecordReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterGetMessageReq {
    pub msg_id: ObjId,
}

impl MsgCenterGetMessageReq {
    pub fn new(msg_id: ObjId) -> Self {
        Self { msg_id }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterGetMessageReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterResolveDidReq {
    pub platform: String,
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_hint: Option<Value>,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterResolveDidReq {
    pub fn new(
        platform: String,
        account_id: String,
        profile_hint: Option<Value>,
        contact_mgr_owner: Option<DID>,
    ) -> Self {
        Self {
            platform,
            account_id,
            profile_hint,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterResolveDidReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterGetPreferredBindingReq {
    pub did: DID,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterGetPreferredBindingReq {
    pub fn new(did: DID, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            did,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterGetPreferredBindingReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterCheckAccessPermissionReq {
    pub did: DID,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterCheckAccessPermissionReq {
    pub fn new(did: DID, context_id: Option<String>, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            did,
            context_id,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterCheckAccessPermissionReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterGrantTemporaryAccessReq {
    pub dids: Vec<DID>,
    pub context_id: String,
    pub duration_secs: u64,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterGrantTemporaryAccessReq {
    pub fn new(
        dids: Vec<DID>,
        context_id: String,
        duration_secs: u64,
        contact_mgr_owner: Option<DID>,
    ) -> Self {
        Self {
            dids,
            context_id,
            duration_secs,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterGrantTemporaryAccessReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterBlockContactReq {
    pub did: DID,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterBlockContactReq {
    pub fn new(did: DID, reason: Option<String>, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            did,
            reason,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterBlockContactReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterImportContactsReq {
    pub contacts: Vec<ImportContactEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_to_friend: Option<bool>,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterImportContactsReq {
    pub fn new(
        contacts: Vec<ImportContactEntry>,
        upgrade_to_friend: Option<bool>,
        contact_mgr_owner: Option<DID>,
    ) -> Self {
        Self {
            contacts,
            upgrade_to_friend,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterImportContactsReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterMergeContactsReq {
    pub target_did: DID,
    pub source_did: DID,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterMergeContactsReq {
    pub fn new(target_did: DID, source_did: DID, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            target_did,
            source_did,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterMergeContactsReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterUpdateContactReq {
    pub did: DID,
    pub patch: ContactPatch,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterUpdateContactReq {
    pub fn new(did: DID, patch: ContactPatch, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            did,
            patch,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterUpdateContactReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterGetContactReq {
    pub did: DID,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterGetContactReq {
    pub fn new(did: DID, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            did,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterGetContactReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterListContactsReq {
    pub query: ContactQuery,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterListContactsReq {
    pub fn new(query: ContactQuery, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            query,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterListContactsReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterGetGroupSubscribersReq {
    pub group_id: DID,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterGetGroupSubscribersReq {
    pub fn new(
        group_id: DID,
        limit: Option<usize>,
        offset: Option<u64>,
        contact_mgr_owner: Option<DID>,
    ) -> Self {
        Self {
            group_id,
            limit,
            offset,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterGetGroupSubscribersReq")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgCenterSetGroupSubscribersReq {
    pub group_id: DID,
    pub subscribers: Vec<DID>,
    // None means using system-wide contact-manager view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_mgr_owner: Option<DID>,
}

impl MsgCenterSetGroupSubscribersReq {
    pub fn new(group_id: DID, subscribers: Vec<DID>, contact_mgr_owner: Option<DID>) -> Self {
        Self {
            group_id,
            subscribers,
            contact_mgr_owner,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        parse_from_json(value, "MsgCenterSetGroupSubscribersReq")
    }
}

pub enum MsgCenterClient {
    InProcess(Box<dyn MsgCenterHandler>),
    KRPC(Box<kRPC>),
}

impl MsgCenterClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn MsgCenterHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await;
            }
        }
    }

    pub async fn dispatch(
        &self,
        msg: MsgObject,
        ingress_ctx: Option<IngressContext>,
        idempotency_key: Option<String>,
    ) -> std::result::Result<DispatchResult, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_dispatch(msg, ingress_ctx, idempotency_key, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterDispatchReq::new(msg, ingress_ctx, idempotency_key);
                let req_json = serialize_to_json(&req, "MsgCenterDispatchReq")?;
                let result = client.call(METHOD_MSG_DISPATCH, req_json).await?;
                parse_rpc_response(result, "DispatchResult")
            }
        }
    }

    pub async fn post_send(
        &self,
        msg: MsgObject,
        send_ctx: Option<SendContext>,
        idempotency_key: Option<String>,
    ) -> std::result::Result<PostSendResult, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_post_send(msg, send_ctx, idempotency_key, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterPostSendReq::new(msg, send_ctx, idempotency_key);
                let req_json = serialize_to_json(&req, "MsgCenterPostSendReq")?;
                let result = client.call(METHOD_MSG_POST_SEND, req_json).await?;
                parse_rpc_response(result, "PostSendResult")
            }
        }
    }

    pub async fn get_next(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        lock_on_take: Option<bool>,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_get_next(owner, box_kind, state_filter, lock_on_take, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterGetNextReq::new(owner, box_kind, state_filter, lock_on_take);
                let req_json = serialize_to_json(&req, "MsgCenterGetNextReq")?;
                let result = client.call(METHOD_MSG_GET_NEXT, req_json).await?;
                parse_optional_rpc_response(result, "MsgRecordWithObject")
            }
        }
    }

    pub async fn peek_box(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
    ) -> std::result::Result<Vec<MsgRecordWithObject>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_peek_box(owner, box_kind, state_filter, limit, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterPeekBoxReq::new(owner, box_kind, state_filter, limit);
                let req_json = serialize_to_json(&req, "MsgCenterPeekBoxReq")?;
                let result = client.call(METHOD_MSG_PEEK_BOX, req_json).await?;
                parse_rpc_response(result, "Vec<MsgRecordWithObject>")
            }
        }
    }

    pub async fn list_box_by_time(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
    ) -> std::result::Result<MsgRecordPage, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_list_box_by_time(
                        owner,
                        box_kind,
                        state_filter,
                        limit,
                        cursor_sort_key,
                        cursor_record_id,
                        descending,
                        ctx,
                    )
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterListBoxByTimeReq::new(
                    owner,
                    box_kind,
                    state_filter,
                    limit,
                    cursor_sort_key,
                    cursor_record_id,
                    descending,
                );
                let req_json = serialize_to_json(&req, "MsgCenterListBoxByTimeReq")?;
                let result = client.call(METHOD_MSG_LIST_BOX_BY_TIME, req_json).await?;
                parse_rpc_response(result, "MsgRecordPage")
            }
        }
    }

    pub async fn update_record_state(
        &self,
        record_id: String,
        new_state: MsgState,
        reason: Option<String>,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_update_record_state(record_id, new_state, reason, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterUpdateRecordStateReq::new(record_id, new_state, reason);
                let req_json = serialize_to_json(&req, "MsgCenterUpdateRecordStateReq")?;
                let result = client
                    .call(METHOD_MSG_UPDATE_RECORD_STATE, req_json)
                    .await?;
                parse_rpc_response(result, "MsgRecord")
            }
        }
    }

    pub async fn report_delivery(
        &self,
        record_id: String,
        result_payload: DeliveryReportResult,
    ) -> std::result::Result<MsgRecord, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_report_delivery(record_id, result_payload, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterReportDeliveryReq::new(record_id, result_payload);
                let req_json = serialize_to_json(&req, "MsgCenterReportDeliveryReq")?;
                let result = client.call(METHOD_MSG_REPORT_DELIVERY, req_json).await?;
                parse_rpc_response(result, "MsgRecord")
            }
        }
    }

    pub async fn set_read_state(
        &self,
        group_id: DID,
        msg_id: ObjId,
        reader_did: DID,
        status: ReadReceiptState,
        reason: Option<String>,
        at_ms: Option<u64>,
    ) -> std::result::Result<MsgReceiptObj, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_set_read_state(group_id, msg_id, reader_did, status, reason, at_ms, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterSetReadStateReq::new(
                    group_id, msg_id, reader_did, status, reason, at_ms,
                );
                let req_json = serialize_to_json(&req, "MsgCenterSetReadStateReq")?;
                let result = client.call(METHOD_MSG_SET_READ_STATE, req_json).await?;
                parse_rpc_response(result, "MsgReceiptObj")
            }
        }
    }

    pub async fn list_read_receipts(
        &self,
        msg_id: ObjId,
        group_id: Option<DID>,
        reader: Option<DID>,
        limit: Option<usize>,
        offset: Option<u64>,
    ) -> std::result::Result<Vec<MsgReceiptObj>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_list_read_receipts(msg_id, group_id, reader, limit, offset, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req =
                    MsgCenterListReadReceiptsReq::new(msg_id, group_id, reader, limit, offset);
                let req_json = serialize_to_json(&req, "MsgCenterListReadReceiptsReq")?;
                let result = client.call(METHOD_MSG_LIST_READ_RECEIPTS, req_json).await?;
                parse_rpc_response(result, "Vec<MsgReceiptObj>")
            }
        }
    }

    pub async fn get_record(
        &self,
        record_id: String,
        with_object: Option<bool>,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_get_record(record_id, with_object, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgCenterGetRecordReq::new(record_id, with_object);
                let req_json = serialize_to_json(&req, "MsgCenterGetRecordReq")?;
                let result = client.call(METHOD_MSG_GET_RECORD, req_json).await?;
                parse_optional_rpc_response(result, "MsgRecordWithObject")
            }
        }
    }

    pub async fn get_message(
        &self,
        msg_id: ObjId,
    ) -> std::result::Result<Option<MsgObject>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_get_message(msg_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgCenterGetMessageReq::new(msg_id);
                let req_json = serialize_to_json(&req, "MsgCenterGetMessageReq")?;
                let result = client.call(METHOD_MSG_GET_MESSAGE, req_json).await?;
                parse_optional_rpc_response(result, "MsgObject")
            }
        }
    }

    pub async fn resolve_did(
        &self,
        platform: String,
        account_id: String,
        profile_hint: Option<Value>,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<DID, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_resolve_did(platform, account_id, profile_hint, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterResolveDidReq::new(
                    platform,
                    account_id,
                    profile_hint,
                    contact_mgr_owner,
                );
                let req_json = serialize_to_json(&req, "MsgCenterResolveDidReq")?;
                let result = client.call(METHOD_CONTACT_RESOLVE_DID, req_json).await?;
                parse_rpc_response(result, "DID")
            }
        }
    }

    pub async fn get_preferred_binding(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<AccountBinding, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_get_preferred_binding(did, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterGetPreferredBindingReq::new(did, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterGetPreferredBindingReq")?;
                let result = client
                    .call(METHOD_CONTACT_GET_PREFERRED_BINDING, req_json)
                    .await?;
                parse_rpc_response(result, "AccountBinding")
            }
        }
    }

    pub async fn check_access_permission(
        &self,
        did: DID,
        context_id: Option<String>,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<AccessDecision, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_check_access_permission(did, context_id, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req =
                    MsgCenterCheckAccessPermissionReq::new(did, context_id, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterCheckAccessPermissionReq")?;
                let result = client
                    .call(METHOD_CONTACT_CHECK_ACCESS_PERMISSION, req_json)
                    .await?;
                parse_rpc_response(result, "AccessDecision")
            }
        }
    }

    pub async fn grant_temporary_access(
        &self,
        dids: Vec<DID>,
        context_id: String,
        duration_secs: u64,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<GrantTemporaryAccessResult, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_grant_temporary_access(
                        dids,
                        context_id,
                        duration_secs,
                        contact_mgr_owner,
                        ctx,
                    )
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterGrantTemporaryAccessReq::new(
                    dids,
                    context_id,
                    duration_secs,
                    contact_mgr_owner,
                );
                let req_json = serialize_to_json(&req, "MsgCenterGrantTemporaryAccessReq")?;
                let result = client
                    .call(METHOD_CONTACT_GRANT_TEMPORARY_ACCESS, req_json)
                    .await?;
                parse_rpc_response(result, "GrantTemporaryAccessResult")
            }
        }
    }

    pub async fn block_contact(
        &self,
        did: DID,
        reason: Option<String>,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_block_contact(did, reason, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterBlockContactReq::new(did, reason, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterBlockContactReq")?;
                client.call(METHOD_CONTACT_BLOCK_CONTACT, req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn import_contacts(
        &self,
        contacts: Vec<ImportContactEntry>,
        upgrade_to_friend: Option<bool>,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<ImportReport, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_import_contacts(contacts, upgrade_to_friend, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req =
                    MsgCenterImportContactsReq::new(contacts, upgrade_to_friend, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterImportContactsReq")?;
                let result = client
                    .call(METHOD_CONTACT_IMPORT_CONTACTS, req_json)
                    .await?;
                parse_rpc_response(result, "ImportReport")
            }
        }
    }

    pub async fn merge_contacts(
        &self,
        target_did: DID,
        source_did: DID,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<Contact, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_merge_contacts(target_did, source_did, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterMergeContactsReq::new(target_did, source_did, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterMergeContactsReq")?;
                let result = client.call(METHOD_CONTACT_MERGE_CONTACTS, req_json).await?;
                parse_rpc_response(result, "Contact")
            }
        }
    }

    pub async fn update_contact(
        &self,
        did: DID,
        patch: ContactPatch,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<Contact, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_update_contact(did, patch, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterUpdateContactReq::new(did, patch, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterUpdateContactReq")?;
                let result = client.call(METHOD_CONTACT_UPDATE_CONTACT, req_json).await?;
                parse_rpc_response(result, "Contact")
            }
        }
    }

    pub async fn get_contact(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<Option<Contact>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_get_contact(did, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterGetContactReq::new(did, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterGetContactReq")?;
                let result = client.call(METHOD_CONTACT_GET_CONTACT, req_json).await?;
                parse_optional_rpc_response(result, "Contact")
            }
        }
    }

    pub async fn list_contacts(
        &self,
        query: ContactQuery,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<Vec<Contact>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_list_contacts(query, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterListContactsReq::new(query, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterListContactsReq")?;
                let result = client.call(METHOD_CONTACT_LIST_CONTACTS, req_json).await?;
                parse_rpc_response(result, "Vec<Contact>")
            }
        }
    }

    pub async fn get_group_subscribers(
        &self,
        group_id: DID,
        limit: Option<usize>,
        offset: Option<u64>,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<Vec<DID>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_get_group_subscribers(group_id, limit, offset, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgCenterGetGroupSubscribersReq::new(
                    group_id,
                    limit,
                    offset,
                    contact_mgr_owner,
                );
                let req_json = serialize_to_json(&req, "MsgCenterGetGroupSubscribersReq")?;
                let result = client
                    .call(METHOD_CONTACT_GET_GROUP_SUBSCRIBERS, req_json)
                    .await?;
                parse_rpc_response(result, "Vec<DID>")
            }
        }
    }

    pub async fn set_group_subscribers(
        &self,
        group_id: DID,
        subscribers: Vec<DID>,
        contact_mgr_owner: Option<DID>,
    ) -> std::result::Result<SetGroupSubscribersResult, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_set_group_subscribers(group_id, subscribers, contact_mgr_owner, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req =
                    MsgCenterSetGroupSubscribersReq::new(group_id, subscribers, contact_mgr_owner);
                let req_json = serialize_to_json(&req, "MsgCenterSetGroupSubscribersReq")?;
                let result = client
                    .call(METHOD_CONTACT_SET_GROUP_SUBSCRIBERS, req_json)
                    .await?;
                parse_rpc_response(result, "SetGroupSubscribersResult")
            }
        }
    }
}

#[async_trait]
pub trait MsgCenterHandler: Send + Sync {
    async fn handle_dispatch(
        &self,
        msg: MsgObject,
        ingress_ctx: Option<IngressContext>,
        idempotency_key: Option<String>,
        ctx: RPCContext,
    ) -> std::result::Result<DispatchResult, RPCErrors>;

    async fn handle_post_send(
        &self,
        msg: MsgObject,
        send_ctx: Option<SendContext>,
        idempotency_key: Option<String>,
        ctx: RPCContext,
    ) -> std::result::Result<PostSendResult, RPCErrors>;

    async fn handle_get_next(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        lock_on_take: Option<bool>,
        ctx: RPCContext,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors>;

    async fn handle_peek_box(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<MsgRecordWithObject>, RPCErrors>;

    async fn handle_list_box_by_time(
        &self,
        owner: DID,
        box_kind: BoxKind,
        state_filter: Option<Vec<MsgState>>,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        ctx: RPCContext,
    ) -> std::result::Result<MsgRecordPage, RPCErrors>;

    async fn handle_update_record_state(
        &self,
        record_id: String,
        new_state: MsgState,
        reason: Option<String>,
        ctx: RPCContext,
    ) -> std::result::Result<MsgRecord, RPCErrors>;

    async fn handle_report_delivery(
        &self,
        record_id: String,
        result_payload: DeliveryReportResult,
        ctx: RPCContext,
    ) -> std::result::Result<MsgRecord, RPCErrors>;

    async fn handle_set_read_state(
        &self,
        group_id: DID,
        msg_id: ObjId,
        reader_did: DID,
        status: ReadReceiptState,
        reason: Option<String>,
        at_ms: Option<u64>,
        ctx: RPCContext,
    ) -> std::result::Result<MsgReceiptObj, RPCErrors>;

    async fn handle_list_read_receipts(
        &self,
        msg_id: ObjId,
        group_id: Option<DID>,
        reader: Option<DID>,
        limit: Option<usize>,
        offset: Option<u64>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<MsgReceiptObj>, RPCErrors>;

    async fn handle_get_record(
        &self,
        record_id: String,
        with_object: Option<bool>,
        ctx: RPCContext,
    ) -> std::result::Result<Option<MsgRecordWithObject>, RPCErrors>;

    async fn handle_get_message(
        &self,
        msg_id: ObjId,
        ctx: RPCContext,
    ) -> std::result::Result<Option<MsgObject>, RPCErrors>;

    async fn handle_resolve_did(
        &self,
        platform: String,
        account_id: String,
        profile_hint: Option<Value>,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<DID, RPCErrors>;

    async fn handle_get_preferred_binding(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<AccountBinding, RPCErrors>;

    async fn handle_check_access_permission(
        &self,
        did: DID,
        context_id: Option<String>,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<AccessDecision, RPCErrors>;

    async fn handle_grant_temporary_access(
        &self,
        dids: Vec<DID>,
        context_id: String,
        duration_secs: u64,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<GrantTemporaryAccessResult, RPCErrors>;

    async fn handle_block_contact(
        &self,
        did: DID,
        reason: Option<String>,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_import_contacts(
        &self,
        contacts: Vec<ImportContactEntry>,
        upgrade_to_friend: Option<bool>,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<ImportReport, RPCErrors>;

    async fn handle_merge_contacts(
        &self,
        target_did: DID,
        source_did: DID,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<Contact, RPCErrors>;

    async fn handle_update_contact(
        &self,
        did: DID,
        patch: ContactPatch,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<Contact, RPCErrors>;

    async fn handle_get_contact(
        &self,
        did: DID,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<Option<Contact>, RPCErrors>;

    async fn handle_list_contacts(
        &self,
        query: ContactQuery,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<Contact>, RPCErrors>;

    async fn handle_get_group_subscribers(
        &self,
        group_id: DID,
        limit: Option<usize>,
        offset: Option<u64>,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<DID>, RPCErrors>;

    async fn handle_set_group_subscribers(
        &self,
        group_id: DID,
        subscribers: Vec<DID>,
        contact_mgr_owner: Option<DID>,
        ctx: RPCContext,
    ) -> std::result::Result<SetGroupSubscribersResult, RPCErrors>;
}

pub struct MsgCenterServerHandler<T: MsgCenterHandler>(pub T);

impl<T: MsgCenterHandler> MsgCenterServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: MsgCenterHandler> RPCHandler for MsgCenterServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            METHOD_MSG_DISPATCH | "dispatch" => {
                let dispatch_req = MsgCenterDispatchReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_dispatch(
                        dispatch_req.msg,
                        dispatch_req.ingress_ctx,
                        dispatch_req.idempotency_key,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_POST_SEND | "post_send" => {
                let post_send_req = MsgCenterPostSendReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_post_send(
                        post_send_req.msg,
                        post_send_req.send_ctx,
                        post_send_req.idempotency_key,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_GET_NEXT | "get_next" => {
                let next_req = MsgCenterGetNextReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_next(
                        next_req.owner,
                        next_req.box_kind,
                        next_req.state_filter,
                        next_req.lock_on_take,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_PEEK_BOX | "peek_box" => {
                let peek_req = MsgCenterPeekBoxReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_peek_box(
                        peek_req.owner,
                        peek_req.box_kind,
                        peek_req.state_filter,
                        peek_req.limit,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_LIST_BOX_BY_TIME | "list_box_by_time" => {
                let list_req = MsgCenterListBoxByTimeReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_list_box_by_time(
                        list_req.owner,
                        list_req.box_kind,
                        list_req.state_filter,
                        list_req.limit,
                        list_req.cursor_sort_key,
                        list_req.cursor_record_id,
                        list_req.descending,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_UPDATE_RECORD_STATE | "update_record_state" => {
                let update_req = MsgCenterUpdateRecordStateReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_record_state(
                        update_req.record_id,
                        update_req.new_state,
                        update_req.reason,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_REPORT_DELIVERY | "report_delivery" => {
                let report_req = MsgCenterReportDeliveryReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_report_delivery(report_req.record_id, report_req.result, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_SET_READ_STATE | "set_read_state" => {
                let read_req = MsgCenterSetReadStateReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_set_read_state(
                        read_req.group_id,
                        read_req.msg_id,
                        read_req.reader_did,
                        read_req.status,
                        read_req.reason,
                        read_req.at_ms,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_LIST_READ_RECEIPTS | "list_read_receipts" => {
                let list_req = MsgCenterListReadReceiptsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_list_read_receipts(
                        list_req.msg_id,
                        list_req.group_id,
                        list_req.reader,
                        list_req.limit,
                        list_req.offset,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_GET_RECORD | "get_record" => {
                let get_req = MsgCenterGetRecordReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_record(get_req.record_id, get_req.with_object, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_MSG_GET_MESSAGE | "get_message" => {
                let get_req = MsgCenterGetMessageReq::from_json(req.params)?;
                let result = self.0.handle_get_message(get_req.msg_id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_RESOLVE_DID | "resolve_did" => {
                let resolve_req = MsgCenterResolveDidReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_resolve_did(
                        resolve_req.platform,
                        resolve_req.account_id,
                        resolve_req.profile_hint,
                        resolve_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_GET_PREFERRED_BINDING | "get_preferred_binding" => {
                let binding_req = MsgCenterGetPreferredBindingReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_preferred_binding(
                        binding_req.did,
                        binding_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_CHECK_ACCESS_PERMISSION | "check_access_permission" => {
                let access_req = MsgCenterCheckAccessPermissionReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_check_access_permission(
                        access_req.did,
                        access_req.context_id,
                        access_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_GRANT_TEMPORARY_ACCESS | "grant_temporary_access" => {
                let grant_req = MsgCenterGrantTemporaryAccessReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_grant_temporary_access(
                        grant_req.dids,
                        grant_req.context_id,
                        grant_req.duration_secs,
                        grant_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_BLOCK_CONTACT | "block_contact" => {
                let block_req = MsgCenterBlockContactReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_block_contact(
                        block_req.did,
                        block_req.reason,
                        block_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_IMPORT_CONTACTS | "import_contacts" => {
                let import_req = MsgCenterImportContactsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_import_contacts(
                        import_req.contacts,
                        import_req.upgrade_to_friend,
                        import_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_MERGE_CONTACTS | "merge_contacts" => {
                let merge_req = MsgCenterMergeContactsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_merge_contacts(
                        merge_req.target_did,
                        merge_req.source_did,
                        merge_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_UPDATE_CONTACT | "update_contact" => {
                let update_req = MsgCenterUpdateContactReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_contact(
                        update_req.did,
                        update_req.patch,
                        update_req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_GET_CONTACT | "get_contact" => {
                let get_req = MsgCenterGetContactReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_contact(get_req.did, get_req.contact_mgr_owner, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_LIST_CONTACTS | "list_contacts" => {
                let list_req = MsgCenterListContactsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_list_contacts(list_req.query, list_req.contact_mgr_owner, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_GET_GROUP_SUBSCRIBERS | "get_group_subscribers" => {
                let req = MsgCenterGetGroupSubscribersReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_group_subscribers(
                        req.group_id,
                        req.limit,
                        req.offset,
                        req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            METHOD_CONTACT_SET_GROUP_SUBSCRIBERS | "set_group_subscribers" => {
                let req = MsgCenterSetGroupSubscribersReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_set_group_subscribers(
                        req.group_id,
                        req.subscribers,
                        req.contact_mgr_owner,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

pub fn generate_msg_center_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        MSG_CENTER_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Message Center")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}
