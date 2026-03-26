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
    pub system: String,
    pub tools: BehaviorToolsConfig,
    pub skills: BehaviorSkillsConfig,
    pub memory: BehaviorMemoryConfig,
    pub input: String,
    pub output_protocol: BehaviorOutputProtocol,

    // 如果因为系统原因失败了（比如 step_limit),切换到哪个 behavior，不设置时切回 session 默认 behavior
    pub faild_back: Option<String>,
    pub step_limit: u32,
    pub llm: LLMBehaviorConfig,
    pub limits: StepLimits,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            system: String::new(),
            input: String::new(),
            memory: BehaviorMemoryConfig::default(),
            faild_back: None,
            step_limit: 0,
            output_protocol: BehaviorOutputProtocol::default(),
            tools: BehaviorToolsConfig::default(),
            skills: BehaviorSkillsConfig::default(),
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
        let mut cfg = serde_json::from_value::<BehaviorConfig>(value).map_err(|err| {
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
        cfg.system = cfg.system.trim().to_string();
        if cfg.system.is_empty() {
            return Err(BehaviorConfigError::Invalid {
                path: path.display().to_string(),
                message: "system must be provided in behavior config".to_string(),
            });
        }

        cfg.tools.normalize();
        cfg.skills.normalize();
        cfg.input = cfg.input.trim().to_string();
        cfg.output_protocol.normalize();
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
        fs::create_dir_all(&merge_root).await.map_err(|source| {
            BehaviorConfigError::ReadFailed {
                path: merge_root.display().to_string(),
                source,
            }
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
            fs::write(&root_file, root_toml).await.map_err(|source| {
                BehaviorConfigError::ReadFailed {
                    path: root_file.display().to_string(),
                    source,
                }
            })?;

            let merged = ConfigMerger::load_dir(&merge_root).await.map_err(|err| {
                BehaviorConfigError::Invalid {
                    path: merge_root.display().to_string(),
                    message: err.to_string(),
                }
            })?;
            Self::parse_from_value(
                &merge_root.join(format!("{behavior_name}.merged.json")),
                merged,
            )
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
    pub total_limit: u32,
    pub agent_memory: BehaviorMemoryBucketConfig,
    pub history_messages: BehaviorMemoryBucketConfig,
    pub session_step_records: BehaviorMemoryBucketConfig,
    pub workspace_worklog: BehaviorMemoryBucketConfig,

    pub first_prompt: Option<String>,
    pub last_prompt: Option<String>,
}

impl Default for BehaviorMemoryConfig {
    fn default() -> Self {
        Self {
            total_limit: 0,
            agent_memory: BehaviorMemoryBucketConfig::default(),
            history_messages: BehaviorMemoryBucketConfig::default(),
            session_step_records: BehaviorMemoryBucketConfig::default(),
            workspace_worklog: BehaviorMemoryBucketConfig::default(),
            first_prompt: None,
            last_prompt: None,
        }
    }
}

impl BehaviorMemoryConfig {
    fn normalize(&mut self) {
        self.agent_memory.normalize();
        self.history_messages.normalize();
        self.session_step_records.normalize();
        self.workspace_worklog.normalize();
        self.first_prompt = Self::normalize_optional_text(self.first_prompt.take());
        self.last_prompt = Self::normalize_optional_text(self.last_prompt.take());
    }

    pub fn is_empty(&self) -> bool {
        self.total_limit == 0
            && self.agent_memory.is_empty()
            && self.history_messages.is_empty()
            && self.session_step_records.is_empty()
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
    pub skip_last_n: Option<u32>,
}

impl Default for BehaviorMemoryBucketConfig {
    fn default() -> Self {
        Self {
            limit: 0,
            max_percent: None,
            is_enable: false,
            skip_last_n: None,
        }
    }
}

impl BehaviorMemoryBucketConfig {
    fn normalize(&mut self) {
        self.max_percent = self.max_percent.filter(|value| {
            let v = *value;
            v.is_finite() && v > 0.0 && v <= 1.0
        });
        self.skip_last_n = self.skip_last_n.filter(|value| *value > 0);
    }

    fn is_empty(&self) -> bool {
        self.limit == 0 && self.max_percent.is_none()
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
        normalize_unique_string_list(&mut self.names);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorSkillMode {
    Union,
    SessionOnly,
    BehaviorOnly,
}

impl Default for BehaviorSkillMode {
    fn default() -> Self {
        Self::Union
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BehaviorSkillsConfig {
    pub mode: BehaviorSkillMode,
    pub load_skills: Vec<String>,
}

impl Default for BehaviorSkillsConfig {
    fn default() -> Self {
        Self {
            mode: BehaviorSkillMode::Union,
            load_skills: vec![],
        }
    }
}

impl BehaviorSkillsConfig {
    fn normalize(&mut self) {
        normalize_unique_string_list(&mut self.load_skills);
    }
}

fn normalize_unique_string_list(values: &mut Vec<String>) {
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum BehaviorOutputProtocol {
    Text(String),
    Structured(BehaviorOutputProtocolStructured),
    None,
}

impl Default for BehaviorOutputProtocol {
    fn default() -> Self {
        Self::None
    }
}

impl BehaviorOutputProtocol {
    pub fn normalize(&mut self) {
        let normalized = match self {
            BehaviorOutputProtocol::Text(text) => {
                let trimmed = text.trim();
                if trimmed.eq_ignore_ascii_case("none") {
                    BehaviorOutputProtocol::None
                } else {
                    BehaviorOutputProtocol::Text(trimmed.to_string())
                }
            }
            BehaviorOutputProtocol::Structured(spec) => {
                spec.mode = normalize_output_mode(spec.mode.as_str());
                spec.text = spec
                    .text
                    .take()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());

                if spec.mode == "none" {
                    BehaviorOutputProtocol::None
                } else {
                    BehaviorOutputProtocol::Structured(spec.clone())
                }
            }
            BehaviorOutputProtocol::None => BehaviorOutputProtocol::None,
        };
        *self = normalized;
    }

    pub fn is_disabled(&self) -> bool {
        matches!(self, BehaviorOutputProtocol::None)
    }

    pub fn to_prompt_text(&self) -> String {
        let mode = self.mode_name();
        match self {
            BehaviorOutputProtocol::None => String::new(),
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
            BehaviorOutputProtocol::None => "auto".to_string(),
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
        "none" => "none".to_string(),
        "behavior_llm_result" => "behavior_llm_result".to_string(),
        "route_result" => "route_result".to_string(),
        "behavior_llm_no_action_result" => "behavior_llm_no_action_result".to_string(),
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
  <conclusion>Distilled conclusion from the previous step's action result (required, even if the conclusion is "no new findings")</conclusion>
  <thinking>Reasoning based on conclusion and current task state</thinking>
  <reply>Message to the user; optional; only fill when reporting important updates</reply>
  <shell_commands>
    <![CDATA[
      ls -l
      cat readme.txt
      echo "Hello, world!" > readme.txt
    ]]>
  </shell_commands>
  <next_behavior>...</next_behavior>
</response>
```
- The conclusion MUST be self-contained—later steps may not see the raw action_result, so key data and judgments MUST be fully expressed in conclusion.

## shell_commands
- Put one shell command per line inside `shell_commands` CDATA.
- Commands run sequentially in a session-bound bash environment; execution stops on first failure.
- Results are available in `last_step` on the next step. MUST limit read output size to avoid context overflow.
- Common CLI tools and behavior-declared tools are pre-installed. NEVER check availability before calling."#
        .to_string()
}

fn build_behavior_llm_result_protocol() -> String {
    r#"The response MUST be valid XML only. Do not output JSON.
Use this schema (omit unused nodes):
```xml
<response>
  <conclusion>Distilled conclusion from the previous step's action result (required, even if the conclusion is "no new findings")</conclusion>
  <thinking>Reasoning based on conclusion and current task state</thinking>
  <reply>Message to the user; optional; only fill when reporting important updates</reply>
  <shell_commands>
    <![CDATA[
        ls -l
        cat readme.txt
        echo "Hello, world!" > readme.txt
    ]]>
  </shell_commands>
  <actions>
    <exec name="write_file" path="readme.txt" mode="write">
    <![CDATA[
        file content
    ]]>
    </exec>
  </actions>
  <next_behavior>...</next_behavior>
</response>
```
- The conclusion MUST be self-contained—later steps may not see the raw action_result, so key data and judgments MUST be fully expressed in conclusion.

## actions
- Commands run sequentially in a session-bound bash env. On failure: `failed_end` stops, `all` continues.
- `<exec name="...">` means structured cmd_action; XML attributes are action args.
- `shell_commands` runs before `actions` commands (shell first, then actions). NEVER put structured actions in `shell_commands`.
- MUST use write_file / edit_file action for writing text files. NEVER use shell commands (echo/cat) to write files.
- Common CLI tools and behavior-declared tools are pre-installed. NEVER check availability before calling.

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
```"#
        .to_string()
}

fn build_route_result_protocol() -> String {
    r#"The response MUST be valid XML only. Do not output JSON.
Use this schema (omit unused nodes):
```xml
<response>
  <thinking>...</thinking>
  <reply>...</reply>
  <set_memory>
    <item key="reminder/2026-03-25/dentist">Dentist appointment at 3pm</item>
  </set_memory>
  <topic_tags>
    <tag>travel</tag>
    <tag>tokyo</tag>
  </topic_tags>
  <work_session_id>...</work_session_id>
  <new_work_session>
    <title>...</title>
    <summary>...</summary>
  </new_work_session>
</response>
```

All nodes are optional—NEVER include unused nodes.
If `<reply>` content is long or contains special characters (e.g. `<`, `>`, `&`), wrap it in a CDATA section: `<reply><![CDATA[...long content...]]></reply>`.

**set_memory**: Long-lived key–value notes (same `key="..."` shape as the schema). Keys are slash-separated paths: `scope/topic/leaf`—usually 2–4 segments. Use short, stable segment names; avoid spaces in keys.
Examples:
- `user/alice/birthday` → `March 15`
- `preference/coding/language` → `TypeScript`
- `reminder/2026-03-25/dentist` → `Dentist appointment at 3pm`

* NEVER write temporary noise into memory.
Omit `<set_memory>` when nothing is worth storing.

**topic_tags**: 0-5 short labels for this conversation (for later retrieval).

**Routing**: MUST provide exactly one of `work_session_id` or `new_work_session`. NEVER both. Both MUST follow process rules."#
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
            fs::create_dir_all(parent).await.map_err(|source_err| {
                BehaviorConfigError::ReadFailed {
                    path: parent.display().to_string(),
                    source: source_err,
                }
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

    let mut read_dir =
        fs::read_dir(source)
            .await
            .map_err(|source_err| BehaviorConfigError::ReadFailed {
                path: source.display().to_string(),
                source: source_err,
            })?;
    while let Some(entry) =
        read_dir
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
    fn behavior_config_yaml_requires_system() {
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
        .expect_err("system should be required");

        let msg = err.to_string();
        assert!(msg.contains("system"));
    }

    #[test]
    fn behavior_config_yaml_parses_system_and_allowlist() {
        let path = Path::new("on_wakeup.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: |
  test_rule
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
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(cfg.name, "on_wakeup");
        assert_eq!(cfg.system, "test_rule");
        assert_eq!(cfg.input, "{{new_msg}}");
        assert_eq!(cfg.step_limit, 6);
        assert_eq!(cfg.memory.total_limit, 12_000);
        assert_eq!(cfg.memory.history_messages.limit, 3_000);
        assert_eq!(cfg.memory.history_messages.max_percent, Some(0.3));
        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["exec_bash".to_string()]);
        assert_eq!(cfg.llm.process_name, "opendan-llm-behavior");
        assert_eq!(cfg.limits.max_prompt_tokens, 200_000);
        assert!(cfg.llm.force_json);
        assert!(matches!(cfg.output_protocol, BehaviorOutputProtocol::None));
        assert_eq!(cfg.llm.output_mode, "auto");
        assert!(cfg.llm.output_protocol.is_empty());
    }

    #[test]
    fn behavior_config_parses_faild_back() {
        let path = Path::new("route.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: test_rule
faild_back: plan
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(cfg.faild_back.as_deref(), Some("plan"));
    }

    #[test]
    fn behavior_config_output_protocol_object_can_override_text() {
        let path = Path::new("on_msg.yml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: test_rule
output_protocol:
  mode: route_result
  text: protocol_from_cfg
llm:
  process_name: custom-process
  must_features:
    - web_search
  model_policy:
    preferred: fast-model
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.name, "on_msg");
        assert_eq!(cfg.llm.output_protocol, "protocol_from_cfg".to_string());
        assert_eq!(cfg.llm.output_mode, "route_result");
        assert_eq!(cfg.llm.process_name, "custom-process");
        assert_eq!(cfg.llm.must_features, vec!["web_search".to_string()]);
        assert_eq!(cfg.llm.model_policy.preferred, "fast-model");
        assert_eq!(cfg.llm.model_policy.temperature, 0.2);
    }

    #[test]
    fn behavior_config_output_protocol_none_disables_auto_protocol_prompt() {
        let path = Path::new("on_msg.yml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: test_rule
output_protocol: None
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.output_protocol, BehaviorOutputProtocol::None);
        assert!(cfg.output_protocol.is_disabled());
        assert_eq!(cfg.llm.output_protocol, "");
        assert_eq!(cfg.llm.output_mode, "auto");
    }

    #[test]
    fn behavior_config_output_protocol_structured_none_disables_auto_protocol_prompt() {
        let path = Path::new("on_msg.yml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: test_rule
output_protocol:
  mode: none
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.output_protocol, BehaviorOutputProtocol::None);
        assert_eq!(cfg.llm.output_protocol, "");
        assert_eq!(cfg.llm.output_mode, "auto");
    }

    #[test]
    fn route_result_protocol_includes_session_routing_keys() {
        let protocol = build_route_result_protocol();
        assert!(protocol.contains("work_session_id"));
        assert!(protocol.contains("new_work_session"));
        assert!(protocol.contains("MUST provide exactly one of"));
    }

    #[test]
    fn behavior_llm_result_protocol_disallows_session_routing_keys() {
        let protocol = build_behavior_llm_result_protocol();
        assert!(!protocol.contains("work_session_id"));
        assert!(!protocol.contains("new_work_session"));
    }

    #[test]
    fn behavior_config_tools_allowlist_normalizes_names() {
        let path = Path::new("route.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: route_rule
tools:
  mode: allow_list
  names:
    - load_memory
    - load_memory
"#,
        )
        .expect("parse behavior yaml");
        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["load_memory".to_string()]);
    }

    #[test]
    fn behavior_config_skills_defaults_to_union_and_normalizes_load_skills() {
        let path = Path::new("plan.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: plan rule
skills:
  load_skills:
    - planner
    - planner
    - chatonly
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.skills.mode, BehaviorSkillMode::Union);
        assert_eq!(
            cfg.skills.load_skills,
            vec!["planner".to_string(), "chatonly".to_string()]
        );
    }

    #[test]
    fn behavior_config_skills_preserves_explicit_mode() {
        let path = Path::new("execute.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: execute rule
skills:
  mode: behavior_only
  load_skills:
    - coding/rust
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.skills.mode, BehaviorSkillMode::BehaviorOnly);
        assert_eq!(cfg.skills.load_skills, vec!["coding/rust".to_string()]);
    }

    #[test]
    fn behavior_config_memory_normalizes_percent() {
        let path = Path::new("memory.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: memory_rule
memory:
  total_limit: 1024
  first_prompt: "  memory header text  "
  last_prompt: "   "
  session_step_records:
    limit: 256
    max_percent: 1.5
    skip_last_n: 0
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
        assert_eq!(cfg.memory.session_step_records.limit, 256);
        assert_eq!(cfg.memory.session_step_records.max_percent, None);
        assert_eq!(cfg.memory.session_step_records.skip_last_n, None);
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
    fn behavior_config_tools_none_mode_is_preserved() {
        let path = Path::new("route.yaml");
        let cfg = BehaviorConfig::parse_from_str(
            path,
            r#"
system: route_rule
tools:
  mode: none
  names:
    - exec_bash
"#,
        )
        .expect("parse behavior yaml");

        assert_eq!(cfg.tools.mode, BehaviorToolMode::None);
        assert_eq!(cfg.tools.names, vec!["exec_bash".to_string()]);
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
system: package_rule
tools:
  mode: allow_list
  names:
    - exec_bash
"#,
        )
        .await
        .expect("write package behavior");
        fs::write(
            env_root.join("plan.yaml"),
            r#"
system: env_rule
"#,
        )
        .await
        .expect("write env behavior");

        let cfg =
            BehaviorConfig::load_from_roots(&[env_root.clone(), package_root.clone()], "plan")
                .await
                .expect("merge behavior config");

        assert_eq!(cfg.system, "env_rule");
        assert_eq!(cfg.tools.mode, BehaviorToolMode::AllowList);
        assert_eq!(cfg.tools.names, vec!["exec_bash".to_string()]);
    }
}
