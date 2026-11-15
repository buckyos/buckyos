//! Wish runs (许愿格详细设计 §3, §13; AI Native 第一期 §2; 许愿格交互升级 §3.3): `proc.start` creates a run record and
//! returns at once. Every stage of `wish.xllm@1` — analyze, code, run — executes on an executor outside the service: the
//! browser that started it (local) or the Owner's Agent (hosted). The service keeps the stage snapshot, answers the
//! stage tools (`ws_*`, the macro's `sql`, the `llm_cache`), validates what the executor hands in, and when the stage
//! ends plans what it produced and writes it in one Commit with the run's record in the wish's `history` — nothing
//! waits for a confirmation; a record is rolled back instead (`proc.rollback`). It calls no model and runs no program.

pub mod exec;
pub mod stages;
pub mod tools;

use crate::AppState;
use aiworkspace_core::model::ReadCtx;
use aiworkspace_core::access::{Access, Cap};
use aiworkspace_core::wish::{STAGES, STAGE_ANALYZE, STAGE_CODE, STAGE_RUN};
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::wish::context::Stage;
use aiworkspace_store::wish::plan::{Plan, PlanRequest};
use aiworkspace_store::wish::results::Collector;
use aiworkspace_store::wish::runs::{is_wish_program, ACTIVE_STATES, PROGRAM_XLLM};
use aiworkspace_store::Caller;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub const AIWS_JS: &str = include_str!("../../../aiws/aiws.js");
pub const RUN_JS: &str = include_str!("../../../aiws/run.js");
pub const HOST_JS: &str = include_str!("../../../aiws/host.js");
pub const GUIDE: &str = include_str!("../../../aiws/README.md");
pub const RENDERERS: &str = include_str!("../../../aiws/renderers.json");
pub const CONTROLS: &str = include_str!("../../../aiws/controls.json");
pub const ANALYZE_SYSTEM: &str = include_str!("../../../aiws/prompts/analyze.md");
pub const CODE_SYSTEM: &str = include_str!("../../../aiws/prompts/code.md");

/// Deployment configuration of wish runs: the models and limits every stage hands its executor (`budget`).
#[derive(Clone)]
pub struct WishConfig {
    pub analyze_model: String,
    pub execute_model: String,
    pub map_model: String,
    pub complete_model: String,
    pub image_model: String,
    pub video_model: String,
    pub analyze_tool_iterations: u32,
    pub execute_tool_iterations: u32,
    pub max_output_tokens: u32,
    pub program_timeout_secs: u64,
    pub llm_map_max_items: usize,
    pub ai_max_calls: u32,
    pub ai_max_tokens: u32,
    pub sql_max_calls: u32,
    pub image_max_calls: u32,
    pub video_max_calls: u32,
    pub media_timeout_secs: u64,
    pub snapshot_download_max_bytes: u64,
    pub map_budget_chars: usize,
    /// Execution mode of a run that names none (D14): `local` | `hosted`.
    pub default_execution: String,
}

impl Default for WishConfig {
    fn default() -> Self {
        WishConfig::from_settings(&buckyos_api::AiWorkspaceWishSettings::default())
    }
}

impl WishConfig {
    pub fn from_settings(s: &buckyos_api::AiWorkspaceWishSettings) -> WishConfig {
        let mut c = WishConfig {
            analyze_model: s.analyze_model.clone(),
            execute_model: s.execute_model.clone(),
            map_model: s.map_model.clone(),
            complete_model: s.complete_model.clone(),
            image_model: s.image_model.clone(),
            video_model: s.video_model.clone(),
            analyze_tool_iterations: s.analyze_tool_iterations,
            execute_tool_iterations: s.execute_tool_iterations,
            max_output_tokens: s.max_output_tokens,
            program_timeout_secs: s.program_timeout_secs,
            llm_map_max_items: s.llm_map_max_items as usize,
            ai_max_calls: s.ai_max_calls,
            ai_max_tokens: s.ai_max_tokens,
            sql_max_calls: s.sql_max_calls,
            image_max_calls: s.image_max_calls,
            video_max_calls: s.video_max_calls,
            media_timeout_secs: s.media_timeout_secs,
            snapshot_download_max_bytes: s.snapshot_download_max_bytes,
            map_budget_chars: 16_000,
            default_execution: String::new(),
        };
        c.set_default_execution(&s.default_execution);
        c
    }

    /// `local` or `hosted`; anything else is configured wrong and falls back to `local`.
    fn set_default_execution(&mut self, v: &str) {
        self.default_execution = match exec::Mode::parse(v) {
            Some(m) => m.as_str().to_string(),
            None => {
                log::warn!("wish.default_execution {v:?} is neither local nor hosted: using local");
                "local".into()
            }
        };
    }

    /// Standalone mode: `{ analyze_model, execute_model, map_model, complete_model, … }` overrides.
    pub fn apply_json(&mut self, v: &Value) {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        for (k, slot) in [("analyze_model", &mut self.analyze_model), ("execute_model", &mut self.execute_model), ("map_model", &mut self.map_model), ("complete_model", &mut self.complete_model),
                          ("image_model", &mut self.image_model), ("video_model", &mut self.video_model)] {
            if let Some(x) = s(k) {
                *slot = x;
            }
        }
        if let Some(x) = s("default_execution") {
            self.set_default_execution(&x);
        }
        for (k, slot) in [("analyze_tool_iterations", &mut self.analyze_tool_iterations), ("execute_tool_iterations", &mut self.execute_tool_iterations), ("max_output_tokens", &mut self.max_output_tokens),
                          ("ai_max_calls", &mut self.ai_max_calls), ("ai_max_tokens", &mut self.ai_max_tokens), ("sql_max_calls", &mut self.sql_max_calls),
                          ("image_max_calls", &mut self.image_max_calls), ("video_max_calls", &mut self.video_max_calls)] {
            if let Some(x) = n(k) {
                *slot = x as u32;
            }
        }
        if let Some(x) = n("program_timeout_secs") {
            self.program_timeout_secs = x;
        }
        if let Some(x) = n("media_timeout_secs") {
            self.media_timeout_secs = x;
        }
        if let Some(x) = n("llm_map_max_items") {
            self.llm_map_max_items = x as usize;
        }
        if let Some(x) = n("snapshot_download_max_bytes") {
            self.snapshot_download_max_bytes = x;
        }
    }
}

/// One run some worker owns right now.
#[derive(Default)]
pub struct ActiveRun {
    pub cancelled: AtomicBool,
    pub progress: Mutex<Value>,
}

pub struct WishRuntime {
    pub config: WishConfig,
    active: Mutex<HashMap<String, Arc<ActiveRun>>>,
    /// run id → the stage an executor works on, from its claim to its report
    hosted: Mutex<HashMap<String, Arc<StageState>>>,
}

impl WishRuntime {
    pub fn new(config: WishConfig) -> Arc<WishRuntime> {
        Arc::new(WishRuntime { config, active: Mutex::default(), hosted: Mutex::default() })
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

    /// Close the stage an executor worked on (its tools answer "cancelled" from now on).
    pub fn drop_hosted(&self, run_id: &str) {
        if let Some(st) = self.hosted.lock().unwrap().remove(run_id) {
            st.active.cancelled.store(true, Ordering::SeqCst);
        }
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
    /// Running a stored macro: its source and manifest.
    pub stored: Option<(String, Value)>,
    /// What ends a run of the stored macro that failed (`PROGRAM_FAILED`, `MACRO_CODE_CHANGED`).
    pub failure: Mutex<Option<WsError>>,
    pub budget: Value,
    /// The log of the last macro run the executor handed in (a record keeps it).
    pub last_log: Mutex<String>,
    /// Whose AI the run uses (its `aicc.user_id`): the AI answer cache is kept per that user, so one person's executor
    /// never answers another's runs from what it wrote (权限需求 R35).
    pub ai_user: String,
    /// The owner's limit on the model's tool rounds in a hosted run (`limits.max_tool_iterations`, 权限需求 R30); a
    /// macro's own bridge calls (`sql`, `llm_cache`) are not rounds.
    pub max_tool_rounds: Option<u64>,
    prompts: Vec<String>,
    tool_calls: AtomicU32,
    program_runs: AtomicU32,
    sql_calls: AtomicU32,
}

impl StageState {
    pub fn cancelled(&self) -> bool {
        self.active.cancelled.load(Ordering::SeqCst)
    }

    /// The model's tool rounds so far.
    pub fn tool_rounds(&self) -> u64 {
        self.tool_calls.load(Ordering::SeqCst) as u64
    }

    pub fn note_tool(&self, name: &str) {
        let n = self.tool_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mut p = self.active.progress.lock().unwrap();
        p["tool_calls"] = json!(n);
        p["activity"] = json!(match name {
            "ws_outline" | "ws_find" | "ws_profile" | "ws_neighbors" | "ws_read" | "ws_query" => "reading",
            "run_program" => "running_program",
            "check_results" | "finish" => "checking",
            "submit_analysis" => "submitting",
            _ => "working",
        });
        p["last_tool"] = json!(name);
    }

    pub fn note_program_run(&self, ai: &Value) {
        let n = self.program_runs.fetch_add(1, Ordering::SeqCst) + 1;
        let mut p = self.active.progress.lock().unwrap();
        p["program_runs"] = json!(n);
        if !ai.is_null() {
            p["ai"] = ai.clone();
        }
    }

    /// Count one `aiws.sql` call; `false` once the run's budget is spent.
    pub fn take_sql_call(&self) -> bool {
        let max = self.budget["sql_max_calls"].as_u64().unwrap_or(0) as u32;
        self.sql_calls.fetch_add(1, Ordering::SeqCst) < max
    }

    /// Texts whose numbers a written result may repeat without a fact (the request itself).
    pub fn prompt_texts(&self) -> Vec<String> {
        self.prompts.clone()
    }
}

/// The readiness of a stage (第一期 §2.1): coding needs a current analysis; running needs that and a stored macro that
/// still fits its inputs (`NEEDS_CODING` with the reasons otherwise).
pub fn ready(ctx: &dyn ReadCtx, payload: &aiworkspace_core::model::JsonMap, stage: &str) -> WsResult<()> {
    if stage == STAGE_ANALYZE {
        return Ok(());
    }
    if payload.get("analysis").is_none() {
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_ANALYSIS", "This wish has not been analyzed yet: analyze it first"));
    }
    if aiworkspace_core::wish::needs_analysis(payload) {
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_ANALYSIS", "The request, knowledge, refinements, inputs or execution configuration changed after the analysis: analyze again first"));
    }
    if !aiworkspace_core::freshness::analysis_sources_changed(ctx, payload)?.is_empty() {
        return Err(WsError::sub(Code::InvalidOperation, "NEEDS_ANALYSIS", "Documents the task description relies on (definitions, rules) changed after the analysis: analyze again first"));
    }
    if stage == STAGE_RUN {
        let reasons = aiworkspace_core::wish::macro_check(ctx, payload)?;
        if !reasons.is_empty() {
            let what: Vec<String> = reasons.iter().map(|r| match r["code"].as_str().unwrap_or("") {
                "no_macro" => "there is no macro yet".to_string(),
                "api_version" => "the macro was written for another program API".to_string(),
                "field_deleted" => format!("field {} of input {} was deleted", r["name"], r["input"].as_str().unwrap_or("")),
                "field_type_changed" => format!("field {} of input {} changed its type from {} to {}", r["name"], r["input"].as_str().unwrap_or(""), r["from"], r["to"]),
                c => format!("input {} ({c})", r["input"].as_str().unwrap_or("")),
            }).collect();
            return Err(WsError::sub(Code::InvalidOperation, "NEEDS_CODING", format!("The macro cannot run on the inputs as they are now: {}; code the wish again", what.join("; "))).with_data(json!({ "reasons": reasons })));
        }
    }
    Ok(())
}

fn progress_line(state: &str) -> Value {
    json!({ "phase": state })
}

/// The run as clients see it (`proc.get`, `proc.list`): `record` is the record the run wrote into its wish's
/// `history` (null until it ended, and for a cancelled or interrupted run). What the stage produced stays here.
pub fn run_view(rt: &WishRuntime, run: &Value, full: bool) -> Value {
    let mut v = json!({
        "ok": true, "run_id": run["run_id"], "program": run["program"], "state": run["state"],
        "stage": run["params"]["stage"], "wish_id": run["params"]["wish_id"], "params": run["params"],
        "created_at": run["created_at"], "updated_at": run["updated_at"], "record": run["detail"]["record"],
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
        v["evidence"] = json!({ "read_set": run["evidence"]["read_set"], "appended_inputs": run["evidence"]["appended_inputs"], "inputs": run["evidence"]["inputs"] });
        v["program_log"] = run["detail"]["program_log"].clone();
    }
    v
}

/// Whose AICC a new run uses (AI Center Quota PRD §3.3), fixed when the run is accepted and following where it runs:
/// a local run's model calls come from the starter's own session (an Account, or an Agent working as its own executor,
/// 第一期 §2.9); a hosted run uses the workspace owner's, through the Owner's Agent.
pub fn run_attribution(ws: &aiworkspace_store::workspace::Workspace, caller: &Caller, mode: exec::Mode) -> WsResult<Value> {
    use aiworkspace_store::workspace::{CallerClass, WISH_AICC_CURRENT_USER, WISH_AICC_WORKSPACE_OWNER};
    let (mode, user_id) = match mode {
        exec::Mode::Local => {
            let own = matches!(caller.class, CallerClass::Account { .. }) || (matches!(caller.class, CallerClass::Service) && caller.kind.as_deref() == Some("agent"));
            if !own {
                return Err(WsError::sub(Code::PermissionDenied, "AI_ATTRIBUTION_UNAVAILABLE", "You have no AI resources in this Zone: a wish block runs here with the AI resources of the user who starts it")
                    .with_data(json!({ "aicc": { "mode": WISH_AICC_CURRENT_USER } })));
            }
            (WISH_AICC_CURRENT_USER, caller.principal.clone())
        }
        exec::Mode::Hosted => (
            WISH_AICC_WORKSPACE_OWNER,
            ws.owner()?.ok_or_else(|| WsError::sub(Code::DependencyUnavailable, "AI_ATTRIBUTION_UNAVAILABLE", "The workspace has no owner whose AI resources the wish block could use"))?,
        ),
    };
    Ok(json!({ "mode": mode, "user_id": user_id, "operator_id": caller.principal }))
}

fn s<'a>(p: &'a Value, k: &str) -> WsResult<&'a str> {
    p.get(k).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{k} required")))
}

/// The caller a run acts for (权限需求 R14): its trigger as recorded — class, restriction, an Agent with its owner — with
/// its group and contact memberships settled now for this Workspace. Never `Caller::user(<name>)`: a Limited trigger
/// stays Limited, a member of a group keeps that group, a refused group stays refusing.
pub async fn delegated(state: &Arc<AppState>, ws_id: &str, run: &Value) -> Caller {
    let principal = run["principal"].as_str().unwrap_or("");
    let identity = &run["detail"]["context"]["trigger_identity"];
    let mut c = if identity.is_object() { Caller::delegated(identity, principal) } else { Caller::delegated(&json!({ "class": "account", "restricted": true }), principal) };
    crate::prepare_caller(state, &mut c, "proc.delegate", Some(ws_id)).await;
    c
}

/// Whether a run of `wish_id` written as `identity` could be written at all: its results and last run are kept on the
/// wish (`update` there), and it writes where its results go — the output container, or a result it produced before.
/// The Commit itself checks each operation; this only keeps a run nobody could write from starting.
fn macro_can_write(ctx: &dyn ReadCtx, identity: &Access, wish_id: &str, payload: &aiworkspace_core::model::JsonMap, group: Option<&str>) -> WsResult<()> {
    let refuse = |what: &str| {
        let within = group.map(|g| format!(", within the group @{g} its macro is bound to with intersection")).unwrap_or_default();
        WsError::sub(Code::PermissionDenied, "MACRO_CANNOT_WRITE", format!(
            "A run writes its results as whoever starts it{within}, and that cannot write {what}. Someone holding only run on a wish cannot run it yet: its results would need a trusted executor"))
            .with_data(json!({ "missing": what, "group_id": group }))
    };
    if !identity.can(ctx, wish_id, Cap::Update)? {
        return Err(refuse("the wish (update: its results and last run are kept there)"));
    }
    let mut places: Vec<String> = payload.get("output").and_then(|o| o.get("container_id")).and_then(Value::as_str).map(str::to_string)
        .or_else(|| ctx.edge(wish_id).ok().flatten().map(|e| e.parent_id)).into_iter().collect();
    places.extend(aiworkspace_core::wish::current_results(payload).iter().filter_map(|(_, b)| b.get("entity_id").and_then(Value::as_str).map(str::to_string)));
    for id in &places {
        if identity.caps(ctx, id)?.any_write() {
            return Ok(());
        }
    }
    Err(refuse("where its results go"))
}

/// Whose permissions a run's result is written with (§4.5, R28): an analysis is its analyst's own work — the trigger as
/// it is now; coding and running write as the macro identity.
pub fn run_identity(ws: &aiworkspace_store::workspace::Workspace, trigger: &Caller, run: &Value) -> WsResult<Access> {
    let wish_id = run["params"]["wish_id"].as_str().unwrap_or("");
    if run["params"]["stage"] == json!(STAGE_ANALYZE) {
        ws.access(trigger)
    } else {
        ws.macro_access(trigger, wish_id)
    }
}

/// The wish a run's macro acts for: its coding and running stages read and write as the macro identity (§4.5); an
/// analysis is its analyst's.
pub fn macro_scope(run: &Value) -> Option<String> {
    let stage = run["params"]["stage"].as_str().unwrap_or("");
    (is_wish_program(run["program"].as_str().unwrap_or("")) && (stage == STAGE_CODE || stage == STAGE_RUN)).then(|| run["params"]["wish_id"].as_str().map(str::to_string)).flatten()
}

/// [`run_view`] as `caller` may see it: how the owner's hosted execution is configured (its limits, its digest, the
/// Agent's goal) only for whoever manages the Workspace (权限需求 V1).
fn view_for(ws: &aiworkspace_store::workspace::Workspace, caller: &Caller, rt: &WishRuntime, run: &Value, full: bool) -> WsResult<Value> {
    let mut v = run_view(rt, run, full);
    if !ws.access(caller)?.ws_caps.has(Cap::Manage) {
        if let Some(e) = v["context"].get_mut("executor").and_then(Value::as_object_mut) {
            e.remove("limits");
            e.remove("config_digest");
        }
        if v["dispatch"].is_object() {
            v["dispatch"] = json!({ "status": v["dispatch"]["status"], "at": v["dispatch"]["at"] });
        }
    }
    Ok(v)
}

/// `proc.start` for `wish.xllm@1` (stages `analyze`, `code`, `run`).
pub fn start(state: &Arc<AppState>, caller: &Caller, ws_id: &str, program: &str, params: &Value, idem: &str) -> WsResult<Value> {
    let wish_id = s(params, "wish_id")?.to_string();
    let stage = s(params, "stage")?.to_string();
    if !STAGES.contains(&stage.as_str()) {
        return Err(WsError::invalid_op(format!("{program} has no stage {stage}")));
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
        // analysis and coding change the macro and hand the caller's words to a model: `update` on the wish (权限需求
        // R29); running it: `run` or any editing right on the wish (R22), wherever it executes
        let has = |c: &str| read["capabilities"].as_array().is_some_and(|caps| caps.iter().any(|x| x == c));
        let allowed = if stage == STAGE_RUN { ["run", "update", "append", "delete", "structure"].iter().any(|c| has(c)) } else { has("update") };
        if !allowed {
            let needed = if stage == STAGE_RUN { "run" } else { "update" };
            return Err(WsError::denied(format!("the {stage} stage of a wish needs the {needed} capability on it")));
        }
        let executor = read["content"]["payload"]["executor"].as_str().unwrap_or("");
        if executor != "xllm" {
            return Err(WsError::invalid_op(format!("the wish's executor is {executor}, not the one of {program}")));
        }
        if let Some(parent) = params.get("parent_run_id").and_then(Value::as_str) {
            let p = ws.wish_run(caller, parent)?;
            if p["params"]["wish_id"] != json!(wish_id) {
                return Err(WsError::invalid_op("parent_run_id belongs to another wish"));
            }
        }
        let payload = read["content"]["payload"].as_object().cloned().unwrap_or_default();
        ws.read_ctx(|ctx| ready(ctx, &payload, &stage))?;
        // a run is written as its macro identity — this phase the trigger, bounded by an `intersection` group; only a
        // trusted executor could write more (§4.5, Q12): a run that could never be written does not start, and spends
        // nobody's AI — a collaborator holding only `run` waits for trusted executors (权限需求 Q31)
        if stage != STAGE_ANALYZE {
            let identity = ws.macro_access(caller, &wish_id)?;
            let binding = ws.macro_binding(&wish_id)?;
            let payload = read["content"]["payload"].as_object().cloned().unwrap_or_default();
            ws.read_ctx(|ctx| macro_can_write(ctx, &identity, &wish_id, &payload, binding.as_ref().map(|(g, _)| g.as_str())))?;
        }
        let aicc = run_attribution(ws, caller, mode)?;
        let hosted = if mode == exec::Mode::Hosted { Some(ws.hosted_exec_admits(caller, &wish_id)?) } else { None };
        // the owner's limits on hosted runs are enforced, not just stored (权限需求 R30)
        if let Some(max) = hosted.as_ref().and_then(|c| c["limits"]["max_runs_per_day"].as_u64()) {
            let since = aiworkspace_core::value::format_utc_ms((ws.clock)() - 24 * 3_600_000);
            if ws.hosted_runs_since(&since)? >= max {
                return Err(WsError::limit(format!("the owner allows {max} hosted runs a day; try again later")));
            }
        }
        let context = exec::execution_context(ws, caller, &wish_id, &read["content"]["payload"], mode, &aicc, hosted.as_ref())?;
        let (exec_record, claim) = exec::new_exec_record(mode);
        let payload = &read["content"]["payload"];
        // what an executor starts from (inputs are read through the run token)
        let brief = json!({ "prompt": payload["prompt"], "knowledge": payload["knowledge"], "refinements": payload["refinements"], "analysis": payload["analysis"],
                            "inputs": payload["inputs"], "output": payload["output"], "request": params.get("request").cloned().unwrap_or(Value::Null), "stage": stage });
        let (run_id, replayed) = ws.wish_run_create_with(caller, program, params, idem, &aicc, &json!({ "context": context, "exec": exec_record, "brief": brief }))?;
        Ok((run_id, replayed, mode, claim, hosted, context))
    })?;
    let (run_id, replayed, mode, claim, hosted, context) = (run_id.0, run_id.1, run_id.2, run_id.3, run_id.4, run_id.5);
    if !replayed && mode == exec::Mode::Hosted {
        let (st, w, rid, token, cfg, ctx) = (state.clone(), ws_id.to_string(), run_id.clone(), claim.clone().unwrap_or_default(), hosted.clone().unwrap_or(Value::Null), context.clone());
        tokio::runtime::Handle::current().spawn(async move {
            // the claim token goes to the configured Agent only, never into the run's detail (权限需求 S6, R30): when the
            // dispatch fails the run waits for a retry or a cancel
            let dispatch = exec::dispatch_hosted(&st, &w, &rid, &token, &cfg, &ctx).await;
            let _ = st.with_ws(&w, |ws| ws.wish_run_set(&rid, None, None, None, None, Some(&json!({ "dispatch": dispatch }))));
        });
    }
    let mut v = get(state, caller, ws_id, &run_id)?;
    if !replayed {
        if let Some(t) = claim.filter(|_| mode == exec::Mode::Local) {
            // whoever started a local run is its executor: the claim token goes to it, once
            v["claim_token"] = json!(t);
        }
    }
    Ok(v)
}

/// `proc.get`: state, progress, and — once it ended — its Commit and record.
pub fn get(state: &Arc<AppState>, caller: &Caller, ws_id: &str, run_id: &str) -> WsResult<Value> {
    let rt = state.wish.clone();
    state.with_ws(ws_id, |ws| {
        let mut run = ws.wish_run(caller, run_id)?;
        if exec::expire_if_due(ws, &run)? {
            rt.drop_hosted(run_id);
            rt.unregister(run_id);
            run = ws.wish_run(caller, run_id)?;
        }
        let st = run["state"].as_str().unwrap_or("").to_string();
        // a run an executor owns waits for its claim (queued) or its lease (running), not for a service worker
        let executor_owned = run["detail"]["exec"]["mode"].as_str().is_some_and(|m| exec::Mode::parse(m).is_some()) && matches!(st.as_str(), "queued" | "running");
        if ACTIVE_STATES.contains(&st.as_str()) && st != "applying" && !executor_owned && rt.active(run_id).is_none() {
            // nobody owns it any more (service restart): interrupted, never silently re-run (§13.1)
            ws.wish_run_set(run_id, Some("interrupted"), None, None, None, Some(&json!({ "error": { "code": "INTERRUPTED", "detail": "The service restarted or was interrupted during the run; the run does not restart automatically" } })))?;
            run = ws.wish_run(caller, run_id)?;
        }
        if st == "applying" && rt.active(run_id).is_none() {
            ws.wish_reconcile_apply(run_id)?;
            run = ws.wish_run(caller, run_id)?;
        }
        view_for(ws, caller, &rt, &run, true)
    })
}

/// What a stage writes: the analysis write-back, the macro, or the results.
/// `identity`: what the Commit is written as ([`run_identity`]); the pre-check of the results is judged as it.
pub fn plan_for(ws: &mut aiworkspace_store::workspace::Workspace, caller: &Caller, run: &Value, identity: &Access) -> WsResult<Plan> {
    let cand = &run["candidate"];
    let run_id = run["run_id"].as_str().unwrap_or("");
    match run["params"]["stage"].as_str() {
        Some(STAGE_ANALYZE) => return ws.wish_analysis_plan(caller, cand),
        Some(STAGE_CODE) => return ws.wish_macro_plan(caller, run_id, cand),
        _ => {}
    }
    let mut plan = ws.wish_plan(caller, &PlanRequest { run_id, candidate: cand, location: &run["params"]["location"] })?;
    if plan.ready {
        // a dry run of the very commit: validation the planner cannot see (values, permissions, locks)
        let req = json!({ "protocol_version": aiworkspace_core::model::PROTOCOL_VERSION, "workspace_id": ws.workspace_id, "epoch": ws.epoch,
                          "idempotency_key": format!("precheck/{run_id}"), "origin": "program", "run_id": run["run_id"], "session_id": run["params"]["session_id"],
                          "preconditions": plan.preconditions, "operations": plan.operations });
        let opts = aiworkspace_store::workspace::CommitOpts { as_principal: run["principal"].as_str().map(str::to_string), authorize_as: Some(identity.clone()), ..Default::default() };
        let pre = ws.prepare_with(&req, caller, &opts);
        if pre["status"] != json!("ok") {
            plan.ready = false;
            let detail = pre["errors"][0]["detail"].as_str().or(pre["code"].as_str()).unwrap_or("").to_string();
            if let Some(a) = plan.summary["problems"].as_array_mut() {
                a.push(json!(format!("Pre-check failed ({}): {detail}", pre["code"].as_str().unwrap_or(""))));
            }
            plan.summary["precheck"] = pre;
        }
    }
    Ok(plan)
}

/// The delegation a run was started under still holds (D16, §9.5): the task configuration it bound, the owner's hosted
/// configuration, its context's life, and the attempt that produced what it hands in.
fn delegation_holds(ws: &aiworkspace_store::workspace::Workspace, caller: &Caller, run: &Value) -> WsResult<()> {
    if let Some(bound) = run["detail"]["context"]["task"]["digest"].as_str() {
        let wish_id = run["params"]["wish_id"].as_str().unwrap_or("");
        let payload = ws.read(caller, wish_id, None)?["content"]["payload"].as_object().cloned().unwrap_or_default();
        if aiworkspace_core::wish::basis_digest(&payload)[..32] != *bound {
            return Err(WsError::new(Code::DelegationStale, "the wish configuration changed after the run was started; run it again under the current configuration").with_data(json!({ "bound": bound })));
        }
    }
    // a hosted run still runs under the owner's configuration it was started with, within its context's life (R30)
    if let Some(bound) = run["detail"]["context"]["executor"]["config_digest"].as_str() {
        if ws.hosted_exec()?["digest"].as_str() != Some(bound) {
            return Err(WsError::new(Code::DelegationStale, "the owner's hosted execution configuration changed after the run was started; run it again").with_data(json!({ "bound": bound })));
        }
    }
    if run["detail"]["context"]["expires_at"].as_str().is_some_and(|e| e < ws.now().as_str()) {
        return Err(WsError::new(Code::DelegationStale, "the run's execution context expired; run it again"));
    }
    if let (Some(produced), Some(current)) = (run["detail"]["exec"]["provided"]["attempt"].as_u64(), run["detail"]["exec"]["attempt"].as_u64()) {
        if produced != current {
            return Err(WsError::new(Code::RunRevoked, format!("the stage's work came from attempt {produced}; attempt {current} superseded it")));
        }
    }
    Ok(())
}

/// The end of a stage (许愿格交互升级 §3.3): what it produced, planned and written with its record in one Commit — as
/// its macro identity ∩ the declared outputs (§4.5, R28), while the delegation holds. A plan with problems, a failed
/// pre-check, a conflict or a stale delegation is the run's error: nothing but the failure's record is written.
/// `log`: the last macro run's output, kept with the record (coding, running).
pub fn commit_stage(ws: &mut aiworkspace_store::workspace::Workspace, trigger: &Caller, run: &Value, log: &str) -> WsResult<()> {
    use aiworkspace_store::wish::history::log_head;
    delegation_holds(ws, trigger, run)?;
    let identity = run_identity(ws, trigger, run)?;
    let plan = plan_for(ws, trigger, run, &identity)?;
    if !plan.ready {
        let problems: Vec<String> = plan.summary["problems"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        return Err(WsError::sub(Code::InvalidOperation, "PLAN_PROBLEMS", format!("The stage's results cannot be written: {}", problems.join("; ")))
            .with_data(json!({ "problems": problems, "precheck": plan.summary["precheck"] })));
    }
    let (run_id, wish_id) = (run["run_id"].as_str().unwrap_or(""), run["params"]["wish_id"].as_str().unwrap_or(""));
    let stage = run["params"]["stage"].as_str().unwrap_or(STAGE_RUN);
    let payload = ws.read(trigger, wish_id, None)?["content"]["payload"].as_object().cloned().unwrap_or_default();
    let cand = &run["candidate"];
    let mut record = ws.wish_stage_record(run, cand, &run["evidence"], &plan, &payload)?;
    // what is large is kept as content objects: the request, the task contract as written, the macro, its log
    let written = |key: &str| plan.operations.iter().find(|o| o["op"] == json!("entity.set_keys")).and_then(|o| o["keys"].as_array()?.iter().find(|k| k["key"] == json!(key)).map(|k| k["value"].clone()));
    let mut parts: Vec<(&str, Vec<u8>)> = Vec::new();
    if stage != STAGE_RUN {
        parts.push(("prompt", payload.get("prompt").and_then(Value::as_str).unwrap_or("").as_bytes().to_vec()));
    }
    let mut source = None;
    match stage {
        STAGE_ANALYZE => {
            // as the wish keeps it: with the basis the core states for it, so a rollback restores it as it was
            let mut after = payload.clone();
            after.insert("analysis".into(), written("analysis").unwrap_or(Value::Null));
            after.insert("inputs".into(), cand["inputs"].clone());
            aiworkspace_core::wish::with_basis(&mut after);
            parts.push(("analysis", json!({ "analysis": after.get("analysis"), "inputs": cand["inputs"] }).to_string().into_bytes()));
        }
        STAGE_CODE => {
            let program = written("program").or_else(|| payload.get("program").cloned()).unwrap_or(Value::Null);
            source = program["source"].as_str().map(str::to_string);
            parts.push(("program", program.to_string().into_bytes()));
        }
        _ => {}
    }
    if stage != STAGE_ANALYZE && !log.is_empty() {
        parts.push(("log", log_head(log).as_bytes().to_vec()));
    }
    let mut objects = ws.wish_stage_objects(parts)?;
    if let Some(src) = source {
        objects["source"] = json!(src);
    }
    record["objects"] = objects;
    let attribution = json!({ "triggered_by": run["principal"], "executed_by": run["detail"]["exec"]["executor"]["principal"],
                              "judged_as": identity.principal, "macro": ws.macro_binding(wish_id)?.map(|(g, m)| json!({ "group_id": g, "mode": m })) });
    ws.wish_commit_stage(trigger, run_id, &plan, &record, Some(identity), Some(attribution))?;
    Ok(())
}

/// `proc.rollback { run_id, confirm?, session_id? }` (§2.1): the stage's current record undone (`update` on its wish).
pub fn rollback(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let run_id = s(p, "run_id")?;
    state.with_ws(ws_id, |ws| ws.wish_rollback(caller, run_id, p.get("confirm") == Some(&json!(true)), p.get("session_id").and_then(Value::as_str)))
}

/// `proc.cancel`: the run's stage closes (its executor's next call is refused); a run that wrote says so.
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
        }
        rt.drop_hosted(run_id);
        ws.wish_run_set(run_id, Some("cancelled"), None, None, None, Some(&json!({ "progress": progress_line("cancelled") })))?;
        Ok(json!({ "ok": true, "state": "cancelled", "already_applied": false }))
    })
}

/// `proc.list`: the caller's runs of a wish, newest first.
pub fn list(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let rt = state.wish.clone();
    state.with_ws(ws_id, |ws| {
        let runs = ws.wish_runs_list(caller, p.get("wish_id").and_then(Value::as_str), p.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize)?;
        let views = runs.iter().map(|r| view_for(ws, caller, &rt, r, false)).collect::<WsResult<Vec<_>>>()?;
        Ok(json!({ "ok": true, "runs": views }))
    })
}

pub fn is_wish(program: &str) -> bool {
    is_wish_program(program)
}

pub fn builtin_catalog() -> Value {
    serde_json::from_str(RENDERERS).unwrap_or(json!([]))
}

pub fn controls_catalog() -> Value {
    serde_json::from_str(CONTROLS).unwrap_or(json!({ "controls": [] }))
}
