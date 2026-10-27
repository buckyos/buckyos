//! A public link against the real process: a guest token (`gt_…`) in the kRPC envelope or as a bearer
//! stands in for a session, reaches `public` grants only, and can own nothing.

#[path = "common/nfs.rs"]
mod nfs;

use nfs::Nfs;
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const GUEST: &str = "gt_YWl3cy1ndWVzdC10b2tlbi1maXh0dXJlLTMyYnl0ZXM";

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

impl Server {
    async fn start(dir: &std::path::Path, nfs: &Nfs) -> Server {
        let auth = dir.join("tokens.json");
        std::fs::write(&auth, json!({ "tokens": { "tok-alice": { "principal": "alice" } } }).to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .args(nfs.args()).arg("--log-level").arg("warn").env_remove("AIWS_FAILPOINT")
            .stdout(Stdio::null()).stderr(Stdio::null())
            .spawn()
            .expect("spawn aiworkspace");
        let server = Server { child, base: format!("http://127.0.0.1:{port}/kapi/aiworkspace"), http: reqwest::Client::new() };
        for _ in 0..200 {
            if server.http.get(format!("{}/healthz", server.base)).send().await.is_ok_and(|r| r.status().is_success()) {
                return server;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("server did not start");
    }

    async fn raw(&self, token: &str, method: &str, params: Value) -> Result<Value, String> {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [seq, token] })).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
        match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None => Ok(v["result"].clone()),
        }
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        self.raw(token, method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"))
    }
}

fn code(v: &Value) -> &str {
    v["error"]["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn a_guest_token_reaches_public_grants_only() {
    let dir = tempfile::tempdir().unwrap();
    let nfs = Nfs::start(dir.path()).await;
    let srv = Server::start(dir.path(), &nfs).await;
    let ws = srv.rpc("tok-alice", "ws.create", json!({ "title": "公开的" })).await;
    let id = ws["workspace_id"].as_str().unwrap().to_string();
    // an asset to fetch through the bearer route
    let bound = nfs.put("alice", "assets", "upload/abcd", b"abcd".to_vec()).await;
    let staged = srv.rpc("tok-alice", "asset.import_object", json!({ "workspace_id": id, "bucket_id": bound["bucket_id"], "key": bound["key"] })).await;
    let object_id = staged["object_id"].as_str().unwrap().to_string();
    let r = srv.rpc("tok-alice", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": ws["epoch"], "idempotency_key": "a/1", "session_id": "s",
        "operations": [{ "op": "entity.create", "entity_id": "pic", "type_id": "buckyos.asset-ref", "parent_id": "data", "order_key": "m", "payload": { "object_id": object_id, "media_type": "text/plain", "size": 4 } }] })).await;
    assert_eq!(r["status"], "accepted", "{r}");

    // not public yet: the guest token is accepted as an identity, the workspace does not exist for it
    let r = srv.rpc(GUEST, "ws.get_info", json!({ "workspace_id": id })).await;
    assert_eq!(code(&r), "NOT_FOUND", "{r}");
    assert_eq!(srv.rpc(GUEST, "ws.list", json!({})).await["workspaces"], json!([]));
    assert_eq!(code(&srv.rpc(GUEST, "ws.create", json!({ "title": "x" })).await), "PERMISSION_DENIED");
    assert_eq!(code(&srv.rpc(GUEST, "ws.list_subjects", json!({})).await), "PERMISSION_DENIED");
    assert_eq!(code(&srv.rpc(GUEST, "ws.import", json!({ "bucket_id": bound["bucket_id"], "key": bound["key"], "semantics": "new" })).await), "PERMISSION_DENIED");
    assert!(srv.raw("gt_tooshort", "ws.get_info", json!({ "workspace_id": id })).await.is_err(), "a malformed guest token is no session");
    let asset = srv.http.get(format!("{}/asset/{}/{}", srv.base, id, object_id)).bearer_auth(GUEST).send().await.unwrap();
    assert_eq!(asset.status(), 404);

    // alice publishes the link
    srv.rpc("tok-alice", "ws.grant", json!({ "workspace_id": id, "subject": "public", "capabilities": ["read", "comment"] })).await;
    let info = srv.rpc(GUEST, "ws.get_info", json!({ "workspace_id": id })).await;
    assert_eq!(info["capabilities"], json!(["read", "comment"]), "{info}");
    assert!(info["principal"].as_str().unwrap().starts_with("guest:"));
    assert!(srv.rpc(GUEST, "doc.outline", json!({ "workspace_id": id })).await["entities"].as_array().unwrap().iter().any(|e| e["entity_id"] == "pic"));
    let asset = srv.http.get(format!("{}/asset/{}/{}", srv.base, id, object_id)).bearer_auth(GUEST).send().await.unwrap();
    assert_eq!(asset.status(), 200);
    assert_eq!(asset.text().await.unwrap(), "abcd");
    let r = srv.rpc(GUEST, "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": ws["epoch"], "idempotency_key": "g/1", "session_id": "g",
        "operations": [{ "op": "entity.create", "entity_id": "note", "type_id": "buckyos.annotation", "parent_id": "data", "order_key": "n", "payload": { "target": { "entity_id": "pic" }, "kind": "note", "body": "赞", "author_name": "路人" } }] })).await;
    assert_eq!(r["status"], "accepted", "{r}");
    let note = srv.rpc(GUEST, "doc.read", json!({ "workspace_id": id, "entity_id": "note" })).await;
    assert_eq!(note["content"]["payload"]["author"], info["principal"]);
    // guests cannot change who may come in
    assert_eq!(code(&srv.rpc(GUEST, "ws.revoke", json!({ "workspace_id": id, "subject": "public" })).await), "PERMISSION_DENIED");
    srv.rpc("tok-alice", "ws.revoke", json!({ "workspace_id": id, "subject": "public" })).await;
    assert_eq!(code(&srv.rpc(GUEST, "ws.get_info", json!({ "workspace_id": id })).await), "NOT_FOUND");
}
