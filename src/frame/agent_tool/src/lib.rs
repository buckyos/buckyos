// TODO(stage 5/12): split this file into trait.rs / envelope.rs / host.rs /
// manager.rs / tools/*.rs (target ≤600 lines per file). Deferred from the
// initial cleanup pass because the AgentToolResult/MCPTool/manager blocks
// share several private helpers (`truncate_text`, `compact_json_text`,
// history constants) that need coordinated relocation.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::{ToolResultStatusView, ToolResultView};
use log::{info, warn};
use schemars::JsonSchema;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value as Json};
use tokio::time::{timeout, Duration};

pub mod agent_attention_signal;
pub mod agent_memory;
pub mod agent_notebook;
pub mod dcrontab_tool;
pub mod file_tools;
pub mod glob_tool;
pub mod grep_tool;
pub mod json_args;
pub mod llm_bash;
pub mod llm_compress;
pub mod llm_explore;
pub mod llm_tool_carft;
pub mod llm_understand_media;
pub mod local_llm_context;
pub mod path_utils;
pub mod run_local_llm;
pub mod runtime_context;
pub mod skills_mgr;
pub mod todo_tools;
pub mod tool;
pub mod workspace;

pub use tool::{
    BasicToolHost, CallingConventions, CliInvocation, ContentInput, NullToolHost, ToolCtx,
    ToolHost, TypedTool, TypedToolHandle,
};

pub use agent_attention_signal::{
    AgentAttentionSignalStage1Runner, AgentAttentionSignalStore, AttentionSignal,
    AttentionSignalConfig, AttentionSignalError, AttentionSignalExtractionInput,
    AttentionSignalExtractor, AttentionSignalHistoryEntry, AttentionSignalHistoryReader,
    AttentionSignalPayload, AttentionSignalSessionWindow, AttentionSignalStage1RunReport,
    AttentionSignalStoreConfig, AttentionSignalTimeWindow, AttentionSignalToolRuntime,
    AttentionSignalWriteResult, DiscoverEventTool, DiscoverObjectObservationTool,
    DiscoverRelationshipTool, DiscoverSkillCoverageGapTool, ExtractionWindow, MarkScannedInput,
    ScanCheckpoint, TOOL_DISCOVER_EVENT, TOOL_DISCOVER_OBJECT_OBSERVATION,
    TOOL_DISCOVER_RELATIONSHIP, TOOL_DISCOVER_SKILL_COVERAGE_GAP,
};
pub use agent_memory::{
    AgentMemory, AgentMemoryConfig, AgentMemoryError, Envelope as AgentMemoryEnvelope, LoadItem,
    LoadOptions, MemoryHint, MemoryHintBudget, MemoryHintType, MemoryRecallOptions, Preamble,
    VerifyReport,
};
pub use dcrontab_tool::{DcrontabTool, TOOL_DCRONTAB};
pub use file_tools::{
    parse_read_file_bash_args, rewrite_read_file_path_with_shell_cwd, EditFileTool, FileToolConfig,
    FileWriteAuditBackend, FileWriteAuditRecord, NoopFileWriteAudit, ReadFileTool, WriteFileTool,
    TOOL_EDIT_FILE, TOOL_READ_FILE, TOOL_WRITE_FILE,
};
pub use glob_tool::{GlobTool, TOOL_GLOB};
pub use grep_tool::{GrepTool, TOOL_GREP};
pub use json_args::{
    optional_string_arg, optional_trimmed_string_arg, optional_u64_arg, read_bool_from_map,
    read_string_from_map, read_u64_from_map, require_string_arg, require_trimmed_string_arg,
    u64_to_usize_arg,
};
pub use llm_bash::{
    BashRunOutput, BashRunRequest, BashRunner, BashTarget, BashTargetSpec, BinOverlayConfig,
    ExecBashTool, LlmBashConfig, LocalProcessBashRunner, TOOL_EXEC_BASH,
};
pub use path_utils::{
    normalize_abs_path, normalize_root_path, resolve_path_from_root, resolve_path_under_root,
    rewrite_path_with_shell_cwd, sanitize_session_id_for_path, session_record_path, to_abs_path,
    MAX_SESSION_ID_LEN,
};
pub use runtime_context::{
    AgentIdentity, RuntimeContext, RuntimeContextSource, BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV,
    OPENDAN_AGENT_ROOT_ENV, OPENDAN_SESSION_ID_ENV, OPENDAN_TRACE_ID_ENV,
};
pub use skills_mgr::{
    CreateCandidateInput as SkillCreateCandidateInput, InstallResult as SkillInstallResult,
    LifecycleState as SkillLifecycleState, ListSkillsInput, OwnerScope as SkillOwnerScope,
    PromptFragment as SkillPromptFragment, RenderSelectedInput as SkillRenderSelectedInput,
    RiskLevel as SkillRiskLevel, SelectedSkillSet, SkillHint, SkillPackageFile,
    SkillPackageSummary, SkillSelectionValidation, SkillSourceType, SkillTaskResult, SkillType,
    SkillUsage, SkillUsageCost, SkillsMgr, SkillsMgrConfig, SkillsMgrError, UsageMode,
    UserFeedback, ValidateSelectionInput as SkillValidateSelectionInput, VerificationStatus,
};
pub use todo_tools::{
    DelegateTaskArgs, DelegateTaskOutput, DelegateTaskTool, StubTaskDelegator, TaskDelegator,
    TaskEntry, TaskListItem, TodoArgs, TodoListItem, TodoOutput, TodoRecord, TodoStatus, TodoTool,
    TodoToolConfig, TodoView, TOOL_DELEGATE_TASK, TOOL_TODO,
};
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub use llm_compress::{compress, LlmSummarizeCompressor, DEFAULT_KEEP_RECENT_MESSAGES};
pub use llm_understand_media::{LlmUnderstandMediaTool, TOOL_LLM_UNDERSTAND_MEDIA};
pub use local_llm_context::{
    Compressor, FileSnapshotStore, LocalLLMContext, OneShotRequest, RunMetaState, RunStatus,
    SnapshotStore, SuspendKind, DEFAULT_CONTEXT_YIELD_RATIO, DEFAULT_MAX_CONSECUTIVE_ERRORS,
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
    #[serde(default = "default_read_token_limit")]
    pub read_token_limit: u32,
}

pub const DEFAULT_READ_TOKEN_LIMIT: u32 = 20 * 1024;

fn default_read_token_limit() -> u32 {
    DEFAULT_READ_TOKEN_LIMIT
}

impl SessionRuntimeContext {
    pub fn effective_read_token_limit(&self) -> u32 {
        if self.read_token_limit == 0 {
            DEFAULT_READ_TOKEN_LIMIT
        } else {
            self.read_token_limit
        }
    }
}

pub const TOOL_GET_SESSION: &str = "get_session";
pub const TOOL_READ: &str = "read";
pub const TOOL_LIST_SESSION: &str = "list_session";
pub const TOOL_LIST_EXTERNAL_WORKSPACES: &str = "list_external_workspaces";
pub const TOOL_BIND_EXTERNAL_WORKSPACE: &str = "bind_external_workspace";
pub const TOOL_CREATE_WORKSPACE: &str = "create_workspace";
pub const TOOL_BIND_WORKSPACE: &str = "bind_workspace";
pub const TOOL_LOAD_MEMORY: &str = "load_memory";
pub const TOOL_SET_MEMORY: &str = "set_memory";
pub const TOOL_REMOVE_MEMORY: &str = "remove_memory";
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
const HISTORY_OUTPUT_FULL_LINES: usize = 512;

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
    #[serde(deserialize_with = "deserialize_agent_tool_protocol")]
    pub agent_tool_protocol: String,
    /// Logical tool name when the result is the rendered output of a
    /// registered agent tool. Used by the CLI front-end and surfaces in
    /// downstream consumers; absent for raw bash results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
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

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(rename = "detail", default = "default_result_detail")]
    pub details: Json,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// Current `agent_tool_protocol` schema version emitted in the AgentToolResult JSON.
pub const AGENT_TOOL_PROTOCOL_VERSION: &str = "1";

fn default_agent_tool_protocol() -> String {
    AGENT_TOOL_PROTOCOL_VERSION.to_string()
}

fn default_result_detail() -> Json {
    json!({})
}

fn deserialize_agent_tool_protocol<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == AGENT_TOOL_PROTOCOL_VERSION {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "unsupported agent_tool_protocol `{value}`"
        )))
    }
}

/// Output of running an agent tool through the CLI front-end.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliRunOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub const CLI_EXIT_SUCCESS: i32 = 0;
pub const CLI_EXIT_ERROR: i32 = 1;
pub const CLI_EXIT_USAGE: i32 = 2;
pub const CLI_EXIT_COMMAND_NOT_FOUND: i32 = 127;

/// Subcommand the bash `command_not_found_handle` hook proxies to. Lives here
/// (not in `agent_tool_cli_dev`) because both ends — the shell-side hook
/// injected by `opendan::agent_bash::build_exec_script` and the CLI-side
/// dispatcher in `agent_tool_cli_dev` — must spell it the same way.
pub const CLI_COMMAND_NOT_FOUND_SUBCOMMAND: &str = "__command_not_found__";

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
            agent_tool_protocol: default_agent_tool_protocol(),
            tool: None,
            status: AgentToolStatus::Success,
            title: String::new(),
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

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Tag the result with the registered tool name. Empty/whitespace
    /// inputs clear the field.
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        let tool = tool.into();
        self.tool = (!tool.trim().is_empty()).then_some(tool);
        self
    }

    pub fn with_status(mut self, status: AgentToolStatus) -> Self {
        self.status = status;
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
        self.render_protocol_result_for_level(level)
    }

    pub fn render_for_last_step(&self) -> String {
        self.render_protocol_result_for_last_step()
    }

    pub fn command_line_text(&self) -> Option<String> {
        self.cmd_name.as_ref().map(|cmd_name| {
            match self
                .cmd_args
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(cmd_args) => format!("{cmd_name} {cmd_args}"),
                None => cmd_name.clone(),
            }
        })
    }

    pub fn to_tool_result_view(&self) -> ToolResultView {
        ToolResultView {
            agent_tool_protocol: Some(self.agent_tool_protocol.clone()),
            status: match self.status {
                AgentToolStatus::Success => ToolResultStatusView::Success,
                AgentToolStatus::Error => ToolResultStatusView::Error,
                AgentToolStatus::Pending => ToolResultStatusView::Pending,
            },
            cmd_name: self.cmd_name.clone(),
            cmd_args: self.cmd_args.clone(),
            title: self.title.clone(),
            summary: self.summary.clone(),
            output: self.output.clone(),
            detail: match &self.details {
                Json::Null => None,
                Json::Object(map) if map.is_empty() => None,
                value => Some(value.clone()),
            },
            return_code: self.return_code,
            task_id: self.task_id.clone(),
            partial_output: self.partial_output.clone(),
            pending_reason: self
                .pending_reason
                .map(history_pending_reason_label)
                .map(str::to_string),
            check_after: self.check_after,
        }
    }

    pub fn refresh_default_title(mut self) -> Self {
        self.title = derive_default_title(&self);
        self
    }

    fn render_protocol_result_for_level(&self, level: AgentHistoryShowLevel) -> String {
        match level {
            AgentHistoryShowLevel::Min => {
                let title = self.title.trim();
                if title.is_empty() {
                    self.render_command_with_status(self.history_compact_command_text())
                } else {
                    title.to_string()
                }
            }
            AgentHistoryShowLevel::Mini | AgentHistoryShowLevel::Medium => {
                let summary = self.summary.trim();
                if summary.is_empty() {
                    let title = self.title.trim();
                    if title.is_empty() {
                        self.render_command_with_status(self.history_compact_command_text())
                    } else {
                        title.to_string()
                    }
                } else {
                    summary.to_string()
                }
            }
            AgentHistoryShowLevel::Full => self.render_protocol_full(false),
        }
    }

    fn render_protocol_result_for_last_step(&self) -> String {
        self.render_protocol_full(true)
    }

    fn render_protocol_full(&self, uncompressed: bool) -> String {
        let command = self
            .command_line_text()
            .or_else(|| self.history_compact_command_text())
            .unwrap_or_else(|| "action".to_string());
        let mut lines = vec![command];
        let mut has_body = false;
        if let Some(output) = self
            .output
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            lines.push("```output".to_string());
            lines.push(if uncompressed {
                output.to_string()
            } else {
                take_head_lines(output, HISTORY_OUTPUT_FULL_LINES)
            });
            lines.push("```".to_string());
            has_body = true;
        }
        if let Some(details) = if uncompressed {
            self.render_details_block_uncompressed()
        } else {
            self.render_details_block()
        } {
            lines.push("```json".to_string());
            lines.push(details);
            lines.push("```".to_string());
            has_body = true;
        }
        if !has_body {
            lines.push(self.history_result_text());
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

    fn render_details_block(&self) -> Option<String> {
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

    fn render_details_block_uncompressed(&self) -> Option<String> {
        match &self.details {
            Json::Null => None,
            Json::Object(map) if map.is_empty() => None,
            value => {
                Some(serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
            }
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
    let cmd_line = cmd_line.into();
    let summary = summary.into();
    let mut result = AgentToolResult::from_details(details)
        .with_cmd_line(cmd_line)
        .with_result(summary);
    if result.title.trim().is_empty() {
        result.title = derive_default_title(&result);
    }
    result
}

pub(crate) fn derive_default_title(result: &AgentToolResult) -> String {
    let cmd = result
        .command_line_text()
        .map(|value| truncate_text(value.trim(), HISTORY_COMPACT_CMD_MAX_CHARS))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "action".to_string());
    let status_text = match result.status {
        AgentToolStatus::Success => "success".to_string(),
        AgentToolStatus::Error => "error".to_string(),
        AgentToolStatus::Pending => match result.pending_reason {
            Some(reason) => format!("pending ({})", history_pending_reason_label(reason)),
            None => "pending".to_string(),
        },
    };
    format!("{cmd} => {status_text}")
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
    out.push_str(format!("... [TRUNCATED: showing first {max_lines} lines only] ...").as_str());
    out
}

/// Decorate an `AgentToolResult` produced by a tool with the CLI-specific
/// defaults (fill in the tool name and a default summary/title when missing).
pub fn cli_result_from_tool_result(
    tool_name: &str,
    mut result: AgentToolResult,
) -> AgentToolResult {
    if result.summary.trim().is_empty() {
        result.summary = "completed".to_string();
    }
    if !tool_name.trim().is_empty() {
        result.tool = Some(tool_name.to_string());
    }
    if result.title.trim().is_empty() {
        result.title = derive_default_title(&result);
    }
    result
}

/// Build the CLI result returned when a tool errors out before
/// producing a result of its own.
pub fn cli_error_result(tool_name: Option<&str>, err: &AgentToolError) -> AgentToolResult {
    let message = err.to_string();
    let title = match tool_name {
        Some(name) => format!("{name} => error"),
        None => "error".to_string(),
    };
    AgentToolResult {
        agent_tool_protocol: default_agent_tool_protocol(),
        tool: tool_name.map(|value| value.to_string()),
        cmd_name: None,
        status: AgentToolStatus::Error,
        task_id: None,
        pending_reason: None,
        check_after: None,
        estimated_wait: None,
        title,
        summary: message.clone(),
        details: json!({}),
        cmd_args: None,
        return_code: None,
        partial_output: None,
        output: Some(message),
    }
}

/// Build the CLI result returned for synthetic success messages such
/// as `--help`, where there is no underlying tool execution.
pub fn cli_success_result(
    tool: Option<String>,
    detail: Json,
    summary: impl Into<String>,
) -> AgentToolResult {
    AgentToolResult {
        agent_tool_protocol: default_agent_tool_protocol(),
        tool,
        cmd_name: None,
        status: AgentToolStatus::Success,
        task_id: None,
        pending_reason: None,
        check_after: None,
        estimated_wait: None,
        title: String::new(),
        summary: summary.into(),
        details: detail,
        cmd_args: None,
        return_code: None,
        partial_output: None,
        output: None,
    }
}

pub fn render_cli_output(payload: &AgentToolResult, exit_code: i32) -> CliRunOutput {
    let stdout = serde_json::to_string(payload).unwrap_or_else(|_| {
        "{\"agent_tool_protocol\":\"1\",\"status\":\"error\",\"summary\":\"serialize cli result failed\",\"detail\":{}}"
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

/// Type-erased dispatch trait used internally by [`AgentToolManager`].
///
/// **Prefer [`TypedTool`] for new tools** — it gives you typed
/// `Args`/`Output`, automatic JSON (de)serialization and removes the
/// `spec()` boilerplate. The manager wraps every `TypedTool` into a
/// `TypedToolHandle` that implements this trait, so registering a
/// typed tool via [`AgentToolManager::register_typed_tool`] is the
/// idiomatic path.
///
/// Implement this trait directly only when the tool needs to produce
/// non-trivial [`AgentToolResult`] variants the typed pipeline cannot
/// express (e.g. `Pending` long-running tasks with `task_id`,
/// `partial_output`, custom `pending_reason`).
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    fn calling(&self) -> CallingConventions;

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

    /// Convert a CLI argv (after the tool name) into a dispatch
    /// invocation. Default treats it as bash form. Tools whose CLI
    /// uses `--flag value` syntax override this; the override lives
    /// next to the tool definition rather than in `cli.rs`.
    fn parse_cli_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<crate::tool::CliInvocation, AgentToolError> {
        Ok(crate::tool::CliInvocation::Bash {
            line: build_bash_cli_line(self.spec().name.as_str(), tokens),
        })
    }

    /// True for tools that emit a single text payload the CLI should
    /// stream out raw (no JSON wrapper) when stdout is non-interactive.
    /// Currently only `read_file` opts in.
    fn cli_plain_text_stdout(&self) -> bool {
        false
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

#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSessionArgs {
    #[serde(default)]
    pub session_id: Option<String>,
}

#[async_trait]
impl TypedTool for GetSessionTool {
    type Args = GetSessionArgs;
    type Output = Json;

    fn name(&self) -> &str {
        TOOL_GET_SESSION
    }
    fn description(&self) -> &str {
        "Read current session state and status. Used by runtime before each LLM round."
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH
    }

    fn usage(&self) -> Option<String> {
        Some("get_session [session_id]".to_string())
    }

    fn parse_bash_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        if tokens.is_empty() {
            return Ok(json!({}));
        }
        if tokens.len() == 1 && !tokens[0].contains('=') {
            return Ok(json!({ "session_id": tokens[0].trim() }));
        }
        parse_default_bash_exec_args(tokens)
    }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<crate::CliInvocation, AgentToolError> {
        // Accept `--session-id <v>`, `session_id=<v>`, or one positional;
        // otherwise rebuild a bash line and let `parse_bash_args` handle it.
        let mut session_id: Option<String> = None;
        let mut idx = 0usize;
        while idx < tokens.len() {
            match tokens[idx].as_str() {
                "--session-id" => {
                    idx += 1;
                    session_id = Some(
                        tokens
                            .get(idx)
                            .ok_or_else(|| {
                                AgentToolError::InvalidArgs(
                                    "missing value for `--session-id` (get_session)".to_string(),
                                )
                            })?
                            .clone(),
                    );
                }
                token if token.starts_with("--") => {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "unsupported flag `{token}` (get_session)"
                    )));
                }
                token if token.contains('=') => {
                    let (key, value) = token.split_once('=').unwrap();
                    match key {
                        "session_id" | "session" => session_id = Some(value.to_string()),
                        _ => {
                            return Err(AgentToolError::InvalidArgs(format!(
                                "unsupported arg `{key}` (get_session)"
                            )))
                        }
                    }
                }
                value => {
                    if session_id.is_some() {
                        return Err(AgentToolError::InvalidArgs(format!(
                            "unexpected positional arg `{value}` (get_session)"
                        )));
                    }
                    session_id = Some(value.to_string());
                }
            }
            idx += 1;
        }

        let mut forwarded = Vec::new();
        if let Some(id) = session_id {
            forwarded.push(format!("session_id={id}"));
        }
        Ok(crate::CliInvocation::Bash {
            line: build_bash_cli_line(TOOL_GET_SESSION, &forwarded),
        })
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        match args
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(id) => Some(format!("{TOOL_GET_SESSION} {id}")),
            None => Some(TOOL_GET_SESSION.to_string()),
        }
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let session_id = args
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ctx.session().session_id.trim().to_string());
        if session_id.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is required".to_string(),
            ));
        }
        let session = self.backend.session_view(session_id.as_str()).await?;
        Ok(json!({ "ok": true, "session": session }))
    }
}

// NOTE(beta2.2): the legacy MemoryLoadBackend / MemoryMutationBackend traits
// and their `LoadMemoryTool` / `SetMemoryTool` / `RemoveMemoryTool` wrappers
// were removed in the v2.10 Memory Graph rewrite. Agents now invoke the `agent-memory`
// CLI via the standard bash tool; the in-process API lives in
// `crate::agent_memory`. The TOOL_*_MEMORY constants above are kept only as
// stable command names for prompt scaffolding.

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

#[derive(Deserialize, JsonSchema)]
pub struct CreateWorkspaceArgs {
    pub name: String,
    pub summary: String,
}

#[async_trait]
impl TypedTool for CreateWorkspaceTool {
    type Args = CreateWorkspaceArgs;
    type Output = Json;

    fn name(&self) -> &str {
        TOOL_CREATE_WORKSPACE
    }
    fn description(&self) -> &str {
        "创建session的wrokspace并设置为session的default workspace"
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH
    }

    fn usage(&self) -> Option<String> {
        Some("create_workspace <name> <summary>".to_string())
    }

    fn parse_bash_args(
        &self,

        tokens: &[String],

        _shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        if tokens.len() < 2 {
            return Err(AgentToolError::InvalidArgs(
                "missing required arguments: <name> <summary>".to_string(),
            ));
        }
        if tokens.len() > 2 {
            return Err(AgentToolError::InvalidArgs(
                "create_workspace only supports arguments: <name> <summary>".to_string(),
            ));
        }
        let name = tokens[0].trim();
        if name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace name cannot be empty".to_string(),
            ));
        }
        let summary = tokens[1].trim();
        if summary.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace summary cannot be empty".to_string(),
            ));
        }
        Ok(json!({ "name": name, "summary": summary }))
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        Some(format!(
            "{TOOL_CREATE_WORKSPACE} {} {}",
            args.name, args.summary
        ))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        let workspace = output
            .get("workspace_id")
            .or_else(|| output.pointer("/workspace/workspace_id"))
            .or_else(|| output.get("id"))
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("workspace");
        format!("created workspace {workspace}")
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        let workspace = output
            .get("workspace_id")
            .or_else(|| output.pointer("/workspace/workspace_id"))
            .or_else(|| output.get("id"))
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("workspace");
        Some(format!("{TOOL_CREATE_WORKSPACE} {workspace} => created"))
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let name = args.name.trim();
        let summary = args.summary.trim();
        if name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace name cannot be empty".to_string(),
            ));
        }
        if summary.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace summary cannot be empty".to_string(),
            ));
        }
        self.backend
            .create_workspace(ctx.session(), name.to_string(), summary.to_string())
            .await
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

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindExternalWorkspaceArgs {
    /// Local mount name.
    pub name: String,
    /// Absolute or relative source workspace path.
    pub workspace_path: String,
    /// Optional target agent DID. Defaults to current agent DID.
    #[serde(default)]
    pub agent_did: Option<String>,
}

#[async_trait]
impl TypedTool for BindExternalWorkspaceTool {
    type Args = BindExternalWorkspaceArgs;
    type Output = Json;

    fn name(&self) -> &str {
        TOOL_BIND_EXTERNAL_WORKSPACE
    }
    fn description(&self) -> &str {
        "Bind an external workspace directory so this agent can access it from runtime."
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let agent = args.agent_did.as_deref().map(str::trim).unwrap_or("");
        let mut out = format!(
            "{TOOL_BIND_EXTERNAL_WORKSPACE} {} {}",
            args.name.trim(),
            args.workspace_path.trim()
        );
        if !agent.is_empty() {
            out.push_str(format!(" agent_did={agent}").as_str());
        }
        Some(out)
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        let name = output
            .pointer("/binding/name")
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("external workspace");
        format!("bound external workspace {name}")
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        let name = output
            .pointer("/binding/name")
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("external workspace");
        Some(format!("{TOOL_BIND_EXTERNAL_WORKSPACE} {name} => bound"))
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let name = args.name.trim();
        let workspace_path = args.workspace_path.trim();
        if name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "missing required arg `name`".to_string(),
            ));
        }
        if workspace_path.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "missing required arg `workspace_path`".to_string(),
            ));
        }
        let agent_did = args
            .agent_did
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ctx.session().agent_name.clone());
        let binding = self
            .backend
            .bind_external_workspace(agent_did.as_str(), name, workspace_path)
            .await?;
        Ok(json!({ "ok": true, "binding": binding }))
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

#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListExternalWorkspacesArgs {
    /// Optional agent DID. Defaults to current agent DID.
    #[serde(default)]
    pub agent_did: Option<String>,
}

#[async_trait]
impl TypedTool for ListExternalWorkspacesTool {
    type Args = ListExternalWorkspacesArgs;
    type Output = Json;

    fn name(&self) -> &str {
        TOOL_LIST_EXTERNAL_WORKSPACES
    }
    fn description(&self) -> &str {
        "List bound external workspaces visible to current agent."
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let mut out = TOOL_LIST_EXTERNAL_WORKSPACES.to_string();
        if let Some(agent) = args
            .agent_did
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            out.push_str(format!(" agent_did={agent}").as_str());
        }
        Some(out)
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        let count = output
            .get("workspaces")
            .and_then(Json::as_array)
            .map(|arr| arr.len())
            .unwrap_or(0);
        format!("listed {count} external workspace(s)")
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let agent_did = args
            .agent_did
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ctx.session().agent_name.clone());
        let workspaces = self
            .backend
            .list_external_workspaces(agent_did.as_str())
            .await?;
        Ok(json!({ "ok": true, "workspaces": workspaces }))
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct BindWorkspaceArgs {
    pub workspace: String,
}

#[async_trait]
impl TypedTool for BindWorkspaceTool {
    type Args = BindWorkspaceArgs;
    type Output = Json;

    fn name(&self) -> &str {
        TOOL_BIND_WORKSPACE
    }
    fn description(&self) -> &str {
        "设置agent_session的当前workspace"
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH
    }

    fn usage(&self) -> Option<String> {
        Some("bind_workspace <workspace_id|workspace_path>".to_string())
    }

    fn parse_bash_args(
        &self,

        tokens: &[String],

        _shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "missing workspace argument".to_string(),
            ));
        }
        if tokens.len() > 1 {
            return Err(AgentToolError::InvalidArgs(
                "bind_workspace only supports one argument: <workspace_id|workspace_path>"
                    .to_string(),
            ));
        }
        let raw_arg = tokens[0].trim();
        let workspace = if let Some((key, value)) = raw_arg.split_once('=') {
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
        if workspace.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace argument cannot be empty".to_string(),
            ));
        }
        Ok(json!({ "workspace": workspace }))
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        Some(format!("{TOOL_BIND_WORKSPACE} {}", args.workspace.trim()))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        let workspace = output
            .get("workspace_id")
            .or_else(|| output.get("local_workspace_id"))
            .or_else(|| output.pointer("/binding/local_workspace_id"))
            .or_else(|| output.get("workspace"))
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("workspace");
        format!("bound session to workspace {workspace}")
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        let workspace = output
            .get("workspace_id")
            .or_else(|| output.get("local_workspace_id"))
            .or_else(|| output.pointer("/binding/local_workspace_id"))
            .or_else(|| output.get("workspace"))
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("workspace");
        Some(format!("{TOOL_BIND_WORKSPACE} {workspace} => bound"))
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let workspace_ref = args.workspace.trim();
        if workspace_ref.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace argument cannot be empty".to_string(),
            ));
        }
        let session = ctx.session();
        if session.session_id.trim().is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is required".to_string(),
            ));
        }
        let workspace_id = self
            .backend
            .resolve_workspace_id(workspace_ref, ctx.shell_cwd())
            .await?;
        self.backend
            .bind_workspace(session, session.session_id.as_str(), workspace_id.as_str())
            .await
    }
}

#[async_trait]
impl TypedTool for WorklogTool {
    type Args = Json;
    type Output = Json;

    fn name(&self) -> &str {
        TOOL_WORKLOG_MANAGE
    }
    fn description(&self) -> &str {
        "Append-only audit log of agent runtime events. Used for debugging and post-hoc analysis; does not feed into prompts."
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH
    }

    fn args_schema(&self) -> Json {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "append_worklog",
                        "list_worklog",
                        "get_worklog"
                    ]
                },
                "record": { "type": "object" },
                "id": { "type": "string" },
                "step_id": { "type": "string" },
                "owner_session_id": { "type": "string" },
                "workspace_id": { "type": "string" },
                "type": { "type": "string" },
                "status": { "type": "string" },
                "keyword": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 },
                "offset": { "type": "integer", "minimum": 0 }
            },
            "required": ["action"],
            "additionalProperties": true
        })
    }

    fn output_schema(&self) -> Json {
        json!({
            "type": "object",
            "properties": {
                "ok": { "type": "boolean" },
                "action": { "type": "string" },
                "record": { "type": "object" },
                "records": { "type": "array", "items": { "type": "object" } },
                "total": { "type": "integer" }
            }
        })
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let action = args.get("action").and_then(Json::as_str).unwrap_or("");
        Some(build_worklog_manage_cmd_line(action, args))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        output
            .get("action")
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("ok")
            .to_string()
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        self.backend.execute_action(ctx.session(), args).await
    }
}

fn build_worklog_manage_cmd_line(action: &str, args: &Json) -> String {
    let mut out = TOOL_WORKLOG_MANAGE.to_string();
    if !action.is_empty() {
        out.push_str(format!(" {action}").as_str());
    }
    for key in [
        "id",
        "step_id",
        "owner_session_id",
        "workspace_id",
        "type",
        "status",
        "keyword",
    ] {
        if let Some(value) = args
            .get(key)
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            out.push_str(format!(" {key}={value}").as_str());
        }
    }
    if let Some(limit) = args.get("limit").and_then(Json::as_u64) {
        out.push_str(format!(" limit={limit}").as_str());
    }
    if let Some(offset) = args.get("offset").and_then(Json::as_u64) {
        if offset > 0 {
            out.push_str(format!(" offset={offset}").as_str());
        }
    }
    out
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
impl TypedTool for MCPTool {
    type Args = Json;
    type Output = Json;

    fn name(&self) -> &str {
        self.spec.name.as_str()
    }

    fn description(&self) -> &str {
        self.spec.description.as_str()
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH | CallingConventions::ACTION
    }

    fn args_schema(&self) -> Json {
        self.spec.args_schema.clone()
    }

    fn output_schema(&self) -> Json {
        self.spec.output_schema.clone()
    }

    fn build_cmd_line(&self, _args: &Self::Args) -> Option<String> {
        Some(self.spec.name.clone())
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        let content_count = output
            .get("content")
            .and_then(Json::as_array)
            .map(|items| items.len());
        match content_count {
            Some(count) => format!(
                "mcp {} completed with {count} content item(s)",
                self.spec.name
            ),
            None => format!("mcp {} completed", self.spec.name),
        }
    }

    fn build_title(&self, _output: &Self::Output) -> Option<String> {
        Some(format!("mcp {} => success", self.spec.name))
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let ctx = ctx.session();
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

        Ok(result)
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

#[derive(Default)]
struct ToolNamespaceRegistry {
    tools: HashMap<String, Arc<dyn AgentTool>>,
}

fn registered_tool_spec(name: &str, tool: &dyn AgentTool) -> ToolSpec {
    let mut spec = tool.spec();
    spec.name = name.to_string();
    spec.usage = spec
        .usage
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    spec
}

#[derive(Clone)]
pub struct AgentToolManager {
    namespaces: Arc<StdRwLock<ToolNamespaceRegistry>>,
    host: Arc<dyn ToolHost>,
}

impl Default for AgentToolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentToolManager {
    pub fn new() -> Self {
        Self::with_host(Arc::new(NullToolHost))
    }

    pub fn with_host(host: Arc<dyn ToolHost>) -> Self {
        Self {
            namespaces: Arc::new(StdRwLock::new(ToolNamespaceRegistry::default())),
            host,
        }
    }

    /// Currently configured `ToolHost`. Stage 3 will start consuming
    /// this for typed-tool registrations; for now it is exposed so
    /// embedders can share a single host between manager-managed and
    /// ad-hoc tool invocations.
    pub fn host(&self) -> Arc<dyn ToolHost> {
        self.host.clone()
    }

    pub fn set_host(&mut self, host: Arc<dyn ToolHost>) {
        self.host = host;
    }

    pub fn register_tool<T>(&self, tool: T) -> Result<(), AgentToolError>
    where
        T: AgentTool + 'static,
    {
        self.register_tool_arc(Arc::new(tool))
    }

    /// Register a `TypedTool` implementation. The manager wraps it
    /// into a `TypedToolHandle` capturing the current host so the
    /// tool gets `ctx.host()` access at call time.
    pub fn register_typed_tool<T>(&self, tool: T) -> Result<(), AgentToolError>
    where
        T: TypedTool,
    {
        let handle = TypedToolHandle::new(tool, self.host.clone());
        self.register_tool_arc(Arc::new(handle))
    }

    pub fn register_tool_arc(&self, tool: Arc<dyn AgentTool>) -> Result<(), AgentToolError> {
        let spec = tool.spec();
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
        let calling = tool.calling();
        if calling.is_empty() {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` must support at least one namespace",
                normalized_name
            )));
        }

        let mut guard = self
            .namespaces
            .write()
            .map_err(|_| AgentToolError::ExecFailed("tool namespace lock poisoned".to_string()))?;
        if guard.tools.contains_key(&normalized_name) {
            return Err(AgentToolError::AlreadyExists(normalized_name));
        }
        guard.tools.insert(normalized_name.clone(), tool);
        if normalized_name != original_name {
            warn!(
                "tool name normalized by trimming: original={} normalized={}",
                original_name, normalized_name
            );
        }
        Ok(())
    }

    pub fn register_mcp_tool(&self, cfg: MCPToolConfig) -> Result<(), AgentToolError> {
        self.register_typed_tool(MCPTool::new(cfg)?)
    }

    pub fn unregister_tool(&self, name: &str) -> bool {
        let normalized_name = normalize_tool_name(name);
        if normalized_name.is_empty() {
            return false;
        }
        let Ok(mut guard) = self.namespaces.write() else {
            return false;
        };
        guard.tools.remove(normalized_name.as_str()).is_some()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.get_tool(name).is_some()
    }

    pub fn get_tool(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        self.get_tool_by_calling(name, CallingConventions::LLM)
    }

    fn get_tool_by_calling(
        &self,
        name: &str,
        calling: CallingConventions,
    ) -> Option<Arc<dyn AgentTool>> {
        let normalized_name = normalize_tool_name(name);
        if normalized_name.is_empty() {
            return None;
        }
        let Ok(guard) = self.namespaces.read() else {
            return None;
        };
        guard
            .tools
            .get(normalized_name.as_str())
            .filter(|tool| tool.calling().contains(calling))
            .cloned()
    }

    pub fn get_bash_cmd(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        self.get_tool_by_calling(name, CallingConventions::BASH)
    }

    pub fn get_action(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        self.get_tool_by_calling(name, CallingConventions::ACTION)
    }

    pub fn get_tool_spec(&self, name: &str) -> Option<ToolSpec> {
        let normalized_name = normalize_tool_name(name);
        self.get_tool(normalized_name.as_str())
            .map(|tool| registered_tool_spec(normalized_name.as_str(), tool.as_ref()))
    }

    pub fn list_tool_specs(&self) -> Vec<ToolSpec> {
        let Ok(guard) = self.namespaces.read() else {
            return vec![];
        };
        let mut specs: Vec<ToolSpec> = guard
            .tools
            .iter()
            .filter(|(_, tool)| tool.calling().supports_llm_tool_call())
            .map(|(name, tool)| registered_tool_spec(name, tool.as_ref()))
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn list_bash_cmd_specs(&self) -> Vec<ToolSpec> {
        let Ok(guard) = self.namespaces.read() else {
            return vec![];
        };
        let mut specs: Vec<ToolSpec> = guard
            .tools
            .iter()
            .filter(|(_, tool)| tool.calling().supports_bash())
            .map(|(name, tool)| registered_tool_spec(name, tool.as_ref()))
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn list_action_tool_specs(&self) -> Vec<ToolSpec> {
        let Ok(guard) = self.namespaces.read() else {
            return vec![];
        };
        let mut specs: Vec<ToolSpec> = guard
            .tools
            .iter()
            .filter(|(_, tool)| tool.calling().supports_action())
            .map(|(name, tool)| registered_tool_spec(name, tool.as_ref()))
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
            .tools
            .get(normalized.as_str())
            .filter(|tool| tool.calling().supports_bash())
            .is_some()
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
        let spec = registered_tool_spec(tool_name.as_str(), tool.as_ref());
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
        let normalized_name = normalize_tool_name(name);
        if normalized_name.is_empty() {
            return None;
        }
        let Ok(guard) = self.namespaces.read() else {
            return None;
        };
        guard.tools.get(normalized_name.as_str()).cloned()
    }

    /// Public lookup that ignores namespace (bash/llm/action). The CLI
    /// dispatcher uses this since it picks between `call`/`exec` based on
    /// the parsed command shape, not on which namespace the tool advertises.
    pub fn get_any_tool(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        self.get_registered_tool(name)
    }
}

/// Stitch a tool name plus argv tokens back into a bash command line,
/// shell-quoting each token. Used by tools whose CLI form is "just the
/// bash form" so they can produce the line their `exec` consumes.
pub fn build_bash_cli_line(tool_name: &str, tokens: &[String]) -> String {
    let mut line = String::from(tool_name);
    for token in tokens {
        line.push(' ');
        line.push_str(&shell_quote_token(token));
    }
    line
}

fn shell_quote_token(raw: &str) -> String {
    if raw.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
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

        fn calling(&self) -> CallingConventions {
            CallingConventions::ALL
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
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
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

        assert_eq!(result.agent_tool_protocol, AGENT_TOOL_PROTOCOL_VERSION);
        assert_eq!(result.summary, "show usage");
        assert_eq!(
            result.command_line_text().as_deref(),
            Some("read_file --help")
        );
        assert_eq!(result["tool"], "read_file");
    }

    #[tokio::test]
    async fn get_session_tool_marks_result_as_builtin_tool() {
        let tool = GetSessionTool::new(Arc::new(StaticSessionBackend));
        let handle = TypedToolHandle::with_null_host(tool);
        let result = AgentTool::call(
            &handle,
            &test_call_ctx(),
            json!({ "session_id": "session-9" }),
        )
        .await
        .expect("get session");

        assert_eq!(result.agent_tool_protocol, AGENT_TOOL_PROTOCOL_VERSION);
        assert_eq!(result.summary, "ok");
        assert_eq!(
            result.command_line_text().as_deref(),
            Some("get_session session-9")
        );
        assert_eq!(result["session"]["session_id"], "session-9");
    }

    #[tokio::test]
    async fn worklog_tool_marks_result_as_builtin_tool() {
        let tool = WorklogTool::new(Arc::new(StaticWorklogBackend));
        let handle = TypedToolHandle::with_null_host(tool);
        let result = AgentTool::call(
            &handle,
            &test_call_ctx(),
            json!({ "action": "list_worklog" }),
        )
        .await
        .expect("worklog call");

        assert_eq!(result.agent_tool_protocol, AGENT_TOOL_PROTOCOL_VERSION);
        assert_eq!(result.summary, "list_worklog");
        assert_eq!(
            result.command_line_text().as_deref(),
            Some("worklog_manage list_worklog")
        );
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
    fn agent_tool_result_deserialize_requires_protocol_marker() {
        assert!(serde_json::from_value::<AgentToolResult>(json!({
            "agent_tool_protocol": "1",
            "status": "success",
            "summary": "ok",
            "detail": {}
        }))
        .is_ok());
        assert!(serde_json::from_value::<AgentToolResult>(json!({
            "is_agent_tool": true,
            "status": "success",
            "summary": "ok",
            "detail": {}
        }))
        .is_err());
        assert!(serde_json::from_value::<AgentToolResult>(json!({
            "status": "success",
            "summary": "ok",
            "detail": {}
        }))
        .is_err());
    }

    #[test]
    fn render_for_level_standard_agent_tool_uses_summary_and_details() {
        let result = AgentToolResult::from_details(json!({
            "ok": true,
            "path": "demo.txt",
            "bytes": 12
        }))
        .with_cmd_line("read_file demo.txt range=1-2")
        .with_result("read 12 bytes");

        let min = result.render_for_level(AgentHistoryShowLevel::Min);
        let mini = result.render_for_level(AgentHistoryShowLevel::Mini);
        let full = result.render_for_level(AgentHistoryShowLevel::Full);

        assert!(min.contains("read_file demo.txt range=1-2 => success"));
        assert_eq!(mini, "read 12 bytes");
        assert!(full.contains("read_file demo.txt range=1-2"));
        assert!(full.contains("```json"));
        assert!(full.contains("\"path\": \"demo.txt\""));
    }

    #[test]
    fn render_for_level_bash_result_uses_summary_for_compressed_levels() {
        let result = AgentToolResult::from_details(json!({}))
            .with_cmd_line("cargo test --package demo")
            .with_status(AgentToolStatus::Error)
            .with_return_code(1)
            .with_result("cargo test failed with exit code 1")
            .with_output("line-1\nline-2\nline-3\nline-4\nline-5\nline-6\nline-7\nline-8\nline-9");

        let mini = result.render_for_level(AgentHistoryShowLevel::Mini);
        let full = result.render_for_level(AgentHistoryShowLevel::Full);

        assert_eq!(mini, "cargo test failed with exit code 1");
        assert!(full.contains("cargo test --package demo"));
        assert!(full.contains("```output"));
        assert!(full.contains("line-1"));
        assert!(full.contains("line-9"));
    }

    #[test]
    fn tool_result_view_preserves_protocol_fields() {
        let view = AgentToolResult::from_details(json!({"rows": [1, 2]}))
            .with_cmd_line("read file:///demo.txt")
            .with_status(AgentToolStatus::Pending)
            .with_result("waiting for task")
            .with_title("read file:///demo.txt => pending")
            .with_task_id("task-9")
            .with_pending_reason(AgentToolPendingReason::LongRunning)
            .with_check_after(5000)
            .with_partial_output("partial")
            .to_tool_result_view();

        assert_eq!(view.status, ToolResultStatusView::Pending);
        assert_eq!(view.cmd_name.as_deref(), Some("read"));
        assert_eq!(view.summary, "waiting for task");
        assert_eq!(view.task_id.as_deref(), Some("task-9"));
        assert_eq!(view.pending_reason.as_deref(), Some("long_running"));
        assert_eq!(view.check_after, Some(5000));
        assert_eq!(view.partial_output.as_deref(), Some("partial"));
        assert!(view.detail.is_some());
    }

    #[test]
    fn render_for_level_full_non_agent_tool_keeps_512_success_lines() {
        let output = (1..=600)
            .map(|idx| format!("line-{idx:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = AgentToolResult::from_details(json!({}))
            .with_cmd_line("sed -n '1,600p' demo.log")
            .with_output(output)
            .render_for_level(AgentHistoryShowLevel::Full);

        assert!(rendered.contains("line-001"));
        assert!(rendered.contains("line-512"));
        assert!(!rendered.contains("line-513"));
        assert!(rendered.contains("showing first 512 lines only"));
    }

    #[test]
    fn render_for_last_step_non_agent_tool_keeps_full_output() {
        let output = (1..=30)
            .map(|idx| format!("line-{idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = AgentToolResult::from_details(json!({}))
            .with_cmd_line("sed -n '1,30p' demo.log")
            .with_output(output)
            .render_for_last_step();

        assert!(rendered.contains("sed -n"));
        assert!(rendered.contains("demo.log"));
        assert!(rendered.contains("line-01"));
        assert!(rendered.contains("line-30"));
        assert!(!rendered.contains("TRUNCATED"));
    }
}
