//! Inputs to one LLMContext invocation: request + policies.
//!
//! All fields here are the immutable "code segment" half of the process
//! analogy — the mutable "registers + stack" half lives in
//! [`crate::state::LLMContextState`].

use buckyos_api::AiMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Owner identity. Used purely for tracing / auditing — waist itself does not
/// inspect it. Carry whatever the scheduler needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextOwnerRef {
    Agent {
        session_id: String,
    },
    Workflow {
        instance_id: String,
        node_id: String,
    },
    OneShot {
        id: String,
    },
    Other {
        label: String,
    },
}

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
/// * **actions** — XML behavior-loop tags (`exec_bash`, `write_file`, …)
///   parsed out of the LLM response. Controlled by `action_mode` +
///   `action_whitelist`. Filters dispatch only — the recognized tag set
///   itself is hardcoded by the XML parser.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ToolPolicy {
    pub mode: ToolMode,
    pub whitelist: Vec<String>,
    pub action_mode: ToolMode,
    pub action_whitelist: Vec<String>,
    /// 0 disables the tool loop (one inference only).
    pub max_rounds: u32,
    pub max_calls_per_round: u32,
    pub max_observation_bytes: u32,
    pub disable_capabilities: Vec<String>,
    /// Whether tool calls within the same round may run concurrently.
    pub parallel: bool,
    /// Whether ToolManager is allowed to return `Observation::Pending` and
    /// therefore yield `Outcome::PendingTool`. First version: false.
    pub allow_deferred: bool,
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            mode: ToolMode::All,
            whitelist: Vec::new(),
            action_mode: ToolMode::All,
            action_whitelist: Vec::new(),
            max_rounds: 8,
            max_calls_per_round: 8,
            max_observation_bytes: 32 * 1024,
            disable_capabilities: Vec::new(),
            parallel: false,
            allow_deferred: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputSpec {
    /// Free-form text, caller parses it themselves.
    Text,
    /// Force JSON. Optional schema is informational for now; strict mode is
    /// declared but enforcement depth depends on the provider adapter.
    Json {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        #[serde(default)]
        strict: bool,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextThreshold {
    /// Fraction of provider window in [0.0, 1.0].
    Ratio { value: f32 },
    /// Absolute used-token count.
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
    /// Warning threshold for context window. `None` disables — hitting the
    /// provider hard edge then becomes a normal `Outcome::Error`.
    pub context_yield_threshold: Option<ContextThreshold>,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct HumanPolicy {
    /// Tool/action names that require human approval before dispatch.
    pub approval_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ErrorPolicy {
    /// Recoverable errors are folded into the accumulated history as a
    /// tool/user observation so the LLM can self-correct. Once the count
    /// of consecutive recoverable errors exceeds `max_consecutive_errors`,
    /// the loop escalates to a terminal `Outcome::Error`.
    ///
    /// Safety net against "feed error → see error → produce same error"
    /// loops. 0 disables the cap (not recommended).
    pub max_consecutive_errors: u32,
}

impl Default for ErrorPolicy {
    fn default() -> Self {
        Self {
            max_consecutive_errors: 3,
        }
    }
}

/// Classification of an error after waist sees it. `Fatal` cannot be
/// overridden by `ErrorPolicy.mode`.
#[derive(Debug, Clone)]
pub enum ErrorClass {
    Recoverable(crate::error::LLMComputeError),
    Fatal(crate::error::LLMComputeError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMContextRequest {
    pub owner: ContextOwnerRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,

    /// Natural-language objective for audit / worklog. Not sent to the LLM.
    #[serde(default)]
    pub objective: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub behavior_name: String,

    /// Already-compiled conversation history (system / user / assistant / tool).
    /// The L4 prompt compiler is responsible for template expansion before
    /// reaching the waist.
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
    pub human_policy: HumanPolicy,
    #[serde(default)]
    pub error_policy: ErrorPolicy,

    /// Hard constraint for Behavior Loop: when `true`, any `<next_behavior>`
    /// produced by the parser is discarded (logged) and the step is treated
    /// as if `next_behavior == None`. Used by fork sub-contexts whose
    /// control flow must end with the run, never jump to another behavior.
    ///
    /// Default `false` — set via `RequestOverrides.forbid_next_behavior` on
    /// the snapshot-rebuild path, or directly in the request when building
    /// a fresh fork context.
    #[serde(default)]
    pub forbid_next_behavior: bool,
}
