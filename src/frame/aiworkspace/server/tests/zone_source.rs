//! Zone sources and data source state (AI Native 余下任务 §7.2: B0, B3-1, B4-1, B5, B7): a table defined before its
//! data is pending (no SQL, a definition); a sync program's `aiws.sync.v1` output in the owner's home becomes its data,
//! cut to `def.limit`, identified by the primary key, with a data version and the last attempt; a failed or malformed
//! output keeps the last valid version and says why; only the current owner's home is read; triggering a refresh is
//! `run`, changing a Source the owner's; a wish codes on the pending source, runs once it synced and reads stale after
//! the next sync.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::{json, Value};
use wishkit::*;

fn tokens() -> Value {
    json!({ "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" }, "tok-carol": { "principal": "carol" } })
}

async fn start(root: &std::path::Path) -> Srv {
    let limits = aiworkspace_server::Limits { zone_root: Some(root.to_path_buf()), ..Default::default() };
    Srv::start_with_limits(tempfile::tempdir().unwrap(), model(), tokens(), limits).await
}

fn quotes_source(path: &str) -> Value {
    json!({ "kind": "zone", "path": path, "refresh": { "interval_s": 900 },
            "def": { "meaning": "Daily closing prices of the watched stocks", "origin": "the exchange's daily file", "primary_key": ["symbol"], "freshness": { "max_age_s": 86400 }, "limit": 2 } })
}

fn fields() -> Value {
    json!([{ "field_id": "symbol", "name": "Symbol", "type": "text" }, { "field_id": "price", "name": "Price", "type": "number" }])
}

fn envelope(status: &str, records: Value) -> Value {
    json!({ "format": "aiws.sync.v1", "status": status, "error": if status == "failed" { json!("the exchange answered 503") } else { Value::Null },
            "program": { "name": "quotes", "version": "3" }, "business_time": "2026-01-15T08:00:00Z", "produced_at": "2026-01-15T08:01:12Z",
            "records": records, "truncated": false })
}

fn write(path: &std::path::Path, v: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(v).unwrap()).unwrap();
}

async fn refresh_as(srv: &Srv, token: &str, entity: &str) -> Value {
    static K: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let key = format!("z{}", K.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let run = srv.call_as(token, "proc.start", json!({ "program": "source.refresh@1", "idempotency_key": key, "params": { "entity_id": entity } })).await;
    match run["result"]["run_id"].as_str() {
        Some(id) => srv.wait_as(token, id).await,
        None => run,
    }
}

async fn meta(srv: &Srv, id: &str) -> Value {
    srv.rpc("doc.read", json!({ "entity_id": id, "as": "meta" })).await
}

#[test]
fn a_zone_source_reads_the_owners_home() {
    run_test(|| async move {
    let zone = tempfile::tempdir().unwrap();
    let file = zone.path().join("home/alice/sync/quotes.json");
    let mut srv = start(zone.path()).await;
    srv.ws = srv.rpc("ws.create", json!({ "title": "行情" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.commit(json!([{ "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "quotes",
                        "schema_id": "acme.quotes@1", "payload": { "fields": fields(), "source": quotes_source("/home/alice/sync/quotes.json") } }])).await;
    // defined, not synced: no rows to query, a definition to read
    let m = meta(&srv, "quotes").await;
    assert_eq!((m["data"]["state"].as_str(), m["body"]["state"].as_str()), (Some("pending"), Some("pending")), "{m}");
    assert!(m["data"]["attempt"].is_null() && m["data"]["expired"] == json!(false), "{m}");
    let q = srv.rpc_raw("doc.query", json!({ "sql": "SELECT * FROM quotes" })).await;
    assert_eq!(q["error"]["data"]["sub_code"].as_str().or(q["error"]["sub_code"].as_str()).or(q["result"]["error"]["sub_code"].as_str()).unwrap_or(&q.to_string()).contains("DATA_PENDING"), true, "{q}");
    // a missing file fails the refresh; the source stays pending and says why
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!(done["state"], "failed", "{done}");
    let m = meta(&srv, "quotes").await;
    assert_eq!((m["data"]["state"].as_str(), m["data"]["attempt"]["status"].as_str()), (Some("pending"), Some("failed")), "{m}");
    // the program's output: cut to the limit, identified by the key, with its version
    write(&file, &envelope("ok", json!([{ "symbol": "AAPL", "price": 190.5 }, { "symbol": "MSFT", "price": 410 }, { "symbol": "NVDA", "price": 140 }])));
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!((done["state"].as_str(), done["result"]["status"].as_str()), (Some("succeeded"), Some("accepted")), "{done}");
    let rows = srv.records("quotes").await;
    assert_eq!(rows.len(), 2, "limit 2: {rows:?}");
    let aapl = aiworkspace_core::wish::record_id_for_key(&[json!("AAPL")]);
    assert!(rows.iter().any(|r| r["record_id"] == json!(aapl) && r["values"]["price"] == json!(190.5)), "{rows:?}");
    let m = meta(&srv, "quotes").await;
    let v = &m["data"]["version"];
    assert_eq!((m["data"]["state"].as_str(), v["business_time"].as_str(), v["program"]["version"].as_str(), v["records"].as_u64(), v["truncated"].as_bool()),
               (Some("valid"), Some("2026-01-15T08:00:00Z"), Some("3"), Some(2), Some(true)), "{m}");
    assert!(v["synced_at"].is_string() && m["data"]["attempt"]["status"] == "succeeded", "{m}");
    assert_eq!(m["data"]["expired"], true, "the business time is long past a day: {m}");
    assert_eq!(m["data"]["def"]["primary_key"], json!(["symbol"]));
    // the same output again is no new version, nor is the same data the program produced again later
    let head = srv.rpc("ws.get_info", json!({})).await["head_seq"].clone();
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!(done["result"]["status"], "unchanged", "{done}");
    let mut later = envelope("ok", json!([{ "symbol": "AAPL", "price": 190.5 }, { "symbol": "MSFT", "price": 410 }, { "symbol": "NVDA", "price": 140 }]));
    later["produced_at"] = json!("2026-01-15T09:30:00Z");
    write(&file, &later);
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!(done["result"]["status"], "unchanged", "{done}");
    assert_eq!(srv.rpc("ws.get_info", json!({})).await["head_seq"], head);
    // a failed run of the program, a duplicate key, a record without its key: the last valid version stays
    write(&file, &envelope("failed", json!([])));
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!(done["state"], "failed");
    assert!(done["result"]["error"]["detail"].as_str().unwrap_or("").contains("the exchange answered 503"), "{done}");
    let m = meta(&srv, "quotes").await;
    assert!(m["data"]["attempt"]["error"]["detail"].as_str().unwrap_or("").contains("503") && m["data"]["state"] == "valid", "{m}");
    for (records, code) in [(json!([{ "symbol": "A", "price": 1 }, { "symbol": "A", "price": 2 }]), "DUPLICATE_KEY"), (json!([{ "price": 2 }]), "PRIMARY_KEY_REQUIRED")] {
        write(&file, &envelope("ok", records));
        let done = refresh_as(&srv, "tok-alice", "quotes").await;
        assert_eq!((done["state"].as_str(), done["result"]["error"]["sub_code"].as_str()), (Some("failed"), Some(code)), "{done}");
    }
    assert_eq!(srv.records("quotes").await.len(), 2);
    // a plain file is read like a URL's body; only the current owner's home is read, never through a link out of it
    std::fs::write(zone.path().join("home/alice/sync/plain.csv"), "symbol,price\nAAPL,191\n").unwrap();
    std::fs::create_dir_all(zone.path().join("home/bob")).unwrap();
    std::fs::write(zone.path().join("home/bob/secret.csv"), "symbol,price\nX,1\n").unwrap();
    std::os::unix::fs::symlink(zone.path().join("home/bob/secret.csv"), zone.path().join("home/alice/sync/link.csv")).unwrap();
    let mut src = quotes_source("/home/alice/sync/plain.csv");
    srv.commit(json!([{ "op": "entity.create", "entity_id": "plain", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "plain", "payload": { "source": src.clone() } }])).await;
    assert_eq!(refresh_as(&srv, "tok-alice", "plain").await["state"], "succeeded");
    assert_eq!(srv.records("plain").await[0]["values"]["price"].as_f64(), Some(191.0));
    for (id, key, path, why) in [("theirs", "c", "/home/bob/secret.csv", "not in the home"), ("link", "d", "/home/alice/sync/link.csv", "not a file of the owner's home")] {
        src["path"] = json!(path);
        srv.commit(json!([{ "op": "entity.create", "entity_id": id, "type_id": "buckyos.table-source", "parent_id": "data", "order_key": key, "name": id, "payload": { "source": src.clone() } }])).await;
        let done = refresh_as(&srv, "tok-alice", id).await;
        assert!(done["state"] == "failed" && done.to_string().contains(why), "{id}: {done}");
    }

    // B4-1: bob may trigger a refresh with `run` and see it; changing the Source is the owner's; carol reads nothing
    srv.rpc("acl.set", json!({ "changes": [{ "subject": "bob", "resource": "/", "allow": ["read", "run", "update", "structure", "export"] }] })).await;
    let done = refresh_as(&srv, "tok-bob", "plain").await;
    assert_eq!(done["state"], "succeeded", "{done}");
    let epoch = srv.rpc("ws.get_info", json!({})).await["epoch"].clone();
    let rev = srv.rpc("doc.read", json!({ "entity_id": "plain" })).await["content"]["key_revs"]["source"].clone();
    let refused = srv.call_as("tok-bob", "doc.commit", json!({ "protocol_version": "1.0", "epoch": epoch, "idempotency_key": "bob-src", "session_id": "s2",
        "operations": [{ "op": "entity.set_keys", "entity_id": "plain", "keys": [{ "key": "source", "value": quotes_source("/home/alice/sync/quotes.json"), "expect": { "rev": rev } }] }] })).await;
    assert_eq!(refused["result"]["errors"][0]["code"], "PERMISSION_DENIED", "{refused}");
    let seen = srv.call_as("tok-carol", "proc.get", json!({ "run_id": done["run_id"] })).await;
    assert!(seen["error"].is_object() || seen["result"]["error"].is_object(), "{seen}");

    // B7-4: a clone made by bob keeps the data and the definition, and reads bob's home from now on
    let cloned = srv.call_as("tok-bob", "ws.clone", json!({})).await["result"]["workspace_id"].as_str().unwrap().to_string();
    let theirs = Srv { ws: cloned, ..srv };
    let m = theirs.call_as("tok-bob", "doc.read", json!({ "entity_id": "quotes", "as": "meta" })).await["result"].clone();
    assert_eq!((m["data"]["state"].as_str(), m["data"]["version"]["program"]["version"].as_str()), (Some("valid"), Some("3")), "{m}");
    assert!(m["data"]["attempt"].is_null(), "attempts are this workspace's own: {m}");
    let done = refresh_as(&theirs, "tok-bob", "quotes").await;
    assert!(done["state"] == "failed" && done.to_string().contains("not in the home of this workspace's owner (bob)"), "{done}");
    });
}

/// B5-1: a URL table is identified by `def.primary_key` or an `id` column, never by the row position.
#[test]
fn url_rows_need_a_primary_key() {
    run_test(|| async move {
    let app = axum::Router::new()
        .route("/a.csv", axum::routing::get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/csv")], "name,n\nx,1\ny,2\n") }))
        .route("/b.json", axum::routing::get(|| async { axum::Json(json!([{ "id": "p1", "n": 1 }, { "id": "p2", "n": 2 }])) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let limits = aiworkspace_server::Limits { fetch_private: true, ..Default::default() };
    let mut srv = Srv::start_with_limits(tempfile::tempdir().unwrap(), model(), tokens(), limits).await;
    srv.ws = srv.rpc("ws.create", json!({ "title": "u" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "nokey", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "nokey", "payload": { "source": { "kind": "url", "url": format!("{http}/a.csv") } } },
        { "op": "entity.create", "entity_id": "keyed", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "keyed", "payload": { "source": { "kind": "url", "url": format!("{http}/a.csv"), "def": { "meaning": "names", "primary_key": ["name"] } } } },
        { "op": "entity.create", "entity_id": "ids", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "c", "name": "ids", "payload": { "source": { "kind": "url", "url": format!("{http}/b.json") } } },
    ])).await;
    let done = refresh_as(&srv, "tok-alice", "nokey").await;
    assert_eq!(done["result"]["error"]["sub_code"], "PRIMARY_KEY_REQUIRED", "{done}");
    assert_eq!(refresh_as(&srv, "tok-alice", "keyed").await["state"], "succeeded");
    assert!(srv.records("keyed").await.iter().any(|r| r["record_id"] == json!(aiworkspace_core::wish::record_id_for_key(&[json!("x")]))));
    assert_eq!(refresh_as(&srv, "tok-alice", "ids").await["state"], "succeeded");
    assert!(srv.records("ids").await.iter().any(|r| r["record_id"] == json!(aiworkspace_core::wish::record_id_for_key(&[json!("p1")]))));
    });
}

const PROGRAM: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { quotes: ['Symbol', 'Price'] } })
  const { rows } = await aiws.sql(`SELECT "Symbol" AS symbol, "Price" AS price FROM quotes ORDER BY "Price" DESC`)
  aiws.result.table('movers', rows, { key: ['symbol'], fields: { symbol: 'text', price: 'number' } })
}
"#;

fn analysis(t: &Turn) -> Reply {
    match t.results.len() {
        0 => {
            let q = handle(&t.user, "\"quotes\"");
            assert!(t.user.contains("data_source.state: pending"), "the map says the source has not synced:\n{}", t.user);
            call("submit_analysis", json!({ "analysis": {
                "schema_version": "wish.analysis.v3", "status": "ready", "context_prompt": format!("Sort {q} by price, highest first."),
                "inputs": [{ "name": "quotes", "ref": q }],
                "output_contract": { "results": [{ "name": "movers", "type": "table", "title": "Movers", "key": ["symbol"], "schema_id": "acme.quotes@1" }] },
                "checks": [], "blockers": [], "warnings": [] } }))
        }
        _ => text("done"),
    }
}

/// B2-2, B7-1, B0-4: the wish binds and codes on a pending source, cannot run on it, runs after its first sync with a
/// result of the source's schema, and its result reads stale once the source synced again. B2-1: an analysis that needs
/// outside data fails with NEEDS_EXTERNAL_DATA.
#[test]
fn a_wish_on_a_pending_source() {
    run_test(|| async move {
    let zone = tempfile::tempdir().unwrap();
    let file = zone.path().join("home/alice/sync/quotes.json");
    let mut srv = start(zone.path()).await;
    *srv.model.analyze.lock().unwrap() = Box::new(analysis);
    *srv.model.execute.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => {
            assert!(t.user.contains("a data source with no data yet"), "the brief says the input has not synced:\n{}", t.user);
            call("run_program", json!({ "source": PROGRAM }))
        }
        1 => call("finish", json!({ "summary": "sorted" })),
        _ => text("done"),
    });
    srv.ws = srv.rpc("ws.create", json!({ "title": "行情" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "quotes",
          "schema_id": "acme.quotes@1", "payload": { "fields": fields(), "source": quotes_source("/home/alice/sync/quotes.json") } },
        { "op": "entity.create", "entity_id": "w", "type_id": "buckyos.wish", "parent_id": "data", "order_key": "w", "name": "movers",
          "payload": { "title": "movers", "prompt": "sort the quotes by price", "executor": "xllm", "output_mode": "overwrite" } },
    ])).await;
    let st = |stage: &str| json!({ "wish_id": "w", "stage": stage, "location": {}, "request": { "today": "2026-11-15" } });
    srv.done(&srv.run("wish.xllm@1", st("analyze")).await);
    srv.done(&srv.run("wish.xllm@1", st("code")).await);
    let main = srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0].clone();
    assert!(main["input_problems"].as_array().unwrap().iter().any(|p| p["reason"] == "pending" && p["input"] == "quotes"), "{main}");
    let refused = srv.run("wish.xllm@1", st("run")).await;
    assert_eq!((refused["state"].as_str(), refused["error"]["sub_code"].as_str()), (Some("refused"), Some("NEEDS_INPUT")), "{refused}");
    // the first sync: the wish runs, its result follows the source's schema
    write(&file, &envelope("ok", json!([{ "symbol": "AAPL", "price": 190 }, { "symbol": "MSFT", "price": 410 }])));
    assert_eq!(refresh_as(&srv, "tok-alice", "quotes").await["state"], "succeeded");
    let run = srv.run("wish.xllm@1", st("run")).await;
    srv.done(&run);
    let result = srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0].clone();
    assert_eq!(result["status"], "current", "{result}");
    let movers = result["results"][0]["entity_id"].as_str().unwrap().to_string();
    assert_eq!(srv.rpc("doc.read", json!({ "entity_id": movers, "as": "meta" })).await["schema_id"], "acme.quotes@1");
    // the next sync changes the data: the result reads stale, nothing runs by itself (B7-1)
    write(&file, &json!({ "format": "aiws.sync.v1", "status": "ok", "program": { "name": "quotes", "version": "3" }, "business_time": "2026-11-16T08:00:00Z",
                          "records": [{ "symbol": "AAPL", "price": 200 }, { "symbol": "MSFT", "price": 410 }] }));
    assert_eq!(refresh_as(&srv, "tok-alice", "quotes").await["state"], "succeeded");
    let after = srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0].clone();
    assert_eq!(after["status"], "stale", "{after}");

    // an analysis that needs data from outside fails and says so
    *srv.model.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("submit_analysis", json!({ "analysis": {
            "schema_version": "wish.analysis.v3", "status": "needs_input", "context_prompt": "-", "inputs": [], "output_contract": { "results": [] }, "checks": [],
            "blockers": [{ "code": "external_data", "message": "no weather data in this workspace", "wanted": { "meaning": "daily weather of Paris", "origin": "a weather service", "freshness": "daily", "fields": [{ "name": "date", "type": "date" }] } }],
            "warnings": [] } })),
        _ => text("done"),
    });
    srv.commit(json!([{ "op": "entity.create", "entity_id": "w2", "type_id": "buckyos.wish", "parent_id": "data", "order_key": "x", "name": "weather",
                        "payload": { "title": "weather", "prompt": "how warm was Paris this week", "executor": "xllm" } }])).await;
    let failed = srv.run("wish.xllm@1", json!({ "wish_id": "w2", "stage": "analyze", "location": {} })).await;
    assert_eq!((failed["state"].as_str(), failed["error"]["sub_code"].as_str()), (Some("failed"), Some("NEEDS_EXTERNAL_DATA")), "{failed}");
    assert_eq!(failed["error"]["data"]["blockers"][0]["wanted"]["meaning"], "daily weather of Paris", "{failed}");
    });
}
