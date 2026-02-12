use async_trait::async_trait;

use crate::agent_tool::{ToolCall, ToolSpec};

use super::types::ProcessInput;

#[async_trait]
pub trait PolicyEngine: Send + Sync {
    async fn allowed_tools(&self, input: &ProcessInput) -> Result<Vec<ToolSpec>, String>;

    async fn gate_tool_calls(
        &self,
        input: &ProcessInput,
        calls: &[ToolCall],
    ) -> Result<Vec<ToolCall>, String>;
}
