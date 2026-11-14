//! `presence.*` and `ws.work_log` against the real process (UI 框架 PRD §4.2, §4.5, K2, K3): windows register and renew,
//! a person with two windows is one member on the canvas it moved to last, a reader sees itself first, others' groups
//! only as a manager and others' canvases only where it may read them, a guest under the name it chose; a window that
//! stops renewing drops out, one that leaves is gone at once; the log returns Commits with their messages, newest first,
//! and the non-public shows the service ran.

use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const FIXTURE: &str = include_str!("../../fixtures/presentation/commits.json");
const TTL_MS: u64 = 1500;
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
    async fn start(dir: &std::path::Path) -> Server {
        let auth = dir.join("tokens.json");
        let tokens = json!({ "tokens": { "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" }, "tok-carol": { "principal": "carol" } } });
        std::fs::write(&auth, tokens.to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--log-level").arg("warn").env("AIWS_PRESENCE_TTL_MS", TTL_MS.to_string()).env_remove("AIWS_FAILPOINT")
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
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [seq, token] })).send().await.unwrap().json().await.unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    /// alice's courseware (two canvases, `path-intro`); bob edits it, carol may read all but `s-work`; anyone may look.
    async fn courseware(&self) -> (String, String) {
        let ws = self.rpc("tok-alice", "ws.create", json!({ "title": "课件" })).await;
        let (id, epoch) = (ws["workspace_id"].as_str().unwrap().to_string(), ws["epoch"].as_str().unwrap().to_string());
        let fx: Value = serde_json::from_str(FIXTURE).unwrap();
        for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
            let mut req = commit_req(&id, &epoch, &format!("fixture/{i}"), c["operations"].clone());
            req["message"] = json!(format!("第 {i} 步"));
            let r = self.rpc("tok-alice", "doc.commit", req).await;
            assert_eq!(r["status"], "accepted", "{r}");
        }
        let changes = json!([{ "subject": "@editors", "allow": ["read", "update", "structure"] }, { "subject": "carol", "allow": ["read"] },
                             { "subject": "carol", "resource": "s-work", "deny": ["read"] }]);
        assert_eq!(self.rpc("tok-alice", "acl.set_group", json!({ "workspace_id": id, "group_id": "editors", "title": "编辑" })).await["ok"], true);
        assert_eq!(self.rpc("tok-alice", "acl.members", json!({ "workspace_id": id, "group_id": "editors", "add": ["bob"] })).await["ok"], true);
        let r = self.rpc("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": changes })).await;
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(self.rpc("tok-alice", "acl.set_anonymous", json!({ "workspace_id": id, "enabled": true })).await["ok"], true);
        let r = self.rpc("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "@guest", "allow": ["read"] }] })).await;
        assert_eq!(r["ok"], true, "{r}");
        (id, epoch)
    }

    async fn beat(&self, token: &str, ws: &str, conn: &str, surface: Option<&str>, name: Option<&str>) -> Value {
        let r = self.rpc(token, "presence.heartbeat", json!({ "workspace_id": ws, "conn_id": conn, "surface_id": surface, "name": name })).await;
        assert_eq!(r["ok"], true, "{r}");
        r
    }
}

fn commit_req(ws: &str, epoch: &str, key: &str, ops: Value) -> Value {
    json!({ "protocol_version": "1.0", "workspace_id": ws, "epoch": epoch, "idempotency_key": key, "session_id": "s1", "operations": ops })
}

fn member<'a>(list: &'a Value, principal: &str) -> &'a Value {
    list["members"].as_array().unwrap().iter().find(|m| m["principal"] == principal).unwrap_or_else(|| panic!("{principal} not in {list}"))
}

fn principals(list: &Value) -> Vec<String> {
    list["members"].as_array().unwrap().iter().map(|m| m["principal"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn presence_lists_people_on_their_canvases_with_groups_for_managers() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path()).await;
    let (ws, _) = srv.courseware().await;

    let first = srv.beat("tok-alice", &ws, "a1", Some("s-launch"), Some("不是我的名字")).await;
    assert_eq!((first["interval_ms"].as_u64(), first["ttl_ms"].as_u64()), (Some(TTL_MS / 3), Some(TTL_MS)));
    assert_eq!(principals(&first), ["alice"]);
    // bob: two windows, the second one moved to `s-work` last
    srv.beat("tok-bob", &ws, "b1", Some("s-launch"), None).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    srv.beat("tok-bob", &ws, "b2", Some("s-work"), None).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    srv.beat("tok-bob", &ws, "b1", Some("s-launch"), None).await;
    srv.beat("tok-carol", &ws, "c1", Some("s-work"), None).await;
    srv.beat(GUEST, &ws, "g1", Some("s-launch"), Some("  小李同学  ")).await;

    // the owner manages: herself first, everyone's groups, every canvas
    let seen = srv.rpc("tok-alice", "presence.list", json!({ "workspace_id": ws })).await;
    assert_eq!(principals(&seen)[0], "alice");
    assert_eq!(principals(&seen).len(), 4, "{seen}");
    let me = member(&seen, "alice");
    assert_eq!((me["self"].as_bool(), me["owner"].as_bool(), me["name"].clone()), (Some(true), Some(true), Value::Null), "an account's name is not taken from its window: {me}");
    let bob = member(&seen, "bob");
    assert_eq!((bob["devices"].as_u64(), bob["surface_id"].as_str()), (Some(2), Some("s-work")), "one person, the canvas it moved to last: {bob}");
    assert_eq!(bob["groups"], json!([{ "group_id": "editors", "title": "编辑" }]));
    assert_eq!(member(&seen, "carol")["groups"], json!([]), "in no group: {seen}");
    assert_eq!(member(&seen, "carol")["surface_id"], json!(null), "carol may not read s-work: her window could not report it");
    let guest = seen["members"].as_array().unwrap().iter().find(|m| m["guest"] == true).unwrap();
    assert_eq!(guest["name"], "小李同学");

    // bob: himself first with his own groups; nobody else's
    let seen = srv.beat("tok-bob", &ws, "b2", Some("s-work"), None).await;
    assert_eq!(principals(&seen)[0], "bob");
    assert_eq!(member(&seen, "bob")["groups"], json!([{ "group_id": "editors", "title": "编辑" }]));
    assert!(member(&seen, "alice").get("groups").is_none() && member(&seen, "alice").get("owner").is_none(), "{seen}");
    // carol does not see bob on s-work, a canvas she may not read
    let seen = srv.rpc("tok-carol", "presence.list", json!({ "workspace_id": ws })).await;
    assert_eq!((member(&seen, "bob")["surface_id"].clone(), member(&seen, "alice")["surface_id"].clone()), (Value::Null, json!("s-launch")), "{seen}");

    // leaving is at once; one of two windows leaves the person in
    assert_eq!(srv.rpc("tok-bob", "presence.leave", json!({ "workspace_id": ws, "conn_id": "b2" })).await["removed"], true);
    let seen = srv.rpc("tok-alice", "presence.list", json!({ "workspace_id": ws })).await;
    assert_eq!((member(&seen, "bob")["devices"].as_u64(), member(&seen, "bob")["surface_id"].as_str()), (Some(1), Some("s-launch")));
    // someone else's connection id is its own: carol cannot remove bob's
    assert_eq!(srv.rpc("tok-carol", "presence.leave", json!({ "workspace_id": ws, "conn_id": "b1" })).await["removed"], false);

    // windows that stop renewing drop out; the one that renews stays
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(TTL_MS / 3)).await;
        srv.beat("tok-alice", &ws, "a1", Some("s-launch"), None).await;
    }
    let seen = srv.rpc("tok-alice", "presence.list", json!({ "workspace_id": ws })).await;
    assert_eq!(principals(&seen), ["alice"], "{seen}");

    // someone who may not enter is told the Workspace does not exist; a bad connection id is refused
    let outsider = srv.rpc("tok-carol", "presence.heartbeat", json!({ "workspace_id": "ws_nothing", "conn_id": "x" })).await;
    assert_eq!(outsider["ok"], false, "{outsider}");
    let bad = srv.rpc("tok-bob", "presence.heartbeat", json!({ "workspace_id": ws, "conn_id": "a b" })).await;
    assert_eq!(bad["error"]["code"], "INVALID_OPERATION", "{bad}");
}

#[tokio::test]
async fn the_work_log_has_commits_with_messages_and_the_shows() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path()).await;
    let (ws, _) = srv.courseware().await;
    let start = srv.rpc("tok-bob", "show.start", json!({ "workspace_id": ws, "path_id": "path-intro", "live": false })).await;
    assert_eq!(start["ok"], true, "{start}");
    let show = start["show_id"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(srv.rpc("tok-bob", "show.end", json!({ "workspace_id": ws, "show_id": show })).await["ok"], true);

    let page = srv.rpc("tok-alice", "ws.work_log", json!({ "workspace_id": ws, "limit": 50 })).await;
    assert_eq!(page["ok"], true, "{page}");
    let entries = page["entries"].as_array().unwrap();
    let kinds: Vec<&str> = entries.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(&kinds[..2], ["show", "show"], "{kinds:?}");
    assert_eq!((entries[0]["phase"].as_str(), entries[1]["phase"].as_str(), entries[0]["actor"].as_str()), (Some("end"), Some("start"), Some("bob")));
    assert_eq!(entries[0]["target"]["entity_id"], "path-intro");
    let commits: Vec<&str> = entries.iter().filter(|e| e["kind"] == "commit").map(|e| e["message"].as_str().unwrap()).collect();
    let n = serde_json::from_str::<Value>(FIXTURE).unwrap()["commits"].as_array().unwrap().len();
    assert_eq!(commits.len(), n);
    assert_eq!(commits[0], format!("第 {} 步", n - 1), "newest first");
    assert!(kinds.contains(&"acl"), "the owner sees the permission record");
    // bob is no manager: no permission record; mine: his show only
    let bobs = srv.rpc("tok-bob", "ws.work_log", json!({ "workspace_id": ws })).await;
    assert!(bobs["entries"].as_array().unwrap().iter().all(|e| e["kind"] != "acl"), "{bobs}");
    let mine = srv.rpc("tok-bob", "ws.work_log", json!({ "workspace_id": ws, "filter": "mine" })).await;
    assert_eq!(mine["entries"].as_array().unwrap().len(), 2, "{mine}");
    // paging
    let first = srv.rpc("tok-alice", "ws.work_log", json!({ "workspace_id": ws, "limit": 2 })).await;
    assert_eq!((first["entries"].as_array().unwrap().len(), first["more"].as_bool()), (2, Some(true)));
    let next = srv.rpc("tok-alice", "ws.work_log", json!({ "workspace_id": ws, "limit": 2, "before": first["next_before"] })).await;
    assert_eq!(next["entries"][0]["id"], entries[2]["id"], "{next}");
    let bad = srv.rpc("tok-alice", "ws.work_log", json!({ "workspace_id": ws, "filter": "nope" })).await;
    assert_eq!(bad["error"]["code"], "INVALID_OPERATION");
}
