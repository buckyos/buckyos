//! Goals on the host (phase 1 of the Goal Session TODO): the Goal Session of
//! each Goal's current epoch is created by the host from the Goal list
//! (reconcile; creating a Goal only writes the Goal), epochs that ended are
//! recorded, and scheduled run opportunities come from a [`ScheduleBackend`]
//! through one admission chain. A Goal Session never wakes itself.
//!
//! Fires and starts are posted to the Goal Session's input bus with stable
//! keys (a re-post after a crash is consumed once); the progress the host
//! keeps is `state/goals/<id>/schedule.json`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::goal::{
    activate_key, current_usage, defaults, effective_budget, ensure_goal_session,
    period_exhausted, post_activation, post_feedback, scheduled_key, start_key, window_secs,
    GOAL_CLASS,
};
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::GoalStore;

use super::maintenance::{self, GateDecision, GateReason};
use super::Supervisor;

/// Milliseconds now (injectable for tests).
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        crate::now_ms()
    }
}

/// A clock a test moves by hand.
#[derive(Default)]
pub struct ManualClock(pub AtomicU64);

impl ManualClock {
    pub fn at(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }

    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A scheduled run opportunity that became due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fire {
    pub slot: u64,
    pub key: String,
    /// Slots that passed since the last fire (1 = on time).
    pub missed: u64,
    /// Admitted before (pending since a crash): post it again.
    pub retry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FireOutcome {
    Posted,
    Skipped(String),
}

/// Where scheduled fires come from. Phase 1: [`LocalScheduleBackend`];
/// phase 2 puts the workflow service's scheduled tasks behind the same
/// trait. Pausing, resuming and archiving a Goal are an [`ensure`] with its
/// new status.
///
/// [`ensure`]: ScheduleBackend::ensure
#[async_trait]
pub trait ScheduleBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Bring the schedule of `goal` in line with its definition and status
    /// (idempotent; called on every reconcile).
    async fn ensure(&self, goals: &dyn GoalStore, goal: &GoalDefinition, now_ms: u64) -> Result<()>;
    /// The fire due now, if any (not recorded yet).
    async fn due(&self, goals: &dyn GoalStore, goal: &GoalDefinition, now_ms: u64)
        -> Result<Option<Fire>>;
    /// Record that an admitted fire is about to be posted (written before
    /// the post: a crash re-posts the same key).
    async fn pending(&self, goals: &dyn GoalStore, goal_id: &str, fire: &Fire) -> Result<()>;
    async fn settle(
        &self,
        goals: &dyn GoalStore,
        goal_id: &str,
        fire: &Fire,
        outcome: &FireOutcome,
        now_ms: u64,
    ) -> Result<()>;
}

/// `every_secs` slots counted on the host's clock (`slot = now /
/// every_secs`); slots that passed while nothing could fire are folded into
/// one fire. Driven by the host's tick, kept in `schedule.json`.
pub struct LocalScheduleBackend;

const LOCAL: &str = "local";

async fn schedule_of(goals: &dyn GoalStore, goal_id: &str) -> Result<GoalSchedule> {
    Ok(goals
        .schedule(goal_id)
        .await?
        .unwrap_or_else(|| GoalSchedule::new(goal_id, LOCAL)))
}

#[async_trait]
impl ScheduleBackend for LocalScheduleBackend {
    fn name(&self) -> &'static str {
        LOCAL
    }

    async fn ensure(&self, goals: &dyn GoalStore, goal: &GoalDefinition, now_ms: u64) -> Result<()> {
        let mut s = schedule_of(goals, &goal.goal_id).await?;
        let every = goal.schedule.every_secs.filter(|e| *e > 0);
        if s.every_secs == every && (every.is_none() || s.last_slot.is_some()) {
            return Ok(());
        }
        // A new or changed interval counts from the current slot.
        s.backend = LOCAL.into();
        s.every_secs = every;
        s.last_slot = every.map(|e| now_ms / 1000 / e);
        s.pending = None;
        s.updated_at_ms = now_ms;
        goals.write_schedule(&s).await
    }

    async fn due(
        &self,
        goals: &dyn GoalStore,
        goal: &GoalDefinition,
        now_ms: u64,
    ) -> Result<Option<Fire>> {
        let s = schedule_of(goals, &goal.goal_id).await?;
        if let Some(p) = &s.pending {
            return Ok(Some(Fire {
                slot: p.slot,
                key: p.key.clone(),
                missed: p.missed,
                retry: true,
            }));
        }
        let (Some(every), Some(last)) = (s.every_secs, s.last_slot) else {
            return Ok(None);
        };
        let slot = now_ms / 1000 / every;
        if slot <= last {
            return Ok(None);
        }
        let missed = slot - last;
        Ok(Some(Fire {
            slot,
            key: scheduled_key(&goal.goal_id, slot, missed),
            missed,
            retry: false,
        }))
    }

    async fn pending(&self, goals: &dyn GoalStore, goal_id: &str, fire: &Fire) -> Result<()> {
        let mut s = schedule_of(goals, goal_id).await?;
        s.pending = Some(PendingFire {
            slot: fire.slot,
            key: fire.key.clone(),
            missed: fire.missed,
        });
        goals.write_schedule(&s).await
    }

    async fn settle(
        &self,
        goals: &dyn GoalStore,
        goal_id: &str,
        fire: &Fire,
        outcome: &FireOutcome,
        now_ms: u64,
    ) -> Result<()> {
        let mut s = schedule_of(goals, goal_id).await?;
        s.pending = None;
        s.last_slot = Some(s.last_slot.unwrap_or(0).max(fire.slot));
        match outcome {
            FireOutcome::Posted => {
                s.last_posted_key = Some(fire.key.clone());
                s.last_fire_at_ms = now_ms;
                s.last_error = None;
            }
            FireOutcome::Skipped(reason) => s.skip(fire.slot, now_ms, reason),
        }
        s.updated_at_ms = now_ms;
        goals.write_schedule(&s).await
    }
}

/// What an admission check sees.
pub struct AdmissionCtx<'a> {
    pub goal: &'a GoalDefinition,
    pub state: Option<&'a GoalState>,
    pub budget: &'a GoalBudget,
    pub defaults: &'a crate::template::GoalDefaults,
    pub now_ms: u64,
}

/// One link of the admission chain of scheduled fires (no wake-up, no
/// LLM). `Some(reason)`: the fire is skipped and recorded.
#[async_trait]
pub trait AdmissionCheck: Send + Sync {
    async fn check(&self, ctx: &AdmissionCtx<'_>) -> Option<String>;
}

/// The Goal is active.
pub struct ActiveCheck;

#[async_trait]
impl AdmissionCheck for ActiveCheck {
    async fn check(&self, ctx: &AdmissionCtx<'_>) -> Option<String> {
        match ctx.goal.status {
            GoalStatus::Active => None,
            GoalStatus::Paused => Some("paused".into()),
            _ => Some("not_active".into()),
        }
    }
}

/// Not closer than `min_interval_secs` to the last Activation.
pub struct MinIntervalCheck;

#[async_trait]
impl AdmissionCheck for MinIntervalCheck {
    async fn check(&self, ctx: &AdmissionCtx<'_>) -> Option<String> {
        let min = ctx
            .goal
            .schedule
            .min_interval_secs
            .unwrap_or(ctx.defaults.min_interval_secs);
        let last = ctx.state?.last_activation.as_ref()?.at_ms;
        (ctx.now_ms.saturating_sub(last) < min * 1000).then(|| "min_interval".to_string())
    }
}

/// The period budget is not used up.
pub struct PeriodBudgetCheck;

#[async_trait]
impl AdmissionCheck for PeriodBudgetCheck {
    async fn check(&self, ctx: &AdmissionCtx<'_>) -> Option<String> {
        let usage = current_usage(ctx.state, ctx.now_ms, window_secs(ctx.budget));
        period_exhausted(ctx.budget, &usage).map(|_| "period_budget".to_string())
    }
}

/// How the host runs Goals.
#[derive(Clone)]
pub struct GoalHostOptions {
    /// `.llm_context` of the Goal Sessions (the agent's base one).
    pub llm_context: Value,
    /// Where Goal Session directories are created (`<agent_root>/sessions`).
    pub sessions_dir: PathBuf,
    /// Session class of Goal Sessions.
    pub class: String,
    pub clock: Arc<dyn Clock>,
    pub backend: Arc<dyn ScheduleBackend>,
    /// How often the notifications of a Goal's open questions are looked
    /// at ([`GoalHost::recheck_answers`] looks at once).
    pub answers_every: std::time::Duration,
}

impl GoalHostOptions {
    pub fn new(sessions_dir: PathBuf, llm_context: Value) -> Self {
        Self {
            llm_context,
            sessions_dir,
            class: GOAL_CLASS.to_string(),
            clock: Arc::new(SystemClock),
            backend: Arc::new(LocalScheduleBackend),
            answers_every: std::time::Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalEventRecord {
    pub at_ms: u64,
    pub goal_id: String,
    pub what: String,
}

/// What `loader.status` shows of the goals module (memory only).
#[derive(Debug, Clone, Default, Serialize)]
pub struct GoalHostStatus {
    pub backend: String,
    pub goals: usize,
    pub active: usize,
    pub paused: usize,
    pub ticks: u64,
    pub last_tick_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fire: Option<GoalEventRecord>,
    pub skipped: Vec<GoalEventRecord>,
    pub errors: Vec<GoalEventRecord>,
}

const STATUS_KEEP: usize = 16;

pub struct GoalHost {
    pub options: GoalHostOptions,
    checks: Vec<Arc<dyn AdmissionCheck>>,
    status: Mutex<GoalHostStatus>,
    /// One reconcile at a time.
    busy: tokio::sync::Mutex<()>,
    /// Answer inputs posted by this process.
    answered: Mutex<std::collections::BTreeSet<String>>,
    /// When the questions of a Goal were looked at last.
    answers_checked: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// The built-in consolidation Goal was seeded by this process.
    seeded: AtomicBool,
}

fn push(list: &mut Vec<GoalEventRecord>, r: GoalEventRecord) {
    list.push(r);
    let extra = list.len().saturating_sub(STATUS_KEEP);
    list.drain(..extra);
}

impl GoalHost {
    pub fn new(options: GoalHostOptions) -> Self {
        Self {
            status: Mutex::new(GoalHostStatus {
                backend: options.backend.name().to_string(),
                ..Default::default()
            }),
            options,
            checks: vec![
                Arc::new(ActiveCheck),
                Arc::new(MinIntervalCheck),
                Arc::new(PeriodBudgetCheck),
            ],
            busy: tokio::sync::Mutex::new(()),
            answered: Mutex::new(std::collections::BTreeSet::new()),
            answers_checked: Mutex::new(std::collections::HashMap::new()),
            seeded: AtomicBool::new(false),
        }
    }

    /// Seed the agent's default consolidation Goal once per process when
    /// `[session.goal.maintenance] enabled` (idempotent across restarts by
    /// its key; a failure is retried at the next tick).
    async fn seed_builtin(&self, sup: &Supervisor) {
        if self.seeded.load(Ordering::SeqCst) {
            return;
        }
        let agent = sup.host.agent.as_ref();
        let d = defaults(agent.agent_root(), &self.options.class);
        if !d.maintenance.enabled {
            return;
        }
        match maintenance::seed(agent.goals(), agent.agent_did(), &d.maintenance).await {
            Ok(g) => {
                log::info!("goals: memory maintenance goal {} (policy {})", g.goal_id, g.opendan().map(|e| e.policy_version).unwrap_or(0));
                self.seeded.store(true, Ordering::SeqCst);
            }
            Err(e) => self.error(BUILTIN_MEMORY_MAINTENANCE, format!("seeding: {e}")),
        }
    }

    /// The gate of the built-in Goal: decides whether `reason` becomes a run
    /// opportunity, records its water marks in `schedule.json.gate`.
    async fn maintenance_gate(
        &self,
        sup: &Supervisor,
        goal: &GoalDefinition,
        reason: GateReason,
        now: u64,
    ) -> Result<GateDecision> {
        let agent = sup.host.agent.as_ref();
        let goals = agent.goals();
        let Some(root) = agent.agent_root() else {
            return Ok(GateDecision::Skip("no_agent_root".into()));
        };
        let state = goals.state(&goal.goal_id).await?;
        let d = defaults(Some(root), &self.options.class);
        let budget = effective_budget(goal, &d);
        let mut schedule = schedule_of(goals, &goal.goal_id).await?;
        let gate = schedule.gate.take().unwrap_or_default();
        let (decision, gate) = maintenance::decide(
            maintenance::memory_of(root),
            goal,
            state.as_ref(),
            &budget,
            gate,
            reason,
            now,
        )
        .await;
        // The schedule may have moved while the checks ran: re-read, set the gate.
        let mut schedule = schedule_of(goals, &goal.goal_id).await.unwrap_or(schedule);
        schedule.gate = Some(gate);
        schedule.updated_at_ms = now;
        goals.write_schedule(&schedule).await?;
        if let GateDecision::Skip(reason) = &decision {
            if reason == "mechanical" {
                // Recovery, sweep and cleanup need the drive, not a model.
                self.drive(sup, &goal.session.sid, "memory maintenance (mechanical)").await;
            }
            let mut s = self.status.lock().expect("goal status");
            push(
                &mut s.skipped,
                GoalEventRecord {
                    at_ms: now,
                    goal_id: goal.goal_id.clone(),
                    what: format!("gate: {reason}"),
                },
            );
        }
        Ok(decision)
    }

    /// A notification may have changed (a task event): every Goal's
    /// questions are looked at by the next tick.
    pub fn recheck_answers(&self) {
        self.answers_checked.lock().expect("answers checked").clear();
    }

    /// Another link at the end of the admission chain (phase 2: cheap
    /// gates such as Memory's pending work).
    pub fn with_check(mut self, check: Arc<dyn AdmissionCheck>) -> Self {
        self.checks.push(check);
        self
    }

    pub fn status(&self) -> GoalHostStatus {
        self.status.lock().expect("goal status").clone()
    }

    fn error(&self, goal_id: &str, what: String) {
        log::warn!("goals: {goal_id}: {what}");
        let mut s = self.status.lock().expect("goal status");
        push(
            &mut s.errors,
            GoalEventRecord {
                at_ms: crate::now_ms(),
                goal_id: goal_id.to_string(),
                what,
            },
        );
    }

    fn now(&self) -> u64 {
        self.options.clock.now_ms()
    }

    /// One look at every Goal.
    pub(super) async fn tick(&self, sup: &Supervisor) -> Result<()> {
        let _busy = self.busy.lock().await;
        self.seed_builtin(sup).await;
        let goals = sup.host.agent.goals().list().await?;
        {
            let mut s = self.status.lock().expect("goal status");
            s.goals = goals.len();
            s.active = goals.iter().filter(|g| g.status == GoalStatus::Active).count();
            s.paused = goals.iter().filter(|g| g.status == GoalStatus::Paused).count();
            s.ticks += 1;
            s.last_tick_ms = self.now();
        }
        for goal in goals {
            if let Err(e) = self.reconcile(sup, &goal).await {
                self.error(&goal.goal_id, e.to_string());
            }
        }
        Ok(())
    }

    /// Reconcile one Goal now (after a change made through the service).
    pub(super) async fn tick_one(&self, sup: &Supervisor, goal_id: &str) -> Result<()> {
        let _busy = self.busy.lock().await;
        let goal = sup
            .host
            .agent
            .goals()
            .get(goal_id)
            .await?
            .ok_or_else(|| OpenDanError::NotFound(format!("goal {goal_id}")))?;
        self.reconcile(sup, &goal).await
    }

    /// Questions of the Goal whose notification was answered (or closed)
    /// since: the answer goes to the Goal Session as an `answer` run
    /// opportunity (the key makes a repeat the same input). The session
    /// records the answer when that Activation closes.
    async fn collect_answers(&self, sup: &Supervisor, goal: &GoalDefinition) -> Result<()> {
        let Some(notify) = sup.host.notify.clone() else {
            return Ok(());
        };
        {
            let mut checked = self.answers_checked.lock().expect("answers checked");
            if checked
                .get(&goal.goal_id)
                .is_some_and(|t| t.elapsed() < self.options.answers_every)
            {
                return Ok(());
            }
            checked.insert(goal.goal_id.clone(), std::time::Instant::now());
        }
        let agent = sup.host.agent.as_ref();
        let Some(state) = agent.goals().state(&goal.goal_id).await? else {
            return Ok(());
        };
        for q in &state.questions {
            let (Some(id), None | Some(GoalQuestionStatus::Open)) = (&q.notify_id, q.status) else {
                continue;
            };
            let key = crate::goal::answer_key(&goal.goal_id, id);
            if self.answered.lock().expect("answered").contains(&key) {
                continue;
            }
            let view = notify.state(id).await?;
            if !view.status.is_closed() {
                continue;
            }
            let answer = view.result.as_ref().and_then(|r| {
                r.text().map(str::to_string).or_else(|| r.option().map(str::to_string))
            });
            let note = match (view.status, answer) {
                (crate::notify::NotifyStatus::Answered, Some(a)) => {
                    format!("the user answered the question \"{}\": {a}", q.text)
                }
                (status, _) => format!("the question \"{}\" was {}", q.text, status.as_str()),
            };
            post_activation(agent, goal, &sup.host.who, ActivationReasonKind::Answer, &key, &note).await?;
            self.answered.lock().expect("answered").insert(key);
            self.drive(sup, &goal.session.sid, "goal question answered").await;
        }
        Ok(())
    }

    /// The session of an epoch is finished and published everything.
    fn settled(entry: &RegistryEntry) -> Result<bool> {
        if entry.status.run_state != RunState::Finished {
            return Ok(false);
        }
        let state = SessionDir::open(&entry.location)?.state()?;
        Ok(!state.goal.as_ref().is_some_and(|g| g.needs_flush()))
    }

    async fn reconcile(&self, sup: &Supervisor, goal: &GoalDefinition) -> Result<()> {
        let agent = sup.host.agent.as_ref();
        let goals = agent.goals();
        let builtin = goal.is_builtin(BUILTIN_MEMORY_MAINTENANCE);
        if builtin && !defaults(agent.agent_root(), &self.options.class).maintenance.enabled {
            // Switched off: no session, no checks; the Goal and the
            // cognitions stay as they are.
            let mut schedule = schedule_of(goals, &goal.goal_id).await?;
            if schedule.gate.as_ref().is_none_or(|g| g.status != "disabled") {
                let mut gate = schedule.gate.take().unwrap_or_default();
                gate.status = "disabled".into();
                gate.last_skip_reason = Some("disabled".into());
                gate.last_checked_at_ms = self.now();
                schedule.gate = Some(gate);
                schedule.updated_at_ms = self.now();
                goals.write_schedule(&schedule).await?;
            }
            return Ok(());
        }
        // Epochs before the current one end first (their sessions end by
        // their gate; the state they leave is what the next one starts
        // from).
        let mut previous_open = false;
        for e in goal.epochs.iter().filter(|e| e.epoch < goal.session.epoch) {
            if e.ended_at_ms.is_some() {
                continue;
            }
            match agent.sessions().lookup(&e.sid).await? {
                Some(entry) if Self::settled(&entry)? => {
                    goals
                        .end_epoch(
                            &goal.goal_id,
                            e.epoch,
                            e.end.unwrap_or(EpochEnd::Restarted),
                            e.end_reason.clone(),
                        )
                        .await?;
                }
                Some(_) => {
                    previous_open = true;
                    self.drive(sup, &e.sid, "goal epoch ending").await;
                }
                None => {
                    goals
                        .end_epoch(&goal.goal_id, e.epoch, EpochEnd::Restarted, e.end_reason.clone())
                        .await?;
                }
            }
        }
        let epoch = goal.session.epoch;
        let sid = goal.session.sid.clone();
        match goal.status {
            GoalStatus::Archived | GoalStatus::Achieved => {
                let end = if goal.status == GoalStatus::Archived {
                    EpochEnd::Archived
                } else {
                    EpochEnd::Achieved
                };
                if goal.current_epoch().is_some_and(|e| e.ended_at_ms.is_some()) {
                    return Ok(());
                }
                match agent.sessions().lookup(&sid).await? {
                    Some(entry) if Self::settled(&entry)? => {
                        goals
                            .end_epoch(&goal.goal_id, epoch, end, goal.status_reason.clone())
                            .await?;
                    }
                    Some(_) => self.drive(sup, &sid, "goal ended").await,
                    None => {
                        goals
                            .end_epoch(&goal.goal_id, epoch, end, goal.status_reason.clone())
                            .await?;
                    }
                }
                return Ok(());
            }
            GoalStatus::Active | GoalStatus::Paused => {}
        }
        if previous_open {
            return Ok(());
        }
        // The current epoch's Goal Session (created by nobody else).
        let entry = match agent.sessions().lookup(&sid).await? {
            Some(e) => e,
            None => {
                goals.prepare_epoch(&goal.goal_id, epoch).await?;
                crate::fault::point("goal:before_session_create");
                let (sd, _) = ensure_goal_session(
                    agent,
                    goal,
                    epoch,
                    &sup.host.who,
                    sup.host.inputs.as_ref(),
                    &self.options.sessions_dir,
                    &self.options.llm_context,
                    &self.options.class,
                )
                .await?;
                log::info!("goals: {} epoch {epoch}: goal session {} created", goal.goal_id, sd.sid());
                agent
                    .sessions()
                    .lookup(&sid)
                    .await?
                    .ok_or_else(|| OpenDanError::Unregistered(sid.clone()))?
            }
        };
        if entry.status.run_state == RunState::Finished {
            return Err(OpenDanError::GoalConflict(format!(
                "the goal session {sid} of epoch {epoch} finished while the goal is {}; restart the goal for a new context",
                goal.status.as_str()
            )));
        }
        if let Err(e) = self.collect_answers(sup, goal).await {
            self.error(&goal.goal_id, format!("answers: {e}"));
        }
        let now = self.now();
        // The start of the epoch: one event, posted once.
        let mut schedule = goals
            .schedule(&goal.goal_id)
            .await?
            .unwrap_or_else(|| GoalSchedule::new(&goal.goal_id, self.options.backend.name()));
        if schedule.start_posted_epoch < epoch {
            let start = goal.current_epoch().map(|e| e.start).unwrap_or(EpochStart::Created);
            let kind = match start {
                EpochStart::Created => ActivationReasonKind::Created,
                EpochStart::Reopened => ActivationReasonKind::Reopened,
                EpochStart::Restarted => ActivationReasonKind::Restarted,
            };
            let start_note = match goal.current_epoch().and_then(|e| e.reason.clone()) {
                Some(r) => format!("epoch {epoch} started ({}): {r}", crate::goal::event_name(kind)),
                None => format!("goal {} was created", goal.goal_id),
            };
            let post = if builtin {
                // The built-in Goal starts with a check, never a model call
                // on an empty Memory.
                match self.maintenance_gate(sup, goal, GateReason::Start, now).await? {
                    GateDecision::Run(gate_note) => Some(format!("{start_note}; {gate_note}")),
                    GateDecision::Skip(_) => None,
                }
            } else if kind != ActivationReasonKind::Created || goal.schedule.start_on_create {
                Some(start_note)
            } else {
                None
            };
            if let Some(note) = post {
                post_activation(agent, goal, &sup.host.who, kind, &start_key(&goal.goal_id, epoch), &note)
                    .await?;
                self.drive(sup, &sid, "goal epoch started").await;
            }
            // The gate may have written its water marks meanwhile.
            schedule = schedule_of(goals, &goal.goal_id).await?;
            schedule.start_posted_epoch = epoch;
            schedule.updated_at_ms = now;
            goals.write_schedule(&schedule).await?;
        }
        let backend = self.options.backend.clone();
        backend.ensure(goals, goal, now).await?;
        let Some(fire) = backend.due(goals, goal, now).await? else {
            return Ok(());
        };
        let mut gate_note = None;
        if !fire.retry {
            let state = goals.state(&goal.goal_id).await?;
            let d = defaults(agent.agent_root(), &self.options.class);
            let budget = effective_budget(goal, &d);
            let ctx = AdmissionCtx {
                goal,
                state: state.as_ref(),
                budget: &budget,
                defaults: &d,
                now_ms: now,
            };
            for check in &self.checks {
                if let Some(reason) = check.check(&ctx).await {
                    backend
                        .settle(goals, &goal.goal_id, &fire, &FireOutcome::Skipped(reason.clone()), now)
                        .await?;
                    let mut s = self.status.lock().expect("goal status");
                    push(
                        &mut s.skipped,
                        GoalEventRecord {
                            at_ms: now,
                            goal_id: goal.goal_id.clone(),
                            what: format!("slot {} skipped: {reason}", fire.slot),
                        },
                    );
                    return Ok(());
                }
            }
            if builtin {
                match self.maintenance_gate(sup, goal, GateReason::Scheduled(fire.slot), now).await? {
                    GateDecision::Run(n) => gate_note = Some(n),
                    GateDecision::Skip(reason) => {
                        backend
                            .settle(goals, &goal.goal_id, &fire, &FireOutcome::Skipped(reason), now)
                            .await?;
                        return Ok(());
                    }
                }
            }
            backend.pending(goals, &goal.goal_id, &fire).await?;
            crate::fault::point("goal:after_fire_pending");
        }
        let mut note = if fire.missed > 1 {
            format!(
                "scheduled run (every {}s); {} slots passed since the last one",
                goal.schedule.every_secs.unwrap_or(0),
                fire.missed
            )
        } else {
            format!("scheduled run (every {}s)", goal.schedule.every_secs.unwrap_or(0))
        };
        if let Some(n) = gate_note {
            note = format!("{note}; {n}");
        }
        post_activation(agent, goal, &sup.host.who, ActivationReasonKind::Scheduled, &fire.key, &note)
            .await?;
        crate::fault::point("goal:after_fire_post");
        backend
            .settle(goals, &goal.goal_id, &fire, &FireOutcome::Posted, now)
            .await?;
        {
            let mut s = self.status.lock().expect("goal status");
            s.last_fire = Some(GoalEventRecord {
                at_ms: now,
                goal_id: goal.goal_id.clone(),
                what: fire.key.clone(),
            });
        }
        self.drive(sup, &sid, "goal scheduled").await;
        Ok(())
    }

    /// Make sure a serving loop advances `sid` (the host drives what it
    /// created; another driver's session is that driver's).
    async fn drive(&self, sup: &Supervisor, sid: &str, reason: &str) {
        match sup.ensure_task(sid, reason).await {
            Ok(()) | Err(OpenDanError::NotDriver { .. }) => {}
            Err(e) => log::warn!("goals: session {sid} not hosted: {e}"),
        }
    }
}

impl Supervisor {
    /// The goals module of this host (`None`: goals are not hosted).
    pub fn goals(&self) -> Option<&GoalHost> {
        self.goals.as_deref()
    }

    /// A Goal changed through this process: reconcile it now (its session,
    /// its epoch, its schedule) instead of at the next tick.
    pub async fn goal_changed(&self, goal_id: &str) -> Result<()> {
        match &self.goals {
            Some(g) => g.tick_one(self, goal_id).await,
            None => Ok(()),
        }
    }

    /// A manual run opportunity (never subject to admission).
    pub async fn goal_activate(&self, goal_id: &str, request_id: &str, note: &str) -> Result<u64> {
        let goal = self.live_goal(goal_id).await?;
        let key = activate_key(goal_id, request_id);
        let index = post_activation(
            self.host.agent.as_ref(),
            &goal,
            &self.host.who,
            ActivationReasonKind::Manual,
            &key,
            note,
        )
        .await?;
        self.ensure_task(&goal.session.sid, "goal activated").await?;
        Ok(index)
    }

    /// A message handed over to the Goal (`from`: where it comes from).
    pub async fn goal_feedback(&self, goal_id: &str, text: &str, from: &str) -> Result<(u64, String)> {
        let goal = self.live_goal(goal_id).await?;
        let r = post_feedback(self.host.agent.as_ref(), &goal, &self.host.who, text, from).await?;
        self.ensure_task(&goal.session.sid, "goal feedback").await?;
        Ok(r)
    }

    async fn live_goal(&self, goal_id: &str) -> Result<GoalDefinition> {
        let goal = self
            .host
            .agent
            .goals()
            .get(goal_id)
            .await?
            .ok_or_else(|| OpenDanError::NotFound(format!("goal {goal_id}")))?;
        if !goal.status.is_live() {
            return Err(OpenDanError::GoalConflict(format!(
                "goal {goal_id} is {}",
                goal.status.as_str()
            )));
        }
        if let Some(g) = &self.goals {
            // Its session exists before anything is posted to it.
            g.tick_one(self, goal_id).await?;
        }
        Ok(goal)
    }
}
