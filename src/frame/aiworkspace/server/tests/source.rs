//! Sources and publication by path (内核架构改进 M4): a Workspace publishes a table, another inserts it as a
//! live Source and refreshes it (one Commit of origin `source`), the sourced copy is read-only, the publisher
//! lists its referrer, a fixed Source pins a content root, a public URL (CSV) becomes a table, a failed fetch
//! keeps the last valid version, and an unpublished name answers nothing.

#[path = "common/wishkit.rs"]
mod wishkit;
use axum::routing::get;
use serde_json::{json, Value};
use wishkit::*;

async fn csv_server() -> String {
    let app = axum::Router::new()
        .route("/data.csv", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8")], "名称,数量,备注\n甲,1,\"a, b\"\n乙,2,\n") }))
        .route("/rows.json", get(|| async { axum::Json(json!([{ "id": "x1", "city": "上海", "n": 3 }, { "id": "x2", "city": "北京", "n": 5 }])) }))
        .route("/note.md", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/markdown")], "# 标题\n\n第一段。\n") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A publication link of this service pasted as a plain URL (`kind: url`) is read directly, never fetched over
/// HTTP (the Zone host here does not resolve); the link's host label is the workspace id without `ws_`.
#[test]
fn own_publication_link_reads_directly() {
    run_test(|| async move {
    let limits = aiworkspace_server::Limits { public_base: Some("https://aiws-{workspace_id}.zone.invalid".into()), ..Default::default() };
    let mut srv = Srv::start_with_limits(tempfile::tempdir().unwrap(), model(), json!({ "tok-alice": { "principal": "alice" } }), limits).await;
    let a = srv.rpc("ws.create", json!({ "title": "销售" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.ws = a.clone();
    srv.commit(json!([{ "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "订单",
        "payload": { "fields": [{ "field_id": "month", "name": "月份", "type": "text" }] } }])).await;
    srv.commit(json!([{ "op": "table.insert_records", "source_id": "orders", "records": [{ "record_id": "r1", "values": { "month": "2026-07" } }, { "record_id": "r2", "values": { "month": "2026-08" } }] }])).await;
    let published = srv.rpc("ws.publish", json!({ "entity_id": "orders", "name": "销售表" })).await;
    let label = a.strip_prefix("ws_").unwrap();
    let url = published["url"].as_str().unwrap().to_string();
    assert!(url.starts_with(&format!("https://aiws-{label}.zone.invalid/")), "{url}");
    assert_eq!(srv.rpc("ws.get_info", json!({})).await["public_base"], json!(format!("https://aiws-{label}.zone.invalid")));
    srv.ws = String::new();
    let b = srv.rpc("ws.create", json!({ "title": "消费方" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.ws = b.clone();
    let by_path = format!("https://zone.invalid/kapi/aiworkspace/pub/{a}/orders");
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "by_host", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "host", "payload": { "source": { "kind": "url", "url": url, "mode": "live" } } },
        { "op": "entity.create", "entity_id": "by_path", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "path", "payload": { "source": { "kind": "url", "url": by_path, "mode": "live" } } },
        { "op": "entity.create", "entity_id": "own_zone", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "c", "name": "zone", "payload": { "source": { "kind": "workspace", "workspace_id": a, "entity_id": "orders", "zone": "zone.invalid", "mode": "live" } } },
    ])).await;
    for (key, entity) in [("h", "by_host"), ("p", "by_path"), ("z", "own_zone")] {
        let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": key, "params": { "entity_id": entity } })).await;
        let done = srv.wait(run["run_id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "succeeded", "{entity}: {done}");
        assert_eq!(srv.rpc("doc.query", json!({ "source_id": entity, "limit": 10 })).await["rows"].as_array().unwrap().len(), 2, "{entity}");
    }
    srv.ws = a.clone();
    let referrers = srv.rpc("ws.list_referrers", json!({ "entity_id": "orders" })).await;
    for entity in ["by_host", "by_path", "own_zone"] {
        assert!(referrers["referrers"].as_array().unwrap().iter().any(|r| r["referrer"]["workspace_id"] == json!(b) && r["referrer"]["entity_id"] == json!(entity)), "{entity}: {referrers}");
    }
    });
}

#[test]
fn publish_insert_refresh_and_unpublish() {
    run_test(|| async move {
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
    // the publisher
    let a = srv.rpc("ws.create", json!({ "title": "发布方 季度" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.ws = a.clone();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "订单",
          "payload": { "fields": [{ "field_id": "month", "name": "月份", "type": "text" }, { "field_id": "amount", "name": "金额", "type": "number" }] } },
        { "op": "entity.create", "entity_id": "notes", "type_id": "buckyos.richtext", "parent_id": "data", "order_key": "b", "name": "说明",
          "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "口径说明" }] }] } } },
    ])).await;
    srv.commit(json!([{ "op": "table.insert_records", "source_id": "orders", "records": [
        { "record_id": "r1", "values": { "month": "2026-07", "amount": 10 } }, { "record_id": "r2", "values": { "month": "2026-08", "amount": 20 } }, { "record_id": "r3", "values": { "month": "2026-09", "amount": 30 } }] }])).await;
    let published = srv.rpc("ws.publish", json!({ "entity_id": "orders" })).await;
    assert_eq!((published["name"].as_str(), published["path"].as_str()), (Some("orders"), Some("/workspace/orders")), "{published}");
    assert_eq!(published["reference"], json!({ "workspace_id": a, "entity_id": "orders", "mode": "live" }));
    srv.rpc("ws.publish", json!({ "entity_id": "notes", "name": "Notes" })).await;
    let listed = srv.rpc("ws.list_publications", json!({})).await;
    assert_eq!(listed["publications"].as_array().unwrap().len(), 2, "{listed}");
    // the publication over HTTP, without a session: JSON for a consumer, HTML for a crawler
    let pub_url = format!("{}/pub/{a}/orders", srv.base);
    let v: Value = srv.http.get(&pub_url).send().await.unwrap().json().await.unwrap();
    assert_eq!((v["format"].as_str(), v["type_id"].as_str(), v["content"]["records"].as_array().map(Vec::len)), (Some("buckyos.ai-workspace/pub/1.0"), Some("buckyos.table-source"), Some(3)), "{v}");
    let html = srv.http.get(&pub_url).header("accept", "text/html").send().await.unwrap().text().await.unwrap();
    assert!(html.contains("<table>") && html.contains("2026-08") && html.contains("<h1>"), "{html}");
    let fixed: Value = srv.http.get(format!("{pub_url}?fixed=1")).send().await.unwrap().json().await.unwrap();
    let root = fixed["content_root"].as_str().unwrap_or_else(|| panic!("fixed: {fixed}")).to_string();
    assert_eq!(fixed["reference"]["mode"], "fixed");
    let by_id: Value = srv.http.get(format!("{}/pub/{a}/orders", srv.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(by_id["name"], "orders");
    // the consumer inserts the link as a live Source and refreshes it
    srv.ws = String::new();
    let b = srv.rpc("ws.create", json!({ "title": "消费方" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.ws = b.clone();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "imported", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "订单（引用）",
          "payload": { "source": { "kind": "workspace", "workspace_id": a, "entity_id": "orders", "mode": "live", "name": "imported" } } },
        { "op": "entity.create", "entity_id": "pinned", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "订单（固定）",
          "payload": { "source": { "kind": "workspace", "workspace_id": a, "entity_id": "orders", "mode": "fixed", "version": { "content_root": root } } } },
        { "op": "entity.create", "entity_id": "doc", "type_id": "buckyos.richtext", "parent_id": "data", "order_key": "c", "name": "说明（引用）",
          "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "empty" }, "content": [] }] }, "source": { "kind": "workspace", "workspace_id": a, "entity_id": "notes", "mode": "live" } } },
    ])).await;
    let outline = srv.rpc("doc.outline", json!({})).await;
    let imported = outline["entities"].as_array().unwrap().iter().find(|e| e["entity_id"] == "imported").unwrap();
    assert_eq!((imported["source"]["kind"].as_str(), imported["source"]["mode"].as_str()), (Some("workspace"), Some("live")), "{imported}");
    let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": "r1", "params": { "entity_id": "imported" } })).await;
    let done = srv.wait(run["run_id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "succeeded", "{done}");
    assert_eq!(done["result"]["status"], "accepted", "{done}");
    let read = srv.rpc("doc.read", json!({ "entity_id": "imported" })).await;
    assert_eq!(read["content"]["fields"].as_array().unwrap().len(), 2, "{read}");
    let rows = srv.rpc("doc.query", json!({ "source_id": "imported", "limit": 10 })).await;
    assert_eq!(rows["rows"].as_array().unwrap().len(), 3, "{rows}");
    let epoch = srv.rpc("ws.get_info", json!({})).await["epoch"].clone();
    let commit = srv.rpc("doc.get_changes", json!({ "epoch": epoch, "after_seq": 0, "limit": 20, "filter": { "detail": "touched" } })).await;
    assert!(commit["changes"].as_array().unwrap().iter().any(|e| e["origin"] == "source"), "the refresh commit has origin source: {commit}");
    // a sourced copy is read-only for people and Blocks
    let rev = srv.rpc("doc.read", json!({ "entity_id": "imported", "selector": { "kind": "table_record", "record_id": "r1" } })).await["content"]["rev"].clone();
    let refused = srv.call_as("tok-alice", "doc.commit", json!({ "workspace_id": b, "protocol_version": "1.0", "epoch": srv.rpc("ws.get_info", json!({})).await["epoch"], "idempotency_key": "edit-1", "session_id": "s1",
        "operations": [{ "op": "table.set_values", "source_id": "imported", "values": [{ "record_id": "r1", "field_id": "amount", "value": 99, "expect": { "rev": rev } }] }] })).await;
    assert_eq!(refused["result"]["errors"][0]["sub_code"], "SOURCE_READ_ONLY", "{refused}");
    // the fixed copy pins the content root; the rich text copy follows
    let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": "r2", "params": { "entity_id": "pinned" } })).await;
    let done = srv.wait(run["run_id"].as_str().unwrap()).await;
    assert_eq!((done["state"].as_str(), done["result"]["content_root"].as_str()), (Some("succeeded"), Some(root.as_str())), "{done}");
    let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": "r3", "params": { "entity_id": "doc" } })).await;
    let done = srv.wait(run["run_id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "succeeded", "{done}");
    assert!(srv.rpc("doc.read", json!({ "entity_id": "doc" })).await["content"]["content"]["content"][0]["content"][0]["text"] == json!("口径说明"));
    // the publisher sees its referrer; its change reaches the live copy on the next refresh, not the pinned one
    srv.ws = a.clone();
    let referrers = srv.rpc("ws.list_referrers", json!({ "entity_id": "orders" })).await;
    assert!(referrers["referrers"].as_array().unwrap().iter().any(|r| r["referrer"]["workspace_id"] == json!(b)), "{referrers}");
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "r1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "r1", "field_id": "amount", "value": 11, "expect": { "rev": rev } }] },
                      { "op": "table.insert_records", "source_id": "orders", "records": [{ "record_id": "r4", "values": { "month": "2026-10", "amount": 40 } }] }])).await;
    srv.ws = b.clone();
    let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": "r4", "params": { "entity_id": "imported" } })).await;
    assert_eq!(srv.wait(run["run_id"].as_str().unwrap()).await["state"], "succeeded");
    let rows = srv.rpc("doc.query", json!({ "source_id": "imported", "limit": 10 })).await;
    assert_eq!(rows["rows"].as_array().unwrap().len(), 4);
    assert!(rows["rows"].as_array().unwrap().iter().any(|r| r["record_id"] == "r1" && r["values"]["amount"] == json!(11)), "{rows}");
    let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": "r5", "params": { "entity_id": "pinned" } })).await;
    assert_eq!(srv.wait(run["run_id"].as_str().unwrap()).await["state"], "succeeded");
    assert_eq!(srv.rpc("doc.query", json!({ "source_id": "pinned", "limit": 10 })).await["rows"].as_array().unwrap().len(), 3, "a fixed Source does not follow");
    // a public URL: CSV and JSON become tables, Markdown a rich text; a failed fetch keeps the last version
    let http = csv_server().await;
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "csv", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "d", "name": "csv", "payload": { "source": { "kind": "url", "url": format!("{http}/data.csv"), "refresh": { "interval_s": 3600 } } } },
        { "op": "entity.create", "entity_id": "rows", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "e", "name": "rows", "payload": { "source": { "kind": "url", "url": format!("{http}/rows.json") } } },
        { "op": "entity.create", "entity_id": "md", "type_id": "buckyos.richtext", "parent_id": "data", "order_key": "f", "name": "md", "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "empty" }, "content": [] }] }, "source": { "kind": "url", "url": format!("{http}/note.md") } } },
    ])).await;
    for (id, key) in [("csv", "u1"), ("rows", "u2"), ("md", "u3")] {
        let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": key, "params": { "entity_id": id } })).await;
        let done = srv.wait(run["run_id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "succeeded", "{id}: {done}");
    }
    let csv = srv.rpc("doc.read", json!({ "entity_id": "csv" })).await;
    let types: Vec<(String, String)> = csv["content"]["fields"].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_string(), f["type"].as_str().unwrap().to_string())).collect();
    assert_eq!(types, vec![("名称".into(), "text".into()), ("数量".into(), "number".into()), ("备注".into(), "text".into())], "{csv}");
    let rows = srv.rpc("doc.query", json!({ "source_id": "csv", "limit": 10 })).await;
    assert_eq!(rows["rows"].as_array().unwrap().len(), 2);
    assert!(rows["rows"].as_array().unwrap().iter().any(|r| r["values"].as_object().unwrap().values().any(|v| v == &json!("a, b"))), "quoted cells: {rows}");
    let rows = srv.rpc("doc.query", json!({ "source_id": "rows", "limit": 10 })).await;
    assert!(rows["rows"].as_array().unwrap().iter().any(|r| r["record_id"] == "x1"), "{rows}");
    assert!(srv.rpc("doc.read", json!({ "entity_id": "md" })).await["content"]["content"]["content"][0]["type"] == json!("heading"));
    let info = srv.rpc("ws.get_info", json!({})).await;
    assert!(info["traffic"].as_u64().unwrap() > 0, "{info}");
    srv.commit(json!([{ "op": "entity.set_keys", "entity_id": "csv", "keys": [{ "key": "source", "value": { "kind": "url", "url": format!("{http}/missing.csv") }, "expect": { "rev": srv.rpc("doc.read", json!({ "entity_id": "csv" })).await["content"]["key_revs"]["source"] } }] }])).await;
    let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": "u4", "params": { "entity_id": "csv" } })).await;
    let done = srv.wait(run["run_id"].as_str().unwrap()).await;
    assert_eq!((done["state"].as_str(), done["result"]["error"]["code"].as_str()), (Some("failed"), Some("DEPENDENCY_UNAVAILABLE")), "{done}");
    assert_eq!(srv.rpc("doc.query", json!({ "source_id": "csv", "limit": 10 })).await["rows"].as_array().unwrap().len(), 2, "the last valid version stays");
    // unpublished: the link answers nothing and a live refresh fails without touching the copy
    srv.ws = a.clone();
    srv.rpc("ws.unpublish", json!({ "entity_id": "orders" })).await;
    assert_eq!(srv.http.get(&pub_url).send().await.unwrap().status(), 404);
    srv.ws = b.clone();
    let run = srv.rpc("proc.start", json!({ "program": "source.refresh@1", "idempotency_key": "r6", "params": { "entity_id": "imported" } })).await;
    let done = srv.wait(run["run_id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(srv.rpc("doc.query", json!({ "source_id": "imported", "limit": 10 })).await["rows"].as_array().unwrap().len(), 4);
    });
}
