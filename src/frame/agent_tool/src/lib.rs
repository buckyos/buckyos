use std::collections::HashMap;
use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use buckyos_api::AiToolCall;
use log::{info, warn};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value as Json};
use tokio::time::{timeout, Duration};

pub mod cli;
pub mod file_tools;
pub mod json_args;
pub mod memory;
pub mod path_utils;
pub mod runtime_utils;
pub mod todo;
pub mod workspace;

pub use file_tools::{
    parse_read_file_bash_args, rewrite_read_file_path_with_shell_cwd, EditFileTool, FileToolConfig,
    FileWriteAuditBackend, FileWriteAuditRecord, NoopFileWriteAudit, ReadFileTool, WriteFileTool,
    TOOL_EDIT_FILE, TOOL_READ_FILE, TOOL_WRITE_FILE,
};
pub use json_args::{
    optional_string_arg, optional_trimmed_string_arg, optional_u64_arg, read_bool_from_map,
    read_string_from_map, read_u64_from_map, require_string_arg, require_trimmed_string_arg,
    u64_to_usize_arg,
};
pub use memory::{AgentMemory, AgentMemoryConfig, MemoryRankItem};
pub use path_utils::{
    normalize_abs_path, normalize_root_path, resolve_path_from_root, resolve_path_under_root,
    sanitize_session_id_for_path, session_record_path, to_abs_path, MAX_SESSION_ID_LEN,
};
pub use runtime_utils::now_ms;
pub use todo::{
    get_next_ready_todo_code, get_next_ready_todo_text, get_session_todo_text_by_ref,
    TodoAdminListItem, TodoAdminListOptions, TodoAdminListResult, TodoTool, TodoToolConfig,
};
pub use workspace::{
    ExternalWorkspaceBinding, ExternalWorkspaceRuntimeBackend, ExternalWorkspaceServiceConfig,
    LocalWorkspaceLock, LocalWorkspaceSessionBinding, ManagedExternalWorkspaceBackend,
    ManagedWorkspaceRecord, ManagedWorkspaceToolBackend, SessionWorkspaceBindingView,
    WorkspaceErrorSummary, WorkspaceOwner, WorkspaceRecordView, WorkspaceRuntimeBackend,
    WorkspaceStatus, WorkspaceType,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeContext {
    pub trace_id: String,
    pub agent_name: String,
    pub behavior: String,
    pub step_idx: u32,
    pub wakeup_id: String,
    pub session_id: String,
}

pub const TOOL_GET_SESSION: &str = "get_session";
pub const TOOL_LIST_SESSION: &str = "list_session";
pub const TOOL_LIST_EXTERNAL_WORKSPACES: &str = "list_external_workspaces";
pub const TOOL_BIND_EXTERNAL_WORKSPACE: &str = "bind_external_workspace";
pub const TOOL_CREATE_WORKSPACE: &str = "create_workspace";
pub const TOOL_BIND_WORKSPACE: &str = "bind_workspace";
pub const TOOL_LOAD_MEMORY: &str = "load_memory";
pub const TOOL_TODO_MANAGE: &str = "todo_manage";
pub const TOOL_WORKLOG_MANAGE: &str = "worklog_manage";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionMode {
    Serial,
    Parallel,
}

impl Default for ActionExecutionMode {
    fn default() -> Self {
        Self::Serial
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsScope {
    pub read_roots: Vec<String>,
    pub write_roots: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionCall {
    pub call_action_name: String,
    pub call_params: Json,
}

impl ActionCall {
    pub fn parse_json(value: &Json) -> Result<Self, String> {
        if let Some(items) = value.as_array() {
            if items.len() != 2 {
                return Err(format!(
                    "action call array must have 2 items, got {}",
                    items.len()
                ));
            }

            let action_name = items[0]
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| "action call first item must be non-empty string".to_string())?
                .to_string();

            let params = items[1].clone();
            if !params.is_object() {
                return Err("action call second item must be json object params".to_string());
            }

            return Ok(Self {
                call_action_name: action_name,
                call_params: params,
            });
        }

        let Some(map) = value.as_object() else {
            return Err(
                "action call must be array [\"action_id\", {...}] or object {\"action_id\": {...}}"
                    .to_string(),
            );
        };

        if map.len() != 1 {
            return Err(format!(
                "action call object must have exactly one action key, got {}",
                map.len()
            ));
        }
        let (action_name, raw_params) = map.iter().next().unwrap();
        let action_name = action_name.trim();
        if action_name.is_empty() {
            return Err("action call object key must be non-empty string".to_string());
        }

        let params = if raw_params.is_null() {
            json!({})
        } else if raw_params.is_object() {
            raw_params.clone()
        } else {
            return Err("action call object value must be json object params".to_string());
        };

        Ok(Self {
            call_action_name: action_name.to_string(),
            call_params: params,
        })
    }
}

impl Serialize for ActionCall {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.call_action_name)?;
        seq.serialize_element(&self.call_params)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for ActionCall {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Json::deserialize(deserializer)?;
        Self::parse_json(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum DoAction {
    //One line bash command
    //TODO: 确保是一行
    Exec(String),
    Call(ActionCall),
}

fn default_do_actions_mode() -> String {
    "failed_end".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DoActions {
    #[serde(default = "default_do_actions_mode")]
    pub mode: String,
    #[serde(default)]
    pub cmds: Vec<DoAction>,
}

impl Default for DoActions {
    fn default() -> Self {
        Self {
            mode: default_do_actions_mode(),
            cmds: Vec::new(),
        }
    }
}

impl DoActions {
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub args_schema: Json,
    pub output_schema: Json,
    #[serde(default)]
    pub usage: Option<String>,
}

impl ToolSpec {
    pub fn render_for_prompt(tools: &[ToolSpec]) -> String {
        serde_json::to_string(tools).unwrap_or_else(|_| "[]".to_string())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum AgentToolError {
    #[error("tool not found: {0}")]
    NotFound(String),
    #[error("tool already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid args: {0}")]
    InvalidArgs(String),
    #[error("execution failed: {0}")]
    ExecFailed(String),
    #[error("timeout")]
    Timeout,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MCPToolConfig {
    pub name: String,
    pub endpoint: String,
    #[serde(default)]
    pub mcp_tool_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_json_object")]
    pub args_schema: Json,
    #[serde(default = "default_json_object")]
    pub output_schema: Json,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default = "default_mcp_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_json_object() -> Json {
    json!({"type":"object"})
}

fn default_mcp_timeout_ms() -> u64 {
    30_000
}

pub fn normalize_tool_name(name: &str) -> String {
    name.trim().to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentHistoryShowLevel {
    Min,
    Mini,
    Medium,
    Full,
}

const HISTORY_COMPACT_CMD_MAX_CHARS: usize = 96;
const HISTORY_STD_DETAILS_MAX_CHARS: usize = 1600;
const HISTORY_BASH_OUTPUT_MINI_LINES: usize = 8;
const HISTORY_BASH_OUTPUT_FULL_LINES: usize = 24;


#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolStatus {
    #[default]
    Success,
    Error,
    Pending,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolPendingReason {
    LongRunning,
    UserApproval,
    #[serde(alias = "external_callback")]
    WaitForInstall,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentToolResult {
    #[serde(default)]
    pub is_agent_tool: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_name: Option<String>,
    #[serde(default)]
    pub status: AgentToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<AgentToolPendingReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_wait: Option<String>,

    #[serde(default)]
    pub summary: String,
    #[serde(rename = "detail", default = "default_json_object")]
    pub details: Json,

    //下面的都是is_agent_tool = false 的属性
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliStatus {
    Success,
    Error,
    Pending,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliPendingReason {
    LongRunning,
    UserApproval,
    #[serde(alias = "external_callback")]
    WaitForInstall,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliRunOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CliResultEnvelope {
    #[serde(default)]
    pub is_agent_tool: bool,
    pub status: CliStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmd_line: Option<String>,
    pub detail: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<CliPendingReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_wait: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_after: Option<u64>,
}

pub const CLI_EXIT_SUCCESS: i32 = 0;
pub const CLI_EXIT_ERROR: i32 = 1;
pub const CLI_EXIT_USAGE: i32 = 2;
pub const CLI_EXIT_COMMAND_NOT_FOUND: i32 = 127;

const PROMPT_STDIO_MAX_LINES: usize = 3000;

fn truncate_prompt_stream_lines(content: &str, max_lines: usize) -> String {
    if max_lines == 0 {
        return String::new();
    }

    let mut kept = Vec::<&str>::new();
    let mut total_lines = 0usize;
    for line in content.lines() {
        total_lines = total_lines.saturating_add(1);
        if kept.len() < max_lines {
            kept.push(line);
        }
    }

    if total_lines <= max_lines {
        return content.to_string();
    }

    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(
        format!(
            "... [TRUNCATED FOR ACTION PREVIEW: Showing first {} lines only] ...",
            max_lines
        )
        .as_str(),
    );
    out
}

impl AgentToolResult {
    pub fn from_details(details: Json) -> Self {
        Self {
            is_agent_tool: false,
            status: AgentToolStatus::Success,
            summary: String::new(),
            details,
            return_code: None,
            cmd_name: None,
            cmd_args: None,
            task_id: None,
            partial_output: None,
            pending_reason: None,
            check_after: None,
            estimated_wait: None,
            output: None,
        }
    }

    pub fn with_status(mut self, status: AgentToolStatus) -> Self {
        self.status = status;
        self
    }

    pub fn with_is_agent_tool(mut self, is_agent_tool: bool) -> Self {
        self.is_agent_tool = is_agent_tool;
        self
    }

    pub fn with_cmd_line(mut self, cmd_line: impl Into<String>) -> Self {
        let cmd_line = cmd_line.into();
        self = self.with_command_metadata_from_line(cmd_line.as_str());
        self
    }

    pub fn with_result(mut self, result: impl Into<String>) -> Self {
        self.summary = result.into();
        self
    }

    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        let output = output.into();
        self.output = (!output.trim().is_empty()).then_some(output);
        self
    }

    pub fn with_return_code(mut self, return_code: i32) -> Self {
        self.return_code = Some(return_code);
        self
    }

    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        let task_id = task_id.into();
        self.task_id = (!task_id.trim().is_empty()).then_some(task_id);
        self
    }

    pub fn with_pending_reason(mut self, pending_reason: AgentToolPendingReason) -> Self {
        self.pending_reason = Some(pending_reason);
        self
    }

    pub fn with_check_after(mut self, check_after: u64) -> Self {
        self.check_after = Some(check_after);
        self
    }

    pub fn with_partial_output(mut self, partial_output: impl Into<String>) -> Self {
        let partial_output = partial_output.into();
        self.partial_output = (!partial_output.trim().is_empty()).then_some(partial_output);
        self
    }

    pub fn with_estimated_wait(mut self, estimated_wait: impl Into<String>) -> Self {
        let estimated_wait = estimated_wait.into();
        self.estimated_wait = (!estimated_wait.trim().is_empty()).then_some(estimated_wait);
        self
    }

    pub fn with_command_metadata(
        mut self,
        cmd_name: impl Into<String>,
        cmd_args: impl Into<String>,
    ) -> Self {
        let cmd_name = cmd_name.into();
        let cmd_args = cmd_args.into();
        self.cmd_name = (!cmd_name.trim().is_empty()).then_some(cmd_name);
        self.cmd_args = (!cmd_args.trim().is_empty()).then_some(cmd_args);
        self
    }

    pub fn with_command_metadata_from_line(mut self, cmd_line: &str) -> Self {
        if self.cmd_name.is_some() || self.cmd_args.is_some() {
            return self;
        }

        let tokens = tokenize_bash_command_line(cmd_line)
            .ok()
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| {
                cmd_line
                    .split_whitespace()
                    .map(|item| item.to_string())
                    .collect()
            });
        if let Some(first) = tokens.first() {
            self.cmd_name = Some(first.clone());
            if tokens.len() > 1 {
                self.cmd_args = Some(tokens[1..].join(" "));
            }
        }
        self
    }

    pub fn render_prompt(&self) -> String {
        let mut lines = Vec::<String>::new();
        let cmd_line = self.command_line_text();
        let summary = match (
            cmd_line.as_deref().unwrap_or_default().trim().is_empty(),
            Some(self.summary.trim()).filter(|value| !value.is_empty()),
        ) {
            (false, Some(result)) => format!(
                "{} => {}",
                cmd_line.as_deref().unwrap_or_default().trim(),
                result
            ),
            (false, None) => cmd_line.unwrap_or_default().trim().to_string(),
            (true, Some(result)) => result.to_string(),
            (true, None) => compact_json_text(&self.details, 220),
        };
        lines.push(summary);

        if let Some(output) = self
            .output
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push("```output".to_string());
            lines.push(truncate_prompt_stream_lines(output, PROMPT_STDIO_MAX_LINES));
            lines.push("```".to_string());
        }
        lines.join("\n")
    }

    pub fn render_for_level(&self, level: AgentHistoryShowLevel) -> String {
        if self.is_agent_tool {
            self.render_agent_tool_for_level(level)
        } else {
            self.render_bash_result_for_level(level)
        }
    }

    pub fn command_line_text(&self) -> Option<String> {
        self.cmd_name.as_ref().map(|cmd_name| match self
            .cmd_args
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(cmd_args) => format!("{cmd_name} {cmd_args}"),
            None => cmd_name.clone(),
        })
    }

    fn render_agent_tool_for_level(&self, level: AgentHistoryShowLevel) -> String {
        match level {
            AgentHistoryShowLevel::Min => {
                self.render_command_with_status(self.history_compact_command_text())
            }
            AgentHistoryShowLevel::Mini | AgentHistoryShowLevel::Medium => {
                let summary = self.summary.trim();
                if summary.is_empty() {
                    self.render_command_with_status(self.history_compact_command_text())
                } else {
                    summary.to_string()
                }
            }
            AgentHistoryShowLevel::Full => {
                let command = self
                    .history_compact_command_text()
                    .or_else(|| self.command_line_text())
                    .unwrap_or_else(|| "action".to_string());
                let mut lines = vec![command];
                let mut body = vec![self.history_result_text()];
                if let Some(details) = self.render_agent_tool_details_block() {
                    body.push(details);
                }
                lines.push("```result".to_string());
                lines.push(body.join("\n"));
                lines.push("```".to_string());
                lines.join("\n")
            }
        }
    }

    fn render_bash_result_for_level(&self, level: AgentHistoryShowLevel) -> String {
        let command = match level {
            AgentHistoryShowLevel::Min | AgentHistoryShowLevel::Mini => {
                self.history_compact_command_text()
            }
            AgentHistoryShowLevel::Medium | AgentHistoryShowLevel::Full => {
                self.command_line_text()
                    .or_else(|| self.history_compact_command_text())
            }
        };
        let mut lines = vec![self.render_command_with_status(command)];

        let excerpt = match level {
            AgentHistoryShowLevel::Min => None,
            AgentHistoryShowLevel::Mini => match self.status {
                AgentToolStatus::Error => self.render_output_excerpt(false, HISTORY_BASH_OUTPUT_MINI_LINES),
                _ => None,
            },
            AgentHistoryShowLevel::Medium => match self.status {
                AgentToolStatus::Error => self.render_output_excerpt(false, HISTORY_BASH_OUTPUT_MINI_LINES),
                AgentToolStatus::Success => self.render_output_excerpt(true, HISTORY_BASH_OUTPUT_MINI_LINES),
                AgentToolStatus::Pending => None,
            },
            AgentHistoryShowLevel::Full => match self.status {
                AgentToolStatus::Error => self.render_output_excerpt(false, HISTORY_BASH_OUTPUT_FULL_LINES),
                AgentToolStatus::Success => self.render_output_excerpt(true, HISTORY_BASH_OUTPUT_FULL_LINES),
                AgentToolStatus::Pending => None,
            },
        };

        if let Some(excerpt) = excerpt {
            lines.push("```output".to_string());
            lines.push(excerpt);
            lines.push("```".to_string());
        }
        lines.join("\n")
    }

    fn history_compact_command_text(&self) -> Option<String> {
        let cmd = self.command_line_text()?;
        Some(truncate_text(cmd.trim(), HISTORY_COMPACT_CMD_MAX_CHARS))
    }

    fn render_command_with_status(&self, command: Option<String>) -> String {
        let status = self.history_result_text();
        match command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(command) => format!("{command} => {status}"),
            None => status,
        }
    }

    fn history_result_text(&self) -> String {
        match self.status {
            AgentToolStatus::Success => "success".to_string(),
            AgentToolStatus::Pending => match self.history_pending_reason_text() {
                Some(reason) => format!("pending ({reason})"),
                None => "pending".to_string(),
            },
            AgentToolStatus::Error => match self.history_error_reason_text() {
                Some(reason) => format!("failed ({reason})"),
                None => "failed".to_string(),
            },
        }
    }

    fn history_pending_reason_text(&self) -> Option<String> {
        self.pending_reason
            .map(history_pending_reason_label)
            .map(str::to_string)
    }

    fn history_error_reason_text(&self) -> Option<String> {
        self.output
            .as_deref()
            .and_then(last_non_empty_line)
            .map(|value| collapse_inline_whitespace(value, 120))
            .or_else(|| self.return_code.map(|code| format!("exit={code}")))
            .or_else(|| {
                let summary = self.summary.trim();
                (!summary.is_empty()).then(|| collapse_inline_whitespace(summary, 120))
            })
    }

    fn render_agent_tool_details_block(&self) -> Option<String> {
        match &self.details {
            Json::Null => None,
            Json::Object(map) if map.is_empty() => None,
            value => Some(truncate_text(
                serde_json::to_string_pretty(value)
                    .unwrap_or_else(|_| compact_json_text(value, HISTORY_STD_DETAILS_MAX_CHARS))
                    .as_str(),
                HISTORY_STD_DETAILS_MAX_CHARS,
            )),
        }
    }

    fn render_output_excerpt(&self, head: bool, max_lines: usize) -> Option<String> {
        let output = self
            .output
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if head {
            Some(take_head_lines(output, max_lines))
        } else {
            Some(take_tail_lines(output, max_lines))
        }
    }
}

impl Deref for AgentToolResult {
    type Target = Json;

    fn deref(&self) -> &Self::Target {
        &self.details
    }
}

impl std::fmt::Display for AgentToolResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.render_prompt())
    }
}

pub(crate) fn build_builtin_tool_result(
    details: Json,
    cmd_line: impl Into<String>,
    summary: impl Into<String>,
) -> AgentToolResult {
    AgentToolResult::from_details(details)
        .with_is_agent_tool(true)
        .with_cmd_line(cmd_line)
        .with_result(summary)
}

fn history_pending_reason_label(reason: AgentToolPendingReason) -> &'static str {
    match reason {
        AgentToolPendingReason::LongRunning => "long_running",
        AgentToolPendingReason::UserApproval => "user_approval",
        AgentToolPendingReason::WaitForInstall => "wait_for_install",
    }
}

fn collapse_inline_whitespace(input: &str, max_chars: usize) -> String {
    let collapsed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text(collapsed.as_str(), max_chars)
}

fn last_non_empty_line(input: &str) -> Option<&str> {
    input
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

fn take_head_lines(content: &str, max_lines: usize) -> String {
    if max_lines == 0 {
        return String::new();
    }
    let lines = content.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return content.to_string();
    }
    let mut out = lines[..max_lines].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(
        format!("... [TRUNCATED: showing first {max_lines} lines only] ...").as_str(),
    );
    out
}

fn take_tail_lines(content: &str, max_lines: usize) -> String {
    if max_lines == 0 {
        return String::new();
    }
    let lines = content.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return content.to_string();
    }
    let start = lines.len().saturating_sub(max_lines);
    let mut out = String::from("... [TRUNCATED: showing last ");
    out.push_str(max_lines.to_string().as_str());
    out.push_str(" lines only] ...\n");
    out.push_str(lines[start..].join("\n").as_str());
    out
}

impl CliResultEnvelope {
    pub fn from_tool_result(tool_name: &str, result: AgentToolResult) -> Self {
        let detail = result.details.clone();
        Self {
            is_agent_tool: true,
            status: result.status.into(),
            summary: if result.summary.trim().is_empty() {
                "completed".to_string()
            } else {
                result.summary.clone()
            },
            tool: Some(tool_name.to_string()),
            cmd_line: result.command_line_text(),
            detail,
            output: result.output,
            return_code: result.return_code,
            pending_reason: result.pending_reason.map(Into::into),
            task_id: result.task_id,
            estimated_wait: result.estimated_wait,
            check_after: result.check_after,
        }
    }

    pub fn error(tool_name: Option<&str>, err: &AgentToolError) -> Self {
        let message = err.to_string();
        Self {
            is_agent_tool: true,
            status: CliStatus::Error,
            summary: message.clone(),
            tool: tool_name.map(|value| value.to_string()),
            cmd_line: None,
            detail: json!({}),
            output: Some(message.clone()),
            return_code: None,
            pending_reason: None,
            task_id: None,
            estimated_wait: None,
            check_after: None,
        }
    }

    pub fn success(tool: Option<String>, detail: Json, summary: impl Into<String>) -> Self {
        Self {
            is_agent_tool: true,
            status: CliStatus::Success,
            summary: summary.into(),
            tool,
            cmd_line: None,
            detail,
            output: None,
            return_code: None,
            pending_reason: None,
            task_id: None,
            estimated_wait: None,
            check_after: None,
        }
    }

    pub fn into_tool_result(self) -> AgentToolResult {
        let Self {
            is_agent_tool,
            status,
            summary,
            tool: _tool,
            cmd_line,
            detail,
            output,
            return_code,
            pending_reason,
            task_id,
            estimated_wait,
            check_after,
        } = self;
        let mut result = AgentToolResult::from_details(detail)
            .with_is_agent_tool(is_agent_tool)
            .with_status(status.into())
            .with_result(summary);
        if let Some(cmd_line) = cmd_line.as_deref() {
            result = result.with_command_metadata_from_line(cmd_line);
        }
        result.output = result.output.or(output);
        result.return_code = result.return_code.or(return_code);
        result.task_id = task_id;
        result.check_after = check_after;
        result.pending_reason = pending_reason.map(Into::into);
        result.estimated_wait = estimated_wait;
        result
    }
}

pub fn render_cli_output(payload: &CliResultEnvelope, exit_code: i32) -> CliRunOutput {
    let stdout = serde_json::to_string(&payload.clone().into_tool_result()).unwrap_or_else(|_| {
        "{\"status\":\"error\",\"summary\":\"serialize cli result failed\",\"detail\":{}}"
            .to_string()
    });
    CliRunOutput {
        exit_code,
        stdout,
        stderr: String::new(),
    }
}

pub fn cli_exit_code_for_error(err: &AgentToolError) -> i32 {
    match err {
        AgentToolError::InvalidArgs(_) | AgentToolError::NotFound(_) => CLI_EXIT_USAGE,
        AgentToolError::AlreadyExists(_)
        | AgentToolError::ExecFailed(_)
        | AgentToolError::Timeout => CLI_EXIT_ERROR,
    }
}

impl From<AgentToolStatus> for CliStatus {
    fn from(value: AgentToolStatus) -> Self {
        match value {
            AgentToolStatus::Success => CliStatus::Success,
            AgentToolStatus::Error => CliStatus::Error,
            AgentToolStatus::Pending => CliStatus::Pending,
        }
    }
}

impl From<CliStatus> for AgentToolStatus {
    fn from(value: CliStatus) -> Self {
        match value {
            CliStatus::Success => AgentToolStatus::Success,
            CliStatus::Error => AgentToolStatus::Error,
            CliStatus::Pending => AgentToolStatus::Pending,
        }
    }
}

impl From<AgentToolPendingReason> for CliPendingReason {
    fn from(value: AgentToolPendingReason) -> Self {
        match value {
            AgentToolPendingReason::LongRunning => CliPendingReason::LongRunning,
            AgentToolPendingReason::UserApproval => CliPendingReason::UserApproval,
            AgentToolPendingReason::WaitForInstall => CliPendingReason::WaitForInstall,
        }
    }
}

impl From<CliPendingReason> for AgentToolPendingReason {
    fn from(value: CliPendingReason) -> Self {
        match value {
            CliPendingReason::LongRunning => AgentToolPendingReason::LongRunning,
            CliPendingReason::UserApproval => AgentToolPendingReason::UserApproval,
            CliPendingReason::WaitForInstall => AgentToolPendingReason::WaitForInstall,
        }
    }
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    fn support_bash(&self) -> bool;
    fn support_action(&self) -> bool;
    fn support_llm_tool_call(&self) -> bool;

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError>;

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        _shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "empty bash command line".to_string(),
            ));
        }
        let args = parse_default_bash_exec_args(&tokens[1..])?;
        self.call(ctx, args).await
    }
}

#[async_trait]
pub trait SessionViewBackend: Send + Sync {
    async fn session_view(&self, session_id: &str) -> Result<Json, AgentToolError>;
}

#[derive(Clone)]
pub struct GetSessionTool {
    backend: Arc<dyn SessionViewBackend>,
}

impl GetSessionTool {
    pub fn new(backend: Arc<dyn SessionViewBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl AgentTool for GetSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_GET_SESSION.to_string(),
            description:
                "Read current session state and status. Used by runtime before each LLM round."
                    .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" }
                },
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "session": { "type": "object" }
                }
            }),
            usage: Some("get_session [session_id]".to_string()),
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let session_id = args
            .get("session_id")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| ctx.session_id.trim());
        if session_id.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is required".to_string(),
            ));
        }
        let session = self.backend.session_view(session_id).await?;
        Ok(build_builtin_tool_result(
            json!({
            "ok": true,
            "session": session
            }),
            format!("{TOOL_GET_SESSION} {session_id}"),
            "ok",
        ))
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        _shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "empty bash command line".to_string(),
            ));
        }
        if tokens.len() == 1 {
            return self.call(ctx, json!({})).await;
        }
        if tokens.len() == 2 && !tokens[1].contains('=') {
            return self
                .call(ctx, json!({ "session_id": tokens[1].trim() }))
                .await;
        }
        let args = parse_default_bash_exec_args(&tokens[1..])?;
        self.call(ctx, args).await
    }
}

#[derive(Clone, Debug)]
pub struct MemoryLoadPreview {
    pub rendered: String,
    pub item_count: usize,
}

#[async_trait]
pub trait MemoryLoadBackend: Send + Sync {
    async fn load_memory_preview(
        &self,
        token_limit: Option<u32>,
        tags: Vec<String>,
        current_time: Option<String>,
    ) -> Result<MemoryLoadPreview, AgentToolError>;
}

#[derive(Clone)]
pub struct LoadMemoryTool {
    backend: Arc<dyn MemoryLoadBackend>,
}

impl LoadMemoryTool {
    pub fn new(backend: Arc<dyn MemoryLoadBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl AgentTool for LoadMemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LOAD_MEMORY.to_string(),
            description: "Read memory summary using default retrieval strategy.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "token_limit": {"type":"number"},
                    "tags": {
                        "type":"array",
                        "items": {"type":"string"}
                    },
                    "current_time": {"type":"string"}
                }
            }),
            output_schema: json!({
                "type":"string"
            }),
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        true
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let token_limit = args
            .get("token_limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(u32::MAX as u64) as u32);
        let tags = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        let current_time = args
            .get("current_time")
            .and_then(|v| v.as_str())
            .map(|raw| raw.to_string());

        let preview = self
            .backend
            .load_memory_preview(token_limit, tags, current_time)
            .await?;
        Ok(build_builtin_tool_result(
            Json::String(preview.rendered),
            TOOL_LOAD_MEMORY.to_string(),
            format!("loaded {} memory item(s)", preview.item_count),
        ))
    }
}

#[async_trait]
pub trait WorklogActionBackend: Send + Sync {
    async fn execute_action(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<Json, AgentToolError>;
}

#[derive(Clone)]
pub struct WorklogTool {
    backend: Arc<dyn WorklogActionBackend>,
}

impl WorklogTool {
    pub fn new(backend: Arc<dyn WorklogActionBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
pub trait WorkspaceToolBackend: Send + Sync {
    async fn create_workspace(
        &self,
        ctx: &SessionRuntimeContext,
        name: String,
        summary: String,
    ) -> Result<Json, AgentToolError>;

    async fn resolve_workspace_id(
        &self,
        workspace_ref: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<String, AgentToolError>;

    async fn bind_workspace(
        &self,
        ctx: &SessionRuntimeContext,
        session_id: &str,
        workspace_id: &str,
    ) -> Result<Json, AgentToolError>;
}

#[derive(Clone)]
pub struct CreateWorkspaceTool {
    backend: Arc<dyn WorkspaceToolBackend>,
}

impl CreateWorkspaceTool {
    pub fn new(backend: Arc<dyn WorkspaceToolBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl AgentTool for CreateWorkspaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CREATE_WORKSPACE.to_string(),
            description: "创建session的wrokspace并设置为session的default workspace".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "summary": { "type": "string" }
                },
                "required": ["name", "summary"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "workspace": { "type": "object" },
                    "binding": { "type": "object" },
                    "summary_path": { "type": "string" },
                    "session_id": { "type": "string" },
                    "session_updated": { "type": "boolean" }
                }
            }),
            usage: Some("create_workspace <name> <summary>".to_string()),
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        _args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        Err(AgentToolError::InvalidArgs(
            "not support: create_workspace only supports bash mode".to_string(),
        ))
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        _shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.len() < 3 {
            return Err(AgentToolError::InvalidArgs(
                "missing required arguments: <name> <summary>".to_string(),
            ));
        }
        if tokens.len() > 3 {
            return Err(AgentToolError::InvalidArgs(
                "create_workspace only supports arguments: <name> <summary>".to_string(),
            ));
        }

        let name = tokens[1].trim();
        if name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace name cannot be empty".to_string(),
            ));
        }
        let summary = tokens[2].trim();
        if summary.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace summary cannot be empty".to_string(),
            ));
        }

        self.backend
            .create_workspace(ctx, name.to_string(), summary.to_string())
            .await
            .map(|details| build_builtin_tool_result(details, line.trim().to_string(), "ok"))
    }
}

#[derive(Clone)]
pub struct BindWorkspaceTool {
    backend: Arc<dyn WorkspaceToolBackend>,
}

impl BindWorkspaceTool {
    pub fn new(backend: Arc<dyn WorkspaceToolBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
pub trait ExternalWorkspaceBackend: Send + Sync {
    async fn bind_external_workspace(
        &self,
        agent_did: &str,
        name: &str,
        workspace_path: &str,
    ) -> Result<Json, AgentToolError>;

    async fn list_external_workspaces(&self, agent_did: &str) -> Result<Json, AgentToolError>;
}

#[derive(Clone)]
pub struct BindExternalWorkspaceTool {
    backend: Arc<dyn ExternalWorkspaceBackend>,
}

impl BindExternalWorkspaceTool {
    pub fn new(backend: Arc<dyn ExternalWorkspaceBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl AgentTool for BindExternalWorkspaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_BIND_EXTERNAL_WORKSPACE.to_string(),
            description:
                "Bind an external workspace directory so this agent can access it from runtime."
                    .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Local mount name." },
                    "workspace_path": { "type": "string", "description": "Absolute or relative source workspace path." },
                    "agent_did": { "type": "string", "description": "Optional target agent DID. Defaults to current agent DID." }
                },
                "required": ["name", "workspace_path"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "binding": { "type": "object" }
                }
            }),
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let agent_did =
            optional_trimmed_string_arg(&args, "agent_did")?.unwrap_or(ctx.agent_name.clone());
        let name = require_trimmed_string_arg(&args, "name")?;
        let workspace_path = require_trimmed_string_arg(&args, "workspace_path")?;
        let binding = self
            .backend
            .bind_external_workspace(agent_did.as_str(), name.as_str(), workspace_path.as_str())
            .await?;

        Ok(build_builtin_tool_result(
            json!({
                "ok": true,
                "binding": binding
            }),
            TOOL_BIND_EXTERNAL_WORKSPACE.to_string(),
            "ok",
        ))
    }
}

#[derive(Clone)]
pub struct ListExternalWorkspacesTool {
    backend: Arc<dyn ExternalWorkspaceBackend>,
}

impl ListExternalWorkspacesTool {
    pub fn new(backend: Arc<dyn ExternalWorkspaceBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl AgentTool for ListExternalWorkspacesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LIST_EXTERNAL_WORKSPACES.to_string(),
            description: "List bound external workspaces visible to current agent.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "agent_did": { "type": "string", "description": "Optional agent DID. Defaults to current agent DID." }
                },
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "workspaces": { "type": "array", "items": { "type": "object" } }
                }
            }),
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let agent_did =
            optional_trimmed_string_arg(&args, "agent_did")?.unwrap_or(ctx.agent_name.clone());
        let workspaces = self
            .backend
            .list_external_workspaces(agent_did.as_str())
            .await?;
        Ok(build_builtin_tool_result(
            json!({
                "ok": true,
                "workspaces": workspaces
            }),
            TOOL_LIST_EXTERNAL_WORKSPACES.to_string(),
            "ok",
        ))
    }
}

#[async_trait]
impl AgentTool for BindWorkspaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_BIND_WORKSPACE.to_string(),
            description: "设置agent_session的当前workspace".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "workspace": { "type": "string" }
                },
                "required": ["workspace"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "binding": { "type": "object" },
                    "session_id": { "type": "string" },
                    "session_updated": { "type": "boolean" }
                }
            }),
            usage: Some("bind_workspace <workspace_id|workspace_path>".to_string()),
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        _args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        Err(AgentToolError::InvalidArgs(
            "not support: bind_workspace only supports bash mode".to_string(),
        ))
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.len() < 2 {
            return Err(AgentToolError::InvalidArgs(
                "missing workspace argument".to_string(),
            ));
        }
        if tokens.len() > 2 {
            return Err(AgentToolError::InvalidArgs(
                "bind_workspace only supports one argument: <workspace_id|workspace_path>"
                    .to_string(),
            ));
        }

        let raw_arg = tokens[1].trim();
        let workspace_ref = if let Some((key, value)) = raw_arg.split_once('=') {
            match key.trim() {
                "workspace" | "workspace_id" | "workspace_path" | "local_workspace_id" => {
                    value.trim()
                }
                other => {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "unsupported argument `{other}`; expected workspace/workspace_id/workspace_path"
                    )));
                }
            }
        } else {
            raw_arg
        };

        if workspace_ref.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace argument cannot be empty".to_string(),
            ));
        }
        if ctx.session_id.trim().is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is required".to_string(),
            ));
        }

        let workspace_id = self
            .backend
            .resolve_workspace_id(workspace_ref, shell_cwd)
            .await?;
        self.backend
            .bind_workspace(ctx, ctx.session_id.as_str(), workspace_id.as_str())
            .await
            .map(|details| build_builtin_tool_result(details, line.trim().to_string(), "ok"))
    }
}

#[async_trait]
impl AgentTool for WorklogTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_WORKLOG_MANAGE.to_string(),
            description: "Structured workspace worklog with event records, step summary and prompt-safe rendering.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "append_worklog",
                            "append_step_summary",
                            "mark_step_committed",
                            "list_worklog",
                            "get_worklog",
                            "list_step",
                            "build_prompt_worklog",
                            "append",
                            "list",
                            "get",
                            "render_for_prompt"
                        ]
                    },
                    "record": { "type": "object" },
                    "log_id": { "type": "string" },
                    "id": { "type": "string" },
                    "step_id": { "type": "string" },
                    "owner_session_id": { "type": "string" },
                    "workspace_id": { "type": "string" },
                    "todo_id": { "type": "string" },
                    "type": {
                        "type": "string",
                        "enum": [
                            "GetMessage",
                            "ReplyMessage",
                            "FunctionRecord",
                            "ActionRecord",
                            "CreateSubAgent",
                            "StepSummary"
                        ]
                    },
                    "status": { "type": "string" },
                    "tag": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "token_budget": { "type": "integer", "minimum": 1 }
                },
                "required": ["action"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "action": { "type": "string" },
                    "record": { "type": "object" },
                    "records": { "type": "array", "items": { "type": "object" } },
                    "total": { "type": "integer" },
                    "text": { "type": "string" },
                    "prompt_text": { "type": "string" },
                    "updated": { "type": "integer" }
                }
            }),
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let details = self.backend.execute_action(ctx, args).await?;
        let action = details
            .get("action")
            .and_then(Json::as_str)
            .unwrap_or("worklog")
            .to_string();
        Ok(build_builtin_tool_result(
            details,
            TOOL_WORKLOG_MANAGE.to_string(),
            action,
        ))
    }
}

pub struct MCPTool {
    spec: ToolSpec,
    endpoint: String,
    mcp_tool_name: String,
    headers: HashMap<String, String>,
    timeout_ms: u64,
    client: reqwest::Client,
}

impl MCPTool {
    pub fn new(cfg: MCPToolConfig) -> Result<Self, AgentToolError> {
        let tool_name = cfg.name.trim();
        if tool_name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "mcp tool `name` cannot be empty".to_string(),
            ));
        }

        let endpoint = cfg.endpoint.trim();
        if endpoint.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "mcp tool `endpoint` cannot be empty".to_string(),
            ));
        }

        if cfg.timeout_ms == 0 {
            return Err(AgentToolError::InvalidArgs(
                "mcp tool `timeout_ms` must be > 0".to_string(),
            ));
        }

        let mcp_tool_name = cfg
            .mcp_tool_name
            .unwrap_or_else(|| tool_name.to_string())
            .trim()
            .to_string();
        if mcp_tool_name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "mcp tool `mcp_tool_name` cannot be empty".to_string(),
            ));
        }

        let description = cfg
            .description
            .unwrap_or_else(|| format!("MCP tool `{}`", mcp_tool_name));

        let client = reqwest::Client::builder().build().map_err(|err| {
            AgentToolError::ExecFailed(format!("build mcp http client failed: {err}"))
        })?;

        Ok(Self {
            spec: ToolSpec {
                name: tool_name.to_string(),
                description,
                args_schema: cfg.args_schema,
                output_schema: cfg.output_schema,
                usage: None,
            },
            endpoint: endpoint.to_string(),
            mcp_tool_name,
            headers: cfg.headers,
            timeout_ms: cfg.timeout_ms,
            client,
        })
    }
}

#[async_trait]
impl AgentTool for MCPTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        true
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": format!(
                "{}:{}:{}:{}:{}",
                ctx.trace_id, ctx.wakeup_id, ctx.behavior, ctx.step_idx, self.spec.name
            ),
            "method": "tools/call",
            "params": {
                "name": self.mcp_tool_name,
                "arguments": args
            }
        });

        let mut req = self.client.post(&self.endpoint).json(&request_body);
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }

        let response = timeout(Duration::from_millis(self.timeout_ms), req.send())
            .await
            .map_err(|_| AgentToolError::Timeout)?
            .map_err(|err| AgentToolError::ExecFailed(format!("mcp request failed: {err}")))?;

        let status = response.status();
        let body = timeout(Duration::from_millis(self.timeout_ms), response.text())
            .await
            .map_err(|_| AgentToolError::Timeout)?
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("read mcp response failed: {err}"))
            })?;

        if !status.is_success() {
            return Err(AgentToolError::ExecFailed(format!(
                "mcp server returned http {}: {}",
                status.as_u16(),
                truncate_text(&body, 512)
            )));
        }

        let payload: Json = serde_json::from_str(&body).map_err(|err| {
            AgentToolError::ExecFailed(format!("invalid mcp response json: {err}"))
        })?;

        if let Some(err_obj) = payload.get("error") {
            let msg = extract_jsonrpc_error_message(err_obj);
            return Err(AgentToolError::ExecFailed(format!(
                "mcp tool call error: {msg}"
            )));
        }

        let result = payload.get("result").cloned().ok_or_else(|| {
            AgentToolError::ExecFailed("mcp response missing `result` field".to_string())
        })?;

        if let Some(message) = extract_mcp_result_error(&result) {
            return Err(AgentToolError::ExecFailed(format!(
                "mcp tool returned error: {message}"
            )));
        }

        Ok(AgentToolResult::from_details(result)
            .with_cmd_line(self.spec.name.clone())
            .with_result("OK"))
    }
}

fn extract_jsonrpc_error_message(value: &Json) -> String {
    if let Some(msg) = value.get("message").and_then(|v| v.as_str()) {
        return msg.to_string();
    }
    truncate_text(&value.to_string(), 512)
}

fn extract_mcp_result_error(result: &Json) -> Option<String> {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_error {
        return None;
    }

    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        for item in content {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                if !text.trim().is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }

    Some(truncate_text(&result.to_string(), 512))
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect::<String>() + "...[TRUNCATED]"
}

fn compact_json_text(value: &Json, max_chars: usize) -> String {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    truncate_text(rendered.as_str(), max_chars)
}

struct RegisteredTool {
    spec: ToolSpec,
    inner: Arc<dyn AgentTool>,
    support_bash: bool,
    support_action: bool,
    support_llm_tool_call: bool,
}

#[async_trait]
impl AgentTool for RegisteredTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn support_bash(&self) -> bool {
        self.support_bash
    }

    fn support_action(&self) -> bool {
        self.support_action
    }

    fn support_llm_tool_call(&self) -> bool {
        self.support_llm_tool_call
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        self.inner.call(ctx, args).await
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        self.inner.exec(ctx, line, shell_cwd).await
    }
}

#[derive(Default)]
struct ToolNamespaceRegistry {
    all_tools: HashMap<String, Arc<dyn AgentTool>>,
    llm_tools: HashMap<String, Arc<dyn AgentTool>>,
    bash_cmds: HashMap<String, Arc<dyn AgentTool>>,
}

#[derive(Clone)]
pub struct AgentToolManager {
    namespaces: Arc<StdRwLock<ToolNamespaceRegistry>>,
}

impl Default for AgentToolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentToolManager {
    pub fn new() -> Self {
        Self {
            namespaces: Arc::new(StdRwLock::new(ToolNamespaceRegistry::default())),
        }
    }

    pub fn register_tool<T>(&self, tool: T) -> Result<(), AgentToolError>
    where
        T: AgentTool + 'static,
    {
        self.register_tool_arc(Arc::new(tool))
    }

    pub fn register_tool_arc(&self, tool: Arc<dyn AgentTool>) -> Result<(), AgentToolError> {
        let mut spec = tool.spec();
        let original_name = spec.name.trim().to_string();
        if original_name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "tool name cannot be empty".to_string(),
            ));
        }
        let normalized_name = normalize_tool_name(original_name.as_str());
        if normalized_name.is_empty() {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool name `{}` is invalid after normalization",
                original_name
            )));
        }
        spec.name = normalized_name.clone();
        spec.usage = spec
            .usage
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let support_bash = tool.support_bash();
        let support_action = tool.support_action();
        let support_llm_tool_call = tool.support_llm_tool_call();
        if !support_bash && !support_action && !support_llm_tool_call {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` must support at least one namespace",
                normalized_name
            )));
        }

        let registered: Arc<dyn AgentTool> = Arc::new(RegisteredTool {
            spec,
            inner: tool,
            support_bash,
            support_action,
            support_llm_tool_call,
        });

        let mut guard = self
            .namespaces
            .write()
            .map_err(|_| AgentToolError::ExecFailed("tool namespace lock poisoned".to_string()))?;
        if guard.all_tools.contains_key(&normalized_name) {
            return Err(AgentToolError::AlreadyExists(normalized_name));
        }
        guard
            .all_tools
            .insert(normalized_name.clone(), registered.clone());
        if support_llm_tool_call {
            guard
                .llm_tools
                .insert(normalized_name.clone(), registered.clone());
        }
        if support_bash {
            guard
                .bash_cmds
                .insert(normalized_name.clone(), registered.clone());
        }
        if normalized_name != original_name {
            warn!(
                "tool name normalized by trimming: original={} normalized={}",
                original_name, normalized_name
            );
        }
        Ok(())
    }

    pub fn register_mcp_tool(&self, cfg: MCPToolConfig) -> Result<(), AgentToolError> {
        self.register_tool(MCPTool::new(cfg)?)
    }

    pub fn unregister_tool(&self, name: &str) -> bool {
        let normalized_name = normalize_tool_name(name);
        if normalized_name.is_empty() {
            return false;
        }
        let Ok(mut guard) = self.namespaces.write() else {
            return false;
        };
        let removed = guard.all_tools.remove(normalized_name.as_str()).is_some();
        if !removed {
            return false;
        }

        guard.llm_tools.remove(normalized_name.as_str());
        guard.bash_cmds.remove(normalized_name.as_str());
        true
    }

    pub fn has_tool(&self, name: &str) -> bool {
        let Ok(guard) = self.namespaces.read() else {
            return false;
        };
        guard.llm_tools.contains_key(name)
    }

    pub fn get_tool(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        let Ok(guard) = self.namespaces.read() else {
            return None;
        };
        guard.llm_tools.get(name).cloned()
    }

    pub fn get_bash_cmd(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        let Ok(guard) = self.namespaces.read() else {
            return None;
        };
        guard.bash_cmds.get(name).cloned()
    }

    pub fn get_action(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        let Ok(guard) = self.namespaces.read() else {
            return None;
        };
        guard
            .all_tools
            .get(name)
            .filter(|tool| tool.support_action())
            .cloned()
    }

    pub fn get_tool_spec(&self, name: &str) -> Option<ToolSpec> {
        self.get_tool(name).map(|tool| tool.spec())
    }

    pub fn list_tool_specs(&self) -> Vec<ToolSpec> {
        let Ok(guard) = self.namespaces.read() else {
            return vec![];
        };
        let mut specs: Vec<ToolSpec> = guard.llm_tools.values().map(|tool| tool.spec()).collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn list_bash_cmd_specs(&self) -> Vec<ToolSpec> {
        let Ok(guard) = self.namespaces.read() else {
            return vec![];
        };
        let mut specs: Vec<ToolSpec> = guard.bash_cmds.values().map(|tool| tool.spec()).collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn list_action_tool_specs(&self) -> Vec<ToolSpec> {
        let Ok(guard) = self.namespaces.read() else {
            return vec![];
        };
        let mut specs: Vec<ToolSpec> = guard
            .all_tools
            .values()
            .filter(|tool| tool.support_action())
            .map(|tool| tool.spec())
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn parse_bash_command_name(line: &str) -> Option<String> {
        let tokens = tokenize_bash_command_line(line).ok()?;
        let first = tokens.first()?.trim();
        if first.is_empty() {
            return None;
        }
        Some(first.to_string())
    }

    pub fn resolve_bash_registered_tool_name(&self, line: &str) -> Option<String> {
        let raw_name = Self::parse_bash_command_name(line)?;
        let normalized = normalize_tool_name(raw_name.as_str());
        if normalized.is_empty() {
            return None;
        }
        let Ok(guard) = self.namespaces.read() else {
            return None;
        };
        guard
            .bash_cmds
            .contains_key(normalized.as_str())
            .then_some(normalized)
    }

    pub async fn call_tool_from_bash_line(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
    ) -> Result<Option<AgentToolResult>, AgentToolError> {
        self.call_tool_from_bash_line_with_cwd(ctx, line, None)
            .await
    }

    pub async fn call_tool_from_bash_line_with_cwd(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<Option<AgentToolResult>, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Ok(None);
        }

        let tool_name = normalize_tool_name(tokens[0].as_str());
        if tool_name.is_empty() {
            return Ok(None);
        }
        let Some(tool) = self.get_bash_cmd(tool_name.as_str()) else {
            return Ok(None);
        };
        let spec = tool.spec();
        let usage = render_bash_tool_usage(&spec);
        if is_help_flag(&tokens[1..]) {
            return Ok(Some(build_builtin_tool_result(
                json!({
                    "ok": true,
                    "tool": tool_name,
                    "usage": usage,
                    "args_schema": spec.args_schema
                }),
                line.trim().to_string(),
                "show usage",
            )));
        }

        let call_id = format!("bash-cli-{}-{}", ctx.trace_id, ctx.step_idx);
        info!(
            "opendan.tool_call: status=start tool={} call_id={} trace_id={} session_id={} source=bash",
            tool_name, call_id, ctx.trace_id, ctx.session_id
        );
        let result = tool.exec(ctx, line, shell_cwd).await;
        if let Err(err) = &result {
            warn!(
                "opendan.tool_call: status=failed tool={} call_id={} trace_id={} session_id={} source=bash err={}",
                tool_name, call_id, ctx.trace_id, ctx.session_id, err
            );
        }
        let result = result.map_err(|err| {
            if let Some(usage) = usage.as_deref() {
                append_usage_on_invalid_args(err, usage)
            } else {
                err
            }
        })?;
        info!(
            "opendan.tool_call: status=success tool={} call_id={} trace_id={} session_id={} source=bash",
            tool_name, call_id, ctx.trace_id, ctx.session_id
        );
        Ok(Some(result))
    }

    pub async fn call_tool(
        &self,
        ctx: &SessionRuntimeContext,
        call: AiToolCall,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tool_name = call.name;
        let call_id = call.call_id;
        let args = Json::Object(call.args.into_iter().collect());
        let session_id = ctx.session_id.as_str();

        info!(
            "opendan.tool_call: status=start tool={} call_id={} trace_id={} session_id={}",
            tool_name, call_id, ctx.trace_id, session_id
        );

        let Some(tool) = self.get_registered_tool(&tool_name) else {
            warn!(
                "opendan.tool_call: status=failed tool={} call_id={} trace_id={} session_id={} err=tool not found",
                tool_name, call_id, ctx.trace_id, session_id
            );
            return Err(AgentToolError::NotFound(tool_name));
        };

        let result = tool.call(ctx, args).await;
        match &result {
            Ok(_) => info!(
                "opendan.tool_call: status=success tool={} call_id={} trace_id={} session_id={}",
                tool_name, call_id, ctx.trace_id, session_id
            ),
            Err(err) => warn!(
                "opendan.tool_call: status=failed tool={} call_id={} trace_id={} session_id={} err={}",
                tool_name, call_id, ctx.trace_id, session_id, err
            ),
        }
        result
    }

    fn get_registered_tool(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        let Ok(guard) = self.namespaces.read() else {
            return None;
        };
        guard.all_tools.get(name).cloned()
    }
}

pub fn tokenize_bash_command_line(line: &str) -> Result<Vec<String>, AgentToolError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            '\\' if !in_single => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ch if ch.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if in_single || in_double {
        return Err(AgentToolError::InvalidArgs(
            "unterminated quote in bash command line".to_string(),
        ));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn render_bash_tool_usage(spec: &ToolSpec) -> Option<String> {
    spec.usage
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn is_help_flag(tokens: &[String]) -> bool {
    tokens.len() == 1 && tokens[0] == "--help"
}

fn append_usage_on_invalid_args(err: AgentToolError, usage: &str) -> AgentToolError {
    match err {
        AgentToolError::InvalidArgs(message) => {
            if message.contains("Usage:") {
                return AgentToolError::InvalidArgs(message);
            }
            let trimmed = message.trim();
            if trimmed.is_empty() {
                AgentToolError::InvalidArgs(format!("Usage: {usage}"))
            } else {
                AgentToolError::InvalidArgs(format!("{trimmed}\nUsage: {usage}"))
            }
        }
        other => other,
    }
}

pub fn parse_default_bash_exec_args(tokens: &[String]) -> Result<Json, AgentToolError> {
    if tokens.is_empty() {
        return Ok(json!({}));
    }

    if tokens.len() == 1 {
        let raw = tokens[0].trim();
        if raw.starts_with('{') {
            let parsed: Json = serde_json::from_str(raw).map_err(|err| {
                AgentToolError::InvalidArgs(format!(
                    "invalid json object args: {err}; quote as: tool '{{\"key\":\"value\"}}'"
                ))
            })?;
            if !parsed.is_object() {
                return Err(AgentToolError::InvalidArgs(
                    "bash args json must be an object".to_string(),
                ));
            }
            return Ok(parsed);
        }
    }

    let mut out = serde_json::Map::<String, Json>::new();
    for token in tokens {
        let (raw_key, raw_value) = token.split_once('=').ok_or_else(|| {
            AgentToolError::InvalidArgs(
                "default bash exec requires key=value args or one json object".to_string(),
            )
        })?;
        let key = raw_key.trim();
        if key.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "arg key cannot be empty".to_string(),
            ));
        }
        out.insert(
            key.to_string(),
            parse_default_bash_exec_value(raw_value.trim()),
        );
    }
    Ok(Json::Object(out))
}

fn parse_default_bash_exec_value(raw: &str) -> Json {
    let value = raw.trim();
    if value.is_empty() {
        return Json::String(String::new());
    }
    serde_json::from_str(value).unwrap_or_else(|_| Json::String(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoTool {
        name: String,
        usage: Option<String>,
    }

    struct StaticSessionBackend;

    #[async_trait]
    impl SessionViewBackend for StaticSessionBackend {
        async fn session_view(&self, session_id: &str) -> Result<Json, AgentToolError> {
            Ok(json!({
                "session_id": session_id,
                "status": "wait"
            }))
        }
    }

    struct StaticWorklogBackend;

    #[async_trait]
    impl WorklogActionBackend for StaticWorklogBackend {
        async fn execute_action(
            &self,
            _ctx: &SessionRuntimeContext,
            args: Json,
        ) -> Result<Json, AgentToolError> {
            Ok(json!({
                "ok": true,
                "action": args.get("action").cloned().unwrap_or_else(|| json!("list")),
                "records": []
            }))
        }
    }

    #[async_trait]
    impl AgentTool for EchoTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: "echo args".to_string(),
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "range": {"type": "string"}
                    }
                }),
                output_schema: json!({"type":"object"}),
                usage: self.usage.clone(),
            }
        }

        fn support_bash(&self) -> bool {
            true
        }

        fn support_action(&self) -> bool {
            true
        }

        fn support_llm_tool_call(&self) -> bool {
            true
        }

        async fn call(
            &self,
            _ctx: &SessionRuntimeContext,
            args: Json,
        ) -> Result<AgentToolResult, AgentToolError> {
            Ok(AgentToolResult::from_details(json!({
                "ok": true,
                "args": args,
            }))
            .with_result("ok"))
        }
    }

    fn test_call_ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "did:opendan:test".to_string(),
            behavior: "plan".to_string(),
            step_idx: 3,
            wakeup_id: "wake-1".to_string(),
            session_id: "session-1".to_string(),
        }
    }

    #[test]
    fn normalize_tool_name_only_trims_whitespace() {
        assert_eq!(
            normalize_tool_name(" workshop.exec_bash "),
            "workshop.exec_bash"
        );
        assert_eq!(normalize_tool_name("todo_manage"), "todo_manage");
        assert_eq!(normalize_tool_name(""), "");
    }

    #[test]
    fn parse_default_bash_exec_args_supports_json_and_key_value() {
        let json_args = parse_default_bash_exec_args(&["{\"path\":\"a.txt\"}".to_string()])
            .expect("parse json args");
        assert_eq!(json_args["path"], "a.txt");

        let kv_args = parse_default_bash_exec_args(&[
            "path=a.txt".to_string(),
            "count=2".to_string(),
            "flag=true".to_string(),
        ])
        .expect("parse key value args");
        assert_eq!(kv_args["path"], "a.txt");
        assert_eq!(kv_args["count"], 2);
        assert_eq!(kv_args["flag"], true);
    }

    #[tokio::test]
    async fn manager_keeps_registered_tool_name_and_routes_bash_calls() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(EchoTool {
            name: "read_file".to_string(),
            usage: Some("read_file path=<path>".to_string()),
        })
        .expect("register tool");

        assert!(mgr.has_tool("read_file"));

        let result = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file path=\"a.txt\" range=\"1:2\"")
            .await
            .expect("bash call succeeds")
            .expect("tool matched");
        assert_eq!(result.details["ok"], true);
        assert_eq!(result.details["args"]["path"], "a.txt");
        assert_eq!(result.details["args"]["range"], "1:2");
    }

    #[tokio::test]
    async fn manager_help_result_is_marked_as_builtin_tool() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(EchoTool {
            name: "read_file".to_string(),
            usage: Some("read_file path=<path>".to_string()),
        })
        .expect("register tool");

        let result = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file --help")
            .await
            .expect("bash help succeeds")
            .expect("tool matched");

        assert!(result.is_agent_tool);
        assert_eq!(result.summary, "show usage");
        assert_eq!(result.command_line_text().as_deref(), Some("read_file --help"));
        assert_eq!(result["tool"], "read_file");
    }

    #[tokio::test]
    async fn get_session_tool_marks_result_as_builtin_tool() {
        let tool = GetSessionTool::new(Arc::new(StaticSessionBackend));
        let result = tool
            .call(&test_call_ctx(), json!({ "session_id": "session-9" }))
            .await
            .expect("get session");

        assert!(result.is_agent_tool);
        assert_eq!(result.summary, "ok");
        assert_eq!(result.command_line_text().as_deref(), Some("get_session session-9"));
        assert_eq!(result["session"]["session_id"], "session-9");
    }

    #[tokio::test]
    async fn worklog_tool_marks_result_as_builtin_tool() {
        let tool = WorklogTool::new(Arc::new(StaticWorklogBackend));
        let result = tool
            .call(&test_call_ctx(), json!({ "action": "list_worklog" }))
            .await
            .expect("worklog call");

        assert!(result.is_agent_tool);
        assert_eq!(result.summary, "list_worklog");
        assert_eq!(result.command_line_text().as_deref(), Some("worklog_manage"));
        assert_eq!(result["action"], "list_worklog");
    }

    #[test]
    fn agent_tool_result_render_prompt_truncates_output_by_lines() {
        let output = (0..(PROMPT_STDIO_MAX_LINES + 10))
            .map(|idx| format!("line-{idx:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = AgentToolResult::from_details(json!({"ok": true}))
            .with_cmd_line("read_file a.txt")
            .with_result("ok")
            .with_output(output)
            .render_prompt();

        assert!(rendered.contains("line-0000"));
        assert!(rendered.contains(format!("line-{:04}", PROMPT_STDIO_MAX_LINES - 1).as_str()));
        assert!(!rendered.contains(format!("line-{:04}", PROMPT_STDIO_MAX_LINES).as_str()));
        assert!(rendered.contains("TRUNCATED FOR ACTION PREVIEW"));
    }

    #[test]
    fn render_for_level_standard_agent_tool_uses_summary_and_details() {
        let result = AgentToolResult::from_details(json!({
            "ok": true,
            "path": "demo.txt",
            "bytes": 12
        }))
        .with_is_agent_tool(true)
        .with_cmd_line("read_file demo.txt range=1-2")
        .with_result("read 12 bytes");

        let min = result.render_for_level(AgentHistoryShowLevel::Min);
        let mini = result.render_for_level(AgentHistoryShowLevel::Mini);
        let full = result.render_for_level(AgentHistoryShowLevel::Full);

        assert!(min.contains("read_file demo.txt range=1-2 => success"));
        assert_eq!(mini, "read 12 bytes");
        assert!(full.contains("read_file demo.txt range=1-2"));
        assert!(full.contains("```result"));
        assert!(full.contains("\"path\": \"demo.txt\""));
    }

    #[test]
    fn render_for_level_non_agent_tool_shows_tail_on_failure() {
        let result = AgentToolResult::from_details(json!({}))
        .with_cmd_line("cargo test --package demo")
        .with_status(AgentToolStatus::Error)
        .with_return_code(1)
        .with_output("line-1\nline-2\nline-3\nline-4\nline-5\nline-6\nline-7\nline-8\nline-9");

        let mini = result.render_for_level(AgentHistoryShowLevel::Mini);
        let full = result.render_for_level(AgentHistoryShowLevel::Full);

        assert!(mini.contains("cargo test --package demo => failed (line-9)"));
        assert!(mini.contains("```output"));
        assert!(mini.contains("line-9"));
        assert!(full.contains("cargo test --package demo => failed (line-9)"));
        assert!(full.contains("line-1"));
        assert!(full.contains("line-9"));
    }
}
