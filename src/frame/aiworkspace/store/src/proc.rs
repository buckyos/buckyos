//! Controlled processing and the deterministic Mock (design doc §7).
//!
//! A run produces a *candidate*: an ordinary Commit request stored as data.
//! Applying it goes through `doc.commit` like any other caller — the Mock has
//! no entry that bypasses validation, locks or the read-set preconditions.

use crate::docdb::db_err;
use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::id::check_idempotency_key;
use aiworkspace_core::model::*;
use aiworkspace_core::read::readable_entity;
use aiworkspace_core::value::{days_from_civil, parse_date};
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub const MOCK_PROGRAM: &str = "mock.task-summary@1";

fn para(id: &str, text: &str) -> Value {
    json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] })
}

impl Workspace {
    fn run_json(&self, run_id: &str) -> WsResult<Value> {
        self.local
            .query_row(
                "SELECT run_id, program, principal, state, params_json, inputs_json, candidate_json, warnings_json, result_json, created_at, updated_at \
                 FROM runs WHERE run_id = ?1",
                [run_id],
                |r| {
                    let parse = |s: Option<String>| s.and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null);
                    Ok(json!({ "run_id": r.get::<_, String>(0)?, "program": r.get::<_, String>(1)?, "principal": r.get::<_, String>(2)?,
                               "state": r.get::<_, String>(3)?, "params": parse(r.get(4)?), "inputs": parse(r.get(5)?),
                               "candidate": parse(r.get(6)?), "warnings": parse(r.get(7)?), "result": parse(r.get(8)?),
                               "created_at": r.get::<_, String>(9)?, "updated_at": r.get::<_, String>(10)?,
                               // every Mock output is labelled as simulated
                               "simulated": r.get::<_, String>(1)?.starts_with("mock.") }))
                },
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| WsError::not_found("run not found"))
    }

    fn set_run(&self, run_id: &str, state: &str, result: Option<&Value>) -> WsResult<()> {
        self.local
            .execute(
                "UPDATE runs SET state = ?2, result_json = COALESCE(?3, result_json), updated_at = ?4 WHERE run_id = ?1",
                params![run_id, state, result.map(Value::to_string), self.now()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// `proc.start`: fix the inputs in one consistent read and store the candidate.
    pub fn proc_start(&mut self, caller: &Caller, program: &str, params: &Value, idem_key: &str) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        check_idempotency_key(idem_key)?;
        if program != MOCK_PROGRAM {
            return Err(WsError::new(Code::MissingExtension, format!("unknown program {program}")));
        }
        let existing: Option<String> = self
            .local
            .query_row("SELECT run_id FROM runs WHERE principal = ?1 AND idem_key = ?2", params![caller.principal, idem_key], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        if let Some(run_id) = existing {
            let mut v = self.run_json(&run_id)?;
            v["ok"] = json!(true);
            v["replayed"] = json!(true);
            return Ok(v);
        }
        let run_id = random_id("run_");
        let now = self.now();
        self.local
            .execute(
                "INSERT INTO runs (run_id, program, principal, idem_key, state, params_json, created_at, updated_at) VALUES (?1,?2,?3,?4,'planning',?5,?6,?6)",
                params![run_id, program, caller.principal, idem_key, params.to_string(), now],
            )
            .map_err(db_err)?;
        self.set_run(&run_id, "running", None)?;
        match self.mock_task_summary(caller, &run_id, params) {
            Ok((candidate, inputs, warnings)) => {
                self.local
                    .execute(
                        "UPDATE runs SET candidate_json = ?2, inputs_json = ?3, warnings_json = ?4 WHERE run_id = ?1",
                        params![run_id, candidate.to_string(), inputs.to_string(), warnings.to_string()],
                    )
                    .map_err(db_err)?;
                self.set_run(&run_id, "validating", None)?;
                self.set_run(&run_id, "waiting_confirmation", None)?;
                if params.get("auto_apply") == Some(&json!(true)) {
                    return self.proc_apply(caller, &run_id, params.get("session_id").and_then(Value::as_str));
                }
            }
            Err(e) => self.set_run(&run_id, "failed", Some(&json!({ "error": e.to_json() })))?,
        }
        self.proc_get(caller, &run_id)
    }

    /// `proc.get`: run state plus what applying the candidate would do right now.
    pub fn proc_get(&mut self, caller: &Caller, run_id: &str) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let mut run = self.run_json(run_id)?;
        if run["principal"] != json!(caller.principal) {
            return Err(WsError::not_found("run not found"));
        }
        if run["state"] == json!("waiting_confirmation") && run["candidate"].is_object() {
            run["prepare"] = self.prepare(&run["candidate"].clone(), caller);
        }
        run["ok"] = json!(true);
        Ok(run)
    }

    /// `proc.apply`: the candidate becomes one Commit, submitted as the caller
    /// (its session must hold any required write locks).
    pub fn proc_apply(&mut self, caller: &Caller, run_id: &str, session_id: Option<&str>) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let run = self.run_json(run_id)?;
        if run["principal"] != json!(caller.principal) {
            return Err(WsError::not_found("run not found"));
        }
        match run["state"].as_str() {
            Some("waiting_confirmation") => {}
            Some("cancelled") => return Err(WsError::new(Code::RunCancelled, "the run was cancelled")),
            Some("succeeded") => {
                let mut v = run.clone();
                v["ok"] = json!(true);
                return Ok(v);
            }
            s => return Err(WsError::invalid_op(format!("run is {}", s.unwrap_or("?")))),
        }
        let mut candidate = run["candidate"].clone();
        if let Some(s) = session_id {
            candidate["session_id"] = json!(s);
        }
        self.set_run(run_id, "applying", None)?;
        let result = self.commit(&candidate, caller, &CommitOpts::default());
        let state = if result["status"] == json!("accepted") { "succeeded" } else { "failed" };
        self.set_run(run_id, state, Some(&result))?;
        let mut v = self.run_json(run_id)?;
        v["ok"] = json!(true);
        Ok(v)
    }

    /// `proc.cancel`. A run that already applied reports that truthfully.
    pub fn proc_cancel(&mut self, caller: &Caller, run_id: &str) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let run = self.run_json(run_id)?;
        if run["principal"] != json!(caller.principal) {
            return Err(WsError::not_found("run not found"));
        }
        if run["state"] == json!("succeeded") {
            return Ok(json!({ "ok": true, "state": "succeeded", "already_applied": true, "commit_id": run["result"]["commit_id"] }));
        }
        self.set_run(run_id, "cancelled", None)?;
        Ok(json!({ "ok": true, "state": "cancelled", "already_applied": false }))
    }

    /// `mock.task-summary@1`: fixed input → fixed output. No clock (today comes
    /// from `params.today`), no randomness.
    fn mock_task_summary(&self, caller: &Caller, run_id: &str, params: &Value) -> WsResult<(Value, Value, Value)> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let p = |k: &str, d: &str| params.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
        // the summary is data (under `data`), its Block goes on a Surface (phase two §4)
        let (source_id, summary_id, parent_id) = (p("source_id", "tasks"), p("summary_id", "summary"), p("parent_id", DATA_ID));
        let (cell_id, surface_id) = (p("cell_id", "cell-summary"), p("surface_id", "surface-main"));
        let today = parse_date(&p("today", "")).ok_or_else(|| WsError::invalid_op("params.today must be YYYY-MM-DD"))?;
        let today_days = days_from_civil(today.0, today.1, today.2);
        let source = readable_entity(&ctx, &access, &source_id)?;
        if !source.alive() || source.type_id != TYPE_TABLE {
            return Err(WsError::invalid_op("source_id must be a live table"));
        }
        let field = |id: &str| -> WsResult<FieldRow> {
            ctx.field(&source_id, id)?.filter(|f| f.alive()).ok_or_else(|| WsError::invalid_op(format!("the table needs a {id} field")))
        };
        let (status, due, risk, title) = (field("status")?, field("due")?, field("risk")?, field("title")?);
        for o in ["option-high", "option-medium", "option-low"] {
            if !risk.def.has_option(o) {
                return Err(WsError::invalid_op(format!("risk field lacks option {o}")));
            }
        }
        // read set: field types, the membership-deciding column and set, every `due` actually read
        let mut pre = vec![
            json!({ "target": { "entity_id": source_id, "selector": { "kind": "table_field_type", "field_id": "status" } }, "expect": { "rev": status.type_rev } }),
            json!({ "target": { "entity_id": source_id, "selector": { "kind": "table_field_type", "field_id": "due" } }, "expect": { "rev": due.type_rev } }),
            json!({ "target": { "entity_id": source_id, "selector": { "kind": "table_field_type", "field_id": "risk" } }, "expect": { "rev": risk.type_rev } }),
            json!({ "target": { "entity_id": source_id, "selector": { "kind": "table_field_values", "field_id": "status" } }, "expect": { "rev": status.values_rev } }),
            json!({ "target": { "entity_id": source_id, "selector": { "kind": "table_members" } }, "expect": { "rev": source.key_rev(MEMBERS_KEY) } }),
        ];
        let (mut values, mut warnings, mut high_titles) = (Vec::new(), Vec::new(), Vec::new());
        let (mut total, mut open, mut overdue) = (0u64, 0u64, 0u64);
        ctx.scan_records(&source_id, &mut |r| {
            if !r.alive() {
                return Ok(true);
            }
            total += 1;
            if r.values.get("status").and_then(Value::as_str) == Some("option-done") {
                return Ok(true);
            }
            open += 1;
            pre.push(json!({ "target": { "entity_id": source_id, "selector": { "kind": "table_cell", "record_id": r.record_id, "field_id": "due" } },
                             "expect": { "rev": r.value_rev("due") } }));
            let Some((y, m, d)) = r.values.get("due").and_then(Value::as_str).and_then(parse_date) else { return Ok(true) };
            let left = days_from_civil(y, m, d) - today_days;
            let level = if left < 0 { "option-high" } else if left <= 7 { "option-medium" } else { "option-low" };
            if level == "option-high" {
                overdue += 1;
                high_titles.push(r.values.get(&title.field_id).and_then(Value::as_str).unwrap_or(&r.record_id).to_string());
            }
            if r.meta.get("risk").is_some_and(|m| m["manual_override"] == json!(true)) {
                // a human overrode this derived value: never overwrite it silently
                warnings.push(json!({ "code": "MANUAL_OVERRIDE_SKIPPED", "record_id": r.record_id, "field_id": "risk" }));
                return Ok(true);
            }
            if r.values.get("risk").and_then(Value::as_str) != Some(level) {
                values.push(json!({ "record_id": r.record_id, "field_id": "risk", "value": level, "expect": { "rev": r.value_rev("risk") } }));
            }
            Ok(true)
        })?;
        let inputs = json!([{ "entity_id": source_id, "content_rev": source.content_rev }]);
        let mut ops = Vec::new();
        if !values.is_empty() {
            ops.push(json!({ "op": "table.set_values", "source_id": source_id, "field_type_revs": { "risk": risk.type_rev },
                             "values": values, "derived": { "program": MOCK_PROGRAM, "inputs": inputs } }));
        }
        let blocks = [
            json!({ "type": "heading", "attrs": { "block_id": "sum-title", "level": 2 }, "content": [{ "type": "text", "text": "任务摘要（模拟生成）" }] }),
            para("sum-stats", &format!("共 {total} 项任务，未完成 {open} 项，逾期 {overdue} 项。")),
            para("sum-risk", &if high_titles.is_empty() { "没有逾期任务。".to_string() } else { format!("逾期：{}", high_titles.join("、")) }),
        ];
        match ctx.entity(&summary_id)?.filter(|e| e.alive()) {
            None => {
                ops.push(json!({ "op": "entity.create", "entity_id": summary_id, "type_id": TYPE_RICHTEXT, "parent_id": parent_id,
                                 "order_key": p("summary_order_key", "y"), "name": "任务摘要",
                                 "payload": { "content": { "type": "doc", "content": blocks } } }));
                if ctx.entity(&cell_id)?.is_none() {
                    ops.push(json!({ "op": "entity.create", "entity_id": cell_id, "type_id": TYPE_CELL, "parent_id": surface_id,
                                     "order_key": p("cell_order_key", "z"),
                                     "payload": { "source_ref": { "entity_id": summary_id }, "view": { "type": "richtext" }, "title": "任务摘要" } }));
                }
            }
            Some(_) => {
                let rt = ctx.richtext(&summary_id)?.ok_or_else(|| WsError::invalid_op("summary_id is not a rich text"))?;
                let current = aiworkspace_core::richtext::index_blocks(&rt.meta.ast);
                let mut after: Option<String> = None;
                for b in &blocks {
                    let id = b["attrs"]["block_id"].as_str().unwrap();
                    let canon = aiworkspace_core::richtext::canonicalize(b)?;
                    match (current.get(id), rt.meta.block_index.get(id)) {
                        (Some(loc), Some(info)) => {
                            if loc.node != canon {
                                ops.push(json!({ "op": "richtext.replace_block", "entity_id": summary_id, "block_id": id,
                                                 "expect": { "hash": info.hash }, "node": canon }));
                            }
                        }
                        _ => {
                            let position = match &after {
                                Some(a) => json!({ "after": a }),
                                None => json!({ "start": true }),
                            };
                            ops.push(json!({ "op": "richtext.insert_blocks", "entity_id": summary_id, "position": position, "blocks": [canon] }));
                        }
                    }
                    after = Some(id.to_string());
                }
            }
        }
        match params.get("inject").and_then(Value::as_str) {
            Some("unknown_option") => {
                // a well-formed write (current rev) of an illegal value
                let mut first: Option<(String, u64)> = None;
                ctx.scan_records(&source_id, &mut |r| {
                    if r.alive() {
                        first = Some((r.record_id.clone(), r.value_rev("risk")));
                    }
                    Ok(first.is_none())
                })?;
                let (rid, rev) = first.ok_or_else(|| WsError::invalid_op("no records"))?;
                ops = vec![json!({ "op": "table.set_values", "source_id": source_id,
                    "values": [{ "record_id": rid, "field_id": "risk", "value": "option-nope", "expect": { "rev": rev } }] })];
            }
            Some("dangling_ref") => ops.push(json!({ "op": "richtext.insert_blocks", "entity_id": summary_id, "position": { "end": true },
                "blocks": [{ "type": "object_embed", "attrs": { "block_id": "sum-bad", "ref": { "entity_id": "no-such-cell" } } }] })),
            Some("over_limit") => {
                let n = self.limits.max_ops + 1;
                ops = (0..n).map(|i| json!({ "op": "entity.rename", "entity_id": format!("x{i}"), "name": "x", "expect": { "rev": 0 } })).collect();
            }
            _ => {}
        }
        if ops.is_empty() {
            return Err(WsError::invalid_op("nothing to change: the derived values and the summary are up to date"));
        }
        let candidate = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch,
                                "idempotency_key": format!("run/{run_id}"), "origin": "program", "run_id": run_id,
                                "message": format!("{MOCK_PROGRAM}（模拟结果）"), "preconditions": pre, "operations": ops });
        Ok((candidate, inputs, json!(warnings)))
    }
}
