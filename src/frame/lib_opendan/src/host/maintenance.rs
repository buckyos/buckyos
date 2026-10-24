//! The agent's default consolidation Goal on the host (Jarvis default Goal
//! TODO §2, §6): seeded once per Agent with a deterministic key, and given
//! run opportunities through a deterministic gate instead of plain slots.
//!
//! The gate runs inside `GoalHost::reconcile` for the start events
//! (`created / reopened / restarted`) and for every scheduled fire of the
//! built-in Goal: a cheap Memory snapshot comparison first, the full
//! `pending_work` / due-review check only when something moved or a
//! baseline slot (every 4th 30-minute slot, UTC aligned) came; a run
//! opportunity is posted only when there is material for a model, purely
//! mechanical work (cleanup retries, unfinished receipts) drives the session
//! without one. Errors of the checks are `unknown` with a bounded backoff,
//! never "no work". Water marks live in `schedule.json.gate`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::Result;
use crate::goal::{current_usage, period_exhausted, window_secs};
use crate::memory::{Memory, MemoryConfig, MemoryError, PendingWork};
use crate::protocol::*;
use crate::state::{GoalCaller, GoalCreate, GoalRevise, GoalStore};
use crate::template::MaintenanceDefaults;

/// Slot length of the built-in Goal's schedule (30 minutes).
pub const MAINTENANCE_SLOT_SECS: u64 = 1800;
/// Every this many slots is a baseline check (2 hours).
pub const MAINTENANCE_BASELINE_SLOTS: u64 = 4;
/// Backoff of the gate after an `unknown` check: 5m → 15m → 30m.
const BACKOFF_MS: [u64; 3] = [5 * 60_000, 15 * 60_000, 30 * 60_000];

/// The seeded definition (§2; budget per §2 "首版限额").
pub fn memory_maintenance_create(behavior: Option<String>, owner: &str) -> GoalCreate {
    GoalCreate {
        key: Some(BUILTIN_MEMORY_MAINTENANCE_KEY.to_string()),
        title: Some("Memory maintenance".to_string()),
        objective: "Consolidate what the agent perceived into cognitions it can rely on: absorb, defer or discard every perception that reached its processing condition, keep cognitions traceable and reviewed on time, and give the goals or schedules they reveal a clear destination (candidates for the owner to confirm).".to_string(),
        scope: GoalScope::default(),
        lifecycle: GoalLifecycle::Maintain,
        achieved_when: None,
        healthy_state: Some("Perceptions that reached their processing condition are reliably disposed of; cognitions are traceable (source, scope, basis, evidence time) and reviewed when due; discovered goals and schedules have a clear destination.".to_string()),
        distance_evaluation: "New material, due deferrals, failed cleanups, cognitions due for review, candidates awaiting confirmation, actions not materialized, and the age of the oldest backlog.".to_string(),
        constraints: vec![
            "A cognition's subject is the agent itself, its owner, or a workspace; anything else is a world observation with an expiry or review date, never a stable fact.".to_string(),
            "Only material read in the current batch is disposed of; nothing is invented when a batch supports no cognition.".to_string(),
            "The goal records no perceptions of its own and creates no work sessions.".to_string(),
        ],
        initial_skills: Vec::new(),
        beneficiaries: Vec::new(),
        budget: GoalBudget {
            activation: ActivationBudget {
                max_total_tokens: Some(32_000),
                max_runs: Some(4),
            },
            period: PeriodBudget {
                window_secs: Some(86_400),
                max_total_tokens: Some(200_000),
                max_activations: Some(48),
                max_works: Some(0),
            },
        },
        schedule: GoalScheduleSpec {
            every_secs: Some(MAINTENANCE_SLOT_SECS),
            min_interval_secs: Some(60),
            start_on_create: false,
        },
        behavior,
        owner: Some(owner.to_string()),
        extensions: [(
            OPENDAN_EXTENSION.to_string(),
            OpendanGoalExtension::memory_maintenance().to_value(),
        )]
        .into_iter()
        .collect(),
    }
}

/// Seed the built-in Goal once per Agent (the deterministic key makes a
/// repeat return the same Goal), and lift the host's marks of an older
/// policy version without touching what the owner may have changed.
pub async fn seed(goals: &dyn GoalStore, agent_did: &str, d: &MaintenanceDefaults) -> Result<GoalDefinition> {
    let caller = GoalCaller::owner(agent_did);
    let goal = goals
        .create(&memory_maintenance_create(d.behavior.clone(), agent_did), &caller)
        .await?;
    let current = goal.opendan().map(|e| e.policy_version).unwrap_or(0);
    if current >= MEMORY_MAINTENANCE_POLICY_VERSION {
        return Ok(goal);
    }
    let mut ext = goal.extensions.clone();
    ext.insert(
        OPENDAN_EXTENSION.to_string(),
        OpendanGoalExtension::memory_maintenance().to_value(),
    );
    goals
        .revise(
            &goal.goal_id,
            &GoalRevise {
                expected_revision: goal.revision,
                reason: Some(format!(
                    "memory maintenance policy {current} → {MEMORY_MAINTENANCE_POLICY_VERSION}"
                )),
                extensions: Some(ext),
                ..Default::default()
            },
            &caller,
        )
        .await
}

/// Why the gate looks at the Goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateReason {
    /// The epoch's start event (`created / reopened / restarted`).
    Start,
    /// A scheduled fire of `slot`.
    Scheduled(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Post a run opportunity with this note.
    Run(String),
    /// No model now; the reason is recorded (`mechanical`: the session is
    /// driven without an input, for its recovery and cleanup).
    Skip(String),
}

fn parse_ms(t: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|t| t.with_timezone(&Utc).timestamp_millis().max(0) as u64)
}

async fn probe<T: Send + 'static>(
    f: impl FnOnce() -> std::result::Result<T, MemoryError> + Send + 'static,
) -> std::result::Result<T, MemoryError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| MemoryError::Unavailable(e.to_string()))?
}

fn pending_of(p: &PendingWork) -> MaintenancePending {
    MaintenancePending {
        new: p.new,
        due: p.due,
        waiting: p.waiting,
        retry_cleanup: p.retry_cleanup,
        next_deadline: p.next_deadline.clone(),
        oldest_at_ms: p.oldest_at_ms,
    }
}

/// The deterministic check (§6.1). `state`: the Goal State as published.
/// Returns the decision and the gate to persist.
pub async fn decide(
    memory: Arc<Memory>,
    goal: &GoalDefinition,
    state: Option<&GoalState>,
    budget: &GoalBudget,
    mut gate: MaintenanceGate,
    reason: GateReason,
    now_ms: u64,
) -> (GateDecision, MaintenanceGate) {
    let every = goal.schedule.every_secs.filter(|e| *e > 0).unwrap_or(MAINTENANCE_SLOT_SECS);
    let slot = match reason {
        GateReason::Scheduled(s) => s,
        GateReason::Start => now_ms / 1000 / every,
    };
    gate.last_checked_at_ms = now_ms;
    gate.last_checked_slot = Some(slot);
    let skip = |mut gate: MaintenanceGate, status: &str| {
        gate.status = status.to_string();
        gate.last_skip_reason = Some(status.to_string());
        (GateDecision::Skip(status.to_string()), gate)
    };
    if goal.status == GoalStatus::Paused {
        return skip(gate, "paused");
    }
    if gate.backoff_until_ms.is_some_and(|until| until > now_ms) {
        return skip(gate, "backoff");
    }
    let usage = current_usage(state, now_ms, window_secs(budget));
    if period_exhausted(budget, &usage).is_some() {
        return skip(gate, "period_budget");
    }
    let unfinished = state
        .and_then(|s| s.maintenance.as_ref())
        .map(|m| m.open_receipts().count())
        .unwrap_or(0);
    gate.unfinished_receipts = unfinished;
    let unknown = |mut gate: MaintenanceGate, e: String| {
        let level = (gate.backoff_level as usize).min(BACKOFF_MS.len() - 1);
        gate.backoff_until_ms = Some(now_ms + BACKOFF_MS[level]);
        gate.backoff_level = gate.backoff_level.saturating_add(1);
        gate.last_error = Some(e);
        gate.status = "unknown".to_string();
        gate.last_skip_reason = Some("unknown".to_string());
        (GateDecision::Skip("unknown".to_string()), gate)
    };
    // Fast path (A.9): the version vector, no Graph replay, no lock.
    let digest = match probe({
        let memory = memory.clone();
        move || memory.snapshot()
    })
    .await
    {
        Ok(s) => s.token(),
        Err(e) => return unknown(gate, format!("memory snapshot: {e}")),
    };
    let baseline = slot % MAINTENANCE_BASELINE_SLOTS == 0 && gate.last_baseline_slot != Some(slot);
    let changed = gate.last_inputs_digest.as_deref() != Some(digest.as_str());
    let backlog = gate.pending.as_ref().is_some_and(MaintenancePending::backlog);
    let deadline_due = gate
        .pending
        .as_ref()
        .and_then(|p| p.next_deadline.as_deref())
        .and_then(parse_ms)
        .is_some_and(|t| t <= now_ms);
    let review_due = gate
        .next_review_at
        .as_deref()
        .and_then(parse_ms)
        .is_some_and(|t| t <= now_ms);
    if reason != GateReason::Start
        && !baseline
        && !changed
        && !backlog
        && !deadline_due
        && !review_due
        && unfinished == 0
    {
        return skip(gate, "no_change");
    }
    // Full deterministic check: what Memory holds for the Goal.
    let full = probe({
        let memory = memory.clone();
        move || Ok((memory.pending_work()?, memory.due_reviews()?))
    })
    .await;
    let (pending, (reviews, next_review)) = match full {
        Ok(v) => v,
        Err(e) => return unknown(gate, format!("memory pending work: {e}")),
    };
    gate.pending = Some(pending_of(&pending));
    gate.due_reviews = reviews.len();
    gate.next_review_at = next_review;
    gate.last_inputs_digest = Some(digest);
    gate.last_full_check_at_ms = now_ms;
    gate.backoff_until_ms = None;
    gate.backoff_level = 0;
    gate.last_error = None;
    if baseline {
        gate.last_baseline_slot = Some(slot);
    }
    let semantic = pending.needs_run() || !reviews.is_empty();
    let mechanical = pending.retry_cleanup > 0 || unfinished > 0;
    if !semantic && !mechanical {
        return skip(gate, "no_work");
    }
    if !semantic {
        return skip(gate, "mechanical");
    }
    gate.status = "active".to_string();
    gate.last_skip_reason = None;
    gate.last_run_at_ms = now_ms;
    gate.last_run_slot = Some(slot);
    let mut note = format!(
        "memory maintenance: {} new, {} due, {} waiting, {} cognitions due for review",
        pending.new, pending.due, pending.waiting, reviews.len()
    );
    if let Some(o) = pending.oldest_at_ms {
        note.push_str(&format!("; oldest material {}", crate::memory::iso_ms(o)));
    }
    if unfinished > 0 {
        note.push_str(&format!("; {unfinished} unfinished receipt(s)"));
    }
    (GateDecision::Run(note), gate)
}

/// The Memory component of the agent at `agent_root`.
pub fn memory_of(agent_root: &std::path::Path) -> Arc<Memory> {
    Arc::new(Memory::open(agent_root, MemoryConfig::default()))
}

/// `schedule.json.gate` as JSON (status summaries).
pub fn gate_value(gate: &MaintenanceGate) -> Value {
    serde_json::to_value(gate).unwrap_or(Value::Null)
}
