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
pub mod xml_behavior;

pub use behavior_loop::{
    HistorySummaryRecord, LLMBehaviorResult, LLMResultParser, StepCompressionLevel, StepMeta,
    StepRecord, StepRenderer, StepResultHook, StepResultHookOutput,
};
pub use context_loop::LLMContext;
pub use deps::{
    AllowAllPolicy, ByteHeuristicTokenizer, LLMContextDeps, LlmClient, LlmInferenceRequest,
    NoopWorklogSink, PolicyEngine, Tokenizer, ToolManager, ToolSpecLite, TurnHook, WorkEvent,
    WorklogSink,
};
pub use error::LLMComputeError;
pub use interrupt::{InferenceAbortToken, InferenceAbortTrace, LLMContextInterruptHandle};
pub use msg_parser::{
    ai_message_to_msg_object, ai_message_to_msg_object_with_base,
    ai_message_to_msg_object_with_base_validated,
    ai_message_to_msg_object_with_base_validated_async,
    ai_message_to_msg_object_with_base_validated_with_options, msg_object_control_command,
    msg_object_to_ai_message, msg_object_to_ai_message_text_attachments,
    msg_object_to_ai_message_with_role, msg_object_to_ai_message_with_role_text_attachments,
    parse_msg_object, parse_msg_object_text_attachments, AttachmentTag, AttachmentValidation,
    AttachmentValidator, LocalFileResolver, MsgEgressOptions, MsgParseOutput, MsgParserError,
    PermissiveAttachmentValidator, SystemControlCommand,
};
pub use observation::{
    Observation, PendingToolCall, ToolExecRecord, ToolResultStatusView, ToolResultView,
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
pub use state::{LLMContextSnapshot, LLMContextState};
pub use step_record::XmlStepRenderer;
pub use xml_behavior::{XmlBehaviorParser, XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT};

#[cfg(test)]
mod tests;
