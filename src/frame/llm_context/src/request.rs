//! Inputs to one LLMContext invocation: request + policies.
//!
//! All fields here are the immutable "code segment" half of the process
//! analogy — the mutable "registers + stack" half lives in
//! [`crate::state::LLMContextState`].

use buckyos_api::AiMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelPolicy {
    /// Preferred model alias passed to the provider adapter.
    pub preferred: String,
    /// Optional fallback chain (in order). The provider adapter decides how
    /// to use this; waist only carries it.
    pub fallbacks: Vec<String>,
    /// Sampling temperature (0.0–2.0). `None` lets the provider pick.
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    /// Free-form provider-specific options, passed through opaquely.
    pub provider_options: Option<Value>,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            preferred: String::new(),
            fallbacks: Vec::new(),
            temperature: None,
            max_completion_tokens: None,
            provider_options: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolMode {
    /// No calls of this kind are allowed.
    None,
    /// Only entries in the matching `whitelist` field are dispatched.
    Whitelist,
    /// All known entries are dispatched (for tools: every spec exposed by
    /// `ToolManager`; for actions: every tag the parser recognizes).
    All,
}

/// Dispatch policy for one invocation surface.
///
/// Two surfaces are tracked separately, both as fields on [`ToolPolicy`]:
///
/// * **tools** — provider-native function calls exposed by `ToolManager`.
///   Controlled by `mode` + `whitelist`. Filters spec advertisement
///   (see [`crate::deps::resolve_tool_specs`]) and gates dispatch.
/// * **actions** — behavior-loop actions parsed out of the LLM response by
///   the host's `LLMResultParser`. Controlled by `action_mode` +
///   `action_whitelist`. Filters dispatch only — the recognized action set
///   is the parser's.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ToolPolicy {
    pub mode: ToolMode,
    pub whitelist: Vec<String>,
    pub action_mode: ToolMode,
    pub action_whitelist: Vec<String>,
    /// Tool iteration budget. One iteration is one completed native tool
    /// batch (all provider tool calls of one response, dispatched) or one
    /// behavior step that dispatches actions; native batches inside a
    /// behavior step and the step's actions share this budget. It does not
    /// count inferences (Rounds): the final tool-free answer, a pure
    /// report / decision step and output-parse corrections consume nothing.
    /// 0 disables tool dispatch (one inference only).
    pub max_tool_iterations: u32,
    /// Native tool calls one model response (one Round) may request; more
    /// calls reject the whole batch. Does not limit the actions of a
    /// behavior step.
    pub max_calls_per_round: u32,
    pub disable_capabilities: Vec<String>,
    /// Graceful finish (`LLMContextInterruptHandle::finish`): how long to
    /// wait for a running tool that cannot cancel its work before the
    /// finish falls back to an interrupt.
    pub finish_grace_ms: u64,
    /// Tools still offered once `max_tool_iterations` is used up: the model
    /// can only end (report) or hand over (ask the user). A batch of only
    /// these tools (the native calls of one response, or the actions of one
    /// behavior step) is dispatched without consuming an iteration, at most
    /// `wrap_up_rounds` times per run; a batch with any other tool still
    /// ends the run with `BudgetExhausted`. Names are tool spec / action
    /// names. Empty (default) keeps the old behavior: no tools.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wrap_up_tools: Vec<String>,
    /// How many tool batches may run in wrap-up mode before the context
    /// gives up with `BudgetExhausted` (`LLMContextState.wrap_up_rounds_used`).
    #[serde(skip_serializing_if = "is_default_wrap_up_rounds")]
    pub wrap_up_rounds: u32,
}

/// Default `ToolPolicy.wrap_up_rounds`.
pub const DEFAULT_WRAP_UP_ROUNDS: u32 = 2;

fn is_default_wrap_up_rounds(n: &u32) -> bool {
    *n == DEFAULT_WRAP_UP_ROUNDS
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            mode: ToolMode::All,
            whitelist: Vec::new(),
            action_mode: ToolMode::All,
            action_whitelist: Vec::new(),
            max_tool_iterations: 8,
            max_calls_per_round: 8,
            disable_capabilities: Vec::new(),
            finish_grace_ms: 30_000,
            wrap_up_tools: Vec::new(),
            wrap_up_rounds: DEFAULT_WRAP_UP_ROUNDS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputSpec {
    /// Free-form text, caller parses it themselves.
    Text,
    /// Force JSON. `schema` is forwarded to the provider adapter, which
    /// enforces it natively where the provider supports structured output;
    /// without one the provider is asked for any JSON object.
    ///
    /// Without `max_repairs` the waist only parses: `strict = true` makes a
    /// parse failure an LLM-correctable `OutputParse` error (bounded by
    /// `ErrorPolicy`); `strict = false` returns the raw text as
    /// `ContextOutput::Text` instead.
    ///
    /// With `max_repairs = Some(n)` the output is a reply contract: the final
    /// reply must parse, match `schema` (`json_check`, a subset of JSON
    /// Schema) and pass the host's `OutputValidator`. A reply that fails gets
    /// a correction message and another inference, at most `n` times per
    /// run (`LLMContextState.output_repairs`); after that the raw text is
    /// returned as `ContextOutput::Invalid` and the host decides. Repairs do
    /// not count as consecutive errors.
    Json {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        #[serde(default)]
        strict: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_repairs: Option<u32>,
    },
}

impl Default for OutputSpec {
    fn default() -> Self {
        OutputSpec::Text
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAction {
    Fail,
    ReturnPartial,
    EscalateHuman,
}

/// When the waist yields `Outcome::ContextLimitReached { which:
/// ApproachingWindow }`: the estimated size of the request about to be sent
/// reaches (`>=`) this many tokens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextThreshold {
    /// Fraction of `BudgetSpec.context_window_tokens`, in `(0.0, 1.0]`.
    /// Requires the window; the threshold is `floor(window * value)`.
    Ratio { value: f32 },
    /// Absolute token count, `> 0`.
    AbsoluteTokens { value: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BudgetSpec {
    pub max_total_tokens: Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub max_wallclock_ms: Option<u64>,
    pub max_cost_units: Option<u32>,
    pub on_exhausted: BudgetAction,
    /// Early-warning threshold for the context window (see
    /// [`ContextThreshold`]). `None` disables the early yield only; a known
    /// window overflow and a structured provider refusal still yield
    /// `ContextLimitReached`. An invalid threshold (zero, ratio outside
    /// `(0, 1]`, ratio without a window) ends the run with `Internal` before
    /// any inference.
    pub context_yield_threshold: Option<ContextThreshold>,
    /// Effective context window of the model(s) this request may run on, in
    /// tokens, supplied by the caller / provider adapter (the waist never
    /// looks it up). When set, a request whose estimate plus the completion
    /// reserve (`model_policy.max_completion_tokens`) exceeds it is not sent:
    /// the run yields `ContextLimitReached { which: HardLimit }`.
    pub context_window_tokens: Option<u32>,
}

impl Default for BudgetSpec {
    fn default() -> Self {
        Self {
            max_total_tokens: None,
            max_completion_tokens: None,
            max_wallclock_ms: None,
            max_cost_units: None,
            on_exhausted: BudgetAction::Fail,
            context_yield_threshold: None,
            context_window_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ErrorPolicy {
    /// LLM-correctable errors (`LLMComputeError::llm_correctable`) are fed
    /// back into the history so the next inference can self-correct. The
    /// counter increments once per failed iteration (one response and its
    /// tool batch, or one behavior step), no matter how many calls in that
    /// iteration failed, and resets only after an iteration completes
    /// without any correctable error. A successful provider request by
    /// itself does not reset it.
    ///
    /// `N` means at most `N` error feedbacks; the `N+1`-th consecutive
    /// failure ends the run with `Outcome::Error`. `0` disables the cap and
    /// leaves only the budget as a stop condition (not recommended).
    /// Provider, runtime, snapshot and internal errors never touch this
    /// counter: they end the run immediately.
    pub max_consecutive_errors: u32,
}

impl Default for ErrorPolicy {
    fn default() -> Self {
        Self {
            max_consecutive_errors: 3,
        }
    }
}

/// Classification of an error after waist sees it. Derived exhaustively
/// from `LLMComputeError::llm_correctable`; `Fatal` cannot be overridden by
/// `ErrorPolicy`.
#[derive(Debug, Clone)]
pub enum ErrorClass {
    Recoverable(crate::error::LLMComputeError),
    Fatal(crate::error::LLMComputeError),
}

impl From<crate::error::LLMComputeError> for ErrorClass {
    fn from(err: crate::error::LLMComputeError) -> Self {
        if err.llm_correctable() {
            ErrorClass::Recoverable(err)
        } else {
            ErrorClass::Fatal(err)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMContextRequest {
    /// Who runs this context, for tracing / auditing only: opaque to the
    /// waist (the scheduler's own identity shape).
    #[serde(default)]
    pub owner: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,

    /// Natural-language objective for audit / worklog. Not sent to the LLM.
    #[serde(default)]
    pub objective: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub behavior_name: String,

    /// Already-compiled conversation history (system / user / assistant / tool).
    /// The host is responsible for template expansion before reaching the
    /// waist.
    pub input: Vec<AiMessage>,

    #[serde(default)]
    pub model_policy: ModelPolicy,
    #[serde(default)]
    pub tool_policy: ToolPolicy,
    #[serde(default)]
    pub output: OutputSpec,
    #[serde(default)]
    pub budget: BudgetSpec,
    #[serde(default)]
    pub error_policy: ErrorPolicy,

    /// Hard constraint for Behavior Loop: when `true`, any `<next_behavior>`
    /// produced by the parser is discarded (logged) and the step is treated
    /// as if `next_behavior == None`. Used by fork sub-contexts whose
    /// control flow must end with the run, never jump to another behavior.
    ///
    /// Default `false` — set directly in the request when building a fork
    /// context.
    #[serde(default)]
    pub forbid_next_behavior: bool,
}
