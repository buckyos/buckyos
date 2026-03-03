use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use buckyos_api::{value_to_object_map, AiToolCall};
use log::{debug, info, warn};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value as Json};
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::behavior::{BehaviorConfig, BehaviorExecInput, PolicyEngine, SessionRuntimeContext};
use crate::buildin_tool::{builtin_tool_args_schema, builtin_tool_summary};

pub const TOOL_CREATE_SUB_AGENT: &str = "create_sub_agent";
pub use crate::buildin_tool::{TOOL_EDIT_FILE, TOOL_EXEC_BASH, TOOL_READ_FILE, TOOL_WRITE_FILE};

pub const TOOL_GET_SESSION: &str = "get_session";
pub const TOOL_LIST_SESSION: &str = "list_session";
pub const TOOL_LIST_EXTERNAL_WORKSPACES: &str = "list_external_workspaces";
pub const TOOL_BIND_EXTERNAL_WORKSPACE: &str = "bind_external_workspace";
pub const TOOL_CREATE_LOCAL_WORKSPACE: &str = "create_local_workspace";
pub const TOOL_BIND_LOCAL_WORKSPACE: &str = "bind_local_workspace";
pub const TOOL_LOAD_MEMORY: &str = "load_memory";
pub const TOOL_TODO_MANAGE: &str = "todo_manage";
pub const TOOL_WORKLOG_MANAGE: &str = "worklog_manage";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    CallTool, //指向一个内置的tool
              //ExecScript,//运行一个特定的脚本
}

impl Default for ActionKind {
    fn default() -> Self {
        Self::CallTool
    }
}

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionSpec {
    #[serde(default)]
    pub kind: ActionKind,
    pub name: String,
    pub introduce: String,
    pub description: Option<String>,
}

impl ActionSpec {
    pub fn render_introduce_prompt(&self) -> String {
        return format!("- {} : {}", self.name, self.introduce);
    }

    pub fn render_prompt(&self) -> String {
        let action_name = self.name.trim();
        let usage = render_action_usage(action_name);
        let description = self
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(|| render_action_description(action_name));

        format!(
            "**{}**\n - Action Name: {}\n - Kind: call_tool\n - Usage: {}\n - Description: {}",
            action_name, action_name, usage, description
        )
    }
}

fn render_action_usage(action_name: &str) -> String {
    let schema = builtin_action_args_schema(action_name);
    format!(
        "[\"{}\", {}]",
        action_name,
        serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
    )
}

fn render_action_description(action_name: &str) -> String {
    let summary = builtin_action_summary(action_name);
    let args_schema = builtin_action_args_schema(action_name);
    let args_text = serde_json::to_string(&args_schema).unwrap_or_else(|_| "{}".to_string());
    format!("{summary} Args schema: {args_text}")
}

fn builtin_action_summary(action_name: &str) -> &'static str {
    if let Some(summary) = builtin_tool_summary(action_name) {
        return summary;
    }

    match action_name {
        TOOL_CREATE_SUB_AGENT => "Create a sub-agent execution session.",
        TOOL_GET_SESSION => "Get current session detail.",
        TOOL_LIST_SESSION => "List available sessions.",
        TOOL_LIST_EXTERNAL_WORKSPACES => "List bindable external workspaces.",
        TOOL_BIND_EXTERNAL_WORKSPACE => "Bind an external workspace to current session.",
        TOOL_CREATE_LOCAL_WORKSPACE => "Create and optionally bind a local workspace.",
        TOOL_BIND_LOCAL_WORKSPACE => {
            "Bind an existing local workspace to current session (without rebind)."
        }
        TOOL_LOAD_MEMORY => "Load memory entries by token budget, tags, and reference time.",
        TOOL_TODO_MANAGE => "Manage workspace todos.",
        _ => "Call runtime tool action.",
    }
}

fn builtin_action_args_schema(action_name: &str) -> Json {
    if let Some(schema) = builtin_tool_args_schema(action_name) {
        return schema;
    }

    match action_name {
        TOOL_CREATE_SUB_AGENT => json!({
            "type": "object",
            "properties": {
                "role": { "type": "string" },
                "goal": { "type": "string" }
            }
        }),
        TOOL_GET_SESSION => json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            }
        }),
        TOOL_LIST_SESSION => json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1 }
            }
        }),
        TOOL_LIST_EXTERNAL_WORKSPACES => json!({
            "type": "object",
            "properties": {
                "provider": { "type": "string" }
            }
        }),
        TOOL_BIND_EXTERNAL_WORKSPACE => json!({
            "type": "object",
            "properties": {
                "workspace_id": { "type": "string" }
            },
            "required": ["workspace_id"]
        }),
        TOOL_CREATE_LOCAL_WORKSPACE => json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "template": { "type": "string" },
                "owner": {
                    "type": "string",
                    "enum": ["agent_created", "user_provided"]
                },
                "policy_profile_id": { "type": "string" },
                "created_by_session": { "type": "string" },
                "session_id": { "type": "string" },
                "bind_session": { "type": "boolean" }
            },
            "required": ["name"]
        }),
        TOOL_BIND_LOCAL_WORKSPACE => json!({
            "type": "object",
            "properties": {
                "local_workspace_id": { "type": "string" },
                "session_id": { "type": "string" }
            },
            "required": ["local_workspace_id"]
        }),
        TOOL_LOAD_MEMORY => json!({
            "type": "object",
            "properties": {
                "token_limit": { "type":"number" },
                "tags": {
                    "type":"array",
                    "items": { "type":"string" }
                },
                "current_time": { "type":"string" }
            }
        }),
        TOOL_TODO_MANAGE => json!({
            "type": "object",
            "properties": {
                "ops": { "type": "array" },
                "workspace_id": { "type": "string" }
            },
            "required": ["ops"]
        }),
        _ => json!({
            "type": "object"
        }),
    }
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
    // do action cmd -> do result
    // "cat abc.json" -> "{"aaa":"bbb"}"
    pub details: HashMap<String, Json>,
}

pub struct AgentSkillRecord {
    pub name: String,
    pub introduce: String,
}

pub struct AgentSkillSpec {
    pub introduce: String,
    pub rules: String,
    //先不支持自定义action,只能引用runtime里已经定义好的Action
    pub actions: Vec<String>,
    //先不支持自定义tool,只能引用runtime里已经定义好的tool
    pub loaded_tools: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub args_schema: Json,
    pub output_schema: Json,
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

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, ctx: &SessionRuntimeContext, args: Json) -> Result<Json, AgentToolError>;
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

    async fn call(&self, ctx: &SessionRuntimeContext, args: Json) -> Result<Json, AgentToolError> {
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

struct RegisteredTool {
    spec: ToolSpec,
    inner: Arc<dyn AgentTool>,
}

#[async_trait]
impl AgentTool for RegisteredTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn call(&self, ctx: &SessionRuntimeContext, args: Json) -> Result<Json, AgentToolError> {
        self.inner.call(ctx, args).await
    }
}

pub struct AgentActionManager {
    actions: StdRwLock<HashMap<String, ActionSpec>>,
}

impl Default for AgentActionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentActionManager {
    pub fn new() -> Self {
        Self {
            actions: StdRwLock::new(HashMap::new()),
        }
    }

    pub fn register_action_spec(&self, mut spec: ActionSpec) -> Result<(), AgentToolError> {
        let normalized_name = normalize_tool_name(spec.name.as_str());
        if normalized_name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "action name cannot be empty".to_string(),
            ));
        }
        spec.name = normalized_name.clone();
        spec.introduce = spec.introduce.trim().to_string();
        if spec.introduce.is_empty() {
            spec.introduce = format!("Call `{}` action", spec.name);
        }
        spec.description = spec
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let mut guard = self
            .actions
            .write()
            .map_err(|_| AgentToolError::ExecFailed("action registry lock poisoned".to_string()))?;
        if guard.contains_key(&normalized_name) {
            return Err(AgentToolError::AlreadyExists(normalized_name));
        }
        guard.insert(normalized_name, spec);
        Ok(())
    }

    pub fn unregister_action_spec(&self, name: &str) -> bool {
        let normalized_name = normalize_tool_name(name);
        if normalized_name.is_empty() {
            return false;
        }
        let Ok(mut guard) = self.actions.write() else {
            return false;
        };
        guard.remove(normalized_name.as_str()).is_some()
    }

    pub fn has_action_spec(&self, name: &str) -> bool {
        let normalized_name = normalize_tool_name(name);
        if normalized_name.is_empty() {
            return false;
        }
        let Ok(guard) = self.actions.read() else {
            return false;
        };
        guard.contains_key(normalized_name.as_str())
    }

    pub fn get_action_spec(&self, name: &str) -> Option<ActionSpec> {
        let normalized_name = normalize_tool_name(name);
        if normalized_name.is_empty() {
            return None;
        }
        let Ok(guard) = self.actions.read() else {
            return None;
        };
        guard.get(normalized_name.as_str()).cloned()
    }

    pub fn list_action_specs(&self) -> Vec<ActionSpec> {
        let Ok(guard) = self.actions.read() else {
            return vec![];
        };
        let mut specs = guard.values().cloned().collect::<Vec<_>>();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }
}

fn action_spec_from_tool_spec(spec: &ToolSpec) -> ActionSpec {
    let name = normalize_tool_name(spec.name.as_str());
    let introduce = spec
        .description
        .split('.')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("Call `{}` tool action", name));
    let args_schema = serde_json::to_string(&spec.args_schema).unwrap_or_else(|_| "{}".to_string());
    let output_schema =
        serde_json::to_string(&spec.output_schema).unwrap_or_else(|_| "{}".to_string());
    let description = Some(format!(
        "{} Args schema: {} Output schema: {}",
        spec.description.trim(),
        args_schema,
        output_schema
    ));

    ActionSpec {
        kind: ActionKind::CallTool,
        name,
        introduce,
        description,
    }
}

#[derive(Clone)]
pub struct AgentToolManager {
    tools: Arc<StdRwLock<HashMap<String, Arc<dyn AgentTool>>>>,
    actions: Arc<AgentActionManager>,
}

impl Default for AgentToolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentToolManager {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(StdRwLock::new(HashMap::new())),
            actions: Arc::new(AgentActionManager::new()),
        }
    }

    pub fn action_mgr(&self) -> &AgentActionManager {
        self.actions.as_ref()
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
        let action_spec = action_spec_from_tool_spec(&spec);
        let registered: Arc<dyn AgentTool> = Arc::new(RegisteredTool { spec, inner: tool });

        let mut guard = self
            .tools
            .write()
            .map_err(|_| AgentToolError::ExecFailed("tool registry lock poisoned".to_string()))?;
        if guard.contains_key(&normalized_name) {
            return Err(AgentToolError::AlreadyExists(normalized_name));
        }

        self.actions.register_action_spec(action_spec)?;
        guard.insert(normalized_name.clone(), registered);
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
        let Ok(mut guard) = self.tools.write() else {
            return false;
        };
        let removed = guard.remove(normalized_name.as_str()).is_some();
        if removed {
            let _ = self
                .actions
                .unregister_action_spec(normalized_name.as_str());
        }
        removed
    }

    pub fn has_tool(&self, name: &str) -> bool {
        let Ok(guard) = self.tools.read() else {
            return false;
        };
        guard.contains_key(name)
    }

    pub fn get_tool(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        let Ok(guard) = self.tools.read() else {
            return None;
        };
        guard.get(name).cloned()
    }

    pub fn get_tool_spec(&self, name: &str) -> Option<ToolSpec> {
        self.get_tool(name).map(|tool| tool.spec())
    }

    pub fn list_tool_specs(&self) -> Vec<ToolSpec> {
        let Ok(guard) = self.tools.read() else {
            return vec![];
        };
        let mut specs: Vec<ToolSpec> = guard.values().map(|tool| tool.spec()).collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn list_action_specs(&self) -> Vec<ActionSpec> {
        self.actions.list_action_specs()
    }

    pub fn get_action_spec(&self, name: &str) -> Option<ActionSpec> {
        self.actions.get_action_spec(name)
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
        let Ok(guard) = self.tools.read() else {
            return None;
        };
        guard
            .contains_key(normalized.as_str())
            .then_some(normalized)
    }

    pub async fn call_tool_from_bash_line(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
    ) -> Result<Option<Json>, AgentToolError> {
        self.call_tool_from_bash_line_with_cwd(ctx, line, None)
            .await
    }

    pub async fn call_tool_from_bash_line_with_cwd(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<Option<Json>, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Ok(None);
        }

        let tool_name = normalize_tool_name(tokens[0].as_str());
        if tool_name.is_empty() {
            return Ok(None);
        }
        let Some(tool) = self.get_tool(tool_name.as_str()) else {
            return Ok(None);
        };

        let mut args = parse_bash_style_tool_args(&tool.spec(), &tokens[1..])?;
        if let Some(shell_cwd) = shell_cwd {
            rewrite_path_args_for_shell_cwd(tool_name.as_str(), &mut args, shell_cwd);
        }
        let result = self
            .call_tool(
                ctx,
                AiToolCall {
                    name: tool_name,
                    args: value_to_object_map(args),
                    call_id: format!("bash-cli-{}-{}", ctx.trace_id, ctx.step_idx),
                },
            )
            .await?;
        Ok(Some(result))
    }

    pub async fn call_tool(
        &self,
        ctx: &SessionRuntimeContext,
        call: AiToolCall,
    ) -> Result<Json, AgentToolError> {
        let tool_name = call.name;
        let call_id = call.call_id;
        let args = Json::Object(call.args.into_iter().collect());
        let session_id = ctx.session_id.as_str();

        info!(
            "opendan.tool_call: status=start tool={} call_id={} trace_id={} session_id={}",
            tool_name, call_id, ctx.trace_id, session_id
        );

        let Some(tool) = self.get_tool(&tool_name) else {
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
}

fn tokenize_bash_command_line(line: &str) -> Result<Vec<String>, AgentToolError> {
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

fn parse_bash_style_tool_args(spec: &ToolSpec, tokens: &[String]) -> Result<Json, AgentToolError> {
    let properties = spec
        .args_schema
        .get("properties")
        .and_then(|value| value.as_object());
    let mut out = serde_json::Map::<String, Json>::new();

    let has_key_value = tokens.iter().any(|token| token.contains('='));
    if has_key_value {
        if tokens.iter().any(|token| !token.contains('=')) {
            return Err(AgentToolError::InvalidArgs(
                "bash style tool args cannot mix positional args with key=value args".to_string(),
            ));
        }
        for token in tokens {
            let (raw_key, raw_value) = token.split_once('=').ok_or_else(|| {
                AgentToolError::InvalidArgs("invalid key=value token".to_string())
            })?;
            let key = raw_key.trim();
            if key.is_empty() {
                return Err(AgentToolError::InvalidArgs(
                    "arg key cannot be empty".to_string(),
                ));
            }
            let schema = properties.and_then(|props| props.get(key));
            out.insert(
                key.to_string(),
                parse_bash_arg_value(raw_value.trim(), schema),
            );
        }
        return Ok(Json::Object(out));
    }

    if tokens.is_empty() {
        return Ok(Json::Object(out));
    }
    let Some(props) = properties else {
        return Err(AgentToolError::InvalidArgs(format!(
            "tool `{}` does not support positional bash args",
            spec.name
        )));
    };
    let keys = build_positional_arg_keys(spec, props);
    if tokens.len() > keys.len() {
        return Err(AgentToolError::InvalidArgs(format!(
            "too many positional args for tool `{}`: got {}, max {}",
            spec.name,
            tokens.len(),
            keys.len()
        )));
    }

    for (idx, token) in tokens.iter().enumerate() {
        let key = &keys[idx];
        let schema = props.get(key.as_str());
        out.insert(key.clone(), parse_bash_arg_value(token.as_str(), schema));
    }
    Ok(Json::Object(out))
}

fn build_positional_arg_keys(
    spec: &ToolSpec,
    props: &serde_json::Map<String, Json>,
) -> Vec<String> {
    if spec.name == TOOL_READ_FILE {
        return ["path", "range", "first_chunk"]
            .iter()
            .filter(|key| props.contains_key(**key))
            .map(|key| (*key).to_string())
            .collect();
    }

    let mut keys = Vec::<String>::new();
    if let Some(required) = spec
        .args_schema
        .get("required")
        .and_then(|value| value.as_array())
    {
        for key in required {
            if let Some(key) = key.as_str() {
                let key = key.trim();
                if !key.is_empty() && props.contains_key(key) && !keys.iter().any(|k| k == key) {
                    keys.push(key.to_string());
                }
            }
        }
    }
    for key in props.keys() {
        if !keys.iter().any(|item| item == key) {
            keys.push(key.clone());
        }
    }
    keys
}

fn parse_bash_arg_value(raw: &str, schema: Option<&Json>) -> Json {
    let value = raw.trim();
    if value.is_empty() {
        return Json::String(String::new());
    }
    let type_hint = schema
        .and_then(|item| item.get("type"))
        .and_then(|item| item.as_str());
    match type_hint {
        Some("boolean") => match value.to_ascii_lowercase().as_str() {
            "true" => Json::Bool(true),
            "false" => Json::Bool(false),
            _ => Json::String(value.to_string()),
        },
        Some("integer") => value
            .parse::<i64>()
            .map(Json::from)
            .unwrap_or_else(|_| Json::String(value.to_string())),
        Some("number") => value
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Json::Number)
            .unwrap_or_else(|| Json::String(value.to_string())),
        Some("array") | Some("object") => {
            serde_json::from_str(value).unwrap_or_else(|_| Json::String(value.to_string()))
        }
        _ => Json::String(value.to_string()),
    }
}

fn rewrite_path_args_for_shell_cwd(tool_name: &str, args: &mut Json, shell_cwd: &Path) {
    let path_keys: &[&str] = match tool_name {
        TOOL_READ_FILE | TOOL_WRITE_FILE | TOOL_EDIT_FILE => &["path"],
        _ => &[],
    };
    if path_keys.is_empty() {
        return;
    }
    let Some(map) = args.as_object_mut() else {
        return;
    };

    for key in path_keys {
        let Some(raw_path) = map.get(*key).and_then(|value| value.as_str()) else {
            continue;
        };
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = Path::new(trimmed);
        if parsed.is_absolute() {
            continue;
        }
        let joined = shell_cwd.join(parsed);
        map.insert(
            (*key).to_string(),
            Json::String(joined.to_string_lossy().to_string()),
        );
    }
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
    use crate::agent_memory::{AgentMemory, AgentMemoryConfig};
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

    fn documented_action_names() -> Vec<&'static str> {
        vec![
            TOOL_EXEC_BASH,
            TOOL_EDIT_FILE,
            TOOL_WRITE_FILE,
            TOOL_READ_FILE,
            TOOL_CREATE_SUB_AGENT,
            TOOL_GET_SESSION,
            TOOL_LIST_SESSION,
            TOOL_LIST_EXTERNAL_WORKSPACES,
            TOOL_BIND_EXTERNAL_WORKSPACE,
            TOOL_CREATE_LOCAL_WORKSPACE,
            TOOL_BIND_LOCAL_WORKSPACE,
            TOOL_LOAD_MEMORY,
            TOOL_TODO_MANAGE,
        ]
    }

    fn documented_tool_specs() -> Vec<ToolSpec> {
        let mut specs = documented_action_names()
            .iter()
            .map(|name| ToolSpec {
                name: (*name).to_string(),
                description: builtin_action_summary(name).to_string(),
                args_schema: builtin_action_args_schema(name),
                output_schema: json!({ "type": "object" }),
            })
            .collect::<Vec<_>>();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    fn documented_action_specs() -> Vec<ActionSpec> {
        let mut specs = documented_action_names()
            .iter()
            .map(|name| ActionSpec {
                kind: ActionKind::CallTool,
                name: (*name).to_string(),
                introduce: builtin_action_summary(name).to_string(),
                description: None,
            })
            .collect::<Vec<_>>();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    #[test]
    fn print_tool_and_action_prompt_catalog_for_review() {
        let tool_specs = documented_tool_specs();
        let action_specs = documented_action_specs();

        println!("\n================ TOOL PROMPTS ================");
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

        println!("\n[Prompt Payload] ToolSpec::render_for_prompt output");
        println!("{}", ToolSpec::render_for_prompt(&tool_specs));

        println!("\n================ ACTION PROMPTS ================");
        println!("[List Mode] name + introduce");
        for spec in &action_specs {
            println!("{}", spec.render_introduce_prompt());
        }

        println!("\n[Detail Mode] one action prompt per block");
        for spec in &action_specs {
            println!("\n### ACTION {}", spec.name);
            println!("{}", spec.render_prompt());
        }

        assert!(
            !tool_specs.is_empty(),
            "documented tool specs should not be empty"
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
    fn action_spec_render_prompt_falls_back_to_builtin_schema_description() {
        let spec = ActionSpec {
            kind: ActionKind::CallTool,
            name: TOOL_EXEC_BASH.to_string(),
            introduce: "run shell command".to_string(),
            description: None,
        };
        let rendered = spec.render_prompt();
        assert!(rendered.contains("Action Name: exec"));
        assert!(rendered.contains("Kind: call_tool"));
        assert!(rendered.contains("[\"exec\","));
        assert!(rendered.contains("Args schema"));
    }

    #[tokio::test]
    async fn load_memory_action_schema_matches_runtime_tool_spec() {
        let temp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(temp.path()))
            .await
            .expect("create agent memory");
        let mgr = AgentToolManager::new();
        memory.register_tools(&mgr).expect("register memory tools");

        let runtime_spec = mgr
            .get_tool_spec(TOOL_LOAD_MEMORY)
            .expect("load_memory tool should be registered");
        let action_schema = builtin_action_args_schema(TOOL_LOAD_MEMORY);
        assert_eq!(
            runtime_spec.args_schema, action_schema,
            "load_memory action schema must match runtime tool args schema"
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
            }
        }

        async fn call(
            &self,
            _ctx: &SessionRuntimeContext,
            _args: Json,
        ) -> Result<Json, AgentToolError> {
            Ok(json!({"ok": true}))
        }
    }

    struct EchoArgsTool {
        name: String,
        args_schema: Json,
    }

    #[async_trait]
    impl AgentTool for EchoArgsTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: "echo args".to_string(),
                args_schema: self.args_schema.clone(),
                output_schema: json!({"type":"object"}),
            }
        }

        async fn call(
            &self,
            _ctx: &SessionRuntimeContext,
            args: Json,
        ) -> Result<Json, AgentToolError> {
            Ok(json!({"ok": true, "args": args}))
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
        assert!(action_specs[0].introduce.contains("dummy"));

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
        })
        .expect("register tool");

        let result = mgr
            .call_tool_from_bash_line(&test_call_ctx(), "read_file ~/1.txt 0:200")
            .await
            .expect("bash style call should succeed")
            .expect("tool should be matched");

        assert_eq!(result["ok"], true);
        assert_eq!(result["args"]["path"], "~/1.txt");
        assert_eq!(result["args"]["range"], "0:200");
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

        assert_eq!(result["ok"], true);
        assert_eq!(result["args"]["path"], "~/1.txt");
        assert_eq!(result["args"]["range"], "0:200");
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

        assert_eq!(result["ok"], true);
        assert_eq!(result["args"]["path"], "/tmp/opendan-shell-cwd/1.txt");
        assert_eq!(result["args"]["range"], "1:1");
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

        assert_eq!(output["data"]["answer"], 42);

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
