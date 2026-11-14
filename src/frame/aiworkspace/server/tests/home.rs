//! The UI framework's service interfaces over HTTP (UI 框架 PRD §4.6, §9; K4, K6, K7): personal preferences shared by
//! every Workspace and kept apart per person, the Workspace's shared settings in `ws.get_info`, the open record behind
//! `ws.list.opened_at`, and `session.list_open` with a courseware reader who never joined opening the room without a link.

use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const FIXTURE: &str = include_str!("../../fixtures/presentation/commits.json");
const GUEST: &str = "gt_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

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
    async fn start(dir: &std::path::Path) -> Server {
        let auth = dir.join("tokens.json");
        std::fs::write(&auth, json!({ "tokens": { "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" } } }).to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--public-base").arg("http://aiws-{workspace_id}.localhost:4120")
            .arg("--log-level").arg("warn").env_remove("AIWS_FAILPOINT")
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

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [SEQ.fetch_add(1, Ordering::SeqCst), token] })).send().await.unwrap().json().await.unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    async fn ok(&self, token: &str, method: &str, params: Value) -> Value {
        let v = self.rpc(token, method, params).await;
        assert!(v["ok"] == json!(true) || v["status"] == json!("accepted"), "{method}: {v}");
        v
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn preferences_settings_opened_and_the_public_lecture_catalogue() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path()).await;
    // preferences: one object per person, whichever Workspace page writes it; another person sees nothing of it
    srv.ok("tok-alice", "user.set_prefs", json!({ "merge": { "inspector_dock": "right" } })).await;
    let again = srv.ok("tok-alice", "user.set_prefs", json!({ "merge": { "status_expanded": true }, "expect": 1 })).await;
    assert_eq!(again["prefs"], json!({ "inspector_dock": "right", "status_expanded": true }));
    assert_eq!(srv.ok("tok-alice", "user.get_prefs", json!({})).await["revision"], 2);
    assert_eq!(srv.ok("tok-bob", "user.get_prefs", json!({})).await["prefs"], json!({}));
    let stale = srv.rpc("tok-alice", "user.set_prefs", json!({ "merge": { "x": 1 }, "expect": 1 })).await;
    assert_eq!(stale["error"]["code"], "REVISION_CONFLICT", "{stale}");
    let guest = srv.rpc(GUEST, "user.get_prefs", json!({})).await;
    assert_eq!(guest["error"]["code"], "PERMISSION_DENIED", "{guest}");

    // the courseware: settings, an open record, a session
    let ws = srv.ok("tok-alice", "ws.create", json!({ "title": "公开课" })).await;
    let (id, epoch) = (ws["workspace_id"].as_str().unwrap().to_string(), ws["epoch"].as_str().unwrap().to_string());
    let fx: Value = serde_json::from_str(FIXTURE).unwrap();
    for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
        srv.ok("tok-alice", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": format!("fx/{i}"), "operations": c["operations"] })).await;
    }
    let toolbar = json!({ "zones": { "collab": { "items": [{ "action": "show.start" }] } } });
    srv.ok("tok-alice", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": "settings",
        "operations": [{ "op": "entity.set_props", "entity_id": "root", "key": "config", "props": { "toolbar": toolbar } }] })).await;
    srv.ok("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "bob", "resource": "/", "allow": ["read"] }] })).await;
    let info = srv.ok("tok-bob", "ws.get_info", json!({ "workspace_id": id })).await;
    assert_eq!(info["settings"], json!({ "toolbar": toolbar }), "every member reads the shared settings");
    assert!(info["settings_rev"].as_u64().unwrap() > 0);
    let opened = |list: &Value| list["workspaces"].as_array().unwrap().iter().find(|w| w["workspace_id"] == json!(id)).unwrap()["opened_at"].clone();
    assert_eq!(opened(&srv.ok("tok-bob", "ws.list", json!({})).await), Value::Null, "a plain ws.get_info records nothing");
    srv.ok("tok-bob", "ws.get_info", json!({ "workspace_id": id, "record_open": true })).await;
    assert!(opened(&srv.ok("tok-bob", "ws.list", json!({})).await).is_string());
    assert_eq!(opened(&srv.ok("tok-alice", "ws.list", json!({})).await), Value::Null, "the record is the caller's own");

    // bob reads the courseware and never joined: the catalogue offers the room, and he opens it without a link
    let created = srv.ok("tok-alice", "session.create", json!({ "workspace_id": id, "path_id": "path-intro", "title": "第一讲" })).await;
    let sid = created["session"]["session_id"].as_str().unwrap().to_string();
    let list = srv.ok("tok-bob", "session.list_open", json!({})).await;
    let item = &list["sessions"][0];
    assert_eq!((&item["session_id"], &item["my_role"], &item["presenting"], &item["online"], &item["cover"]), (&json!(sid), &json!("member"), &json!(false), &Value::Null, &Value::Null), "{list}");
    assert!(item["url"].as_str().is_some_and(|u| u.contains(&format!("?session={sid}"))), "{item}");
    assert!(srv.ok("tok-bob", "session.list_mine", json!({})).await["sessions"].as_array().unwrap().is_empty(), "not among the ones he joined");
    let mine = srv.ok("tok-alice", "session.list_open", json!({})).await;
    assert_eq!((&mine["sessions"][0]["my_role"], &mine["sessions"][0]["presenting"]), (&json!("presenter"), &json!(true)));
    let got = srv.ok("tok-bob", "session.get", json!({ "session_id": sid })).await;
    assert_eq!(got["role"], "audience", "{got}");
    let joined = srv.ok("tok-bob", "session.join", json!({ "session_id": sid, "name": "Bob" })).await;
    let run = joined["workspace_id"].as_str().unwrap().to_string();
    srv.ok("tok-bob", "doc.outline", json!({ "workspace_id": run })).await;
    assert_eq!(srv.ok("tok-bob", "session.list_open", json!({})).await["sessions"][0]["my_role"], "audience");
}
