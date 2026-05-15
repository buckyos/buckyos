//! §8 of NewOpenDANRuntime — UI-session-only worksession control tools.
//!
//! Two LLM-callable tools live here:
//!   - [`CreateWorksessionTool`] (`create_worksession`) — fully-parameterized
//!     work-session creation. Per §8.1 this is normally only advertised
//!     inside the `try_create_worksession` fork sub-context; we register
//!     it on every session for now because the fork-mode plumbing isn't
//!     wired yet. Behavior whitelists keep it out of UI session prompts.
//!   - [`ForwardMsgTool`] (`forward_msg`) — process-internal route that
//!     pushes the *most recent* user message into a target worksession's
//!     pending queue. Per §8.4 the worker should stash the originating
//!     message for the tool to pick up automatically, but until that
//!     plumbing exists the tool takes the text explicitly so the surface
//!     is usable today.
//!
//! Both tools hold a `Weak<AIAgent>` so they can call agent-level methods
//! without forming an Arc cycle (AIAgent → sessions → tool manager →
//! tools → AIAgent would otherwise pin the agent forever).

use std::sync::Weak;

use agent_tool::{
    AgentToolError, AgentToolManager, CallingConventions, ToolCtx, TypedTool,
};
use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole};
use llm_context::{
    outcome::ContextOutput,
    request::{ToolMode, ToolPolicy},
};
use log::warn;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::{AIAgent, CreateWorkSessionParams};
use crate::llm_context_helper::RequestOverrides;

/// Tool name advertised to the LLM. Behaviors that want to expose this
/// add the string to their `tool_whitelist`.
pub const TOOL_CREATE_WORKSESSION: &str = "create_worksession";
/// Tool name advertised to the LLM for cross-session forwarding.
pub const TOOL_FORWARD_MSG: &str = "forward_msg";
/// Tool name advertised to UI sessions for fork-based worksession decisions.
/// The tool runs a fork sub-context that internally calls `create_worksession`.
pub const TOOL_TRY_CREATE_WORKSESSION: &str = "try_create_worksession";

/// `create_worksession` tool arguments. Mirrors §8.1.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateWorksessionArgs {
    /// Short label for the new work session (≤ 80 chars; informational).
    pub title: String,
    /// Goal / task statement. Surfaced into the system prompt of the new
    /// session. Required — a worksession without an objective wouldn't
    /// know what to do.
    pub objective: String,
    /// Reuse an existing workspace by id. Empty / absent ⇒ mint a fresh
    /// workspace bound to the new session.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Override the behavior the worksession starts on. Empty / absent
    /// uses the agent's `default_work_behavior`.
    #[serde(default)]
    pub behavior: Option<String>,
    /// Verbatim user messages that prompted creation. Recorded into the
    /// new session's `readme.md` for audit / debugging.
    #[serde(default)]
    pub reason_message: Vec<String>,
}

/// Tool output — same shape returned to the calling LLM as JSON.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreateWorksessionOutput {
    pub session_id: String,
    pub title: String,
    pub workspace_id: String,
    /// `"created"` or `"reused"`.
    pub workspace_status: String,
    pub behavior: String,
    /// Always `"created"` on the happy path — signals to the parent LLM
    /// that the session is now live (its worker has started).
    pub status: String,
}

pub struct CreateWorksessionTool {
    agent: Weak<AIAgent>,
    source_session_id: String,
}

impl CreateWorksessionTool {
    pub fn new(agent: Weak<AIAgent>, source_session_id: impl Into<String>) -> Self {
        Self {
            agent,
            source_session_id: source_session_id.into(),
        }
    }
}

#[async_trait]
impl TypedTool for CreateWorksessionTool {
    type Args = CreateWorksessionArgs;
    type Output = CreateWorksessionOutput;

    fn name(&self) -> &str {
        TOOL_CREATE_WORKSESSION
    }

    fn description(&self) -> &str {
        "Create a new work session bound to a workspace and start its worker. Returns the new session id."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let agent = self
            .agent
            .upgrade()
            .ok_or_else(|| AgentToolError::ExecFailed("agent is shutting down".to_string()))?;
        let outcome = agent
            .create_work_session(CreateWorkSessionParams {
                title: args.title,
                objective: args.objective,
                workspace_id: args.workspace_id,
                behavior: args.behavior,
                created_by_session_id: self.source_session_id.clone(),
                reason_messages: args.reason_message,
            })
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        Ok(CreateWorksessionOutput {
            session_id: outcome.session_id,
            title: outcome.title,
            workspace_id: outcome.workspace_id,
            workspace_status: outcome.workspace_status,
            behavior: outcome.behavior,
            status: "created".to_string(),
        })
    }
}

/// `forward_msg` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ForwardMsgArgs {
    /// Target work-session id. Must exist, be a Work session (not UI),
    /// and not yet have Ended.
    pub target_worksession_id: String,
    /// The text to forward. Per §8.4 the runtime is supposed to attach
    /// the "current user message" automatically; until that plumbing
    /// exists the LLM passes the body explicitly.
    pub message: String,
}

/// Tool output. Always reflects what was actually enqueued so the LLM
/// can include the synthetic record id in its reply / next turn.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ForwardMsgOutput {
    pub forwarded: bool,
    pub target_session_id: String,
    pub record_id: String,
}

pub struct ForwardMsgTool {
    agent: Weak<AIAgent>,
    source_session_id: String,
}

impl ForwardMsgTool {
    pub fn new(agent: Weak<AIAgent>, source_session_id: impl Into<String>) -> Self {
        Self {
            agent,
            source_session_id: source_session_id.into(),
        }
    }
}

#[async_trait]
impl TypedTool for ForwardMsgTool {
    type Args = ForwardMsgArgs;
    type Output = ForwardMsgOutput;

    fn name(&self) -> &str {
        TOOL_FORWARD_MSG
    }

    fn description(&self) -> &str {
        "Forward a message to another worksession's pending input queue."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let agent = self
            .agent
            .upgrade()
            .ok_or_else(|| AgentToolError::ExecFailed("agent is shutting down".to_string()))?;
        let record_id = agent
            .forward_message(
                &args.target_worksession_id,
                &self.source_session_id,
                &args.message,
            )
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        Ok(ForwardMsgOutput {
            forwarded: true,
            target_session_id: args.target_worksession_id,
            record_id,
        })
    }
}

/// `try_create_worksession` arguments. Per §8.2 the only LLM-supplied
/// input is a free-text `reason`; the fork sub-context derives everything
/// else (title / objective / workspace_id) by inspecting the parent
/// session's inherited history.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TryCreateWorksessionArgs {
    /// Free-text justification — why does the parent think a worksession
    /// should be created? The fork sub-context sees this verbatim plus
    /// the parent's accumulated chat history.
    pub reason: String,
}

/// `try_create_worksession` output. The sub-context's terminal
/// [`ContextOutput`] is surfaced to the parent LLM as JSON:
/// - `ContextOutput::Json` ⇒ value passed through verbatim (typically
///   the result of the sub-ctx's `create_worksession` tool call)
/// - `ContextOutput::Text` ⇒ wrapped as `{ "decision_text": <body> }`
///   for the rare case the sub-ctx terminates without calling
///   `create_worksession` (the parent LLM can read the rationale)
pub struct TryCreateWorksessionTool {
    agent: Weak<AIAgent>,
    source_session_id: String,
}

impl TryCreateWorksessionTool {
    pub fn new(agent: Weak<AIAgent>, source_session_id: impl Into<String>) -> Self {
        Self {
            agent,
            source_session_id: source_session_id.into(),
        }
    }
}

#[async_trait]
impl TypedTool for TryCreateWorksessionTool {
    type Args = TryCreateWorksessionArgs;
    type Output = serde_json::Value;

    fn name(&self) -> &str {
        TOOL_TRY_CREATE_WORKSESSION
    }

    fn description(&self) -> &str {
        "Decide whether the current request warrants a new worksession; if so, create it. Runs a short fork sub-context that may call `create_worksession`."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
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
        // Drive the sub-ctx's deps from the parent's current behavior so
        // parser/renderer / approval list / one_line_status sink stay
        // consistent. Only the request side is overridden.
        let parent_behavior = session.meta.lock().await.current_behavior.clone();

        // Sub-context system prompt: minimal directive + the reason.
        // A follow-up will inject the parent's recent chat history and
        // the existing-worksession list (§8.2 design); the fork-mode
        // plumbing itself is what Phase 4 verifies.
        let sub_system = vec![AiMessage::text(
            AiRole::System,
            format!(
                "You are a short-lived fork sub-context. Your only job: decide \
                 whether the parent session's situation needs a new worksession, \
                 and if so create it by calling the `create_worksession` tool. \
                 \n\nReason supplied by the parent:\n{}\n\n\
                 If you create one, end with `<next_behavior>END</next_behavior>` \
                 after the call. If you decide not to, explain why and end with \
                 `<next_behavior>END</next_behavior>`. Do not call \
                 `try_create_worksession` recursively.",
                args.reason
            ),
        )];

        // Sub tool whitelist: only the actual landing tool. The parent's
        // session-aware tools (try_create_worksession, forward_msg) are
        // explicitly excluded so the sub-ctx can't recurse.
        let sub_tool_policy = ToolPolicy {
            mode: ToolMode::Whitelist,
            whitelist: vec![TOOL_CREATE_WORKSESSION.to_string()],
            max_rounds: 8,
            ..ToolPolicy::default()
        };

        let overrides = RequestOverrides {
            system_messages: Some(sub_system),
            tool_policy: Some(sub_tool_policy),
            objective: Some(format!("Decide+create worksession for: {}", args.reason)),
            // Let fork_and_run rewrite trace to `<parent>::fork-<n>`.
            trace: Some(None),
            reset_rounds: true,
            reset_errors: true,
            forbid_next_behavior: false, // TODO: wire when waist supports it
            ..Default::default()
        };

        let output = session
            .fork_and_run(overrides, &parent_behavior)
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("fork failed: {err:#}")))?;
        Ok(match output {
            ContextOutput::Json { content } => content,
            ContextOutput::Text { content } => serde_json::json!({
                "decision_text": content,
            }),
        })
    }
}

/// Register all three worksession-control tools on `manager`. Idempotent —
/// re-registering on an already-populated manager replaces the prior
/// instances (the manager's `register_typed_tool` handles dedup).
pub fn register_worksession_tools(
    manager: &AgentToolManager,
    agent: Weak<AIAgent>,
    source_session_id: &str,
) {
    if let Err(err) = manager.register_typed_tool(CreateWorksessionTool::new(
        agent.clone(),
        source_session_id,
    )) {
        warn!(
            "opendan.worksession_tools: register `{TOOL_CREATE_WORKSESSION}` failed: {err}"
        );
    }
    if let Err(err) = manager.register_typed_tool(ForwardMsgTool::new(
        agent.clone(),
        source_session_id,
    )) {
        warn!(
            "opendan.worksession_tools: register `{TOOL_FORWARD_MSG}` failed: {err}"
        );
    }
    if let Err(err) = manager.register_typed_tool(TryCreateWorksessionTool::new(
        agent,
        source_session_id,
    )) {
        warn!(
            "opendan.worksession_tools: register `{TOOL_TRY_CREATE_WORKSESSION}` failed: {err}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tool names are advertised through behavior whitelists — if these
    // strings change without a coordinated update, behavior.toml files
    // silently stop activating the tools.
    #[test]
    fn tool_names_are_stable() {
        assert_eq!(TOOL_CREATE_WORKSESSION, "create_worksession");
        assert_eq!(TOOL_FORWARD_MSG, "forward_msg");
        assert_eq!(TOOL_TRY_CREATE_WORKSESSION, "try_create_worksession");
    }
}
