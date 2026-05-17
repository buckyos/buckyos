//! §9.6 of NewOpenDANRuntime — msg-center / kevent inbound pump.
//!
//! Bridges buckyos's msg-center inbox boxes into [`AIAgent::inbox()`]:
//!
//! ```text
//!   kevent_client.create_event_reader(["/msg_center/{owner}/box/**", ...])
//!       └── pull_event(1s) ──┐
//!                            ├─ Ok(Some(e))  → derive BoxKind from eventid → drain
//!                            ├─ Ok(None)     → sweep all inbox boxes (timeout fallback)
//!                            └─ Err(closed)  → drop reader, retry create
//!   msg_center.get_next(owner, box_kind, [Unread], lock_on_take=true)
//!       └── loop until None  → push Inbound::Msg{record_id, ...} into inbox_tx
//! ```
//!
//! Boundary discipline:
//!   - The pump is purely a *fetcher*: it translates buckyos-api items into
//!     `Inbound` variants and drops them onto the tokio mpsc into
//!     `AIAgent::run`. It does NOT ack — ack happens in the dispatcher
//!     *after* the item is durably parked on a session.
//!   - kevent is a poll accelerator, not the truth source: timeouts and
//!     reader resets MUST fall back to a full inbox sweep through the same
//!     `get_next` code path.

use std::sync::Arc;
use std::time::Duration;

use buckyos_api::{
    BoxKind, Event, EventReader, KEventClient, KEventError, MsgCenterClient, MsgRecordWithObject,
    MsgState,
};
use llm_context::{parse_msg_object, MsgParseOutput};

use crate::command_dispatcher::BUILTIN_COMMANDS;
use log::{debug, info, warn};
use name_lib::DID;
use tokio::sync::{mpsc, Notify};
use tokio::time::sleep;

use crate::agent::Inbound;
use crate::contact::ContactLookup;

/// Per-tick kevent wait. Short enough that shutdown latency stays bounded;
/// long enough that idle agents don't burn the CPU on RPC churn.
const EVENT_PULL_TIMEOUT_MS: u64 = 1_000;
/// Max msgs drained from a single box per dispatch tick — keeps a flood from
/// monopolizing the pump.
const MAX_MSG_PULL_PER_TICK: usize = 128;

/// Box names the kevent patterns alias to — keep in sync with msg-center's
/// publish side. The pump tolerates both `box/<name>` (canonical) and
/// `<name>` (legacy) eventid layouts.
const MSG_CENTER_EVENT_BOX_PATTERN_NAMES: [&str; 9] = [
    "in",
    "inbox",
    "INBOX",
    "group_in",
    "group_inbox",
    "GROUP_INBOX",
    "request",
    "request_box",
    "REQUEST_BOX",
];

/// Inputs to [`run`]. Bundled because `tokio::spawn` would otherwise want a
/// long argument list and we'd lose the field-name documentation at call
/// sites.
pub struct PumpConfig {
    pub agent_name: String,
    pub owner_did: DID,
    pub msg_center: Arc<MsgCenterClient>,
    pub kevent_client: Arc<KEventClient>,
    pub inbox_tx: mpsc::Sender<Inbound>,
    pub shutdown: Arc<Notify>,
    /// Optional contact lookup. When set the pump enriches inbound
    /// records whose `from_name` is missing — keeps the LLM prompt
    /// readable for unknown peers without spamming get_contact RPCs.
    pub contact_lookup: Option<Arc<ContactLookup>>,
}

/// Run the pump loop until `shutdown` is notified or the inbox sender's
/// receiver is dropped. Always returns cleanly — errors are logged and the
/// loop carries on, since dropping the pump would silently strand the agent.
pub async fn run(cfg: PumpConfig) {
    info!(
        "opendan.msg_pump[{}]: starting (owner={})",
        cfg.agent_name,
        cfg.owner_did.to_string()
    );
    let patterns = build_msg_center_event_patterns(&cfg.owner_did);
    let mut reader: Option<Arc<EventReader>> = None;

    loop {
        if cfg.inbox_tx.is_closed() {
            info!(
                "opendan.msg_pump[{}]: inbox receiver closed, exiting",
                cfg.agent_name
            );
            return;
        }

        // (Re)create the kevent reader on demand. Failure here is non-fatal —
        // we still fall through to a periodic sweep so that an unhealthy
        // kevent daemon doesn't make msg-center unreachable.
        if reader.is_none() {
            match cfg
                .kevent_client
                .create_event_reader(patterns.clone())
                .await
            {
                Ok(r) => {
                    info!(
                        "opendan.msg_pump[{}]: event_reader created reader_id={} patterns={:?}",
                        cfg.agent_name,
                        r.reader_id(),
                        patterns
                    );
                    reader = Some(Arc::new(r));
                }
                Err(err) => {
                    warn!(
                        "opendan.msg_pump[{}]: create_event_reader failed: {err:?} — sweeping inbox without kevent acceleration",
                        cfg.agent_name
                    );
                }
            }
        }

        // Decide which boxes to drain this tick. The kevent reader (when
        // present) acts as a hint — on miss/timeout we always sweep all
        // inbox-style boxes, per the "kevent is acceleration only" rule.
        let mut boxes_to_pull = Vec::<BoxKind>::new();

        if let Some(r) = reader.as_ref().cloned() {
            tokio::select! {
                _ = cfg.shutdown.notified() => {
                    info!("opendan.msg_pump[{}]: shutdown signal received", cfg.agent_name);
                    let _ = r.close().await;
                    return;
                }
                res = r.pull_event(Some(EVENT_PULL_TIMEOUT_MS)) => match res {
                    Ok(Some(event)) => {
                        debug!(
                            "opendan.msg_pump[{}]: event_pull_hit event_id={}",
                            cfg.agent_name, event.eventid
                        );
                        collect_event_pull_targets(&event, &mut boxes_to_pull);
                    }
                    Ok(None) => {
                        debug!(
                            "opendan.msg_pump[{}]: event_pull_timeout, falling back to box sweep",
                            cfg.agent_name
                        );
                        append_all_inbox_boxes(&mut boxes_to_pull);
                    }
                    Err(err) => {
                        warn!(
                            "opendan.msg_pump[{}]: pull_event failed: {err:?}",
                            cfg.agent_name
                        );
                        if matches!(err, KEventError::ReaderClosed(_)) {
                            reader = None;
                        }
                        append_all_inbox_boxes(&mut boxes_to_pull);
                    }
                }
            }
        } else {
            // No reader — degrade to a polled sweep with the same cadence
            // as the kevent timeout so the surrounding loop pacing is
            // unchanged. `select!` here so shutdown isn't blocked by sleep.
            tokio::select! {
                _ = cfg.shutdown.notified() => {
                    info!("opendan.msg_pump[{}]: shutdown signal received (no reader)", cfg.agent_name);
                    return;
                }
                _ = sleep(Duration::from_millis(EVENT_PULL_TIMEOUT_MS)) => {
                    append_all_inbox_boxes(&mut boxes_to_pull);
                }
            }
        }

        if boxes_to_pull.is_empty() {
            continue;
        }

        for box_kind in boxes_to_pull {
            drain_box(&cfg, box_kind).await;
        }
    }
}

async fn drain_box(cfg: &PumpConfig, box_kind: BoxKind) {
    let state_filter = match box_kind {
        BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => Some(vec![MsgState::Unread]),
        // Outbox kinds aren't drained by the agent.
        _ => return,
    };

    for attempt in 0..MAX_MSG_PULL_PER_TICK {
        match cfg
            .msg_center
            .get_next(
                cfg.owner_did.clone(),
                box_kind.clone(),
                state_filter.clone(),
                Some(true), // lock_on_take — moves record from Unread → Reading
                Some(true), // with_object — inline the MsgObject so we can lower it
            )
            .await
        {
            Ok(Some(record)) => {
                if !matches!(record.record.state, MsgState::Unread | MsgState::Reading) {
                    warn!(
                        "opendan.msg_pump[{}]: unexpected msg state record_id={} state={:?} — skipping box",
                        cfg.agent_name, record.record.record_id, record.record.state
                    );
                    break;
                }
                if !deliver_record(cfg, record).await {
                    // Either the receiver is closed (shutdown) or the
                    // record had nothing actionable — either way, stop
                    // draining; the outer loop handles shutdown.
                    if cfg.inbox_tx.is_closed() {
                        return;
                    }
                }
            }
            Ok(None) => {
                debug!(
                    "opendan.msg_pump[{}]: box={:?} drained after {} pulls",
                    cfg.agent_name, box_kind, attempt
                );
                break;
            }
            Err(err) => {
                warn!(
                    "opendan.msg_pump[{}]: get_next box={:?} failed: {err}",
                    cfg.agent_name, box_kind
                );
                break;
            }
        }
    }
}

/// Push one pulled record into the agent inbox channel. Returns `false`
/// when the record has no usable message payload (logged + dropped — the
/// dispatcher would have nothing to act on) or the inbox receiver is gone.
///
/// Note: this function does NOT ack the record back to msg-center. The
/// dispatcher does that after the session has durably parked the input,
/// so a crash here leaves the record in `Reading` and msg-center's lease
/// recovery will replay it on next boot.
async fn deliver_record(cfg: &PumpConfig, record: MsgRecordWithObject) -> bool {
    let record_id = record.record.record_id.clone();
    let Some(msg) = record.msg.as_ref() else {
        debug!(
            "opendan.msg_pump[{}]: drop record_id={} (missing MsgObject)",
            cfg.agent_name, record_id
        );
        return false;
    };
    let text = msg.content.content.trim().to_string();
    if text.is_empty() && msg.content.refs.is_empty() && msg.content.machine.is_none() {
        debug!(
            "opendan.msg_pump[{}]: drop record_id={} (empty message content)",
            cfg.agent_name, record_id
        );
        return false;
    }
    let from = record.record.from.to_raw_host_name();
    let from_did_str = record.record.from.to_string();
    let from_did = Some(from_did_str.clone()).filter(|s| !s.is_empty());
    let tunnel_did = record
        .record
        .route
        .as_ref()
        .and_then(|r| r.tunnel_did.as_ref().map(|d| d.to_string()))
        .filter(|s| !s.is_empty());
    let session_id = record
        .record
        .ui_session_id
        .clone()
        .filter(|s| !s.trim().is_empty());

    // `from_name` enrichment: prefer the value msg-center attached to the
    // record; otherwise consult ContactLookup. Failures stay as `None` —
    // the LLM still gets the raw `from` host as a fallback display token.
    let mut from_name = record
        .record
        .from_name
        .clone()
        .filter(|s| !s.trim().is_empty());
    if from_name.is_none() {
        if let Some(lookup) = cfg.contact_lookup.as_ref() {
            from_name = lookup.from_name(&record.record.from).await;
        }
    }

    // §3 — slash-command interception. `parse_msg_object` applies the
    // registered command whitelist at the protocol boundary, so user text
    // like `/etc/nginx ...` flows back into LLM inference unchanged.
    let inbound = match parse_msg_object(msg, BUILTIN_COMMANDS) {
        MsgParseOutput::ControlCommand(cmd) => Inbound::Command {
            record_id,
            from,
            from_did,
            tunnel_did,
            command: cmd.command,
            args: cmd.args,
        },
        MsgParseOutput::Message(ai_message) => Inbound::Msg {
            record_id,
            from,
            from_did,
            from_name,
            tunnel_did,
            session_id,
            text,
            ai_message,
        },
    };
    if let Err(err) = cfg.inbox_tx.send(inbound).await {
        warn!(
            "opendan.msg_pump[{}]: inbox send failed (receiver closed): {err}",
            cfg.agent_name
        );
        return false;
    }
    true
}

fn append_all_inbox_boxes(target: &mut Vec<BoxKind>) {
    for kind in [BoxKind::Inbox, BoxKind::GroupInbox, BoxKind::RequestBox] {
        if !target.contains(&kind) {
            target.push(kind);
        }
    }
}

/// Build the kevent patterns the agent subscribes to so msg-center
/// publishes accelerate inbox draining. Mirrors the legacy paths the
/// msg-center server emits today.
pub fn build_msg_center_event_patterns(owner: &DID) -> Vec<String> {
    let owner_token = owner.to_raw_host_name();
    let normalized = owner_token.to_ascii_lowercase();
    let mut owner_tokens = vec![owner_token.clone()];
    if normalized != owner_token {
        owner_tokens.push(normalized);
    }

    let mut out = Vec::new();
    for owner_token in owner_tokens {
        for box_name in MSG_CENTER_EVENT_BOX_PATTERN_NAMES {
            for pattern in [
                format!("/msg_center/{owner_token}/box/{box_name}/**"),
                format!("/msg_center/{owner_token}/{box_name}/**"),
            ] {
                if !out.contains(&pattern) {
                    out.push(pattern);
                }
            }
        }
    }
    out
}

fn collect_event_pull_targets(event: &Event, msg_pull_boxes: &mut Vec<BoxKind>) {
    if let Some(box_kind) = msg_center_event_box_kind(event) {
        if !msg_pull_boxes.contains(&box_kind) {
            msg_pull_boxes.push(box_kind);
        }
        return;
    }
    if event.eventid.starts_with("/msg_center/") {
        // Unknown /msg_center/ event id shape — be defensive and sweep all
        // inbox boxes rather than dropping a potentially relevant signal.
        warn!(
            "opendan.msg_pump: unrecognized msg-center event id={} — sweeping all inboxes",
            event.eventid
        );
        append_all_inbox_boxes(msg_pull_boxes);
    }
    // Non-msg_center events (custom kevent subscriptions) aren't routed yet —
    // §9.6's session_sub_kevent flow lands later.
}

fn msg_center_event_box_kind(event: &Event) -> Option<BoxKind> {
    let parts: Vec<&str> = event.eventid.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 || parts[0] != "msg_center" {
        return None;
    }
    if let Some(idx) = parts.iter().position(|p| *p == "box") {
        return parts
            .get(idx + 1)
            .and_then(|name| event_name_to_box_kind(name));
    }
    parts.get(2).and_then(|name| event_name_to_box_kind(name))
}

fn event_name_to_box_kind(raw: &str) -> Option<BoxKind> {
    let n = raw.trim().to_ascii_lowercase().replace('-', "_");
    match n.as_str() {
        "in" | "inbox" => Some(BoxKind::Inbox),
        "group_in" | "group_inbox" => Some(BoxKind::GroupInbox),
        "request" | "request_box" => Some(BoxKind::RequestBox),
        _ => None,
    }
}

/// Parse `agent.toml`'s `agent_did` into a `DID`. Returns `None` on empty /
/// malformed input — callers treat that as "no inbox pump", not as an error,
/// since first-boot agents may not have a DID yet.
pub fn parse_owner_did(raw: &str) -> Option<DID> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    DID::from_str(trimmed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn ev(id: &str) -> Event {
        Event {
            eventid: id.to_string(),
            source_node: "test".to_string(),
            source_pid: 0,
            ingress_node: None,
            timestamp: 0,
            data: Value::Null,
        }
    }

    #[test]
    fn classifies_box_kind_from_canonical_path() {
        let e = ev("/msg_center/alice/box/inbox/changed");
        assert_eq!(msg_center_event_box_kind(&e), Some(BoxKind::Inbox));
    }

    #[test]
    fn classifies_box_kind_from_legacy_path() {
        let e = ev("/msg_center/alice/request/changed");
        assert_eq!(msg_center_event_box_kind(&e), Some(BoxKind::RequestBox));
    }

    #[test]
    fn unknown_box_returns_none() {
        let e = ev("/msg_center/alice/box/mystery/changed");
        assert_eq!(msg_center_event_box_kind(&e), None);
    }

    #[test]
    fn timeout_fallback_appends_all_inbox_kinds() {
        let mut buf = Vec::new();
        append_all_inbox_boxes(&mut buf);
        assert!(buf.contains(&BoxKind::Inbox));
        assert!(buf.contains(&BoxKind::GroupInbox));
        assert!(buf.contains(&BoxKind::RequestBox));
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn parses_owner_did() {
        assert!(parse_owner_did("did:dev:alice").is_some());
        assert!(parse_owner_did("").is_none());
        assert!(parse_owner_did("   ").is_none());
    }
}
