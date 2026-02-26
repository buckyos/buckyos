use std::collections::HashSet;
use std::path::{Path, PathBuf};

use buckyos_api::AiToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tokio::fs;

use crate::agent_tool::ToolSpec;

use super::types::{LLMBehaviorConfig, StepLimits};

const BEHAVIOR_CONFIG_EXTENSIONS: [&str; 3] = ["yaml", "yml", "json"];

#[derive(thiserror::Error, Debug)]
pub enum BehaviorConfigError {
    #[error("behavior name cannot be empty")]
    EmptyBehaviorName,
    #[error(
        "behavior `{behavior}` config not found in `{dir}` (tried: {tried_paths})",
        tried_paths = .tried_paths.join(", ")
    )]
    NotFound {
        behavior: String,
        dir: String,
        tried_paths: Vec<String>,
    },
    #[error("read behavior config `{path}` failed: {source}")]
    ReadFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid behavior config `{path}`: {message}")]
    Invalid { path: String, message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorConfig {
    pub name: String,
    pub process_rule: String,
    pub policy: String,
    pub input: String,
    pub memory: BehaviorMemoryConfig,
    pub step_limit: u32,
    pub output_protocol: BehaviorOutputProtocol,
    pub tools: BehaviorToolsConfig,
    pub toolbox: BehaviorToolboxConfig,
    pub llm: LLMBehaviorConfig,
    pub limits: StepLimits,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            process_rule: String::new(),
            policy: String::new(),
            input: String::new(),
            memory: BehaviorMemoryConfig::default(),
            step_limit: 0,
            output_protocol: BehaviorOutputProtocol::default(),
            tools: BehaviorToolsConfig::default(),
            toolbox: BehaviorToolboxConfig::default(),
            llm: LLMBehaviorConfig::default(),
            limits: StepLimits::default(),
        }
    }
}

impl BehaviorConfig {
    pub async fn load_from_dir(
        behaviors_dir: impl AsRef<Path>,
        behavior_name: &str,
    ) -> Result<Self, BehaviorConfigError> {
        let behavior_name = behavior_name.trim();
        if behavior_name.is_empty() {
            return Err(BehaviorConfigError::EmptyBehaviorName);
        }

        let behaviors_dir = behaviors_dir.as_ref();
        let mut tried_paths = Vec::new();
        let candidate_paths = candidate_paths_for_behavior(behaviors_dir, behavior_name);
        for path in candidate_paths {
            tried_paths.push(path.display().to_string());
            if !is_file(&path).await {
                continue;
            }
            return Self::load_from_path(path).await;
        }

        Err(BehaviorConfigError::NotFound {
            behavior: behavior_name.to_string(),
            dir: behaviors_dir.display().to_string(),
            tried_paths,
        })
    }

    pub async fn load_from_path(path: impl AsRef<Path>) -> Result<Self, BehaviorConfigError> {
        let path = path.as_ref().to_path_buf();
        let content =
            fs::read_to_string(&path)
                .await
                .map_err(|source| BehaviorConfigError::ReadFailed {
                    path: path.display().to_string(),
                    source,
                })?;
        Self::parse_from_str(&path, &content)
    }

    pub fn parse_from_str(path: &Path, content: &str) -> Result<Self, BehaviorConfigError> {
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());

        let mut cfg =
            match ext.as_deref() {
                Some("json") => serde_json::from_str::<BehaviorConfig>(content).map_err(|err| {
                    BehaviorConfigError::Invalid {
                        path: path.display().to_string(),
                        message: err.to_string(),
                    }
                })?,
                Some("yaml") | Some("yml") => serde_yaml::from_str::<BehaviorConfig>(content)
                    .map_err(|err| BehaviorConfigError::Invalid {
                        path: path.display().to_string(),
                        message: err.to_string(),
                    })?,
                _ => serde_json::from_str::<BehaviorConfig>(content).or_else(|json_err| {
                    serde_yaml::from_str::<BehaviorConfig>(content).map_err(|yaml_err| {
                        BehaviorConfigError::Invalid {
                            path: path.display().to_string(),
                            message: format!(
                                "json parse failed: {json_err}; yaml parse failed: {yaml_err}"
                            ),
                        }
                    })
                })?,
            };

        if cfg.name.trim().is_empty() {
            cfg.name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
        }
        if cfg.process_rule.trim().is_empty() {
            return Err(BehaviorConfigError::Invalid {
                path: path.display().to_string(),
                message: "process_rule must be provided in behavior config".to_string(),
            });
        }

        cfg.tools.normalize();
        cfg.toolbox.normalize();
        if cfg.toolbox.tools != BehaviorToolsConfig::default() {
            cfg.tools = cfg.toolbox.tools.clone();
        } else {
            cfg.toolbox.tools = cfg.tools.clone();
        }
        cfg.policy = cfg.policy.trim().to_string();
        cfg.input = cfg.input.trim().to_string();
        cfg.memory.normalize();
        cfg.llm.output_protocol = cfg.output_protocol.to_prompt_text();
        cfg.llm.output_mode = cfg.output_protocol.mode_name();

        Ok(cfg)
    }

    pub fn to_llm_behavior_config(&self) -> LLMBehaviorConfig {
        let mut cfg = self.llm.clone();
        cfg.output_mode = self.output_protocol.mode_name();
        cfg.output_protocol = self.output_protocol.to_prompt_text();
        cfg
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorMemoryConfig {
    #[serde(alias = "total_limt")]
    pub total_limit: u32,
    pub agent_memory: BehaviorMemoryBucketConfig,
    #[serde(alias = "session_summary")]
    pub session_summaries: BehaviorMemoryBucketConfig,
    pub history_messages: BehaviorMemoryBucketConfig,
    pub workspace_summary: BehaviorMemoryBucketConfig,
    pub workspace_worklog: BehaviorMemoryBucketConfig,
    pub workspace_todo: BehaviorMemoryBucketConfig,
}

impl Default for BehaviorMemoryConfig {
    fn default() -> Self {
        Self {
            total_limit: 0,
            agent_memory: BehaviorMemoryBucketConfig::default(),
            session_summaries: BehaviorMemoryBucketConfig::default(),
            history_messages: BehaviorMemoryBucketConfig::default(),
            workspace_summary: BehaviorMemoryBucketConfig::default(),
            workspace_worklog: BehaviorMemoryBucketConfig::default(),
            workspace_todo: BehaviorMemoryBucketConfig::default(),
        }
    }
}

impl BehaviorMemoryConfig {
    fn normalize(&mut self) {
        self.agent_memory.normalize();
        self.session_summaries.normalize();
        self.history_messages.normalize();
        self.workspace_summary.normalize();
        self.workspace_worklog.normalize();
        self.workspace_todo.normalize();
    }

    pub fn is_empty(&self) -> bool {
        self.total_limit == 0
            && self.agent_memory.is_empty()
            && self.session_summaries.is_empty()
            && self.history_messages.is_empty()
            && self.workspace_summary.is_empty()
            && self.workspace_worklog.is_empty()
            && self.workspace_todo.is_empty()
    }

    pub fn to_json_value(&self) -> Json {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorMemoryBucketConfig {
    pub limit: u32,
    pub max_percent: Option<f32>,
}

impl Default for BehaviorMemoryBucketConfig {
    fn default() -> Self {
        Self {
            limit: 0,
            max_percent: None,
        }
    }
}

impl BehaviorMemoryBucketConfig {
    fn normalize(&mut self) {
        self.max_percent = self.max_percent.filter(|value| {
            let v = *value;
            v.is_finite() && v > 0.0 && v <= 1.0
        });
    }

    fn is_empty(&self) -> bool {
        self.limit == 0 && self.max_percent.is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BehaviorToolboxConfig {
    pub tools: BehaviorToolsConfig,
    pub skills: Vec<String>,
}

impl Default for BehaviorToolboxConfig {
    fn default() -> Self {
        Self {
            tools: BehaviorToolsConfig::default(),
            skills: vec![],
        }
    }
}

impl BehaviorToolboxConfig {
    fn normalize(&mut self) {
        self.tools.normalize();
        let mut uniq = HashSet::<String>::new();
        let mut normalized = Vec::<String>::new();
        for skill in &self.skills {
            let trimmed = skill.trim();
            if trimmed.is_empty() {
                continue;
            }
            if uniq.insert(trimmed.to_string()) {
                normalized.push(trimmed.to_string());
            }
        }
        self.skills = normalized;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorToolMode {
    All,
    AllowList,
    None,
}

impl Default for BehaviorToolMode {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BehaviorToolsConfig {
    pub mode: BehaviorToolMode,
    pub names: Vec<String>,
}

impl Default for BehaviorToolsConfig {
    fn default() -> Self {
        Self {
            mode: BehaviorToolMode::All,
            names: vec![],
        }
    }
}

impl BehaviorToolsConfig {
    pub fn filter_tool_specs(&self, specs: &[ToolSpec]) -> Vec<ToolSpec> {
        match self.mode {
            BehaviorToolMode::All => specs.to_vec(),
            BehaviorToolMode::None => vec![],
            BehaviorToolMode::AllowList => {
                let allow = self
                    .names
                    .iter()
                    .map(|name| name.trim())
                    .filter(|name| !name.is_empty())
                    .collect::<HashSet<_>>();
                specs
                    .iter()
                    .filter(|spec| allow.contains(spec.name.as_str()))
                    .cloned()
                    .collect()
            }
        }
    }

    pub fn filter_ai_tool_specs(&self, specs: &[AiToolSpec]) -> Vec<AiToolSpec> {
        match self.mode {
            BehaviorToolMode::All => specs.to_vec(),
            BehaviorToolMode::None => vec![],
            BehaviorToolMode::AllowList => {
                let allow = self
                    .names
                    .iter()
                    .map(|name| name.trim())
                    .filter(|name| !name.is_empty())
                    .collect::<HashSet<_>>();
                specs
                    .iter()
                    .filter(|spec| allow.contains(spec.name.as_str()))
                    .cloned()
                    .collect()
            }
        }
    }

    fn normalize(&mut self) {
        let mut uniq = HashSet::<String>::new();
        let mut normalized = Vec::<String>::new();
        for name in &self.names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            if uniq.insert(trimmed.to_string()) {
                normalized.push(trimmed.to_string());
            }
        }
        self.names = normalized;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum BehaviorOutputProtocol {
    Text(String),
    Structured(BehaviorOutputProtocolStructured),
}

impl Default for BehaviorOutputProtocol {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl BehaviorOutputProtocol {
    pub fn to_prompt_text(&self) -> String {
        let mode = self.mode_name();
        match self {
            BehaviorOutputProtocol::Text(text) => text.trim().to_string(),
            BehaviorOutputProtocol::Structured(spec) => {
                if let Some(text) = &spec.text {
                    if !text.trim().is_empty() {
                        return text.clone();
                    }
                }
                spec.schema_hint
                    .as_ref()
                    .map(|schema_hint| schema_hint.to_string())
                    .unwrap_or_else(|| default_output_protocol_text(mode.as_str()))
            }
        }
    }

    pub fn mode_name(&self) -> String {
        match self {
            BehaviorOutputProtocol::Text(_) => "auto".to_string(),
            BehaviorOutputProtocol::Structured(spec) => normalize_output_mode(spec.mode.as_str()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorOutputProtocolStructured {
    pub mode: String,
    pub text: Option<String>,
    pub schema_hint: Option<Json>,
}

impl Default for BehaviorOutputProtocolStructured {
    fn default() -> Self {
        Self {
            mode: "json_v1".to_string(),
            text: None,
            schema_hint: None,
        }
    }
}

fn normalize_output_mode(mode: &str) -> String {
    let normalized = mode.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "auto" => "auto".to_string(),
        "json_v1" | "behavior_llm_result" | "behavior_result" | "executor" => {
            "behavior_llm_result".to_string()
        }
        "route_result" | "route" | "route_v1" => "behavior_llm_result".to_string(),
        _ => "auto".to_string(),
    }
}

fn default_output_protocol_text(mode: &str) -> String {
    match mode {
        "behavior_llm_result" => "Return ONLY a JSON object that follows BehaviorLLMResult fields (next_behavior, reply, todo, set_memory, actions, session_delta).".to_string(),
        _ => String::new(),
    }
}

fn candidate_paths_for_behavior(behaviors_dir: &Path, behavior_name: &str) -> Vec<PathBuf> {
    let requested = behaviors_dir.join(behavior_name);
    if requested.extension().is_some() {
        return vec![requested];
    }

    let mut paths = Vec::with_capacity(BEHAVIOR_CONFIG_EXTENSIONS.len());
    for ext in BEHAVIOR_CONFIG_EXTENSIONS {
        paths.push(behaviors_dir.join(format!("{behavior_name}.{ext}")));
    }
    paths
}

async fn is_file(path: &Path) -> bool {
    fs::metadata(path)
        .await
        .map(|meta| meta.is_file())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_config_yaml_requires_process_rule() {
        let path = Path::new("on_wakeup.yaml");
        let err = BehaviorConfig::parse_from_str(
            path,
            r#"
tools:
  mode: allow_list
  names:
    - exec_bash
"#,
        )
        .expect_err("process_rule should be required");

        let msg = err.to_string();
        assert!(msg.contains("process_rule"));
    }

    #[test]
    fn behavior_config_yaml_parses_process_rule_and_allowlist() {
        let path = Path::new("on_wakeup.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: test_rule
policy: safe_only
input: |
  {{new_msg}}
memory:
  total_limit: 12000
  history_messages:
    limit: 3000
    max_percent: 0.3
step_limit: 6
tools:
  mode: allow_list
  names:
    - exec_bash
toolbox:
  skills:
    - plan
    - plan
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(cfg.name, "on_wakeup");
        assert_eq!(cfg.process_rule, "test_rule");
        assert_eq!(cfg.policy, "safe_only");
        assert_eq!(cfg.input, "{{new_msg}}");
        assert_eq!(cfg.step_limit, 6);
        assert_eq!(cfg.memory.total_limit, 12_000);
        assert_eq!(cfg.memory.history_messages.limit, 3_000);
        assert_eq!(cfg.memory.history_messages.max_percent, Some(0.3));
        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["exec_bash".to_string()]);
        assert_eq!(cfg.toolbox.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.toolbox.skills, vec!["plan".to_string()]);
        assert_eq!(cfg.llm.process_name, "opendan-llm-behavior");
        assert_eq!(cfg.limits.max_prompt_tokens, 12_000);
        assert!(cfg.llm.force_json);
        assert_eq!(cfg.llm.output_mode, "auto");
        assert!(cfg.llm.output_protocol.trim().is_empty());
    }

    #[test]
    fn behavior_config_output_protocol_object_can_override_text() {
        let path = Path::new("on_msg.yml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: test_rule
output_protocol:
  mode: route_result
  text: protocol_from_cfg
llm:
  process_name: custom-process
  model_policy:
    preferred: fast-model
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.name, "on_msg");
        assert_eq!(cfg.llm.output_protocol, "protocol_from_cfg".to_string());
        assert_eq!(cfg.llm.output_mode, "behavior_llm_result");
        assert_eq!(cfg.llm.process_name, "custom-process");
        assert_eq!(cfg.llm.model_policy.preferred, "fast-model");
        assert_eq!(cfg.llm.model_policy.temperature, 0.2);
    }

    #[test]
    fn behavior_config_toolbox_tools_override_legacy_tools_field() {
        let path = Path::new("route.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: route_rule
tools:
  mode: all
toolbox:
  tools:
    mode: allow_list
    names:
      - load_memory
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["load_memory".to_string()]);
        assert_eq!(cfg.toolbox.tools.mode, BehaviorToolMode::AllowList);
    }

    #[test]
    fn behavior_config_memory_supports_total_limt_alias_and_normalizes_percent() {
        let path = Path::new("memory.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: memory_rule
memory:
  total_limt: 1024
  session_summaries:
    limit: 256
    max_percent: 1.5
  history_messages:
    max_percent: 0.4
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.memory.total_limit, 1024);
        assert_eq!(cfg.memory.session_summaries.limit, 256);
        assert_eq!(cfg.memory.session_summaries.max_percent, None);
        assert_eq!(cfg.memory.history_messages.max_percent, Some(0.4));
    }

    #[test]
    fn behavior_allowlist_does_not_apply_legacy_module_prefixed_name() {
        let tools = BehaviorToolsConfig {
            mode: BehaviorToolMode::AllowList,
            names: vec!["exec_bash".to_string()],
        };
        let specs = vec![
            ToolSpec {
                name: "exec_bash".to_string(),
                description: "bash".to_string(),
                args_schema: serde_json::json!({"type":"object"}),
                output_schema: serde_json::json!({"type":"object"}),
            },
            ToolSpec {
                name: "load_memory".to_string(),
                description: "memory".to_string(),
                args_schema: serde_json::json!({"type":"object"}),
                output_schema: serde_json::json!({"type":"object"}),
            },
        ];

        let filtered = tools.filter_tool_specs(&specs);
        assert_eq!(filtered.len(), 1);
    }
}
