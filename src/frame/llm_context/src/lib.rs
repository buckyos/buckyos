//! LLMContext — narrow-waist primitive for bounded LLM execution.
//!
//! See `doc/opendan/LLM Context 设计.md` for the full design. This crate
//! implements L2 (the "process context" layer): one LLMContext object owns
//! a request, an evolving message history, and the dispatch glue between
//! the LLM provider and the tool manager. Schedulers (Agent / Workflow /
//! OneShot) sit above and below this waist but do not appear here.

pub mod behavior_loop;
pub mod context_loop;
pub mod context_window;
pub mod deps;
pub mod error;
pub mod interrupt;
pub mod msg_parser;
pub mod observation;
pub mod outcome;
pub mod prompt_budget;
pub mod prompt_compose;
pub mod prompt_engine;
pub mod request;
pub mod snapshot_overrides;
pub mod state;
pub mod step_record;
pub mod suspension;
pub mod tasks;
pub mod xml_behavior;
mod xml_util;

pub use behavior_loop::{
    is_terminal_next_behavior, HistorySummaryRecord, LLMBehaviorResult, LLMResultParser,
    StepCompressionLevel, StepMeta, StepRecord, StepRenderer, StepResultHook, StepResultHookOutput,
    NEXT_BEHAVIOR_END,
};
pub use context_loop::LLMContext;
pub use deps::{
    AllowAllPolicy, ByteHeuristicTokenizer, CancelCause, CheckpointHook, InferenceHook, Injection,
    InjectionPosition, LLMContextDeps, LlmClient, LlmInferenceRequest, NoopWorklogSink,
    PolicyEngine, Tokenizer, ToolCallCtx, ToolDispatchError, ToolManager, ToolSpecLite, WorkEvent,
    WorklogSink, MAX_INJECTIONS_PER_BOUNDARY,
};
pub use error::{CheckpointStage, ErrorSource, LLMComputeError, ProviderFailure};
pub use interrupt::{InferenceAbortToken, InferenceAbortTrace, LLMContextInterruptHandle};
pub use msg_parser::{
    ai_message_to_msg_object_with_base_validated_async, msg_object_to_ai_message_structured,
    parse_msg_object_structured, AttachmentTag, AttachmentValidation, AttachmentValidator,
    LocalFileResolver, MsgEgressOptions, MsgParseOutput, MsgParserError,
    PermissiveAttachmentValidator, SystemControlCommand, PROVIDER_MSG_METADATA,
};
pub use observation::{
    Observation, PendingToolCall, ToolExecRecord, ToolExecStatus, ToolResultStatusView,
    ToolResultView,
};
pub use outcome::{
    BudgetKind, ContextLimitKind, ContextOutput, ContextRunTrace, LLMContextOutcome, ResumeFill,
};
pub use prompt_budget::{BudgetedSection, FitOutcome, FittedSection, PromptBudgeter, TruncFrom};
pub use prompt_compose::{
    compose, CompositionError, CompositionOutcome, CompositionRequest, SectionSpec,
};
pub use prompt_engine::{
    EngineConfig, NullValueLoader, PromptRenderEngine, RenderError, RenderResult, RenderStats,
    RenderVars, ValueLoader,
};
pub use request::{
    BudgetAction, BudgetSpec, ContextOwnerRef, ContextThreshold, ErrorClass, ErrorPolicy,
    HumanPolicy, LLMContextRequest, ModelPolicy, OutputSpec, ToolMode, ToolPolicy,
};
pub use snapshot_overrides::{
    apply_overrides_to_snapshot, build_fresh, rebuild_with_inherit, RequestOverrides,
};
pub use state::{
    ActionStep, LLMContextSnapshot, LLMContextState, Suspension, ToolBatch, SNAPSHOT_FORMAT_VERSION,
};
pub use suspension::{is_thinking, strip_thinking};
pub use tasks::{
    next_step_hint, render_background_env, task_state_observation, CancelUnsupported,
    RunningTaskResolver, TaskBrief, TaskResult, TaskState, DEFAULT_TASK_WAIT_MS,
    MAX_IN_TOOL_WAIT_MS,
};
pub use context_window::ContextLimits;
pub use step_record::XmlStepRenderer;
pub use xml_behavior::{XmlBehaviorParser, XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT};

/// Current time in ms since the epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
