//! LLMContext — narrow-waist primitive for bounded LLM execution.
//!
//! See `doc/llm_context/LLM Context 设计.md` for the full design. This crate
//! only runs the loop: inference, dispatching tool calls, pairing their
//! results back into the history, deciding where to stop and how to resume,
//! and when to checkpoint. Tasks, tool result envelopes, prompt templates and
//! message protocols belong to the runtime / host above it.

pub mod behavior_loop;
pub mod context_derive;
pub mod context_loop;
pub mod context_window;
pub mod deps;
pub mod error;
pub mod interrupt;
pub mod json_check;
pub mod observation;
pub mod outcome;
pub mod request;
pub mod state;
pub mod suspension;

pub use behavior_loop::{
    DecisionVerdict, HistorySummaryRecord, LLMBehaviorResult, LLMResultParser,
    StepCompressionLevel, StepDecision, StepMeta, StepRecord, StepRenderer,
};
pub use context_derive::{
    derive_child, fork_snapshot, DeriveError, DerivedContext, ForkOptions, ForkPoint,
    InheritBoundary, InheritHistory,
};
pub use context_loop::LLMContext;
pub use context_window::ContextLimits;
pub use deps::{
    AllowAllPolicy, ByteHeuristicTokenizer, CancelCause, CheckpointHook, Injection,
    InjectionPosition, LLMContextDeps, LlmClient, LlmInferenceRequest, NoopWorklogSink,
    OutputValidator, PolicyEngine, Tokenizer, ToolCallCtx, ToolDispatchError, ToolManager,
    ToolSpecLite, WorkEvent, WorklogSink, MAX_INJECTIONS_PER_BOUNDARY,
};
pub use error::{CheckpointStage, ErrorSource, LLMComputeError, ProviderFailure};
pub use interrupt::{InferenceAbortToken, InferenceAbortTrace, LLMContextInterruptHandle};
pub use observation::{Observation, ToolExecRecord, ToolExecStatus};
pub use outcome::{
    BudgetKind, ContextLimitKind, ContextOutput, ContextRunTrace, LLMContextOutcome, ResumeFill,
};
pub use request::{
    BudgetAction, BudgetSpec, ContextThreshold, ErrorClass, ErrorPolicy, LLMContextRequest,
    ModelPolicy, OutputSpec, ToolMode, ToolPolicy, DEFAULT_WRAP_UP_ROUNDS,
};
pub use state::{
    ActionStep, LLMContextSnapshot, LLMContextState, Suspension, ToolBatch, SNAPSHOT_FORMAT_VERSION,
};
pub use suspension::{is_thinking, strip_snapshot_thinking, strip_thinking};

/// Current time in ms since the epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
