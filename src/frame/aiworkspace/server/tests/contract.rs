//! Contract tests of the stable kRPC methods (内核架构改进 D11, T0-3): every `stable` method of
//! `METHOD_LEVELS` is exercised once over HTTP and the *shape* of its request and response (keys and
//! value types, never the volatile values) is compared with `fixtures/protocol/stable-methods.json`.
//! `AIWS_REGEN=1` re-records the file. The README method table must list the same levels.

#[path = "common/nfs.rs"]
mod nfs;

use aiworkspace_server::auth::StaticTokens;
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{build_router, AppState, Limits, METHOD_LEVELS};
use aiworkspace_store::Service;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/protocol/stable-methods.json");
const README: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md");

/// Stable methods no in-process test can reach, with the reason (the README says the same).
const NOT_COVERED: &[(&str, &str)] = &[];

struct Srv {
    base: String,
    http: reqwest::Client,
    steps: Vec<Value>,
    nfs: nfs::Nfs,
}

/// The shape of a JSON value: scalars become their type name, arrays keep the shape of their first element.
fn shape(v: &Value) -> Value {
    match v {
        Value::Null => Value::Null,
        Value::Bool(_) => json!("boolean"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(a) => json!(a.first().map(shape).into_iter().collect::<Vec<_>>()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), shape(v))).collect::<Map<_, _>>()),
    }
}

impl Srv {
    async fn start(dir: &std::path::Path) -> Srv {
        let nfs = nfs::Nfs::start(dir).await;
        let svc = Service::open(&dir.join("data"), nfs.objects().await).unwrap();
        let tokens: StaticTokens = serde_json::from_value(json!({ "tokens": { "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" } } })).unwrap();
        let state = AppState::new(svc, Arc::new(tokens), Limits::default(), None, WishConfig::default());
        let _ = state.ndm_gateway.set(nfs.base.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await.unwrap() });
        Srv { base: format!("http://{addr}/kapi/aiworkspace"), http: reqwest::Client::new(), steps: Vec::new(), nfs }
    }

    /// Call `method` and record the shapes of its request and result.
    async fn call(&mut self, token: Option<&str>, method: &str, params: Value) -> Value {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let sys = match token {
            Some(t) => json!([seq, t]),
            None => json!([seq]),
        };
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": sys })).send().await.unwrap().json().await.unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        let result = v["result"].clone();
        assert!(result["ok"] == json!(true) || matches!(result["status"].as_str(), Some("accepted" | "ok")), "{method}: {result}");
        self.steps.push(json!({ "method": method, "params": shape(&params), "result": shape(&result) }));
        result
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.call(Some("tok-alice"), method, params).await
    }

}

fn commit(ws: &str, epoch: &str, key: &str, ops: Value) -> Value {
    json!({ "protocol_version": aiworkspace_core::model::PROTOCOL_VERSION, "workspace_id": ws, "epoch": epoch, "idempotency_key": key, "session_id": "s1", "operations": ops })
}

#[tokio::test(flavor = "multi_thread")]
async fn every_stable_method_keeps_its_recorded_shape() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Srv::start(dir.path()).await;
    let created = s.rpc("ws.create", json!({ "title": "契约" })).await;
    let (ws, epoch) = (created["workspace_id"].as_str().unwrap().to_string(), created["epoch"].as_str().unwrap().to_string());
    let wsp = |extra: Value| {
        let mut p = json!({ "workspace_id": ws });
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    s.rpc("ws.get_info", wsp(json!({}))).await;
    s.rpc("ws.list", json!({})).await;
    let ops = json!([
        { "op": "entity.create", "entity_id": "t1", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "表",
          "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }, { "field_id": "n", "name": "数", "type": "number" }] } },
        { "op": "table.insert_records", "source_id": "t1", "records": [{ "record_id": "r1", "values": { "title": "a", "n": 1 } }] }
    ]);
    s.rpc("doc.prepare", commit(&ws, &epoch, "c1", ops.clone())).await;
    let accepted = s.rpc("doc.commit", commit(&ws, &epoch, "c1", ops)).await;
    let head = accepted["seq"].as_u64().unwrap();
    s.rpc("doc.get_submission", wsp(json!({ "epoch": epoch, "idempotency_key": "c1" }))).await;
    s.rpc("doc.read", wsp(json!({ "entity_id": "t1" }))).await;
    s.rpc("doc.resolve", wsp(json!({ "reference": { "entity_id": "t1", "mode": "live" } }))).await;
    s.rpc("doc.query", wsp(json!({ "source_id": "t1", "limit": 10 }))).await;
    s.rpc("doc.get_changes", wsp(json!({ "epoch": epoch, "after_seq": 0 }))).await;
    s.rpc("doc.wait_changes", wsp(json!({ "epoch": epoch, "after_seq": head, "timeout_ms": 50 }))).await;
    s.rpc("ws.grant", wsp(json!({ "subject": "bob", "capabilities": ["read"] }))).await;
    s.rpc("ws.list_grants", wsp(json!({}))).await;
    s.rpc("ws.list_subjects", json!({})).await;
    s.rpc("ws.revoke", wsp(json!({ "subject": "bob" }))).await;
    s.rpc("ws.set_user_state", wsp(json!({ "entries": { "panel": { "open": true } } }))).await;
    s.rpc("ws.get_user_state", wsp(json!({}))).await;
    s.rpc("ws.set_wish_aicc", wsp(json!({ "mode": "workspace_owner" }))).await;
    let entry = s.rpc("ws.create_entry", wsp(json!({ "capabilities": ["read"], "methods": { "guest": true } }))).await;
    let entry_id = entry["entry"]["entry_id"].as_str().unwrap().to_string();
    s.call(None, "entry.describe", json!({ "entry_id": entry_id })).await;
    s.rpc("ws.list_entries", wsp(json!({}))).await;
    s.call(Some("tok-bob"), "ws.join_entry", json!({ "entry_id": entry_id })).await;
    s.rpc("ws.close_entry", json!({ "entry_id": entry_id })).await;
    let lock = s.rpc("lock.acquire", wsp(json!({ "entity_ids": ["t1"], "session_id": "s1" }))).await;
    let lock_ids: Vec<Value> = lock["locks"].as_array().map(|a| a.iter().map(|l| l["lock_id"].clone()).collect()).unwrap_or_default();
    s.rpc("lock.list", wsp(json!({}))).await;
    s.rpc("lock.renew", wsp(json!({ "lock_ids": lock_ids }))).await;
    s.rpc("lock.break", wsp(json!({ "entity_id": "t1" }))).await;
    let lock = s.rpc("lock.acquire", wsp(json!({ "entity_ids": ["t1"], "session_id": "s1" }))).await;
    let lock_ids: Vec<Value> = lock["locks"].as_array().map(|a| a.iter().map(|l| l["lock_id"].clone()).collect()).unwrap_or_default();
    s.rpc("lock.release", wsp(json!({ "lock_ids": lock_ids }))).await;
    let uploaded = s.nfs.put("alice", "assets", "upload/hello.txt", b"hello contract".to_vec()).await;
    s.rpc("asset.import_object", wsp(json!({ "bucket_id": uploaded["bucket_id"], "key": uploaded["key"] }))).await;
    let export = s.http.post(&s.base).json(&json!({ "method": "doc.export", "params": wsp(json!({ "mode": "share", "self_contained": true })), "sys": [9000, "tok-alice"] })).send().await.unwrap().json::<Value>().await.unwrap();
    let pkg = s.nfs.fetch(&export["result"]["download"]).await;
    let package = s.nfs.put("alice", "exports", "import/contract.bcanvas", pkg).await;
    let imported = s.rpc("ws.import", json!({ "bucket_id": package["bucket_id"], "key": package["key"], "semantics": "new" })).await;
    s.rpc("doc.undo", wsp(json!({ "epoch": epoch, "commit_id": accepted["commit_id"], "idempotency_key": "u1" }))).await;
    // the maintenance channel (D19)
    let cp = s.http.post(&s.base).json(&json!({ "method": "doc.checkpoint", "params": wsp(json!({})), "sys": [9001, "tok-alice"] })).send().await.unwrap().json::<Value>().await.unwrap();
    s.rpc("ws.release_retention", wsp(json!({ "kind": "export", "ref": export["result"]["export_id"] }))).await;
    s.rpc("ws.delete_checkpoint", wsp(json!({ "snapshot_id": cp["result"]["snapshot"] }))).await;
    s.rpc("ws.purge_deleted", wsp(json!({}))).await;
    s.rpc("ws.clear_staged", wsp(json!({}))).await;
    let forked = s.rpc("ws.fork", wsp(json!({}))).await;
    s.rpc("ws.delete", json!({ "workspace_id": forked["workspace_id"] })).await;
    s.rpc("ws.delete", json!({ "workspace_id": imported["workspace_id"] })).await;

    // every stable method was exercised (or is listed as not coverable here)
    let stable: Vec<&str> = METHOD_LEVELS.iter().filter(|(_, l)| *l == "stable").map(|(m, _)| *m).collect();
    let seen: std::collections::BTreeSet<&str> = s.steps.iter().filter_map(|st| st["method"].as_str()).collect();
    for m in &stable {
        assert!(seen.contains(m) || NOT_COVERED.iter().any(|(n, _)| n == m), "stable method {m} has no contract step");
    }
    for st in &s.steps {
        assert!(stable.contains(&st["method"].as_str().unwrap()), "{} is recorded but not stable", st["method"]);
    }
    let recorded = json!({ "description": "Request / response shapes of the stable methods, in the order server/tests/contract.rs calls them. Values are types, not data. Regenerate with AIWS_REGEN=1.", "not_covered": NOT_COVERED.iter().map(|(m, why)| json!({ "method": m, "reason": why })).collect::<Vec<_>>(), "steps": s.steps });
    if std::env::var("AIWS_REGEN").is_ok() {
        std::fs::write(FIXTURE, serde_json::to_string_pretty(&recorded).unwrap() + "\n").unwrap();
        return;
    }
    let file: Value = serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("fixtures/protocol/stable-methods.json (AIWS_REGEN=1 records it)")).unwrap();
    let want = file["steps"].as_array().unwrap();
    assert_eq!(want.len(), s.steps.len(), "the number of contract steps changed; re-record with AIWS_REGEN=1 if that is intended");
    for (i, (w, got)) in want.iter().zip(s.steps.iter()).enumerate() {
        assert_eq!(w, got, "step {i} ({}) changed shape; re-record with AIWS_REGEN=1 if the protocol changed on purpose", got["method"]);
    }
}

#[test]
fn readme_method_table_mirrors_the_levels() {
    let readme = std::fs::read_to_string(README).unwrap();
    let mut listed = std::collections::BTreeMap::new();
    let mut in_table = false;
    for line in readme.lines() {
        if line.starts_with("| Method | Level |") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if !line.starts_with("| `") {
            if !line.starts_with("| ---") {
                break;
            }
            continue;
        }
        let cells: Vec<&str> = line.split(" | ").collect();
        let level = cells[1].trim();
        let mut rest = cells[0];
        while let Some(start) = rest.find('`') {
            let after = &rest[start + 1..];
            let end = after.find('`').unwrap();
            let name = &after[..end];
            if name.contains('.') && !name.contains(' ') && !name.contains('<') {
                listed.insert(name.to_string(), level.to_string());
            }
            rest = &after[end + 1..];
        }
    }
    let code: std::collections::BTreeMap<String, String> = METHOD_LEVELS.iter().map(|(m, l)| (m.to_string(), l.to_string())).collect();
    assert_eq!(listed, code, "README method table and METHOD_LEVELS differ");
}
