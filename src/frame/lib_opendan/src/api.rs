//! Session creation, reading and posting (§4.8, §4.5).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::InputChannelFactory;
use crate::error::{OpenDanError, Result};
use crate::ids;
use crate::protocol::*;
use crate::session::{render_readme, Publish, SessionDir};
use crate::state::AgentStateClient;

/// What a creator asks for. Everything not given takes the protocol default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    pub kind: SessionKind,
    #[serde(default = "default_class")]
    pub class: String,
    /// Explicit session id; must be globally unique (§4.2) — the caller owns
    /// that guarantee. Normally left empty.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Driving identity (defaults to the creator).
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default = "default_via")]
    pub via: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub route_key: Option<String>,
    #[serde(default)]
    pub origin: Option<Origin>,
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub end_condition: EndCondition,
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub input_policy: InputPolicy,
    #[serde(default)]
    pub acl: Acl,
    #[serde(default)]
    pub task_binding: Option<Value>,
    /// User time zone bound to the session (IANA name).
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub prompt: PromptSection,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default)]
    pub workspace: Option<WorkspaceRef>,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
    /// Session template policy (`session.policy`).
    #[serde(default)]
    pub policy: SessionPolicy,
    /// Whether the session gets an input queue. `None`: one is created
    /// when the host has a queue client.
    #[serde(default)]
    pub input_channel: Option<InputChannel>,
    /// Reply coordinates of a session bound to one conversation.
    #[serde(default)]
    pub outbound: Option<OutboundBinding>,
    /// Freeze the session's behaviors from the agent's catalog at creation
    /// (xAgent §6.3). A creator that cannot read the catalog leaves it to
    /// the driver's first drive.
    #[serde(default)]
    pub freeze: bool,
    /// Goal Sessions: the Goal and epoch (see [`ensure_goal_session`]).
    #[serde(default)]
    pub goal: Option<GoalBinding>,
    /// The agent's owner (DID), when the creator knows it.
    #[serde(default)]
    pub owner: Option<String>,
}

/// Input channel of a session (xAgent §4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputChannel {
    /// No queue: bootstrap material comes from `prompt.initial_inputs`,
    /// nothing can be posted later.
    None,
    Queue,
}

fn default_class() -> String {
    "work".to_string()
}

fn default_via() -> String {
    "app".to_string()
}

impl SessionSpec {
    pub fn work(objective: impl Into<String>) -> Self {
        Self {
            kind: SessionKind::Work,
            class: default_class(),
            session_id: None,
            driver: None,
            via: default_via(),
            idempotency_key: None,
            route_key: None,
            origin: None,
            objective: objective.into(),
            end_condition: EndCondition::default(),
            scope: None,
            input_policy: InputPolicy::Any,
            acl: Acl::default(),
            task_binding: None,
            timezone: None,
            prompt: PromptSection::default(),
            runtime: RuntimeSection::default(),
            workspace: None,
            artifact_id: None,
            subscriptions: Vec::new(),
            extensions: BTreeMap::new(),
            policy: SessionPolicy::default(),
            input_channel: None,
            outbound: None,
            freeze: false,
            goal: None,
            owner: None,
        }
    }
}

fn same_session_identity(c: &SessionConfig, agent_did: &str, spec: &SessionSpec, who: &str, driver: &str) -> bool {
    c.session.agent_did == agent_did
        && c.session.kind == spec.kind
        && c.session.created_by.principal == who
        && c.session.driver.principal == driver
        && c.session.idempotency_key == spec.idempotency_key
        && c.workspace == spec.workspace
}

/// Create a session directory under `parent_dir` (any location, §4.1),
/// create its kmsg input queue, and register it in the Agent State.
///
/// Idempotent for the same `(agent, creator, idempotency_key)`: a retry
/// finishes a half-done creation (directory exists → registration still
/// happens).
pub async fn create_session(
    parent_dir: &Path,
    spec: SessionSpec,
    agent: &dyn AgentStateClient,
    who: &str,
    channels: &dyn InputChannelFactory,
) -> Result<SessionDir> {
    let mut spec = spec;
    if spec.kind == SessionKind::Goal && (spec.goal.is_none() || spec.session_id.is_none()) {
        return Err(OpenDanError::InvalidArgument(
            "goal sessions are created by the host for a goal (`xagent goal create`, kRPC goals.create)".into(),
        ));
    }
    if spec.kind == SessionKind::Ui && spec.route_key.is_none() {
        // Not bound to a mailbox (a local dialogue): a route of its own.
        spec.route_key = Some(match &spec.idempotency_key {
            Some(k) => format!("local:{k}"),
            None => format!("local:{}", uuid::Uuid::new_v4().simple()),
        });
    }
    let agent_did = agent.agent_did().to_string();
    let sid = match &spec.session_id {
        Some(s) => {
            ids::validate_session_id(s)?;
            s.clone()
        }
        None => ids::derive_session_id(
            spec.kind,
            &agent_did,
            who,
            spec.idempotency_key.as_deref(),
            spec.route_key.as_deref(),
        )?,
    };
    let driver = spec.driver.clone().unwrap_or_else(|| who.to_string());
    if spec.prompt.initial_inputs.len() > MAX_PENDING_INPUTS {
        return Err(OpenDanError::InvalidArgument(format!(
            "prompt.initial_inputs holds at most {MAX_PENDING_INPUTS} records"
        )));
    }
    for i in &spec.prompt.initial_inputs {
        i.validate()?;
        if !matches!(i.input, SessionInput::Msg(_)) {
            return Err(OpenDanError::InvalidArgument(
                "prompt.initial_inputs only takes msg records".into(),
            ));
        }
    }
    let queue_client = match spec.input_channel {
        Some(InputChannel::None) => None,
        Some(InputChannel::Queue) => Some(channels.queue_client().ok_or_else(|| {
            OpenDanError::Channel("the session needs an input queue but the host has no queue client".into())
        })?),
        None => channels.queue_client(),
    };
    if queue_client.is_some() && !spec.prompt.initial_inputs.is_empty() {
        return Err(OpenDanError::InvalidArgument(
            "prompt.initial_inputs is the bootstrap material of a session without an input queue; post to the queue instead".into(),
        ));
    }

    // Input queue: created by the creator (usually the driver's app), other
    // apps may post (control / msg). "Already exists" counts as success.
    let mut inputs = Vec::new();
    if let Some(client) = queue_client {
        let (app, owner) = ids::parse_app_principal(&driver)
            .unwrap_or_else(|| ("opendan".to_string(), "unknown".to_string()));
        let queue =
            crate::channel::kmsg::ensure_queue(&client, &ids::queue_name(&sid), &app, &owner)
                .await?;
        let subscriber = ids::subscriber_id(agent.agent_id(), &sid);
        crate::channel::kmsg::ensure_subscription(
            &client,
            &queue,
            &subscriber,
            &owner,
            &app,
            buckyos_api::msg_queue::SubPosition::Earliest,
        )
        .await?;
        inputs.push(InputSourceConfig::Kmsg {
            id: "q".to_string(),
            queue,
            subscriber,
        });
    }
    let wake_event = ids::wake_event(agent.agent_id(), &sid);

    let mut config = SessionConfig {
        schema: SESSION_CONFIG_SCHEMA.to_string(),
        config_rev: 1,
        session: SessionSection {
            session_id: sid.clone(),
            agent_did: agent_did.clone(),
            kind: spec.kind,
            class: spec.class.clone(),
            created_at_ms: crate::now_ms(),
            created_by: CreatedBy {
                principal: who.to_string(),
                via: spec.via.clone(),
            },
            driver: DriverRef {
                principal: driver.clone(),
            },
            idempotency_key: spec.idempotency_key.clone(),
            route_key: spec.route_key.clone(),
            origin: spec.origin.clone(),
            objective: spec.objective.clone(),
            end_condition: spec.end_condition.clone(),
            scope: spec.scope.clone(),
            input_policy: spec.input_policy,
            acl: spec.acl.clone(),
            task_binding: spec.task_binding.clone(),
            timezone: spec.timezone.clone(),
            policy: spec.policy.clone(),
            goal: spec.goal.clone(),
            owner: spec.owner.clone(),
        },
        prompt: spec.prompt.clone(),
        runtime: spec.runtime.clone(),
        workspace: spec.workspace.clone(),
        workspace_binding: crate::runtime::resolve_workspace(agent, spec.workspace.as_ref())
            .await?,
        artifact_id: spec.artifact_id.clone(),
        subscriptions: spec.subscriptions.clone(),
        channels: Channels {
            inputs,
            outbound: spec.outbound.clone(),
            wake_event: Some(wake_event.clone()),
        },
        extensions: spec.extensions.clone(),
    };
    crate::runtime::validate_workspace_workdir(&config)?;
    if spec.freeze {
        match crate::state::freeze_config(&mut config, agent.behaviors(), who).await {
            Ok(()) => {}
            // A wrong behavior / entry mode is the creator's error.
            Err(e @ OpenDanError::InvalidArgument(_)) => return Err(e),
            Err(e) => log::info!(
                "session {sid}: behaviors not frozen at creation ({e}); the driver freezes them"
            ),
        }
    }
    let created = WorklogBody::Created {
        session_id: sid.clone(),
        kind: spec.kind.as_str().to_string(),
        by: who.to_string(),
        objective: spec.objective.clone(),
        at_ms: crate::now_ms(),
    };
    let readme = render_readme(&config);
    // ACL: no file-level ACL exists yet (§1.5); `acl` is recorded only.
    let (sd, publish) = SessionDir::publish_new(parent_dir, &config, created, &readme)?;
    let existing = sd.config()?;
    if publish == Publish::AlreadyExists
        && !((spec.idempotency_key.is_some() || spec.kind == SessionKind::Ui)
            && same_session_identity(&existing, &agent_did, &spec, who, &driver))
    {
        return Err(OpenDanError::SessionIdConflict(sid));
    }
    let canonical = sd
        .path()
        .canonicalize()
        .unwrap_or_else(|_| sd.path().to_path_buf());
    let entry = RegistryEntry {
        session_id: sid.clone(),
        kind: existing.session.kind,
        class: existing.session.class.clone(),
        created_by: existing.session.created_by.clone(),
        idempotency_key: existing.session.idempotency_key.clone(),
        route_key: existing.session.route_key.clone(),
        driver: existing.session.driver.clone(),
        location: canonical.display().to_string(),
        input_queue: existing.channels.kmsg().map(|(_, q, _)| q.to_string()),
        wake_event: existing.channels.wake_event.clone(),
        agent_access: existing.session.acl.agent_access,
        origin: existing.session.origin.clone(),
        workspace: existing.workspace.clone(),
        artifact_id: existing.artifact_id.clone(),
        objective: existing.session.objective.clone(),
        scope: existing.session.scope.clone(),
        goal_id: existing
            .session
            .goal
            .as_ref()
            .map(|g| g.goal_id.clone())
            .or_else(|| {
                existing
                    .session
                    .origin
                    .as_ref()
                    .and_then(|o| o.goal.as_ref())
                    .map(|g| g.goal_id.clone())
            }),
        status: SessionStatus::created(crate::now_ms()),
        unreachable: false,
        location_rev: 0,
    };
    agent.sessions().register(entry, who).await?;
    // A parent's attention to its sub sessions is implicit: its runner
    // reads them from the registry by `origin.parent_session` (xAgent §4.14).
    Ok(sd)
}

/// What a reader gets back (§4.8 `read_session`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    pub entry: RegistryEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<SessionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worklog: Vec<WorklogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Read a session through the registry. `agent_side` readers honour
/// `agent_access = status_only`. Worklog tails are read backwards from the
/// committed end only.
pub async fn read_session(
    agent: &dyn AgentStateClient,
    sid: &str,
    agent_side: bool,
    with_report: bool,
    worklog_tail: usize,
) -> Result<SessionView> {
    let entry = agent
        .sessions()
        .lookup(sid)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
    let mut view = SessionView {
        entry: entry.clone(),
        state: None,
        report: None,
        worklog: Vec::new(),
        note: None,
    };
    if agent_side && entry.agent_access == AgentAccess::StatusOnly {
        view.note = Some("session exposes its status only (agent_access = status_only)".into());
        return Ok(view);
    }
    let sd = match SessionDir::open(&entry.location) {
        Ok(sd) => sd,
        Err(e) => {
            view.note = Some(format!("session directory not readable: {e}"));
            return Ok(view);
        }
    };
    let state = sd.state()?;
    if with_report {
        view.report = sd.report();
    }
    if worklog_tail > 0 {
        view.worklog = sd
            .worklog()
            .recent(state.worklog.committed_bytes, worklog_tail)?
            .into_iter()
            .map(|(_, e)| e)
            .collect();
    }
    view.state = Some(state);
    Ok(view)
}

/// Post a record to a session's input bus through the registry (anyone
/// with write access to its queue). Publishes the wake event when a waker
/// is configured. `input_full` (64 pending records) and `queue_missing`
/// (the driver recreates a lost queue) are retryable.
pub async fn post_input(agent: &dyn AgentStateClient, sid: &str, input: &PostedInput) -> Result<u64> {
    agent.sessions().post_input(sid, input).await
}

/// Where a sub session works.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubWorkspace {
    #[default]
    Inherit,
    None,
    Id(String),
}

/// What a session asks for when it hands a piece of work to a sub session
/// (xAgent §4.11). A sub session is an ordinary session whose
/// `origin.parent_session` names its parent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubSessionSpec {
    pub objective: String,
    /// First inputs (text messages from the agent itself).
    #[serde(default)]
    pub msgs: Vec<String>,
    /// Data objects attached to the first message.
    #[serde(default)]
    pub attachments: Vec<(String, Option<String>)>,
    /// Attach an excerpt of the parent's last `n` dialogue records.
    #[serde(default)]
    pub context_recent: usize,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub behavior: Option<String>,
    /// The sub session's cmd_skills, chosen now (at most
    /// [`MAX_CMD_SKILLS`]). `None`: a Work parent's frozen choice (the
    /// manuals still installed), else the template's default.
    #[serde(default)]
    pub cmd_skills: Option<Vec<String>>,
    #[serde(default)]
    pub workspace: SubWorkspace,
    /// Explicit runtime id requirement (`None`: inherit the parent's
    /// runtime configuration; a tmux runtime is derived per session).
    #[serde(default)]
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub report: ReportMode,
    /// The sub session gets an input queue and may wait for input.
    #[serde(default)]
    pub interactive: bool,
    /// `(run_id, call_id)` of the creating tool call: the idempotency key.
    #[serde(default)]
    pub call: Option<(String, String)>,
    /// Idempotency key when there is no call identity.
    #[serde(default)]
    pub key: Option<String>,
}

/// Last `n` dialogue records of a session (user / assistant text), oldest
/// first: a labelled excerpt, not the session's context.
pub fn dialogue_excerpt(sd: &SessionDir, n: usize) -> Result<String> {
    if n == 0 {
        return Ok(String::new());
    }
    let state = sd.state()?;
    let mut lines = Vec::new();
    let mut r = sd.worklog().reverse(state.worklog.committed_bytes, 0)?;
    while lines.len() < n {
        let Some((_, e)) = r.next_json::<WorklogEntry>()? else {
            break;
        };
        let (role, text) = match e.body {
            WorklogBody::UserMessage { content, .. } => ("user", content),
            WorklogBody::AssistantMessage { assistant, .. }
            | WorklogBody::Step { assistant, .. } => ("assistant", assistant),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        let cut: String = text.trim().chars().take(600).collect();
        lines.push(format!("[{role}] {cut}"));
    }
    lines.reverse();
    Ok(lines.join("\n"))
}

/// Nesting depth of `sid` below its root session.
async fn session_depth(agent: &dyn AgentStateClient, entry: &RegistryEntry) -> Result<usize> {
    let mut depth = 0;
    let mut cur = entry.origin.as_ref().and_then(|o| o.parent_session.clone());
    while let Some(p) = cur {
        depth += 1;
        if depth > 32 {
            break;
        }
        cur = agent
            .sessions()
            .lookup(&p)
            .await?
            .and_then(|e| e.origin.and_then(|o| o.parent_session));
    }
    Ok(depth)
}

/// Idempotency key of a sub session: its creating call, or the explicit
/// key (`None`: neither, every creation is a new session).
fn sub_session_key(parent_sid: &str, sub: &SubSessionSpec) -> Option<String> {
    match (&sub.call, &sub.key) {
        (Some((run, call)), _) => Some(format!("sub:{}", ids::h(&[parent_sid, run, call]))),
        (None, Some(k)) => Some(format!("sub:{}", ids::h(&[parent_sid, k]))),
        (None, None) => None,
    }
}

/// The id [`create_sub_session`] gives the sub session `sub` of
/// `parent_sid` created by `who` — known before it exists, so a creator can
/// record it first and find it again after a crash. Needs an idempotency
/// identity (`sub.call` or `sub.key`).
pub fn sub_session_id(
    agent: &dyn AgentStateClient,
    who: &str,
    parent_sid: &str,
    sub: &SubSessionSpec,
) -> Result<String> {
    let key = sub_session_key(parent_sid, sub).ok_or_else(|| {
        OpenDanError::InvalidArgument("a sub session id is only known for a creating call or a key".into())
    })?;
    let class = sub.class.clone().unwrap_or_else(|| "work".to_string());
    let template = crate::template::SessionTemplate::load(&class, agent.agent_root())?;
    ids::derive_session_id(template.kind, agent.agent_did(), who, Some(&key), None)
}

/// Apply `accept | discard` to a finished Work that has no input queue,
/// where no drive would ever apply a posted decision: under the session's
/// own lock, by its driver (`who`). The artifact version, when there is
/// one, is decided first. Returns the new acceptance.
pub async fn decide_unqueued(
    agent: &dyn AgentStateClient,
    who: &str,
    sid: &str,
    decision: &str,
    note: Option<&str>,
) -> Result<Acceptance> {
    let entry = agent
        .sessions()
        .lookup(sid)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
    let sd = SessionDir::open(&entry.location)?;
    let holder = HolderInfo {
        runner_id: ids::new_runner_id(),
        principal: who.to_string(),
        host: Some(crate::runtime::native_host_id()),
        pid: std::process::id(),
        runtime_id: None,
    };
    let lease = match sd.acquire(holder)? {
        crate::lock::Acquire::Acquired(l) => l,
        crate::lock::Acquire::Busy(info) => {
            return Err(OpenDanError::Busy {
                resource: format!("session:{sid}"),
                holder: info.and_then(|i| serde_json::to_value(i).ok()),
            })
        }
    };
    let result = decide_locked(agent, &sd, &lease, who, decision, note).await;
    lease.release();
    result
}

async fn decide_locked(
    agent: &dyn AgentStateClient,
    sd: &SessionDir,
    lease: &crate::lock::Lease,
    who: &str,
    decision: &str,
    note: Option<&str>,
) -> Result<Acceptance> {
    let mut s = sd.load(lease)?;
    if s.config.session.kind != SessionKind::Work || !s.state.is_finished() {
        return Err(OpenDanError::InvalidArgument(format!(
            "session {} is not a finished work session: a decision applies to a result",
            sd.sid()
        )));
    }
    let target = match decision {
        "accept" => Acceptance::Accepted,
        "discard" => Acceptance::Discarded,
        other => {
            return Err(OpenDanError::InvalidArgument(format!(
                "decide takes accept | discard, not `{other}`"
            )))
        }
    };
    if s.state.acceptance == target {
        return Ok(target);
    }
    let valid = match target {
        Acceptance::Accepted => s.state.acceptance == Acceptance::Pending,
        _ => matches!(s.state.acceptance, Acceptance::Pending | Acceptance::Accepted),
    };
    if !valid {
        return Err(OpenDanError::InvalidArgument(format!(
            "decide `{decision}` is not valid while acceptance is {:?}",
            s.state.acceptance
        )));
    }
    let mut head_after = None;
    if let Some(aid) = s.config.artifact_id.clone() {
        let holder = HolderInfo {
            runner_id: ids::new_runner_id(),
            principal: who.to_string(),
            host: Some(crate::runtime::native_host_id()),
            pid: std::process::id(),
            runtime_id: None,
        };
        let alease = match agent.locks().acquire(&format!("artifact:{aid}"), holder)? {
            crate::lock::Acquire::Acquired(l) => l,
            crate::lock::Acquire::Busy(info) => {
                return Err(OpenDanError::Busy {
                    resource: format!("artifact:{aid}"),
                    holder: info.and_then(|i| serde_json::to_value(i).ok()),
                })
            }
        };
        let ver = format!("v-{}", sd.sid());
        let r = agent.artifacts().decide(&alease, &aid, &ver, decision).await;
        alease.release();
        head_after = Some(r?.head_after);
    }
    let mut report = Value::Null;
    if target == Acceptance::Discarded {
        let r = DiscardReport {
            reverted: Vec::new(),
            workspace: if s.config.workspace.is_some() { "unsupported" } else { "none" }.into(),
            unsupported: crate::runner::side_effects_of(&s)?,
            head_moved_to: head_after,
            note: note.map(str::to_string),
        };
        report = serde_json::to_value(&r).unwrap_or(Value::Null);
        let mut result = s.state.result.clone().unwrap_or_else(|| serde_json::json!({}));
        result["discard_report"] = report.clone();
        s.state.result = Some(result);
    }
    s.state.acceptance = target;
    s.state.pending_decision = None;
    s.append_worklog(
        lease,
        vec![WorklogBody::Decide {
            decision: decision.to_string(),
            by: who.to_string(),
            report,
        }],
    )?;
    s.commit_state(lease)?;
    let status = s.status(lease.epoch());
    if let Err(e) = agent.sessions().report_state(lease, sd.sid(), status).await {
        log::warn!("report state of {}: {e}", sd.sid());
    }
    Ok(target)
}

/// Create a sub session of `parent_sid`: registered with
/// `origin = {parent_session, report, created_by_call}`, driven by the
/// parent's driver, not advanced here. Idempotent for the same creating
/// call. Refused beyond the parent's `max_sub_sessions` (not finished at the
/// same time) and `max_session_depth`.
pub async fn create_sub_session(
    agent: &dyn AgentStateClient,
    who: &str,
    parent_sid: &str,
    sub: SubSessionSpec,
    channels: &dyn InputChannelFactory,
) -> Result<SessionDir> {
    let parent = agent
        .sessions()
        .lookup(parent_sid)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("parent session {parent_sid}")))?;
    let parent_dir = SessionDir::open(&parent.location)?;
    let pcfg = parent_dir.config()?;
    let key = sub_session_key(parent_sid, &sub)
        .unwrap_or_else(|| format!("sub:{}", uuid::Uuid::new_v4().simple()));
    let class = sub.class.clone().unwrap_or_else(|| "work".to_string());
    let template = crate::template::SessionTemplate::load(&class, agent.agent_root())?;
    if template.kind == SessionKind::Goal {
        return Err(OpenDanError::InvalidArgument(
            "a goal session is never a sub session: create a goal (`agent-session goal create`)".into(),
        ));
    }
    let sid = ids::derive_session_id(template.kind, agent.agent_did(), who, Some(&key), None)?;
    // The same call again: the session it created, never a second one.
    if let Some(existing) = agent.sessions().lookup(&sid).await? {
        return SessionDir::open(&existing.location);
    }
    // An explicit runtime is one of the agent's registered runtimes, usable
    // now; the agent's own environment needs no requirement of its own.
    let runtime_requirement = match sub.runtime_id.as_deref() {
        None => None,
        Some(id) => {
            let record = agent.runtimes().lookup(id).await?.ok_or_else(|| {
                OpenDanError::Bind(format!("runtime `{id}` is not registered (see `runtime list`)"))
            })?;
            if record.availability != RuntimeAvailability::Available {
                return Err(OpenDanError::Bind(format!(
                    "runtime `{id}` is not available: {}",
                    record
                        .last_error
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", record.availability))
                )));
            }
            (!is_native_runtime(&record.runtime_id)).then(|| record.runtime_id.clone())
        }
    };
    let policy = &pcfg.session.policy;
    // A Goal's Work counts for the Goal, whichever epoch created it.
    let goal_origin = match &pcfg.session.goal {
        Some(binding) => Some(crate::goal::work_admission(agent, &pcfg, &parent_dir, binding).await?),
        None => None,
    };
    let siblings = match &goal_origin {
        Some(g) => agent
            .sessions()
            .query(&RegistryQuery {
                goal_id: Some(g.goal_id.clone()),
                ..Default::default()
            })
            .await?
            .into_iter()
            .filter(|e| e.kind != SessionKind::Goal && e.status.run_state != RunState::Finished)
            .count(),
        None => agent
            .sessions()
            .children_of(&[parent_sid.to_string()])
            .await?
            .into_iter()
            .filter(|e| e.status.run_state != RunState::Finished)
            .count(),
    };
    if siblings >= policy.max_sub_sessions as usize {
        return Err(OpenDanError::InvalidArgument(format!(
            "session {parent_sid} already has {siblings} unfinished sub sessions (max_sub_sessions = {}); wait for one or stop it",
            policy.max_sub_sessions
        )));
    }
    let depth = session_depth(agent, &parent).await? + 1;
    if depth > policy.max_session_depth as usize {
        return Err(OpenDanError::InvalidArgument(format!(
            "sub sessions are nested {} deep (max_session_depth = {}); do the work in this session",
            depth - 1,
            policy.max_session_depth
        )));
    }
    let mut spec = template.spec(sub.objective.clone());
    spec.idempotency_key = Some(key);
    spec.driver = Some(parent.driver.principal.clone());
    spec.via = format!("session:{parent_sid}");
    spec.origin = Some(Origin {
        parent_session: Some(parent_sid.to_string()),
        intent_ref: None,
        reason_messages: Vec::new(),
        report: Some(sub.report),
        created_by_call: sub.call.as_ref().map(|(run, call)| format!("{run}/{call}")),
        parent_task: parent_dir
            .state()
            .ok()
            .and_then(|s| s.open_turn_task().map(|t| t.task_id.clone()))
            .or_else(|| {
                pcfg.session
                    .origin
                    .as_ref()
                    .and_then(|o| o.parent_task.clone())
            }),
        goal: goal_origin,
    });
    spec.timezone = pcfg.session.timezone.clone();
    spec.prompt.llm_context = pcfg.prompt.llm_context.clone();
    spec.runtime = pcfg.runtime.clone();
    // The binding identity is the sub session's own: only an explicit
    // requirement is carried, the runtime configuration is inherited.
    spec.runtime.requirement.runtime_id = runtime_requirement;
    spec.policy.max_session_depth = policy.max_session_depth;
    if let Some(b) = sub.behavior.clone() {
        spec.prompt.behavior = Some(b);
    }
    spec.prompt.cmd_skills = match &sub.cmd_skills {
        Some(names) => {
            crate::state::check_cmd_skill_names(names)?;
            names.clone()
        }
        // A Work hands its manuals on to the work it splits off; a UI's or
        // a Goal's own manuals are never a Work's.
        None if pcfg.session.kind == SessionKind::Work => {
            let chosen: Vec<String> = match &pcfg.prompt.frozen {
                Some(f) => f.cmd_skills.iter().map(|c| c.name.clone()).collect(),
                None => pcfg.prompt.cmd_skills.clone(),
            };
            let installed: Vec<String> = agent
                .behaviors()
                .skills(Some(SkillKind::Cmd))
                .await?
                .into_iter()
                .map(|m| m.name)
                .collect();
            chosen.into_iter().filter(|n| installed.contains(n)).collect()
        }
        None => spec.prompt.cmd_skills.clone(),
    };
    spec.freeze = spec.prompt.behavior.is_some()
        || pcfg.prompt.frozen.is_some()
        || !spec.prompt.cmd_skills.is_empty();
    spec.workspace = match &sub.workspace {
        SubWorkspace::Inherit => pcfg.workspace.clone(),
        SubWorkspace::None => None,
        SubWorkspace::Id(id) => {
            if pcfg
                .workspace
                .as_ref()
                .is_some_and(|w| w.workspace_id != *id)
                || (pcfg.workspace.is_none() && pcfg.session.kind != SessionKind::Ui)
            {
                return Err(OpenDanError::InvalidArgument(
                    "a child session cannot acquire a different workspace through inheritance"
                        .into(),
                ));
            }
            Some(WorkspaceRef {
                workspace_id: id.clone(),
                access: pcfg
                    .workspace
                    .as_ref()
                    .map(|w| w.access)
                    .unwrap_or_default(),
            })
        }
    };
    if let Some(runtime) = spec
        .prompt
        .llm_context
        .get_mut("runtime")
        .and_then(Value::as_object_mut)
    {
        runtime.remove("workdir");
    }
    if sub.interactive {
        spec.input_channel = Some(InputChannel::Queue);
        spec.policy.wait_user_msg = WaitPolicy::Allowed;
    }
    // First inputs: messages of the agent itself.
    let me = parse_did(agent.agent_did())?;
    let mut texts = sub.msgs.clone();
    if sub.context_recent > 0 {
        let excerpt = dialogue_excerpt(&parent_dir, sub.context_recent)?;
        if !excerpt.is_empty() {
            texts.push(format!(
                "Excerpt of the parent session's recent dialogue (for reference only):\n{excerpt}"
            ));
        }
    }
    let mut first = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        let mut msg = text_msg(&me, &me, t.clone());
        if i == 0 {
            for (id, name) in &sub.attachments {
                let obj_id = ndn_lib::ObjId::new(id).map_err(|e| {
                    OpenDanError::InvalidArgument(format!("attachment `{id}` is not an ObjId: {e}"))
                })?;
                msg = attach(msg, obj_id, name.clone());
            }
        }
        first.push(PostedInput::msg(who, msg, MsgDelivery::default())?);
    }
    let queued = spec.input_channel == Some(InputChannel::Queue);
    if !queued {
        spec.prompt.initial_inputs = first.clone();
    }
    let parent_of_dirs = parent_dir
        .path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| parent_dir.path().to_path_buf());
    let sd = create_session(&parent_of_dirs, spec, agent, who, channels).await?;
    if queued {
        for input in &first {
            agent.sessions().post_input(sd.sid(), input).await?;
        }
    }
    Ok(sd)
}
