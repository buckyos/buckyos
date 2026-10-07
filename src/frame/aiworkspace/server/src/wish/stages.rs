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
    let cancelled = active.cancelled.load(Ordering::SeqCst);
    let progress = active.progress.lock().unwrap().clone();
    let (rid, final_state) = (run_id.clone(), if cancelled { "cancelled" } else { "failed" });
    let outcome = match res {
        Ok(()) if !cancelled => None,
        Ok(()) => Some(json!({ "code": "RUN_CANCELLED", "detail": "已取消" })),
        Err(e) => Some(if cancelled { json!({ "code": "RUN_CANCELLED", "detail": "已取消" }) } else { e.to_json() }),
    };
    let _ = with_ws(&state, &ws_id, move |ws| {
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
    state.wish.unregister(&run_id);
}

fn wish_payload(stage: &Stage) -> Value {
    Value::Object(stage.wish.payload.clone())
}

async fn drive_inner(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str, active: &Arc<ActiveRun>) -> WsResult<()> {
    let (c, rid) = (caller.clone(), run_id.to_string());
    let (run, ws_dir) = with_ws(state, ws_id, move |ws| {
        let r = ws.wish_run(&c, &rid)?;
        ws.wish_run_set(&rid, Some("snapshotting"), None, None, None, Some(&json!({ "progress": { "phase": "snapshotting" } })))?;
        Ok((r, ws.dir.clone()))
    })
    .await?;
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
        map_budget_chars: state.wish.config.map_budget_chars,
        materialize: kind != StageKind::Analyze && program != super::PROGRAM_MOCK,
    };
    let c = caller.clone();
    let snap = with_ws(state, ws_id, move |ws| ws.wish_snapshot(&c)).await?;
    let (wid, rid, wd) = (wish_id.clone(), run_id.to_string(), workdir.clone());
    let stage = blocking(move || Stage::open(snap, &wid, &rid, &wd, &opts)).await?;
    // the snapshot is fixed: the stage runs from here on (a cancel that came first stays)
    let (c, rid) = (caller.clone(), run_id.to_string());
    with_ws(state, ws_id, move |ws| {
        if ws.wish_run(&c, &rid)?["state"] == json!("snapshotting") {
            ws.wish_run_set(&rid, Some("running"), None, None, None, None)?;
        }
        Ok(())
    })
    .await?;
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
        prompts,
        tool_calls: Default::default(),
        program_runs: Default::default(),
        map_items: Default::default(),
        map_calls: Default::default(),
        map_cached: Default::default(),
    });
    state.wish.stages.lock().unwrap().insert(token.clone(), st.clone());
    let res = match (program.as_str(), stage_name.as_str()) {
        (super::PROGRAM_MOCK, _) => mock(state, caller, ws_id, &st, &params).await,
        (_, "analyze") => analyze(state, caller, ws_id, &st, &run_dir).await,
        (_, "rerun_program") => rerun(state, caller, ws_id, &st).await,
        _ => execute(state, caller, ws_id, &st, &run_dir, &params).await,
    };
    state.wish.stages.lock().unwrap().remove(&token);
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
    let llm_ctx = llm::llm_context(cfg, model, iterations, &run_dir.join("xllm"), &names, shell);
    write(&ctx_file, &serde_json::to_string_pretty(&llm_ctx).unwrap_or_default())?;
    let mut deps = XllmDeps::default().with_observer(Arc::new(Observer { active: st.active.clone() })).with_lock_dir(run_dir.join("locks"));
    for t in tools(st, kinds) {
        deps = deps.with_host_tool(t);
    }
    if let Some(l) = &cfg.llm_override {
        deps = deps.with_llm(l.clone());
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
        .map_err(|e| WsError::new(Code::DependencyUnavailable, format!("准备模型运行失败：{e}")))?;
    // nothing but this stage's configuration may shape the run (§13.3)
    let own = ctx_file.canonicalize().map(|p| p.display().to_string()).unwrap_or_default();
    if prepared.merged.files.iter().any(|f| *f != own && std::path::Path::new(f).canonicalize().map(|p| p.display().to_string()).unwrap_or_default() != own) {
        return Err(WsError::invalid_op(format!("unexpected .llm_context files above the run directory: {:?}", prepared.merged.files)));
    }
    let mut run = XllmRun::start(prepared, deps).await.map_err(|e| WsError::new(Code::DependencyUnavailable, format!("启动模型运行失败：{e}")))?;
    *st.active.interrupter.lock().unwrap() = Some(run.interrupter());
    if st.cancelled() {
        run.interrupter().interrupt("cancelled by the user");
    }
    set_progress(&st.active, "running");
    let outcome = run.execute().await.map_err(|e| WsError::new(Code::DependencyUnavailable, format!("模型运行失败：{e}")))?;
    *st.active.interrupter.lock().unwrap() = None;
    let rec = outcome.record();
    let usage = json!({ "main": rec.usage.main, "llm_requests": rec.usage.llm_requests });
    Ok(json!({
        "xllm_run_id": rec.run_id, "status": rec.status.as_str(), "model": rec.config.model,
        "error": rec.last_error.as_ref().map(|e| json!({ "phase": format!("{:?}", e.phase), "message": e.message })),
        "limit_reason": rec.limit_reason, "answer": rec.result.as_ref().map(|r| r.raw.chars().take(2000).collect::<String>()),
        "usage": usage,
    }))
}

fn section(title: &str, body: &str) -> String {
    format!("## {title}\n\n{}\n\n", if body.trim().is_empty() { "（无）" } else { body.trim() })
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

async fn analyze(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, run_dir: &Path) -> WsResult<()> {
    let (user, evidence) = {
        let stage = st.stage.lock().unwrap();
        let p = wish_payload(&stage);
        let map = std::fs::read_to_string(stage.workdir.join("WORKSPACE.md")).unwrap_or_default();
        let mut u = String::from("# 许愿格（分析阶段）\n\n");
        u.push_str(&section("原始需求", p["prompt"].as_str().unwrap_or("")));
        u.push_str(&section("许愿格知识（口径、背景、偏好）", p["knowledge"].as_str().unwrap_or("")));
        u.push_str(&section("修改意见（以后每次都要遵守）", &refinement_lines(&p)));
        u.push_str(&section("固定参数（request）", &serde_json::to_string(&stage.request).unwrap_or_default()));
        if let Some(a) = p.get("analysis") {
            u.push_str(&section(
                "上一次分析（重新分析时默认沿用其结果名、类型、键和视图）",
                &format!("context_prompt：{}\n\noutput_contract：{}\n\nchecks：{}", a["context_prompt"].as_str().unwrap_or(""), a["output_contract"], a["checks"]),
            ));
        }
        u.push_str("---\n\n");
        u.push_str(&map);
        u.push_str("\n\n读完地图后：需要时用 ws_* 工具核实，然后调用 submit_analysis 提交。");
        (u, stage.evidence()?)
    };
    let x = run_xllm(st, run_dir, ANALYZE_SYSTEM.to_string(), user, &st.runtime.config.analyze_model, st.runtime.config.analyze_tool_iterations, &[Kind::Outline, Kind::Find, Kind::Profile, Kind::Neighbors, Kind::Read, Kind::Query, Kind::SubmitAnalysis], false).await?;
    if st.cancelled() {
        return Ok(());
    }
    let accepted = st.analysis.lock().unwrap().take();
    let Some(v) = accepted else {
        return Err(WsError::new(Code::DependencyUnavailable, format!("模型没有提交被接受的分析（运行状态 {}）{}", x["status"].as_str().unwrap_or("?"),
            x["error"]["message"].as_str().map(|m| format!("：{m}")).or_else(|| x["limit_reason"].as_str().map(|m| format!("：{m}"))).unwrap_or_default()))
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
        let mut l = format!("- `{}`：{}「{}」（{}）", i.name, aiworkspace_store::wish::context::type_label(&i.type_id), i.label, meta["handle"].as_str().unwrap_or(""));
        if let Some(r) = meta["rows"].as_u64() {
            l.push_str(&format!("，{r} 行"));
        }
        if let Some(v) = meta.get("view").filter(|v| v.is_object()) {
            l.push_str(&format!("，按视图 {} 读取（{}）", v["handle"].as_str().unwrap_or(""), v["summary"].as_str().unwrap_or("")));
        } else if let Some(s) = &i.selector {
            l.push_str(&format!("，范围 {s}"));
        }
        if i.appended {
            l.push_str("（上次执行时追加的输入）");
        }
        l.push_str(&format!("；文件 context/entities/{}/", i.entity_id));
        out.push(l);
    }
    if out.is_empty() {
        "没有数据输入（纯创作任务）".into()
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
    let Some(a) = p.get("analysis") else { return Err(WsError::invalid_op("这个许愿格还没有分析：先点“分析”")) };
    if aiworkspace_core::wish::needs_analysis(p) {
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_ANALYSIS", "需求、知识、修改意见、输入或执行配置在分析之后改变了：先重新分析"));
    }
    if !aiworkspace_core::freshness::analysis_sources_changed(&stage.ctx(), p)?.is_empty() {
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_ANALYSIS", "任务说明引用的文档（口径、规则）在分析之后改变了：先重新分析"));
    }
    if a["status"] != json!("ready") {
        let blockers: Vec<String> = a["blockers"].as_array().into_iter().flatten().filter_map(|b| b["message"].as_str().map(str::to_string)).collect();
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_INPUT", format!("分析结论是缺少输入：{}", blockers.join("；"))));
    }
    Ok(())
}

async fn execute(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, run_dir: &Path, params: &Value) -> WsResult<()> {
    let repair = params["stage"] == json!("repair_program");
    // the previous round of a feedback / repair chain: its program is the starting point
    let parent = match params.get("parent_run_id").and_then(Value::as_str) {
        Some(pid) => {
            let (c, p) = (caller.clone(), pid.to_string());
            let run = with_ws(state, ws_id, move |ws| ws.wish_run(&c, &p)).await?;
            let src = std::fs::read_to_string(st.ws_dir.join("runs").join(pid).join("work/program/main.js")).ok();
            Some((run, src))
        }
        None => None,
    };
    let user = {
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
        let mut u = String::from(if repair { "# 许愿格（执行阶段：修复程序）\n\n" } else { "# 许愿格（执行阶段）\n\n" });
        u.push_str(&section("原始需求", p["prompt"].as_str().unwrap_or("")));
        u.push_str(&section("许愿格知识（口径、背景、偏好）", p["knowledge"].as_str().unwrap_or("")));
        u.push_str(&section("修改意见（必须遵守）", &refinement_lines(&p)));
        u.push_str(&section("任务说明（context 提示词）", a["context_prompt"].as_str().unwrap_or("")));
        u.push_str(&section("输入（程序中用 aiws.input(输入名) 读取）", &input_lines(&stage)));
        u.push_str(&section("输出约定", &format!("```json\n{}\n```", serde_json::to_string_pretty(&a["output_contract"]).unwrap_or_default())));
        u.push_str(&section("验收检查（program 型由程序 aiws.check 报告；review 型在 finish 中自评）", &format!("```json\n{}\n```", serde_json::to_string_pretty(&a["checks"]).unwrap_or_default())));
        u.push_str(&section("固定参数（aiws.request）", &serde_json::to_string(&stage.request).unwrap_or_default()));
        u.push_str(&section(
            "程序",
            if existing.is_some() { "program/main.js 已有程序（上次应用的或上一轮的）：在它上面做最小修改，保持结果名、字段和键稳定。" } else { "还没有程序：在 program/main.js 中编写。" },
        ));
        if let Some((prun, _)) = &parent {
            let prev = &prun["candidate"];
            if repair {
                let err = prun["detail"]["error"].clone();
                u.push_str(&section("程序运行失败（只重跑程序时）", &format!("{}\n\n{}", err["detail"].as_str().unwrap_or(""), prun["detail"]["program_log"].as_str().unwrap_or(""))));
                u.push_str("请读取输入的最新 schema 与画像，做让程序在新数据上正确运行的最小修改，然后 run_program、finish。direct 结果（文字）保持原样，不需要重新提交。\n\n");
            } else {
                let results: Vec<Value> = prev["results"].as_array().into_iter().flatten().map(|r| json!({ "name": r["name"], "type": r["type"], "approach": r["approach"],
                    "rows": r["table"]["rows"].as_array().map(Vec::len), "fields": r["table"]["fields"], "excerpt": r["markdown"].as_str().map(|m| m.chars().take(300).collect::<String>()) })).collect();
                u.push_str(&section("上一轮候选", &format!("结果：{}\n检查：{}\nfacts：{}\n摘要：{}", json!(results), prev["checks"], prev["facts"], prev["summary"].as_str().unwrap_or(""))));
                // direct results of the previous round are carried over unless the model resubmits them
                u.push_str(&section("用户反馈（本轮要满足）", st.feedback.as_deref().unwrap_or("")));
                u.push_str("在已有程序上修改以满足反馈；不受反馈影响的结果保持不变（direct 结果需要时重新 put_result）。finish 时在 refinements 中写出整理后的要求。\n\n");
            }
        }
        u.push_str("---\n\n");
        u.push_str(&map);
        u
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
    let system = EXECUTE_SYSTEM.replace("{AIWS_API}", AIWS_API);
    let mut kinds = Kind::READ.to_vec();
    kinds.extend(Kind::DELIVERY);
    let x = run_xllm(st, run_dir, system, user, &st.runtime.config.execute_model, st.runtime.config.execute_tool_iterations, &kinds, true).await?;
    if st.cancelled() {
        return Ok(());
    }
    if st.collector.lock().unwrap().finished.is_none() {
        let tail = st.collector.lock().unwrap().program.as_ref().map(|p| p.logs.clone()).unwrap_or_default();
        return Err(WsError::new(Code::DependencyUnavailable, format!("执行没有以 finish 结束（运行状态 {}）{}", x["status"].as_str().unwrap_or("?"),
            x["error"]["message"].as_str().map(|m| format!("：{m}")).or_else(|| x["limit_reason"].as_str().map(|m| format!("：{m}"))).unwrap_or_default()))
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

/// Files the results reference, and the program source, become staged assets (content addressed;
/// re-staged at application).
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
            out.push((k, ws.stage_asset(&c, &bytes)?["object_id"].clone()));
        }
        Ok(out)
    })
    .await?;
    for (k, obj) in staged {
        if k == "program" {
            cand["program"]["object_id"] = obj;
        } else if let Some(i) = k.strip_prefix('r').and_then(|n| n.parse::<usize>().ok()) {
            cand["results"][i]["file"]["object_id"] = obj;
        }
    }
    Ok(())
}

// ---- re-running the stored program (§9.2): no model

async fn rerun(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>) -> WsResult<()> {
    {
        let stage = st.stage.lock().unwrap();
        let p = &stage.wish.payload;
        if p.get("analysis").is_none() {
            return Err(WsError::invalid_op("这个许愿格还没有分析"));
        }
        let src = program_source(&stage).ok_or_else(|| WsError::invalid_op("这个许愿格还没有本服务能运行的程序：先执行一次（生成）"))?;
        write(&stage.workdir.join("program/main.js"), &src)?;
        *st.collector.lock().unwrap() = collector_for(&stage, true)?;
    }
    set_progress(&st.active, "running");
    st.note_tool("run_program");
    let out = runner::run(st).await.map_err(|e| WsError::new(Code::DependencyUnavailable, e))?;
    let log = out.tail();
    let fail = |msg: String| WsError::sub(Code::InvalidOperation, "PROGRAM_FAILED", msg).with_data(json!({ "program_log": log }));
    if out.timed_out {
        return Err(fail(format!("程序运行超时（{} 秒）", st.runtime.config.program_timeout_secs)));
    }
    let raw = out.results.clone().ok_or_else(|| fail(format!("程序没有写出结果（退出码 {:?}）", out.code)))?;
    if let Some(e) = raw.get("error").and_then(Value::as_str) {
        return Err(fail(format!("程序抛出错误：{}", e.lines().next().unwrap_or(""))));
    }
    let source = std::fs::read_to_string(st.workdir.join("program/main.js")).unwrap_or_default();
    let (mut candidate, evidence) = {
        let stage = st.stage.lock().unwrap();
        let mut c = st.collector.lock().unwrap();
        let run = program_output(&c, &raw, &source, &log).map_err(|errs| fail(format!("程序结果没有通过校验：{}", errs.join("；"))))?;
        c.program = Some(run);
        let blocking = c.blocking();
        if !blocking.is_empty() {
            return Err(fail(format!("程序结果不完整：{}", blocking.join("；"))));
        }
        c.finished = Some(json!({ "summary": "只重跑程序（没有调用模型）" }));
        (c.candidate(), stage.evidence()?)
    };
    st.note_program_run();
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
                    bad.push(format!("「{name}」的「{n}」声明为 {t}，值是 {v}"));
                }
            }
        }
        for f in r["table"]["fields"].as_array().into_iter().flatten() {
            let (n, t) = (f["name"].as_str().unwrap_or(""), f["type"].as_str().unwrap_or("text"));
            if let Some(v) = r["table"]["rows"].as_array().into_iter().flatten().filter_map(|row| row.get(n)).find(|v| !check_value(t, v)) {
                bad.push(format!("「{name}」的字段「{n}」声明为 {t}，有值是 {v}"));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(WsError::invalid_schema(format!("模拟结果不合法：{}", bad.join("；"))))
    }
}

async fn mock(state: &Arc<AppState>, caller: &Caller, ws_id: &str, st: &Arc<StageState>, params: &Value) -> WsResult<()> {
    let provided = params["provided"].clone();
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
                "properties": [{ "name": "帧", "type": "text", "key": "frames" }, { "name": "时长(毫秒)", "type": "number", "key": "duration" }, { "name": "说明", "type": "text", "key": "caption" },
                               { "name": "宽", "type": "number", "key": "width" }, { "name": "高", "type": "number", "key": "height" }],
                "props": { "帧": serde_json::to_string(&frames).unwrap_or_default(), "时长(毫秒)": duration, "说明": r["video"]["caption"], "宽": r["video"]["width"], "高": r["video"]["height"] },
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
    fill_candidate(&mut candidate, &evidence, "mock", st, "generate");
    finish_stage(state, caller, ws_id, st, candidate, evidence, json!({})).await
}
