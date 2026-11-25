use std::path::{Component, Path};
use std::sync::Arc;

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, CallingConventions, SessionRuntimeContext, ToolSpec,
};
use agent_tool::runtime::node::RemoteNodeTarget;
use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;
use ndn_lib::ObjId;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::shared::Shared;
use crate::deliveries::{DeliveryRecord, DeliverySource};
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
    pub outcome: Option<ReportOutcome>,
}

impl ReportArgs {
    pub(super) fn from_behavior(
        report: String,
        is_end: bool,
        outcome: Option<&str>,
        payload: Option<&Value>,
    ) -> Result<Self> {
        let (artifacts, result) = agent_tool::xllm::split_report_result(payload);
        let outcome = outcome
            .map(|s| serde_json::from_value(Value::String(s.to_owned())))
            .transpose()
            .map_err(|e| invalid(e.to_string()))?;
        Ok(Self { report, artifacts, result, is_end, outcome })
    }
}

/// An object leaving a session as the host keeps it: its id and the
/// `obj://bucket/...` binding that retains it (`None`: no bucket keeps it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub obj_id: ObjId,
    pub url: Option<String>,
}

/// The host's object store: report artifacts leave a session as named
/// objects. A file artifact is put in when its report is accepted.
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Put the file at `path` in as an object named `name`; `Ok` means the
    /// host retains it.
    async fn put_file(&self, path: &Path, name: &str) -> std::result::Result<StoredObject, String>;

    /// Retain what an `obj://bucket/...` reference binds, the way `put_file`
    /// retains a file.
    async fn retain_url(&self, url: &str) -> std::result::Result<StoredObject, String> {
        Err(format!("`{url}` cannot be attached: the host keeps no bucket objects"))
    }

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

/// An artifact given as a bucket object (`obj://bucket/<bucket_id>/<key>`).
pub(super) fn is_bucket_url(artifact: &str) -> bool {
    buckyos_api::parse_bucket_obj_url(artifact).is_some()
}

/// How the model leaves the schema when it cannot meet it.
const COMPAT_HINT: &str = "If you cannot give it in this format, switch this conversation to the compatible reply mode: run `agent-session ctl reply-mode compat` with shell, then reply with the plain text for the user (no JSON). It stays on until `agent-session ctl reply-mode schema`.";

pub(super) struct SubmitReplyTool;

#[async_trait]
impl AgentTool for SubmitReplyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_SUBMIT_REPLY.into(),
            description: "Compatible reply mode only (`agent-session ctl reply-mode compat`): give your reply as one call, and the Turn ends with it. Use it when you deliver files (`artifacts`), or when the topic became clear or changed (`topic`): a topic left stale keeps related memory from reaching this conversation. Any other reply is plain final text. Tasks that run go on.".into(),
            args_schema: json!({"type":"object", "additionalProperties":false, "properties": {
                "reply":{"type":"string","description":"What is read: your answer, self-contained."},
                "topic":{"type":["object","null"],"additionalProperties":false,"required":["title","tags"],"properties":{
                    "title":{"type":"string"},
                    "tags":{"type":"array","items":{"type":"string"}}
                }},
                "artifacts":{"type":"array","items":{"type":"string"},"description":"Files delivered with the reply: paths relative to the working directory, or object ids."}
            },"required":["reply"]}),
            output_schema: json!({"type":"object"}),
            usage: Some("submit_reply({reply: \"...\", topic: {title: \"...\", tags: [\"...\"]}, artifacts: [\"out.txt\"]})".into()),
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
            "submit_reply requires the Session host executor".into(),
        ))
    }
}

/// `submit_reply`: accepted, it is the reply of the Turn and the run ends;
/// what still runs is followed by the session, as after any reply.
pub(super) async fn submit_reply(
    sh: &Arc<Shared>,
    run: &RunHandle,
    call: &AiToolCall,
) -> Result<Observation> {
    match accept_reply(sh, run, call).await {
        Ok(id) => {
            if let Some(handle) = sh.interrupt.lock().expect("interrupt").as_ref() {
                handle.finish("accepted reply");
            }
            let content = json!({"reply_id": id, "accepted": true});
            Ok(Observation::Success {
                call_id: call.call_id.clone(),
                bytes: content.to_string().len(),
                content,
                truncated: false,
                host: None,
                images: Vec::new(),
            })
        }
        Err(OpenDanError::InvalidArgument(message) | OpenDanError::Artifact(message)) => Ok(Observation::Error {
            call_id: call.call_id.clone(),
            message,
            host: None,
        }),
        Err(e) => Err(e),
    }
}

async fn accept_reply(sh: &Arc<Shared>, run: &RunHandle, call: &AiToolCall) -> Result<String> {
    if super::outcome::call_site_of(sh, run.run_id()).await.0 {
        return Err(invalid("only the conversation itself submits its reply; finish this branch with its own result"));
    }
    let source = ReportSource::Reply {
        call_id: call.call_id.clone(),
    };
    let id = crate::ids::h(&[
        run.run_id(),
        &serde_json::to_string(&source).map_err(|e| invalid(e.to_string()))?,
    ]);
    let reports = run.reports()?;
    if reports.iter().any(|r| r.id == id) {
        return Ok(id);
    }
    if reports.iter().any(|r| r.is_end) {
        return Err(invalid("this Turn already has its reply"));
    }
    // A mode change of the same batch is still queued.
    super::inputs::route_inputs(sh, true).await?;
    {
        let s = sh.session.lock().await;
        if s.state.reply_mode != ReplyMode::Compat {
            return Err(invalid(
                "submit_reply is for the compatible reply mode: run `agent-session ctl reply-mode compat` first, then call it again; otherwise reply with the JSON of the reply format",
            ));
        }
        if s.state.stop_requested || s.state.turn_stop_requested || sh.deps.stop.requested() {
            return Err(invalid("a stop of this Turn is requested already"));
        }
    }
    let mut reply = super::flush::canonical_args(&call.args);
    if let Some(args) = reply.as_object_mut() {
        args.entry("topic").or_insert(Value::Null);
        args.entry("artifacts").or_insert_with(|| json!([]));
    }
    let snapshot = run.latest_snapshot()?;
    if let Some(llm_context::OutputSpec::Json { schema: Some(schema), .. }) =
        snapshot.as_ref().map(|s| &s.request.output)
    {
        llm_context::json_check::validate(schema, &reply).map_err(invalid)?;
    }
    let validator = ReplyValidator {
        sh: sh.clone(),
        run: run.clone(),
    };
    llm_context::OutputValidator::validate(&validator, &reply)
        .await
        .map_err(invalid)?;
    let text = reply["reply"].as_str().unwrap_or_default().trim().to_string();
    let paths: Vec<String> = reply["artifacts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if text.is_empty() && paths.is_empty() {
        return Err(invalid("the reply is empty"));
    }
    let artifacts = store_artifacts(sh, &id, &paths, DeliverySource::Reply).await?;
    run.save_report(&ReportSubmission {
        id: id.clone(),
        run_id: run.run_id().to_string(),
        source,
        report: text,
        artifacts,
        result: Some(json!({ "topic": reply["topic"] })),
        is_end: true,
        outcome: None,
        accepted_at_ms: crate::now_ms(),
    })?;
    crate::fault::point("report:after_persist");
    Ok(id)
}

pub(super) struct ReportTool;

#[async_trait]
impl AgentTool for ReportTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_REPORT.into(),
            description: "Submit a progress report, or finish this context with is_end=true. On a top-level Work session’s final report, set outcome to succeeded when the objective was verified, blocked when a prerequisite is missing, failed when the work or its verification failed. Only succeeded releases dependent Todos. Finish or cancel running tasks before the final report. Other contexts and progress reports omit outcome. Accepted completion skips remaining calls in the batch. Artifacts are explicit files relative to the work directory, bucket objects (`obj://bucket/…` a command returned) or object ids (e.g. `cyfile:…` from another report); the host keeps them in the agent's `artifacts` bucket and delivers them with the final report as attachments. Result is optional application JSON. Reports are delivered by the host, not sent as messages by this tool.".into(),
            args_schema: json!({"type":"object", "additionalProperties":false, "properties": {
                "report":{"type":"string","minLength":1},
                "artifacts":{"type":"array","items":{"type":"string"}},
                "result":{}, "is_end":{"type":"boolean","default":false},
                "outcome":{"type":"string","enum":["succeeded","failed","blocked"]}
            },"required":["report"]}),
            output_schema: json!({"type":"object"}),
            usage: Some("report({report: \"...\", artifacts: [\"output.txt\"], is_end: true, outcome: \"succeeded\"})".into()),
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

fn validate_artifact_path(name: &str) -> Result<()> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains(':')
        || name.contains('\\')
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid(
            "artifact paths must be relative files without '.' or '..'",
        ));
    }
    Ok(())
}

fn remote_artifact_path(sh: &Shared, name: &str) -> Result<String> {
    validate_artifact_path(name)?;
    let workdir = crate::runtime::target_workdir(&sh.dir)?
        .ok_or_else(|| invalid("the session has no working directory on its runtime yet"))?;
    Ok(crate::runtime::target_join(&workdir, &[name]))
}

async fn remote_artifact_target(sh: &Shared, cfg: &SessionConfig) -> Result<RemoteNodeTarget> {
    let frozen = cfg.runtime.frozen.as_ref().ok_or_else(|| invalid("missing frozen runtime"))?;
    let config = crate::runtime::node_config_of(&frozen.runtime_id, &frozen.exec())?;
    let (connector, config) = match sh.deps.xllm.node_executor.clone() {
        Some(connector) => (connector, config),
        None => {
            let (connector, config, _) = crate::runtime::connector_for(&frozen.runtime_id, &config).await?;
            (connector, config)
        }
    };
    let node = config.remote_node.ok_or_else(|| invalid("missing node executor configuration"))?;
    let target = RemoteNodeTarget::open(
        connector,
        node.connect_target().unwrap_or_default(),
        node.profile_id.as_deref().unwrap_or_default(),
        &frozen.runtime_id,
        sh.dir.state_dir().join("runtime_ops"),
    ).await.map_err(|e| invalid(format!("artifact runtime: {e}")))?;
    let binding = sh.dir.binding_opt()?.ok_or_else(|| invalid("artifact runtime is not bound"))?;
    let observed = serde_json::to_value(&target.described().target).map_err(|e| invalid(e.to_string()))?;
    for key in ["node_did", "profile_id", "uid", "gid", "store_id", "protocol_version", "workdir_roots"] {
        if binding.target.get(key) != observed.get(key) {
            return Err(invalid(format!("artifact runtime binding changed: {key}")));
        }
    }
    if binding.target.get("identity_mode") != Some(&json!(target.described().info.identity_mode)) {
        return Err(invalid("artifact runtime binding changed: identity_mode"));
    }
    Ok(target)
}

fn artifact_bytes(workdir: &Path, name: &str) -> Result<Vec<u8>> {
    validate_artifact_path(name)?;
    let path = Path::new(name);
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
        if artifact_obj_id(path).is_some() || is_bucket_url(path) {
            continue;
        }
        let cfg = sh.session.lock().await.config.clone();
        if cfg.runtime.is_node() {
            remote_artifact_path(sh, path)?;
            continue;
        }
        let workdir = crate::runtime::resolve_workdir(&sh.dir)?;
        artifact_bytes(&workdir, path)?;
    }
    if !args.is_end {
        if args.outcome.is_some() {
            return Err(invalid("outcome is only valid on a final report (is_end=true)"));
        }
        return Ok(());
    }
    let child = super::outcome::call_site_of(sh, run.run_id()).await.0;
    // Sub sessions (`session:`) are settled by the check below. A task the
    // run handed work to counts until it ended (its end may already have
    // come back as input); one nobody can answer for, until that unknown
    // end was delivered.
    let resolver = sh.tasks.lock().expect("tasks").clone();
    let recent = sh.session.lock().await.state.recent_keys.clone();
    let mut running = Vec::new();
    for t in run.noted_tasks() {
        if t.starts_with(super::children::SESSION_TASK_PREFIX) {
            continue;
        }
        let ended = match &resolver {
            Some(r) => match r.state(&t).await {
                agent_tool::TaskState::Finished(_) => true,
                agent_tool::TaskState::Running { .. } => false,
                agent_tool::TaskState::Unknown { .. } => {
                    recent.contains(&super::receipts::internal_task_key(&t))
                }
            },
            None => false,
        };
        if !ended {
            running.push(t);
        }
    }
    let s = sh.session.lock().await;
    if !child && s.config.session.kind == SessionKind::Work {
        if s.config.session.policy.completion == CompletionPolicy::ExplicitReport && args.outcome.is_none() {
            return Err(invalid("a final Work report requires outcome: succeeded, failed or blocked"));
        }
    } else if args.outcome.is_some() {
        return Err(invalid("outcome is only valid on a top-level Work session's final report"));
    }
    if s.state.stop_requested || sh.deps.stop.requested() || s.state.run_state == RunState::Finished
    {
        return Err(invalid("session stop or completion already requested"));
    }
    if !child {
        for t in &s.state.watched_tasks {
            if !running.contains(t) {
                running.push(t.clone());
            }
        }
    }
    if !running.is_empty() {
        if super::goal::is_goal(&s.config) {
            return Err(invalid(format!(
                "this activation still follows what it started ({}): an activation follows its own commands to their end, within its wall clock limit (activation_max_wall_secs), and their ends come back to you as `<<task_result>>`. Cancel what you no longer need (`agent-session task cancel <task_id>`); anything long belongs in a work session",
                running.join(", ")
            )));
        }
        return Err(invalid(format!(
            "cannot finish while tasks you started are still active ({}): their ends come back to you as `<<task_result>>`; reply to close this Turn meanwhile, or cancel what you no longer need",
            running.join(", ")
        )));
    }
    drop(s);
    // The Todo this work session executes changed while it worked: the
    // change comes first (its event opens the next Turn).
    if !child && super::inputs::todo_update_queued(sh).await {
        return Err(invalid(
            "your Todo was updated while you worked and the update waits as your next input: do not finish yet. Reply with one sentence on where things stand (no final report); that ends this Turn and the update opens the next one",
        ));
    }
    if !child && super::inputs::unread_messages(sh).await {
        return Err(invalid(
            "a message for you arrived while you worked and is not read yet: do not finish. It is your next input; take it in, then report again",
        ));
    }
    Ok(())
}

fn same_args(old: &ReportSubmission, args: &ReportArgs) -> bool {
    old.report == args.report
        && old.is_end == args.is_end
        && old.outcome == args.outcome
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
    let artifacts = store_artifacts(sh, &id, &args.artifacts, DeliverySource::Report).await?;
    let submission = ReportSubmission {
        id,
        run_id: run.run_id().to_string(),
        source,
        report: args.report,
        artifacts,
        result: args.result,
        is_end: args.is_end,
        outcome: args.outcome,
        accepted_at_ms: crate::now_ms(),
    };
    run.save_report(&submission)?;
    crate::fault::point("report:after_persist");
    sync_latest(sh, run).await?;
    Ok(submission)
}

/// Artifacts leaving the session under `id` (a report, a structured reply):
/// an object id as it is; a bucket object retained by the host's store; a
/// file of the work directory copied to a stable place
/// (`state/reports/<id>/…`) and put into the host's object store, the copy
/// removed once a bucket retains it.
pub(super) async fn store_artifacts(
    sh: &Shared,
    id: &str,
    paths: &[String],
    source: DeliverySource,
) -> Result<Vec<ReportArtifact>> {
    let (artifacts, sizes) = store_objects(sh, id, paths).await?;
    index_deliveries(sh, &artifacts, &sizes, source).await;
    Ok(artifacts)
}

/// The UI Session at the top of `sid`'s origin chain and its conversation.
async fn conversation_of(sh: &Shared, sid: &str) -> (Option<String>, Option<String>) {
    let mut at = sid.to_string();
    for _ in 0..16 {
        let Ok(Some(entry)) = sh.deps.agent.sessions().lookup(&at).await else {
            break;
        };
        if entry.kind == SessionKind::Ui {
            return (Some(entry.session_id), entry.route_key);
        }
        match entry.origin.and_then(|o| o.parent_session) {
            Some(parent) => at = parent,
            None => break,
        }
    }
    (None, None)
}

/// One line of the delivery index per artifact that became an object. A
/// host without an object store (`--dev`) or without the AgentRoot writes
/// none; a line that cannot be written is not the delivery's failure.
async fn index_deliveries(sh: &Shared, artifacts: &[ReportArtifact], sizes: &[Option<u64>], source: DeliverySource) {
    let (Some(store), Some(root)) = (sh.deps.objects.as_ref(), sh.deps.agent.agent_root()) else {
        return;
    };
    let delivered: Vec<(&ReportArtifact, Option<u64>)> = artifacts
        .iter()
        .zip(sizes.iter().copied())
        .filter(|(a, _)| a.obj_id.is_some())
        .collect();
    if delivered.is_empty() {
        return;
    }
    let sid = sh.dir.sid().to_string();
    let (ui_session_id, route_key) = conversation_of(sh, &sid).await;
    for (artifact, size) in delivered {
        let obj_id = artifact.obj_id.clone().unwrap_or_default();
        let name = match ObjId::new(&obj_id) {
            Ok(id) if artifact.reference.is_empty() && artifact.url.is_none() => {
                store.name_of(&id).await.unwrap_or_else(|| artifact.name().to_string())
            }
            _ => artifact.name().to_string(),
        };
        let record = DeliveryRecord {
            at_ms: crate::now_ms(),
            mime: crate::deliveries::mime_of(&name),
            name,
            obj_id,
            url: artifact.url.clone(),
            size,
            session_id: sid.clone(),
            ui_session_id: ui_session_id.clone(),
            route_key: route_key.clone(),
            source,
        };
        if let Err(e) = crate::deliveries::record(root, &record) {
            log::warn!("session {sid}: delivery of {} not indexed: {e}", record.name);
        }
    }
}

/// The artifacts as the host keeps them, and the size of each one that was
/// a file.
async fn store_objects(
    sh: &Shared,
    id: &str,
    paths: &[String],
) -> Result<(Vec<ReportArtifact>, Vec<Option<u64>>)> {
    let cfg = sh.session.lock().await.config.clone();
    let workdir = crate::runtime::resolve_workdir(&sh.dir)?;
    let mut artifacts = Vec::new();
    let mut sizes = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if let Some(obj_id) = artifact_obj_id(path) {
            artifacts.push(ReportArtifact {
                path: path.clone(),
                reference: String::new(),
                digest: String::new(),
                obj_id: Some(obj_id.to_string()),
                url: None,
            });
            sizes.push(None);
            continue;
        }
        if is_bucket_url(path) {
            let store = sh
                .deps
                .objects
                .as_ref()
                .ok_or_else(|| invalid(format!("artifact {path}: the host has no object store")))?;
            let stored = store
                .retain_url(path)
                .await
                .map_err(|e| OpenDanError::Artifact(format!("artifact {path} could not be attached: {e}")))?;
            artifacts.push(ReportArtifact {
                path: path.clone(),
                reference: String::new(),
                digest: String::new(),
                obj_id: Some(stored.obj_id.to_string()),
                url: stored.url,
            });
            sizes.push(None);
            continue;
        }
        let (digest, staging) = if cfg.runtime.is_node() {
            let remote_path = remote_artifact_path(sh, path)?;
            let target = remote_artifact_target(sh, &cfg).await?;
            let parent = sh.dir.state_dir().join("reports").join(id);
            std::fs::create_dir_all(&parent).map_err(|e| OpenDanError::io(&parent, e))?;
            let staging = parent.join(format!(".{index}.download"));
            let root = crate::runtime::target_workdir(&sh.dir)?
                .ok_or_else(|| invalid("missing artifact working directory"))?;
            let downloaded = target.download_file(&root, &remote_path, &staging).await;
            match downloaded {
                Ok(file) => (file.hash.trim_start_matches("sha256:").to_string(), Some(staging)),
                Err(e) => {
                    let _ = std::fs::remove_file(staging);
                    return Err(invalid(format!("artifact {path}: {e}")));
                }
            }
        } else {
            let bytes = artifact_bytes(Path::new(&workdir), path)?;
            let digest = hex::encode(Sha256::digest(&bytes));
            let reference = format!("{STATE_DIR}/reports/{id}/{index}-{digest}");
            crate::fsutil::atomic_replace(&sh.dir.path().join(&reference), &bytes)?;
            (digest, None)
        };
        let reference = format!("{STATE_DIR}/reports/{id}/{index}-{digest}");
        let stable = sh.dir.path().join(&reference);
        if let Some(staging) = staging {
            std::fs::rename(&staging, &stable).map_err(|e| OpenDanError::io(&stable, e))?;
            crate::fsutil::fsync_dir(stable.parent().unwrap())?;
        }
        sizes.push(std::fs::metadata(&stable).ok().map(|m| m.len()));
        let mut artifact = ReportArtifact {
            path: path.clone(),
            reference,
            digest,
            obj_id: None,
            url: None,
        };
        if let Some(store) = &sh.deps.objects {
            let stored = store
                .put_file(&stable, artifact.name())
                .await
                .map_err(|e| OpenDanError::Artifact(format!("artifact {path} could not be attached: {e}")))?;
            artifact.obj_id = Some(stored.obj_id.to_string());
            artifact.url = stored.url;
            if artifact.url.is_some() {
                if let Err(e) = std::fs::remove_file(&stable) {
                    log::warn!("artifact {path}: stable copy {} not removed: {e}", stable.display());
                }
            }
        }
        artifacts.push(artifact);
    }
    Ok((artifacts, sizes))
}

/// The host check of a reply under a reply contract (`[reply]`): what the
/// schema cannot say. `artifacts` (when the reply has them) must be object
/// ids or files of the work directory, each once; the checks of the named
/// `contract` follow.
pub(super) struct ReplyValidator {
    pub sh: Arc<Shared>,
    pub run: RunHandle,
}

/// run.json `host.extra` key: the reply contract of the run's context
/// (`{asks_field, contract}`), kept for a resumed run.
pub(super) const EXTRA_REPLY: &str = "reply";

fn reply_extra(run: &RunHandle, key: &str) -> Option<Value> {
    run.record()
        .host
        .as_ref()?
        .extra
        .get(EXTRA_REPLY)?
        .get(key)
        .filter(|v| !v.is_null())
        .cloned()
}

impl ReplyValidator {
    /// The run is a conversation's own (not a branch of it).
    async fn conversation(&self) -> bool {
        self.sh.session.lock().await.config.session.kind == SessionKind::Ui
            && !super::outcome::call_site_of(&self.sh, self.run.run_id()).await.0
    }

    /// [`ReplyContract::IntentAnalysis`].
    async fn check_intent(&self, reply: &Value) -> std::result::Result<(), String> {
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if text(reply, "report").is_empty() {
            return Err("`report` must not be empty: it is what the conversation tells the user".into());
        }
        let agent = self.sh.agent();
        let part = |key: &str| reply.get(key).filter(|v| v.is_object());
        match text(reply, "outcome").as_str() {
            "work_created" => {
                let work = part("work").ok_or("outcome `work_created` needs `work`: the session you created")?;
                let id = text(work, "session_id");
                let entry = agent.sessions().lookup(&id).await.map_err(|e| e.to_string())?;
                let mine = entry.is_some_and(|e| {
                    e.origin.as_ref().is_some_and(|o| {
                        o.parent_session.as_deref() == Some(self.sh.dir.sid())
                            && match &o.created_by_call {
                                Some(call) => call.split('/').next() == Some(self.run.run_id()),
                                // Created under a key of its own.
                                None => crate::session::SessionDir::open(&e.location)
                                    .and_then(|d| d.config())
                                    .is_ok_and(|c| {
                                        c.session.created_at_ms >= self.run.record().created_at_ms
                                    }),
                            }
                    })
                });
                if !mine {
                    return Err(format!(
                        "work.session_id `{id}` is not a session this analysis created (`create-worksession` prints its id); a session that was there before is `forwarded` or `direct`"
                    ));
                }
            }
            "goal_created" => {
                let goal = part("goal").ok_or("outcome `goal_created` needs `goal`: the goal you created")?;
                let id = text(goal, "goal_id");
                if agent.goals().get(&id).await.map_err(|e| e.to_string())?.is_none() {
                    return Err(format!("goal.goal_id `{id}` does not exist"));
                }
            }
            "forwarded" => {
                let to = part("forwarded").ok_or("outcome `forwarded` needs `forwarded`: the goal or work session you handed the request to")?;
                let id = text(to, "id");
                let exists = match text(to, "kind").as_str() {
                    "goal" => agent.goals().get(&id).await.map_err(|e| e.to_string())?.is_some(),
                    _ => agent
                        .sessions()
                        .lookup(&id)
                        .await
                        .map_err(|e| e.to_string())?
                        .is_some_and(|e| {
                            e.kind == SessionKind::Work
                                && e.input_queue.is_some()
                                && crate::session::SessionDir::open(&e.location)
                                    .and_then(|d| d.config())
                                    .is_ok_and(|c| c.session.input_policy != InputPolicy::None)
                        }),
                };
                if !exists {
                    return Err(format!("forwarded.id `{id}` is not a goal or a work session that takes messages"));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// What the host adds to a reply under its contract: the questions this
/// context asked the user and their answers, in the notifications' words.
pub(super) async fn complete_reply(sh: &Shared, run: &RunHandle, reply: &mut Value) {
    let Some(field) = reply_extra(run, "asks_field").and_then(|f| f.as_str().map(str::to_string)) else {
        return;
    };
    let asked = match questions_of(sh, run).await {
        Ok(asked) => asked,
        Err(e) => {
            log::warn!("run {}: the questions it asked cannot be read: {e}", run.run_id());
            Vec::new()
        }
    };
    if let Some(o) = reply.as_object_mut() {
        o.insert(field, Value::Array(asked));
    }
}

/// Questions (`ask_user`) of this run, oldest first.
async fn questions_of(sh: &Shared, run: &RunHandle) -> Result<Vec<Value>> {
    use crate::notify::{answer_text, NotifyKind, NotifyListReq};
    let Some(notify) = &sh.deps.notify else {
        return Ok(Vec::new());
    };
    let record = run.record();
    let tasks = record
        .host
        .as_ref()
        .and_then(|h| h.extra.get("task_calls"))
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect::<std::collections::BTreeSet<_>>())
        .unwrap_or_default();
    let mut views = notify
        .list(NotifyListReq {
            kind: Some(NotifyKind::Input),
            session_id: Some(sh.dir.sid().to_string()),
            ..Default::default()
        })
        .await?;
    views.retain(|v| {
        v.task_id.as_ref().is_some_and(|t| tasks.contains(t)) || v.created_at_ms >= record.created_at_ms
    });
    views.sort_by_key(|v| v.created_at_ms);
    Ok(views
        .into_iter()
        .map(|v| {
            let question = v
                .prompt
                .message
                .clone()
                .filter(|m| !m.trim().is_empty())
                .or_else(|| v.prompt.title.clone())
                .unwrap_or_default();
            json!({
                "question": question,
                "answer": answer_text(&v),
                "status": v.status,
                "notify_id": v.notify_id,
            })
        })
        .collect())
}

#[async_trait]
impl llm_context::OutputValidator for ReplyValidator {
    async fn validate(&self, reply: &Value) -> std::result::Result<(), String> {
        if let Some(list) = reply.get("artifacts").and_then(Value::as_array) {
            let cfg = self.sh.session.lock().await.config.clone();
            let workdir =
                crate::runtime::resolve_workdir(&self.sh.dir)
                    .map_err(|e| e.to_string())?;
            let mut seen = std::collections::HashSet::new();
            for a in list {
                let Some(path) = a.as_str() else {
                    return Err("artifacts must be strings".into());
                };
                if !seen.insert(path) {
                    return Err(format!("artifact `{path}` is listed twice"));
                }
                if artifact_obj_id(path).is_none() && !is_bucket_url(path) {
                    if cfg.runtime.is_node() {
                        remote_artifact_path(&self.sh, path).map_err(|e| e.to_string())?;
                        continue;
                    }
                    artifact_bytes(Path::new(&workdir), path).map_err(|e| {
                        format!("{e}; an artifact is a file you made (path relative to your working directory, created first), an `obj://bucket/…` a command returned, or an object id")
                    })?;
                }
            }
        }
        match reply_extra(&self.run, "contract").and_then(|c| serde_json::from_value(c).ok()) {
            Some(ReplyContract::IntentAnalysis) => self.check_intent(reply).await,
            None => Ok(()),
        }
    }

    async fn accepts_text(&self) -> bool {
        self.sh.session.lock().await.state.reply_mode == ReplyMode::Compat && self.conversation().await
    }

    async fn repair_hint(&self) -> Option<String> {
        let schema = self.sh.session.lock().await.state.reply_mode == ReplyMode::Schema;
        (schema && self.conversation().await).then(|| COMPAT_HINT.to_string())
    }
}

pub(super) async fn sync_latest(sh: &Shared, run: &RunHandle) -> Result<()> {
    if super::outcome::call_site_of(sh, run.run_id()).await.0 {
        return Ok(());
    }
    let Some(latest) = run.reports()?.into_iter().rfind(|r| !r.is_reply()) else {
        return Ok(());
    };
    let mut s = sh.session.lock().await;
    if s.state.latest_report.as_ref() != Some(&latest) {
        s.state.latest_report = Some(latest);
        s.commit_state(&sh.lease)?;
    }
    Ok(())
}

/// A Work that reads messages does not end with one unread: its accepted
/// final report becomes a progress report when a message came meanwhile
/// (`true`). Otherwise nobody can post one until the end is committed
/// (`Shared::post_lock`).
pub(super) async fn reopen_for_message(sh: &Shared, run: &RunHandle) -> Result<bool> {
    let stopping = {
        let s = sh.session.lock().await;
        if !s.config.session.policy.reads_messages {
            return Ok(false);
        }
        s.state.stop_requested || s.state.child_call().is_some()
    };
    if stopping {
        return Ok(false);
    }
    let lock = crate::state::lock_posts(&sh.dir.state_dir()).await?;
    if !super::inputs::unread_messages(sh).await {
        *sh.post_lock.lock().expect("post lock") = Some(lock);
        return Ok(false);
    }
    run.reopen_final_report()?;
    sync_latest(sh, run).await?;
    log::info!("session {}: a message came with its final report; it goes on", sh.dir.sid());
    Ok(true)
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
                host: None,
                images: Vec::new(),
            })
        }
        Err(OpenDanError::InvalidArgument(message) | OpenDanError::Artifact(message)) => Ok(Observation::Error {
            call_id: call.call_id.clone(),
            message,
            host: None,
        }),
        Err(e) => Err(e),
    }
}

/// Whether `snapshot` holds a result for the call `call_id` (a native tool
/// result or an action result of a behavior step).
pub(super) fn has_result(snapshot: &LLMContextSnapshot, call_id: &str) -> bool {
    let st = &snapshot.state;
    st.accumulated.iter().any(|m| {
        m.content.iter().any(|c| {
            matches!(c, buckyos_api::AiContent::ToolResult { call_id: id, .. } if id == call_id)
        })
    }) || st
        .steps
        .iter()
        .chain(st.last_step.iter())
        .chain(st.action_step.iter().map(|a| &a.step))
        .any(|step| {
            step.actions
                .iter()
                .take(step.action_results.len())
                .any(|a| a.call_id == call_id)
        })
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
                ReportArgs::from_behavior(
                    report.clone(),
                    step.report_end,
                    step.report_outcome.as_deref(),
                    step.report_result.as_ref(),
                )?,
            )
            .await?;
        }
    }
    sync_latest(sh, run).await
}
