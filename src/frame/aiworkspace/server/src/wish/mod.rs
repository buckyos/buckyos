//! Wish runs on the service (许愿格详细设计 §3, §13): `proc.start` creates a run record and returns
//! at once; a background worker takes the stage snapshot, builds the context, drives xllm (or the
//! program runner) outside the Workspace lock, and leaves a candidate with a previewed plan;
//! `proc.apply` commits exactly the plan the user previewed. Cancelling interrupts the model and
//! kills the program; a run found active without a worker (service restart) is reported
//! interrupted, never restarted.

pub mod exec;
pub mod llm;
pub mod runner;
pub mod stages;
pub mod tools;

use crate::AppState;
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::wish::context::Stage;
use aiworkspace_store::wish::plan::{Plan, PlanRequest};
use aiworkspace_store::wish::results::Collector;
use aiworkspace_store::wish::runs::{is_wish_program, ACTIVE_STATES, PROGRAM_MOCK, PROGRAM_XLLM};
use aiworkspace_store::Caller;
use agent_tool::xllm::{LlmClientFactory, XllmInterrupter};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const AIWS_JS: &str = include_str!("../../../aiws/aiws.js");
pub const RUN_JS: &str = include_str!("../../../aiws/run.js");
pub const AIWS_API: &str = include_str!("../../../aiws/README.md");
pub const RENDERERS: &str = include_str!("../../../aiws/renderers.json");
pub const ANALYZE_SYSTEM: &str = include_str!("../../../aiws/prompts/analyze.md");
pub const EXECUTE_SYSTEM: &str = include_str!("../../../aiws/prompts/execute.md");

/// Deployment configuration of wish runs.
#[derive(Clone)]
pub struct WishConfig {
    /// `{ type: buckyos | openai, base_url?, api_key_env?, headers? }`
    pub provider: Value,
    pub analyze_model: String,
    pub execute_model: String,
    pub map_model: String,
    pub deno: String,
    pub analyze_tool_iterations: u32,
    pub execute_tool_iterations: u32,
    pub max_output_tokens: u32,
    pub stage_timeout_secs: u64,
    pub program_timeout_secs: u64,
    pub program_memory_mb: u32,
    pub llm_map_max_items: usize,
    pub map_budget_chars: usize,
    pub context_window: Option<u32>,
    /// Tests: every model client comes from this factory, which sees the provider of each run.
    pub llm_override: Option<Arc<dyn LlmClientFactory>>,
    /// Execution mode of a run that names none (D14): `service` | `local` | `hosted`.
    pub default_execution: String,
    /// The transitional in-service executor may be chosen (T3-9 switches it off after hosted DV).
    pub allow_service_execution: bool,
}

impl Default for WishConfig {
    fn default() -> Self {
        let s = buckyos_api::AiWorkspaceWishSettings::default();
        WishConfig::from_settings(&s, json!({ "type": "buckyos" }))
    }
}

impl WishConfig {
    pub fn from_settings(s: &buckyos_api::AiWorkspaceWishSettings, provider: Value) -> WishConfig {
        WishConfig {
            provider,
            analyze_model: s.analyze_model.clone(),
            execute_model: s.execute_model.clone(),
            map_model: s.map_model.clone(),
            deno: s.deno.clone(),
            analyze_tool_iterations: s.analyze_tool_iterations,
            execute_tool_iterations: s.execute_tool_iterations,
            max_output_tokens: s.max_output_tokens,
            stage_timeout_secs: s.stage_timeout_secs,
            program_timeout_secs: s.program_timeout_secs,
            program_memory_mb: s.program_memory_mb,
            llm_map_max_items: s.llm_map_max_items as usize,
            map_budget_chars: 16_000,
            context_window: None,
            llm_override: None,
            default_execution: s.default_execution.clone(),
            allow_service_execution: s.allow_service_execution,
        }
    }

    /// Standalone mode: `{ provider, analyze_model, execute_model, map_model, deno, … }` overrides.
    pub fn apply_json(&mut self, v: &Value) {
        if let Some(p) = v.get("provider").filter(|p| p.is_object()) {
            self.provider = p.clone();
        }
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        if let Some(x) = s("analyze_model") {
            self.analyze_model = x;
        }
        if let Some(x) = s("execute_model") {
            self.execute_model = x;
        }
        if let Some(x) = s("map_model") {
            self.map_model = x;
        }
        if let Some(x) = s("deno") {
            self.deno = x;
        }
        if let Some(x) = s("default_execution") {
            self.default_execution = x;
        }
        if let Some(x) = v.get("allow_service_execution").and_then(Value::as_bool) {
            self.allow_service_execution = x;
        }
        if let Some(x) = n("analyze_tool_iterations") {
            self.analyze_tool_iterations = x as u32;
        }
        if let Some(x) = n("execute_tool_iterations") {
            self.execute_tool_iterations = x as u32;
        }
        if let Some(x) = n("max_output_tokens") {
            self.max_output_tokens = x as u32;
        }
        if let Some(x) = n("stage_timeout_secs") {
            self.stage_timeout_secs = x;
        }
        if let Some(x) = n("program_timeout_secs") {
            self.program_timeout_secs = x;
        }
        if let Some(x) = n("llm_map_max_items") {
            self.llm_map_max_items = x as usize;
        }
        if let Some(x) = n("context_window") {
            self.context_window = Some(x as u32);
        }
    }
}

/// One run some worker owns right now.
#[derive(Default)]
pub struct ActiveRun {
    pub cancelled: AtomicBool,
    pub interrupter: Mutex<Option<XllmInterrupter>>,
    pub child: Mutex<Option<u32>>,
    pub progress: Mutex<Value>,
}

pub struct WishRuntime {
    pub config: WishConfig,
    active: Mutex<HashMap<String, Arc<ActiveRun>>>,
    /// callback token → stage (programs' `llm.map`)
    stages: Mutex<HashMap<String, Arc<StageState>>>,
    /// run id → the stage an executor outside the service works on (T3-4), from its claim to its report
    hosted: Mutex<HashMap<String, Arc<StageState>>>,
    /// `http://127.0.0.1:<port>/kapi/aiworkspace` once the service listens.
    pub host_base: Mutex<Option<String>>,
}

impl WishRuntime {
    pub fn new(config: WishConfig) -> Arc<WishRuntime> {
        Arc::new(WishRuntime { config, active: Mutex::default(), stages: Mutex::default(), hosted: Mutex::default(), host_base: Mutex::default() })
    }

    pub fn active(&self, run_id: &str) -> Option<Arc<ActiveRun>> {
        self.active.lock().unwrap().get(run_id).cloned()
    }

    pub(super) fn register(&self, run_id: &str) -> Arc<ActiveRun> {
        let a = Arc::new(ActiveRun::default());
        *a.progress.lock().unwrap() = json!({ "phase": "queued" });
        self.active.lock().unwrap().insert(run_id.to_string(), a.clone());
        a
    }

    pub(super) fn unregister(&self, run_id: &str) {
        self.active.lock().unwrap().remove(run_id);
    }

    pub fn hosted(&self, run_id: &str) -> Option<Arc<StageState>> {
        self.hosted.lock().unwrap().get(run_id).cloned()
    }

    pub(super) fn set_hosted(&self, run_id: &str, st: Arc<StageState>) {
        self.hosted.lock().unwrap().insert(run_id.to_string(), st);
    }

    /// Close the stage an outside executor worked on (its tools answer "cancelled" from now on).
    pub fn drop_hosted(&self, run_id: &str) {
        if let Some(st) = self.hosted.lock().unwrap().remove(run_id) {
            st.active.cancelled.store(true, Ordering::SeqCst);
            self.stages.lock().unwrap().remove(&st.token);
        }
    }

    pub fn stage_by_token(&self, token: &str) -> Option<Arc<StageState>> {
        self.stages.lock().unwrap().get(token).cloned()
    }
}

/// Everything a stage's tools share.
pub struct StageState {
    pub runtime: Arc<WishRuntime>,
    pub active: Arc<ActiveRun>,
    pub run_id: String,
    pub workspace_id: String,
    pub ws_dir: PathBuf,
    pub workdir: PathBuf,
    pub stage: Mutex<Stage>,
    pub collector: Mutex<Collector>,
    pub analysis: Mutex<Option<aiworkspace_store::wish::analysis::Validated>>,
    pub feedback: Option<String>,
    pub token: String,
    /// Whose AICC the run uses (fixed at acceptance; `None` without a BuckyOS AICC).
    pub aicc: Option<buckyos_api::AiccOnBehalfOf>,
    prompts: Vec<String>,
    tool_calls: AtomicU32,
    program_runs: AtomicU32,
    map_items: AtomicUsize,
    map_calls: AtomicUsize,
    map_cached: AtomicUsize,
}

impl StageState {
    pub fn cancelled(&self) -> bool {
        self.active.cancelled.load(Ordering::SeqCst)
    }

    pub fn note_tool(&self, name: &str) {
        let n = self.tool_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mut p = self.active.progress.lock().unwrap();
        p["tool_calls"] = json!(n);
        p["activity"] = json!(match name {
            "ws_outline" | "ws_find" | "ws_profile" | "ws_neighbors" | "ws_read" | "ws_query" => "reading",
            "run_program" => "running_program",
            "put_result" => "writing",
            "check_results" | "finish" => "checking",
            "submit_analysis" => "submitting",
            _ => "working",
        });
        p["last_tool"] = json!(name);
    }

    pub fn note_program_run(&self) {
        let n = self.program_runs.fetch_add(1, Ordering::SeqCst) + 1;
        self.active.progress.lock().unwrap()["program_runs"] = json!(n);
    }

    /// Count items asked of `llm.map` in this run; returns the total so far.
    pub fn map_items(&self, n: usize) -> usize {
        self.map_items.fetch_add(n, Ordering::SeqCst) + n
    }

    pub fn note_map(&self, calls: usize, cached: usize) {
        self.map_calls.fetch_add(calls, Ordering::SeqCst);
        self.map_cached.fetch_add(cached, Ordering::SeqCst);
        let mut p = self.active.progress.lock().unwrap();
        p["llm_map"] = json!({ "calls": self.map_calls.load(Ordering::SeqCst), "cached": self.map_cached.load(Ordering::SeqCst), "items": self.map_items.load(Ordering::SeqCst) });
    }

    pub fn set_child(&self, pid: Option<u32>) {
        *self.active.child.lock().unwrap() = pid;
    }

    /// Texts whose numbers a written result may repeat without a fact (the request itself).
    pub fn prompt_texts(&self) -> Vec<String> {
        self.prompts.clone()
    }

    /// Environment of the program: the callback for `llm.map`.
    pub fn program_env(&self) -> Vec<(String, String)> {
        match self.runtime.host_base.lock().unwrap().clone() {
            Some(base) => vec![("AIWS_HOST".into(), format!("{base}/wish-host/{}", self.token)), ("AIWS_TOKEN".into(), self.token.clone())],
            None => Vec::new(),
        }
    }
}

fn progress_line(state: &str) -> Value {
    json!({ "phase": state })
}

/// The run as clients see it (`proc.get`, `proc.list`).
pub fn run_view(rt: &WishRuntime, run: &Value, full: bool) -> Value {
    let mut v = json!({
        "ok": true, "run_id": run["run_id"], "program": run["program"], "state": run["state"],
        "stage": run["params"]["stage"], "wish_id": run["params"]["wish_id"], "params": run["params"],
        "simulated": run["program"] == json!(PROGRAM_MOCK),
        "created_at": run["created_at"], "updated_at": run["updated_at"],
        "warnings": run["warnings"], "error": run["detail"]["error"], "usage": run["detail"]["usage"],
        "aicc": run["detail"]["aicc"],
        "context": run["detail"]["context"], "dispatch": run["detail"]["dispatch"],
        "execution": { "mode": run["detail"]["exec"]["mode"], "attempt": run["detail"]["exec"]["attempt"], "lease_until": run["detail"]["exec"]["lease_until"], "executor": run["detail"]["exec"]["executor"] },
        "brief": run["detail"]["brief"],
        "parent_run_id": run["params"]["parent_run_id"], "feedback": run["params"]["feedback"],
    });
    let mut progress = run["detail"]["progress"].clone();
    if let Some(a) = rt.active(run["run_id"].as_str().unwrap_or("")) {
        progress = a.progress.lock().unwrap().clone();
    }
    v["progress"] = progress;
    if let Some(app) = run["detail"].get("applied") {
        v["applied"] = app.clone();
        v["result"] = app["commit"].clone();
    }
    if full {
        v["candidate"] = run["candidate"].clone();
        v["evidence"] = json!({ "read_set": run["evidence"]["read_set"], "appended_inputs": run["evidence"]["appended_inputs"], "inputs": run["evidence"]["inputs"] });
        if let Some(d) = run["detail"].get("plan_order").and_then(Value::as_array).and_then(|o| o.last()).and_then(Value::as_str) {
            let plan = &run["detail"]["plans"][d];
            v["preview"] = json!({ "plan_digest": d, "ready": plan["ready"], "summary": plan["summary"] });
        }
        v["xllm"] = run["detail"]["xllm"].clone();
        v["program_log"] = run["detail"]["program_log"].clone();
    }
    v
}

/// Whose AICC a new run uses (AI Center Quota PRD §3.3), fixed when the run is accepted:
/// the workspace owner's by default, or the starting user's. Nobody falls back to the other.
pub fn run_attribution(ws: &aiworkspace_store::workspace::Workspace, caller: &Caller, force_current_user: bool) -> WsResult<Value> {
    use aiworkspace_store::workspace::{CallerClass, WISH_AICC_CURRENT_USER, WISH_AICC_WORKSPACE_OWNER};
    // a local run's model calls come from the starter's own browser session (D14): nobody else can pay for them
    let mode = if force_current_user { WISH_AICC_CURRENT_USER.to_string() } else { ws.wish_aicc_mode()? };
    let user_id = if mode == WISH_AICC_CURRENT_USER {
        if !matches!(caller.class, CallerClass::Account { .. }) {
            return Err(WsError::sub(Code::PermissionDenied, "AI_ATTRIBUTION_UNAVAILABLE", "You have no AI resources in this Zone: this workspace runs wish blocks with the AI resources of the user who starts them")
                .with_data(json!({ "aicc": { "mode": WISH_AICC_CURRENT_USER } })));
        }
        caller.principal.clone()
    } else {
        ws.owner()?.ok_or_else(|| WsError::sub(Code::DependencyUnavailable, "AI_ATTRIBUTION_UNAVAILABLE", "The workspace has no owner whose AI resources the wish block could use"))?
    };
    let mode = if mode == WISH_AICC_CURRENT_USER { WISH_AICC_CURRENT_USER } else { WISH_AICC_WORKSPACE_OWNER };
    Ok(json!({ "mode": mode, "user_id": user_id, "operator_id": caller.principal }))
}

/// `on_behalf_of` of the AICC calls of a run, from its stored attribution.
pub fn on_behalf_of(run: &Value, ws_id: &str) -> Option<buckyos_api::AiccOnBehalfOf> {
    let aicc = &run["detail"]["aicc"];
    let mode = serde_json::from_value::<buckyos_api::AiccAttributionMode>(aicc["mode"].clone()).ok()?;
    Some(buckyos_api::AiccOnBehalfOf {
        user_id: aicc["user_id"].as_str()?.to_string(),
        operator_id: aicc["operator_id"].as_str().map(str::to_string),
        mode,
        run_ref: Some(buckyos_api::AiccRunRef {
            workspace_id: Some(ws_id.to_string()),
            entity_id: run["params"]["wish_id"].as_str().map(str::to_string),
            run_id: run["run_id"].as_str().map(str::to_string),
        }),
    })
}

/// Sub-code of a wish run refused under the AI quota or resource policy (§7.4).
pub fn refusal_sub_code(kind: &str) -> Option<&'static str> {
    match kind {
        "quota_exhausted" => Some("AI_QUOTA_EXHAUSTED"),
        "quota_budget_exceeded" => Some("AI_QUOTA_BUDGET_EXCEEDED"),
        "resource_not_permitted" => Some("AI_RESOURCE_NOT_PERMITTED"),
        "attribution_unavailable" => Some("AI_ATTRIBUTION_UNAVAILABLE"),
        _ => None,
    }
}

/// The refusal as the run's operator may see it: never the quota rules of another user.
pub fn refusal_error(sub: &'static str, aicc: &Value) -> WsError {
    let own = aicc["user_id"] == aicc["operator_id"];
    let detail = match (sub, own) {
        ("AI_RESOURCE_NOT_PERMITTED", true) => "The model this wish block uses is not permitted to you; ask an administrator or change the wish configuration",
        ("AI_RESOURCE_NOT_PERMITTED", false) => "The model this wish block uses is not available with the AI resources of the workspace owner",
        ("AI_ATTRIBUTION_UNAVAILABLE", _) => "The user whose AI resources this run uses cannot be determined",
        (_, true) => "Your AI token quota does not cover this run; see AI Center › Quota",
        (_, false) => "This wish block is temporarily limited by the AI quota of the workspace owner",
    };
    WsError::sub(Code::DependencyUnavailable, sub, detail).with_data(json!({ "aicc": aicc }))
}

fn s<'a>(p: &'a Value, k: &str) -> WsResult<&'a str> {
    p.get(k).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{k} required")))
}

/// `proc.start` for `wish.xllm@1` / `wish.mock@1`.
pub fn start(state: &Arc<AppState>, caller: &Caller, ws_id: &str, program: &str, params: &Value, idem: &str) -> WsResult<Value> {
    let wish_id = s(params, "wish_id")?.to_string();
    let stage = s(params, "stage")?.to_string();
    let allowed: &[&str] = if program == PROGRAM_XLLM { &["analyze", "execute", "rerun_program", "repair_program"] } else { &["execute"] };
    if !allowed.contains(&stage.as_str()) {
        return Err(WsError::invalid_op(format!("{program} has no stage {stage}")));
    }
    if program == PROGRAM_MOCK && !params.get("provided").is_some_and(Value::is_object) {
        return Err(WsError::invalid_op("wish.mock@1 needs params.provided (the Mock executor's results)"));
    }
    let run_id = state.with_ws(ws_id, |ws| {
        // the wish must exist and be writable by the caller: a run ends in writing it
        let read = ws.read(caller, &wish_id, None)?;
        if read["type_id"] != json!("buckyos.wish") {
            return Err(WsError::invalid_op(format!("{wish_id} is not a wish")));
        }
        // the execution context (§9.6): named in full, or the run does not start
        let mode_name = params["execution"]["mode"].as_str().map(str::to_string).unwrap_or_else(|| state.wish.config.default_execution.clone());
        let mode = exec::Mode::parse(&mode_name).ok_or_else(|| WsError::invalid_op(format!("unknown execution mode {mode_name}")))?;
        // hosted execution is what a collaborator with only `run` on the wish gets (D22); other modes write as the caller
        let needed = if mode == exec::Mode::Hosted { "run" } else { "update" };
        if !read["capabilities"].as_array().is_some_and(|c| c.iter().any(|x| x == needed)) {
            return Err(WsError::denied(format!("running a wish {} needs the {needed} capability on it", mode.as_str())));
        }
        let executor = read["content"]["payload"]["executor"].as_str().unwrap_or("");
        if (program == PROGRAM_XLLM && executor != "xllm") || (program == PROGRAM_MOCK && executor != "mock") {
            return Err(WsError::invalid_op(format!("the wish's executor is {executor}, not the one of {program}")));
        }
        if let Some(parent) = params.get("parent_run_id").and_then(Value::as_str) {
            let p = ws.wish_run(caller, parent)?;
            if p["params"]["wish_id"] != json!(wish_id) {
                return Err(WsError::invalid_op("parent_run_id belongs to another wish"));
            }
        }
        let aicc = run_attribution(ws, caller, mode == exec::Mode::Local)?;
        if mode == exec::Mode::Service && (program == PROGRAM_XLLM && !state.wish.config.allow_service_execution) {
            return Err(WsError::new(Code::ExecutionContextMissing, "the in-service executor is switched off; run locally or hosted"));
        }
        let hosted = if mode == exec::Mode::Hosted { Some(ws.hosted_exec_admits(caller, &wish_id)?) } else { None };
        let context = exec::execution_context(ws, caller, &wish_id, &read["content"]["payload"], mode, &aicc, hosted.as_ref())?;
        let (exec_record, claim) = exec::new_exec_record(mode);
        let payload = &read["content"]["payload"];
        // what an executor outside the service starts from (inputs are read through the run token)
        let brief = json!({ "prompt": payload["prompt"], "knowledge": payload["knowledge"], "refinements": payload["refinements"], "analysis": payload["analysis"],
                            "inputs": payload["inputs"], "output": payload["output"], "request": params.get("request").cloned().unwrap_or(Value::Null), "stage": stage });
        let (run_id, replayed) = ws.wish_run_create_with(caller, program, params, idem, &aicc, &json!({ "context": context, "exec": exec_record, "brief": brief }))?;
        Ok((run_id, replayed, mode, claim, hosted, context))
    })?;
    let (run_id, replayed, mode, claim, hosted, context) = (run_id.0, run_id.1, run_id.2, run_id.3, run_id.4, run_id.5);
    if !replayed && (mode == exec::Mode::Service || program == PROGRAM_MOCK) {
        let active = state.wish.register(&run_id);
        let (st, c, ws, rid) = (state.clone(), caller.clone(), ws_id.to_string(), run_id.clone());
        tokio::runtime::Handle::current().spawn(async move {
            stages::drive(st, c, ws, rid, active).await;
        });
    }
    if !replayed && mode == exec::Mode::Hosted {
        let (st, w, rid, token, cfg, ctx) = (state.clone(), ws_id.to_string(), run_id.clone(), claim.clone().unwrap_or_default(), hosted.clone().unwrap_or(Value::Null), context.clone());
        tokio::runtime::Handle::current().spawn(async move {
            let mut dispatch = exec::dispatch_hosted(&st, &w, &rid, &token, &cfg, &ctx).await;
            if dispatch["status"] == json!("unavailable") {
                // nobody got the token: whoever may read the run (its trigger, an editor) hands it to an executor
                dispatch["claim_token"] = json!(token);
            }
            let _ = st.with_ws(&w, |ws| ws.wish_run_set(&rid, None, None, None, None, Some(&json!({ "dispatch": dispatch }))));
        });
    }
    let mut v = get(state, caller, ws_id, &run_id, &Value::Null)?;
    if !replayed {
        if let Some(t) = claim.filter(|_| mode == exec::Mode::Local) {
            // the browser that started a local run is its executor: the claim token goes to it, once
            v["claim_token"] = json!(t);
        }
    }
    Ok(v)
}

/// `proc.get`: state, progress, candidate and — when waiting for confirmation — the preview of the
/// plan for the given `choices` (kept for `proc.apply`).
pub fn get(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str, choices: &Value) -> WsResult<Value> {
    let rt = state.wish.clone();
    state.with_ws(ws_id, |ws| {
        let mut run = ws.wish_run(caller, run_id)?;
        if exec::expire_if_due(ws, &run)? {
            rt.drop_hosted(run_id);
            rt.unregister(run_id);
            run = ws.wish_run(caller, run_id)?;
        }
        let st = run["state"].as_str().unwrap_or("").to_string();
        // a run an outside executor owns waits for its claim (queued) or its lease (running), not for a service worker
        let executor_owned = run["detail"]["exec"]["mode"].as_str().is_some_and(|m| m != "service") && matches!(st.as_str(), "queued" | "running");
        if ACTIVE_STATES.contains(&st.as_str()) && st != "applying" && !executor_owned && rt.active(run_id).is_none() {
            // nobody owns it any more (service restart): interrupted, never silently re-run (§13.1)
            ws.wish_run_set(run_id, Some("interrupted"), None, None, None, Some(&json!({ "error": { "code": "INTERRUPTED", "detail": "The service restarted or was interrupted during the run; the run does not restart automatically" } })))?;
            run = ws.wish_run(caller, run_id)?;
        }
        if st == "applying" && rt.active(run_id).is_none() {
            ws.wish_reconcile_apply(run_id)?;
            run = ws.wish_run(caller, run_id)?;
        }
        if !choices.is_null() && matches!(run["state"].as_str(), Some("waiting_confirmation" | "conflict" | "rejected")) {
            let plan = plan_for(ws, caller, &run, choices)?;
            ws.wish_run_keep_plan(run_id, &plan)?;
            run = ws.wish_run(caller, run_id)?;
        }
        Ok(run_view(&rt, &run, true))
    })
}

/// The plan of a waiting run (analysis write-back or results).
pub fn plan_for(ws: &mut aiworkspace_store::workspace::Workspace, caller: &Caller, run: &Value, choices: &Value) -> WsResult<Plan> {
    let cand = &run["candidate"];
    if run["params"]["stage"] == json!("analyze") {
        return ws.wish_analysis_plan(caller, cand);
    }
    let mut plan = ws.wish_plan(caller, &PlanRequest { run_id: run["run_id"].as_str().unwrap_or(""), candidate: cand, choices, location: &run["params"]["location"] })?;
    if plan.ready {
        // a dry run of the very commit: validation the planner cannot see (values, permissions, locks)
        let req = json!({ "protocol_version": aiworkspace_core::model::PROTOCOL_VERSION, "workspace_id": ws.workspace_id, "epoch": ws.epoch,
                          "idempotency_key": format!("precheck/{}", run["run_id"].as_str().unwrap_or("")), "origin": "program", "run_id": run["run_id"],
                          "preconditions": plan.preconditions, "operations": plan.operations });
        let pre = ws.prepare(&req, caller);
        if pre["status"] != json!("ok") {
            plan.ready = false;
            let detail = pre["errors"][0]["detail"].as_str().or(pre["code"].as_str()).unwrap_or("").to_string();
            plan.summary["problems"].as_array_mut().map(|a| a.push(json!(format!("Pre-check failed ({}): {detail}", pre["code"].as_str().unwrap_or("")))));
            plan.summary["precheck"] = pre;
        }
    }
    Ok(plan)
}

/// `proc.apply`: re-stage the plan's assets (content-addressed: the same ids), then commit it.
pub fn apply(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let run_id = s(p, "run_id")?;
    state.with_ws(ws_id, |ws| {
        let run = ws.wish_run(caller, run_id)?;
        // the delegation still holds (D16, §9.5): the task configuration the run bound, the attempt that
        // produced the candidate, and the run itself
        if run["state"] == json!("cancelled") {
            return Err(WsError::new(Code::RunRevoked, "the run was cancelled"));
        }
        if let Some(bound) = run["detail"]["context"]["task"]["digest"].as_str() {
            let wish_id = run["params"]["wish_id"].as_str().unwrap_or("");
            let payload = ws.read(caller, wish_id, None)?["content"]["payload"].as_object().cloned().unwrap_or_default();
            if aiworkspace_core::wish::basis_digest(&payload)[..32] != *bound {
                return Err(WsError::new(Code::DelegationStale, "the wish configuration changed after the run was started; run it again under the current configuration").with_data(json!({ "bound": bound })));
            }
        }
        if let (Some(produced), Some(current)) = (run["detail"]["exec"]["provided"]["attempt"].as_u64(), run["detail"]["exec"]["attempt"].as_u64()) {
            if produced != current {
                return Err(WsError::new(Code::RunRevoked, format!("the candidate came from attempt {produced}; attempt {current} superseded it")));
            }
        }
        let digest = match p.get("plan_digest").and_then(Value::as_str) {
            Some(d) => d.to_string(),
            None => run["detail"]["plan_order"].as_array().and_then(|o| o.last()).and_then(Value::as_str).map(str::to_string).ok_or_else(|| WsError::invalid_op("plan_digest required: preview first"))?,
        };
        if let Some(plan) = run["detail"]["plans"].get(&digest) {
            // the result files were staged when the candidate was collected: bound again for this Commit
            for a in plan["assets"].as_array().into_iter().flatten() {
                ws.stage_object(caller, a["object_id"].as_str().unwrap_or(""), state.limits.max_asset_bytes)?;
            }
        }
        let applied = ws.wish_apply(caller, run_id, &digest, p.get("session_id").and_then(Value::as_str))?;
        let run = ws.wish_run(caller, run_id)?;
        let mut v = run_view(&state.wish, &run, true);
        v["applied"] = applied;
        Ok(v)
    })
}

/// `proc.cancel`: the model is interrupted and the program killed; an applied run says so.
pub fn cancel(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str) -> WsResult<Value> {
    let rt = state.wish.clone();
    state.with_ws(ws_id, |ws| {
        let run = ws.wish_run(caller, run_id)?;
        match run["state"].as_str() {
            Some("succeeded") => return Ok(json!({ "ok": true, "state": "succeeded", "already_applied": true, "commit_id": run["detail"]["applied"]["commit"]["commit_id"] })),
            Some("cancelled" | "failed" | "interrupted") => return Ok(json!({ "ok": true, "state": run["state"], "already_applied": false })),
            _ => {}
        }
        if let Some(a) = rt.active(run_id) {
            a.cancelled.store(true, Ordering::SeqCst);
            if let Some(i) = a.interrupter.lock().unwrap().as_ref() {
                i.interrupt("cancelled by the user");
            }
            if let Some(pid) = *a.child.lock().unwrap() {
                kill(pid);
            }
        }
        rt.drop_hosted(run_id);
        ws.wish_run_set(run_id, Some("cancelled"), None, None, None, Some(&json!({ "progress": progress_line("cancelled") })))?;
        Ok(json!({ "ok": true, "state": "cancelled", "already_applied": false }))
    })
}

#[cfg(unix)]
fn kill(pid: u32) {
    let _ = std::process::Command::new("kill").arg("-9").arg(pid.to_string()).status();
}
#[cfg(not(unix))]
fn kill(pid: u32) {
    let _ = buckyos_api::hide_child_console(&mut std::process::Command::new("taskkill")).args(["/F", "/PID", &pid.to_string()]).status();
}

/// `proc.list`: the caller's runs of a wish, newest first.
pub fn list(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let rt = state.wish.clone();
    state.with_ws(ws_id, |ws| {
        let runs = ws.wish_runs_list(caller, p.get("wish_id").and_then(Value::as_str), p.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize)?;
        Ok(json!({ "ok": true, "runs": runs.iter().map(|r| run_view(&rt, r, false)).collect::<Vec<_>>() }))
    })
}

pub fn is_wish(program: &str) -> bool {
    is_wish_program(program)
}

pub fn builtin_catalog() -> Value {
    serde_json::from_str(RENDERERS).unwrap_or(json!([]))
}
