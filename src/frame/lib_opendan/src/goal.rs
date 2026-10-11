//! Goals and their Goal Sessions — what the host, the runner and the CLIs
//! share: deriving the Goal Session of an epoch, posting run opportunities,
//! the effective budget and its statistics period, the admission of a Work.
//!
//! The Goal object (`state/goals/<goal_id>/goal.json`) is the truth; a Goal
//! Session only carries `session.goal = {goal_id, epoch}`.

use std::path::Path;

use serde_json::Value;

use crate::api::{create_session, SessionSpec};
use crate::channel::InputChannelFactory;
use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::AgentStateClient;
use crate::template::{GoalDefaults, SessionTemplate};

/// Session class of Goal Sessions (`agent.toml [session.goal]`).
pub const GOAL_CLASS: &str = "goal";

/// `[session.<class>]` defaults of a Goal (built-in when the class has
/// none).
pub fn defaults(agent_root: Option<&Path>, class: &str) -> GoalDefaults {
    SessionTemplate::load(class, agent_root)
        .ok()
        .and_then(|t| t.goal)
        .unwrap_or_default()
}

/// The Goal's own budget over the defaults.
pub fn effective_budget(goal: &GoalDefinition, d: &GoalDefaults) -> GoalBudget {
    goal.budget.over(&d.budget)
}

pub fn window_secs(b: &GoalBudget) -> u64 {
    b.period.window_secs.filter(|w| *w > 0).unwrap_or(86_400)
}

/// Start of the fixed statistics window `now_ms` falls in.
pub fn period_start(now_ms: u64, window_secs: u64) -> u64 {
    let w = window_secs.max(1) * 1000;
    now_ms / w * w
}

/// Move `usage` into the window of `now_ms` (a new window starts at zero).
pub fn roll_usage(usage: &mut GoalUsage, now_ms: u64, window_secs: u64) {
    let start = period_start(now_ms, window_secs);
    if usage.period_start_ms < start {
        *usage = GoalUsage {
            period_start_ms: start,
            ..Default::default()
        };
    }
}

/// Usage of the current window (zero when the stored one is older).
pub fn current_usage(state: Option<&GoalState>, now_ms: u64, window_secs: u64) -> GoalUsage {
    let mut u = state.map(|s| s.usage.clone()).unwrap_or_default();
    roll_usage(&mut u, now_ms, window_secs);
    u
}

/// Why the period budget refuses more, if it does.
pub fn period_exhausted(b: &GoalBudget, u: &GoalUsage) -> Option<String> {
    if let Some(max) = b.period.max_total_tokens {
        if u.tokens + u.work_tokens >= max {
            return Some(format!(
                "{} of {max} tokens of the period used",
                u.tokens + u.work_tokens
            ));
        }
    }
    if let Some(max) = b.period.max_activations {
        if u.activations >= u64::from(max) {
            return Some(format!("{} of {max} activations of the period used", u.activations));
        }
    }
    None
}

/// Input key of the event that starts epoch `epoch`.
pub fn start_key(goal_id: &str, epoch: u64) -> String {
    format!("goal:{goal_id}:start:{epoch}")
}

/// Input key of a scheduled fire (`missed`: slots folded into it).
pub fn scheduled_key(goal_id: &str, slot: u64, missed: u64) -> String {
    format!("goal:{goal_id}:scheduled:{slot}:{missed}")
}

pub fn activate_key(goal_id: &str, request_id: &str) -> String {
    format!("goal:{goal_id}:activate:{request_id}")
}

/// `event` name of a run opportunity of the runtime.
pub fn event_name(kind: ActivationReasonKind) -> &'static str {
    match kind {
        ActivationReasonKind::Created => "created",
        ActivationReasonKind::Scheduled => "scheduled",
        ActivationReasonKind::Manual => "activate",
        ActivationReasonKind::Reopened => "reopened",
        ActivationReasonKind::Restarted => "restarted",
        ActivationReasonKind::Child => "child",
        ActivationReasonKind::Feedback => "feedback",
        ActivationReasonKind::Event => "event",
    }
}

/// The reason an input of an Activation's batch stands for.
pub fn reason_of(input: &FetchedInput) -> ActivationReason {
    let base = |kind| ActivationReason {
        kind,
        key: input.key.clone(),
        reference: None,
        missed: None,
        note: None,
    };
    match &input.input {
        Ok(SessionInput::Event(ev)) if ev.source.kind == "system" && ev.source.id == GOAL_SOURCE_ID => {
            let kind = match ev.event.as_str() {
                "created" => ActivationReasonKind::Created,
                "scheduled" => ActivationReasonKind::Scheduled,
                "reopened" => ActivationReasonKind::Reopened,
                "restarted" => ActivationReasonKind::Restarted,
                _ => ActivationReasonKind::Manual,
            };
            let mut r = base(kind);
            if kind == ActivationReasonKind::Scheduled {
                r.missed = input
                    .key
                    .rsplit(':')
                    .next()
                    .and_then(|n| n.parse::<u64>().ok())
                    .filter(|n| *n > 1);
            }
            r.note = Some(ev.summary.clone()).filter(|s| !s.is_empty());
            r
        }
        Ok(SessionInput::Event(ev)) if input.src == INTERNAL_CHILD_SRC => {
            let mut r = base(ActivationReasonKind::Child);
            r.reference = Some(ev.source.id.clone());
            r.note = Some(ev.event.clone());
            r
        }
        Ok(SessionInput::Event(ev)) => {
            let mut r = base(ActivationReasonKind::Event);
            r.reference = Some(ev.source.label());
            r.note = Some(ev.summary.chars().take(200).collect());
            r
        }
        Ok(SessionInput::Msg(m)) => {
            let mut r = base(ActivationReasonKind::Feedback);
            r.reference = Some(input.key.clone());
            r.note = m.delivery.from_name.clone();
            r
        }
        _ => base(ActivationReasonKind::Event),
    }
}

/// A run opportunity for the Goal's current Goal Session, posted to its
/// input bus with a stable `key` (a re-post is consumed once).
pub async fn post_activation(
    agent: &dyn AgentStateClient,
    goal: &GoalDefinition,
    who: &str,
    kind: ActivationReasonKind,
    key: &str,
    note: &str,
) -> Result<u64> {
    let ev = AgentEvent {
        subscription_id: Some(GOAL_ACTIVATION_SUBSCRIPTION.to_string()),
        source: EventSource::new("system", GOAL_SOURCE_ID),
        event: event_name(kind).to_string(),
        seq: None,
        summary: note.chars().take(800).collect(),
        data_ref: None,
        terminal: false,
    };
    agent
        .sessions()
        .post_input(&goal.session.sid, &PostedInput::event(who, key, ev))
        .await
}

/// A message handed over to the Goal (from a UI, the owner): posted as a
/// message of the agent itself, its origin in `delivery.from_name`.
pub async fn post_feedback(
    agent: &dyn AgentStateClient,
    goal: &GoalDefinition,
    who: &str,
    text: &str,
    from: &str,
) -> Result<(u64, String)> {
    if text.trim().is_empty() {
        return Err(OpenDanError::InvalidArgument("feedback text is empty".into()));
    }
    let me = parse_did(agent.agent_did())?;
    let msg = text_msg(&me, &me, text.to_string());
    let input = PostedInput::msg(
        who,
        msg,
        MsgDelivery {
            from_name: Some(format!("feedback from {from}")),
            conversation_name: Some(format!("goal {}", goal.goal_id)),
            ..Default::default()
        },
    )?;
    let index = agent
        .sessions()
        .post_input(&goal.session.sid, &input)
        .await?;
    Ok((index, input.key))
}

/// The objective of every Goal Session: a pointer, never a copy of the
/// definition (which is shown at the start of each Activation).
pub fn session_objective(goal_id: &str) -> String {
    format!(
        "Pursue goal {goal_id}. Its current definition, state, open work and budget are given at the start of every activation (<goal_activation>); each activation observes, judges and acts within its budget, then ends with a final report."
    )
}

/// Create the Goal Session of epoch `epoch` (idempotent: an existing one is
/// returned). `false` in the result: it existed already.
#[allow(clippy::too_many_arguments)]
pub async fn ensure_goal_session(
    agent: &dyn AgentStateClient,
    goal: &GoalDefinition,
    epoch: u64,
    who: &str,
    channels: &dyn InputChannelFactory,
    sessions_dir: &Path,
    llm_context: &Value,
    class: &str,
) -> Result<(SessionDir, bool)> {
    let e = goal.epoch(epoch).ok_or_else(|| {
        OpenDanError::NotFound(format!("epoch {epoch} of goal {}", goal.goal_id))
    })?;
    if let Some(existing) = agent.sessions().lookup(&e.sid).await? {
        return Ok((SessionDir::open(&existing.location)?, false));
    }
    let template = SessionTemplate::load(class, agent.agent_root())?;
    if template.kind != SessionKind::Goal {
        return Err(OpenDanError::InvalidArgument(format!(
            "session class `{class}` is a {} session; goal sessions need a class based on `goal`",
            template.kind.as_str()
        )));
    }
    let mut spec: SessionSpec = template.spec(session_objective(&goal.goal_id));
    spec.session_id = Some(e.sid.clone());
    spec.idempotency_key = Some(format!("goal:{}:{epoch}", goal.goal_id));
    spec.via = "opendan".into();
    spec.driver = Some(who.to_string());
    spec.goal = Some(GoalBinding {
        goal_id: goal.goal_id.clone(),
        epoch,
    });
    if !goal.scope.objects.is_empty() || !goal.scope.paths.is_empty() {
        spec.scope = Some(Scope {
            objects: goal.scope.objects.clone(),
            paths: goal.scope.paths.clone(),
        });
    }
    spec.prompt.llm_context = llm_context.clone();
    if let Some(b) = &goal.behavior {
        spec.prompt.behavior = Some(b.clone());
    }
    spec.workspace = goal.scope.workspace.as_ref().map(|w| WorkspaceRef {
        workspace_id: w.clone(),
        access: WorkspaceAccess::ReadWrite,
    });
    spec.freeze = spec.prompt.behavior.is_some();
    std::fs::create_dir_all(sessions_dir).map_err(|err| OpenDanError::io(sessions_dir, err))?;
    let sd = create_session(sessions_dir, spec, agent, who, channels).await?;
    Ok((sd, true))
}

/// May the Goal Session `pcfg` create a Work now? The Goal's period budget
/// and the epoch decide; the Work is told the Goal, the revision and the
/// Activation it comes from.
pub(crate) async fn work_admission(
    agent: &dyn AgentStateClient,
    pcfg: &SessionConfig,
    parent: &SessionDir,
    binding: &GoalBinding,
) -> Result<GoalOrigin> {
    let goal = agent
        .goals()
        .get(&binding.goal_id)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("goal {}", binding.goal_id)))?;
    if goal.session.epoch != binding.epoch {
        return Err(OpenDanError::GoalConflict(format!(
            "goal {} moved on to epoch {}; epoch {} creates no more work",
            goal.goal_id, goal.session.epoch, binding.epoch
        )));
    }
    if !goal.status.is_live() {
        return Err(OpenDanError::GoalConflict(format!(
            "goal {} is {}",
            goal.goal_id,
            goal.status.as_str()
        )));
    }
    let d = defaults(agent.agent_root(), &pcfg.session.class);
    let budget = effective_budget(&goal, &d);
    let window = window_secs(&budget);
    let now = crate::now_ms();
    let start = period_start(now, window);
    if let Some(max) = budget.period.max_works {
        let created = agent
            .sessions()
            .query(&RegistryQuery {
                goal_id: Some(goal.goal_id.clone()),
                ..Default::default()
            })
            .await?
            .into_iter()
            .filter(|e| {
                e.kind != SessionKind::Goal
                    && e.origin
                        .as_ref()
                        .and_then(|o| o.goal.as_ref())
                        .is_some_and(|g| g.at_ms >= start)
            })
            .count();
        if created >= max as usize {
            return Err(OpenDanError::GoalBudgetExhausted(format!(
                "goal {} created {created} of {max} works allowed in this period",
                goal.goal_id
            )));
        }
    }
    let state = parent.state()?;
    let shadow = state.goal.as_ref();
    let usage = current_usage(shadow.map(|g| &g.state), now, window);
    if let Some(why) = period_exhausted(&budget, &usage) {
        return Err(OpenDanError::GoalBudgetExhausted(format!(
            "goal {}: {why}",
            goal.goal_id
        )));
    }
    let open = shadow.and_then(|g| g.activation.as_ref());
    Ok(GoalOrigin {
        goal_id: goal.goal_id.clone(),
        revision: open.map(|a| a.goal_revision).unwrap_or(goal.revision),
        epoch: binding.epoch,
        activation_seq: open.map(|a| a.seq).unwrap_or(state.turn_seq),
        at_ms: now,
    })
}
