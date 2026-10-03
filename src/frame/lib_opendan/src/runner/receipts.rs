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

pub fn position_of(p: InjectionPosition) -> MessagePos {
    match p {
        InjectionPosition::RequestInput(i) => MessagePos::RequestInput { index: i as u64 },
        InjectionPosition::Accumulated(i) => MessagePos::Accumulated { index: i as u64 },
        InjectionPosition::Step(i) => MessagePos::Step { index: i as u64 },
        InjectionPosition::None => MessagePos::None,
    }
}

/// Where the waist will place an injection into `snapshot` (deterministic:
/// used by the checkpoint hook, which must describe the position in the
/// receipt it hands over together with the message).
pub fn predict_position(snapshot: &LLMContextSnapshot, behavior: bool) -> MessagePos {
    if !behavior {
        return MessagePos::Accumulated {
            index: snapshot.state.accumulated.len() as u64,
        };
    }
    let current = snapshot.request.behavior_name.as_str();
    match snapshot.state.last_step.as_ref().or(snapshot
        .state
        .steps
        .last()
        .filter(|s| s.meta.behavior_name == current))
    {
        Some(step) => MessagePos::Step {
            index: step.meta.step_index as u64,
        },
        None => MessagePos::RequestInput {
            index: snapshot.request.input.len() as u64,
        },
    }
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
    let change_ids: Vec<String> = r.changes.iter().map(|c| c.id.clone()).collect();
    match live.turns.last_mut() {
        Some(last) if last.turn == r.turn && !r.opens_turn => {
            last.inputs.extend(input_ids);
            last.changes.extend(change_ids);
        }
        _ => live.turns.push(TurnInputs {
            turn: r.turn,
            inputs: input_ids,
            changes: change_ids,
            hook: r.hook.clone(),
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
            hook: r.hook.clone(),
            inputs: logical,
            at_ms: r.at_ms,
        });
        state.turn_seq = state.turn_seq.max(r.turn);
    } else if let Some(t) = state.open_turn.as_mut() {
        t.inputs.extend(logical);
    }
    for i in r.inputs.iter().chain(r.consumed_only.iter()) {
        state.source_mut(&i.src).mark(i.index);
        state.recent_keys.push(&i.key);
    }
    for c in &r.changes {
        if !c.cursor.is_null() {
            state
                .subscription_cursors
                .insert(c.subscription.clone(), c.cursor.clone());
        }
    }
    if r.bootstrap {
        state.bootstrap_done = true;
    }
    if r.hook.as_deref() != Some(OBSERVATION_HOOK) {
        state.run_state = RunState::Running;
        state.waiting_for = None;
    }
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
