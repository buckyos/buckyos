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

use std::sync::Weak;

use agent_tool::{AgentToolError, AgentToolManager, CallingConventions, ToolCtx, TypedTool};
use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiRole};
use llm_context::{
    outcome::ContextOutput,
    request::{ToolMode, ToolPolicy},
};
use log::warn;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::{AIAgent, CreateWorkSessionParams};
use crate::llm_context_helper::RequestOverrides;
use crate::local_workspace::WorkspaceRecord;
use crate::session_model::{SessionKind, SessionStatus, SessionSummary};
use crate::session_topic::{
    RecallPolicy, SessionTopicError, SessionTopicUpdater, UpdateSessionTopicInput,
    UpdateSessionTopicResult,
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
    /// uses the work session class's `default_behavior` from `agent.toml`.
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
    /// One-line topic hint for the current session. Write for the future self,
    /// not for the user; this is not a session summary.
    pub topic: String,
    /// Optional short tags used as coarse recall keys.
    #[serde(default)]
    pub tags: Vec<String>,
}

pub struct UpdateSessionTopicTool {
    agent: Weak<AIAgent>,
    source_session_id: String,
    updater: SessionTopicUpdater,
}

impl UpdateSessionTopicTool {
    pub fn new(agent: Weak<AIAgent>, source_session_id: impl Into<String>) -> Self {
        Self {
            agent,
            source_session_id: source_session_id.into(),
            updater: SessionTopicUpdater::with_default_recall(RecallPolicy::default()),
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
        "Update this session's one-line topic hint. Call only when the topic first becomes clear, significantly drifts, or reaches a final form. Write for your future self; do not use this for detailed summaries."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
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
        self.updater
            .update(UpdateSessionTopicInput {
                session_id: self.source_session_id.clone(),
                session_dir: session.session_dir.clone(),
                topic: args.topic,
                tags: args.tags,
                current_turn: ctx.session().step_idx,
            })
            .await
            .map_err(map_session_topic_error)
    }
}

fn map_session_topic_error(err: SessionTopicError) -> AgentToolError {
    match err {
        SessionTopicError::InvalidInput(msg) => AgentToolError::InvalidArgs(msg),
        other => AgentToolError::ExecFailed(format!("{other:#}")),
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
        "Decide whether the current request warrants a new worksession"
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
        let parent_workspace_id = session.workspace_id().await;

        // Inventory + history snapshots that drive the sub-LLM's decision:
        // - worksession_list: existing sessions (excl. caller) it might reuse
        // - workspace_list: workspaces available for binding
        // - parent_recent_history: last few user/assistant messages so the
        //   sub-LLM understands the context that produced `reason`
        let worksession_list = agent
            .list_session_summaries(Some(&self.source_session_id))
            .await;
        let workspace_list = match agent.workspaces().list().await {
            Ok(mut ws) => {
                // Surface the freshest workspaces first so the sub-LLM
                // sees current candidates.
                ws.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
                ws
            }
            Err(err) => {
                warn!(
                    "opendan.worksession_tools: list workspaces for sub-prompt failed: {err}; sub-LLM will see an empty list"
                );
                Vec::new()
            }
        };
        // Parent snapshot for chat-history extraction. Missing snapshot is
        // not fatal (fork_and_run will produce its own error if it's truly
        // gone) — the sub-prompt just falls through to "no history available".
        let parent_snap = session.try_load_snapshot_for_prompt();
        let parent_history_block = parent_snap
            .as_ref()
            .map(|s| render_parent_recent_history(&s.state.accumulated))
            .unwrap_or_default();

        let sub_system_text = render_sub_system_prompt(
            &args.reason,
            parent_workspace_id.as_deref(),
            &worksession_list,
            &workspace_list,
            &parent_history_block,
        );
        let sub_system = vec![AiMessage::text(AiRole::System, sub_system_text)];

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
            // Fork sub-ctx must end into its caller — never jump to a sibling
            // behavior. Waist scrubs any `<next_behavior>` the sub-LLM emits.
            forbid_next_behavior: true,
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

/// Register non-CLI session tools on `manager`. Idempotent —
/// re-registering on an already-populated manager replaces the prior
/// instances (the manager's `register_typed_tool` handles dedup).
pub fn register_worksession_tools(
    manager: &AgentToolManager,
    agent: Weak<AIAgent>,
    source_session_id: &str,
) {
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
/// sub-LLM's task, the existing worksession inventory, the workspace
/// inventory, and the parent's recent chat history. All sections degrade
/// gracefully when empty (skipped with a one-line note) so the sub-LLM
/// always has a coherent prompt to read.
fn render_sub_system_prompt(
    reason: &str,
    parent_workspace_id: Option<&str>,
    worksession_list: &[SessionSummary],
    workspace_list: &[WorkspaceRecord],
    parent_recent_history: &str,
) -> String {
    let mut out = String::new();
    out.push_str(
        "You are a short-lived fork sub-context spawned by `try_create_worksession`. \
         Your only job is to decide whether the parent session's situation needs a \
         new worksession, and if so, create it by calling the `create_worksession` \
         tool with concrete arguments.\n\n\
         Decide using this order:\n\
         1. If one of the existing worksessions below already covers the goal, \
            **do not call `create_worksession`** — explain the match in your reply.\n\
         2. Otherwise call `create_worksession` exactly once with:\n   \
            - `title`: short label you synthesize\n   \
            - `objective`: the work to do, in your own words\n   \
            - `workspace_id`: empty to mint a new workspace, or the id of an \
              existing one from the list below that fits\n   \
            - `behavior`: empty to use the agent's default, override only when \
              you have a strong reason\n   \
            - `reason_message`: 0–3 verbatim user messages from the recent \
              parent history that explain why this worksession is needed\n\
         3. Always terminate the run with `<next_behavior>END</next_behavior>` \
            after either creating or declining. Do not call \
            `try_create_worksession` recursively.\n",
    );
    if let Some(ws) = parent_workspace_id {
        out.push_str(&format!(
            "\nParent session is currently bound to workspace `{}`. Prefer reusing it \
             unless the new work clearly needs an isolated workspace.\n",
            ws
        ));
    }
    out.push_str("\n## Reason supplied by the parent\n");
    let reason_trim = reason.trim();
    if reason_trim.is_empty() {
        out.push_str("(parent did not include a reason; rely on the recent history below)\n");
    } else {
        out.push_str(reason_trim);
        out.push('\n');
    }

    out.push_str("\n## Existing worksessions\n");
    out.push_str(&render_worksession_inventory(worksession_list));

    out.push_str("\n## Available workspaces\n");
    out.push_str(&render_workspace_inventory(workspace_list));

    out.push_str("\n## Parent recent history\n");
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
        SessionKind::Ui => 1,
    });
    if live.is_empty() {
        return "(no live sessions)\n".to_string();
    }
    let truncated = live.len() > MAX_WORKSESSION_LIST;
    let mut buf = String::new();
    for s in live.iter().take(MAX_WORKSESSION_LIST) {
        let kind_tag = match s.kind {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
        };
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
fn render_workspace_inventory(workspaces: &[WorkspaceRecord]) -> String {
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
        assert_eq!(TOOL_CREATE_WORKSESSION, "create_worksession");
        assert_eq!(TOOL_FORWARD_MSG, "forward_msg");
        assert_eq!(TOOL_TRY_CREATE_WORKSESSION, "try_create_worksession");
        assert_eq!(TOOL_UPDATE_SESSION_TOPIC, "update_session_topic");
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
        let history = "[user] first thing\n[assistant] sure\n";
        let prompt = render_sub_system_prompt(
            "User asked about migrations",
            Some("ws-id"),
            &list,
            &ws,
            history,
        );
        assert!(prompt.contains("Existing worksessions"));
        assert!(prompt.contains("Available workspaces"));
        assert!(prompt.contains("Parent recent history"));
        assert!(prompt.contains("`ws-1`"));
        assert!(prompt.contains("`ws-id`"));
        assert!(prompt.contains("[user] first thing"));
        assert!(prompt.contains("User asked about migrations"));
        // Parent workspace hint is included
        assert!(prompt.contains("currently bound to workspace `ws-id`"));
    }
}
