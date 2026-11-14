//! The stage host (内核架构改进 §9; 浏览器优先; AI Native 第一期 §2): every stage of a wish run executes on an executor
//! outside the service. At `proc.claim` the service takes the stage snapshot, builds the stage directory and answers
//! the stage's brief — a model stage's prompts, tools and budget (analyze, code), or the stored macro with its manifest
//! (run). It answers the stage tools (`ws_*`, `submit_analysis`, `check_results`, `finish`, the macro's `sql`, the
//! `llm_cache`), judges each macro run the executor hands in (`proc.program`), and when the executor reports it is
//! done, plans what the stage collected and leaves it waiting for confirmation. Failures and cancellation never touch
//! the Workspace. The Mock executor's simulated results are planned here directly.

use super::tools::Kind;
use super::{ActiveRun, StageState, WishConfig, AIWS_JS, ANALYZE_SYSTEM, CODE_SYSTEM, GUIDE, HOST_JS, RUN_JS};
use crate::AppState;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::wish::{STAGE_ANALYZE, STAGE_CODE, STAGE_RUN};
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::wish::context::{Stage, StageKind, StageOptions};
use aiworkspace_store::wish::results::{code_changed, program_output, Collector};
use aiworkspace_store::Caller;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Keys `llm_cache` takes per call, and the size of one cached value.
const CACHE_MAX_KEYS: usize = 2000;
const CACHE_MAX_VALUE_BYTES: usize = 64 * 1024;

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

/// The worker of a run the service plans by itself (a Mock's simulated results, an analysis an executor reported):
/// never panics into the service, always leaves a final state.
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
                if let Some(log) = err.get("data").and_then(|d| d.get("program_log")).cloned() {
                    detail["program_log"] = log;
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

/// The stored macro of a wish this host runs (program API v3): its source and manifest.
fn stored_macro(stage: &Stage) -> Option<(String, Value)> {
    let prog = stage.wish.payload.get("program")?;
    if prog.get("api_version").and_then(Value::as_u64) != Some(aiworkspace_core::wish::PROGRAM_API_VERSION) {
        return None;
    }
    let src = String::from_utf8(stage.snap.asset_bytes(prog.get("source")?.as_str()?).ok()?).ok()?;
    Some((src, prog.get("manifest").cloned().unwrap_or(Value::Null)))
}

/// What every stage hands its executor to stay within (浏览器优先 T1-4, 第一期 §2.3–§2.4): the deployment's limits.
/// The executor takes the smaller of these and what the macro declares (an undeclared AI capability is refused).
pub fn budget(cfg: &WishConfig, stage: &str) -> Value {
    json!({
        "iterations": if stage == STAGE_ANALYZE { cfg.analyze_tool_iterations } else { cfg.execute_tool_iterations },
        "program_timeout_secs": cfg.program_timeout_secs,
        "max_output_tokens": cfg.max_output_tokens,
        "map_model": cfg.map_model,
        "complete_model": cfg.complete_model,
        "llm_map_max_items": cfg.llm_map_max_items,
        "ai_max_calls": cfg.ai_max_calls,
        "ai_max_tokens": cfg.ai_max_tokens,
        "sql_max_calls": cfg.sql_max_calls,
        "image_model": cfg.image_model,
        "video_model": cfg.video_model,
        "image_max_calls": cfg.image_max_calls,
        "video_max_calls": cfg.video_max_calls,
        "media_timeout_secs": cfg.media_timeout_secs,
    })
}

/// A stage opened for a run: its state and what the run record said.
pub struct Opened {
    pub st: Arc<StageState>,
    pub run: Value,
    pub params: Value,
    pub stage_name: String,
    pub program: String,
}

/// Take the stage snapshot, build the stage directory and the shared state of a run's tools. With `transition` the run
/// record moves queued → snapshotting → running (a run the service plans itself); an executor already holds the run in
/// `running` under its lease.
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
    let workdir = ws_dir.join("runs").join(run_id).join("work");
    std::fs::create_dir_all(&workdir).map_err(|e| WsError::io(e.to_string()))?;
    let mock = program == super::PROGRAM_MOCK;
    let kind = match stage_name.as_str() {
        STAGE_ANALYZE => StageKind::Analyze,
        STAGE_CODE => StageKind::Code,
        _ => StageKind::Run,
    };
    // coding and running read as the macro identity (§4.5): bounded by the wish's group when it is bound with
    // `intersection`, so a macro never reads what it could not write about
    let as_macro = Caller { macro_of: super::macro_scope(&run), ..caller.clone() };
    let snap = state.with_ws(ws_id, |ws| ws.wish_snapshot(&as_macro))?;
    // running the stored macro: only what its manifest declares is materialized
    let manifest = if kind == StageKind::Run && !mock {
        snap.readable(&wish_id)?.payload.get("program").and_then(|p| p.get("manifest")).cloned().filter(|m| m.is_object())
    } else {
        None
    };
    let request = request_params(&params);
    let opts = StageOptions {
        kind,
        location: params.get("location").cloned().unwrap_or(json!({})),
        request: request.clone(),
        catalog: super::builtin_catalog(),
        controls: super::controls_catalog(),
        map_budget_chars: state.wish.config.map_budget_chars,
        materialize: kind != StageKind::Analyze && !mock,
        manifest,
    };
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
    // the program host every executor runs the macro with
    write(&workdir.join("lib/aiws.js"), AIWS_JS)?;
    write(&workdir.join("lib/run.js"), RUN_JS)?;
    write(&workdir.join("lib/host.js"), HOST_JS)?;
    let payload = wish_payload(&stage);
    let mut req = request.clone();
    if kind != StageKind::Run {
        for k in ["prompt", "knowledge", "refinements"] {
            if let Some(v) = payload.get(k) {
                req[k] = v.clone();
            }
        }
        req["context_prompt"] = payload["analysis"]["context_prompt"].clone();
    }
    req["stage"] = json!(stage_name);
    write(&workdir.join("request.json"), &serde_json::to_string_pretty(&req).unwrap_or_default())?;
    let feedback = params.get("feedback").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).map(str::to_string);
    let prompts = vec![
        payload["prompt"].as_str().unwrap_or("").to_string(),
        payload["knowledge"].as_str().unwrap_or("").to_string(),
        payload["analysis"]["context_prompt"].as_str().unwrap_or("").to_string(),
        feedback.clone().unwrap_or_default(),
    ];
    let stored = if kind == StageKind::Run && !mock { stored_macro(&stage) } else { None };
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
        stored,
        failure: Mutex::new(None),
        budget: budget(&state.wish.config, &stage_name),
        ai_user: run["detail"]["aicc"]["user_id"].as_str().unwrap_or("").to_string(),
        max_tool_rounds: run["detail"]["context"]["executor"]["limits"]["max_tool_iterations"].as_u64(),
        prompts,
        tool_calls: Default::default(),
        program_runs: Default::default(),
        sql_calls: Default::default(),
    });
    Ok(Opened { st, run, params, stage_name, program })
}

async fn drive_inner(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str, active: &Arc<ActiveRun>) -> WsResult<()> {
    let (st, c, w, r, a) = (state.clone(), caller.clone(), ws_id.to_string(), run_id.to_string(), active.clone());
    let Opened { st, run, params, program, .. } = blocking(move || open_stage(&st, &c, &w, &r, &a, true)).await?;
    let provided = run["detail"]["exec"]["provided"].clone();
    if provided.get("analysis").is_some_and(Value::is_object) {
        // an executor reported an analysis made elsewhere (第一期 §2.9): validated like the model's own
        return provided_analysis(state, caller, ws_id, &st, &provided).await;
    }
    if program == super::PROGRAM_MOCK {
        return mock(state, caller, ws_id, &st, &params["provided"], "mock").await;
    }
    Err(WsError::invalid_op("nothing to plan: the run's executor did not report"))
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
        let identity = super::run_identity(ws, &c, &run)?;
        let plan = super::plan_for(ws, &c, &run, &json!({}), &identity)?;
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


// ---- coding (第一期 §2.1): writing and trial-running the macro; repairing it; a feedback round

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
            l.push_str(" (input appended while coding)");
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


fn collector_for(stage: &Stage, stored_manifest: Option<Value>) -> WsResult<Collector> {
    let a = stage.wish.payload.get("analysis").cloned().unwrap_or(json!({}));
    Ok(Collector {
        contract: a.get("output_contract").cloned().unwrap_or(json!({ "results": [] })),
        checks_def: a.get("checks").and_then(Value::as_array).cloned().unwrap_or_default(),
        prev_bindings: aiworkspace_core::wish::current_results(&stage.wish.payload).into_iter().collect(),
        inputs: stage.input_infos()?,
        composites: stage.composite_defs()?,
        workdir: stage.workdir.clone(),
        stored_manifest,
        ..Default::default()
    })
}

/// What the coding stage starts from: the user message and the macro already there (`program/main.js`) — the parent
/// round's (a repair, a feedback round) or the stored one.
fn code_prepare(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, params: &Value) -> WsResult<(String, Option<String>)> {
    let parent = match params.get("parent_run_id").and_then(Value::as_str) {
        Some(pid) => {
            let run = state.with_ws(ws_id, |ws| ws.wish_run(caller, pid))?;
            let src = std::fs::read_to_string(st.ws_dir.join("runs").join(pid).join("work/program/main.js")).ok();
            Some((run, src))
        }
        None => None,
    };
    let stage = st.stage.lock().unwrap();
    super::ready(&stage.ctx(), &stage.wish.payload, STAGE_CODE)?;
    *st.collector.lock().unwrap() = collector_for(&stage, None)?;
    let p = wish_payload(&stage);
    let a = &p["analysis"];
    let existing = parent.as_ref().and_then(|(_, s)| s.clone()).or_else(|| stored_macro(&stage).map(|(s, _)| s));
    if let Some(src) = &existing {
        write(&stage.workdir.join("program/main.js"), src)?;
    }
    let map = std::fs::read_to_string(stage.workdir.join("WORKSPACE.md")).unwrap_or_default();
    let mut u = String::from("# Wish (coding stage)\n\n");
    u.push_str(&section("Original request", p["prompt"].as_str().unwrap_or("")));
    u.push_str(&section("Wish knowledge (definitions, background, preferences)", p["knowledge"].as_str().unwrap_or("")));
    u.push_str(&section("Refinements (must be followed)", &refinement_lines(&p)));
    u.push_str(&section("Task description (context prompt)", a["context_prompt"].as_str().unwrap_or("")));
    u.push_str(&section("Inputs (declare them with aiws.declare, then read them with aiws.input(name) or aiws.sql)", &input_lines(&stage)));
    u.push_str(&section("Output contract (every result comes from the macro)", &format!("```json\n{}\n```", serde_json::to_string_pretty(&a["output_contract"]).unwrap_or_default())));
    u.push_str(&section("Acceptance checks (the macro reports program checks with aiws.check; review checks are self-assessed in finish)", &format!("```json\n{}\n```", serde_json::to_string_pretty(&a["checks"]).unwrap_or_default())));
    u.push_str(&section("Fixed parameters (aiws.request)", &serde_json::to_string(&stage.request).unwrap_or_default()));
    u.push_str(&section(
        "Macro",
        if existing.is_some() { "program/main.js already has a macro (the stored one or the previous round's): make the smallest change to it, keeping result names, fields and keys stable." } else { "No macro yet: write it in program/main.js." },
    ));
    if let Some((prun, _)) = &parent {
        match &st.feedback {
            Some(feedback) => {
                let prev = &prun["candidate"];
                let results: Vec<Value> = prev["results"].as_array().into_iter().flatten().map(|r| json!({ "name": r["name"], "type": r["type"],
                    "rows": r["table"]["rows"].as_array().map(Vec::len), "fields": r["table"]["fields"], "excerpt": r["markdown"].as_str().map(|m| m.chars().take(300).collect::<String>()) })).collect();
                u.push_str(&section("Previous round's candidate", &format!("Results: {}\nChecks: {}\nfacts: {}\nSummary: {}", json!(results), prev["checks"], prev["facts"], prev["summary"].as_str().unwrap_or(""))));
                u.push_str(&section("User feedback (to be satisfied in this round)", feedback));
                u.push_str("Change the existing macro to satisfy the feedback; results the feedback does not affect stay unchanged. In finish, write the distilled requirements in refinements.\n\n");
            }
            None if prun["state"] == json!("failed") => {
                let err = &prun["detail"]["error"];
                u.push_str(&section("The macro failed when it ran", &format!("{}\n\n{}", err["detail"].as_str().unwrap_or(""), prun["detail"]["program_log"].as_str().unwrap_or(""))));
                u.push_str("Read the inputs' current schema and profile, make the smallest change that makes the macro run correctly on the data as it is now, then run_program and finish.\n\n");
            }
            None => {}
        }
    }
    u.push_str("---\n\n");
    u.push_str(&map);
    Ok((u, existing))
}

fn code_system() -> String {
    CODE_SYSTEM.replace("{AIWS_GUIDE}", GUIDE)
}

fn code_kinds() -> Vec<Kind> {
    let mut kinds = Kind::READ.to_vec();
    kinds.extend(Kind::DELIVERY);
    kinds
}

fn candidate_from(st: &Arc<StageState>, stage: &Stage, c: &Collector) -> WsResult<(Value, Value)> {
    let mut cand = c.candidate();
    let texts = st.prompt_texts();
    cand["uncited_numbers"] = json!(c.uncited_numbers(&texts.iter().map(String::as_str).collect::<Vec<_>>()));
    Ok((cand, stage.evidence()?))
}

/// The coding stage ended with `finish`: the last trial run is the candidate, with the macro version it ran (第一期
/// §2.1: applying it writes the macro and the results in one Commit).
async fn code_tail(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>) -> WsResult<()> {
    if st.cancelled() {
        return Ok(());
    }
    if st.collector.lock().unwrap().finished.is_none() {
        let tail = st.collector.lock().unwrap().program.as_ref().map(|p| p.logs.clone()).unwrap_or_default();
        return Err(WsError::new(Code::DependencyUnavailable, "The coding did not end with finish").with_data(json!({ "program_log": tail })));
    }
    let (mut candidate, evidence) = {
        let stage = st.stage.lock().unwrap();
        let c = st.collector.lock().unwrap();
        let (mut cand, ev) = candidate_from(st, &stage, &c)?;
        let refinements: Vec<Value> = c.finished.as_ref().and_then(|f| f["refinements"].as_array().cloned()).unwrap_or_default()
            .iter().filter_map(Value::as_str).filter(|t| !t.trim().is_empty())
            .map(|t| json!({ "text": t.chars().take(aiworkspace_core::wish::MAX_REFINEMENT_CHARS).collect::<String>(), "at": now(), "run_id": st.run_id })).collect();
        cand["refinements"] = json!(refinements);
        if let Some(p) = &c.program {
            cand["program"] = json!({ "digest": p.digest, "path": st.workdir.join("program/main.js").display().to_string(), "manifest": p.manifest,
                                      "produces": p.results.iter().map(|r| r["name"].clone()).collect::<Vec<_>>() });
        }
        (cand, ev)
    };
    stage_files(state, caller, ws_id, &mut candidate).await?;
    fill_candidate(&mut candidate, &evidence, "xllm", st, STAGE_CODE);
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({})).await
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
    let mut objects: Vec<(usize, String, String)> = Vec::new();
    for (i, r) in cand["results"].as_array().into_iter().flatten().enumerate() {
        if let Some(p) = r["file"]["path"].as_str() {
            paths.push((format!("r{i}"), PathBuf::from(p)));
        } else if let Some(id) = r["file"]["object_id"].as_str() {
            objects.push((i, id.to_string(), r["name"].as_str().unwrap_or("file").to_string()));
        }
    }
    if let Some(p) = cand["program"]["path"].as_str() {
        paths.push(("program".into(), PathBuf::from(p)));
    }
    if paths.is_empty() && objects.is_empty() {
        return Ok(());
    }
    let c = caller.clone();
    let (max, media) = (state.limits.max_asset_bytes, state.limits.max_media_bytes);
    let (staged, referenced) = with_ws(state, ws_id, move |ws| {
        let mut out = Vec::new();
        for (k, p) in paths {
            let bytes = std::fs::read(&p).map_err(|e| WsError::io(format!("{}: {e}", p.display())))?;
            out.push((k, ws.stage_asset(&c, &bytes)?));
        }
        // a file the macro holds by its object id (a generated picture or video, an asset input) is one this
        // Workspace already has or staged: staged again for the plan, never an object of the store at large
        let mut refs = Vec::new();
        for (i, id, name) in objects {
            if ws.asset_info(&id)?.is_none() {
                return Err(WsError::invalid_op(format!("result {name}: {id} is not a file of this Workspace (a macro writes files it generated or read)")));
            }
            let mut staged = ws.stage_upload(&c, &id, max, media)?;
            let media_type = staged["media_type"].as_str().unwrap_or("").to_string();
            let ext = match media_type.as_str() {
                "image/png" => ".png",
                "image/jpeg" => ".jpg",
                "image/webp" => ".webp",
                "image/gif" => ".gif",
                "video/mp4" => ".mp4",
                "video/webm" => ".webm",
                "video/quicktime" => ".mov",
                "audio/mpeg" => ".mp3",
                "audio/wav" | "audio/x-wav" => ".wav",
                _ => "",
            };
            staged["file_name"] = json!(format!("{name}{ext}"));
            refs.push((i, staged));
        }
        Ok((out, refs))
    })
    .await?;
    for (i, staged) in referenced {
        let f = &mut cand["results"][i]["file"];
        for field in ["object_id", "key", "version", "media_type", "size", "image", "file_name"] {
            if !staged[field].is_null() {
                f[field] = staged[field].clone();
            }
        }
    }
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


// ---- running the stored macro (第一期 §2.1): no model loop

/// The stored macro, written into the stage for its run; refused (`NEEDS_CODING`) when it no longer fits its inputs.
fn run_prepare(st: &Arc<StageState>) -> WsResult<(String, Value)> {
    let stage = st.stage.lock().unwrap();
    super::ready(&stage.ctx(), &stage.wish.payload, STAGE_RUN)?;
    let (src, manifest) = st.stored.clone().ok_or_else(|| WsError::sub(Code::InvalidOperation, "NEEDS_CODING", "This wish has no macro this service can run: code it first"))?;
    write(&stage.workdir.join("program/main.js"), &src)?;
    *st.collector.lock().unwrap() = collector_for(&stage, Some(manifest.clone()))?;
    Ok((src, manifest))
}

/// A run of the stored macro, judged: it must produce the contract with the code it fixed. A failure ends the run
/// (`PROGRAM_FAILED`, `MACRO_CODE_CHANGED`: code it again), it is never repaired here.
fn judge_run(st: &Arc<StageState>, results: Option<&Value>, log: &str) -> WsResult<String> {
    let fail = |code: &'static str, msg: String| WsError::sub(Code::InvalidOperation, code, msg).with_data(json!({ "program_log": log }));
    let raw = results.cloned().ok_or_else(|| fail("PROGRAM_FAILED", "The macro wrote no results".into()))?;
    if let Some(e) = raw.get("error").and_then(Value::as_str) {
        // a refusal of the AI the macro called is the run's reason, as for any model call (§7.4)
        let code = match raw.get("error_code").and_then(Value::as_str) {
            Some(c @ ("AI_QUOTA_EXHAUSTED" | "AI_QUOTA_BUDGET_EXCEEDED" | "AI_RESOURCE_NOT_PERMITTED" | "AI_ATTRIBUTION_UNAVAILABLE" | "AI_UNAVAILABLE")) => {
                return Err(WsError::sub(Code::DependencyUnavailable, ai_code(c), format!("The macro's AI call was refused: {}", e.lines().next().unwrap_or(""))).with_data(json!({ "program_log": log })));
            }
            _ => "PROGRAM_FAILED",
        };
        return Err(fail(code, format!("The macro threw an error: {}", e.lines().next().unwrap_or(""))));
    }
    let source = std::fs::read_to_string(st.workdir.join("program/main.js")).unwrap_or_default();
    let mut c = st.collector.lock().unwrap();
    let run = program_output(&c, &raw, &source, log).map_err(|errs| fail("PROGRAM_FAILED", format!("The macro's results failed validation: {}", errs.join("; "))))?;
    let changed = code_changed(run.manifest.as_ref().unwrap_or(&Value::Null), &run.results);
    if !changed.is_empty() {
        return Err(fail("MACRO_CODE_CHANGED", format!("The macro produced HTML code other than the code it was coded with ({}): code the wish again", changed.join(", "))));
    }
    let ai = run.ai.clone();
    c.program = Some(run);
    let blocking = c.blocking();
    if !blocking.is_empty() {
        c.program = None;
        return Err(fail("PROGRAM_FAILED", format!("The macro's results are incomplete: {}", blocking.join("; "))));
    }
    st.note_program_run(&ai);
    Ok("The macro ran; its results are collected.".into())
}

fn ai_code(c: &str) -> &'static str {
    match c {
        "AI_QUOTA_BUDGET_EXCEEDED" => "AI_QUOTA_BUDGET_EXCEEDED",
        "AI_RESOURCE_NOT_PERMITTED" => "AI_RESOURCE_NOT_PERMITTED",
        "AI_ATTRIBUTION_UNAVAILABLE" => "AI_ATTRIBUTION_UNAVAILABLE",
        "AI_UNAVAILABLE" => "AI_UNAVAILABLE",
        _ => "AI_QUOTA_EXHAUSTED",
    }
}

/// The run of the stored macro becomes the candidate.
async fn run_tail(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>) -> WsResult<()> {
    if let Some(e) = st.failure.lock().unwrap().take() {
        return Err(e);
    }
    let (mut candidate, evidence, log) = {
        let stage = st.stage.lock().unwrap();
        let mut c = st.collector.lock().unwrap();
        let Some(log) = c.program.as_ref().map(|p| p.logs.clone()) else {
            return Err(WsError::sub(Code::InvalidOperation, "PROGRAM_FAILED", "The macro did not run"));
        };
        c.finished = Some(json!({ "summary": "Ran the macro (no model loop)" }));
        let (cand, ev) = candidate_from(st, &stage, &c)?;
        (cand, ev, log)
    };
    stage_files(state, caller, ws_id, &mut candidate).await?;
    fill_candidate(&mut candidate, &evidence, "xllm", st, STAGE_RUN);
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
                let stage = st.stage.lock().unwrap();
                let collector = Collector { contract: json!({ "results": [], "dynamic": true }), inputs: stage.input_infos()?, composites: stage.composite_defs()?, ..Default::default() };
                drop(stage);
                let mut raw = json!({ "name": name, "type": "control", "title": v["title"] });
                for k in ["kind", "config", "children", "under", "def", "slots"] {
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

/// An executor's reported analysis (第一期 §2.9): validated exactly like the model's `submit_analysis`.
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
    fill_candidate(&mut candidate, &evidence, executor, st, "run");
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({})).await
}

// ---- the stage host

/// What an executor starts a run from, opened at `proc.claim`: a model stage's prompts, tools and budget (analyze,
/// code), or the stored macro and its manifest (run). The stage stays open under the run id until the executor
/// reports, the run is cancelled, or its lease lapses.
pub fn host_open(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str) -> WsResult<Value> {
    state.wish.drop_hosted(run_id);
    let active = state.wish.register(run_id);
    *active.progress.lock().unwrap() = json!({ "phase": "claimed" });
    let opened = open_stage(state, caller, ws_id, run_id, &active, false).and_then(|Opened { st, params, stage_name, .. }| {
        let cfg = &state.wish.config;
        let brief = match stage_name.as_str() {
            STAGE_ANALYZE => {
                let (user, _) = analyze_user(&st)?;
                json!({ "kind": "model", "stage": stage_name, "system": ANALYZE_SYSTEM, "user": user, "tools": tool_specs(&ANALYZE_KINDS), "model": cfg.analyze_model,
                        "iterations": cfg.analyze_tool_iterations, "budget": st.budget })
            }
            STAGE_RUN => {
                let (source, manifest) = run_prepare(&st)?;
                json!({ "kind": "program", "stage": stage_name, "source": source, "manifest": manifest, "budget": st.budget })
            }
            _ => {
                let (mut user, existing) = code_prepare(state, caller, ws_id, &st, &params)?;
                if let Some(src) = &existing {
                    user.push_str(&format!("\n\n## Current macro (program/main.js)\n\n```js\n{src}\n```\n"));
                }
                user.push_str("\n\nThere is no shell and no file tool here: give run_program the complete macro as its `source`; the executor runs it exactly as every later run will, and it is kept as program/main.js.");
                json!({ "kind": "model", "stage": stage_name, "system": code_system(), "user": user, "tools": tool_specs(&code_kinds()), "model": cfg.execute_model,
                        "iterations": cfg.execute_tool_iterations, "source": existing, "budget": st.budget })
            }
        };
        Ok((st, brief))
    });
    let (st, brief) = match opened {
        Ok(o) => o,
        Err(e) => {
            state.wish.unregister(run_id);
            return Err(e);
        }
    };
    set_progress(&active, "running");
    state.wish.set_hosted(run_id, st);
    Ok(brief)
}

/// The tools of a stage as a model sees them.
fn tool_specs(kinds: &[Kind]) -> Vec<Value> {
    kinds
        .iter()
        .map(|k| {
            let (description, parameters) = k.spec();
            json!({ "name": k.name(), "description": description, "parameters": parameters })
        })
        .collect()
}

fn tool_error(text: impl Into<String>, code: Option<&str>) -> Value {
    let mut v = json!({ "ok": true, "status": "error", "text": text.into() });
    if let Some(c) = code {
        v["code"] = json!(c);
    }
    v
}

/// `proc.tool`: one call of an executor. The stage tools answer text; `sql` serves a macro's `aiws.sql` and
/// `llm_cache` the cache of its AI calls (the executor calls the model, the service only keeps the answers);
/// `run_program` is `proc.program`.
pub async fn host_tool(st: &Arc<StageState>, name: &str, args: &Value) -> Value {
    if st.cancelled() {
        return tool_error("the run was cancelled", Some("RUN_CANCELLED"));
    }
    if name == "sql" || name == "llm_cache" {
        let (s, a, n) = (st.clone(), args.clone(), name.to_string());
        return tokio::task::spawn_blocking(move || if n == "sql" { macro_sql(&s, &a) } else { llm_cache(&s, &a) }).await.unwrap_or_else(|e| tool_error(e.to_string(), None));
    }
    let Some(kind) = super::tools::kind_of(name) else {
        return tool_error(format!("unknown tool {name}"), None);
    };
    if kind == Kind::RunProgram {
        return tool_error("run the macro on the executor and hand it in with proc.program", None);
    }
    st.note_tool(name);
    let (s, a) = (st.clone(), args.clone());
    let answer = tokio::task::spawn_blocking(move || super::tools::blocking_call(&s, kind, &a)).await.unwrap_or_else(|e| Err(e.to_string()));
    match answer {
        Ok(text) => json!({ "ok": true, "status": "ok", "text": text }),
        Err(text) => tool_error(text, None),
    }
}

/// `sql { query, params?, declare? }` (第一期 §2.3): the declared table inputs only, with their declared fields under
/// the names the macro uses — the stored manifest when running, the macro's own `declare` while coding.
fn macro_sql(st: &Arc<StageState>, args: &Value) -> Value {
    let mut stage = st.stage.lock().unwrap();
    if stage.kind == StageKind::Analyze {
        return tool_error("aiws.sql runs inside a macro (coding or running)", None);
    }
    let Some(query) = args.get("query").and_then(Value::as_str) else { return tool_error("aiws.sql(query, params?): query required", None) };
    if !st.take_sql_call() {
        return tool_error(format!("aiws.sql budget exceeded: at most {} queries per run", st.budget["sql_max_calls"]), Some("SQL_BUDGET_EXCEEDED"));
    }
    let mut inputs: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    match &st.stored {
        Some((_, manifest)) => {
            for (name, d) in manifest["inputs"].as_object().into_iter().flatten() {
                let cols = d["fields"].as_array().into_iter().flatten().filter_map(|f| Some((f["field_id"].as_str()?.to_string(), f["name"].as_str()?.to_string()))).collect();
                inputs.insert(name.clone(), cols);
            }
        }
        None => {
            let Some(declared) = args.get("declare").and_then(|d| d.get("inputs")).and_then(Value::as_object) else {
                return tool_error("aiws.sql: declare the inputs first (aiws.declare({ inputs: { name: [fields] } }))", Some("NOT_DECLARED"));
            };
            for (name, spec) in declared {
                let wanted: Vec<String> = match spec {
                    Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                    Value::Object(o) => o.get("fields").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
                    _ => Vec::new(),
                };
                let fields = match stage.input_fields(name) {
                    Ok(Some((_, f))) => f,
                    Ok(None) => return tool_error(format!("aiws.declare: there is no input named {name}"), Some("NOT_DECLARED")),
                    Err(e) => return tool_error(e.detail, None),
                };
                let mut cols = Vec::new();
                for w in wanted {
                    match fields.iter().find(|(_, n, _)| *n == w) {
                        Some((id, n, _)) => cols.push((id.clone(), n.clone())),
                        None => return tool_error(format!("aiws.declare: input {name} has no field {w}"), Some("NOT_DECLARED")),
                    }
                }
                inputs.insert(name.clone(), cols);
            }
        }
    }
    match stage.macro_sql(query, args.get("params"), &inputs) {
        Ok(r) => json!({ "ok": true, "status": "ok", "sql": { "columns": r["columns"], "rows": r["rows"], "truncated": r["truncated"] } }),
        Err(e) => tool_error(e.detail.clone(), Some(e.sub.unwrap_or(e.code.as_str()))),
    }
}

fn cache_key_ok(k: &str) -> bool {
    k.len() == 64 && k.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// `llm_cache { get: [key…] }` → `{ hits: { key: value } }`; `{ put: [{ key, value }] }` → `{ ok }` (浏览器优先 T1-5).
/// The workspace's cache of AI answers, by content: the executor computes the keys and calls the model.
fn llm_cache(st: &Arc<StageState>, args: &Value) -> Value {
    let user = aiworkspace_core::canonical::sha256_hex(st.ai_user.as_bytes());
    let dir = st.ws_dir.join("cache").join("llm_map").join(&user[..32]);
    if let Some(keys) = args.get("get").and_then(Value::as_array) {
        if keys.len() > CACHE_MAX_KEYS {
            return tool_error(format!("llm_cache: at most {CACHE_MAX_KEYS} keys per call"), None);
        }
        let mut hits = Map::new();
        for k in keys.iter().filter_map(Value::as_str) {
            if !cache_key_ok(k) {
                return tool_error(format!("llm_cache: {k} is not a sha256 hex key"), None);
            }
            if let Some(v) = std::fs::read_to_string(dir.join(format!("{k}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                hits.insert(k.to_string(), v);
            }
        }
        return json!({ "ok": true, "status": "ok", "hits": hits });
    }
    if let Some(items) = args.get("put").and_then(Value::as_array) {
        if items.len() > CACHE_MAX_KEYS {
            return tool_error(format!("llm_cache: at most {CACHE_MAX_KEYS} values per call"), None);
        }
        let _ = std::fs::create_dir_all(&dir);
        for it in items {
            let (Some(k), Some(v)) = (it["key"].as_str(), it.get("value")) else { return tool_error("llm_cache put: [{ key, value }]", None) };
            let text = v.to_string();
            if !cache_key_ok(k) || text.len() > CACHE_MAX_VALUE_BYTES {
                return tool_error(format!("llm_cache: {k} is not a sha256 hex key, or its value is over {} KiB", CACHE_MAX_VALUE_BYTES / 1024), None);
            }
            let _ = std::fs::write(dir.join(format!("{k}.json")), text);
        }
        return json!({ "ok": true, "status": "ok" });
    }
    tool_error("llm_cache: { get: [keys] } or { put: [{ key, value }] }", None)
}

/// `proc.program`: the executor ran `source` on the stage's snapshot and hands in what the program host collected
/// (`output/.aiws/results.json`), the files it wrote under `output/` and its log. While coding it is a trial run the
/// model reads about; running, it is the stored macro's run, and a failure ends the run.
pub fn host_program(st: &Arc<StageState>, source: &str, results: Option<&Value>, log: &str, files: &Value) -> WsResult<Value> {
    if st.cancelled() {
        return Ok(tool_error("the run was cancelled", Some("RUN_CANCELLED")));
    }
    let kind = st.stage.lock().unwrap().kind;
    if kind == StageKind::Analyze {
        return Ok(tool_error("an analysis has no macro to run", None));
    }
    if let Some((stored, _)) = &st.stored {
        if stored != source {
            return Ok(tool_error("running a wish runs its stored macro, unchanged: code it to change the macro", Some("MACRO_CODE_CHANGED")));
        }
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
    if kind == StageKind::Run {
        return Ok(match judge_run(st, results, log) {
            Ok(text) => json!({ "ok": true, "status": "ok", "text": text }),
            Err(e) => {
                let v = tool_error(e.detail.clone(), e.sub);
                *st.failure.lock().unwrap() = Some(e);
                v
            }
        });
    }
    let answer = super::tools::judge_program(st, results, "the executor reported no results", log, "");
    Ok(match answer {
        Ok(text) => json!({ "ok": true, "status": "ok", "text": text }),
        Err(text) => tool_error(text, None),
    })
}

/// `proc.files`: the stage directory an executor's program host reads (`context/`, `request.json`, `catalog/`,
/// `lib/` — the program host, `program/main.js` when there is one), text as is, bytes base64, and the stage's
/// `budget` and stored `manifest`. More than the snapshot budget is `SNAPSHOT_TOO_LARGE` (浏览器优先 Q6).
pub fn host_files(st: &Arc<StageState>) -> WsResult<Value> {
    use base64::Engine;
    let mut files = serde_json::Map::new();
    let root = st.workdir.clone();
    let mut stack = vec![root.join("context"), root.join("catalog"), root.join("lib")];
    let mut paths = vec![root.join("request.json"), root.join("WORKSPACE.md"), root.join("program/main.js")];
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
    let limit = st.runtime.config.snapshot_download_max_bytes;
    let mut total: u64 = 0;
    for p in paths {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        total += bytes.len() as u64;
        if total > limit {
            let all: u64 = walk_size(&root);
            return Err(WsError::sub(Code::LimitExceeded, "SNAPSHOT_TOO_LARGE", format!("The inputs of this run are about {} MiB, over the {} MiB a browser may download: narrow the inputs (a table view), or run it hosted", all / (1024 * 1024), limit / (1024 * 1024)))
                .with_data(json!({ "bytes": all, "limit": limit })));
        }
        let rel = p.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        match String::from_utf8(bytes) {
            Ok(text) => files.insert(rel, json!({ "text": text })),
            Err(e) => files.insert(rel, json!({ "base64": base64::engine::general_purpose::STANDARD.encode(e.into_bytes()) })),
        };
    }
    let manifest = st.stored.as_ref().map(|(_, m)| m.clone()).unwrap_or(Value::Null);
    Ok(json!({ "ok": true, "files": files, "stage": { "budget": st.budget, "manifest": manifest } }))
}

fn walk_size(dir: &Path) -> u64 {
    let mut total = 0;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "output") {
                continue;
            }
            total += walk_size(&p);
        } else {
            total += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

/// The executor reported it is done (`proc.report { finished: true }`): what the stage collected becomes the
/// candidate, planned and left waiting for confirmation; failures end the run.
pub async fn drive_host(state: Arc<AppState>, caller: Caller, ws_id: String, run_id: String, st: Arc<StageState>) {
    let res = host_finish(&state, &caller, &ws_id, &st).await;
    settle(&state, caller, &ws_id, &run_id, &st.active, res).await;
    state.wish.drop_hosted(&run_id);
    state.wish.unregister(&run_id);
}

async fn host_finish(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>) -> WsResult<()> {
    let kind = st.stage.lock().unwrap().kind;
    match kind {
        StageKind::Analyze => {
            let evidence = st.stage.lock().unwrap().evidence()?;
            analyze_tail(state, caller, ws_id, st, evidence, json!({ "status": "external" })).await
        }
        StageKind::Code => code_tail(state, caller, ws_id, st).await,
        StageKind::Run => run_tail(state, caller, ws_id, st).await,
    }
}
