//! Inputs the runner produces for a multi-Turn Work Session (simple Work
//! Session TODO §3.3 / §3.4):
//!
//! - **timer ticks**: while the session waits for the work it started
//!   (tasks it follows, sub sessions it must hear from) and no Turn is
//!   open, every `timer:` subscription ticks each `policy.tick_secs`. An
//!   Active tick is an input that opens a Turn (the model replies with
//!   where things stand); a Semi one joins the pending events. Not gated:
//!   the interval is the cost control. The next tick is counted from the
//!   last one taken in, or from when the session began waiting.
//! - **the nudge**: a reply that neither ended the session nor waits for
//!   anything gets one correction in its Turn (`system:runner`, `nudge`);
//!   a second such reply fails the session (`missing_end_report`).
//!
//! Both are synthesized inputs like a task's end: no queue is involved,
//! the receipt of the batch that took them in records them.

use std::sync::Arc;

use crate::error::Result;
use crate::protocol::*;

use super::shared::{commit_and_report, Shared};

/// Event of a timer tick.
pub(super) const TICK_EVENT: &str = "tick";
/// `system:<id>` of the runner's own corrections.
pub(super) const RUNNER_SOURCE_ID: &str = "runner";
/// Event of the correction.
pub(super) const NUDGE_EVENT: &str = "nudge";

/// The correction a reply gets when nothing is running and the session did
/// not end.
pub(super) const NUDGE_TEXT: &str = "Nothing you started is running and this session has not ended. Finish with `report` (is_end: true) — what you did, where the results are and how you verified them — or start the next step now.";

/// Whether the session waits for work it started: tasks it follows, sub
/// sessions it must hear from.
pub(super) fn waits_for_work(state: &SessionState) -> bool {
    !state.watched_tasks.is_empty()
        || state
            .waiting_for
            .as_ref()
            .is_some_and(|w| matches!(w.kind, WaitingKind::Tool | WaitingKind::Children) && !w.refs.is_empty())
}

fn fmt_secs(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// The ticks due now (Active ones as inputs; Semi ones merged into the
/// pending events here).
pub(super) async fn poll_ticks(sh: &Arc<Shared>) -> Result<Vec<FetchedInput>> {
    let (cfg, state) = {
        let s = sh.session.lock().await;
        (s.config.clone(), s.state.clone())
    };
    let every_ms = cfg.session.policy.tick_secs.saturating_mul(1000);
    if every_ms == 0
        || state.run_state != RunState::Waiting
        || state.open_turn.is_some()
        || !waits_for_work(&state)
    {
        return Ok(Vec::new());
    }
    let timers: Vec<&Subscription> = cfg
        .subscriptions
        .iter()
        .filter(|s| matches!(s.source, SubscriptionSource::Timer { .. }))
        .collect();
    if timers.is_empty() {
        return Ok(Vec::new());
    }
    let now = crate::now_ms();
    let since = match state.task_watch.last_tick_ms {
        0 => state.updated_at_ms,
        t => t,
    };
    if now < since.saturating_add(every_ms) {
        return Ok(Vec::new());
    }
    let tasks = state.watched_tasks.len();
    let children = state
        .waiting_for
        .as_ref()
        .filter(|w| w.kind == WaitingKind::Children)
        .map(|w| w.refs.len())
        .unwrap_or(0);
    let mut running = Vec::new();
    if tasks > 0 {
        running.push(format!("{tasks} task(s)"));
    }
    if children > 0 {
        running.push(format!("{children} sub session(s)"));
    }
    let summary = format!(
        "{} still running, {} since your last reply. New output since you last saw it, if any, is in <<background_context>>.",
        running.join(" and "),
        fmt_secs(now.saturating_sub(since))
    );
    let mut inputs = Vec::new();
    let mut semi = Vec::new();
    for sub in timers {
        let SubscriptionSource::Timer { name } = &sub.source else {
            continue;
        };
        let ev = AgentEvent {
            subscription_id: Some(sub.id.clone()),
            source: EventSource::new("timer", name.clone()),
            event: TICK_EVENT.into(),
            seq: Some(now),
            summary: summary.clone(),
            data_ref: None,
            terminal: false,
            call_id: None,
        };
        let key = format!("tick:{name}:{now}");
        match sub.mode {
            SubscriptionMode::Active => inputs.push(FetchedInput {
                src: INTERNAL_TIMER_SRC.to_string(),
                index: 0,
                kind: INPUT_TYPE_EVENT.to_string(),
                key,
                from: "runner".to_string(),
                at_ms: now,
                input: Ok(SessionInput::Event(ev)),
            }),
            SubscriptionMode::Semi => semi.push((sub.id.clone(), ev, key)),
        }
    }
    if !semi.is_empty() {
        let mut s = sh.session.lock().await;
        for (sub_id, ev, key) in semi {
            merge_pending_event(&mut s.state.pending_events, Some(&sub_id), &ev, &key, None, now);
        }
        if inputs.is_empty() {
            // Taken in by nothing: the next one is counted from now.
            s.state.task_watch.last_tick_ms = now;
        }
        commit_and_report(sh, &mut s).await?;
    }
    Ok(inputs)
}

/// The correction waiting to be taken in, if any.
pub(super) async fn poll_nudge(sh: &Arc<Shared>) -> Vec<FetchedInput> {
    let state = sh.session.lock().await.state.clone();
    if !state.nudge.pending {
        return Vec::new();
    }
    let now = crate::now_ms();
    vec![FetchedInput {
        src: INTERNAL_NUDGE_SRC.to_string(),
        index: 0,
        kind: INPUT_TYPE_EVENT.to_string(),
        key: format!("nudge:{}:{}", state.current_turn(), state.nudge.count),
        from: "runner".to_string(),
        at_ms: now,
        input: Ok(SessionInput::Event(AgentEvent {
            subscription_id: None,
            source: EventSource::new("system", RUNNER_SOURCE_ID),
            event: NUDGE_EVENT.into(),
            seq: None,
            summary: NUDGE_TEXT.into(),
            data_ref: None,
            terminal: false,
            call_id: None,
        })),
    }]
}
