use async_trait::async_trait;

use super::types::{SessionRuntimeContext, TokenUsage};

#[async_trait]
pub trait WorklogSink: Send + Sync {
    async fn emit(&self, event: AgentWorkEvent);
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentWorkEvent {
    LLMStarted {
        trace: SessionRuntimeContext,
        model: String,
    },
    LLMFinished {
        trace: SessionRuntimeContext,
        usage: TokenUsage,
        ok: bool,
    },
    ToolCallPlanned {
        trace: SessionRuntimeContext,
        tool: String,
        call_id: String,
    },
    ToolCallFinished {
        trace: SessionRuntimeContext,
        tool: String,
        call_id: String,
        ok: bool,
        duration_ms: u64,
    },
    ParseWarning {
        trace: SessionRuntimeContext,
        msg: String,
    },
}
