//! The agent's built-in Goal on the host, `self_improve` (Jarvis default Goal
//! TODO §2): seeded once per Agent with a deterministic key. Whether a run
//! opportunity of it becomes an inference is decided like any Goal's
//! (architecture §12.1.1): its check script comes with the agent package
//! (`goal_checks/self_improve.ts` of the AgentRoot) and is only where it
//! starts — the Goal may rewrite its own, a package upgrade replaces it only
//! while the Goal still runs the package's.
//!
//! The script asks Memory through [`memory_status`] (the check's
//! `memory.status` call); purely mechanical work of the consolidation
//! (unfinished receipts, the sweep, cleanup retries) is driven by the host at
//! every scheduled slot without a model.

use std::path::Path;
use std::sync::Arc;

use crate::error::Result;
use crate::memory::{Memory, MemoryConfig, MemoryError, PendingWork};
use crate::protocol::*;
use crate::state::{GoalCaller, GoalCheckChange, GoalCreate, GoalRestart, GoalRevise, GoalStatusChange, GoalStore};

/// Check timetable of the self improvement Goal (30 minutes).
pub const SELF_IMPROVE_SLOT_SECS: u64 = 1800;
/// Its package check script (relative to the AgentRoot).
pub const SELF_IMPROVE_CHECK: &str = "goal_checks/self_improve.ts";
/// What was built in before it (`extensions.opendan.builtin`): the Goal that
/// consolidated Memory becomes the self improvement Goal, one for the manuals
/// is merged into it.
const FORMER: [&str; 2] = ["memory_maintenance", "skill_crystallize"];

/// Goal skills the self improvement Goal starts with: one per duty, and how it keeps itself cheap.
pub const SELF_IMPROVE_SKILLS: [&str; 4] = ["self_improve_memory", "self_improve_manuals", "self_improve_tools", "goal_upkeep"];

/// The seeded definition of the self improvement Goal: this agent first able to do its work, then cheaper at it.
pub fn self_improve_create(owner: &str) -> GoalCreate {
    GoalCreate {
        key: Some(BUILTIN_SELF_IMPROVE_KEY.to_string()),
        title: Some("Self improvement".to_string()),
        objective: "Make this agent better at its work by its own record: first that it can do what it is asked, then that it needs fewer rounds and tokens for it. Duties, each with a goal skill of its own that is read before the duty is worked on: `memory` (skill self_improve_memory: consolidate what the agent perceived into cognitions it can rely on; schedule, to-dos and lasting duties found there become questions for the owner), `manuals` (self_improve_manuals: keep the manuals in step with what work really had to do), `tools` (self_improve_tools: make the commands work keeps lacking or keeps composing again). The reason of an activation names the duty that has material: work on that one only, the check looked at the others.".to_string(),
        scope: GoalScope::default(),
        lifecycle: GoalLifecycle::Maintain,
        achieved_when: None,
        healthy_state: Some("Perceptions that reached their processing condition are reliably disposed of; cognitions are traceable (source, scope, basis, evidence time) and reviewed when due; what they reveal for the owner to do or decide was asked once. Manuals say what the recent work that used them actually had to do; work that recurs has a manual or a tool, or a draft waiting for the owner; no command is missing that two pieces of work needed.".to_string()),
        distance_evaluation: "Memory: new material, due deferrals, failed cleanups, cognitions due for review, the age of the oldest backlog, finds not yet answered. Manuals and tools: work frozen with each manual since it was last looked at, work created without any, commands work did not find, manuals unused for 60 days, drafts waiting for the owner, whether the last tool or revision was used and helped.".to_string(),
        constraints: vec![
            "The owner switches duties on and off (`.meta/skills_state.json` `self_improve`; a duty that is not there is on). The check names no duty that is off; one that is off is not yours.".to_string(),
            "Memory: a cognition's subject is the agent itself, its owner, or a workspace; anything else is a world observation with an expiry or review date, never a stable fact. Only material read in the current batch is disposed of; nothing is invented when a batch supports no cognition. The goal records no perceptions of its own.".to_string(),
            "A judgement about a manual or a missing tool rests on work sessions actually read, at least two of them; one is an incident, not a pattern.".to_string(),
            "Only the text of manuals of the agent's own (origin local or imported) is rewritten, with `agent-session skill revise`; a new manual of its own (scope work) comes with a tool it describes, with `agent-session skill add`. A package's manual, every change of scope / commands / baseline / platforms and everything for conversations (scope ui) is a draft under skills_drafts/ for the owner.".to_string(),
            "Nothing is installed, removed, enabled or disabled; which manuals conversations get is never changed. No secrets, tokens, personal data or one task's parameters go into a manual or a tool.".to_string(),
        ],
        initial_skills: SELF_IMPROVE_SKILLS
            .iter()
            .map(|name| GoalSkill { name: name.to_string(), reference: Some(SkillKind::Goal.manual(name)), version: None, note: None })
            .collect(),
        beneficiaries: Vec::new(),
        budget: GoalBudget {
            activation: ActivationBudget { max_total_tokens: Some(500_000), max_runs: Some(6), max_wall_secs: None },
            period: PeriodBudget { window_secs: Some(86_400), max_total_tokens: Some(1_800_000), max_activations: Some(48), max_works: None },
        },
        schedule: GoalScheduleSpec { every_secs: Some(SELF_IMPROVE_SLOT_SECS), ..Default::default() },
        behavior: None,
        owner: Some(owner.to_string()),
        extensions: [(OPENDAN_EXTENSION.to_string(), OpendanGoalExtension::self_improve().to_value())].into_iter().collect(),
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
/// runs an older package version (one it changed itself is kept, unless it
/// `became` this Goal: the script of another duty would never name these),
/// with the timetable `every_secs`.
pub async fn sync_package_check(
    goals: &dyn GoalStore,
    goal: GoalDefinition,
    agent_root: Option<&Path>,
    rel: &str,
    became: Option<u64>,
    caller: &GoalCaller,
) -> Result<GoalDefinition> {
    let Some(script) = package_check(agent_root, rel) else {
        return Ok(goal);
    };
    let digest = crate::state::goals::script_digest(&script);
    let current = goal.schedule.check.as_ref();
    let replace = match current {
        None => true,
        Some(c) => (became.is_some() || c.source.starts_with("package:")) && c.sha256 != digest,
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
                every_secs: became,
                reason: Some(format!("check script of the agent package ({rel})")),
                ..Default::default()
            },
            caller,
        )
        .await
}

/// Seed the self improvement Goal (switched on in `agent.toml`) once per Agent: the deterministic key makes a repeat
/// return the same Goal. An agent that had the built-in Goals before it keeps the one that consolidated Memory (with
/// its receipts), rewritten to this policy with a fresh context — its behavior is gone — and the package's check, and
/// the other is archived. What the owner set stays: status, token budgets, intervals.
pub async fn seed_self_improve(goals: &dyn GoalStore, agent_did: &str, agent_root: Option<&Path>) -> Result<GoalDefinition> {
    let caller = GoalCaller::owner(agent_did);
    let policy = self_improve_create(agent_did);
    let all = goals.list().await?;
    let kept = std::iter::once(BUILTIN_SELF_IMPROVE)
        .chain(FORMER)
        .find_map(|name| all.iter().find(|g| g.is_builtin(name) && (name == BUILTIN_SELF_IMPROVE || g.status.is_live())));
    let mut goal = match kept {
        Some(g) => g.clone(),
        None => goals.create(&policy, &caller).await?,
    };
    let before = goal.opendan().unwrap_or_default();
    let became = (!before.is_builtin(BUILTIN_SELF_IMPROVE)).then_some(SELF_IMPROVE_SLOT_SECS);
    // Before the marker changes: what is not finished is done again.
    goal = sync_package_check(goals, goal, agent_root, SELF_IMPROVE_CHECK, became, &caller).await?;
    if !before.is_builtin(BUILTIN_SELF_IMPROVE) || before.policy_version < SELF_IMPROVE_POLICY_VERSION {
        let mut ext = goal.extensions.clone();
        ext.extend(policy.extensions.clone());
        let mut budget = goal.budget.clone();
        budget.period.max_works = policy.budget.period.max_works;
        goal = goals
            .revise(
                &goal.goal_id,
                &GoalRevise {
                    expected_revision: goal.revision,
                    reason: Some(format!("self improvement policy {SELF_IMPROVE_POLICY_VERSION}")),
                    title: policy.title.clone(),
                    objective: Some(policy.objective.clone()),
                    healthy_state: policy.healthy_state.clone(),
                    distance_evaluation: Some(policy.distance_evaluation.clone()),
                    constraints: Some(policy.constraints.clone()),
                    initial_skills: Some(policy.initial_skills.clone()),
                    budget: Some(budget),
                    behavior: Some(crate::goal::GOAL_CLASS.to_string()),
                    extensions: Some(ext),
                    ..Default::default()
                },
                &caller,
            )
            .await?;
        if goal.status.is_live() {
            let restart = GoalRestart { reason: "self improvement takes over".into(), carry_state: false };
            goal = goals.restart(&goal.goal_id, &restart, &caller).await?;
        }
    }
    for g in all.iter().filter(|g| g.goal_id != goal.goal_id && g.status.is_live() && FORMER.iter().any(|n| g.is_builtin(n))) {
        let merged = GoalStatusChange {
            status: GoalStatus::Archived,
            reason: Some(format!("merged into self improvement ({})", goal.goal_id)),
            evidence: None,
        };
        goals.set_status(&g.goal_id, &merged, &caller).await?;
    }
    Ok(goal)
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
