//! Sharing a finished user task (`share` TODO §5.2): a Work created under a
//! UI session with `--report final` that ended `Succeeded` with a final
//! report is turned into the agent's own post, once per run.

use std::sync::Arc;

use agent_tool::xllm::create_run_llm;
use serde_json::json;

use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::share::{compose, ShareArtifact, ShareInput};

use super::outcome::Next;
use super::shared::Shared;

/// What the end of a run hands over to the share decision.
pub(super) struct Finished {
    pub(super) config: SessionConfig,
    pub(super) turns: u64,
}

/// Whether this session's end is a user task's (its parent is a UI session
/// and it reports to it).
fn user_task(config: &SessionConfig) -> Option<String> {
    if config.session.kind != SessionKind::Work {
        return None;
    }
    let origin = config.session.origin.as_ref()?;
    if origin.report != Some(ReportMode::Final) {
        return None;
    }
    origin.parent_session.clone()
}

/// Share the run's result when it is a successful user task; runs in the
/// background so the delivery of the report never waits for it.
pub(super) fn after_finish(sh: &Arc<Shared>, run: &RunHandle, next: &Next, finished: Finished) {
    let Some(share) = sh.deps.share.clone() else { return };
    if !(next.finished && next.outcome == Some(Outcome::Succeeded)) {
        return;
    }
    let Some(parent) = user_task(&finished.config) else { return };
    let Some(report) = super::reports::final_report(run).ok().flatten() else { return };
    if run.shared() {
        return;
    }
    let sh = sh.clone();
    let run = run.clone();
    tokio::spawn(async move {
        let sid = sh.dir.sid().to_string();
        let parent_is_ui = match sh.agent().sessions().lookup(&parent).await {
            Ok(Some(e)) => e.kind == SessionKind::Ui,
            Ok(None) => false,
            Err(e) => {
                log::warn!("share: parent {parent} of {sid} unreadable: {e}");
                return;
            }
        };
        if !parent_is_ui || !share.enabled().await {
            return;
        }
        let record = run.record();
        let llm = match create_run_llm(&record, &sh.deps.xllm).await {
            Ok(llm) => llm,
            Err(e) => {
                log::warn!("share: no model client for {sid}: {e}");
                return;
            }
        };
        let artifacts: Vec<ShareArtifact> = report
            .artifacts
            .iter()
            .filter_map(|a| {
                a.obj_id
                    .as_deref()
                    .filter(|id| id.starts_with("cyfile:"))
                    .map(|id| ShareArtifact { name: a.name().to_string(), obj_id: id.to_string() })
            })
            .collect();
        let input = ShareInput {
            session_id: sid.clone(),
            objective: finished.config.session.objective.clone(),
            report: report.report.clone(),
            artifacts: report.artifacts.iter().map(|a| a.name().to_string()).collect(),
            cmd_skills: finished.config.prompt.cmd_skills.clone(),
            turns: finished.turns,
        };
        let post = match compose(&llm, &record.config.model, &input, artifacts).await {
            Ok(Some(post)) => post,
            Ok(None) => {
                let _ = run.mark_shared();
                return;
            }
            Err(e) => {
                log::warn!("share: compose for {sid} failed: {e}");
                return;
            }
        };
        match share.publish(&post).await {
            Ok(receipt) => {
                let _ = run.mark_shared();
                log::info!(
                    "opendan.share.published {}",
                    json!({ "session_id": sid, "key": post.key, "entry": receipt.entry, "obj_id": receipt.obj_id })
                );
                crate::fault::point("share:after_publish");
            }
            Err(e) => log::warn!("share: publish of {} failed: {e}", post.key),
        }
    });
}
