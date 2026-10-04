//! The reply path out of a session: a Turn's reply becomes a complete
//! MsgObject in the commit that closes the Turn (`state.outbox`), is handed
//! to the host's [`OutboundSink`] after that commit, and is handed over
//! again — the same message under the same key — until the sink answers.

use std::sync::Arc;

use async_trait::async_trait;
use llm_context::msg_parser::{
    ai_message_to_msg_object_with_base_validated_async, AttachmentValidator, LocalFileResolver,
    MsgEgressOptions,
};
use ndn_lib::MsgObject;

use crate::bridge::{outbound_base, outbound_key, OutboundRecord};
use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::Session;

use super::shared::{commit_and_report, Shared};

/// The Turn a reply is produced for.
#[derive(Debug, Clone)]
pub struct TurnReply {
    pub turn: u64,
    pub status: TurnStatus,
    pub answer: Option<String>,
    /// Bus keys of the Turn's logical input (a message's key is its ObjId).
    pub inputs: Vec<String>,
    pub error: Option<serde_json::Value>,
}

impl TurnReply {
    /// Whether a message (not only system events) is among the inputs.
    pub fn answers_a_message(&self) -> bool {
        self.inputs.iter().any(|k| k.starts_with("cymsg:"))
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
    /// send). The default sends the answer of a completed Turn as text.
    async fn compose(
        &self,
        cfg: &SessionConfig,
        base: MsgObject,
        reply: &TurnReply,
    ) -> Result<Option<MsgObject>> {
        let _ = cfg;
        if reply.status != TurnStatus::Completed {
            return Ok(None);
        }
        compose_text(base, reply.answer.as_deref().unwrap_or_default(), None, None).await
    }

    async fn send(&self, sid: &str, record: &OutboundRecord) -> SendResult;
}

struct AllowAll;
impl AttachmentValidator for AllowAll {}

struct NoLocalFiles;
#[async_trait]
impl LocalFileResolver for NoLocalFiles {
    async fn resolve(&self, path: &str) -> std::result::Result<ndn_lib::ObjId, String> {
        Err(format!("local file `{path}` cannot be attached: the host has no file resolver"))
    }
}

/// Fill `base` with `answer` (text, `<attachment>` tags as refs). `None`
/// when there is neither text nor an attachment.
pub async fn compose_text(
    base: MsgObject,
    answer: &str,
    validator: Option<&dyn AttachmentValidator>,
    resolver: Option<&dyn LocalFileResolver>,
) -> Result<Option<MsgObject>> {
    if answer.trim().is_empty() {
        return Ok(None);
    }
    let message = buckyos_api::AiMessage::text(buckyos_api::AiRole::Assistant, answer);
    let msg = ai_message_to_msg_object_with_base_validated_async(
        &message,
        base,
        validator.unwrap_or(&AllowAll),
        MsgEgressOptions::default(),
        resolver.unwrap_or(&NoLocalFiles),
    )
    .await
    .map_err(|e| OpenDanError::InvalidArgument(format!("reply cannot be converted: {e}")))?;
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
pub(super) async fn queue_reply(sh: &Shared, s: &mut Session, run_id: &str, reply: TurnReply) {
    let Some(sink) = sh.deps.outbound.clone() else {
        return;
    };
    let Some(route) = s.state.reply.clone() else {
        return;
    };
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
        return;
    }
    let key = outbound_key(s.sid(), reply.turn, run_id, 0);
    if s.state.outbox.iter().any(|e| e.key == key) {
        return;
    }
    let base = match outbound_base(&route, &s.config.session.agent_did) {
        Ok(Some(base)) => base,
        Ok(None) => return,
        Err(e) => {
            log::warn!("session {}: reply envelope: {e}", s.sid());
            return;
        }
    };
    let composed = sink.compose(&s.config, base, &reply).await;
    let msg = match composed {
        Ok(Some(msg)) => msg,
        Ok(None) => return,
        Err(e) => {
            log::warn!("session {}: reply of turn {} not sent: {e}", s.sid(), reply.turn);
            return;
        }
    };
    let mismatch = s
        .config
        .channels
        .outbound
        .as_ref()
        .and_then(|b| route_mismatch(&route, b));
    if let Some(m) = &mismatch {
        log::error!("session {}: {m}", s.sid());
    }
    s.state.outbox.push(OutboxEntry {
        key,
        msg,
        turn: reply.turn,
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
                break;
            }
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
