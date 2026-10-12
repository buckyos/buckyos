//! System Notify: the contract an App uses to notify a zone user.
//!
//! A Notify is a `kind = Notify` MsgObject in msg-center whose machine
//! content carries [`NotifyMessageData`] (intent [`NOTIFY_INTENT`]). A
//! `notice` is only that message: read / dismissed / expired are states of
//! its mailbox records. An `input` additionally owns a TaskMgr HumanSet task
//! (`human.input/v1`, input [`NotifyTaskInput`]); waiting for and reading the
//! answer is waiting for and reading that task.
//!
//! Stable for Apps (fields may only be added): the `notify.*` methods, the
//! request / view types, the status, category and level enums, idempotency
//! and the permission rules. Where a notification is delivered and how
//! MessageHub groups it are not part of the contract.

use crate::{
    CommitResultReq, HumanPromptControl, HumanPromptOption, HumanTaskPrompt, Task,
    TaskDataResult, TaskOutcome, TaskPhase, TASK_ERR_ALREADY_COMPLETED,
    TASK_ERR_REVISION_CONFLICT,
};
use ::kRPC::{RPCErrors, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

pub const NOTIFY_INTENT: &str = "buckyos.notify";
/// Written into an Agent's own INBOX when a notification of that Agent was
/// also delivered into one of its conversations.
pub const NOTIFY_DELIVERED_INTENT: &str = "buckyos.notify.delivered";
pub const NOTIFY_DATA_VERSION: u32 = 1;
pub const NOTIFY_ID_PREFIX: &str = "nt-";
/// Owner-local sessions of a user that hold the notifications by category.
pub const NOTIFY_SESSION_PREFIX: &str = "sysnotify:";
pub const NOTIFY_RBAC_RESOURCE: &str = "obj://msg-center/notify";
pub const NOTIFY_CHOICE_CONTROL_ID: &str = "option";
pub const NOTIFY_TEXT_CONTROL_ID: &str = "text";
pub const NOTIFY_ACTION_ANSWER: &str = "answer";
pub const NOTIFY_ACTION_DISMISS: &str = "dismiss";
/// Path of the MessageHub card of a notification: `<path>?notify=<id>`.
pub const NOTIFY_LINK_PATH: &str = "/messagehub";

pub const METHOD_NOTIFY_SEND: &str = "notify.send";
pub const METHOD_NOTIFY_GET: &str = "notify.get";
pub const METHOD_NOTIFY_LIST: &str = "notify.list";
pub const METHOD_NOTIFY_ACK: &str = "notify.ack";
pub const METHOD_NOTIFY_DISMISS: &str = "notify.dismiss";
pub const METHOD_NOTIFY_CANCEL: &str = "notify.cancel";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    /// Tell the user something; nothing to answer.
    Notice,
    /// Ask the user; the answer is the result of the notification's task.
    Input,
}

impl NotifyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Input => "input",
        }
    }
}

/// How much a notification matters to the user. Derived by msg-center from
/// the caller: an App never names its own category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyCategory {
    /// Work assigned to the user (system services such as workflow).
    Todo,
    /// From an Agent: its work usually waits for the user.
    Agent,
    /// Ordinary application notification.
    App,
}

impl NotifyCategory {
    pub const ALL: [NotifyCategory; 3] = [Self::Todo, Self::Agent, Self::App];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Agent => "agent",
            Self::App => "app",
        }
    }

    pub fn session_id(self) -> String {
        format!("{}{}", NOTIFY_SESSION_PREFIX, self.as_str())
    }

    pub fn from_session_id(session_id: &str) -> Option<Self> {
        match session_id.strip_prefix(NOTIFY_SESSION_PREFIX)? {
            "todo" => Some(Self::Todo),
            "agent" => Some(Self::Agent),
            "app" => Some(Self::App),
            _ => None,
        }
    }
}

/// How strongly the sender wants to reach the user. Where that leads is up
/// to the system and may change between versions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    Quiet,
    #[default]
    Normal,
    Timely,
    Urgent,
}

impl NotifyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Normal => "normal",
            Self::Timely => "timely",
            Self::Urgent => "urgent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyStatus {
    Open,
    /// `notice` only: some record of it was read, or it was acknowledged.
    Read,
    /// `input` only.
    Answered,
    Dismissed,
    Expired,
    Canceled,
}

impl NotifyStatus {
    /// Nothing can change any more.
    pub fn is_closed(self) -> bool {
        !matches!(self, Self::Open | Self::Read)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Read => "read",
            Self::Answered => "answered",
            Self::Dismissed => "dismissed",
            Self::Expired => "expired",
            Self::Canceled => "canceled",
        }
    }
}

/// A conversation of the user with the source: where a copy may go and
/// where an answer given in chat is expected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyConversation {
    /// The user's endpoint (DID or tunnel endpoint DID).
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_session: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifySource {
    /// Filled by msg-center from the caller's identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Filled by msg-center when the caller is an Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    /// One sentence: what this is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// A page of the App about this matter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<NotifyConversation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifySendReq {
    /// Zone user id of the recipient.
    pub to_user: String,
    pub kind: NotifyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<NotifyLevel>,
    /// Only system services may name a category (`todo`); for anyone else
    /// it must equal the derived one or be absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<NotifyCategory>,
    pub prompt: HumanTaskPrompt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<NotifySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    /// Same caller + same key → the same notification, delivered once.
    pub idempotency_key: String,
}

impl NotifySendReq {
    pub fn notice(to_user: impl Into<String>, title: impl Into<String>, message: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            to_user: to_user.into(),
            kind: NotifyKind::Notice,
            level: None,
            category: None,
            prompt: HumanTaskPrompt {
                title: Some(title.into()),
                message: Some(message.into()).filter(|m: &String| !m.is_empty()),
                controls: Vec::new(),
            },
            source: None,
            expires_at_ms: None,
            idempotency_key: key.into(),
        }
    }

    /// An `input` with the prompt [`input_prompt`] builds.
    pub fn input(
        to_user: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
        options: Vec<HumanPromptOption>,
        allow_text: bool,
        key: impl Into<String>,
    ) -> Self {
        Self {
            to_user: to_user.into(),
            kind: NotifyKind::Input,
            level: None,
            category: None,
            prompt: input_prompt(title, message, options, allow_text),
            source: None,
            expires_at_ms: None,
            idempotency_key: key.into(),
        }
    }
}

/// The prompt of an `input`: a required `option` choice when there are
/// options, a `text` input when there are none or free text is allowed.
pub fn input_prompt(
    title: impl Into<String>,
    message: impl Into<String>,
    options: Vec<HumanPromptOption>,
    allow_text: bool,
) -> HumanTaskPrompt {
    let mut controls = Vec::new();
    let has_options = !options.is_empty();
    if has_options {
        controls.push(HumanPromptControl::Choice {
            id: NOTIFY_CHOICE_CONTROL_ID.to_string(),
            label: None,
            options,
            multiple: false,
            required: !allow_text,
        });
    }
    if !has_options || allow_text {
        controls.push(HumanPromptControl::TextInput {
            id: NOTIFY_TEXT_CONTROL_ID.to_string(),
            label: None,
            placeholder: None,
            default_value: None,
            multiline: true,
            required: !has_options,
        });
    }
    let message: String = message.into();
    HumanTaskPrompt {
        title: Some(title.into()),
        message: Some(message).filter(|m| !m.is_empty()),
        controls,
    }
}

/// The options of a prompt's `option` choice.
pub fn prompt_options(prompt: &HumanTaskPrompt) -> Vec<HumanPromptOption> {
    prompt
        .controls
        .iter()
        .find_map(|c| match c {
            HumanPromptControl::Choice { id, options, .. } if id == NOTIFY_CHOICE_CONTROL_ID => {
                Some(options.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// One mailbox record of a notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyRecordRef {
    /// Mailbox owner (the user, or for a copy the user as well).
    pub owner: String,
    pub session_id: String,
    pub record_id: String,
    pub msg_id: String,
    /// Sender of the message (the system notify sender, or the Agent).
    pub from: String,
}

/// A copy of an Agent's notification in one of its conversations with the
/// user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyDelivery {
    /// The Agent INBOX session (conversation) the copy belongs to.
    pub session_id: String,
    /// `native` or the tunnel platform.
    pub platform: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What the user (or someone on their behalf) submitted for an `input`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NotifyResult {
    /// `answer` or `dismiss`.
    pub action: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<i64>,
    /// `<user>|<app>` that committed it, from TaskMgr.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_by: Option<String>,
}

impl NotifyResult {
    pub fn option(&self) -> Option<&str> {
        self.values
            .get(NOTIFY_CHOICE_CONTROL_ID)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    pub fn text(&self) -> Option<&str> {
        self.values
            .get(NOTIFY_TEXT_CONTROL_ID)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifyView {
    pub notify_id: String,
    /// `https://<zone host>/messagehub?notify=<id>`, or the bare path when
    /// the zone host is unknown.
    pub url: String,
    pub to_user: String,
    pub kind: NotifyKind,
    pub category: NotifyCategory,
    pub level: NotifyLevel,
    pub status: NotifyStatus,
    pub prompt: HumanTaskPrompt,
    pub source: NotifySource,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_reason: Option<String>,
    /// Who acknowledged a `notice` on the user's behalf, when not the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<NotifyResult>,
    /// The record in the user's notification session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<NotifyRecordRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<NotifyDelivery>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NotifyListReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<NotifyStatus>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<NotifyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<NotifyCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<u64>,
    /// Zone user whose notifications to list; only system services may name
    /// someone else than the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NotifyListResult {
    pub items: Vec<NotifyView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyGetReq {
    pub notify_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyAckReq {
    pub notify_id: String,
    /// Set when someone acknowledges on the user's behalf (an Agent in the
    /// conversation the user answered in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_behalf: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyDismissReq {
    pub notify_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyCancelReq {
    pub notify_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `content.machine.data` of a notification message (`intent =
/// buckyos.notify`): the record in the user's notification session
/// (`home = true`) and every copy in a conversation carry the same data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifyMessageData {
    pub version: u32,
    pub notify_id: String,
    pub kind: NotifyKind,
    pub category: NotifyCategory,
    pub level: NotifyLevel,
    pub home: bool,
    pub source: NotifySource,
    pub prompt: HumanTaskPrompt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

/// `content.machine.data` of the record msg-center writes into an Agent's
/// own INBOX session after a copy reached that conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifyDeliveredData {
    pub notify_id: String,
    pub url: String,
    pub kind: NotifyKind,
    pub category: NotifyCategory,
    pub level: NotifyLevel,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<HumanPromptOption>,
    pub source: NotifySource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_msg_id: Option<String>,
    pub reply_route: NotifyConversation,
    pub at_ms: u64,
}

/// Input of the `human.input/v1` task of an `input` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifyTaskInput {
    pub notify: NotifyTaskEnvelope,
    pub prompt: HumanTaskPrompt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifyTaskEnvelope {
    pub notify_id: String,
    pub kind: NotifyKind,
    pub category: NotifyCategory,
    pub level: NotifyLevel,
    pub source: NotifySource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    pub url: String,
}

/// The answer to an `input`, or what to do with a `notice`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyAnswer {
    /// `true`: dismiss instead of answering.
    #[serde(default)]
    pub dismiss: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl NotifyAnswer {
    pub fn option(value: impl Into<String>) -> Self {
        Self {
            option: Some(value.into()),
            ..Default::default()
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self {
            text: Some(value.into()),
            ..Default::default()
        }
    }

    pub fn dismiss() -> Self {
        Self {
            dismiss: true,
            ..Default::default()
        }
    }

    /// The task result of this answer (`TaskDataResult::Human`).
    pub fn to_task_result(&self, submitted_by: &str, submitted_at_ms: u64) -> Value {
        let mut values = BTreeMap::new();
        if let Some(option) = self.option.as_ref().filter(|o| !o.is_empty()) {
            values.insert(NOTIFY_CHOICE_CONTROL_ID.to_string(), Value::String(option.clone()));
        }
        if let Some(text) = self.text.as_ref().filter(|t| !t.trim().is_empty()) {
            values.insert(NOTIFY_TEXT_CONTROL_ID.to_string(), Value::String(text.clone()));
        }
        serde_json::to_value(TaskDataResult::Human {
            action: Some(
                if self.dismiss {
                    NOTIFY_ACTION_DISMISS
                } else {
                    NOTIFY_ACTION_ANSWER
                }
                .to_string(),
            ),
            values,
            submitted_by: Some(submitted_by.to_string()),
            submitted_at: Some(submitted_at_ms as i64),
        })
        .unwrap_or(Value::Null)
    }

    /// Checks the answer against the prompt of an `input`.
    pub fn validate(&self, prompt: &HumanTaskPrompt) -> std::result::Result<(), String> {
        if self.dismiss {
            return Ok(());
        }
        let options = prompt_options(prompt);
        let accepts_text = prompt.controls.iter().any(
            |c| matches!(c, HumanPromptControl::TextInput { id, .. } if id == NOTIFY_TEXT_CONTROL_ID),
        );
        if let Some(option) = self.option.as_deref() {
            if !options.iter().any(|o| o.value == option) {
                return Err(format!(
                    "no option '{option}'; options: {}",
                    options
                        .iter()
                        .map(|o| o.value.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        let has_text = self.text.as_deref().is_some_and(|t| !t.trim().is_empty());
        if has_text && !accepts_text {
            return Err("this question takes one of its options, not free text".to_string());
        }
        if self.option.is_none() && !has_text {
            return Err("an answer needs an option or a text".to_string());
        }
        Ok(())
    }
}

/// `/messagehub?notify=<id>` under `https://<host>` when the host is known.
pub fn notify_url(zone_host: Option<&str>, notify_id: &str) -> String {
    let path = format!("{NOTIFY_LINK_PATH}?notify={notify_id}");
    match zone_host.map(str::trim).filter(|h| !h.is_empty()) {
        Some(host) => format!("https://{host}{path}"),
        None => path,
    }
}

/// Machine content `{intent, data}` of a message.
pub fn machine_content<T: Serialize>(intent: &str, data: &T) -> Result<ndn_lib::MachineContent> {
    let value = serde_json::to_value(data)
        .map_err(|e| RPCErrors::ReasonError(format!("encode {intent} data: {e}")))?;
    Ok(ndn_lib::MachineContent {
        intent: Some(intent.to_string()),
        data: serde_json::from_value(value)
            .map_err(|e| RPCErrors::ReasonError(format!("encode {intent} data: {e}")))?,
    })
}

/// The `intent` data of a message, when it carries that intent.
pub fn machine_data<T: serde::de::DeserializeOwned>(msg: &ndn_lib::MsgObject, intent: &str) -> Option<T> {
    let machine = msg.content.machine.as_ref()?;
    if machine.intent.as_deref() != Some(intent) {
        return None;
    }
    serde_json::from_value(serde_json::to_value(&machine.data).ok()?).ok()
}

/// The notification data of a message, when it is one.
pub fn notify_message_data(msg: &ndn_lib::MsgObject) -> Option<NotifyMessageData> {
    machine_data(msg, NOTIFY_INTENT)
}

/// The answer an `input` task carries, when it has one.
pub fn notify_task_result(task: &Task) -> Option<NotifyResult> {
    let value = task.result.as_ref()?;
    let TaskDataResult::Human {
        action,
        values,
        submitted_by,
        submitted_at,
    } = serde_json::from_value::<TaskDataResult>(value.clone()).ok()?
    else {
        return None;
    };
    Some(NotifyResult {
        action: action.unwrap_or_else(|| NOTIFY_ACTION_ANSWER.to_string()),
        values,
        submitted_by,
        submitted_at,
        committed_by: task
            .completed_by
            .as_ref()
            .map(|a| format!("{}|{}", a.user_id, a.app_id)),
    })
}

/// Status of an `input` from its task; `closed_reason` is the reason
/// msg-center recorded when it canceled the task itself.
pub fn input_status_of(task: &Task, closed_reason: Option<&str>) -> NotifyStatus {
    if !task.phase.is_terminal() {
        return NotifyStatus::Open;
    }
    match task.outcome {
        Some(TaskOutcome::Succeeded) => match notify_task_result(task) {
            Some(r) if r.action == NOTIFY_ACTION_DISMISS => NotifyStatus::Dismissed,
            _ => NotifyStatus::Answered,
        },
        _ if closed_reason == Some("expired") => NotifyStatus::Expired,
        _ => NotifyStatus::Canceled,
    }
}

/// The TaskMgr side of answering an `input`.
#[async_trait::async_trait]
pub trait NotifyTaskAccess: Send + Sync {
    async fn get_task(&self, task_id: &str) -> Result<Task>;
    async fn commit_result(&self, req: CommitResultReq) -> Result<Task>;
}

#[async_trait::async_trait]
impl NotifyTaskAccess for crate::TaskManagerClient {
    async fn get_task(&self, task_id: &str) -> Result<Task> {
        crate::TaskManagerClient::get_task(self, task_id).await
    }

    async fn commit_result(&self, req: CommitResultReq) -> Result<Task> {
        crate::TaskManagerClient::commit_result(self, req).await
    }
}

/// How an attempt to answer ended.
#[derive(Debug, Clone)]
pub enum NotifyCommit {
    /// This answer is the result.
    Committed(Task),
    /// Someone else was first (or the task was closed): the task as it is.
    AlreadyClosed(Task),
}

/// Commit `answer` to the task of an `input`: read the latest snapshot and
/// write with `expected_revision`; a lost race re-reads once more so the
/// caller sees the final state.
pub async fn commit_notify_answer(
    tasks: &dyn NotifyTaskAccess,
    task_id: &str,
    answer: &NotifyAnswer,
    submitted_by: &str,
    now_ms: u64,
) -> Result<NotifyCommit> {
    for _ in 0..3 {
        let task = tasks.get_task(task_id).await?;
        if task.phase == TaskPhase::Terminal || task.result.is_some() {
            return Ok(NotifyCommit::AlreadyClosed(task));
        }
        let req = CommitResultReq {
            task_id: task_id.to_string(),
            result: answer.to_task_result(submitted_by, now_ms),
            app_instance_id: None,
            runner_epoch: None,
            expected_revision: task.revision,
        };
        match tasks.commit_result(req).await {
            Ok(task) => return Ok(NotifyCommit::Committed(task)),
            Err(e) => match crate::task_mgr_error_code(&e) {
                Some(TASK_ERR_ALREADY_COMPLETED) => {
                    return Ok(NotifyCommit::AlreadyClosed(tasks.get_task(task_id).await?));
                }
                Some(TASK_ERR_REVISION_CONFLICT) => continue,
                _ => return Err(e),
            },
        }
    }
    let task = tasks.get_task(task_id).await?;
    if task.phase.is_terminal() {
        return Ok(NotifyCommit::AlreadyClosed(task));
    }
    Err(RPCErrors::ReasonError(format!(
        "task {task_id} keeps changing; answer not committed"
    )))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Wait until `fetch` reports a closed notification or `deadline_ms` passes;
/// returns the last view either way. `pause(d)` waits at most `d` (a kevent
/// read returns early).
pub async fn wait_notify_loop<F, Fut, P, PFut>(
    mut fetch: F,
    deadline_ms: u64,
    mut pause: P,
) -> Result<NotifyView>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<NotifyView>>,
    P: FnMut(Duration) -> PFut,
    PFut: Future<Output = ()>,
{
    let mut interval = Duration::from_millis(500);
    loop {
        let view = fetch().await?;
        if view.status.is_closed() {
            return Ok(view);
        }
        let now = now_ms();
        if now >= deadline_ms {
            return Ok(view);
        }
        let remaining = Duration::from_millis(deadline_ms - now);
        pause(interval.min(remaining)).await;
        interval = (interval * 2).min(Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActorRef, ChildControlPolicy, StorageDomain, TaskControlProfile, TaskExecutor,
        TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn task(revision: u64) -> Task {
        Task {
            task_id: "t1".into(),
            name: "notify".into(),
            parent_id: None,
            root_id: "t1".into(),
            child_control_policy: ChildControlPolicy::default(),
            schema_id: crate::HUMAN_INPUT_TASK_SCHEMA_ID.into(),
            schema_version: 1,
            input: json!({}),
            input_digest: String::new(),
            creator: ActorRef::new("ood1", "system:msg-center"),
            storage_domain: StorageDomain::User,
            idempotency_key: "notify:nt-1".into(),
            origin_ref: None,
            retry_of: None,
            supersedes: None,
            executor: TaskExecutor::HumanSet,
            runner_epoch: 0,
            assignees: Some(vec!["alice".into()]),
            phase: TaskPhase::Waiting,
            wait_reason: None,
            pending_control: None,
            control_profile: TaskControlProfile::baseline(0),
            progress: None,
            message: None,
            outcome: None,
            result: None,
            error: None,
            completed_by: None,
            policy_preset: TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1.into(),
            permission_boundary: false,
            revision,
            data_scope: None,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            archived_at: None,
        }
    }

    /// One task; a commit with a stale revision loses.
    struct OneTask(Arc<Mutex<Task>>);

    #[async_trait::async_trait]
    impl NotifyTaskAccess for OneTask {
        async fn get_task(&self, _task_id: &str) -> Result<Task> {
            Ok(self.0.lock().unwrap().clone())
        }

        async fn commit_result(&self, req: CommitResultReq) -> Result<Task> {
            let mut t = self.0.lock().unwrap();
            if t.phase.is_terminal() {
                return Err(crate::task_mgr_error(TASK_ERR_ALREADY_COMPLETED, "terminal"));
            }
            if t.revision != req.expected_revision {
                return Err(crate::task_mgr_error(TASK_ERR_REVISION_CONFLICT, "stale"));
            }
            t.result = Some(req.result);
            t.outcome = Some(TaskOutcome::Succeeded);
            t.phase = TaskPhase::Terminal;
            t.revision += 1;
            t.completed_by = Some(ActorRef::new("alice", "system:control-panel"));
            Ok(t.clone())
        }
    }

    #[test]
    fn message_data_and_view_round_trip() {
        let data = NotifyMessageData {
            version: NOTIFY_DATA_VERSION,
            notify_id: "nt-1".into(),
            kind: NotifyKind::Input,
            category: NotifyCategory::Agent,
            level: NotifyLevel::Timely,
            home: true,
            source: NotifySource {
                agent_did: Some("did:web:jarvis.test".into()),
                session_id: Some("work-1".into()),
                conversation: Some(NotifyConversation {
                    to: "did:bns:alice".into(),
                    to_session: None,
                }),
                ..Default::default()
            },
            prompt: input_prompt(
                "Which one?",
                "pick",
                vec![HumanPromptOption::new("a", "A")],
                true,
            ),
            task_id: Some("t1".into()),
            url: notify_url(Some("test.buckyos.io"), "nt-1"),
            expires_at_ms: Some(5),
        };
        let value = serde_json::to_value(&data).unwrap();
        assert_eq!(value["kind"], "input");
        assert_eq!(value["category"], "agent");
        assert_eq!(value["prompt"]["controls"][0]["id"], "option");
        assert_eq!(value["prompt"]["controls"][1]["kind"], "text_input");
        assert_eq!(value["url"], "https://test.buckyos.io/messagehub?notify=nt-1");
        let back: NotifyMessageData = serde_json::from_value(value).unwrap();
        assert_eq!(back, data);
        assert_eq!(NotifyCategory::from_session_id("sysnotify:agent"), Some(NotifyCategory::Agent));
        assert_eq!(NotifyCategory::Todo.session_id(), "sysnotify:todo");
        assert!(NotifyStatus::Expired.is_closed());
        assert!(!NotifyStatus::Read.is_closed());
        let req: NotifySendReq = serde_json::from_value(json!({
            "to_user": "alice", "kind": "notice", "prompt": {"title": "hi"},
            "idempotency_key": "k"
        }))
        .unwrap();
        assert_eq!(req.level, None);
        assert_eq!(NotifyLevel::default(), NotifyLevel::Normal);
    }

    #[test]
    fn answer_is_checked_against_the_prompt() {
        let choice = input_prompt("q", "", vec![HumanPromptOption::new("a", "A")], false);
        assert!(NotifyAnswer::option("a").validate(&choice).is_ok());
        assert!(NotifyAnswer::option("b").validate(&choice).is_err());
        assert!(NotifyAnswer::text("free").validate(&choice).is_err());
        assert!(NotifyAnswer::dismiss().validate(&choice).is_ok());
        let text = input_prompt("q", "", vec![], false);
        assert!(NotifyAnswer::text("free").validate(&text).is_ok());
        assert!(NotifyAnswer::default().validate(&text).is_err());
        let value = NotifyAnswer::option("a").to_task_result("alice", 7);
        assert_eq!(value["kind"], "human");
        assert_eq!(value["action"], "answer");
        assert_eq!(value["values"]["option"], "a");
        assert_eq!(value["submitted_by"], "alice");
    }

    #[tokio::test]
    async fn concurrent_answers_one_wins_the_other_sees_the_result() {
        let shared = Arc::new(Mutex::new(task(3)));
        let tasks = OneTask(shared.clone());
        let first = commit_notify_answer(&tasks, "t1", &NotifyAnswer::option("a"), "alice", 1)
            .await
            .unwrap();
        let NotifyCommit::Committed(done) = first else {
            panic!("first answer must win");
        };
        assert_eq!(input_status_of(&done, None), NotifyStatus::Answered);
        let second = commit_notify_answer(&tasks, "t1", &NotifyAnswer::text("late"), "alice", 2)
            .await
            .unwrap();
        let NotifyCommit::AlreadyClosed(t) = second else {
            panic!("second answer must lose");
        };
        let result = notify_task_result(&t).unwrap();
        assert_eq!(result.option(), Some("a"));
        assert_eq!(result.committed_by.as_deref(), Some("alice|system:control-panel"));

        // A stale snapshot loses the CAS, re-reads and still wins when the
        // task is open.
        struct Racy(OneTask, Mutex<bool>);
        #[async_trait::async_trait]
        impl NotifyTaskAccess for Racy {
            async fn get_task(&self, id: &str) -> Result<Task> {
                let mut t = self.0.get_task(id).await?;
                let mut first = self.1.lock().unwrap();
                if *first {
                    *first = false;
                    t.revision -= 1;
                }
                Ok(t)
            }
            async fn commit_result(&self, req: CommitResultReq) -> Result<Task> {
                self.0.commit_result(req).await
            }
        }
        let racy = Racy(OneTask(Arc::new(Mutex::new(task(5)))), Mutex::new(true));
        let r = commit_notify_answer(&racy, "t1", &NotifyAnswer::dismiss(), "alice", 3)
            .await
            .unwrap();
        let NotifyCommit::Committed(t) = r else {
            panic!("retry after a revision conflict");
        };
        assert_eq!(input_status_of(&t, None), NotifyStatus::Dismissed);
        let mut canceled = task(1);
        canceled.phase = TaskPhase::Terminal;
        canceled.outcome = Some(TaskOutcome::Canceled);
        assert_eq!(input_status_of(&canceled, Some("expired")), NotifyStatus::Expired);
        assert_eq!(input_status_of(&canceled, Some("ask_timeout")), NotifyStatus::Canceled);
    }

    fn view(status: NotifyStatus) -> NotifyView {
        NotifyView {
            notify_id: "nt-1".into(),
            url: notify_url(None, "nt-1"),
            to_user: "alice".into(),
            kind: NotifyKind::Input,
            category: NotifyCategory::App,
            level: NotifyLevel::Normal,
            status,
            prompt: HumanTaskPrompt::default(),
            source: NotifySource::default(),
            created_at_ms: 0,
            expires_at_ms: None,
            closed_at_ms: None,
            closed_reason: None,
            acked_by: None,
            task_id: None,
            result: None,
            home: None,
            deliveries: vec![],
        }
    }

    #[tokio::test]
    async fn waiting_ends_at_the_deadline_or_when_closed() {
        let start = now_ms();
        let polls = Arc::new(Mutex::new(0));
        let p = polls.clone();
        let v = wait_notify_loop(
            move || {
                *p.lock().unwrap() += 1;
                async { Ok(view(NotifyStatus::Open)) }
            },
            start + 300,
            |d| tokio::time::sleep(d),
        )
        .await
        .unwrap();
        assert_eq!(v.status, NotifyStatus::Open);
        assert!(now_ms() >= start + 300);
        assert!(*polls.lock().unwrap() >= 2);

        let n = Arc::new(Mutex::new(0));
        let c = n.clone();
        let v = wait_notify_loop(
            move || {
                let mut k = c.lock().unwrap();
                *k += 1;
                let status = if *k >= 2 {
                    NotifyStatus::Answered
                } else {
                    NotifyStatus::Open
                };
                async move { Ok(view(status)) }
            },
            start + 60_000,
            |_| async {},
        )
        .await
        .unwrap();
        assert_eq!(v.status, NotifyStatus::Answered);
    }
}
