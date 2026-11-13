//! SQL over tables (核心 API 调整 §3.4, §3.5; design draft §7): the logical table every local table is to a SQL query,
//! and the compiler that turns a saved table view (filter, sorts, group, manual order, fields) into one SELECT on it.
//!
//! The logical table is a temporary view over the shared `table_records` (one JSON of values per record), built per
//! query connection for the tables a caller may read: system columns `_id` (record id), `_rev`, `_body` (the path of
//! the record's body), `_record` (the record as `doc.read` gives it: `{ record_id, rev, values, revs, meta, body_ref? }`,
//! live fields only), and per live field a display column `"<name>"` (what `body` shows: a select's label, a
//! reference's path, a currency's amount) and the stored value `"<name>#raw"` (option ids, the reference object, the
//! exact decimal); a currency field also has `"<name>#currency"`. A field whose name is a system column's is
//! `"<name>#field"`.
//!
//! The compiled view SQL must give the rows, the order and the group counts of `filter::run_query` (the evaluator the
//! browser replica keeps, R16); `tests/viewsql.rs` holds them side by side. It uses three functions the executor
//! registers: `aiws_label(x, map)` (map a key through a JSON object, unknown keys stay), `aiws_labels(array, map)` (the
//! same over a JSON array, giving a JSON array), `aiws_pick(object, keys)` (keep the keys of a JSON object), and the
//! collation `aiws_dec` (exact decimal order); `aiws_dec_cmp(a, b)` compares two decimal strings (-1 / 0 / 1).

use crate::access::Access;
use crate::error::{WsError, WsResult};
use crate::filter::{compile, live_fields, Compiled, Operator, MAX_SORTS};
use crate::model::*;
use crate::value::{FieldDef, FieldType};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// System columns of every logical table.
pub const SYSTEM_COLUMNS: &[&str] = &["_id", "_rev", "_body", "_record"];

/// `"…"`: an SQL identifier.
pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `'…'`: an SQL string literal.
pub fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn json_path(field_id: &str) -> String {
    format!("$.\"{}\"", field_id.replace('"', "\\\""))
}

/// The column names of one live field: display, raw, and (currency) the code.
#[derive(Debug, Clone)]
pub struct FieldColumns {
    pub def: FieldDef,
    pub display: String,
    pub raw: String,
    pub currency: Option<String>,
}

/// The columns of a table's live fields, in field order.
pub fn field_columns(fields: &[FieldRow]) -> Vec<FieldColumns> {
    fields
        .iter()
        .filter(|f| f.alive())
        .map(|f| {
            let display = if SYSTEM_COLUMNS.contains(&f.def.name.as_str()) { format!("{}#field", f.def.name) } else { f.def.name.clone() };
            FieldColumns {
                raw: format!("{display}#raw"),
                currency: (f.def.ty == FieldType::Currency).then(|| format!("{display}#currency")),
                display,
                def: f.def.clone(),
            }
        })
        .collect()
}

fn options_map(def: &FieldDef) -> String {
    let m: Map<String, Value> = def.options.iter().flatten().map(|o| (o.option_id.clone(), json!(o.label))).collect();
    Value::Object(m).to_string()
}

/// The display expression of a field over `table_records.values_json` (`v`). Formula values and values of another
/// type are NULL, as they are empty to the evaluator.
fn display_expr(def: &FieldDef, refs: &str) -> String {
    let p = lit(&json_path(&def.field_id));
    let t = format!("json_type(values_json, {p})");
    let x = format!("json_extract(values_json, {p})");
    match def.ty {
        FieldType::Text | FieldType::Date | FieldType::Datetime => format!("CASE WHEN {t} = 'text' THEN {x} END"),
        FieldType::Number => format!("CASE WHEN {t} IN ('integer', 'real') THEN {x} END"),
        FieldType::Decimal => format!("CASE WHEN {t} = 'text' THEN CAST({x} AS REAL) END"),
        FieldType::Boolean => format!("CASE {t} WHEN 'true' THEN 1 WHEN 'false' THEN 0 END"),
        FieldType::Select => format!("CASE WHEN {t} = 'text' THEN aiws_label({x}, {}) END", lit(&options_map(def))),
        FieldType::MultiSelect => format!("CASE WHEN {t} = 'array' THEN aiws_labels({x}, {}) END", lit(&options_map(def))),
        FieldType::ObjectRef => {
            let id = lit(&format!("{}.entity_id", json_path(&def.field_id)));
            let f = lit(&format!("{}.\"$formula\"", json_path(&def.field_id)));
            format!("CASE WHEN {t} = 'object' AND json_type(values_json, {f}) IS NULL THEN aiws_label(json_extract(values_json, {id}), {}) END", lit(refs))
        }
        FieldType::Currency => {
            let a = lit(&format!("{}.amount", json_path(&def.field_id)));
            format!("CASE WHEN json_type(values_json, {a}) = 'text' THEN CAST(json_extract(values_json, {a}) AS REAL) END")
        }
        FieldType::Json => {
            let f = lit(&format!("{}.\"$formula\"", json_path(&def.field_id)));
            format!("CASE WHEN {t} = 'object' AND json_type(values_json, {f}) IS NULL THEN {x} END")
        }
    }
}

/// `entity_id → text` of what the records of `table` point at (`value` references for display, `body` references for
/// `_body`): readable targets by their path, the rest as `~<id>`.
fn target_labels(ctx: &dyn ReadCtx, access: &Access, table: &str, kind: &str) -> WsResult<String> {
    let mut m = Map::new();
    for r in ctx.refs_from(table)? {
        if r.kind != kind || !r.dst_workspace_id.is_empty() || r.dst_entity_id.is_empty() || m.contains_key(&r.dst_entity_id) {
            continue;
        }
        let label = match ctx.entity(&r.dst_entity_id)? {
            Some(e) if e.alive() && access.can_read(ctx, &e)? => crate::path::path_of(ctx, access, &e.entity_id)?,
            _ => format!("~{}", r.dst_entity_id),
        };
        m.insert(r.dst_entity_id.clone(), json!(label));
    }
    Ok(Value::Object(m).to_string())
}

/// `CREATE TEMP VIEW "<view_name>"` of the logical table of `table` (a live local table the caller may read).
pub fn logical_table_ddl(ctx: &dyn ReadCtx, access: &Access, table: &EntityRow, view_name: &str) -> WsResult<String> {
    let fields = live_fields(ctx, &table.entity_id)?;
    let cols = field_columns(&fields);
    let refs = if cols.iter().any(|c| c.def.ty == FieldType::ObjectRef) { target_labels(ctx, access, &table.entity_id, "value")? } else { "{}".into() };
    let bodies = target_labels(ctx, access, &table.entity_id, "body")?;
    let ids = lit(&Value::Array(cols.iter().map(|c| json!(c.def.field_id)).collect()).to_string());
    let mut select = vec![
        "record_id AS \"_id\"".to_string(),
        "rev AS \"_rev\"".to_string(),
        format!("CASE WHEN body_entity_id IS NOT NULL THEN aiws_label(body_entity_id, {}) END AS \"_body\"", lit(&bodies)),
    ];
    for c in &cols {
        select.push(format!("{} AS {}", display_expr(&c.def, &refs), ident(&c.display)));
        select.push(format!("json_extract(values_json, {}) AS {}", lit(&json_path(&c.def.field_id)), ident(&c.raw)));
        if let Some(cur) = &c.currency {
            let code = lit(&format!("{}.currency", json_path(&c.def.field_id)));
            let amount = lit(&format!("{}.amount", json_path(&c.def.field_id)));
            select.push(format!("CASE WHEN json_type(values_json, {amount}) = 'text' THEN json_extract(values_json, {code}) END AS {}", ident(cur)));
        }
    }
    select.push(format!(
        "json_object('record_id', record_id, 'rev', rev, 'values', json(aiws_pick(values_json, {ids})), 'revs', json(aiws_pick(revs_json, {ids})), \
         'meta', json(aiws_pick(meta_json, {ids})), 'body_ref', CASE WHEN body_entity_id IS NOT NULL THEN json_object('entity_id', body_entity_id, 'version', json_object('mode', 'live_head')) END) AS \"_record\""
    ));
    Ok(format!(
        "CREATE TEMP VIEW {} AS SELECT {} FROM main.table_records WHERE source_id = {} AND deleted_seq IS NULL",
        ident(view_name),
        select.join(", "),
        lit(&table.entity_id)
    ))
}

/// A saved table view: what a table Block keeps besides its binding.
#[derive(Debug, Clone, Default)]
pub struct ViewDef {
    pub filter: Option<Value>,
    pub sorts: Vec<Value>,
    pub fields: Option<Vec<String>>,
    pub group: Option<String>,
    pub manual_order: Option<JsonMap>,
}

impl ViewDef {
    /// The view of a table Block (`view.type: table`).
    pub fn of_cell(cell: &EntityRow) -> ViewDef {
        ViewDef {
            filter: cell.payload.get("filter").filter(|f| !f.is_null()).cloned(),
            sorts: cell.payload.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default(),
            fields: cell.payload.get("fields").and_then(Value::as_array).map(|a| a.iter().filter_map(|f| f["field_id"].as_str()).map(str::to_string).collect()),
            group: cell.payload.get("group").and_then(|g| g["field_id"].as_str()).map(str::to_string),
            manual_order: cell.payload.get("manual_order").and_then(Value::as_object).cloned(),
        }
    }

    /// The view parameters of a request (`filter`, `sorts`, `fields`, `group`, `manual_order`).
    pub fn of_params(p: &Value) -> ViewDef {
        ViewDef {
            filter: p.get("filter").filter(|f| !f.is_null()).cloned(),
            sorts: p.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default(),
            fields: p.get("fields").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()),
            group: p.get("group").and_then(Value::as_str).map(str::to_string),
            manual_order: p.get("manual_order").and_then(Value::as_object).cloned(),
        }
    }
}

/// What the compiled SELECT returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// `_id` and the display columns of the view's fields (all live fields without `fields`): what an Agent reads.
    Display,
    /// `_record` only: the native rows a client renders and edits.
    Records,
}

/// A view compiled to SQL over the logical table `table` (its quoted name in the query).
#[derive(Debug, Clone)]
pub struct CompiledView {
    /// The rows in the view's order (no LIMIT).
    pub sql: String,
    /// `SELECT count(*)` of the rows.
    pub count_sql: String,
    /// `SELECT key, count` per value of the group field, in the evaluator's group order (`key` ascending).
    pub group_sql: Option<String>,
    /// Dangling `fields` / `sorts` / `group` entries left out (as `run_query` reports them).
    pub diagnostics: Vec<Value>,
}

fn sql_value(v: &Value) -> String {
    match v {
        Value::String(s) => lit(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => (if *b { "1" } else { "0" }).into(),
        Value::Null => "NULL".into(),
        other => lit(&other.to_string()),
    }
}

/// The empty test of the evaluator: unset, null or a formula (display NULL); an empty multi_select.
fn empty_sql(c: &FieldColumns) -> String {
    let d = ident(&c.display);
    if c.def.ty == FieldType::MultiSelect {
        format!("({d} IS NULL OR {d} = '[]')")
    } else {
        format!("{d} IS NULL")
    }
}

fn leaf(c: &FieldColumns, op: Operator, value: &Value) -> String {
    let d = ident(&c.display);
    let r = ident(&c.raw);
    let one = |x: &Value| -> String {
        match c.def.ty {
            FieldType::Select => format!("{r} = {} /* {} */", sql_value(x), x.as_str().and_then(|id| c.def.option_label(id)).unwrap_or("").replace("*/", "* /")),
            FieldType::Decimal => format!("aiws_dec_cmp({r}, {}) = 0", sql_value(x)),
            FieldType::Currency => {
                let (code, amount) = crate::value::currency_parts(x);
                format!("(json_extract({r}, '$.currency') = {} AND aiws_dec_cmp(json_extract({r}, '$.amount'), {}) = 0)", lit(code), lit(amount))
            }
            FieldType::ObjectRef => {
                let ws = match x.get("workspace_id").and_then(Value::as_str) {
                    Some(w) => format!("= {}", lit(w)),
                    None => "IS NULL".into(),
                };
                format!("(json_extract({r}, '$.entity_id') = {} AND json_extract({r}, '$.workspace_id') {ws})", lit(crate::value::reference_entity_id(x).unwrap_or("")))
            }
            FieldType::Boolean => format!("{d} = {}", sql_value(x)),
            _ => format!("{d} = {}", sql_value(x)),
        }
    };
    let ordered = |cmp: &str| -> String {
        match c.def.ty {
            FieldType::Decimal => format!("aiws_dec_cmp({r}, {}) {cmp} 0", sql_value(value)),
            FieldType::Currency => {
                let (code, amount) = crate::value::currency_parts(value);
                format!("(json_extract({r}, '$.currency') = {} AND aiws_dec_cmp(json_extract({r}, '$.amount'), {}) {cmp} 0)", lit(code), lit(amount))
            }
            _ => format!("{d} {cmp} {}", sql_value(value)),
        }
    };
    let test = match op {
        Operator::Eq => one(value),
        Operator::Ne => format!("NOT ({})", one(value)),
        Operator::Lt => ordered("<"),
        Operator::Lte => ordered("<="),
        Operator::Gt => ordered(">"),
        Operator::Gte => ordered(">="),
        Operator::Contains => format!("instr({d}, {}) > 0", sql_value(value)),
        Operator::StartsWith => {
            let s = value.as_str().unwrap_or("");
            format!("substr({d}, 1, {}) = {}", s.chars().count(), lit(s))
        }
        Operator::In => {
            let parts: Vec<String> = value.as_array().into_iter().flatten().map(one).collect();
            if parts.is_empty() {
                "0".into()
            } else {
                format!("({})", parts.join(" OR "))
            }
        }
        Operator::HasAny | Operator::HasAll => {
            let want: Vec<String> = value.as_array().into_iter().flatten().filter_map(Value::as_str).map(lit).collect();
            if want.is_empty() {
                return if op == Operator::HasAll { format!("{d} IS NOT NULL") } else { "0".into() };
            }
            let n = want.len();
            let labels: Vec<String> = value.as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(|id| c.def.option_label(id)).map(|l| l.replace("*/", "* /")).collect();
            let count = format!("(SELECT count(DISTINCT j.value) FROM json_each({r}) j WHERE j.value IN ({}) /* {} */)", want.join(", "), labels.join(", "));
            if op == Operator::HasAny {
                format!("{count} > 0")
            } else {
                format!("{count} = {n}")
            }
        }
        Operator::IsEmpty => return empty_sql(c),
        Operator::IsNotEmpty => return format!("NOT {}", empty_sql(c)),
    };
    // unset / null / formula is false for every other operator, `ne` included
    format!("({d} IS NOT NULL AND COALESCE({test}, 0))")
}

fn where_sql(c: &Compiled, cols: &BTreeMap<String, FieldColumns>) -> String {
    match c {
        Compiled::And(a) => format!("({})", a.iter().map(|x| where_sql(x, cols)).collect::<Vec<_>>().join(" AND ")),
        Compiled::Or(a) => format!("({})", a.iter().map(|x| where_sql(x, cols)).collect::<Vec<_>>().join(" OR ")),
        Compiled::Not(a) => format!("(NOT {})", where_sql(a, cols)),
        Compiled::Cmp { def, op, value } => match cols.get(&def.field_id) {
            Some(col) => leaf(col, *op, value),
            None => "0".into(),
        },
    }
}

/// The ORDER BY terms of one sort key: empties last in both directions.
fn sort_terms(c: &FieldColumns, desc: bool) -> Vec<String> {
    let d = ident(&c.display);
    let r = ident(&c.raw);
    let dir = if desc { " DESC" } else { "" };
    let empty = format!("({d} IS NULL)");
    match c.def.ty {
        FieldType::Select => {
            let cases: Vec<String> = c.def.options.iter().flatten().enumerate().map(|(i, o)| format!("WHEN {} THEN {i}", lit(&o.option_id))).collect();
            let key = if cases.is_empty() { "0".to_string() } else { format!("CASE {r} {} ELSE 1000000000 END", cases.join(" ")) };
            vec![empty, format!("{key}{dir}")]
        }
        FieldType::Decimal => vec![empty, format!("CASE WHEN {d} IS NOT NULL THEN {r} END COLLATE aiws_dec{dir}")],
        FieldType::Currency => vec![
            empty,
            format!("json_extract({r}, '$.currency'){dir}"),
            format!("json_extract({r}, '$.amount') COLLATE aiws_dec{dir}"),
        ],
        _ => vec![empty, format!("{d}{dir}")],
    }
}

/// Compile `view` over the logical table named `table` (an SQL identifier, quoted) whose live fields are `fields`.
/// Clauses on deleted fields or options are `VIEW_BROKEN` (as `run_query`); dangling sorts, fields and group are left
/// out and reported in `diagnostics`.
pub fn compile_view(table: &str, fields: &[FieldRow], view: &ViewDef, projection: Projection) -> WsResult<CompiledView> {
    let cols = field_columns(fields);
    let by_id: BTreeMap<String, FieldColumns> = cols.iter().map(|c| (c.def.field_id.clone(), c.clone())).collect();
    let live: BTreeMap<String, FieldDef> = cols.iter().map(|c| (c.def.field_id.clone(), c.def.clone())).collect();
    let mut diagnostics = Vec::new();
    let filter = match &view.filter {
        Some(f) => Some(compile(f, &live)?),
        None => None,
    };
    if view.sorts.len() > MAX_SORTS {
        return Err(WsError::invalid_op("at most 4 sorts"));
    }
    let mut order: Vec<String> = Vec::new();
    for s in &view.sorts {
        let fid = s.get("field_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("sort needs field_id"))?;
        let desc = match s.get("direction").and_then(Value::as_str).unwrap_or("asc") {
            "asc" => false,
            "desc" => true,
            _ => return Err(WsError::invalid_op("sort direction must be asc or desc")),
        };
        match by_id.get(fid) {
            Some(c) if c.def.ty.sortable() => order.extend(sort_terms(c, desc)),
            Some(_) => return Err(WsError::invalid_op(format!("field {fid} is not sortable"))),
            None => diagnostics.push(json!({ "key": "sorts", "code": "FIELD_DELETED", "field_id": fid })),
        }
    }
    if order.is_empty() {
        if let Some(m) = view.manual_order.as_ref().filter(|m| !m.is_empty()) {
            let cases: Vec<String> = m.iter().filter_map(|(id, k)| k.as_str().map(|k| format!("WHEN {} THEN {}", lit(id), lit(k)))).collect();
            if !cases.is_empty() {
                let key = format!("CASE \"_id\" {} END", cases.join(" "));
                order.push(format!("({key} IS NULL)"));
                order.push(key);
            }
        }
    }
    order.push("\"_id\"".into());
    // the view's fields that are still live (a dangling entry is reported, as `run_query` does)
    let shown: Option<Vec<&FieldColumns>> = view.fields.as_ref().map(|list| {
        list.iter()
            .filter_map(|f| {
                let c = by_id.get(f);
                if c.is_none() {
                    diagnostics.push(json!({ "key": "fields", "code": "FIELD_DELETED", "field_id": f }));
                }
                c
            })
            .collect()
    });
    let select = match (projection, &shown) {
        (Projection::Records, None) => "\"_record\"".to_string(),
        // values, revs and meta of the view's fields only (`run_query` with `fields`)
        (Projection::Records, Some(list)) => {
            let ids = lit(&Value::Array(list.iter().map(|c| json!(c.def.field_id)).collect()).to_string());
            let part = |k: &str| format!("'$.{k}', json(aiws_pick(json_extract(\"_record\", '$.{k}'), {ids}))");
            format!("json_set(\"_record\", {}, {}, {}) AS \"_record\"", part("values"), part("revs"), part("meta"))
        }
        (Projection::Display, Some(list)) => std::iter::once("\"_id\"".to_string()).chain(list.iter().map(|c| ident(&c.display))).collect::<Vec<_>>().join(", "),
        (Projection::Display, None) => std::iter::once("\"_id\"".to_string()).chain(cols.iter().map(|c| ident(&c.display))).collect::<Vec<_>>().join(", "),
    };
    let where_clause = filter.as_ref().map(|f| format!(" WHERE {}", where_sql(f, &by_id))).unwrap_or_default();
    let sql = format!("SELECT {select} FROM {table}{where_clause} ORDER BY {}", order.join(", "));
    let count_sql = format!("SELECT count(*) FROM {table}{where_clause}");
    let group_sql = match &view.group {
        Some(g) => match by_id.get(g) {
            Some(c) if matches!(c.def.ty, FieldType::Select | FieldType::MultiSelect | FieldType::Boolean | FieldType::Date) => {
                let r = ident(&c.raw);
                Some(match c.def.ty {
                    FieldType::MultiSelect => format!(
                        "SELECT COALESCE(j.value, '') AS key, count(*) AS count FROM {table} LEFT JOIN json_each(CASE WHEN json_valid({r}) THEN {r} END) j{} GROUP BY 1 ORDER BY 1",
                        if where_clause.is_empty() { String::new() } else { where_clause.clone() }
                    ),
                    FieldType::Boolean => format!(
                        "SELECT CASE WHEN {r} IS NULL THEN '' WHEN {r} THEN 'true' ELSE 'false' END AS key, count(*) AS count FROM {table}{where_clause} GROUP BY 1 ORDER BY 1"
                    ),
                    _ => format!("SELECT COALESCE({r}, '') AS key, count(*) AS count FROM {table}{where_clause} GROUP BY 1 ORDER BY 1"),
                })
            }
            Some(_) => return Err(WsError::invalid_op("group supports select, multi_select, boolean and date fields")),
            None => {
                diagnostics.push(json!({ "key": "group", "code": "FIELD_DELETED", "field_id": g }));
                None
            }
        },
        None => None,
    };
    Ok(CompiledView { sql, count_sql, group_sql, diagnostics })
}

/// A table view Block compiled over its table's path (`meta.shows.view_sql`): what the Block shows, as an Agent may
/// run or rewrite it. `None` when the Block is no table view of a readable local table; a view that no longer
/// compiles (`VIEW_BROKEN`) is the error.
pub fn cell_view(ctx: &dyn ReadCtx, access: &Access, cell: &EntityRow, projection: Projection) -> WsResult<Option<CompiledView>> {
    if cell.payload.get("view").and_then(|v| v["type"].as_str()) != Some("table") {
        return Ok(None);
    }
    let Some(source_id) = cell.payload.get("source_ref").and_then(crate::value::reference_entity_id) else { return Ok(None) };
    let Some(table) = ctx.entity(source_id)?.filter(|t| t.alive() && t.type_id == TYPE_TABLE) else { return Ok(None) };
    if !access.can_read(ctx, &table)? || crate::types::is_url_table(&table) {
        return Ok(None);
    }
    let path = crate::path::path_of(ctx, access, &table.entity_id)?;
    let fields = live_fields(ctx, &table.entity_id)?;
    compile_view(&ident(&path), &fields, &ViewDef::of_cell(cell), projection).map(Some)
}

/// `meta.shows` of a table view Block: `view_sql`, `view_count_sql`, `view_group_sql`, `view_diagnostics`, or
/// `view_error`.
pub fn describe_cell_view(ctx: &dyn ReadCtx, access: &Access, cell: &EntityRow, into: &mut Value) -> WsResult<()> {
    match cell_view(ctx, access, cell, Projection::Display) {
        Ok(Some(v)) => {
            into["view_sql"] = json!(v.sql);
            into["view_count_sql"] = json!(v.count_sql);
            if let Some(g) = v.group_sql {
                into["view_group_sql"] = json!(g);
            }
            if !v.diagnostics.is_empty() {
                into["view_diagnostics"] = json!(v.diagnostics);
            }
        }
        Ok(None) => {}
        Err(e) if e.code.is_conflict() || matches!(e.code, crate::Code::InvalidOperation | crate::Code::InvalidSchema) => into["view_error"] = e.to_json(),
        Err(e) => return Err(e),
    }
    Ok(())
}
