//! Behavior TOML config — per doc/opendan/Agent配置改进.md §5.
//!
//! A behavior lives at `<agent_root>/behaviors/<name>.toml`. The runtime
//! reads it on session start and on every `switch_behavior(...)` outcome
//! and translates the fields into an `LLMContextRequest` + `LLMContextDeps`
//! pair consumable by the waist (`llm_context`).
//!
//! New schema groups fields into:
//!   * `[meta]` — name / objective
//!   * `[prompt]` — parser choice + per-event prompt templates
//!     (`on_init` / `on_input_msg` / `on_input_event`) + output spec
//!   * `[capabilities]` — tool_whitelist / approval_required / tool_plan
//!     (v0 placeholder; §5.3 — will be redone in beta2.3 alongside skill
//!     bundle / function-call tool unification)
//!   * `[budget]` — round / error / token / wallclock caps
//!   * `[model]` — preferred / fallbacks / temperature / provider options
//!   * `[on_xxx]` — optional bypass switches (see [`HookPoint`])
//!
//! `mode` / `switch_mode` / `renderer` / `parser_strict` / `renderer_cfg`
//! from the pre-beta2.2 schema are gone — `loop_mode` and `switch_mode`
//! moved to the session class (`[session.<class>]` in `agent.toml`);
//! renderer details default to runtime built-ins.

use std::path::Path;
use std::sync::Arc;

use llm_context::{
    behavior_loop::{LLMResultParser, StepRenderer},
    request::{
        BudgetSpec, ErrorPolicy, HumanPolicy, ModelPolicy, OutputSpec,
        ToolMode, ToolPolicy,
    },
    step_record::XmlStepRenderer,
    xml_behavior::XmlBehaviorParser,
};
use serde::{Deserialize, Serialize};

use crate::agent_config::LoopMode;
use crate::hook_point::HookPoint;

// ─── `[meta]` ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MetaCfg {
    pub name: String,
    pub objective: String,
}

// ─── `[prompt]` ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorOutput {
    Text,
    Json {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<serde_json::Value>,
        #[serde(default)]
        strict: bool,
    },
}

impl Default for BehaviorOutput {
    fn default() -> Self {
        BehaviorOutput::Text
    }
}

impl BehaviorOutput {
    pub fn to_output_spec(&self) -> OutputSpec {
        match self {
            BehaviorOutput::Text => OutputSpec::Text,
            BehaviorOutput::Json { schema, strict } => OutputSpec::Json {
                schema: schema.clone(),
                strict: *strict,
            },
        }
    }
}

/// Prompt templates per "renders the prompt" event. Each `on_*` field is
/// rendered with the simple `{var}` substitution engine — same one
/// `format_event_for_turn` uses for per-subscription event templates,
/// so behavior authors and subscription authors learn one syntax.
///
/// **Per-event hook names map onto the actual prompt-construction sites
/// the runtime has today**:
///
/// | template          | callsite                              | empty ⇒              |
/// |-------------------|---------------------------------------|---------------------|
/// | `on_init`         | `render_system_messages` (System body)| `role.md` + `self.md` + objective + readme fallback |
/// | `on_input_msg`    | wraps each `PendingInput::Msg` text   | raw `AiMessage` is passed through (today's default) |
/// | `on_input_event`  | behavior-wide event-format fallback   | falls through to subscription template, then to `format_event_for_turn_with_subscriptions` default |
///
/// Available variables (passed in by the consuming callsite):
///   * `{agent_name}`, `{behavior_name}`, `{session_id}`
///   * `on_init`: `{objective}`, `{role_md}`, `{self_md}`, `{workspace_id}`
///   * `on_input_msg`: `{msg_text}`, `{msg_from_did}`, `{msg_from_name}`
///   * `on_input_event`: `{event_id}`, `{event_data}` plus any top-level
///     scalar field of the JSON payload as `{<key>}`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PromptCfg {
    /// XML parser variant. v0 only accepts `"xml"`.
    pub parser: String,
    /// Strict-parse toggle for malformed `<actions>` blocks. Default
    /// `false` ⇒ runtime salvages partials and continues.
    pub parser_strict: bool,
    pub on_init: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_input_msg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_input_event: Option<String>,
    pub output: BehaviorOutput,
}

impl Default for PromptCfg {
    fn default() -> Self {
        Self {
            parser: "xml".to_string(),
            parser_strict: false,
            on_init: String::new(),
            on_input_msg: None,
            on_input_event: None,
            output: BehaviorOutput::Text,
        }
    }
}

// ─── `[capabilities]` (v0 placeholder — see doc §5.3) ──────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CapabilitiesCfg {
    pub tool_whitelist: Vec<String>,
    pub approval_required: Vec<String>,
    /// Optional tool plan name (§9.2) — resolves to
    /// `<agent_root>/tool_plans/<name>.toml`. Empty ⇒ no tombstones.
    pub tool_plan: String,
}

// ─── `[budget]` ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BudgetCfg {
    pub max_rounds: u32,
    pub max_consecutive_errors: u32,
    pub max_total_tokens: Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub max_wallclock_ms: Option<u64>,
    pub max_cost_units: Option<u32>,
}

impl Default for BudgetCfg {
    fn default() -> Self {
        Self {
            max_rounds: 16,
            max_consecutive_errors: 3,
            max_total_tokens: None,
            max_completion_tokens: None,
            max_wallclock_ms: None,
            max_cost_units: None,
        }
    }
}

impl BudgetCfg {
    pub fn to_budget_spec(&self) -> BudgetSpec {
        BudgetSpec {
            max_total_tokens: self.max_total_tokens,
            max_completion_tokens: self.max_completion_tokens,
            max_wallclock_ms: self.max_wallclock_ms,
            max_cost_units: self.max_cost_units,
            ..Default::default()
        }
    }
}

// ─── `[model]` ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelCfg {
    pub preferred: String,
    pub fallbacks: Vec<String>,
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    pub provider_options: Option<serde_json::Value>,
}

impl ModelCfg {
    pub fn to_model_policy(&self) -> ModelPolicy {
        ModelPolicy {
            preferred: self.preferred.clone(),
            fallbacks: self.fallbacks.clone(),
            temperature: self.temperature,
            max_completion_tokens: self.max_completion_tokens,
            provider_options: self.provider_options.clone(),
        }
    }
}

// ─── Behavior root ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorCfg {
    pub meta: MetaCfg,
    pub prompt: PromptCfg,
    pub capabilities: CapabilitiesCfg,
    pub budget: BudgetCfg,
    pub model: ModelCfg,
    /// Bypass switches — written ⇒ enable; omitted ⇒ runtime default.
    /// See [`behavior_hooks`](crate::behavior_hooks).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_context_limit_reached: Option<HookPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_provider_failed: Option<HookPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_interrupt_graceful: Option<HookPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_interrupt_discard: Option<HookPoint>,
}

#[derive(Debug, thiserror::Error)]
pub enum BehaviorCfgError {
    #[error("read {path}: {err}")]
    Io { path: String, err: std::io::Error },
    #[error("parse {path}: {err}")]
    Parse { path: String, err: toml::de::Error },
    #[error("invalid behavior `{name}`: {reason}")]
    Invalid { name: String, reason: String },
}

impl BehaviorCfg {
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn load_from_file(path: &Path) -> Result<Self, BehaviorCfgError> {
        let bytes = std::fs::read_to_string(path).map_err(|err| BehaviorCfgError::Io {
            path: path.display().to_string(),
            err,
        })?;
        let cfg: BehaviorCfg = toml::from_str(&bytes).map_err(|err| BehaviorCfgError::Parse {
            path: path.display().to_string(),
            err,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), BehaviorCfgError> {
        if self.meta.name.trim().is_empty() {
            return Err(BehaviorCfgError::Invalid {
                name: self.meta.name.clone(),
                reason: "meta.name must not be empty".to_string(),
            });
        }
        match self.prompt.parser.as_str() {
            "xml" | "agent" => {}
            other => {
                return Err(BehaviorCfgError::Invalid {
                    name: self.meta.name.clone(),
                    reason: format!("unknown parser `{other}` (expected `xml` or `agent`)"),
                });
            }
        }
        Ok(())
    }

    /// Convenience accessor — kept for callers that still want a single
    /// `name` reference rather than `cfg.meta.name`.
    pub fn name(&self) -> &str {
        &self.meta.name
    }

    /// Convenience accessor mirroring [`Self::name`].
    pub fn objective(&self) -> &str {
        &self.meta.objective
    }

    pub fn to_tool_policy(&self) -> ToolPolicy {
        let mode = if self.capabilities.tool_whitelist.is_empty() {
            ToolMode::All
        } else {
            ToolMode::Whitelist
        };
        ToolPolicy {
            mode,
            whitelist: self.capabilities.tool_whitelist.clone(),
            max_rounds: self.budget.max_rounds,
            ..Default::default()
        }
    }

    pub fn to_human_policy(&self) -> HumanPolicy {
        HumanPolicy {
            approval_required: self.capabilities.approval_required.clone(),
        }
    }

    pub fn to_error_policy(&self) -> ErrorPolicy {
        ErrorPolicy {
            max_consecutive_errors: self.budget.max_consecutive_errors,
        }
    }

    pub fn to_budget_spec(&self) -> BudgetSpec {
        self.budget.to_budget_spec()
    }

    pub fn to_output_spec(&self) -> OutputSpec {
        self.prompt.output.to_output_spec()
    }

    pub fn to_model_policy(&self) -> ModelPolicy {
        self.model.to_model_policy()
    }

    /// Build the parser+renderer Arc-pair to plug into `LLMContextDeps`.
    /// The loop mode comes from the session class, not the behavior — pass
    /// `LoopMode::Agent` to get `None` (traditional loop, no parser).
    pub fn build_parser_and_renderer(
        &self,
        loop_mode: LoopMode,
    ) -> Option<(Arc<dyn LLMResultParser>, Arc<dyn StepRenderer>)> {
        if !matches!(loop_mode, LoopMode::Behavior) {
            return None;
        }
        let parser: Arc<dyn LLMResultParser> = match self.prompt.parser.as_str() {
            "xml" => Arc::new(XmlBehaviorParser {
                strict: self.prompt.parser_strict,
            }),
            _ => return None,
        };
        let renderer: Arc<dyn StepRenderer> = Arc::new(XmlStepRenderer::new());
        Some((parser, renderer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse() {
        let toml_src = r#"
            [meta]
            name = "ui_default"
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml_src).unwrap();
        assert_eq!(cfg.name(), "ui_default");
        assert_eq!(cfg.prompt.parser, "xml");
        assert!(matches!(cfg.to_tool_policy().mode, ToolMode::All));
        assert_eq!(cfg.budget.max_rounds, 16);
    }

    #[test]
    fn whitelist_drives_tool_mode() {
        let toml_src = r#"
            [meta]
            name = "x"

            [capabilities]
            tool_whitelist = ["exec_bash", "read_file"]
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml_src).unwrap();
        let pol = cfg.to_tool_policy();
        assert!(matches!(pol.mode, ToolMode::Whitelist));
        assert_eq!(pol.whitelist, vec!["exec_bash", "read_file"]);
    }

    #[test]
    fn reject_empty_name() {
        let toml_src = r#"
            [meta]
            name = ""
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml_src).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn reject_unknown_parser() {
        let toml_src = r#"
            [meta]
            name = "bad"
            [prompt]
            parser = "yaml"
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml_src).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn behavior_loop_mode_builds_xml_parser() {
        let cfg = BehaviorCfg {
            meta: MetaCfg {
                name: "x".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(cfg.build_parser_and_renderer(LoopMode::Behavior).is_some());
    }

    #[test]
    fn agent_loop_mode_no_parser() {
        let cfg = BehaviorCfg {
            meta: MetaCfg {
                name: "x".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(cfg.build_parser_and_renderer(LoopMode::Agent).is_none());
    }

    #[test]
    fn on_xxx_round_trip() {
        let toml_src = r#"
            [meta]
            name = "x"

            [on_context_limit_reached]
            mode = "compress_then_continue"

            [on_provider_failed]
            mode = "fallback_behavior"
            target = "safe_mode"
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml_src).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.on_context_limit_reached.as_ref().unwrap().mode,
            "compress_then_continue"
        );
        assert_eq!(
            cfg.on_provider_failed
                .as_ref()
                .unwrap()
                .require_string("on_provider_failed", "target")
                .unwrap(),
            "safe_mode"
        );
    }

    #[test]
    fn prompt_on_init_round_trip() {
        let toml_src = r#"
            [meta]
            name = "explorer"

            [prompt]
            parser = "xml"
            on_init = "Hello {agent_name}, you are {behavior_name}."
            on_input_msg = "[from {msg_from_did}] {msg_text}"
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml_src).unwrap();
        assert!(cfg.prompt.on_init.contains("{agent_name}"));
        assert_eq!(
            cfg.prompt.on_input_msg.as_deref(),
            Some("[from {msg_from_did}] {msg_text}")
        );
    }
}
