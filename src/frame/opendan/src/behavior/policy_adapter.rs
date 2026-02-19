use async_trait::async_trait;

use crate::agent_tool::{ToolCall, ToolSpec};

use super::types::BehaviorExecInput;

#[async_trait]
pub trait PolicyEngine: Send + Sync {
    async fn allowed_tools(&self, input: &BehaviorExecInput) -> Result<Vec<ToolSpec>, String>;

    async fn gate_tool_calls(
        &self,
        input: &BehaviorExecInput,
        calls: &[ToolCall],
    ) -> Result<Vec<ToolCall>, String>;
}
