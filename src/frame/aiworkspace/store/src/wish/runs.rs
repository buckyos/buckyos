//! Wish runs in `local.sqlite.runs` (许愿格 §4.2, §13): progress, evidence and the stage's candidate stay with this
//! deployment and never enter the shared document; what a stage produced is written when it ends, with its record in
//! the wish's `history` (许愿格交互升级 §3.3). The columns are reused as JSON: `params_json` (the start request),
//! `inputs_json` (evidence: read set, appended inputs, basis), `candidate_json` (the analysis or the result candidate),
//! `result_json` (detail: progress, usage, error, the Commit and the record).

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

/// States a worker owns; a run found in one of them without a worker was interrupted (`applying` is
/// reconciled through its idempotency record instead of being marked, see `wish_reconcile_apply`).
pub const ACTIVE_STATES: &[&str] = &["queued", "snapshotting", "running", "validating", "applying"];

/// `ACTIVE_STATES` as a SQL list.
pub fn active_states_sql() -> String {
    ACTIVE_STATES.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")
}

pub fn is_wish_program(p: &str) -> bool {
    p == PROGRAM_XLLM
}

fn parse(s: Option<String>) -> Value {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

impl Workspace {
    pub(crate) fn wish_row(&self, run_id: &str) -> WsResult<Value> {
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

    /// A run as the caller sees it: its own, or — holding `update` or `manage` on its wish — anyone's (D08: a run is
    /// the Workspace's, its result is written as the run's identity whoever applies it). `run` alone shows only one's
    /// own runs (权限需求 R22, E4).
    pub fn wish_run(&self, caller: &Caller, run_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let r = self.wish_row(run_id)?;
        let caps = match r["params"]["wish_id"].as_str() {
            Some(w) => {
                let now = self.now();
                access.caps(&self.ctx(&now), w)?
            }
            None => access.ws_caps,
        };
        let visible = r["principal"] == json!(caller.principal) || caps.has(Cap::Update) || caps.has(Cap::Manage);
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
        self.require_run(caller, params.get("wish_id").and_then(Value::as_str))?;
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
        if exec["mode"].as_str().is_none() || !matches!(run["state"].as_str(), Some("running")) {
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
        // when the state last changed: where the work log (UI 框架 PRD K3) puts a run's end
        if state.is_some_and(|s| row["state"] != json!(s)) {
            d["state_at"] = json!(self.now());
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
                "SELECT run_id FROM runs WHERE principal = ?1 AND program = ?2 AND (?3 IS NULL OR json_extract(params_json, '$.wish_id') = ?3) \
                 ORDER BY created_at DESC, run_id DESC LIMIT ?4",
            )
            .map_err(db_err)?;
        let ids: Vec<String> = st
            .query_map(params![caller.principal, PROGRAM_XLLM, wish_id, limit.clamp(1, 100) as i64], |r| r.get(0))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        ids.iter().map(|id| self.wish_row(id)).collect()
    }

    /// Runs some worker should own (restart reconciliation).
    pub fn wish_runs_active(&self) -> WsResult<Vec<Value>> {
        let mut st = self
            .local
            .prepare("SELECT run_id FROM runs WHERE program = ?1 AND state IN ('queued','snapshotting','running','validating','applying')")
            .map_err(db_err)?;
        let ids: Vec<String> = st.query_map(params![PROGRAM_XLLM], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        ids.iter().map(|id| self.wish_row(id)).collect()
    }

    /// Write what a stage produced (许愿格交互升级 §3.3): one ordinary Commit written as the run (origin `program`),
    /// authorized as `identity` (its macro identity, 权限需求 R28) with who triggered and executed it (R31), guarded by
    /// the plan's preconditions, idempotent by run, in the session the run was started from (`params.session_id`: the write
    /// locks it holds). The wish's operation carries `record` into `history`. `applying`
    /// keeps the idempotency key and the record, so a crash between the Commit and the run's state is reconciled. A
    /// Commit that is not accepted is the run's error.
    pub fn wish_commit_stage(&mut self, caller: &Caller, run_id: &str, plan: &Plan, record: &Value, identity: Option<aiworkspace_core::access::Access>, attribution: Option<Value>) -> WsResult<Value> {
        let run = self.wish_run(caller, run_id)?;
        match run["state"].as_str().unwrap_or("") {
            "cancelled" => return Err(WsError::new(Code::RunCancelled, "the run was cancelled")),
            "validating" => {}
            s => return Err(WsError::invalid_op(format!("run is {s}"))),
        }
        if run["evidence"]["epoch"].as_str().is_some_and(|e| e != self.epoch) {
            return Err(WsError::new(Code::EpochMismatch, "the Workspace history was replaced after this run started"));
        }
        self.check_output_declaration(&run, &plan.operations)?;
        let wish_id = run["params"]["wish_id"].as_str().unwrap_or("").to_string();
        let now = self.now();
        let wish = self.ctx(&now).entity(&wish_id)?.filter(|e| e.alive()).ok_or_else(|| WsError::deleted("the wish was deleted"))?;
        let commit_id = random_id("cm_");
        let mut record = record.clone();
        record["commit_id"] = json!(commit_id);
        let history = super::history::with_record(wish.payload.get("history"), &record);
        let item = json!({ "key": "history", "value": history, "expect": { "rev": wish.key_rev("history") } });
        let mut ops = plan.operations.clone();
        match ops.iter_mut().rev().find(|o| o["op"] == json!("entity.set_keys") && o["entity_id"] == json!(wish_id)) {
            Some(op) => op["keys"].as_array_mut().map(|k| k.push(item)).unwrap_or(()),
            None => ops.push(json!({ "op": "entity.set_keys", "entity_id": wish_id, "keys": [item] })),
        }
        let stage = run["params"]["stage"].as_str().unwrap_or("run");
        let message = match stage {
            "analyze" => "Write back wish block analysis",
            "code" => "Store wish block macro",
            _ => "Write wish block results",
        };
        let mut request = json!({
            "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch,
            "idempotency_key": format!("run/{run_id}"), "origin": "program", "run_id": run_id, "message": message,
            "preconditions": plan.preconditions, "operations": ops,
        });
        if let Some(s) = run["params"]["session_id"].as_str() {
            request["session_id"] = json!(s);
        }
        self.wish_run_set(run_id, Some("applying"), None, None, None, Some(&json!({ "applying": { "idempotency_key": request["idempotency_key"], "record": record } })))?;
        let opts = CommitOpts { as_principal: run["principal"].as_str().map(str::to_string), authorize_as: identity, attribution: attribution.clone(), commit_id: Some(commit_id), ..CommitOpts::default() };
        let result = self.commit(&request, caller, &opts);
        if result["status"] != json!("accepted") {
            self.wish_run_set(run_id, Some("validating"), None, None, None, Some(&json!({ "applying": Value::Null })))?;
            return Err(super::history::commit_error(&result, "the stage's results were not written"));
        }
        let applied = json!({ "status": "accepted", "commit": result, "attribution": attribution });
        self.wish_run_set(run_id, Some("succeeded"), None, None, None, Some(&json!({ "applied": applied, "record": record, "applying": Value::Null })))?;
        Ok(applied)
    }

    /// Results may only land where the task declared (D16, §9.4; 权限需求 R33): the wish entity, the results it already
    /// produced, the declared output container (and what is under it; undeclared: the folder the wish lives in), and the
    /// declared Surface — or, with none declared, the Surface of a Block showing this wish. A run's `location` only
    /// chooses among those. An input is never a root: it is written only as itself, by a derived-column result the
    /// output contract declares. Anything else is `PERMISSION_DENIED` / `OUTPUT_NOT_DECLARED`, whatever the writer may
    /// do by hand.
    fn check_output_declaration(&self, run: &Value, ops: &[Value]) -> WsResult<()> {
        let wish_id = run["params"]["wish_id"].as_str().unwrap_or("").to_string();
        let now = self.now();
        let ctx = self.ctx(&now);
        let Some(wish) = ctx.entity(&wish_id)? else { return Ok(()) };
        let output = wish.payload.get("output").cloned().unwrap_or(Value::Null);
        let container = output.get("container_id").and_then(Value::as_str).map(str::to_string).or_else(|| ctx.edge(&wish_id).ok().flatten().map(|e| e.parent_id));
        let mut allowed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        allowed.insert(wish_id.clone());
        for (_, b) in aiworkspace_core::wish::current_results(&wish.payload) {
            if let Some(id) = b.get("entity_id").and_then(Value::as_str) {
                allowed.insert(id.to_string());
            }
        }
        if let Some(c) = &container {
            allowed.insert(c.clone());
        }
        // the Surface results show on: the declared one; else the one a Block showing this wish sits on (where the run
        // was started from, when the client says so)
        let shows_wish = |cell: &str| -> WsResult<bool> {
            Ok(ctx.entity(cell)?.is_some_and(|c| c.payload.get("source_ref").and_then(|r| r.get("entity_id")).and_then(Value::as_str) == Some(wish_id.as_str())))
        };
        match output.get("surface_id").and_then(Value::as_str) {
            Some(sf) => {
                allowed.insert(sf.to_string());
            }
            None => {
                if let Some(cell) = run["params"]["location"]["cell_id"].as_str() {
                    if shows_wish(cell)? {
                        if let Some(e) = ctx.edge(cell)? {
                            allowed.insert(e.parent_id);
                        }
                    }
                }
            }
        }
        // inputs: written only as themselves, by a declared derived-column result
        let declares_columns = wish.payload.get("analysis").and_then(|a| a["output_contract"]["results"].as_array()).is_some_and(|rs| rs.iter().any(|r| matches!(r["type"].as_str(), Some("columns" | "table_columns"))));
        let mut exact: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        if declares_columns {
            for i in wish.payload.get("inputs").and_then(Value::as_array).into_iter().flatten() {
                if let Some(id) = i["entity_id"].as_str() {
                    exact.insert(id.to_string());
                }
            }
        }
        // inside a declared place: the entity itself, or anything under one of them (results folders included)
        let inside = |id: &str| -> WsResult<bool> {
            if exact.contains(id) {
                return Ok(true);
            }
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
                    let fine = parent.is_some_and(|p| created.contains(p)) || parent.map_or(false, |p| !exact.contains(p) && inside(p).unwrap_or(false));
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

    /// A run left in `applying` (process exit): the Commit either happened or not — the idempotency record says which
    /// (§11.4).
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
        // committed: its record is in the history already; not: the stage's work is lost with the process
        let (state, detail) = match row.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            Some(result) => ("succeeded", json!({ "applied": { "status": "accepted", "commit": result, "reconciled": true }, "record": run["detail"]["applying"]["record"], "applying": Value::Null })),
            None => ("interrupted", json!({ "applying": Value::Null, "error": { "code": "INTERRUPTED", "detail": "The service stopped while writing the stage's results; nothing was written: run the stage again" } })),
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
