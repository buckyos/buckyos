//! §9.3 of NewOpenDANRuntime — Behavior TOML config.
//!
//! A behavior is one entry in `<agent_root>/behaviors/<name>.toml`. The runtime
//! reads it on session start and on every `switch_behavior(...)` outcome, then
//! translates the fields into a `LLMContextRequest` + `LLMContextDeps` pair
//! consumable by the waist (`llm_context`).

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

/// Behavior mode — picked up at deps assembly time. `Agent` ⇒ traditional
/// agent loop (provider `tool_calls`). `Behavior` ⇒ plug parser + renderer
/// into deps so the waist runs the behavior outer-loop.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorMode {
    Agent,
    Behavior,
}

impl Default for BehaviorMode {
    fn default() -> Self {
        BehaviorMode::Behavior
    }
}

/// How `switch_behavior(next)` rebases the next LLMContext. See §3 of the
/// notepad — only `Normal` is wired in MVP; `Fork` / `Independent` are
/// recognised so the TOML round-trips, runtime falls back to `Normal`
/// for the unwired variants.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwitchMode {
    Normal,
    Fork,
    Independent,
}

impl Default for SwitchMode {
    fn default() -> Self {
        SwitchMode::Normal
    }
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RendererCfg {
    pub recent_full_steps: Option<usize>,
    pub summary_chars: Option<usize>,
    pub max_result_chars: Option<usize>,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BudgetCfg {
    pub max_total_tokens: Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub max_wallclock_ms: Option<u64>,
    pub max_cost_units: Option<u32>,
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

/// Single-behavior TOML config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorCfg {
    pub name: String,
    pub objective: String,
    pub system_prompt_template: String,
    pub tool_whitelist: Vec<String>,
    pub approval_required: Vec<String>,
    /// Optional tool plan name (§9.2) — resolves to
    /// `<agent_root>/tool_plans/<name>.toml`. Empty ⇒ no tombstones.
    /// Behavior config and tool plan are decoupled on purpose: the
    /// whitelist gates *visibility to the LLM*, the tool plan gates
    /// *what gets executed when an LLM-issued bash command happens to
    /// invoke a name from the lower bin layers*. Both are needed.
    pub tool_plan: String,
    pub mode: BehaviorMode,
    pub parser: String,
    pub renderer: String,
    pub parser_strict: bool,
    pub renderer_cfg: RendererCfg,
    pub output: BehaviorOutput,
    pub max_rounds: u32,
    pub max_consecutive_errors: u32,
    pub switch_mode: SwitchMode,
    pub model: ModelCfg,
    pub budget: Option<BudgetCfg>,
}

impl Default for BehaviorCfg {
    fn default() -> Self {
        Self {
            name: String::new(),
            objective: String::new(),
            system_prompt_template: String::new(),
            tool_whitelist: Vec::new(),
            approval_required: Vec::new(),
            tool_plan: String::new(),
            mode: BehaviorMode::default(),
            parser: "xml".to_string(),
            renderer: "xml".to_string(),
            parser_strict: false,
            renderer_cfg: RendererCfg::default(),
            output: BehaviorOutput::default(),
            max_rounds: 16,
            max_consecutive_errors: 3,
            switch_mode: SwitchMode::default(),
            model: ModelCfg::default(),
            budget: None,
        }
    }
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
        if self.name.trim().is_empty() {
            return Err(BehaviorCfgError::Invalid {
                name: self.name.clone(),
                reason: "name must not be empty".to_string(),
            });
        }
        match self.parser.as_str() {
            "xml" | "agent" => {}
            other => {
                return Err(BehaviorCfgError::Invalid {
                    name: self.name.clone(),
                    reason: format!("unknown parser `{other}` (expected `xml` or `agent`)"),
                });
            }
        }
        match self.renderer.as_str() {
            "xml" => {}
            other => {
                return Err(BehaviorCfgError::Invalid {
                    name: self.name.clone(),
                    reason: format!("unknown renderer `{other}` (only `xml` supported)"),
                });
            }
        }
        Ok(())
    }

    pub fn to_tool_policy(&self) -> ToolPolicy {
        let mode = if self.tool_whitelist.is_empty() {
            ToolMode::All
        } else {
            ToolMode::Whitelist
        };
        ToolPolicy {
            mode,
            whitelist: self.tool_whitelist.clone(),
            max_rounds: self.max_rounds,
            ..Default::default()
        }
    }

    pub fn to_human_policy(&self) -> HumanPolicy {
        HumanPolicy {
            approval_required: self.approval_required.clone(),
        }
    }

    pub fn to_error_policy(&self) -> ErrorPolicy {
        ErrorPolicy {
            max_consecutive_errors: self.max_consecutive_errors,
        }
    }

    pub fn to_budget_spec(&self) -> BudgetSpec {
        self.budget
            .as_ref()
            .map(|b| b.to_budget_spec())
            .unwrap_or_default()
    }

    pub fn to_output_spec(&self) -> OutputSpec {
        self.output.to_output_spec()
    }

    pub fn to_model_policy(&self) -> ModelPolicy {
        self.model.to_model_policy()
    }

    /// Build the parser+renderer Arc-pair to plug into `LLMContextDeps`.
    /// Returns `None` for `BehaviorMode::Agent` (traditional loop, no parser).
    pub fn build_parser_and_renderer(
        &self,
    ) -> Option<(Arc<dyn LLMResultParser>, Arc<dyn StepRenderer>)> {
        if self.mode != BehaviorMode::Behavior {
            return None;
        }
        let parser: Arc<dyn LLMResultParser> = match self.parser.as_str() {
            "xml" => Arc::new(XmlBehaviorParser {
                strict: self.parser_strict,
            }),
            _ => return None,
        };
        let mut renderer = XmlStepRenderer::new();
        if let Some(n) = self.renderer_cfg.recent_full_steps {
            renderer = renderer.with_recent_full_steps(n);
        }
        if let Some(n) = self.renderer_cfg.summary_chars {
            renderer = renderer.with_summary_chars(n);
        }
        if let Some(n) = self.renderer_cfg.max_result_chars {
            renderer = renderer.with_max_result_chars(n);
        }
        let renderer: Arc<dyn StepRenderer> = Arc::new(renderer);
        Some((parser, renderer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse() {
        let toml = r#"
            name = "ui_default"
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml).unwrap();
        assert_eq!(cfg.name, "ui_default");
        assert_eq!(cfg.parser, "xml");
        assert!(matches!(cfg.mode, BehaviorMode::Behavior));
        assert!(matches!(cfg.switch_mode, SwitchMode::Normal));
        assert!(matches!(cfg.to_tool_policy().mode, ToolMode::All));
    }

    #[test]
    fn whitelist_drives_tool_mode() {
        let toml = r#"
            name = "x"
            tool_whitelist = ["exec_bash", "read_file"]
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml).unwrap();
        let pol = cfg.to_tool_policy();
        assert!(matches!(pol.mode, ToolMode::Whitelist));
        assert_eq!(pol.whitelist, vec!["exec_bash", "read_file"]);
    }

    #[test]
    fn reject_unknown_parser() {
        let toml = r#"
            name = "bad"
            parser = "yaml"
        "#;
        let cfg = BehaviorCfg::from_toml_str(toml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn behavior_mode_builds_xml_parser_renderer() {
        let cfg = BehaviorCfg {
            name: "x".to_string(),
            ..Default::default()
        };
        assert!(cfg.build_parser_and_renderer().is_some());
    }

    #[test]
    fn agent_mode_no_parser() {
        let cfg = BehaviorCfg {
            name: "x".to_string(),
            mode: BehaviorMode::Agent,
            ..Default::default()
        };
        assert!(cfg.build_parser_and_renderer().is_none());
    }
}
