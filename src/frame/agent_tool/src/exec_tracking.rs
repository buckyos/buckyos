//! In-flight tool actions shared by xllm and libOpenDAN (Agent Session SDK
//! plan §8.7 X6, long-tool TODO §3.2).
//!
//! A run lock only protects the run directory. A runner that is killed
//! (`kill -9`) leaves the actions it had dispatched without a persisted
//! result. The next executor does **not** verify or stop processes: it
//! turns every such action into an explicit "interrupted, result unknown"
//! observation — for a `shell` command with the facts its runtime can read
//! (exit code and output tail from the execution directory, or "may still
//! be running" with where to look) — and lets the model decide. Nothing is
//! replayed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool action that was dispatched but whose result is not persisted yet.
/// Written (fsync) before the tool starts; cleared only after a checkpoint
/// that contains its result (or an explicit `Unresolved`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InflightAction {
    pub call_id: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    /// `read_only | idempotent | side_effect | unknown`.
    #[serde(default = "unknown_effect")]
    pub effect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Behavior step index the action belongs to (behavior loop only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_index: Option<u32>,
    pub started_at_ms: u64,
}

fn unknown_effect() -> String {
    "unknown".to_string()
}

/// Marks a run assembled by a host (e.g. libOpenDAN) rather than by xllm's
/// own `.llm_context` preparation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostRunInfo {
    /// `libopendan` …
    pub assembled_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// `native | tmux | remote_ssh`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// Environment facts xllm must verify before continuing (PATH layers,
    /// required tools …).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub env_check: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

pub fn now_ms() -> u64 {
    crate::now_ms()
}

// -------------------------------------------------------------------------
// Unresolved in-flight actions
// -------------------------------------------------------------------------

use buckyos_api::{AiContent, AiMessage, AiRole, AiToolCall};
use llm_context::behavior_loop::{StepMeta, StepRecord};
use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;

/// Text shown to the model for an action (other than `shell`) whose effect
/// is unknown.
pub fn unresolved_reason(tool: &str) -> String {
    format!(
        "the `{tool}` call was dispatched before the previous executor stopped; its effect is unknown. Do not assume it succeeded or failed — check the actual state before repeating it."
    )
}

fn args_map(v: &Value) -> HashMap<String, Value> {
    match v {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => HashMap::new(),
    }
}

fn fc_has_result(snapshot: &LLMContextSnapshot, call_id: &str) -> bool {
    snapshot.state.accumulated.iter().any(|m| {
        m.content
            .iter()
            .any(|c| matches!(c, AiContent::ToolResult { call_id: id, .. } if id == call_id))
    })
}

fn fc_has_call(snapshot: &LLMContextSnapshot, call_id: &str) -> bool {
    snapshot.state.accumulated.iter().any(|m| {
        m.content
            .iter()
            .any(|c| matches!(c, AiContent::ToolUse { call_id: id, .. } if id == call_id))
    })
}

/// Inject an explicit "result unknown" for every in-flight action that has
/// no persisted outcome in `snapshot`. `reasons` gives the text per call id
/// (see `AgentRuntime::describe_interrupted`); an action without one gets
/// [`unresolved_reason`]. Actions that already have a result are left alone.
/// Returns the call ids injected.
///
/// Function call mode: appends the assistant tool call (if missing) and a
/// tool result. Behavior mode: completes a partial step, or appends a
/// synthetic step carrying the action and an `Unresolved` observation.
pub fn materialize_unresolved(
    snapshot: &mut LLMContextSnapshot,
    inflight: &[InflightAction],
    behavior: bool,
    reasons: &HashMap<String, String>,
) -> Vec<String> {
    let mut injected = Vec::new();
    for a in inflight {
        let reason = reasons
            .get(&a.call_id)
            .cloned()
            .unwrap_or_else(|| unresolved_reason(&a.tool));
        if !behavior {
            if fc_has_result(snapshot, &a.call_id) {
                continue;
            }
            if !fc_has_call(snapshot, &a.call_id) {
                snapshot.state.accumulated.push(AiMessage::new(
                    AiRole::Assistant,
                    vec![AiContent::tool_use(
                        a.call_id.clone(),
                        a.tool.clone(),
                        args_map(&a.args),
                    )],
                ));
            }
            snapshot.state.accumulated.push(AiMessage::new(
                AiRole::Tool,
                vec![AiContent::tool_result_text(
                    a.call_id.clone(),
                    format!("[unresolved: result unknown] {reason}"),
                    true,
                )],
            ));
            injected.push(a.call_id.clone());
            continue;
        }
        // Behavior mode.
        let state = &mut snapshot.state;
        let mut handled = false;
        for step in state.steps.iter_mut().chain(state.last_step.iter_mut()) {
            if let Some(pos) = step.actions.iter().position(|x| x.call_id == a.call_id) {
                if step.action_results.len() > pos {
                    handled = true; // already has an outcome
                } else {
                    while step.action_results.len() < pos {
                        let missing = step.actions[step.action_results.len()].call_id.clone();
                        step.action_results.push(Observation::Unresolved {
                            call_id: missing,
                            reason: "not executed".into(),
                            effect_unknown: false,
                        });
                    }
                    step.action_results.push(Observation::Unresolved {
                        call_id: a.call_id.clone(),
                        reason: reason.clone(),
                        effect_unknown: true,
                    });
                    injected.push(a.call_id.clone());
                    handled = true;
                }
                break;
            }
        }
        if handled {
            continue;
        }
        let step_index = state.next_step_index;
        state.next_step_index = state.next_step_index.saturating_add(1);
        if let Ok(n) = a.call_id.parse::<u32>() {
            state.next_action_id = state.next_action_id.max(n);
        }
        let step = StepRecord {
            meta: StepMeta {
                behavior_name: snapshot.request.behavior_name.clone(),
                step_index,
                started_at_ms: a.started_at_ms,
                ended_at_ms: Some(a.started_at_ms),
                compression_level: Default::default(),
            },
            assistant_text: format!(
                "(recovered) dispatched `{}` before the previous runner stopped",
                a.tool
            ),
            actions: vec![AiToolCall {
                name: a.tool.clone(),
                args: args_map(&a.args),
                call_id: a.call_id.clone(),
            }],
            action_results: vec![Observation::Unresolved {
                call_id: a.call_id.clone(),
                reason,
                effect_unknown: true,
            }],
            ..Default::default()
        };
        if let Some(prev) = state.last_step.replace(step) {
            state.steps.push(prev);
        }
        injected.push(a.call_id.clone());
    }
    injected
}

/// Call ids that have a persisted outcome in `snapshot`.
pub fn persisted_outcome_ids(snapshot: &LLMContextSnapshot) -> Vec<String> {
    let mut out = Vec::new();
    for m in &snapshot.state.accumulated {
        for c in &m.content {
            if let AiContent::ToolResult { call_id, .. } = c {
                out.push(call_id.clone());
            }
        }
    }
    let st = &snapshot.state;
    for step in st.steps.iter().chain(st.last_step.iter()) {
        for (i, a) in step.actions.iter().enumerate() {
            if i < step.action_results.len() {
                out.push(a.call_id.clone());
            }
        }
    }
    out
}
