//! Turning a run snapshot into worklog entries (§4.4 `flush_run`).
//!
//! What was already written is tracked by [`FlushMarks`]
//! (`live_run.flushed_step` / `flushed_input_seq`), so suspending a process
//! (flush so far) and ending the run later never writes anything twice, and a
//! fork child never re-writes the steps it inherited.
//!
//! - function call runs: the messages of `accumulated` after `request.input`,
//!   in order (append-only, counted by position within the current history
//!   epoch: a mid-run rewrite first flushes everything, then replaces the
//!   prefix and starts a new epoch, see `HostMeta.history_epoch`);
//! - behavior runs: steps (identified by `step_index`) and injected messages
//!   (identified by their receipt `input_seq`), ordered by where each message
//!   was injected: before step `after_step` (`request_input`) or right after
//!   the step it was attached to (`step`).

use std::collections::BTreeMap;

use buckyos_api::{AiContent, AiRole};
use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;
use serde_json::Value;

use crate::protocol::*;

use super::receipts::snapshot_host_meta;
use super::tools::classify_effect;

/// What of a run is already in the worklog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlushMarks {
    /// fc: messages after the prefix; behavior: steps with index below.
    pub step: u64,
    /// behavior: receipts with `input_seq ≤` this.
    pub input_seq: u64,
    /// fc: the history epoch `step` counts in.
    pub epoch: u64,
}

impl FlushMarks {
    pub fn of(live: &LiveRun) -> Self {
        Self {
            step: live.flushed_step,
            input_seq: live.flushed_input_seq,
            epoch: live.flushed_epoch,
        }
    }

    pub fn apply(&self, live: &mut LiveRun) {
        live.flushed_step = self.step;
        live.flushed_input_seq = self.input_seq;
        live.flushed_epoch = self.epoch;
    }
}

/// Canonical (sorted keys) JSON of tool args.
pub fn canonical_args(args: &std::collections::HashMap<String, Value>) -> Value {
    let sorted: BTreeMap<&String, &Value> = args.iter().collect();
    serde_json::to_value(sorted).unwrap_or(Value::Null)
}

pub fn observation_view(o: &Observation) -> (String, String) {
    match o {
        Observation::Success { content, .. } => (
            "ok".into(),
            match content {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        ),
        Observation::Error { message, .. } => ("error".into(), message.clone()),
        Observation::Pending { .. } => ("pending".into(), String::new()),
        Observation::Cancelled { reason, .. } => ("cancelled".into(), reason.clone()),
        Observation::Unresolved {
            reason,
            effect_unknown,
            ..
        } => (
            "unresolved".into(),
            if *effect_unknown {
                format!("effect unknown: {reason}")
            } else {
                format!("not executed: {reason}")
            },
        ),
    }
}

fn user_entries(
    run_id: &str,
    r: &InputReceipt,
    round: &mut u64,
    fallback: String,
) -> Vec<WorklogBody> {
    let mut out = Vec::new();
    if r.opens_round {
        *round = r.round;
        out.push(WorklogBody::RoundStarted {
            run_id: run_id.to_string(),
            round: r.round,
            inputs: r.inputs.clone(),
            changes: r.changes.iter().map(|c| c.id.clone()).collect(),
            hook: r.hook.clone(),
            at_ms: r.at_ms,
        });
    }
    out.push(WorklogBody::UserMessage {
        run_id: run_id.to_string(),
        round: *round,
        content: if r.content.is_empty() {
            fallback
        } else {
            r.content.clone()
        },
    });
    out
}

fn step_entries(
    run_id: &str,
    round: u64,
    step: &llm_context::behavior_loop::StepRecord,
) -> Vec<WorklogBody> {
    let actions = step
        .actions
        .iter()
        .map(|c| ActionEntry {
            effect: classify_effect(&c.name).to_string(),
            args: canonical_args(&c.args),
            call_id: c.call_id.clone(),
            tool: c.name.clone(),
        })
        .collect();
    let mut out = vec![WorklogBody::Step {
        run_id: run_id.to_string(),
        round,
        behavior: if step.meta.behavior_name.is_empty() {
            None
        } else {
            Some(step.meta.behavior_name.clone())
        },
        assistant: step.assistant_text.clone(),
        actions,
    }];
    for (idx, o) in step.action_results.iter().enumerate() {
        let (status, result) = observation_view(o);
        let call_id = step
            .actions
            .get(idx)
            .map(|a| a.call_id.clone())
            .unwrap_or_else(|| o.call_id().to_string());
        out.push(WorklogBody::ActionResult {
            run_id: run_id.to_string(),
            round,
            call_id,
            status,
            result,
        });
    }
    out
}

/// Entries not written yet and the new marks.
pub fn run_history_entries(
    run_id: &str,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
    marks: FlushMarks,
    default_round: u64,
) -> (Vec<WorklogBody>, FlushMarks) {
    let meta = snapshot_host_meta(snapshot);
    let mut out = Vec::new();
    let mut round = meta
        .input_receipts
        .iter()
        .map(|r| r.round)
        .min()
        .unwrap_or(default_round);
    let max_receipt = meta
        .input_receipts
        .iter()
        .map(|r| r.input_seq)
        .max()
        .unwrap_or(0);

    if !behavior {
        // Positions only mean something within the current epoch.
        let receipt_at = |i: usize| {
            meta.input_receipts.iter().find(|r| {
                r.input_seq > meta.epoch_input_seq
                    && r.message_pos == MessagePos::Accumulated { index: i as u64 }
            })
        };
        let flushed = if marks.epoch == meta.history_epoch {
            marks.step
        } else {
            0
        };
        if meta.history_epoch > 0 {
            round = meta.epoch_round;
        }
        let base = snapshot.request.input.len();
        let mut unit = 0u64;
        for (i, m) in snapshot.state.accumulated.iter().enumerate().skip(base) {
            let this = unit;
            unit += 1;
            if this < flushed {
                if let Some(r) = receipt_at(i) {
                    if r.opens_round {
                        round = r.round;
                    }
                }
                continue;
            }
            match m.role {
                AiRole::User => match receipt_at(i) {
                    Some(r) => out.extend(user_entries(run_id, r, &mut round, m.text_content())),
                    None => out.push(WorklogBody::UserMessage {
                        run_id: run_id.to_string(),
                        round,
                        content: m.text_content(),
                    }),
                },
                AiRole::Assistant => {
                    let actions = m
                        .tool_calls()
                        .into_iter()
                        .map(|c| ActionEntry {
                            effect: classify_effect(&c.name).to_string(),
                            args: canonical_args(&c.args),
                            call_id: c.call_id,
                            tool: c.name,
                        })
                        .collect();
                    out.push(WorklogBody::Step {
                        run_id: run_id.to_string(),
                        round,
                        behavior: None,
                        assistant: m.text_content(),
                        actions,
                    });
                }
                AiRole::Tool => {
                    for c in &m.content {
                        if let AiContent::ToolResult {
                            call_id,
                            content,
                            is_error,
                        } = c
                        {
                            let text = content
                                .iter()
                                .filter_map(|x| x.text_str().map(str::to_string))
                                .collect::<Vec<_>>()
                                .join("\n");
                            let status = if text.starts_with("[unresolved]") {
                                "unresolved"
                            } else if *is_error {
                                "error"
                            } else {
                                "ok"
                            };
                            out.push(WorklogBody::ActionResult {
                                run_id: run_id.to_string(),
                                round,
                                call_id: call_id.clone(),
                                status: status.to_string(),
                                result: text,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        return (
            out,
            FlushMarks {
                step: unit.max(flushed),
                input_seq: max_receipt.max(marks.input_seq),
                epoch: meta.history_epoch,
            },
        );
    }

    // Behavior runs: order steps and injected messages, then emit what is new.
    enum Ev<'a> {
        Msg(&'a InputReceipt),
        Step(&'a llm_context::behavior_loop::StepRecord),
    }
    let mut evs: Vec<((u64, u8, u64), Ev)> = Vec::new();
    for r in &meta.input_receipts {
        let key = match r.message_pos {
            MessagePos::Step { index } => (index, 2, r.input_seq),
            _ => (r.after_step as u64, 0, r.input_seq),
        };
        evs.push((key, Ev::Msg(r)));
    }
    let mut max_step = marks.step;
    for s in snapshot
        .state
        .steps
        .iter()
        .chain(snapshot.state.last_step.iter())
    {
        if s.meta.step_index < meta.inherited_below {
            continue; // inherited from the parent process (fork child)
        }
        max_step = max_step.max(s.meta.step_index as u64 + 1);
        evs.push(((s.meta.step_index as u64, 1, 0), Ev::Step(s)));
    }
    evs.sort_by_key(|(k, _)| *k);
    for (_, ev) in evs {
        match ev {
            Ev::Msg(r) => {
                if r.input_seq <= marks.input_seq {
                    if r.opens_round {
                        round = r.round;
                    }
                    continue;
                }
                out.extend(user_entries(run_id, r, &mut round, String::new()));
            }
            Ev::Step(step) => {
                if (step.meta.step_index as u64) < marks.step {
                    continue;
                }
                out.extend(step_entries(run_id, round, step));
            }
        }
    }
    (
        out,
        FlushMarks {
            step: max_step,
            input_seq: max_receipt.max(marks.input_seq),
            epoch: meta.history_epoch,
        },
    )
}
