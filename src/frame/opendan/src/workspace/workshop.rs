use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{timeout, Duration, Instant};

use super::todo::{TodoTool, TodoToolConfig, TOOL_TODO_MANAGE};
use super::worklog::{WorklogTool, WorklogToolConfig, TOOL_WORKLOG_MANAGE};
use crate::agent_tool::{
    AgentTool, MCPToolConfig, ToolCallContext, ToolError, ToolManager, ToolSpec,
};

pub const TOOL_EXEC_BASH: &str = "exec_bash";
pub const TOOL_EDIT_FILE: &str = "edit_file";

const DEFAULT_BASH_PATH: &str = "/bin/bash";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_DIFF_LINES: usize = 200;
const DEFAULT_MAX_FILE_WRITE_BYTES: usize = 256 * 1024;
const DEFAULT_TOOLS_JSON_REL_PATH: &str = "tools/tools.json";
const DEFAULT_TOOLS_MD_REL_PATH: &str = "tools/tools.md";
const DEFAULT_TODO_DB_REL_PATH: &str = "todo/todo.db";
const DEFAULT_WORKLOG_DB_REL_PATH: &str = "worklog/worklog.db";
const DEFAULT_TODO_LIST_LIMIT: usize = 32;
const DEFAULT_TODO_MAX_LIST_LIMIT: usize = 128;
const DEFAULT_WORKLOG_LIST_LIMIT: usize = 64;
const DEFAULT_WORKLOG_MAX_LIST_LIMIT: usize = 256;

#[derive(Clone, Debug)]
pub struct AgentWorkshopConfig {
    pub workspace_root: PathBuf,
    pub bash_path: PathBuf,
    pub default_timeout_ms: u64,
    pub max_output_bytes: usize,
    pub default_max_diff_lines: usize,
    pub default_max_file_write_bytes: usize,
    pub tools_json_rel_path: PathBuf,
    pub tools_markdown_rel_path: PathBuf,
}

impl AgentWorkshopConfig {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            bash_path: PathBuf::from(DEFAULT_BASH_PATH),
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            default_max_diff_lines: DEFAULT_MAX_DIFF_LINES,
            default_max_file_write_bytes: DEFAULT_MAX_FILE_WRITE_BYTES,
            tools_json_rel_path: PathBuf::from(DEFAULT_TOOLS_JSON_REL_PATH),
            tools_markdown_rel_path: PathBuf::from(DEFAULT_TOOLS_MD_REL_PATH),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentWorkshopToolsConfig {
    pub enabled_tools: Vec<WorkshopToolConfig>,
}

impl Default for AgentWorkshopToolsConfig {
    fn default() -> Self {
        Self {
            enabled_tools: vec![
                WorkshopToolConfig::enabled(TOOL_EXEC_BASH),
                WorkshopToolConfig::enabled(TOOL_EDIT_FILE),
                WorkshopToolConfig::enabled(TOOL_TODO_MANAGE),
                WorkshopToolConfig::enabled(TOOL_WORKLOG_MANAGE),
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkshopToolConfig {
    pub name: String,
    #[serde(default = "default_tool_kind")]
    pub kind: String,
    pub enabled: bool,
    pub params: Json,
}

impl Default for WorkshopToolConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: default_tool_kind(),
            enabled: true,
            params: Json::Object(serde_json::Map::new()),
        }
    }
}

impl WorkshopToolConfig {
    pub fn enabled(name: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: default_tool_kind(),
            enabled: true,
            params: Json::Object(serde_json::Map::new()),
        }
    }
}

fn default_tool_kind() -> String {
    "builtin".to_string()
}

#[derive(Clone, Debug)]
pub struct AgentWorkshop {
    cfg: AgentWorkshopConfig,
    tools_cfg: AgentWorkshopToolsConfig,
}

impl AgentWorkshop {
    pub async fn new(mut cfg: AgentWorkshopConfig) -> Result<Self, ToolError> {
        let workspace_root = normalize_workspace_root(&cfg.workspace_root)?;
        create_minimal_workspace_dirs(&workspace_root).await?;
        cfg.workspace_root = workspace_root.clone();

        let tools_cfg = load_tools_config(&workspace_root, &cfg).await?;
        validate_tools_config(&tools_cfg)?;

        Ok(Self { cfg, tools_cfg })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.cfg.workspace_root
    }

    pub fn tools_config(&self) -> &AgentWorkshopToolsConfig {
        &self.tools_cfg
    }

    pub fn register_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        for tool in self
            .tools_cfg
            .enabled_tools
            .iter()
            .filter(|tool| tool.enabled)
        {
            match tool.kind.as_str() {
                "builtin" => match tool.name.as_str() {
                    TOOL_EXEC_BASH => {
                        tool_mgr.register_tool(ExecBashTool {
                            cfg: self.cfg.clone(),
                            policy: ExecBashPolicy::from_tool_config(&self.cfg, tool)?,
                        })?;
                    }
                    TOOL_EDIT_FILE => {
                        tool_mgr.register_tool(EditFileTool {
                            cfg: self.cfg.clone(),
                            policy: EditFilePolicy::from_tool_config(&self.cfg, tool)?,
                        })?;
                    }
                    TOOL_TODO_MANAGE => {
                        let policy = TodoToolPolicy::from_tool_config(&self.cfg, tool)?;
                        tool_mgr.register_tool(TodoTool::new(TodoToolConfig {
                            db_path: policy.db_path,
                            default_list_limit: policy.default_list_limit,
                            max_list_limit: policy.max_list_limit,
                        })?)?;
                    }
                    TOOL_WORKLOG_MANAGE => {
                        let policy = WorklogToolPolicy::from_tool_config(&self.cfg, tool)?;
                        tool_mgr.register_tool(WorklogTool::new(WorklogToolConfig {
                            db_path: policy.db_path,
                            default_list_limit: policy.default_list_limit,
                            max_list_limit: policy.max_list_limit,
                        })?)?;
                    }
                    unsupported => {
                        return Err(ToolError::InvalidArgs(format!(
                            "builtin tool `{unsupported}` is not supported by current runtime"
                        )));
                    }
                },
                "mcp" => {
                    tool_mgr.register_mcp_tool(build_mcp_tool_config(tool)?)?;
                }
                unsupported_kind => {
                    return Err(ToolError::InvalidArgs(format!(
                        "tool `{}` has unsupported kind `{unsupported_kind}`",
                        tool.name
                    )));
                }
            }
        }
        Ok(())
    }
}

// Backward compatibility alias for existing callers.
pub type BasicWorkshop = AgentWorkshop;
pub type BasicWorkshopConfig = AgentWorkshopConfig;

#[derive(Clone, Debug)]
struct ExecBashPolicy {
    default_timeout_ms: u64,
    max_timeout_ms: u64,
    allow_env: bool,
    allowed_cwd_roots: Vec<PathBuf>,
}

impl ExecBashPolicy {
    fn from_tool_config(
        workshop_cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, ToolError> {
        let params = tool_cfg.params.as_object().ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "tool `{}` params must be a json object",
                tool_cfg.name
            ))
        })?;

        let default_timeout_raw = read_u64_from_map(params, "default_timeout_ms")?;
        let max_timeout_raw = read_u64_from_map(params, "max_timeout_ms")?;
        let max_timeout_ms = max_timeout_raw
            .unwrap_or(default_timeout_raw.unwrap_or(workshop_cfg.default_timeout_ms));
        let default_timeout_ms =
            default_timeout_raw.unwrap_or(workshop_cfg.default_timeout_ms.min(max_timeout_ms));
        if default_timeout_ms == 0 || max_timeout_ms == 0 || default_timeout_ms > max_timeout_ms {
            return Err(ToolError::InvalidArgs(format!(
                "tool `{}` has invalid timeout bounds",
                tool_cfg.name
            )));
        }

        let allow_env = read_bool_from_map(params, "allow_env")?.unwrap_or(true);
        let allowed_cwd_roots = parse_workspace_relative_roots(
            params.get("allowed_cwd_roots"),
            &workshop_cfg.workspace_root,
        )?
        .unwrap_or_else(|| vec![workshop_cfg.workspace_root.clone()]);

        Ok(Self {
            default_timeout_ms,
            max_timeout_ms,
            allow_env,
            allowed_cwd_roots,
        })
    }
}

#[derive(Clone, Debug)]
struct EditFilePolicy {
    allow_create: bool,
    allow_replace: bool,
    max_write_bytes: usize,
    max_diff_lines: usize,
    allowed_write_roots: Vec<PathBuf>,
}

impl EditFilePolicy {
    fn from_tool_config(
        workshop_cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, ToolError> {
        let params = tool_cfg.params.as_object().ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "tool `{}` params must be a json object",
                tool_cfg.name
            ))
        })?;

        let allow_create = read_bool_from_map(params, "allow_create")?.unwrap_or(true);
        let allow_replace = read_bool_from_map(params, "allow_replace")?.unwrap_or(true);

        let max_write_bytes = read_u64_from_map(params, "max_write_bytes")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(workshop_cfg.default_max_file_write_bytes);
        if max_write_bytes == 0 {
            return Err(ToolError::InvalidArgs(format!(
                "tool `{}` max_write_bytes must be > 0",
                tool_cfg.name
            )));
        }

        let max_diff_lines = read_u64_from_map(params, "max_diff_lines")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(workshop_cfg.default_max_diff_lines);
        if max_diff_lines == 0 {
            return Err(ToolError::InvalidArgs(format!(
                "tool `{}` max_diff_lines must be > 0",
                tool_cfg.name
            )));
        }

        let allowed_write_roots = parse_workspace_relative_roots(
            params.get("allowed_write_roots"),
            &workshop_cfg.workspace_root,
        )?
        .unwrap_or_else(|| vec![workshop_cfg.workspace_root.clone()]);

        Ok(Self {
            allow_create,
            allow_replace,
            max_write_bytes,
            max_diff_lines,
            allowed_write_roots,
        })
    }
}

#[derive(Clone, Debug)]
struct TodoToolPolicy {
    db_path: PathBuf,
    default_list_limit: usize,
    max_list_limit: usize,
}

impl TodoToolPolicy {
    fn from_tool_config(
        workshop_cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, ToolError> {
        let params = tool_cfg.params.as_object().ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "tool `{}` params must be a json object",
                tool_cfg.name
            ))
        })?;

        let db_path = if let Some(raw_db_path) = read_string_from_map(params, "db_path")? {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, &raw_db_path)?
        } else {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, DEFAULT_TODO_DB_REL_PATH)?
        };

        let default_list_limit = read_u64_from_map(params, "default_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_TODO_LIST_LIMIT);
        let max_list_limit = read_u64_from_map(params, "max_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_TODO_MAX_LIST_LIMIT.max(default_list_limit));

        if default_list_limit == 0 || max_list_limit == 0 || default_list_limit > max_list_limit {
            return Err(ToolError::InvalidArgs(format!(
                "tool `{}` has invalid list limit bounds",
                tool_cfg.name
            )));
        }

        Ok(Self {
            db_path,
            default_list_limit,
            max_list_limit,
        })
    }
}

#[derive(Clone, Debug)]
struct WorklogToolPolicy {
    db_path: PathBuf,
    default_list_limit: usize,
    max_list_limit: usize,
}

impl WorklogToolPolicy {
    fn from_tool_config(
        workshop_cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, ToolError> {
        let params = tool_cfg.params.as_object().ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "tool `{}` params must be a json object",
                tool_cfg.name
            ))
        })?;

        let db_path = if let Some(raw_db_path) = read_string_from_map(params, "db_path")? {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, &raw_db_path)?
        } else {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, DEFAULT_WORKLOG_DB_REL_PATH)?
        };

        let default_list_limit = read_u64_from_map(params, "default_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_WORKLOG_LIST_LIMIT);
        let max_list_limit = read_u64_from_map(params, "max_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_WORKLOG_MAX_LIST_LIMIT.max(default_list_limit));

        if default_list_limit == 0 || max_list_limit == 0 || default_list_limit > max_list_limit {
            return Err(ToolError::InvalidArgs(format!(
                "tool `{}` has invalid list limit bounds",
                tool_cfg.name
            )));
        }

        Ok(Self {
            db_path,
            default_list_limit,
            max_list_limit,
        })
    }
}

#[derive(Clone, Debug)]
struct ExecBashTool {
    cfg: AgentWorkshopConfig,
    policy: ExecBashPolicy,
}

#[async_trait]
impl AgentTool for ExecBashTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_EXEC_BASH.to_string(),
            description: "Run a Linux bash command inside workshop scope.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string", "description": "Optional cwd under workspace root." },
                    "timeout_ms": { "type": "integer", "minimum": 1 },
                    "env": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["command"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "exit_code": { "type": ["integer", "null"] },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "stdout_truncated": { "type": "boolean" },
                    "stderr_truncated": { "type": "boolean" },
                    "duration_ms": { "type": "integer" },
                    "cwd": { "type": "string" }
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let command = require_string(&args, "command")?;
        let cwd = if let Some(raw_cwd) = optional_string(&args, "cwd")? {
            resolve_path_in_workspace(&self.cfg.workspace_root, &raw_cwd)?
        } else {
            self.cfg.workspace_root.clone()
        };
        if !is_path_under_any(&cwd, &self.policy.allowed_cwd_roots) {
            return Err(ToolError::InvalidArgs(format!(
                "cwd `{}` not allowed by workshop tool policy",
                cwd.display()
            )));
        }

        let timeout_ms =
            optional_u64(&args, "timeout_ms")?.unwrap_or(self.policy.default_timeout_ms);
        if timeout_ms == 0 || timeout_ms > self.policy.max_timeout_ms {
            return Err(ToolError::InvalidArgs(format!(
                "timeout_ms out of range: {} (max: {})",
                timeout_ms, self.policy.max_timeout_ms
            )));
        }

        let mut command_builder = Command::new(&self.cfg.bash_path);
        command_builder.arg("-lc").arg(&command).current_dir(&cwd);
        command_builder.kill_on_drop(true);

        if let Some(env) = args.get("env") {
            if !self.policy.allow_env {
                return Err(ToolError::InvalidArgs(
                    "env injection is disabled by workshop tool policy".to_string(),
                ));
            }
            let env_obj = env.as_object().ok_or_else(|| {
                ToolError::InvalidArgs("env must be an object of string values".to_string())
            })?;
            for (key, value) in env_obj {
                let value = value
                    .as_str()
                    .ok_or_else(|| ToolError::InvalidArgs(format!("env.{key} must be a string")))?;
                command_builder.env(key, value);
            }
        }

        let started = Instant::now();
        let output =
            match timeout(Duration::from_millis(timeout_ms), command_builder.output()).await {
                Ok(result) => result
                    .map_err(|err| ToolError::ExecFailed(format!("wait bash failed: {err}")))?,
                Err(_) => return Err(ToolError::Timeout),
            };

        let duration_ms = started.elapsed().as_millis() as u64;
        let (stdout, stdout_truncated) = truncate_bytes(
            String::from_utf8_lossy(&output.stdout).as_bytes(),
            self.cfg.max_output_bytes,
        );
        let (stderr, stderr_truncated) = truncate_bytes(
            String::from_utf8_lossy(&output.stderr).as_bytes(),
            self.cfg.max_output_bytes,
        );

        Ok(json!({
            "ok": output.status.success(),
            "exit_code": output.status.code(),
            "stdout": stdout,
            "stderr": stderr,
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated,
            "duration_ms": duration_ms,
            "command": command,
            "cwd": cwd.to_string_lossy().to_string()
        }))
    }
}

#[derive(Clone, Debug)]
struct EditFileTool {
    cfg: AgentWorkshopConfig,
    policy: EditFilePolicy,
}

#[async_trait]
impl AgentTool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_EDIT_FILE.to_string(),
            description: "Edit file content under workshop scope and return a diff.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string", "description": "Used by overwrite/append mode." },
                    "append": { "type": "boolean", "default": false },
                    "old_text": { "type": "string", "description": "If present, run replace mode." },
                    "new_text": { "type": "string" },
                    "replace_all": { "type": "boolean", "default": false },
                    "create_if_missing": { "type": "boolean", "default": true }
                },
                "required": ["path"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "path": { "type": "string" },
                    "operation": { "type": "string" },
                    "created": { "type": "boolean" },
                    "changed": { "type": "boolean" },
                    "bytes_before": { "type": "integer" },
                    "bytes_after": { "type": "integer" },
                    "diff": { "type": "string" },
                    "diff_truncated": { "type": "boolean" }
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let file_path = require_string(&args, "path")?;
        let abs_path = resolve_path_in_workspace(&self.cfg.workspace_root, &file_path)?;
        if !is_path_under_any(&abs_path, &self.policy.allowed_write_roots) {
            return Err(ToolError::InvalidArgs(format!(
                "path `{file_path}` is not writable by workshop tool policy"
            )));
        }

        let create_if_missing = optional_bool(&args, "create_if_missing")?.unwrap_or(true);
        let exists = fs::metadata(&abs_path).await.is_ok();
        if !exists && (!create_if_missing || !self.policy.allow_create) {
            return Err(ToolError::InvalidArgs(format!(
                "file does not exist or create disabled by policy: {file_path}"
            )));
        }

        let original_content = if exists {
            read_text_file_lossy(&abs_path).await?
        } else {
            String::new()
        };

        let has_replace_mode = args.get("old_text").is_some();
        let (operation, updated_content) = if has_replace_mode {
            if !self.policy.allow_replace {
                return Err(ToolError::InvalidArgs(
                    "replace mode disabled by workshop tool policy".to_string(),
                ));
            }
            let old_text = require_string(&args, "old_text")?;
            let new_text = require_string(&args, "new_text")?;
            let replace_all = optional_bool(&args, "replace_all")?.unwrap_or(false);
            if !original_content.contains(&old_text) {
                return Err(ToolError::InvalidArgs(format!(
                    "old_text not found in file: {file_path}"
                )));
            }
            let replaced = if replace_all {
                original_content.replace(&old_text, &new_text)
            } else {
                original_content.replacen(&old_text, &new_text, 1)
            };
            ("replace".to_string(), replaced)
        } else {
            let content = require_string(&args, "content")?;
            let append = optional_bool(&args, "append")?.unwrap_or(false);
            if append {
                ("append".to_string(), format!("{original_content}{content}"))
            } else if exists {
                ("overwrite".to_string(), content)
            } else {
                ("create".to_string(), content)
            }
        };

        if updated_content.len() > self.policy.max_write_bytes {
            return Err(ToolError::InvalidArgs(format!(
                "file content too large: {} > {} bytes",
                updated_content.len(),
                self.policy.max_write_bytes
            )));
        }

        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|err| ToolError::ExecFailed(format!("create parent dir failed: {err}")))?;
        }
        fs::write(&abs_path, updated_content.as_bytes())
            .await
            .map_err(|err| ToolError::ExecFailed(format!("write file failed: {err}")))?;

        let changed = original_content != updated_content;
        let (diff, diff_truncated) = build_simple_diff(
            &file_path,
            &original_content,
            &updated_content,
            self.policy.max_diff_lines,
        );

        Ok(json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "operation": operation,
            "created": !exists,
            "changed": changed,
            "bytes_before": original_content.len(),
            "bytes_after": updated_content.len(),
            "diff": diff,
            "diff_truncated": diff_truncated
        }))
    }
}

fn require_string(args: &Json, key: &str) -> Result<String, ToolError> {
    let value = args
        .get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or invalid `{key}`")))?;
    if value.is_empty() {
        return Err(ToolError::InvalidArgs(format!("`{key}` cannot be empty")));
    }
    Ok(value)
}

fn optional_string(args: &Json, key: &str) -> Result<Option<String>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(raw.to_string()))
}

fn optional_u64(args: &Json, key: &str) -> Result<Option<u64>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a positive integer")))
}

fn optional_bool(args: &Json, key: &str) -> Result<Option<bool>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a boolean")))
}

fn read_u64_from_map(
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<u64>, ToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be an integer")))
}

fn read_string_from_map(
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<String>, ToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(value.to_string()))
}

fn read_bool_from_map(
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<bool>, ToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a boolean")))
}

fn parse_workspace_relative_roots(
    value: Option<&Json>,
    workspace_root: &Path,
) -> Result<Option<Vec<PathBuf>>, ToolError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let arr = value.as_array().ok_or_else(|| {
        ToolError::InvalidArgs("tool params path roots must be string array".to_string())
    })?;
    let mut roots = Vec::with_capacity(arr.len());
    for item in arr {
        let raw = item.as_str().ok_or_else(|| {
            ToolError::InvalidArgs("tool params path roots must be string array".to_string())
        })?;
        roots.push(resolve_path_in_workspace(workspace_root, raw)?);
    }
    Ok(Some(roots))
}

fn build_mcp_tool_config(tool_cfg: &WorkshopToolConfig) -> Result<MCPToolConfig, ToolError> {
    let params = tool_cfg.params.as_object().ok_or_else(|| {
        ToolError::InvalidArgs(format!(
            "mcp tool `{}` params must be a json object",
            tool_cfg.name
        ))
    })?;

    let endpoint = read_string_from_map(params, "endpoint")?.ok_or_else(|| {
        ToolError::InvalidArgs(format!(
            "mcp tool `{}` requires params.endpoint",
            tool_cfg.name
        ))
    })?;

    let mcp_tool_name = read_string_from_map(params, "mcp_tool_name")?;
    let description = read_string_from_map(params, "description")?;
    let timeout_ms = read_u64_from_map(params, "timeout_ms")?.unwrap_or(30_000);

    let headers = match params.get("headers") {
        None => HashMap::new(),
        Some(value) => {
            let obj = value.as_object().ok_or_else(|| {
                ToolError::InvalidArgs(format!(
                    "mcp tool `{}` params.headers must be an object",
                    tool_cfg.name
                ))
            })?;
            let mut headers = HashMap::with_capacity(obj.len());
            for (key, value) in obj {
                let val = value.as_str().ok_or_else(|| {
                    ToolError::InvalidArgs(format!(
                        "mcp tool `{}` params.headers.{key} must be string",
                        tool_cfg.name
                    ))
                })?;
                headers.insert(key.to_string(), val.to_string());
            }
            headers
        }
    };

    let args_schema = params
        .get("args_schema")
        .cloned()
        .unwrap_or_else(|| json!({"type":"object"}));
    let output_schema = params
        .get("output_schema")
        .cloned()
        .unwrap_or_else(|| json!({"type":"object"}));

    Ok(MCPToolConfig {
        name: tool_cfg.name.clone(),
        endpoint,
        mcp_tool_name,
        description,
        args_schema,
        output_schema,
        headers,
        timeout_ms,
    })
}

fn normalize_workspace_root(root: &Path) -> Result<PathBuf, ToolError> {
    let root_abs = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| ToolError::ExecFailed(format!("read current_dir failed: {err}")))?
            .join(root)
    };
    Ok(normalize_abs_path(&root_abs))
}

async fn create_minimal_workspace_dirs(workspace_root: &Path) -> Result<(), ToolError> {
    let roots = [
        workspace_root.to_path_buf(),
        workspace_root.join("worklog"),
        workspace_root.join("todo"),
        workspace_root.join("tools"),
        workspace_root.join("artifacts"),
    ];
    for dir in roots {
        fs::create_dir_all(&dir).await.map_err(|err| {
            ToolError::ExecFailed(format!("create dir `{}` failed: {err}", dir.display()))
        })?;
    }
    Ok(())
}

async fn load_tools_config(
    workspace_root: &Path,
    workshop_cfg: &AgentWorkshopConfig,
) -> Result<AgentWorkshopToolsConfig, ToolError> {
    let tools_json_path = workspace_root.join(&workshop_cfg.tools_json_rel_path);
    match fs::read_to_string(&tools_json_path).await {
        Ok(content) => {
            let cfg =
                serde_json::from_str::<AgentWorkshopToolsConfig>(&content).map_err(|err| {
                    ToolError::InvalidArgs(format!(
                        "invalid tools config json `{}`: {err}",
                        tools_json_path.display()
                    ))
                })?;
            return Ok(cfg);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(ToolError::ExecFailed(format!(
                "read tools config json `{}` failed: {err}",
                tools_json_path.display()
            )));
        }
    }

    let tools_md_path = workspace_root.join(&workshop_cfg.tools_markdown_rel_path);
    match fs::read_to_string(&tools_md_path).await {
        Ok(content) => parse_tools_markdown_config(&tools_md_path, &content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(AgentWorkshopToolsConfig::default())
        }
        Err(err) => Err(ToolError::ExecFailed(format!(
            "read tools config markdown `{}` failed: {err}",
            tools_md_path.display()
        ))),
    }
}

fn parse_tools_markdown_config(
    source_path: &Path,
    content: &str,
) -> Result<AgentWorkshopToolsConfig, ToolError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(AgentWorkshopToolsConfig::default());
    }

    if let Ok(cfg) = serde_json::from_str::<AgentWorkshopToolsConfig>(trimmed) {
        return Ok(cfg);
    }

    let Some(json_block) = try_extract_json_block(trimmed) else {
        return Err(ToolError::InvalidArgs(format!(
            "tools markdown `{}` does not contain valid json config block",
            source_path.display()
        )));
    };

    serde_json::from_str::<AgentWorkshopToolsConfig>(&json_block).map_err(|err| {
        ToolError::InvalidArgs(format!(
            "invalid tools markdown config `{}`: {err}",
            source_path.display()
        ))
    })
}

fn validate_tools_config(cfg: &AgentWorkshopToolsConfig) -> Result<(), ToolError> {
    let mut seen = HashSet::new();
    for tool in &cfg.enabled_tools {
        if tool.name.trim().is_empty() {
            return Err(ToolError::InvalidArgs(
                "tool config contains empty tool name".to_string(),
            ));
        }
        if !seen.insert(tool.name.clone()) {
            return Err(ToolError::InvalidArgs(format!(
                "duplicate tool config entry: {}",
                tool.name
            )));
        }
    }
    Ok(())
}

fn try_extract_json_block(content: &str) -> Option<String> {
    let fence_parts: Vec<&str> = content.split("```").collect();
    if fence_parts.len() >= 3 {
        for segment in fence_parts.iter().skip(1).step_by(2) {
            let trimmed = segment.trim();
            let payload = if let Some(rest) = trimmed.strip_prefix("json") {
                rest.trim()
            } else {
                trimmed
            };
            if serde_json::from_str::<Json>(payload).is_ok() {
                return Some(payload.to_string());
            }
        }
    }

    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    let candidate = &content[start..=end];
    if serde_json::from_str::<Json>(candidate).is_ok() {
        return Some(candidate.to_string());
    }
    None
}

fn resolve_path_in_workspace(workspace_root: &Path, raw_path: &str) -> Result<PathBuf, ToolError> {
    if raw_path.trim().is_empty() {
        return Err(ToolError::InvalidArgs("path cannot be empty".to_string()));
    }
    let user_path = Path::new(raw_path);
    let candidate = if user_path.is_absolute() {
        user_path.to_path_buf()
    } else {
        workspace_root.join(user_path)
    };
    let normalized = normalize_abs_path(&candidate);
    if !normalized.starts_with(workspace_root) {
        return Err(ToolError::InvalidArgs(format!(
            "path out of workspace scope: {raw_path}"
        )));
    }
    Ok(normalized)
}

fn normalize_abs_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    normalized
}

fn is_path_under_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

async fn read_text_file_lossy(path: &Path) -> Result<String, ToolError> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| ToolError::ExecFailed(format!("read file failed: {err}")))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn truncate_bytes(input: &[u8], max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (String::from_utf8_lossy(input).to_string(), false);
    }
    (
        String::from_utf8_lossy(&input[..max_bytes]).to_string(),
        true,
    )
}

fn build_simple_diff(
    display_path: &str,
    before: &str,
    after: &str,
    max_body_lines: usize,
) -> (String, bool) {
    let before_lines = before.lines().collect::<Vec<_>>();
    let after_lines = after.lines().collect::<Vec<_>>();

    let mut body = Vec::new();
    let mut truncated = false;
    let max_len = before_lines.len().max(after_lines.len());
    for idx in 0..max_len {
        if body.len() >= max_body_lines {
            truncated = true;
            break;
        }
        let old = before_lines.get(idx).copied();
        let new = after_lines.get(idx).copied();
        match (old, new) {
            (Some(a), Some(b)) if a == b => body.push(format!(" {a}")),
            (Some(a), Some(b)) => {
                body.push(format!("-{a}"));
                if body.len() >= max_body_lines {
                    truncated = true;
                    break;
                }
                body.push(format!("+{b}"));
            }
            (Some(a), None) => body.push(format!("-{a}")),
            (None, Some(b)) => body.push(format!("+{b}")),
            (None, None) => {}
        }
    }

    let mut diff = Vec::new();
    diff.push(format!("--- a/{display_path}"));
    diff.push(format!("+++ b/{display_path}"));
    diff.push(format!(
        "@@ -1,{} +1,{} @@",
        before_lines.len(),
        after_lines.len()
    ));
    diff.extend(body);
    if truncated {
        diff.push("... [DIFF_TRUNCATED]".to_string());
    }

    (diff.join("\n"), truncated)
}

fn u64_to_usize(v: u64) -> Result<usize, ToolError> {
    usize::try_from(v).map_err(|_| ToolError::InvalidArgs(format!("value too large: {v}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_tool::{ToolCall, ToolCallContext};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_workspace_root(test_name: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("opendan-{test_name}-{ts}"))
    }

    async fn call(tool_mgr: &ToolManager, name: &str, args: Json) -> Result<Json, ToolError> {
        tool_mgr
            .call_tool(
                &ToolCallContext {
                    trace_id: "trace-test".to_string(),
                    agent_did: "did:example:agent".to_string(),
                    behavior: "on_wakeup".to_string(),
                    step_idx: 0,
                    wakeup_id: "wakeup-test".to_string(),
                },
                ToolCall {
                    name: name.to_string(),
                    args,
                    call_id: "call-test".to_string(),
                },
            )
            .await
    }

    async fn write_tools_json(root: &Path, payload: Json) {
        let path = root.join("tools/tools.json");
        fs::create_dir_all(path.parent().expect("tools parent"))
            .await
            .expect("create tools dir");
        fs::write(
            path,
            serde_json::to_string_pretty(&payload).expect("serialize tools config"),
        )
        .await
        .expect("write tools config");
    }

    #[tokio::test]
    async fn exec_bash_tool_runs_linux_command() {
        let root = unique_workspace_root("exec-bash");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        let result = call(
            &tool_mgr,
            TOOL_EXEC_BASH,
            json!({
                "command": "printf 'hello-linux'",
            }),
        )
        .await
        .expect("exec bash should succeed");

        assert_eq!(result["ok"], true);
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["stdout"], "hello-linux");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn edit_file_tool_writes_file_and_returns_diff() {
        let root = unique_workspace_root("edit-file");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "notes/todo.md",
                "content": "line1\nline2\n"
            }),
        )
        .await
        .expect("create file should succeed");

        let result = call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "notes/todo.md",
                "old_text": "line2",
                "new_text": "lineX"
            }),
        )
        .await
        .expect("replace should succeed");

        let content = fs::read_to_string(root.join("notes/todo.md"))
            .await
            .expect("read file");
        assert!(content.contains("lineX"));
        let diff = result["diff"].as_str().unwrap_or_default();
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+lineX"));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_config_can_enable_subset_of_runtime_tools() {
        let root = unique_workspace_root("tool-subset");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    { "name": "edit_file", "enabled": true, "params": {} }
                ]
            }),
        )
        .await;

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool(TOOL_EDIT_FILE));
        assert!(!tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(!tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let err = call(
            &tool_mgr,
            TOOL_EXEC_BASH,
            json!({"command":"echo should_not_run"}),
        )
        .await
        .expect_err("tool should not be registered");
        assert!(matches!(err, ToolError::NotFound(_)));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tool_params_apply_workshop_boundary_controls() {
        let root = unique_workspace_root("tool-policy");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    {
                        "name": "edit_file",
                        "enabled": true,
                        "params": {
                            "allowed_write_roots": ["todo"],
                            "allow_create": true,
                            "max_write_bytes": 128,
                            "max_diff_lines": 40
                        }
                    }
                ]
            }),
        )
        .await;

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "todo/ok.md",
                "content": "ok"
            }),
        )
        .await
        .expect("path under policy root should be writable");

        let err = call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "artifacts/out.md",
                "content": "blocked"
            }),
        )
        .await
        .expect_err("path outside policy root should be denied");
        assert!(matches!(err, ToolError::InvalidArgs(_)));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_config_can_register_mcp_tool() {
        let root = unique_workspace_root("tool-mcp");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    {
                        "name": "mcp.weather",
                        "kind": "mcp",
                        "enabled": true,
                        "params": {
                            "endpoint": "http://127.0.0.1:9",
                            "mcp_tool_name": "weather.query",
                            "timeout_ms": 3000
                        }
                    }
                ]
            }),
        )
        .await;

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool("weather"));
        assert!(!tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(!tool_mgr.has_tool(TOOL_EDIT_FILE));
        assert!(!tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_markdown_json_block_is_loaded() {
        let root = unique_workspace_root("tool-md");
        let md_path = root.join("tools/tools.md");
        fs::create_dir_all(md_path.parent().expect("tools parent"))
            .await
            .expect("create tools dir");
        fs::write(
            &md_path,
            r#"
# Tools

```json
{
  "enabled_tools": [
    { "name": "exec_bash", "enabled": true, "params": { "max_timeout_ms": 30 } }
  ]
}
```
"#,
        )
        .await
        .expect("write tools.md");

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(!tool_mgr.has_tool(TOOL_EDIT_FILE));
        assert!(!tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn todo_manage_tool_supports_create_list_update_and_task_bridge_fields() {
        let root = unique_workspace_root("todo-manage");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let created = call(
            &tool_mgr,
            TOOL_TODO_MANAGE,
            json!({
                "action": "create",
                "title": "Implement todo bridge",
                "description": "sync user todo and task execution state",
                "status": "todo",
                "priority": "high",
                "tags": ["runtime", "bridge"]
            }),
        )
        .await
        .expect("create todo should succeed");
        let todo_id = created["todo"]["id"]
            .as_str()
            .expect("todo id should exist")
            .to_string();

        let listed = call(
            &tool_mgr,
            TOOL_TODO_MANAGE,
            json!({
                "action": "list",
                "include_closed": false
            }),
        )
        .await
        .expect("list todo should succeed");
        let listed_todos = listed["todos"]
            .as_array()
            .expect("todos should be an array");
        assert!(!listed_todos.is_empty());
        assert!(listed_todos
            .iter()
            .any(|item| item.get("id").and_then(|v| v.as_str()) == Some(todo_id.as_str())));

        let updated = call(
            &tool_mgr,
            TOOL_TODO_MANAGE,
            json!({
                "action": "update",
                "id": todo_id,
                "status": "in_progress",
                "task_id": 42,
                "task_status": "running"
            }),
        )
        .await
        .expect("update todo should succeed");
        assert_eq!(updated["todo"]["status"], "in_progress");
        assert_eq!(updated["todo"]["task_id"], 42);
        assert_eq!(updated["todo"]["task_status"], "running");
        assert!(fs::metadata(root.join("todo/todo.db")).await.is_ok());

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn worklog_manage_tool_supports_append_and_query_fields() {
        let root = unique_workspace_root("worklog-manage");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let tool_mgr = ToolManager::new();
        workshop
            .register_tools(&tool_mgr)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let appended = call(
            &tool_mgr,
            TOOL_WORKLOG_MANAGE,
            json!({
                "action": "append",
                "type": "function_call",
                "status": "success",
                "step_id": "step-1",
                "thread_id": "thread-alpha",
                "summary": "exec_bash finished",
                "tags": ["tool", "runtime"],
                "payload": { "tool": "exec_bash", "ok": true }
            }),
        )
        .await
        .expect("append worklog should succeed");
        let log_id = appended["log"]["log_id"]
            .as_str()
            .expect("log id should exist")
            .to_string();

        let listed = call(
            &tool_mgr,
            TOOL_WORKLOG_MANAGE,
            json!({
                "action": "list",
                "thread_id": "thread-alpha",
                "tag": "runtime"
            }),
        )
        .await
        .expect("list worklog should succeed");
        let listed_logs = listed["logs"].as_array().expect("logs should be an array");
        assert_eq!(listed_logs.len(), 1);
        assert_eq!(listed_logs[0]["log_id"], log_id);

        let got = call(
            &tool_mgr,
            TOOL_WORKLOG_MANAGE,
            json!({
                "action": "get",
                "log_id": log_id
            }),
        )
        .await
        .expect("get worklog should succeed");
        assert_eq!(got["log"]["type"], "function_call");
        assert_eq!(got["log"]["thread_id"], "thread-alpha");
        assert!(fs::metadata(root.join("worklog/worklog.db")).await.is_ok());

        let _ = fs::remove_dir_all(root).await;
    }
}
