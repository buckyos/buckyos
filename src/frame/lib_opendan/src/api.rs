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
        }
    }
}

fn same_session_identity(c: &SessionConfig, agent_did: &str, spec: &SessionSpec, who: &str, driver: &str) -> bool {
    c.session.agent_did == agent_did
        && c.session.kind == spec.kind
        && c.session.created_by.principal == who
        && c.session.driver.principal == driver
        && c.session.idempotency_key == spec.idempotency_key
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
    if spec.kind == SessionKind::Ui {
        return Err(OpenDanError::InvalidArgument(
            "ui sessions are deferred (V1): only work / self_improve / self_check sessions".into(),
        ));
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

    // Input queue: created by the creator (usually the driver's app), other
    // apps may post (control / msg). "Already exists" counts as success.
    let mut inputs = Vec::new();
    if let Some(client) = channels.queue_client() {
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

    let config = SessionConfig {
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
        },
        prompt: spec.prompt.clone(),
        runtime: spec.runtime.clone(),
        workspace: spec.workspace.clone(),
        artifact_id: spec.artifact_id.clone(),
        subscriptions: spec.subscriptions.clone(),
        channels: Channels {
            inputs,
            outbound: None,
            wake_event: Some(wake_event.clone()),
        },
        extensions: spec.extensions.clone(),
    };
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
        && !(spec.idempotency_key.is_some()
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
        status: SessionStatus::created(crate::now_ms()),
        unreachable: false,
        location_rev: 0,
    };
    agent.sessions().register(entry, who).await?;
    // A work session derived from a UI session: the parent semi-subscribes
    // to it (S-17).
    if let Some(parent) = spec.origin.as_ref().and_then(|o| o.parent_session.clone()) {
        let cmd = ControlCommand::Subscribe {
            subscription: Subscription {
                id: format!("child-{sid}"),
                mode: SubscriptionMode::Semi,
                source: SubscriptionSource::Session {
                    session_ref: sid.clone(),
                },
                watch: vec![
                    "run_state".into(),
                    "outcome".into(),
                    "acceptance".into(),
                    "one_line_status".into(),
                ],
            },
        };
        let input = PostedInput::control(who, format!("subscribe:{sid}"), cmd);
        if let Err(e) = agent.sessions().post_input(&parent, &input).await {
            log::warn!("could not subscribe parent session {parent} to {sid}: {e}");
        }
    }
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
/// is configured. `input_full` (64 pending records) is retryable.
pub async fn post_input(agent: &dyn AgentStateClient, sid: &str, input: &PostedInput) -> Result<u64> {
    agent.sessions().post_input(sid, input).await
}

/// Open a self-improve session over the current perception backlog
/// (appendix A.5 without the scheduling: start conditions such as "agent is
/// idle" belong to the host). Idempotent for the same backlog window
/// (`si:<window digest>`). `None` when there is nothing to consolidate.
pub async fn create_self_improve_session(
    parent_dir: &Path,
    agent: &dyn AgentStateClient,
    who: &str,
    channels: &dyn InputChannelFactory,
    prompt: PromptSection,
) -> Result<Option<SessionDir>> {
    let cursor = agent.perception().cursor().await?;
    let backlog = agent.perception().backlog(&cursor).await?;
    if backlog.is_empty() {
        return Ok(None);
    }
    let mut spec = SessionSpec::work(
        "Consolidate the perceptions below into memory / notebook with the agent_tool CLIs, then report what was kept.",
    );
    spec.kind = SessionKind::SelfImprove;
    spec.class = "self_improve".into();
    spec.via = "opendan".into();
    spec.idempotency_key = Some(format!("si:{}", backlog.window_digest()));
    spec.prompt = prompt;
    spec.extensions.insert(
        "opendan".into(),
        serde_json::json!({ "perception_window": backlog }),
    );
    create_session(parent_dir, spec, agent, who, channels)
        .await
        .map(Some)
}
