//! The reply path out of a session: a Turn's reply becomes a complete
//! MsgObject in the commit that closes the Turn (`state.outbox`), is handed
//! to the host's [`OutboundSink`] after that commit, and is handed over
//! again — the same message under the same key — until the sink answers.
//!
//! A Turn bound to a task that is still open after `placeholder_delay` (or
//! reached its first tool call) and whose target can be edited later gets a
//! placeholder message first; its reply then leaves as one edit of that
//! placeholder. The Turn's task id travels as `agent_task` on the anchor
//! message (the placeholder, or the reply itself when there is none).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use llm_context::msg_parser::{
    ai_message_to_msg_object_with_base_validated_async, AttachmentValidator, LocalFileResolver,
    MsgEgressOptions,
};
use ndn_lib::{MsgObject, MsgRelType, MsgRelation, ObjId, RefItem, RefRole, RefTarget};
use serde_json::json;

use crate::bridge::{outbound_base, outbound_key, OutboundRecord};
use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::Session;

use super::reports::ObjectStore;
use super::shared::{commit_and_report, Shared};

/// The Turn a reply is produced for.
#[derive(Debug, Clone)]
pub struct TurnReply {
    pub turn: u64,
    pub status: TurnStatus,
    /// The answer; the final report's text when the Turn ended with one.
    pub answer: Option<String>,
    /// The final report's artifacts: attachments of the reply.
    pub artifacts: Vec<ReportArtifact>,
    /// The session's work directory: relative `<attachment>` paths of the
    /// answer are resolved against it (set when the reply is queued).
    pub workdir: Option<PathBuf>,
    /// The Turn's logical input (`<source>#<index>`).
    pub inputs: Vec<String>,
    /// A message (not only system events) is among the inputs.
    pub has_msg: bool,
    pub error: Option<serde_json::Value>,
    /// The reply replaces a placeholder already sent for the Turn: something
    /// has to end it, whatever the status.
    pub has_placeholder: bool,
}

impl TurnReply {
    pub fn answers_a_message(&self) -> bool {
        self.has_msg
    }
}

/// What the sink did with a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendResult {
    Sent {
        msg_id: Option<String>,
        deliveries: Vec<String>,
    },
    /// Refused for good: recorded, never retried.
    Rejected { reason: String },
    /// Not reached: the record stays pending.
    Retry { error: String },
}

/// Where a host sends the replies of the sessions it drives.
#[async_trait]
pub trait OutboundSink: Send + Sync {
    /// The message of `reply` on the envelope `base` (`None`: nothing to
    /// send); `objects` is the store of the session's runner. The default
    /// sends the answer of a completed Turn ([`compose_reply`]).
    async fn compose(
        &self,
        cfg: &SessionConfig,
        base: MsgObject,
        reply: &TurnReply,
        objects: Option<&dyn ObjectStore>,
    ) -> Result<Option<MsgObject>> {
        let _ = cfg;
        if reply.status != TurnStatus::Completed {
            return Ok(None);
        }
        compose_reply(base, reply, objects).await
    }

    /// The message telling the other side that `turn` is being worked on,
    /// on the envelope `base`. `None` (the default): the host has no such
    /// text, or the target cannot replace a message in place later — the
    /// Turn then only sends its reply.
    async fn placeholder(
        &self,
        cfg: &SessionConfig,
        base: MsgObject,
        turn: u64,
    ) -> Option<MsgObject> {
        let _ = (cfg, base, turn);
        None
    }

    /// A question to the user (`agent_ask`) about to be queued, on the
    /// reply envelope with its relation and meta set; the host may add
    /// what its channel needs (e.g. who is addressed in a group).
    async fn question(&self, cfg: &SessionConfig, msg: MsgObject) -> MsgObject {
        let _ = cfg;
        msg
    }

    async fn send(&self, sid: &str, record: &OutboundRecord) -> SendResult;
}

/// `meta` key of the Turn's task on its anchor message.
pub const AGENT_TASK_META: &str = "agent_task";

fn placeholder_key(sid: &str, turn: u64) -> String {
    format!("{sid}:{turn}:placeholder")
}

fn set_agent_task(msg: &mut MsgObject, task_id: &str) {
    msg.meta
        .insert(AGENT_TASK_META.to_string(), json!({ "task_id": task_id }));
}

/// The envelope of a message this session may send along its reply path
/// (`None`: a sub session, a reply to the agent itself, no message route).
fn reply_base(s: &Session) -> Option<(ReplyRoute, MsgObject)> {
    let route = s.state.reply.clone()?;
    // A sub session's result goes to its parent, whatever its inputs were
    // (its first inputs are messages of the agent itself); and the agent
    // never writes to itself.
    let to_parent = s
        .config
        .session
        .origin
        .as_ref()
        .is_some_and(|o| o.parent_session.is_some());
    let to_self = matches!(&route, ReplyRoute::Message { to, .. } if to == &s.config.session.agent_did);
    if to_parent || to_self {
        return None;
    }
    match outbound_base(&route, &s.config.session.agent_did) {
        Ok(Some(base)) => Some((route, base)),
        Ok(None) => None,
        Err(e) => {
            log::warn!("session {}: reply envelope: {e}", s.sid());
            None
        }
    }
}

fn binding_mismatch(s: &Session, route: &ReplyRoute) -> Option<String> {
    s.config
        .channels
        .outbound
        .as_ref()
        .and_then(|b| route_mismatch(route, b))
}

/// Queue the placeholder of the open Turn when it is due: the Turn has a
/// task, was opened by a message, has no placeholder yet, and is open for
/// `placeholder_delay` (`force`: a tool was called). Sent right away.
pub(super) async fn maybe_placeholder(sh: &Arc<Shared>, force: bool) {
    {
        let mut s = sh.session.lock().await;
        if !push_placeholder(sh, &mut s, force).await {
            return;
        }
        if let Err(e) = commit_and_report(sh, &mut s).await {
            log::warn!("session {}: placeholder not committed: {e}", sh.dir.sid());
            return;
        }
    }
    crate::fault::point("outbound:after_placeholder_commit");
    flush_outbox(sh).await;
}

/// Add the open Turn's placeholder to the outbox (uncommitted) when it is
/// due. `true`: one was added.
async fn push_placeholder(sh: &Shared, s: &mut Session, force: bool) -> bool {
    let Some(sink) = sh.deps.outbound.clone() else {
        return false;
    };
    let Some(b) = s.state.open_turn_task() else {
        return false;
    };
    let turn = b.turn;
    let task_id = b.task_id.clone();
    let waited = crate::now_ms().saturating_sub(b.opened_at_ms);
    let due = force || waited >= sh.deps.options.placeholder_delay.as_millis() as u64;
    if !b.has_msg || b.placeholder.is_some() || b.closed.is_some() || !due {
        return false;
    }
    let key = placeholder_key(s.sid(), turn);
    if s.state.outbox.iter().any(|e| e.key == key) {
        return false;
    }
    let Some((route, base)) = reply_base(s) else {
        return false;
    };
    if binding_mismatch(s, &route).is_some() {
        return false;
    }
    let Some(mut msg) = sink.placeholder(&s.config, base, turn).await else {
        return false;
    };
    set_agent_task(&mut msg, &task_id);
    let anchor = msg_key(&msg);
    s.state.outbox.push(OutboxEntry {
        key,
        msg,
        turn,
        purpose: OutboxPurpose::Placeholder,
        status: OutboxStatus::Pending,
        msg_id: None,
        deliveries: Vec::new(),
        error: None,
        attempts: 0,
        updated_at_ms: crate::now_ms(),
    });
    if let Some(b) = s.state.turn_tasks.iter_mut().find(|b| b.turn == turn) {
        b.placeholder = Some(anchor);
    }
    true
}

fn question_key(sid: &str, turn: u64, call_id: &str) -> String {
    format!("{sid}:{turn}:ask:{call_id}")
}

/// Called inside the commit that makes the session wait for the user's
/// answer to `call_id`: the open Turn's placeholder is made sure of first
/// (the anchor of the questions, sent now whatever its delay), then the
/// question joins the outbox as a `thread` message on it. Without a
/// placeholder (a channel that cannot edit) the question is a message of
/// its own. Nothing is queued without a reply path.
pub(super) async fn queue_question(
    sh: &Shared,
    s: &mut Session,
    call_id: &str,
    ask: &UserInputRequest,
) {
    let Some(sink) = sh.deps.outbound.clone() else {
        return;
    };
    let turn = s.state.current_turn();
    let key = question_key(s.sid(), turn, call_id);
    if s.state.outbox.iter().any(|e| e.key == key) {
        return;
    }
    push_placeholder(sh, s, true).await;
    let Some((route, mut msg)) = reply_base(s) else {
        return;
    };
    let anchor = s
        .state
        .turn_tasks
        .iter()
        .find(|b| b.turn == turn && b.closed.is_none())
        .and_then(|b| b.placeholder.clone());
    msg.content.content = ask.text();
    msg.meta.insert(
        AGENT_ASK_META.to_string(),
        json!({
            "anchor": anchor.clone().unwrap_or_default(),
            "call_id": call_id,
            "turn": turn,
            "input": ask.input,
        }),
    );
    if let Some(target) = anchor.as_deref().and_then(|a| ObjId::new(a).ok()) {
        msg.relates_to = Some(MsgRelation::new(MsgRelType::Thread, target));
    }
    let msg = sink.question(&s.config, msg).await;
    let mismatch = binding_mismatch(s, &route);
    if let Some(m) = &mismatch {
        log::error!("session {}: {m}", s.sid());
    }
    s.state.outbox.push(OutboxEntry {
        key,
        msg,
        turn,
        purpose: OutboxPurpose::Question,
        status: if mismatch.is_some() {
            OutboxStatus::Failed
        } else {
            OutboxStatus::Pending
        },
        msg_id: None,
        deliveries: Vec::new(),
        error: mismatch,
        attempts: 0,
        updated_at_ms: crate::now_ms(),
    });
    trim(&mut s.state.outbox);
}

/// Ids the questions of `turn` were sent as (and their anchor): a message
/// replying to one of them answers a question.
pub(super) fn question_targets(state: &SessionState, turn: u64) -> Vec<String> {
    let mut ids: Vec<String> = state
        .turn_tasks
        .iter()
        .filter(|b| b.turn == turn)
        .filter_map(|b| b.placeholder.clone())
        .collect();
    for e in state
        .outbox
        .iter()
        .filter(|e| e.turn == turn && e.purpose == OutboxPurpose::Question)
    {
        ids.push(e.msg_id.clone().unwrap_or_else(|| msg_key(&e.msg)));
    }
    ids
}

/// Questions of `turn` not sent yet follow their anchor: the id the
/// placeholder was sent as, or none (they become messages of their own).
fn retarget_questions(state: &mut SessionState, turn: u64, anchor: Option<&str>) {
    let target = anchor.and_then(|id| ObjId::new(id).ok());
    for e in state.outbox.iter_mut().filter(|e| {
        e.turn == turn && e.purpose == OutboxPurpose::Question && e.status == OutboxStatus::Pending
    }) {
        e.msg.relates_to = target
            .clone()
            .map(|t| MsgRelation::new(MsgRelType::Thread, t));
        if let Some(ask) = e.msg.meta.get_mut(AGENT_ASK_META) {
            ask["anchor"] = json!(anchor.unwrap_or_default());
        }
    }
}

/// The placeholder of `turn` the reply has to replace. A placeholder that
/// was never handed to the sink is withdrawn instead: the reply then leaves
/// as a message of its own.
fn take_placeholder(s: &mut Session, turn: u64) -> Option<ObjId> {
    let anchor = s
        .state
        .turn_tasks
        .iter()
        .find(|b| b.turn == turn)
        .and_then(|b| b.placeholder.clone())?;
    let key = placeholder_key(s.sid(), turn);
    let unsent = s
        .state
        .outbox
        .iter()
        .any(|e| e.key == key && e.status == OutboxStatus::Pending && e.attempts == 0);
    let failed = s
        .state
        .outbox
        .iter()
        .any(|e| e.key == key && e.status == OutboxStatus::Failed);
    if unsent || failed {
        s.state.outbox.retain(|e| !(e.key == key && unsent));
        if let Some(b) = s.state.turn_tasks.iter_mut().find(|b| b.turn == turn) {
            b.placeholder = None;
        }
        retarget_questions(&mut s.state, turn, None);
        return None;
    }
    ObjId::new(&anchor).ok()
}

/// What ends a placeholder when the host has no text for the Turn's result.
fn closing_text(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Completed => "Done.",
        TurnStatus::Failed => "Failed.",
        TurnStatus::BudgetExhausted => "Stopped: the budget is exhausted.",
        TurnStatus::Stopped => "Stopped.",
    }
}

struct AllowAll;
impl AttachmentValidator for AllowAll {}

/// `<attachment path=…>` of an answer: the file is put in the host's
/// object store (a relative path is taken in the work directory).
struct AnswerFiles<'a> {
    objects: Option<&'a dyn ObjectStore>,
    workdir: Option<&'a Path>,
}

#[async_trait]
impl LocalFileResolver for AnswerFiles<'_> {
    async fn resolve(&self, path: &str) -> std::result::Result<ObjId, String> {
        let Some(objects) = self.objects else {
            return Err(format!("local file `{path}` cannot be attached: the host has no object store"));
        };
        let path = Path::new(path);
        let file = match self.workdir {
            _ if path.is_absolute() => path.to_path_buf(),
            Some(dir) => dir.join(path),
            None => return Err(format!("`{}` is relative and there is no work directory", path.display())),
        };
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        objects.put_file(&file, &name).await
    }
}

/// Fill `base` with the reply: its answer (text, `<attachment>` tags as
/// refs, local files put in `objects`) and the final report's artifacts as
/// object refs. An artifact without an object is named in the text. `None`
/// when there is neither text nor an attachment.
pub async fn compose_reply(
    base: MsgObject,
    reply: &TurnReply,
    objects: Option<&dyn ObjectStore>,
) -> Result<Option<MsgObject>> {
    let mut answer = reply.answer.clone().unwrap_or_default();
    for a in reply.artifacts.iter().filter(|a| a.obj_id.is_none()) {
        answer.push_str(&format!("\n\n[{}] (not attached)", a.name()));
    }
    let mut msg = if answer.trim().is_empty() {
        base
    } else {
        let message = buckyos_api::AiMessage::text(buckyos_api::AiRole::Assistant, &answer);
        let files = AnswerFiles {
            objects,
            workdir: reply.workdir.as_deref(),
        };
        ai_message_to_msg_object_with_base_validated_async(
            &message,
            base,
            &AllowAll,
            MsgEgressOptions::default(),
            &files,
        )
        .await
        .map_err(|e| OpenDanError::InvalidArgument(format!("reply cannot be converted: {e}")))?
    };
    for a in &reply.artifacts {
        let Some(id) = &a.obj_id else { continue };
        let obj_id = ObjId::new(id)
            .map_err(|e| OpenDanError::InvalidArgument(format!("artifact {id}: {e}")))?;
        // A file is shown by its name; a forwarded object by the name it
        // carries, when the store knows it.
        let label = match objects {
            _ if !a.reference.is_empty() => Some(a.name().to_string()),
            Some(store) => store.name_of(&obj_id).await,
            None => None,
        };
        msg.content.refs.push(RefItem {
            role: RefRole::Input,
            target: RefTarget::DataObj {
                obj_id,
                uri_hint: None,
            },
            label,
        });
    }
    if msg.content.content.trim().is_empty() && msg.content.refs.is_empty() {
        return Ok(None);
    }
    Ok(Some(msg))
}

/// Why a reply along `route` must not leave a session bound to `binding`.
fn route_mismatch(route: &ReplyRoute, binding: &OutboundBinding) -> Option<String> {
    let ReplyRoute::Message {
        to,
        to_session,
        kind,
        ..
    } = route
    else {
        return None;
    };
    (to != &binding.to || to_session != &binding.to_session || kind != &binding.kind).then(|| {
        format!(
            "route_mismatch: the reply goes to {to} / {} ({kind}), the session is bound to {} / {} ({})",
            to_session.as_deref().unwrap_or("-"),
            binding.to,
            binding.to_session.as_deref().unwrap_or("-"),
            binding.kind
        )
    })
}

/// Called inside the commit that closes a Turn: the reply joins
/// `state.outbox` and lands with that commit.
pub(super) async fn queue_reply(sh: &Shared, s: &mut Session, run_id: &str, mut reply: TurnReply) {
    let Some(sink) = sh.deps.outbound.clone() else {
        return;
    };
    let key = outbound_key(s.sid(), reply.turn, run_id, 0);
    if s.state.outbox.iter().any(|e| e.key == key) {
        return;
    }
    let Some((route, base)) = reply_base(s) else {
        return;
    };
    let anchor = take_placeholder(s, reply.turn);
    reply.has_placeholder = anchor.is_some();
    reply.workdir = crate::runtime::resolve_workdir(&sh.dir, &s.config, sh.agent_root.as_deref()).ok();
    let objects = sh.deps.objects.as_deref();
    let composed = match sink.compose(&s.config, base.clone(), &reply, objects).await {
        Ok(m) => m,
        Err(e) => {
            log::warn!("session {}: reply of turn {} not sent: {e}", s.sid(), reply.turn);
            None
        }
    };
    let mut msg = match (composed, &anchor) {
        (Some(msg), _) => msg,
        // A placeholder never stays "working" after its Turn ended.
        (None, Some(_)) => {
            let mut msg = base;
            msg.content.content = closing_text(reply.status).to_string();
            msg
        }
        (None, None) => return,
    };
    let purpose = match anchor {
        Some(target) => {
            msg.relates_to = Some(MsgRelation::new(MsgRelType::Edit, target));
            OutboxPurpose::FinalEdit
        }
        None => {
            if let Some(b) = s.state.turn_tasks.iter().find(|b| b.turn == reply.turn) {
                set_agent_task(&mut msg, &b.task_id);
            }
            OutboxPurpose::Reply
        }
    };
    let mismatch = binding_mismatch(s, &route);
    if let Some(m) = &mismatch {
        log::error!("session {}: {m}", s.sid());
    }
    s.state.outbox.push(OutboxEntry {
        key,
        msg,
        turn: reply.turn,
        purpose,
        status: if mismatch.is_some() {
            OutboxStatus::Failed
        } else {
            OutboxStatus::Pending
        },
        msg_id: None,
        deliveries: Vec::new(),
        error: mismatch,
        attempts: 0,
        updated_at_ms: crate::now_ms(),
    });
    trim(&mut s.state.outbox);
}

/// The placeholder of `turn` settled: its final edit follows the id the
/// message service gave it; without a placeholder at the other side the
/// reply leaves as a message of its own.
fn placeholder_settled(state: &mut SessionState, turn: u64, sent_as: Option<&str>) {
    let task_id = state
        .turn_tasks
        .iter()
        .find(|b| b.turn == turn)
        .map(|b| b.task_id.clone());
    if let Some(b) = state.turn_tasks.iter_mut().find(|b| b.turn == turn) {
        b.placeholder = sent_as.map(str::to_string);
    }
    retarget_questions(state, turn, sent_as);
    let Some(edit) = state.outbox.iter_mut().find(|e| {
        e.turn == turn && e.purpose == OutboxPurpose::FinalEdit && e.status == OutboxStatus::Pending
    }) else {
        return;
    };
    match sent_as.and_then(|id| ObjId::new(id).ok()) {
        Some(target) => edit.msg.relates_to = Some(MsgRelation::new(MsgRelType::Edit, target)),
        None => {
            edit.msg.relates_to = None;
            edit.purpose = OutboxPurpose::Reply;
            if let Some(task_id) = task_id {
                set_agent_task(&mut edit.msg, &task_id);
            }
        }
    }
}

fn trim(outbox: &mut Vec<OutboxEntry>) {
    let settled = outbox
        .iter()
        .filter(|e| e.status != OutboxStatus::Pending)
        .count();
    let mut drop = settled.saturating_sub(OUTBOX_KEEP_SETTLED);
    outbox.retain(|e| {
        if drop > 0 && e.status != OutboxStatus::Pending {
            drop -= 1;
            return false;
        }
        true
    });
}

/// A record the sink never took is given up after this many hand-overs
/// (hours, with the back-off below): later replies stop waiting behind it.
const MAX_SEND_ATTEMPTS: u32 = 240;

fn retry_after_ms(attempts: u32) -> u64 {
    (1000u64 << attempts.min(6)).min(60_000)
}

/// Hand the pending records to the sink, in order. A record the sink could
/// not reach stops the pass (later ones wait behind it) and is tried again
/// by a later drive; nothing here fails the drive.
pub(super) async fn flush_outbox(sh: &Arc<Shared>) {
    let Some(sink) = sh.deps.outbound.clone() else {
        return;
    };
    let sid = sh.dir.sid().to_string();
    // One hand-over at a time: the placeholder timer and the drive loop
    // both get here.
    let _one = sh.flush.lock().await;
    let mut changed = false;
    loop {
        let next = {
            let s = sh.session.lock().await;
            s.state
                .outbox
                .iter()
                .find(|e| e.status == OutboxStatus::Pending)
                .cloned()
        };
        let Some(entry) = next else {
            break;
        };
        if entry.attempts > 0
            && crate::now_ms() < entry.updated_at_ms + retry_after_ms(entry.attempts)
        {
            break;
        }
        let result = sink
            .send(
                &sid,
                &OutboundRecord {
                    key: entry.key.clone(),
                    msg: entry.msg.clone(),
                },
            )
            .await;
        crate::fault::point("outbound:after_send");
        let mut s = sh.session.lock().await;
        let Some(e) = s.state.outbox.iter_mut().find(|e| e.key == entry.key) else {
            break;
        };
        e.attempts += 1;
        e.updated_at_ms = crate::now_ms();
        changed = true;
        let mut retry = false;
        match result {
            SendResult::Sent { msg_id, deliveries } => {
                e.status = OutboxStatus::Sent;
                e.msg_id = msg_id;
                e.deliveries = deliveries;
                e.error = None;
            }
            SendResult::Rejected { reason } => {
                log::warn!("session {sid}: outbound {} rejected: {reason}", entry.key);
                e.status = OutboxStatus::Failed;
                e.error = Some(reason);
            }
            SendResult::Retry { error } if e.attempts >= MAX_SEND_ATTEMPTS => {
                log::error!("session {sid}: outbound {} given up after {} attempts: {error}", entry.key, e.attempts);
                e.status = OutboxStatus::Failed;
                e.error = Some(format!("gave up after {} attempts: {error}", e.attempts));
            }
            SendResult::Retry { error } => {
                log::warn!("session {sid}: outbound {} not sent, will retry: {error}", entry.key);
                e.error = Some(error);
                retry = true;
            }
        }
        if retry {
            break;
        }
        if entry.purpose == OutboxPurpose::Placeholder {
            let sent_as = match e.status {
                OutboxStatus::Sent => Some(e.msg_id.clone().unwrap_or_else(|| msg_key(&entry.msg))),
                _ => None,
            };
            placeholder_settled(&mut s.state, entry.turn, sent_as.as_deref());
        }
    }
    if changed {
        let mut s = sh.session.lock().await;
        trim(&mut s.state.outbox);
        if let Err(e) = commit_and_report(sh, &mut s).await {
            log::warn!("session {sid}: outbox state not committed: {e}");
        }
    }
}

/// Whether the session still has replies to hand over.
pub fn has_pending_outbound(state: &SessionState) -> bool {
    state
        .outbox
        .iter()
        .any(|e| e.status == OutboxStatus::Pending)
}
