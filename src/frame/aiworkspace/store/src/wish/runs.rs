//! Wish runs in `local.sqlite.runs` (许愿格 §4.2, §13): progress, evidence, candidates and cached
//! plans stay with this deployment and never enter the shared document. The columns are reused as
//! JSON: `params_json` (the start request), `inputs_json` (evidence: read set, appended inputs,
//! basis), `candidate_json` (the analysis or the result candidate), `result_json` (detail: progress,
//! usage, error, plans by digest, the application).

use super::plan::Plan;
use crate::docdb::db_err;
use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_core::id::check_idempotency_key;
use aiworkspace_core::model::PROTOCOL_VERSION;
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub const PROGRAM_XLLM: &str = "wish.xllm@1";
pub const PROGRAM_MOCK: &str = "wish.mock@1";
/// Plans kept per run (the latest previews).
const PLANS_KEPT: usize = 4;

/// States a worker owns; a run found in one of them without a worker was interrupted.
pub const ACTIVE_STATES: &[&str] = &["queued", "snapshotting", "running", "validating"];

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

    /// A run of the caller (others' runs are not found).
    pub fn wish_run(&self, caller: &Caller, run_id: &str) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let r = self.wish_row(run_id)?;
        if r["principal"] != json!(caller.principal) || !is_wish_program(r["program"].as_str().unwrap_or("")) {
            return Err(WsError::not_found("run not found"));
        }
        Ok(r)
    }

    /// Create a run record (`queued`). The same caller sending the same key gets the same run; the same
    /// key with other parameters is refused.
    pub fn wish_run_create(&mut self, caller: &Caller, program: &str, params: &Value, idem_key: &str) -> WsResult<(String, bool)> {
        self.require_ws_any(caller)?;
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
                return Err(WsError::sub(Code::InvalidOperation, "IDEMPOTENCY_MISMATCH", "the same idempotency key was used for another request"));
            }
            return Ok((run_id, true));
        }
        let run_id = random_id("run_");
        let now = self.now();
        self.local
            .execute(
                "INSERT INTO runs (run_id, program, principal, idem_key, state, params_json, result_json, created_at, updated_at) VALUES (?1,?2,?3,?4,'queued',?5,?6,?7,?7)",
                params![run_id, program, caller.principal, idem_key, params.to_string(), json!({ "progress": { "phase": "queued" } }).to_string(), now],
            )
            .map_err(db_err)?;
        Ok((run_id, false))
    }

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
        let request = json!({
            "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch,
            "idempotency_key": format!("run/{run_id}/{}", &plan_digest[..plan_digest.len().min(32)]),
            "origin": "program", "run_id": run_id,
            "message": if stage == "analyze" { "许愿格分析写回".to_string() } else { format!("应用许愿格结果{}", if run["program"] == json!(PROGRAM_MOCK) { "（模拟）" } else { "" }) },
            "preconditions": plan["preconditions"], "operations": plan["operations"],
        });
        let mut request = request;
        if let Some(s) = session_id {
            request["session_id"] = json!(s);
        }
        self.wish_run_set(run_id, Some("applying"), None, None, None, Some(&json!({ "applying": { "plan_digest": plan_digest, "idempotency_key": request["idempotency_key"] } })))?;
        let result = self.commit(&request, caller, &CommitOpts::default());
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
