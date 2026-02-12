use async_trait::async_trait;

use super::types::{TokenUsage, TraceCtx};

#[async_trait]
pub trait WorklogSink: Send + Sync {
    async fn emit(&self, event: Event);
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    LLMStarted {
        trace: TraceCtx,
        model: String,
    },
    LLMFinished {
        trace: TraceCtx,
        usage: TokenUsage,
        ok: bool,
    },
    ToolCallPlanned {
        trace: TraceCtx,
        tool: String,
        call_id: String,
    },
    ToolCallFinished {
        trace: TraceCtx,
        tool: String,
        call_id: String,
        ok: bool,
        duration_ms: u64,
    },
    ParseWarning {
        trace: TraceCtx,
        msg: String,
    },
}
