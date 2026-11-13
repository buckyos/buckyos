//! `doc.query { sql }` (核心 API 调整 R3, R12, §3.4; design draft §7): one read-only SELECT run by SQLite itself on
//! the document database, over the logical tables of the local tables the caller may read. Nothing is translated:
//! the only layer is the temporary view of each table (`sqlview::logical_table_ddl`), created on the query's
//! connection the first time the query names it (`"/data/销售"`, `"~<entity_id>"`, an entity id, or an alias of
//! `tables`). An authorizer admits SELECT, reads through those views, and functions; everything else — the kernel
//! tables, writes, ATTACH, PRAGMA, recursive CTEs — is refused. A progress handler bounds the time; rows and bytes are
//! bounded too. The live Workspace runs it on a fresh read-only connection, a wish stage on its snapshot's.

use crate::docdb::db_err;
use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::sqlview::logical_table_ddl;
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::functions::FunctionFlags;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_SQL_BYTES: usize = 16 * 1024;
pub const ROWS_DEFAULT: usize = 1000;
pub const ROWS_MAX: usize = 5000;
pub const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MILLIS: u64 = 2000;
/// Logical tables one query may name.
pub const MAX_TABLES: usize = 64;

/// Functions a query may not call (the bundled SQLite has no file functions; these are the rest of the risky ones).
const DENIED_FUNCTIONS: &[&str] = &["load_extension", "readfile", "writefile", "edit", "fts3_tokenizer", "sqlite_offset"];

/// Register the functions and the collation the logical tables and the compiled views use.
pub fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    let text_of = |v: ValueRef<'_>| -> Option<String> {
        match v {
            ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Integer(i) => Some(i.to_string()),
            ValueRef::Real(r) => Some(r.to_string()),
            _ => None,
        }
    };
    let map_aux = |ctx: &rusqlite::functions::Context<'_>, arg: i32| -> rusqlite::Result<Arc<Map<String, Value>>> {
        ctx.get_or_create_aux(arg, |v| -> Result<Map<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(match v {
                ValueRef::Text(t) => serde_json::from_slice::<Value>(t).ok().and_then(|x| x.as_object().cloned()).unwrap_or_default(),
                _ => Map::new(),
            })
        })
    };
    conn.create_scalar_function("aiws_label", 2, det, move |ctx| {
        let Some(key) = text_of(ctx.get_raw(0)) else { return Ok(None) };
        let map = map_aux(ctx, 1)?;
        Ok(Some(map.get(&key).and_then(Value::as_str).map(str::to_string).unwrap_or(key)))
    })?;
    conn.create_scalar_function("aiws_labels", 2, det, move |ctx| {
        let Some(text) = text_of(ctx.get_raw(0)) else { return Ok(None) };
        let map = map_aux(ctx, 1)?;
        let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else { return Ok(None) };
        let labels: Vec<Value> = items
            .iter()
            .map(|x| match x.as_str() {
                Some(id) => json!(map.get(id).and_then(Value::as_str).unwrap_or(id)),
                None => x.clone(),
            })
            .collect();
        Ok(Some(Value::Array(labels).to_string()))
    })?;
    conn.create_scalar_function("aiws_pick", 2, det, move |ctx| {
        let keys = ctx.get_or_create_aux(1, |v| -> Result<BTreeSet<String>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(match v {
                ValueRef::Text(t) => serde_json::from_slice::<Vec<String>>(t).unwrap_or_default().into_iter().collect(),
                _ => BTreeSet::new(),
            })
        })?;
        let Some(text) = text_of(ctx.get_raw(0)) else { return Ok(Some("{}".to_string())) };
        let obj = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
        let kept: Map<String, Value> = obj.into_iter().filter(|(k, _)| keys.contains(k)).collect();
        Ok(Some(Value::Object(kept).to_string()))
    })?;
    conn.create_scalar_function("aiws_dec_cmp", 2, det, move |ctx| {
        match (text_of(ctx.get_raw(0)), text_of(ctx.get_raw(1))) {
            (Some(a), Some(b)) => Ok(Some(match aiworkspace_core::value::cmp_decimal(&a, &b) {
                std::cmp::Ordering::Less => -1i64,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            })),
            _ => Ok(None),
        }
    })?;
    conn.create_collation("aiws_dec", |a, b| aiworkspace_core::value::cmp_decimal(a, b))?;
    Ok(())
}

#[derive(Default)]
struct AuthState {
    /// The caller's statement is being prepared or run: only then the rules apply (the service's own reads on the
    /// same connection — resolving tables, creating the logical tables — are not the caller's).
    enforcing: bool,
    /// Logical tables created on this connection: view name → entity id.
    views: BTreeMap<String, String>,
    /// Views the query read through.
    read: BTreeSet<String>,
    denied: Option<String>,
}

fn authorize(state: &Mutex<AuthState>, a: AuthContext<'_>) -> Authorization {
    let mut s = state.lock().unwrap();
    if !s.enforcing {
        return Authorization::Allow;
    }
    let verdict = match a.action {
        AuthAction::Select => true,
        AuthAction::Read { table_name, .. } => {
            let via = a.accessor.filter(|v| s.views.contains_key(*v)).map(str::to_string);
            let ok = match (a.database_name, table_name) {
                (Some("main"), "table_records") => via.is_some(),
                (Some("temp"), t) => s.views.contains_key(t),
                // table-valued functions (json_each, json_tree) over values the query already has
                (_, "json_each" | "json_tree") => true,
                _ => false,
            };
            if let Some(v) = via.filter(|_| ok) {
                s.read.insert(v);
            }
            ok
        }
        AuthAction::Function { function_name } => !DENIED_FUNCTIONS.contains(&function_name.to_ascii_lowercase().as_str()),
        _ => false,
    };
    if verdict {
        Authorization::Allow
    } else {
        if s.denied.is_none() {
            s.denied = Some(format!("{:?}", a.action));
        }
        Authorization::Deny
    }
}

fn json_of(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(r) => serde_json::Number::from_f64(r).map(Value::Number).unwrap_or(Value::Null),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => json!({ "blob_bytes": b.len() }),
    }
}

fn sql_param(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as V;
    match v {
        Value::Null => V::Null,
        Value::Bool(b) => V::Integer(*b as i64),
        Value::Number(n) => match n.as_i64() {
            Some(i) => V::Integer(i),
            None => V::Real(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => V::Text(s.clone()),
        other => V::Text(other.to_string()),
    }
}

/// The name SQLite could not find, from its message.
fn missing_table(e: &rusqlite::Error) -> Option<String> {
    let msg = e.to_string();
    let i = msg.find("no such table: ")?;
    let name = msg[i + "no such table: ".len()..].trim();
    Some(name.strip_prefix("main.").unwrap_or(name).to_string())
}

/// Is `sql` one statement (a `;` only at its end)? Quotes (`'…'`, `"…"`, `` `…` ``, `[…]`) and comments are skipped.
fn single_statement(sql: &str) -> bool {
    let b = sql.as_bytes();
    let mut i = 0;
    let mut ended = false;
    while i < b.len() {
        let c = b[i];
        if ended && !c.is_ascii_whitespace() && c != b';' {
            if c == b'-' && b.get(i + 1) == Some(&b'-') {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if c == b'/' && b.get(i + 1) == Some(&b'*') {
                i = sql[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                continue;
            }
            return false;
        }
        match c {
            b'\'' | b'"' | b'`' => {
                let close = c;
                i += 1;
                while i < b.len() {
                    if b[i] == close {
                        if b.get(i + 1) == Some(&close) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
            }
            b'[' => {
                while i < b.len() && b[i] != b']' {
                    i += 1;
                }
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 1);
            }
            b';' => ended = true,
            _ => {}
        }
        i += 1;
    }
    true
}

fn is_alias(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 64 && (b[0].is_ascii_alphabetic() || b[0] == b'_') && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_') && !s.to_ascii_lowercase().starts_with("sqlite_")
}

/// Names a caller of [`execute`] adds to paths and ids (a wish stage's handles, `@T1`).
pub type Names<'a> = Option<&'a dyn Fn(&str) -> Option<String>>;

/// A local table the caller may read, for a name the query used.
fn table_for(ctx: &dyn ReadCtx, access: &Access, name: &str, names: Names) -> WsResult<EntityRow> {
    let id = match names.and_then(|f| f(name)) {
        Some(id) => aiworkspace_core::path::resolve(ctx, access, &id),
        None => aiworkspace_core::path::resolve(ctx, access, name),
    }
    .map_err(|_| WsError::not_found(format!("no table {name:?}")))?;
    let e = ctx.entity(&id)?.filter(|e| e.alive() && e.type_id == TYPE_TABLE && e.degraded().is_none()).ok_or_else(|| WsError::not_found(format!("no table {name:?}")))?;
    if aiworkspace_core::types::is_url_table(&e) {
        return Err(WsError::sub(Code::InvalidOperation, "NOT_MATERIALIZED", format!("{name} is a URL table: its rows are remote (doc.source_query)")));
    }
    Ok(e)
}

/// Create the logical table `name` of `table` on the connection.
fn create_view(conn: &Connection, state: &Mutex<AuthState>, ctx: &dyn ReadCtx, access: &Access, name: &str, table: &EntityRow) -> WsResult<()> {
    let ddl = logical_table_ddl(ctx, access, table, name)?;
    conn.execute_batch(&ddl).map_err(db_err)?;
    state.lock().unwrap().views.insert(name.to_string(), table.entity_id.clone());
    Ok(())
}

fn drop_views(conn: &Connection) -> WsResult<()> {
    let names: Vec<String> = {
        let mut st = conn.prepare("SELECT name FROM temp.sqlite_master WHERE type = 'view'").map_err(db_err)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)?
    };
    for n in names {
        conn.execute_batch(&format!("DROP VIEW temp.{}", aiworkspace_core::sqlview::ident(&n))).map_err(db_err)?;
    }
    Ok(())
}

/// Run `params.sql` on `conn` (inside a read transaction the caller opened). `head_seq` is the version the connection
/// sees.
pub fn execute(conn: &Connection, ctx: &dyn ReadCtx, access: &Access, params: &Value, head_seq: u64, names: Names) -> WsResult<Value> {
    let sql = params.get("sql").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("sql must be a string"))?;
    if sql.len() > MAX_SQL_BYTES {
        return Err(WsError::limit(format!("sql is longer than {MAX_SQL_BYTES} bytes")));
    }
    if !single_statement(sql) {
        return Err(WsError::sub(Code::InvalidOperation, "SQL_NOT_ALLOWED", "one statement only"));
    }
    let max_rows = match params.get("max_rows") {
        None | Some(Value::Null) => ROWS_DEFAULT,
        Some(v) => v.as_u64().filter(|n| *n >= 1 && *n as usize <= ROWS_MAX).ok_or_else(|| WsError::limit(format!("max_rows must be 1..{ROWS_MAX}")))? as usize,
    };
    let args: Vec<rusqlite::types::Value> = match params.get("params") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(sql_param).collect(),
        Some(_) => return Err(WsError::invalid_op("params must be an array (positional)")),
    };
    register_functions(conn).map_err(db_err)?;
    drop_views(conn)?;
    let state = Arc::new(Mutex::new(AuthState::default()));
    // aliases first: only these tables then, under these names
    if let Some(tables) = params.get("tables").filter(|t| !t.is_null()) {
        let tables = tables.as_object().ok_or_else(|| WsError::invalid_op("tables must be { alias: path or entity_id }"))?;
        if tables.len() > MAX_TABLES {
            return Err(WsError::limit(format!("at most {MAX_TABLES} tables")));
        }
        for (alias, target) in tables {
            if !is_alias(alias) {
                return Err(WsError::invalid_op(format!("table alias {alias:?} must match ^[A-Za-z_][A-Za-z0-9_]{{0,63}}$")));
            }
            let target = target.as_str().ok_or_else(|| WsError::invalid_op("tables values are paths or entity ids"))?;
            let t = table_for(ctx, access, target, names)?;
            create_view(conn, &state, ctx, access, alias, &t)?;
        }
    }
    let deadline = Instant::now() + Duration::from_millis(MAX_MILLIS);
    let timed_out = Arc::new(Mutex::new(false));
    {
        let st = state.clone();
        conn.authorizer(Some(move |a: AuthContext<'_>| authorize(&st, a)));
        let flag = timed_out.clone();
        conn.progress_handler(
            1000,
            Some(move || {
                let over = Instant::now() > deadline;
                if over {
                    *flag.lock().unwrap() = true;
                }
                over
            }),
        );
    }
    let restricted = params.get("tables").is_some_and(|t| !t.is_null());
    let result = (|| -> WsResult<Value> {
        let mut created = 0usize;
        let enforce = |on: bool| state.lock().unwrap().enforcing = on;
        let mut stmt = loop {
            state.lock().unwrap().denied = None;
            enforce(true);
            let prepared = conn.prepare(sql);
            enforce(false);
            match prepared {
                Ok(s) => break s,
                Err(e) => {
                    if let Some(name) = missing_table(&e).filter(|_| !restricted) {
                        created += 1;
                        if created > MAX_TABLES {
                            return Err(WsError::limit(format!("a query names at most {MAX_TABLES} tables")));
                        }
                        let t = table_for(ctx, access, &name, names)?;
                        create_view(conn, &state, ctx, access, &name, &t)?;
                        continue;
                    }
                    if let Some(name) = missing_table(&e) {
                        return Err(WsError::not_found(format!("no table {name:?}")));
                    }
                    let denied = state.lock().unwrap().denied.take();
                    return Err(match denied {
                        Some(what) => WsError::sub(Code::InvalidOperation, "SQL_NOT_ALLOWED", format!("only reading the logical tables is allowed ({what})")),
                        None => WsError::sub(Code::InvalidOperation, "SQL_ERROR", e.to_string()),
                    });
                }
            }
        };
        if !stmt.readonly() {
            return Err(WsError::sub(Code::InvalidOperation, "SQL_NOT_ALLOWED", "only a SELECT is allowed"));
        }
        if stmt.parameter_count() != args.len() {
            return Err(WsError::invalid_op(format!("the query has {} parameters, {} given", stmt.parameter_count(), args.len())));
        }
        let columns: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
        let n = columns.len();
        enforce(true);
        let stepped = (|| -> WsResult<(Vec<Value>, bool)> {
        let mut rows = stmt.query(rusqlite::params_from_iter(args.iter())).map_err(|e| WsError::sub(Code::InvalidOperation, "SQL_ERROR", e.to_string()))?;
        let mut out: Vec<Value> = Vec::new();
        let mut bytes = 0usize;
        let mut truncated = false;
        loop {
            let row = match rows.next() {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(e) => {
                    if *timed_out.lock().unwrap() {
                        return Err(WsError::limit(format!("the query ran longer than {MAX_MILLIS} ms")).with_data(json!({ "limit": "time", "max_ms": MAX_MILLIS })));
                    }
                    return Err(WsError::sub(Code::InvalidOperation, "SQL_ERROR", e.to_string()));
                }
            };
            if out.len() == max_rows {
                truncated = true;
                break;
            }
            let mut cells = Vec::with_capacity(n);
            for i in 0..n {
                let v = json_of(row.get_ref(i).map_err(db_err)?);
                bytes += match &v {
                    Value::String(s) => s.len() + 2,
                    _ => 8,
                };
                cells.push(v);
            }
            if bytes > MAX_RESULT_BYTES {
                return Err(WsError::limit(format!("the result is larger than {} bytes", MAX_RESULT_BYTES)).with_data(json!({ "limit": "bytes", "max_bytes": MAX_RESULT_BYTES })));
            }
            out.push(Value::Array(cells));
        }
        Ok((out, truncated))
        })();
        enforce(false);
        let (out, truncated) = stepped?;
        // collected first: reading the entities below goes through this connection, whose authorizer takes the lock
        let read_views: Vec<(String, String)> = {
            let s = state.lock().unwrap();
            s.read.iter().filter_map(|v| s.views.get(v).map(|id| (v.clone(), id.clone()))).collect()
        };
        let read: Vec<Value> = {
            read_views
                .into_iter()
                .map(|(name, id)| {
                    let e = ctx.entity(&id).ok().flatten();
                    json!({ "table": name, "entity_id": id, "path": aiworkspace_core::path::path_of(ctx, access, &id).unwrap_or_default(),
                            "content_rev": e.map(|e| e.content_rev) })
                })
                .collect()
        };
        Ok(json!({ "columns": columns, "rows": out, "truncated": truncated, "read": read, "head_seq": head_seq }))
    })();
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    conn.progress_handler(0, None::<fn() -> bool>);
    let _ = drop_views(conn);
    result
}

/// The native records (`doc.read`'s record form) a saved view or an ad hoc view of `table` selects, in its order,
/// through the compiled view SQL on `conn` (trusted, internal: a wish stage's inputs and table reads). Returns the
/// page from `offset` (all rows without `limit`), the row count and the compiler's diagnostics.
pub fn view_records(conn: &Connection, ctx: &dyn ReadCtx, access: &Access, table: &EntityRow, view: &aiworkspace_core::sqlview::ViewDef, limit: Option<usize>, offset: usize) -> WsResult<(Vec<Value>, u64, Vec<Value>)> {
    use aiworkspace_core::sqlview::{compile_view, ident, Projection};
    register_functions(conn).map_err(db_err)?;
    drop_views(conn)?;
    let name = "__view_records";
    let ddl = logical_table_ddl(ctx, access, table, name)?;
    conn.execute_batch(&ddl).map_err(db_err)?;
    let result = (|| -> WsResult<(Vec<Value>, u64, Vec<Value>)> {
        let fields = aiworkspace_core::filter::live_fields(ctx, &table.entity_id)?;
        let v = compile_view(&ident(name), &fields, view, Projection::Records)?;
        let total: u64 = conn.query_row(&v.count_sql, [], |r| r.get::<_, i64>(0)).map_err(db_err)? as u64;
        let sql = match limit {
            Some(n) => format!("{} LIMIT {n} OFFSET {offset}", v.sql),
            None => format!("{} LIMIT -1 OFFSET {offset}", v.sql),
        };
        let mut st = conn.prepare(&sql).map_err(db_err)?;
        let rows: Vec<String> = st.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let records = rows.iter().map(|t| serde_json::from_str::<Value>(t).map_err(|e| WsError::io(e.to_string()))).collect::<WsResult<Vec<_>>>()?;
        Ok((records, total, v.diagnostics))
    })();
    let _ = drop_views(conn);
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_statement() {
        assert!(super::single_statement("SELECT 1"));
        assert!(super::single_statement("SELECT 1; "));
        assert!(super::single_statement("SELECT ';' FROM \"a;b\" -- x; y\n"));
        assert!(super::single_statement("SELECT 1; -- done"));
        assert!(!super::single_statement("SELECT 1; SELECT 2"));
        assert!(!super::single_statement("SELECT 1;/* */DELETE FROM x"));
    }
}
