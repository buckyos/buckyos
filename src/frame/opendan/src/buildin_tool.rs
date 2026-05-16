use std::sync::Weak;

use agent_tool::{AgentToolError, AgentToolManager, CallingConventions, ToolCtx, TypedTool};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::AIAgent;

pub const TOOL_SUBSCRIBE_EVENT: &str = "subscribe_event";
pub const TOOL_UNSUBSCRIBE_EVENT: &str = "unsubscribe_event";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SubscribeEventArgs {
    /// KEvent path pattern, for example `/task_mgr/42` or `/approval/**`.
    pub pattern: String,
    /// Optional natural-language rendering used when a matching event wakes
    /// the session. Supports `{event_id}`, `{data}`, and top-level JSON
    /// fields such as `{status}` or `{message}`.
    #[serde(default)]
    pub message_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SubscribeEventOutput {
    pub subscribed: bool,
    pub pattern: String,
}

pub struct SubscribeEventTool {
    agent: Weak<AIAgent>,
    source_session_id: String,
}

impl SubscribeEventTool {
    pub fn new(agent: Weak<AIAgent>, source_session_id: impl Into<String>) -> Self {
        Self {
            agent,
            source_session_id: source_session_id.into(),
        }
    }
}

#[async_trait]
impl TypedTool for SubscribeEventTool {
    type Args = SubscribeEventArgs;
    type Output = SubscribeEventOutput;

    fn name(&self) -> &str {
        TOOL_SUBSCRIBE_EVENT
    }

    fn description(&self) -> &str {
        "Subscribe this Agent Session to a KEvent path pattern. Matching events are batched and delivered as natural-language user wakeup messages."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let pattern = args.pattern.trim();
        if pattern.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "`pattern` must not be empty".to_string(),
            ));
        }
        let agent = self
            .agent
            .upgrade()
            .ok_or_else(|| AgentToolError::ExecFailed("agent is shutting down".to_string()))?;
        let session = agent
            .get_session(&self.source_session_id)
            .await
            .ok_or_else(|| {
                AgentToolError::ExecFailed(format!(
                    "session `{}` not mounted",
                    self.source_session_id
                ))
            })?;
        let subscribed = session
            .subscribe_event_with_template(pattern.to_string(), args.message_template)
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        Ok(SubscribeEventOutput {
            subscribed,
            pattern: pattern.to_string(),
        })
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UnsubscribeEventArgs {
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UnsubscribeEventOutput {
    pub unsubscribed: bool,
    pub pattern: String,
}

pub struct UnsubscribeEventTool {
    agent: Weak<AIAgent>,
    source_session_id: String,
}

impl UnsubscribeEventTool {
    pub fn new(agent: Weak<AIAgent>, source_session_id: impl Into<String>) -> Self {
        Self {
            agent,
            source_session_id: source_session_id.into(),
        }
    }
}

#[async_trait]
impl TypedTool for UnsubscribeEventTool {
    type Args = UnsubscribeEventArgs;
    type Output = UnsubscribeEventOutput;

    fn name(&self) -> &str {
        TOOL_UNSUBSCRIBE_EVENT
    }

    fn description(&self) -> &str {
        "Remove a KEvent subscription from this Agent Session."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let pattern = args.pattern.trim();
        if pattern.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "`pattern` must not be empty".to_string(),
            ));
        }
        let agent = self
            .agent
            .upgrade()
            .ok_or_else(|| AgentToolError::ExecFailed("agent is shutting down".to_string()))?;
        let session = agent
            .get_session(&self.source_session_id)
            .await
            .ok_or_else(|| {
                AgentToolError::ExecFailed(format!(
                    "session `{}` not mounted",
                    self.source_session_id
                ))
            })?;
        let unsubscribed = session
            .unsubscribe_event(pattern)
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        Ok(UnsubscribeEventOutput {
            unsubscribed,
            pattern: pattern.to_string(),
        })
    }
}

pub fn register_event_subscription_tools(
    manager: &AgentToolManager,
    agent: Weak<AIAgent>,
    source_session_id: &str,
) {
    let _ = manager.register_typed_tool(SubscribeEventTool::new(
        agent.clone(),
        source_session_id.to_string(),
    ));
    let _ = manager.register_typed_tool(UnsubscribeEventTool::new(
        agent,
        source_session_id.to_string(),
    ));
}
