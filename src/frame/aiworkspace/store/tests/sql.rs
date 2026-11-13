//! SQL over the logical tables (核心 API 调整 §3.4, §3.5; design draft §7): the authorizer, the budgets, the read set,
//! and the compiled view SQL against the core evaluator (`filter::run_query`) row for row.

mod common;
use aiworkspace_core::filter::{run_query, QuerySpec};
use aiworkspace_core::sqlview::ViewDef;
use common::*;
use serde_json::{json, Value};

fn table(ws: &mut aiworkspace_store::Workspace) {
    ok(ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "sales-folder", "type_id": "buckyos.container", "parent_id": "data", "order_key": "s", "name": "销售", "payload": { "kind": "folder" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "sales-folder", "order_key": "a", "name": "订单",
          "payload": { "fields": [
            { "field_id": "month", "name": "月份", "type": "date" },
            { "field_id": "customer", "name": "客户", "type": "text" },
            { "field_id": "region", "name": "地区", "type": "select", "options": [{ "option_id": "east", "label": "华东" }, { "option_id": "south", "label": "华南" }] },
            { "field_id": "amount", "name": "销售额", "type": "decimal", "scale": 2 },
            { "field_id": "n", "name": "数量", "type": "number" },
            { "field_id": "ok", "name": "完成", "type": "boolean" },
            { "field_id": "tags", "name": "标签", "type": "multi_select", "options": [{ "option_id": "a", "label": "甲" }, { "option_id": "b", "label": "乙" }] },
            { "field_id": "price", "name": "价格", "type": "currency", "currency": "CNY" } ] } },
    ]));
    let rows: Vec<Value> = (0..40)
        .map(|i| {
            let mut v = json!({ "customer": format!("客户{}", i % 7), "region": if i % 4 == 0 { "south" } else { "east" }, "amount": format!("{}.50", 100 + (i * 37) % 90) });
            if i % 5 != 0 {
                v["month"] = json!(["2026-07-01", "2026-08-01", "2026-09-01"][i % 3]);
            }
            if i % 3 != 0 {
                v["n"] = json!(i % 6);
            }
            if i % 2 == 0 {
                v["ok"] = json!(i % 4 == 0);
            }
            v["tags"] = match i % 4 { 0 => json!([]), 1 => json!(["a"]), 2 => json!(["a", "b"]), _ => json!(["b"]) };
            if i % 7 != 0 {
                v["price"] = json!({ "amount": format!("{}.00", i * 3 % 50), "currency": if i % 6 == 0 { "USD" } else { "CNY" } });
            }
            json!({ "record_id": format!("o{i:02}"), "values": v })
        })
        .collect();
    ok(ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "orders", "records": rows }]));
}

fn ids_of(q: &Value) -> Vec<String> {
    q["rows"].as_array().unwrap().iter().map(|r| r[0].as_str().unwrap().to_string()).collect()
}

#[test]
fn logical_tables_and_the_authorizer() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "SQL", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    table(&mut ws);
    // by path, by id, by alias; display and raw columns
    let q = sql(&ws, &alice(), r#"SELECT "地区", "地区#raw", "销售额", "价格", "价格#currency", "标签" FROM "/data/销售/订单" WHERE _id = 'o01'"#).unwrap();
    assert_eq!(q["rows"][0], json!(["华东", "east", 137.5, 3.0, "CNY", "[\"甲\"]"]), "{q}");
    assert_eq!(q["read"][0]["path"], "/data/销售/订单");
    let q = sql(&ws, &alice(), "SELECT count(*) FROM orders").unwrap();
    assert_eq!(q["rows"][0][0], 40);
    let q = ws.query(&alice(), &json!({ "sql": "SELECT \"客户\", sum(\"销售额\") AS s FROM t GROUP BY 1 ORDER BY s DESC LIMIT 2", "tables": { "t": "/data/销售/订单" } })).unwrap();
    assert_eq!(q["rows"].as_array().unwrap().len(), 2);
    // params, joins, CTEs and window functions work
    let q = ws.query(&alice(), &json!({ "sql": "WITH x AS (SELECT _id, row_number() OVER (ORDER BY _id) AS k FROM orders) SELECT a._id FROM x a JOIN orders b ON a._id = b._id WHERE a.k <= ?", "params": [3] })).unwrap();
    assert_eq!(ids_of(&q), vec!["o00", "o01", "o02"]);
    // kernel tables, writes, ATTACH, PRAGMA, recursive CTEs and the raw record table are refused
    for bad in [
        "SELECT * FROM entities",
        "SELECT * FROM table_records",
        "SELECT * FROM sqlite_master",
        "SELECT * FROM orders o WHERE (SELECT count(*) FROM grants) > 0",
        "DELETE FROM orders",
        "PRAGMA table_info(entities)",
        "ATTACH DATABASE 'x.db' AS x",
        "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r) SELECT n FROM r",
        "SELECT load_extension('x')",
        "SELECT 1; SELECT 2",
    ] {
        let e = sql(&ws, &alice(), bad).unwrap_err();
        assert!(matches!(e.code.as_str(), "INVALID_OPERATION" | "NOT_FOUND"), "{bad}: {e}");
    }
    assert_eq!(sql(&ws, &alice(), "SELECT * FROM entities").unwrap_err().sub, Some("SQL_NOT_ALLOWED"));
    assert_eq!(sql(&ws, &alice(), "SELEC 1").unwrap_err().sub, Some("SQL_ERROR"));
    // a table the caller cannot read does not exist
    ws.grant(&alice(), "bob", Some("data"), &["read".into()]).unwrap();
    assert_eq!(sql(&ws, &bob(), "SELECT count(*) FROM orders").unwrap()["rows"][0][0], 40);
    ws.revoke(&alice(), "bob", Some("data")).unwrap();
    assert_eq!(sql(&ws, &bob(), "SELECT count(*) FROM orders").unwrap_err().code.as_str(), "NOT_FOUND");
    assert_eq!(sql(&ws, &alice(), "SELECT * FROM \"/data/没有\"").unwrap_err().code.as_str(), "NOT_FOUND");
    // budgets: rows truncate, time is a structured error
    let q = ws.query(&alice(), &json!({ "sql": "SELECT _id FROM orders", "max_rows": 5 })).unwrap();
    assert_eq!((q["rows"].as_array().unwrap().len(), q["truncated"].as_bool()), (5, Some(true)));
    let e = sql(&ws, &alice(), "SELECT count(*) FROM orders a, orders b, orders c, orders d, orders e").unwrap_err();
    assert_eq!((e.code.as_str(), e.data.as_ref().map(|d| d["limit"].clone())), ("LIMIT_EXCEEDED", Some(json!("time"))), "{e}");
    // the head a Desktop expects
    let e = ws.query(&alice(), &json!({ "sql": "SELECT 1", "expect_head": { "epoch": ws.epoch, "seq": 0 } })).unwrap_err();
    assert_eq!((e.code.as_str(), e.sub), ("SNAPSHOT_EXPIRED", Some("HEAD_MISMATCH")));
    let e = ws.query(&alice(), &json!({ "source_id": "orders" })).unwrap_err();
    assert_eq!(e.code.as_str(), "INVALID_OPERATION", "the old table query is gone");
}

/// The compiled SQL of a view gives the evaluator's rows, order and group counts.
#[test]
fn compiled_views_match_the_evaluator() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "SQL", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    table(&mut ws);
    let views = [
        json!({}),
        json!({ "sorts": [{ "field_id": "month", "direction": "desc" }] }),
        json!({ "sorts": [{ "field_id": "month", "direction": "asc" }, { "field_id": "amount", "direction": "desc" }] }),
        json!({ "sorts": [{ "field_id": "region", "direction": "desc" }, { "field_id": "n", "direction": "asc" }] }),
        json!({ "sorts": [{ "field_id": "price", "direction": "asc" }] }),
        json!({ "sorts": [{ "field_id": "ok", "direction": "desc" }] }),
        json!({ "filter": { "op": "cmp", "field_id": "region", "operator": "eq", "value": "east" }, "sorts": [{ "field_id": "month", "direction": "desc" }] }),
        json!({ "filter": { "op": "cmp", "field_id": "amount", "operator": "gt", "value": "150" } }),
        json!({ "filter": { "op": "cmp", "field_id": "amount", "operator": "eq", "value": "107.5" } }),
        json!({ "filter": { "op": "cmp", "field_id": "n", "operator": "ne", "value": 2 } }),
        json!({ "filter": { "op": "not", "arg": { "op": "cmp", "field_id": "n", "operator": "eq", "value": 2 } } }),
        json!({ "filter": { "op": "cmp", "field_id": "month", "operator": "is_empty" } }),
        json!({ "filter": { "op": "cmp", "field_id": "tags", "operator": "is_empty" } }),
        json!({ "filter": { "op": "cmp", "field_id": "tags", "operator": "has_any", "value": ["b"] } }),
        json!({ "filter": { "op": "cmp", "field_id": "tags", "operator": "has_all", "value": ["a", "b"] } }),
        json!({ "filter": { "op": "cmp", "field_id": "customer", "operator": "contains", "value": "户3" } }),
        json!({ "filter": { "op": "cmp", "field_id": "customer", "operator": "starts_with", "value": "客户1" } }),
        json!({ "filter": { "op": "cmp", "field_id": "price", "operator": "gte", "value": { "amount": "20", "currency": "CNY" } } }),
        json!({ "filter": { "op": "cmp", "field_id": "price", "operator": "ne", "value": { "amount": "3", "currency": "CNY" } } }),
        json!({ "filter": { "op": "cmp", "field_id": "ok", "operator": "eq", "value": false } }),
        json!({ "filter": { "op": "or", "args": [{ "op": "cmp", "field_id": "n", "operator": "in", "value": [1, 3] }, { "op": "cmp", "field_id": "region", "operator": "eq", "value": "south" }] } }),
        json!({ "filter": { "op": "cmp", "field_id": "month", "operator": "gte", "value": "2026-08-01" }, "group": "region" }),
        json!({ "group": "tags" }),
        json!({ "group": "ok" }),
        json!({ "group": "month" }),
        json!({ "manual_order": { "o05": "a", "o03": "b", "o30": "c" } }),
    ];
    let ctx_ids = |ws: &aiworkspace_store::Workspace, v: &Value| -> (Vec<String>, Option<Value>) {
        let mut spec = QuerySpec { limit: 1000, best_effort: true, ..Default::default() };
        let def = ViewDef::of_params(v);
        spec.filter = def.filter.clone();
        spec.sorts = def.sorts.clone();
        spec.group = def.group.clone();
        spec.manual_order = def.manual_order.clone();
        let rows = ws.read_ctx(|ctx| run_query(ctx, &ctx.entity("orders")?.unwrap(), &spec)).unwrap();
        (rows["rows"].as_array().unwrap().iter().map(|r| r["record_id"].as_str().unwrap().to_string()).collect(), rows.get("groups").cloned())
    };
    for v in views {
        let (want, groups) = ctx_ids(&ws, &v);
        let fields = ws.read_ctx(|ctx| aiworkspace_core::filter::live_fields(ctx, "orders")).unwrap();
        let c = aiworkspace_core::sqlview::compile_view("\"/data/销售/订单\"", &fields, &ViewDef::of_params(&v), aiworkspace_core::sqlview::Projection::Display).unwrap();
        let got = ids_of(&sql(&ws, &alice(), &c.sql).unwrap());
        assert_eq!(got, want, "{v}\n{}", c.sql);
        let n = sql(&ws, &alice(), &c.count_sql).unwrap()["rows"][0][0].as_u64().unwrap();
        assert_eq!(n as usize, want.len(), "{v}");
        if let Some(g) = groups {
            let q = sql(&ws, &alice(), c.group_sql.as_ref().unwrap()).unwrap();
            let got: Vec<Value> = q["rows"].as_array().unwrap().iter().map(|r| json!({ "key": r[0], "count": r[1] })).collect();
            assert_eq!(Value::Array(got), g, "{v}\n{}", c.group_sql.unwrap());
        }
    }
}

/// A view's `fields` keep only those fields in the records the compiled SQL returns, as the evaluator does.
#[test]
fn view_fields_project_records_as_the_evaluator_does() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "SQL", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    table(&mut ws);
    let v = json!({ "fields": ["customer", "amount", "gone"], "sorts": [{ "field_id": "amount", "direction": "desc" }] });
    let def = ViewDef::of_params(&v);
    let mut spec = QuerySpec { limit: 1000, best_effort: true, fields: def.fields.clone(), sorts: def.sorts.clone(), ..Default::default() };
    spec.with_meta = true;
    let want = ws.read_ctx(|ctx| run_query(ctx, &ctx.entity("orders")?.unwrap(), &spec)).unwrap();
    let fields = ws.read_ctx(|ctx| aiworkspace_core::filter::live_fields(ctx, "orders")).unwrap();
    let c = aiworkspace_core::sqlview::compile_view("\"/data/销售/订单\"", &fields, &def, aiworkspace_core::sqlview::Projection::Records).unwrap();
    assert_eq!(c.diagnostics, vec![json!({ "key": "fields", "code": "FIELD_DELETED", "field_id": "gone" })]);
    let got = sql(&ws, &alice(), &c.sql).unwrap();
    let rows: Vec<Value> = got["rows"].as_array().unwrap().iter().map(|r| serde_json::from_str(r[0].as_str().unwrap()).unwrap()).collect();
    let want = want["rows"].as_array().unwrap();
    assert_eq!(rows.len(), want.len());
    for (g, w) in rows.iter().zip(want) {
        assert_eq!((&g["record_id"], &g["values"], &g["revs"]), (&w["record_id"], &w["values"], &w["revs"]), "{}", c.sql);
        let keys: Vec<&String> = g["values"].as_object().unwrap().keys().collect();
        assert!(keys.iter().all(|k| *k == "customer" || *k == "amount"), "{g}");
    }
}

/// A wish stage reads through its snapshot: SQL and views on the snapshot's connection see the snapshot.
#[test]
fn snapshot_reads_hold_while_the_workspace_moves_on() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "SQL", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    table(&mut ws);
    let snap = ws.wish_snapshot(&alice()).unwrap();
    let rev = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_record", "record_id": "o01" }))).unwrap()["content"]["rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "table.delete_records", "source_id": "orders", "records": [{ "record_id": "o01", "expect": { "rev": rev } }] }]));
    for q in ["SELECT _id FROM orders ORDER BY _id", "SELECT _record FROM orders ORDER BY _id DESC"] {
        let r = aiworkspace_store::sql::execute(snap.conn(), &snap.ctx(), &snap.access, &json!({ "sql": q, "max_rows": 3 }), snap.head_seq, None).unwrap();
        assert_eq!(r["rows"].as_array().unwrap().len(), 3, "{q}: {r}");
        assert_eq!(r["read"][0]["entity_id"], "orders");
    }
    let def = ViewDef::of_params(&json!({ "sorts": [{ "field_id": "month", "direction": "desc" }] }));
    let table = snap.entity("orders").unwrap().unwrap();
    let (rows, total, _) = aiworkspace_store::sql::view_records(snap.conn(), &snap.ctx(), &snap.access, &table, &def, Some(5), 0).unwrap();
    assert_eq!((rows.len(), total), (5, 40));
    let q = aiworkspace_store::sql::execute(snap.conn(), &snap.ctx(), &snap.access, &json!({ "sql": "SELECT count(*) FROM orders" }), snap.head_seq, None).unwrap();
    assert_eq!(q["rows"][0][0], 40);
    assert_eq!(sql(&ws, &alice(), "SELECT count(*) FROM orders").unwrap()["rows"][0][0], 39);
}

/// A URL table has no local rows (核心 API 调整 R7): its body is its definition, a text query covers its name and
/// definition and says it skipped the rows, SQL refuses it by name; its rows are `doc.source_query`'s.
#[test]
fn a_url_table_is_definition_only() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "URL", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "events", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "v", "name": "事件",
        "payload": { "data_mode": "url_query", "title_field_id": "event_id",
            "source_ref": { "kind": "url_query", "source_url": "fixture://events?rows=10", "query": {}, "version": { "mode": "live_head" }, "consistency": "best_effort" },
            "fields": [{ "field_id": "event_id", "name": "事件编号", "type": "text" }] } }]));
    let body = ws.read_as(&alice(), &json!({ "entity_id": "events", "as": "body" })).unwrap();
    assert_eq!((body["state"].as_str(), body["complete"].as_bool()), (Some("definition"), Some(true)), "{body}");
    assert!(body["text"].as_str().unwrap().contains("事件编号"), "{body}");
    let m = meta(&ws, &alice(), "events");
    assert_eq!((m["data"]["mode"].as_str(), m["data"]["write"].as_str()), (Some("url_query"), Some("none")));
    // a text query finds the definition and reports the rows it could not search
    let q = ws.query(&alice(), &json!({ "text": "事件编号" })).unwrap();
    assert!(q["hits"].as_array().unwrap().iter().any(|h| h["entity_id"] == "events"), "{q}");
    assert!(q["coverage"]["skipped"].as_array().unwrap().iter().any(|s| s["entity_id"] == "events" && s["reason"] == "definition"), "{q}");
    // SQL has no rows to read
    let e = sql(&ws, &alice(), r#"SELECT count(*) FROM "/data/事件""#).unwrap_err();
    assert_eq!((e.code.as_str(), e.sub), ("INVALID_OPERATION", Some("NOT_MATERIALIZED")), "{e}");
}
