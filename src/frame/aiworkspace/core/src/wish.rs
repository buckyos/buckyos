//! Wish protocol v3 (许愿格详细设计 §4, §7.2, §8.5; AI Native 第一期 §2): the persisted shapes of
//! `knowledge`, `refinements`, `program` (the macro: source and manifest) and `analysis`
//! (`wish.analysis.v3`), the three stages (analyze → code → run), the structure check of a macro
//! against the inputs as they are now, and the two digests the core, the service and the browser
//! replica share:
//!
//! - the **analysis basis**: what an analysis was made from (prompt, knowledge, refinements,
//!   executor and its configuration, input bindings, the analysis itself). Computed by the core
//!   whenever `analysis` is written, never taken from the writer; a different value now means
//!   "needs analysis".
//! - the **config digest**: the basis plus the macro (source and manifest). Results record it; a
//!   different value now means the result was generated under another configuration.
//!
//! Run-local handles never reach these shapes: everything persisted uses real ids.

use crate::canonical::{canonical_json, is_obj_id, sha256_hex};
use crate::error::{WsError, WsResult};
use crate::id::is_valid_id;
use crate::model::{JsonMap, TYPE_TABLE};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const ANALYSIS_SCHEMA: &str = "wish.analysis.v3";
pub const RESULTS_SCHEMA: &str = "wish.results.v2";
/// The `aiws` program API a stored macro is written for (`declare`, `sql`, `ai.complete`, `zone.fetch`).
pub const PROGRAM_API_VERSION: u64 = 3;
/// Stages of `wish.xllm@1` (第一期 §2.1): analysis → task contract; coding → macro + trial run; running the macro.
pub const STAGE_ANALYZE: &str = "analyze";
pub const STAGE_CODE: &str = "code";
pub const STAGE_RUN: &str = "run";
pub const STAGES: &[&str] = &[STAGE_ANALYZE, STAGE_CODE, STAGE_RUN];
pub const MAX_MANIFEST_INPUTS: usize = 32;
pub const MAX_MANIFEST_FIELDS: usize = 200;
pub const MAX_KNOWLEDGE_CHARS: usize = 20_000;
pub const MAX_REFINEMENTS: usize = 50;
pub const MAX_REFINEMENT_CHARS: usize = 2000;
pub const MAX_CONTRACT_RESULTS: usize = 32;
pub const MAX_CHECKS: usize = 32;
pub const MAX_VIEWS: usize = 4;
/// Result types of `wish.results.v2`. A `control` result is a Block-control instance on the canvas (a mind map…):
/// no data entity, its tree in the result.
pub const RESULT_TYPES: &[&str] = &["richtext", "record", "table", "table_columns", "image", "asset", "html", "video", "control"];

fn bad(detail: impl Into<String>) -> WsError {
    WsError::invalid_schema(detail)
}

fn text_ok(v: Option<&Value>, max: usize) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| s.chars().count() <= max)
}

fn only(o: &Map<String, Value>, keys: &[&str], what: &str) -> WsResult<()> {
    match o.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(bad(format!("{what}: unknown key {k}"))),
        None => Ok(()),
    }
}

/// Stable input name used by programs and context prompts: `^[A-Za-z_][A-Za-z0-9_]{0,63}$`.
pub fn is_input_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// Logical result key: 1–64 chars, no path separators, no `..`, no control characters or spaces at
/// the ends. Titles are separate and free-form.
pub fn is_result_name(s: &str) -> bool {
    let n = s.chars().count();
    n >= 1
        && n <= 64
        && s.trim() == s
        && !s.contains("..")
        && !s.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// Check ids: `^[A-Za-z_][A-Za-z0-9_-]{0,63}$`.
pub fn is_check_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 64 && (b[0].is_ascii_alphabetic() || b[0] == b'_') && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

pub fn check_knowledge(v: &Value) -> WsResult<()> {
    if text_ok(Some(v), MAX_KNOWLEDGE_CHARS) {
        Ok(())
    } else {
        Err(bad(format!("knowledge must be a string of at most {MAX_KNOWLEDGE_CHARS} chars")))
    }
}

/// `[{ text, at?, run_id? }]`, at most 50 entries.
pub fn check_refinements(v: &Value) -> WsResult<()> {
    let list = v.as_array().ok_or_else(|| bad("refinements must be an array"))?;
    if list.len() > MAX_REFINEMENTS {
        return Err(WsError::limit(format!("at most {MAX_REFINEMENTS} refinements")));
    }
    for r in list {
        let o = r.as_object().ok_or_else(|| bad("refinement must be an object"))?;
        only(o, &["text", "at", "run_id"], "refinement")?;
        if !text_ok(o.get("text"), MAX_REFINEMENT_CHARS) || o["text"].as_str().is_some_and(|s| s.trim().is_empty()) {
            return Err(bad(format!("refinement.text must be a non-empty string of at most {MAX_REFINEMENT_CHARS} chars")));
        }
        for k in ["at", "run_id"] {
            if o.get(k).is_some_and(|x| !text_ok(Some(x), 128)) {
                return Err(bad(format!("refinement.{k} must be a short string")));
            }
        }
    }
    Ok(())
}

/// `{ language: "js", api_version: 3, source: <object id>, digest, manifest, produces: [names], run_id?, size? }`.
/// Returns the source object id (an uploaded text asset).
pub fn check_program(v: &Value) -> WsResult<&str> {
    let o = v.as_object().ok_or_else(|| bad("program must be an object"))?;
    only(o, &["language", "api_version", "source", "digest", "manifest", "produces", "run_id", "size", "edited_by"], "program")?;
    if o.get("language").and_then(Value::as_str) != Some("js") {
        return Err(bad("program.language must be js"));
    }
    // format only: a macro for another `aiws` version still travels; the structure check asks for coding again
    if o.get("api_version").and_then(Value::as_u64).is_none_or(|n| n == 0) {
        return Err(bad("program.api_version must be a positive integer"));
    }
    let source = o.get("source").and_then(Value::as_str).filter(|s| is_obj_id(s)).ok_or_else(|| bad("program.source must be an object id"))?;
    if !o.get("digest").and_then(Value::as_str).is_some_and(is_hex_digest) {
        return Err(bad("program.digest must be a sha256 hex digest"));
    }
    if o.get("api_version").and_then(Value::as_u64) == Some(PROGRAM_API_VERSION) {
        check_manifest(o.get("manifest").ok_or_else(|| bad("program.manifest required (api_version 3)"))?)?;
    }
    let produces = o.get("produces").and_then(Value::as_array).ok_or_else(|| bad("program.produces must be an array"))?;
    if produces.len() > MAX_CONTRACT_RESULTS || !produces.iter().all(|n| n.as_str().is_some_and(is_result_name)) {
        return Err(bad("program.produces must list result names"));
    }
    for k in ["run_id", "edited_by"] {
        if o.get(k).is_some_and(|x| !text_ok(Some(x), 128)) {
            return Err(bad(format!("program.{k} must be a short string")));
        }
    }
    Ok(source)
}

fn is_hex_digest(d: &str) -> bool {
    d.len() == 64 && d.bytes().all(|c| c.is_ascii_hexdigit())
}

fn str_list(v: Option<&Value>, max: usize, max_chars: usize) -> bool {
    v.is_none_or(|x| x.as_array().is_some_and(|a| a.len() <= max && a.iter().all(|s| text_ok(Some(s), max_chars))))
}

fn positive(v: Option<&Value>) -> bool {
    v.is_none_or(|x| x.as_u64().is_some_and(|n| n > 0))
}

/// The manifest of a macro (第一期 §2.2, PRD §6.6): what the host enforces when it runs.
///
/// ```json
/// { "inputs": { "sales": { "entity_id": "…", "type_id": "buckyos.table-source", "fields": [{ "field_id": "f3", "name": "月份", "type": "date" }] } },
///   "results": { "chart": { "type": "html", "code_digest": "…" }, "monthly": { "type": "table" } },
///   "ai": { "complete": { "max_calls": 20, "max_tokens": 20000, "models": ["llm.chat"] }, "map": { "max_items": 500 } },
///   "zone": ["/kapi/…"], "params": ["today", "timezone"], "resources": { "timeout_secs": 120 } }
/// ```
pub fn check_manifest(v: &Value) -> WsResult<()> {
    let o = v.as_object().ok_or_else(|| bad("manifest must be an object"))?;
    only(o, &["inputs", "results", "ai", "zone", "params", "resources"], "manifest")?;
    let inputs = o.get("inputs").and_then(Value::as_object).ok_or_else(|| bad("manifest.inputs must be an object"))?;
    if inputs.len() > MAX_MANIFEST_INPUTS {
        return Err(WsError::limit(format!("a macro declares at most {MAX_MANIFEST_INPUTS} inputs")));
    }
    for (name, i) in inputs {
        if !is_input_name(name) {
            return Err(bad(format!("manifest input {name:?} is not an input name")));
        }
        let io = i.as_object().ok_or_else(|| bad(format!("manifest input {name} must be an object")))?;
        only(io, &["entity_id", "type_id", "fields"], "manifest input")?;
        if !io.get("entity_id").and_then(Value::as_str).is_some_and(is_valid_id) || !text_ok(io.get("type_id"), 64) {
            return Err(bad(format!("manifest input {name} needs entity_id and type_id")));
        }
        if let Some(fields) = io.get("fields") {
            let list = fields.as_array().filter(|a| a.len() <= MAX_MANIFEST_FIELDS).ok_or_else(|| bad(format!("manifest input {name}: fields must be a list")))?;
            let mut ids = BTreeSet::new();
            for f in list {
                let fo = f.as_object().ok_or_else(|| bad("manifest field must be an object"))?;
                only(fo, &["field_id", "name", "type"], "manifest field")?;
                let id = fo.get("field_id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 128).ok_or_else(|| bad(format!("manifest input {name}: field_id required")))?;
                if !ids.insert(id) || !text_ok(fo.get("name"), 128) || !text_ok(fo.get("type"), 32) {
                    return Err(bad(format!("manifest input {name}: field {id} needs a unique field_id, a name and a type")));
                }
            }
        }
    }
    let results = o.get("results").and_then(Value::as_object).ok_or_else(|| bad("manifest.results must be an object"))?;
    if results.len() > MAX_CONTRACT_RESULTS {
        return Err(WsError::limit(format!("at most {MAX_CONTRACT_RESULTS} results")));
    }
    for (name, r) in results {
        let ro = r.as_object().filter(|_| is_result_name(name)).ok_or_else(|| bad(format!("manifest result {name:?} is invalid")))?;
        only(ro, &["type", "code_digest"], "manifest result")?;
        if !text_ok(ro.get("type"), 32) || ro.get("code_digest").is_some_and(|d| !d.as_str().is_some_and(is_hex_digest)) {
            return Err(bad(format!("manifest result {name}: type, and code_digest a sha256 hex digest")));
        }
    }
    if let Some(ai) = o.get("ai") {
        let ao = ai.as_object().ok_or_else(|| bad("manifest.ai must be an object"))?;
        only(ao, &["complete", "map", "image", "video"], "manifest.ai")?;
        if let Some(c) = ao.get("complete") {
            let co = c.as_object().ok_or_else(|| bad("manifest.ai.complete must be an object"))?;
            only(co, &["max_calls", "max_tokens", "models"], "manifest.ai.complete")?;
            if !positive(co.get("max_calls")) || !positive(co.get("max_tokens")) || !str_list(co.get("models"), 8, 128) {
                return Err(bad("manifest.ai.complete: max_calls and max_tokens are positive, models a list of model names"));
            }
        }
        if let Some(m) = ao.get("map") {
            let mo = m.as_object().ok_or_else(|| bad("manifest.ai.map must be an object"))?;
            only(mo, &["max_items"], "manifest.ai.map")?;
            if !positive(mo.get("max_items")) {
                return Err(bad("manifest.ai.map.max_items must be positive"));
            }
        }
        for kind in ["image", "video"] {
            if let Some(m) = ao.get(kind) {
                let mo = m.as_object().ok_or_else(|| bad(format!("manifest.ai.{kind} must be an object")))?;
                only(mo, &["max_calls", "models"], &format!("manifest.ai.{kind}"))?;
                if !positive(mo.get("max_calls")) || !str_list(mo.get("models"), 8, 128) {
                    return Err(bad(format!("manifest.ai.{kind}: max_calls is positive, models a list of model names")));
                }
            }
        }
    }
    if !str_list(o.get("zone"), 32, 512) || !str_list(o.get("params"), 16, 64) {
        return Err(bad("manifest.zone and manifest.params are lists of short strings"));
    }
    if let Some(r) = o.get("resources") {
        let ro = r.as_object().ok_or_else(|| bad("manifest.resources must be an object"))?;
        only(ro, &["timeout_secs"], "manifest.resources")?;
        if !positive(ro.get("timeout_secs")) {
            return Err(bad("manifest.resources.timeout_secs must be positive"));
        }
    }
    Ok(())
}

/// The digest of a macro version: its source and its manifest (§2.2).
pub fn macro_digest(source: &str, manifest: &Value) -> String {
    digest(&json!({ "source": sha256_hex(source.as_bytes()), "manifest": manifest }))
}

/// Why the stored macro cannot run on the inputs as they are now (第一期 §2.7, PRD §5.7): no macro, a macro
/// of another program API, an input it declares that is gone or bound to something else, a field it reads
/// that was deleted or changed its type. A renamed field or table is no reason: the macro reads by id.
pub fn macro_check(ctx: &dyn crate::model::ReadCtx, payload: &JsonMap) -> WsResult<Vec<Value>> {
    let Some(prog) = payload.get("program").filter(|p| p.is_object()) else { return Ok(vec![json!({ "code": "no_macro" })]) };
    let version = prog.get("api_version").and_then(Value::as_u64);
    if version != Some(PROGRAM_API_VERSION) {
        return Ok(vec![json!({ "code": "api_version", "found": version, "expected": PROGRAM_API_VERSION })]);
    }
    let Some(inputs) = prog.get("manifest").and_then(|m| m.get("inputs")).and_then(Value::as_object) else { return Ok(vec![json!({ "code": "no_macro" })]) };
    let bound: Vec<&Value> = payload.get("inputs").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let mut out = Vec::new();
    for (name, decl) in inputs {
        let id = decl.get("entity_id").and_then(Value::as_str).unwrap_or("");
        match bound.iter().find(|i| i.get("name").and_then(Value::as_str) == Some(name.as_str())) {
            None => {
                out.push(json!({ "code": "input_unbound", "input": name }));
                continue;
            }
            Some(i) if i.get("entity_id").and_then(Value::as_str) != Some(id) => {
                out.push(json!({ "code": "input_changed", "input": name, "entity_id": i.get("entity_id") }));
                continue;
            }
            _ => {}
        }
        let Some(e) = ctx.entity(id)?.filter(|e| e.alive()) else {
            out.push(json!({ "code": "input_missing", "input": name, "entity_id": id }));
            continue;
        };
        let Some(fields) = decl.get("fields").and_then(Value::as_array) else { continue };
        if e.type_id != TYPE_TABLE {
            out.push(json!({ "code": "input_changed", "input": name, "entity_id": id }));
            continue;
        }
        let live = crate::filter::live_fields(ctx, id)?;
        for f in fields {
            let fid = f.get("field_id").and_then(Value::as_str).unwrap_or("");
            let fname = f.get("name").cloned().unwrap_or(Value::Null);
            match live.iter().find(|x| x.field_id == fid) {
                None => out.push(json!({ "code": "field_deleted", "input": name, "field_id": fid, "name": fname })),
                Some(x) if f.get("type").and_then(Value::as_str) != Some(x.def.ty.as_str()) => {
                    out.push(json!({ "code": "field_type_changed", "input": name, "field_id": fid, "name": fname, "from": f.get("type"), "to": x.def.ty.as_str() }))
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

fn check_views(v: &Value) -> WsResult<()> {
    let list = v.as_array().ok_or_else(|| bad("views must be an array"))?;
    if list.len() > MAX_VIEWS {
        return Err(WsError::limit(format!("at most {MAX_VIEWS} views per result")));
    }
    for view in list {
        let o = view.as_object().ok_or_else(|| bad("view must be an object"))?;
        only(o, &["renderer", "config", "title", "size"], "view")?;
        if !o.get("renderer").and_then(Value::as_str).is_some_and(crate::types::is_renderer_id) {
            return Err(bad("view.renderer must be a renderer id"));
        }
        if o.get("config").is_some_and(|c| !c.is_object()) {
            return Err(bad("view.config must be an object"));
        }
        if o.get("title").is_some_and(|t| !text_ok(Some(t), 256)) {
            return Err(bad("view.title must be a short string"));
        }
        if let Some(s) = o.get("size") {
            let dim = |k: &str| s.get(k).and_then(Value::as_f64).is_some_and(|n| n > 0.0 && n <= 20_000.0);
            if !(dim("w") && dim("h")) {
                return Err(bad("view.size must be { w, h }"));
            }
        }
    }
    Ok(())
}

/// One result of an output contract (§7.2).
pub fn check_contract_result(r: &Value) -> WsResult<()> {
    let o = r.as_object().ok_or_else(|| bad("contract result must be an object"))?;
    only(o, &["name", "type", "title", "key", "target", "views", "description", "fields", "kind", "def"], "contract result")?;
    let name = o.get("name").and_then(Value::as_str).filter(|s| is_result_name(s)).ok_or_else(|| bad("contract result needs a valid name"))?;
    let ty = o.get("type").and_then(Value::as_str).unwrap_or("");
    if !RESULT_TYPES.contains(&ty) {
        return Err(bad(format!("result {name}: unknown type {ty}")));
    }
    if o.get("title").is_some_and(|t| !text_ok(Some(t), 256)) || o.get("description").is_some_and(|t| !text_ok(Some(t), 2000)) {
        return Err(bad(format!("result {name}: title / description too long")));
    }
    for k in ["key", "fields"] {
        if let Some(list) = o.get(k) {
            let ok = list.as_array().is_some_and(|a| a.len() <= 64 && a.iter().all(|f| text_ok(Some(f), 128)));
            if !ok {
                return Err(bad(format!("result {name}: {k} must list field names")));
            }
        }
    }
    if ty == "table_columns" && !o.get("target").and_then(Value::as_str).is_some_and(is_input_name) {
        return Err(bad(format!("result {name}: table_columns needs target (an input name)")));
    }
    if ty != "table_columns" && o.contains_key("target") {
        return Err(bad(format!("result {name}: target is only valid on table_columns")));
    }
    match o.get("kind") {
        Some(k) if ty != "control" || !k.as_str().is_some_and(crate::types::is_control_kind) => {
            return Err(bad(format!("result {name}: kind names the control of a control result (catalog/controls.json)")));
        }
        _ => {}
    }
    // a composite result names its definition (扩展控件 P3): `<author_did>#<name>`
    if let Some(d) = o.get("def") {
        let ok = ty == "control" && o.get("kind").is_none_or(|k| k == crate::types::COMPOSITE) && d.as_str().and_then(|d| d.split_once('#')).is_some_and(|(a, n)| crate::types::is_author_did(a) && crate::types::is_renderer_id(n));
        if !ok {
            return Err(bad(format!("result {name}: def names a composite control of the Workspace (<author_did>#<name>) on a control result")));
        }
    }
    if ty == "control" && o.contains_key("views") {
        return Err(bad(format!("result {name}: a control result is drawn by its control and has no views")));
    }
    if let Some(v) = o.get("views") {
        check_views(v)?;
    }
    Ok(())
}

/// The persisted form of `wish.analysis.v3`. `basis.digest` is filled by `with_basis`, never trusted.
pub fn check_analysis(v: &Value) -> WsResult<()> {
    let o = v.as_object().ok_or_else(|| bad("analysis must be an object"))?;
    only(
        o,
        &["schema_version", "status", "prompt", "context_prompt", "output_contract", "checks", "blockers", "warnings", "basis", "run_id", "at", "executor", "summary"],
        "analysis",
    )?;
    if o.get("schema_version").and_then(Value::as_str) != Some(ANALYSIS_SCHEMA) {
        return Err(bad(format!("analysis.schema_version must be {ANALYSIS_SCHEMA}")));
    }
    if !matches!(o.get("status").and_then(Value::as_str), Some("ready" | "needs_input")) {
        return Err(bad("analysis.status must be ready or needs_input"));
    }
    if !text_ok(o.get("prompt"), crate::types::MAX_PROMPT_CHARS) {
        return Err(bad("analysis.prompt (the prompt it was derived from) required"));
    }
    if !text_ok(o.get("context_prompt"), crate::types::MAX_PROMPT_CHARS * 2) {
        return Err(bad("analysis.context_prompt required"));
    }
    for k in ["run_id", "at", "executor"] {
        if o.get(k).is_some_and(|x| !text_ok(Some(x), 128)) {
            return Err(bad(format!("analysis.{k} must be a short string")));
        }
    }
    if o.get("summary").is_some_and(|x| !text_ok(Some(x), 4000)) {
        return Err(bad("analysis.summary too long"));
    }
    let contract = o.get("output_contract").and_then(Value::as_object).ok_or_else(|| bad("analysis.output_contract required"))?;
    only(contract, &["results", "placement", "dynamic"], "output_contract")?;
    let results = contract.get("results").and_then(Value::as_array).ok_or_else(|| bad("output_contract.results must be an array"))?;
    if results.len() > MAX_CONTRACT_RESULTS {
        return Err(WsError::limit(format!("at most {MAX_CONTRACT_RESULTS} results")));
    }
    let mut names = BTreeSet::new();
    for r in results {
        check_contract_result(r)?;
        if !names.insert(r["name"].as_str().unwrap_or("")) {
            return Err(bad(format!("duplicate result name {}", r["name"])));
        }
    }
    if contract.get("placement").is_some_and(|p| !text_ok(Some(p), 128)) {
        return Err(bad("output_contract.placement must be a short string"));
    }
    if contract.get("dynamic").is_some_and(|d| !d.is_boolean()) {
        return Err(bad("output_contract.dynamic must be a boolean"));
    }
    let checks = o.get("checks").and_then(Value::as_array).ok_or_else(|| bad("analysis.checks must be an array"))?;
    if checks.len() > MAX_CHECKS {
        return Err(WsError::limit(format!("at most {MAX_CHECKS} checks")));
    }
    let mut ids = BTreeSet::new();
    for c in checks {
        let co = c.as_object().ok_or_else(|| bad("check must be an object"))?;
        only(co, &["id", "kind", "text"], "check")?;
        let id = co.get("id").and_then(Value::as_str).filter(|s| is_check_id(s)).ok_or_else(|| bad("check.id must be an identifier"))?;
        if !ids.insert(id) {
            return Err(bad(format!("duplicate check id {id}")));
        }
        if !matches!(co.get("kind").and_then(Value::as_str), Some("program" | "review")) {
            return Err(bad("check.kind must be program or review"));
        }
        if !text_ok(co.get("text"), 1000) {
            return Err(bad("check.text required"));
        }
    }
    let blockers = o.get("blockers").and_then(Value::as_array).ok_or_else(|| bad("analysis.blockers must be an array"))?;
    for b in blockers {
        let bo = b.as_object().ok_or_else(|| bad("blocker must be an object"))?;
        only(bo, &["code", "message", "input_label", "candidates"], "blocker")?;
        if !text_ok(bo.get("code"), 64) || !text_ok(bo.get("message"), 2000) || bo.get("input_label").is_some_and(|l| !text_ok(Some(l), 256)) {
            return Err(bad("blocker needs code and message"));
        }
        if let Some(c) = bo.get("candidates") {
            if !c.as_array().is_some_and(|a| a.len() <= 20 && a.iter().all(|x| x.as_str().is_some_and(is_valid_id))) {
                return Err(bad("blocker.candidates must list entity ids"));
            }
        }
    }
    if o.get("status").and_then(Value::as_str) == Some("ready") && !blockers.is_empty() {
        return Err(bad("an analysis with blockers cannot be ready"));
    }
    if !o.get("warnings").and_then(Value::as_array).is_some_and(|w| w.len() <= 64 && w.iter().all(|x| text_ok(Some(x), 2000))) {
        return Err(bad("analysis.warnings must be a list of strings"));
    }
    if let Some(b) = o.get("basis") {
        let bo = b.as_object().ok_or_else(|| bad("analysis.basis must be an object"))?;
        only(bo, &["digest", "reads"], "analysis.basis")?;
        if let Some(r) = bo.get("reads") {
            if !r.as_array().is_some_and(|a| a.len() <= 200 && a.iter().all(Value::is_object)) {
                return Err(bad("analysis.basis.reads must be a list of version cells"));
            }
        }
    }
    Ok(())
}

/// The fields of one input that decide what it means (label, appended_by and versions do not).
fn input_essence(i: &Value) -> Value {
    json!({
        "entity_id": i.get("entity_id"),
        "name": i.get("name"),
        "selector": i.get("selector"),
        "mode": i.get("version").and_then(|v| v.get("mode")).cloned().unwrap_or(json!("follow")),
        "object_id": i.get("version").and_then(|v| v.get("object_id")),
    })
}

fn digest(v: &Value) -> String {
    sha256_hex(canonical_json(v).unwrap_or_default().as_bytes())
}

/// Digest of what an analysis was made from (§4.1 `basis`).
pub fn basis_digest(payload: &JsonMap) -> String {
    let refinements: Vec<Value> = payload
        .get("refinements")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|r| r.get("text").cloned().unwrap_or(Value::Null)).collect())
        .unwrap_or_default();
    let inputs: Vec<Value> = payload.get("inputs").and_then(Value::as_array).map(|a| a.iter().map(input_essence).collect()).unwrap_or_default();
    let a = payload.get("analysis");
    let mut contract = a.and_then(|a| a.get("output_contract")).cloned().unwrap_or(Value::Null);
    if let Some(c) = contract.as_object_mut() {
        c.remove("placement");
    }
    digest(&json!({
        "prompt": payload.get("prompt"),
        "knowledge": payload.get("knowledge"),
        "refinements": refinements,
        "executor": payload.get("executor"),
        "executor_config": payload.get("executor_config"),
        "inputs": inputs,
        "context_prompt": a.and_then(|a| a.get("context_prompt")),
        "contract": contract,
        "checks": a.and_then(|a| a.get("checks")),
        "reads": a.and_then(|a| a.get("basis")).and_then(|b| b.get("reads")),
    }))
}

/// Digest of the configuration results are generated under (§4.3 `config_digest`): the basis and the macro.
pub fn config_digest(payload: &JsonMap) -> String {
    let program = payload.get("program").and_then(|p| p.get("digest")).cloned().unwrap_or(Value::Null);
    digest(&json!({ "basis": basis_digest(payload), "program": program }))[..32].to_string()
}

/// Set `analysis.basis.digest` from the payload as it is now (called when `analysis` was written).
pub fn with_basis(payload: &mut JsonMap) {
    let d = basis_digest(payload);
    if let Some(a) = payload.get_mut("analysis").and_then(Value::as_object_mut) {
        let basis = a.entry("basis").or_insert_with(|| json!({}));
        if let Some(b) = basis.as_object_mut() {
            b.insert("digest".into(), json!(d));
        }
    }
}

/// Whether the wish must be analysed (again) before it can be coded or run.
pub fn needs_analysis(payload: &JsonMap) -> bool {
    let Some(a) = payload.get("analysis").filter(|a| a.is_object()) else { return true };
    let recorded = a.get("basis").and_then(|b| b.get("digest")).and_then(Value::as_str);
    recorded != Some(basis_digest(payload).as_str())
}

/// The results of the current result group (`last_run.result_bindings.results`): `(name, binding)`.
pub fn current_results(payload: &JsonMap) -> Vec<(String, Value)> {
    payload
        .get("last_run")
        .and_then(|r| r.get("result_bindings"))
        .and_then(|b| b.get("results"))
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// A stable record id for a table result row from its logical key values (§10.2: the mapping is
/// rebuilt from the key, never stored row by row).
pub fn record_id_for_key(key_values: &[Value]) -> String {
    let h = digest(&Value::Array(key_values.to_vec()));
    format!("k{}", &h[..30])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> JsonMap {
        json!({
            "prompt": "p", "executor": "xllm", "knowledge": "k",
            "inputs": [{ "entity_id": "sales", "name": "sales", "label": "Orders", "version": { "mode": "follow", "rev": 3 } }],
            "analysis": { "schema_version": ANALYSIS_SCHEMA, "status": "ready", "prompt": "p", "context_prompt": "c",
                          "output_contract": { "results": [{ "name": "monthly", "type": "table", "key": ["month"] }], "placement": "right_of_wish" },
                          "checks": [{ "id": "total", "kind": "program", "text": "totals match" }], "blockers": [], "warnings": [] },
            "output_mode": "overwrite",
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    #[test]
    fn basis_follows_meaning_not_bookkeeping() {
        let mut p = payload();
        check_analysis(&p["analysis"]).unwrap();
        with_basis(&mut p);
        assert!(!needs_analysis(&p));
        let before = config_digest(&p);
        // label, recorded versions, placement, output mode and last run are bookkeeping
        p["inputs"][0]["label"] = json!("new name");
        p["inputs"][0]["version"]["rev"] = json!(9);
        p["analysis"]["output_contract"]["placement"] = json!("below_wish");
        p.insert("output_mode".into(), json!("new"));
        p.insert("last_run".into(), json!({ "run_id": "r" }));
        assert!(!needs_analysis(&p));
        assert_eq!(config_digest(&p), before);
        // meaning: knowledge, refinements, inputs, executor configuration
        for (k, v) in [("knowledge", json!("k2")), ("refinements", json!([{ "text": "split out East China" }])), ("executor_config", json!({ "model": "x" }))] {
            let mut q = p.clone();
            q.insert(k.into(), v);
            assert!(needs_analysis(&q), "{k}");
        }
        let mut q = p.clone();
        q["inputs"][0]["selector"] = json!({ "kind": "table_view", "cell_id": "blk" });
        assert!(needs_analysis(&q));
        // the program changes the configuration, not the analysis
        let mut q = p.clone();
        q.insert("program".into(), json!({ "digest": "a".repeat(64) }));
        assert!(!needs_analysis(&q));
        assert_ne!(config_digest(&q), before);
    }

    #[test]
    fn analysis_shape() {
        let p = payload();
        let mut a = p["analysis"].clone();
        a["output_contract"]["results"][0]["approach"] = json!("program");
        assert!(check_analysis(&a).is_err(), "v3 has no approach: every result comes from the macro");
        let mut a = p["analysis"].clone();
        a["blockers"] = json!([{ "code": "MISSING_INPUT", "message": "no sales table found" }]);
        assert!(check_analysis(&a).is_err(), "ready with blockers");
        a["status"] = json!("needs_input");
        check_analysis(&a).unwrap();
        let mut a = p["analysis"].clone();
        a["output_contract"]["results"][0]["name"] = json!("../x");
        assert!(check_analysis(&a).is_err());
        // i18n-exempt: result names may be Chinese (user input)
        assert!(is_result_name("月度汇总") && !is_result_name("a/b") && !is_result_name(" a"));
        // i18n-exempt: input names must be ASCII identifiers, a Chinese one is refused
        assert!(is_input_name("sales_2") && !is_input_name("2sales") && !is_input_name("销售"));
    }

    #[test]
    fn record_ids_from_keys_are_stable_ids() {
        let a = record_id_for_key(&[json!("2026-07")]);
        assert_eq!(a, record_id_for_key(&[json!("2026-07")]));
        assert_ne!(a, record_id_for_key(&[json!("2026-08")]));
        assert!(is_valid_id(&a));
    }

    // ---- macros (AI Native 第一期 §2.2, §2.7)

    /// The manifest shape of `check_manifest`'s documentation, every optional part present.
    fn manifest() -> Value {
        json!({
            "inputs": {
                "sales": { "entity_id": "orders", "type_id": TYPE_TABLE,
                           "fields": [{ "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "n", "name": "数量", "type": "number" }] }, // i18n-exempt: test data (user-written names and text)
                "rules": { "entity_id": "rules", "type_id": crate::model::TYPE_RICHTEXT }
            },
            "results": { "chart": { "type": "html", "code_digest": "a".repeat(64) }, "monthly": { "type": "table" } },
            "ai": { "complete": { "max_calls": 20, "max_tokens": 20000, "models": ["llm.chat"] }, "map": { "max_items": 500 },
                    "image": { "max_calls": 6, "models": ["image.txt2img.gemini"] }, "video": { "max_calls": 1 } },
            "zone": ["/kapi/x"], "params": ["today", "timezone"], "resources": { "timeout_secs": 120 }
        })
    }

    #[test]
    fn manifest_shape() {
        check_manifest(&manifest()).unwrap();
        // only inputs and results are required; an input without fields is a whole entity
        check_manifest(&json!({ "inputs": {}, "results": {} })).unwrap();
        check_manifest(&json!({ "inputs": { "doc": { "entity_id": "d1", "type_id": "buckyos.richtext" } }, "results": {} })).unwrap();
        let bad = |edit: &dyn Fn(&mut Value), why: &str| {
            let mut m = manifest();
            edit(&mut m);
            assert!(check_manifest(&m).is_err(), "{why}: {m}");
        };
        bad(&|m| *m = json!([]), "not an object");
        bad(&|m| m["extra"] = json!(1), "unknown key");
        bad(&|m| { m.as_object_mut().unwrap().remove("inputs"); }, "inputs required");
        bad(&|m| { m.as_object_mut().unwrap().remove("results"); }, "results required");
        bad(&|m| m["inputs"] = json!([]), "inputs is a map by input name");
        // i18n-exempt: an input name must be an ASCII identifier, a Chinese one is refused
        bad(&|m| m["inputs"]["销售"] = m["inputs"]["sales"].clone(), "input name");
        bad(&|m| { m["inputs"]["sales"].as_object_mut().unwrap().remove("entity_id"); }, "entity_id required");
        bad(&|m| m["inputs"]["sales"]["entity_id"] = json!("not an id!"), "entity_id is an id");
        bad(&|m| { m["inputs"]["sales"].as_object_mut().unwrap().remove("type_id"); }, "type_id required");
        bad(&|m| m["inputs"]["sales"]["selector"] = json!({}), "unknown input key");
        bad(&|m| m["inputs"]["sales"]["fields"] = json!({ "month": "date" }), "fields is a list");
        bad(&|m| m["inputs"]["sales"]["fields"][1]["field_id"] = json!("month"), "duplicate field_id");
        bad(&|m| { m["inputs"]["sales"]["fields"][0].as_object_mut().unwrap().remove("name"); }, "field name required");
        bad(&|m| { m["inputs"]["sales"]["fields"][0].as_object_mut().unwrap().remove("type"); }, "field type required");
        bad(&|m| m["inputs"]["sales"]["fields"][0]["field_id"] = json!(""), "empty field_id");
        bad(&|m| m["inputs"]["sales"]["fields"][0]["options"] = json!([]), "unknown field key");
        bad(&|m| m["results"]["../x"] = json!({ "type": "table" }), "result name");
        bad(&|m| m["results"]["chart"]["code_digest"] = json!("abc"), "code_digest is a sha256 hex digest");
        bad(&|m| { m["results"]["monthly"].as_object_mut().unwrap().remove("type"); }, "result type required");
        bad(&|m| m["results"]["monthly"]["views"] = json!([]), "unknown result key");
        bad(&|m| m["ai"]["complete"]["max_calls"] = json!(0), "max_calls positive");
        bad(&|m| m["ai"]["complete"]["max_tokens"] = json!(-1), "max_tokens positive");
        bad(&|m| m["ai"]["complete"]["models"] = json!("llm.chat"), "models is a list");
        bad(&|m| m["ai"]["map"]["max_items"] = json!(0), "max_items positive");
        bad(&|m| m["ai"]["agent"] = json!({}), "no agent loop in a macro");
        bad(&|m| m["ai"]["image"]["max_calls"] = json!(0), "image max_calls positive");
        bad(&|m| m["ai"]["video"]["models"] = json!("video.generate"), "video models is a list");
        bad(&|m| m["ai"]["image"]["max_tokens"] = json!(10), "unknown image key");
        bad(&|m| m["zone"] = json!("/kapi/x"), "zone is a list");
        bad(&|m| m["params"] = json!([1]), "params are strings");
        bad(&|m| m["resources"]["timeout_secs"] = json!(0), "timeout positive");
        bad(&|m| m["resources"]["memory"] = json!(1), "unknown resource");
        // too many inputs is a limit, not a schema error
        let mut m = json!({ "inputs": {}, "results": {} });
        for i in 0..=MAX_MANIFEST_INPUTS {
            m["inputs"][format!("in{i}")] = json!({ "entity_id": format!("t{i}"), "type_id": TYPE_TABLE });
        }
        assert_eq!(check_manifest(&m).unwrap_err().code, crate::Code::LimitExceeded);
    }

    #[test]
    fn program_v3_carries_a_manifest_and_its_digest_covers_it() {
        let source = format!("mix256:{}", "c".repeat(64));
        let mut p = json!({ "language": "js", "api_version": PROGRAM_API_VERSION, "source": source, "digest": "b".repeat(64), "manifest": manifest(), "produces": ["monthly"] });
        assert_eq!(check_program(&p).unwrap(), source);
        let mut no_manifest = p.clone();
        no_manifest.as_object_mut().unwrap().remove("manifest");
        assert!(check_program(&no_manifest).is_err(), "api_version 3 needs its manifest");
        p["manifest"]["inputs"]["销售"] = json!({ "entity_id": "x", "type_id": TYPE_TABLE }); // i18n-exempt: test data (user-written names and text)
        assert!(check_program(&p).is_err(), "the manifest is checked with the program");
        // a macro of an older API still travels (the structure check asks for coding again)
        no_manifest["api_version"] = json!(2);
        check_program(&no_manifest).unwrap();
        no_manifest["api_version"] = json!(0);
        assert!(check_program(&no_manifest).is_err());
        // the macro version is its source and its manifest
        let d = macro_digest("main()", &manifest());
        assert_eq!(d, macro_digest("main()", &manifest()));
        assert_ne!(d, macro_digest("main() ", &manifest()));
        let mut m = manifest();
        m["inputs"]["sales"]["fields"][0]["name"] = json!("月"); // i18n-exempt: test data (user-written names and text)
        assert_ne!(d, macro_digest("main()", &m));
        assert!(is_hex_digest(&d));
    }

    /// The plain text of a manifest (a wish's body): what it reads under which names, what it writes, its AI and
    /// Zone allowances, its parameters.
    #[test]
    fn manifest_as_text() {
        let t = crate::body::manifest_text(&manifest());
        assert_eq!(
            t,
            [
                "macro:",
                "- reads rules",
                "- reads sales: 月份, 数量", // i18n-exempt: test data (user-written names and text)
                "- writes chart (html), monthly (table)",
                "- AI: single inferences, at most 20 calls and 20000 tokens",
                "- AI: per-item judgements, at most 500 items",
                "- AI: generated images, at most 6",
                "- AI: generated videos, at most 1",
                "- Zone resources: /kapi/x",
                "- parameters: today, timezone",
            ]
            .join("\n")
        );
        // limits left to the Workspace are "default"; nothing declared, nothing listed
        let t = crate::body::manifest_text(&json!({ "inputs": {}, "results": {}, "ai": { "complete": { "max_calls": 3 }, "map": {} } }));
        assert_eq!(t, "macro:\n- AI: single inferences, at most 3 calls and default tokens\n- AI: per-item judgements, at most default items");
        let t = crate::body::manifest_text(&json!({ "inputs": {}, "results": {}, "ai": { "image": { "max_calls": 6 }, "video": {} } }));
        assert_eq!(t, "macro:\n- AI: generated images, at most 6\n- AI: generated videos, at most default");
        assert_eq!(crate::body::manifest_text(&json!({ "inputs": {}, "results": {}, "zone": [], "params": [] })), "macro:");
    }

    /// A Workspace with the table `orders` (月份 date, 数量 number, 客户 text) and the rich text `rules`.
    fn orders() -> crate::testkit::MemWorkspace {
        use crate::model::{DATA_ID, TYPE_RICHTEXT};
        let mut ws = crate::testkit::MemWorkspace::new();
        ws.ok(json!([
            { "op": "entity.create", "entity_id": "orders", "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": "a", "name": "订单", // i18n-exempt: test data (user-written names and text)
              "payload": { "fields": [{ "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "n", "name": "数量", "type": "number" }, // i18n-exempt: test data (user-written names and text)
                                      { "field_id": "customer", "name": "客户", "type": "text" }] } }, // i18n-exempt: test data (user-written names and text)
            { "op": "entity.create", "entity_id": "other", "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": "b", "name": "其他", // i18n-exempt: test data (user-written names and text)
              "payload": { "fields": [{ "field_id": "month", "name": "月份", "type": "date" }] } }, // i18n-exempt: test data (user-written names and text)
            { "op": "entity.create", "entity_id": "rules", "type_id": TYPE_RICHTEXT, "parent_id": DATA_ID, "order_key": "c", "name": "口径", // i18n-exempt: test data (user-written names and text)
              "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "p1" } }] } } },
        ]));
        ws
    }

    /// A wish payload bound to `orders` (as `sales`) and `rules`, whose macro declares 月份 and 数量 of `sales`.
    fn coded() -> JsonMap {
        let m = json!({ "inputs": {
                            "sales": { "entity_id": "orders", "type_id": TYPE_TABLE,
                                       "fields": [{ "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "n", "name": "数量", "type": "number" }] }, // i18n-exempt: test data (user-written names and text)
                            "rules": { "entity_id": "rules", "type_id": crate::model::TYPE_RICHTEXT } },
                        "results": { "monthly": { "type": "table" } } });
        json!({
            "prompt": "p", "executor": "xllm",
            "inputs": [{ "entity_id": "orders", "name": "sales" }, { "entity_id": "rules", "name": "rules" }],
            "program": { "language": "js", "api_version": PROGRAM_API_VERSION, "source": format!("mix256:{}", "c".repeat(64)),
                         "digest": macro_digest("main()", &m), "manifest": m, "produces": ["monthly"] },
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    fn codes(r: &[Value]) -> Vec<&str> {
        r.iter().map(|x| x["code"].as_str().unwrap_or("")).collect()
    }

    fn def_rev(ws: &crate::testkit::MemWorkspace, field: &str) -> u64 {
        ws.store.fields[&("orders".to_string(), field.to_string())].def_rev
    }

    #[test]
    fn macro_check_reasons() {
        let mut ws = orders();
        let p = coded();
        assert_eq!(macro_check(&ws.store, &p).unwrap(), Vec::<Value>::new(), "a macro that fits its inputs");
        // no macro at all, or one without a manifest
        let mut none = p.clone();
        none.remove("program");
        assert_eq!(macro_check(&ws.store, &none).unwrap(), vec![json!({ "code": "no_macro" })]);
        let mut null = p.clone();
        null.insert("program".into(), Value::Null);
        assert_eq!(codes(&macro_check(&ws.store, &null).unwrap()), ["no_macro"]);
        let mut bare = p.clone();
        bare["program"].as_object_mut().unwrap().remove("manifest");
        assert_eq!(codes(&macro_check(&ws.store, &bare).unwrap()), ["no_macro"]);
        // a macro for program API 2 (no declare, no SQL): coded again, nothing else is looked at
        let mut v2 = p.clone();
        v2["program"]["api_version"] = json!(2);
        v2["inputs"] = json!([]);
        assert_eq!(macro_check(&ws.store, &v2).unwrap(), vec![json!({ "code": "api_version", "found": 2, "expected": PROGRAM_API_VERSION })]);
        // the input name now binds another table, or nothing
        let mut rebound = p.clone();
        rebound["inputs"][0]["entity_id"] = json!("other");
        assert_eq!(macro_check(&ws.store, &rebound).unwrap(), vec![json!({ "code": "input_changed", "input": "sales", "entity_id": "other" })]);
        let mut unbound = p.clone();
        unbound["inputs"][1]["name"] = json!("notes");
        assert_eq!(macro_check(&ws.store, &unbound).unwrap(), vec![json!({ "code": "input_unbound", "input": "rules" })]);

        // a rename is no reason: the macro reads by id
        ws.ok(json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "month", "changes": { "name": "统计月份" }, "expect": { "rev": def_rev(&ws, "month") } }])); // i18n-exempt: test data (user-written names and text)
        let meta = ws.store.entities["orders"].meta_rev;
        ws.ok(json!([{ "op": "entity.rename", "entity_id": "orders", "name": "订单2026", "expect": { "rev": meta } }])); // i18n-exempt: test data (user-written names and text)
        assert_eq!(macro_check(&ws.store, &p).unwrap(), Vec::<Value>::new(), "renamed field and table");
        // a field it does not declare changing is no reason either
        let rev = ws.store.fields[&("orders".to_string(), "customer".to_string())].def_rev;
        ws.ok(json!([{ "op": "table.delete_field", "source_id": "orders", "field_id": "customer", "expect": { "rev": rev } }]));
        assert_eq!(macro_check(&ws.store, &p).unwrap(), Vec::<Value>::new(), "an undeclared field deleted");

        // a declared field changing its type
        ws.ok(json!([{ "op": "table.migrate_field", "source_id": "orders", "field_id": "n", "expect": { "rev": def_rev(&ws, "n") }, "to": { "type": "decimal", "scale": 2 } }]));
        assert_eq!(macro_check(&ws.store, &p).unwrap(),
                   vec![json!({ "code": "field_type_changed", "input": "sales", "field_id": "n", "name": "数量", "from": "number", "to": "decimal" })]); // i18n-exempt: test data (user-written names and text)
        // a declared field deleted (named as the macro knew it)
        ws.ok(json!([{ "op": "table.delete_field", "source_id": "orders", "field_id": "month", "expect": { "rev": def_rev(&ws, "month") } }]));
        let r = macro_check(&ws.store, &p).unwrap();
        assert_eq!(codes(&r), ["field_deleted", "field_type_changed"], "{r:?}");
        assert_eq!(r[0], json!({ "code": "field_deleted", "input": "sales", "field_id": "month", "name": "月份" })); // i18n-exempt: test data (user-written names and text)

        // the input itself deleted; an input declared without fields only has to exist
        let life = ws.store.entities["orders"].life_rev;
        ws.ok(json!([{ "op": "entity.delete", "entity_id": "orders", "expect": { "rev": life } }]));
        assert_eq!(macro_check(&ws.store, &p).unwrap(), vec![json!({ "code": "input_missing", "input": "sales", "entity_id": "orders" })]);
        let life = ws.store.entities["rules"].life_rev;
        ws.ok(json!([{ "op": "entity.delete", "entity_id": "rules", "expect": { "rev": life } }]));
        assert_eq!(codes(&macro_check(&ws.store, &p).unwrap()), ["input_missing", "input_missing"]);
    }
}
