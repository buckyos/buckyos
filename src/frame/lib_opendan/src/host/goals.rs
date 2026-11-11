//! Goals on the host (phase 1 of the Goal Session TODO): the Goal Session of
//! each Goal's current epoch is created by the host from the Goal list
//! (reconcile; creating a Goal only writes the Goal), epochs that ended are
//! recorded, and run opportunities meet one gate before they become
//! inferences (architecture §12.1.1): the Goal sleeps → none; closer than
//! the hard interval to the last inference → none; past the hard guarantee →
//! one, whatever follows; the period budget or the agent's background total
//! used up → none; a scheduled check runs the Goal's check script, `true`
//! infers. A Goal Session never wakes itself.
//!
//! Fires and starts are posted to the Goal Session's input bus with stable
//! keys (a re-post after a crash is consumed once); the progress the host
//! keeps is `state/goals/<id>/schedule.json`. Events (a Work's end, a
//! message, an answer) go to the session directly; its runner holds them at
//! the same gate (steps 1–4).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::goal::{
    activate_key, defaults, effective_budget, ensure_goal_session, gate_now, guarantee_since_ms, post_activation,
    post_feedback, scheduled_key, start_key, window_secs, GateVerdict, GOAL_CLASS,
};
use crate::goal_check::{CheckRun, CheckRunner, DenoCheckRunner, MissingRunner};
use crate::goal_time::Timetable;
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::GoalStore;
use crate::template::GoalDefaults;

use super::maintenance;
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

/// The clock Goal decisions read (`RunnerOptions.goal_clock`: the runner's
/// gate of events, the times of its Activations).
#[derive(Clone)]
pub struct GoalClock(pub Arc<dyn Clock>);

impl Default for GoalClock {
    fn default() -> Self {
        Self(Arc::new(SystemClock))
    }
}

impl std::fmt::Debug for GoalClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GoalClock({})", self.0.now_ms())
    }
}

impl GoalClock {
    pub fn now_ms(&self) -> u64 {
        self.0.now_ms()
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

/// Where scheduled check slots come from. Phase 1: [`LocalScheduleBackend`];
/// phase 2 puts the workflow service's scheduled tasks behind the same
/// trait. Pausing, resuming and archiving a Goal are an [`ensure`] with its
/// new status.
///
/// [`ensure`]: ScheduleBackend::ensure
#[async_trait]
pub trait ScheduleBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Bring the schedule of `goal` in line with its timetable (idempotent;
    /// called on every reconcile).
    async fn ensure(
        &self,
        goals: &dyn GoalStore,
        goal: &GoalDefinition,
        timetable: Option<&Timetable>,
        now_ms: u64,
    ) -> Result<()>;
    /// The fire due now, if any (not recorded yet).
    async fn due(
        &self,
        goals: &dyn GoalStore,
        goal: &GoalDefinition,
        timetable: Option<&Timetable>,
        now_ms: u64,
    ) -> Result<Option<Fire>>;
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

/// Slots of the timetable counted on the host's clock (`every_secs`: `slot
/// = now / every_secs`; cron: the UTC minute of the latest matching wall
/// clock minute); slots that passed while nothing could fire are folded into
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

    async fn ensure(
        &self,
        goals: &dyn GoalStore,
        goal: &GoalDefinition,
        timetable: Option<&Timetable>,
        now_ms: u64,
    ) -> Result<()> {
        let mut s = schedule_of(goals, &goal.goal_id).await?;
        let key = timetable.map(Timetable::key);
        if s.timetable == key && (key.is_none() || s.last_slot.is_some()) {
            return Ok(());
        }
        // A new or changed timetable counts from the current slot.
        s.backend = LOCAL.into();
        s.timetable = key;
        s.last_slot = timetable.and_then(|t| t.slot(now_ms));
        s.pending = None;
        s.updated_at_ms = now_ms;
        goals.write_schedule(&s).await
    }

    async fn due(
        &self,
        goals: &dyn GoalStore,
        goal: &GoalDefinition,
        timetable: Option<&Timetable>,
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
        let Some(t) = timetable else {
            return Ok(None);
        };
        let (Some(slot), Some(last)) = (t.slot(now_ms), s.last_slot) else {
            return Ok(None);
        };
        if slot <= last {
            return Ok(None);
        }
        let missed = t.missed(last, slot);
        Ok(Some(Fire {
            slot,
            key: scheduled_key(&goal.goal_id, &t.tag(), slot, missed),
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

/// How long after a Goal woke the host still looks for Work its sleep
/// paused (a resume that could not be posted at once is retried meanwhile).
const WAKE_WINDOW_MS: u64 = 600_000;

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
    /// Where check scripts run (Deno with read and network only).
    pub checks: Arc<dyn CheckRunner>,
    /// How often the budget of a Goal with running Work is read (its Work is
    /// paused when it runs out, resumed when the period resets).
    pub budget_every: std::time::Duration,
}

impl GoalHostOptions {
    pub fn new(sessions_dir: PathBuf, llm_context: Value) -> Self {
        let work_dir = sessions_dir
            .parent()
            .map(|p| p.join(".cache").join("goal_checks"))
            .unwrap_or_else(|| std::env::temp_dir().join("opendan-goal-checks"));
        let checks: Arc<dyn CheckRunner> = match DenoCheckRunner::discover(work_dir) {
            Some(d) => Arc::new(d),
            None => Arc::new(MissingRunner),
        };
        Self {
            llm_context,
            sessions_dir,
            class: GOAL_CLASS.to_string(),
            clock: Arc::new(SystemClock),
            backend: Arc::new(LocalScheduleBackend),
            answers_every: std::time::Duration::from_secs(30),
            checks,
            budget_every: std::time::Duration::from_secs(30),
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
    status: Mutex<GoalHostStatus>,
    /// One reconcile at a time.
    busy: tokio::sync::Mutex<()>,
    /// Answer inputs posted by this process.
    answered: Mutex<std::collections::BTreeSet<String>>,
    /// When the questions of a Goal were looked at last.
    answers_checked: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// The built-in consolidation Goal was seeded by this process.
    seeded: AtomicBool,
    /// The skill crystallization Goal was.
    seeded_crystallize: AtomicBool,
    /// When the budget of a Goal was read last (pausing its Work).
    budget_checked: Mutex<std::collections::HashMap<String, std::time::Instant>>,
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
            busy: tokio::sync::Mutex::new(()),
            answered: Mutex::new(std::collections::BTreeSet::new()),
            answers_checked: Mutex::new(std::collections::HashMap::new()),
            seeded: AtomicBool::new(false),
            seeded_crystallize: AtomicBool::new(false),
            budget_checked: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Seed the agent's built-in Goals once per process when they are
    /// switched on in `agent.toml` (idempotent across restarts by their
    /// keys; a failure is retried at the next tick). Seeding also gives them
    /// the package's check script while they still run the package's.
    async fn seed_builtin(&self, sup: &Supervisor) {
        let agent = sup.host.agent.as_ref();
        let d = defaults(agent.agent_root(), &self.options.class);
        if d.crystallize.enabled && !self.seeded_crystallize.load(Ordering::SeqCst) {
            match maintenance::seed_crystallize(agent.goals(), agent.agent_did(), agent.agent_root(), d.crystallize.behavior.clone()).await {
                Ok(g) => {
                    log::info!("goals: skill crystallization goal {}", g.goal_id);
                    self.seeded_crystallize.store(true, Ordering::SeqCst);
                }
                Err(e) => self.error(BUILTIN_SKILL_CRYSTALLIZE, format!("seeding: {e}")),
            }
        }
        if self.seeded.load(Ordering::SeqCst) || !d.maintenance.enabled {
            return;
        }
        match maintenance::seed(agent.goals(), agent.agent_did(), agent.agent_root(), &d.maintenance).await {
            Ok(g) => {
                log::info!("goals: memory maintenance goal {} (policy {})", g.goal_id, g.opendan().map(|e| e.policy_version).unwrap_or(0));
                self.seeded.store(true, Ordering::SeqCst);
            }
            Err(e) => self.error(BUILTIN_MEMORY_MAINTENANCE, format!("seeding: {e}")),
        }
    }

    fn skipped(&self, goal_id: &str, what: String, now: u64) {
        let mut s = self.status.lock().expect("goal status");
        push(
            &mut s.skipped,
            GoalEventRecord {
                at_ms: now,
                goal_id: goal_id.to_string(),
                what,
            },
        );
    }

    /// Run the Goal's check script once and record what it said in
    /// `schedule.json` (last check, consecutive errors, its `state`, the
    /// period's counters).
    async fn check(&self, sup: &Supervisor, goal: &GoalDefinition, trigger: &str, now: u64) -> Result<CheckRun> {
        let agent = sup.host.agent.as_ref();
        let run = crate::goal_check::run_for(agent, self.options.checks.as_ref(), goal, None, trigger, now).await?;
        let d = defaults(agent.agent_root(), &self.options.class);
        let start = crate::goal::period_start(now, window_secs(&effective_budget(goal, &d)));
        let goals = agent.goals();
        let mut schedule = schedule_of(goals, &goal.goal_id).await?;
        let outcome = run.outcome();
        let reason = match &run.verdict {
            Ok(v) => v.reason.clone(),
            Err(e) => Some(e.clone()),
        };
        schedule.last_check = Some(CheckRecord {
            at_ms: now,
            outcome,
            reason: reason.map(|r| r.chars().take(2000).collect()),
            script_version: goal.schedule.check.as_ref().map(|c| c.version).unwrap_or(0),
            elapsed_ms: run.elapsed_ms,
        });
        let stats = schedule.stats_mut(start);
        match outcome {
            CheckOutcome::True => stats.checks_true += 1,
            CheckOutcome::False => stats.checks_false += 1,
            CheckOutcome::Error => stats.checks_error += 1,
        }
        match &run.verdict {
            Ok(v) => {
                schedule.check_errors = 0;
                if let Some(st) = &v.state {
                    schedule.check_state = Some(st.clone());
                }
            }
            Err(e) => {
                schedule.check_errors = schedule.check_errors.saturating_add(1);
                schedule.last_check_error = Some(e.chars().take(2000).collect());
                log::info!("goals: {}: check failed ({} in a row): {e}", goal.goal_id, schedule.check_errors);
            }
        }
        schedule.updated_at_ms = now;
        goals.write_schedule(&schedule).await?;
        Ok(run)
    }

    /// Count a scheduled run opportunity the gate refused.
    async fn count_skip(&self, goals: &dyn GoalStore, goal: &GoalDefinition, d: &GoalDefaults, reason: HoldReason, now: u64) -> Result<()> {
        let start = crate::goal::period_start(now, window_secs(&effective_budget(goal, d)));
        let mut schedule = schedule_of(goals, &goal.goal_id).await?;
        let stats = schedule.stats_mut(start);
        match reason {
            HoldReason::MinInterval => stats.skipped_min_interval += 1,
            HoldReason::PeriodBudget | HoldReason::AgentBudget => stats.skipped_budget += 1,
        }
        schedule.updated_at_ms = now;
        goals.write_schedule(&schedule).await
    }

    /// A notification may have changed (a task event): every Goal's
    /// questions are looked at by the next tick.
    pub fn recheck_answers(&self) {
        self.answers_checked.lock().expect("answers checked").clear();
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

    /// Post a pause or a resume to the running Work of a Goal (not to its
    /// Goal Session, not to finished Work).
    async fn tell_works(&self, sup: &Supervisor, goal: &GoalDefinition, pick: impl Fn(Option<PauseSource>) -> Option<ControlCommand>, key_tag: &str) {
        let agent = sup.host.agent.as_ref();
        let query = RegistryQuery { goal_id: Some(goal.goal_id.clone()), ..Default::default() };
        let Ok(works) = agent.sessions().query(&query).await else {
            return;
        };
        for e in works {
            if e.kind == SessionKind::Goal || e.status.run_state == RunState::Finished || e.input_queue.is_none() {
                continue;
            }
            let Some(command) = pick(e.status.paused) else {
                continue;
            };
            let key = format!("{}:goal:{}:{key_tag}", command.name(), goal.goal_id);
            let input = PostedInput::control(&sup.host.who, key, command);
            match agent.sessions().post_input(&e.session_id, &input).await {
                Ok(_) => self.drive(sup, &e.session_id, "goal work control").await,
                Err(err) => log::warn!("goals: {}: work {} not told: {err}", goal.goal_id, e.session_id),
            }
        }
    }

    /// A dormant Goal infers nothing: the Work it started is paused with it
    /// and resumed when it wakes (only what its sleep paused).
    async fn sync_dormant_work(&self, sup: &Supervisor, goal: &GoalDefinition) {
        let dormant = goal.status == GoalStatus::Paused;
        // Awake for a while: what its sleep paused was resumed long ago.
        if !dormant && crate::now_ms().saturating_sub(goal.status_changed_at_ms) > WAKE_WINDOW_MS {
            return;
        }
        let reason = crate::goal::dormant_reason(&goal.goal_id);
        let woke = format!("goal {} woke", goal.goal_id);
        let tag = goal.status_changed_at_ms.to_string();
        self.tell_works(
            sup,
            goal,
            |paused| match (dormant, paused) {
                (true, None) => Some(ControlCommand::Pause { reason: Some(reason.clone()) }),
                (false, Some(PauseSource::Goal)) => Some(ControlCommand::Resume { reason: Some(woke.clone()) }),
                _ => None,
            },
            &tag,
        )
        .await;
    }

    /// A Goal whose period budget (or the agent's background total) is used
    /// up infers no more, nor does its Work: running Work is paused at its
    /// next inference boundary and resumed when the period resets (only what
    /// the budget paused). Read at most every `budget_every` per Goal.
    async fn sync_budget_work(&self, sup: &Supervisor, goal: &GoalDefinition, d: &GoalDefaults, state: Option<&GoalState>, now: u64) {
        if goal.status != GoalStatus::Active {
            return;
        }
        {
            let mut checked = self.budget_checked.lock().expect("budget checked");
            if checked
                .get(&goal.goal_id)
                .is_some_and(|t| t.elapsed() < self.options.budget_every)
            {
                return;
            }
            checked.insert(goal.goal_id.clone(), std::time::Instant::now());
        }
        let agent = sup.host.agent.as_ref();
        let works_running = state.is_some_and(|s| !s.active_works.is_empty())
            || agent
                .sessions()
                .query(&RegistryQuery { goal_id: Some(goal.goal_id.clone()), ..Default::default() })
                .await
                .map(|w| w.iter().any(|e| e.kind != SessionKind::Goal && e.status.run_state != RunState::Finished))
                .unwrap_or(false);
        if !works_running {
            return;
        }
        let budget = effective_budget(goal, d);
        let usage = match crate::goal::period_usage(agent, &goal.goal_id, state, now, window_secs(&budget)).await {
            Ok(u) => u,
            Err(e) => {
                self.error(&goal.goal_id, format!("budget: {e}"));
                return;
            }
        };
        let total = crate::goal::agent_usage(agent, d, now).await.unwrap_or_default();
        let out = crate::goal::period_exhausted(&budget, &usage).is_some() || total.exhausted();
        let reason = crate::goal::budget_pause_reason(&goal.goal_id);
        let tag = if out { format!("out:{}", usage.period_start_ms) } else { format!("in:{}", usage.period_start_ms) };
        self.tell_works(
            sup,
            goal,
            |paused| match (out, paused) {
                (true, None) => Some(ControlCommand::Pause { reason: Some(reason.clone()) }),
                (false, Some(PauseSource::Budget)) => {
                    Some(ControlCommand::Resume { reason: Some("a new budget period started".into()) })
                }
                _ => None,
            },
            &tag,
        )
        .await;
    }

    fn settled(entry: &RegistryEntry) -> Result<bool> {
        if entry.status.run_state != RunState::Finished {
            return Ok(false);
        }
        let state = SessionDir::open(&entry.location)?.state()?;
        Ok(!state.goal.as_ref().is_some_and(|g| g.needs_flush()))
    }

    /// A built-in Goal switched off in `agent.toml`.
    fn switched_off(&self, sup: &Supervisor, goal: &GoalDefinition) -> bool {
        let d = defaults(sup.host.agent.agent_root(), &self.options.class);
        (goal.is_builtin(BUILTIN_MEMORY_MAINTENANCE) && !d.maintenance.enabled)
            || (goal.is_builtin(BUILTIN_SKILL_CRYSTALLIZE) && !d.crystallize.enabled)
    }

    async fn reconcile(&self, sup: &Supervisor, goal: &GoalDefinition) -> Result<()> {
        let agent = sup.host.agent.as_ref();
        let goals = agent.goals();
        let off = self.switched_off(sup, goal);
        {
            let mut schedule = schedule_of(goals, &goal.goal_id).await?;
            if schedule.switched_off != off {
                schedule.switched_off = off;
                schedule.updated_at_ms = self.now();
                goals.write_schedule(&schedule).await?;
            }
        }
        if off {
            // Switched off: no session, no checks; the Goal and the
            // cognitions stay as they are.
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
        self.sync_dormant_work(sup, goal).await;
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
        let d = defaults(agent.agent_root(), &self.options.class);
        let state = goals.state(&goal.goal_id).await?;
        self.sync_budget_work(sup, goal, &d, state.as_ref(), now).await;
        // The start of the epoch: one event, posted once.
        let schedule = schedule_of(goals, &goal.goal_id).await?;
        if schedule.start_posted_epoch < epoch {
            self.start_epoch(sup, goal, &d, state.as_ref(), now).await?;
            let mut schedule = schedule_of(goals, &goal.goal_id).await?;
            schedule.start_posted_epoch = epoch;
            schedule.updated_at_ms = now;
            goals.write_schedule(&schedule).await?;
        }
        // The hard guarantee: too long without an inference.
        self.guarantee(sup, goal, &d, state.as_ref(), now).await?;
        self.scheduled(sup, goal, &d, state.as_ref(), now).await
    }

    /// The start event of an epoch. Explicit starts (`--start`, and a reopen
    /// or restart of a Goal without a check script) are posted; a Goal with
    /// a check script starts with a check instead.
    async fn start_epoch(&self, sup: &Supervisor, goal: &GoalDefinition, d: &GoalDefaults, state: Option<&GoalState>, now: u64) -> Result<()> {
        let agent = sup.host.agent.as_ref();
        let epoch = goal.session.epoch;
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
        let has_check = goal.schedule.check.is_some();
        let explicit = (kind == ActivationReasonKind::Created && goal.schedule.start_on_create)
            || (kind != ActivationReasonKind::Created && !has_check);
        let note = if explicit {
            Some(start_note)
        } else if has_check {
            match gate_now(agent, goal, state, d, now).await?.0 {
                GateVerdict::Pass { guarantee: true } => Some(format!("{start_note}; the hard guarantee is due")),
                GateVerdict::Pass { guarantee: false } => {
                    let run = self.check(sup, goal, "start", now).await?;
                    match run.verdict {
                        Ok(v) if v.run => Some(match v.reason {
                            Some(r) => format!("{start_note}; check: {r}"),
                            None => format!("{start_note}; the check passed"),
                        }),
                        _ => None,
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(note) = note {
            post_activation(agent, goal, &sup.host.who, kind, &start_key(&goal.goal_id, epoch), &note).await?;
            self.drive(sup, &goal.session.sid, "goal epoch started").await;
        }
        Ok(())
    }

    /// One run opportunity past every gate when the Goal went too long
    /// without an inference (once per quiet stretch). It says how the checks
    /// went: a Goal that broke its own check script learns it here.
    async fn guarantee(&self, sup: &Supervisor, goal: &GoalDefinition, d: &GoalDefaults, state: Option<&GoalState>, now: u64) -> Result<()> {
        if goal.status != GoalStatus::Active || !crate::goal::guarantee_due(goal, state, d, now) {
            return Ok(());
        }
        let since = guarantee_since_ms(goal, state);
        let agent = sup.host.agent.as_ref();
        let goals = agent.goals();
        let mut schedule = schedule_of(goals, &goal.goal_id).await?;
        if schedule.guarantee_posted_for_ms == since {
            return Ok(());
        }
        let secs = crate::goal::guarantee_secs(goal, d).unwrap_or(0);
        let span = if secs >= 3600 && secs % 3600 == 0 {
            format!("{}h", secs / 3600)
        } else if secs >= 60 {
            format!("{}m", secs / 60)
        } else {
            format!("{secs}s")
        };
        let mut note = format!("hard guarantee: no inference for {span} or more");
        match &schedule.last_check {
            Some(c) => note.push_str(&format!(
                "; last check {} at {}",
                c.outcome.as_str(),
                crate::runner::input_view::utc_rfc3339(c.at_ms)
            )),
            None => note.push_str("; no check ran"),
        }
        if schedule.check_errors > 0 {
            note.push_str(&format!(
                "; {} check(s) in a row failed, the last with: {}",
                schedule.check_errors,
                schedule.last_check_error.as_deref().unwrap_or("?")
            ));
        }
        post_activation(
            agent,
            goal,
            &sup.host.who,
            ActivationReasonKind::Guarantee,
            &crate::goal::guarantee_key(&goal.goal_id, since),
            &note,
        )
        .await?;
        schedule = schedule_of(goals, &goal.goal_id).await?;
        schedule.guarantee_posted_for_ms = since;
        let start = crate::goal::period_start(now, window_secs(&effective_budget(goal, d)));
        schedule.stats_mut(start).guarantees += 1;
        schedule.updated_at_ms = now;
        goals.write_schedule(&schedule).await?;
        self.drive(sup, &goal.session.sid, "goal hard guarantee").await;
        Ok(())
    }

    /// A due slot of the check timetable meets the gate; past it the check
    /// script decides (no script: the slot passes).
    async fn scheduled(&self, sup: &Supervisor, goal: &GoalDefinition, d: &GoalDefaults, state: Option<&GoalState>, now: u64) -> Result<()> {
        let agent = sup.host.agent.as_ref();
        let goals = agent.goals();
        let sid = goal.session.sid.clone();
        let timetable = Timetable::of(goal, d);
        let backend = self.options.backend.clone();
        backend.ensure(goals, goal, timetable.as_ref(), now).await?;
        let Some(fire) = backend.due(goals, goal, timetable.as_ref(), now).await? else {
            return Ok(());
        };
        let describe = timetable.as_ref().map(Timetable::describe).unwrap_or_else(|| "a timetable".into());
        let mut note = if fire.missed > 1 {
            format!("scheduled check ({describe}); {} slots passed since the last one", fire.missed)
        } else {
            format!("scheduled check ({describe})")
        };
        let kind = ActivationReasonKind::Scheduled;
        let key = fire.key.clone();
        if !fire.retry {
            if goal.has_capability(CAP_MEMORY_CONSOLIDATION) && goal.status == GoalStatus::Active {
                // Recovery, sweep and cleanup need the drive, not a model.
                self.drive(sup, &sid, "memory maintenance (mechanical)").await;
            }
            let skip = |reason: &str| FireOutcome::Skipped(reason.to_string());
            match gate_now(agent, goal, state, d, now).await?.0 {
                GateVerdict::Dormant => {
                    let reason = if goal.status == GoalStatus::Paused { "paused" } else { "not_active" };
                    backend.settle(goals, &goal.goal_id, &fire, &skip(reason), now).await?;
                    self.skipped(&goal.goal_id, format!("slot {} skipped: {reason}", fire.slot), now);
                    return Ok(());
                }
                GateVerdict::Held { reason, why, .. } => {
                    backend.settle(goals, &goal.goal_id, &fire, &skip(reason.as_str()), now).await?;
                    self.count_skip(goals, goal, d, reason, now).await?;
                    self.skipped(&goal.goal_id, format!("slot {} skipped: {why}", fire.slot), now);
                    return Ok(());
                }
                GateVerdict::Pass { guarantee: true } => {
                    // The hard guarantee was posted on its own this pass.
                    backend.settle(goals, &goal.goal_id, &fire, &skip("guarantee"), now).await?;
                    return Ok(());
                }
                GateVerdict::Pass { guarantee: false } if goal.schedule.check.is_some() => {
                    let run = self.check(sup, goal, "scheduled", now).await?;
                    match run.verdict {
                        Ok(v) if v.run => {
                            note = match v.reason {
                                Some(r) => format!("{note}: {r}"),
                                None => format!("{note}: the check passed"),
                            }
                        }
                        Ok(_) => {
                            backend.settle(goals, &goal.goal_id, &fire, &skip("check_false"), now).await?;
                            return Ok(());
                        }
                        Err(e) => {
                            backend.settle(goals, &goal.goal_id, &fire, &skip("check_error"), now).await?;
                            self.skipped(&goal.goal_id, format!("slot {} check failed: {}", fire.slot, e.chars().take(200).collect::<String>()), now);
                            return Ok(());
                        }
                    }
                }
                GateVerdict::Pass { guarantee: false } => {}
            }
            backend.pending(goals, &goal.goal_id, &fire).await?;
            crate::fault::point("goal:after_fire_pending");
        }
        post_activation(agent, goal, &sup.host.who, kind, &key, &note).await?;
        crate::fault::point("goal:after_fire_post");
        backend
            .settle(goals, &goal.goal_id, &fire, &FireOutcome::Posted, now)
            .await?;
        {
            let mut s = self.status.lock().expect("goal status");
            s.last_fire = Some(GoalEventRecord {
                at_ms: now,
                goal_id: goal.goal_id.clone(),
                what: key.clone(),
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
