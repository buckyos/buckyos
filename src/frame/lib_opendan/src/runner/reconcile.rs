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

use super::live::remove_if_safe;
use super::outcome::{
    call_site_of, classify_done, finish_run, freeze_target, hand_over, has_report,
    hold_for_children, CallSite, FinishKind, Next,
};
use super::tools::pending_sub_call;
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
    if redo_transfer(sh, &run, &record, &snapshot).await? {
        return Ok(Reconciled::None);
    }
    Ok(Reconciled::Resume(run, snapshot))
}

/// The live run stopped at a transfer that state has not committed yet: a
/// `next_behavior` hand-over recorded in run.json (ours before a crash, or
/// xllm's, which never commits one), or a `call_behavior` suspension. The
/// transfer is committed here, exactly once: after the commit the run is no
/// longer the live run. `true` = the run was set aside or ended.
async fn redo_transfer(
    sh: &Arc<Shared>,
    run: &RunHandle,
    record: &RunRecord,
    snapshot: &LLMContextSnapshot,
) -> Result<bool> {
    let behavior = record.config.loop_model == LoopModel::Behavior;
    let (cfg, completed, returned, committed) = {
        let s = sh.session.lock().await;
        (
            s.config.clone(),
            s.state.turns_completed,
            s.state.tool_return_pending(),
            s.state.live_run.as_ref().map(|l| l.handover_at_ms).unwrap_or(0),
        )
    };
    let assembler = sh.deps.assembler.clone();
    let entry_cfg = cfg.clone();
    let entry = move |b: &str| assembler.behavior_entry(&entry_cfg, b);
    // A record this state already committed (the run was suspended by it
    // and is live again) is history, not a transfer left to do.
    if let Some(h) = record.handover.as_ref().filter(|h| h.at_ms != committed) {
        freeze_target(sh, Some(&h.next_behavior)).await;
        let cfg = sh.session.lock().await.config.clone();
        let assembler = sh.deps.assembler.clone();
        let entry_cfg = cfg.clone();
        let entry = move |b: &str| assembler.behavior_entry(&entry_cfg, b);
        let (child, depth) = call_site_of(sh, &record.run_id).await;
        let site = CallSite {
            child,
            depth,
            entry: &entry,
        };
        let st = &snapshot.state;
        let last_step = st.last_step.as_ref().or(st.steps.last());
        let answer = st
            .last_report
            .clone()
            .or_else(|| last_step.and_then(|s| s.self_report.clone()));
        let replied =
            has_report(snapshot) || last_step.is_some_and(|s| !s.messages_sent.is_empty());
        let mut next = classify_done(
            &cfg,
            behavior,
            Some(h.next_behavior.clone()),
            answer,
            replied,
            &site,
            completed,
        );
        next.usage = record.usage.main.clone();
        hold_for_children(sh, &mut next).await?;
        if next.kind == FinishKind::Switch {
            hand_over(sh, run, behavior, snapshot, &mut next).await?;
        } else {
            finish_run(sh, run, snapshot, behavior, next).await?;
        }
        return Ok(true);
    }
    if !returned {
        if let Some(call) = pending_sub_call(snapshot, &entry)? {
            let mut next = Next {
                kind: FinishKind::Switch,
                next_behavior: Some(call.behavior.clone()),
                call: Some(call),
                ..Default::default()
            };
            hand_over(sh, run, behavior, snapshot, &mut next).await?;
            return Ok(true);
        }
    }
    Ok(false)
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
        None => {
            let mut next = derive_next(sh, record, snapshot, behavior).await;
            hold_for_children(sh, &mut next).await?;
            next
        }
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
            // A terminal record has no hand-over left (a hosted run that
            // hands over stays `paused`, see `redo_transfer`): whatever the
            // last step named, the run delivered its result.
            let nb = nb.filter(|b| !agent_tool::xllm::is_handover_target(b));
            let (child, depth) = call_site_of(sh, &record.run_id).await;
            let assembler = sh.deps.assembler.clone();
            let entry_cfg = cfg.clone();
            let entry = move |b: &str| assembler.behavior_entry(&entry_cfg, b);
            let site = CallSite {
                child,
                depth,
                entry: &entry,
            };
            let mut next = classify_done(&cfg, behavior, nb, answer, replied, &site, completed);
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
