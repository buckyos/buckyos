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
    /// Every native function call carries a non-empty `intent` argument:
    /// why the call is made now and what it should yield.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub call_intent: bool,
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
    /// A hand-over to this behavior (`next_behavior`) executes the session's
    /// next serial Todo: the host admits it and hands its body over as the
    /// sub task (`create_sub_context` only).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub todo: bool,
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

/// `[reply]`: the reply contract of a behavior's context. The final reply
/// (an assistant text without tool calls) is one JSON value matching
/// `schema`: the provider enforces it natively where it can, the host
/// checks it and asks for one correction (`OutputSpec::Json { max_repairs:
/// Some(1) }`). Function call loop only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorReply {
    /// JSON Schema of the reply (an object schema).
    pub schema: Value,
    /// An array field of the reply that must list every question the
    /// context asked the user (`agent-session ask-user`), one entry each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asks_field: Option<String>,
}

impl BehaviorReply {
    pub fn validate(&self, behavior: &str) -> std::result::Result<(), String> {
        if self.schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(format!(
                "behavior `{behavior}`: [reply] schema must be a JSON Schema of type object"
            ));
        }
        if let Some(f) = &self.asks_field {
            let declared = self
                .schema
                .pointer(&format!("/properties/{f}/type"))
                .and_then(Value::as_str);
            if declared != Some("array") {
                return Err(format!(
                    "behavior `{behavior}`: [reply] asks_field `{f}` must be an array property of the schema"
                ));
            }
        }
        Ok(())
    }
}

/// A session takes at most this many cmd_skills (`prompt.cmd_skills`).
pub const MAX_CMD_SKILLS: usize = 8;

/// `prompt.frozen.cmd_skills[]`: a command manual of the session, frozen
/// with its behaviors (`cmd_skills/<name>.md` without its front matter).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CmdSkill {
    pub name: String,
    pub text: String,
}

/// Kind of an installed skill: a command manual (`cmd_skills/`) or a
/// method a Goal uses (`goal_skills/`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    #[default]
    Cmd,
    Goal,
}

impl SkillKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillKind::Cmd => "cmd",
            SkillKind::Goal => "goal",
        }
    }

    /// Directory of the AgentRoot holding skills of this kind.
    pub fn dir(self) -> &'static str {
        match self {
            SkillKind::Cmd => "cmd_skills",
            SkillKind::Goal => "goal_skills",
        }
    }
}

/// A skill as `skill list` shows it: the front matter of its manual and
/// where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillMeta {
    pub kind: SkillKind,
    pub name: String,
    /// What the skill is for (front matter `description`).
    #[serde(default)]
    pub description: String,
    /// What it needs before it works (front matter `requires`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub requires: String,
    /// `package` (the agent package) | `installed` (`skills.install`).
    #[serde(default)]
    pub origin: String,
    /// An installed skill: the package it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<SkillInstallRecord>,
    /// Commands it brings (`tools/<name>/bin`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// The environment it works in (front matter `baseline`): a session
    /// whose session-runtime declares less does not take it.
    #[serde(default = "standard_baseline")]
    pub baseline: buckyos_api::RuntimeBaseline,
    /// Platforms it works on (front matter `platforms`, comma separated:
    /// `linux`, `macos`, `windows`); empty: all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
}

fn standard_baseline() -> buckyos_api::RuntimeBaseline {
    buckyos_api::RuntimeBaseline::Standard
}

impl Default for SkillMeta {
    fn default() -> Self {
        Self {
            kind: SkillKind::Cmd,
            name: String::new(),
            description: String::new(),
            requires: String::new(),
            origin: String::new(),
            install: None,
            tools: Vec::new(),
            baseline: standard_baseline(),
            platforms: Vec::new(),
        }
    }
}

/// Platform names a skill may require.
pub const SKILL_PLATFORMS: &[&str] = &["linux", "macos", "windows"];

impl SkillMeta {
    /// Why this skill does not work on a runtime of `baseline` and
    /// `platform` (`None`: it does). `platform` empty: unknown, not checked.
    pub fn unmet_on(&self, baseline: buckyos_api::RuntimeBaseline, platform: &str) -> Option<String> {
        if !baseline.meets(self.baseline) {
            return Some(format!(
                "needs an {} environment; the session-runtime is {}",
                self.baseline.as_str(),
                baseline.as_str()
            ));
        }
        if !platform.is_empty() && !self.platforms.is_empty() && !self.platforms.iter().any(|p| p == platform) {
            return Some(format!(
                "runs on {}; the session-runtime is {platform}",
                self.platforms.join(", ")
            ));
        }
        None
    }
}

/// The `baseline` and `platforms` of a skill's front matter; an error names
/// the value that is not one of the fixed words.
pub fn skill_requirements(
    fields: &BTreeMap<String, String>,
) -> std::result::Result<(buckyos_api::RuntimeBaseline, Vec<String>), String> {
    let baseline = match fields.get("baseline").map(|s| s.trim()) {
        None | Some("") | Some("standard") => buckyos_api::RuntimeBaseline::Standard,
        Some("extended") => buckyos_api::RuntimeBaseline::Extended,
        Some(other) => return Err(format!("baseline `{other}` is not standard | extended")),
    };
    let mut platforms = Vec::new();
    for p in fields.get("platforms").map(String::as_str).unwrap_or_default().split(',') {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if !SKILL_PLATFORMS.contains(&p) {
            return Err(format!("platform `{p}` is not one of {}", SKILL_PLATFORMS.join(", ")));
        }
        if !platforms.iter().any(|x| x == p) {
            platforms.push(p.to_string());
        }
    }
    Ok((baseline, platforms))
}

/// Where an installed skill came from (`.meta/skills.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillInstallRecord {
    /// The PIKG package (`pkg_name`).
    pub package: String,
    #[serde(default)]
    pub version: String,
    /// Digest of the installed content (`sha256:` over its files).
    #[serde(default)]
    pub digest: String,
    /// Digest of the `.pikg` file it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
    #[serde(default)]
    pub installed_at_ms: u64,
    #[serde(default)]
    pub installed_by: String,
}

/// Split a skill manual into its front matter (`---` lines around
/// `key: value` lines) and its body. A manual without front matter is all
/// body.
pub fn split_front_matter(text: &str) -> (BTreeMap<String, String>, &str) {
    let mut fields = BTreeMap::new();
    let trimmed = text.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---\n").or_else(|| trimmed.strip_prefix("---\r\n")) else {
        return (fields, text.trim());
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let l = line.trim();
        if l == "---" {
            return (fields, rest[offset..].trim());
        }
        if let Some((k, v)) = l.split_once(':') {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            fields.insert(k.trim().to_string(), v.to_string());
        }
    }
    // No closing line: not front matter.
    (BTreeMap::new(), text.trim())
}

/// The metadata of the manual `text` of skill `name`.
pub fn skill_meta(kind: SkillKind, name: &str, text: &str) -> SkillMeta {
    let (fields, _) = split_front_matter(text);
    // A value that is not one of the fixed words never widens where the
    // skill applies: it counts as extended (installation refuses it).
    let (baseline, platforms) = skill_requirements(&fields).unwrap_or_else(|e| {
        log::warn!("skill {name}: {e}; taken as baseline extended");
        (buckyos_api::RuntimeBaseline::Extended, Vec::new())
    });
    SkillMeta {
        kind,
        name: name.to_string(),
        description: fields.get("description").cloned().unwrap_or_default(),
        requires: fields.get("requires").cloned().unwrap_or_default(),
        baseline,
        platforms,
        ..Default::default()
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<BehaviorReply>,
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
            todo: self.entry.todo,
            reply: self.reply.clone(),
            call_intent: self.capabilities.call_intent,
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
    /// The session's cmd_skills (`prompt.cmd_skills`, in that order),
    /// frozen once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cmd_skills: Vec<CmdSkill>,
    /// cmd_skills named for the session that its session-runtime does not
    /// meet (baseline, platform): not frozen, reported with the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_cmd_skills: Vec<UnavailableCmdSkill>,
}

/// A cmd_skill a session did not take, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnavailableCmdSkill {
    pub name: String,
    pub reason: String,
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
