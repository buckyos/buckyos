//! UI sessions and the message tunnel: every inbox of the agent in
//! msg-center is bound to one UI session. Messages enter through the
//! session's input queue (the bridge here is the inbox's only consumer);
//! replies leave along the same conversation through the outbound sink.
//!
//! The binding is not stored anywhere of its own: the registry entry of a
//! UI session carries the inbox address as its `route_key`, and the current
//! session of an inbox is the latest unfinished one with that key.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{
    MailboxAddress, MailboxKind, MailboxRecordWithObject, MsgCenterClient, MsgEditCapability,
    PostSendResult, RecipientState,
};
use libopendan::api::create_session;
use libopendan::bridge::{route_msg_record, MsgBridgeCtx, MsgBridgeOutput, OutboundRecord, SlashCommand};
use libopendan::channel::{InputChannelFactory, Waker};
use libopendan::host::Supervisor;
use libopendan::protocol::*;
use libopendan::runner::{compose_text, OutboundSink, SendResult, TurnReply};
use libopendan::state::AgentStateClient;
use libopendan::{OpenDanError, SessionTemplate};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject};
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::{AgentConfig, ON_MSG_CHAT, ON_MSG_GROUP};

/// What the tunnel needs from msg-center.
#[async_trait]
pub trait MailService: Send + Sync {
    async fn list_inboxes(&self, owner: &DID) -> Result<Vec<MailboxAddress>, String>;
    /// The oldest unread record of `mailbox`, without locking it: `Reading`
    /// has no lease, a consumer that died would leave the record stuck.
    async fn next_unread(&self, mailbox: &MailboxAddress)
        -> Result<Option<MailboxRecordWithObject>, String>;
    async fn mark_read(&self, record_id: &str) -> Result<(), String>;
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
        buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| e.to_string())?
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
                return Ok((reply.answers_a_message() || reply.has_placeholder)
                    .then(|| self.notice(base, &self.texts.turn_failed))
                    .flatten());
            }
        }
        let answer = reply.answer.as_deref().unwrap_or_default();
        match compose_text(base.clone(), answer, None, None).await {
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
    pub mail: Arc<dyn MailService>,
    pub channels: Arc<dyn InputChannelFactory>,
    pub supervisor: Arc<Supervisor>,
    /// Where session directories are created.
    pub sessions_dir: PathBuf,
    status: Mutex<BTreeMap<String, InboxStatus>>,
    scan: Mutex<UiStatus>,
}

/// Session id of generation `generation` of the inbox `route_key`. The
/// address itself has `:`, `/` and `%`; a session id goes into kevent paths
/// and tmux names.
pub fn ui_session_id(agent_did: &str, route_key: &str, generation: usize) -> String {
    format!(
        "ui-{}",
        libopendan::ids::h(&["ui", agent_did, route_key, &generation.to_string()])
    )
}

impl UiModule {
    pub fn new(
        config: AgentConfig,
        agent: Arc<dyn AgentStateClient>,
        who: &str,
        mail: Arc<dyn MailService>,
        channels: Arc<dyn InputChannelFactory>,
        supervisor: Arc<Supervisor>,
        sessions_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            agent,
            who: who.to_string(),
            mail,
            channels,
            supervisor,
            sessions_dir,
            status: Mutex::new(BTreeMap::new()),
            scan: Mutex::new(UiStatus::default()),
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

    /// The session bound to `route_key`: the latest unfinished one, else a
    /// new generation created from the rule's template. `None`: the inbox is
    /// driven by someone else, or no rule takes this kind of message.
    async fn session_for(
        &self,
        route_key: &str,
        first: &MsgObject,
        key: &str,
    ) -> libopendan::Result<Option<String>> {
        let bound: Vec<RegistryEntry> = self
            .agent
            .sessions()
            .query(&RegistryQuery {
                kind: Some(SessionKind::Ui),
                ..Default::default()
            })
            .await?
            .into_iter()
            .filter(|e| e.route_key.as_deref() == Some(route_key))
            .collect();
        if let Some(current) = bound
            .iter()
            .filter(|e| e.status.run_state != RunState::Finished)
            .max_by_key(|e| e.status.updated_at_ms)
        {
            if current.driver.principal != self.who {
                self.note(route_key, |s| {
                    s.held = Some(format!("driven by {}", current.driver.principal))
                });
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
        let mut spec = template.spec(format!("{on} {route_key}"));
        spec.session_id = Some(ui_session_id(self.agent.agent_did(), route_key, bound.len()));
        spec.route_key = Some(route_key.to_string());
        spec.driver = Some(self.who.clone());
        spec.via = "opendan".to_string();
        spec.prompt.llm_context = self.config.llm_context.clone();
        spec.freeze = true;
        // The conversation an inbox stands for does not change: where the
        // replies go is fixed here and checked against every reply.
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
        spec.outbound = Some(OutboundBinding {
            to,
            to_session,
            kind,
        });
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
        let owner = did_of_principal(&self.who).ok();
        MsgBridgeCtx {
            agent_did: parse_did(self.agent.agent_did()).ok(),
            command_senders: owner.into_iter().collect(),
            commands: [("stop".to_string(), SlashCommand::Stop)].into_iter().collect(),
            contact_name: None,
            conversation_name: None,
        }
    }

    /// Move the unread records of one inbox into its session, oldest first.
    /// A record is acknowledged only after it was posted (or dropped for
    /// good); a full session queue holds the inbox at that record.
    async fn drain(&self, mailbox: &MailboxAddress) -> Result<(), String> {
        let route_key = mailbox.to_string();
        let ctx = self.bridge_ctx();
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
            let key = msg_key(&msg);
            let output = route_msg_record(&record, &msg, &ctx);
            let posted = match &output {
                MsgBridgeOutput::Drop { reason } => {
                    log::debug!("ui: record {} of {route_key} dropped: {reason}", record.record_id);
                    None
                }
                // No application command is registered: nothing to do.
                MsgBridgeOutput::AppCommand { .. } => None,
                MsgBridgeOutput::Deliver { .. } | MsgBridgeOutput::Control { .. } => {
                    let Some(sid) = self
                        .session_for(&route_key, &msg, &key)
                        .await
                        .map_err(|e| e.to_string())?
                    else {
                        return Ok(());
                    };
                    let input = output
                        .clone()
                        .into_input(&self.who, &msg)
                        .map_err(|e| e.to_string())?
                        .expect("deliver / control produce a record");
                    match self.agent.sessions().post_input(&sid, &input).await {
                        Ok(_) => Some(sid),
                        Err(e @ OpenDanError::InputFull { .. }) => {
                            // Back-pressure: not acknowledged upstream.
                            self.note(&route_key, |s| s.held = Some(e.to_string()));
                            let _ = self.supervisor.ensure_task(&sid, "input queue full").await;
                            return Ok(());
                        }
                        Err(e) => return Err(format!("post to {sid}: {e}")),
                    }
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
            .map(|d| format!("/msg_center/{}/INBOX/**", d.to_raw_host_name()));
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
