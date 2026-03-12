use std::collections::HashSet;
use std::path::{Path, PathBuf};

use buckyos_api::AiToolSpec;
use buckyos_kit::ConfigMerger;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tokio::fs;
use uuid::Uuid;

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
    pub step_summary: String,
    pub output_protocol: BehaviorOutputProtocol,
    pub tools: BehaviorToolsConfig,
    pub toolbox: BehaviorToolboxConfig,

    // 如果因为系统原因失败了（比如 step_limit),切换到哪个 behavior，不设置时切回 session 默认 behavior
    #[serde(alias = "fallto", alias = "failed_back", alias = "fallback_behavior")]
    pub faild_back: Option<String>,
    pub step_limit: u32,
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
            step_summary: String::new(),
            faild_back: None,
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
    pub async fn load_from_roots(
        behavior_roots: &[PathBuf],
        behavior_name: &str,
    ) -> Result<Self, BehaviorConfigError> {
        let behavior_name = behavior_name.trim();
        if behavior_name.is_empty() {
            return Err(BehaviorConfigError::EmptyBehaviorName);
        }

        let mut tried_paths = Vec::new();
        let mut sources = Vec::new();
        for root in behavior_roots.iter().rev() {
            let mut selected = None::<PathBuf>;
            for path in candidate_paths_for_behavior(root, behavior_name) {
                tried_paths.push(path.display().to_string());
                if is_file(&path).await || is_dir(&path).await {
                    selected = Some(path);
                    break;
                }
            }

            if let Some(path) = selected {
                sources.push(path);
            }
        }

        if sources.is_empty() {
            let dir = behavior_roots
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            return Err(BehaviorConfigError::NotFound {
                behavior: behavior_name.to_string(),
                dir,
                tried_paths,
            });
        }

        if sources.len() == 1 && is_file(&sources[0]).await {
            return Self::load_from_path(&sources[0]).await;
        }

        Self::load_from_sources(behavior_name, &sources).await
    }

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
            if is_file(&path).await {
                return Self::load_from_path(path).await;
            }
            if !is_dir(&path).await {
                continue;
            }
            return Self::load_from_sources(behavior_name, &[path]).await;
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

        Self::normalize_loaded_config(path, &mut cfg)?;
        Ok(cfg)
    }

    pub fn parse_from_value(path: &Path, value: Json) -> Result<Self, BehaviorConfigError> {
        let mut cfg =
            serde_json::from_value::<BehaviorConfig>(value).map_err(|err| {
                BehaviorConfigError::Invalid {
                    path: path.display().to_string(),
                    message: err.to_string(),
                }
            })?;

        Self::normalize_loaded_config(path, &mut cfg)?;
        Ok(cfg)
    }

    fn normalize_loaded_config(
        path: &Path,
        cfg: &mut BehaviorConfig,
    ) -> Result<(), BehaviorConfigError> {
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
        let resolved_tools = cfg.toolbox.resolve_tools(&cfg.tools);
        cfg.tools = resolved_tools.clone();
        cfg.toolbox.tools = resolved_tools;
        cfg.policy = cfg.policy.trim().to_string();
        cfg.input = cfg.input.trim().to_string();
        cfg.faild_back = cfg
            .faild_back
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        cfg.memory.normalize();
        cfg.llm.output_protocol = cfg.output_protocol.to_prompt_text();
        cfg.llm.output_mode = cfg.output_protocol.mode_name();

        Ok(())
    }

    async fn load_from_sources(
        behavior_name: &str,
        sources: &[PathBuf],
    ) -> Result<Self, BehaviorConfigError> {
        let merge_root = std::env::temp_dir().join(format!(
            "opendan-behavior-merge-{}-{}",
            sanitize_merge_name(behavior_name),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&merge_root)
            .await
            .map_err(|source| BehaviorConfigError::ReadFailed {
                path: merge_root.display().to_string(),
                source,
            })?;

        let merge_result = async {
            let mut root_toml = String::new();
            for (idx, source) in sources.iter().enumerate() {
                let item_name = source_item_name(idx, source);
                let target = merge_root.join(&item_name);
                copy_merge_source(source, &target).await?;
                root_toml.push_str("[[includes]]\n");
                root_toml.push_str(&format!("path = \"{}\"\n\n", item_name));
            }

            let root_file = merge_root.join("root.toml");
            fs::write(&root_file, root_toml)
                .await
                .map_err(|source| BehaviorConfigError::ReadFailed {
                    path: root_file.display().to_string(),
                    source,
                })?;

            let merged = ConfigMerger::load_dir(&merge_root).await.map_err(|err| {
                BehaviorConfigError::Invalid {
                    path: merge_root.display().to_string(),
                    message: err.to_string(),
                }
            })?;
            Self::parse_from_value(&merge_root.join(format!("{behavior_name}.merged.json")), merged)
        }
        .await;

        let _ = fs::remove_dir_all(&merge_root).await;
        merge_result
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
    pub workspace_summary: BehaviorMemoryBucketConfig,

    pub history_messages: BehaviorMemoryBucketConfig,
    pub workspace_worklog: BehaviorMemoryBucketConfig,

    pub session_summaries: BehaviorMemoryBucketConfig,

    pub first_prompt: Option<String>,
    pub last_prompt: Option<String>,
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
            first_prompt: None,
            last_prompt: None,
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
        self.first_prompt = Self::normalize_optional_text(self.first_prompt.take());
        self.last_prompt = Self::normalize_optional_text(self.last_prompt.take());
    }

    pub fn is_empty(&self) -> bool {
        self.total_limit == 0
            && self.agent_memory.is_empty()
            && self.session_summaries.is_empty()
            && self.history_messages.is_empty()
            && self.workspace_summary.is_empty()
            && self.workspace_worklog.is_empty()
            && self
                .first_prompt
                .as_ref()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
            && self
                .last_prompt
                .as_ref()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
    }

    pub fn to_json_value(&self) -> Json {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }

    fn normalize_optional_text(value: Option<String>) -> Option<String> {
        value
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorMemoryBucketConfig {
    pub limit: u32,
    pub max_percent: Option<f32>,
    pub is_enable: bool,
}

impl Default for BehaviorMemoryBucketConfig {
    fn default() -> Self {
        Self {
            limit: 0,
            max_percent: None,
            is_enable: false,
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
    pub mode: BehaviorToolboxMode,
    pub tools: BehaviorToolsConfig,
    pub skills: Vec<String>,
    pub load_skills: Vec<String>,
    #[serde(alias = "allow_tools")]
    pub loaded_tools: Vec<String>,
    pub default_load_actions: Vec<String>,
}

impl Default for BehaviorToolboxConfig {
    fn default() -> Self {
        Self {
            mode: BehaviorToolboxMode::default(),
            tools: BehaviorToolsConfig::default(),
            skills: vec![],
            load_skills: vec![],
            loaded_tools: vec![],
            default_load_actions: vec![],
        }
    }
}

impl BehaviorToolboxConfig {
    fn normalize(&mut self) {
        self.tools.normalize();
        Self::normalize_string_list(&mut self.skills);
        Self::normalize_string_list(&mut self.load_skills);
        Self::normalize_string_list(&mut self.loaded_tools);
        Self::normalize_string_list(&mut self.default_load_actions);
        if self.mode == BehaviorToolboxMode::None {
            self.tools.mode = BehaviorToolMode::None;
            self.tools.names.clear();
        }
    }

    pub fn is_none_mode(&self) -> bool {
        self.mode == BehaviorToolboxMode::None
    }

    pub fn effective_skills(&self) -> Vec<String> {
        match self.mode {
            BehaviorToolboxMode::None => vec![],
            BehaviorToolboxMode::Alone => self.skills.clone(),
            BehaviorToolboxMode::Inherit => Self::merge_unique(&self.load_skills, &self.skills),
        }
    }

    pub fn resolve_tools(&self, legacy_tools: &BehaviorToolsConfig) -> BehaviorToolsConfig {
        if self.mode == BehaviorToolboxMode::None {
            return BehaviorToolsConfig {
                mode: BehaviorToolMode::None,
                names: vec![],
            };
        }
        if self.tools != BehaviorToolsConfig::default() {
            return self.tools.clone();
        }

        match self.mode {
            BehaviorToolboxMode::None => BehaviorToolsConfig {
                mode: BehaviorToolMode::None,
                names: vec![],
            },
            BehaviorToolboxMode::Inherit => legacy_tools.clone(),
            BehaviorToolboxMode::Alone => {
                if self.loaded_tools.is_empty() {
                    return BehaviorToolsConfig {
                        mode: BehaviorToolMode::None,
                        names: vec![],
                    };
                }
                BehaviorToolsConfig {
                    mode: BehaviorToolMode::AllowList,
                    names: self.loaded_tools.clone(),
                }
            }
        }
    }

    fn normalize_string_list(values: &mut Vec<String>) {
        let mut uniq = HashSet::<String>::new();
        let mut normalized = Vec::<String>::new();
        for value in values.iter() {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if uniq.insert(trimmed.to_string()) {
                normalized.push(trimmed.to_string());
            }
        }
        *values = normalized;
    }

    fn merge_unique(primary: &[String], secondary: &[String]) -> Vec<String> {
        let mut out = Vec::<String>::new();
        let mut uniq = HashSet::<String>::new();

        for value in primary.iter().chain(secondary.iter()) {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if uniq.insert(trimmed.to_string()) {
                out.push(trimmed.to_string());
            }
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorToolboxMode {
    Inherit,
    Alone,
    None,
}

impl Default for BehaviorToolboxMode {
    fn default() -> Self {
        Self::Inherit
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
        Self::Structured(BehaviorOutputProtocolStructured::default())
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
            mode: "behavior_llm_result".to_string(),
            text: None,
            schema_hint: None,
        }
    }
}

fn normalize_output_mode(mode: &str) -> String {
    let normalized = mode.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "auto" => "auto".to_string(),
        "behavior_llm_result" | "behavior_result" => "behavior_llm_result".to_string(),
        "route_result" | "route" | "route_v1" => "route_result".to_string(),
        "behavior_llm_no_action_result" | "behavior_llm_bash_result" | "behavior_llm_no_action" => {
            "behavior_llm_no_action_result".to_string()
        }
        _ => "auto".to_string(),
    }
}

fn default_output_protocol_text(mode: &str) -> String {
    match mode {
        "route_result" => build_route_result_protocol(),
        "behavior_llm_result" => build_behavior_llm_result_protocol(),
        "behavior_llm_no_action_result" => behavior_llm_no_action_result(),
        _ => String::new(),
    }
}

fn behavior_llm_no_action_result() -> String {
    r#"The response MUST be valid XML only. Do not output JSON.
Use this schema (omit unused nodes):
```xml
<response>
  <next_behavior>...</next_behavior>
  <thinking>...</thinking>
  <reply>...</reply>
  <shell_commands>
    <![CDATA[
      ls -l
      cat readme.txt
      echo "Hello, world!" > readme.txt
    ]]>
  </shell_commands>
</response>
```

## shell_commands
- Put one shell command per line inside `shell_commands` CDATA.
- Commands run sequentially in a session-bound bash environment; execution stops on first failure.
- Results persist in step_summary for the next step. MUST limit read output size to avoid context overflow.
- Common CLI tools and process_rule-declared tools are pre-installed. NEVER check availability before calling."#
        .to_string()
}

fn build_behavior_llm_result_protocol() -> String {
    r#"The response MUST be valid XML only. Do not output JSON.
Use this schema (omit unused nodes):
```xml
<response>
  <next_behavior>...</next_behavior>
  <thinking>...</thinking>
  <reply>...</reply>
  <shell_commands>
    <![CDATA[
        ls -l
        cat readme.txt
        echo "Hello, world!" > readme.txt
    ]]>
  </shell_commands>
  <actions mode="failed_end|all">
    <command>...</command>
    <exec name="write_file" path="readme.txt" mode="write">
    <![CDATA[
        file content
    ]]>
    </exec>
  </actions>
</response>
```
All nodes are optional—NEVER include unused nodes.

## actions
- Commands run sequentially in a session-bound bash env. On failure: `failed_end` stops, `all` continues.
- `<command>` means shell command.
- `<exec name="...">` means structured cmd_action; XML attributes are action args.
- `shell_commands` runs before `actions` commands (shell first, then actions). NEVER put structured actions in `shell_commands`.
- MUST use write_file / edit_file cmd_action for writing text files. NEVER use shell commands (echo/cat) to write files.
- Results persist in step_summary for the next step. MUST limit read output size to avoid context overflow.
- Common CLI tools and process_rule-declared tools are pre-installed. NEVER check availability before calling.

### write_file
```xml
<exec name="write_file" path="notes.txt" mode="write">
<![CDATA[
hello
]]>
</exec>
```

### edit_file
```xml
<exec name="edit_file" path="notes.txt" pos_chunk="hello" mode="replace">
<![CDATA[
hello world
]]>
</exec>
```

NEVER include `session_id` or `new_session` in this mode."#
        .to_string()
}

fn build_route_result_protocol() -> String {
    r#"The response MUST be valid XML only. Do not output JSON.
Use this schema (omit unused nodes):
```xml
<response>
  <reply>...</reply>
  <set_memory>
    <item key="user/name">Alice</item>
    <item key="project/stack">React + Go</item>
  </set_memory>
  <topic_tags>
    <tag>travel</tag>
    <tag>tokyo</tag>
  </topic_tags>
  <route_session_id>...</route_session_id>
  <new_session>
    <title>...</title>
    <summary>...</summary>
  </new_session>
</response>
```

All nodes are optional—NEVER include unused nodes.
If `<reply>` content is long or contains special characters (e.g. `<`, `>`, `&`), wrap it in a CDATA section: `<reply><![CDATA[...long content...]]></reply>`.

**set_memory**: A persistent notebook organized by path keys.
Write when the user reveals info worth remembering long-term.
Paths use "/" hierarchy, e.g. `user/name`, `project/stack`.
Set value to empty string to delete.

**topic_tags**: 0-5 short labels for this conversation (for later retrieval).

**Routing**: MUST provide exactly one of `route_session_id` or `new_session`. NEVER both. Both MUST follow process rules."#
            .to_string()
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

async fn is_dir(path: &Path) -> bool {
    fs::metadata(path)
        .await
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
}

async fn copy_merge_source(source: &Path, target: &Path) -> Result<(), BehaviorConfigError> {
    if is_file(source).await {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source_err| BehaviorConfigError::ReadFailed {
                    path: parent.display().to_string(),
                    source: source_err,
                })?;
        }
        fs::copy(source, target)
            .await
            .map_err(|source_err| BehaviorConfigError::ReadFailed {
                path: source.display().to_string(),
                source: source_err,
            })?;
        return Ok(());
    }

    fs::create_dir_all(target)
        .await
        .map_err(|source_err| BehaviorConfigError::ReadFailed {
            path: target.display().to_string(),
            source: source_err,
        })?;

    let mut read_dir = fs::read_dir(source)
        .await
        .map_err(|source_err| BehaviorConfigError::ReadFailed {
            path: source.display().to_string(),
            source: source_err,
        })?;
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|source_err| BehaviorConfigError::ReadFailed {
            path: source.display().to_string(),
            source: source_err,
        })?
    {
        let child_source = entry.path();
        let child_target = target.join(entry.file_name());
        Box::pin(copy_merge_source(&child_source, &child_target)).await?;
    }

    Ok(())
}

fn source_item_name(idx: usize, source: &Path) -> String {
    let suffix = source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    format!("source.{}{}", idx + 1, suffix)
}

fn sanitize_merge_name(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

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
    fn behavior_config_parses_faild_back_and_legacy_fallto() {
        let path = Path::new("route.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: test_rule
faild_back: plan
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(cfg.faild_back.as_deref(), Some("plan"));

        let legacy_cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: test_rule
fallto: do
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(legacy_cfg.faild_back.as_deref(), Some("do"));
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
        assert_eq!(cfg.llm.output_mode, "route_result");
        assert_eq!(cfg.llm.process_name, "custom-process");
        assert_eq!(cfg.llm.model_policy.preferred, "fast-model");
        assert_eq!(cfg.llm.model_policy.temperature, 0.2);
    }

    #[test]
    fn route_result_protocol_includes_session_routing_keys() {
        let protocol = build_route_result_protocol();
        assert!(protocol.contains("route_session_id"));
        assert!(protocol.contains("new_session"));
        assert!(protocol.contains("MUST provide exactly one of"));
    }

    #[test]
    fn behavior_llm_result_protocol_disallows_session_routing_keys() {
        let protocol = build_behavior_llm_result_protocol();
        assert!(!protocol.contains("route_session_id"));
        assert!(protocol.contains("NEVER include `session_id` or `new_session` in this mode."));
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
  first_prompt: "  memory header text  "
  last_prompt: "   "
  session_summaries:
    limit: 256
    max_percent: 1.5
  history_messages:
    max_percent: 0.4
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.memory.total_limit, 1024);
        assert_eq!(
            cfg.memory.first_prompt.as_deref(),
            Some("memory header text")
        );
        assert_eq!(cfg.memory.last_prompt, None);
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
                usage: None,
            },
            ToolSpec {
                name: "load_memory".to_string(),
                description: "memory".to_string(),
                args_schema: serde_json::json!({"type":"object"}),
                output_schema: serde_json::json!({"type":"object"}),
                usage: None,
            },
        ];

        let filtered = tools.filter_tool_specs(&specs);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn behavior_config_toolbox_none_mode_disables_tools_and_skills() {
        let path = Path::new("route.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: route_rule
tools:
  mode: allow_list
  names:
    - exec_bash
toolbox:
  mode: none
  skills:
    - coding/rust
  default_load_skills:
    - buildin
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.tools.mode, BehaviorToolMode::None);
        assert!(cfg.tools.names.is_empty());
        assert_eq!(cfg.toolbox.tools.mode, BehaviorToolMode::None);
        assert!(cfg.toolbox.effective_skills().is_empty());
    }

    #[test]
    fn behavior_config_toolbox_alone_mode_uses_default_allow_functions_alias() {
        let path = Path::new("do.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: do_rule
tools:
  mode: all
toolbox:
  mode: alone
  skills:
    - coding/rust
    - coding/rust
  default_allow_functions:
    - read_file
    - read_file
    - bash
  default_load_skills:
    - buildin
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(
            cfg.tools.names,
            vec!["read_file".to_string(), "bash".to_string()]
        );
        assert_eq!(cfg.toolbox.loaded_tools, cfg.tools.names);
        assert_eq!(
            cfg.toolbox.effective_skills(),
            vec!["coding/rust".to_string()]
        );
    }

    #[test]
    fn behavior_config_toolbox_inherit_mode_merges_default_and_behavior_skills() {
        let path = Path::new("plan.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
process_rule: plan_rule
tools:
  mode: allow_list
  names:
    - exec_bash
toolbox:
  default_load_skills:
    - buildin
    - buildin
  skills:
    - coding/rust
    - buildin
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["exec_bash".to_string()]);
        assert_eq!(
            cfg.toolbox.effective_skills(),
            vec!["buildin".to_string(), "coding/rust".to_string()]
        );
    }

    #[tokio::test]
    async fn behavior_config_load_from_roots_merges_package_and_env_sources() {
        let temp = tempdir().expect("create temp dir");
        let package_root = temp.path().join("package");
        let env_root = temp.path().join("env");
        fs::create_dir_all(&package_root)
            .await
            .expect("create package dir");
        fs::create_dir_all(&env_root).await.expect("create env dir");

        fs::write(
            package_root.join("plan.yaml"),
            r#"
process_rule: package_rule
tools:
  mode: allow_list
  names:
    - exec_bash
toolbox:
  default_load_skills:
    - buildin
"#,
        )
        .await
        .expect("write package behavior");
        fs::write(
            env_root.join("plan.yaml"),
            r#"
process_rule: env_rule
toolbox:
  skills:
    - coding/rust
"#,
        )
        .await
        .expect("write env behavior");

        let cfg = BehaviorConfig::load_from_roots(
            &[env_root.clone(), package_root.clone()],
            "plan",
        )
        .await
        .expect("merge behavior config");

        assert_eq!(cfg.process_rule, "env_rule");
        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["exec_bash".to_string()]);
        assert_eq!(cfg.toolbox.skills, vec!["coding/rust".to_string()]);
    }
}
