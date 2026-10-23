//! The execution model of a wish run (内核架构改进 §9, D14–D16, D22, D23, M3): every run binds an
//! explicit execution context; an executor outside the service — the browser (local) or an Agent Work
//! Session the Owner configured (hosted) — claims the run, keeps a lease alive, and reports candidate
//! results that the service plans and the user confirms. The transitional in-service executor stays
//! selectable until hosted execution passed DV (T3-9).

use super::{run_view, stages};
use crate::AppState;
use aiworkspace_core::canonical::sha256_hex;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::workspace::{random_id, Caller, Workspace};
use serde_json::{json, Value};
use std::sync::Arc;

/// Where a run executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Service,
    Local,
    Hosted,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "service" => Some(Mode::Service),
            "local" => Some(Mode::Local),
            "hosted" => Some(Mode::Hosted),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Service => "service",
            Mode::Local => "local",
            Mode::Hosted => "hosted",
        }
    }
}

/// A run's context lasts this long: an attempt reporting after it is refused.
const CONTEXT_TTL_MS: i64 = 24 * 3_600_000;

/// The execution context of a new run (§9.6): who triggers, who executes, where, whose AI and compute
/// resources, the task configuration it binds (D16) and the outputs it may write. Anything that cannot be
/// named is `EXECUTION_CONTEXT_MISSING` — never the service's or another user's identity.
pub fn execution_context(ws: &Workspace, caller: &Caller, wish_id: &str, payload: &Value, mode: Mode, aicc: &Value, hosted: Option<&Value>) -> WsResult<Value> {
    let missing = |what: &str| WsError::new(Code::ExecutionContextMissing, format!("the run cannot be started: {what}"));
    if caller.principal.is_empty() {
        return Err(missing("no trigger identity"));
    }
    if !aicc["user_id"].as_str().is_some_and(|u| !u.is_empty()) {
        return Err(missing("no AI resource attribution"));
    }
    let owner = ws.owner()?.ok_or_else(|| missing("the workspace has no owner"))?;
    let (executor, compute) = match mode {
        Mode::Service => (json!({ "kind": "service", "principal": "system:aiworkspace" }), json!({ "principal": owner, "kind": "service" })),
        Mode::Local => (json!({ "kind": "browser", "principal": caller.principal }), json!({ "principal": caller.principal, "kind": "browser" })),
        Mode::Hosted => {
            let cfg = hosted.ok_or_else(|| missing("hosted execution is not configured"))?;
            let agent = cfg["agent_id"].as_str().filter(|a| !a.is_empty()).ok_or_else(|| missing("the owner configured no agent"))?;
            (json!({ "kind": "agent", "principal": agent, "owner": owner, "config_digest": cfg["digest"] }), json!({ "principal": owner, "kind": "agent" }))
        }
    };
    let payload_map = payload.as_object().cloned().unwrap_or_default();
    let contract: Vec<Value> = payload["analysis"]["output_contract"]["results"].as_array().map(|a| a.iter().map(|r| json!({ "name": r["name"], "type": r["type"] })).collect()).unwrap_or_default();
    Ok(json!({
        "trigger": caller.principal,
        "executor": executor,
        "location": mode.as_str(),
        "ai": aicc,
        "compute": compute,
        // what the delegation binds (D16): the task as confirmed — prompt, knowledge, refinements, executor
        // configuration, inputs and contract — not the program a run writes back itself
        "task": { "wish_id": wish_id, "digest": aiworkspace_core::wish::basis_digest(&payload_map)[..32].to_string() },
        "outputs": { "container_id": payload["output"]["container_id"], "surface_id": payload["output"]["surface_id"], "name": payload["output"]["name"], "results": contract },
        "expires_at": format_utc_ms(now_ms(ws) + CONTEXT_TTL_MS),
    }))
}

fn now_ms(ws: &Workspace) -> i64 {
    (ws.clock)()
}

/// The executor record a new run starts with; the claim token leaves the service once, in the
/// `proc.start` answer (local) or the Agent's goal (hosted).
pub fn new_exec_record(mode: Mode) -> (Value, Option<String>) {
    match mode {
        Mode::Service => (json!({ "mode": "service", "attempt": 0 }), None),
        _ => {
            let token = random_id("ct_");
            (json!({ "mode": mode.as_str(), "attempt": 0, "claim_hash": sha256_hex(token.as_bytes()) }), Some(token))
        }
    }
}

/// `proc.claim { run_id, claim_token, executor? }`: the attempt, its run token and lease, and — for a
/// model run — the stage the executor works on (`stage`: prompts, tools, model; or the program to re-run).
pub fn claim(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let run_id = p.get("run_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("run_id required"))?;
    let token = p.get("claim_token").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("claim_token required"))?;
    let executor = json!({ "principal": caller.principal, "kind": caller.kind.clone().unwrap_or_else(|| "session".into()), "detail": p.get("executor").cloned().unwrap_or(Value::Null) });
    let (mut claimed, program) = state.with_ws(ws_id, |ws| {
        let run = ws.wish_run(caller, run_id)?;
        if run["detail"]["context"]["expires_at"].as_str().is_some_and(|e| e < ws.now().as_str()) {
            return Err(WsError::new(Code::RunRevoked, "the run's execution context expired"));
        }
        Ok((ws.wish_run_claim(run_id, token, &executor)?, run["program"].as_str().unwrap_or("").to_string()))
    })?;
    if program == super::PROGRAM_XLLM {
        // the run's own principal opens the stage: what the executor may read is what the trigger may
        let principal = state.with_ws(ws_id, |ws| Ok(ws.wish_run(caller, run_id)?["principal"].as_str().unwrap_or("").to_string()))?;
        let as_run = Caller { principal, ..Caller::user("") };
        match stages::host_open(state, &as_run, ws_id, run_id) {
            Ok(stage) => claimed["stage"] = stage,
            Err(e) => {
                let _ = state.with_ws(ws_id, |ws| ws.wish_run_set(run_id, Some("failed"), None, None, None, Some(&json!({ "error": e.to_json(), "progress": { "phase": "failed" } }))));
                return Err(e);
            }
        }
    }
    Ok(claimed)
}

/// The open stage of a run an outside executor works on, for its executor only.
fn hosted_stage(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<(Arc<super::StageState>, String, u64)> {
    let run_id = p.get("run_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("run_id required"))?.to_string();
    let attempt = p.get("attempt").and_then(Value::as_u64).ok_or_else(|| WsError::invalid_op("attempt required"))?;
    state.with_ws(ws_id, |ws| {
        let run = ws.wish_run(caller, &run_id)?;
        check_executor(caller, &run)?;
        ws.wish_run_check_attempt(&run, attempt)
    })?;
    let st = state.wish.hosted(&run_id).ok_or_else(|| WsError::invalid_op("the run has no open stage: claim it first"))?;
    Ok((st, run_id, attempt))
}

/// `proc.tool { run_id, attempt, name, args }`.
pub async fn tool(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let (st, c, w, body) = (state.clone(), caller.clone(), ws_id.to_string(), p.clone());
    let (stage, _, _) = tokio::task::spawn_blocking(move || hosted_stage(&st, &c, &w, &body)).await.map_err(|e| WsError::io(e.to_string()))??;
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("name required"))?;
    Ok(stages::host_tool(&stage, name, p.get("args").unwrap_or(&Value::Null)).await)
}

/// `proc.program { run_id, attempt, source, results?, log?, files? }`.
pub fn program(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let (stage, _, _) = hosted_stage(state, caller, ws_id, p)?;
    let source = p.get("source").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("source required"))?;
    stages::host_program(&stage, source, p.get("results").filter(|r| r.is_object()), p.get("log").and_then(Value::as_str).unwrap_or(""), p.get("files").unwrap_or(&Value::Null))
}

/// `proc.files { run_id, attempt }`.
pub fn files(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let (stage, _, _) = hosted_stage(state, caller, ws_id, p)?;
    stages::host_files(&stage)
}

/// `proc.heartbeat { run_id, attempt, progress? }`.
pub fn heartbeat(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let run_id = p.get("run_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("run_id required"))?;
    let attempt = p.get("attempt").and_then(Value::as_u64).ok_or_else(|| WsError::invalid_op("attempt required"))?;
    let v = state.with_ws(ws_id, |ws| {
        let run = ws.wish_run(caller, run_id)?;
        check_executor(caller, &run)?;
        ws.wish_run_heartbeat(run_id, attempt, p.get("progress"))
    })?;
    note_progress(state, run_id, p.get("progress"));
    Ok(v)
}

/// What the executor says about its progress also reaches the live view of a run with an open stage.
fn note_progress(state: &Arc<AppState>, run_id: &str, progress: Option<&Value>) {
    if let (Some(active), Some(o)) = (state.wish.active(run_id), progress.and_then(Value::as_object)) {
        let mut p = active.progress.lock().unwrap();
        for (k, v) in o {
            p[k] = v.clone();
        }
    }
}

/// `proc.report { run_id, attempt, progress? | error? | results… | analysis }`: the executor's word. A
/// report with results (`results`, `read_set?`, `summary?`, `warnings?`, `assumptions?`) or with an
/// `analysis` becomes the run's candidate: the service validates and plans it, the user confirms it.
pub fn report(state: &Arc<AppState>, caller: &Caller, ws_id: &str, p: &Value) -> WsResult<Value> {
    let run_id = p.get("run_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("run_id required"))?.to_string();
    let attempt = p.get("attempt").and_then(Value::as_u64).ok_or_else(|| WsError::invalid_op("attempt required"))?;
    let has_results = p.get("results").is_some_and(Value::is_array);
    let has_analysis = p.get("analysis").is_some_and(Value::is_object);
    let (c, rid, body) = (caller.clone(), run_id.clone(), p.clone());
    let provided = state.with_ws(ws_id, move |ws| {
        let run = ws.wish_run(&c, &rid)?;
        check_executor(&c, &run)?;
        ws.wish_run_check_attempt(&run, attempt)?;
        if !matches!(run["state"].as_str(), Some("running")) {
            return Err(WsError::invalid_op(format!("run is {}", run["state"].as_str().unwrap_or("?"))));
        }
        if let Some(err) = body.get("error").filter(|e| !e.is_null()) {
            ws.wish_run_set(&rid, Some("failed"), None, None, None, Some(&json!({ "error": err, "progress": { "phase": "failed", "attempt": attempt } })))?;
            return Ok(None);
        }
        if body.get("finished") == Some(&json!(true)) {
            // the executor worked on the service's stage: what it collected is the candidate
            ws.wish_run_set(&rid, Some("validating"), None, None, None, Some(&json!({ "exec": merge_exec_field(&run["detail"]["exec"], json!({ "provided": { "attempt": attempt, "stage_host": true } })), "progress": { "phase": "validating", "attempt": attempt } })))?;
            return Ok(Some(json!({ "stage_host": true })));
        }
        if !has_results && !has_analysis {
            let mut progress = body.get("progress").cloned().unwrap_or_else(|| json!({ "phase": "running" }));
            progress["attempt"] = json!(attempt);
            ws.wish_run_heartbeat(&rid, attempt, Some(&progress))?;
            return Ok(Some(json!({ "progress": progress })));
        }
        let stage = run["params"]["stage"].as_str().unwrap_or("execute");
        if has_analysis && stage != "analyze" {
            return Err(WsError::invalid_op("an analysis is reported by an analyze run"));
        }
        if has_results && stage == "analyze" {
            return Err(WsError::invalid_op("an analyze run reports an analysis, not results"));
        }
        let mut provided = json!({ "attempt": attempt, "read_set": body.get("read_set").cloned().unwrap_or(json!([])), "summary": body.get("summary").cloned().unwrap_or(json!("")),
                                   "warnings": body.get("warnings").cloned().unwrap_or(json!([])), "assumptions": body.get("assumptions").cloned().unwrap_or(json!([])) });
        if has_results {
            provided["results"] = body["results"].clone();
        }
        if has_analysis {
            provided["analysis"] = body["analysis"].clone();
        }
        ws.wish_run_set(&rid, Some("validating"), None, None, None, Some(&json!({ "exec": merge_exec_field(&run["detail"]["exec"], json!({ "provided": provided.clone() })), "progress": { "phase": "validating", "attempt": attempt } })))?;
        Ok(Some(provided))
    })?;
    if let Some(provided) = provided.filter(|v| v.get("progress").is_none()) {
        let principal = state.with_ws(ws_id, |ws| Ok(ws.wish_run(caller, &run_id)?["principal"].as_str().unwrap_or("").to_string()))?;
        let as_run = Caller { principal, ..Caller::user("") };
        let (st, ws, rid) = (state.clone(), ws_id.to_string(), run_id.clone());
        if provided["stage_host"] == json!(true) {
            let stage = state.wish.hosted(&run_id).ok_or_else(|| WsError::invalid_op("the run has no open stage: claim it first"))?;
            let repair = state.with_ws(ws_id, |ws| Ok(ws.wish_run(caller, &run_id)?["params"]["stage"] == json!("repair_program")))?;
            tokio::runtime::Handle::current().spawn(async move {
                stages::drive_host(st, as_run, ws, rid, stage, repair).await;
            });
        } else {
            // the service plans what was reported, exactly like a Mock executor's results (§3)
            state.wish.drop_hosted(&run_id);
            let active = state.wish.register(&run_id);
            tokio::runtime::Handle::current().spawn(async move {
                stages::drive(st, as_run, ws, rid, active).await;
            });
        }
    } else if p.get("error").is_some_and(|e| !e.is_null()) {
        state.wish.drop_hosted(&run_id);
        state.wish.unregister(&run_id);
    } else {
        note_progress(state, &run_id, p.get("progress"));
    }
    state.with_ws(ws_id, |ws| {
        let run = ws.wish_run(caller, &run_id)?;
        Ok(run_view(&state.wish, &run, false))
    })
}

fn merge_exec_field(old: &Value, mut patch: Value) -> Value {
    if let (Some(o), Some(n)) = (old.as_object(), patch.as_object_mut()) {
        for (k, v) in o {
            n.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    patch
}

/// Only the run's executor speaks for it: the run token's holder, or a session of the run's own
/// principal (the browser that started a local run) or of the configured Agent.
fn check_executor(caller: &Caller, run: &Value) -> WsResult<()> {
    if caller.kind.as_deref() == Some("run") {
        if caller.run_scope.as_deref() != run["run_id"].as_str() {
            return Err(WsError::denied("this run token belongs to another run"));
        }
        return Ok(());
    }
    let exec = &run["detail"]["context"]["executor"];
    if run["principal"] == json!(caller.principal) || exec["principal"] == json!(caller.principal) {
        return Ok(());
    }
    Err(WsError::denied("only the run's executor reports on it"))
}

/// A local or hosted run whose lease ran out is `interrupted` the moment it is read (D15).
pub fn expire_if_due(ws: &mut Workspace, run: &Value) -> WsResult<bool> {
    ws.wish_run_expire_lease(run)
}

/// Hosted dispatch (D23, T3-5): the Owner's Agent gets a Goal with the run reference and its claim
/// token, and is activated. Without a Zone runtime (standalone, tests) the run stays claimable and the
/// dispatch is recorded as `unavailable`.
pub async fn dispatch_hosted(state: &Arc<AppState>, ws_id: &str, run_id: &str, claim_token: &str, cfg: &Value, context: &Value) -> Value {
    let agent = cfg["agent_id"].as_str().unwrap_or("").to_string();
    let owner = context["compute"]["principal"].as_str().unwrap_or("").to_string();
    let objective = format!(
        "Execute AI Workspace run {run_id} of workspace {ws_id} (wish {}). Follow the `aiworkspace_run` command manual: claim the run with \
         `buckyos aiws run-claim --workspace {ws_id} --run {run_id} --claim-token {claim_token}`, read its brief and inputs, produce the results, \
         report them with `buckyos aiws run-report`, and keep the lease alive with `buckyos aiws run-heartbeat` while you work. Resources: the workspace owner's ({owner}).",
        context["task"]["wish_id"].as_str().unwrap_or("")
    );
    let _ = state;
    match dispatch_goal(&agent, &owner, run_id, &objective).await {
        Ok(goal) => json!({ "status": "dispatched", "agent_id": agent, "goal": goal, "at": format_utc_ms(now_wall()) }),
        Err(e) => json!({ "status": "unavailable", "agent_id": agent, "detail": e, "at": format_utc_ms(now_wall()) }),
    }
}

fn now_wall() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `goals.create` + `goals.activate` on the Agent's OpenDAN service (its AgentSpec binding names the service).
async fn dispatch_goal(agent_id: &str, owner: &str, run_id: &str, objective: &str) -> Result<Value, String> {
    let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| format!("no zone runtime: {e}"))?;
    let agent = buckyos_api::AgentId::parse(agent_id).map_err(|e| format!("bad agent id: {e:?}"))?;
    let sysconf = runtime.get_system_config_client().await.map_err(|e| format!("system-config: {e}"))?;
    let key = buckyos_api::agent_spec_key(owner, &agent);
    let spec = sysconf.get(&key).await.map_err(|e| format!("agent spec {key}: {e}"))?;
    let spec: buckyos_api::AgentSpec = serde_json::from_str(&spec.value).map_err(|e| format!("agent spec: {e}"))?;
    let client = runtime.get_zone_service_krpc_client(&spec.binding.service_name).await.map_err(|e| format!("agent service: {e}"))?;
    let created = client
        .call("goals.create", json!({ "request": { "key": format!("aiws/{run_id}"), "title": format!("AI Workspace run {run_id}"), "objective": objective, "initial_skills": [{ "name": "aiworkspace_run" }] } }))
        .await
        .map_err(|e| format!("goals.create: {e}"))?;
    let goal_id = created["goal_id"].as_str().or_else(|| created["id"].as_str()).unwrap_or("").to_string();
    if !goal_id.is_empty() {
        let _ = client.call("goals.activate", json!({ "goal_id": goal_id, "reason": format!("AI Workspace run {run_id}") })).await.map_err(|e| format!("goals.activate: {e}"))?;
    }
    Ok(json!({ "goal_id": goal_id, "service": spec.binding.service_name }))
}
