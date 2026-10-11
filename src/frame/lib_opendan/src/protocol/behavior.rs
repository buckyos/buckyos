//! Behavior configuration of an agent and its frozen form inside
//! `session_config.prompt.frozen` (xAgent §6).
//!
//! A behavior is the agent's configuration (`<agent_root>/behaviors/<name>.toml`):
//! the Agent State provides it (`BehaviorCatalog`), a session freezes what it
//! uses when it is constructed. Frozen material is text and policy, never a
//! rendered result.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::config::{
    BehaviorEntry, BehaviorPrompt, ContextMode, InheritMode, InputSection, SessionConfig,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub objective: String,
    /// Behaviors this one may hand over to or call: the frozen closure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
}

/// Loop of the behavior's own context.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorLoop {
    #[default]
    FunctionCall,
    Behavior,
}

impl BehaviorLoop {
    pub fn as_str(&self) -> &'static str {
        match self {
            BehaviorLoop::FunctionCall => "function_call",
            BehaviorLoop::Behavior => "behavior",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorPromptConfig {
    /// `None`: the session's `prompt.llm_context.loop_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<BehaviorLoop>,
    /// System template of the behavior's context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_init: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_context_switch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semi_subscription_snapshot: Option<String>,
    /// Goal Sessions: template of the Goal header (variable `goal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_activation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorCapabilities {
    /// Tools offered as native functions (`None`: the session's tools). An
    /// entry is a tool name or `group:<builtin group>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_whitelist: Option<Vec<String>>,
    /// Named tools offered as actions (behavior loop).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_whitelist: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_consecutive_errors: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wallclock_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// How the behavior is entered (decided by the target, §3.3). `mode` may
/// only be left out by a session's entry behavior (`switch_context`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorEntryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ContextMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit: Option<InheritMode>,
}

/// `[tool]`: a sub context behavior offered to its callers as a named
/// function. Calling it runs the behavior exactly like `call_behavior`
/// does; the declaration only decides what the caller's model sees.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorToolConfig {
    /// Function name (letters, digits, `_`); default: the behavior name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// When to call it: the caller's routing guide.
    #[serde(default)]
    pub description: String,
    /// JSON Schema of the arguments (an object); default: `{task: string}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /// Template of the sub context's task (`{{args.<name>}}` only); default:
    /// the arguments as JSON (or `args.task` with the default parameters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// Prefix of a `tool_whitelist` entry naming a behavior's `[tool]`.
pub const BEHAVIOR_TOOL_PREFIX: &str = "behavior:";

fn valid_function_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `{{ args.<name> }}` placeholders of a task template, in order; an error
/// for any other template syntax.
fn task_placeholders(template: &str) -> std::result::Result<Vec<(usize, usize, String)>, String> {
    if template.contains("{%") {
        return Err("a task template only takes {{args.<name>}} placeholders".into());
    }
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = template[at..].find("{{").map(|i| at + i) {
        let end = template[start..]
            .find("}}")
            .map(|i| start + i + 2)
            .ok_or_else(|| "unclosed {{ in the task template".to_string())?;
        let inner = template[start + 2..end - 2].trim();
        let key = inner
            .strip_prefix("args.")
            .filter(|k| valid_function_name(k))
            .ok_or_else(|| format!("task template placeholder `{{{{{inner}}}}}`: only args.<name> is available"))?;
        out.push((start, end, key.to_string()));
        at = end;
    }
    Ok(out)
}

impl BehaviorToolConfig {
    pub fn function_name(&self, behavior: &str) -> String {
        self.name.clone().unwrap_or_else(|| behavior.to_string())
    }

    pub fn args_schema(&self) -> Value {
        self.parameters.clone().unwrap_or_else(|| {
            json!({
                "type": "object",
                "properties": { "task": { "type": "string", "description": "what the sub context must do and return" } },
                "required": ["task"]
            })
        })
    }

    pub fn validate(&self, behavior: &str) -> std::result::Result<(), String> {
        let name = self.function_name(behavior);
        if !valid_function_name(&name) {
            return Err(format!(
                "behavior `{behavior}`: [tool] name `{name}` must be letters, digits and `_` (at most 64)"
            ));
        }
        if self.description.trim().is_empty() {
            return Err(format!("behavior `{behavior}`: [tool] needs a description"));
        }
        let schema = self.args_schema();
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(format!(
                "behavior `{behavior}`: [tool] parameters must be a JSON Schema of type object"
            ));
        }
        if let Some(t) = &self.task {
            task_placeholders(t).map_err(|e| format!("behavior `{behavior}`: {e}"))?;
        }
        Ok(())
    }

    /// The sub context's task for a call with `args`.
    pub fn render_task(&self, args: &Value) -> String {
        let text = |v: &Value| match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            v => v.to_string(),
        };
        let Some(template) = &self.task else {
            return match (&self.parameters, args.get("task")) {
                (None, Some(t)) => text(t),
                _ => args.to_string(),
            };
        };
        let Ok(holes) = task_placeholders(template) else {
            return args.to_string();
        };
        let mut out = String::with_capacity(template.len());
        let mut at = 0;
        for (start, end, key) in holes {
            out.push_str(&template[at..start]);
            out.push_str(&args.get(&key).map(text).unwrap_or_default());
            at = end;
        }
        out.push_str(&template[at..]);
        out.trim().to_string()
    }
}

/// One behavior of the agent, includes expanded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorConfig {
    #[serde(default)]
    pub meta: BehaviorMeta,
    #[serde(default)]
    pub prompt: BehaviorPromptConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputSection>,
    #[serde(default)]
    pub capabilities: BehaviorCapabilities,
    #[serde(default)]
    pub budget: BehaviorBudget,
    #[serde(default)]
    pub model: BehaviorModel,
    #[serde(default)]
    pub entry: BehaviorEntryConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<BehaviorToolConfig>,
    /// Host hooks (`on_context_limit_reached` …): carried, not interpreted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hooks: BTreeMap<String, Value>,
}

impl BehaviorConfig {
    /// Keys laid over `prompt.llm_context` for a run of this behavior (only
    /// keys `prepare_hosted` uses: model, loop_model, limits, tools).
    pub fn overlay_llm_context(&self, base: &Value) -> Value {
        let mut over = serde_json::Map::new();
        if let Some(m) = &self.model.preferred {
            over.insert("model".into(), json!(m));
        }
        if let Some(mode) = self.prompt.mode {
            over.insert("loop_model".into(), json!(mode.as_str()));
        }
        if let Some(n) = self.budget.max_tool_iterations {
            over.insert("max_tool_iterations".into(), json!(n));
        }
        if let Some(n) = self.budget.max_completion_tokens {
            over.insert("max_tokens".into(), json!(n));
        }
        // `group:<name>` names a builtin tool group, anything else one tool.
        let named = |names: &Vec<String>| -> Value {
            names
                .iter()
                .map(|n| match n.strip_prefix("group:") {
                    Some(g) => json!({ "groupname": g }),
                    None => json!({ "name": n }),
                })
                .collect()
        };
        if self.capabilities.tool_whitelist.is_some() || self.capabilities.action_whitelist.is_some()
        {
            let mut tools = base
                .get("tools")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(w) = &self.capabilities.tool_whitelist {
                tools.insert("tools".into(), named(w));
            }
            if let Some(w) = &self.capabilities.action_whitelist {
                tools.insert("actions".into(), named(w));
            }
            over.insert("tools".into(), Value::Object(tools));
        }
        if over.is_empty() {
            Value::Null
        } else {
            Value::Object(over)
        }
    }

    /// The entry configuration the runner uses
    /// (`extensions.opendan.behaviors.<name>`). `initial`: the session's
    /// entry behavior, which may leave its mode out.
    pub fn to_entry(
        &self,
        name: &str,
        base_llm_context: &Value,
        initial: bool,
    ) -> std::result::Result<BehaviorEntry, String> {
        let mode = match (self.entry.mode, initial) {
            (Some(m), _) => m,
            (None, true) => ContextMode::SwitchContext,
            (None, false) => {
                return Err(format!(
                    "behavior `{name}` declares no entry mode ([entry] mode = switch_context | create_sub_context | fork)"
                ))
            }
        };
        let mut llm_context = self.overlay_llm_context(base_llm_context);
        if mode == ContextMode::Fork {
            // A fork keeps the caller's configuration; its `[budget]` limits
            // the branch's run and is applied by the runner, not here.
            if let Some(o) = llm_context.as_object_mut() {
                o.remove("max_tool_iterations");
                o.remove("max_tokens");
                if o.is_empty() {
                    llm_context = Value::Null;
                }
            }
        }
        let entry = BehaviorEntry {
            mode,
            prompt: BehaviorPrompt {
                system: self.prompt.system.clone(),
                on_init: self.prompt.on_init.clone(),
                on_input: self.prompt.on_input.clone(),
                on_context_switch: self.prompt.on_context_switch.clone(),
                semi_subscription_snapshot: self.prompt.semi_subscription_snapshot.clone(),
                goal_activation: self.prompt.goal_activation.clone(),
            },
            input: self.input,
            llm_context,
            inherit: self.entry.inherit.unwrap_or(match mode {
                ContextMode::SwitchContext if initial => InheritMode::RecentDialogue,
                _ => InheritMode::None,
            }),
            tool: self.tool.clone(),
        };
        entry.validate(name)?;
        Ok(entry)
    }
}

/// Identity text of the agent (`role.md`, `self.md`, `i18n/`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IdentityText {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, rename = "self", skip_serializing_if = "String::is_empty")]
    pub self_text: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub i18n: BTreeMap<String, String>,
}

/// `prompt.frozen`: what the session took from the behavior catalog when it
/// was constructed. Later catalog changes never reach the session; a
/// behavior used for the first time afterwards is added once
/// (`control_applied{behavior_frozen}` in the worklog).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrozenPrompt {
    /// Catalog revision at the first freeze (audit).
    #[serde(default)]
    pub catalog_rev: String,
    #[serde(default)]
    pub frozen_at_ms: u64,
    #[serde(default)]
    pub frozen_by: String,
    #[serde(default)]
    pub identity: IdentityText,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub behaviors: BTreeMap<String, BehaviorConfig>,
}

impl SessionConfig {
    /// Write the entry of a frozen behavior into
    /// `extensions.opendan.behaviors`. An application entry that differs is
    /// a conflict: a frozen session takes its behaviors from the catalog.
    pub fn set_frozen_entry(
        &mut self,
        name: &str,
        behavior: &BehaviorConfig,
    ) -> std::result::Result<(), String> {
        let initial = self.prompt.behavior.as_deref() == Some(name);
        let entry = behavior.to_entry(name, &self.prompt.llm_context, initial)?;
        let value = serde_json::to_value(&entry).map_err(|e| e.to_string())?;
        let opendan = self
            .extensions
            .entry("opendan".to_string())
            .or_insert_with(|| json!({}));
        if !opendan.is_object() {
            return Err("extensions.opendan must be an object".into());
        }
        let table = &mut opendan["behaviors"];
        if table.is_null() {
            *table = json!({});
        }
        match table.get(name) {
            Some(existing) if existing != &value => Err(format!(
                "behavior `{name}` is frozen from the agent's catalog; extensions.opendan.behaviors.{name} conflicts with it"
            )),
            _ => {
                table[name] = value;
                Ok(())
            }
        }
    }
}
