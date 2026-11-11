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

pub use crate::template::AgentBudget;

/// Session class of Goal Sessions (`agent.toml [session.goal]`).
pub const GOAL_CLASS: &str = "goal";

/// `[session.<class>]` defaults of a Goal (built-in when the class has
/// none).
pub fn defaults(agent_root: Option<&Path>, class: &str) -> GoalDefaults {
    let mut d = SessionTemplate::load(class, agent_root)
        .ok()
        .and_then(|t| t.goal)
        .unwrap_or_default();
    // The owner's switch of the crystallization Goal (Skills page) wins.
    if let Some(on) = agent_root.and_then(|r| crate::skills::load_state(r).ok()).and_then(|s| s.crystallize) {
        d.crystallize.enabled = on;
    }
    d
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

/// Move `usage` into the window of `now_ms` (a new window starts at zero;
/// the counters of the same window keep the window's start).
pub fn roll_usage(usage: &mut GoalUsage, now_ms: u64, window_secs: u64) {
    let start = period_start(now_ms, window_secs);
    if usage.period_start_ms < start {
        *usage = GoalUsage {
            period_start_ms: start,
            ..Default::default()
        };
    } else {
        usage.period_start_ms = start;
    }
}

/// The counters of the current window the Goal State keeps (Activations,
/// Work created; zero when the stored window is older). Tokens come from
/// [`period_usage`].
pub fn current_usage(state: Option<&GoalState>, now_ms: u64, window_secs: u64) -> GoalUsage {
    let mut u = state.map(|s| s.usage.clone()).unwrap_or_default();
    roll_usage(&mut u, now_ms, window_secs);
    u
}

/// Tokens of the rounds a session ran since `since_ms` (`usage.jsonl`).
fn session_tokens_since(e: &RegistryEntry, since_ms: u64) -> u64 {
    if e.status.run_state == RunState::Finished && e.status.updated_at_ms < since_ms {
        return 0;
    }
    let Ok(dir) = SessionDir::open(&e.location) else {
        return 0;
    };
    dir.usage()
        .map(|u| u.iter().filter(|r| r.at_ms >= since_ms).map(|r| r.total_tokens).sum())
        .unwrap_or(0)
}

/// Tokens a Goal used since `since_ms`: its Goal Sessions (every epoch) and
/// its Work with every sub session below it, finished or running.
pub async fn goal_tokens_since(agent: &dyn AgentStateClient, goal_id: &str, since_ms: u64) -> Result<(u64, u64)> {
    let entries = agent
        .sessions()
        .query(&RegistryQuery {
            goal_id: Some(goal_id.to_string()),
            ..Default::default()
        })
        .await?;
    let (mut own, mut work) = (0, 0);
    let mut below: Vec<String> = Vec::new();
    for e in &entries {
        let t = session_tokens_since(e, since_ms);
        if e.kind == SessionKind::Goal {
            own += t;
        } else {
            work += t;
            below.push(e.session_id.clone());
        }
    }
    let mut seen: std::collections::BTreeSet<String> = entries.iter().map(|e| e.session_id.clone()).collect();
    while !below.is_empty() {
        let children = agent.sessions().children_of(&below).await?;
        below.clear();
        for c in children {
            if seen.insert(c.session_id.clone()) {
                work += session_tokens_since(&c, since_ms);
                below.push(c.session_id.clone());
            }
        }
    }
    Ok((own, work))
}

/// Usage of the Goal's current window: the counters of its Goal State, the
/// tokens of every session of the Goal counted by the time they ran.
pub async fn period_usage(
    agent: &dyn AgentStateClient,
    goal_id: &str,
    state: Option<&GoalState>,
    now_ms: u64,
    window_secs: u64,
) -> Result<GoalUsage> {
    let mut u = current_usage(state, now_ms, window_secs);
    let (own, work) = goal_tokens_since(agent, goal_id, u.period_start_ms).await?;
    u.tokens = own;
    u.work_tokens = work;
    Ok(u)
}

/// The agent's background total in its current window.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AgentUsage {
    pub period_start_ms: u64,
    pub resets_at_ms: u64,
    pub used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
}

impl AgentUsage {
    pub fn exhausted(&self) -> bool {
        self.max.is_some_and(|m| self.used >= m)
    }
}

/// Tokens every Goal of the agent (and their Work) used in the agent's
/// current window.
pub async fn agent_usage(agent: &dyn AgentStateClient, d: &GoalDefaults, now_ms: u64) -> Result<AgentUsage> {
    let window = d.agent_budget.window_secs.max(1);
    let start = period_start(now_ms, window);
    let mut used = 0;
    if d.agent_budget.max_total_tokens.is_some() {
        for g in agent.goals().list().await? {
            let (own, work) = goal_tokens_since(agent, &g.goal_id, start).await?;
            used += own + work;
        }
    }
    Ok(AgentUsage {
        period_start_ms: start,
        resets_at_ms: start + window * 1000,
        used,
        max: d.agent_budget.max_total_tokens,
    })
}

/// Why the period budget refuses more, if it does.
pub fn period_exhausted(b: &GoalBudget, u: &GoalUsage) -> Option<String> {
    if let Some(max) = b.period.max_total_tokens {
        if u.total_tokens() >= max {
            return Some(format!("{} of {max} tokens of the period used", u.total_tokens()));
        }
    }
    if let Some(max) = b.period.max_activations {
        if u.activations >= u64::from(max) {
            return Some(format!("{} of {max} activations of the period used", u.activations));
        }
    }
    None
}

/// Hard interval of a Goal (seconds).
pub fn min_interval_secs(goal: &GoalDefinition, d: &GoalDefaults) -> u64 {
    goal.schedule.min_interval_secs.unwrap_or(d.min_interval_secs)
}

/// Hard guarantee of a Goal (`None`: off).
pub fn guarantee_secs(goal: &GoalDefinition, d: &GoalDefaults) -> Option<u64> {
    Some(goal.schedule.guarantee_secs.unwrap_or(d.guarantee_secs)).filter(|g| *g > 0)
}

/// Check interval of a Goal without a cron timetable: its own, else the
/// default when it has a check script (`None`: no scheduled checks).
pub fn check_every_secs(goal: &GoalDefinition, d: &GoalDefaults) -> Option<u64> {
    if goal.schedule.cron.is_some() {
        return None;
    }
    goal.schedule
        .every_secs
        .filter(|e| *e > 0)
        .or_else(|| goal.schedule.check.as_ref().map(|_| d.check_every_secs))
}

/// When the Goal inferred last (the end of its last Activation).
pub fn last_inference_ms(state: Option<&GoalState>) -> Option<u64> {
    state.and_then(|s| s.last_activation.as_ref()).map(|l| l.at_ms)
}

/// What the hard guarantee counts from: the last inference, else the start
/// of the current epoch.
pub fn guarantee_since_ms(goal: &GoalDefinition, state: Option<&GoalState>) -> u64 {
    last_inference_ms(state)
        .or_else(|| goal.current_epoch().map(|e| e.started_at_ms))
        .unwrap_or(goal.created_at_ms)
}

/// The gate a run opportunity meets before it becomes an inference
/// (architecture §12.1.1; the order is the TODO's H2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// Infer (`guarantee`: the hard guarantee let it through, past the
    /// budget and the check).
    Pass { guarantee: bool },
    /// Not now: the Goal sleeps or ended.
    Dormant,
    /// Not now; lifts by itself at `until_ms` (interval end, period reset).
    Held { reason: HoldReason, until_ms: Option<u64>, why: String },
}

/// What the gate needs to know.
pub struct GateInput<'a> {
    pub goal: &'a GoalDefinition,
    pub state: Option<&'a GoalState>,
    pub defaults: &'a GoalDefaults,
    pub budget: &'a GoalBudget,
    pub usage: &'a GoalUsage,
    pub agent: &'a AgentUsage,
    pub now_ms: u64,
}

/// Steps 1–4 of the gate: dormant, hard interval, hard guarantee, budgets.
/// The check (step 5) is the host's, for its timetable only.
pub fn gate(i: &GateInput<'_>) -> GateVerdict {
    if i.goal.status != GoalStatus::Active {
        return GateVerdict::Dormant;
    }
    if let Some(last) = last_inference_ms(i.state) {
        let min = min_interval_secs(i.goal, i.defaults) * 1000;
        if i.now_ms.saturating_sub(last) < min {
            return GateVerdict::Held {
                reason: HoldReason::MinInterval,
                until_ms: Some(last + min),
                why: format!("closer than the hard interval ({}s) to the last inference", min / 1000),
            };
        }
    }
    if guarantee_due(i.goal, i.state, i.defaults, i.now_ms) {
        return GateVerdict::Pass { guarantee: true };
    }
    if let Some(why) = period_exhausted(i.budget, i.usage) {
        return GateVerdict::Held {
            reason: HoldReason::PeriodBudget,
            until_ms: Some(i.usage.period_start_ms + window_secs(i.budget) * 1000),
            why,
        };
    }
    if i.agent.exhausted() {
        return GateVerdict::Held {
            reason: HoldReason::AgentBudget,
            until_ms: Some(i.agent.resets_at_ms),
            why: format!(
                "{} of the agent's {} background tokens of the period used",
                i.agent.used,
                i.agent.max.unwrap_or(0)
            ),
        };
    }
    GateVerdict::Pass { guarantee: false }
}

/// The hard guarantee is due: too long without an inference.
pub fn guarantee_due(goal: &GoalDefinition, state: Option<&GoalState>, d: &GoalDefaults, now_ms: u64) -> bool {
    guarantee_secs(goal, d).is_some_and(|g| now_ms.saturating_sub(guarantee_since_ms(goal, state)) >= g * 1000)
}

/// Everything the gate needs, read now.
pub async fn gate_now(
    agent: &dyn AgentStateClient,
    goal: &GoalDefinition,
    state: Option<&GoalState>,
    d: &GoalDefaults,
    now_ms: u64,
) -> Result<(GateVerdict, GoalUsage, AgentUsage)> {
    let budget = effective_budget(goal, d);
    let usage = period_usage(agent, &goal.goal_id, state, now_ms, window_secs(&budget)).await?;
    let agent_use = agent_usage(agent, d, now_ms).await?;
    let v = gate(&GateInput {
        goal,
        state,
        defaults: d,
        budget: &budget,
        usage: &usage,
        agent: &agent_use,
        now_ms,
    });
    Ok((v, usage, agent_use))
}

/// Input key of the event that starts epoch `epoch`.
pub fn start_key(goal_id: &str, epoch: u64) -> String {
    format!("goal:{goal_id}:start:{epoch}")
}

/// Input key of a scheduled fire (`tag`: the timetable's, `missed`: slots
/// folded into it).
pub fn scheduled_key(goal_id: &str, tag: &str, slot: u64, missed: u64) -> String {
    format!("goal:{goal_id}:scheduled:{tag}:{slot}:{missed}")
}

pub fn activate_key(goal_id: &str, request_id: &str) -> String {
    format!("goal:{goal_id}:activate:{request_id}")
}

/// Input key of the hard guarantee counted from `since_ms` (one per quiet
/// stretch: an inference moves `since_ms`).
pub fn guarantee_key(goal_id: &str, since_ms: u64) -> String {
    format!("goal:{goal_id}:guarantee:{since_ms}")
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
        ActivationReasonKind::Answer => "answer",
        ActivationReasonKind::Guarantee => "guarantee",
    }
}

/// Input key of the answer to a Goal question (`notify_id`: its
/// notification).
pub fn answer_key(goal_id: &str, notify_id: &str) -> String {
    format!("goal:{goal_id}:answer:{notify_id}")
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
                "answer" => ActivationReasonKind::Answer,
                "guarantee" => ActivationReasonKind::Guarantee,
                _ => ActivationReasonKind::Manual,
            };
            let mut r = base(kind);
            if kind == ActivationReasonKind::Answer {
                r.reference = input.key.rsplit_once(":answer:").map(|(_, id)| id.to_string());
            }
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
        call_id: None,
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
    let usage = period_usage(agent, &goal.goal_id, shadow.map(|g| &g.state), now, window).await?;
    if let Some(why) = period_exhausted(&budget, &usage) {
        return Err(OpenDanError::GoalBudgetExhausted(format!(
            "goal {}: {why}",
            goal.goal_id
        )));
    }
    let total = agent_usage(agent, &d, now).await?;
    if total.exhausted() {
        return Err(OpenDanError::GoalBudgetExhausted(format!(
            "goal {}: the agent's background total ({} of {} tokens) is used up",
            goal.goal_id,
            total.used,
            total.max.unwrap_or(0)
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

/// Where the text of a Goal's skill is on the AgentRoot: its `ref` when that
/// is a file of the AgentRoot (or an absolute path), else the installed goal
/// skill of that name (`goal_skills/<name>/SKILL.md`).
pub fn skill_path(agent_root: Option<&Path>, k: &GoalSkill) -> Option<std::path::PathBuf> {
    if let Some(r) = k.reference.as_deref().filter(|r| !r.contains("://")) {
        let p = Path::new(r);
        let p = if p.is_absolute() { p.to_path_buf() } else { agent_root?.join(p) };
        return p.is_file().then_some(p);
    }
    if k.reference.is_some() {
        return None;
    }
    let p = agent_root?.join(SkillKind::Goal.manual(&k.name));
    p.is_file().then_some(p)
}

/// The `ref` a skill named without one gets: the installed goal skill of
/// that name, if there is one.
pub async fn goal_skill_ref(agent: &dyn AgentStateClient, name: &str) -> Option<String> {
    let installed = agent.behaviors().skills(Some(SkillKind::Goal)).await.ok()?;
    installed
        .iter()
        .any(|m| m.name == name && m.enabled)
        .then(|| SkillKind::Goal.manual(name))
}

/// Reason of the pause a dormant Goal hands to the Work it started.
pub fn dormant_reason(goal_id: &str) -> String {
    format!("goal {goal_id} is dormant")
}

/// Reason of the pause a Goal out of budget hands to the Work it started.
pub fn budget_pause_reason(goal_id: &str) -> String {
    format!("goal {goal_id} used up its budget of the period")
}
