//! `show.*` against the real process (第三期规划 §8, §11.3, P3-12, P3-13, P3-15): the show lock refuses writes from
//! every other session, a live show gets a clone that writes stay in, the prompter token commands / watches /
//! reads notes and nothing else, the relay keeps the newest state, a stage that stops renewing loses the show,
//! and a manager can end someone else's show.

use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const FIXTURE: &str = include_str!("../../fixtures/presentation/commits.json");
const LEASE_MS: u64 = 1500;

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
        std::fs::write(&auth, json!({ "tokens": { "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" }, "tok-carol": { "principal": "carol" } } }).to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--log-level").arg("warn").env("AIWS_SHOW_LEASE_MS", LEASE_MS.to_string()).env_remove("AIWS_FAILPOINT")
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

    async fn raw(&self, token: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let sys = match token {
            Some(t) => json!([seq, t]),
            None => json!([seq]),
        };
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": sys })).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
        match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None => Ok(v["result"].clone()),
        }
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        self.raw(Some(token), method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    /// A prompter: no session, only the show's token.
    async fn prompter(&self, method: &str, mut params: Value, token: &str) -> Value {
        params["prompter_token"] = json!(token);
        self.raw(None, method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    async fn presentation(&self) -> (String, String) {
        let ws = self.rpc("tok-alice", "ws.create", json!({ "title": "发布会" })).await;
        let (id, epoch) = (ws["workspace_id"].as_str().unwrap().to_string(), ws["epoch"].as_str().unwrap().to_string());
        let fx: Value = serde_json::from_str(FIXTURE).unwrap();
        for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
            let r = self.rpc("tok-alice", "doc.commit", commit_req(&id, &epoch, &format!("fixture/{i}"), c["operations"].clone())).await;
            assert_eq!(r["status"], "accepted", "{r}");
        }
        for (who, caps) in [("bob", json!(["read", "update", "structure"])), ("carol", json!(["read"]))] {
            let r = self.rpc("tok-alice", "ws.grant", json!({ "workspace_id": id, "subject": who, "capabilities": caps })).await;
            assert_eq!(r["ok"], true, "{r}");
        }
        (id, epoch)
    }

    /// Bob renames `shape-cover` (any write will do).
    async fn bob_writes(&self, ws: &str, epoch: &str, key: &str) -> Value {
        let read = self.rpc("tok-bob", "doc.read", json!({ "workspace_id": ws, "entity_id": "shape-cover" })).await;
        let rev = read["content"]["key_revs"]["title"].as_u64().unwrap_or(0);
        let op = json!([{ "op": "entity.set_keys", "entity_id": "shape-cover", "keys": [{ "key": "title", "value": key, "expect": { "rev": rev } }] }]);
        self.rpc("tok-bob", "doc.commit", commit_req(ws, epoch, key, op)).await
    }
}

fn commit_req(ws: &str, epoch: &str, key: &str, ops: Value) -> Value {
    json!({ "protocol_version": "0.5", "workspace_id": ws, "epoch": epoch, "idempotency_key": key, "session_id": "s1", "operations": ops })
}

fn code(v: &Value) -> &str {
    v["code"].as_str().or_else(|| v["error"]["code"].as_str()).unwrap_or("")
}

#[tokio::test]
async fn a_live_show_locks_clones_relays_and_ends() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path()).await;
    let (ws, epoch) = srv.presentation().await;
    let start = srv.rpc("tok-alice", "show.start", json!({ "workspace_id": ws, "path_id": "path-intro", "live": true })).await;
    assert_eq!((start["locked"].clone(), start["live"].clone()), (json!(true), json!(true)), "{start}");
    let show = start["show_id"].as_str().unwrap().to_string();
    let token = start["prompter_token"].as_str().unwrap().to_string();
    let clone = start["clone_workspace_id"].as_str().unwrap().to_string();
    let at = |extra: Value| {
        let mut p = json!({ "workspace_id": ws, "show_id": show });
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };

    // writes to the original are closed for everyone; the clone is not listed but writable
    let r = srv.bob_writes(&ws, &epoch, "bob/1").await;
    assert_eq!((r["status"].as_str(), code(&r)), (Some("rejected"), "SHOW_LOCKED"), "{r}");
    assert_eq!(r["errors"][0]["data"]["presenter"], "alice");
    assert_eq!(srv.rpc("tok-bob", "ws.get_info", json!({ "workspace_id": ws })).await["show_lock"]["show_id"], json!(show));
    let list = srv.rpc("tok-alice", "ws.list", json!({})).await;
    assert_eq!(list["workspaces"].as_array().unwrap().len(), 1, "{list}");
    let ci = srv.rpc("tok-alice", "ws.get_info", json!({ "workspace_id": clone })).await;
    assert_eq!(ci["purpose"], "show");
    let rev = srv.rpc("tok-alice", "doc.read", json!({ "workspace_id": clone, "entity_id": "d-note-live" })).await["content"]["key_revs"]["body"].as_u64().unwrap();
    let r = srv.rpc("tok-alice", "doc.commit", commit_req(&clone, ci["epoch"].as_str().unwrap(), "live/1",
        json!([{ "op": "entity.set_keys", "entity_id": "d-note-live", "keys": [{ "key": "body", "value": "现场写的", "expect": { "rev": rev } }] }]))).await;
    assert_eq!(r["status"], "accepted", "{r}");

    // the prompter: notes as the presenter reads them, commands, the newest state
    let notes = srv.prompter("show.notes", at(json!({})), &token).await;
    let steps = notes["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 5);
    assert!(steps[0]["notes"].as_str().unwrap().contains("开场"));
    assert_eq!(steps[2]["title"], "工作流全貌");
    assert_eq!(steps[3]["caption"], "放映时可以直接在这张便签上写");
    let c1 = srv.prompter("show.command", at(json!({ "command": { "command_id": "p-1", "kind": "next" } })), &token).await;
    assert_eq!(c1["duplicate"], false, "{c1}");
    let again = srv.prompter("show.command", at(json!({ "command": { "command_id": "p-1", "kind": "next" } })), &token).await;
    assert_eq!(again["duplicate"], true);
    let got = srv.rpc("tok-alice", "show.watch", at(json!({ "after_command": 0, "timeout_ms": 2000 }))).await;
    assert_eq!(got["commands"].as_array().unwrap().len(), 1, "{got}");
    assert_eq!(got["commands"][0]["command"]["kind"], "next");
    // a long poll for the state wakes when the stage publishes
    let watcher = {
        let (base, http, p) = (srv.base.clone(), srv.http.clone(), at(json!({ "after_seq": 0, "timeout_ms": 5000, "prompter_token": token })));
        tokio::spawn(async move { http.post(&base).json(&json!({ "method": "show.watch", "params": p, "sys": [1] })).send().await.unwrap().json::<Value>().await.unwrap() })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let p = srv.rpc("tok-alice", "show.publish", at(json!({ "seq": 2, "state": { "step_id": "s2", "black": false } }))).await;
    assert_eq!(p["seq"], 2);
    let woke = watcher.await.unwrap();
    assert_eq!(woke["result"]["state"]["step_id"], "s2", "{woke}");
    // an older state arriving late does not win
    srv.rpc("tok-alice", "show.publish", at(json!({ "seq": 1, "state": { "step_id": "s1" } }))).await;
    let now = srv.prompter("show.watch", at(json!({})), &token).await;
    assert_eq!((now["seq"].clone(), now["state"]["step_id"].clone()), (json!(2), json!("s2")));
    let idle = srv.prompter("show.watch", at(json!({ "after_seq": 2, "timeout_ms": 300 })), &token).await;
    assert_eq!(idle["timed_out"], true);

    // the token is for this show's commands, watching and notes only
    for method in ["show.publish", "show.heartbeat", "show.end"] {
        let mut p = at(json!({ "seq": 9, "state": {}, "prompter_token": token }));
        p["prompter_token"] = json!(token);
        let r = srv.raw(None, method, p).await.unwrap();
        assert_eq!(r["ok"], false, "{method}: {r}");
    }
    let wrong = srv.raw(None, "show.watch", at(json!({ "prompter_token": "pt_aaaaaaaaaaaaaaaaaaaaaaaaaa" }))).await.unwrap();
    assert_eq!(wrong["error"]["sub_code"], "SHOW_ENDED", "{wrong}");
    assert!(srv.raw(None, "doc.outline", json!({ "workspace_id": ws, "prompter_token": token })).await.is_err(), "a token is no session");
    // only the presenter's sessions drive the stage
    let r = srv.rpc("tok-bob", "show.publish", at(json!({ "seq": 5, "state": {} }))).await;
    assert_eq!(code(&r), "PERMISSION_DENIED", "{r}");

    // the presenter ends it: lock released, clone deleted, token dead
    srv.rpc("tok-alice", "show.end", at(json!({}))).await;
    assert_eq!(srv.bob_writes(&ws, &epoch, "bob/2").await["status"], "accepted");
    assert_eq!(code(&srv.rpc("tok-alice", "ws.get_info", json!({ "workspace_id": clone })).await), "NOT_FOUND");
    assert!(!dir.path().join("data/workspaces").join(&clone).exists());
    let r = srv.prompter("show.watch", at(json!({})), &token).await;
    assert_eq!(r["error"]["sub_code"], "SHOW_ENDED", "{r}");
    // the original never saw the live write
    let body = srv.rpc("tok-alice", "doc.read", json!({ "workspace_id": ws, "entity_id": "d-note-live" })).await;
    assert_eq!(body["content"]["payload"]["body"], "放映时可以在这里写");
}

#[tokio::test]
async fn read_only_shows_crashed_stages_and_forced_ends() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path()).await;
    let (ws, epoch) = srv.presentation().await;

    // a reader presents without locking or cloning, whatever it asks for
    let ro = srv.rpc("tok-carol", "show.start", json!({ "workspace_id": ws, "path_id": "path-guide", "live": true })).await;
    assert_eq!((ro["locked"].clone(), ro["live"].clone(), ro["clone_workspace_id"].clone()), (json!(false), json!(false), Value::Null), "{ro}");
    assert_eq!(srv.bob_writes(&ws, &epoch, "bob/a").await["status"], "accepted");
    let r = srv.rpc("tok-carol", "show.start", json!({ "workspace_id": ws, "path_id": "shape-cover" })).await;
    assert_eq!(code(&r), "INVALID_OPERATION", "{r}");

    // a stage that keeps renewing keeps the lock; one that stops loses the show and its clone
    let s = srv.rpc("tok-alice", "show.start", json!({ "workspace_id": ws, "path_id": "path-intro", "live": true })).await;
    let show = s["show_id"].as_str().unwrap().to_string();
    let clone = s["clone_workspace_id"].as_str().unwrap().to_string();
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(LEASE_MS / 3)).await;
        let hb = srv.rpc("tok-alice", "show.heartbeat", json!({ "workspace_id": ws, "show_id": show })).await;
        assert_eq!(hb["ok"], true, "{hb}");
    }
    assert_eq!(code(&srv.bob_writes(&ws, &epoch, "bob/b").await), "SHOW_LOCKED");
    tokio::time::sleep(Duration::from_millis(LEASE_MS + 800)).await;
    assert_eq!(srv.bob_writes(&ws, &epoch, "bob/c").await["status"], "accepted");
    assert!(!dir.path().join("data/workspaces").join(&clone).exists());
    let hb = srv.rpc("tok-alice", "show.heartbeat", json!({ "workspace_id": ws, "show_id": show })).await;
    assert_eq!(hb["error"]["sub_code"], "SHOW_ENDED", "{hb}");

    // bob presents; another show cannot take the Workspace; the owner ends bob's show from the lock it sees
    let b = srv.rpc("tok-bob", "show.start", json!({ "workspace_id": ws, "path_id": "path-intro" })).await;
    assert_eq!(b["locked"], true, "{b}");
    let r = srv.rpc("tok-alice", "show.start", json!({ "workspace_id": ws, "path_id": "path-intro" })).await;
    assert_eq!(code(&r), "SHOW_LOCKED", "{r}");
    let held = srv.rpc("tok-alice", "ws.get_info", json!({ "workspace_id": ws })).await["show_lock"].clone();
    assert_eq!(held["presenter"], "bob");
    let r = srv.rpc("tok-carol", "show.end", json!({ "workspace_id": ws, "show_id": held["show_id"] })).await;
    assert_eq!(code(&r), "PERMISSION_DENIED", "{r}");
    let r = srv.rpc("tok-alice", "show.end", json!({ "workspace_id": ws, "show_id": held["show_id"] })).await;
    assert_eq!(r["ok"], true, "{r}");
    assert!(srv.rpc("tok-alice", "ws.get_info", json!({ "workspace_id": ws })).await["show_lock"].is_null());
}
