use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
    pub output_protocol: BehaviorOutputProtocol,
    pub tools: BehaviorToolsConfig,
    pub llm: LLMBehaviorConfig,
    pub limits: StepLimits,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            process_rule: String::new(),
            output_protocol: BehaviorOutputProtocol::default(),
            tools: BehaviorToolsConfig::default(),
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
        cfg.llm.output_protocol = cfg.output_protocol.to_prompt_text();

        Ok(cfg)
    }

    pub fn to_llm_behavior_config(&self) -> LLMBehaviorConfig {
        let mut cfg = self.llm.clone();
        cfg.output_protocol = self.output_protocol.to_prompt_text();
        cfg
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
                    .unwrap_or_default()
            }
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
tools:
  mode: allow_list
  names:
    - exec_bash
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(cfg.name, "on_wakeup");
        assert_eq!(cfg.process_rule, "test_rule");
        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["exec_bash".to_string()]);
        assert_eq!(cfg.llm.process_name, "opendan-llm-behavior");
        assert_eq!(cfg.limits.max_prompt_tokens, 12_000);
        assert!(cfg.llm.force_json);
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
  mode: json_v1
  text: protocol_from_cfg
llm:
  process_name: custom-process
  model_policy:
    preferred: fast-model
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.name, "on_msg");
        assert_eq!(
            cfg.llm.output_protocol,
            "protocol_from_cfg".to_string()
        );
        assert_eq!(cfg.llm.process_name, "custom-process");
        assert_eq!(cfg.llm.model_policy.preferred, "fast-model");
        assert_eq!(cfg.llm.model_policy.temperature, 0.2);
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
