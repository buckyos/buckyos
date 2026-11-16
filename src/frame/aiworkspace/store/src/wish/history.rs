//! The run records of a wish (许愿格交互升级 §2.2, §3.4): `payload.history`, written by the service only — in the Commit
//! that ends a stage, in a Commit of the record alone when a run fails (D7), and by a rollback. A record keeps small
//! fields; what is large (the request, the task contract, the macro, its log) is a content object of the Workspace the
//! record names, kept, exported and cloned with the wish. Summary text is the reader's: a record keeps its materials.

use super::plan::Plan;
use crate::docdb::db_err;
use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::canonical::canonical_json;
use aiworkspace_core::materialize::ObjectSource;
use aiworkspace_core::model::*;
use aiworkspace_core::value::parse_datetime_ms;
use aiworkspace_core::wish::{HISTORY_LOG_MAX_BYTES, HISTORY_PER_STAGE, MAX_HISTORY_BYTES, STAGES, STAGE_ANALYZE, STAGE_CODE, STAGE_RUN};
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const MAX_WARNINGS: usize = 16;
const MAX_WARNING_CHARS: usize = 500;
const MAX_CHECK_TEXT: usize = 300;
const MAX_SUMMARY_CHARS: usize = 2000;

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

/// The head of a macro's log a record keeps (at most [`HISTORY_LOG_MAX_BYTES`], on a character boundary).
pub fn log_head(log: &str) -> &str {
    if log.len() <= HISTORY_LOG_MAX_BYTES {
        return log;
    }
    let mut end = HISTORY_LOG_MAX_BYTES;
    while !log.is_char_boundary(end) {
        end -= 1;
    }
    &log[..end]
}

/// What a run is called in its record: a coding with a parent is a repair.
pub fn action_of(run: &Value) -> &'static str {
    match run["params"]["stage"].as_str() {
        Some(STAGE_ANALYZE) => "analyze",
        Some(STAGE_CODE) if run["params"]["parent_run_id"].is_string() => "repair",
        Some(STAGE_CODE) => "code",
        _ => "run",
    }
}

/// The record of `run` ending now: its identity and timing, who ran it, with whose AI.
pub fn base_record(run: &Value, now: &str, ok: bool) -> Value {
    let at = run["created_at"].as_str().unwrap_or(now);
    let ms = |t: &str| parse_datetime_ms(t).unwrap_or(0);
    let mut r = json!({
        "id": run["run_id"], "stage": run["params"]["stage"], "action": action_of(run), "ok": ok, "at": at, "ended_at": now,
        "duration_ms": (ms(now) - ms(at)).max(0), "by": run["principal"],
        "warnings": [], "summary": {}, "basis": {}, "inputs": [], "outputs": {}, "objects": {},
    });
    let aicc = &run["detail"]["aicc"];
    if aicc.is_object() {
        r["aicc"] = json!({ "mode": aicc["mode"], "user_id": aicc["user_id"] });
    }
    r
}

/// What the wish's configuration a run rested on is: the analysis it followed, the macro and the coding that wrote it.
pub fn basis_of(payload: &JsonMap, stage: &str) -> Value {
    let mut b = Map::new();
    if stage != STAGE_ANALYZE {
        if let Some(id) = payload.get("analysis").and_then(|a| a.get("run_id")) {
            b.insert("analysis".into(), id.clone());
        }
    }
    if stage == STAGE_RUN {
        if let Some(p) = payload.get("program") {
            b.insert("program".into(), p["digest"].clone());
            if let Some(id) = p.get("run_id") {
                b.insert("program_record".into(), id.clone());
            }
        }
    }
    Value::Object(b)
}

/// `usage` of a record (§2.9): what the executor reported for a model stage; for running, the macro's own AI calls —
/// none when it made none.
pub fn record_usage(stage: &str, reported: &Value, ai: &Value) -> Option<Value> {
    if stage != STAGE_RUN {
        let o = reported.as_object()?;
        let u: Map<String, Value> = ["model", "input_tokens", "output_tokens", "calls"].iter().filter_map(|k| o.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect();
        return (!u.is_empty()).then_some(Value::Object(u));
    }
    let n = |kind: &str, k: &str| ai[kind][k].as_u64().unwrap_or(0);
    let calls = ["complete", "map", "image", "video"].iter().map(|k| n(k, "calls")).sum::<u64>();
    if calls == 0 {
        return None;
    }
    let mut models: Vec<String> = Vec::new();
    for m in ai["models"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !models.iter().any(|x| x == m) {
            models.push(m.to_string());
        }
    }
    let mut u = json!({ "calls": calls, "input_tokens": n("complete", "input_tokens") + n("map", "input_tokens"), "output_tokens": n("complete", "tokens") + n("map", "tokens") });
    if !models.is_empty() {
        u["model"] = json!(models.join(", "));
    }
    Some(u)
}

/// The checks as a record keeps them (their level from the contract).
pub fn record_checks(checks: &[Value]) -> Vec<Value> {
    checks
        .iter()
        .map(|c| {
            let mut v = json!({ "id": c["id"], "kind": c["kind"], "status": c["status"], "text": clip(&text_of(&c["text"]), MAX_CHECK_TEXT) });
            if let Some(l) = c.get("level").filter(|l| l.is_string()) {
                v["level"] = l.clone();
            }
            v
        })
        .collect()
}

fn text_of(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_else(|| if v.is_null() { String::new() } else { v.to_string() })
}

/// The level a check is judged at (D2): the contract's; a program check the contract does not define is a warning.
pub fn check_level(c: &Value) -> &str {
    c["level"].as_str().unwrap_or("warning")
}

/// Program checks of error level that failed: they fail a run.
pub fn failed_errors(checks: &[Value]) -> Vec<Value> {
    checks.iter().filter(|c| c["kind"] == json!("program") && c["status"] == json!("failed") && check_level(c) == "error").cloned().collect()
}

/// A warning the service gives (README "Run records"): a code and its materials — the reader's client words it in the
/// reader's language. What a model or an executor wrote stays its own text.
pub fn warning(code: &str, params: Value) -> Value {
    let mut w = json!({ "code": code });
    for (k, v) in params.as_object().into_iter().flatten() {
        w[k] = v.clone();
    }
    w
}

/// Warnings the checks give (in front of a record's warnings): a failed warning-level check (`check_failed`), an
/// error-level one that did not run (`check_not_run`).
pub fn check_warnings(checks: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for c in checks.iter().filter(|c| c["kind"] == json!("program")) {
        let params = json!({ "id": c["id"], "text": clip(&text_of(&c["text"]), MAX_CHECK_TEXT) });
        match (c["status"].as_str(), check_level(c)) {
            (Some("failed"), "warning") => out.push(warning("check_failed", params)),
            (Some("not_run"), "error") => out.push(warning("check_not_run", params)),
            _ => {}
        }
    }
    out
}

/// Numbers of a text result its macro did not compute (`uncited_numbers`), as warnings.
pub fn uncited_warnings(uncited: &Value) -> Vec<Value> {
    uncited.as_array().into_iter().flatten().map(|u| warning("uncited_numbers", json!({ "name": u["result"], "numbers": u["numbers"] }))).collect()
}

fn strings(v: &Value) -> Vec<Value> {
    v.as_array().into_iter().flatten().map(text_of).filter(|s| !s.trim().is_empty()).map(Value::from).collect()
}

/// A record within its bounds: warnings (free text clipped; the service's own are short), summary text.
fn bounded(mut r: Value) -> Value {
    let w: Vec<Value> = r["warnings"].as_array().cloned().unwrap_or_default().iter().take(MAX_WARNINGS).map(|x| if x.is_object() { x.clone() } else { json!(clip(&text_of(x), MAX_WARNING_CHARS)) }).collect();
    r["warnings"] = json!(w);
    if let Some(t) = r["summary"]["text"].as_str().map(|t| clip(t, MAX_SUMMARY_CHARS)) {
        r["summary"]["text"] = json!(t);
    }
    r
}

/// The record of a run's error: its codes, why, and the inputs an analysis found missing.
fn record_error(e: &Value) -> Value {
    let mut out = json!({ "code": e["code"].as_str().unwrap_or("INTERNAL"), "detail": clip(&text_of(&e["detail"]), MAX_SUMMARY_CHARS) });
    if let Some(s) = e.get("sub_code").filter(|s| s.is_string()) {
        out["sub_code"] = s.clone();
    }
    let missing: Vec<Value> = e["data"]["blockers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| {
            let mut m = json!({ "message": b["message"] });
            if let Some(l) = b.get("input_label").filter(|l| l.is_string()) {
                m["label"] = l.clone();
            }
            if let Some(c) = b.get("candidates").filter(|c| c.is_array()) {
                m["candidates"] = c.clone();
            }
            if let Some(w) = b.get("wanted").filter(|w| w.is_object()) {
                m["wanted"] = w.clone();
            }
            m
        })
        .collect();
    if !missing.is_empty() {
        out["missing_inputs"] = json!(missing);
    }
    // a coding that stopped without finish: why its last trial failed, and what the model said
    for k in ["last_trial_error", "said"] {
        if let Some(t) = e["data"][k].as_str().filter(|t| !t.is_empty()) {
            out[k] = json!(clip(t, MAX_SUMMARY_CHARS));
        }
    }
    out
}

/// The current record of a stage's list: the newest successful one not rolled back (rollbacks are no state).
pub fn current_record(list: &[Value]) -> Option<&Value> {
    list.iter().find(|r| r["ok"] == json!(true) && r.get("rolled_back").is_none_or(Value::is_null) && r["action"] != json!("rollback"))
}

/// `history` with `record` in front of its stage's list, at most [`HISTORY_PER_STAGE`] each and [`MAX_HISTORY_BYTES`] in
/// all (the oldest go first); a successful run counts towards `results_count`, which never goes down.
pub fn with_record(history: Option<&Value>, record: &Value) -> Value {
    let mut h = history.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    for s in STAGES {
        if !h[*s].is_array() {
            h[*s] = json!([]);
        }
    }
    let stage = record["stage"].as_str().filter(|s| STAGES.contains(s)).unwrap_or(STAGE_RUN);
    let list = h[stage].as_array_mut().unwrap();
    list.insert(0, record.clone());
    list.truncate(HISTORY_PER_STAGE);
    let mut count = h["results_count"].as_u64().unwrap_or(0);
    if stage == STAGE_RUN && record["ok"] == json!(true) && record["action"] == json!("run") {
        count += 1;
    }
    h["results_count"] = json!(count);
    while canonical_json(&h).map(|s| s.len()).unwrap_or(0) > MAX_HISTORY_BYTES {
        let longest = STAGES.iter().copied().max_by_key(|s| h[*s].as_array().map_or(0, Vec::len)).unwrap_or(STAGE_RUN);
        let list = h[longest].as_array_mut().unwrap();
        if list.len() <= 1 {
            break;
        }
        list.pop();
    }
    h
}

/// A rollback that would discard later edits names them (only those the caller may read; the rest are counted).
struct Lost {
    items: Vec<Value>,
    hidden: usize,
}

impl Workspace {
    /// The record of a stage that ended well (§3.3), from what it produced (`candidate`) and what its Commit writes
    /// (`plan`). Its objects and Commit are added by the caller.
    pub fn wish_stage_record(&self, run: &Value, candidate: &Value, evidence: &Value, plan: &Plan, payload: &JsonMap) -> WsResult<Value> {
        let now = self.now();
        let stage = run["params"]["stage"].as_str().unwrap_or(STAGE_RUN);
        let mut r = base_record(run, &now, true);
        r["basis"] = basis_of(payload, stage);
        let checks: Vec<Value> = candidate["checks"].as_array().cloned().unwrap_or_default();
        let mut warnings: Vec<Value> = Vec::new();
        match stage {
            STAGE_ANALYZE => {
                let a = &candidate["analysis"];
                let (results, checks) = (a["output_contract"]["results"].as_array().map_or(0, Vec::len), a["checks"].as_array().map_or(0, Vec::len));
                r["summary"] = json!({ "results": results, "checks": checks });
                if let Some(t) = a["summary"].as_str() {
                    r["summary"]["text"] = json!(t);
                }
                r["outputs"] = json!({ "analysis": { "results": results, "checks": checks } });
                warnings.extend(strings(&a["warnings"]));
                r["inputs"] = json!(self.record_inputs(candidate["inputs"].as_array().map(Vec::as_slice).unwrap_or(&[]))?);
            }
            STAGE_CODE => {
                let program: Vec<&Value> = checks.iter().filter(|c| c["kind"] == json!("program")).collect();
                r["summary"] = json!({ "trial": { "results": candidate["results"].as_array().map_or(0, Vec::len),
                    "checks_passed": program.iter().filter(|c| c["status"] == json!("passed")).count(), "checks_total": program.len() } });
                if let Some(t) = candidate["summary"].as_str().filter(|t| !t.trim().is_empty()) {
                    r["summary"]["text"] = json!(t);
                }
                r["checks"] = json!(record_checks(&checks));
                warnings.extend(check_warnings(&checks));
                warnings.extend(uncited_warnings(&candidate["uncited_numbers"]));
                warnings.extend(strings(&candidate["warnings"]));
                let prog = &candidate["program"];
                r["outputs"] = json!({ "program": { "digest": prog["digest"], "ai": prog["manifest"]["ai"].is_object() } });
                r["inputs"] = json!(self.record_inputs(evidence["inputs"].as_array().map(Vec::as_slice).unwrap_or(&[]))?);
            }
            _ => {
                let bindings = plan.operations.iter().rev().find(|o| o["op"] == json!("entity.set_keys") && o["entity_id"] == run["params"]["wish_id"])
                    .and_then(|o| o["keys"].as_array()?.iter().find(|k| k["key"] == json!("last_run")).map(|k| k["value"]["result_bindings"]["results"].clone()))
                    .unwrap_or(Value::Null);
                let mut results = Vec::new();
                let (mut created, mut updated) = (0, 0);
                for s in plan.summary["results"].as_array().into_iter().flatten() {
                    let name = s["name"].as_str().unwrap_or("");
                    let action = if s["action"] == json!("create") { "create" } else { "update" };
                    if action == "create" { created += 1 } else { updated += 1 }
                    let mut o = json!({ "name": name, "type": s["type"], "entity_id": s["entity_id"], "action": action });
                    if let Some(t) = s.get("title").filter(|t| t.is_string()) {
                        o["title"] = t.clone();
                    }
                    if let Some(c) = bindings[name].get("cells").filter(|c| c.as_array().is_some_and(|a| !a.is_empty())) {
                        o["cells"] = c.clone();
                    }
                    results.push(o);
                }
                r["summary"] = json!({ "created": created, "updated": updated });
                r["outputs"] = json!({ "results": results });
                r["checks"] = json!(record_checks(&checks));
                warnings.extend(check_warnings(&checks));
                warnings.extend(plan.warnings.iter().cloned());
                warnings.extend(uncited_warnings(&candidate["uncited_numbers"]));
                r["inputs"] = json!(self.record_inputs(evidence["inputs"].as_array().map(Vec::as_slice).unwrap_or(&[]))?);
            }
        }
        if let Some(u) = record_usage(stage, &run["detail"]["usage"], &candidate["ai"]) {
            r["usage"] = u;
        }
        r["warnings"] = json!(warnings);
        Ok(bounded(r))
    }

    /// Inputs as a record lists them: name, label and the version of the entity as the run's Commit finds it.
    fn record_inputs(&self, inputs: &[Value]) -> WsResult<Vec<Value>> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for i in inputs {
            let Some(id) = i["entity_id"].as_str() else { continue };
            let mut v = json!({ "entity_id": id });
            for k in ["name", "label"] {
                if let Some(x) = i.get(k).filter(|x| x.is_string()) {
                    v[k] = x.clone();
                }
            }
            if let Some(e) = ctx.entity(id)? {
                v["version"] = json!({ "rev": e.content_rev });
            }
            out.push(v);
        }
        Ok(out)
    }

    /// Stage the large parts of a record as content objects of this Workspace (as its owner: the service writes them):
    /// `{ name: object_id }`.
    pub fn wish_stage_objects(&mut self, parts: Vec<(&str, Vec<u8>)>) -> WsResult<Value> {
        let owner = Caller::internal(&self.owner()?.unwrap_or_default());
        let mut out = Map::new();
        for (name, bytes) in parts {
            let staged = self.stage_asset(&owner, &bytes)?;
            out.insert(name.to_string(), staged["object_id"].clone());
        }
        Ok(Value::Object(out))
    }

    /// A run ends failed (`detail` merged with its error), and its record goes into the wish's history in a Commit of
    /// the record alone (D7). Recording is best effort: a run that cannot be recorded (its wish is gone) still fails.
    pub fn wish_run_fail(&mut self, run_id: &str, error: &Value, detail: Option<&Value>) -> WsResult<()> {
        let mut d = detail.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        d["error"] = error.clone();
        if !d["progress"].is_object() {
            d["progress"] = json!({});
        }
        d["progress"]["phase"] = json!("failed");
        self.wish_run_set(run_id, Some("failed"), None, None, None, Some(&d))?;
        match self.wish_record_failure(run_id) {
            Ok(Some(record)) => self.wish_run_set(run_id, None, None, None, None, Some(&json!({ "record": record })))?,
            Ok(None) => {}
            Err(e) => log::warn!("the failed run {run_id} of {} was not recorded: {e}", self.workspace_id),
        }
        Ok(())
    }

    fn wish_record_failure(&mut self, run_id: &str) -> WsResult<Option<Value>> {
        let run = self.wish_row(run_id)?;
        let wish_id = run["params"]["wish_id"].as_str().unwrap_or("").to_string();
        let now = self.now();
        let Some(wish) = self.ctx(&now).entity(&wish_id)?.filter(|e| e.alive() && e.type_id == TYPE_WISH) else { return Ok(None) };
        let stage = run["params"]["stage"].as_str().unwrap_or(STAGE_RUN).to_string();
        let error = &run["detail"]["error"];
        let mut r = base_record(&run, &now, false);
        r["error"] = record_error(error);
        r["basis"] = basis_of(&wish.payload, &stage);
        let read: Vec<Value> = run["evidence"]["inputs"].as_array().cloned().unwrap_or_default();
        r["inputs"] = json!(self.record_inputs(&read)?);
        let checks: Vec<Value> = error["data"]["checks"].as_array().cloned().unwrap_or_default();
        if !checks.is_empty() {
            r["checks"] = json!(record_checks(&checks));
            r["warnings"] = json!(check_warnings(&checks));
        }
        if let Some(u) = record_usage(&stage, &run["detail"]["usage"], &error["data"]["ai"]) {
            r["usage"] = u;
        }
        let mut parts: Vec<(&str, Vec<u8>)> = Vec::new();
        if stage != STAGE_RUN {
            if let Some(p) = run["detail"]["brief"]["prompt"].as_str() {
                parts.push(("prompt", p.as_bytes().to_vec()));
            }
        }
        let log = run["detail"]["program_log"].as_str().or_else(|| error["data"]["program_log"].as_str()).unwrap_or("");
        if stage != STAGE_ANALYZE && !log.is_empty() {
            parts.push(("log", log_head(log).as_bytes().to_vec()));
        }
        match self.wish_stage_objects(parts) {
            Ok(o) => r["objects"] = o,
            Err(e) => log::warn!("the objects of the failed run {run_id} were not kept: {e}"),
        }
        let r = bounded(r);
        let history = with_record(wish.payload.get("history"), &r);
        let ops = json!([{ "op": "entity.set_keys", "entity_id": wish_id, "keys": [{ "key": "history", "value": history, "expect": { "rev": wish.key_rev("history") } }] }]);
        let owner = Caller::internal(&self.owner()?.unwrap_or_default());
        let opts = CommitOpts { internal: true, as_principal: run["principal"].as_str().map(str::to_string), ..CommitOpts::default() };
        let result = self.commit(&self.request(&format!("run/{run_id}/record"), "Record a failed wish run", ops, None), &owner, &opts);
        if result["status"] != json!("accepted") {
            return Err(WsError::invalid_op(format!("the record was not written: {result}")));
        }
        Ok(Some(r))
    }

    fn request(&self, key: &str, message: &str, ops: Value, session_id: Option<&str>) -> Value {
        let mut req = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch,
                              "idempotency_key": key, "message": message, "operations": ops });
        if let Some(s) = session_id {
            req["session_id"] = json!(s);
        }
        req
    }

    /// The wish whose history holds the record `run_id`: the run's own (this deployment ran it), else the wish whose
    /// records name it (a cloned or imported Workspace keeps the records, not the runs).
    fn wish_of_record(&self, run_id: &str) -> WsResult<String> {
        if let Some(w) = self.local.query_row("SELECT json_extract(params_json, '$.wish_id') FROM runs WHERE run_id = ?1", [run_id], |r| r.get::<_, Option<String>>(0)).optional().map_err(db_err)?.flatten() {
            return Ok(w);
        }
        let mut st = self.doc.prepare("SELECT entity_id FROM entities WHERE type_id = ?1 AND deleted_seq IS NULL AND payload_json LIKE ?2").map_err(db_err)?;
        let ids: Vec<String> = st.query_map(params![TYPE_WISH, format!("%{run_id}%")], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        ids.into_iter().next().ok_or_else(|| WsError::not_found("run not found"))
    }

    /// `proc.rollback { run_id, confirm? }` (§2.1, §2.3): the stage's current record is undone in one Commit — a run's
    /// writes by the inverse of its Commit, a macro or an analysis back to the previous record's — and a `rollback`
    /// record goes in front, the rolled back one marked. Later edits the rollback would discard are named first
    /// (`needs_confirm`) and discarded only with `confirm`.
    pub fn wish_rollback(&mut self, caller: &Caller, run_id: &str, confirm: bool, session_id: Option<&str>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let wish_id = self.wish_of_record(run_id)?;
        let now = self.now();
        let wish = {
            let ctx = self.ctx(&now);
            let w = aiworkspace_core::read::readable_entity(&ctx, &access, &wish_id)?;
            if !access.can(&ctx, &wish_id, Cap::Update)? {
                return Err(WsError::denied("rolling a wish back needs update on it"));
            }
            w
        };
        let history = wish.payload.get("history").cloned().unwrap_or(Value::Null);
        let (stage, record) = STAGES
            .iter()
            .find_map(|s| history[*s].as_array().and_then(|l| l.iter().find(|r| r["id"] == json!(run_id))).map(|r| (*s, r.clone())))
            .ok_or_else(|| WsError::not_found("run not found"))?;
        let list: Vec<Value> = history[stage].as_array().cloned().unwrap_or_default();
        if current_record(&list).map(|r| &r["id"]) != Some(&json!(run_id)) {
            return Err(WsError::sub(Code::InvalidOperation, "NOT_CURRENT_RECORD", "only the stage's current record can be rolled back: the newest successful one not rolled back yet"));
        }
        let (mut ops, lost) = match stage {
            STAGE_RUN => self.run_inverse(&access, &wish_id, &record, &history)?,
            _ => self.config_rollback(&access, &wish, stage, &record, &list)?,
        };
        if (!lost.items.is_empty() || lost.hidden > 0) && !confirm {
            return Ok(json!({ "ok": true, "status": "needs_confirm", "lost": lost.items, "hidden_lost": lost.hidden }));
        }
        let rb_id = random_id("rb_");
        let mut rb = json!({ "id": rb_id, "stage": stage, "action": "rollback", "ok": true, "at": now, "ended_at": now, "duration_ms": 0, "by": caller.principal,
                             "warnings": [], "summary": { "of": run_id }, "basis": {}, "inputs": [], "outputs": {}, "objects": {}, "rollback_of": run_id });
        if !lost.items.is_empty() || lost.hidden > 0 {
            rb["warnings"] = json!([warning("discarded_edits", json!({ "count": lost.items.len() + lost.hidden }))]);
        }
        let mut marked = history.clone();
        for r in marked[stage].as_array_mut().into_iter().flatten() {
            if r["id"] == json!(run_id) {
                r["rolled_back"] = json!({ "id": rb_id, "by": caller.principal, "at": now });
            }
        }
        let commit_id = random_id("cm_");
        rb["commit_id"] = json!(commit_id);
        let new_history = with_record(Some(&marked), &rb);
        ops.push(json!({ "op": "entity.set_keys", "entity_id": wish_id, "keys": [{ "key": "history", "value": new_history, "expect": "any" }] }));
        let gone = self.reclaimed_objects(&ops)?;
        if !gone.is_empty() {
            return Err(WsError::new(Code::ResourceReclaimed, "the rollback needs resources reclaimed after their retention period").with_data(json!({ "object_ids": gone })));
        }
        let opts = CommitOpts { internal: true, commit_id: Some(commit_id), ..CommitOpts::default() };
        let req = self.request(&format!("rollback/{rb_id}"), &format!("Roll back wish run {run_id}"), json!(ops), session_id);
        let result = self.commit(&req, caller, &opts);
        if result["status"] != json!("accepted") {
            return Err(commit_error(&result, "the rollback was not written"));
        }
        Ok(json!({ "ok": true, "status": "accepted", "commit": result, "record": rb }))
    }

    /// A run's writes undone: the inverse of its Commit without its `history` part, every expectation `any` (the
    /// later edits were named and confirmed). Later edits are the Commits after it touching what it wrote, but for the
    /// runs rolled back since and rollbacks themselves.
    fn run_inverse(&self, access: &aiworkspace_core::access::Access, wish_id: &str, record: &Value, history: &Value) -> WsResult<(Vec<Value>, Lost)> {
        let not_here = || WsError::new(Code::NotUndoable, "this run's Commit is not in this Workspace's history (a cloned or imported Workspace keeps the records, not the Commits)");
        let commit_id = record["commit_id"].as_str().ok_or_else(not_here)?;
        let seq: u64 = self.doc.query_row("SELECT seq FROM commits WHERE commit_id = ?1", [commit_id], |r| r.get(0)).optional().map_err(db_err)?.ok_or_else(not_here)?;
        let mut st = self.doc.prepare("SELECT inverse_json, touched_json FROM commit_ops WHERE seq = ?1 ORDER BY op_index DESC").map_err(db_err)?;
        let rows: Vec<(Option<String>, String)> = st.query_map([seq], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        let mut ops: Vec<Value> = Vec::new();
        let mut touched: BTreeSet<String> = BTreeSet::new();
        for (inv, t) in rows {
            let inv = inv.ok_or_else(|| WsError::new(Code::NotUndoable, "the run's Commit holds an operation that cannot be undone"))?;
            for mut op in serde_json::from_str::<Vec<Value>>(&inv).map_err(|e| WsError::io(e.to_string()))? {
                if op["entity_id"] == json!(wish_id) {
                    if let Some(keys) = op["keys"].as_array_mut() {
                        keys.retain(|k| k["key"] != json!("history") && k != &json!("history"));
                        if keys.is_empty() {
                            continue;
                        }
                    }
                }
                any_expect(&mut op);
                ops.push(op);
            }
            for x in serde_json::from_str::<Vec<Value>>(&t).unwrap_or_default() {
                if let Some(id) = x["entity_id"].as_str().filter(|id| *id != wish_id) {
                    touched.insert(id.to_string());
                }
            }
        }
        // what the run created goes (a Block's data with it): what was done to it afterwards in the run is moot
        let created: BTreeSet<String> = ops.iter().filter(|o| o["op"] == json!("entity.delete")).filter_map(|o| o["entity_id"].as_str().map(str::to_string)).collect();
        ops.retain(|o| o["op"] == json!("entity.delete") || !o.get("entity_id").or_else(|| o.get("source_id")).and_then(Value::as_str).is_some_and(|id| created.contains(id)));
        let rolled: BTreeSet<String> = STAGES.iter().flat_map(|s| history[*s].as_array().cloned().unwrap_or_default()).filter(|r| r.get("rolled_back").is_some_and(Value::is_object)).filter_map(|r| r["id"].as_str().map(str::to_string)).collect();
        let mut st = self.doc.prepare("SELECT seq, run_id, idem_key FROM commits WHERE seq > ?1 ORDER BY seq").map_err(db_err)?;
        let later: Vec<(u64, Option<String>, String)> = st.query_map([seq], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        let mut changed: BTreeSet<String> = BTreeSet::new();
        for (s, run, key) in later {
            if run.as_ref().is_some_and(|r| rolled.contains(r)) || key.starts_with("rollback/") {
                continue;
            }
            let mut st = self.doc.prepare_cached("SELECT touched_json FROM commit_ops WHERE seq = ?1").map_err(db_err)?;
            for t in st.query_map([s], |r| r.get::<_, String>(0)).map_err(db_err)? {
                for x in serde_json::from_str::<Vec<Value>>(&t.map_err(db_err)?).unwrap_or_default() {
                    if let Some(id) = x["entity_id"].as_str().filter(|id| touched.contains(*id)) {
                        changed.insert(id.to_string());
                    }
                }
            }
        }
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut lost = Lost { items: Vec::new(), hidden: 0 };
        for id in changed {
            let e = ctx.entity(&id)?;
            match &e {
                Some(e) if access.can_read(&ctx, e)? => {
                    let mut item = json!({ "entity_id": id, "type_id": e.type_id, "what": if e.alive() { "edited" } else { "deleted" } });
                    if let Some(t) = e.payload.get("title").and_then(Value::as_str).or(e.name.as_deref()) {
                        item["title"] = json!(t);
                    }
                    lost.items.push(item);
                }
                _ => lost.hidden += 1,
            }
        }
        Ok((ops, lost))
    }

    /// A macro or an analysis back to the previous record's (the previous successful one not rolled back); without
    /// one, gone. Changed by hand since the record was written: a later edit to confirm.
    fn config_rollback(&self, access: &aiworkspace_core::access::Access, wish: &EntityRow, stage: &str, record: &Value, list: &[Value]) -> WsResult<(Vec<Value>, Lost)> {
        let pos = list.iter().position(|r| r["id"] == record["id"]).unwrap_or(0);
        let prev = current_record(&list[pos + 1..]);
        let (object, key) = if stage == STAGE_CODE { ("program", "program") } else { ("analysis", "analysis") };
        let read = |r: &Value| -> WsResult<Option<Value>> {
            match r["objects"][object].as_str() {
                Some(id) => Ok(Some(serde_json::from_slice(&ObjectSource::get_file(&*self.objects, id)?).map_err(|e| WsError::invalid_schema(format!("record object {id}: {e}")))?)),
                None => Ok(None),
            }
        };
        let mine = read(record)?;
        let before = match prev {
            Some(p) => Some(read(p)?.ok_or_else(|| WsError::new(Code::NotUndoable, format!("the previous record keeps no {object}")))?),
            None => None,
        };
        let now_value = wish.payload.get(key).cloned();
        let ours = mine.map(|m| if stage == STAGE_CODE { m } else { m["analysis"].clone() });
        let edited = ours.is_some() && now_value.as_ref() != ours.as_ref();
        let mut keys: Vec<Value> = Vec::new();
        let mut ops: Vec<Value> = Vec::new();
        match (&before, stage) {
            (Some(p), STAGE_CODE) => keys.push(json!({ "key": "program", "value": p, "expect": "any" })),
            (Some(p), _) => {
                keys.push(json!({ "key": "analysis", "value": p["analysis"], "expect": "any" }));
                if p["inputs"].is_array() {
                    keys.push(json!({ "key": "inputs", "value": p["inputs"], "expect": "any" }));
                }
            }
            (None, _) if now_value.is_some() => ops.push(json!({ "op": "entity.unset_keys", "entity_id": wish.entity_id, "keys": [{ "key": key, "expect": "any" }] })),
            (None, _) => {}
        }
        if !keys.is_empty() {
            ops.push(json!({ "op": "entity.set_keys", "entity_id": wish.entity_id, "keys": keys }));
        }
        let _ = access;
        let mut lost = Lost { items: Vec::new(), hidden: 0 };
        if edited {
            let mut item = json!({ "entity_id": wish.entity_id, "type_id": wish.type_id, "what": "edited" });
            if let Some(t) = wish.payload.get("title").and_then(Value::as_str).or(wish.name.as_deref()) {
                item["title"] = json!(t);
            }
            lost.items.push(item);
        }
        Ok((ops, lost))
    }
}

/// Every expectation of an inverse operation (its own and its items') becomes `any`.
fn any_expect(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                if k == "expect" && x.is_object() {
                    *x = json!("any");
                } else {
                    any_expect(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(any_expect),
        _ => {}
    }
}

/// A Commit that was not accepted, as the error of what it was for.
pub fn commit_error(result: &Value, what: &str) -> WsError {
    let code = result["code"].as_str().and_then(Code::parse).unwrap_or(Code::InvalidOperation);
    let detail = result["errors"][0]["detail"].as_str().or(result["detail"].as_str()).unwrap_or("");
    let sub = if result["status"] == json!("conflict") { "COMMIT_CONFLICT" } else { "COMMIT_REJECTED" };
    WsError::sub(code, sub, format!("{what} ({}): {detail}", result["code"].as_str().unwrap_or("?"))).with_data(json!({ "commit": result }))
}
