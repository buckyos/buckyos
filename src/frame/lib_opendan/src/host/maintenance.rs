//! The agent's built-in Goals on the host (Jarvis default Goal TODO §2):
//! seeded once per Agent with a deterministic key. Whether a run opportunity
//! of theirs becomes an inference is decided like any Goal's (architecture
//! §12.1.1): their check scripts come with the agent package
//! (`goal_checks/<name>.ts` of the AgentRoot) and are only where they start —
//! a Goal may rewrite its own, a package upgrade replaces a script only while
//! the Goal still runs the package's.
//!
//! The consolidation Goal's script asks Memory through [`memory_status`]
//! (the check's `memory.status` call); purely mechanical work (unfinished
//! receipts, the sweep, cleanup retries) is driven by the host at every
//! scheduled slot without a model.

use std::path::Path;
use std::sync::Arc;

use crate::error::Result;
use crate::memory::{Memory, MemoryConfig, MemoryError, PendingWork};
use crate::protocol::*;
use crate::state::{GoalCaller, GoalCheckChange, GoalCreate, GoalRevise, GoalStore};
use crate::template::MaintenanceDefaults;

/// Check timetable of the consolidation Goal (30 minutes).
pub const MAINTENANCE_SLOT_SECS: u64 = 1800;
/// Package check scripts of the built-in Goals (relative to the AgentRoot).
pub const MEMORY_MAINTENANCE_CHECK: &str = "goal_checks/memory_maintenance.ts";
pub const SKILL_CRYSTALLIZE_CHECK: &str = "goal_checks/skill_crystallize.ts";

/// The seeded definition (§2). Budget: every round of an Activation re-sends
/// its whole context (the header and a batch are about 9k tokens), so an
/// Activation of begin → commit → report needs well over 32k tokens; the
/// first DV run (2026-10-23) exhausted 32k in four rounds.
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
                max_total_tokens: Some(120_000),
                max_runs: Some(4),
                max_wall_secs: None,
            },
            period: PeriodBudget {
                window_secs: Some(86_400),
                max_total_tokens: Some(1_200_000),
                max_activations: Some(48),
                max_works: Some(0),
            },
        },
        schedule: GoalScheduleSpec {
            every_secs: Some(MAINTENANCE_SLOT_SECS),
            ..Default::default()
        },
        behavior,
        owner: Some(owner.to_string()),
        extensions: [(
            OPENDAN_EXTENSION.to_string(),
            OpendanGoalExtension::memory_maintenance().to_value(),
        )]
        .into_iter()
        .collect(),
        check_script: None,
        check_package: None,
    }
}

/// The seeded definition of the skill crystallization Goal: once a day it reads the work the usage record points at
/// and improves the agent's manuals — its own directly, the others as drafts for the owner.
pub fn skill_crystallize_create(behavior: Option<String>, owner: &str) -> GoalCreate {
    GoalCreate {
        key: Some(BUILTIN_SKILL_CRYSTALLIZE_KEY.to_string()),
        title: Some("Skill crystallization".to_string()),
        objective: "Keep the agent's skills in step with what its work shows: revise manuals that led work astray or left steps out, turn kinds of work that recur without a manual into draft skills, and point out manuals nothing uses.".to_string(),
        scope: GoalScope::default(),
        lifecycle: GoalLifecycle::Maintain,
        achieved_when: None,
        healthy_state: Some("Manuals say what the recent work that used them actually had to do; recurring work without a manual has a draft waiting for the owner; unused or misleading manuals are reported.".to_string()),
        distance_evaluation: "Work sessions frozen with each manual since it was last looked at, work created without any manual, manuals not used for 60 days, drafts waiting for the owner.".to_string(),
        constraints: vec![
            "Only the text of manuals that are the agent's own (origin local or imported) is rewritten, with `agent-session skill revise`; the earlier text is kept.".to_string(),
            "A package's manual, a change to scope / commands / baseline / platforms, new commands and every new skill go to skills_drafts/ for the owner; nothing is installed, disabled or added to the UI set.".to_string(),
            "A judgement about a manual rests on work sessions actually read in this activation.".to_string(),
        ],
        initial_skills: Vec::new(),
        beneficiaries: Vec::new(),
        budget: GoalBudget {
            activation: ActivationBudget { max_total_tokens: Some(300_000), max_runs: Some(6), max_wall_secs: None },
            period: PeriodBudget { window_secs: Some(86_400), max_total_tokens: Some(600_000), max_activations: Some(2), max_works: Some(0) },
        },
        schedule: GoalScheduleSpec { every_secs: Some(86_400), min_interval_secs: Some(3_600), ..Default::default() },
        behavior,
        owner: Some(owner.to_string()),
        extensions: [(OPENDAN_EXTENSION.to_string(), OpendanGoalExtension::skill_crystallize().to_value())].into_iter().collect(),
        check_script: None,
        check_package: None,
    }
}

/// The package's check script of a built-in Goal, with what to record
/// about it (`None`: the AgentRoot has none).
fn package_check(agent_root: Option<&Path>, rel: &str) -> Option<String> {
    std::fs::read_to_string(agent_root?.join(rel)).ok().filter(|s| !s.trim().is_empty())
}

/// Give a built-in Goal its package check script: when it has none, or still
/// runs an older package version (one it changed itself is kept).
pub async fn sync_package_check(
    goals: &dyn GoalStore,
    goal: GoalDefinition,
    agent_root: Option<&Path>,
    rel: &str,
    caller: &GoalCaller,
) -> Result<GoalDefinition> {
    let Some(script) = package_check(agent_root, rel) else {
        return Ok(goal);
    };
    let digest = crate::state::goals::script_digest(&script);
    let current = goal.schedule.check.as_ref();
    let replace = match current {
        None => true,
        Some(c) => c.source.starts_with("package:") && c.sha256 != digest,
    };
    if !replace {
        return Ok(goal);
    }
    goals
        .set_check(
            &goal.goal_id,
            &GoalCheckChange {
                script: Some(script),
                package: Some(rel.to_string()),
                reason: Some(format!("check script of the agent package ({rel})")),
                ..Default::default()
            },
            caller,
        )
        .await
}

/// Seed the built-in Goal once per Agent (the deterministic key makes a
/// repeat return the same Goal), lift the host's marks of an older policy
/// version without touching what the owner may have changed, and give it
/// the package's check script.
pub async fn seed(
    goals: &dyn GoalStore,
    agent_did: &str,
    agent_root: Option<&Path>,
    d: &MaintenanceDefaults,
) -> Result<GoalDefinition> {
    let caller = GoalCaller::owner(agent_did);
    let mut goal = goals
        .create(&memory_maintenance_create(d.behavior.clone(), agent_did), &caller)
        .await?;
    let current = goal.opendan().map(|e| e.policy_version).unwrap_or(0);
    if current < MEMORY_MAINTENANCE_POLICY_VERSION {
        let mut ext = goal.extensions.clone();
        ext.insert(
            OPENDAN_EXTENSION.to_string(),
            OpendanGoalExtension::memory_maintenance().to_value(),
        );
        goal = goals
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
            .await?;
    }
    sync_package_check(goals, goal, agent_root, MEMORY_MAINTENANCE_CHECK, &caller).await
}

/// Seed the skill crystallization Goal (switched on in `agent.toml`).
pub async fn seed_crystallize(
    goals: &dyn GoalStore,
    agent_did: &str,
    agent_root: Option<&Path>,
    behavior: Option<String>,
) -> Result<GoalDefinition> {
    let caller = GoalCaller::owner(agent_did);
    let goal = goals.create(&skill_crystallize_create(behavior, agent_did), &caller).await?;
    sync_package_check(goals, goal, agent_root, SKILL_CRYSTALLIZE_CHECK, &caller).await
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

/// What Memory holds for the consolidation: the version vector always (no
/// Graph replay, no lock), the full check (`pending_work`, due reviews) with
/// `full`.
pub async fn memory_status(agent_root: &Path, full: bool) -> std::result::Result<MemoryStatus, MemoryError> {
    let memory = memory_of(agent_root);
    let digest = probe({
        let memory = memory.clone();
        move || memory.snapshot()
    })
    .await?
    .token();
    if !full {
        return Ok(MemoryStatus { digest, ..Default::default() });
    }
    let (pending, (reviews, next_review)) = probe(move || Ok((memory.pending_work()?, memory.due_reviews()?))).await?;
    Ok(MemoryStatus {
        digest,
        pending: Some(pending_of(&pending)),
        due_reviews: reviews.len(),
        next_review_at: next_review,
    })
}

/// The Memory component of the agent at `agent_root`.
pub fn memory_of(agent_root: &std::path::Path) -> Arc<Memory> {
    Arc::new(Memory::open(agent_root, MemoryConfig::default()))
}
