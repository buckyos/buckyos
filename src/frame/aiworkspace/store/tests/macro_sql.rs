//! `aiws.sql` of a macro (AI Native 第一期 §2.3): one SELECT over the declared table inputs only, each a logical
//! table named by its input name whose columns are `_id` and the declared fields under the names the macro was coded
//! with (core `sqlview::manifest_table_ddl`, store `sql::execute_views`, `Stage::macro_sql`). A view or query input is
//! the rows it selects; other tables, kernel tables and writes are refused.

mod common;
use aiworkspace_core::sqlview::{manifest_table_ddl, ViewDef};
use aiworkspace_store::wish::context::{Stage, StageKind, StageOptions};
use aiworkspace_store::workspace::Workspace;
use common::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const SOURCE: &str = "export default async function main(aiws) {}";

/// Orders (月份, 客户, 地区, 销售额, 价格) shown through a 华东 view on a canvas, another table `other` (数), a note,
/// and the wish `w` bound to them: `sales` (the view), `big` (a query on `other`: 数 > 2), `notes`.
fn setup(ws: &mut Workspace) -> Vec<Value> {
    let who = alice();
    ok(ws, &who, json!([
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "画布" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "画布",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "画布", "content_folder_id": "sf-content" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "订单",
          "payload": { "fields": [
            { "field_id": "month", "name": "月份", "type": "date" },
            { "field_id": "customer", "name": "客户", "type": "text" },
            { "field_id": "region", "name": "地区", "type": "select", "options": [{ "option_id": "east", "label": "华东" }, { "option_id": "south", "label": "华南" }] },
            { "field_id": "amount", "name": "销售额", "type": "decimal", "scale": 2 },
            { "field_id": "price", "name": "价格", "type": "currency", "currency": "CNY" } ] } },
        { "op": "entity.create", "entity_id": "other", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "其他",
          "payload": { "fields": [{ "field_id": "n", "name": "数", "type": "number" }, { "field_id": "label", "name": "标签", "type": "text" }] } },
        { "op": "entity.create", "entity_id": "notes", "type_id": "buckyos.richtext", "parent_id": "data", "order_key": "c", "name": "说明",
          "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "口径" }] }] } } },
    ]));
    let rows: Vec<Value> = (0..20)
        .map(|i| {
            let month = ["2026-07-01", "2026-08-01", "2026-09-01"][i % 3];
            json!({ "record_id": format!("o{i:02}"), "values": {
                "month": month, "customer": format!("客户{}", i % 4),
                "region": if i % 4 == 0 { "south" } else { "east" }, "amount": format!("{}.50", 100 + i * 10),
                "price": { "amount": format!("{}.00", i), "currency": if i == 1 { "USD" } else { "CNY" } } } })
        })
        .collect();
    ok(ws, &who, json!([{ "op": "table.insert_records", "source_id": "orders", "records": rows }]));
    let others: Vec<Value> = (0..6).map(|i| json!({ "record_id": format!("x{i}"), "values": { "n": i, "label": format!("l{i}") } })).collect();
    ok(ws, &who, json!([{ "op": "table.insert_records", "source_id": "other", "records": others }]));
    ok(ws, &who, json!([
        { "op": "entity.create", "entity_id": "blk", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 400, "h": 300 },
          "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "orders" }, "title": "华东订单",
                       "filter": { "op": "cmp", "field_id": "region", "operator": "eq", "value": "east" }, "sorts": [{ "field_id": "month", "direction": "desc" }] } },
        { "op": "entity.create", "entity_id": "w", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w", "name": "汇总",
          "payload": { "title": "汇总", "prompt": "汇总", "executor": "xllm", "output_mode": "overwrite", "output": { "container_id": "sf-content", "name": "结果" } } },
    ]));
    let inputs = vec![
        json!({ "entity_id": "orders", "name": "sales", "selector": { "kind": "table_view", "cell_id": "blk" } }),
        json!({ "entity_id": "other", "name": "big", "selector": { "kind": "table_query", "filter": { "op": "cmp", "field_id": "n", "operator": "gt", "value": 2 } } }),
        json!({ "entity_id": "notes", "name": "notes" }),
    ];
    ok(ws, &who, json!([{ "op": "entity.set_keys", "entity_id": "w", "keys": [{ "key": "inputs", "value": inputs, "expect": { "rev": 0 } }] }]));
    inputs
}

fn workspace(env: &Env) -> std::sync::Arc<std::sync::Mutex<Workspace>> {
    let id = env.svc.create_workspace(&alice(), "SQL", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    env.svc.workspace(&id).unwrap()
}

fn open(ws: &Workspace, kind: StageKind, manifest: Option<Value>, dir: &std::path::Path) -> Stage {
    let snap = ws.wish_snapshot(&alice()).unwrap();
    let o = StageOptions {
        kind,
        location: json!({}),
        request: json!({ "today": "2026-10-06" }),
        catalog: json!([]),
        controls: json!({ "controls": [] }),
        map_budget_chars: 8_000,
        materialize: true,
        manifest,
    };
    Stage::open(snap, "w", "run-1", &dir.join("run-1"), &o).unwrap()
}

/// `{ input: [(field_id, name)] }` as the host passes it to `macro_sql`.
fn cols(spec: &[(&str, &[(&str, &str)])]) -> BTreeMap<String, Vec<(String, String)>> {
    spec.iter().map(|(n, f)| (n.to_string(), f.iter().map(|(id, name)| (id.to_string(), name.to_string())).collect())).collect()
}

fn ids(r: &Value) -> Vec<String> {
    let mut v: Vec<String> = r["rows"].as_array().unwrap().iter().map(|r| r[0].as_str().unwrap().to_string()).collect();
    v.sort();
    v
}

#[test]
fn a_macro_reads_declared_columns_of_declared_inputs() {
    let env = env();
    let h = workspace(&env);
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let work = tempfile::tempdir().unwrap();
    let mut st = open(&ws, StageKind::Code, None, work.path());
    let inputs = cols(&[("sales", &[("month", "月份"), ("amount", "销售额"), ("price", "价格")]), ("big", &[("n", "数")])]);
    // the declared columns only: display, raw, and a currency's code
    let r = st.macro_sql("SELECT * FROM sales WHERE _id = 'o01'", None, &inputs).unwrap();
    assert_eq!(r["columns"], json!(["_id", "月份", "月份#raw", "销售额", "销售额#raw", "价格", "价格#raw", "价格#currency"]), "{r}");
    let raw_price = json!({ "amount": "1.00", "currency": "USD" }).to_string();
    assert_eq!(r["rows"][0], json!(["o01", "2026-08-01", "2026-08-01", 110.5, "110.50", 1.0, raw_price, "USD"]), "{r}");
    assert_eq!((r["read"][0]["table"].as_str(), r["read"][0]["entity_id"].as_str()), (Some("sales"), Some("orders")), "{r}");
    // a view input is the rows its view shows: the 华东 rows, though 地区 is not a column
    let r = st.macro_sql("SELECT _id FROM sales", None, &inputs).unwrap();
    let mut shown = view_ids(&ws, &alice(), "blk").unwrap();
    shown.sort();
    assert_eq!(ids(&r), shown);
    assert_eq!(shown.len(), 15);
    // a query input is the rows its query selects
    let r = st.macro_sql("SELECT \"数\" FROM big ORDER BY 1", None, &inputs).unwrap();
    assert_eq!(r["rows"], json!([[3], [4], [5]]), "{r}");
    // SQL over them: params, aggregates, joins between inputs, CTEs
    let r = st.macro_sql("SELECT count(*), sum(\"销售额\") FROM sales WHERE \"销售额\" > ?", Some(&json!([200])), &inputs).unwrap();
    let want: Vec<f64> = (0..20).filter(|i| i % 4 != 0).map(|i| 100.5 + (i * 10) as f64).filter(|a| *a > 200.0).collect();
    assert_eq!(r["rows"][0], json!([want.len(), want.iter().sum::<f64>()]), "{r}");
    let r = st.macro_sql("WITH m AS MATERIALIZED (SELECT \"月份\" AS m, count(*) AS c FROM sales GROUP BY 1) SELECT m, c FROM m ORDER BY m", None, &inputs).unwrap();
    assert_eq!(r["rows"], json!([["2026-07-01", 5], ["2026-08-01", 5], ["2026-09-01", 5]]), "{r}");
    let r = st.macro_sql("SELECT count(*) FROM sales s JOIN big b ON b.\"数\" = 3", None, &inputs).unwrap();
    assert_eq!(r["rows"][0][0], 15);
    assert_eq!(r["read"].as_array().unwrap().len(), 2);

    // undeclared columns do not exist, even the one the view filters on, nor do the system columns besides _id
    // (named unquoted or qualified here: a double-quoted name that is no column reads as a string, see below)
    for q in ["SELECT 客户 FROM sales", "SELECT sales.\"客户\" FROM sales", "SELECT 地区 FROM sales", "SELECT s.\"地区#raw\" FROM sales s", "SELECT _record FROM sales",
              "SELECT _rev FROM sales", "SELECT _body FROM sales", "SELECT big.\"标签\" FROM big"] {
        let e = st.macro_sql(q, None, &inputs).unwrap_err();
        assert_eq!((e.code.as_str(), e.sub), ("INVALID_OPERATION", Some("SQL_ERROR")), "{q}: {e}");
        assert!(e.detail.contains("no such column"), "{q}: {e}");
    }
    // other tables do not exist: not by id, path or name, not another input of this run that was not declared
    for q in ["SELECT * FROM orders", "SELECT * FROM other", "SELECT * FROM \"/data/订单\"", "SELECT * FROM notes", "SELECT * FROM sales, other"] {
        assert_eq!(st.macro_sql(q, None, &inputs).unwrap_err().code.as_str(), "NOT_FOUND", "{q}");
    }
    let only_sales = cols(&[("sales", &[("month", "月份")])]);
    assert_eq!(st.macro_sql("SELECT * FROM big", None, &only_sales).unwrap_err().code.as_str(), "NOT_FOUND", "big is bound but not declared");
    // a rich text input has no logical table; an unknown input is refused
    let with_notes = cols(&[("sales", &[("month", "月份")]), ("notes", &[])]);
    assert_eq!(st.macro_sql("SELECT * FROM notes", None, &with_notes).unwrap_err().code.as_str(), "NOT_FOUND");
    let e = st.macro_sql("SELECT 1", None, &cols(&[("nope", &[])])).unwrap_err();
    assert!(e.detail.contains("nope is not an input of this run"), "{e}");
    // kernel tables, writes, schema changes, PRAGMA, ATTACH and several statements are refused
    for q in [
        "SELECT * FROM table_records",
        "SELECT * FROM main.table_records",
        "SELECT count(*) FROM sales WHERE (SELECT count(*) FROM main.table_records) > 0",
        "SELECT * FROM entities",
        "SELECT * FROM sqlite_master",
        "SELECT * FROM temp.sqlite_master",
        "DELETE FROM sales",
        "UPDATE sales SET \"月份\" = '2026-01-01'",
        "INSERT INTO sales (_id) VALUES ('x')",
        "CREATE TABLE t (a)",
        "DROP VIEW sales",
        "PRAGMA table_info(sales)",
        "ATTACH DATABASE 'x.db' AS x",
        "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r) SELECT n FROM r",
        "SELECT 1; SELECT 2",
    ] {
        let e = st.macro_sql(q, None, &inputs).unwrap_err();
        assert!(matches!(e.code.as_str(), "INVALID_OPERATION" | "NOT_FOUND"), "{q}: {e}");
    }
    for q in ["SELECT * FROM table_records", "SELECT * FROM entities", "SELECT count(*) FROM sales WHERE (SELECT count(*) FROM main.table_records) > 0", "PRAGMA table_info(sales)"] {
        assert_eq!(st.macro_sql(q, None, &inputs).unwrap_err().sub, Some("SQL_NOT_ALLOWED"), "{q}");
    }
    assert_eq!(st.macro_sql("SELECT count(*), sum(\"销售额\") FROM sales", None, &inputs).unwrap()["rows"][0], json!([15, (0..20).filter(|i| i % 4 != 0).map(|i| 100.5 + (i * 10) as f64).sum::<f64>()]),
               "nothing was written");

    // the stage's snapshot holds while the Workspace moves on
    let rev = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_record", "record_id": "o01" }))).unwrap()["content"]["rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "table.delete_records", "source_id": "orders", "records": [{ "record_id": "o01", "expect": { "rev": rev } }] }]));
    assert_eq!(st.macro_sql("SELECT count(*) FROM sales", None, &inputs).unwrap()["rows"][0][0], 15);
    // the logical tables are gone after the query: the next one sees only its own
    assert_eq!(st.macro_sql("SELECT * FROM big", None, &only_sales).unwrap_err().code.as_str(), "NOT_FOUND");
}

/// Running a stored macro: the columns carry the names the macro was coded with, whatever they are called now.
#[test]
fn a_stored_macro_reads_by_its_coding_names() {
    let env = env();
    let h = workspace(&env);
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let manifest = json!({ "inputs": { "sales": { "entity_id": "orders", "type_id": "buckyos.table-source",
                                                  "fields": [{ "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "amount", "name": "销售额", "type": "decimal" }] } },
                           "results": {} });
    let staged = ws.stage_asset(&alice(), SOURCE.as_bytes()).unwrap();
    let program = json!({ "language": "js", "api_version": aiworkspace_core::wish::PROGRAM_API_VERSION, "source": staged["object_id"],
                          "digest": aiworkspace_core::wish::macro_digest(SOURCE, &manifest), "manifest": manifest, "produces": [] });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "w", "keys": [{ "key": "program", "value": program, "expect": { "rev": 0 } }] }]));
    // renamed after coding: 销售额 → 金额, and 客户 takes the old name
    let def_rev = |ws: &Workspace, f: &str| ws.read(&alice(), "orders", Some(&json!({ "kind": "table_field", "field_id": f }))).unwrap()["content"]["def_rev"].as_u64().unwrap();
    let r = def_rev(&ws, "amount");
    ok(&mut ws, &alice(), json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "amount", "changes": { "name": "金额" }, "expect": { "rev": r } }]));
    let r = def_rev(&ws, "customer");
    ok(&mut ws, &alice(), json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "customer", "changes": { "name": "销售额" }, "expect": { "rev": r } }]));
    let work = tempfile::tempdir().unwrap();
    let mut st = open(&ws, StageKind::Run, Some(manifest.clone()), work.path());
    assert_eq!(st.inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["sales"], "only the declared inputs are bound");
    let inputs = cols(&[("sales", &[("month", "月份"), ("amount", "销售额")])]);
    let r = st.macro_sql("SELECT * FROM sales WHERE _id = 'o02'", None, &inputs).unwrap();
    assert_eq!(r["columns"], json!(["_id", "月份", "月份#raw", "销售额", "销售额#raw"]), "{r}");
    assert_eq!(r["rows"][0], json!(["o02", "2026-09-01", "2026-09-01", 120.5, "120.50"]), "the amount, not the customer now called 销售额: {r}");
    assert!(st.macro_sql("SELECT 金额 FROM sales", None, &inputs).unwrap_err().detail.contains("no such column"));
    assert!(st.macro_sql("SELECT sales.\"金额\" FROM sales", None, &inputs).unwrap_err().detail.contains("no such column"));
    // the view still decides the rows, by its filter on a field the macro never declared
    assert_eq!(st.macro_sql("SELECT count(*) FROM sales", None, &inputs).unwrap()["rows"][0][0], 15);
    // an input bound but not declared is not even an input of this run
    assert!(st.macro_sql("SELECT 1", None, &cols(&[("big", &[("n", "数")])])).unwrap_err().detail.contains("big is not an input of this run"));
}

/// The pieces under `macro_sql`: `manifest_table_ddl` builds one logical table from `(field_id, name)` pairs (a gone
/// field has no column, a system column's name gets `#field`, a view keeps its rows), and `execute_views` runs a query
/// over exactly the tables it is given.
#[test]
fn manifest_tables_and_execute_views() {
    let env = env();
    let h = workspace(&env);
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let snap = ws.wish_snapshot(&alice()).unwrap();
    let ctx = snap.ctx();
    let table = snap.entity("orders").unwrap().unwrap();
    let columns = vec![("customer".to_string(), "_rev".to_string()), ("gone".to_string(), "旧字段".to_string()), ("amount".to_string(), "额".to_string())];
    let east = ViewDef::of_params(&json!({ "filter": { "op": "cmp", "field_id": "region", "operator": "eq", "value": "south" } }));
    let ddl = manifest_table_ddl(&ctx, &snap.access, &table, "t", &columns, Some(&east)).unwrap();
    let all = manifest_table_ddl(&ctx, &snap.access, &table, "u", &columns[..1], None).unwrap();
    let run = |q: &str, views: Vec<(String, String, String)>| aiworkspace_store::sql::execute_views(snap.conn(), &ctx, &snap.access, &json!({ "sql": q }), snap.head_seq, views);
    let r = run("SELECT * FROM t ORDER BY _id", vec![("t".into(), "orders".into(), ddl.clone())]).unwrap();
    assert_eq!(r["columns"], json!(["_id", "_rev#field", "_rev#field#raw", "额", "额#raw"]), "{r}");
    assert_eq!(ids(&r), ["o00", "o04", "o08", "o12", "o16"], "the view's rows");
    assert_eq!(r["rows"][0], json!(["o00", "客户0", "客户0", 100.5, "100.50"]));
    assert_eq!(r["head_seq"], json!(snap.head_seq));
    // two tables, joined; the read set names both
    let r = run("SELECT count(*) FROM t JOIN u ON t._id = u._id", vec![("t".into(), "orders".into(), ddl.clone()), ("u".into(), "orders".into(), all.clone())]).unwrap();
    assert_eq!(r["rows"][0][0], 5);
    assert_eq!(r["read"].as_array().unwrap().iter().map(|x| x["table"].as_str().unwrap()).collect::<Vec<_>>(), ["t", "u"]);
    // nothing else: no table is made up for a name the query uses, kernel tables and writes are refused
    assert_eq!(run("SELECT * FROM orders", vec![("t".into(), "orders".into(), ddl.clone())]).unwrap_err().code.as_str(), "NOT_FOUND");
    assert_eq!(run("SELECT * FROM \"/data/其他\"", vec![]).unwrap_err().code.as_str(), "NOT_FOUND");
    assert_eq!(run("SELECT * FROM table_records", vec![("t".into(), "orders".into(), ddl.clone())]).unwrap_err().sub, Some("SQL_NOT_ALLOWED"));
    assert_eq!(run("DELETE FROM t", vec![("t".into(), "orders".into(), ddl.clone())]).unwrap_err().code.as_str(), "INVALID_OPERATION");
    assert_eq!(run("SELECT count(*) FROM t", vec![("t".into(), "orders".into(), ddl.clone())]).unwrap()["rows"][0][0], 5, "nothing was deleted");
    // the budgets of `execute` hold: rows truncate
    let r = aiworkspace_store::sql::execute_views(snap.conn(), &ctx, &snap.access, &json!({ "sql": "SELECT _id FROM u", "max_rows": 3 }), snap.head_seq, vec![("u".into(), "orders".into(), all)]).unwrap();
    assert_eq!((r["rows"].as_array().unwrap().len(), r["truncated"].as_bool()), (3, Some(true)));
}

/// A double-quoted name that is not a column must be an error: a macro coded against a field that was renamed,
/// deleted or never declared would otherwise compute on the name itself. SQLite's legacy "double-quoted string"
/// fallback turns `"金额"` into the string literal '金额' when no column has that name, so the query runs and
/// `sum("金额")` is silently 0. The executor (`store::sql`) does not turn the fallback off (`SQLITE_DBCONFIG_DQS_DML`).
#[test]
fn an_unknown_quoted_column_is_an_error() {
    let env = env();
    let h = workspace(&env);
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let work = tempfile::tempdir().unwrap();
    let mut st = open(&ws, StageKind::Code, None, work.path());
    let inputs = cols(&[("sales", &[("amount", "销售额")])]);
    for q in ["SELECT \"金额\" FROM sales", "SELECT sum(\"客户\") FROM sales", "SELECT count(*) FROM sales WHERE \"地区\" = '华东'"] {
        let r = st.macro_sql(q, None, &inputs);
        assert!(r.as_ref().is_err_and(|e| e.detail.contains("no such column")), "{q}: {r:?}");
    }
    // the same holds for `doc.query`
    let r = sql(&ws, &alice(), "SELECT sum(\"金额\") FROM orders");
    assert!(r.is_err(), "{r:?}");
}
