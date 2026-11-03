//! Budgets of a run as runaway protection, not work quotas (work context
//! budget TODO §3.2 / §3.4):
//!
//! - **warning**: once per budget fill, when at most a fifth of the tool
//!   iterations is left, the model sees one `<<budget>>` line before its
//!   next inference (injected by the checkpoint hook; never a worklog
//!   entry).
//! - **wrap-up tools**: an explicit-report function call run still offers
//!   `report` once its iterations are used up (`ToolPolicy.wrap_up_tools`,
//!   llm_context's wrap-up mode).
//! - **wrap-up on time / tokens**: a top-level run out of wallclock or total
//!   tokens gets one wrap-up per Turn instead of failing at once: its clock
//!   restarts, its iterations are set to 0 (wrap-up mode) and a `<<budget>>`
//!   line tells it to end now.

use std::sync::Arc;

use agent_tool::xllm::RunStatus;
use buckyos_api::{AiMessage, AiRole};
use llm_context::deps::Injection;
use llm_context::outcome::{BudgetKind, ResumeFill};
use llm_context::request::ToolMode;
use llm_context::state::LLMContextSnapshot;
use llm_context::LLMContext;
use serde_json::json;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;

use super::flush::{run_history_entries, FlushMarks};
use super::receipts::{snapshot_host_meta, with_host_meta};
use super::shared::{LiveCtx, Shared};

/// Marks the host's budget lines in the model's view.
pub const BUDGET_TAG: &str = "<<budget>>";

/// What an explicit-report function call run keeps once its tool
/// iterations are used up (those the run offers as functions).
pub const DEFAULT_WRAP_UP_TOOLS: &[&str] = &[TOOL_REPORT, "ask_user"];

/// The warning comes when `left × WARN_SHARE ≤ max` (20%).
const WARN_SHARE: u32 = 5;

/// Completion reserve assumed for a token wrap-up when the run sets none.
const DEFAULT_WRAP_UP_RESERVE: u64 = 8192;

/// Wrap-up batches a token wrap-up leaves room for.
const WRAP_UP_INFERENCES: u64 = 2;

/// A budget line the host injected (never written to the worklog).
pub(super) fn is_budget_line(text: &str) -> bool {
    text.starts_with(BUDGET_TAG)
}

/// How the run ends now.
fn finish_hint(snapshot: &LLMContextSnapshot) -> &'static str {
    if !snapshot.request.behavior_name.is_empty() {
        "end with your final report (`<report end=\"true\">`): what you did, where the results are and what is left"
    } else if snapshot
        .request
        .tool_policy
        .wrap_up_tools
        .iter()
        .any(|t| t == TOOL_REPORT)
    {
        "end with `report` (is_end: true): what you did, where the results are and what is left"
    } else {
        "give your final answer"
    }
}

/// The wrap-up tools of a new run: the behavior's `[budget] wrap_up_tools`,
/// else [`DEFAULT_WRAP_UP_TOOLS`] for an explicit-report function call run;
/// only those the run offers as functions (`offered`). A behavior loop
/// needs none: its report is part of a step, which costs no iteration.
pub(super) fn wrap_up_tools(
    configured: Option<&Vec<String>>,
    explicit_report: bool,
    function_call: bool,
    offered: &[String],
) -> Vec<String> {
    if !function_call {
        return Vec::new();
    }
    let wanted: Vec<String> = match configured {
        Some(names) => names.clone(),
        None if explicit_report => DEFAULT_WRAP_UP_TOOLS
            .iter()
            .map(|s| s.to_string())
            .collect(),
        None => return Vec::new(),
    };
    wanted.into_iter().filter(|n| offered.contains(n)).collect()
}

/// A Turn refills the run's tool budget: iterations, wrap-up batches and
/// the warning.
pub(super) fn refill(snapshot: &mut LLMContextSnapshot) {
    snapshot.state.tool_iterations_left = snapshot.request.tool_policy.max_tool_iterations;
    snapshot.state.wrap_up_rounds_used = 0;
    let mut meta = snapshot_host_meta(snapshot);
    if meta.budget_warned {
        meta.budget_warned = false;
        snapshot.state.host = Some(with_host_meta(snapshot.state.host.as_ref(), &meta));
    }
}

/// The `<<budget>>` warning due before the next inference, if any: at most
/// a fifth of the iterations left and not told since the budget was
/// filled. The mark goes with the injection, so the next checkpoint
/// commits both.
pub(super) fn warning(snapshot: &LLMContextSnapshot) -> Option<Injection> {
    let policy = &snapshot.request.tool_policy;
    let max = policy.max_tool_iterations;
    let left = snapshot.state.tool_iterations_left;
    if max == 0
        || left.saturating_mul(WARN_SHARE) > max
        || (policy.mode == ToolMode::None && policy.action_mode == ToolMode::None)
    {
        return None;
    }
    let mut meta = snapshot_host_meta(snapshot);
    if meta.budget_warned {
        return None;
    }
    meta.budget_warned = true;
    let hint = finish_hint(snapshot);
    let text = if left == 0 {
        match policy.wrap_up_tools.as_slice() {
            [] => format!("{BUDGET_TAG} The tool budget of this Turn ({max} iterations) is used up: no more tool calls. Now {hint}."),
            tools => format!(
                "{BUDGET_TAG} The tool budget of this Turn ({max} iterations) is used up: only {} remain. Now {hint}.",
                tools.join(" / ")
            ),
        }
    } else {
        format!("{BUDGET_TAG} {left} of {max} tool iterations left in this Turn. Wrap up: finish the step you are on and {hint} before they run out.")
    };
    Some(Injection {
        messages: vec![AiMessage::text(AiRole::User, text)],
        host: Some(with_host_meta(snapshot.state.host.as_ref(), &meta)),
    })
}

/// A top-level run that ran out of wallclock / total tokens, turned into
/// its wrap-up (once per Turn): the history so far goes to the worklog with
/// a `control_applied{budget_wrap_up}`, the clock restarts, no tool
/// iteration is left (wrap-up mode: only the wrap-up tools, at most
/// `wrap_up_rounds` batches), the token cap leaves room for those
/// inferences, and a `<<budget>>` line asks for the end. `None`: the
/// outcome stands (a sub context, another kind, already wrapped up).
pub(super) async fn wrap_up(
    sh: &Arc<Shared>,
    lc: &LiveCtx,
    which: BudgetKind,
) -> Result<Option<LLMContext>> {
    if !matches!(which, BudgetKind::Wallclock | BudgetKind::Tokens) {
        return Ok(None);
    }
    let run_id = lc.run.run_id().to_string();
    if super::outcome::call_site_of(sh, &run_id).await.0 {
        return Ok(None);
    }
    let mut snapshot = lc.ctx.snapshot();
    if snapshot.state.suspended.is_some() || snapshot.state.has_continuation() {
        return Ok(None);
    }
    let mut s = sh.session.lock().await;
    let turn = s.state.current_turn().max(1);
    let mut meta = snapshot_host_meta(&snapshot);
    if meta.budget_wrap_up_turn == turn {
        return Ok(None);
    }
    let Some(live) = s.state.live_run.clone().filter(|l| l.run_id == run_id) else {
        return Ok(None);
    };
    let elapsed_ms = crate::now_ms().saturating_sub(snapshot.state.started_at_ms);
    let used = snapshot.state.usage.total_tokens.unwrap_or(0);
    let hint = finish_hint(&snapshot);
    let text = match which {
        BudgetKind::Wallclock => format!(
            "{BUDGET_TAG} This run has used its time budget ({} min of continuous running). Do not start anything new: {hint} now.",
            elapsed_ms / 60_000
        ),
        _ => format!(
            "{BUDGET_TAG} This run has used its token budget ({used} tokens). Do not start anything new: {hint} now."
        ),
    };
    // What the run did so far, then the wrap-up, in time order.
    let (mut bodies, marks) =
        run_history_entries(&run_id, &snapshot, lc.behavior, FlushMarks::of(&live), turn);
    bodies.push(WorklogBody::ControlApplied {
        input: InputRef {
            src: "_runner".into(),
            index: 0,
            key: format!("budget_wrap_up:{run_id}:{turn}"),
            kind: INPUT_TYPE_CONTROL.into(),
        },
        command: "budget_wrap_up".into(),
        detail: json!({
            "which": which,
            "run_id": run_id,
            "turn": turn,
            "elapsed_ms": elapsed_ms,
            "total_tokens": used,
        }),
    });
    s.append_worklog(&sh.lease, bodies)?;
    if let Some(l) = s.state.live_run.as_mut() {
        marks.apply(l);
    }
    s.commit_state(&sh.lease)?;
    drop(s);
    snapshot.state.started_at_ms = crate::now_ms();
    snapshot.state.tool_iterations_left = 0;
    snapshot.state.wrap_up_rounds_used = 0;
    if which == BudgetKind::Tokens {
        let reserve = snapshot
            .request
            .model_policy
            .max_completion_tokens
            .map(u64::from)
            .unwrap_or(DEFAULT_WRAP_UP_RESERVE);
        let request = super::live::prefix_tokens(&snapshot, &lc.deps)
            + llm_context::context_window::estimate_messages(
                lc.deps.tokenizer.as_ref(),
                &snapshot.state.accumulated,
            );
        let cap = used + WRAP_UP_INFERENCES * (request + reserve);
        snapshot.request.budget.max_total_tokens = Some(cap.min(u64::from(u32::MAX)) as u32);
    }
    meta.budget_wrap_up_turn = turn;
    meta.budget_warned = true;
    snapshot.state.host = Some(with_host_meta(snapshot.state.host.as_ref(), &meta));
    let mut ctx = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, lc.deps.clone())
        .map_err(|e| OpenDanError::blocked(format!("budget wrap-up: {e}"), Some(&run_id)))?;
    ctx.inject(Injection {
        messages: vec![AiMessage::text(AiRole::User, text)],
        host: None,
    });
    lc.run
        .checkpoint_with_results(&ctx.snapshot(), Some(RunStatus::Running))?;
    log::info!(
        "opendan.budget.wrap_up {}",
        json!({
            "session_id": sh.dir.sid(),
            "run_id": run_id,
            "which": which,
            "turn": turn,
            "elapsed_ms": elapsed_ms,
            "total_tokens": used,
        })
    );
    Ok(Some(ctx))
}
