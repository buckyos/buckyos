//! Input receipts (§8.3): every batch of inputs that enters the context is
//! described by a structured receipt persisted in the run snapshot (host
//! metadata, `LLMContextState.host["libopendan"]`) together with its message.
//! Recovery re-applies receipts newer than `live_run.applied_input_seq` to
//! state.json — metadata only, the message is already in the snapshot.

use llm_context::state::LLMContextSnapshot;
use llm_context::InjectionPosition;
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;

pub fn host_meta_of(host: Option<&Value>) -> HostMeta {
    host.and_then(|h| h.get(HOST_META_KEY))
        .and_then(|v| serde_json::from_value::<HostMeta>(v.clone()).ok())
        .unwrap_or_default()
}

pub fn snapshot_host_meta(s: &LLMContextSnapshot) -> HostMeta {
    host_meta_of(s.state.host.as_ref())
}

/// New host value with `meta` stored under the libopendan key (other hosts'
/// keys are preserved).
pub fn with_host_meta(host: Option<&Value>, meta: &HostMeta) -> Value {
    let mut v = match host {
        Some(Value::Object(m)) => Value::Object(m.clone()),
        _ => Value::Object(Default::default()),
    };
    v[HOST_META_KEY] = serde_json::to_value(meta).unwrap_or(Value::Null);
    v
}

/// Positions of `count` messages placed by one injection starting at `p`.
/// In a behavior step all of them are merged into that step's
/// `next_user_message`.
pub fn positions_of(p: InjectionPosition, count: usize) -> Vec<MessagePos> {
    (0..count as u64)
        .filter_map(|i| match p {
            InjectionPosition::RequestInput(at) => Some(MessagePos::RequestInput {
                index: at as u64 + i,
            }),
            InjectionPosition::Accumulated(at) => Some(MessagePos::Accumulated {
                index: at as u64 + i,
            }),
            InjectionPosition::Step(at) => Some(MessagePos::Step { index: at as u64 }),
            InjectionPosition::None => None,
        })
        .collect()
}

/// Task id of a synthesized task completion input (`task:<id>:terminal`).
pub fn task_of_internal_key(key: &str) -> Option<&str> {
    key.strip_prefix("task:")?.strip_suffix(":terminal")
}

pub fn internal_task_key(task_id: &str) -> String {
    format!("task:{task_id}:terminal")
}

/// Apply one receipt to the session state (idempotent: receipts at or below
/// `live_run.applied_input_seq` are ignored). A receipt that opens a Turn
/// makes it the open Turn; any other receipt must belong to the open Turn.
pub fn apply_receipt(state: &mut SessionState, r: &InputReceipt) -> Result<bool> {
    let live = state.live_run.get_or_insert_with(|| LiveRun {
        run_id: r.run_id.clone(),
        turns: Vec::new(),
        applied_input_seq: 0,
        flushed_message_count: 0,
        flushed_step_index: 0,
        flushed_input_seq: 0,
        flushed_epoch: 0,
        process_entry: None,
        handover_at_ms: 0,
    });
    if live.run_id != r.run_id {
        return Err(OpenDanError::blocked(
            format!(
                "receipt of run {} does not belong to the live run {}",
                r.run_id, live.run_id
            ),
            Some(&r.run_id),
        ));
    }
    if r.input_seq <= live.applied_input_seq {
        return Ok(false);
    }
    if r.input_seq != live.applied_input_seq + 1 {
        return Err(OpenDanError::blocked(
            format!(
                "receipt batch {} does not follow applied batch {}",
                r.input_seq, live.applied_input_seq
            ),
            Some(&r.run_id),
        ));
    }
    let joins = state.open_turn.as_ref().map(|t| t.index);
    if !r.opens_turn && joins != Some(r.turn) {
        return Err(OpenDanError::blocked(
            format!(
                "receipt batch {} joins turn {} but the open turn is {joins:?}",
                r.input_seq, r.turn
            ),
            Some(&r.run_id),
        ));
    }
    live.applied_input_seq = r.input_seq;
    let input_ids: Vec<String> = r.inputs.iter().map(|i| i.id()).collect();
    let event_keys: Vec<String> = r.events.iter().map(|e| e.key.clone()).collect();
    match live.turns.last_mut() {
        Some(last) if last.turn == r.turn && !r.opens_turn => {
            last.inputs.extend(input_ids);
            last.events.extend(event_keys);
        }
        _ => live.turns.push(TurnInputs {
            turn: r.turn,
            inputs: input_ids,
            events: event_keys,
            hook: Some(r.hook.clone()),
            input_seq: r.input_seq,
            at_ms: r.at_ms,
        }),
    }
    // The Turn's logical input: msg / event inputs of every batch.
    let logical: Vec<String> = r
        .inputs
        .iter()
        .filter(|i| i.kind == "msg" || i.kind == "event")
        .map(|i| i.id())
        .collect();
    if r.opens_turn {
        state.open_turn = Some(OpenTurn {
            index: r.turn,
            run_id: r.run_id.clone(),
            input_seq: r.input_seq,
            hook: Some(r.hook.clone()),
            inputs: logical,
            at_ms: r.at_ms,
        });
        state.turn_seq = state.turn_seq.max(r.turn);
    } else if let Some(t) = state.open_turn.as_mut() {
        t.inputs.extend(logical);
    }
    for i in &r.inputs {
        if i.src == INTERNAL_TASK_SRC {
            // The completion of a watched task entered the context: the
            // runner stops following it.
            if let Some(task) = task_of_internal_key(&i.key) {
                state.watched_tasks.retain(|t| t != task);
            }
        } else {
            state.source_mut(&i.src).mark(i.index);
        }
        state.recent_keys.push(&i.key);
    }
    // Exactly the semi-subscription versions this batch injected.
    clear_pending_events(&mut state.pending_events, &r.events);
    // The default reply path is restored from the receipt, never from the
    // rendered text or the bus.
    if let Some(reply) = &r.reply {
        state.reply = Some(reply.clone());
    }
    if r.bootstrap {
        state.bootstrap_done = true;
    }
    state.run_state = RunState::Running;
    state.waiting_for = None;
    if r.extra.contains_key("continuation") {
        // The hand-over turn (behavior switch / fork result) was delivered.
        state.internal_continuation = None;
        state.process_result = None;
    }
    Ok(true)
}

/// Recovery: receipts of the published snapshot newer than state.
pub fn receipts_after(meta: &HostMeta, applied: u64) -> Vec<InputReceipt> {
    let mut v: Vec<InputReceipt> = meta
        .input_receipts
        .iter()
        .filter(|r| r.input_seq > applied)
        .cloned()
        .collect();
    v.sort_by_key(|r| r.input_seq);
    v
}

/// Validate receipts of a snapshot: contiguous batch ids for one run, and
/// the snapshot must cover what state already consumed (never recover from
/// an older snapshot by rolling consumption back).
pub fn validate_receipts(meta: &HostMeta, run_id: &str, applied: u64) -> Result<()> {
    let mut seqs: Vec<u64> = Vec::new();
    for r in &meta.input_receipts {
        if r.run_id != run_id {
            return Err(OpenDanError::blocked(
                format!("snapshot carries a receipt of run {}", r.run_id),
                Some(run_id),
            ));
        }
        seqs.push(r.input_seq);
    }
    seqs.sort_unstable();
    for (i, s) in seqs.iter().enumerate() {
        if *s != i as u64 + 1 {
            return Err(OpenDanError::blocked(
                format!("receipt batches are not contiguous: {seqs:?}"),
                Some(run_id),
            ));
        }
    }
    let max = seqs.last().copied().unwrap_or(0);
    if applied > max {
        return Err(OpenDanError::blocked(
            format!(
                "state already consumed input batch {applied} but the published snapshot only has {max}"
            ),
            Some(run_id),
        ));
    }
    Ok(())
}
