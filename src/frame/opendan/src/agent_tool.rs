use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::time::{timeout, Duration};

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: Json,
    pub call_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolCallContext {
    pub trace_id: String,
    pub agent_did: String,
    pub behavior: String,
    pub step_idx: u32,
    pub wakeup_id: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ToolError {
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
    async fn call(&self, ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError>;
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
    pub fn new(cfg: MCPToolConfig) -> Result<Self, ToolError> {
        let tool_name = cfg.name.trim();
        if tool_name.is_empty() {
            return Err(ToolError::InvalidArgs(
                "mcp tool `name` cannot be empty".to_string(),
            ));
        }

        let endpoint = cfg.endpoint.trim();
        if endpoint.is_empty() {
            return Err(ToolError::InvalidArgs(
                "mcp tool `endpoint` cannot be empty".to_string(),
            ));
        }

        if cfg.timeout_ms == 0 {
            return Err(ToolError::InvalidArgs(
                "mcp tool `timeout_ms` must be > 0".to_string(),
            ));
        }

        let mcp_tool_name = cfg
            .mcp_tool_name
            .unwrap_or_else(|| tool_name.to_string())
            .trim()
            .to_string();
        if mcp_tool_name.is_empty() {
            return Err(ToolError::InvalidArgs(
                "mcp tool `mcp_tool_name` cannot be empty".to_string(),
            ));
        }

        let description = cfg
            .description
            .unwrap_or_else(|| format!("MCP tool `{}`", mcp_tool_name));

        let client = reqwest::Client::builder()
            .build()
            .map_err(|err| ToolError::ExecFailed(format!("build mcp http client failed: {err}")))?;

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

    async fn call(&self, ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
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
            .map_err(|_| ToolError::Timeout)?
            .map_err(|err| ToolError::ExecFailed(format!("mcp request failed: {err}")))?;

        let status = response.status();
        let body = timeout(Duration::from_millis(self.timeout_ms), response.text())
            .await
            .map_err(|_| ToolError::Timeout)?
            .map_err(|err| ToolError::ExecFailed(format!("read mcp response failed: {err}")))?;

        if !status.is_success() {
            return Err(ToolError::ExecFailed(format!(
                "mcp server returned http {}: {}",
                status.as_u16(),
                truncate_text(&body, 512)
            )));
        }

        let payload: Json = serde_json::from_str(&body)
            .map_err(|err| ToolError::ExecFailed(format!("invalid mcp response json: {err}")))?;

        if let Some(err_obj) = payload.get("error") {
            let msg = extract_jsonrpc_error_message(err_obj);
            return Err(ToolError::ExecFailed(format!("mcp tool call error: {msg}")));
        }

        let result = payload.get("result").cloned().ok_or_else(|| {
            ToolError::ExecFailed("mcp response missing `result` field".to_string())
        })?;

        if let Some(message) = extract_mcp_result_error(&result) {
            return Err(ToolError::ExecFailed(format!(
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

    async fn call(&self, ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        self.inner.call(ctx, args).await
    }
}

pub struct ToolManager {
    tools: RwLock<HashMap<String, Arc<dyn AgentTool>>>,
}

impl Default for ToolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolManager {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
        }
    }

    pub fn register_tool<T>(&self, tool: T) -> Result<(), ToolError>
    where
        T: AgentTool + 'static,
    {
        self.register_tool_arc(Arc::new(tool))
    }

    pub fn register_tool_arc(&self, tool: Arc<dyn AgentTool>) -> Result<(), ToolError> {
        let mut spec = tool.spec();
        let original_name = spec.name.trim().to_string();
        if original_name.is_empty() {
            return Err(ToolError::InvalidArgs(
                "tool name cannot be empty".to_string(),
            ));
        }
        let normalized_name = normalize_tool_name(original_name.as_str());
        if normalized_name.is_empty() {
            return Err(ToolError::InvalidArgs(format!(
                "tool name `{}` is invalid after normalization",
                original_name
            )));
        }
        spec.name = normalized_name.clone();
        let registered: Arc<dyn AgentTool> = Arc::new(RegisteredTool { spec, inner: tool });

        let mut guard = self
            .tools
            .write()
            .map_err(|_| ToolError::ExecFailed("tool registry lock poisoned".to_string()))?;
        if guard.contains_key(&normalized_name) {
            return Err(ToolError::AlreadyExists(normalized_name));
        }
        guard.insert(normalized_name.clone(), registered);
        if normalized_name != original_name {
            warn!(
                "tool name normalized for provider compatibility: original={} normalized={}",
                original_name, normalized_name
            );
        }
        Ok(())
    }

    pub fn register_mcp_tool(&self, cfg: MCPToolConfig) -> Result<(), ToolError> {
        self.register_tool(MCPTool::new(cfg)?)
    }

    pub fn unregister_tool(&self, name: &str) -> bool {
        let Ok(mut guard) = self.tools.write() else {
            return false;
        };
        guard.remove(name).is_some()
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

    pub async fn call_tool(
        &self,
        ctx: &ToolCallContext,
        call: ToolCall,
    ) -> Result<Json, ToolError> {
        let Some(tool) = self.get_tool(&call.name) else {
            return Err(ToolError::NotFound(call.name));
        };
        tool.call(ctx, call.args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn test_call_ctx() -> ToolCallContext {
        ToolCallContext {
            trace_id: "trace-1".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-1".to_string(),
        }
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

        async fn call(&self, _ctx: &ToolCallContext, _args: Json) -> Result<Json, ToolError> {
            Ok(json!({"ok": true}))
        }
    }

    #[tokio::test]
    async fn register_tool_normalizes_module_prefixed_name_without_alias() {
        let mgr = ToolManager::new();
        mgr.register_tool(DummyTool {
            name: "workshop.exec_bash".to_string(),
        })
        .expect("register tool");

        assert!(mgr.has_tool("exec_bash"));
        assert!(!mgr.has_tool("workshop.exec_bash"));

        let specs = mgr.list_tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "exec_bash");

        let err = mgr
            .call_tool(
                &test_call_ctx(),
                ToolCall {
                    name: "workshop.exec_bash".to_string(),
                    args: json!({}),
                    call_id: "call-1".to_string(),
                },
            )
            .await
            .expect_err("legacy alias should not call");
        assert!(matches!(err, ToolError::NotFound(_)));

        mgr.call_tool(
            &test_call_ctx(),
            ToolCall {
                name: "exec_bash".to_string(),
                args: json!({}),
                call_id: "call-2".to_string(),
            },
        )
        .await
        .expect("normalized name should call");
    }

    #[test]
    fn unregister_tool_by_normalized_name() {
        let mgr = ToolManager::new();
        mgr.register_tool(DummyTool {
            name: "workshop.exec_bash".to_string(),
        })
        .expect("register tool");

        assert!(mgr.unregister_tool("exec_bash"));
        assert!(!mgr.has_tool("exec_bash"));
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

        assert!(matches!(err, ToolError::ExecFailed(_)));
        assert!(err.to_string().contains("boom"));
    }
}
