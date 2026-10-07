//! Content profiles (许愿格 §5.3): enough to decide "is this the data, and how must it be handled"
//! without reading it all. Computed deterministically on the snapshot; understanding material, not a
//! generation dependency. Also the conversion of stored values into the plain values programs and
//! models see (option labels instead of option ids, numbers instead of decimal strings).

use aiworkspace_core::filter::live_fields;
use aiworkspace_core::model::*;
use aiworkspace_core::value::{FieldDef, FieldType};
use aiworkspace_core::WsResult;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const SAMPLE_ROWS: usize = 5;
const TOP_VALUES: usize = 6;
const DISTINCT_LIMIT: usize = 2000;

/// A stored table value as programs see it.
pub fn plain_value(def: &FieldDef, v: &Value) -> Value {
    let label = |id: &str| def.options.as_ref().and_then(|o| o.iter().find(|x| x.option_id == id)).map(|x| x.label.clone()).unwrap_or_else(|| id.to_string());
    match (def.ty, v) {
        (FieldType::Decimal, Value::String(s)) => s.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| v.clone()),
        (FieldType::Select, Value::String(s)) => json!(label(s)),
        (FieldType::MultiSelect, Value::Array(a)) => Value::Array(a.iter().map(|x| x.as_str().map(|s| json!(label(s))).unwrap_or_else(|| x.clone())).collect()),
        _ => v.clone(),
    }
}

/// Text that parses as a number once separators and currency/percent signs are removed.
pub fn numeric_text(s: &str) -> Option<f64> {
    let t: String = s.chars().filter(|c| !matches!(c, ',' | '¥' | '$' | '%' | '￥' | ' ' | '\u{a0}')).collect();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn as_num(def: &FieldDef, v: &Value) -> Option<f64> {
    match (def.ty, v) {
        (FieldType::Number, Value::Number(n)) => n.as_f64(),
        (FieldType::Decimal, Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

fn short(v: &Value, max: usize) -> Value {
    match v {
        Value::String(s) if s.chars().count() > max => json!(format!("{}…", s.chars().take(max).collect::<String>())),
        _ => v.clone(),
    }
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{:.2}", n)
    }
}

/// Profile of a table (all live records; `exclude` hides fields owned by the reading wish, §10.4).
pub fn table_profile(ctx: &dyn ReadCtx, table: &EntityRow, field_handles: &[String], exclude: &[String]) -> WsResult<Value> {
    let fields: Vec<FieldRow> = live_fields(ctx, &table.entity_id)?;
    struct Acc {
        nulls: u64,
        min: Option<f64>,
        max: Option<f64>,
        dmin: Option<String>,
        dmax: Option<String>,
        counts: BTreeMap<String, u64>,
        overflow: bool,
        numeric_text: u64,
        texts: u64,
        nums: Vec<f64>,
    }
    let mut acc: Vec<Acc> = fields
        .iter()
        .map(|_| Acc { nulls: 0, min: None, max: None, dmin: None, dmax: None, counts: BTreeMap::new(), overflow: false, numeric_text: 0, texts: 0, nums: Vec::new() })
        .collect();
    let mut rows = 0u64;
    let mut samples = Vec::new();
    ctx.scan_records(&table.entity_id, &mut |r| {
        if !r.alive() {
            return Ok(true);
        }
        rows += 1;
        if samples.len() < SAMPLE_ROWS {
            let mut row = Map::new();
            for f in &fields {
                if exclude.contains(&f.field_id) {
                    continue;
                }
                if let Some(v) = r.values.get(&f.field_id) {
                    row.insert(f.def.name.clone(), short(&plain_value(&f.def, v), 80));
                }
            }
            samples.push(Value::Object(row));
        }
        for (i, f) in fields.iter().enumerate() {
            let a = &mut acc[i];
            let Some(v) = r.values.get(&f.field_id).filter(|v| !v.is_null() && v.as_str() != Some("")) else {
                a.nulls += 1;
                continue;
            };
            if let Some(n) = as_num(&f.def, v) {
                a.min = Some(a.min.map_or(n, |m| m.min(n)));
                a.max = Some(a.max.map_or(n, |m| m.max(n)));
                a.nums.push(n);
            }
            if matches!(f.def.ty, FieldType::Date | FieldType::Datetime) {
                if let Some(s) = v.as_str() {
                    if a.dmin.as_deref().map_or(true, |m| s < m) {
                        a.dmin = Some(s.to_string());
                    }
                    if a.dmax.as_deref().map_or(true, |m| s > m) {
                        a.dmax = Some(s.to_string());
                    }
                }
            }
            if matches!(f.def.ty, FieldType::Select | FieldType::MultiSelect | FieldType::Boolean | FieldType::Text) {
                let keys: Vec<String> = match plain_value(&f.def, v) {
                    Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect(),
                    Value::String(s) => vec![s],
                    other => vec![other.to_string()],
                };
                for k in keys {
                    if a.counts.len() < DISTINCT_LIMIT || a.counts.contains_key(&k) {
                        *a.counts.entry(k).or_default() += 1;
                    } else {
                        a.overflow = true;
                    }
                }
            }
            if f.def.ty == FieldType::Text {
                a.texts += 1;
                if v.as_str().and_then(numeric_text).is_some() {
                    a.numeric_text += 1;
                }
            }
        }
        Ok(true)
    })?;
    let mut out_fields = Vec::new();
    let mut issues = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        if exclude.contains(&f.field_id) {
            continue;
        }
        let a = &mut acc[i];
        let mut fo = Map::new();
        if let Some(h) = field_handles.get(i) {
            fo.insert("handle".into(), json!(h));
        }
        fo.insert("field_id".into(), json!(f.field_id));
        fo.insert("name".into(), json!(f.def.name));
        fo.insert("type".into(), json!(f.def.ty.as_str()));
        if let Some(o) = &f.def.options {
            fo.insert("options".into(), json!(o.iter().map(|x| x.label.clone()).collect::<Vec<_>>()));
        }
        if f.def.ty == FieldType::Decimal {
            fo.insert("scale".into(), json!(f.def.scale));
        }
        fo.insert("nulls".into(), json!(a.nulls));
        if let (Some(lo), Some(hi)) = (a.min, a.max) {
            fo.insert("min".into(), json!(lo));
            fo.insert("max".into(), json!(hi));
            fo.insert("sum".into(), json!(a.nums.iter().sum::<f64>()));
        }
        if let (Some(lo), Some(hi)) = (&a.dmin, &a.dmax) {
            fo.insert("min".into(), json!(lo));
            fo.insert("max".into(), json!(hi));
        }
        let non_null = rows - a.nulls;
        if !a.counts.is_empty() && non_null > 0 {
            let distinct = a.counts.len();
            fo.insert("distinct".into(), if a.overflow { json!(format!(">{DISTINCT_LIMIT}")) } else { json!(distinct) });
            // categorical: few distinct values relative to the rows
            if f.def.ty != FieldType::Text || distinct <= 20 || (distinct as u64) * 4 <= non_null {
                let mut top: Vec<(&String, &u64)> = a.counts.iter().collect();
                top.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
                fo.insert(
                    "top".into(),
                    json!(top.iter().take(TOP_VALUES).map(|(k, n)| json!([short(&json!(k), 40), format!("{:.0}%", **n as f64 * 100.0 / non_null as f64)])).collect::<Vec<_>>()),
                );
            }
            if f.def.ty == FieldType::Text && distinct as u64 != non_null && (f.field_id == table.payload.get("title_field_id").and_then(Value::as_str).unwrap_or("") || i == 0) {
                issues.push(format!("字段「{}」有重复值（{} 行只有 {} 个不同值）", f.def.name, non_null, distinct));
            }
        }
        if non_null == 0 && rows > 0 {
            issues.push(format!("字段「{}」全部为空", f.def.name));
        } else if a.nulls > 0 && matches!(f.def.ty, FieldType::Number | FieldType::Decimal) {
            issues.push(format!("数值字段「{}」有 {} 个空值", f.def.name, a.nulls));
        }
        if a.texts >= 3 && a.numeric_text * 10 >= a.texts * 8 {
            issues.push(format!("字段「{}」是文本类型，但 {}/{} 个值是数字（计算前需要转换）", f.def.name, a.numeric_text, a.texts));
        }
        if a.nums.len() >= 8 {
            let mut v = a.nums.clone();
            v.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
            let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
            let (q1, q3) = (q(0.25), q(0.75));
            let iqr = q3 - q1;
            if iqr > 0.0 {
                let n = v.iter().filter(|x| **x < q1 - 3.0 * iqr || **x > q3 + 3.0 * iqr).count();
                if n > 0 {
                    issues.push(format!("字段「{}」有 {} 个异常值（超出四分位距 3 倍；范围 {}…{}）", f.def.name, n, fmt_num(v[0]), fmt_num(v[v.len() - 1])));
                }
            }
        }
        out_fields.push(Value::Object(fo));
    }
    Ok(json!({ "kind": "table", "rows": rows, "fields": out_fields, "samples": samples, "issues": issues }))
}

/// Profile of any data entity (tables via `table_profile`).
pub fn entity_profile(ctx: &dyn ReadCtx, e: &EntityRow, field_handles: &[String], exclude: &[String]) -> WsResult<Value> {
    Ok(match e.type_id.as_str() {
        TYPE_TABLE => table_profile(ctx, e, field_handles, exclude)?,
        TYPE_RECORD => {
            let nested = aiworkspace_core::types::record_nested(&e.payload);
            let schema = aiworkspace_core::types::record_schema(&e.payload).unwrap_or_default();
            let props: Vec<Value> = schema
                .values()
                .map(|d| {
                    let v = nested["props"].get(&d.field_id).map(|v| short(&plain_value(d, v), 200)).unwrap_or(Value::Null);
                    json!({ "key": d.field_id, "name": d.name, "type": d.ty.as_str(), "value": v })
                })
                .collect();
            json!({ "kind": "record", "properties": props })
        }
        TYPE_RICHTEXT => {
            let ast = ctx.richtext(&e.entity_id)?.map(|r| r.meta.ast).unwrap_or(Value::Null);
            let md = aiworkspace_core::markdown::from_richtext(&ast);
            let headings: Vec<String> = md.lines().filter(|l| l.starts_with('#')).map(|l| l.trim_start_matches('#').trim().to_string()).take(20).collect();
            let paragraphs: Vec<String> = md
                .split("\n\n")
                .map(str::trim)
                .filter(|p| !p.is_empty() && !p.starts_with('#'))
                .take(3)
                .map(|p| p.chars().take(300).collect())
                .collect();
            json!({ "kind": "richtext", "chars": md.chars().count(), "headings": headings, "first_paragraphs": paragraphs })
        }
        TYPE_ASSET => json!({ "kind": "asset", "media_type": e.payload.get("media_type"), "size": e.payload.get("size"),
                              "file_name": e.payload.get("file_name"), "image": e.payload.get("image") }),
        TYPE_ANNOTATION => json!({ "kind": "annotation", "body": e.payload.get("body"), "target": e.payload.get("target").and_then(|t| t.get("entity_id")) }),
        TYPE_WISH => json!({ "kind": "wish", "prompt": short(e.payload.get("prompt").unwrap_or(&Value::Null), 200), "executor": e.payload.get("executor") }),
        TYPE_BLOCK_DEF => json!({ "kind": "block_def", "def_id": e.payload.get("def_id"), "title": e.payload.get("title"),
                                  "accepts": e.payload.get("accepts"), "description": e.payload.get("description") }),
        TYPE_CONTAINER => {
            let mut by_type: BTreeMap<String, u64> = BTreeMap::new();
            let mut first = Vec::new();
            for edge in ctx.children(&e.entity_id)? {
                if let Some(c) = ctx.entity(&edge.child_id)?.filter(|c| c.alive()) {
                    *by_type.entry(c.type_id.trim_start_matches("buckyos.").to_string()).or_default() += 1;
                    if first.len() < 10 {
                        first.push(json!({ "name": c.name.clone().or_else(|| c.payload.get("title").and_then(Value::as_str).map(str::to_string)), "type": c.type_id }));
                    }
                }
            }
            json!({ "kind": "folder", "members": by_type.values().sum::<u64>(), "by_type": by_type, "first": first })
        }
        _ => json!({ "kind": e.type_id }),
    })
}

/// One line describing a profile for the map (`· 12,480 行 · 11 列`).
pub fn profile_line(p: &Value) -> String {
    match p["kind"].as_str().unwrap_or("") {
        "table" => format!("{} 行 · {} 列", p["rows"], p["fields"].as_array().map_or(0, Vec::len)),
        "record" => format!("{} 个属性", p["properties"].as_array().map_or(0, Vec::len)),
        "richtext" => {
            let h: Vec<&str> = p["headings"].as_array().into_iter().flatten().filter_map(Value::as_str).take(6).collect();
            if h.is_empty() {
                format!("{} 字", p["chars"])
            } else {
                format!("{} 字 · 标题：{}", p["chars"], h.join(" / "))
            }
        }
        "asset" => format!("{} {}", p["media_type"].as_str().unwrap_or(""), p["file_name"].as_str().unwrap_or("")),
        "folder" => format!("{} 项", p["members"]),
        _ => String::new(),
    }
}

/// Field lines for the map: `f2 月份 date（2026-01-01…2026-09-30）· f4 地区 select（华东 52%、华南 31%）`.
pub fn field_lines(p: &Value, max_fields: usize) -> Vec<String> {
    let mut out = Vec::new();
    for f in p["fields"].as_array().into_iter().flatten().take(max_fields) {
        let h = f["handle"].as_str().map(|h| h.rsplit('.').next().unwrap_or(h).to_string()).unwrap_or_default();
        let mut s = format!("{h} {} {}", f["name"].as_str().unwrap_or(""), f["type"].as_str().unwrap_or(""));
        let mut extra = Vec::new();
        if !f["min"].is_null() {
            let show = |v: &Value| v.as_f64().map(fmt_num).unwrap_or_else(|| v.as_str().unwrap_or("").to_string());
            extra.push(format!("{}…{}", show(&f["min"]), show(&f["max"])));
        }
        if let Some(top) = f["top"].as_array().filter(|t| !t.is_empty()) {
            extra.push(top.iter().take(4).map(|t| format!("{} {}", t[0].as_str().unwrap_or(""), t[1].as_str().unwrap_or(""))).collect::<Vec<_>>().join("、"));
        }
        if f["nulls"].as_u64().unwrap_or(0) > 0 {
            extra.push(format!("{} 个空值", f["nulls"]));
        }
        if !extra.is_empty() {
            s.push_str(&format!("（{}）", extra.join("，")));
        }
        out.push(s);
    }
    out
}
