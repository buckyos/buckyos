//! Controlled filter AST, sorting and table queries (design doc §2.5.4, §3.5.3).
//! This Rust evaluator is the semantic authority; any SQL pushdown must be
//! proven row-for-row equal to it.

use crate::canonical::{canonical_json, sha256_hex};
use crate::error::{Code, WsError, WsResult};
use crate::model::{EntityRow, FieldRow, JsonMap, ReadCtx, RecordRow};
use crate::value::{self, FieldDef, FieldType};
use base64::Engine;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;

pub const MAX_DEPTH: usize = 5;
pub const MAX_NODES: usize = 64;
pub const MAX_ARGS: usize = 32;
pub const MAX_SORTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
    StartsWith,
    In,
    HasAny,
    HasAll,
    IsEmpty,
    IsNotEmpty,
}

impl Operator {
    fn parse(s: &str) -> Option<Operator> {
        Some(match s {
            "eq" => Operator::Eq,
            "ne" => Operator::Ne,
            "lt" => Operator::Lt,
            "lte" => Operator::Lte,
            "gt" => Operator::Gt,
            "gte" => Operator::Gte,
            "contains" => Operator::Contains,
            "starts_with" => Operator::StartsWith,
            "in" => Operator::In,
            "has_any" => Operator::HasAny,
            "has_all" => Operator::HasAll,
            "is_empty" => Operator::IsEmpty,
            "is_not_empty" => Operator::IsNotEmpty,
            _ => return None,
        })
    }

    /// The operator × type matrix of §3.5.3.
    fn allowed(&self, t: FieldType) -> bool {
        use FieldType::*;
        match self {
            Operator::Eq | Operator::Ne => !matches!(t, MultiSelect),
            Operator::Lt | Operator::Lte | Operator::Gt | Operator::Gte => {
                matches!(t, Number | Decimal | Date | Datetime)
            }
            Operator::Contains | Operator::StartsWith => matches!(t, Text),
            Operator::In => matches!(t, Text | Number | Decimal | Date | Datetime | Select),
            Operator::HasAny | Operator::HasAll => matches!(t, MultiSelect),
            Operator::IsEmpty | Operator::IsNotEmpty => true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Compiled {
    And(Vec<Compiled>),
    Or(Vec<Compiled>),
    Not(Box<Compiled>),
    Cmp { def: FieldDef, op: Operator, value: Value },
}

fn view_broken(detail: String, field_id: &str, code: &str) -> WsError {
    WsError::sub(Code::InvalidOperation, "VIEW_BROKEN", detail)
        .with_data(json!({ "code": code, "field_id": field_id }))
}

fn norm_operand(def: &FieldDef, v: &Value) -> WsResult<Value> {
    if v.is_null() {
        return Err(WsError::invalid_op("filter value must not be null; use is_empty"));
    }
    match def.ty {
        FieldType::ObjectRef => match v {
            Value::String(s) => Ok(json!({ "entity_id": s })),
            _ => value::normalize_reference(v).map_err(|e| WsError::invalid_op(e.detail)),
        },
        FieldType::Select => match v.as_str() {
            Some(s) if def.has_option(s) => Ok(v.clone()),
            Some(s) => Err(view_broken(format!("filter references deleted option {s}"), &def.field_id, "OPTION_DELETED")),
            None => Err(WsError::invalid_op("filter value must be an option id")),
        },
        _ => {
            let mut d = def.clone();
            d.nullable = false;
            d.normalize(v).map_err(|e| WsError::invalid_op(e.detail))
        }
    }
}

/// Validate structure, operator/type matrix and operand types against the live
/// fields. A clause on a deleted/unknown field or option is `VIEW_BROKEN`:
/// silently dropping a filter condition would show more rows.
pub fn compile(filter: &Value, fields: &BTreeMap<String, FieldDef>) -> WsResult<Compiled> {
    let mut nodes = 0;
    compile_node(filter, fields, 1, &mut nodes)
}

fn compile_node(v: &Value, fields: &BTreeMap<String, FieldDef>, depth: usize, nodes: &mut usize) -> WsResult<Compiled> {
    *nodes += 1;
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err(WsError::invalid_op("filter too deep or too large"));
    }
    let o = v.as_object().ok_or_else(|| WsError::invalid_op("filter node must be an object"))?;
    let op = o.get("op").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("filter node needs op"))?;
    match op {
        "and" | "or" => {
            let args = o.get("args").and_then(Value::as_array).ok_or_else(|| WsError::invalid_op("args required"))?;
            if args.is_empty() || args.len() > MAX_ARGS || o.len() != 2 {
                return Err(WsError::invalid_op("and/or takes 1..32 args"));
            }
            let c = args.iter().map(|a| compile_node(a, fields, depth + 1, nodes)).collect::<WsResult<Vec<_>>>()?;
            Ok(if op == "and" { Compiled::And(c) } else { Compiled::Or(c) })
        }
        "not" => {
            let arg = o.get("arg").filter(|_| o.len() == 2).ok_or_else(|| WsError::invalid_op("not takes arg"))?;
            Ok(Compiled::Not(Box::new(compile_node(arg, fields, depth + 1, nodes)?)))
        }
        "cmp" => {
            for k in o.keys() {
                if !matches!(k.as_str(), "op" | "field_id" | "operator" | "value") {
                    return Err(WsError::invalid_op(format!("unknown filter key {k}")));
                }
            }
            let field_id = o.get("field_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("field_id required"))?;
            let operator = o
                .get("operator")
                .and_then(Value::as_str)
                .and_then(Operator::parse)
                .ok_or_else(|| WsError::invalid_op("unknown operator"))?;
            let def = fields.get(field_id).ok_or_else(|| {
                view_broken(format!("filter references missing field {field_id}"), field_id, "FIELD_DELETED")
            })?;
            if !operator.allowed(def.ty) {
                return Err(WsError::invalid_op(format!(
                    "operator {} not allowed on {}",
                    o["operator"].as_str().unwrap_or(""),
                    def.ty.as_str()
                )));
            }
            let raw = o.get("value");
            let value = match operator {
                Operator::IsEmpty | Operator::IsNotEmpty => {
                    if raw.is_some() {
                        return Err(WsError::invalid_op("is_empty takes no value"));
                    }
                    Value::Null
                }
                Operator::In => {
                    let a = raw.and_then(Value::as_array).ok_or_else(|| WsError::invalid_op("in takes an array"))?;
                    Value::Array(a.iter().map(|x| norm_operand(def, x)).collect::<WsResult<_>>()?)
                }
                Operator::HasAny | Operator::HasAll => {
                    let a = raw.and_then(Value::as_array).ok_or_else(|| WsError::invalid_op("expected array"))?;
                    for x in a {
                        match x.as_str() {
                            Some(s) if def.has_option(s) => {}
                            Some(s) => {
                                return Err(view_broken(
                                    format!("filter references deleted option {s}"),
                                    field_id,
                                    "OPTION_DELETED",
                                ))
                            }
                            None => return Err(WsError::invalid_op("expected option ids")),
                        }
                    }
                    Value::Array(a.clone())
                }
                Operator::Contains | Operator::StartsWith => {
                    raw.filter(|x| x.is_string()).cloned().ok_or_else(|| WsError::invalid_op("expected string"))?
                }
                _ => norm_operand(def, raw.ok_or_else(|| WsError::invalid_op("value required"))?)?,
            };
            Ok(Compiled::Cmp { def: def.clone(), op: operator, value })
        }
        _ => Err(WsError::invalid_op(format!("unknown filter op {op}"))),
    }
}

fn is_empty_value(def: &FieldDef, v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Array(a)) => def.ty == FieldType::MultiSelect && a.is_empty(),
        _ => false,
    }
}

pub fn eval(c: &Compiled, values: &JsonMap) -> bool {
    match c {
        Compiled::And(a) => a.iter().all(|x| eval(x, values)),
        Compiled::Or(a) => a.iter().any(|x| eval(x, values)),
        Compiled::Not(a) => !eval(a, values),
        Compiled::Cmp { def, op, value } => {
            let cell = values.get(&def.field_id);
            match op {
                Operator::IsEmpty => return is_empty_value(def, cell),
                Operator::IsNotEmpty => return !is_empty_value(def, cell),
                _ => {}
            }
            // unset / null is false for every other operator, `ne` included
            let cell = match cell {
                None | Some(Value::Null) => return false,
                Some(v) => v,
            };
            match op {
                Operator::Eq => value::values_equal(def, cell, value),
                Operator::Ne => !value::values_equal(def, cell, value),
                Operator::Lt => value::compare(def, cell, value) == Ordering::Less,
                Operator::Lte => value::compare(def, cell, value) != Ordering::Greater,
                Operator::Gt => value::compare(def, cell, value) == Ordering::Greater,
                Operator::Gte => value::compare(def, cell, value) != Ordering::Less,
                Operator::Contains => cell.as_str().is_some_and(|s| s.contains(value.as_str().unwrap_or(""))),
                Operator::StartsWith => cell.as_str().is_some_and(|s| s.starts_with(value.as_str().unwrap_or(""))),
                Operator::In => value.as_array().is_some_and(|a| a.iter().any(|x| value::values_equal(def, cell, x))),
                Operator::HasAny | Operator::HasAll => {
                    let have: Vec<&str> = cell.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    let mut want = value.as_array().map(|a| a.iter().filter_map(Value::as_str)).into_iter().flatten();
                    if *op == Operator::HasAny {
                        want.any(|w| have.contains(&w))
                    } else {
                        want.all(|w| have.contains(&w))
                    }
                }
                Operator::IsEmpty | Operator::IsNotEmpty => unreachable!(),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SortKey {
    pub def: FieldDef,
    pub desc: bool,
}

/// Empty values sort last in both directions; `record_id` is the final tiebreak.
pub fn compare_rows(sorts: &[SortKey], a: (&JsonMap, &str), b: (&JsonMap, &str)) -> Ordering {
    for s in sorts {
        let va = a.0.get(&s.def.field_id).filter(|v| !v.is_null());
        let vb = b.0.get(&s.def.field_id).filter(|v| !v.is_null());
        let o = match (va, vb) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => {
                let o = value::compare(&s.def, x, y);
                if s.desc {
                    o.reverse()
                } else {
                    o
                }
            }
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.1.cmp(b.1)
}

#[derive(Debug, Clone, Default)]
pub struct QuerySpec {
    pub filter: Option<Value>,
    /// `[{ field_id, direction }]`
    pub sorts: Vec<Value>,
    pub fields: Option<Vec<String>>,
    pub group: Option<String>,
    pub manual_order: Option<JsonMap>,
    pub limit: usize,
    pub cursor: Option<String>,
    pub best_effort: bool,
    pub with_meta: bool,
}

pub fn live_fields(ctx: &dyn ReadCtx, source_id: &str) -> WsResult<Vec<FieldRow>> {
    Ok(ctx.fields(source_id)?.into_iter().filter(|f| f.alive()).collect())
}

fn row_json(r: &RecordRow, live: &BTreeMap<String, FieldDef>, proj: Option<&Vec<String>>, with_meta: bool) -> Value {
    let keep = |k: &String| live.contains_key(k) && proj.map_or(true, |p| p.contains(k));
    let values: JsonMap = r.values.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let revs: JsonMap = r.revs.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), json!(v))).collect();
    let mut out = json!({ "record_id": r.record_id, "rev": r.rev, "values": values, "revs": revs });
    let m = out.as_object_mut().unwrap();
    if with_meta {
        let meta: JsonMap = r.meta.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        if !meta.is_empty() {
            m.insert("meta".into(), Value::Object(meta));
        }
    }
    if let Some(b) = &r.body_entity_id {
        m.insert("body_ref".into(), json!({ "entity_id": b, "version": { "mode": "live_head" } }));
    }
    out
}

/// Baseline query: scan the source, filter and sort in Rust, keyset-paginate.
/// Snapshot consistency: the cursor pins `content_rev`; if the source moved the
/// call fails with `SNAPSHOT_EXPIRED` instead of pretending offsets are stable.
pub fn run_query(ctx: &dyn ReadCtx, source: &EntityRow, spec: &QuerySpec) -> WsResult<Value> {
    let fields = live_fields(ctx, &source.entity_id)?;
    let live: BTreeMap<String, FieldDef> = fields.iter().map(|f| (f.field_id.clone(), f.def.clone())).collect();
    let mut diagnostics = Vec::new();
    let compiled = match &spec.filter {
        Some(f) if !f.is_null() => Some(compile(f, &live)?),
        _ => None,
    };
    if spec.sorts.len() > MAX_SORTS {
        return Err(WsError::invalid_op("at most 4 sorts"));
    }
    let mut sorts = Vec::new();
    for s in &spec.sorts {
        let fid = s.get("field_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("sort needs field_id"))?;
        let desc = match s.get("direction").and_then(Value::as_str).unwrap_or("asc") {
            "asc" => false,
            "desc" => true,
            _ => return Err(WsError::invalid_op("sort direction must be asc or desc")),
        };
        match live.get(fid) {
            Some(def) if def.ty.sortable() => sorts.push(SortKey { def: def.clone(), desc }),
            Some(_) => return Err(WsError::invalid_op(format!("field {fid} is not sortable"))),
            // dangling sort entries do not change the row set: skip and report
            None => diagnostics.push(json!({ "key": "sorts", "code": "FIELD_DELETED", "field_id": fid })),
        }
    }
    let proj: Option<Vec<String>> = spec.fields.as_ref().map(|p| {
        p.iter()
            .filter(|f| {
                let ok = live.contains_key(*f);
                if !ok {
                    diagnostics.push(json!({ "key": "fields", "code": "FIELD_DELETED", "field_id": f }));
                }
                ok
            })
            .cloned()
            .collect()
    });
    let group_def = match &spec.group {
        Some(g) => match live.get(g) {
            Some(d) if matches!(d.ty, FieldType::Select | FieldType::Boolean | FieldType::Date) => Some(d.clone()),
            Some(_) => return Err(WsError::invalid_op("group supports select, boolean and date fields")),
            None => {
                diagnostics.push(json!({ "key": "group", "code": "FIELD_DELETED", "field_id": g }));
                None
            }
        },
        None => None,
    };

    let type_revs: BTreeMap<&str, u64> = fields.iter().map(|f| (f.field_id.as_str(), f.type_rev)).collect();
    let digest = sha256_hex(
        canonical_json(&json!({ "f": spec.filter, "s": spec.sorts, "g": spec.group, "m": spec.manual_order, "t": type_revs }))?
            .as_bytes(),
    )[..16]
        .to_string();
    let cursor = match &spec.cursor {
        Some(c) => {
            let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(c)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .ok_or_else(|| WsError::invalid_op("bad cursor"))?;
            if raw["d"] != json!(digest) {
                return Err(WsError::new(Code::SnapshotExpired, "cursor does not match this query or schema"));
            }
            if !spec.best_effort && raw["r"] != json!(source.content_rev) {
                return Err(WsError::new(Code::SnapshotExpired, "source changed during consistent paging"));
            }
            Some(raw)
        }
        None => None,
    };

    let manual = if sorts.is_empty() { spec.manual_order.as_ref() } else { None };
    let mut rows: Vec<RecordRow> = Vec::new();
    ctx.scan_records(&source.entity_id, &mut |r| {
        if r.alive() && compiled.as_ref().map_or(true, |c| eval(c, &r.values)) {
            rows.push(r.clone());
        }
        Ok(true)
    })?;
    let manual_key = |id: &str| manual.and_then(|m| m.get(id)).and_then(Value::as_str).map(str::to_string);
    let cmp = |a: (&JsonMap, &str), b: (&JsonMap, &str)| -> Ordering {
        if manual.is_some() {
            // records without a manual key follow the keyed ones, by record_id
            let o = match (manual_key(a.1), manual_key(b.1)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            };
            return o.then_with(|| a.1.cmp(b.1));
        }
        compare_rows(&sorts, a, b)
    };
    rows.sort_by(|a, b| cmp((&a.values, &a.record_id), (&b.values, &b.record_id)));
    let total = rows.len();

    let mut groups = None;
    if let Some(gd) = &group_def {
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        for r in &rows {
            let k = match r.values.get(&gd.field_id) {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
            };
            *counts.entry(k).or_default() += 1;
        }
        groups = Some(counts.into_iter().map(|(k, n)| json!({ "key": k, "count": n })).collect::<Vec<_>>());
    }

    let start = match &cursor {
        Some(c) => {
            let last_vals = c["k"].as_object().cloned().unwrap_or_default();
            let last_id = c["id"].as_str().unwrap_or("").to_string();
            rows.partition_point(|r| cmp((&r.values, &r.record_id), (&last_vals, &last_id)) != Ordering::Greater)
        }
        None => 0,
    };
    let limit = spec.limit.max(1);
    let end = (start + limit).min(total);
    let page = &rows[start..end];
    let mut out = json!({
        "rows": page.iter().map(|r| row_json(r, &live, proj.as_ref(), spec.with_meta)).collect::<Vec<_>>(),
        "total": total,
        "content_rev": source.content_rev,
        "consistency": if spec.best_effort { "best_effort" } else { "snapshot" },
    });
    let m = out.as_object_mut().unwrap();
    if end < total {
        let last = &page[page.len() - 1];
        let k: JsonMap = sorts
            .iter()
            .filter_map(|s| last.values.get(&s.def.field_id).map(|v| (s.def.field_id.clone(), v.clone())))
            .collect();
        let raw = json!({ "r": source.content_rev, "d": digest, "k": k, "id": last.record_id });
        m.insert(
            "next_cursor".into(),
            json!(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.to_string())),
        );
    }
    if let Some(g) = groups {
        m.insert("groups".into(), json!(g));
    }
    if !diagnostics.is_empty() {
        m.insert("diagnostics".into(), json!(diagnostics));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> BTreeMap<String, FieldDef> {
        let v = json!([
            {"field_id":"title","name":"t","type":"text"},
            {"field_id":"n","name":"n","type":"number"},
            {"field_id":"budget","name":"b","type":"decimal","scale":2},
            {"field_id":"due","name":"d","type":"date"},
            {"field_id":"done","name":"x","type":"boolean"},
            {"field_id":"status","name":"s","type":"select","options":[
                {"option_id":"open","label":"O"},{"option_id":"done","label":"D"}]},
            {"field_id":"tags","name":"g","type":"multi_select","options":[
                {"option_id":"a","label":"A"},{"option_id":"b","label":"B"}]},
            {"field_id":"ref","name":"r","type":"object_ref"}
        ]);
        v.as_array().unwrap().iter().map(|d| {
            let d = FieldDef::from_value(d).unwrap();
            (d.field_id.clone(), d)
        }).collect()
    }

    fn row(v: Value) -> JsonMap {
        v.as_object().unwrap().clone()
    }

    fn check(filter: Value, values: Value) -> bool {
        eval(&compile(&filter, &defs()).unwrap(), &row(values))
    }

    #[test]
    fn matrix() {
        let d = defs();
        let ops = ["eq", "ne", "lt", "lte", "gt", "gte", "contains", "starts_with", "in", "has_any", "has_all", "is_empty", "is_not_empty"];
        let sample = |f: &str, op: &str| -> Value {
            let single = match f {
                "title" => json!("x"),
                "n" => json!(1),
                "budget" => json!("1.00"),
                "due" => json!("2026-01-01"),
                "done" => json!(true),
                "status" => json!("open"),
                "tags" => json!("a"),
                _ => json!("tasks"),
            };
            match op {
                "in" | "has_any" | "has_all" => json!({"op":"cmp","field_id":f,"operator":op,"value":[single]}),
                "is_empty" | "is_not_empty" => json!({"op":"cmp","field_id":f,"operator":op}),
                _ => json!({"op":"cmp","field_id":f,"operator":op,"value":single}),
            }
        };
        let expect = |f: &str, op: &str| -> bool {
            match op {
                "eq" | "ne" => f != "tags",
                "lt" | "lte" | "gt" | "gte" => matches!(f, "n" | "budget" | "due"),
                "contains" | "starts_with" => f == "title",
                "in" => matches!(f, "title" | "n" | "budget" | "due" | "status"),
                "has_any" | "has_all" => f == "tags",
                _ => true,
            }
        };
        for f in d.keys() {
            for op in ops {
                assert_eq!(compile(&sample(f, op), &d).is_ok(), expect(f, op), "{f} {op}");
            }
        }
    }

    #[test]
    fn semantics() {
        // empty: unset or null; "" is not empty; ne is false on empty cells
        assert!(check(json!({"op":"cmp","field_id":"title","operator":"is_empty"}), json!({})));
        assert!(check(json!({"op":"cmp","field_id":"title","operator":"is_empty"}), json!({"title":null})));
        assert!(!check(json!({"op":"cmp","field_id":"title","operator":"is_empty"}), json!({"title":""})));
        assert!(check(json!({"op":"cmp","field_id":"tags","operator":"is_empty"}), json!({"tags":[]})));
        assert!(!check(json!({"op":"cmp","field_id":"status","operator":"ne","value":"done"}), json!({})));
        assert!(check(json!({"op":"not","arg":{"op":"cmp","field_id":"status","operator":"eq","value":"done"}}), json!({})));
        // decimals compare numerically, operands are normalized like writes
        assert!(check(json!({"op":"cmp","field_id":"budget","operator":"gt","value":"9.5"}), json!({"budget":"10.00"})));
        assert!(check(json!({"op":"cmp","field_id":"budget","operator":"eq","value":"10"}), json!({"budget":"10.00"})));
        assert!(check(json!({"op":"cmp","field_id":"title","operator":"contains","value":"验收"}), json!({"title":"写验收用例"})));
        assert!(!check(json!({"op":"cmp","field_id":"title","operator":"starts_with","value":"A"}), json!({"title":"abc"})));
        assert!(check(json!({"op":"cmp","field_id":"tags","operator":"has_all","value":["a","b"]}), json!({"tags":["a","b"]})));
        assert!(!check(json!({"op":"cmp","field_id":"tags","operator":"has_all","value":["a","b"]}), json!({"tags":["a"]})));
        assert!(check(json!({"op":"cmp","field_id":"ref","operator":"eq","value":"tasks"}),
            json!({"ref":{"entity_id":"tasks","version":{"mode":"live_head"}}})));
        // dangling references are VIEW_BROKEN, not ignored
        let e = compile(&json!({"op":"cmp","field_id":"gone","operator":"is_empty"}), &defs()).unwrap_err();
        assert_eq!(e.sub, Some("VIEW_BROKEN"));
        let e = compile(&json!({"op":"cmp","field_id":"status","operator":"eq","value":"zzz"}), &defs()).unwrap_err();
        assert_eq!(e.sub, Some("VIEW_BROKEN"));
        // limits
        let mut deep = json!({"op":"cmp","field_id":"title","operator":"is_empty"});
        for _ in 0..5 {
            deep = json!({"op":"not","arg":deep});
        }
        assert!(compile(&deep, &defs()).is_err());
    }

    #[test]
    fn ordering() {
        let d = defs();
        let sorts = vec![SortKey { def: d["budget"].clone(), desc: false }];
        let a = row(json!({"budget":"10.00"}));
        let b = row(json!({"budget":"9.50"}));
        let e = row(json!({}));
        assert_eq!(compare_rows(&sorts, (&a, "a"), (&b, "b")), Ordering::Greater);
        assert_eq!(compare_rows(&sorts, (&e, "a"), (&b, "b")), Ordering::Greater);
        let desc = vec![SortKey { def: d["budget"].clone(), desc: true }];
        assert_eq!(compare_rows(&desc, (&a, "a"), (&b, "b")), Ordering::Less);
        assert_eq!(compare_rows(&desc, (&e, "a"), (&b, "b")), Ordering::Greater, "empties last in both directions");
        assert_eq!(compare_rows(&desc, (&a, "a"), (&a, "b")), Ordering::Less);
    }
}
