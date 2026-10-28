//! Stage workers (许愿格 §7, §8, §9.2): each takes its own snapshot, builds the stage directory,
//! runs the model (or only the program) outside the Workspace lock, and leaves a validated candidate
//! with a previewed plan. Failures, cancellation and budget exhaustion never touch the Workspace.

use super::tools::{tools, Kind};
use super::{llm, runner, ActiveRun, StageState, WishRuntime, AIWS_API, AIWS_JS, ANALYZE_SYSTEM, EXECUTE_SYSTEM, RUN_JS};
use crate::AppState;
use agent_tool::xllm::{RunEvent, RunObserver, TaskInput, TaskOverrides, XllmDeps, XllmRun, XllmTask};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::wish::context::{Stage, StageKind, StageOptions};
use aiworkspace_store::wish::results::{program_output, Collector};
use aiworkspace_store::Caller;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> WsResult<T> + Send + 'static) -> WsResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| WsError::io(e.to_string()))?
}

async fn with_ws<T: Send + 'static>(state: &Arc<AppState>, ws_id: &str, f: impl FnOnce(&mut aiworkspace_store::workspace::Workspace) -> WsResult<T> + Send + 'static) -> WsResult<T> {
    let (st, id) = (state.clone(), ws_id.to_string());
    blocking(move || st.with_ws(&id, f)).await
}

fn now() -> String {
    format_utc_ms(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0))
}

fn set_progress(active: &ActiveRun, phase: &str) {
    let mut p = active.progress.lock().unwrap();
    p["phase"] = json!(phase);
}

/// The worker of one run: never panics into the service, always leaves a final state.
pub async fn drive(state: Arc<AppState>, caller: Caller, ws_id: String, run_id: String, active: Arc<ActiveRun>) {
    let res = drive_inner(&state, &caller, &ws_id, &run_id, &active).await;
    settle(&state, caller, &ws_id, &run_id, &active, res).await;
    state.wish.unregister(&run_id);
}

/// The end of a worker: a failure or cancellation becomes the run's final state, a success leaves
/// what the stage wrote.
async fn settle(state: &Arc<AppState>, caller: Caller, ws_id: &str, run_id: &str, active: &Arc<ActiveRun>, res: WsResult<()>) {
    let cancelled = active.cancelled.load(Ordering::SeqCst);
    let progress = active.progress.lock().unwrap().clone();
    let (rid, final_state) = (run_id.to_string(), if cancelled { "cancelled" } else { "failed" });
    let outcome = match res {
        Ok(()) if !cancelled => None,
        Ok(()) => Some(json!({ "code": "RUN_CANCELLED", "detail": "Cancelled" })),
        Err(e) => Some(if cancelled { json!({ "code": "RUN_CANCELLED", "detail": "Cancelled" }) } else { e.to_json() }),
    };
    let _ = with_ws(state, ws_id, move |ws| {
        let run = ws.wish_run(&caller, &rid)?;
        let mut detail = json!({ "progress": progress });
        match outcome {
            None => ws.wish_run_set(&rid, None, None, None, None, Some(&detail)),
            Some(err) => {
                // a run that reached waiting / applied keeps that state (cancel after the fact is honest)
                if matches!(run["state"].as_str(), Some("succeeded" | "applying")) {
                    return ws.wish_run_set(&rid, None, None, None, None, Some(&detail));
                }
                detail["error"] = err;
                detail["progress"]["phase"] = json!(final_state);
                ws.wish_run_set(&rid, Some(final_state), None, None, None, Some(&detail))
            }
        }
    })
    .await;
}

fn wish_payload(stage: &Stage) -> Value {
    Value::Object(stage.wish.payload.clone())
}

/// A stage opened for a run: its state and what the run record said.
pub struct Opened {
    pub st: Arc<StageState>,
    pub run: Value,
    pub params: Value,
    pub run_dir: PathBuf,
    pub stage_name: String,
    pub program: String,
}

/// Take the stage snapshot, build the stage directory and the shared state of a run's tools. With
/// `transition` the run record moves queued → snapshotting → running (the in-service worker); an
/// executor outside the service already holds the run in `running` under its lease.
pub fn open_stage(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str, active: &Arc<ActiveRun>, transition: bool) -> WsResult<Opened> {
    let (run, ws_dir) = state.with_ws(ws_id, |ws| {
        let r = ws.wish_run(caller, run_id)?;
        if transition {
            ws.wish_run_set(run_id, Some("snapshotting"), None, None, None, Some(&json!({ "progress": { "phase": "snapshotting" } })))?;
        }
        Ok((r, ws.dir.clone()))
    })?;
    set_progress(active, "snapshotting");
    let params = run["params"].clone();
    let stage_name = params["stage"].as_str().unwrap_or("").to_string();
    let program = run["program"].as_str().unwrap_or("").to_string();
    let wish_id = params["wish_id"].as_str().unwrap_or("").to_string();
    let run_dir = ws_dir.join("runs").join(run_id);
    let workdir = run_dir.join("work");
    std::fs::create_dir_all(&workdir).map_err(|e| WsError::io(e.to_string()))?;
    let kind = match (program.as_str(), stage_name.as_str()) {
        (super::PROGRAM_MOCK, _) => StageKind::Program,
        (_, "analyze") => StageKind::Analyze,
        (_, "rerun_program") => StageKind::Program,
        _ => StageKind::Execute,
    };
    let request = request_params(&params);
    let opts = StageOptions {
        kind,
        location: params.get("location").cloned().unwrap_or(json!({})),
        request: request.clone(),
        catalog: super::builtin_catalog(),
        controls: super::controls_catalog(),
        map_budget_chars: state.wish.config.map_budget_chars,
        materialize: kind != StageKind::Analyze && program != super::PROGRAM_MOCK,
    };
    let snap = state.with_ws(ws_id, |ws| ws.wish_snapshot(caller))?;
    let stage = Stage::open(snap, &wish_id, run_id, &workdir, &opts)?;
    // the snapshot is fixed: the stage runs from here on (a cancel that came first stays)
    if transition {
        state.with_ws(ws_id, |ws| {
            if ws.wish_run(caller, run_id)?["state"] == json!("snapshotting") {
                ws.wish_run_set(run_id, Some("running"), None, None, None, None)?;
            }
            Ok(())
        })?;
    }
    // the program host the model debugs with is the one the service runs
    write(&workdir.join("lib/aiws.js"), AIWS_JS)?;
    write(&workdir.join("lib/run.js"), RUN_JS)?;
    let payload = wish_payload(&stage);
    let mut req = request.clone();
    for k in ["prompt", "knowledge", "refinements"] {
        if let Some(v) = payload.get(k) {
            req[k] = v.clone();
        }
    }
    req["stage"] = json!(stage_name);
    req["context_prompt"] = payload["analysis"]["context_prompt"].clone();
    write(&workdir.join("request.json"), &serde_json::to_string_pretty(&req).unwrap_or_default())?;
    let token = aiworkspace_store::workspace::random_id("tok_");
    let feedback = params.get("feedback").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).map(str::to_string);
    let prompts = vec![
        payload["prompt"].as_str().unwrap_or("").to_string(),
        payload["knowledge"].as_str().unwrap_or("").to_string(),
        payload["analysis"]["context_prompt"].as_str().unwrap_or("").to_string(),
        feedback.clone().unwrap_or_default(),
    ];
    let st = Arc::new(StageState {
        runtime: state.wish.clone(),
        active: active.clone(),
        run_id: run_id.to_string(),
        workspace_id: ws_id.to_string(),
        ws_dir: ws_dir.clone(),
        workdir: workdir.clone(),
        stage: Mutex::new(stage),
        collector: Mutex::new(Collector::default()),
        analysis: Mutex::new(None),
        feedback,
        token: token.clone(),
        aicc: super::on_behalf_of(&run, ws_id),
        prompts,
        tool_calls: Default::default(),
        program_runs: Default::default(),
        map_items: Default::default(),
        map_calls: Default::default(),
        map_cached: Default::default(),
    });
    state.wish.stages.lock().unwrap().insert(token, st.clone());
    Ok(Opened { st, run, params, run_dir, stage_name, program })
}

async fn drive_inner(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str, active: &Arc<ActiveRun>) -> WsResult<()> {
    let (st, c, w, r, a) = (state.clone(), caller.clone(), ws_id.to_string(), run_id.to_string(), active.clone());
    let Opened { st, run, params, run_dir, stage_name, program } = blocking(move || open_stage(&st, &c, &w, &r, &a, true)).await?;
    let provided = run["detail"]["exec"]["provided"].clone();
    let executor_label = run["detail"]["exec"]["mode"].as_str().unwrap_or("service").to_string();
    let res = match (program.as_str(), stage_name.as_str(), provided.is_object()) {
        // an executor outside the service reported its candidate (M3): plan it like a Mock's results
        (_, "analyze", true) => provided_analysis(state, caller, ws_id, &st, &provided).await,
        (_, _, true) => mock(state, caller, ws_id, &st, &provided, &executor_label).await,
        (super::PROGRAM_MOCK, _, _) => mock(state, caller, ws_id, &st, &params["provided"], "mock").await,
        (_, "analyze", _) => analyze(state, caller, ws_id, &st, &run_dir).await,
        (_, "rerun_program", _) => rerun(state, caller, ws_id, &st).await,
        _ => execute(state, caller, ws_id, &st, &run_dir, &params).await,
    };
    state.wish.stages.lock().unwrap().remove(&st.token);
    res
}

fn write(path: &Path, text: &str) -> WsResult<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| WsError::io(e.to_string()))?;
    }
    std::fs::write(path, text).map_err(|e| WsError::io(e.to_string()))
}

/// Fixed parameters of the run (§7.2): what the UI sent, with a date and a time zone always present.
fn request_params(params: &Value) -> Value {
    let mut r = params.get("request").cloned().filter(Value::is_object).unwrap_or(json!({}));
    if r.get("today").and_then(Value::as_str).is_none() {
        r["today"] = json!(now()[..10].to_string());
    }
    if r.get("timezone").and_then(Value::as_str).is_none() {
        r["timezone"] = json!("UTC");
    }
    r
}

// ---- xllm

struct Observer {
    active: Arc<ActiveRun>,
}

impl RunObserver for Observer {
    fn on_event(&self, _run_id: &str, event: RunEvent) {
        let mut p = self.active.progress.lock().unwrap();
        match event {
            // the loop as a whole starts / ends (llm_context has no per-request event): between
            // tool calls the model is working
            RunEvent::LlmStarted { model } => {
                p["phase"] = json!("running");
                p["waiting_model"] = json!(true);
                p["model"] = json!(model);
            }
            RunEvent::LlmFinished { .. } => p["waiting_model"] = json!(false),
            RunEvent::ToolFinished { .. } => p["waiting_model"] = json!(true),
            RunEvent::ToolStarted { name, command, .. } => {
                p["waiting_model"] = json!(false);
                if name == "shell" {
                    p["activity"] = json!("shell");
                    p["last_command"] = json!(command.map(|c| c.chars().take(200).collect::<String>()));
                } else if matches!(name.as_str(), "write_file" | "edit_file") {
                    p["activity"] = json!("writing_program");
                } else if name == "read_file" {
                    p["activity"] = json!("reading");
                }
            }
            RunEvent::Warning(w) => p["warning"] = json!(w.chars().take(300).collect::<String>()),
            _ => {}
        }
    }
}

/// Run one xllm stage in `st.workdir` with exactly `kinds` as host tools (plus the bash group).
async fn run_xllm(st: &Arc<StageState>, run_dir: &Path, system: String, user: String, model: &str, iterations: u32, kinds: &[Kind], shell: bool) -> WsResult<Value> {
    let rt: &Arc<WishRuntime> = &st.runtime;
    let cfg = &rt.config;
    let names: Vec<&str> = kinds.iter().map(|k| k.name()).collect();
    let ctx_file = st.workdir.join(".llm_context");
    let llm_ctx = llm::llm_context(cfg, model, iterations, &run_dir.join("xllm"), &names, shell, llm::attribution(cfg, st.aicc.as_ref())?);
    write(&ctx_file, &serde_json::to_string_pretty(&llm_ctx).unwrap_or_default())?;
    let mut deps = XllmDeps::default().with_observer(Arc::new(Observer { active: st.active.clone() })).with_lock_dir(run_dir.join("locks"));
    for t in tools(st, kinds) {
        deps = deps.with_host_tool(t);
    }
    if let Some(l) = &cfg.llm_override {
        deps.llm_factory = l.clone();
    }
    deps.skip_workdir_lock = true;
    for (k, v) in st.program_env() {
        deps.runtime_env.insert(k, v);
    }
    deps.runtime_env.insert("NO_COLOR".into(), "1".into());
    deps.runtime_env.insert("DENO_NO_UPDATE_CHECK".into(), "1".into());
    if let Some(d) = runner::find_deno(&cfg.deno).and_then(|p| p.parent().map(Path::to_path_buf)) {
        deps.runtime_path_prefix.push(d);
    }
    let overrides = TaskOverrides { system: Some(system), tools: Some(true), ..Default::default() };
    let prepared = XllmTask::prepare(&st.workdir, TaskInput::question(user), overrides, &deps)
        .await
        .map_err(|e| WsError::new(Code::DependencyUnavailable, format!("Preparing the model run failed: {e}")))?;
    // nothing but this stage's configuration may shape the run (§13.3)
    let own = ctx_file.canonicalize().map(|p| p.display().to_string()).unwrap_or_default();
    if prepared.merged.files.iter().any(|f| *f != own && std::path::Path::new(f).canonicalize().map(|p| p.display().to_string()).unwrap_or_default() != own) {
        return Err(WsError::invalid_op(format!("unexpected .llm_context files above the run directory: {:?}", prepared.merged.files)));
    }
    let mut run = XllmRun::start(prepared, deps).await.map_err(|e| WsError::new(Code::DependencyUnavailable, format!("Starting the model run failed: {e}")))?;
    *st.active.interrupter.lock().unwrap() = Some(run.interrupter());
    if st.cancelled() {
        run.interrupter().interrupt("cancelled by the user");
    }
    set_progress(&st.active, "running");
    let outcome = run.execute().await.map_err(|e| WsError::new(Code::DependencyUnavailable, format!("The model run failed: {e}")))?;
    *st.active.interrupter.lock().unwrap() = None;
    let rec = outcome.record();
    let usage = json!({ "main": rec.usage.main, "llm_requests": rec.usage.llm_requests });
    let x = json!({
        "xllm_run_id": rec.run_id, "status": rec.status.as_str(), "model": rec.config.model,
        "error": rec.last_error.as_ref().map(|e| json!({ "phase": format!("{:?}", e.phase), "kind": e.kind, "message": e.message })),
        "limit_reason": rec.limit_reason, "answer": rec.result.as_ref().map(|r| r.raw.chars().take(2000).collect::<String>()),
        "usage": usage,
    });
    if let Some(sub) = rec.last_error.as_ref().and_then(|e| super::refusal_sub_code(&e.kind)) {
        let aicc = st.aicc.as_ref().map(|a| json!({ "mode": a.mode, "user_id": a.user_id, "operator_id": a.operator_id })).unwrap_or(Value::Null);
        let mut x = x;
        x["error"]["message"] = Value::Null;
        return Err(super::refusal_error(sub, &aicc).with_data(json!({ "aicc": aicc, "xllm": x })));
    }
    Ok(x)
}

fn section(title: &str, body: &str) -> String {
    format!("## {title}\n\n{}\n\n", if body.trim().is_empty() { "(none)" } else { body.trim() })
}

fn refinement_lines(payload: &Value) -> String {
    payload["refinements"].as_array().map(|a| a.iter().filter_map(|r| r["text"].as_str()).map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

async fn finish_stage(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, candidate: Value, evidence: Value, detail: Value) -> WsResult<()> {
    if st.cancelled() {
        return Ok(());
    }
    let (c, rid) = (caller.clone(), st.run_id.clone());
    let progress = st.active.progress.lock().unwrap().clone();
    with_ws(state, ws_id, move |ws| {
        let run = ws.wish_run(&c, &rid)?;
        if run["state"] == json!("cancelled") {
            return Ok(());
        }
        let mut d = detail;
        d["progress"] = progress;
        d["progress"]["phase"] = json!("waiting_confirmation");
        ws.wish_run_set(&rid, Some("validating"), Some(&candidate), Some(&evidence), None, Some(&d))?;
        let run = ws.wish_run(&c, &rid)?;
        let plan = super::plan_for(ws, &c, &run, &json!({}))?;
        ws.wish_run_keep_plan(&rid, &plan)?;
        ws.wish_run_set(&rid, Some("waiting_confirmation"), None, None, None, None)
    })
    .await
}

// ---- analysis (§7)

/// The user message of the analysis stage and the run's evidence.
fn analyze_user(st: &Arc<StageState>) -> WsResult<(String, Value)> {
    let stage = st.stage.lock().unwrap();
    let p = wish_payload(&stage);
    let map = std::fs::read_to_string(stage.workdir.join("WORKSPACE.md")).unwrap_or_default();
    let mut u = String::from("# Wish (analysis stage)\n\n");
    u.push_str(&section("Original request", p["prompt"].as_str().unwrap_or("")));
    u.push_str(&section("Wish knowledge (definitions, background, preferences)", p["knowledge"].as_str().unwrap_or("")));
    u.push_str(&section("Refinements (to be followed every time from now on)", &refinement_lines(&p)));
    u.push_str(&section("Fixed parameters (request)", &serde_json::to_string(&stage.request).unwrap_or_default()));
    if let Some(a) = p.get("analysis") {
        u.push_str(&section(
            "Previous analysis (a new analysis keeps its result names, types, keys and views by default)",
            &format!("context_prompt: {}\n\noutput_contract: {}\n\nchecks: {}", a["context_prompt"].as_str().unwrap_or(""), a["output_contract"], a["checks"]),
        ));
    }
    u.push_str("---\n\n");
    u.push_str(&map);
    u.push_str("\n\nAfter reading the map: verify with the ws_* tools where needed, then submit with submit_analysis.");
    Ok((u, stage.evidence()?))
}

const ANALYZE_KINDS: [Kind; 7] = [Kind::Outline, Kind::Find, Kind::Profile, Kind::Neighbors, Kind::Read, Kind::Query, Kind::SubmitAnalysis];

async fn analyze(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, run_dir: &Path) -> WsResult<()> {
    let (user, evidence) = analyze_user(st)?;
    let x = run_xllm(st, run_dir, ANALYZE_SYSTEM.to_string(), user, &st.runtime.config.analyze_model, st.runtime.config.analyze_tool_iterations, &ANALYZE_KINDS, false).await?;
    analyze_tail(state, caller, ws_id, st, evidence, x).await
}

/// The accepted analysis becomes the candidate (`x`: the model run's record, or what an outside executor said).
async fn analyze_tail(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, evidence: Value, x: Value) -> WsResult<()> {
    if st.cancelled() {
        return Ok(());
    }
    let accepted = st.analysis.lock().unwrap().take();
    let Some(v) = accepted else {
        return Err(WsError::new(Code::DependencyUnavailable, format!("The model did not submit an accepted analysis (run status {}){}", x["status"].as_str().unwrap_or("?"),
            x["error"]["message"].as_str().map(|m| format!(": {m}")).or_else(|| x["limit_reason"].as_str().map(|m| format!(": {m}"))).unwrap_or_default()))
            .with_data(json!({ "xllm": x })));
    };
    let candidate = json!({ "wish_id": evidence["wish_id"], "analysis": v.analysis, "inputs": v.inputs, "basis": evidence["basis"], "executed_at": now() });
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({ "xllm": x, "usage": x["usage"] })).await
}

// ---- execution (§8): generation, feedback rounds and repairing a program

fn input_lines(stage: &Stage) -> String {
    let mut out = Vec::new();
    for i in &stage.inputs {
        let meta: Value = std::fs::read_to_string(stage.workdir.join("context/entities").join(&i.entity_id).join("meta.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
        let mut l = format!("- `{}`: {} \"{}\" ({})", i.name, aiworkspace_store::wish::context::type_label(&i.type_id), i.label, meta["handle"].as_str().unwrap_or(""));
        if let Some(r) = meta["rows"].as_u64() {
            l.push_str(&format!(", {}", aiworkspace_store::wish::profile::count(r, "row", "rows")));
        }
        if let Some(k) = meta["control"].as_str() {
            let what = if meta["node"] == json!(true) { format!("a node of a {k} with its branch") } else { format!("a {k}") };
            l.push_str(&format!(", {what} ({}) as a Markdown outline in outline.md", aiworkspace_store::wish::profile::count(meta["nodes"].as_u64().unwrap_or(0), "node", "nodes")));
        }
        if let Some(v) = meta.get("view").filter(|v| v.is_object()) {
            l.push_str(&format!(", read through view {} ({})", v["handle"].as_str().unwrap_or(""), v["summary"].as_str().unwrap_or("")));
        } else if let Some(s) = &i.selector {
            l.push_str(&format!(", range {s}"));
        }
        if i.appended {
            l.push_str(" (input appended during the last execution)");
        }
        l.push_str(&format!("; files in context/entities/{}/", i.entity_id));
        out.push(l);
    }
    if out.is_empty() {
        "No data inputs (a purely creative task)".into()
    } else {
        out.join("\n")
    }
}

/// The stored program when this host runs its `aiws` version; one written for another version is
/// regenerated by an execution instead.
fn program_source(stage: &Stage) -> Option<String> {
    let prog = stage.wish.payload.get("program")?;
    if prog.get("api_version").and_then(Value::as_u64) != Some(aiworkspace_core::wish::PROGRAM_API_VERSION) {
        return None;
    }
    let src = prog.get("source")?.as_str()?;
    String::from_utf8(stage.snap.asset_bytes(src).ok()?).ok()
}

fn collector_for(stage: &Stage, program_only: bool) -> WsResult<Collector> {
    let a = stage.wish.payload.get("analysis").cloned().unwrap_or(json!({}));
    Ok(Collector {
        contract: a.get("output_contract").cloned().unwrap_or(json!({ "results": [] })),
        checks_def: a.get("checks").and_then(Value::as_array).cloned().unwrap_or_default(),
        prev_bindings: aiworkspace_core::wish::current_results(&stage.wish.payload).into_iter().collect(),
        inputs: stage.input_infos()?,
        workdir: stage.workdir.clone(),
        program_only,
        ..Default::default()
    })
}

/// The analysis must be current and ready before anything executes (§7.3).
fn ready_to_execute(stage: &Stage) -> WsResult<()> {
    let p = &stage.wish.payload;
    let Some(a) = p.get("analysis") else { return Err(WsError::invalid_op("This wish has not been analyzed yet: click \"Analyze\" first")) };
    if aiworkspace_core::wish::needs_analysis(p) {
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_ANALYSIS", "The request, knowledge, refinements, inputs or execution configuration changed after the analysis: analyze again first"));
    }
    if !aiworkspace_core::freshness::analysis_sources_changed(&stage.ctx(), p)?.is_empty() {
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_ANALYSIS", "Documents the task description relies on (definitions, rules) changed after the analysis: analyze again first"));
    }
    if a["status"] != json!("ready") {
        let blockers: Vec<String> = a["blockers"].as_array().into_iter().flatten().filter_map(|b| b["message"].as_str().map(str::to_string)).collect();
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_INPUT", format!("The analysis concluded that inputs are missing: {}", blockers.join("; "))));
    }
    Ok(())
}

/// What the execution stage starts from: the user message, whether this repairs a program, and the
/// program already present (`program/main.js`) when there is one.
fn execute_prepare(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, params: &Value) -> WsResult<(String, bool, Option<String>)> {
    let repair = params["stage"] == json!("repair_program");
    // the previous round of a feedback / repair chain: its program is the starting point
    let parent = match params.get("parent_run_id").and_then(Value::as_str) {
        Some(pid) => {
            let run = state.with_ws(ws_id, |ws| ws.wish_run(caller, pid))?;
            let src = std::fs::read_to_string(st.ws_dir.join("runs").join(pid).join("work/program/main.js")).ok();
            Some((run, src))
        }
        None => None,
    };
    let (user, existing) = {
        let stage = st.stage.lock().unwrap();
        ready_to_execute(&stage)?;
        // repairing makes the stored program work again: written results stay as they are (§9.2)
        *st.collector.lock().unwrap() = collector_for(&stage, repair)?;
        let p = wish_payload(&stage);
        let a = &p["analysis"];
        let existing = parent.as_ref().and_then(|(_, s)| s.clone()).or_else(|| program_source(&stage));
        if let Some(src) = &existing {
            write(&stage.workdir.join("program/main.js"), src)?;
        }
        let map = std::fs::read_to_string(stage.workdir.join("WORKSPACE.md")).unwrap_or_default();
        let mut u = String::from(if repair { "# Wish (execution stage: repairing the program)\n\n" } else { "# Wish (execution stage)\n\n" });
        u.push_str(&section("Original request", p["prompt"].as_str().unwrap_or("")));
        u.push_str(&section("Wish knowledge (definitions, background, preferences)", p["knowledge"].as_str().unwrap_or("")));
        u.push_str(&section("Refinements (must be followed)", &refinement_lines(&p)));
        u.push_str(&section("Task description (context prompt)", a["context_prompt"].as_str().unwrap_or("")));
        u.push_str(&section("Inputs (read them in the program with aiws.input(input name))", &input_lines(&stage)));
        u.push_str(&section("Output contract", &format!("```json\n{}\n```", serde_json::to_string_pretty(&a["output_contract"]).unwrap_or_default())));
        u.push_str(&section("Acceptance checks (the program reports program checks with aiws.check; review checks are self-assessed in finish)", &format!("```json\n{}\n```", serde_json::to_string_pretty(&a["checks"]).unwrap_or_default())));
        u.push_str(&section("Fixed parameters (aiws.request)", &serde_json::to_string(&stage.request).unwrap_or_default()));
        u.push_str(&section(
            "Program",
            if existing.is_some() { "program/main.js already has a program (the last applied one or the previous round's): make the smallest change to it, keeping result names, fields and keys stable." } else { "No program yet: write it in program/main.js." },
        ));
        if let Some((prun, _)) = &parent {
            let prev = &prun["candidate"];
            if repair {
                let err = prun["detail"]["error"].clone();
                u.push_str(&section("Program run failed (when re-running only the program)", &format!("{}\n\n{}", err["detail"].as_str().unwrap_or(""), prun["detail"]["program_log"].as_str().unwrap_or(""))));
                u.push_str("Read the inputs' latest schema and profile, make the smallest change that makes the program run correctly on the new data, then run_program and finish. Direct results (text) stay as they are and need not be submitted again.\n\n");
            } else {
                let results: Vec<Value> = prev["results"].as_array().into_iter().flatten().map(|r| json!({ "name": r["name"], "type": r["type"], "approach": r["approach"],
                    "rows": r["table"]["rows"].as_array().map(Vec::len), "fields": r["table"]["fields"], "excerpt": r["markdown"].as_str().map(|m| m.chars().take(300).collect::<String>()) })).collect();
                u.push_str(&section("Previous round's candidate", &format!("Results: {}\nChecks: {}\nfacts: {}\nSummary: {}", json!(results), prev["checks"], prev["facts"], prev["summary"].as_str().unwrap_or(""))));
                // direct results of the previous round are carried over unless the model resubmits them
                u.push_str(&section("User feedback (to be satisfied in this round)", st.feedback.as_deref().unwrap_or("")));
                u.push_str("Change the existing program to satisfy the feedback; results the feedback does not affect stay unchanged (put_result direct results again when needed). In finish, write the distilled requirements in refinements.\n\n");
            }
        }
        u.push_str("---\n\n");
        u.push_str(&map);
        (u, existing)
    };
    // a feedback round starts from the previous round's written results
    if let Some((prun, _)) = &parent {
        if !repair {
            let mut c = st.collector.lock().unwrap();
            for r in prun["candidate"]["results"].as_array().into_iter().flatten() {
                if r["approach"] == json!("direct") {
                    if let Some(n) = r["name"].as_str() {
                        c.direct.insert(n.to_string(), r.clone());
                    }
                }
            }
        }
    }
    Ok((user, repair, existing))
}

fn execute_system() -> String {
    EXECUTE_SYSTEM.replace("{AIWS_API}", AIWS_API)
}

fn execute_kinds() -> Vec<Kind> {
    let mut kinds = Kind::READ.to_vec();
    kinds.extend(Kind::DELIVERY);
    kinds
}

async fn execute(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, run_dir: &Path, params: &Value) -> WsResult<()> {
    let (user, repair, _) = execute_prepare(state, caller, ws_id, st, params)?;
    let x = run_xllm(st, run_dir, execute_system(), user, &st.runtime.config.execute_model, st.runtime.config.execute_tool_iterations, &execute_kinds(), true).await?;
    execute_tail(state, caller, ws_id, st, x, repair).await
}

/// The collector's results become the candidate once `finish` was accepted.
async fn execute_tail(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, x: Value, repair: bool) -> WsResult<()> {
    if st.cancelled() {
        return Ok(());
    }
    if st.collector.lock().unwrap().finished.is_none() {
        let tail = st.collector.lock().unwrap().program.as_ref().map(|p| p.logs.clone()).unwrap_or_default();
        return Err(WsError::new(Code::DependencyUnavailable, format!("The execution did not end with finish (run status {}){}", x["status"].as_str().unwrap_or("?"),
            x["error"]["message"].as_str().map(|m| format!(": {m}")).or_else(|| x["limit_reason"].as_str().map(|m| format!(": {m}"))).unwrap_or_default()))
            .with_data(json!({ "xllm": x, "program_log": tail })));
    }
    let (mut candidate, evidence) = {
        let stage = st.stage.lock().unwrap();
        let c = st.collector.lock().unwrap();
        let mut cand = c.candidate();
        let texts = st.prompt_texts();
        cand["uncited_numbers"] = json!(c.uncited_numbers(&texts.iter().map(String::as_str).collect::<Vec<_>>()));
        let ev = stage.evidence()?;
        let refinements: Vec<Value> = c.finished.as_ref().and_then(|f| f["refinements"].as_array().cloned()).unwrap_or_default()
            .iter().filter_map(Value::as_str).filter(|t| !t.trim().is_empty())
            .map(|t| json!({ "text": t.chars().take(aiworkspace_core::wish::MAX_REFINEMENT_CHARS).collect::<String>(), "at": now(), "run_id": st.run_id })).collect();
        cand["refinements"] = json!(refinements);
        if let Some(p) = &c.program {
            cand["program"] = json!({ "digest": p.digest, "path": st.workdir.join("program/main.js").display().to_string(),
                                      "produces": p.results.iter().map(|r| r["name"].clone()).collect::<Vec<_>>() });
        }
        (cand, ev)
    };
    stage_files(state, caller, ws_id, &mut candidate).await?;
    fill_candidate(&mut candidate, &evidence, "xllm", st, if repair { "program" } else { "generate" });
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({ "xllm": x, "usage": x["usage"] })).await
}

fn fill_candidate(c: &mut Value, ev: &Value, executor: &str, st: &Arc<StageState>, mode: &str) {
    for k in ["wish_id", "read_set", "appended_inputs", "basis"] {
        c[k] = ev[k].clone();
    }
    c["executor"] = json!(executor);
    c["run_id"] = json!(st.run_id);
    c["executed_at"] = json!(now());
    c["mode"] = json!(mode);
    c["simulated"] = json!(executor == "mock");
}

/// Files the results reference, and the program source, become staged assets (content addressed; bound
/// again at application). The run keeps their object ids and bindings, not the paths in its work folder.
async fn stage_files(state: &Arc<AppState>, caller: &Caller, ws_id: &str, cand: &mut Value) -> WsResult<()> {
    let mut paths: Vec<(String, PathBuf)> = Vec::new();
    for (i, r) in cand["results"].as_array().into_iter().flatten().enumerate() {
        if let Some(p) = r["file"]["path"].as_str() {
            paths.push((format!("r{i}"), PathBuf::from(p)));
        }
    }
    if let Some(p) = cand["program"]["path"].as_str() {
        paths.push(("program".into(), PathBuf::from(p)));
    }
    if paths.is_empty() {
        return Ok(());
    }
    let c = caller.clone();
    let staged = with_ws(state, ws_id, move |ws| {
        let mut out = Vec::new();
        for (k, p) in paths {
            let bytes = std::fs::read(&p).map_err(|e| WsError::io(format!("{}: {e}", p.display())))?;
            out.push((k, ws.stage_asset(&c, &bytes)?));
        }
        Ok(out)
    })
    .await?;
    for (k, staged) in staged {
        let target = if k == "program" {
            &mut cand["program"]
        } else if let Some(i) = k.strip_prefix('r').and_then(|n| n.parse::<usize>().ok()) {
            &mut cand["results"][i]["file"]
        } else {
            continue;
        };
        for field in ["object_id", "key", "version"] {
            target[field] = staged[field].clone();
        }
        if let Some(o) = target.as_object_mut() {
            o.remove("path");
        }
    }
    Ok(())
}

// ---- re-running the stored program (§9.2): no model

/// The stored program, written into the stage for a run without a model.
fn rerun_prepare(st: &Arc<StageState>) -> WsResult<String> {
    let stage = st.stage.lock().unwrap();
    let p = &stage.wish.payload;
    if p.get("analysis").is_none() {
        return Err(WsError::invalid_op("This wish has not been analyzed yet"));
    }
    let src = program_source(&stage).ok_or_else(|| WsError::invalid_op("This wish has no program this service can run yet: execute it once first (generate)"))?;
    write(&stage.workdir.join("program/main.js"), &src)?;
    *st.collector.lock().unwrap() = collector_for(&stage, true)?;
    Ok(src)
}

async fn rerun(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>) -> WsResult<()> {
    rerun_prepare(st)?;
    set_progress(&st.active, "running");
    st.note_tool("run_program");
    let out = runner::run(st).await.map_err(|e| WsError::new(Code::DependencyUnavailable, e))?;
    let log = out.tail();
    let fail = |msg: String| WsError::sub(Code::InvalidOperation, "PROGRAM_FAILED", msg).with_data(json!({ "program_log": log }));
    if out.timed_out {
        return Err(fail(format!("The program timed out ({} s)", st.runtime.config.program_timeout_secs)));
    }
    let raw = out.results.clone().ok_or_else(|| fail(format!("The program wrote no results (exit code {:?})", out.code)))?;
    if let Some(e) = raw.get("error").and_then(Value::as_str) {
        return Err(fail(format!("The program threw an error: {}", e.lines().next().unwrap_or(""))));
    }
    let source = std::fs::read_to_string(st.workdir.join("program/main.js")).unwrap_or_default();
    {
        let mut c = st.collector.lock().unwrap();
        let run = program_output(&c, &raw, &source, &log).map_err(|errs| fail(format!("The program's results failed validation: {}", errs.join("; "))))?;
        c.program = Some(run);
    }
    st.note_program_run();
    rerun_tail(state, caller, ws_id, st, log).await
}

/// The program's validated output becomes the candidate of a run without a model.
async fn rerun_tail(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, log: String) -> WsResult<()> {
    let fail = |msg: String| WsError::sub(Code::InvalidOperation, "PROGRAM_FAILED", msg).with_data(json!({ "program_log": log }));
    let (mut candidate, evidence) = {
        let stage = st.stage.lock().unwrap();
        let mut c = st.collector.lock().unwrap();
        if c.program.is_none() {
            return Err(fail("The program did not run".into()));
        }
        let blocking = c.blocking();
        if !blocking.is_empty() {
            return Err(fail(format!("The program's results are incomplete: {}", blocking.join("; "))));
        }
        c.finished = Some(json!({ "summary": "Re-ran only the program (no model call)" }));
        (c.candidate(), stage.evidence()?)
    };
    stage_files(state, caller, ws_id, &mut candidate).await?;
    fill_candidate(&mut candidate, &evidence, "xllm", st, "program");
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({ "program_log": log })).await
}

// ---- the Mock executor's results (simulated): the browser executed, the service plans (§3)

fn mock_slug(name: &str) -> String {
    aiworkspace_store::wish::plan::slug(name)
}

/// Results of the Mock (phase two `ResultSpec`) → `wish.results.v2`.
fn mock_results(st: &Arc<StageState>, provided: &Value) -> WsResult<Vec<Value>> {
    let mut out = Vec::new();
    for r in provided["results"].as_array().into_iter().flatten() {
        let name = r["name"].as_str().unwrap_or("").to_string();
        if !aiworkspace_core::wish::is_result_name(&name) {
            return Err(WsError::invalid_op(format!("Mock result name {name:?} is invalid")));
        }
        let mut v = json!({ "name": name, "title": r.get("title").cloned().unwrap_or(json!(name)) });
        let mut view = Map::new();
        if let Some(x) = r.get("renderer") {
            view.insert("renderer".into(), x.clone());
        }
        if let Some(x) = r.get("config") {
            view.insert("config".into(), x.clone());
        }
        if let Some(x) = r.get("size") {
            view.insert("size".into(), x.clone());
        }
        if view.contains_key("renderer") {
            v["views"] = json!([Value::Object(view)]);
        } else if let Some(sz) = r.get("size") {
            v["size"] = sz.clone();
        }
        let c = &r["content"];
        match r["type"].as_str().unwrap_or("") {
            "richtext" => {
                v["type"] = json!("richtext");
                v["approach"] = json!("direct");
                v["markdown"] = c["markdown"].clone();
            }
            "record" => {
                v["type"] = json!("record");
                v["approach"] = json!("direct");
                let props_schema = c["schema"]["properties"].as_array().cloned().unwrap_or_default();
                let mut props = Map::new();
                let mut properties = Vec::new();
                for p in &props_schema {
                    let (key, nm) = (p["key"].as_str().unwrap_or(""), p["name"].as_str().unwrap_or(""));
                    properties.push(json!({ "name": nm, "type": p["type"], "key": key }));
                    if let Some(x) = c["props"].get(key) {
                        props.insert(nm.to_string(), x.clone());
                    }
                }
                v["record"] = json!({ "properties": properties, "props": props });
            }
            "table" => {
                v["type"] = json!("table");
                v["approach"] = json!("program");
                let fields = c["fields"].as_array().cloned().unwrap_or_default();
                let names: Map<String, Value> = fields.iter().map(|f| (f["field_id"].as_str().unwrap_or("").to_string(), f["name"].clone())).collect();
                let rows: Vec<Value> = c["rows"].as_array().into_iter().flatten().map(|row| {
                    Value::Object(row.as_object().into_iter().flatten().filter_map(|(k, x)| names.get(k).and_then(Value::as_str).map(|n| (n.to_string(), x.clone()))).collect())
                }).collect();
                v["table"] = json!({ "fields": fields.iter().map(|f| json!({ "name": f["name"], "type": f["type"] })).collect::<Vec<_>>(), "key": [], "rows": rows });
            }
            "image" => {
                let path = st.workdir.join("output").join(format!("{}.svg", mock_slug(&name)));
                write(&path, c["svg"].as_str().unwrap_or(""))?;
                let bytes = std::fs::read(&path).map_err(|e| WsError::io(e.to_string()))?;
                let (media, dims) = aiworkspace_store::wish::results::sniff_file(&bytes);
                v["type"] = json!("image");
                v["approach"] = json!("direct");
                let w = c["width"].as_u64().or(dims.map(|d| d.0));
                let h = c["height"].as_u64().or(dims.map(|d| d.1));
                v["file"] = json!({ "path": path.display().to_string(), "media_type": media, "size": bytes.len(), "file_name": format!("{}.svg", mock_slug(&name)) });
                if let (Some(w), Some(h)) = (w, h) {
                    v["file"]["image"] = json!({ "width": w, "height": h });
                }
            }
            "video" => {
                v["type"] = json!("video");
                v["approach"] = json!("direct");
                let mut frames = Vec::new();
                for (i, f) in c["frames"].as_array().into_iter().flatten().enumerate() {
                    let path = st.workdir.join("output").join(format!("{}-f{i}.svg", mock_slug(&name)));
                    write(&path, f["svg"].as_str().unwrap_or(""))?;
                    frames.push(json!({ "path": path.display().to_string(), "duration_ms": f["durationMs"], "caption": f.get("caption").cloned().unwrap_or(json!("")) }));
                }
                v["frames"] = json!(frames);
                v["video"] = json!({ "width": c["width"], "height": c["height"], "caption": c.get("caption").cloned().unwrap_or(json!("")) });
                if v.get("views").is_none() {
                    v["views"] = json!([{ "renderer": "sample.video" }]);
                }
            }
            "control" => {
                // checked like the model's: the tree and its bounds, `under` against this run's inputs
                let collector = Collector { contract: json!({ "results": [], "dynamic": true }), inputs: st.stage.lock().unwrap().input_infos()?, ..Default::default() };
                let mut raw = json!({ "name": name, "type": "control", "title": v["title"] });
                for k in ["kind", "config", "children", "under"] {
                    if let Some(x) = c.get(k) {
                        raw[k] = x.clone();
                    }
                }
                v = collector.normalize(&raw, "direct").map_err(|e| WsError::invalid_op(format!("Result {name}: {}", e.join("; "))))?;
            }
            t => return Err(WsError::invalid_op(format!("Mock result type {t} is not supported"))),
        }
        out.push(v);
    }
    Ok(out)
}

/// Values of a simulated result must still be values of their declared types (§8.6).
fn check_mock(results: &[Value]) -> WsResult<()> {
    use aiworkspace_store::wish::results::check_value;
    let mut bad = Vec::new();
    for r in results {
        let name = r["name"].as_str().unwrap_or("");
        for p in r["record"]["properties"].as_array().into_iter().flatten() {
            let (n, t) = (p["name"].as_str().unwrap_or(""), p["type"].as_str().unwrap_or("text"));
            if let Some(v) = r["record"]["props"].get(n) {
                if !check_value(t, v) {
                    bad.push(format!("\"{name}\": \"{n}\" is declared {t}, but its value is {v}"));
                }
            }
        }
        for f in r["table"]["fields"].as_array().into_iter().flatten() {
            let (n, t) = (f["name"].as_str().unwrap_or(""), f["type"].as_str().unwrap_or("text"));
            if let Some(v) = r["table"]["rows"].as_array().into_iter().flatten().filter_map(|row| row.get(n)).find(|v| !check_value(t, v)) {
                bad.push(format!("\"{name}\": field \"{n}\" is declared {t}, but has the value {v}"));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(WsError::invalid_schema(format!("The simulated results are invalid: {}", bad.join("; "))))
    }
}

/// An executor's reported analysis (M3): validated exactly like the model's `submit_analysis`.
async fn provided_analysis(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, provided: &Value) -> WsResult<()> {
    let (validated, evidence) = {
        let mut stage = st.stage.lock().unwrap();
        let v = stage.validate_analysis(&provided["analysis"])?;
        (v, stage.evidence()?)
    };
    let v = match validated {
        Ok(v) => v,
        Err(problems) => return Err(WsError::invalid_op("the reported analysis was not accepted").with_data(json!({ "problems": problems }))),
    };
    let candidate = json!({ "wish_id": evidence["wish_id"], "analysis": v.analysis, "inputs": v.inputs, "basis": evidence["basis"], "executed_at": now() });
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({})).await
}

async fn mock(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, provided: &Value, executor: &str) -> WsResult<()> {
    let provided = provided.clone();
    let mut results = mock_results(st, &provided)?;
    check_mock(&results)?;
    // video frames: staged now, kept as a record of frame assets (the phase-two frame preview)
    let mut frame_paths: Vec<(usize, usize, PathBuf)> = Vec::new();
    for (i, r) in results.iter().enumerate() {
        for (j, f) in r["frames"].as_array().into_iter().flatten().enumerate() {
            frame_paths.push((i, j, PathBuf::from(f["path"].as_str().unwrap_or(""))));
        }
    }
    if !frame_paths.is_empty() {
        let c = caller.clone();
        let fp = frame_paths.clone();
        let staged = with_ws(state, ws_id, move |ws| {
            fp.iter().map(|(_, _, p)| Ok(ws.stage_asset(&c, &std::fs::read(p).map_err(|e| WsError::io(e.to_string()))?)?)).collect::<WsResult<Vec<Value>>>()
        })
        .await?;
        for ((i, j, _), s) in frame_paths.iter().zip(staged) {
            results[*i]["frames"][*j]["object_id"] = s["object_id"].clone();
            results[*i]["frames"][*j]["media_type"] = s["media_type"].clone();
        }
        for r in &mut results {
            if r["type"] != json!("video") {
                continue;
            }
            let frames: Vec<Value> = r["frames"].as_array().cloned().unwrap_or_default().iter()
                .map(|f| json!({ "object_id": f["object_id"], "media_type": f["media_type"], "duration_ms": f["duration_ms"], "caption": f["caption"] })).collect();
            let duration: u64 = frames.iter().filter_map(|f| f["duration_ms"].as_u64()).sum();
            r["record"] = json!({
                "properties": [{ "name": "Frames", "type": "text", "key": "frames" }, { "name": "Duration (ms)", "type": "number", "key": "duration" }, { "name": "Caption", "type": "text", "key": "caption" },
                               { "name": "Width", "type": "number", "key": "width" }, { "name": "Height", "type": "number", "key": "height" }],
                "props": { "Frames": serde_json::to_string(&frames).unwrap_or_default(), "Duration (ms)": duration, "Caption": r["video"]["caption"], "Width": r["video"]["width"], "Height": r["video"]["height"] },
            });
            r.as_object_mut().unwrap().remove("frames");
            r.as_object_mut().unwrap().remove("video");
        }
    }
    let evidence = {
        let stage = st.stage.lock().unwrap();
        let mut ev = stage.evidence()?;
        // what the browser executor read is the read set of a simulated run
        ev["read_set"] = provided.get("read_set").cloned().unwrap_or(json!([]));
        ev["appended_inputs"] = json!([]);
        ev
    };
    let mut candidate = json!({
        "schema_version": aiworkspace_core::wish::RESULTS_SCHEMA, "results": results, "facts": {}, "checks": [],
        "summary": provided.get("summary").cloned().unwrap_or(json!("")), "assumptions": provided.get("assumptions").cloned().unwrap_or(json!([])),
        "warnings": provided.get("warnings").cloned().unwrap_or(json!([])), "external_data": [], "model_judgment": [], "refinements": [],
    });
    stage_files(state, caller, ws_id, &mut candidate).await?;
    fill_candidate(&mut candidate, &evidence, executor, st, "generate");
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({})).await
}

// ---- the stage host (内核架构改进 §9, T3-4): an executor outside the service drives the model loop and
// runs the program; the service keeps the snapshot, answers the stage tools, validates and plans

/// What an outside executor starts a run from, opened at `proc.claim`: the stage's prompts and tool
/// catalogue (a model stage), or the stored program (re-running only the program). The stage stays
/// open under the run id until the executor reports, the run is cancelled, or its lease lapses.
pub fn host_open(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str) -> WsResult<Value> {
    state.wish.drop_hosted(run_id);
    let active = state.wish.register(run_id);
    *active.progress.lock().unwrap() = json!({ "phase": "claimed" });
    let Opened { st, params, stage_name, .. } = match open_stage(state, caller, ws_id, run_id, &active, false) {
        Ok(o) => o,
        Err(e) => {
            state.wish.unregister(run_id);
            return Err(e);
        }
    };
    let cfg = &state.wish.config;
    let brief = match stage_name.as_str() {
        "analyze" => {
            let (user, _) = analyze_user(&st)?;
            json!({ "kind": "model", "stage": stage_name, "system": ANALYZE_SYSTEM, "user": user, "tools": tool_specs(&ANALYZE_KINDS), "model": cfg.analyze_model, "iterations": cfg.analyze_tool_iterations })
        }
        "rerun_program" => {
            let source = rerun_prepare(&st)?;
            json!({ "kind": "program", "stage": stage_name, "source": source })
        }
        _ => {
            let (mut user, _, existing) = execute_prepare(state, caller, ws_id, &st, &params)?;
            if let Some(src) = &existing {
                user.push_str(&format!("\n\n## Current program (program/main.js)\n\n```js\n{src}\n```\n"));
            }
            user.push_str("\n\nThere is no shell and no file tool here: give run_program the complete program as its `source`; the host runs it and keeps it as program/main.js.");
            json!({ "kind": "model", "stage": stage_name, "system": execute_system(), "user": user, "tools": tool_specs(&execute_kinds()), "model": cfg.execute_model, "iterations": cfg.execute_tool_iterations, "source": existing })
        }
    };
    set_progress(&active, "running");
    state.wish.set_hosted(run_id, st);
    Ok(brief)
}

/// The tools of a stage as a model sees them; `run_program` takes the program text here.
fn tool_specs(kinds: &[Kind]) -> Vec<Value> {
    kinds
        .iter()
        .map(|k| {
            let (description, mut parameters) = k.spec();
            if *k == Kind::RunProgram {
                parameters = json!({ "type": "object", "properties": { "source": { "type": "string", "description": "The complete program/main.js (an ES module exporting `default async function main(aiws)`)" } }, "required": ["source"] });
            }
            json!({ "name": k.name(), "description": description, "parameters": parameters })
        })
        .collect()
}

/// `proc.tool`: one stage tool call of an outside executor. `llm_map` serves a program's `aiws.llm.map`
/// (the service calls the model under the run's attribution and keeps the cache); `run_program` is
/// `proc.program`.
pub async fn host_tool(st: &Arc<StageState>, name: &str, args: &Value) -> Value {
    if st.cancelled() {
        return json!({ "ok": true, "status": "error", "text": "the run was cancelled" });
    }
    if name == "llm_map" {
        return json!({ "ok": true, "status": "ok", "map": llm::llm_map(st, args).await });
    }
    let Some(kind) = super::tools::kind_of(name) else {
        return json!({ "ok": true, "status": "error", "text": format!("unknown tool {name}") });
    };
    if kind == Kind::RunProgram {
        return json!({ "ok": true, "status": "error", "text": "run the program on the executor and report it with proc.program" });
    }
    st.note_tool(name);
    let (s, a) = (st.clone(), args.clone());
    let answer = tokio::task::spawn_blocking(move || super::tools::blocking_call(&s, kind, &a)).await.unwrap_or_else(|e| Err(e.to_string()));
    match answer {
        Ok(text) => json!({ "ok": true, "status": "ok", "text": text }),
        Err(text) => json!({ "ok": true, "status": "error", "text": text }),
    }
}

/// `proc.program`: the executor ran `source` on the stage's snapshot and hands in what the program host
/// collected (`output/.aiws/results.json`), the files it wrote under `output/` and its log; the service
/// judges it exactly like its own `run_program`.
pub fn host_program(st: &Arc<StageState>, source: &str, results: Option<&Value>, log: &str, files: &Value) -> WsResult<Value> {
    if st.cancelled() {
        return Ok(json!({ "ok": true, "status": "error", "text": "the run was cancelled" }));
    }
    st.note_tool("run_program");
    write(&st.workdir.join("program/main.js"), source)?;
    let out = st.workdir.join("output");
    for (path, content) in files.as_object().into_iter().flatten() {
        let rel = Path::new(path.strip_prefix("output/").unwrap_or(path));
        if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) || rel.as_os_str().is_empty() {
            return Err(WsError::invalid_op(format!("program file {path}: only paths under output/")));
        }
        let bytes = match content {
            Value::String(text) => text.as_bytes().to_vec(),
            Value::Object(o) => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(o["base64"].as_str().unwrap_or("")).map_err(|e| WsError::invalid_op(format!("program file {path}: {e}")))?
            }
            _ => return Err(WsError::invalid_op(format!("program file {path}: text or {{ base64 }}"))),
        };
        let target = out.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WsError::io(e.to_string()))?;
        }
        std::fs::write(&target, bytes).map_err(|e| WsError::io(e.to_string()))?;
    }
    if let Some(r) = results {
        write(&out.join(".aiws/results.json"), &r.to_string())?;
    }
    let answer = super::tools::judge_program(st, results, "the executor reported no results", log, "");
    Ok(match answer {
        Ok(text) => json!({ "ok": true, "status": "ok", "text": text }),
        Err(text) => json!({ "ok": true, "status": "error", "text": text }),
    })
}

/// `proc.files`: the stage directory an executor's program host reads (`context/`, `request.json`,
/// `catalog/`, and `lib/` so a Deno executor runs `lib/run.js` as the service would), text as is, bytes base64.
pub fn host_files(st: &Arc<StageState>) -> WsResult<Value> {
    use base64::Engine;
    let mut files = serde_json::Map::new();
    let root = st.workdir.clone();
    let mut stack = vec![root.join("context"), root.join("catalog"), root.join("lib")];
    let mut paths = vec![root.join("request.json"), root.join("WORKSPACE.md")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                paths.push(p);
            }
        }
    }
    for p in paths {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let rel = p.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        if bytes.len() > 16 * 1024 * 1024 {
            files.insert(rel, json!({ "skipped": "larger than 16 MiB" }));
            continue;
        }
        match String::from_utf8(bytes) {
            Ok(text) => files.insert(rel, json!({ "text": text })),
            Err(e) => files.insert(rel, json!({ "base64": base64::engine::general_purpose::STANDARD.encode(e.into_bytes()) })),
        };
    }
    Ok(json!({ "ok": true, "files": files }))
}

/// The executor reported it is done (`proc.report { finished: true }`): what the stage collected
/// becomes the candidate, planned and left waiting for confirmation; failures end the run.
pub async fn drive_host(state: Arc<AppState>, caller: Caller, ws_id: String, run_id: String, st: Arc<StageState>, repair: bool) {
    let res = host_finish(&state, &caller, &ws_id, &st, repair).await;
    settle(&state, caller, &ws_id, &run_id, &st.active, res).await;
    state.wish.drop_hosted(&run_id);
    state.wish.unregister(&run_id);
}

async fn host_finish(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, repair: bool) -> WsResult<()> {
    let kind = st.stage.lock().unwrap().kind;
    let external = json!({ "status": "external" });
    match kind {
        StageKind::Analyze => {
            let evidence = st.stage.lock().unwrap().evidence()?;
            analyze_tail(state, caller, ws_id, st, evidence, external).await
        }
        StageKind::Program => {
            let log = st.collector.lock().unwrap().program.as_ref().map(|p| p.logs.clone()).unwrap_or_default();
            rerun_tail(state, caller, ws_id, st, log).await
        }
        StageKind::Execute => execute_tail(state, caller, ws_id, st, external, repair).await,
    }
}
