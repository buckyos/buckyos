//! Session Input Bus records (`opendan.session_input/3`) and input receipts.
//!
//! A bus record is one JSON object — the *logical record*:
//!
//! ```json
//! { "schema": "opendan.session_input/3", "type": "msg | event | control",
//!   "key": "...", "from": "...", "at_ms": 0, "payload": { } }
//! ```
//!
//! `payload` is decided by `type`: a [`SessionMsg`] (a cyfs-ndn `MsgObject`
//! as it is, plus a small delivery layer), an [`AgentEvent`] or a
//! [`ControlCommand`]. On kmsg the envelope fields travel as headers and the
//! payload as the message body. Posting and consuming validate with the same
//! function ([`parse_record`]); a record that does not pass is refused when
//! posted and rejected (consumed, never entering the context) when fetched.

use std::collections::BTreeMap;

use name_lib::DID;
use ndn_lib::{
    MsgContent, MsgObjKind, MsgObject, NamedObject, ObjId, RefItem, RefRole, RefTarget,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::Subscription;
use super::state::Touching;
use super::worklog::InputRef;
use crate::error::{OpenDanError, Result};

pub const SESSION_INPUT_SCHEMA: &str = "opendan.session_input/3";

pub const HEADER_SCHEMA: &str = "schema";
pub const HEADER_TYPE: &str = "type";
pub const HEADER_KEY: &str = "key";
pub const HEADER_FROM: &str = "from";
pub const HEADER_AT_MS: &str = "at_ms";

pub const INPUT_TYPE_MSG: &str = "msg";
pub const INPUT_TYPE_EVENT: &str = "event";
pub const INPUT_TYPE_CONTROL: &str = "control";

/// kRPC bodies are capped at 1 MB and payloads travel as JSON number arrays,
/// so one payload must stay below ~250 KB.
pub const MAX_PAYLOAD_BYTES: usize = 250 * 1024;
pub const MAX_KEY_BYTES: usize = 256;
pub const MAX_FROM_BYTES: usize = 256;
pub const MAX_DELIVERY_FIELD_BYTES: usize = 256;
pub const MAX_EVENT_NAME_BYTES: usize = 64;
pub const MAX_EVENT_SOURCE_ID_BYTES: usize = 256;
pub const MAX_EVENT_SUMMARY_BYTES: usize = 1024;

/// Records a session's bus may hold that are not consumed yet (`msg`,
/// `event` and `control` share the limit). A full bus refuses the append
/// with [`OpenDanError::InputFull`]; nothing is overwritten.
pub const MAX_PENDING_INPUTS: usize = 64;

/// `type = "msg"` payload: the message object as it is plus what a
/// MsgObject does not carry by design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionMsg {
    /// cyfs-ndn MsgObject v2, never rewritten or trimmed.
    #[schemars(with = "Value")]
    pub msg: MsgObject,
    #[serde(default, skip_serializing_if = "MsgDelivery::is_empty")]
    pub delivery: MsgDelivery,
}

/// Delivery layer information; everything optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MsgDelivery {
    /// Display name of the speaker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
    /// Display name of the conversation (group name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_name: Option<String>,
    /// msg-center record id (audit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    /// DID of the ingress tunnel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<String>,
}

impl MsgDelivery {
    pub fn is_empty(&self) -> bool {
        self.from_name.is_none()
            && self.conversation_name.is_none()
            && self.record_id.is_none()
            && self.tunnel.is_none()
    }
}

/// Where an event comes from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct EventSource {
    /// `object | session | task | timer | system`; unknown kinds are not a
    /// format error (an event without a matching subscription is dropped).
    pub kind: String,
    /// object: object id; session: sid; task: task_id; timer: timer name;
    /// system: may be empty.
    #[serde(default)]
    pub id: String,
}

impl EventSource {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }

    pub fn task(task_id: impl Into<String>) -> Self {
        Self::new("task", task_id)
    }

    /// `kind:id`.
    pub fn label(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }
}

/// `type = "event"` payload: something this session cares about happened.
/// The poster only describes the event; whether it is an Input or an
/// Observe is decided by the receiving session's subscriptions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEvent {
    /// Explicit subscription id; may be empty for implicit subscriptions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_id: Option<String>,
    pub source: EventSource,
    /// `changed | updated | finished | fired ...`.
    pub event: String,
    /// Version inside the source; Observe merging compares it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// One sentence for the LLM (≤ 1 KB); never parsed mechanically.
    pub summary: String,
    /// ObjId or a path relative to the session directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_ref: Option<String>,
    /// The source's final event.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terminal: bool,
}

/// `type = "control"` payload: session control, executed by the runner.
/// Never enters the LLM context and produces no input receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ControlCommand {
    Stop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// The only input a finished session still accepts.
    Decide {
        /// `accept | discard`.
        decision: String,
        #[serde(default)]
        by: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Subscribe {
        subscription: Subscription,
    },
    Unsubscribe {
        id: String,
    },
    /// Activity declaration (e.g. `agent-session activity --touch <ref>`).
    Activity {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        touch: Vec<Touching>,
        #[serde(default)]
        clear: bool,
    },
    /// A perception record of this session (the former `perception` input).
    Perceive {
        #[serde(default = "default_perceive_kind")]
        kind: String,
        #[serde(default)]
        summary: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        objects: Vec<String>,
    },
}

fn default_perceive_kind() -> String {
    "observation".to_string()
}

const CONTROL_COMMANDS: &[&str] = &[
    "stop",
    "decide",
    "subscribe",
    "unsubscribe",
    "activity",
    "perceive",
];

impl ControlCommand {
    pub fn name(&self) -> &'static str {
        match self {
            ControlCommand::Stop { .. } => "stop",
            ControlCommand::Decide { .. } => "decide",
            ControlCommand::Subscribe { .. } => "subscribe",
            ControlCommand::Unsubscribe { .. } => "unsubscribe",
            ControlCommand::Activity { .. } => "activity",
            ControlCommand::Perceive { .. } => "perceive",
        }
    }
}

/// What a bus record carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum SessionInput {
    Msg(SessionMsg),
    Event(AgentEvent),
    Control(ControlCommand),
}

impl SessionInput {
    pub fn type_name(&self) -> &'static str {
        match self {
            SessionInput::Msg(_) => INPUT_TYPE_MSG,
            SessionInput::Event(_) => INPUT_TYPE_EVENT,
            SessionInput::Control(_) => INPUT_TYPE_CONTROL,
        }
    }

    /// Inputs a finished session still accepts (`decide`).
    pub fn allowed_after_finish(&self) -> bool {
        matches!(self, SessionInput::Control(ControlCommand::Decide { .. }))
    }

    /// The payload as it travels (JSON object).
    pub fn payload(&self) -> Value {
        match self {
            SessionInput::Msg(m) => serde_json::to_value(m),
            SessionInput::Event(e) => serde_json::to_value(e),
            SessionInput::Control(c) => serde_json::to_value(c),
        }
        .unwrap_or(Value::Null)
    }
}

/// Why a record is refused (posting) or rejected (consuming). Deterministic:
/// the same record gets the same reason whenever and by whichever runner it
/// is consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// `schema` missing or not the current version.
    UnsupportedSchema,
    /// `type` is not `msg / event / control`.
    UnknownType,
    /// `key` / `from` / `at_ms` missing or out of range; a `msg` whose `key`
    /// is not the message's ObjId.
    InvalidEnvelope,
    PayloadNotJson,
    PayloadTooLarge,
    /// Required field missing, wrong type, constraint violated; a `msg` that
    /// fails `MsgObject::validate()`.
    InvalidPayload,
    /// `control` with an unknown `command`.
    UnknownCommand,
    /// Anything but `decide` after the session finished.
    SessionFinished,
    /// The session does not take msg / event inputs (`input_policy = none`).
    InputPolicy,
}

impl RejectReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RejectReason::UnsupportedSchema => "unsupported_schema",
            RejectReason::UnknownType => "unknown_type",
            RejectReason::InvalidEnvelope => "invalid_envelope",
            RejectReason::PayloadNotJson => "payload_not_json",
            RejectReason::PayloadTooLarge => "payload_too_large",
            RejectReason::InvalidPayload => "invalid_payload",
            RejectReason::UnknownCommand => "unknown_command",
            RejectReason::SessionFinished => "session_finished",
            RejectReason::InputPolicy => "input_policy",
        }
    }
}

/// A rejection with a diagnostic detail (the detail is not protocol).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rejected {
    pub reason: RejectReason,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Rejected {
    pub fn new(reason: RejectReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{}", self.reason.as_str())
        } else {
            write!(f, "{}: {}", self.reason.as_str(), self.detail)
        }
    }
}

impl From<Rejected> for OpenDanError {
    fn from(r: Rejected) -> Self {
        OpenDanError::InvalidArgument(format!("input refused ({r})"))
    }
}

fn bad(reason: RejectReason, detail: impl Into<String>) -> Rejected {
    Rejected::new(reason, detail)
}

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max
}

/// ObjId of a message in the string form used on the bus (the same form a
/// `thread.reply_to` / `relates_to.target` takes in JSON).
pub fn msg_key(msg: &MsgObject) -> String {
    msg.gen_obj_id().0.to_string()
}

fn parse_msg(key: &str, payload: &Value) -> std::result::Result<SessionMsg, Rejected> {
    let Some(raw) = payload.get("msg") else {
        return Err(bad(RejectReason::InvalidPayload, "payload.msg is missing"));
    };
    let (msg, obj_id) = MsgObject::from_json_value_checked(raw.clone())
        .map_err(|e| bad(RejectReason::InvalidPayload, format!("payload.msg: {e}")))?;
    if obj_id.to_string() != key {
        return Err(bad(
            RejectReason::InvalidEnvelope,
            format!("key is not the message ObjId ({obj_id})"),
        ));
    }
    if msg.content.content.trim().is_empty()
        && msg.content.refs.is_empty()
        && msg.content.machine.is_none()
    {
        return Err(bad(
            RejectReason::InvalidPayload,
            "message has no content, refs or machine payload",
        ));
    }
    // Attachments on the bus are data objects with an ObjId (guaranteed by
    // the type); local files are registered in the NamedStore first.
    let delivery: MsgDelivery = match payload.get("delivery") {
        None | Some(Value::Null) => MsgDelivery::default(),
        Some(d) => serde_json::from_value(d.clone())
            .map_err(|e| bad(RejectReason::InvalidPayload, format!("payload.delivery: {e}")))?,
    };
    for (name, v) in [
        ("from_name", &delivery.from_name),
        ("conversation_name", &delivery.conversation_name),
        ("record_id", &delivery.record_id),
        ("tunnel", &delivery.tunnel),
    ] {
        if v.as_ref().is_some_and(|s| s.len() > MAX_DELIVERY_FIELD_BYTES) {
            return Err(bad(
                RejectReason::InvalidPayload,
                format!("delivery.{name} exceeds {MAX_DELIVERY_FIELD_BYTES} bytes"),
            ));
        }
    }
    Ok(SessionMsg { msg, delivery })
}

fn parse_event(payload: &Value) -> std::result::Result<AgentEvent, Rejected> {
    let ev: AgentEvent = serde_json::from_value(payload.clone())
        .map_err(|e| bad(RejectReason::InvalidPayload, e.to_string()))?;
    if ev.source.kind.is_empty() {
        return Err(bad(RejectReason::InvalidPayload, "source.kind is empty"));
    }
    if ev.source.id.len() > MAX_EVENT_SOURCE_ID_BYTES
        || (ev.source.id.is_empty() && ev.source.kind != "system")
    {
        return Err(bad(RejectReason::InvalidPayload, "source.id is missing or too long"));
    }
    if !bounded(&ev.event, MAX_EVENT_NAME_BYTES) {
        return Err(bad(RejectReason::InvalidPayload, "event must be 1-64 bytes"));
    }
    if ev.summary.len() > MAX_EVENT_SUMMARY_BYTES {
        return Err(bad(
            RejectReason::InvalidPayload,
            format!("summary exceeds {MAX_EVENT_SUMMARY_BYTES} bytes"),
        ));
    }
    Ok(ev)
}

fn parse_control(payload: &Value) -> std::result::Result<ControlCommand, Rejected> {
    let Some(command) = payload.get("command").and_then(Value::as_str) else {
        return Err(bad(RejectReason::InvalidPayload, "command is missing"));
    };
    if !CONTROL_COMMANDS.contains(&command) {
        return Err(bad(RejectReason::UnknownCommand, command));
    }
    serde_json::from_value(payload.clone())
        .map_err(|e| bad(RejectReason::InvalidPayload, format!("{command}: {e}")))
}

/// Validate one record and parse its payload — the single rule set shared
/// by producers and consumers. Unknown payload fields are ignored (a
/// producer may be newer than the consumer); an unknown control command is
/// rejected.
pub fn parse_record(
    schema: Option<&str>,
    input_type: &str,
    key: &str,
    from: &str,
    at_ms: Option<u64>,
    payload: &[u8],
) -> std::result::Result<SessionInput, Rejected> {
    if schema != Some(SESSION_INPUT_SCHEMA) {
        return Err(bad(
            RejectReason::UnsupportedSchema,
            schema.unwrap_or("missing"),
        ));
    }
    if ![INPUT_TYPE_MSG, INPUT_TYPE_EVENT, INPUT_TYPE_CONTROL].contains(&input_type) {
        return Err(bad(RejectReason::UnknownType, input_type));
    }
    if !bounded(key, MAX_KEY_BYTES) || key.chars().any(char::is_control) {
        return Err(bad(
            RejectReason::InvalidEnvelope,
            "key must be 1-256 bytes without control characters",
        ));
    }
    if !bounded(from, MAX_FROM_BYTES) {
        return Err(bad(RejectReason::InvalidEnvelope, "from must be 1-256 bytes"));
    }
    if at_ms.is_none() {
        return Err(bad(RejectReason::InvalidEnvelope, "at_ms is missing or not a number"));
    }
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(bad(
            RejectReason::PayloadTooLarge,
            format!("{} bytes (limit {MAX_PAYLOAD_BYTES})", payload.len()),
        ));
    }
    let value: Value = serde_json::from_slice(payload)
        .map_err(|e| bad(RejectReason::PayloadNotJson, e.to_string()))?;
    if !value.is_object() {
        return Err(bad(RejectReason::PayloadNotJson, "payload is not a JSON object"));
    }
    Ok(match input_type {
        INPUT_TYPE_MSG => SessionInput::Msg(parse_msg(key, &value)?),
        INPUT_TYPE_EVENT => SessionInput::Event(parse_event(&value)?),
        _ => SessionInput::Control(parse_control(&value)?),
    })
}

/// Validate a logical record given as JSON exactly as a consumer would
/// (nothing is defaulted): the cross-language form of [`parse_record`].
pub fn parse_logical_record(record: &Value) -> std::result::Result<SessionInput, Rejected> {
    let text = |k: &str| record.get(k).and_then(Value::as_str);
    let payload = match record.get("payload") {
        Some(p) => serde_json::to_vec(p).unwrap_or_default(),
        None => b"null".to_vec(),
    };
    parse_record(
        text("schema"),
        text("type").unwrap_or_default(),
        text("key").unwrap_or_default(),
        text("from").unwrap_or_default(),
        record.get("at_ms").and_then(Value::as_u64),
        &payload,
    )
}

/// Producer side: one logical record to post.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PostedInput {
    pub schema: String,
    /// Dedup key of the logical input: a re-post keeps it, another input
    /// uses another one. `msg`: the message's ObjId.
    pub key: String,
    /// Poster principal (self-reported, audit only): neither the speaker nor
    /// a source of permissions.
    pub from: String,
    /// Posting time; display / diagnosis only, never ordering.
    pub at_ms: u64,
    #[serde(flatten)]
    pub input: SessionInput,
}

impl PostedInput {
    /// A message record: validates the MsgObject and takes its ObjId as key.
    pub fn msg(poster: &str, msg: MsgObject, delivery: MsgDelivery) -> Result<Self> {
        let key = msg_key(&msg);
        let p = Self {
            schema: SESSION_INPUT_SCHEMA.to_string(),
            key,
            from: poster.to_string(),
            at_ms: crate::now_ms(),
            input: SessionInput::Msg(SessionMsg { msg, delivery }),
        };
        p.validated()
    }

    /// A plain text message from `poster` to `agent` (`text_msg` + `msg`).
    pub fn text(poster: &str, agent_did: &str, text: impl Into<String>) -> Result<Self> {
        let from = did_of_principal(poster)?;
        let agent = parse_did(agent_did)?;
        Self::msg(poster, text_msg(&from, &agent, text), MsgDelivery::default())
    }

    pub fn event(poster: &str, key: impl Into<String>, event: AgentEvent) -> Self {
        Self {
            schema: SESSION_INPUT_SCHEMA.to_string(),
            key: key.into(),
            from: poster.to_string(),
            at_ms: crate::now_ms(),
            input: SessionInput::Event(event),
        }
    }

    pub fn control(poster: &str, key: impl Into<String>, command: ControlCommand) -> Self {
        Self {
            schema: SESSION_INPUT_SCHEMA.to_string(),
            key: key.into(),
            from: poster.to_string(),
            at_ms: crate::now_ms(),
            input: SessionInput::Control(command),
        }
    }

    /// Parse a hand-written logical record (CLI `post --json`). `schema`,
    /// `from` and `at_ms` may be omitted; so may the `key` of a `msg` (it is
    /// computed from the MsgObject). `src` / `index` belong to the channel
    /// and are refused.
    pub fn from_json(mut value: Value, poster: &str) -> Result<Self> {
        let obj = value
            .as_object_mut()
            .ok_or_else(|| OpenDanError::InvalidArgument("a record must be a JSON object".into()))?;
        for k in ["src", "index"] {
            if obj.contains_key(k) {
                return Err(OpenDanError::InvalidArgument(format!(
                    "`{k}` is assigned by the channel and cannot be posted"
                )));
            }
        }
        let text = |obj: &serde_json::Map<String, Value>, k: &str| {
            obj.get(k).and_then(Value::as_str).map(str::to_string)
        };
        let schema = text(obj, "schema").unwrap_or_else(|| SESSION_INPUT_SCHEMA.to_string());
        let input_type = text(obj, "type")
            .ok_or_else(|| OpenDanError::InvalidArgument("`type` is missing".into()))?;
        let from = text(obj, "from").unwrap_or_else(|| poster.to_string());
        let at_ms = match obj.get("at_ms") {
            None => crate::now_ms(),
            Some(v) => v
                .as_u64()
                .ok_or_else(|| OpenDanError::InvalidArgument("`at_ms` must be a number".into()))?,
        };
        let payload = obj.get("payload").cloned().unwrap_or(Value::Null);
        let key = match text(obj, "key") {
            Some(k) => k,
            None if input_type == INPUT_TYPE_MSG => {
                let raw = payload.get("msg").cloned().unwrap_or(Value::Null);
                let (_, id) = MsgObject::from_json_value_checked(raw)
                    .map_err(|e| OpenDanError::InvalidArgument(format!("payload.msg: {e}")))?;
                id.to_string()
            }
            None => return Err(OpenDanError::InvalidArgument("`key` is missing".into())),
        };
        let bytes = serde_json::to_vec(&payload)
            .map_err(|e| OpenDanError::InvalidArgument(format!("payload: {e}")))?;
        let input = parse_record(Some(&schema), &input_type, &key, &from, Some(at_ms), &bytes)?;
        Ok(Self {
            schema,
            key,
            from,
            at_ms,
            input,
        })
    }

    /// Payload bytes as they travel.
    pub fn payload_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.input.payload())
            .map_err(|e| OpenDanError::InvalidArgument(format!("payload: {e}")))
    }

    /// Check the record with the consumer's rules.
    pub fn validate(&self) -> Result<()> {
        let bytes = self.payload_bytes()?;
        parse_record(
            Some(&self.schema),
            self.input.type_name(),
            &self.key,
            &self.from,
            Some(self.at_ms),
            &bytes,
        )?;
        Ok(())
    }

    fn validated(self) -> Result<Self> {
        self.validate()?;
        Ok(self)
    }
}

/// Consumer side: one delivery fetched from an input source.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedInput {
    /// Input source id (`channels.inputs[].id`).
    pub src: String,
    /// Delivery position inside the source (kmsg index); consumption
    /// cursors and the cumulative ack use it.
    pub index: u64,
    /// `type` as posted (also for rejected records).
    pub kind: String,
    pub key: String,
    pub from: String,
    pub at_ms: u64,
    /// `Err`: the record is rejected (consumed, `input_rejected`).
    pub input: std::result::Result<SessionInput, Rejected>,
}

impl FetchedInput {
    pub fn input_ref(&self) -> InputRef {
        InputRef {
            src: self.src.clone(),
            index: self.index,
            key: self.key.clone(),
            kind: self.kind.clone(),
        }
    }

    pub fn msg(&self) -> Option<&SessionMsg> {
        match &self.input {
            Ok(SessionInput::Msg(m)) => Some(m),
            _ => None,
        }
    }

    pub fn event(&self) -> Option<&AgentEvent> {
        match &self.input {
            Ok(SessionInput::Event(e)) => Some(e),
            _ => None,
        }
    }

    pub fn control(&self) -> Option<&ControlCommand> {
        match &self.input {
            Ok(SessionInput::Control(c)) => Some(c),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// MsgObject construction helpers (construction only, no new wire format).
// ---------------------------------------------------------------------------

pub fn parse_did(did: &str) -> Result<DID> {
    if !did.starts_with("did:") {
        return Err(OpenDanError::InvalidArgument(format!("`{did}` is not a DID")));
    }
    DID::from_str(did).map_err(|e| OpenDanError::InvalidArgument(format!("DID `{did}`: {e}")))
}

/// DID a principal speaks as: a DID is taken as it is, an app principal
/// (`app:<appid>@<owner>`) speaks as its owner.
pub fn did_of_principal(principal: &str) -> Result<DID> {
    if principal.starts_with("did:") {
        return parse_did(principal);
    }
    let owner = crate::ids::parse_app_principal(principal)
        .map(|(_, owner)| owner)
        .unwrap_or_else(|| principal.to_string());
    parse_did(&format!("did:bns:{owner}"))
}

/// A text message for an agent: `kind = chat`, with `created_at_ms` and a
/// nonce (two messages with the same text get different ObjIds).
pub fn text_msg(from: &DID, agent: &DID, text: impl Into<String>) -> MsgObject {
    MsgObject {
        from: from.clone(),
        to: vec![agent.clone()],
        kind: MsgObjKind::Chat,
        created_at_ms: crate::now_ms(),
        nonce: Some(u64::from(uuid::Uuid::new_v4().as_u128() as u32)),
        content: MsgContent {
            content: text.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Append a data object attachment (`RefRole::Input`).
pub fn attach(mut msg: MsgObject, obj_id: ObjId, name: Option<String>) -> MsgObject {
    msg.content.refs.push(RefItem {
        role: RefRole::Input,
        target: RefTarget::DataObj {
            obj_id,
            uri_hint: name.clone(),
        },
        label: name,
    });
    msg
}

/// Mark the message as a reply to `target`.
pub fn reply_to(mut msg: MsgObject, target: ObjId) -> MsgObject {
    msg.thread.reply_to = Some(target);
    msg
}

// ---------------------------------------------------------------------------
// Input receipts
// ---------------------------------------------------------------------------

/// Where a message of a receipt batch lives inside the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessagePos {
    /// In `request.input` (behavior runs before their first step).
    RequestInput { index: u64 },
    /// In `state.accumulated` (function-call runs).
    Accumulated { index: u64 },
    /// Merged into a behavior step's `next_user_message`.
    Step { index: u64 },
}

pub const PART_SNAPSHOT: &str = "semi_subscription_snapshot";
pub const PART_INPUT: &str = "input";

pub const HOOK_ON_INIT: &str = "on_init";
pub const HOOK_ON_INPUT: &str = "on_input";
pub const HOOK_ON_CONTEXT_SWITCH: &str = "on_context_switch";

/// One message injected by a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReceiptPart {
    /// `semi_subscription_snapshot | input`.
    pub part: String,
    pub pos: MessagePos,
    /// The message's text block (evidence, worklog `user_message`). Image /
    /// document blocks are not repeated: read them at `pos` in the snapshot.
    pub text: String,
}

/// A semi-subscription state version a batch actually injected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventReceipt {
    #[serde(default)]
    pub subscription_id: Option<String>,
    pub source: EventSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    pub key: String,
}

/// Default reply path of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "route", rename_all = "snake_case")]
pub enum ReplyRoute {
    /// Reply the way the last consumed input message came.
    Message {
        /// The sender (one-to-one) or the group.
        to: String,
        #[serde(default)]
        to_session: Option<String>,
        /// MsgObject kind of the reply.
        kind: String,
        /// ObjId of the message replied to.
        #[serde(default)]
        reply_to: Option<String>,
        #[serde(default)]
        tunnel: Option<String>,
    },
    /// Hand the result back to the parent session.
    ParentSession { session_id: String },
}

impl ReplyRoute {
    /// The way `m` (bus key `key`) came.
    pub fn of_msg(key: &str, m: &SessionMsg) -> Self {
        let group = m.msg.kind == MsgObjKind::GroupMsg;
        let to = if group {
            m.msg.to.first().map(|d| d.to_string()).unwrap_or_default()
        } else {
            m.msg.from.to_string()
        };
        ReplyRoute::Message {
            to,
            to_session: m.msg.to_session.clone(),
            kind: msg_kind_name(&m.msg.kind),
            reply_to: Some(key.to_string()),
            tunnel: m.delivery.tunnel.clone(),
        }
    }
}

/// snake_case name of a MsgObject kind (`chat`, `group_msg`, ...).
pub fn msg_kind_name(kind: &MsgObjKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "chat".to_string())
}

/// Structured receipt of one input batch that entered the context.
/// Persisted inside the run snapshot together with its 1–2 messages;
/// recovery completes state from it and never renders again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InputReceipt {
    pub run_id: String,
    /// Monotonic within the run; batch id = (run_id, input_seq).
    pub input_seq: u64,
    /// Logical Turn the batch belongs to (`SessionState.open_turn`).
    pub turn: u64,
    /// `true` when this batch opened the Turn; hand-over and supplementary
    /// batches join the open Turn.
    #[serde(default)]
    pub opens_turn: bool,
    /// `on_init | on_input | on_context_switch`.
    pub hook: String,
    /// Delivery positions of the msg / Input events this batch consumed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<InputRef>,
    /// Semi-subscription state versions the snapshot message injected;
    /// cleared from `pending_events` by exact `(subscription_id, source,
    /// key)` (and `seq` when present).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<EventReceipt>,
    /// Default reply path after this batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<ReplyRoute>,
    /// The 1–2 messages of the batch, in injection order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ReceiptPart>,
    #[serde(default)]
    pub bootstrap: bool,
    /// Behavior runs: `next_step_index` when the batch was injected (orders
    /// a `request_input` message among the steps).
    #[serde(default)]
    pub after_step: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
    pub at_ms: u64,
}

/// libopendan host metadata stored in `LLMContextState.host["libopendan"]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HostMeta {
    pub session_id: String,
    /// Number of `request.input` messages assembled by the host before the
    /// run's first input batch (system + history). Messages after it belong
    /// to the run.
    #[serde(default)]
    pub base_input_len: u64,
    /// Behavior process entry the run belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_entry: Option<String>,
    /// Fork child runs: steps with `step_index` below this were inherited
    /// from the parent process and are never flushed by this run.
    #[serde(default)]
    pub inherited_below: u32,
    #[serde(default)]
    pub input_receipts: Vec<InputReceipt>,
    /// Mid-run history rewrites (context limit) of this run so far. Each
    /// rewrite starts a new epoch: the run's history before it is in the
    /// worklog, `request.input` is rebuilt from the session history.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub history_epoch: u64,
    /// Turn in effect when the current epoch started.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub epoch_turn: u64,
    /// Receipts with `input_seq ≤` this belong to earlier epochs: their
    /// message positions no longer apply (their identity still does).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub epoch_input_seq: u64,
    /// The run's inline media blocks were removed once after the provider
    /// refused the request (`input.media = inline` degraded to references);
    /// it is not tried again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub media_degraded: bool,
}

pub const HOST_META_KEY: &str = "libopendan";

fn is_zero(v: &u64) -> bool {
    *v == 0
}
