//! LLMContext — narrow-waist primitive for bounded LLM execution.
//!
//! See `doc/opendan/LLM Context 设计.md` for the full design. This crate
//! implements L2 (the "process context" layer): one LLMContext object owns
//! a request, an evolving message history, and the dispatch glue between
//! the LLM provider and the tool manager. Schedulers (Agent / Workflow /
//! OneShot) sit above and below this waist but do not appear here.

pub mod behavior_loop;
pub mod context_loop;
pub mod deps;
pub mod error;
pub mod observation;
pub mod outcome;
pub mod prompt_budget;
pub mod prompt_compose;
pub mod prompt_engine;
pub mod request;
pub mod snapshot_overrides;
pub mod state;
pub mod step_record;
pub mod xml_behavior;

pub use behavior_loop::{
    CompressBudget, CompressError, HistoryCompressor, LLMBehaviorResult,
    LLMResultParser, StepRecord, StepRenderer,
};
pub use context_loop::LLMContext;
pub use deps::{
    AllowAllPolicy, ByteHeuristicTokenizer, LLMContextDeps, LlmClient,
    LlmInferenceRequest, NoopWorklogSink, PolicyEngine, ToolManager,
    ToolSpecLite, Tokenizer, TurnHook, WorkEvent, WorklogSink,
};
pub use error::LLMComputeError;
pub use observation::{Observation, PendingToolCall, ToolExecRecord};
pub use prompt_budget::{
    BudgetedSection, FitOutcome, FittedSection, PromptBudgeter, TruncFrom,
};
pub use prompt_compose::{
    compose, CompositionError, CompositionOutcome, CompositionRequest, SectionSpec,
};
pub use prompt_engine::{
    EngineConfig, NullValueLoader, PromptRenderEngine, RenderError, RenderResult, RenderStats,
    RenderVars, ValueLoader,
};
pub use outcome::{
    BudgetKind, ContextLimitKind, ContextOutput, ContextRunTrace,
    LLMContextOutcome, ResumeFill,
};
pub use request::{
    BudgetAction, BudgetSpec, ContextOwnerRef, ContextThreshold, ErrorClass,
    ErrorMode, ErrorPolicy, HumanPolicy, LLMContextRequest, ModelPolicy,
    OutputSpec, ToolMode, ToolPolicy,
};
pub use snapshot_overrides::{
    apply_overrides_to_snapshot, build_fresh, rebuild_with_inherit, RequestOverrides,
};
pub use state::{LLMContextSnapshot, LLMContextState};
pub use step_record::XmlStepRenderer;
pub use xml_behavior::XmlBehaviorParser;

#[cfg(test)]
mod tests;
