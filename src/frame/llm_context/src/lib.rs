//! LLMContext — narrow-waist primitive for bounded LLM execution.
//!
//! See `doc/opendan/LLM Context 设计.md` for the full design. This crate
//! implements L2 (the "process context" layer): one LLMContext object owns
//! a request, an evolving message history, and the dispatch glue between
//! the LLM provider and the tool manager. Schedulers (Agent / Workflow /
//! OneShot) sit above and below this waist but do not appear here.

pub mod context_loop;
pub mod deps;
pub mod error;
pub mod llm_compress;
pub mod local_llm_context;
pub mod observation;
pub mod outcome;
pub mod request;
pub mod state;

pub use context_loop::LLMContext;
pub use deps::{
    AllowAllPolicy, ByteHeuristicTokenizer, LLMContextDeps, LlmClient,
    LlmInferenceRequest, NoopWorklogSink, PolicyEngine, ToolManager,
    ToolSpecLite, Tokenizer, TurnHook, WorkEvent, WorklogSink,
};
pub use error::LLMComputeError;
pub use observation::{Observation, PendingToolCall, ToolExecRecord};
pub use outcome::{
    BudgetKind, ContextLimitKind, ContextOutput, ContextRunTrace,
    LLMContextOutcome, ResumeFill,
};
pub use request::{
    BudgetAction, BudgetSpec, ContextOwnerRef, ContextThreshold, ErrorClass,
    ErrorMode, ErrorPolicy, HumanPolicy, LLMContextRequest, ModelPolicy,
    OutputSpec, ToolMode, ToolPolicy,
};
pub use local_llm_context::{
    Compressor, FileSnapshotStore, LocalLLMContext, OneShotRequest, RunMetaState,
    RunStatus, SnapshotStore, SuspendKind, DEFAULT_CONTEXT_YIELD_RATIO,
    DEFAULT_ERROR_MODE, DEFAULT_MAX_CONSECUTIVE_ERRORS,
};
pub use state::{LLMContextSnapshot, LLMContextState};

#[cfg(test)]
mod tests;
