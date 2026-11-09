// TODO(stage 5/12): split this file into trait.rs / envelope.rs /
// manager.rs / tools/*.rs (target ≤600 lines per file). Deferred from the
// initial cleanup pass because the AgentToolResult/manager blocks share
// several private helpers (`truncate_text`, `compact_json_text`) that need
// coordinated relocation.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use buckyos_api::AiToolCall;
pub use prompt_engine::{escape_xml_attr, escape_xml_text};
pub use hints::{
    apply_hints, render_hint_block, render_with_hints, HintLedger, MemoryHintLedger,
    HINT_NS_DEDUP, HINT_NS_ONCE,
};
pub use intent::{intent_of, take_intent, with_intent, IntentPolicy, INTENT_ARG};
pub use result_view::{ToolResultStatusView, ToolResultView};
pub use step_record::XmlStepRenderer;
pub use task_model::{
    task_state_observation, task_state_observation_via, CancelUnsupported, RunningTaskResolver,
    TaskBrief, TaskFollowup, TaskResult, TaskState, DEFAULT_TASK_WAIT_MS, MAX_IN_TOOL_WAIT_MS,
};
use log::{info, warn};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value as Json};

pub mod agent_memory;
pub mod agent_notebook;
pub mod aicc_model_tools;
pub mod dcrontab_tool;
pub mod exec_tracking;
pub mod file_tools;
pub mod glob_tool;
pub mod grep_tool;
pub mod hints;
pub mod intent;
pub mod json_args;
pub mod llm_bash;
pub mod llm_compress;
pub mod llm_explore;
pub mod llm_tool_carft;
pub mod llm_understand_media;
pub mod path_utils;
pub mod prompt_engine;
pub mod read_tool;
pub mod result_view;
pub mod run_local_llm;
pub mod runtime;
pub mod runtime_context;
pub mod step_record;
pub mod task_bridge;
pub mod task_model;
pub mod tasks;
pub mod tasks_zone;
pub mod todo_tools;
pub mod tool;
pub mod xllm;
pub mod xml_util;

pub use tool::{CallingConventions, CliInvocation, ContentInput, ToolCtx, TypedTool, TypedToolHandle};

pub use agent_memory::{
    AgentMemory, AgentMemoryConfig, AgentMemoryError, LoadItem,
    LoadOptions, VerifyReport,
};
pub use aicc_model_tools::{
    MakeExactModelArgs, MakeExactModelOutput, MakeExactModelTool, ParseExactModelArgs,
    ParseExactModelOutput, ParseExactModelTool, TOOL_MAKE_EXACT_MODEL, TOOL_PARSE_EXACT_MODEL,
};
pub use dcrontab_tool::{DcrontabTool, TOOL_DCRONTAB};
pub use file_tools::{
    parse_read_file_bash_args, rewrite_read_file_path_with_shell_cwd, EditFileTool, FileToolConfig,
    FileWriteAuditBackend, FileWriteAuditRecord, NoopFileWriteAudit, ReadFileTool, WriteFileTool,
    TOOL_EDIT_FILE, TOOL_READ_FILE, TOOL_WRITE_FILE,
};
pub use glob_tool::{GlobTool, TOOL_GLOB};
pub use grep_tool::{GrepTool, TOOL_GREP};
pub use json_args::u64_to_usize_arg;
pub use llm_bash::{
    kill_running_bash_process_groups, BashRunOutput, BashRunRequest, BashRunner, BashTarget,
    BashTargetSpec, BinOverlayConfig, CommandHandle, CommandProgress, LlmBashConfig,
    LocalProcessBashRunner, RunBinding, RunBindingSlot, ShellResultShape, ShellRuntimeNote,
    ShellTool, SHELL_TASK_FOLLOWUP_HINT, TOOL_SHELL,
};
pub use tasks::{
    exec_dir_state, task_tools, CancelTaskTool, CompositeTaskResolver, GetTaskStateTool,
    InProcessTaskManager, LocalTask, ShellTask, WaitTaskTool, TOOL_CANCEL_TASK,
    TOOL_GET_TASK_STATE, TOOL_WAIT_TASK,
};
pub use tasks_zone::{
    MemoryTaskBackend, ZoneTaskBackend, ZoneTaskMgrBackend, ZoneTaskRecord, ZoneTaskResolver,
    ZONE_TASK_PREFIX,
};
pub use path_utils::{
    normalize_abs_path, resolve_path_from_root,
    rewrite_path_with_shell_cwd, sanitize_session_id_for_path, session_record_path, to_abs_path,
    MAX_SESSION_ID_LEN,
};
pub use runtime_context::{
    AgentIdentity, RuntimeContext, RuntimeContextSource, BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV,
    OPENDAN_AGENT_ROOT_ENV, OPENDAN_SESSION_ID_ENV, OPENDAN_TRACE_ID_ENV,
};
pub use todo_tools::{
    ChildObservation, NewTodo, ReportRef, TodoAttempt, TodoList, TodoMode, TodoPatch, TodoRecord,
    TodoStatus, TodoStore, TodoSummary, TODO_LIST_FILE, TODO_LIST_SCHEMA,
};
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub use llm_compress::{compress, LlmSummarizeCompressor};
pub use llm_understand_media::{LlmUnderstandMediaTool, TOOL_LLM_UNDERSTAND_MEDIA};
pub use xllm::{
    AiccLlmClient, Attachment, DefaultLlmClientFactory, EffectiveConfig, EffectiveTools,
    ExtractedValue, LlmClientFactory, LoopModel, OpenAiLlmClient, ProviderKind, ResultFormat,
    ResumeLimits, ResumeStart, RunEvent, RunLogLevel, RunObserver, RunOutcome, RunPhase, RunRecord,
    RunStatus, RunStore, RunSummary, TaskInput, TaskOverrides, XllmDeps, XllmError, XllmResult,
    XllmRun, XllmTask, DEFAULT_CONTEXT_YIELD_RATIO, DEFAULT_MAX_CONSECUTIVE_ERRORS,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeContext {
    pub trace_id: String,
    pub agent_name: String,
    pub behavior: String,
    /// Sequence number of this tool call within the run, set by the
    /// dispatcher on every call. Not a behavior Step index
    /// (`StepRecord.meta.step_index`) and not a Turn number; identify a call
    /// by its `call_id`.
    pub tool_call_index: u32,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub args_schema: Json,
    pub output_schema: Json,
    #[serde(default)]
    pub usage: Option<String>,
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
    #[error("runtime transport: {message}")]
    Transport {
        message: String,
        effect_unknown: bool,
    },
    /// The call was cancelled by the run (interrupt, graceful finish or
    /// deadline, see `llm_context::ToolCallCtx`). `effect_unknown = false`:
    /// the work was stopped or is known to continue, as `message` says;
    /// `true`: the work was abandoned while running.
    #[error("cancelled: {message}")]
    Cancelled {
        message: String,
        effect_unknown: bool,
    },
}

pub fn normalize_tool_name(name: &str) -> String {
    name.trim().to_string()
}

const HISTORY_COMPACT_CMD_MAX_CHARS: usize = 96;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolStatus {
    #[default]
    Success,
    Error,
    Pending,
}

/// Well-known values of `AgentToolResult.pending_reason`. The field is a
/// free string: a reader must accept values it does not know.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolPendingReason {
    LongRunning,
    UserApproval,
    WaitForInstall,
}

impl AgentToolPendingReason {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentToolPendingReason::LongRunning => "long_running",
            AgentToolPendingReason::UserApproval => "user_approval",
            AgentToolPendingReason::WaitForInstall => "wait_for_install",
        }
    }
}

impl From<AgentToolPendingReason> for String {
    fn from(reason: AgentToolPendingReason) -> Self {
        reason.as_str().to_string()
    }
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
    /// Why the result is pending, for the model (see
    /// [`AgentToolPendingReason`] for well-known values).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<String>,
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

    /// Guidance for the model rendered apart from the output
    /// (`<runtime_hints>`), never mixed into it. A hint with `once` is shown
    /// once per session under that key (`<tool>.<topic>[:<version>]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<ResultHint>,
    /// When a result with the same key was already shown in this run, the
    /// output and detail are replaced by `hint` (e.g. a file read again
    /// unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup: Option<ResultDedup>,
    /// Images shown to the model with the result (a desktop screenshot).
    /// Only an in-process tool can attach them: they never travel in the
    /// protocol JSON, so a command's output cannot make the host load a file.
    #[serde(skip)]
    pub images: Vec<buckyos_api::ResourceRef>,
}

/// One entry of [`AgentToolResult::hints`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResultHint {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<String>,
}

/// [`AgentToolResult::dedup`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResultDedup {
    pub key: String,
    pub hint: String,
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
            hints: Vec::new(),
            dedup: None,
            images: Vec::new(),
        }
    }

    pub fn with_image(mut self, image: buckyos_api::ResourceRef) -> Self {
        self.images.push(image);
        self
    }

    /// Add a hint (see [`AgentToolResult::hints`]).
    pub fn with_hint(mut self, text: impl Into<String>, once: Option<&str>) -> Self {
        let text = text.into();
        if !text.trim().is_empty() {
            self.hints.push(ResultHint {
                text,
                once: once.map(str::to_string),
            });
        }
        self
    }

    pub fn with_dedup(mut self, key: impl Into<String>, hint: impl Into<String>) -> Self {
        self.dedup = Some(ResultDedup {
            key: key.into(),
            hint: hint.into(),
        });
        self
    }

    /// The text the LLM sees for this result: the output (or summary) of a
    /// success, the summary and output of an error. A finished task's
    /// result reads the same as the call that ended synchronously.
    pub fn observation_text(&self) -> String {
        match self.status {
            AgentToolStatus::Success | AgentToolStatus::Pending => self
                .output
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    if self.summary.trim().is_empty() {
                        self.details.to_string()
                    } else {
                        self.summary.clone()
                    }
                }),
            AgentToolStatus::Error => {
                let summary = self.summary.trim();
                let output = self
                    .output
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                match (summary.is_empty(), output) {
                    (false, Some(out)) => format!("{summary}\n{out}"),
                    (false, None) => summary.to_string(),
                    (true, Some(out)) => out.to_string(),
                    (true, None) => "tool error".to_string(),
                }
            }
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

    pub fn with_pending_reason(mut self, pending_reason: impl Into<String>) -> Self {
        let pending_reason = pending_reason.into();
        self.pending_reason = (!pending_reason.trim().is_empty()).then_some(pending_reason);
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

    /// The envelope as carried in `Observation.host` (read back with
    /// [`ToolResultView::from_host`]).
    pub fn to_host(&self) -> Json {
        serde_json::to_value(self).unwrap_or(Json::Null)
    }

    pub fn refresh_default_title(mut self) -> Self {
        self.title = derive_default_title(&self);
        self
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
        AgentToolStatus::Pending => match &result.pending_reason {
            Some(reason) => format!("pending ({reason})"),
            None => "pending".to_string(),
        },
    };
    format!("{cmd} => {status_text}")
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
        hints: Vec::new(),
        dedup: None,
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
        images: Vec::new(),
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
        hints: Vec::new(),
        dedup: None,
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
        images: Vec::new(),
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
        | AgentToolError::Timeout
        | AgentToolError::Transport { .. }
        | AgentToolError::Cancelled { .. } => CLI_EXIT_ERROR,
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

    /// Whether the tool stops its work when the call's `ToolCallCtx` fires
    /// and answers with `AgentToolError::Cancelled`. Default `false`: the
    /// dispatcher cannot cancel the work and abandons the call on an
    /// interrupt (effect unknown); a graceful finish waits for it.
    fn cancellable(&self) -> bool {
        false
    }

    /// Whether running the same call again is harmless (reads, a write of
    /// the same content). On resume, a call a previous executor may have
    /// started is run again only when it is idempotent; otherwise its
    /// result is reported unknown. Default `false`.
    fn idempotent(&self) -> bool {
        false
    }

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

    /// Register a `TypedTool` implementation (wrapped into a
    /// `TypedToolHandle`).
    pub fn register_typed_tool<T>(&self, tool: T) -> Result<(), AgentToolError>
    where
        T: TypedTool,
    {
        let handle = TypedToolHandle::new(tool);
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
            Ok(result) if result.status == AgentToolStatus::Error => warn!(
                "opendan.tool_call: status=failed tool={} call_id={} trace_id={} session_id={} err={}",
                tool_name, call_id, ctx.trace_id, session_id, result.summary
            ),
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

    fn test_call_ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "did:opendan:test".to_string(),
            behavior: "plan".to_string(),
            tool_call_index: 3,
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
    async fn get_session_tool_marks_result_as_builtin_tool() {
        let tool = GetSessionTool::new(Arc::new(StaticSessionBackend));
        let handle = TypedToolHandle::new(tool);
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
            .to_host();
        let view = ToolResultView::from_host(&Some(view)).expect("envelope");

        assert_eq!(view.status, ToolResultStatusView::Pending);
        assert_eq!(view.cmd_name.as_deref(), Some("read"));
        assert_eq!(view.summary, "waiting for task");
        assert_eq!(view.task_id.as_deref(), Some("task-9"));
        assert_eq!(view.pending_reason.as_deref(), Some("long_running"));
        assert_eq!(view.check_after, Some(5000));
        assert_eq!(view.partial_output.as_deref(), Some("partial"));
        assert!(view.detail.is_some());
    }
}
