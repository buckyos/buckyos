use std::path::{Component, Path};
use std::sync::Arc;

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, CallingConventions, SessionRuntimeContext, ToolSpec,
};
use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::shared::Shared;
use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::runs::RunHandle;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReportArgs {
    pub report: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(
        default,
        deserialize_with = "crate::protocol::report::report_json_value"
    )]
    pub result: Option<Value>,
    #[serde(default)]
    pub is_end: bool,
}

pub(super) struct ReportTool;

#[async_trait]
impl AgentTool for ReportTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_REPORT.into(),
            description: "Submit a progress report, or finish this context with is_end=true. Final reports require all tasks and child work to be settled. Accepted completion skips remaining calls in the batch. Artifacts are explicit files relative to the work directory; result is optional application JSON. Reports are delivered by the host, not sent as messages by this tool.".into(),
            args_schema: json!({"type":"object", "additionalProperties":false, "properties": {
                "report":{"type":"string","minLength":1},
                "artifacts":{"type":"array","items":{"type":"string"}},
                "result":{}, "is_end":{"type":"boolean","default":false}
            },"required":["report"]}),
            output_schema: json!({"type":"object"}),
            usage: Some("report({report: \"...\", artifacts: [\"output.txt\"], is_end: true})".into()),
        }
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }
    async fn call(
        &self,
        _: &SessionRuntimeContext,
        _: Value,
    ) -> std::result::Result<AgentToolResult, AgentToolError> {
        Err(AgentToolError::ExecFailed(
            "report requires the Session host executor".into(),
        ))
    }
}

fn invalid(message: impl Into<String>) -> OpenDanError {
    OpenDanError::InvalidArgument(message.into())
}

fn artifact_bytes(workdir: &Path, name: &str) -> Result<Vec<u8>> {
    let path = Path::new(name);
    if name.is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid(
            "artifact paths must be relative files without '.' or '..'",
        ));
    }
    let root = workdir
        .canonicalize()
        .map_err(|e| invalid(format!("artifact work directory: {e}")))?;
    let abs = root.join(path);
    let canonical = abs
        .canonicalize()
        .map_err(|e| invalid(format!("artifact {name}: {e}")))?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        return Err(invalid(format!(
            "artifact is not a file inside the work directory: {name}"
        )));
    }
    std::fs::read(&canonical).map_err(|e| invalid(format!("artifact {name}: {e}")))
}

pub(super) async fn validate(sh: &Shared, run: &RunHandle, args: &ReportArgs) -> Result<()> {
    sh.lease.check()?;
    if args.report.trim().is_empty() {
        return Err(invalid("report must not be blank"));
    }
    let mut seen = std::collections::HashSet::new();
    for path in &args.artifacts {
        if !seen.insert(path) {
            return Err(invalid("duplicate artifact path"));
        }
        artifact_bytes(Path::new(&run.record().workdir), path)?;
    }
    if !args.is_end {
        return Ok(());
    }
    let child = super::outcome::call_site_of(sh, run.run_id()).await.0;
    let s = sh.session.lock().await;
    if s.state.stop_requested || sh.deps.stop.requested() || s.state.run_state == RunState::Finished
    {
        return Err(invalid("session stop or completion already requested"));
    }
    if !run.noted_tasks().is_empty() || (!child && !s.state.watched_tasks.is_empty()) {
        return Err(invalid(
            "cannot finish while tasks are still active; wait for their results",
        ));
    }
    drop(s);
    if !child && !super::children::unsettled_children(sh).await?.is_empty() {
        return Err(invalid("cannot finish while child sessions are unsettled"));
    }
    Ok(())
}

fn same_args(old: &ReportSubmission, args: &ReportArgs) -> bool {
    old.report == args.report
        && old.is_end == args.is_end
        && old.result == args.result
        && old
            .artifacts
            .iter()
            .map(|a| &a.path)
            .eq(args.artifacts.iter())
}

pub(super) async fn submit(
    sh: &Shared,
    run: &RunHandle,
    source: ReportSource,
    args: ReportArgs,
) -> Result<ReportSubmission> {
    let id = crate::ids::h(&[
        run.run_id(),
        &serde_json::to_string(&source).map_err(|e| invalid(e.to_string()))?,
    ]);
    let reports = run.reports()?;
    if let Some(old) = reports.iter().find(|r| r.id == id) {
        return if same_args(old, &args) {
            Ok(old.clone())
        } else {
            Err(invalid(
                "report identity was already submitted with different arguments",
            ))
        };
    }
    if reports.iter().any(|r| r.is_end) {
        return Err(invalid("this context already has a final report"));
    }
    validate(sh, run, &args).await?;
    let workdir = run.record().workdir;
    let mut artifacts = Vec::new();
    for (index, path) in args.artifacts.iter().enumerate() {
        let bytes = artifact_bytes(Path::new(&workdir), path)?;
        let digest = hex::encode(Sha256::digest(&bytes));
        let reference = format!("{STATE_DIR}/reports/{id}/{index}-{digest}");
        crate::fsutil::atomic_replace(&sh.dir.path().join(&reference), &bytes)?;
        artifacts.push(ReportArtifact {
            path: path.clone(),
            reference,
            digest,
        });
    }
    let submission = ReportSubmission {
        id,
        run_id: run.run_id().to_string(),
        source,
        report: args.report,
        artifacts,
        result: args.result,
        is_end: args.is_end,
        accepted_at_ms: crate::now_ms(),
    };
    run.save_report(&submission)?;
    crate::fault::point("report:after_persist");
    sync_latest(sh, run).await?;
    Ok(submission)
}

pub(super) async fn sync_latest(sh: &Shared, run: &RunHandle) -> Result<()> {
    if super::outcome::call_site_of(sh, run.run_id()).await.0 {
        return Ok(());
    }
    let Some(latest) = run.reports()?.last().cloned() else {
        return Ok(());
    };
    let mut s = sh.session.lock().await;
    if s.state.latest_report.as_ref() != Some(&latest) {
        s.state.latest_report = Some(latest);
        s.commit_state(&sh.lease)?;
    }
    Ok(())
}

pub(super) fn final_report(run: &RunHandle) -> Result<Option<ReportSubmission>> {
    Ok(run.reports()?.into_iter().find(|r| r.is_end))
}

pub(super) async fn call(
    sh: &Arc<Shared>,
    run: &RunHandle,
    call: &AiToolCall,
) -> Result<Observation> {
    let parsed = serde_json::from_value::<ReportArgs>(super::flush::canonical_args(&call.args));
    let result = match parsed {
        Ok(args) => {
            submit(
                sh,
                run,
                ReportSource::Tool {
                    call_id: call.call_id.clone(),
                },
                args,
            )
            .await
        }
        Err(e) => Err(invalid(e.to_string())),
    };
    match result {
        Ok(report) => {
            if report.is_end {
                if let Some(handle) = sh.interrupt.lock().expect("interrupt").as_ref() {
                    handle.finish("accepted final report");
                }
            }
            let content = json!({"report_id":report.id,"accepted":true,"is_end":report.is_end});
            Ok(Observation::Success {
                call_id: call.call_id.clone(),
                bytes: content.to_string().len(),
                content,
                truncated: false,
                tool_result: None,
            })
        }
        Err(OpenDanError::InvalidArgument(message)) => Ok(Observation::Error {
            call_id: call.call_id.clone(),
            message,
            tool_result: None,
        }),
        Err(e) => Err(e),
    }
}

pub(super) async fn sync_xml(
    sh: &Shared,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
) -> Result<()> {
    let mut steps: Vec<_> = snapshot.state.steps.iter().collect();
    if let Some(last) = snapshot.state.last_step.as_ref() {
        if !steps
            .iter()
            .any(|s| s.meta.step_index == last.meta.step_index)
        {
            steps.push(last);
        }
    }
    if let Some(action) = snapshot.state.action_step.as_ref() {
        if !steps
            .iter()
            .any(|s| s.meta.step_index == action.step.meta.step_index)
        {
            steps.push(&action.step);
        }
    }
    steps.sort_by_key(|s| s.meta.step_index);
    for step in steps {
        if step.meta.step_index < super::receipts::snapshot_host_meta(snapshot).inherited_below {
            continue;
        }
        if let Some(report) = &step.self_report {
            submit(
                sh,
                run,
                ReportSource::Behavior {
                    step_index: step.meta.step_index,
                },
                ReportArgs {
                    report: report.clone(),
                    artifacts: step.report_artifacts.clone(),
                    result: step.report_result.clone(),
                    is_end: step.report_end,
                },
            )
            .await?;
        }
    }
    sync_latest(sh, run).await
}
