use crate::agent_tool::{ToolCall, ToolCallContext};

use super::types::{Observation, TraceCtx};

#[derive(Clone, Debug, PartialEq)]
pub struct ToolContext {
    pub tool_calls: Vec<ToolCall>,
    pub observations: Vec<Observation>,
}

pub(crate) fn trace_to_tool_call_context(trace: &TraceCtx) -> ToolCallContext {
    ToolCallContext {
        trace_id: trace.trace_id.clone(),
        agent_did: trace.agent_did.clone(),
        behavior: trace.behavior.clone(),
        step_idx: trace.step_idx,
        wakeup_id: trace.wakeup_id.clone(),
        current_session_id: None,
    }
}
