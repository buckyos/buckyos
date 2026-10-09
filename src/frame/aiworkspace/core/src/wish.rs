//! Wish protocol v2 (许愿格详细设计 §4, §7.2, §8.5): the persisted shapes of `knowledge`,
//! `refinements`, `program` and `analysis` (`wish.analysis.v2`), and the two digests the core,
//! the service and the browser replica share:
//!
//! - the **analysis basis**: what an analysis was made from (prompt, knowledge, refinements,
//!   executor and its configuration, input bindings, the analysis itself). Computed by the core
//!   whenever `analysis` is written, never taken from the writer; a different value now means
//!   "needs analysis".
//! - the **config digest**: the basis plus the program. Results record it; a different value now
//!   means the result was generated under another configuration.
//!
//! Run-local handles never reach these shapes: everything persisted uses real ids.

use crate::canonical::{canonical_json, is_obj_id, sha256_hex};
use crate::error::{WsError, WsResult};
use crate::id::is_valid_id;
use crate::model::JsonMap;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const ANALYSIS_SCHEMA: &str = "wish.analysis.v2";
pub const RESULTS_SCHEMA: &str = "wish.results.v2";
pub const PROGRAM_API_VERSION: u64 = 2;
pub const MAX_KNOWLEDGE_CHARS: usize = 20_000;
pub const MAX_REFINEMENTS: usize = 50;
pub const MAX_REFINEMENT_CHARS: usize = 2000;
pub const MAX_CONTRACT_RESULTS: usize = 32;
pub const MAX_CHECKS: usize = 32;
pub const MAX_VIEWS: usize = 4;
/// Result types of `wish.results.v2`.
pub const RESULT_TYPES: &[&str] = &["richtext", "record", "table", "table_columns", "image", "asset", "html", "video"];
/// Result types only a program may produce (§8.2: counting, aggregating and rows come from code).
pub const PROGRAM_ONLY_TYPES: &[&str] = &["table", "table_columns"];

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

/// `{ language: "js", api_version: 2, source: <object id>, digest, produces: [names], run_id?, size? }`.
/// Returns the source object id (an uploaded text asset).
pub fn check_program(v: &Value) -> WsResult<&str> {
    let o = v.as_object().ok_or_else(|| bad("program must be an object"))?;
    only(o, &["language", "api_version", "source", "digest", "produces", "run_id", "size", "edited_by"], "program")?;
    if o.get("language").and_then(Value::as_str) != Some("js") {
        return Err(bad("program.language must be js"));
    }
    // format only: a program for another `aiws` version still travels; the runner decides whether it runs
    if o.get("api_version").and_then(Value::as_u64).is_none_or(|n| n == 0) {
        return Err(bad("program.api_version must be a positive integer"));
    }
    let source = o.get("source").and_then(Value::as_str).filter(|s| is_obj_id(s)).ok_or_else(|| bad("program.source must be an object id"))?;
    if !o.get("digest").and_then(Value::as_str).is_some_and(|d| d.len() == 64 && d.bytes().all(|c| c.is_ascii_hexdigit())) {
        return Err(bad("program.digest must be a sha256 hex digest"));
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
    only(o, &["name", "type", "title", "approach", "key", "target", "views", "description", "fields"], "contract result")?;
    let name = o.get("name").and_then(Value::as_str).filter(|s| is_result_name(s)).ok_or_else(|| bad("contract result needs a valid name"))?;
    let ty = o.get("type").and_then(Value::as_str).unwrap_or("");
    if !RESULT_TYPES.contains(&ty) {
        return Err(bad(format!("result {name}: unknown type {ty}")));
    }
    match o.get("approach").and_then(Value::as_str) {
        Some("program") => {}
        Some("direct") if PROGRAM_ONLY_TYPES.contains(&ty) => return Err(bad(format!("result {name}: {ty} results must be produced by a program"))),
        Some("direct") => {}
        _ => return Err(bad(format!("result {name}: approach must be program or direct"))),
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
    if let Some(v) = o.get("views") {
        check_views(v)?;
    }
    Ok(())
}

/// The persisted form of `wish.analysis.v2`. `basis.digest` is filled by `with_basis`, never trusted.
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

/// Digest of the configuration results are generated under (§4.3 `config_digest`).
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

/// Whether the wish must be analysed (again) before it can execute.
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
            "inputs": [{ "entity_id": "sales", "name": "sales", "label": "订单", "version": { "mode": "follow", "rev": 3 } }],
            "analysis": { "schema_version": ANALYSIS_SCHEMA, "status": "ready", "prompt": "p", "context_prompt": "c",
                          "output_contract": { "results": [{ "name": "monthly", "type": "table", "approach": "program", "key": ["月份"] }], "placement": "right_of_wish" },
                          "checks": [{ "id": "total", "kind": "program", "text": "合计一致" }], "blockers": [], "warnings": [] },
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
        p["inputs"][0]["label"] = json!("新名字");
        p["inputs"][0]["version"]["rev"] = json!(9);
        p["analysis"]["output_contract"]["placement"] = json!("below_wish");
        p.insert("output_mode".into(), json!("new"));
        p.insert("last_run".into(), json!({ "run_id": "r" }));
        assert!(!needs_analysis(&p));
        assert_eq!(config_digest(&p), before);
        // meaning: knowledge, refinements, inputs, executor configuration
        for (k, v) in [("knowledge", json!("k2")), ("refinements", json!([{ "text": "拆分华东" }])), ("executor_config", json!({ "model": "x" }))] {
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
        a["output_contract"]["results"][0]["approach"] = json!("direct");
        assert!(check_analysis(&a).is_err(), "tables come from programs");
        let mut a = p["analysis"].clone();
        a["blockers"] = json!([{ "code": "MISSING_INPUT", "message": "找不到销售表" }]);
        assert!(check_analysis(&a).is_err(), "ready with blockers");
        a["status"] = json!("needs_input");
        check_analysis(&a).unwrap();
        let mut a = p["analysis"].clone();
        a["output_contract"]["results"][0]["name"] = json!("../x");
        assert!(check_analysis(&a).is_err());
        assert!(is_result_name("月度汇总") && !is_result_name("a/b") && !is_result_name(" a"));
        assert!(is_input_name("sales_2") && !is_input_name("2sales") && !is_input_name("销售"));
    }

    #[test]
    fn record_ids_from_keys_are_stable_ids() {
        let a = record_id_for_key(&[json!("2026-07")]);
        assert_eq!(a, record_id_for_key(&[json!("2026-07")]));
        assert_ne!(a, record_id_for_key(&[json!("2026-08")]));
        assert!(is_valid_id(&a));
    }
}
