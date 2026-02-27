use buckyos_api::AiToolCall;

use super::types::{Observation, TraceCtx};

#[derive(Clone, Debug, PartialEq)]
pub struct ToolContext {
    pub tool_calls: Vec<AiToolCall>,
    pub observations: Vec<Observation>,
}

pub(crate) fn trace_to_tool_call_context(trace: &TraceCtx) -> TraceCtx {
    trace.clone()
}
