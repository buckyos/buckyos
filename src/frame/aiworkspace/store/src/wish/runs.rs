//! Wish runs in `local.sqlite.runs` (许愿格 §4.2, §13): progress, evidence, candidates and cached
//! plans stay with this deployment and never enter the shared document. The columns are reused as
//! JSON: `params_json` (the start request), `inputs_json` (evidence: read set, appended inputs,
//! basis), `candidate_json` (the analysis or the result candidate), `result_json` (detail: progress,
//! usage, error, plans by digest, the application).

use super::plan::Plan;
use crate::docdb::db_err;
use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_core::access::Cap;
use aiworkspace_core::id::check_idempotency_key;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::model::{ReadCtx, PROTOCOL_VERSION};
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub const PROGRAM_XLLM: &str = "wish.xllm@1";
pub const PROGRAM_MOCK: &str = "wish.mock@1";
/// Plans kept per run (the latest previews).
const PLANS_KEPT: usize = 4;

/// States a worker owns; a run found in one of them without a worker was interrupted (`applying` is
/// reconciled through its idempotency record instead of being marked, see `wish_reconcile_apply`).
pub const ACTIVE_STATES: &[&str] = &["queued", "snapshotting", "running", "validating", "applying"];

/// `ACTIVE_STATES` as a SQL list.
pub fn active_states_sql() -> String {
    ACTIVE_STATES.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")
}

pub fn is_wish_program(p: &str) -> bool {
    p == PROGRAM_XLLM || p == PROGRAM_MOCK
}

fn parse(s: Option<String>) -> Value {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

impl Workspace {
    fn wish_row(&self, run_id: &str) -> WsResult<Value> {
        self.local
            .query_row(
                "SELECT run_id, program, principal, state, params_json, inputs_json, candidate_json, warnings_json, result_json, created_at, updated_at, idem_key \
                 FROM runs WHERE run_id = ?1",
                [run_id],
                |r| {
                    Ok(json!({ "run_id": r.get::<_, String>(0)?, "program": r.get::<_, String>(1)?, "principal": r.get::<_, String>(2)?,
                               "state": r.get::<_, String>(3)?, "params": parse(r.get(4)?), "evidence": parse(r.get(5)?),
                               "candidate": parse(r.get(6)?), "warnings": parse(r.get(7)?), "detail": parse(r.get(8)?),
                               "created_at": r.get::<_, String>(9)?, "updated_at": r.get::<_, String>(10)?, "idem_key": r.get::<_, String>(11)? }))
                },
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| WsError::not_found("run not found"))
    }

    /// A run as the caller sees it: its own, or — holding `run` or an editing right here — anyone's (D08:
    /// a run is the Workspace's, its result is written as the run's identity whoever applies it).
    pub fn wish_run(&self, caller: &Caller, run_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let r = self.wish_row(run_id)?;
        let visible = r["principal"] == json!(caller.principal) || access.ws_caps.may_run() || access.ws_caps.has(Cap::Manage);
        if !visible || !is_wish_program(r["program"].as_str().unwrap_or("")) {
            return Err(WsError::not_found("run not found"));
        }
        Ok(r)
    }

    /// Create a run record (`queued`). The same caller sending the same key gets the same run; the same
    /// key with other parameters is refused.
    /// `aicc` (whose AICC the run uses, fixed at acceptance) is stored with a new run only.
    pub fn wish_run_create(&mut self, caller: &Caller, program: &str, params: &Value, idem_key: &str, aicc: &Value) -> WsResult<(String, bool)> {
        self.wish_run_create_with(caller, program, params, idem_key, aicc, &json!({}))
    }

    /// [`Self::wish_run_create`] with more initial detail (the execution context and executor record of
    /// 内核架构改进 §9.6 / M3: `context`, `exec`).
    pub fn wish_run_create_with(&mut self, caller: &Caller, program: &str, params: &Value, idem_key: &str, aicc: &Value, detail: &Value) -> WsResult<(String, bool)> {
        self.require_run(caller)?;
        check_idempotency_key(idem_key)?;
        let digest = sha256_hex(canonical_json(params)?.as_bytes());
        let existing: Option<(String, String)> = self
            .local
            .query_row("SELECT run_id, params_json FROM runs WHERE principal = ?1 AND idem_key = ?2", params![caller.principal, idem_key], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(db_err)?;
        if let Some((run_id, p)) = existing {
            let same = serde_json::from_str::<Value>(&p).ok().and_then(|v| canonical_json(&v).ok()).map(|c| sha256_hex(c.as_bytes())) == Some(digest);
            if !same {
                return Err(WsError::new(Code::IdempotencyMismatch, "the same idempotency key was used for another request"));
            }
            return Ok((run_id, true));
        }
        let run_id = random_id("run_");
        let now = self.now();
        let mut d = json!({ "progress": { "phase": "queued" }, "aicc": aicc });
        for (k, v) in detail.as_object().into_iter().flatten() {
            d[k] = v.clone();
        }
        self.local
            .execute(
                "INSERT INTO runs (run_id, program, principal, idem_key, state, params_json, result_json, created_at, updated_at) VALUES (?1,?2,?3,?4,'queued',?5,?6,?7,?7)",
                params![run_id, program, caller.principal, idem_key, params.to_string(), d.to_string(), now],
            )
            .map_err(db_err)?;
        Ok((run_id, false))
    }

    /// `proc.claim` (D15, T3-2): an executor takes the run with its claim token and becomes its current
    /// attempt; it gets the run token every later call of this attempt carries. A second claim (a resume
    /// after `interrupted`, or another executor) starts a new attempt and invalidates the older one.
    pub fn wish_run_claim(&mut self, run_id: &str, claim_token: &str, executor: &Value) -> WsResult<Value> {
        let run = self.wish_row(run_id)?;
        let exec = &run["detail"]["exec"];
        if exec["claim_hash"].as_str() != Some(sha256_hex(claim_token.as_bytes()).as_str()) {
            return Err(WsError::denied("the claim token does not belong to this run"));
        }
        match run["state"].as_str() {
            Some("queued" | "running" | "interrupted") => {}
            Some("cancelled") => return Err(WsError::new(Code::RunRevoked, "the run was cancelled")),
            s => return Err(WsError::invalid_op(format!("run is {}", s.unwrap_or("?")))),
        }
        let attempt = exec["attempt"].as_u64().unwrap_or(0) + 1;
        let run_token = random_id("rt_");
        let lease = format_utc_ms((self.clock)() + run_lease_ms());
        // every attempt's token stays known, so a superseded executor hears RUN_REVOKED, not "unknown token"
        let mut tokens = exec["tokens"].as_object().cloned().unwrap_or_default();
        tokens.insert(sha256_hex(run_token.as_bytes()), json!(attempt));
        let detail = json!({ "exec": { "attempt": attempt, "tokens": tokens, "lease_until": lease, "executor": executor, "claimed_at": self.now() },
                             "progress": { "phase": "claimed", "attempt": attempt }, "error": Value::Null });
        self.wish_run_set(run_id, Some("running"), None, None, None, Some(&merge_exec(&run["detail"], detail)))?;
        Ok(json!({ "ok": true, "run_id": run_id, "attempt": attempt, "run_token": run_token, "lease_until": lease, "lease_ms": run_lease_ms() }))
    }

    /// The run a run token belongs to, with the attempt it was issued for.
    pub fn wish_run_by_token(&self, run_token: &str) -> WsResult<Option<(Value, u64)>> {
        let hash = sha256_hex(run_token.as_bytes());
        let mut st = self.local.prepare_cached("SELECT run_id FROM runs WHERE result_json LIKE ?1").map_err(db_err)?;
        let ids: Vec<String> = st.query_map([format!("%{hash}%")], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        for id in ids {
            let run = self.wish_row(&id)?;
            if let Some(attempt) = run["detail"]["exec"]["tokens"][&hash].as_u64() {
                return Ok(Some((run, attempt)));
            }
        }
        Ok(None)
    }

    /// The executor of `attempt` is still the run's current one and the run still wants results (§9.5).
    pub fn wish_run_check_attempt(&self, run: &Value, attempt: u64) -> WsResult<()> {
        if run["state"] == json!("cancelled") {
            return Err(WsError::new(Code::RunRevoked, "the run was cancelled; this attempt may not report"));
        }
        if run["state"] == json!("interrupted") {
            return Err(WsError::new(Code::RunRevoked, "the lease lapsed; claim the run again to continue (a new attempt)"));
        }
        let current = run["detail"]["exec"]["attempt"].as_u64().unwrap_or(0);
        if attempt != current {
            return Err(WsError::new(Code::RunRevoked, format!("attempt {attempt} was superseded by attempt {current}")).with_data(json!({ "attempt": attempt, "current": current })));
        }
        Ok(())
    }

    /// `proc.heartbeat`: the executor is alive; the lease moves on.
    pub fn wish_run_heartbeat(&mut self, run_id: &str, attempt: u64, progress: Option<&Value>) -> WsResult<Value> {
        let run = self.wish_row(run_id)?;
        self.wish_run_check_attempt(&run, attempt)?;
        let lease = format_utc_ms((self.clock)() + run_lease_ms());
        let mut d = json!({ "exec": { "lease_until": lease } });
        if let Some(p) = progress {
            d["progress"] = p.clone();
            d["progress"]["attempt"] = json!(attempt);
        }
        self.wish_run_set(run_id, None, None, None, None, Some(&merge_exec(&run["detail"], d)))?;
        Ok(json!({ "ok": true, "lease_until": lease, "lease_ms": run_lease_ms() }))
    }

    /// A run an executor owns whose lease ran out: interrupted (D15), never restarted by the service.
    pub fn wish_run_expire_lease(&mut self, run: &Value) -> WsResult<bool> {
        let exec = &run["detail"]["exec"];
        if exec["mode"].as_str().map_or(true, |m| m == "service") || !matches!(run["state"].as_str(), Some("running")) {
            return Ok(false);
        }
        let Some(lease) = exec["lease_until"].as_str() else { return Ok(false) };
        if lease > self.now().as_str() {
            return Ok(false);
        }
        let run_id = run["run_id"].as_str().unwrap_or("");
        self.wish_run_set(run_id, Some("interrupted"), None, None, None, Some(&json!({ "error": { "code": "INTERRUPTED", "detail": "The executor's lease expired; resume the run to continue (a new attempt)" }, "progress": { "phase": "interrupted" } })))?;
        Ok(true)
    }
}

/// Lease an executor must renew with `proc.heartbeat` (D15); `AIWS_RUN_LEASE_MS` shortens it for tests.
pub fn run_lease_ms() -> i64 {
    static LEASE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *LEASE.get_or_init(|| std::env::var("AIWS_RUN_LEASE_MS").ok().and_then(|v| v.parse().ok()).filter(|v: &i64| *v >= 1000).unwrap_or(90_000))
}

/// `patch` merged over `detail` one level into `exec` (and replacing the rest).
fn merge_exec(detail: &Value, mut patch: Value) -> Value {
    if let (Some(old), Some(new)) = (detail["exec"].as_object(), patch["exec"].as_object_mut()) {
        for (k, v) in old {
            new.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    patch
}

impl Workspace {

    /// Update a run. `detail` is merged key by key into the stored detail.
    pub fn wish_run_set(&mut self, run_id: &str, state: Option<&str>, candidate: Option<&Value>, evidence: Option<&Value>, warnings: Option<&Value>, detail: Option<&Value>) -> WsResult<()> {
        let row = self.wish_row(run_id)?;
        let mut d = if row["detail"].is_object() { row["detail"].clone() } else { json!({}) };
        if let Some(patch) = detail.and_then(Value::as_object) {
            for (k, v) in patch {
                if v.is_null() {
                    d.as_object_mut().unwrap().remove(k);
                } else {
                    d[k] = v.clone();
                }
            }
        }
        self.local
            .execute(
                "UPDATE runs SET state = COALESCE(?2, state), candidate_json = COALESCE(?3, candidate_json), inputs_json = COALESCE(?4, inputs_json), \
                 warnings_json = COALESCE(?5, warnings_json), result_json = ?6, updated_at = ?7 WHERE run_id = ?1",
                params![run_id, state, candidate.map(Value::to_string), evidence.map(Value::to_string), warnings.map(Value::to_string), d.to_string(), self.now()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// The caller's runs of one wish, newest first.
    pub fn wish_runs_list(&self, caller: &Caller, wish_id: Option<&str>, limit: usize) -> WsResult<Vec<Value>> {
        self.require_ws_any(caller)?;
        let mut st = self
            .local
            .prepare(
                "SELECT run_id FROM runs WHERE principal = ?1 AND program IN (?2, ?3) AND (?4 IS NULL OR json_extract(params_json, '$.wish_id') = ?4) \
                 ORDER BY created_at DESC, run_id DESC LIMIT ?5",
            )
            .map_err(db_err)?;
        let ids: Vec<String> = st
            .query_map(params![caller.principal, PROGRAM_XLLM, PROGRAM_MOCK, wish_id, limit.clamp(1, 100) as i64], |r| r.get(0))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        ids.iter().map(|id| self.wish_row(id)).collect()
    }

    /// Runs some worker should own (restart reconciliation).
    pub fn wish_runs_active(&self) -> WsResult<Vec<Value>> {
        let mut st = self
            .local
            .prepare("SELECT run_id FROM runs WHERE program IN (?1, ?2) AND state IN ('queued','snapshotting','running','validating','applying')")
            .map_err(db_err)?;
        let ids: Vec<String> = st.query_map(params![PROGRAM_XLLM, PROGRAM_MOCK], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        ids.iter().map(|id| self.wish_row(id)).collect()
    }

    /// Keep a plan the user previewed (by digest) for the application.
    pub fn wish_run_keep_plan(&mut self, run_id: &str, plan: &Plan) -> WsResult<()> {
        let row = self.wish_row(run_id)?;
        let mut plans = row["detail"].get("plans").cloned().filter(Value::is_object).unwrap_or(json!({}));
        let mut order: Vec<Value> = row["detail"].get("plan_order").and_then(Value::as_array).cloned().unwrap_or_default();
        order.retain(|d| *d != json!(plan.digest));
        order.push(json!(plan.digest));
        while order.len() > PLANS_KEPT {
            let old = order.remove(0);
            plans.as_object_mut().unwrap().remove(old.as_str().unwrap_or(""));
        }
        plans[&plan.digest] = plan.to_json();
        self.wish_run_set(run_id, None, None, None, None, Some(&json!({ "plans": plans, "plan_order": order })))
    }

    /// Commit a previewed plan (§11.4): one ordinary Commit as the caller, with the plan's preconditions,
    /// idempotent by run and plan. Conflicts and rejections keep the candidate.
    pub fn wish_apply(&mut self, caller: &Caller, run_id: &str, plan_digest: &str, session_id: Option<&str>) -> WsResult<Value> {
        let run = self.wish_run(caller, run_id)?;
        let state = run["state"].as_str().unwrap_or("");
        if state == "succeeded" {
            if run["detail"]["applied"]["plan_digest"] == json!(plan_digest) {
                return Ok(run["detail"]["applied"].clone());
            }
            return Err(WsError::invalid_op("this run was already applied"));
        }
        if state == "cancelled" {
            return Err(WsError::new(Code::RunCancelled, "the run was cancelled"));
        }
        if !matches!(state, "waiting_confirmation" | "conflict" | "rejected" | "applying") {
            return Err(WsError::invalid_op(format!("run is {state}")));
        }
        let plan = run["detail"]["plans"].get(plan_digest).cloned().ok_or_else(|| WsError::sub(Code::InvalidOperation, "PLAN_UNKNOWN", "this preview is no longer known: preview again"))?;
        if plan["ready"] != json!(true) {
            let problems = plan["summary"]["problems"].clone();
            return Err(WsError::sub(Code::InvalidOperation, "PLAN_NOT_READY", "the preview still has open choices or problems").with_data(json!({ "problems": problems, "manual": plan["summary"]["manual"] })));
        }
        if run["evidence"]["epoch"].as_str().is_some_and(|e| e != self.epoch) {
            self.wish_run_set(run_id, Some("conflict"), None, None, None, Some(&json!({ "error": { "code": "EPOCH_MISMATCH", "detail": "the Workspace history was replaced after the run" } })))?;
            return Err(WsError::new(Code::EpochMismatch, "the Workspace history was replaced after this run started"));
        }
        let stage = run["params"]["stage"].as_str().unwrap_or("execute");
        self.check_output_declaration(&run, plan["operations"].as_array().map(Vec::as_slice).unwrap_or(&[]))?;
        let request = json!({
            "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch,
            "idempotency_key": format!("run/{run_id}/{}", &plan_digest[..plan_digest.len().min(32)]),
            "origin": "program", "run_id": run_id,
            "message": if stage == "analyze" { "Write back wish block analysis".to_string() } else { format!("Apply wish block results{}", if run["program"] == json!(PROGRAM_MOCK) { " (simulated)" } else { "" }) },
            "preconditions": plan["preconditions"], "operations": plan["operations"],
        });
        let mut request = request;
        if let Some(s) = session_id {
            request["session_id"] = json!(s);
        }
        self.wish_run_set(run_id, Some("applying"), None, None, None, Some(&json!({ "applying": { "plan_digest": plan_digest, "idempotency_key": request["idempotency_key"] } })))?;
        // written as the run's execution identity: every apply of this run meets one idempotency record (D08)
        let opts = CommitOpts { as_principal: run["principal"].as_str().map(str::to_string), ..CommitOpts::default() };
        let result = self.commit(&request, caller, &opts);
        let status = result["status"].as_str().unwrap_or("rejected").to_string();
        let applied = json!({ "plan_digest": plan_digest, "status": status, "commit": result });
        let next = match status.as_str() {
            "accepted" => "succeeded",
            "conflict" => "conflict",
            _ => "rejected",
        };
        self.wish_run_set(run_id, Some(next), None, None, None, Some(&json!({ "applied": applied, "applying": Value::Null })))?;
        Ok(applied)
    }

    /// Results may only land where the task declared (D16, §9.4): the wish entity, the declared output
    /// container (and what is under it), results the wish already produced, and the inputs it derives
    /// columns for. Anything else is `PERMISSION_DENIED` / `OUTPUT_NOT_DECLARED`, whatever the writer may do by hand.
    fn check_output_declaration(&self, run: &Value, ops: &[Value]) -> WsResult<()> {
        let wish_id = run["params"]["wish_id"].as_str().unwrap_or("").to_string();
        let now = self.now();
        let ctx = self.ctx(&now);
        let Some(wish) = ctx.entity(&wish_id)? else { return Ok(()) };
        // the declared output container, or — undeclared — the folder the wish lives in
        let container = wish.payload.get("output").and_then(|o| o.get("container_id")).and_then(Value::as_str).map(str::to_string)
            .or_else(|| ctx.edge(&wish_id).ok().flatten().map(|e| e.parent_id));
        let mut allowed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        allowed.insert(wish_id.clone());
        for (_, b) in aiworkspace_core::wish::current_results(&wish.payload) {
            if let Some(id) = b.get("entity_id").and_then(Value::as_str) {
                allowed.insert(id.to_string());
            }
        }
        for i in wish.payload.get("inputs").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = i["entity_id"].as_str() {
                allowed.insert(id.to_string());
            }
        }
        if let Some(c) = &container {
            allowed.insert(c.clone());
        }
        // the Blocks showing the results go on the declared Surface (or the one the run was started from)
        if let Some(sf) = wish.payload.get("output").and_then(|o| o.get("surface_id")).and_then(Value::as_str) {
            allowed.insert(sf.to_string());
        }
        if let Some(sf) = run["params"]["location"]["surface_id"].as_str() {
            allowed.insert(sf.to_string());
        }
        if let Some(cell) = run["params"]["location"]["cell_id"].as_str() {
            if let Some(e) = ctx.edge(cell)? {
                allowed.insert(e.parent_id);
            }
        }
        // inside a declared place: the entity itself, or anything under one of them (results folders included)
        let inside = |id: &str| -> WsResult<bool> {
            let mut cur = id.to_string();
            for _ in 0..4096 {
                if allowed.contains(&cur) {
                    return Ok(true);
                }
                match ctx.edge(&cur)? {
                    Some(e) => cur = e.parent_id,
                    None => return Ok(false),
                }
            }
            Ok(false)
        };
        let mut created: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for op in ops {
            let name = op["op"].as_str().unwrap_or("");
            let target = op.get("entity_id").or_else(|| op.get("source_id")).and_then(Value::as_str).unwrap_or("");
            let parent = op.get("parent_id").and_then(Value::as_str);
            let ok = match name {
                "entity.create" => {
                    let fine = parent.is_some_and(|p| created.contains(p)) || parent.map_or(false, |p| inside(p).unwrap_or(false));
                    if fine {
                        created.insert(target.to_string());
                    }
                    fine
                }
                _ => created.contains(target) || inside(target)?,
            };
            if !ok {
                return Err(WsError::sub(Code::PermissionDenied, "OUTPUT_NOT_DECLARED", format!("the run would write {target}, which the task did not declare as an output")).with_data(json!({ "entity_id": target, "op": name })));
            }
        }
        Ok(())
    }

    /// A run left in `applying` (process exit, lost response): the commit either happened or not —
    /// the idempotency record says which (§11.4).
    pub fn wish_reconcile_apply(&mut self, run_id: &str) -> WsResult<Option<String>> {
        let run = self.wish_row(run_id)?;
        if run["state"] != json!("applying") {
            return Ok(None);
        }
        let key = run["detail"]["applying"]["idempotency_key"].as_str().unwrap_or("").to_string();
        let principal = run["principal"].as_str().unwrap_or("").to_string();
        let row: Option<String> = self
            .doc
            .query_row("SELECT result_json FROM commits WHERE principal = ?1 AND idem_key = ?2", params![principal, key], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        let (state, detail) = match row.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            Some(result) => ("succeeded", json!({ "applied": { "plan_digest": run["detail"]["applying"]["plan_digest"], "status": "accepted", "commit": result, "reconciled": true }, "applying": Value::Null })),
            None => ("waiting_confirmation", json!({ "applying": Value::Null })),
        };
        self.wish_run_set(run_id, Some(state), None, None, None, Some(&detail))?;
        Ok(Some(state.to_string()))
    }
}

impl Workspace {
    /// The program of a run (any caller; used to route `proc.*`).
    pub fn run_program(&self, run_id: &str) -> Option<String> {
        self.local.query_row("SELECT program FROM runs WHERE run_id = ?1", [run_id], |r| r.get(0)).optional().ok().flatten()
    }
}
