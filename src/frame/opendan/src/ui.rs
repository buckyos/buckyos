//! UI sessions and the message tunnel: every inbox of the agent in
//! msg-center is bound to one UI session. Messages enter through the
//! session's input queue (the bridge here is the inbox's only consumer);
//! replies leave along the same conversation through the outbound sink.
//!
//! The binding is not stored anywhere of its own: the registry entry of a
//! UI session carries the inbox address as its `route_key`, and the current
//! session of an inbox is the latest unfinished one with that key.
//!
//! Only the owner's messages reach a session; in a group only those that
//! mention the agent, and only while the owner allows group chat. Anything
//! else is marked read and nothing more. A group request takes the group's
//! latest read messages along as context (`delivery.context`): their
//! speakers are kept, they carry no request of their own.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{
    MailboxAddress, MailboxKind, MailboxRecordWithObject, MsgCenterClient, MsgEditCapability,
    PostSendResult, RecipientState,
};
use libopendan::api::create_session;
use libopendan::bridge::{
    context_msg_record, outbound_base, route_msg_record, MsgBridgeCtx, MsgBridgeOutput,
    OutboundRecord, SlashCommand,
};
use libopendan::channel::{InputChannelFactory, Waker};
use libopendan::host::Supervisor;
use libopendan::protocol::*;
use libopendan::runner::{compose_reply, ObjectStore, OutboundSink, SendResult, TurnReply};
use libopendan::state::AgentStateClient;
use libopendan::{OpenDanError, SessionTemplate};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject};
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::{AgentConfig, ON_MSG_CHAT, ON_MSG_GROUP};
use crate::records::{self, AgentRecords};

/// How many of a group's latest read messages a request takes along.
pub const GROUP_CONTEXT: usize = 20;

/// What the tunnel needs from msg-center.
#[async_trait]
pub trait MailService: Send + Sync {
    async fn list_inboxes(&self, owner: &DID) -> Result<Vec<MailboxAddress>, String>;
    /// The oldest unread record of `mailbox`, without locking it: `Reading`
    /// has no lease, a consumer that died would leave the record stuck.
    async fn next_unread(&self, mailbox: &MailboxAddress)
        -> Result<Option<MailboxRecordWithObject>, String>;
    async fn mark_read(&self, record_id: &str) -> Result<(), String>;
    /// The latest `limit` read records of `mailbox`, oldest first.
    async fn recent_read(
        &self,
        mailbox: &MailboxAddress,
        limit: usize,
    ) -> Result<Vec<MailboxRecordWithObject>, String>;
    async fn post_send(&self, msg: MsgObject, key: &str) -> Result<PostSendResult, String>;
    /// Whether a message on the envelope `msg` could later be replaced in
    /// place at every target it would be delivered to.
    async fn edit_capability(&self, msg: MsgObject) -> Result<MsgEditCapability, String>;
}

/// msg-center of the zone this process runs in. The client is taken from
/// the runtime for every call: the process's session token is renewed.
pub struct ZoneMailService;

impl ZoneMailService {
    async fn client() -> Result<MsgCenterClient, String> {
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        runtime
            .renew_token_from_verify_hub()
            .await
            .map_err(|e| e.to_string())?;
        runtime
            .get_msg_center_client()
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl MailService for ZoneMailService {
    async fn list_inboxes(&self, owner: &DID) -> Result<Vec<MailboxAddress>, String> {
        Self::client()
            .await?
            .list_mailboxes(owner.clone(), MailboxKind::Inbox)
            .await
            .map_err(|e| e.to_string())
    }

    async fn next_unread(
        &self,
        mailbox: &MailboxAddress,
    ) -> Result<Option<MailboxRecordWithObject>, String> {
        Self::client()
            .await?
            .get_next(
                mailbox.clone(),
                MailboxKind::Inbox,
                Some(vec![RecipientState::Unread]),
                Some(false),
                Some(true),
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn mark_read(&self, record_id: &str) -> Result<(), String> {
        Self::client()
            .await?
            .update_record_state(record_id.to_string(), RecipientState::Read)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn recent_read(
        &self,
        mailbox: &MailboxAddress,
        limit: usize,
    ) -> Result<Vec<MailboxRecordWithObject>, String> {
        let page = Self::client()
            .await?
            .list_box_by_time(
                mailbox.clone(),
                MailboxKind::Inbox,
                Some(vec![RecipientState::Read]),
                Some(limit),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut items = page.items;
        items.reverse();
        Ok(items)
    }

    async fn post_send(&self, msg: MsgObject, key: &str) -> Result<PostSendResult, String> {
        Self::client()
            .await?
            .post_send(msg, Some(key.to_string()))
            .await
            .map_err(|e| e.to_string())
    }

    async fn edit_capability(&self, msg: MsgObject) -> Result<MsgEditCapability, String> {
        Self::client()
            .await?
            .get_edit_capability(msg)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Texts of the agent package (`i18n/<language>.toml`, table `[outbound]`).
/// A text the package does not carry is simply not sent.
#[derive(Debug, Clone, Default)]
pub struct OutboundTexts {
    /// Sent by msg-center in place of a reply whose delivery died.
    pub delivery_failure_notice: Option<String>,
    /// Sent when a Turn answering a message failed.
    pub turn_failed: Option<String>,
    /// Sent instead of `turn_failed` when the AI service refused the Turn
    /// under the owner's AI quota or resource policy, where others read it:
    /// no rule or balance (AI Center Quota PRD §9.6).
    pub ai_quota_limited: Option<String>,
    /// The same refusal told to the owner in a direct chat: where to look.
    pub ai_quota_limited_owner: Option<String>,
    /// The owner's model is not open to them (`resource_not_permitted`).
    pub ai_resource_not_permitted_owner: Option<String>,
    /// Sent when the reply could not be converted into a message.
    pub convert_failed: Option<String>,
    /// The placeholder of a Turn that takes long. Without it no placeholder
    /// is sent.
    pub accepted: Option<String>,
    /// Ends the placeholder of a stopped Turn.
    pub stopped: Option<String>,
    /// Ends the placeholder of a Turn that finished without a reply.
    pub finished: Option<String>,
}

impl OutboundTexts {
    pub fn load(agent_root: &std::path::Path, language: &str) -> Self {
        let read = |lang: &str| -> Option<toml::Table> {
            let text = std::fs::read_to_string(agent_root.join("i18n").join(format!("{lang}.toml"))).ok()?;
            toml::from_str::<toml::Table>(&text).ok()
        };
        let Some(table) = read(language).or_else(|| read("en")) else {
            return Self::default();
        };
        let get = |k: &str| {
            table
                .get("outbound")
                .and_then(|o| o.get(k))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
        };
        Self {
            delivery_failure_notice: get("delivery_failure_notice"),
            turn_failed: get("turn_failed"),
            ai_quota_limited: get("ai_quota_limited"),
            ai_quota_limited_owner: get("ai_quota_limited_owner"),
            ai_resource_not_permitted_owner: get("ai_resource_not_permitted_owner"),
            convert_failed: get("convert_failed"),
            accepted: get("accepted"),
            stopped: get("stopped"),
            finished: get("finished"),
        }
    }
}

/// Replies of hosted sessions → `post_send`.
pub struct MsgCenterSink {
    pub mail: Arc<dyn MailService>,
    pub texts: OutboundTexts,
}

impl MsgCenterSink {
    fn notice(&self, mut base: MsgObject, text: &Option<String>) -> Option<MsgObject> {
        let text = text.clone()?;
        base.content.content = text;
        // A notice never triggers another notice.
        base.meta
            .insert("delivery_failure_fallback".to_string(), Value::Bool(true));
        Some(base)
    }

    fn text(mut base: MsgObject, text: &Option<String>) -> Option<MsgObject> {
        base.content.content = text.clone()?;
        Some(base)
    }
}

#[async_trait]
impl OutboundSink for MsgCenterSink {
    async fn compose(
        &self,
        _cfg: &SessionConfig,
        base: MsgObject,
        reply: &TurnReply,
        objects: Option<&dyn ObjectStore>,
    ) -> libopendan::Result<Option<MsgObject>> {
        match reply.status {
            TurnStatus::Completed => {}
            // A stop is the user's own request; nothing to add, except to
            // end a placeholder.
            TurnStatus::Stopped => {
                return Ok(reply
                    .has_placeholder
                    .then(|| Self::text(base, &self.texts.stopped))
                    .flatten());
            }
            // Failures of Turns nobody asked for (timers, task events) have
            // nobody to be reported to.
            TurnStatus::Failed | TurnStatus::BudgetExhausted => {
                let refusal = reply
                    .error
                    .as_ref()
                    .filter(|error| {
                        error.get("kind").and_then(Value::as_str)
                            == Some(libopendan::runner::AI_QUOTA_ERROR_KIND)
                    })
                    .map(|error| error.get("refusal").and_then(Value::as_str).unwrap_or_default());
                // Only the owner's messages reach a session, but a group reads
                // the reply too: it learns only that the Agent is limited.
                let owner_only = base.kind != MsgObjKind::GroupMsg;
                let candidates = match refusal {
                    None => vec![&self.texts.turn_failed],
                    Some("resource_not_permitted") if owner_only => vec![
                        &self.texts.ai_resource_not_permitted_owner,
                        &self.texts.ai_quota_limited,
                        &self.texts.turn_failed,
                    ],
                    Some(_) if owner_only => vec![
                        &self.texts.ai_quota_limited_owner,
                        &self.texts.ai_quota_limited,
                        &self.texts.turn_failed,
                    ],
                    Some(_) => vec![&self.texts.ai_quota_limited, &self.texts.turn_failed],
                };
                let text = candidates
                    .into_iter()
                    .find(|text| text.is_some())
                    .unwrap_or(&self.texts.turn_failed);
                return Ok((reply.answers_a_message() || reply.has_placeholder)
                    .then(|| self.notice(base, text))
                    .flatten());
            }
        }
        match compose_reply(base.clone(), reply, objects).await {
            Ok(Some(mut msg)) => {
                if let Some(text) = &self.texts.delivery_failure_notice {
                    msg.meta
                        .insert("delivery_failure_notice".to_string(), json!(text));
                }
                Ok(Some(msg))
            }
            Ok(None) => Ok(reply
                .has_placeholder
                .then(|| Self::text(base, &self.texts.finished))
                .flatten()),
            Err(e) => {
                // The reply itself stays in the worklog.
                log::warn!("reply of turn {} could not be converted: {e}", reply.turn);
                Ok(self.notice(base, &self.texts.convert_failed))
            }
        }
    }

    async fn placeholder(
        &self,
        _cfg: &SessionConfig,
        base: MsgObject,
        _turn: u64,
    ) -> Option<MsgObject> {
        let msg = Self::text(base, &self.texts.accepted)?;
        // Unknown counts as not editable: the Turn then only sends its reply.
        match self.mail.edit_capability(msg.clone()).await {
            Ok(c) if c.editable => Some(msg),
            Ok(c) => {
                log::debug!(
                    "no placeholder for {:?}: {}",
                    msg.to,
                    c.reason.unwrap_or_else(|| "not editable".to_string())
                );
                None
            }
            Err(e) => {
                log::debug!("no placeholder for {:?}: edit capability unknown: {e}", msg.to);
                None
            }
        }
    }

    async fn send(&self, _sid: &str, record: &OutboundRecord) -> SendResult {
        match self.mail.post_send(record.msg.clone(), &record.key).await {
            Ok(r) if r.ok => SendResult::Sent {
                msg_id: Some(r.msg_id.to_string()),
                deliveries: r.deliveries.into_iter().map(|d| d.delivery_id).collect(),
            },
            Ok(r) => SendResult::Rejected {
                reason: r.reason.unwrap_or_else(|| "rejected by msg-center".to_string()),
            },
            Err(error) => SendResult::Retry { error },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InboxStatus {
    pub route_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub delivered: u64,
    pub dropped: u64,
    /// Why the inbox is not being drained right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    pub last_at_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UiStatus {
    pub scans: u64,
    pub last_scan_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub inboxes: Vec<InboxStatus>,
}

pub struct UiModule {
    pub config: AgentConfig,
    pub agent: Arc<dyn AgentStateClient>,
    pub who: String,
    /// The agent's owner: the only sender whose messages reach a session.
    pub owner: DID,
    /// Last known `settings.allow_group`, read again when a group message
    /// comes in.
    allow_group: AtomicBool,
    records: Arc<dyn AgentRecords>,
    pub mail: Arc<dyn MailService>,
    pub channels: Arc<dyn InputChannelFactory>,
    pub supervisor: Arc<Supervisor>,
    /// Where session directories are created.
    pub sessions_dir: PathBuf,
    status: Mutex<BTreeMap<String, InboxStatus>>,
    scan: Mutex<UiStatus>,
    /// Interruptions the conversation was already told about (session id →
    /// when it was interrupted).
    told: Mutex<BTreeMap<String, u64>>,
}

/// How long `rotate` waits for the archived session to end.
const ARCHIVE_WAIT: Duration = Duration::from_secs(20);

/// Built-in words of the chat commands, for a package without `[commands]`
/// in its i18n.
const COMMAND_TEXTS: &[(&str, &str)] = &[
    ("nothing_to_stop", "Nothing is in progress."),
    ("cleaned", "Started a new conversation. The earlier one is kept and can still be looked up."),
    ("already_new", "This is already a new conversation."),
    ("clean_refused", "Background work started in this conversation is still running: {works}. Stop it or wait for it, then start a new conversation."),
    ("clean_pending", "The earlier conversation is being closed; your next message starts a new one."),
    ("resumed", "Resumed {count} interrupted session(s)."),
    ("nothing_to_resume", "Nothing was interrupted."),
    ("interrupted_notice", "I restarted while working on your last message, so it is on hold. Send /resume to go on with it, or /stop to drop it; this message waits until then."),
    ("command_failed", "That did not work: {error}"),
];

/// Reservation key of generation `generation` of the inbox `route_key`. The
/// address itself has `:`, `/` and `%`; a session id goes into kevent paths.
pub fn ui_session_key(agent_did: &str, route_key: &str, generation: usize) -> String {
    format!(
        "ui-{}",
        libopendan::ids::h(&["ui", agent_did, route_key, &generation.to_string()])
    )
}

impl UiModule {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: AgentConfig,
        agent: Arc<dyn AgentStateClient>,
        who: &str,
        owner: DID,
        allow_group: bool,
        records: Arc<dyn AgentRecords>,
        mail: Arc<dyn MailService>,
        channels: Arc<dyn InputChannelFactory>,
        supervisor: Arc<Supervisor>,
        sessions_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            agent,
            who: who.to_string(),
            owner,
            allow_group: AtomicBool::new(allow_group),
            records,
            mail,
            channels,
            supervisor,
            sessions_dir,
            status: Mutex::new(BTreeMap::new()),
            scan: Mutex::new(UiStatus::default()),
            told: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn status(&self) -> UiStatus {
        let mut s = self.scan.lock().expect("scan").clone();
        s.inboxes = self.status.lock().expect("status").values().cloned().collect();
        s
    }

    fn note(&self, route_key: &str, f: impl FnOnce(&mut InboxStatus)) {
        let mut all = self.status.lock().expect("status");
        let s = all.entry(route_key.to_string()).or_insert_with(|| InboxStatus {
            route_key: route_key.to_string(),
            ..Default::default()
        });
        f(s);
        s.last_at_ms = libopendan::now_ms();
    }

    /// A word of a chat command in the agent's language (`i18n/<lang>.toml
    /// [commands]`, else English, else built in), `{name}` replaced.
    fn command_text(&self, key: &str, vars: &[(&str, String)]) -> String {
        let read = |lang: &str| -> Option<String> {
            let root = self.agent.agent_root()?;
            let text = std::fs::read_to_string(root.join("i18n").join(format!("{lang}.toml"))).ok()?;
            toml::from_str::<toml::Table>(&text)
                .ok()?
                .get("commands")?
                .get(key)?
                .as_str()
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
        };
        let mut text = read(&self.config.language)
            .or_else(|| read("en"))
            .or_else(|| COMMAND_TEXTS.iter().find(|(k, _)| *k == key).map(|(_, t)| t.to_string()))
            .unwrap_or_default();
        for (name, value) in vars {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    /// Say `text` in the conversation `msg` came from (no session involved).
    async fn say(&self, msg: &MsgObject, key: &str, what: &str, text: String) {
        let route = ReplyRoute::of_msg(
            key,
            &SessionMsg {
                msg: msg.clone(),
                delivery: MsgDelivery::default(),
            },
        );
        let base = match outbound_base(&route, self.agent.agent_did()) {
            Ok(Some(base)) => base,
            _ => return,
        };
        let mut out = base;
        out.content.content = text;
        if let Err(e) = self.mail.post_send(out, &format!("{what}:{key}")).await {
            log::warn!("ui: {what} not sent: {e}");
        }
    }

    /// The UI sessions bound to `route_key`, every generation.
    async fn generations(&self, route_key: &str) -> libopendan::Result<Vec<RegistryEntry>> {
        Ok(self
            .agent
            .sessions()
            .query(&RegistryQuery {
                kind: Some(SessionKind::Ui),
                ..Default::default()
            })
            .await?
            .into_iter()
            .filter(|e| e.route_key.as_deref() == Some(route_key))
            .collect())
    }

    fn latest_open(bound: &[RegistryEntry]) -> Option<&RegistryEntry> {
        bound
            .iter()
            .filter(|e| e.status.run_state != RunState::Finished)
            .max_by_key(|e| e.status.updated_at_ms)
    }

    /// The session of the conversation right now, if it has one.
    async fn current(&self, route_key: &str) -> libopendan::Result<Option<RegistryEntry>> {
        Ok(Self::latest_open(&self.generations(route_key).await?).cloned())
    }

    /// Unfinished work sessions below `sid` (every level).
    async fn unfinished_works(&self, sid: &str) -> libopendan::Result<Vec<RegistryEntry>> {
        let mut out: Vec<RegistryEntry> = Vec::new();
        let mut at = vec![sid.to_string()];
        while !at.is_empty() {
            let below = self.agent.sessions().children_of(&at).await?;
            at = below.iter().map(|e| e.session_id.clone()).collect();
            out.extend(below.into_iter().filter(|e| e.status.run_state != RunState::Finished));
        }
        Ok(out)
    }

    /// Archive the UI session `sid` (clean): it ends as stopped, kept as it
    /// is; the next message of its conversation opens a new generation.
    /// Refused while work it started is unfinished. Returns once the
    /// registry shows it finished (`finished: false`: still ending).
    pub async fn rotate(&self, sid: &str, who: &str) -> libopendan::Result<Value> {
        let entry = self
            .agent
            .sessions()
            .lookup(sid)
            .await?
            .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
        if entry.kind != SessionKind::Ui {
            return Err(OpenDanError::InvalidArgument(format!("session {sid} is not a UI session")));
        }
        if entry.driver.principal != self.who {
            return Err(OpenDanError::NotDriver {
                session_id: sid.to_string(),
                principal: self.who.clone(),
                driver: entry.driver.principal,
            });
        }
        if entry.status.run_state == RunState::Finished {
            return Ok(json!({ "archived": sid, "finished": true }));
        }
        let works = self.unfinished_works(sid).await?;
        if !works.is_empty() {
            return Err(OpenDanError::InvalidArgument(format!(
                "unfinished_works: {} work session(s) this conversation started are not finished ({}); stop them or wait, then start a new conversation",
                works.len(),
                works.iter().map(|e| e.session_id.as_str()).collect::<Vec<_>>().join(", ")
            )));
        }
        let input = PostedInput::control(
            who,
            format!("archive:{}", libopendan::ids::new_runner_id()),
            ControlCommand::Stop {
                reason: Some(format!("archived by {who}: a new conversation starts")),
                scope: StopScope::Archive,
            },
        );
        self.agent.sessions().post_input(sid, &input).await?;
        self.supervisor.ensure_task(sid, "archive").await?;
        let started = std::time::Instant::now();
        while started.elapsed() < ARCHIVE_WAIT {
            let finished = self
                .agent
                .sessions()
                .lookup(sid)
                .await?
                .is_none_or(|e| e.status.run_state == RunState::Finished);
            if finished {
                log::info!("ui: session {sid} archived by {who}");
                return Ok(json!({ "archived": sid, "finished": true }));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(json!({ "archived": sid, "finished": false }))
    }

    /// Resume what a restart interrupted in the conversation of `sid`: the
    /// session itself (its interrupted sub sessions follow), else the
    /// topmost interrupted sessions below it. Returns how many were told.
    pub async fn resume_interrupted(&self, sid: &str, who: &str) -> libopendan::Result<usize> {
        let mut told = 0;
        let mut at = vec![self
            .agent
            .sessions()
            .lookup(sid)
            .await?
            .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?];
        while let Some(e) = at.pop() {
            if e.status.run_state == RunState::Finished {
                continue;
            }
            if e.status.paused == Some(PauseSource::Interrupted) {
                let input = PostedInput::control(
                    who,
                    format!("resume:{}", libopendan::ids::new_runner_id()),
                    ControlCommand::Resume { reason: Some("resumed after the restart".into()) },
                );
                self.agent.sessions().post_input(&e.session_id, &input).await?;
                let _ = self.supervisor.ensure_task(&e.session_id, "resume").await;
                told += 1;
                continue;
            }
            at.extend(self.agent.sessions().children_of(&[e.session_id.clone()]).await?);
        }
        Ok(told)
    }

    /// A chat command the host handles itself. `Ok(false)`: hold the inbox.
    async fn command(&self, route_key: &str, msg: &MsgObject, key: &str, name: &str) -> Result<bool, String> {
        let current = self.current(route_key).await.map_err(|e| e.to_string())?;
        let who = self.owner.to_string();
        let text = match (name, current) {
            ("clean" | "new", None) => self.command_text("already_new", &[]),
            ("clean" | "new", Some(e)) => match self.rotate(&e.session_id, &who).await {
                Ok(r) if r["finished"] == true => self.command_text("cleaned", &[]),
                Ok(_) => self.command_text("clean_pending", &[]),
                Err(OpenDanError::InvalidArgument(m)) if m.starts_with("unfinished_works") => {
                    let works = self.unfinished_works(&e.session_id).await.map_err(|e| e.to_string())?;
                    let names = works
                        .iter()
                        .map(|w| w.title.clone().unwrap_or_else(|| w.session_id.clone()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.command_text("clean_refused", &[("works", names)])
                }
                Err(e) => self.command_text("command_failed", &[("error", e.to_string())]),
            },
            ("resume", None) => self.command_text("nothing_to_resume", &[]),
            ("resume", Some(e)) => match self.resume_interrupted(&e.session_id, &who).await {
                Ok(0) => self.command_text("nothing_to_resume", &[]),
                Ok(n) => self.command_text("resumed", &[("count", n.to_string())]),
                Err(e) => self.command_text("command_failed", &[("error", e.to_string())]),
            },
            _ => return Ok(true),
        };
        self.say(msg, key, "cmd", text).await;
        Ok(true)
    }

    /// A message reached a conversation whose Turn a restart interrupted:
    /// it waits in the queue, and the user is told once how to go on.
    async fn tell_interrupted(&self, sid: &str, msg: &MsgObject, key: &str) {
        let Ok(Some(entry)) = self.agent.sessions().lookup(sid).await else {
            return;
        };
        if entry.status.paused != Some(PauseSource::Interrupted) {
            return;
        }
        let at = libopendan::SessionDir::open(&entry.location)
            .and_then(|d| d.state())
            .ok()
            .and_then(|st| st.paused)
            .map(|p| p.at_ms)
            .unwrap_or_default();
        if self.told.lock().expect("told").insert(sid.to_string(), at) == Some(at) {
            return;
        }
        self.say(msg, key, "interrupted", self.command_text("interrupted_notice", &[])).await;
    }

    /// The session bound to `route_key`: the latest unfinished one, else a
    /// new generation created from the rule's template. `None`: the inbox is
    /// driven by someone else, its session is ending (the record waits for
    /// the next generation), or no rule takes this kind of message.
    async fn session_for(
        &self,
        route_key: &str,
        first: &MsgObject,
        key: &str,
        binding: Option<OutboundBinding>,
    ) -> libopendan::Result<Option<String>> {
        let bound = self.generations(route_key).await?;
        if let Some(current) = Self::latest_open(&bound) {
            if current.driver.principal != self.who {
                self.note(route_key, |s| {
                    s.held = Some(format!("driven by {}", current.driver.principal))
                });
                return Ok(None);
            }
            // The registry follows the session's own state: one that is
            // ending takes nothing more.
            let ending = libopendan::SessionDir::open(&current.location)
                .and_then(|d| d.state())
                .is_ok_and(|st| st.is_finished() || st.stop_requested);
            if ending {
                self.note(route_key, |s| s.held = Some(format!("session {} is ending", current.session_id)));
                let _ = self.supervisor.ensure_task(&current.session_id, "ending").await;
                return Ok(None);
            }
            return Ok(Some(current.session_id.clone()));
        }
        let on = if first.kind == MsgObjKind::GroupMsg {
            ON_MSG_GROUP
        } else {
            ON_MSG_CHAT
        };
        let Some(rule) = self.config.ui_rule(on) else {
            self.note(route_key, |s| s.held = Some(format!("no [[loader.ui]] rule for {on}")));
            return Ok(None);
        };
        let template = SessionTemplate::load(&rule.session_class, self.agent.agent_root())?;
        // The objective is the conversation's design intent; whose
        // conversation it is stays in the route key.
        let mut spec = template.spec(template.objective_or_default(on == ON_MSG_GROUP));
        spec.session_id = Some(
            self.agent
                .sessions()
                .reserve_id(
                    SessionKind::Ui,
                    &ui_session_key(self.agent.agent_did(), route_key, bound.len()),
                    Some(&self.sessions_dir),
                )
                .await?,
        );
        spec.route_key = Some(route_key.to_string());
        spec.owner = Some(self.owner.to_string());
        spec.driver = Some(self.who.clone());
        spec.via = "opendan".to_string();
        spec.prompt.llm_context = self.config.llm_context.clone();
        spec.freeze = true;
        // The conversation an inbox stands for does not change: where the
        // replies go is fixed here and checked against every reply. A
        // notification shown first brings its conversation's coordinates.
        spec.outbound = match binding {
            Some(b) => Some(b),
            None => {
                let ReplyRoute::Message {
                    to,
                    to_session,
                    kind,
                    ..
                } = ReplyRoute::of_msg(
                    key,
                    &SessionMsg {
                        msg: first.clone(),
                        delivery: MsgDelivery::default(),
                    },
                )
                else {
                    return Ok(None);
                };
                Some(OutboundBinding {
                    to,
                    to_session,
                    kind,
                })
            }
        };
        let sd = create_session(
            &self.sessions_dir,
            spec,
            self.agent.as_ref(),
            &self.who,
            self.channels.as_ref(),
        )
        .await?;
        log::info!("ui: inbox {route_key} bound to new session {}", sd.sid());
        Ok(Some(sd.sid().to_string()))
    }

    fn bridge_ctx(&self) -> MsgBridgeCtx {
        MsgBridgeCtx {
            agent_did: parse_did(self.agent.agent_did()).ok(),
            owner: self.owner.clone(),
            allow_group: self.allow_group.load(Ordering::SeqCst),
            commands: [
                ("stop", SlashCommand::Stop),
                ("cancel", SlashCommand::Stop),
                ("clean", SlashCommand::App),
                ("new", SlashCommand::App),
                ("resume", SlashCommand::App),
            ]
            .into_iter()
            .map(|(name, command)| (name.to_string(), command))
            .collect(),
            contact_name: None,
            conversation_name: None,
        }
    }

    /// The owner may switch group chat on or off while the agent runs.
    async fn refresh_allow_group(&self) -> bool {
        match records::settings(self.records.as_ref()).await {
            Ok(s) => {
                self.allow_group.store(s.allow_group, Ordering::SeqCst);
                s.allow_group
            }
            Err(e) => {
                log::warn!("ui: agent settings not readable ({e}); allow_group stays as it was");
                self.allow_group.load(Ordering::SeqCst)
            }
        }
    }

    /// Post one record to a session. `false`: the session cannot take it
    /// now (back-pressure, or its queue is being recreated by its driver);
    /// the inbox is held, nothing is acknowledged upstream.
    async fn post(&self, route_key: &str, sid: &str, input: &PostedInput) -> Result<bool, String> {
        match self.agent.sessions().post_input(sid, input).await {
            Ok(_) => Ok(true),
            Err(e @ OpenDanError::InputFull { .. }) => {
                self.note(route_key, |s| s.held = Some(e.to_string()));
                let _ = self.supervisor.ensure_task(sid, "input queue full").await;
                Ok(false)
            }
            Err(e @ OpenDanError::QueueMissing { .. }) => {
                self.note(route_key, |s| s.held = Some(e.to_string()));
                let _ = self.supervisor.ensure_task(sid, "input queue missing").await;
                Ok(false)
            }
            Err(e) => Err(format!("post to {sid}: {e}")),
        }
    }

    /// The group's latest read messages, oldest first, as context of the
    /// request `request_key` (re-posts are deduplicated by the session).
    async fn post_group_context(
        &self,
        mailbox: &MailboxAddress,
        route_key: &str,
        sid: &str,
        request_key: &str,
        ctx: &MsgBridgeCtx,
    ) -> Result<bool, String> {
        let mut seen: BTreeSet<String> = [request_key.to_string()].into_iter().collect();
        for MailboxRecordWithObject { record, msg } in self.mail.recent_read(mailbox, GROUP_CONTEXT).await? {
            let Some(msg) = msg else {
                continue;
            };
            if !seen.insert(msg_key(&msg)) {
                continue;
            }
            let Some(delivery) = context_msg_record(&record, &msg, ctx) else {
                continue;
            };
            let input = match PostedInput::msg(&self.who, msg, delivery) {
                Ok(i) => i,
                Err(e) => {
                    log::debug!("ui: record {} of {route_key} is no context: {e}", record.record_id);
                    continue;
                }
            };
            if !self.post(route_key, sid, &input).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Move the unread records of one inbox into its session, oldest first.
    /// A record is acknowledged only after it was posted (or dropped for
    /// good); a full session queue holds the inbox at that record.
    async fn drain(&self, mailbox: &MailboxAddress) -> Result<(), String> {
        let route_key = mailbox.to_string();
        let mut ctx = self.bridge_ctx();
        let mut settings_read = false;
        let mut last: Option<String> = None;
        loop {
            let Some(MailboxRecordWithObject { record, msg }) = self.mail.next_unread(mailbox).await?
            else {
                return Ok(());
            };
            if last.as_deref() == Some(record.record_id.as_str()) {
                return Err(format!("record {} stays unread after it was acknowledged", record.record_id));
            }
            let Some(msg) = msg else {
                log::warn!("ui: record {} of {route_key} has no message object; skipped", record.record_id);
                self.mail.mark_read(&record.record_id).await?;
                self.note(&route_key, |s| s.dropped += 1);
                last = Some(record.record_id);
                continue;
            };
            if msg.kind == MsgObjKind::GroupMsg && !settings_read {
                ctx.allow_group = self.refresh_allow_group().await;
                settings_read = true;
            }
            let key = msg_key(&msg);
            let output = route_msg_record(&record, &msg, &ctx);
            let posted = match &output {
                MsgBridgeOutput::Drop { reason } => {
                    log::debug!("ui: record {} of {route_key} dropped: {reason}", record.record_id);
                    None
                }
                MsgBridgeOutput::AppCommand { name, .. } => {
                    if !self.command(&route_key, &msg, &key, name).await? {
                        return Ok(());
                    }
                    None
                }
                // A control is for the conversation's session as it is: it
                // never opens one.
                MsgBridgeOutput::Control { command, .. } => {
                    let current = self.current(&route_key).await.map_err(|e| e.to_string())?;
                    let stop = matches!(command, ControlCommand::Stop { .. });
                    match current.filter(|e| !stop || e.status.turn_open) {
                        Some(e) => {
                            let input = output
                                .clone()
                                .into_input(&self.who, &msg)
                                .map_err(|e| e.to_string())?
                                .expect("a control produces a record");
                            if !self.post(&route_key, &e.session_id, &input).await? {
                                return Ok(());
                            }
                            Some(e.session_id)
                        }
                        None => {
                            if stop {
                                self.say(&msg, &key, "cmd", self.command_text("nothing_to_stop", &[])).await;
                            }
                            None
                        }
                    }
                }
                MsgBridgeOutput::Deliver { .. } | MsgBridgeOutput::Observe { .. } => {
                    let binding = match &output {
                        MsgBridgeOutput::Observe { binding, .. } => Some(binding.clone()),
                        _ => None,
                    };
                    let Some(sid) = self
                        .session_for(&route_key, &msg, &key, binding)
                        .await
                        .map_err(|e| e.to_string())?
                    else {
                        return Ok(());
                    };
                    if msg.kind == MsgObjKind::GroupMsg
                        && matches!(output, MsgBridgeOutput::Deliver { .. })
                        && !self.post_group_context(mailbox, &route_key, &sid, &key, &ctx).await?
                    {
                        return Ok(());
                    }
                    let input = output
                        .clone()
                        .into_input(&self.who, &msg)
                        .map_err(|e| e.to_string())?
                        .expect("deliver / observe produce a record");
                    if !self.post(&route_key, &sid, &input).await? {
                        return Ok(());
                    }
                    if matches!(output, MsgBridgeOutput::Deliver { .. }) {
                        self.tell_interrupted(&sid, &msg, &key).await;
                    }
                    Some(sid)
                }
            };
            self.mail.mark_read(&record.record_id).await?;
            match posted {
                Some(sid) => {
                    self.note(&route_key, |s| {
                        s.delivered += 1;
                        s.held = None;
                        s.session_id = Some(sid.clone());
                    });
                    if let Err(e) = self.supervisor.ensure_task(&sid, "message").await {
                        log::warn!("ui: session {sid} not hosted: {e}");
                    }
                }
                None => self.note(&route_key, |s| s.dropped += 1),
            }
            last = Some(record.record_id);
        }
    }

    /// One pass over every inbox of the agent.
    pub async fn scan(&self) -> Result<(), String> {
        let owner = parse_did(self.agent.agent_did()).map_err(|e| e.to_string())?;
        let inboxes = self.mail.list_inboxes(&owner).await?;
        let mut first_error = None;
        for mailbox in inboxes {
            if let Err(e) = self.drain(&mailbox).await {
                let route_key = mailbox.to_string();
                log::warn!("ui: inbox {route_key}: {e}");
                self.note(&route_key, |s| s.held = Some(e.clone()));
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Scan, then wait for an inbox event or the polling interval (events
    /// only speed things up), forever.
    pub async fn run(self: Arc<Self>, waker: Arc<dyn Waker>, poll: Duration) {
        let pattern = parse_did(self.agent.agent_did())
            .ok()
            .and_then(|d| buckyos_api::MailboxAddress::new(d, None).ok())
            .map(|mailbox| mailbox.owner_event_pattern());
        let mut backoff = Duration::from_secs(1);
        loop {
            let result = self.scan().await;
            {
                let mut s = self.scan.lock().expect("scan");
                s.scans += 1;
                s.last_scan_ms = libopendan::now_ms();
                s.last_error = result.clone().err();
            }
            match result {
                Ok(()) => {
                    backoff = Duration::from_secs(1);
                    waker.wait(pattern.as_deref(), poll).await;
                }
                Err(_) => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
}

/// `ui.conversation {peer_did, session_id?}`: the UI sessions of one
/// conversation — the one it is in now and every generation before it.
/// A session belongs to the conversation when its replies go to `peer_did`
/// in the chat session `session_id` (a `dm:` id also matches replies bound
/// to no chat session).
pub async fn conversation(agent: &dyn AgentStateClient, peer_did: &str, session_id: Option<&str>) -> libopendan::Result<Value> {
    let mut generations = Vec::new();
    for e in agent
        .sessions()
        .query(&RegistryQuery {
            kind: Some(SessionKind::Ui),
            ..Default::default()
        })
        .await?
    {
        let Ok(sd) = libopendan::SessionDir::open(&e.location) else {
            continue;
        };
        let (Ok(cfg), Ok(state)) = (sd.config(), sd.state()) else {
            continue;
        };
        let bound = cfg.channels.outbound.as_ref().is_some_and(|o| {
            o.to == peer_did
                && match (session_id, o.to_session.as_deref()) {
                    (None, _) => true,
                    (Some(want), Some(have)) => want == have,
                    (Some(want), None) => want.starts_with("dm:"),
                }
        });
        if bound {
            generations.push((e, cfg, state));
        }
    }
    generations.sort_by_key(|(_, cfg, _)| cfg.session.created_at_ms);
    let current = match generations.iter().rev().find(|(e, _, _)| e.status.run_state != RunState::Finished) {
        None => Value::Null,
        Some((e, cfg, state)) => {
            let mut works = 0;
            let mut at = vec![e.session_id.clone()];
            while !at.is_empty() {
                let below = agent.sessions().children_of(&at).await?;
                works += below.iter().filter(|c| c.status.run_state != RunState::Finished).count();
                at = below.into_iter().map(|c| c.session_id).collect();
            }
            let prompt = match &cfg.prompt.frozen {
                None => json!({ "frozen_at_ms": null, "current": true, "changed": [] }),
                Some(frozen) => {
                    let changed = match libopendan::state::refrozen(cfg, agent.behaviors(), agent.agent_root(), "ui.conversation").await {
                        Ok(next) => next
                            .prompt
                            .frozen
                            .as_ref()
                            .map(|new| libopendan::state::frozen_changes(frozen, new))
                            .unwrap_or_default(),
                        Err(err) => {
                            log::warn!("ui: prompt of {} not compared: {err}", e.session_id);
                            Vec::new()
                        }
                    };
                    json!({ "frozen_at_ms": frozen.frozen_at_ms, "current": changed.is_empty(), "changed": changed })
                }
            };
            json!({
                "session_id": e.session_id,
                "route_key": e.route_key,
                "run_state": e.status.run_state,
                "paused": state.paused.as_ref().map(|p| p.source),
                "needs_input": e.status.needs_input,
                "one_line_status": state.one_line_status,
                "open_turn": state.open_turn.as_ref().map(|t| json!({
                    "turn": t.index,
                    "task_id": state.open_turn_task().map(|b| b.task_id.clone()),
                })),
                "unfinished_works": works,
                "prompt": prompt,
            })
        }
    };
    Ok(json!({
        "current": current,
        "generations": generations.iter().map(|(e, cfg, state)| json!({
            "session_id": e.session_id,
            "created_at_ms": cfg.session.created_at_ms,
            "finished_at_ms": state.finished_at_ms.filter(|_| state.is_finished()),
            "outcome": state.outcome.filter(|_| state.is_finished()),
        })).collect::<Vec<_>>(),
    }))
}
