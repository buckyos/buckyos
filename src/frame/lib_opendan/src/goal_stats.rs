//! What a Goal did and spent, period by period (`agent-session goal stats`,
//! kRPC `goals.stats`): read from the Activation records, the gate's
//! counters (`schedule.json`) and the sessions' `usage.jsonl`. Read only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::state::{ActivationQuery, GoalStore, SessionRegistry};
use crate::template::GoalDefaults;

/// Periods shown when none is asked for, and at most.
pub const STATS_PERIODS: usize = 7;
pub const STATS_PERIODS_MAX: usize = GATE_STATS_KEEP + 1;

/// One statistics period of a Goal (the current one is not over yet).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoalPeriod {
    pub start_ms: u64,
    pub end_ms: u64,
    /// RFC 3339, for reading.
    pub start: String,
    pub activations: u64,
    /// Activations per decision, outcome and kind of reason.
    pub decisions: BTreeMap<String, u64>,
    pub outcomes: BTreeMap<String, u64>,
    pub reasons: BTreeMap<String, u64>,
    /// Work its Activations created.
    pub works_created: u64,
    /// Tokens of the Goal Sessions and of its Work that ran in the period.
    pub tokens_by_goal: u64,
    pub tokens_by_works: u64,
    /// Model tokens of its jobs, and their runs that succeeded and did not.
    pub tokens_by_jobs: u64,
    pub job_runs_ok: u64,
    pub job_runs_failed: u64,
    /// Tokens of one Activation on average (its Work not included).
    pub tokens_per_activation: u64,
    pub checks_true: u64,
    pub checks_false: u64,
    pub checks_error: u64,
    pub notifications_by_checks: u64,
    pub skipped_by_interval: u64,
    pub skipped_by_budget: u64,
    pub hard_guarantees: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoalStats {
    pub goal_id: String,
    pub period_secs: u64,
    /// Oldest first; the last one is the current period.
    pub periods: Vec<GoalPeriod>,
    pub no_action_in_a_row: u32,
    pub activations_failed_in_a_row: u32,
    pub works_failed_in_a_row: u32,
    pub check_errors_in_a_row: u32,
}

fn reason_name(kind: ActivationReasonKind) -> &'static str {
    crate::goal::event_name(kind)
}

/// The last `periods` periods of a Goal.
pub async fn stats(
    goals: &dyn GoalStore,
    sessions: &dyn SessionRegistry,
    goal_id: &str,
    periods: usize,
    d: &GoalDefaults,
    now_ms: u64,
) -> Result<GoalStats> {
    let goal = goals
        .get(goal_id)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("goal {goal_id}")))?;
    let state = goals.state(goal_id).await?;
    let schedule = goals.schedule(goal_id).await?;
    let window = crate::goal::window_secs(&crate::goal::effective_budget(&goal, d));
    let window_ms = window * 1000;
    let current = crate::goal::period_start(now_ms, window);
    let periods = periods.clamp(1, STATS_PERIODS_MAX);
    let first = current.saturating_sub((periods as u64 - 1) * window_ms);
    let mut out: Vec<GoalPeriod> = (0..periods as u64)
        .map(|i| {
            let start_ms = first + i * window_ms;
            GoalPeriod {
                start_ms,
                end_ms: start_ms + window_ms,
                start: crate::runner::input_view::utc_rfc3339(start_ms),
                ..Default::default()
            }
        })
        .collect();
    let at = |ms: u64| (ms >= first).then(|| ((ms - first) / window_ms) as usize).filter(|i| *i < periods);
    let mut spent = vec![0u64; periods];
    for a in goals.activations(goal_id, &ActivationQuery::default()).await? {
        let Some(i) = at(a.ended_at_ms) else {
            continue;
        };
        let p = &mut out[i];
        p.activations += 1;
        *p.decisions.entry(a.decision.as_str().to_string()).or_default() += 1;
        *p.outcomes.entry(a.outcome.as_str().to_string()).or_default() += 1;
        let mut kinds: Vec<&str> = a.reasons.iter().map(|r| reason_name(r.kind)).collect();
        kinds.sort_unstable();
        kinds.dedup();
        for k in kinds {
            *p.reasons.entry(k.to_string()).or_default() += 1;
        }
        p.works_created += a.works_created.len() as u64;
        spent[i] += a.cost.total_tokens;
    }
    for (p, tokens) in out.iter_mut().zip(&spent) {
        p.tokens_per_activation = tokens.checked_div(p.activations).unwrap_or(0);
    }
    for (own, r) in crate::goal::goal_usage(sessions, goal_id, first).await? {
        let Some(i) = at(r.at_ms) else {
            continue;
        };
        if own {
            out[i].tokens_by_goal += r.total_tokens;
        } else {
            out[i].tokens_by_works += r.total_tokens;
        }
    }
    for r in goals.job_usage(goal_id, first).await? {
        if let Some(i) = at(r.at_ms) {
            out[i].tokens_by_jobs += r.total_tokens;
        }
    }
    for j in goals.jobs(goal_id).await? {
        for r in goals.job_runs(goal_id, &j.job.job_id, 0).await? {
            let Some(i) = at(r.ended_at_ms) else {
                continue;
            };
            match r.status {
                crate::goal_jobs::JobRunStatus::Ok => out[i].job_runs_ok += 1,
                crate::goal_jobs::JobRunStatus::Killed => {}
                _ => out[i].job_runs_failed += 1,
            }
        }
    }
    if let Some(s) = &schedule {
        for g in s.past_stats.iter().chain(std::iter::once(&s.stats)) {
            let Some(i) = at(g.period_start_ms).filter(|_| g.period_start_ms > 0) else {
                continue;
            };
            let p = &mut out[i];
            p.checks_true += g.checks_true;
            p.checks_false += g.checks_false;
            p.checks_error += g.checks_error;
            p.notifications_by_checks += g.notified;
            p.skipped_by_interval += g.skipped_min_interval;
            p.skipped_by_budget += g.skipped_budget;
            p.hard_guarantees += g.guarantees;
        }
    }
    Ok(GoalStats {
        goal_id: goal_id.to_string(),
        period_secs: window,
        periods: out,
        no_action_in_a_row: state.as_ref().map(|s| s.consecutive_no_action).unwrap_or(0),
        activations_failed_in_a_row: state.as_ref().map(|s| s.activations_failed_in_a_row).unwrap_or(0),
        works_failed_in_a_row: state.as_ref().map(|s| s.works_failed_in_a_row).unwrap_or(0),
        check_errors_in_a_row: schedule.map(|s| s.check_errors).unwrap_or(0),
    })
}
