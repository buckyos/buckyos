//! Session input message format (§4.5) and input receipts (§8.3).
//!
//! A kmsg message carries a JSON payload; its headers always contain `type`,
//! `key` (dedup key), `from` (self-reported poster principal, audit only) and
//! `at_ms`, optionally `intent` and `reply_to`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::Subscription;
use super::state::Touching;
use super::worklog::InputRef;

pub const HEADER_TYPE: &str = "type";
pub const HEADER_KEY: &str = "key";
pub const HEADER_FROM: &str = "from";
pub const HEADER_AT_MS: &str = "at_ms";
pub const HEADER_INTENT: &str = "intent";
pub const HEADER_REPLY_TO: &str = "reply_to";

/// kRPC bodies are capped at 1 MB and payloads travel as JSON number arrays,
/// so one payload must stay below ~250 KB (§4.5).
pub const MAX_PAYLOAD_BYTES: usize = 250 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Msg,
    Event,
    Change,
    Control,
    Perception,
}

impl InputKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            InputKind::Msg => "msg",
            InputKind::Event => "event",
            InputKind::Change => "change",
            InputKind::Control => "control",
            InputKind::Perception => "perception",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "msg" => InputKind::Msg,
            "event" => InputKind::Event,
            "change" => InputKind::Change,
            "control" => InputKind::Control,
            "perception" => InputKind::Perception,
            _ => return None,
        })
    }

    /// Kinds that enter the LLM context (and therefore need receipts).
    pub fn enters_context(&self) -> bool {
        matches!(self, InputKind::Msg | InputKind::Event | InputKind::Change)
    }
}

/// An input to post (producer side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Input {
    pub kind: InputKind,
    /// Producer dedup key. Change inputs coalesce by key; terminal changes use
    /// a separate `…#terminal` key.
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub payload: Value,
}

impl Input {
    pub fn msg(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            kind: InputKind::Msg,
            key: key.into(),
            intent: None,
            reply_to: None,
            payload: serde_json::json!({ "text": text.into() }),
        }
    }

    pub fn event(key: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: InputKind::Event,
            key: key.into(),
            intent: None,
            reply_to: None,
            payload,
        }
    }

    pub fn change(key: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: InputKind::Change,
            key: key.into(),
            intent: None,
            reply_to: None,
            payload,
        }
    }

    pub fn control(key: impl Into<String>, command: &ControlCommand) -> Self {
        Self {
            kind: InputKind::Control,
            key: key.into(),
            intent: None,
            reply_to: None,
            payload: serde_json::to_value(command).unwrap_or(Value::Null),
        }
    }

    pub fn perception(key: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: InputKind::Perception,
            key: key.into(),
            intent: None,
            reply_to: None,
            payload,
        }
    }

    /// Inputs a finished session still accepts (decide and query-like control).
    pub fn allowed_after_finish(&self) -> bool {
        if self.kind != InputKind::Control {
            return false;
        }
        matches!(
            serde_json::from_value::<ControlCommand>(self.payload.clone()),
            Ok(ControlCommand::Decide { .. })
        )
    }
}

/// `control` payloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ControlCommand {
    Stop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
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
}

impl ControlCommand {
    pub fn name(&self) -> &'static str {
        match self {
            ControlCommand::Stop { .. } => "stop",
            ControlCommand::Decide { .. } => "decide",
            ControlCommand::Subscribe { .. } => "subscribe",
            ControlCommand::Unsubscribe { .. } => "unsubscribe",
            ControlCommand::Activity { .. } => "activity",
        }
    }
}

/// An input as fetched from a source (consumer side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InputMessage {
    /// Input source id (`channels.inputs[].id`).
    pub src: String,
    /// Delivery instance id inside the source (kmsg index).
    pub index: u64,
    pub kind: InputKind,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub payload: Value,
    /// Set when the delivery could not be understood (unknown type, bad
    /// payload). Such inputs are marked consumed with an `input_rejected`
    /// worklog entry so the cumulative ack never gets stuck.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub malformed: Option<String>,
}

impl InputMessage {
    pub fn input_ref(&self) -> InputRef {
        InputRef {
            src: self.src.clone(),
            index: self.index,
            key: self.key.clone(),
            kind: self.kind.as_str().to_string(),
        }
    }

    /// Human readable text of the payload (`text` field or JSON dump).
    pub fn text(&self) -> String {
        match &self.payload {
            Value::String(s) => s.clone(),
            Value::Object(m) => match m.get("text").and_then(Value::as_str) {
                Some(t) => t.to_string(),
                None => self.payload.to_string(),
            },
            other => other.to_string(),
        }
    }

    pub fn control(&self) -> Option<ControlCommand> {
        if self.kind != InputKind::Control {
            return None;
        }
        serde_json::from_value(self.payload.clone()).ok()
    }
}

/// Where the message of a receipt batch lives inside the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessagePos {
    /// In `request.input` (first round of a new run).
    RequestInput { index: u64 },
    /// In `state.accumulated` (function-call runs).
    Accumulated { index: u64 },
    /// Attached to a behavior step as `next_user_message`.
    Step { index: u64 },
    /// Nothing rendered (e.g. bootstrap without inputs).
    None,
}

/// A consumed subscription change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeReceipt {
    /// `s1@16` style id.
    pub id: String,
    pub subscription: String,
    /// New cursor value for `state.subscription_cursors[subscription]`.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub cursor: Value,
}

/// Structured receipt of one batch of inputs that entered the context
/// (§8.3). Persisted inside the run snapshot together with the message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InputReceipt {
    pub run_id: String,
    /// Monotonic within the run; batch id = (run_id, input_seq).
    pub input_seq: u64,
    pub round: u64,
    /// `true` when this batch opened a new round (vs. observation injection).
    #[serde(default)]
    pub opens_round: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<InputRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<ChangeReceipt>,
    /// Inputs consumed by this batch without entering the context (coalesced
    /// changes, dropped changes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumed_only: Vec<InputRef>,
    pub message_pos: MessagePos,
    /// Rendered message text (evidence; also used for `user_message`).
    #[serde(default)]
    pub content: String,
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
    /// first round (system + history). Messages after it belong to the run.
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
}

pub const HOST_META_KEY: &str = "libopendan";
