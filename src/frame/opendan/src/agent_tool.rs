use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use buckyos_api::AiToolCall;
use log::{debug, info, warn};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value as Json};
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::behavior::{BehaviorConfig, BehaviorExecInput, PolicyEngine, SessionRuntimeContext};

pub const TOOL_CREATE_SUB_AGENT: &str = "create_sub_agent";
pub use crate::buildin_tool::{TOOL_EDIT_FILE, TOOL_EXEC_BASH, TOOL_READ_FILE, TOOL_WRITE_FILE};

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
    /// Parse action call from compact forms:
    /// 1) ["action_id", {"arg":"value"}]
    /// 2) {"action_id": {"arg":"value"}}
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DoActionResults {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pwd: Option<String>,
    // do action cmd -> do result
    // "cat abc.json" -> "{"aaa":"bbb"}"
    pub details: HashMap<String, Json>,
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

    pub fn render_action_introduce_prompt(&self) -> String {
        format!("- {} : {}", self.name, self.action_introduce())
    }

    pub fn render_action_prompt(&self) -> String {
        let action_name = self.name.trim();
        let usage = format!(
            "[\"{}\", {}]",
            action_name,
            serde_json::to_string(&self.args_schema).unwrap_or_else(|_| "{}".to_string())
        );
        let args_schema =
            serde_json::to_string(&self.args_schema).unwrap_or_else(|_| "{}".to_string());
        let output_schema =
            serde_json::to_string(&self.output_schema).unwrap_or_else(|_| "{}".to_string());
        let description = format!(
            "{} Args schema: {} Output schema: {}",
            self.description.trim(),
            args_schema,
            output_schema
        );

        format!(
            "**{}**\n - Action Name: {}\n - Kind: call_tool\n - Usage: {}\n - Description: {}",
            action_name, action_name, usage, description
        )
    }

    fn action_introduce(&self) -> String {
        self.description
            .split('.')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(|| format!("Call `{}` tool action", self.name))
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

pub(crate) fn normalize_tool_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .rsplit_once('.')
        .map(|(_, suffix)| suffix.trim())
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

// render to:
// - cmd_line => result
// ```
// stdout
// stderr
// ```
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentToolResult {
    pub cmd_line: String,       // 压缩后的 cmd line
    pub result: Option<String>, // 渲染后一行字结果
    // stdout 或 stderr 有内容，就会渲染到 result 下面的 ``` ``` 中
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub details: Json,
}

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
            cmd_line: String::new(),
            result: None,
            stdout: None,
            stderr: None,
            details,
        }
    }

    pub fn with_cmd_line(mut self, cmd_line: impl Into<String>) -> Self {
        self.cmd_line = cmd_line.into();
        self
    }

    pub fn with_result(mut self, result: impl Into<String>) -> Self {
        self.result = Some(result.into());
        self
    }

    pub fn with_stdout(mut self, stdout: Option<String>) -> Self {
        self.stdout = stdout;
        self
    }

    pub fn with_stderr(mut self, stderr: Option<String>) -> Self {
        self.stderr = stderr;
        self
    }

    pub fn render_prompt(&self) -> String {
        let mut lines = Vec::<String>::new();
        let ok = self
            .details
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let summary = match (
            self.cmd_line.trim().is_empty(),
            self.result
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
        ) {
            (false, Some(result)) => format!("{} => {}", self.cmd_line.trim(), result),
            (false, None) => self.cmd_line.trim().to_string(),
            (true, Some(result)) => result.to_string(),
            (true, None) => compact_json_text(&self.details, 220),
        };
        lines.push(summary);

        if let Some(stdout) = self
            .stdout
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push("```stdout".to_string());
            lines.push(truncate_prompt_stream_lines(stdout, PROMPT_STDIO_MAX_LINES));
            lines.push("```".to_string());
        }
        if !ok {
            if let Some(stderr) = self
                .stderr
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                lines.push("```stderr".to_string());
                lines.push(truncate_prompt_stream_lines(stderr, PROMPT_STDIO_MAX_LINES));
                lines.push("```".to_string());
            }
        }
        lines.join("\n")
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

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    // Explicit namespace exposure flags.
    // Implementers can override these for precise routing.
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
    action_tools: HashMap<String, Arc<dyn AgentTool>>,
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
        if support_action {
            guard
                .action_tools
                .insert(normalized_name.clone(), registered);
        }
        if normalized_name != original_name {
            warn!(
                "tool name normalized for provider compatibility: original={} normalized={}",
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
        guard.action_tools.remove(normalized_name.as_str());
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
        guard.action_tools.get(name).cloned()
    }

    pub fn get_action_tool_spec(&self, name: &str) -> Option<ToolSpec> {
        self.get_action(name).map(|tool| tool.spec())
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
            .action_tools
            .values()
            .map(|tool| tool.spec())
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn list_action_specs(&self) -> Vec<ToolSpec> {
        self.list_action_tool_specs()
    }

    pub fn get_action_spec(&self, name: &str) -> Option<ToolSpec> {
        self.get_action_tool_spec(name)
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
            return Ok(Some(
                AgentToolResult::from_details(json!({
                    "ok": true,
                    "tool": tool_name,
                    "usage": usage,
                    "args_schema": spec.args_schema
                }))
                .with_cmd_line(line.trim().to_string())
                .with_result("show usage"),
            ));
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

pub(crate) fn tokenize_bash_command_line(line: &str) -> Result<Vec<String>, AgentToolError> {
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

fn parse_default_bash_exec_args(tokens: &[String]) -> Result<Json, AgentToolError> {
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

#[derive(Clone)]
pub struct AgentPolicy {
    tool_mgr: Arc<AgentToolManager>,
    behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
}

impl AgentPolicy {
    pub fn new(
        tool_mgr: Arc<AgentToolManager>,
        behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
    ) -> Self {
        Self {
            tool_mgr,
            behavior_cfg_cache,
        }
    }
}

#[async_trait]
impl PolicyEngine for AgentPolicy {
    async fn allowed_tools(&self, input: &BehaviorExecInput) -> Result<Vec<ToolSpec>, String> {
        let all = self.tool_mgr.list_tool_specs();
        let cfg = {
            let guard = self.behavior_cfg_cache.read().await;
            guard.get(&input.trace.behavior).cloned()
        };
        if let Some(cfg) = cfg {
            let filtered = cfg.tools.filter_tool_specs(&all);
            debug!(
                "ai_agent.policy allowed_tools: behavior={} all={} filtered={}",
                input.trace.behavior,
                all.len(),
                filtered.len()
            );
            return Ok(filtered);
        }
        debug!(
            "ai_agent.policy allowed_tools: behavior={} all={} (no_behavior_cfg)",
            input.trace.behavior,
            all.len()
        );
        Ok(all)
    }

    async fn gate_tool_calls(
        &self,
        input: &BehaviorExecInput,
        calls: &[AiToolCall],
    ) -> Result<Vec<AiToolCall>, String> {
        let allowed = self.allowed_tools(input).await?;
        let allowed_set = allowed
            .into_iter()
            .map(|item| item.name)
            .collect::<HashSet<_>>();

        let mut out = Vec::with_capacity(calls.len());
        for call in calls {
            if !allowed_set.contains(&call.name) {
                warn!(
                    "ai_agent.policy deny_tool_call: behavior={} tool={} calls={}",
                    input.trace.behavior,
                    call.name,
                    calls.len()
                );
                return Err(format!(
                    "tool `{}` is not allowed for behavior `{}`",
                    call.name, input.trace.behavior
                ));
            }
            out.push(call.clone());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_environment::AgentEnvironment;
    use crate::agent_memory::{AgentMemory, AgentMemoryConfig};
    use crate::agent_session::{AgentSessionMgr, GetSessionTool};
    use crate::ai_runtime::{AiRuntime, AiRuntimeConfig};
    use buckyos_api::value_to_object_map;
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    async fn spawn_mcp_http_server_once(
        response_json: Json,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test tcp listener");
        let addr = listener.local_addr().expect("read local addr");
        let (req_tx, req_rx) = oneshot::channel::<String>();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept tcp connection");
            let req_text = read_http_request(&mut stream).await;
            let _ = req_tx.send(req_text.clone());

            let body = serde_json::to_string(&response_json).expect("serialize response body");
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.as_bytes().len(),
                body
            );
            stream
                .write_all(resp.as_bytes())
                .await
                .expect("write response");
        });

        (format!("http://{}", addr), req_rx)
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut temp = [0_u8; 1024];
        loop {
            let n = stream.read(&mut temp).await.expect("read request");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&temp[..n]);
            if let Some(body_end) = find_header_end(&buf) {
                let content_len = parse_content_length(&buf[..body_end]).unwrap_or(0);
                let expected_total = body_end + content_len;
                if buf.len() >= expected_total {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    fn find_header_end(data: &[u8]) -> Option<usize> {
        data.windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|idx| idx + 4)
    }

    fn parse_content_length(headers: &[u8]) -> Option<usize> {
        let text = String::from_utf8_lossy(headers).to_lowercase();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("content-length:") {
                return value.trim().parse::<usize>().ok();
            }
        }
        None
    }

    fn test_call_ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-1".to_string(),
            session_id: "session-test".to_string(),
        }
    }

    async fn build_real_tool_catalog_for_review(
    ) -> (Vec<ToolSpec>, Vec<ToolSpec>, Vec<ToolSpec>, Vec<ToolSpec>) {
        let temp = tempdir().expect("create tempdir for tool catalog");
        let workspace_root = temp.path().join("workspace");
        let sessions_root = workspace_root.join("session");
        let agent_root = temp.path().join("agent");
        let runtime_agents_root = temp.path().join("runtime_agents");

        let tool_mgr = AgentToolManager::new();
        let session_store = Arc::new(
            AgentSessionMgr::new("did:example:agent", sessions_root, "on_wakeup".to_string())
                .await
                .expect("create session store"),
        );

        let environment = AgentEnvironment::new(workspace_root)
            .await
            .expect("create agent environment");
        environment
            .register_workshop_tools(&tool_mgr, session_store.clone())
            .expect("register workshop tools");

        let memory = AgentMemory::new(AgentMemoryConfig::new(agent_root))
            .await
            .expect("create agent memory");
        memory
            .register_tools(&tool_mgr)
            .expect("register memory tools");

        tool_mgr
            .register_tool(GetSessionTool::new(session_store))
            .expect("register get_session tool");

        let runtime = AiRuntime::new(AiRuntimeConfig::new(runtime_agents_root))
            .await
            .expect("create ai runtime");
        runtime
            .register_tools(&tool_mgr)
            .await
            .expect("register runtime tools");

        (
            tool_mgr.list_tool_specs(),
            tool_mgr.list_bash_cmd_specs(),
            tool_mgr.list_action_tool_specs(),
            tool_mgr.list_action_specs(),
        )
    }

    #[tokio::test]
    async fn print_all_tools() {
        let (tool_specs, bash_specs, action_tool_specs, action_specs) =
            build_real_tool_catalog_for_review().await;

        println!("\n================ TOOL NAMESPACE (LLM) ================");
        println!("[List Mode] name + summary");
        for spec in &tool_specs {
            println!("- {} : {}", spec.name, spec.description);
        }

        println!("\n[Detail Mode] one tool spec per block");
        for spec in &tool_specs {
            println!("\n### TOOL {}", spec.name);
            println!(
                "{}",
                serde_json::to_string_pretty(spec)
                    .unwrap_or_else(|_| "{\"error\":\"serialize failed\"}".to_string())
            );
        }

        println!("\n================ BASH NAMESPACE ================");
        println!("[List Mode] name + summary");
        for spec in &bash_specs {
            println!("- {} : {}", spec.name, spec.description);
            println!("{:?}", spec.usage);
        }

        println!("\n================ ACTION PROMPTS ================");
        println!("[List Mode] name + introduce");
        for spec in &action_specs {
            println!("{}", spec.render_action_introduce_prompt());
        }

        println!("\n[Detail Mode] one action prompt per block");
        for spec in &action_specs {
            println!("\n### ACTION {}", spec.name);
            println!("{}", spec.render_action_prompt());
        }

        assert!(
            !tool_specs.is_empty(),
            "documented tool specs should not be empty"
        );
        assert!(
            !bash_specs.is_empty(),
            "bash namespace tool specs should not be empty"
        );
        assert!(
            !action_tool_specs.is_empty(),
            "action namespace tool specs should not be empty"
        );
        assert!(
            !action_specs.is_empty(),
            "documented action specs should not be empty"
        );
    }

    #[test]
    fn action_call_accepts_compact_object_form() {
        let call: ActionCall = serde_json::from_value(json!({
            "write": {
                "path": "a.txt",
                "content": "ok"
            }
        }))
        .expect("object form should deserialize");
        assert_eq!(call.call_action_name, "write");
        assert_eq!(call.call_params["path"], "a.txt");
    }

    #[test]
    fn tool_spec_render_action_prompt_includes_schema_details() {
        let spec = ToolSpec {
            name: TOOL_EXEC_BASH.to_string(),
            description: "run shell command".to_string(),
            args_schema: json!({"type":"object","properties":{"command":{"type":"string"}}}),
            output_schema: json!({"type":"object"}),
            usage: None,
        };
        let rendered = spec.render_action_prompt();
        assert!(rendered.contains("Action Name: exec"));
        assert!(rendered.contains("Kind: call_tool"));
        assert!(rendered.contains("[\"exec\","));
        assert!(rendered.contains("Args schema"));
    }

    #[test]
    fn agent_tool_result_render_prompt_truncates_stdout_by_lines() {
        let stdout = (0..(PROMPT_STDIO_MAX_LINES + 20))
            .map(|idx| format!("line-{idx:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = AgentToolResult::from_details(json!({"ok": true}))
            .with_cmd_line("read_file ./large.txt")
            .with_result("read 12345 bytes (truncated)")
            .with_stdout(Some(stdout))
            .render_prompt();
        let last_kept = format!("line-{:04}", PROMPT_STDIO_MAX_LINES - 1);
        let first_dropped = format!("line-{:04}", PROMPT_STDIO_MAX_LINES);
        let truncated_hint = format!(
            "... [TRUNCATED FOR ACTION PREVIEW: Showing first {} lines only] ...",
            PROMPT_STDIO_MAX_LINES
        );

        assert!(rendered.contains("```stdout"));
        assert!(rendered.contains("line-0000"));
        assert!(rendered.contains(last_kept.as_str()));
        assert!(!rendered.contains(first_dropped.as_str()));
        assert!(rendered.contains(truncated_hint.as_str()));
        assert!(rendered.ends_with("```"));
    }

    #[test]
    fn agent_tool_result_render_prompt_truncates_stderr_by_lines_when_failed() {
        let stderr = (0..(PROMPT_STDIO_MAX_LINES + 20))
            .map(|idx| format!("err-{idx:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = AgentToolResult::from_details(json!({"ok": false}))
            .with_cmd_line("some-long-bash")
            .with_result("FAILED (exit=1)")
            .with_stderr(Some(stderr))
            .render_prompt();
        let last_kept = format!("err-{:04}", PROMPT_STDIO_MAX_LINES - 1);
        let first_dropped = format!("err-{:04}", PROMPT_STDIO_MAX_LINES);
        let truncated_hint = format!(
            "... [TRUNCATED FOR ACTION PREVIEW: Showing first {} lines only] ...",
            PROMPT_STDIO_MAX_LINES
        );

        assert!(rendered.contains("```stderr"));
        assert!(rendered.contains("err-0000"));
        assert!(rendered.contains(last_kept.as_str()));
        assert!(!rendered.contains(first_dropped.as_str()));
        assert!(rendered.contains(truncated_hint.as_str()));
        assert!(rendered.ends_with("```"));
    }

    #[tokio::test]
    async fn load_memory_tool_not_exposed_to_action_namespace() {
        let temp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(temp.path()))
            .await
            .expect("create agent memory");
        let mgr = AgentToolManager::new();
        memory.register_tools(&mgr).expect("register memory tools");

        let runtime_spec = mgr
            .get_tool_spec(TOOL_LOAD_MEMORY)
            .expect("load_memory tool should be registered");
        assert_eq!(runtime_spec.name, TOOL_LOAD_MEMORY);
        assert!(
            mgr.get_action_tool_spec(TOOL_LOAD_MEMORY).is_none(),
            "load_memory support_action=false should keep it out of action namespace"
        );
    }

    struct DummyTool {
        name: String,
    }

    #[async_trait]
    impl AgentTool for DummyTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: "dummy".to_string(),
                args_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
                usage: None,
            }
        }

        fn support_bash(&self) -> bool {
            false
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
            _args: Json,
        ) -> Result<AgentToolResult, AgentToolError> {
            Ok(AgentToolResult::from_details(json!({"ok": true})).with_result("ok"))
        }
    }

    struct EchoArgsTool {
        name: String,
        args_schema: Json,
        usage: Option<String>,
    }

    struct StrictArgsTool {
        name: String,
        usage: Option<String>,
    }

    struct NamespaceFilteredTool {
        name: String,
        usage: Option<String>,
        support_bash: bool,
        support_action: bool,
        support_llm_tool_call: bool,
    }

    #[async_trait]
    impl AgentTool for EchoArgsTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: "echo args".to_string(),
                args_schema: self.args_schema.clone(),
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
            false
        }
        async fn call(
            &self,
            _ctx: &SessionRuntimeContext,
            args: Json,
        ) -> Result<AgentToolResult, AgentToolError> {
            Ok(AgentToolResult::from_details(json!({"ok": true, "args": args})).with_result("ok"))
        }

        async fn exec(
            &self,
            _ctx: &SessionRuntimeContext,
            line: &str,
            shell_cwd: Option<&Path>,
        ) -> Result<AgentToolResult, AgentToolError> {
            let tokens = tokenize_bash_command_line(line)?;
            if tokens.is_empty() {
                return Err(AgentToolError::InvalidArgs(
                    "empty bash command line".to_string(),
                ));
            }
            let arg_tokens = &tokens[1..];
            let mut out = serde_json::Map::<String, Json>::new();
            let has_key_value = arg_tokens.iter().any(|token| token.contains('='));

            if has_key_value {
                if arg_tokens.iter().any(|token| !token.contains('=')) {
                    return Err(AgentToolError::InvalidArgs(
                        "bash args cannot mix positional args with key=value args".to_string(),
                    ));
                }
                for token in arg_tokens {
                    let (raw_key, raw_value) = token.split_once('=').ok_or_else(|| {
                        AgentToolError::InvalidArgs("invalid key=value token".to_string())
                    })?;
                    let key = raw_key.trim();
                    if key.is_empty() {
                        return Err(AgentToolError::InvalidArgs(
                            "arg key cannot be empty".to_string(),
                        ));
                    }
                    out.insert(key.to_string(), Json::String(raw_value.trim().to_string()));
                }
            } else {
                if arg_tokens.len() > 2 {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "too many positional args for tool `{}`: got {}, max 2",
                        self.name,
                        arg_tokens.len()
                    )));
                }
                if let Some(path) = arg_tokens.first() {
                    out.insert("path".to_string(), Json::String(path.trim().to_string()));
                }
                if let Some(range) = arg_tokens.get(1) {
                    out.insert("range".to_string(), Json::String(range.trim().to_string()));
                }
            }

            if let Some(shell_cwd) = shell_cwd {
                if let Some(raw_path) = out.get("path").and_then(|value| value.as_str()) {
                    let parsed = Path::new(raw_path);
                    if !parsed.is_absolute() {
                        out.insert(
                            "path".to_string(),
                            Json::String(shell_cwd.join(parsed).to_string_lossy().to_string()),
                        );
                    }
                }
            }
            Ok(
                AgentToolResult::from_details(json!({"ok": true, "args": Json::Object(out)}))
                    .with_cmd_line(line.trim().to_string())
                    .with_result("ok"),
            )
        }
    }

    #[async_trait]
    impl AgentTool for StrictArgsTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: "strict args".to_string(),
                args_schema: json!({
                    "type":"object",
                    "properties": {
                        "path": {"type":"string"}
                    },
                    "required": ["path"]
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
            false
        }

        async fn call(
            &self,
            _ctx: &SessionRuntimeContext,
            args: Json,
        ) -> Result<AgentToolResult, AgentToolError> {
            if args.get("path").and_then(|value| value.as_str()).is_none() {
                return Err(AgentToolError::InvalidArgs(
                    "missing required arg `path`".to_string(),
                ));
            }
            Ok(AgentToolResult::from_details(json!({"ok": true})).with_result("ok"))
        }
    }

    #[async_trait]
    impl AgentTool for NamespaceFilteredTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: "namespace filtered".to_string(),
                args_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
                usage: self.usage.clone(),
            }
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
            _ctx: &SessionRuntimeContext,
            _args: Json,
        ) -> Result<AgentToolResult, AgentToolError> {
            Ok(AgentToolResult::from_details(json!({"ok": true})).with_result("ok"))
        }
    }

    #[tokio::test]
    async fn register_tool_normalizes_module_prefixed_name_without_alias() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(DummyTool {
            name: "workshop.exec_bash".to_string(),
        })
        .expect("register tool");

        assert!(mgr.has_tool("exec_bash"));
        assert!(!mgr.has_tool("workshop.exec_bash"));

        let specs = mgr.list_tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "exec_bash");
        let action_specs = mgr.list_action_specs();
        assert_eq!(action_specs.len(), 1);
        assert_eq!(action_specs[0].name, "exec_bash");
        assert!(action_specs[0]
            .render_action_introduce_prompt()
            .contains("dummy"));

        let err = mgr
            .call_tool(
                &test_call_ctx(),
                AiToolCall {
                    name: "workshop.exec_bash".to_string(),
                    args: value_to_object_map(json!({})),
                    call_id: "call-1".to_string(),
                },
            )
            .await
            .expect_err("legacy alias should not call");
        assert!(matches!(err, AgentToolError::NotFound(_)));

        mgr.call_tool(
            &test_call_ctx(),
            AiToolCall {
                name: "exec_bash".to_string(),
                args: value_to_object_map(json!({})),
                call_id: "call-2".to_string(),
            },
        )
        .await
        .expect("normalized name should call");
    }

    #[test]
    fn unregister_tool_by_normalized_name() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(DummyTool {
            name: "workshop.exec_bash".to_string(),
        })
        .expect("register tool");

        assert!(mgr.get_action_spec("exec_bash").is_some());
        assert!(mgr.unregister_tool("exec_bash"));
        assert!(!mgr.has_tool("exec_bash"));
        assert!(mgr.get_action_spec("exec_bash").is_none());
    }

    #[tokio::test]
    async fn explicit_namespace_flags_take_priority_over_tool_spec_usage() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(NamespaceFilteredTool {
            name: "namespaced_tool".to_string(),
            usage: Some("namespaced_tool --help".to_string()),
            support_bash: false,
            support_action: true,
            support_llm_tool_call: false,
        })
        .expect("register tool");

        assert!(mgr.get_tool("namespaced_tool").is_none());
        assert!(mgr.get_bash_cmd("namespaced_tool").is_none());
        assert!(mgr.get_action("namespaced_tool").is_some());
        assert!(mgr.get_action_spec("namespaced_tool").is_some());
        assert!(mgr
            .list_tool_specs()
            .iter()
            .all(|item| item.name != "namespaced_tool"));

        let result = mgr
            .call_tool(
                &test_call_ctx(),
                AiToolCall {
                    name: "namespaced_tool".to_string(),
                    args: value_to_object_map(json!({})),
                    call_id: "call-action-only-1".to_string(),
                },
            )
            .await
            .expect("registered action namespace tool should still be callable");
        assert_eq!(result.details["ok"], true);

        let bash_match = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "namespaced_tool --help")
            .await
            .expect("bash lookup should complete");
        assert!(bash_match.is_none());
    }

    #[test]
    fn register_tool_must_expose_at_least_one_namespace() {
        let mgr = AgentToolManager::new();
        let err = mgr
            .register_tool(NamespaceFilteredTool {
                name: "hidden_tool".to_string(),
                usage: Some("hidden_tool".to_string()),
                support_bash: false,
                support_action: false,
                support_llm_tool_call: false,
            })
            .expect_err("tool with no namespace exposure should fail");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
        assert!(err
            .to_string()
            .contains("must support at least one namespace"));
    }

    #[tokio::test]
    async fn call_tool_from_bash_line_supports_positional_style() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(EchoArgsTool {
            name: "read_file".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "range": {"type": "string"}
                }
            }),
            usage: Some("read_file <path> [range]".to_string()),
        })
        .expect("register tool");

        let result = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file ~/1.txt 0:200")
            .await
            .expect("bash style call should succeed")
            .expect("tool should be matched");

        assert_eq!(result["ok"], true);
        assert_eq!(result.details["args"]["path"], "~/1.txt");
        assert_eq!(result.details["args"]["range"], "0:200");
    }

    #[tokio::test]
    async fn call_tool_from_bash_line_supports_key_value_style() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(EchoArgsTool {
            name: "read_file".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "range": {"type": "string"}
                }
            }),
            usage: Some("read_file path=<path> [range=<range>]".to_string()),
        })
        .expect("register tool");

        let result = mgr
            .call_tool_from_bash_line(
                &test_call_ctx(),
                "read_file path=\"~/1.txt\" range=\"0:200\"",
            )
            .await
            .expect("bash style kv call should succeed")
            .expect("tool should be matched");

        assert_eq!(result.details["ok"], true);
        assert_eq!(result.details["args"]["path"], "~/1.txt");
        assert_eq!(result.details["args"]["range"], "0:200");
    }

    #[tokio::test]
    async fn call_tool_from_bash_line_rewrites_relative_path_with_shell_cwd() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(EchoArgsTool {
            name: "read_file".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "range": {"type": "string"}
                }
            }),
            usage: Some("read_file <path> [range]".to_string()),
        })
        .expect("register tool");

        let result = mgr
            .call_tool_from_bash_line_with_cwd(
                &test_call_ctx(),
                "read_file 1.txt 1:1",
                Some(std::path::Path::new("/tmp/opendan-shell-cwd")),
            )
            .await
            .expect("bash style cwd rewrite call should succeed")
            .expect("tool should be matched");

        assert_eq!(result.details["ok"], true);
        assert_eq!(
            result.details["args"]["path"],
            "/tmp/opendan-shell-cwd/1.txt"
        );
        assert_eq!(result.details["args"]["range"], "1:1");
    }

    #[tokio::test]
    async fn call_tool_from_bash_line_skips_tool_without_bash_namespace() {
        let mgr = AgentToolManager::new();
        mgr.register_tool(NamespaceFilteredTool {
            name: "read_file".to_string(),
            usage: Some("read_file".to_string()),
            support_bash: false,
            support_action: true,
            support_llm_tool_call: false,
        })
        .expect("register tool");

        let result = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file 1.txt")
            .await
            .expect("non-bash tool lookup should complete");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn call_tool_from_bash_line_help_outputs_usage() {
        let mgr = AgentToolManager::new();
        let usage = "read_file <path> [range]";
        mgr.register_tool(EchoArgsTool {
            name: "read_file".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "range": {"type": "string"}
                }
            }),
            usage: Some(usage.to_string()),
        })
        .expect("register tool");

        let result = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file --help")
            .await
            .expect("help should succeed")
            .expect("tool should be matched");
        assert_eq!(result.details["ok"], true);
        assert_eq!(result.details["usage"], usage);
    }

    #[tokio::test]
    async fn call_tool_from_bash_line_invalid_args_reports_usage() {
        let mgr = AgentToolManager::new();
        let usage = "read_file <path>";
        mgr.register_tool(StrictArgsTool {
            name: "read_file".to_string(),
            usage: Some(usage.to_string()),
        })
        .expect("register tool");

        let err = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file")
            .await
            .expect_err("missing arg should fail");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
        let err_text = err.to_string();
        assert!(err_text.contains("missing required arg `path`"));
        assert!(err_text.contains("Usage:"));
        assert!(err_text.contains(usage));
    }

    #[tokio::test]
    async fn call_tool_from_bash_line_parse_error_reports_usage() {
        let mgr = AgentToolManager::new();
        let usage = "read_file <path> [range]";
        mgr.register_tool(EchoArgsTool {
            name: "read_file".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "range": {"type": "string"}
                }
            }),
            usage: Some(usage.to_string()),
        })
        .expect("register tool");

        let err = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file 1.txt 1:2 extra")
            .await
            .expect_err("too many positional args should fail");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
        let err_text = err.to_string();
        assert!(err_text.contains("too many positional args"));
        assert!(err_text.contains("Usage:"));
        assert!(err_text.contains(usage));
    }

    #[tokio::test]
    async fn mcp_tool_can_call_jsonrpc_tools_call() {
        let (endpoint, req_rx) = spawn_mcp_http_server_once(json!({
            "jsonrpc": "2.0",
            "id": "x",
            "result": {
                "isError": false,
                "content": [{"type":"text","text":"done"}],
                "data": {"answer": 42}
            }
        }))
        .await;

        let tool = MCPTool::new(MCPToolConfig {
            name: "mcp.echo".to_string(),
            endpoint,
            mcp_tool_name: Some("echo".to_string()),
            description: Some("echo mcp".to_string()),
            args_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
            headers: HashMap::new(),
            timeout_ms: 5_000,
        })
        .expect("create mcp tool");

        let output = tool
            .call(&test_call_ctx(), json!({"message":"hello"}))
            .await
            .expect("mcp tool call should succeed");

        assert_eq!(output.details["data"]["answer"], 42);

        let request = req_rx.await.expect("receive http request");
        assert!(request.contains("\"method\":\"tools/call\""));
        assert!(request.contains("\"name\":\"echo\""));
        assert!(request.contains("\"message\":\"hello\""));
    }

    #[tokio::test]
    async fn mcp_tool_maps_jsonrpc_error() {
        let (endpoint, _req_rx) = spawn_mcp_http_server_once(json!({
            "jsonrpc": "2.0",
            "id": "x",
            "error": {
                "code": -32000,
                "message": "boom"
            }
        }))
        .await;

        let tool = MCPTool::new(MCPToolConfig {
            name: "mcp.fail".to_string(),
            endpoint,
            mcp_tool_name: Some("fail".to_string()),
            description: None,
            args_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
            headers: HashMap::new(),
            timeout_ms: 5_000,
        })
        .expect("create mcp tool");

        let err = tool
            .call(&test_call_ctx(), json!({}))
            .await
            .expect_err("mcp jsonrpc error should fail");

        assert!(matches!(err, AgentToolError::ExecFailed(_)));
        assert!(err.to_string().contains("boom"));
    }
}
