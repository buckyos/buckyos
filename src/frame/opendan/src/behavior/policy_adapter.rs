use async_trait::async_trait;
use buckyos_api::AiToolCall;

use crate::agent_tool::ToolSpec;

use super::types::BehaviorExecInput;

#[async_trait]
pub trait PolicyEngine: Send + Sync {
    async fn allowed_tools(&self, input: &BehaviorExecInput) -> Result<Vec<ToolSpec>, String>;

    async fn gate_tool_calls(
        &self,
        input: &BehaviorExecInput,
        calls: &[AiToolCall],
    ) -> Result<Vec<AiToolCall>, String>;
}
