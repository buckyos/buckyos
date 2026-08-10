//! §8 of NewOpenDANRuntime — UI-session-only worksession control tools.
//!
//! LLM-callable non-CLI session tools live here:
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
//!   - [`TryCreateWorksessionTool`] (`try_create_worksession`) — fork-based
//!     UI-session decision helper for creating or reusing worksessions.
//!   - [`UpdateSessionTopicTool`] (`update_session_topic`) — session topic
//!     and tag-set writer that also synchronously drives recall.
//!
//! These tools hold a `Weak<AIAgent>` so they can call agent-level methods
//! without forming an Arc cycle (AIAgent → sessions → tool manager →
//! tools → AIAgent would otherwise pin the agent forever).

use std::collections::HashSet;
use std::sync::Weak;

use agent_tool::{
    AgentToolError, AgentToolManager, CallingConventions, ToolCtx, TypedTool,
    TOOL_CREATE_WORKSPACE, TOOL_EXEC_BASH, TOOL_READ,
};
use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiRole};
use chrono::Utc;
use llm_context::{
    outcome::ContextOutput,
    request::{OutputSpec, ToolMode, ToolPolicy},
};
use log::warn;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::{AIAgent, CreateWorkSessionParams};
use crate::llm_context_helper::RequestOverrides;
use crate::local_workspace::{WorkspaceRecord, WorkspaceStatus};
use crate::round_history::HistoryEvent;
use crate::session_model::{SessionKind, SessionStatus, SessionSummary};
use crate::session_topic::{
    apply_recall_policy_override, RecallPolicy, RecallPolicyOverride, SessionTopicError,
    SessionTopicUpdater, TagInput, UpdateSessionTopicInput, UpdateSessionTopicResult,
};

/// Cap on the number of existing worksessions surfaced in the sub-prompt.
/// Per §8.2 of NewOpenDANRuntime.md; keeps the sub-LLM context small.
const MAX_WORKSESSION_LIST: usize = 64;
/// Cap on the number of parent chat-history entries injected into the
/// sub-prompt. Filters to user/assistant text only (system / tool-result
/// roles are stripped).
const MAX_FORWARDED_HISTORY: usize = 32;
/// Cap on per-message text rendered into the parent-history snippet. Above
/// this we truncate with an ellipsis so a single oversized message can't
/// blow the sub-context budget.
const HISTORY_CHARS_PER_MESSAGE: usize = 480;
/// Cap on workspace list entries in the sub-prompt. The list is sorted by
/// `updated_at_ms` desc so the freshest workspaces win the slots.
const MAX_WORKSPACE_LIST: usize = 32;

/// Tool name advertised to the LLM. Behaviors that want to expose this
/// add the string to their `tool_whitelist`.
pub const TOOL_CREATE_WORKSESSION: &str = "create_worksession";
/// Tool name advertised to the LLM for cross-session forwarding.
pub const TOOL_FORWARD_MSG: &str = "forward_msg";
/// Tool name advertised to UI sessions for fork-based worksession decisions.
/// The tool runs a fork sub-context that internally calls `create_worksession`.
pub const TOOL_TRY_CREATE_WORKSESSION: &str = "try_create_worksession";
/// Tool name advertised to sessions so the LLM can persist the current
/// session's topic hint for later recall.
pub const TOOL_UPDATE_SESSION_TOPIC: &str = "update_session_topic";

/// `create_worksession` tool arguments. Mirrors §8.1.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateWorksessionArgs {
    /// Short label for the new work session (≤ 80 chars; informational).
    #[serde(default)]
    pub title: String,
    /// Goal / task statement. Surfaced into the system prompt of the new
    /// session. Required unless `task_id` points at a Task whose data can
    /// supply the objective.
    #[serde(default)]
    pub objective: String,
    /// Existing TaskManager task to bind. When set, title/objective/workspace
    /// may be derived from the task data.
    #[serde(default)]
    #[schemars(length(min = 1))]
    pub task_id: Option<String>,
    /// Reuse an existing workspace by id. Empty / absent ⇒ mint a fresh
    /// workspace bound to the new session.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Override the behavior the worksession starts on. Empty / absent
    /// uses the work session class's `default_behavior` from `agent.toml`.
    #[serde(default)]
    pub behavior: Option<String>,
    /// Verbatim user messages that prompted creation. Recorded into the
    /// new session's `readme.md` for audit / debugging.
    #[serde(default)]
    pub reason_message: Vec<String>,
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
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
    pub worker_status: String,
    pub auto_started: bool,
    pub task_id: Option<String>,
    pub followup_routing: WorksessionFollowupRouting,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WorksessionFollowupRouting {
    pub tool: String,
    pub target_worksession_id: String,
    pub instruction: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateWorkspaceArgs {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreateWorkspaceOutput {
    pub workspace_id: String,
    pub name: String,
    pub status: String,
}

pub struct CreateWorkspaceTool {
    agent: Weak<AIAgent>,
}

impl CreateWorkspaceTool {
    pub fn new(agent: Weak<AIAgent>) -> Self {
        Self { agent }
    }
}

#[async_trait]
impl TypedTool for CreateWorkspaceTool {
    type Args = CreateWorkspaceArgs;
    type Output = CreateWorkspaceOutput;

    fn name(&self) -> &str {
        TOOL_CREATE_WORKSPACE
    }

    fn description(&self) -> &str {
        "Create an unbound workspace that can be passed to create_worksession.workspace_id."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!("created workspace {}", output.workspace_id)
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        Some(format!(
            "create_workspace {} => created",
            output.workspace_id
        ))
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let name = args.name.trim();
        if name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace name cannot be empty".to_string(),
            ));
        }
        let summary = args.summary.trim();
        if summary.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace summary cannot be empty".to_string(),
            ));
        }
        let agent = self
            .agent
            .upgrade()
            .ok_or_else(|| AgentToolError::ExecFailed("agent is shutting down".to_string()))?;
        let workspace_id = agent.allocate_workspace_id(name).await;
        let record = agent
            .workspaces()
            .create_or_open(&workspace_id, name, None)
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("create workspace: {err}")))?;
        let summary_path = agent
            .workspaces()
            .workspace_dir(&record.workspace_id)
            .join("SUMMARY.md");
        tokio::fs::write(&summary_path, format!("{summary}\n"))
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("write workspace summary: {err}")))?;
        Ok(CreateWorkspaceOutput {
            workspace_id: record.workspace_id,
            name: record.name,
            status: "created".to_string(),
        })
    }
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
        "Create a new work session bound to a workspace. Set auto_start=false to create it without running the first turn."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let mut parts = vec![format!("create_worksession title={}", args.title.trim())];
        if let Some(workspace_id) = args
            .workspace_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("workspace_id={workspace_id}"));
        }
        if let Some(behavior) = args
            .behavior
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("behavior={behavior}"));
        }
        if !args.auto_start {
            parts.push("auto_start=false".to_string());
        }
        Some(parts.join(" "))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        if output.auto_started {
            format!(
                "created and started worksession {} titled `{}` on workspace {} ({}) with behavior {}. Future user follow-up for this task must be forwarded automatically with forward_msg target_worksession_id={}",
                output.session_id,
                output.title,
                output.workspace_id,
                output.workspace_status,
                output.behavior,
                output.session_id
            )
        } else {
            format!(
                "created idle worksession {} titled `{}` on workspace {} ({}) with behavior {}. Start or forward follow-up only when this task should run.",
                output.session_id,
                output.title,
                output.workspace_id,
                output.workspace_status,
                output.behavior
            )
        }
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        Some(format!(
            "create_worksession {} => created",
            output.session_id
        ))
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
                task_binding: None,
                task_id: args.task_id,
                auto_start: args.auto_start,
                bind_task: true,
            })
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        let followup_routing =
            worksession_followup_routing(&outcome.session_id, outcome.auto_started);
        let worker_status = if outcome.auto_started {
            "started"
        } else {
            "idle"
        };
        Ok(CreateWorksessionOutput {
            session_id: outcome.session_id,
            title: outcome.title,
            workspace_id: outcome.workspace_id,
            workspace_status: outcome.workspace_status,
            behavior: outcome.behavior,
            status: "created".to_string(),
            worker_status: worker_status.to_string(),
            auto_started: outcome.auto_started,
            task_id: outcome.task_id,
            followup_routing,
        })
    }
}

fn default_auto_start() -> bool {
    true
}

/// `forward_msg` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ForwardMsgArgs {
    /// Target work-session id.
    pub target_worksession_id: String,
    /// Override the forwarded text. **Usually omit this.**
    #[serde(default)]
    pub message: Option<String>,
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
        "Forward current user message to another worksession"
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        Some(format!("forward_msg {}", args.target_worksession_id.trim()))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        if output.forwarded {
            format!(
                "forwarded current message to worksession {} as record {}",
                output.target_session_id, output.record_id
            )
        } else {
            format!("message not forwarded to {}", output.target_session_id)
        }
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        Some(format!(
            "forward_msg {} => {}",
            output.target_session_id,
            if output.forwarded { "sent" } else { "skipped" }
        ))
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
        // Auto-capture path (preferred): pull the origin user message the
        // worker stashed before running this turn. Caller can override by
        // passing `message` explicitly, but that's reserved for the rare
        // "forward a paraphrase" case — see ForwardMsgArgs doc.
        let body = match args
            .message
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(s) => s.to_string(),
            None => {
                let session = agent
                    .get_session(&self.source_session_id)
                    .await
                    .ok_or_else(|| {
                        AgentToolError::ExecFailed(format!(
                            "session `{}` not mounted; cannot auto-capture origin message",
                            self.source_session_id
                        ))
                    })?;
                session.current_origin_user_message().ok_or_else(|| {
                    AgentToolError::ExecFailed(
                        "forward_msg: no `message` arg and no origin user message to forward — \
                         the current turn appears to have been driven by an event / tool result, \
                         not a user message. Pass `message` explicitly if needed."
                            .to_string(),
                    )
                })?
            }
        };
        let record_id = agent
            .forward_message(&args.target_worksession_id, &self.source_session_id, &body)
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        Ok(ForwardMsgOutput {
            forwarded: true,
            target_session_id: args.target_worksession_id,
            record_id,
        })
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdateSessionTopicArgs {
    /// One-line Topic Title for the current session. Write for the future self,
    /// not for the user; this is not a session summary. The old `topic`
    /// parameter name is accepted as a compatibility alias.
    #[serde(alias = "topic")]
    pub title: String,
    /// Optional short tags used as coarse recall keys. Each reason should be a
    /// compact explanation of why the tag matters now.
    #[serde(default)]
    pub tags: Vec<TagInput>,
    /// Optional per-call recall policy override. This only affects this call's
    /// synchronous recall and does not change topic/tag storage semantics.
    #[serde(default)]
    pub recall_policy_override: Option<RecallPolicyOverride>,
}

pub struct UpdateSessionTopicTool {
    agent: Weak<AIAgent>,
    source_session_id: String,
}

impl UpdateSessionTopicTool {
    pub fn new(agent: Weak<AIAgent>, source_session_id: impl Into<String>) -> Self {
        Self {
            agent,
            source_session_id: source_session_id.into(),
        }
    }
}

#[async_trait]
impl TypedTool for UpdateSessionTopicTool {
    type Args = UpdateSessionTopicArgs;
    type Output = UpdateSessionTopicResult;

    fn name(&self) -> &str {
        TOOL_UPDATE_SESSION_TOPIC
    }

    fn description(&self) -> &str {
        "Update this session's one-line Topic Title and short topic tags. Call only when the topic first becomes clear, significantly drifts, or reaches a final form. Each tag must include a compact reason explaining why it matters now. Write for your future self; do not use this for detailed summaries."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let mut parts = vec![format!("update_session_topic title={}", args.title.trim())];
        if !args.tags.is_empty() {
            parts.push(format!(
                "tags={}",
                args.tags
                    .iter()
                    .map(|tag| tag.name.trim())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        Some(parts.join(" "))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!(
            "updated session topic tags: +{} -{} current={}; recall={}",
            output.tag_set_diff.added.len(),
            output.tag_set_diff.removed.len(),
            output.tag_set_diff.current.len(),
            recall_status_label(&output.recall_status)
        )
    }

    fn build_title(&self, _output: &Self::Output) -> Option<String> {
        Some("update_session_topic => updated".to_string())
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
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
        let mut policy = RecallPolicy {
            mode: crate::session_topic::RecallMode::Mechanical,
            ..RecallPolicy::default()
        };
        apply_recall_policy_override(&mut policy, args.recall_policy_override.as_ref());
        let updater = SessionTopicUpdater::with_local_recall(
            policy,
            agent.config.layout.memory_dir.clone(),
            agent.config.layout.notebook_dir.clone(),
        );
        let (output, record) = updater
            .update_with_record(UpdateSessionTopicInput {
                session_id: self.source_session_id.clone(),
                session_dir: session.session_dir.clone(),
                topic: args.title,
                tags: args.tags,
                current_turn: ctx.session().step_idx,
            })
            .await
            .map_err(map_session_topic_error)?;
        let updated_at = chrono::DateTime::parse_from_rfc3339(&record.updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        session
            .append_history_event(HistoryEvent::SessionTopicUpdated {
                session_id: record.session_id,
                topic: record.topic,
                tags: record.tags,
                tag_reasons: record.tag_reasons,
                current_turn: record.current_turn,
                updated_at,
                topic_changed: record.topic_changed,
            })
            .await;
        if let Err(err) = session
            .record_recall_background_hints(output.recall.as_ref())
            .await
        {
            warn!(
                "opendan.worksession_tools: record recall background fingerprints failed: {err:#}"
            );
        }
        Ok(output)
    }
}

fn map_session_topic_error(err: SessionTopicError) -> AgentToolError {
    match err {
        SessionTopicError::InvalidInput(msg) => AgentToolError::InvalidArgs(msg),
        other => AgentToolError::ExecFailed(format!("{other:#}")),
    }
}

fn recall_status_label(status: &crate::session_topic::RecallStatus) -> String {
    match status {
        crate::session_topic::RecallStatus::NotTriggered => "not_triggered".to_string(),
        crate::session_topic::RecallStatus::Mechanical { ms } => format!("mechanical({ms}ms)"),
        crate::session_topic::RecallStatus::Llm { ms } => format!("llm({ms}ms)"),
        crate::session_topic::RecallStatus::Failed { reason } => format!("failed({reason})"),
    }
}

/// `try_create_worksession` arguments. Per §8.2 the only LLM-supplied
/// input is a free-text `reason`; the fork sub-context derives everything
/// else (title / objective / workspace_id) by inspecting the parent
/// session's inherited history.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TryCreateWorksessionArgs {
    /// why the worksession should be created?
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
        "Try to create a worksession when the current topic feels like a long-lived work task rather than a one-off request"
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        Some(format!(
            "try_create_worksession reason={}",
            args.reason.trim()
        ))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        if let Some(session_id) = json_string(output, "session_id")
            .filter(|_| json_string(output, "status").as_deref() == Some("created"))
        {
            let workspace = json_string(output, "workspace_id").unwrap_or_else(|| "unknown".into());
            format!(
                "created and started worksession {session_id} on workspace {workspace}. Future user follow-up for this task must be forwarded automatically with forward_msg target_worksession_id={session_id}"
            )
        } else if let Some(session_id) = json_string(output, "selected_worksession_id")
            .or_else(|| json_string(output, "target_worksession_id"))
        {
            format!("selected existing worksession {session_id}")
        } else if let Some(decision) = json_string(output, "decision_text") {
            format!(
                "did not create worksession: {}",
                truncate_for_prompt(&decision, 180)
            )
        } else {
            "try_create_worksession completed".to_string()
        }
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        if json_string(output, "session_id").is_some()
            && json_string(output, "status").as_deref() == Some("created")
        {
            Some("try_create_worksession => create".to_string())
        } else if json_string(output, "selected_worksession_id")
            .or_else(|| json_string(output, "target_worksession_id"))
            .is_some()
        {
            Some("try_create_worksession => select".to_string())
        } else if json_string(output, "decision_text").is_some() {
            Some("try_create_worksession => skip".to_string())
        } else {
            Some("try_create_worksession => completed".to_string())
        }
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
        let parent_behavior = session.meta.lock().await.current_behavior.clone();
        let parent_workspace_id = session.workspace_id().await;

        // Inventory + history snapshots that drive the sub-LLM's decision:
        // - worksession_list: existing sessions (excl. caller) it might reuse
        // - workspace_list: workspaces available for binding
        // - parent_recent_history: last few user/assistant messages so the
        //   sub-LLM understands the context that produced `reason`
        let worksession_list = agent
            .list_session_summaries(Some(&self.source_session_id))
            .await;
        let before_session_ids = session_id_set(&worksession_list);
        let workspace_list = match agent.workspaces().list().await {
            Ok(mut ws) => {
                // Surface the freshest workspaces first so the sub-LLM
                // sees current candidates.
                ws.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
                available_workspaces_for_worksession(&ws, parent_workspace_id.as_deref())
            }
            Err(err) => {
                warn!(
                    "opendan.worksession_tools: list workspaces for sub-prompt failed: {err}; sub-LLM will see an empty list"
                );
                Vec::new()
            }
        };
        let before_workspace_ids = workspace_id_set(&workspace_list);
        // Parent snapshot for chat-history extraction. Missing snapshot is
        // not fatal (fork_and_run will produce its own error if it's truly
        // gone) — the sub-prompt just falls through to "no history available".
        let parent_snap = session.try_load_snapshot_for_prompt();
        let parent_history_block = parent_snap
            .as_ref()
            .map(|s| render_parent_recent_history(&s.state.accumulated))
            .unwrap_or_default();
        let parent_history_message = render_parent_recent_history_message(&parent_history_block);

        let sub_system_text = render_sub_system_prompt(
            &args.reason,
            parent_workspace_id.as_deref(),
            &worksession_list,
            &workspace_list,
        );
        let sub_system = vec![AiMessage::text(AiRole::System, sub_system_text)];

        let sub_tool_policy = worksession_sub_context_tool_policy(parent_snap.as_ref());

        let overrides = RequestOverrides {
            system_messages: Some(sub_system),
            user_messages: Some(vec![AiMessage::text(AiRole::User, parent_history_message)]),
            tool_policy: Some(sub_tool_policy),
            objective: Some(format!("Decide+create worksession for: {}", args.reason)),
            output: Some(OutputSpec::Json {
                schema: None,
                strict: false,
            }),
            // Let fork_and_run rewrite trace to `<parent>::fork-<n>`.
            trace: Some(None),
            reset_rounds: true,
            reset_errors: true,
            // Fork sub-ctx must end into its caller — never jump to a sibling
            // behavior. Waist scrubs any `<next_behavior>` the sub-LLM emits.
            forbid_next_behavior: true,
            ..Default::default()
        };

        let output = session
            .fork_and_run_agent_loop(overrides, &parent_behavior)
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("fork failed: {err:#}")))?;
        let created = created_worksession_output_from_diff(
            agent.as_ref(),
            &self.source_session_id,
            &before_session_ids,
            &before_workspace_ids,
        )
        .await;
        Ok(match (created, output) {
            (Some(created), ContextOutput::Json { content }) => {
                merge_created_worksession_output(created, content)
            }
            (Some(created), _) => created,
            (None, ContextOutput::Json { content }) => content,
            (None, ContextOutput::Text { content }) => parse_jsonish_text(&content)
                .unwrap_or_else(|| serde_json::json!({ "decision_text": content })),
        })
    }
}

fn session_id_set(summaries: &[SessionSummary]) -> HashSet<String> {
    summaries.iter().map(|s| s.session_id.clone()).collect()
}

fn workspace_id_set(workspaces: &[WorkspaceRecord]) -> HashSet<String> {
    workspaces.iter().map(|w| w.workspace_id.clone()).collect()
}

fn worksession_sub_context_tool_policy(
    parent: Option<&llm_context::state::LLMContextSnapshot>,
) -> ToolPolicy {
    let mut sub = parent
        .map(|snap| snap.request.tool_policy.clone())
        .unwrap_or_default();
    sub.mode = ToolMode::Whitelist;
    sub.whitelist = vec![
        TOOL_READ.to_string(),
        TOOL_EXEC_BASH.to_string(),
        TOOL_CREATE_WORKSESSION.to_string(),
        TOOL_CREATE_WORKSPACE.to_string(),
    ];
    sub.action_mode = ToolMode::None;
    sub.action_whitelist.clear();
    sub.disable_capabilities = vec![buckyos_api::features::WEB_SEARCH.to_string()];
    sub
}

fn available_workspaces_for_worksession(
    workspaces: &[WorkspaceRecord],
    parent_workspace_id: Option<&str>,
) -> Vec<WorkspaceRecord> {
    workspaces
        .iter()
        .filter(|w| matches!(w.status, WorkspaceStatus::Ready))
        .filter(|w| Some(w.workspace_id.as_str()) != parent_workspace_id)
        .filter(|w| {
            w.current_session
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_none()
        })
        .cloned()
        .collect()
}

async fn created_worksession_output_from_diff(
    agent: &AIAgent,
    source_session_id: &str,
    before_session_ids: &HashSet<String>,
    before_workspace_ids: &HashSet<String>,
) -> Option<serde_json::Value> {
    let after = agent.list_session_summaries(Some(source_session_id)).await;
    for summary in after {
        if before_session_ids.contains(&summary.session_id)
            || !matches!(summary.kind, SessionKind::Work)
        {
            continue;
        }
        let owner_matches = match agent.get_session(&summary.session_id).await {
            Some(session) => session.meta.lock().await.owner == source_session_id,
            None => false,
        };
        if !owner_matches {
            continue;
        }
        let workspace_id = summary.workspace_id.clone().unwrap_or_default();
        let workspace_status =
            if !workspace_id.is_empty() && before_workspace_ids.contains(&workspace_id) {
                "reused"
            } else {
                "created"
            };
        return Some(serde_json::json!({
            "session_id": summary.session_id,
            "title": summary.title,
            "workspace_id": workspace_id,
            "workspace_status": workspace_status,
            "behavior": summary.current_behavior,
            "status": "created",
            "worker_status": "started",
            "auto_started": true,
            "followup_routing": worksession_followup_routing(&summary.session_id, true),
        }));
    }
    None
}

fn merge_created_worksession_output(
    mut detected: serde_json::Value,
    sub_output: serde_json::Value,
) -> serde_json::Value {
    let Some(auto_started) = sub_output
        .get("auto_started")
        .and_then(serde_json::Value::as_bool)
    else {
        return detected;
    };
    if let Some(map) = detected.as_object_mut() {
        map.insert(
            "auto_started".to_string(),
            serde_json::Value::Bool(auto_started),
        );
        map.insert(
            "worker_status".to_string(),
            serde_json::Value::String(if auto_started { "started" } else { "idle" }.to_string()),
        );
        if let Some(session_id) = map
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        {
            map.insert(
                "followup_routing".to_string(),
                serde_json::to_value(worksession_followup_routing(&session_id, auto_started))
                    .unwrap_or(serde_json::Value::Null),
            );
        }
    }
    detected
}

fn worksession_followup_routing(
    session_id: &str,
    auto_started: bool,
) -> WorksessionFollowupRouting {
    let instruction = if auto_started {
        format!(
            "The worksession has already started. Future user follow-up information for this task must be forwarded automatically with forward_msg target_worksession_id={session_id}."
        )
    } else {
        format!(
            "The worksession was created idle. Forward user follow-up with forward_msg target_worksession_id={session_id} only when this task should run."
        )
    };
    WorksessionFollowupRouting {
        tool: TOOL_FORWARD_MSG.to_string(),
        target_worksession_id: session_id.to_string(),
        instruction,
    }
}

fn parse_jsonish_text(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&trimmed[start..=end]).ok()
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Register non-CLI session tools on `manager`. Idempotent —
/// re-registering on an already-populated manager replaces the prior
/// instances (the manager's `register_typed_tool` handles dedup).
pub fn register_worksession_tools(
    manager: &AgentToolManager,
    agent: Weak<AIAgent>,
    source_session_id: &str,
) {
    if let Err(err) = manager.register_typed_tool(CreateWorkspaceTool::new(agent.clone())) {
        warn!("opendan.worksession_tools: register `{TOOL_CREATE_WORKSPACE}` failed: {err}");
    }
    if let Err(err) =
        manager.register_typed_tool(CreateWorksessionTool::new(agent.clone(), source_session_id))
    {
        warn!("opendan.worksession_tools: register `{TOOL_CREATE_WORKSESSION}` failed: {err}");
    }
    if let Err(err) =
        manager.register_typed_tool(ForwardMsgTool::new(agent.clone(), source_session_id))
    {
        warn!("opendan.worksession_tools: register `{TOOL_FORWARD_MSG}` failed: {err}");
    }
    if let Err(err) = manager.register_typed_tool(TryCreateWorksessionTool::new(
        agent.clone(),
        source_session_id,
    )) {
        warn!("opendan.worksession_tools: register `{TOOL_TRY_CREATE_WORKSESSION}` failed: {err}");
    }
    if let Err(err) = manager.register_typed_tool(UpdateSessionTopicTool::new(
        agent.clone(),
        source_session_id,
    )) {
        warn!("opendan.worksession_tools: register `{TOOL_UPDATE_SESSION_TOPIC}` failed: {err}");
    }
}

/// Render the system prompt fed into the `try_create_worksession` fork
/// sub-context. Wraps the parent-supplied `reason` with: a directive on the
/// sub-LLM's task, the existing worksession inventory, and the workspace
/// inventory. Parent recent history is injected as the first User message,
/// not into this system prompt.
fn render_sub_system_prompt(
    reason: &str,
    parent_workspace_id: Option<&str>,
    worksession_list: &[SessionSummary],
    workspace_list: &[WorkspaceRecord],
) -> String {
    let mut out = String::new();
    out.push_str(
        r#"You are a short-lived fork sub-context spawned by `try_create_worksession`.

Step 1: decide whether to select an existing worksession or create a new worksession.
- If one existing worksession below already covers the goal, do not call `create_worksession`. Return JSON only:
  {"status":"selected","selected_worksession_id":"...","reason":"..."}.
- Otherwise create a worksession.

Step 2: if creating, decide whether to reuse an existing workspace or create a fresh workspace. Then call `create_worksession` exactly once with:
  - `task_id`: set only when an existing TaskManager task should own this worksession; otherwise omit it
  - `title`: short label you synthesize
  - `objective`: the work to do, in your own words
  - `workspace_id`: empty to mint a new workspace, or the id of an existing one from the list below that fits
  - `behavior`: empty to use the agent's default, override only when you have a strong reason
  - `reason_message`: 0–3 verbatim user messages from the inherited parent recent history that explain why this worksession is needed
  - `auto_start`: false only when the session should be created for later use without running its first turn; otherwise true

After `create_worksession` returns, return JSON only with the final worksession information from the tool result. "#,
    );
    if let Some(ws) = parent_workspace_id {
        out.push_str(&format!(
            "\nParent UI session is currently bound to workspace `{}`. That UI workspace \
             is intentionally not listed as available; create or use a work workspace \
             for the new worksession.\n",
            ws
        ));
    }
    out.push_str("\n## Reason supplied by the parent\n");
    let reason_trim = reason.trim();
    if reason_trim.is_empty() {
        out.push_str("(parent did not include a reason; rely on the inherited user message)\n");
    } else {
        out.push_str(reason_trim);
        out.push('\n');
    }

    out.push_str("\n## Existing worksessions\n");
    out.push_str(&render_worksession_inventory(worksession_list));

    out.push_str("\n## Available workspaces\n");
    out.push_str(&render_workspace_inventory(workspace_list));
    out
}

/// Render the worksession inventory section. Picks Work sessions first,
/// drops Ended ones (those are dead inventory), and caps the list to
/// [`MAX_WORKSESSION_LIST`].
fn render_worksession_inventory(summaries: &[SessionSummary]) -> String {
    let mut live: Vec<&SessionSummary> = summaries
        .iter()
        .filter(|s| !matches!(s.status, SessionStatus::Ended))
        .collect();
    // Work sessions before UI sessions — a new worksession should compare
    // against existing worksessions first; UI sessions are last-resort
    // context only.
    live.sort_by_key(|s| match s.kind {
        SessionKind::Work => 0,
        SessionKind::SelfCheck | SessionKind::SelfImprove => 1,
        SessionKind::Ui => 2,
    });
    if live.is_empty() {
        return "(no live sessions)\n".to_string();
    }
    let truncated = live.len() > MAX_WORKSESSION_LIST;
    let mut buf = String::new();
    for s in live.iter().take(MAX_WORKSESSION_LIST) {
        let kind_tag = s.kind.as_str();
        let title = if s.title.trim().is_empty() {
            "(no title)"
        } else {
            s.title.trim()
        };
        let objective = if s.objective.trim().is_empty() {
            String::new()
        } else {
            format!(" — objective: {}", truncate_for_prompt(&s.objective, 120))
        };
        let status_tag = format!("{:?}", s.status).to_lowercase();
        let ws_tag = s
            .workspace_id
            .as_deref()
            .filter(|v| !v.is_empty())
            .map(|w| format!(" [workspace `{w}`]"))
            .unwrap_or_default();
        let activity = if s.one_line_status.trim().is_empty() {
            String::new()
        } else {
            format!(" · {}", truncate_for_prompt(&s.one_line_status, 80))
        };
        buf.push_str(&format!(
            "- `{}` [{kind_tag}/{status_tag}] {}{}{}{}\n",
            s.session_id, title, ws_tag, objective, activity
        ));
    }
    if truncated {
        buf.push_str(&format!(
            "...({} more sessions truncated)\n",
            live.len() - MAX_WORKSESSION_LIST
        ));
    }
    buf
}

/// Render the workspace inventory section. Caps to [`MAX_WORKSPACE_LIST`]
/// entries (sorted upstream by recency).
pub(crate) fn render_workspace_inventory(workspaces: &[WorkspaceRecord]) -> String {
    if workspaces.is_empty() {
        return "(no workspaces yet — leave `workspace_id` empty in `create_worksession` to mint a fresh one)\n".to_string();
    }
    let mut buf = String::new();
    let truncated = workspaces.len() > MAX_WORKSPACE_LIST;
    for w in workspaces.iter().take(MAX_WORKSPACE_LIST) {
        let name = if w.name.trim().is_empty() {
            "(unnamed)".to_string()
        } else {
            w.name.trim().to_string()
        };
        let bound = w
            .current_session
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!(" [bound→`{s}`]"))
            .unwrap_or_default();
        let status_tag = format!("{:?}", w.status).to_lowercase();
        buf.push_str(&format!(
            "- `{}` ({}) — {}{}\n",
            w.workspace_id, status_tag, name, bound
        ));
    }
    if truncated {
        buf.push_str(&format!(
            "...({} more workspaces truncated)\n",
            workspaces.len() - MAX_WORKSPACE_LIST
        ));
    }
    buf
}

/// Extract the tail of user/assistant exchanges from the parent's
/// accumulated history. System / tool-result / developer roles are skipped
/// (system already came through as the prompt; tool results are noisy and
/// don't help the sub-LLM decide). Per-message text is truncated to
/// [`HISTORY_CHARS_PER_MESSAGE`].
fn render_parent_recent_history(accumulated: &[AiMessage]) -> String {
    let mut entries: Vec<(AiRole, String)> = Vec::new();
    for m in accumulated.iter() {
        if !matches!(m.role, AiRole::User | AiRole::Assistant) {
            continue;
        }
        let text = collect_message_text(m);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        entries.push((
            m.role,
            truncate_for_prompt(trimmed, HISTORY_CHARS_PER_MESSAGE),
        ));
    }
    if entries.is_empty() {
        return String::new();
    }
    let tail_start = entries.len().saturating_sub(MAX_FORWARDED_HISTORY);
    let mut buf = String::new();
    for (role, body) in entries.iter().skip(tail_start) {
        let tag = match role {
            AiRole::User => "user",
            AiRole::Assistant => "assistant",
            _ => continue,
        };
        buf.push_str(&format!("[{tag}] {}\n", body));
    }
    buf
}

fn render_parent_recent_history_message(parent_recent_history: &str) -> String {
    let mut out = String::from("## Parent recent history\n");
    if parent_recent_history.trim().is_empty() {
        out.push_str("(no inherited chat history available)\n");
    } else {
        out.push_str(parent_recent_history);
        if !parent_recent_history.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Collect the rendered text portion of an `AiMessage`. Ignores non-text
/// blocks (images / tool calls / tool results) — the sub-prompt only needs
/// the conversational backbone, not embedded media or tool internals.
fn collect_message_text(m: &AiMessage) -> String {
    let mut out = String::new();
    for block in &m.content {
        if let AiContent::Text { text } = block {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
    }
    out
}

/// Truncate `s` to `max_chars` Unicode scalars, appending an ellipsis when
/// we cut. Safe to call with `max_chars = 0`.
fn truncate_for_prompt(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut count = 0usize;
    let mut end = s.len();
    for (idx, _) in s.char_indices() {
        if count >= max_chars {
            end = idx;
            break;
        }
        count += 1;
    }
    if end < s.len() {
        let mut out = s[..end].to_string();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_workspace::WorkspaceStatus;

    // Tool names are advertised through behavior whitelists — if these
    // strings change without a coordinated update, behavior.toml files
    // silently stop activating the tools.
    #[test]
    fn tool_names_are_stable() {
        assert_eq!(TOOL_CREATE_WORKSPACE, "create_workspace");
        assert_eq!(TOOL_CREATE_WORKSESSION, "create_worksession");
        assert_eq!(TOOL_FORWARD_MSG, "forward_msg");
        assert_eq!(TOOL_TRY_CREATE_WORKSESSION, "try_create_worksession");
        assert_eq!(TOOL_UPDATE_SESSION_TOPIC, "update_session_topic");
    }

    #[test]
    fn registers_create_workspace_as_llm_tool() {
        let manager = AgentToolManager::new();
        register_worksession_tools(&manager, Weak::new(), "ui-session");
        assert!(manager.get_tool_spec(TOOL_CREATE_WORKSPACE).is_some());
        assert!(manager.get_tool_spec(TOOL_CREATE_WORKSESSION).is_some());
    }

    #[test]
    fn create_worksession_auto_start_defaults_to_true() {
        let args: CreateWorksessionArgs = serde_json::from_value(serde_json::json!({
            "title": "Task",
            "objective": "Do the task"
        }))
        .expect("args");

        assert!(args.auto_start);
    }

    #[test]
    fn create_worksession_task_id_schema_requires_non_empty_id() {
        let manager = AgentToolManager::new();
        register_worksession_tools(&manager, Weak::new(), "ui-session");
        let spec = manager
            .get_tool_spec(TOOL_CREATE_WORKSESSION)
            .expect("create_worksession spec");

        assert_eq!(
            spec.args_schema
                .pointer("/properties/task_id/minLength")
                .and_then(serde_json::Value::as_i64),
            Some(1)
        );
    }

    #[test]
    fn create_worksession_accepts_auto_start_false() {
        let args: CreateWorksessionArgs = serde_json::from_value(serde_json::json!({
            "title": "Task",
            "objective": "Do the task",
            "auto_start": false
        }))
        .expect("args");

        assert!(!args.auto_start);
    }

    #[test]
    fn try_create_summary_tells_parent_session_work_started_and_how_to_route_followups() {
        let tool = TryCreateWorksessionTool::new(Weak::new(), "ui-session");
        let output = serde_json::json!({
            "session_id": "work-1",
            "workspace_id": "workspace-1",
            "status": "created",
            "worker_status": "started",
            "auto_started": true,
            "followup_routing": worksession_followup_routing("work-1", true),
        });

        let summary = tool.build_summary(&output);

        assert!(summary.contains("created and started worksession work-1"));
        assert!(summary.contains("Future user follow-up"));
        assert!(summary.contains("forward_msg target_worksession_id=work-1"));
    }

    #[test]
    fn created_worksession_diff_output_preserves_idle_auto_start() {
        let detected = serde_json::json!({
            "session_id": "work-1",
            "worker_status": "started",
            "auto_started": true,
            "followup_routing": worksession_followup_routing("work-1", true),
        });
        let sub_output = serde_json::json!({
            "session_id": "work-1",
            "worker_status": "idle",
            "auto_started": false,
            "followup_routing": worksession_followup_routing("work-1", false),
        });

        let merged = merge_created_worksession_output(detected, sub_output);

        assert_eq!(
            merged.get("auto_started").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            merged.get("worker_status").and_then(|v| v.as_str()),
            Some("idle")
        );
        assert!(merged
            .pointer("/followup_routing/instruction")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("created idle"));
    }

    fn summary(
        id: &str,
        kind: SessionKind,
        status: SessionStatus,
        title: &str,
        objective: &str,
    ) -> SessionSummary {
        SessionSummary {
            session_id: id.to_string(),
            kind,
            title: title.to_string(),
            objective: objective.to_string(),
            status,
            one_line_status: String::new(),
            workspace_id: None,
            current_behavior: "ui_default".to_string(),
        }
    }

    fn workspace(id: &str, name: &str) -> WorkspaceRecord {
        WorkspaceRecord {
            workspace_id: id.to_string(),
            name: name.to_string(),
            created_by_session: None,
            current_session: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            status: WorkspaceStatus::Ready,
        }
    }

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate_for_prompt("hello", 10), "hello");
    }

    #[test]
    fn truncate_appends_ellipsis_when_cut() {
        let out = truncate_for_prompt("abcdefg", 3);
        assert_eq!(out, "abc…");
    }

    #[test]
    fn worksession_inventory_filters_ended_and_orders_work_first() {
        let list = vec![
            summary("ui-1", SessionKind::Ui, SessionStatus::Idle, "chat", ""),
            summary(
                "ws-done",
                SessionKind::Work,
                SessionStatus::Ended,
                "old",
                "done",
            ),
            summary(
                "ws-live",
                SessionKind::Work,
                SessionStatus::Running,
                "plan",
                "Ship rollout",
            ),
        ];
        let rendered = render_worksession_inventory(&list);
        assert!(rendered.contains("`ws-live`"), "live work session present");
        assert!(rendered.contains("[work/"), "kind tag present");
        assert!(
            !rendered.contains("`ws-done`"),
            "Ended sessions must be omitted"
        );
        // Work session ordering wins over UI
        let work_pos = rendered.find("`ws-live`").unwrap();
        let ui_pos = rendered.find("`ui-1`").unwrap();
        assert!(work_pos < ui_pos, "work sessions must come first");
        assert!(rendered.contains("Ship rollout"), "objective rendered");
    }

    #[test]
    fn worksession_inventory_handles_empty() {
        let out = render_worksession_inventory(&[]);
        assert!(out.contains("(no live sessions)"));
    }

    #[test]
    fn workspace_inventory_renders_or_hints_creation() {
        let ws = vec![workspace("ws-a", "Acme")];
        let out = render_workspace_inventory(&ws);
        assert!(out.contains("`ws-a`"));
        assert!(out.contains("Acme"));

        let empty = render_workspace_inventory(&[]);
        assert!(empty.contains("leave `workspace_id` empty"));
    }

    #[test]
    fn available_workspaces_excludes_parent_and_bound_workspaces() {
        let mut parent = workspace("ui-session-ws", "UI");
        parent.current_session = Some("ui-session".to_string());
        let mut bound = workspace("bound-work", "Bound");
        bound.current_session = Some("work-session".to_string());
        let idle = workspace("idle", "Idle");
        let mut archived = workspace("archived", "Archived");
        archived.status = WorkspaceStatus::Archived;

        let out = available_workspaces_for_worksession(
            &[parent, bound, idle.clone(), archived],
            Some("ui-session-ws"),
        );
        assert_eq!(out, vec![idle]);
    }

    #[test]
    fn parent_recent_history_filters_tool_messages() {
        let msgs = vec![
            AiMessage::text(AiRole::System, "you are an agent"),
            AiMessage::text(AiRole::User, "first message"),
            AiMessage::text(AiRole::Tool, "tool output"),
            AiMessage::text(AiRole::Assistant, "first reply"),
            AiMessage::text(AiRole::User, "second message"),
        ];
        let block = render_parent_recent_history(&msgs);
        assert!(block.contains("[user] first message"));
        assert!(block.contains("[assistant] first reply"));
        assert!(block.contains("[user] second message"));
        assert!(!block.contains("you are an agent"));
        assert!(!block.contains("tool output"));
    }

    #[test]
    fn parent_recent_history_message_is_user_payload() {
        let message = render_parent_recent_history_message("[user] first thing\n");
        assert!(message.starts_with("## Parent recent history\n"));
        assert!(message.contains("[user] first thing"));
    }

    #[test]
    fn parent_recent_history_truncates_long_tail() {
        let mut msgs = Vec::new();
        for i in 0..(MAX_FORWARDED_HISTORY + 4) {
            msgs.push(AiMessage::text(AiRole::User, format!("msg-{i}")));
        }
        let block = render_parent_recent_history(&msgs);
        let kept = block.matches("[user] msg-").count();
        assert_eq!(
            kept, MAX_FORWARDED_HISTORY,
            "should keep exactly the last MAX_FORWARDED_HISTORY entries"
        );
        // The first ones should be dropped:
        assert!(!block.contains("[user] msg-0"));
        assert!(block.contains(&format!("[user] msg-{}", msgs.len() - 1)));
    }

    #[test]
    fn sub_system_prompt_assembles_all_sections() {
        let list = vec![summary(
            "ws-1",
            SessionKind::Work,
            SessionStatus::Running,
            "Project",
            "Build the rollout plan",
        )];
        let ws = vec![workspace("ws-id", "Acme")];
        let prompt =
            render_sub_system_prompt("User asked about migrations", Some("ws-id"), &list, &ws);
        assert!(prompt.contains("Existing worksessions"));
        assert!(prompt.contains("Available workspaces"));
        assert!(!prompt.contains("## Parent recent history"));
        assert!(prompt.contains("Step 1"));
        assert!(prompt.contains("Step 2"));
        assert!(prompt.contains("`ws-1`"));
        assert!(prompt.contains("`ws-id`"));
        assert!(prompt.contains("User asked about migrations"));
        // Parent workspace hint is included
        assert!(prompt.contains("currently bound to workspace `ws-id`"));
    }

    #[test]
    fn parse_jsonish_text_extracts_embedded_object() {
        let parsed = parse_jsonish_text("```json\n{\"status\":\"selected\"}\n```").unwrap();
        assert_eq!(parsed["status"], "selected");
    }

    #[test]
    fn sub_policy_uses_standard_worksession_tool_surface() {
        let mut parent = ToolPolicy {
            mode: ToolMode::Whitelist,
            whitelist: vec![
                TOOL_TRY_CREATE_WORKSESSION.to_string(),
                "forward_msg".to_string(),
            ],
            max_rounds: 24,
            max_calls_per_round: 3,
            ..ToolPolicy::default()
        };
        parent.disable_capabilities = vec!["other".to_string()];
        let request = llm_context::request::LLMContextRequest {
            owner: llm_context::request::ContextOwnerRef::Agent {
                session_id: "s".to_string(),
            },
            trace: None,
            objective: String::new(),
            behavior_name: String::new(),
            input: Vec::new(),
            model_policy: llm_context::request::ModelPolicy::default(),
            tool_policy: parent,
            output: OutputSpec::default(),
            budget: llm_context::request::BudgetSpec::default(),
            human_policy: llm_context::request::HumanPolicy::default(),
            error_policy: llm_context::request::ErrorPolicy::default(),
            forbid_next_behavior: false,
        };
        let snap = llm_context::state::LLMContextSnapshot {
            state: llm_context::state::LLMContextState::from_request(&request, 0),
            request,
        };
        let sub = worksession_sub_context_tool_policy(Some(&snap));
        assert!(matches!(sub.mode, ToolMode::Whitelist));
        assert_eq!(
            sub.whitelist,
            vec![
                TOOL_READ.to_string(),
                TOOL_EXEC_BASH.to_string(),
                TOOL_CREATE_WORKSESSION.to_string(),
                TOOL_CREATE_WORKSPACE.to_string(),
            ]
        );
        assert!(matches!(sub.action_mode, ToolMode::None));
        assert!(sub.disable_capabilities.contains(&"web_search".to_string()));
        assert_eq!(sub.max_rounds, 24);
        assert_eq!(sub.max_calls_per_round, 3);
    }
}
