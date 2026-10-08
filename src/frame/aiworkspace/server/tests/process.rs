//! Integration tests against the real `aiworkspace` process over HTTP (V23,
//! and crash recovery at the three positions: before the transaction, inside
//! it, and after it is durable but before the response).

use base64::Engine;
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const FIXTURE: &str = include_str!("../../fixtures/project-workspace/commits.json");

struct Server {
    child: Child,
    base: String,
    http: reqwest::Client,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

impl Server {
    async fn start(dir: &std::path::Path, failpoint: Option<&str>) -> Server {
        let auth = dir.join("tokens.json");
        std::fs::write(&auth, json!({ "tokens": { "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob", "app_id": "system:control-panel" } } }).to_string()).unwrap();
        let port = free_port();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_aiworkspace"));
        cmd.arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--fixture-sources").arg("--log-level").arg("warn").stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(f) = failpoint {
            cmd.env("AIWS_FAILPOINT", f);
        } else {
            cmd.env_remove("AIWS_FAILPOINT");
        }
        let child = cmd.spawn().expect("spawn aiworkspace");
        let server = Server { child, base: format!("http://127.0.0.1:{port}/kapi/aiworkspace"), http: reqwest::Client::new() };
        for _ in 0..200 {
            if let Ok(r) = server.http.get(format!("{}/healthz", server.base)).send().await {
                if r.status().is_success() {
                    return server;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("server did not start");
    }

    async fn raw(&self, token: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let sys = match token {
            Some(t) => json!([seq, t]),
            None => json!([seq]),
        };
        let resp = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": sys })).send().await.map_err(|e| format!("transport: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("transport: {e}"))?;
        match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None => {
                assert_eq!(v["sys"][0], json!(seq));
                Ok(v["result"].clone())
            }
        }
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        self.raw(Some(token), method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    async fn upload(&self, token: &str, begin: (&str, Value), bytes: Vec<u8>) -> String {
        let b = self.rpc(token, begin.0, begin.1).await;
        let id = b["upload_id"].as_str().unwrap_or_else(|| panic!("{b}")).to_string();
        let r = self.http.put(format!("{}/upload/{id}", self.base)).bearer_auth(token).body(bytes).send().await.unwrap();
        assert!(r.status().is_success(), "{}", r.text().await.unwrap());
        id
    }

    /// Create a Workspace and replay the shared fixture through the public interface only.
    async fn project(&self, token: &str) -> (String, String) {
        let ws = self.rpc(token, "ws.create", json!({ "title": "项目工作区" })).await;
        let (id, epoch) = (ws["workspace_id"].as_str().unwrap().to_string(), ws["epoch"].as_str().unwrap().to_string());
        let fx: Value = serde_json::from_str(FIXTURE).unwrap();
        let mut text = serde_json::to_string(&fx["commits"]).unwrap();
        for a in fx["assets"].as_array().unwrap() {
            let bytes = base64::engine::general_purpose::STANDARD.decode(a["base64"].as_str().unwrap()).unwrap();
            let up = self.upload(token, ("asset.begin_upload", json!({ "workspace_id": id, "size": bytes.len() })), bytes).await;
            let fin = self.rpc(token, "asset.finish_upload", json!({ "workspace_id": id, "upload_id": up })).await;
            assert_eq!(fin["media_type"], "image/png", "{fin}");
            text = text.replace(a["placeholder"].as_str().unwrap(), fin["object_id"].as_str().unwrap());
        }
        for (i, c) in serde_json::from_str::<Vec<Value>>(&text).unwrap().iter().enumerate() {
            let r = self.rpc(token, "doc.commit", commit_req(&id, &epoch, &format!("fixture/{}", i + 1), c["operations"].clone())).await;
            assert_eq!(r["status"], "accepted", "{r}");
        }
        (id, epoch)
    }
}

fn commit_req(ws: &str, epoch: &str, key: &str, ops: Value) -> Value {
    json!({ "protocol_version": "0.4", "workspace_id": ws, "epoch": epoch, "idempotency_key": key, "session_id": "s1", "operations": ops })
}

fn mixed_batch(ws: &str, epoch: &str) -> Value {
    commit_req(ws, epoch, "mixed-1", json!([
        { "op": "tree.place", "entity_id": "cell-notes", "order_key": "e5" },
        { "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "after": "n-title" },
          "blocks": [{ "type": "paragraph", "attrs": { "block_id": "n-crash" }, "content": [{ "type": "text", "text": "崩溃测试段落" }] }] },
        { "op": "table.set_values", "source_id": "tasks",
          "values": [{ "record_id": "task-42", "field_id": "status", "value": "option-done", "expect": { "rev": 2 } }] }
    ]))
}

/// V23 (backend): the service starts on its own, serves the whole workflow through
/// its API with nothing but HTTP, authenticates every call, and keeps accepted
/// commits across a process restart.
#[tokio::test(flavor = "multi_thread")]
async fn v23_service_api_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(dir.path(), None).await;
    // authentication: no token / wrong token never reach a method; identity never comes from the body
    assert!(s.raw(None, "ws.list", json!({})).await.unwrap_err().contains("token"));
    assert!(s.raw(Some("nope"), "ws.list", json!({ "principal": "alice" })).await.is_err());
    assert!(s.raw(Some("tok-alice"), "no.such", json!({})).await.unwrap_err().contains("unknown method"));
    let (id, epoch) = s.project("tok-alice").await;
    let w = json!({ "workspace_id": id });
    // reads, queries, structured business errors in `result`
    let info = s.rpc("tok-alice", "ws.get_info", w.clone()).await;
    assert_eq!((info["head_seq"].as_u64(), info["title"].as_str()), (Some(6), Some("项目工作区")));
    assert_eq!(s.rpc("tok-alice", "doc.outline", w.clone()).await["entities"].as_array().unwrap().len(), 17);
    let open = s.rpc("tok-alice", "doc.query", json!({ "workspace_id": id, "view_id": "cell-open-tasks" })).await;
    assert_eq!(open["total"], 4);
    let batch = s.rpc("tok-alice", "doc.read", json!({ "workspace_id": id, "targets": [{ "entity_id": "notes" }, { "entity_id": "ghost" }] })).await;
    assert_eq!((batch["results"][0]["type_id"].as_str(), batch["results"][1]["error"]["code"].as_str()), (Some("buckyos.richtext"), Some("NOT_FOUND")));
    let bob_view = s.rpc("tok-bob", "ws.get_info", w.clone()).await;
    assert_eq!((bob_view["ok"].as_bool(), bob_view["error"]["code"].as_str()), (Some(false), Some("NOT_FOUND")), "no grant: it does not exist");
    assert_eq!(s.rpc("tok-bob", "ws.list", json!({})).await["workspaces"], json!([]));
    let conflict = s.rpc("tok-alice", "doc.commit", commit_req(&id, &epoch, "stale", json!([{ "op": "table.set_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": "budget", "value": "1", "expect": { "rev": 1 } }] }]))).await;
    assert_eq!((conflict["status"].as_str(), conflict["code"].as_str()), (Some("conflict"), Some("REVISION_CONFLICT")));
    assert_eq!(s.rpc("tok-alice", "doc.commit", commit_req(&id, "ep_old", "x", json!([]))).await["code"], "EPOCH_MISMATCH");
    // asset bytes over the authenticated route
    let obj = s.rpc("tok-alice", "doc.read", json!({ "workspace_id": id, "entity_id": "diagram" })).await["content"]["payload"]["object_id"].as_str().unwrap().to_string();
    let img = s.http.get(format!("{}/asset/{id}/{obj}", s.base)).bearer_auth("tok-alice").send().await.unwrap();
    assert_eq!(img.headers()["content-type"], "image/png");
    assert_eq!(&img.bytes().await.unwrap()[..4], b"\x89PNG");
    assert_eq!(s.http.get(format!("{}/asset/{id}/{obj}", s.base)).send().await.unwrap().status(), 401);
    assert_eq!(s.http.get(format!("{}/asset/{id}/{obj}", s.base)).bearer_auth("tok-bob").send().await.unwrap().status(), 404);
    // long poll wakes up on the next accepted commit; the hint carries no content
    let waiter = {
        let (base, http, id, epoch) = (s.base.clone(), s.http.clone(), id.clone(), epoch.clone());
        tokio::spawn(async move {
            let r: Value = http.post(&base).json(&json!({ "method": "doc.wait_changes",
                "params": { "workspace_id": id, "epoch": epoch, "after_seq": 6, "timeout_ms": 20000 }, "sys": [1, "tok-alice"] })).send().await.unwrap().json().await.unwrap();
            r["result"].clone()
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    let c = s.rpc("tok-alice", "doc.commit", mixed_batch(&id, &epoch)).await;
    assert_eq!(c["status"], "accepted", "{c}");
    let woke = tokio::time::timeout(Duration::from_secs(5), waiter).await.expect("wake-up").unwrap();
    assert_eq!(woke, json!({ "ok": true, "epoch": epoch, "head_seq": 7, "timed_out": false }));
    let quiet = s.rpc("tok-alice", "doc.wait_changes", json!({ "workspace_id": id, "epoch": epoch, "after_seq": 7, "timeout_ms": 100 })).await;
    assert_eq!(quiet["timed_out"], true);
    let changes = s.rpc("tok-alice", "doc.get_changes", json!({ "workspace_id": id, "epoch": epoch, "after_seq": 6 })).await;
    assert_eq!(changes["changes"][0]["ops"].as_array().unwrap().len(), 3);
    // Mock run, URL table and undo through the API
    let run = s.rpc("tok-alice", "proc.start", json!({ "workspace_id": id, "program": "mock.task-summary@1", "idempotency_key": "run-1", "params": { "today": "2026-10-13" } })).await;
    assert_eq!(run["state"], "waiting_confirmation", "{run}");
    let applied = s.rpc("tok-alice", "proc.apply", json!({ "workspace_id": id, "run_id": run["run_id"], "session_id": "s1" })).await;
    assert_eq!(applied["state"], "succeeded", "{applied}");
    let undone = s.rpc("tok-alice", "doc.undo", json!({ "workspace_id": id, "epoch": epoch, "commit_id": applied["result"]["commit_id"], "idempotency_key": "undo-run" })).await;
    assert_eq!(undone["status"], "accepted", "{undone}");
    let r = s.rpc("tok-alice", "doc.commit", commit_req(&id, &epoch, "url", json!([{ "op": "entity.create", "entity_id": "events", "type_id": "buckyos.table-source",
        "parent_id": "data", "order_key": "v", "payload": { "data_mode": "url_query",
        "source_ref": { "kind": "url_query", "source_url": "fixture://events?rows=100000&snapshot=1", "query": {} },
        "fields": [{ "field_id": "event_id", "name": "事件", "type": "text" }] } }]))).await;
    assert_eq!(r["status"], "accepted", "{r}");
    let page = s.rpc("tok-alice", "doc.query", json!({ "workspace_id": id, "source_id": "events", "limit": 5 })).await;
    assert_eq!((page["rows"].as_array().unwrap().len(), page["data_mode"].as_str()), (5, Some("url_query")));
    // export → download → upload into a second, empty deployment → same content root
    let exp = s.rpc("tok-alice", "doc.export", json!({ "workspace_id": id, "mode": "share", "self_contained": true })).await;
    assert!(exp.get("path").is_none());
    let pkg = s.http.get(format!("{}/export/{id}/{}", s.base, exp["export_id"].as_str().unwrap())).bearer_auth("tok-alice").send().await.unwrap().bytes().await.unwrap().to_vec();
    assert_eq!(&pkg[..2], b"PK");
    let dir2 = tempfile::tempdir().unwrap();
    let s2 = Server::start(dir2.path(), None).await;
    let up = s2.upload("tok-bob", ("ws.begin_import", json!({})), pkg).await;
    assert_eq!(s2.rpc("tok-bob", "ws.import", json!({ "upload_id": up, "semantics": "sideways" })).await["error"]["code"], "INVALID_OPERATION", "semantics has no default");
    assert_eq!(s2.rpc("tok-bob", "ws.import", json!({ "upload_id": up, "semantics": "new" })).await["error"]["code"], "NOT_FOUND", "an upload is consumed by its import attempt");
    assert_eq!(s2.rpc("tok-bob", "ws.list", json!({})).await["workspaces"], json!([]), "nothing became visible");
    let pkg2 = s.http.get(format!("{}/export/{id}/{}", s.base, exp["export_id"].as_str().unwrap())).bearer_auth("tok-alice").send().await.unwrap().bytes().await.unwrap().to_vec();
    let up = s2.upload("tok-bob", ("ws.begin_import", json!({})), pkg2).await;
    let imp = s2.rpc("tok-bob", "ws.import", json!({ "upload_id": up, "semantics": "new" })).await;
    assert_eq!(imp["content_root"], exp["manifest"]["content_root"], "{imp}");
    let forked = s.rpc("tok-alice", "ws.fork", w.clone()).await;
    assert_eq!(forked["content_root"], s.rpc("tok-alice", "doc.checkpoint", w.clone()).await["content_root"]);
    // the replica database is downloadable only by a workspace-level reader
    let rep = s.rpc("tok-alice", "replica.bootstrap", w.clone()).await;
    let db = s.http.get(format!("{}/replica/{id}/{}", s.base, rep["replica_id"].as_str().unwrap())).bearer_auth("tok-alice").send().await.unwrap().bytes().await.unwrap();
    assert_eq!(&db[..15], b"SQLite format 3");
    // restart the process: everything accepted is still there, with nothing in memory to rely on
    let head = s.rpc("tok-alice", "ws.get_info", w.clone()).await["head_seq"].clone();
    drop(s);
    let s = Server::start(dir.path(), None).await;
    let info = s.rpc("tok-alice", "ws.get_info", w.clone()).await;
    assert_eq!((&info["head_seq"], &info["epoch"]), (&head, &json!(epoch)));
    let notes = s.rpc("tok-alice", "doc.read", json!({ "workspace_id": id, "entity_id": "notes" })).await;
    assert_eq!(notes["content"]["content"]["content"][1]["attrs"]["block_id"], "n-crash");
    assert_eq!(s.rpc("tok-alice", "diag.verify_refs", w.clone()).await["ok"], true);
    assert_eq!(s.rpc("tok-alice", "doc.commit", mixed_batch(&id, &epoch)).await["replayed"], true);
}

/// Crash at each of the three positions while committing a mixed batch. After the
/// restart there is either nothing of it or all of it, and resending the same
/// request (same key) takes effect exactly once.
#[tokio::test(flavor = "multi_thread")]
async fn crash_recovery_at_three_positions() {
    for (point, durable) in [("before_txn", false), ("in_txn", false), ("after_commit", true)] {
        let dir = tempfile::tempdir().unwrap();
        // the 7th commit reaching the point is the mixed batch (the fixture is 6 commits)
        let mut s = Server::start(dir.path(), Some(&format!("{point}:abort:7"))).await;
        let (id, epoch) = s.project("tok-alice").await;
        let w = json!({ "workspace_id": id });
        let before_root = s.rpc("tok-alice", "doc.checkpoint", w.clone()).await["content_root"].clone();
        let lost = s.raw(Some("tok-alice"), "doc.commit", mixed_batch(&id, &epoch)).await;
        assert!(lost.unwrap_err().starts_with("transport"), "{point}: the process died without answering");
        assert!(!s.child.wait().unwrap().success(), "{point}: aborted");
        drop(s);
        let s = Server::start(dir.path(), None).await;
        let info = s.rpc("tok-alice", "ws.get_info", w.clone()).await;
        let sub = s.rpc("tok-alice", "doc.get_submission", json!({ "workspace_id": id, "epoch": epoch, "idempotency_key": "mixed-1" })).await;
        let root = s.rpc("tok-alice", "doc.checkpoint", w.clone()).await["content_root"].clone();
        if durable {
            assert_eq!((info["head_seq"].as_u64(), sub["status"].as_str()), (Some(7), Some("accepted")), "{point}");
            assert_ne!(root, before_root, "{point}: the whole batch is there");
        } else {
            assert_eq!((info["head_seq"].as_u64(), sub["status"].as_str()), (Some(6), Some("not_found")), "{point}");
            assert_eq!(root, before_root, "{point}: no partial commit");
            assert_eq!(s.rpc("tok-alice", "doc.get_changes", json!({ "workspace_id": id, "epoch": epoch, "after_seq": 6 })).await["changes"], json!([]));
        }
        // the unknown result is resolved by resending the very same request
        let again = s.rpc("tok-alice", "doc.commit", mixed_batch(&id, &epoch)).await;
        assert_eq!((again["status"].as_str(), again["replayed"].as_bool(), again["seq"].as_u64()), (Some("accepted"), Some(durable), Some(7)), "{point}: {again}");
        assert_eq!(s.rpc("tok-alice", "ws.get_info", w.clone()).await["head_seq"], 7, "{point}: applied exactly once");
        // the in-memory CRDT document rebuilt from snapshot + updates agrees with the stored projection
        let notes = s.rpc("tok-alice", "doc.read", json!({ "workspace_id": id, "entity_id": "notes" })).await;
        let blocks: Vec<&str> = notes["content"]["content"]["content"].as_array().unwrap().iter().filter_map(|b| b["attrs"]["block_id"].as_str()).collect();
        assert_eq!(blocks.iter().filter(|b| **b == "n-crash").count(), 1, "{point}");
        let collab = s.rpc("tok-alice", "doc.get_collab_state", json!({ "workspace_id": id, "entity_id": "notes" })).await;
        let doc = loro::LoroDoc::new();
        doc.import(&base64::engine::general_purpose::STANDARD.decode(collab["snapshot"].as_str().unwrap()).unwrap()).unwrap();
        let children = doc.get_deep_value();
        assert!(format!("{children:?}").contains("崩溃测试段落"), "{point}");
        assert_eq!(s.rpc("tok-alice", "diag.verify_refs", w.clone()).await["ok"], true, "{point}");
        assert_eq!(s.rpc("tok-alice", "doc.read", json!({ "workspace_id": id, "entity_id": "tasks",
            "selector": { "kind": "table_cell", "record_id": "task-42", "field_id": "status" } })).await["content"]["value"], "option-done");
    }
}
