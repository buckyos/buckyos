//! Recovery of a session's runs at drive start (§8.6): remove unreferenced
//! runs, re-open the live run, redo the end of a run another executor
//! finished (or whose finish crashed half-way).

use std::sync::Arc;

use agent_tool::xllm::{LoopModel, RunRecord, RunStatus};
use buckyos_api::AiRole;
use llm_context::state::LLMContextSnapshot;
use serde_json::json;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::runs::RunHandle;

use super::live::{remove_if_safe, stop_executions};
use super::outcome::{
    classify_done, decide_end, finish_run, has_report, is_fork_child, FinishKind, Next,
};
use super::receipts::{apply_receipt, receipts_after, snapshot_host_meta, validate_receipts};
use super::shared::Shared;

pub(super) enum Reconciled {
    None,
    Resume(RunHandle, LLMContextSnapshot),
}

pub(super) async fn reconcile_runs(sh: &Arc<Shared>) -> Result<Reconciled> {
    let (keep, live_id) = {
        let mut s = sh.session.lock().await;
        s.truncate_uncommitted(&sh.lease)?;
        (
            s.state.referenced_runs(),
            s.state.live_run.as_ref().map(|l| l.run_id.clone()),
        )
    };
    let runs = sh.dir.runs();
    for run_id in runs.list()? {
        if !keep.contains(&run_id) {
            if let Err(e) = remove_if_safe(sh, &run_id).await {
                log::warn!("cleanup of run {run_id}: {e}");
            }
        }
    }
    let Some(run_id) = live_id else {
        return Ok(Reconciled::None);
    };
    let lock = runs
        .try_lock(&run_id)?
        .ok_or_else(|| OpenDanError::RunBusy {
            run_id: run_id.clone(),
        })?;
    let (record, snapshot) = runs.load_checked(&run_id)?;
    let Some(snapshot) = snapshot else {
        return Err(OpenDanError::blocked(
            "the live run has no published snapshot",
            Some(&run_id),
        ));
    };
    let run = RunHandle::new(runs.store().clone(), record.clone(), lock);
    // kill -9 of a runner does not stop its tools.
    stop_executions(sh, &run).await?;
    // Receipts: fill in state from the snapshot, never re-append messages.
    {
        let mut s = sh.session.lock().await;
        let applied = s
            .state
            .live_run
            .as_ref()
            .map(|l| l.applied_input_seq)
            .unwrap_or(0);
        let meta = snapshot_host_meta(&snapshot);
        validate_receipts(&meta, &run_id, applied)?;
        let delta = receipts_after(&meta, applied);
        if !delta.is_empty() {
            for r in &delta {
                apply_receipt(&mut s.state, r)?;
            }
            s.commit_state(&sh.lease)?;
        }
        if let Some(seq) = run.host_commit_pending() {
            let applied = s
                .state
                .live_run
                .as_ref()
                .map(|l| l.applied_input_seq)
                .unwrap_or(0);
            if applied < seq {
                return Err(OpenDanError::blocked(
                    format!("host commit {seq} is pending but state only applied {applied}"),
                    Some(&run_id),
                ));
            }
            run.complete_host_commit()?;
        }
    }
    if record.status.is_terminal() {
        // Finished by xllm, or our finish crashed half-way: redo the finish.
        finish_terminal_record(sh, &run, &record, &snapshot).await?;
        return Ok(Reconciled::None);
    }
    Ok(Reconciled::Resume(run, snapshot))
}

/// Redo the end of a run from its record (xllm finished it, or finish_run
/// crashed before the commit): the decision recorded with the terminal
/// status, else rebuilt from the published snapshot.
async fn finish_terminal_record(
    sh: &Arc<Shared>,
    run: &RunHandle,
    record: &RunRecord,
    snapshot: &LLMContextSnapshot,
) -> Result<()> {
    let behavior = record.config.loop_model == LoopModel::Behavior;
    let recorded = crate::session::runs::finish_info(record)
        .and_then(|v| serde_json::from_value::<Next>(v).ok());
    let next = match recorded {
        Some(n) => n,
        None => derive_next(sh, record, snapshot, behavior).await,
    };
    finish_run(sh, run, snapshot, behavior, next).await
}

/// Decision of a run finished by another executor (xllm), from its record
/// and final snapshot.
async fn derive_next(
    sh: &Arc<Shared>,
    record: &RunRecord,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
) -> Next {
    let (cfg, completed) = {
        let s = sh.session.lock().await;
        (s.config.clone(), s.state.turns_completed)
    };
    match record.status {
        RunStatus::Completed => {
            let st = &snapshot.state;
            let last_step = st.last_step.as_ref().or(st.steps.last());
            let replied =
                has_report(snapshot) || last_step.is_some_and(|s| !s.messages_sent.is_empty());
            let (nb, answer) = if behavior {
                (
                    last_step.and_then(|s| s.next_behavior.clone()),
                    st.last_report
                        .clone()
                        .or_else(|| last_step.and_then(|s| s.self_report.clone())),
                )
            } else {
                (
                    None,
                    st.accumulated
                        .iter()
                        .rev()
                        .find(|m| m.role == AiRole::Assistant)
                        .map(|m| m.text_content()),
                )
            };
            let answer = answer.or_else(|| record.result.as_ref().map(|r| r.raw.clone()));
            let fork_child = is_fork_child(sh, &record.run_id).await;
            let mut next =
                classify_done(&cfg, behavior, nb, answer, replied, fork_child, completed);
            if next.kind == FinishKind::Switch {
                // The executor ended the run; nothing continues it.
                next.kind = FinishKind::Done;
                next.run_ended = true;
                next.next_behavior = None;
                decide_end(&cfg, &mut next, completed + 1);
            }
            next.usage = record.usage.main.clone();
            next
        }
        RunStatus::Failed => Next {
            run_ended: true,
            kind: FinishKind::Error,
            turn_end: Some(TurnStatus::Failed),
            usage: record.usage.main.clone(),
            error: Some(json!({
                "kind": "run_failed",
                "message": record.last_error.as_ref().map(|e| e.message.clone()).unwrap_or_default(),
            })),
            ..Default::default()
        },
        _ => Next {
            run_ended: true,
            kind: FinishKind::Budget,
            turn_end: Some(TurnStatus::BudgetExhausted),
            usage: record.usage.main.clone(),
            error: Some(json!({
                "kind": "limit_reached",
                "message": record.limit_reason.clone().unwrap_or_default(),
            })),
            ..Default::default()
        },
    }
}
