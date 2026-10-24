use std::path::{Component, Path};
use std::sync::Arc;

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, CallingConventions, SessionRuntimeContext, ToolSpec,
};
use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;
use ndn_lib::ObjId;
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

/// The host's object store: report artifacts leave a session as named
/// objects. A file artifact is put in when its report is accepted.
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Put the file at `path` in as an object named `name`.
    async fn put_file(&self, path: &Path, name: &str) -> std::result::Result<ObjId, String>;

    /// The name an object carries (a file object's name), when known.
    async fn name_of(&self, obj_id: &ObjId) -> Option<String> {
        let _ = obj_id;
        None
    }
}

/// An artifact given as an object id rather than a file: no path
/// separator or whitespace, and a valid id (`cyfile:…`, `chunk:…`).
fn artifact_obj_id(artifact: &str) -> Option<ObjId> {
    if artifact.contains(['/', '\\']) || artifact.chars().any(char::is_whitespace) {
        return None;
    }
    ObjId::new(artifact).ok()
}

pub(super) struct ReportTool;

#[async_trait]
impl AgentTool for ReportTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_REPORT.into(),
            description: "Submit a progress report, or finish this context with is_end=true. Final reports require all tasks and child work to be settled. Accepted completion skips remaining calls in the batch. Artifacts are explicit files relative to the work directory, or object ids (e.g. `cyfile:…` from another report); the host delivers them with the final report as attachments. Result is optional application JSON. Reports are delivered by the host, not sent as messages by this tool.".into(),
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
        if artifact_obj_id(path).is_some() {
            continue;
        }
        let cfg = sh.session.lock().await.config.clone();
        let workdir = crate::runtime::resolve_workdir(&sh.dir, &cfg, sh.agent_root.as_deref())?;
        artifact_bytes(&workdir, path)?;
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
    let goal = super::goal::is_goal(&s.config);
    drop(s);
    // A Goal Activation ends while its Work runs on: the Work's end is a
    // later Activation's input.
    if !child && !goal && !super::children::unsettled_children(sh).await?.is_empty() {
        return Err(invalid("cannot finish while child sessions are unsettled"));
    }
    // The session's result rests on its TodoList: every Todo converged
    // (executed and reviewed, or cancelled with a reason).
    if !child && !goal {
        let open = super::todos::unconverged(sh);
        if !open.is_empty() {
            return Err(invalid(format!(
                "cannot finish while the todo list has not converged: {}. Review results with `agent-session todo review`, cancel what will not be done with `agent-session todo cancel <id> --reason <why>`.",
                open.join("; ")
            )));
        }
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
    // The final report of a Goal Activation carries the Goal result (a
    // behavior-loop report is taken as it is; a wrong result is recorded).
    let child = super::outcome::call_site_of(sh, run.run_id()).await.0;
    if args.is_end
        && !child
        && matches!(source, ReportSource::Tool { .. })
        && super::goal::is_goal(&sh.session.lock().await.config)
    {
        match &args.result {
            Some(v) => {
                GoalResult::parse(v).map_err(invalid)?;
                super::consolidation::check_final_report(sh, v).await?;
            }
            None => {
                return Err(invalid(
                    "a goal activation ends with result={decision: no_action | acted | created_work | needs_info, distance?, pending?, questions?, skills?}",
                ))
            }
        }
    }
    let artifacts = store_artifacts(sh, &id, &args.artifacts).await?;
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

/// Artifacts leaving the session under `id` (a report, a structured reply):
/// an object id as it is; a file of the work directory copied to a stable
/// place (`state/reports/<id>/…`) and put into the host's object store.
pub(super) async fn store_artifacts(
    sh: &Shared,
    id: &str,
    paths: &[String],
) -> Result<Vec<ReportArtifact>> {
    let cfg = sh.session.lock().await.config.clone();
    let workdir = crate::runtime::resolve_workdir(&sh.dir, &cfg, sh.agent_root.as_deref())?;
    let mut artifacts = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if let Some(obj_id) = artifact_obj_id(path) {
            artifacts.push(ReportArtifact {
                path: path.clone(),
                reference: String::new(),
                digest: String::new(),
                obj_id: Some(obj_id.to_string()),
            });
            continue;
        }
        let bytes = artifact_bytes(Path::new(&workdir), path)?;
        let digest = hex::encode(Sha256::digest(&bytes));
        let reference = format!("{STATE_DIR}/reports/{id}/{index}-{digest}");
        let stable = sh.dir.path().join(&reference);
        crate::fsutil::atomic_replace(&stable, &bytes)?;
        let mut artifact = ReportArtifact {
            path: path.clone(),
            reference,
            digest,
            obj_id: None,
        };
        // The stable copy is put in: it does not change after acceptance.
        // Storing is the host's business; the report stands without it.
        if let Some(store) = &sh.deps.objects {
            match store.put_file(&stable, artifact.name()).await {
                Ok(obj_id) => artifact.obj_id = Some(obj_id.to_string()),
                Err(e) => log::warn!("session {}: artifact {path} not stored: {e}", sh.dir.sid()),
            }
        }
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

/// The host check of a reply under a reply contract (`[reply]`): what the
/// schema cannot say. `artifacts` (when the reply has them) must be object
/// ids or files of the work directory, each once; the reply contract's
/// `asks_field` must list every question this context asked the user.
pub(super) struct ReplyValidator {
    pub sh: Arc<Shared>,
    pub run: RunHandle,
}

/// run.json `host.extra` key: the reply contract of the run's context
/// (`{asks_field}`), kept for a resumed run.
pub(super) const EXTRA_REPLY: &str = "reply";

#[async_trait]
impl llm_context::OutputValidator for ReplyValidator {
    async fn validate(&self, reply: &Value) -> std::result::Result<(), String> {
        if let Some(list) = reply.get("artifacts").and_then(Value::as_array) {
            let cfg = self.sh.session.lock().await.config.clone();
            let workdir =
                crate::runtime::resolve_workdir(&self.sh.dir, &cfg, self.sh.agent_root.as_deref())
                    .map_err(|e| e.to_string())?;
            let mut seen = std::collections::HashSet::new();
            for a in list {
                let Some(path) = a.as_str() else {
                    return Err("artifacts must be strings".into());
                };
                if !seen.insert(path) {
                    return Err(format!("artifact `{path}` is listed twice"));
                }
                if artifact_obj_id(path).is_none() {
                    artifact_bytes(Path::new(&workdir), path).map_err(|e| {
                        format!("{e}; an artifact is a file you made (path relative to your working directory, created first) or an object id")
                    })?;
                }
            }
        }
        let record = self.run.record();
        let extra = record.host.as_ref().map(|h| &h.extra);
        if let Some(field) = extra
            .and_then(|x| x.get(EXTRA_REPLY))
            .and_then(|r| r.get("asks_field"))
            .and_then(Value::as_str)
        {
            let asked = extra
                .and_then(|x| x.get("asks"))
                .and_then(Value::as_object)
                .map(|a| a.len())
                .unwrap_or(0);
            let listed = reply.get(field).and_then(Value::as_array).map_or(0, Vec::len);
            if listed != asked {
                return Err(format!(
                    "`{field}` must list every question you asked the user with ask-user in this context, one entry each with the user's answer: you asked {asked}, it lists {listed}"
                ));
            }
        }
        Ok(())
    }
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
