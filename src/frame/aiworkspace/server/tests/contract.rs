//! Contract tests of the stable kRPC methods (内核架构改进 D11, T0-3): every `stable` method of
//! `METHODS` is exercised once over HTTP and the *shape* of its request and response (keys and
//! value types, never the volatile values) is compared with `fixtures/protocol/stable-methods.json`.
//! `AIWS_REGEN=1` re-records the file. The README method table must list the same levels.

#[path = "common/nfs.rs"]
mod nfs;

use aiworkspace_server::auth::StaticTokens;
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{build_router, AppState, Limits, METHODS};
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
    // a Commit sent in parts: the staged part comes first, the final request names its digest
    let rec = |id: &str, key: &str| json!({ "op": "table.insert_records", "source_id": "t1", "records": [{ "record_id": id, "values": { "title": key, "n": 2 } }] });
    let part = s.rpc("doc.stage", wsp(json!({ "epoch": epoch, "idempotency_key": "c-parts", "part": 0, "operations": [rec("p0", "a")] }))).await;
    let mut parted = commit(&ws, &epoch, "c-parts", json!([rec("p1", "b")]));
    parted["staged"] = json!([part["digest"]]);
    let r = s.rpc("doc.commit", parted).await;
    assert_eq!(r["status"], "accepted", "{r}");
    // one request stays under the kRPC body limit, a staged part included
    let over = "x".repeat(Limits::default().max_commit_bytes + 128 * 1024);
    let status = s
        .http
        .post(&s.base)
        .json(&json!({ "method": "doc.stage", "params": wsp(json!({ "epoch": epoch, "idempotency_key": "c-over", "part": 0, "operations": [{ "op": "noop", "pad": over }] })), "sys": [9002, "tok-alice"] }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    s.rpc("doc.read", wsp(json!({ "entity_id": "t1" }))).await;
    s.rpc("doc.read", wsp(json!({ "path": "/data/表", "as": "meta" }))).await;
    s.rpc("doc.read", wsp(json!({ "entity_id": "t1", "as": "body", "parts": true }))).await;
    s.rpc("doc.resolve", wsp(json!({ "reference": { "entity_id": "t1", "mode": "live" } }))).await;
    s.rpc("doc.resolve", wsp(json!({ "path": "/data/表" }))).await;
    s.rpc("doc.list", wsp(json!({ "at": "/data" }))).await;
    s.rpc("doc.find", wsp(json!({ "name": "表" }))).await;
    s.rpc("doc.query", wsp(json!({ "text": "a" }))).await;
    s.rpc("doc.query", wsp(json!({ "sql": "SELECT \"标题\", \"数\" FROM \"/data/表\" ORDER BY _id LIMIT 10" }))).await;
    s.rpc("doc.get_changes", wsp(json!({ "epoch": epoch, "after_seq": 0 }))).await;
    s.rpc("doc.wait_changes", wsp(json!({ "epoch": epoch, "after_seq": head, "timeout_ms": 50 }))).await;
    // permissions (权限需求 §4): entries by path, groups and members, anonymous access, queries, the record
    s.rpc("acl.set", wsp(json!({ "changes": [{ "subject": "bob", "resource": "/data/表", "allow": ["read"], "deny": ["comment"] }] }))).await;
    s.rpc("acl.set_group", wsp(json!({ "group_id": "teachers", "title": "老师" }))).await;
    s.rpc("acl.members", wsp(json!({ "group_id": "teachers", "add": ["bob", "group:staff"] }))).await;
    s.rpc("acl.set", wsp(json!({ "changes": [{ "subject": "@teachers", "resource": "/", "allow": ["read", "update"] }] }))).await;
    s.rpc("acl.groups", wsp(json!({}))).await;
    s.rpc("acl.query", wsp(json!({ "by": "entity", "resource": "/data/表", "expand": true }))).await;
    s.rpc("acl.query", wsp(json!({ "by": "subject", "subject": "@teachers" }))).await;
    s.rpc("acl.query", wsp(json!({ "by": "explain", "resource": "/data/表", "principal": "bob" }))).await;
    s.rpc("acl.query", wsp(json!({ "by": "all" }))).await;
    s.rpc("acl.set_anonymous", wsp(json!({ "enabled": false }))).await;
    s.rpc("ws.list_subjects", json!({})).await;
    s.rpc("acl.set", wsp(json!({ "changes": [{ "subject": "bob", "resource": "/data/表" }] }))).await;
    s.rpc("ws.set_user_state", wsp(json!({ "entries": { "panel": { "open": true } } }))).await;
    s.rpc("ws.get_user_state", wsp(json!({}))).await;
    s.rpc("acl.set_group", wsp(json!({ "group_id": "visitors" }))).await;
    s.rpc("acl.set", wsp(json!({ "changes": [{ "subject": "@visitors", "allow": ["read"] }] }))).await;
    let entry = s.rpc("ws.create_entry", wsp(json!({ "group_id": "visitors", "methods": { "guest": true }, "max_joins": 10 }))).await;
    let entry_id = entry["entry"]["entry_id"].as_str().unwrap().to_string();
    s.call(None, "entry.describe", json!({ "entry_id": entry_id })).await;
    s.rpc("ws.list_entries", wsp(json!({}))).await;
    s.call(Some("tok-bob"), "ws.join_entry", json!({ "entry_id": entry_id })).await;
    s.rpc("ws.close_entry", json!({ "entry_id": entry_id, "remove_members": true })).await;
    s.rpc("acl.delete_group", wsp(json!({ "group_id": "visitors" }))).await;
    let lock = s.rpc("lock.acquire", wsp(json!({ "entity_ids": ["t1"], "session_id": "s1" }))).await;
    let lock_ids: Vec<Value> = lock["locks"].as_array().map(|a| a.iter().map(|l| l["lock_id"].clone()).collect()).unwrap_or_default();
    s.rpc("lock.list", wsp(json!({}))).await;
    s.rpc("lock.renew", wsp(json!({ "lock_ids": lock_ids }))).await;
    s.rpc("lock.break", wsp(json!({ "entity_id": "t1" }))).await;
    let lock = s.rpc("lock.acquire", wsp(json!({ "entity_ids": ["t1"], "session_id": "s1" }))).await;
    let lock_ids: Vec<Value> = lock["locks"].as_array().map(|a| a.iter().map(|l| l["lock_id"].clone()).collect()).unwrap_or_default();
    s.rpc("lock.release", wsp(json!({ "lock_ids": lock_ids }))).await;
    let uploaded = s.nfs.put("alice", "assets", "upload/hello.txt", b"hello contract".to_vec()).await;
    let staged = s.rpc("asset.import_object", wsp(json!({ "bucket_id": uploaded["bucket_id"], "key": uploaded["key"] }))).await;
    // the uploader plays back what it staged (控件机制 TODO G17)
    s.rpc("asset.stream_url", wsp(json!({ "object_id": staged["object_id"] }))).await;
    let export = s.http.post(&s.base).json(&json!({ "method": "doc.export", "params": wsp(json!({ "mode": "share", "self_contained": true })), "sys": [9000, "tok-alice"] })).send().await.unwrap().json::<Value>().await.unwrap();
    let pkg = s.nfs.fetch(&export["result"]["download"]).await;
    let package = s.nfs.put("alice", "exports", "import/contract.bcanvas", pkg).await;
    let imported = s.rpc("ws.import", json!({ "bucket_id": package["bucket_id"], "key": package["key"], "semantics": "new" })).await;
    s.rpc("doc.undo", wsp(json!({ "epoch": epoch, "commit_id": accepted["commit_id"], "idempotency_key": "u1" }))).await;
    // the maintenance channel (D19)
    let cp = s.http.post(&s.base).json(&json!({ "method": "doc.checkpoint", "params": wsp(json!({})), "sys": [9001, "tok-alice"] })).send().await.unwrap().json::<Value>().await.unwrap();
    // the versions a checkpoint made addressable (UI 框架 PRD K9: stable)
    let versions = s.rpc("doc.list_versions", wsp(json!({ "entity_id": "t1" }))).await;
    assert!(!versions["versions"].as_array().unwrap().is_empty(), "{versions}");
    s.rpc("ws.release_retention", wsp(json!({ "kind": "export", "ref": export["result"]["export_id"] }))).await;
    s.rpc("ws.delete_checkpoint", wsp(json!({ "snapshot_id": cp["result"]["snapshot"] }))).await;
    s.rpc("ws.purge_deleted", wsp(json!({}))).await;
    s.rpc("ws.clear_staged", wsp(json!({}))).await;
    // wish runs (浏览器优先, AI Native 第一期): a local run, worked through the executor protocol
    let info = s.rpc("ws.get_info", wsp(json!({}))).await;
    s.rpc("doc.commit", commit(&ws, info["epoch"].as_str().unwrap(), "wish-1", json!([{ "op": "entity.create", "entity_id": "w1", "type_id": "buckyos.wish", "parent_id": "data", "order_key": "w",
        "payload": { "prompt": "写一句欢迎词", "executor": "xllm", "output_mode": "overwrite" } }]))).await;
    // a macro bound to a group (§4.5), and the operation record of everything above
    s.rpc("acl.bind_macro", wsp(json!({ "wish": "w1", "group_id": "teachers", "mode": "intersection" }))).await;
    s.rpc("acl.bind_macro", wsp(json!({ "wish": "w1", "group_id": null }))).await;
    s.rpc("acl.audit", wsp(json!({ "limit": 5 }))).await;
    let start = s.rpc("proc.start", wsp(json!({ "program": "wish.xllm@1", "idempotency_key": "w-a", "params": { "wish_id": "w1", "stage": "analyze", "execution": { "mode": "local" } } }))).await;
    let run_id = start["run_id"].as_str().unwrap().to_string();
    s.rpc("proc.list", wsp(json!({ "wish_id": "w1" }))).await;
    let claimed = s.rpc("proc.claim", wsp(json!({ "run_id": run_id, "claim_token": start["claim_token"] }))).await;
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    let at = |extra: Value| { let mut p = wsp(json!({ "run_id": run_id, "attempt": 1 })); for (k, v) in extra.as_object().unwrap() { p[k] = v.clone(); } p };
    s.call(Some(&rt), "proc.heartbeat", at(json!({ "progress": { "phase": "reading" } }))).await;
    s.call(Some(&rt), "proc.files", at(json!({}))).await;
    s.call(Some(&rt), "proc.tool", at(json!({ "name": "ws_find", "args": { "text": "表" } }))).await;
    s.call(Some(&rt), "proc.program", at(json!({ "source": "export default async function main() {}", "results": null, "log": "", "files": {} }))).await;
    s.call(Some(&rt), "proc.tool", at(json!({ "name": "submit_analysis", "args": { "analysis": { "schema_version": "wish.analysis.v3", "status": "ready", "context_prompt": "写一句欢迎词。", "inputs": [],
        "output_contract": { "results": [{ "name": "hello", "type": "richtext", "title": "欢迎" }] }, "checks": [], "blockers": [], "warnings": [] } } }))).await;
    s.call(Some(&rt), "proc.report", at(json!({ "finished": true }))).await;
    let mut waiting = s.rpc("proc.get", wsp(json!({ "run_id": run_id }))).await;
    for _ in 0..200 {
        if waiting["state"] == json!("waiting_confirmation") { break; }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        waiting = s.rpc("proc.get", wsp(json!({ "run_id": run_id, "choices": {} }))).await;
    }
    s.rpc("proc.apply", wsp(json!({ "run_id": run_id, "plan_digest": waiting["preview"]["plan_digest"], "session_id": "s1" }))).await;
    let other = s.rpc("proc.start", wsp(json!({ "program": "wish.xllm@1", "idempotency_key": "w-c", "params": { "wish_id": "w1", "stage": "analyze", "execution": { "mode": "local" } } }))).await;
    s.rpc("proc.cancel", wsp(json!({ "run_id": other["run_id"] }))).await;
    // named Workspaces (控件机制 TODO G9) and the service clock (G16)
    let named = s.rpc("ws.ensure", json!({ "name": "contract", "title": "契约" })).await;
    let again = s.rpc("ws.ensure", json!({ "name": "contract" })).await;
    assert_eq!((&named["created"], &again["created"], &again["workspace_id"]), (&json!(true), &json!(false), &named["workspace_id"]));
    s.rpc("ws.create", json!({ "title": "命名", "name": "contract-2" })).await;
    s.rpc("ws.time", json!({})).await;
    let cloned = s.rpc("ws.clone", wsp(json!({}))).await;
    s.rpc("ws.delete", json!({ "workspace_id": cloned["workspace_id"] })).await;
    s.rpc("ws.delete", json!({ "workspace_id": imported["workspace_id"] })).await;

    // every stable method was exercised (or is listed as not coverable here)
    let stable: Vec<&str> = METHODS.iter().filter(|m| m.level == "stable").map(|m| m.method).collect();
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
    let code: std::collections::BTreeMap<String, String> = METHODS.iter().map(|m| (m.method.to_string(), m.level.to_string())).collect();
    assert_eq!(listed, code, "README method table and METHODS differ");
}

/// The README's authorization table is generated from `METHODS` (权限需求 R21); `AIWS_REGEN=1` writes it.
#[test]
fn readme_authorization_table_is_generated() {
    const BEGIN: &str = "<!-- authorization-table:begin (generated from server/src/methods.rs; AIWS_REGEN=1) -->\n";
    const END: &str = "<!-- authorization-table:end -->";
    let readme = std::fs::read_to_string(README).unwrap();
    let (head, rest) = readme.split_once(BEGIN).expect("the README has the authorization table markers");
    let (_, tail) = rest.split_once(END).expect("the README has the authorization table end marker");
    let table = aiworkspace_server::methods::authorization_table();
    if std::env::var("AIWS_REGEN").is_ok() {
        std::fs::write(README, format!("{head}{BEGIN}{table}{END}{tail}")).unwrap();
        return;
    }
    assert_eq!(rest.split_once(END).unwrap().0, table, "README authorization table is stale; regenerate with AIWS_REGEN=1");
}
