//! What a run dispatched, read back after the executor stopped (long-tool
//! TODO §3.2, freeze TODO §3.2.1 / F12).
//!
//! The waist checkpoints the outer snapshot before every tool call and
//! dispatches the calls of a batch / step one after the other, so a snapshot
//! tells exactly which call may have been running when an executor was
//! killed: the first call without a result ([`in_flight_call`]). On resume
//! the waist dispatches it again with the same `call_id`; the runtime's tool
//! manager first looks for the work it became (a `shell` command's execution
//! directory, a task keyed by the call id), runs it again only when the tool
//! is idempotent, and otherwise answers "result unknown" with the facts the
//! runtime can read. No process is verified or stopped.

use buckyos_api::AiToolCall;
use llm_context::state::LLMContextSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Marks a run assembled by a host (e.g. libOpenDAN) rather than by xllm's
/// own `.llm_context` preparation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostRunInfo {
    /// `libopendan` …
    pub assembled_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// `native | remote_ssh | remote_node`.
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

/// Text shown to the model for a call (other than `shell`) a previous
/// executor may have started.
pub fn unresolved_reason(tool: &str) -> String {
    format!(
        "the `{tool}` call was dispatched before the previous executor stopped; its effect is unknown. Do not assume it succeeded or failed — check the actual state before repeating it."
    )
}

/// The call a resumed `snapshot` dispatches first and that may already have
/// been started by the executor that wrote it: the next call of the tool
/// batch in progress (function call loop, or the inner loop of a behavior
/// step), or else the next action of the behavior step in progress.
pub fn in_flight_call(snapshot: &LLMContextSnapshot) -> Option<AiToolCall> {
    let state = &snapshot.state;
    if state.suspended.is_some() {
        return None;
    }
    if let Some(call) = state.tool_batch.as_ref().and_then(|b| b.remaining.first()) {
        return Some(call.clone());
    }
    let step = &state.action_step.as_ref()?.step;
    if step
        .action_results
        .last()
        .is_some_and(|o| !matches!(o, llm_context::observation::Observation::Success { .. }))
    {
        return None;
    }
    step.actions.get(step.action_results.len()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_context::behavior_loop::StepRecord;
    use llm_context::observation::Observation;
    use llm_context::state::{ActionStep, LLMContextState, ToolBatch};
    use serde_json::json;
    use std::collections::HashMap;

    fn snapshot() -> LLMContextSnapshot {
        let request = serde_json::from_value(json!({
            "owner": {"kind":"one_shot", "id":"test"}, "input":[]
        }))
        .unwrap();
        LLMContextSnapshot {
            state: LLMContextState::from_request(&request, 1),
            request,
        }
    }

    fn call(id: &str) -> AiToolCall {
        AiToolCall {
            call_id: id.into(),
            name: "shell".into(),
            args: HashMap::new(),
        }
    }

    #[test]
    fn the_next_call_of_a_batch_is_in_flight() {
        let mut snapshot = snapshot();
        assert!(in_flight_call(&snapshot).is_none());
        snapshot.state.tool_batch = Some(ToolBatch {
            remaining: vec![call("started"), call("not-started")],
            batch_error: None,
        });
        assert_eq!(in_flight_call(&snapshot).unwrap().call_id, "started");
    }

    #[test]
    fn the_next_action_of_a_step_is_in_flight_until_one_failed() {
        let mut snapshot = snapshot();
        snapshot.state.action_step = Some(ActionStep {
            step: StepRecord {
                actions: vec![call("1"), call("2")],
                action_results: vec![Observation::Success {
                    call_id: "1".into(),
                    content: json!("ok"),
                    bytes: 2,
                    truncated: false,
                    host: None,
                }],
                ..Default::default()
            },
            response: Default::default(),
        });
        assert_eq!(in_flight_call(&snapshot).unwrap().call_id, "2");
        let step = &mut snapshot.state.action_step.as_mut().unwrap().step;
        step.action_results[0] = Observation::Error {
            call_id: "1".into(),
            message: "boom".into(),
            host: None,
        };
        assert!(in_flight_call(&snapshot).is_none(), "the step stops after an error");
    }
}
