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
        "manual_modified": e.content_rev > generated_rev,
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
            if ty == "table_columns" {
                // derived columns live on an input table: their record is the run's read set
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
    // only re-running the program refreshes program results; written text then needs regenerating (§9.2)
    let direct_stale = results.iter().any(|r| r["approach"] == json!("direct") && r["status"] == json!("stale"))
        && results.iter().filter(|r| r["approach"] == json!("program")).all(|r| r["status"] == json!("current"))
        && results.iter().any(|r| r["approach"] == json!("program"));
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
        "direct_stale": direct_stale,
        "checks": last_run.and_then(|r| r.get("checks")).cloned().unwrap_or(Value::Null),
        "produced": produced,
        "wish": true,
    }))
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
