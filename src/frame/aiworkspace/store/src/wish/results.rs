//! Result collection and validation (许愿格 §8.4–§8.7): what a program run writes and what the model
//! submits with `put_result` become `wish.results.v2` results, checked on the spot against the output
//! contract and the production rules, with a preview the model reads in the same run. The collector
//! also holds the checks, the facts, and the final summary, and assembles the candidate.

use aiworkspace_core::wish::{is_check_id, is_input_name, is_result_name, PROGRAM_ONLY_TYPES, RESULTS_SCHEMA};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const MAX_TABLE_ROWS: usize = 50_000;
pub const MAX_TABLE_FIELDS: usize = 100;
pub const MAX_DIRECT_CHARS: usize = 20_000;
/// A written (direct) table-like answer above this many rows must come from a program (§8.2).
pub const DIRECT_ROWS_LIMIT: usize = 20;
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_HTML_BYTES: usize = 256 * 1024;
const PREVIEW_ROWS: usize = 5;

#[derive(Debug, Clone, Default)]
pub struct ProgramRun {
    pub results: Vec<Value>,
    pub facts: Value,
    pub checks: Vec<Value>,
    pub external: Vec<String>,
    pub llm_map: Value,
    pub source: String,
    pub digest: String,
    pub logs: String,
}

#[derive(Debug, Clone)]
pub struct InputInfo {
    pub entity_id: String,
    pub type_id: String,
    /// table inputs: record ids present in the snapshot
    pub records: Option<BTreeSet<String>>,
    /// a control or a Block laid out by one (a mind map node): the control's kind
    pub control: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Collector {
    pub contract: Value,
    pub checks_def: Vec<Value>,
    pub program: Option<ProgramRun>,
    pub direct: BTreeMap<String, Value>,
    pub prev_bindings: BTreeMap<String, Value>,
    pub inputs: BTreeMap<String, InputInfo>,
    pub workdir: PathBuf,
    pub finished: Option<Value>,
    /// Program results stay; written results are kept as they are (program re-run).
    pub program_only: bool,
}

fn contract_results(contract: &Value) -> Vec<Value> {
    contract.get("results").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn is_datetime(s: &str) -> bool {
    s.len() >= 16 && is_date(&s[..10]) && matches!(s.as_bytes()[10], b'T' | b' ')
}

/// Field type of a column from its values (declared types win).
pub fn infer_type<'a>(values: impl Iterator<Item = &'a Value>) -> &'static str {
    let (mut n, mut nums, mut bools, mut dates, mut dts, mut arrays) = (0, 0, 0, 0, 0, 0);
    for v in values {
        match v {
            Value::Null => continue,
            Value::Number(_) => nums += 1,
            Value::Bool(_) => bools += 1,
            Value::String(s) if is_date(s) => dates += 1,
            Value::String(s) if is_datetime(s) => dts += 1,
            Value::Array(a) if a.iter().all(Value::is_string) => arrays += 1,
            _ => {}
        }
        n += 1;
    }
    if n == 0 {
        return "text";
    }
    if nums == n {
        "number"
    } else if bools == n {
        "boolean"
    } else if dates == n {
        "date"
    } else if dts + dates == n && dts > 0 {
        "datetime"
    } else if arrays == n {
        "multi_select"
    } else {
        "text"
    }
}

const FIELD_TYPES: &[&str] = &["text", "number", "decimal", "boolean", "date", "datetime", "select", "multi_select"];

/// `{ name: type }` or `[{ name, type, options?, scale? }]` → `[{ name, type, options?, scale? }]`.
fn declared_fields(v: Option<&Value>) -> Result<Vec<Value>, String> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, t)| match t {
                Value::String(t) => Ok(json!({ "name": k, "type": t })),
                Value::Object(o) => {
                    let mut f = Value::Object(o.clone());
                    f["name"] = json!(k);
                    Ok(f)
                }
                _ => Err(format!("Field {k} has an invalid type declaration")),
            })
            .collect(),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(_) => Err("fields must be { field name: type } or [{ name, type }]".into()),
    }
}

pub fn check_value(ty: &str, v: &Value) -> bool {
    match (ty, v) {
        (_, Value::Null) => true,
        ("number" | "decimal", Value::Number(n)) => n.as_f64().is_some_and(f64::is_finite),
        ("number" | "decimal", Value::String(s)) => super::profile::numeric_text(s).is_some(),
        ("boolean", Value::Bool(_)) => true,
        ("date", Value::String(s)) => is_date(s) || is_datetime(s),
        ("datetime", Value::String(s)) => is_datetime(s) || is_date(s),
        ("select", Value::String(_)) => true,
        ("multi_select", Value::Array(a)) => a.iter().all(Value::is_string),
        ("multi_select", Value::String(_)) => true,
        ("text", _) => true,
        _ => false,
    }
}

/// Sniff a file's media type and image size from its bytes.
pub fn sniff_file(bytes: &[u8]) -> (String, Option<(u64, u64)>) {
    let media = crate::objects::sniff_media_type(bytes).to_string();
    let dims = match media.as_str() {
        "image/png" if bytes.len() >= 24 => Some((u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as u64, u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as u64)),
        "image/svg+xml" => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_string();
            let attr = |n: &str| -> Option<u64> {
                let i = head.find(&format!("{n}=\""))? + n.len() + 2;
                let rest = &head[i..];
                rest[..rest.find('"')?].trim_end_matches("px").parse::<f64>().ok().map(|f| f.round() as u64)
            };
            let viewbox = || -> Option<(u64, u64)> {
                let i = head.find("viewBox=\"")? + 9;
                let rest = &head[i..];
                let parts: Vec<f64> = rest[..rest.find('"')?].split([' ', ',']).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
                (parts.len() == 4).then(|| (parts[2].round() as u64, parts[3].round() as u64))
            };
            match (attr("width"), attr("height")) {
                (Some(w), Some(h)) => Some((w, h)),
                _ => viewbox(),
            }
        }
        _ => None,
    };
    (media, dims.filter(|(w, h)| *w > 0 && *h > 0 && *w <= 1 << 20 && *h <= 1 << 20))
}

/// All numbers written in a text (with separators, decimals, percent).
pub fn numbers_in(text: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '.' || chars[i - 1] == '_' || chars[i - 1] == '-' && i >= 2 && chars[i - 2].is_ascii_alphanumeric())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ',' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() || chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) {
                i += 1;
            }
            // dates, versions and identifiers are not quantities
            let next = chars.get(i).copied();
            let raw: String = chars[start..i].iter().collect();
            if matches!(next, Some('-') | Some('/') | Some(':')) && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '-' | '/' | ':')) {
                    i += 1;
                }
                continue;
            }
            if let Ok(n) = raw.replace(',', "").parse::<f64>() {
                out.push((raw, n));
            }
            continue;
        }
        i += 1;
    }
    out
}

fn collect_numbers(v: &Value, out: &mut Vec<f64>) {
    match v {
        Value::Number(n) => out.extend(n.as_f64()),
        Value::String(s) => out.extend(numbers_in(s).into_iter().map(|(_, n)| n)),
        Value::Array(a) => a.iter().for_each(|x| collect_numbers(x, out)),
        Value::Object(o) => o.values().for_each(|x| collect_numbers(x, out)),
        _ => {}
    }
}

/// Whether `n` as written matches a known value (rounded the way it was written, or as a percentage).
fn cited(raw: &str, n: f64, known: &[f64]) -> bool {
    let decimals = raw.split('.').nth(1).map_or(0, str::len) as i32;
    let tol = 0.5 * 10f64.powi(-decimals) + 1e-9;
    known.iter().any(|k| (k - n).abs() <= tol || (k * 100.0 - n).abs() <= tol || ((k / 10_000.0) - n).abs() <= tol || ((k / 1e8) - n).abs() <= tol || ((k / 1000.0) - n).abs() <= tol)
}

impl Collector {
    pub fn contract_for(&self, name: &str) -> Option<Value> {
        contract_results(&self.contract).into_iter().find(|r| r["name"] == json!(name))
    }

    fn dynamic(&self) -> bool {
        self.contract.get("dynamic") == Some(&json!(true))
    }

    /// Resolve a path under `output/` (relative to the work directory); anything else is refused.
    pub fn output_path(&self, p: &str) -> Result<PathBuf, String> {
        let rel = Path::new(p);
        let joined = if rel.is_absolute() { rel.to_path_buf() } else { self.workdir.join(rel) };
        let real = joined.canonicalize().map_err(|_| format!("File {p} does not exist"))?;
        let out = self.workdir.join("output").canonicalize().map_err(|e| e.to_string())?;
        if !real.starts_with(&out) {
            return Err(format!("File {p} is not under output/: only files in output/ can become results"));
        }
        let meta = std::fs::metadata(&real).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err(format!("{p} is not a file"));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!("File {p} exceeds {} MiB", MAX_FILE_BYTES / 1024 / 1024));
        }
        Ok(real)
    }

    /// Normalize one raw result (program or `put_result`) into `wish.results.v2`.
    pub fn normalize(&self, raw: &Value, approach: &str) -> Result<Value, Vec<String>> {
        let mut errs = Vec::new();
        let name = raw["name"].as_str().unwrap_or("").to_string();
        if !is_result_name(&name) {
            return Err(vec![format!("Result name \"{name}\" is invalid: 1–64 characters, without path separators, colons or ..")]);
        }
        let contract = self.contract_for(&name);
        if contract.is_none() && !self.dynamic() {
            let names: Vec<String> = contract_results(&self.contract).iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect();
            return Err(vec![format!("Result \"{name}\" is not in the output contract (contracted results: {}): do not extend the task; to get a new result, revise the analysis first", names.join(", "))]);
        }
        let kind = raw["type"].as_str().unwrap_or("");
        let ty = match kind {
            "table" => "table",
            "columns" | "table_columns" => "table_columns",
            "record" => "record",
            "text" | "richtext" | "markdown" => "richtext",
            "file" | "image" | "asset" => "file",
            "html" => "html",
            "video" => "video",
            "control" => "control",
            other => return Err(vec![format!("Result \"{name}\" has unsupported type {other}")]),
        };
        let contract_ty = contract.as_ref().and_then(|c| c["type"].as_str()).unwrap_or("").to_string();
        let contract_approach = contract.as_ref().and_then(|c| c["approach"].as_str()).unwrap_or("").to_string();
        if approach == "direct" {
            if PROGRAM_ONLY_TYPES.contains(&ty) {
                return Err(vec![format!("Result \"{name}\" is of type {ty}: tables and derived columns can only be produced by a program (output them in program/main.js with aiws.result.table / aiws.result.columns, then call run_program)")]);
            }
            if contract_approach == "program" {
                return Err(vec![format!("Result \"{name}\" is contracted to be produced by the program: output it in the program and call run_program instead of writing it directly with put_result")]);
            }
        }
        let mut out = json!({ "name": name, "approach": approach });
        if let Some(c) = &contract {
            out["title"] = c.get("title").cloned().unwrap_or(json!(name));
            if let Some(v) = c.get("views") {
                out["views"] = v.clone();
            }
        }
        if let Some(t) = raw.get("title").and_then(Value::as_str) {
            out["title"] = json!(t.chars().take(256).collect::<String>());
        }
        if let Some(v) = raw.get("views").filter(|v| v.is_array()) {
            out["views"] = v.clone();
        }
        let final_ty = match ty {
            "table" => {
                let rows = raw["rows"].as_array().cloned().unwrap_or_default();
                if rows.len() > MAX_TABLE_ROWS {
                    errs.push(format!("Result \"{name}\" has {} rows, over the limit of {MAX_TABLE_ROWS} rows: narrow the range or aggregate", rows.len()));
                }
                let mut declared = declared_fields(raw.get("fields")).map_err(|e| vec![e])?;
                let mut order: Vec<String> = declared.iter().filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
                for r in &rows {
                    let Some(o) = r.as_object() else {
                        errs.push(format!("Each row of result \"{name}\" must be a {{ field name: value }} object"));
                        break;
                    };
                    for k in o.keys() {
                        if k != "_id" && !order.contains(k) {
                            order.push(k.clone());
                        }
                    }
                }
                if order.len() > MAX_TABLE_FIELDS {
                    errs.push(format!("Result \"{name}\" has {} fields, over the limit of {MAX_TABLE_FIELDS}", order.len()));
                }
                let mut fields = Vec::new();
                for fname in &order {
                    if fname.is_empty() || fname.chars().count() > 128 {
                        errs.push(format!("Result \"{name}\" has an invalid field name \"{fname}\""));
                        continue;
                    }
                    let mut f = declared.iter().find(|f| f["name"] == json!(fname)).cloned().unwrap_or_else(|| json!({ "name": fname }));
                    let t = match f["type"].as_str() {
                        Some(t) if FIELD_TYPES.contains(&t) => t.to_string(),
                        Some(t) => {
                            errs.push(format!("Field \"{fname}\" of result \"{name}\" has unsupported type {t} ({})", FIELD_TYPES.join("/")));
                            continue;
                        }
                        None => infer_type(rows.iter().filter_map(|r| r.get(fname))).to_string(),
                    };
                    let bad: Vec<String> = rows.iter().filter_map(|r| r.get(fname)).filter(|v| !check_value(&t, v)).take(3).map(|v| v.to_string()).collect();
                    if !bad.is_empty() {
                        errs.push(format!("Field \"{fname}\" of result \"{name}\" is declared as {t}, but some values do not match: {}", bad.join(", ")));
                    }
                    f["type"] = json!(t);
                    fields.push(f);
                }
                declared.clear();
                let key: Vec<String> = match raw.get("key").or_else(|| contract.as_ref().and_then(|c| c.get("key"))) {
                    Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                    Some(Value::String(s)) => vec![s.clone()],
                    _ => Vec::new(),
                };
                if key.is_empty() && approach == "program" {
                    errs.push(format!("Result \"{name}\" needs a key (one or more logical key columns), e.g. aiws.result.table('{name}', rows, {{ key: ['month'] }})"));
                }
                for k in &key {
                    if !order.contains(k) {
                        errs.push(format!("Key field \"{k}\" of result \"{name}\" is not in the rows"));
                    }
                }
                if !key.is_empty() && errs.is_empty() {
                    let mut seen = BTreeSet::new();
                    for r in &rows {
                        let kv: Vec<Value> = key.iter().map(|k| r.get(k).cloned().unwrap_or(Value::Null)).collect();
                        if kv.iter().any(Value::is_null) {
                            errs.push(format!("Result \"{name}\" has a row with an empty key field: {}", r));
                            break;
                        }
                        if !seen.insert(serde_json::to_string(&kv).unwrap_or_default()) {
                            errs.push(format!("Result \"{name}\" has duplicate values for key {}: {}", key.join("+"), serde_json::to_string(&kv).unwrap_or_default()));
                            break;
                        }
                    }
                }
                let clean: Vec<Value> = rows
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|o| Value::Object(o.iter().filter(|(k, _)| *k != "_id").map(|(k, v)| (k.clone(), v.clone())).collect()))
                    .collect();
                out["table"] = json!({ "fields": fields, "key": key, "rows": clean });
                "table"
            }
            "table_columns" => {
                let input = raw["input"].as_str().or_else(|| contract.as_ref().and_then(|c| c["target"].as_str())).unwrap_or("").to_string();
                let Some(info) = self.inputs.get(&input).filter(|i| i.type_id == aiworkspace_core::model::TYPE_TABLE) else {
                    return Err(vec![format!("The target {input} of derived-column result \"{name}\" is not a table input")]);
                };
                if contract.as_ref().and_then(|c| c["target"].as_str()).is_some_and(|t| t != input) {
                    errs.push(format!("Derived-column result \"{name}\" is contracted to write to {}, not {input}", contract.as_ref().unwrap()["target"]));
                }
                let values = raw["values"].as_object().cloned().unwrap_or_default();
                let declared = declared_fields(raw.get("fields")).map_err(|e| vec![e])?;
                let mut order: Vec<String> = declared.iter().filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
                let mut unknown = 0;
                for (rid, v) in &values {
                    if info.records.as_ref().is_some_and(|r| !r.contains(rid)) {
                        unknown += 1;
                    }
                    match v {
                        Value::Object(o) => {
                            for k in o.keys() {
                                if !order.contains(k) {
                                    order.push(k.clone());
                                }
                            }
                        }
                        _ => {
                            errs.push(format!("The values of derived-column result \"{name}\" must be {{ record_id: {{ column name: value }} }}"));
                            break;
                        }
                    }
                }
                if unknown > 0 {
                    errs.push(format!("Derived-column result \"{name}\" has {unknown} record ids that are not in input {input} (a row's _id comes from aiws.input('{input}').rows())"));
                }
                let mut fields = Vec::new();
                for fname in &order {
                    let mut f = declared.iter().find(|f| f["name"] == json!(fname)).cloned().unwrap_or_else(|| json!({ "name": fname }));
                    if f["type"].as_str().is_none() {
                        f["type"] = json!(infer_type(values.values().filter_map(|v| v.get(fname))));
                    }
                    fields.push(f);
                }
                out["columns"] = json!({ "target": input, "target_id": info.entity_id, "fields": fields, "values": values });
                "table_columns"
            }
            "record" => {
                let props = raw["props"].as_object().cloned().unwrap_or_default();
                if props.len() > 64 {
                    errs.push(format!("Record \"{name}\" has more than 64 properties"));
                }
                let declared = declared_fields(raw.get("schema").or_else(|| raw.get("fields"))).map_err(|e| vec![e])?;
                let properties: Vec<Value> = props
                    .iter()
                    .map(|(k, v)| {
                        let d = declared.iter().find(|f| f["name"] == json!(k)).cloned();
                        let t = d.as_ref().and_then(|d| d["type"].as_str()).map(str::to_string).unwrap_or_else(|| infer_type(std::iter::once(v)).to_string());
                        let mut p = json!({ "name": k, "type": t });
                        if let Some(o) = d.as_ref().and_then(|d| d.get("options")) {
                            p["options"] = o.clone();
                        } else if t == "select" {
                            p["options"] = json!([v]);
                        }
                        p
                    })
                    .collect();
                out["record"] = json!({ "properties": properties, "props": props });
                "record"
            }
            "richtext" => {
                let md = raw["markdown"].as_str().or_else(|| raw["text"].as_str()).unwrap_or("");
                if md.trim().is_empty() {
                    errs.push(format!("Text result \"{name}\" is empty"));
                }
                if approach == "direct" && md.chars().count() > MAX_DIRECT_CHARS {
                    errs.push(format!("Text result \"{name}\" exceeds {MAX_DIRECT_CHARS} characters: generate long content with a program"));
                }
                if approach == "direct" {
                    // a written table of many rows is data, not text
                    let table_lines = md.lines().filter(|l| l.trim_start().starts_with('|')).count();
                    if table_lines > DIRECT_ROWS_LIMIT + 2 {
                        errs.push(format!("Text result \"{name}\" contains {table_lines} table lines: structured results of more than {DIRECT_ROWS_LIMIT} rows must be produced by a program (aiws.result.table)"));
                    }
                }
                out["markdown"] = json!(md);
                "richtext"
            }
            "file" => {
                let p = raw["path"].as_str().unwrap_or("");
                match self.output_path(p) {
                    Ok(real) => {
                        let bytes = std::fs::read(&real).unwrap_or_default();
                        let (media, dims) = sniff_file(&bytes);
                        let mut f = json!({ "path": real.to_string_lossy(), "media_type": media, "size": bytes.len(),
                                            "file_name": real.file_name().map(|n| n.to_string_lossy().to_string()), "digest": aiworkspace_core::canonical::sha256_hex(&bytes) });
                        if let Some((w, h)) = dims {
                            f["image"] = json!({ "width": w, "height": h });
                        }
                        if let Some(d) = raw.get("media_type").and_then(Value::as_str) {
                            if !media.starts_with("image/") && d.starts_with("image/") {
                                errs.push(format!("File {p} is declared as {d}, but its content is {media}"));
                            }
                        }
                        out["file"] = f;
                    }
                    Err(e) => errs.push(e),
                }
                if out["file"]["media_type"].as_str().is_some_and(|m| m.starts_with("image/")) {
                    "image"
                } else {
                    "asset"
                }
            }
            "html" => {
                let html = raw["html"].as_str().unwrap_or("");
                if html.is_empty() {
                    errs.push(format!("HTML result \"{name}\" needs html"));
                }
                let size = html.len() + raw["css"].as_str().map_or(0, str::len) + raw["js"].as_str().map_or(0, str::len);
                if size > MAX_HTML_BYTES {
                    errs.push(format!("HTML result \"{name}\" exceeds {} KiB", MAX_HTML_BYTES / 1024));
                }
                let mut bindings = Map::new();
                for (k, v) in raw["bindings"].as_object().into_iter().flatten() {
                    let t = v.as_str().unwrap_or("");
                    let ok = match t.split_once(':') {
                        Some(("input", n)) => self.inputs.contains_key(n),
                        Some(("result", n)) => is_result_name(n),
                        _ => false,
                    };
                    if !is_input_name(k) || k == "source" {
                        errs.push(format!("Binding name {k} of HTML result \"{name}\" must be an identifier"));
                    } else if !ok {
                        errs.push(format!("Binding {k} → {t} of HTML result \"{name}\" is invalid (use input:<input name> or result:<result name>)"));
                    } else {
                        bindings.insert(k.clone(), json!(t));
                    }
                }
                out["html"] = json!({ "html": html, "css": raw.get("css").cloned().unwrap_or(json!("")), "js": raw.get("js").cloned().unwrap_or(json!("")), "bindings": bindings });
                "html"
            }
            "control" => {
                self.control(&name, raw, contract.as_ref(), &mut out, &mut errs);
                "control"
            }
            _ => {
                errs.push("video results need a real media tool and are not supported in the first version".into());
                "video"
            }
        };
        if !contract_ty.is_empty() && contract_ty != final_ty && !(contract_ty == "asset" && final_ty == "image" || contract_ty == "image" && final_ty == "asset") {
            errs.push(format!("Result \"{name}\" is contracted as {contract_ty}, but it is {final_ty}"));
        }
        out["type"] = json!(final_ty);
        if errs.is_empty() {
            Ok(out)
        } else {
            Err(errs)
        }
    }

    /// A control result (脑图 TODO M3): `{ kind, config?, children: <Markdown outline> | [{ text, children }], under? }`
    /// → `control: { kind, config, nodes: [{ text, children }], under?, under_id? }`. The tree is bounded and its
    /// titles fit the core; `under` names an input that is a control or one of its nodes.
    fn control(&self, name: &str, raw: &Value, contract: Option<&Value>, out: &mut Value, errs: &mut Vec<String>) {
        let kind = raw["kind"].as_str().unwrap_or("");
        if !aiworkspace_core::types::is_control_kind(kind) {
            errs.push(format!("Control result \"{name}\" needs kind, a control of catalog/controls.json (not {kind:?})"));
            return;
        }
        if let Some(k) = contract.and_then(|c| c["kind"].as_str()).filter(|k| *k != kind) {
            errs.push(format!("Control result \"{name}\" is contracted as a {k}, not a {kind}"));
        }
        let mut config = Map::new();
        match raw.get("config") {
            None | Some(Value::Null) => {}
            Some(Value::Object(c)) => {
                config = c.clone();
                // the attributes a control defines; the core checks their values again
                if kind == "mindmap" {
                    config.retain(|k, _| matches!(k.as_str(), "direction" | "edge" | "spacing" | "branch_colors"));
                }
                let size = aiworkspace_core::canonical::canonical_json(&Value::Object(config.clone())).map_or(usize::MAX, |s| s.len());
                if size > aiworkspace_core::types::MAX_CONFIG_BYTES {
                    errs.push(format!("The config of control result \"{name}\" exceeds {} KiB", aiworkspace_core::types::MAX_CONFIG_BYTES / 1024));
                } else if let Err(e) = aiworkspace_core::types::check_control_config(kind, &config) {
                    errs.push(format!("Control result \"{name}\": {}", e.detail));
                }
            }
            Some(_) => errs.push(format!("The config of control result \"{name}\" must be an object")),
        }
        let tree = match super::outline::result_tree(raw.get("children")) {
            Ok(t) => t,
            Err(e) => {
                errs.push(format!("Control result \"{name}\" {e}"));
                Vec::new()
            }
        };
        let mut c = json!({ "kind": kind, "config": config, "nodes": tree.iter().map(super::outline::OutlineNode::to_json).collect::<Vec<_>>() });
        match raw.get("under").filter(|u| !u.is_null()) {
            None => {}
            Some(u) => match u.as_str().and_then(|n| self.inputs.get(n).map(|i| (n, i))) {
                Some((n, i)) if i.control.as_deref() == Some(kind) => {
                    c["under"] = json!(n);
                    c["under_id"] = json!(i.entity_id);
                }
                Some((n, _)) => errs.push(format!("under: input {n} of control result \"{name}\" is not a {kind} or one of its nodes")),
                None => errs.push(format!("under: {u} of control result \"{name}\" is not an input name")),
            },
        }
        out["control"] = c;
    }

    /// The results now held (program first), by name.
    pub fn all(&self) -> Vec<Value> {
        let mut v: Vec<Value> = self.program.as_ref().map(|p| p.results.clone()).unwrap_or_default();
        for (n, r) in &self.direct {
            if !v.iter().any(|x| x["name"] == json!(n)) {
                v.push(r.clone());
            }
        }
        v
    }

    /// One result as the model and the UI see it before application.
    pub fn preview(&self, r: &Value) -> Value {
        let name = r["name"].as_str().unwrap_or("");
        let prev = self.prev_bindings.get(name);
        let mut p = json!({ "name": name, "type": r["type"], "title": r["title"], "approach": r["approach"], "views": r.get("views").cloned().unwrap_or(json!([])) });
        match r["type"].as_str().unwrap_or("") {
            "table" => {
                let t = &r["table"];
                let rows = t["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                p["rows"] = json!(rows.len());
                p["fields"] = json!(t["fields"].as_array().into_iter().flatten().map(|f| format!("{}:{}", f["name"].as_str().unwrap_or(""), f["type"].as_str().unwrap_or(""))).collect::<Vec<_>>());
                p["key"] = t["key"].clone();
                p["first_rows"] = json!(rows.iter().take(PREVIEW_ROWS).collect::<Vec<_>>());
                if let Some(b) = prev {
                    let before: BTreeSet<String> = b["fields"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                    let after: BTreeSet<String> = t["fields"].as_array().into_iter().flatten().filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
                    let added: Vec<&String> = after.difference(&before).collect();
                    let removed: Vec<&String> = before.difference(&after).collect();
                    let mut diff = json!({});
                    if !added.is_empty() {
                        diff["added_fields"] = json!(added);
                    }
                    if !removed.is_empty() {
                        diff["removed_fields"] = json!(removed);
                    }
                    if b.get("key").is_some_and(|k| *k != t["key"]) {
                        diff["key_changed"] = json!({ "from": b["key"], "to": t["key"] });
                    }
                    p["vs_last_run"] = diff;
                }
            }
            "table_columns" => {
                let c = &r["columns"];
                p["target"] = c["target"].clone();
                p["fields"] = json!(c["fields"].as_array().into_iter().flatten().map(|f| format!("{}:{}", f["name"].as_str().unwrap_or(""), f["type"].as_str().unwrap_or(""))).collect::<Vec<_>>());
                p["records"] = json!(c["values"].as_object().map_or(0, Map::len));
                p["first_values"] = json!(c["values"].as_object().into_iter().flatten().take(PREVIEW_ROWS).map(|(k, v)| json!({ "_id": k, "values": v })).collect::<Vec<_>>());
            }
            "record" => p["props"] = r["record"]["props"].clone(),
            "richtext" => {
                let md = r["markdown"].as_str().unwrap_or("");
                p["chars"] = json!(md.chars().count());
                p["excerpt"] = json!(md.chars().take(400).collect::<String>());
            }
            "image" | "asset" => p["file"] = json!({ "media_type": r["file"]["media_type"], "size": r["file"]["size"], "image": r["file"].get("image") }),
            "html" => p["html"] = json!({ "bytes": r["html"]["html"].as_str().map_or(0, str::len), "bindings": r["html"]["bindings"] }),
            "control" => {
                let c = &r["control"];
                let tree = super::outline::tree_of(&c["nodes"]);
                p["control"] = json!({ "kind": c["kind"], "config": c["config"], "nodes": super::outline::count(&tree), "under": c.get("under"),
                                       "outline": super::outline::format_markdown_outline(&tree).chars().take(400).collect::<String>() });
            }
            _ => {}
        }
        if let Some(b) = prev {
            if b["type"].as_str().is_some_and(|t| t != r["type"].as_str().unwrap_or("")) {
                p["type_changed"] = json!({ "from": b["type"], "to": r["type"] });
            }
        }
        p
    }

    /// Checks: program checks as reported, the rest as defined (review / not run).
    pub fn checks(&self) -> Vec<Value> {
        let reported: BTreeMap<String, Value> = self.program.as_ref().map(|p| p.checks.iter().filter_map(|c| c["id"].as_str().map(|id| (id.to_string(), c.clone()))).collect()).unwrap_or_default();
        let review: BTreeMap<String, Value> = self
            .finished
            .as_ref()
            .and_then(|f| f.get("review_notes"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|n| n["id"].as_str().map(|id| (id.to_string(), n.clone()))).collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        for d in &self.checks_def {
            let id = d["id"].as_str().unwrap_or("");
            let mut c = json!({ "id": id, "kind": d["kind"], "text": d["text"] });
            match d["kind"].as_str() {
                Some("program") => match reported.get(id) {
                    Some(r) => {
                        c["status"] = json!(if r["passed"] == json!(true) { "passed" } else { "failed" });
                        if let Some(det) = r.get("detail") {
                            c["detail"] = det.clone();
                        }
                    }
                    None => c["status"] = json!("not_run"),
                },
                _ => {
                    c["status"] = json!("review");
                    if let Some(n) = review.get(id) {
                        c["self_assessment"] = n.get("note").or_else(|| n.get("text")).cloned().unwrap_or(n.clone());
                    }
                }
            }
            out.push(c);
        }
        // checks the program reported beyond the contract are shown too
        for (id, r) in &reported {
            if !self.checks_def.iter().any(|d| d["id"] == json!(id)) {
                out.push(json!({ "id": id, "kind": "program", "text": r.get("detail").cloned().unwrap_or(json!(id)), "status": if r["passed"] == json!(true) { "passed" } else { "failed" }, "extra": true }));
            }
        }
        out
    }

    /// Numbers in written results that cannot be found in the facts or in program results (§8.7).
    pub fn uncited_numbers(&self, prompt_texts: &[&str]) -> Vec<Value> {
        let mut known = Vec::new();
        if let Some(p) = &self.program {
            collect_numbers(&p.facts, &mut known);
            for r in &p.results {
                collect_numbers(&r["table"]["rows"], &mut known);
                collect_numbers(&r["columns"]["values"], &mut known);
                collect_numbers(&r["record"]["props"], &mut known);
            }
        }
        let mut given = Vec::new();
        for t in prompt_texts {
            given.extend(numbers_in(t).into_iter().map(|(_, n)| n));
        }
        let mut out = Vec::new();
        for (name, r) in &self.direct {
            let text = match r["type"].as_str() {
                Some("richtext") => r["markdown"].as_str().unwrap_or("").to_string(),
                Some("record") => r["record"]["props"].to_string(),
                _ => continue,
            };
            let mut missing: Vec<String> = Vec::new();
            for (raw, n) in numbers_in(&text) {
                let trivial = n.fract() == 0.0 && (0.0..=31.0).contains(&n) || (1900.0..=2100.0).contains(&n) && n.fract() == 0.0;
                if trivial || given.iter().any(|g| (g - n).abs() < 1e-9) || cited(&raw, n, &known) {
                    continue;
                }
                if !missing.contains(&raw) {
                    missing.push(raw);
                }
            }
            if !missing.is_empty() {
                out.push(json!({ "result": name, "numbers": missing }));
            }
        }
        out
    }

    /// Host-level problems that block `finish` (§8.4).
    pub fn blocking(&self) -> Vec<String> {
        let mut out = Vec::new();
        let have: BTreeSet<String> = self.all().iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect();
        for c in contract_results(&self.contract) {
            let n = c["name"].as_str().unwrap_or("");
            if self.program_only && c["approach"] == json!("direct") {
                continue;
            }
            if !have.contains(n) {
                out.push(format!("Contracted result \"{n}\" has not been produced yet ({})", if c["approach"] == json!("program") { "output it from the program, then call run_program" } else { "submit it with put_result" }));
            }
        }
        if !self.program_only {
            for c in self.checks() {
                if c["kind"] == json!("program") && c["status"] == json!("not_run") && c.get("extra").is_none() {
                    out.push(format!("Program check \"{}\" was not run: call aiws.check('{}', passed, detail) in the program", c["id"].as_str().unwrap_or(""), c["id"].as_str().unwrap_or("")));
                }
            }
            let needs_program = contract_results(&self.contract).iter().any(|c| c["approach"] == json!("program")) || self.checks_def.iter().any(|c| c["kind"] == json!("program"));
            if needs_program && self.program.is_none() {
                out.push("The program has not run successfully yet: call run_program first".into());
            }
        }
        out
    }

    /// The candidate (`wish.results.v2`), without the stage evidence the caller adds.
    pub fn candidate(&self) -> Value {
        let results: Vec<Value> = self.all();
        let model_judgment: Vec<Value> = match &self.program {
            Some(p) if p.llm_map.get("items").and_then(Value::as_u64).unwrap_or(0) > 0 => results.iter().filter(|r| r["approach"] == json!("program")).map(|r| r["name"].clone()).collect(),
            _ => Vec::new(),
        };
        let fin = self.finished.clone().unwrap_or(json!({}));
        json!({
            "schema_version": RESULTS_SCHEMA,
            "results": results,
            "facts": self.program.as_ref().map(|p| p.facts.clone()).unwrap_or(json!({})),
            "checks": self.checks(),
            "external_data": self.program.as_ref().map(|p| p.external.clone()).unwrap_or_default(),
            "model_judgment": model_judgment,
            "llm_map": self.program.as_ref().map(|p| p.llm_map.clone()).unwrap_or(Value::Null),
            "summary": fin.get("summary").cloned().unwrap_or(json!("")),
            "assumptions": fin.get("assumptions").cloned().unwrap_or(json!([])),
            "warnings": fin.get("warnings").cloned().unwrap_or(json!([])),
            "review_notes": fin.get("review_notes").cloned().unwrap_or(json!([])),
            "placement": self.contract.get("placement").cloned().unwrap_or(Value::Null),
        })
    }
}

/// Validate a program's `output/.aiws/results.json` into a `ProgramRun` (results normalized).
pub fn program_output(c: &Collector, raw: &Value, source: &str, logs: &str) -> Result<ProgramRun, Vec<String>> {
    let mut errs = Vec::new();
    let mut results = Vec::new();
    let mut names = BTreeSet::new();
    for r in raw["results"].as_array().into_iter().flatten() {
        let n = r["name"].as_str().unwrap_or("").to_string();
        if !names.insert(n.clone()) {
            errs.push(format!("The program output result \"{n}\" twice"));
            continue;
        }
        if c.direct.contains_key(&n) {
            errs.push(format!("Result \"{n}\" was already submitted with put_result: name conflict"));
            continue;
        }
        match c.normalize(r, "program") {
            Ok(v) => results.push(v),
            Err(e) => errs.extend(e),
        }
    }
    let mut checks = Vec::new();
    for ch in raw["checks"].as_array().into_iter().flatten() {
        let id = ch["id"].as_str().unwrap_or("");
        if !is_check_id(id) {
            errs.push(format!("Check id \"{id}\" is invalid"));
            continue;
        }
        checks.push(json!({ "id": id, "passed": ch["passed"] == json!(true), "detail": ch.get("detail").cloned().unwrap_or(Value::Null) }));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(ProgramRun {
        results,
        facts: raw.get("facts").cloned().unwrap_or(json!({})),
        checks,
        external: raw["external"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
        llm_map: raw.get("llm_map").cloned().unwrap_or(Value::Null),
        source: source.to_string(),
        digest: aiworkspace_core::canonical::sha256_hex(source.as_bytes()),
        logs: logs.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_citations() {
        // i18n-exempt: numbers in Chinese prose (the model's written results) are input
        let n: Vec<String> = numbers_in("本季度销售额 1,234.5 元，环比 12.3%，2026-09-30 截止，v2 版本，第3季度").into_iter().map(|(r, _)| r).collect();
        assert_eq!(n, ["1,234.5", "12.3", "3"]);
        assert!(cited("12.3", 12.3, &[0.1234]));
        assert!(cited("1,234.5", 1234.5, &[1234.49]));
        assert!(!cited("1,300", 1300.0, &[1234.5]));
        assert!(cited("12.3", 12.3, &[123_000.0]), "12.3 × 10,000");
        assert!(!cited("12.4", 12.4, &[123_000.0, 0.5]));
    }

    #[test]
    fn inference() {
        assert_eq!(infer_type([json!(1), json!(2.5), Value::Null].iter()), "number");
        assert_eq!(infer_type([json!("2026-07-01"), json!("2026-08-01")].iter()), "date");
        assert_eq!(infer_type([json!("a"), json!(1)].iter()), "text");
        let (m, d) = sniff_file(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360"></svg>"#);
        assert_eq!((m.as_str(), d), ("image/svg+xml", Some((640, 360))));
    }
}
