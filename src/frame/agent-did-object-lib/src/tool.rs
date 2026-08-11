use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, CallingConventions,
    SessionRuntimeContext, ToolSpec,
};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::runtime::AgentDIDObjectRuntime;
use crate::types::{ReadInput, ReadLineRange};

pub const TOOL_READ: &str = "read";

#[derive(Clone)]
pub struct AgentDIDObjectReadTool {
    runtime: Arc<AgentDIDObjectRuntime>,
    file_base_dir: Option<PathBuf>,
}

impl AgentDIDObjectReadTool {
    pub fn new(runtime: AgentDIDObjectRuntime) -> Self {
        Self {
            runtime: Arc::new(runtime),
            file_base_dir: None,
        }
    }

    pub fn with_file_base_dir(mut self, file_base_dir: impl Into<PathBuf>) -> Self {
        self.file_base_dir = Some(file_base_dir.into());
        self
    }
}

#[async_trait]
impl AgentTool for AgentDIDObjectReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_READ.to_string(),
            description: "Read an agent DID object and return LLM-readable text.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "uri": {
                        "type": "string",
                        "description": "Object URL or file path to read."
                    },
                    "object": {
                        "type": "string",
                        "description": "Object URL or file path to read."
                    },
                    "purpose": {"type": "string"},
                    "content_only": {"type": "boolean"},
                    "offset": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "1-based line offset."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum line count."
                    },
                    "options": {"type": "object"}
                }
            }),
            output_schema: json!({ "type": "object" }),
            usage: Some("read uri=<object-or-path> [offset=<1-based-line>] [limit=<lines>] [content_only=true]".to_string()),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let input = self.input_from_args(ctx, args)?;
        self.runtime
            .read(input)
            .await
            .map(|result| result.with_tool_name())
            .map_err(object_error_to_agent_tool_error)
    }
}

impl AgentDIDObjectReadTool {
    fn input_from_args(
        &self,
        ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<ReadInput, AgentToolError> {
        let map = match args {
            Value::Object(map) => map,
            Value::Null => serde_json::Map::new(),
            other => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "read args must be a json object, got {other}"
                )))
            }
        };

        let raw_object = map
            .get("object")
            .or_else(|| map.get("uri"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("`uri` is required".to_string()))?;
        let object = canonical_object_url(raw_object, self.file_base_dir.as_deref())?;
        let offset = parse_usize_arg(map.get("offset"), "offset")?;
        let limit = parse_usize_arg(map.get("limit"), "limit")?;
        let content_only = map
            .get("content_only")
            .or_else(|| map.get("content-only"))
            .map(read_bool_arg)
            .transpose()?
            .unwrap_or(false);
        let purpose = map
            .get("purpose")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let options = map.get("options").cloned().unwrap_or_else(|| json!({}));

        Ok(ReadInput {
            object,
            purpose,
            session_id: Some(ctx.session_id.clone()),
            content_only,
            range: (offset.is_some() || limit.is_some()).then_some(ReadLineRange {
                offset: offset.unwrap_or(1),
                limit,
            }),
            max_tokens: Some(ctx.effective_read_token_limit() as usize),
            options,
        })
    }
}

trait ToolNameExt {
    fn with_tool_name(self) -> Self;
}

impl ToolNameExt for AgentToolResult {
    fn with_tool_name(mut self) -> Self {
        self.tool = Some(TOOL_READ.to_string());
        if self.status == AgentToolStatus::Error {
            self.return_code = Some(1);
        }
        self
    }
}

fn parse_usize_arg(value: Option<&Value>, name: &str) -> Result<Option<usize>, AgentToolError> {
    let Some(value) = value else { return Ok(None) };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(value) = value.as_u64() {
        return usize::try_from(value).map(Some).map_err(|_| {
            AgentToolError::InvalidArgs(format!("`{name}` value is too large: {value}"))
        });
    }
    if let Some(value) = value.as_str() {
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }
        return value.parse::<usize>().map(Some).map_err(|err| {
            AgentToolError::InvalidArgs(format!("`{name}` must be a non-negative integer: {err}"))
        });
    }
    Err(AgentToolError::InvalidArgs(format!(
        "`{name}` must be a non-negative integer"
    )))
}

fn read_bool_arg(value: &Value) -> Result<bool, AgentToolError> {
    if let Some(value) = value.as_bool() {
        return Ok(value);
    }
    if let Some(value) = value.as_str() {
        return match value.trim() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            other => Err(AgentToolError::InvalidArgs(format!(
                "`content_only` must be a bool, got `{other}`"
            ))),
        };
    }
    Err(AgentToolError::InvalidArgs(
        "`content_only` must be a bool".to_string(),
    ))
}

fn canonical_object_url(raw: &str, file_base_dir: Option<&Path>) -> Result<String, AgentToolError> {
    if raw.contains("://") {
        return Ok(raw.to_string());
    }
    let base = file_base_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let path = Path::new(raw);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    Ok(format!(
        "file://{}",
        percent_encode_file_path(&normalize_abs_path(&absolute))
    ))
}

fn normalize_abs_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = normalized.pop();
            }
            std::path::Component::Normal(seg) => normalized.push(seg),
        }
    }
    normalized
}

fn percent_encode_file_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut out = String::new();
    for byte in raw.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn object_error_to_agent_tool_error(err: crate::AgentDIDObjectError) -> AgentToolError {
    use crate::AgentDIDObjectError as ObjErr;
    match err {
        ObjErr::InvalidConfig(message)
        | ObjErr::RouteNotFound(message)
        | ObjErr::UnsupportedObjectRef(message)
        | ObjErr::UnsupportedMethod(message)
        | ObjErr::DeclaredCapabilityNotFound(message)
        | ObjErr::SchemaError(message) => AgentToolError::InvalidArgs(message),
        ObjErr::AdapterNotFound(message)
        | ObjErr::AdapterUnavailable(message)
        | ObjErr::ResolveError(message)
        | ObjErr::HttpError(message)
        | ObjErr::ProtocolError(message)
        | ObjErr::KEventError(message)
        | ObjErr::EventBridgeError(message)
        | ObjErr::AdapterError(message) => AgentToolError::ExecFailed(message),
    }
}

#[cfg(test)]
mod tests {
    use agent_tool::{AgentTool, SessionRuntimeContext, DEFAULT_READ_TOKEN_LIMIT};
    use serde_json::json;
    use tempfile::tempdir;

    use crate::{
        AdapterConfig, AdapterType, AgentDIDObjectRuntime, ObjectRoute, ObjectRouteConfig,
        RouteMatchType, RouteMethod,
    };

    use super::*;

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace".to_string(),
            agent_name: "agent".to_string(),
            behavior: "test".to_string(),
            step_idx: 1,
            wakeup_id: "wakeup".to_string(),
            session_id: "session".to_string(),
            read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    fn runtime(allowed_root: &Path) -> AgentDIDObjectRuntime {
        AgentDIDObjectRuntime::new(ObjectRouteConfig {
            version: 1,
            adapters: vec![AdapterConfig {
                id: "filesystem".to_string(),
                adapter_type: AdapterType::Filesystem,
                endpoint: None,
                auth_token_env: None,
                options: json!({
                    "allowed_read_roots": [allowed_root.display().to_string()]
                }),
            }],
            routes: vec![ObjectRoute {
                id: "file-read".to_string(),
                priority: 0,
                match_type: RouteMatchType::Scheme,
                pattern: "file".to_string(),
                adapter: "filesystem".to_string(),
                methods: vec![RouteMethod::Read],
                options: json!({}),
            }],
        })
        .unwrap()
    }

    #[tokio::test]
    async fn read_tool_resolves_bare_path_against_base_dir() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::write(workspace.join("demo.txt"), "line1\nline2\n")
            .await
            .unwrap();

        let tool = AgentDIDObjectReadTool::new(runtime(&workspace)).with_file_base_dir(&workspace);
        let result = tool
            .call(
                &ctx(),
                json!({
                    "uri": "demo.txt",
                    "content_only": true,
                    "offset": 2,
                    "limit": 1
                }),
            )
            .await
            .unwrap();

        assert_eq!(result.tool.as_deref(), Some(TOOL_READ));
        assert_eq!(result.cmd_name.as_deref(), Some(TOOL_READ));
        assert_eq!(result.output.as_deref(), Some("line2"));
    }
}
