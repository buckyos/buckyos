//! Freshness of generated results and relation queries (phase two §6.2, §7.5). Shared by the
//! service and the WASM replica: the front end subscribes and displays, it never derives.
//!
//! Freshness is computed from the persisted dependency record (`EntityRow::derived`, or a wish's
//! `last_run.read_set`) against the version cells that exist *now* — never from timestamps.
//! Inputs declared `follow` go stale when their cell moved; `fixed` inputs never do. Everything the
//! caller may not read yields "unknown" rather than a guess.

use crate::access::{Access, Cap};
use crate::error::{Code, WsResult};
use crate::model::*;
use crate::plan::resolve_cell;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// Longest dependency chain followed for "upstream stale".
const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Every followed input still has the version the result was generated from.
    Current,
    /// A followed input changed since generation.
    Stale,
    /// Direct inputs are unchanged, but one of them is itself stale (or upstream stale).
    UpstreamStale,
    /// An input is gone (deleted entity, deleted record/field/block, missing fixed snapshot).
    Unavailable,
    /// Cannot be confirmed: an input is unreadable or its version cell cannot be resolved here.
    Unknown,
    /// The entity carries no dependency record.
    None,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Current => "current",
            Status::Stale => "stale",
            Status::UpstreamStale => "upstream_stale",
            Status::Unavailable => "unavailable",
            Status::Unknown => "unknown",
            Status::None => "none",
        }
    }
}

fn worse(a: Status, b: Status) -> Status {
    // unavailable > stale > unknown > upstream_stale > current
    let rank = |s: Status| match s {
        Status::Unavailable => 5,
        Status::Stale => 4,
        Status::Unknown => 3,
        Status::UpstreamStale => 2,
        Status::Current => 1,
        Status::None => 0,
    };
    if rank(a) >= rank(b) { a } else { b }
}

/// The recorded version of an input: `{ rev }` or `{ hash }` inside `version`.
fn recorded_cell(input: &Value) -> Option<Value> {
    let v = input.get("version")?;
    v.get("rev").cloned().or_else(|| v.get("hash").cloned())
}

/// Compare one recorded input with the state now. Returns the input's own status and a report line.
fn check_input(ctx: &dyn ReadCtx, access: &Access, input: &Value) -> WsResult<(Status, Value)> {
    let id = input.get("entity_id").and_then(Value::as_str).unwrap_or("");
    let fixed = input.get("version").and_then(|v| v.get("mode")).and_then(Value::as_str) == Some("fixed");
    let mut line = Map::new();
    line.insert("entity_id".into(), json!(id));
    if let Some(s) = input.get("selector") {
        line.insert("selector".into(), s.clone());
    }
    if let Some(l) = input.get("label") {
        line.insert("label".into(), l.clone());
    }
    line.insert("mode".into(), json!(if fixed { "fixed" } else { "follow" }));
    let Some(e) = ctx.entity(id)? else {
        line.insert("reason".into(), json!("missing"));
        return Ok((Status::Unavailable, Value::Object(line)));
    };
    if !access.can_read(ctx, &e)? {
        line.insert("readable".into(), json!(false));
        return Ok((Status::Unknown, Value::Object(line)));
    }
    line.insert("readable".into(), json!(true));
    line.insert("type_id".into(), json!(e.type_id));
    if let Some(n) = &e.name {
        line.insert("name".into(), json!(n));
    }
    if !e.alive() {
        line.insert("reason".into(), json!("deleted"));
        return Ok((Status::Unavailable, Value::Object(line)));
    }
    let target = json!({ "entity_id": id, "selector": input.get("selector").cloned().unwrap_or(json!({ "kind": "entity" })) });
    let current = match resolve_cell(ctx, &target) {
        Ok(v) => v,
        Err(err) if err.code == Code::NotFound || err.code == Code::TargetDeleted => {
            line.insert("reason".into(), json!("selector_lost"));
            return Ok((Status::Unavailable, Value::Object(line)));
        }
        Err(err) => {
            line.insert("reason".into(), json!(err.code.as_str()));
            return Ok((Status::Unknown, Value::Object(line)));
        }
    };
    let Some(recorded) = recorded_cell(input) else {
        // no version was recorded (e.g. a wish input before any run): nothing to compare against
        line.insert("reason".into(), json!("no_recorded_version"));
        return Ok((Status::Unknown, Value::Object(line)));
    };
    line.insert("recorded".into(), recorded.clone());
    line.insert("current".into(), current.clone());
    if recorded == current {
        return Ok((Status::Current, Value::Object(line)));
    }
    if fixed {
        // a fixed input does not go stale; the newer version is only reported
        line.insert("newer".into(), json!(true));
        return Ok((Status::Current, Value::Object(line)));
    }
    line.insert("changed".into(), json!(true));
    Ok((Status::Stale, Value::Object(line)))
}

/// Freshness of a dependency record (`inputs` in the shape `check_input` reads). `visited` guards cycles.
fn record_freshness(ctx: &dyn ReadCtx, access: &Access, inputs: &[Value], visited: &mut BTreeSet<String>, depth: usize) -> WsResult<(Status, Vec<Value>, Vec<Value>)> {
    let (mut status, mut lines, mut upstream) = (Status::Current, Vec::new(), Vec::new());
    for input in inputs {
        let (st, line) = check_input(ctx, access, input)?;
        status = worse(status, st);
        if st == Status::Current && depth < MAX_DEPTH {
            // the input itself may be a generated result that is stale
            let id = input.get("entity_id").and_then(Value::as_str).unwrap_or("").to_string();
            if !visited.contains(&id) {
                if let Some(e) = ctx.entity(&id)?.filter(|e| e.derived.is_some()) {
                    visited.insert(id.clone());
                    let up = entity_freshness_inner(ctx, access, &e, visited, depth + 1)?;
                    let up_status = up["status"].as_str().unwrap_or("none");
                    if matches!(up_status, "stale" | "upstream_stale" | "unavailable") {
                        upstream.push(json!({ "entity_id": id, "status": up_status, "changed_inputs": up["changed_inputs"] }));
                        status = worse(status, Status::UpstreamStale);
                    }
                }
            }
        }
        lines.push(line);
    }
    Ok((status, lines, upstream))
}

fn status_of(s: &str) -> Status {
    match s {
        "stale" => Status::Stale,
        "upstream_stale" => Status::UpstreamStale,
        "unavailable" => Status::Unavailable,
        "unknown" => Status::Unknown,
        "none" => Status::None,
        _ => Status::Current,
    }
}

/// The configuration digest of the wish now, when it exists and may be read.
fn wish_config_now(ctx: &dyn ReadCtx, access: &Access, wish_id: &str) -> WsResult<Option<String>> {
    match ctx.entity(wish_id)? {
        Some(w) if w.alive() && w.type_id == TYPE_WISH && access.can_read(ctx, &w)? => Ok(Some(crate::wish::config_digest(&w.payload))),
        _ => Ok(None),
    }
}

fn entity_freshness_inner(ctx: &dyn ReadCtx, access: &Access, e: &EntityRow, visited: &mut BTreeSet<String>, depth: usize) -> WsResult<Value> {
    let Some(derived) = &e.derived else { return Ok(json!({ "entity_id": e.entity_id, "status": Status::None.as_str() })) };
    let inputs: Vec<Value> = derived.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default();
    let (mut status, lines, upstream) = record_freshness(ctx, access, &inputs, visited, depth)?;
    // imported while stale: stays stale until the wish runs again (the package's revisions could not be compared)
    let imported_stale = derived.get("stale_at_import") == Some(&json!(true));
    if imported_stale {
        status = worse(status, Status::Stale);
    }
    // generated under another configuration (prompt, knowledge, refinements, inputs, program…)
    let mut config_changed = false;
    if let (Some(recorded), Some(wish)) = (derived.get("config_digest").and_then(Value::as_str), derived.get("wish_id").and_then(Value::as_str)) {
        if let Some(now) = wish_config_now(ctx, access, wish)? {
            if now != recorded {
                config_changed = true;
                status = worse(status, Status::Stale);
            }
        }
    }
    let generated_rev = derived.get("generated_rev").and_then(Value::as_u64).unwrap_or(0);
    let changed: Vec<Value> = lines.iter().filter(|l| l["changed"] == json!(true)).cloned().collect();
    Ok(json!({
        "entity_id": e.entity_id,
        "status": status.as_str(),
        "inputs": lines,
        "changed_inputs": changed,
        "upstream": upstream,
        // the content moved on after generation and not through the wish (a later run re-records)
        "manual_modified": hand_edited(e, generated_rev),
        "imported_stale": imported_stale,
        "config_changed": config_changed,
        "generated_rev": generated_rev,
        "content_rev": e.content_rev,
        "wish_id": derived.get("wish_id"),
        "run_id": derived.get("run_id"),
        "executor": derived.get("executor"),
        "approach": derived.get("approach"),
        "result_key": derived.get("result_key"),
        // read from outside the Workspace by the program: that part cannot be confirmed (§9.3)
        "external_data": derived.get("external_data").cloned().unwrap_or(json!(false)),
        "model_judgment": derived.get("model_judgment").cloned().unwrap_or(json!(false)),
        "simulated": derived.get("simulated").cloned().unwrap_or(json!(false)),
    }))
}

/// Freshness of one entity: a result's dependency record, or a wish's last run. Unreadable
/// entities are reported as not found (existence is not leaked).
pub fn entity_freshness(ctx: &dyn ReadCtx, access: &Access, id: &str) -> WsResult<Value> {
    let e = crate::read::readable_entity(ctx, access, id)?;
    if e.type_id == TYPE_WISH {
        return wish_freshness(ctx, access, &e);
    }
    let mut visited = BTreeSet::new();
    visited.insert(id.to_string());
    entity_freshness_inner(ctx, access, &e, &mut visited, 0)
}

/// A wish: the state of its **current** result group (`last_run.result_bindings`; earlier
/// comparison groups keep their own state and do not make the newest one look stale), whether its
/// analysis still matches what it was made from, and whether its declared inputs are still there.
/// The documents whose rules or values the analysis wrote into its task description
/// (`analysis.basis.reads`, §8.1): changed since → the analysis must be made again.
pub fn analysis_sources_changed(ctx: &dyn ReadCtx, payload: &JsonMap) -> WsResult<Vec<Value>> {
    let mut changed = Vec::new();
    for r in payload.get("analysis").and_then(|a| a.get("basis")).and_then(|b| b.get("reads")).and_then(Value::as_array).into_iter().flatten() {
        let target = json!({ "entity_id": r.get("entity_id").cloned().unwrap_or(Value::Null), "selector": r.get("selector").cloned().unwrap_or(json!({ "kind": "entity" })) });
        let now = resolve_cell(ctx, &target).ok();
        if now.is_none() || now != recorded_cell(r) {
            changed.push(r.clone());
        }
    }
    Ok(changed)
}

fn wish_freshness(ctx: &dyn ReadCtx, access: &Access, e: &EntityRow) -> WsResult<Value> {
    let sources_changed = analysis_sources_changed(ctx, &e.payload)?;
    let needs_analysis = crate::wish::needs_analysis(&e.payload) || !sources_changed.is_empty();
    let analysis_status = e.payload.get("analysis").and_then(|a| a.get("status")).cloned().unwrap_or(Value::Null);
    let declared: Vec<Value> = e.payload.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default();
    // declared inputs: are they all still there and readable?
    let mut input_problems = Vec::new();
    for input in &declared {
        let id = input.get("entity_id").and_then(Value::as_str).unwrap_or("");
        match ctx.entity(id)? {
            Some(t) if t.alive() && access.can_read(ctx, &t)? => {
                if let Some(cell) = input.get("selector").filter(|s| s["kind"] == json!("table_view")).and_then(|s| s["cell_id"].as_str()) {
                    if !ctx.entity(cell)?.is_some_and(|c| c.alive()) {
                        input_problems.push(json!({ "entity_id": id, "reason": "view_missing", "cell_id": cell }));
                    }
                }
            }
            Some(t) if t.alive() => input_problems.push(json!({ "entity_id": id, "reason": "unreadable" })),
            _ => input_problems.push(json!({ "entity_id": id, "reason": "missing" })),
        }
    }
    let last_run = e.payload.get("last_run");
    let read_set: Vec<Value> = last_run.and_then(|r| r.get("read_set")).and_then(Value::as_array).cloned().unwrap_or_default();
    let config_now = crate::wish::config_digest(&e.payload);
    let config_changed = last_run.and_then(|r| r.get("config_digest")).and_then(Value::as_str).is_some_and(|d| d != config_now);
    let mut visited = BTreeSet::new();
    visited.insert(e.entity_id.clone());
    let bindings = crate::wish::current_results(&e.payload);
    let mut results = Vec::new();
    let (mut status, mut lines, mut upstream) = (Status::Current, Vec::<Value>::new(), Vec::<Value>::new());
    let mut merge = |st: Status, ls: &Value, up: &Value, status: &mut Status| {
        *status = worse(*status, st);
        for l in ls.as_array().into_iter().flatten() {
            if !lines.contains(l) {
                lines.push(l.clone());
            }
        }
        for u in up.as_array().into_iter().flatten() {
            if !upstream.contains(u) {
                upstream.push(u.clone());
            }
        }
    };
    let mut produced = Vec::new();
    if !bindings.is_empty() {
        for (name, b) in &bindings {
            let ty = b.get("type").and_then(Value::as_str).unwrap_or("");
            let id = b.get("entity_id").and_then(Value::as_str).unwrap_or("");
            let approach = b.get("approach").cloned().unwrap_or(Value::Null);
            if ty == "control" && !ctx.entity(id)?.is_some_and(|c| c.alive()) {
                status = worse(status, Status::Unavailable);
                results.push(json!({ "name": name, "type": ty, "entity_id": id, "approach": approach, "status": "unavailable", "reason": "missing" }));
                continue;
            }
            if ty == "table_columns" || ty == "control" {
                // derived columns live on an input table, a control on the canvas without a generation record:
                // their record is the run's read set
                let (mut st, ls, up) = record_freshness(ctx, access, &read_set, &mut visited.clone(), 0)?;
                if config_changed {
                    st = worse(st, Status::Stale);
                }
                merge(st, &json!(ls), &json!(up), &mut status);
                results.push(json!({ "name": name, "type": ty, "entity_id": id, "approach": approach, "status": st.as_str() }));
                continue;
            }
            match ctx.entity(id)? {
                Some(res) if res.alive() && res.derived.is_some() && access.can_read(ctx, &res)? => {
                    let f = entity_freshness_inner(ctx, access, &res, &mut visited.clone(), 0)?;
                    let st = status_of(f["status"].as_str().unwrap_or("none"));
                    merge(st, &f["inputs"], &f["upstream"], &mut status);
                    produced.push(res.entity_id.clone());
                    results.push(json!({ "name": name, "type": ty, "entity_id": id, "approach": f["approach"].clone(), "status": st.as_str(),
                                         "manual_modified": f["manual_modified"], "external_data": f["external_data"], "model_judgment": f["model_judgment"] }));
                }
                Some(res) if res.alive() && !access.can_read(ctx, &res)? => {
                    status = worse(status, Status::Unknown);
                    results.push(json!({ "name": name, "type": ty, "approach": approach, "status": "unknown" }));
                }
                _ => {
                    status = worse(status, Status::Unavailable);
                    results.push(json!({ "name": name, "type": ty, "entity_id": id, "approach": approach, "status": "unavailable", "reason": "missing" }));
                }
            }
        }
    } else if last_run.is_none() {
        status = Status::None;
    } else {
        let (st, ls, up) = record_freshness(ctx, access, &read_set, &mut visited, 0)?;
        merge(st, &json!(ls), &json!(up), &mut status);
    }
    if config_changed && status != Status::None {
        status = worse(status, Status::Stale);
    }
    // what the viewer cannot read stays out of the reasons: an input it cannot read is an input problem (`unreadable`),
    // never a story of its fields
    let manifest_inputs = e.payload.get("program").and_then(|p| p.get("manifest")).and_then(|m| m.get("inputs"));
    let mut coding = Vec::new();
    for r in crate::wish::macro_check(ctx, &e.payload)? {
        let id = r.get("entity_id").and_then(Value::as_str).or_else(|| {
            let name = r.get("input").and_then(Value::as_str)?;
            manifest_inputs.and_then(|m| m.get(name)).and_then(|d| d.get("entity_id")).and_then(Value::as_str)
        });
        let hidden = match id.map(|id| ctx.entity(id)).transpose()?.flatten() {
            Some(t) if t.alive() => !access.can_read(ctx, &t)?,
            _ => false,
        };
        if !hidden {
            coding.push(r);
        }
    }
    let changed: Vec<Value> = lines.iter().filter(|l| l["changed"] == json!(true)).cloned().collect();
    Ok(json!({
        "entity_id": e.entity_id,
        "status": status.as_str(),
        "inputs": lines,
        "changed_inputs": changed,
        "upstream": upstream,
        "needs_analysis": needs_analysis,
        "analysis_sources_changed": sources_changed,
        "analysis_status": analysis_status,
        "config_changed": config_changed,
        "config_digest": config_now,
        "input_problems": input_problems,
        "last_run": last_run,
        "results": results,
        "needs_coding": if coding.is_empty() { Value::Null } else { json!({ "reasons": coding }) },
        "checks": last_run.and_then(|r| r.get("checks")).cloned().unwrap_or(Value::Null),
        "produced": produced,
        "wish": true,
    }))
}

/// Whether `e` changed after a run generated it (at `generated_rev`) other than through a run. A manager vouching for a
/// definition's code (`trusted`, 权限需求 Q32) changes no content.
pub fn hand_edited(e: &EntityRow, generated_rev: u64) -> bool {
    if e.content_rev <= generated_rev {
        return false;
    }
    e.type_id != TYPE_BLOCK_DEF || e.key_revs.iter().any(|(k, r)| k != "trusted" && *r > generated_rev)
}

/// Run codes that send a wish back to coding: the macro failed, its code changed, or it no longer fits its inputs.
const CODING_CODES: &[&str] = &["PROGRAM_FAILED", "MACRO_CODE_CHANGED", "NEEDS_CODING"];
/// Run codes that say the wish cannot run here now (permissions, AI quota, execution environment).
const CANNOT_RUN_CODES: &[&str] = &[
    "AI_QUOTA_EXHAUSTED", "AI_QUOTA_BUDGET_EXCEEDED", "AI_RESOURCE_NOT_PERMITTED", "AI_ATTRIBUTION_UNAVAILABLE", "AI_UNAVAILABLE",
    "EXECUTION_CONTEXT_MISSING", "SNAPSHOT_TOO_LARGE", "PERMISSION_DENIED", "ACCESS_DENIED",
];

fn run_code(run: &Value) -> Option<&str> {
    let e = run.get("error")?;
    e.get("sub_code").and_then(Value::as_str).or_else(|| e.get("code").and_then(Value::as_str))
}

/// The one main state of a wish and its next action (PRD §5.8, 第一期 §2.7): the first that holds of
/// needs_input, needs_analysis, needs_coding, waiting_data (no_access when the caller cannot read an input: nothing
/// to wait for), cannot_run, in_progress, pending_confirmation, stale, current. `freshness` is `wish_freshness`; `runs` what the caller knows of the wish's runs: `{ active?: { stage,
/// state }, waiting?: { stage, run_id }, last_failed?: { stage, error: { code, sub_code } } }`. `stale` stays true
/// when an earlier state wins, so a running or failed run never hides that the applied results are out of date.
pub fn main_status(freshness: &Value, runs: &Value) -> Value {
    let f = freshness;
    let analysis = f.get("analysis_status").and_then(Value::as_str);
    let status = f.get("status").and_then(Value::as_str).unwrap_or("none");
    let stale = matches!(status, "stale" | "upstream_stale" | "unavailable") || f.get("config_changed") == Some(&json!(true));
    let failed_code = runs.get("last_failed").and_then(run_code);
    let stage_of = |k: &str| runs.get(k).filter(|r| !r.is_null()).and_then(|r| r.get("stage")).and_then(Value::as_str);
    let (active, waiting) = (stage_of("active"), stage_of("waiting"));
    // a run of the stage that resolves a state (analysis for needs_analysis, coding for needs_coding) is that state's progress
    let resolving = |stage: &str| active == Some(stage) || waiting == Some(stage);
    let mut reasons: Vec<Value> = Vec::new();
    let needs_analysis = analysis.is_none() || f.get("needs_analysis") == Some(&json!(true));
    let needs_coding = f.get("needs_coding").and_then(|n| n.get("reasons")).and_then(Value::as_array).is_some_and(|r| !r.is_empty());
    let (main, next) = if (needs_analysis && resolving(crate::wish::STAGE_ANALYZE)) || (!needs_analysis && needs_coding && resolving(crate::wish::STAGE_CODE)) {
        if active.is_some() {
            ("in_progress", "cancel")
        } else {
            ("pending_confirmation", "apply")
        }
    } else if analysis == Some("needs_input") {
        ("needs_input", "answer")
    } else if needs_analysis {
        if let Some(s) = f.get("analysis_sources_changed").and_then(Value::as_array).filter(|a| !a.is_empty()) {
            reasons.push(json!({ "code": "analysis_sources_changed", "sources": s }));
        }
        ("needs_analysis", "analyze")
    } else if let Some(r) = f.get("needs_coding").and_then(|n| n.get("reasons")).and_then(Value::as_array).filter(|r| !r.is_empty()) {
        reasons.extend(r.iter().cloned());
        ("needs_coding", "code")
    } else if let Some(code) = failed_code.filter(|c| CODING_CODES.contains(c)) {
        reasons.push(json!({ "code": "program_failed", "run_code": code }));
        ("needs_coding", "code")
    } else if let Some(p) = f.get("input_problems").and_then(Value::as_array).filter(|p| !p.is_empty()) {
        reasons.extend(p.iter().cloned());
        if p.iter().any(|x| x["reason"] == json!("unreadable")) {
            ("no_access", "none")
        } else {
            ("waiting_data", "view_inputs")
        }
    } else if let Some(code) = failed_code.filter(|c| CANNOT_RUN_CODES.contains(c)) {
        reasons.push(json!({ "code": code }));
        ("cannot_run", "fix")
    } else if runs.get("active").is_some_and(|a| !a.is_null()) {
        ("in_progress", "cancel")
    } else if runs.get("waiting").is_some_and(|w| !w.is_null()) {
        ("pending_confirmation", "apply")
    } else if stale || status == "none" {
        ("stale", "run")
    } else {
        ("current", "none")
    };
    json!({ "main": main, "next_action": next, "reasons": reasons, "stale": stale })
}

/// Freshness of many entities in one call (unknown / unreadable ids are reported per entry).
pub fn freshness_many(ctx: &dyn ReadCtx, access: &Access, ids: &[String]) -> WsResult<Vec<Value>> {
    let mut out = Vec::new();
    for id in ids {
        out.push(match entity_freshness(ctx, access, id) {
            Ok(v) => v,
            Err(e) => json!({ "entity_id": id, "status": "unknown", "error": e.to_json() }),
        });
    }
    Ok(out)
}

fn brief(ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<Value> {
    let mut v = json!({ "entity_id": e.entity_id, "type_id": e.type_id, "name": e.name, "deleted": !e.alive(),
                        "title": e.payload.get("title").cloned().unwrap_or(Value::Null) });
    if let Some(edge) = ctx.edge(&e.entity_id)? {
        v["parent_id"] = json!(edge.parent_id);
    }
    if e.type_id == TYPE_CONTAINER {
        v["kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
    }
    if e.type_id == TYPE_CELL {
        v["view_type"] = e.payload.get("view").and_then(|v| v.get("type")).cloned().unwrap_or(Value::Null);
    }
    Ok(v)
}

/// What an entity references, what references it, what it was generated from and what was generated
/// from it — filtered to what the caller may read. Incoming references from unreadable entities are
/// reported as a single flag, never as ids, names or counts.
pub fn relations(ctx: &dyn ReadCtx, access: &Access, id: &str) -> WsResult<Value> {
    let e = crate::read::readable_entity(ctx, access, id)?;
    let mut outgoing = Vec::new();
    for r in ctx.refs_from(id)? {
        if !r.dst_workspace_id.is_empty() {
            outgoing.push(json!({ "kind": r.kind, "selector": r.src_selector, "workspace_id": r.dst_workspace_id, "entity_id": r.dst_entity_id, "external": true }));
            continue;
        }
        if r.dst_entity_id.is_empty() {
            if !r.dst_object_id.is_empty() {
                outgoing.push(json!({ "kind": r.kind, "selector": r.src_selector, "object_id": r.dst_object_id }));
            }
            continue;
        }
        let mut line = json!({ "kind": r.kind, "selector": r.src_selector, "entity_id": r.dst_entity_id });
        match ctx.entity(&r.dst_entity_id)? {
            Some(t) if access.can_read(ctx, &t)? => {
                line["target"] = brief(ctx, &t)?;
                line["readable"] = json!(true);
            }
            Some(_) => {
                line["readable"] = json!(false);
            }
            None => {
                line["missing"] = json!(true);
            }
        }
        outgoing.push(line);
    }
    let (mut incoming, mut hidden) = (Vec::new(), false);
    for r in ctx.refs_to(id)? {
        match ctx.entity(&r.src_entity_id)? {
            Some(s) if s.alive() => {
                if access.can_read(ctx, &s)? {
                    incoming.push(json!({ "kind": r.kind, "selector": r.src_selector, "entity_id": r.src_entity_id, "source": brief(ctx, &s)?,
                                          "blocks_delete": r.blocks_delete() }));
                } else {
                    hidden = true;
                }
            }
            _ => {}
        }
    }
    // Blocks showing this data, and (for a wish) the results it produced
    let blocks: Vec<Value> = incoming.iter().filter(|l| l["kind"] == json!("bind")).map(|l| l["source"].clone()).collect();
    let produced: Vec<Value> = incoming.iter().filter(|l| l["kind"] == json!("produced")).map(|l| l["source"].clone()).collect();
    let dependents: Vec<Value> = incoming.iter().filter(|l| l["kind"] == json!("derived") || l["kind"] == json!("input")).map(|l| json!({ "kind": l["kind"], "entity": l["source"] })).collect();
    let mut v = json!({
        "entity_id": id,
        "entity": brief(ctx, &e)?,
        "outgoing": outgoing,
        "incoming": incoming,
        "hidden_incoming": hidden,
        "blocks": blocks,
        "produced": produced,
        "dependents": dependents,
        "derived": e.derived,
    });
    if access.can(ctx, id, Cap::Read)? {
        v["freshness"] = entity_freshness(ctx, access, id)?;
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::MemWorkspace;

    /// A wish's freshness with nothing to do: analysed, coded, run and current.
    fn current() -> Value {
        json!({ "status": "current", "analysis_status": "ready", "needs_analysis": false, "analysis_sources_changed": [],
                "needs_coding": null, "input_problems": [], "config_changed": false })
    }

    fn main_of(f: &Value, runs: &Value) -> (String, String, bool) {
        let m = main_status(f, runs);
        (m["main"].as_str().unwrap().into(), m["next_action"].as_str().unwrap().into(), m["stale"].as_bool().unwrap())
    }

    fn s(main: &str, next: &str, stale: bool) -> (String, String, bool) {
        (main.into(), next.into(), stale)
    }

    #[test]
    fn main_status_takes_the_first_state_that_holds() {
        let none = json!({});
        let active = json!({ "active": { "stage": "run", "state": "running" } });
        let waiting = json!({ "waiting": { "stage": "run", "run_id": "r2" } });
        // as `WsError::to_json` writes a run's error: `sub_code` only when there is one
        let failed = |code: &str, sub: Option<&str>| {
            let mut e = json!({ "code": code });
            if let Some(s) = sub {
                e["sub_code"] = json!(s);
            }
            json!({ "last_failed": { "stage": "run", "error": e } })
        };
        assert_eq!(main_of(&current(), &none), s("current", "none", false));

        // 9 → 1, each state added on top of the ones after it
        let mut f = current();
        f["status"] = json!("stale");
        assert_eq!(main_of(&f, &none), s("stale", "run", true), "8 stale");
        assert_eq!(main_of(&f, &waiting), s("pending_confirmation", "apply", true), "7 pending confirmation over stale");
        let mut both = waiting.clone();
        both["active"] = active["active"].clone();
        assert_eq!(main_of(&f, &both), s("in_progress", "cancel", true), "6 in progress over pending confirmation");
        let mut runs = both.clone();
        runs["last_failed"] = failed("DEPENDENCY_UNAVAILABLE", Some("AI_QUOTA_EXHAUSTED"))["last_failed"].clone();
        assert_eq!(main_of(&f, &runs), s("cannot_run", "fix", true), "5 cannot run over in progress");
        assert_eq!(main_status(&f, &runs)["reasons"], json!([{ "code": "AI_QUOTA_EXHAUSTED" }]));
        f["input_problems"] = json!([{ "entity_id": "orders", "reason": "view_missing", "cell_id": "blk" }]);
        assert_eq!(main_of(&f, &runs), s("waiting_data", "view_inputs", true), "4 waiting data over cannot run");
        assert_eq!(main_status(&f, &runs)["reasons"][0]["reason"], "view_missing");
        let mut g = f.clone();
        g["input_problems"] = json!([{ "entity_id": "orders", "reason": "unreadable" }]);
        assert_eq!(main_of(&g, &runs), s("no_access", "none", true), "4 an input the caller cannot read: nothing to wait for");
        f["needs_coding"] = json!({ "reasons": [{ "code": "field_deleted", "input": "sales", "field_id": "month", "name": "月份" }] }); // i18n-exempt: test data (user-written names and text)
        assert_eq!(main_of(&f, &runs), s("needs_coding", "code", true), "3 needs coding over waiting data");
        assert_eq!(main_status(&f, &runs)["reasons"][0]["code"], "field_deleted");
        f["needs_analysis"] = json!(true);
        f["analysis_sources_changed"] = json!([{ "entity_id": "rules" }]);
        assert_eq!(main_of(&f, &runs), s("needs_analysis", "analyze", true), "2 needs analysis over needs coding");
        assert_eq!(main_status(&f, &runs)["reasons"], json!([{ "code": "analysis_sources_changed", "sources": [{ "entity_id": "rules" }] }]));
        f["analysis_status"] = json!("needs_input");
        assert_eq!(main_of(&f, &runs), s("needs_input", "answer", true), "1 needs input over everything");

        // a run of the stage that resolves the state is its progress: the first macro is stored only when its candidate is applied
        let mut f = current();
        f["needs_coding"] = json!({ "reasons": [{ "code": "no_macro" }] });
        assert_eq!(main_of(&f, &json!({ "active": { "stage": "code", "state": "running" } })).0, "in_progress");
        assert_eq!(main_of(&f, &json!({ "waiting": { "stage": "code", "run_id": "r" } })).0, "pending_confirmation");
        assert_eq!(main_of(&f, &json!({ "waiting": { "stage": "run", "run_id": "r" } })).0, "needs_coding");
        f["needs_analysis"] = json!(true);
        assert_eq!(main_of(&f, &json!({ "waiting": { "stage": "analyze", "run_id": "r" } })).0, "pending_confirmation");
        assert_eq!(main_of(&f, &json!({ "waiting": { "stage": "code", "run_id": "r" } })).0, "needs_analysis", "a coding candidate on a stale analysis does not hide it");

        // needs analysis: no analysis at all, or one that no longer matches its basis
        let mut f = current();
        f["analysis_status"] = Value::Null;
        f["status"] = json!("none");
        assert_eq!(main_of(&f, &none), s("needs_analysis", "analyze", false));
        assert_eq!(main_status(&f, &none)["reasons"], json!([]), "no sources changed: no reason listed");
        let mut f = current();
        f["needs_analysis"] = json!(true);
        assert_eq!(main_of(&f, &none).0, "needs_analysis");

        // a failed run of the macro sends it back to coding (sub code first, then code); an empty reason list does not
        let mut f = current();
        f["needs_coding"] = json!({ "reasons": [] });
        assert_eq!(main_of(&f, &none), s("current", "none", false));
        for (code, sub) in [("INVALID_OPERATION", Some("PROGRAM_FAILED")), ("INVALID_OPERATION", Some("MACRO_CODE_CHANGED")), ("INVALID_OPERATION", Some("NEEDS_CODING")), ("PROGRAM_FAILED", None)] {
            let m = main_status(&f, &failed(code, sub));
            assert_eq!((m["main"].as_str(), m["next_action"].as_str()), (Some("needs_coding"), Some("code")), "{code} {sub:?}");
            assert_eq!(m["reasons"], json!([{ "code": "program_failed", "run_code": sub.unwrap_or(code) }]));
        }
        // a coding failure goes before waiting data, as the stored reasons do
        let mut g = f.clone();
        g["input_problems"] = json!([{ "entity_id": "orders", "reason": "missing" }]);
        assert_eq!(main_of(&g, &failed("INVALID_OPERATION", Some("PROGRAM_FAILED"))).0, "needs_coding");
        // cannot run: permissions, AI quota, execution environment — by code when there is no sub code
        for code in ["PERMISSION_DENIED", "EXECUTION_CONTEXT_MISSING", "SNAPSHOT_TOO_LARGE"] {
            assert_eq!(main_of(&f, &failed(code, None)), s("cannot_run", "fix", false), "{code}");
        }
        // any other failure leaves the state to what the wish is: here current
        assert_eq!(main_of(&f, &failed("INTERNAL", Some("TIMEOUT"))), s("current", "none", false));
        let mut runs = failed("INTERNAL", None);
        runs["active"] = Value::Null;
        runs["waiting"] = Value::Null;
        assert_eq!(main_of(&f, &runs), s("current", "none", false), "null runs are no runs");

        // stale: the results' state, their configuration, a gone input; a wish never run is to be run
        for st in ["stale", "upstream_stale", "unavailable"] {
            let mut f = current();
            f["status"] = json!(st);
            assert_eq!(main_of(&f, &none), s("stale", "run", true), "{st}");
            assert_eq!(main_of(&f, &active), s("in_progress", "cancel", true), "{st}: a running run keeps the results stale");
        }
        let mut f = current();
        f["config_changed"] = json!(true);
        assert_eq!(main_of(&f, &none), s("stale", "run", true), "another configuration");
        f["needs_coding"] = json!({ "reasons": [{ "code": "api_version" }] });
        assert_eq!(main_of(&f, &none), s("needs_coding", "code", true));
        let mut f = current();
        f["status"] = json!("none");
        assert_eq!(main_of(&f, &none), s("stale", "run", false), "never run: run it");
        f["status"] = json!("unknown");
        assert_eq!(main_of(&f, &none), s("current", "none", false), "unknown is not stale");
    }

    fn def_rev(ws: &MemWorkspace, field: &str) -> u64 {
        ws.store.fields[&("t1".to_string(), field.to_string())].def_rev
    }

    /// The wish's own freshness says whether its macro fits its inputs (`needs_coding`), and there is no
    /// `direct_stale` any more: every result comes from the macro.
    #[test]
    fn wish_freshness_needs_coding() {
        let mut ws = MemWorkspace::new();
        ws.ok(json!([
            { "op": "entity.create", "entity_id": "s1-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
              "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s1", "title": "s1" } },
            { "op": "entity.create", "entity_id": "s1", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a", "name": "s1",
              "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "s1", "content_folder_id": "s1-content" } },
            { "op": "entity.create", "entity_id": "t1", "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": "b", "name": "t1",
              "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }, { "field_id": "n", "name": "数", "type": "number" }] } }, // i18n-exempt: test data (user-written names and text)
        ]));
        ws.ok(json!([{ "op": "entity.create", "entity_id": "w1", "type_id": TYPE_WISH, "parent_id": "s1-content", "order_key": "w", "name": "w1",
            "payload": { "title": "许愿格", "prompt": "汇总", "executor": "xllm", "inputs": [{ "entity_id": "t1", "name": "rows" }], // i18n-exempt: test data (user-written names and text)
                         "output": { "container_id": "s1-content", "name": "汇总" }, "output_mode": "overwrite" } }])); // i18n-exempt: test data (user-written names and text)
        let alice = Access::full("alice");
        let fresh = |ws: &MemWorkspace| entity_freshness(&ws.store, &alice, "w1").unwrap();
        let f = fresh(&ws);
        assert!(f.get("direct_stale").is_none(), "{f}");
        assert_eq!(main_status(&f, &json!({}))["main"], "needs_analysis");
        // analysed, not coded yet
        let analysis = json!({ "schema_version": crate::wish::ANALYSIS_SCHEMA, "status": "ready", "prompt": "汇总", "context_prompt": "按数汇总 rows", // i18n-exempt: test data (user-written names and text)
                               "output_contract": { "results": [{ "name": "sum", "type": "table", "key": ["标题"] }] }, "checks": [], "blockers": [], "warnings": [] }); // i18n-exempt: test data (user-written names and text)
        ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "analysis", "value": analysis, "expect": { "rev": 0 } }] }]));
        let f = fresh(&ws);
        assert_eq!((f["needs_analysis"].as_bool(), &f["needs_coding"]), (Some(false), &json!({ "reasons": [{ "code": "no_macro" }] })), "{f}");
        assert_eq!(main_status(&f, &json!({}))["next_action"], "code");
        // coded: the macro declares 数 of rows
        let object_id = format!("mix256:{}", "c".repeat(64));
        ws.store.assets.insert(object_id.clone(), AssetInfo { object_id: object_id.clone(), media_type: "text/plain".into(), size: 10 });
        let manifest = json!({ "inputs": { "rows": { "entity_id": "t1", "type_id": TYPE_TABLE, "fields": [{ "field_id": "n", "name": "数", "type": "number" }] } }, // i18n-exempt: test data (user-written names and text)
                               "results": { "sum": { "type": "table" } } });
        let program = json!({ "language": "js", "api_version": crate::wish::PROGRAM_API_VERSION, "source": object_id,
                              "digest": crate::wish::macro_digest("main()", &manifest), "manifest": manifest, "produces": ["sum"] });
        ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "program", "value": program, "expect": { "rev": 0 } }] }]));
        let f = fresh(&ws);
        assert_eq!(f["needs_coding"], Value::Null, "{f}");
        assert_eq!(main_status(&f, &json!({}))["main"], "stale", "coded but never run: run it");
        // renaming the declared field changes nothing; deleting it asks for coding, naming the field
        ws.ok(json!([{ "op": "table.update_field", "source_id": "t1", "field_id": "n", "changes": { "name": "数量" }, "expect": { "rev": def_rev(&ws, "n") } }])); // i18n-exempt: test data (user-written names and text)
        assert_eq!(fresh(&ws)["needs_coding"], Value::Null);
        ws.ok(json!([{ "op": "table.delete_field", "source_id": "t1", "field_id": "n", "expect": { "rev": def_rev(&ws, "n") } }]));
        let f = fresh(&ws);
        assert_eq!(f["needs_coding"], json!({ "reasons": [{ "code": "field_deleted", "input": "rows", "field_id": "n", "name": "数" }] }), "{f}"); // i18n-exempt: test data (user-written names and text)
        assert_eq!(f["needs_analysis"], false, "the analysis still stands");
        let m = main_status(&f, &json!({ "active": { "stage": "run" } }));
        assert_eq!((m["main"].as_str(), m["next_action"].as_str()), (Some("needs_coding"), Some("code")), "{m}");
    }
}
