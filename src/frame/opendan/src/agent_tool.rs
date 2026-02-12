use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

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

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError>;
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
        let spec = tool.spec();
        let mut guard = self
            .tools
            .write()
            .map_err(|_| ToolError::ExecFailed("tool registry lock poisoned".to_string()))?;
        if guard.contains_key(&spec.name) {
            return Err(ToolError::AlreadyExists(spec.name));
        }
        guard.insert(spec.name, tool);
        Ok(())
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
