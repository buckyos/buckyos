//! Presentation sessions against the real process (第四期规划 P4-02/04/09/13/14/16/17): a room is created from a
//! Workspace, the audience joins through a link over the session channel (WebSocket), the presenter's commands are
//! confirmed and fanned out, the last claim slot goes to one of many concurrent claimants, private answers never reach
//! other participants' channels, display and prompter links read under their own subject, and after a restart the
//! session continues from what was confirmed.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const FIXTURE: &str = include_str!("../../fixtures/presentation/commits.json");
const GUEST: &str = "gt_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

struct Server {
    child: Child,
    port: u16,
    base: String,
    http: reqwest::Client,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tokens() -> Value {
    json!({ "tokens": { "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" }, "tok-carol": { "principal": "carol" },
                        "tok-dave": { "principal": "dave" }, "tok-erin": { "principal": "erin" }, "tok-frank": { "principal": "frank" } } })
}

impl Server {
    async fn start(dir: &std::path::Path) -> Server {
        let auth = dir.join("tokens.json");
        std::fs::write(&auth, tokens().to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--public-base").arg("http://aiws-{workspace_id}.localhost:4120")
            .arg("--log-level").arg("warn").env_remove("AIWS_FAILPOINT")
            .stdout(Stdio::null()).stderr(Stdio::null())
            .spawn()
            .expect("spawn aiworkspace");
        let server = Server { child, port, base: format!("http://127.0.0.1:{port}/kapi/aiworkspace"), http: reqwest::Client::new() };
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

    async fn ok(&self, token: &str, method: &str, params: Value) -> Value {
        let v = self.rpc(token, method, params).await;
        assert_eq!(v["ok"], true, "{method}: {v}");
        v
    }

    async fn courseware(&self) -> String {
        let ws = self.rpc("tok-alice", "ws.create", json!({ "title": "公开课" })).await;
        let (id, epoch) = (ws["workspace_id"].as_str().unwrap().to_string(), ws["epoch"].as_str().unwrap().to_string());
        let fx: Value = serde_json::from_str(FIXTURE).unwrap();
        for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
            let r = self.rpc("tok-alice", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": format!("fx/{i}"), "operations": c["operations"] })).await;
            assert_eq!(r["status"], "accepted", "{r}");
        }
        for feed in ["tasks", "my_responses", "responses", "participants", "stage"] {
            self.ok("tok-alice", "session.feed_sample", json!({ "workspace_id": id, "feed": feed })).await;
        }
        id
    }

    async fn channel(&self, session: &str, token: Option<&str>, link: Option<&str>, purpose: &str, terminal: &str) -> Channel {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/kapi/aiworkspace/session/ws", self.port)).await.expect("ws connect");
        let mut ch = Channel { ws, seen: Vec::new() };
        ch.send(json!({ "type": "hello", "session_id": session, "token": token, "link": link, "purpose": purpose, "terminal_id": terminal, "after": 0, "name": terminal })).await;
        let welcome = ch.next_of("welcome").await;
        assert!(welcome["role"].is_string(), "{welcome}");
        ch.seen.push(welcome);
        ch
    }
}

struct Channel {
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    seen: Vec<Value>,
}

impl Channel {
    async fn send(&mut self, v: Value) {
        self.ws.send(Message::Text(v.to_string().into())).await.unwrap();
    }
    async fn next(&mut self) -> Option<Value> {
        match tokio::time::timeout(Duration::from_secs(5), self.ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => Some(serde_json::from_str(&t).unwrap()),
            _ => None,
        }
    }
    async fn next_of(&mut self, kind: &str) -> Value {
        loop {
            let m = self.next().await.unwrap_or_else(|| panic!("no {kind}"));
            if m["type"] == json!(kind) || m["type"] == json!("error") {
                return m;
            }
            self.seen.push(m);
        }
    }
    /// Read until an event of `kind` arrives (keeping what passed by).
    async fn event(&mut self, kind: &str) -> Value {
        loop {
            let m = self.next().await.unwrap_or_else(|| panic!("no event {kind}"));
            self.seen.push(m.clone());
            if m["type"] == json!("event") && m["event"]["type"] == json!(kind) {
                return m["event"].clone();
            }
        }
    }
    /// Everything received within `ms`.
    async fn drain(&mut self, ms: u64) {
        let until = tokio::time::Instant::now() + Duration::from_millis(ms);
        while let Ok(Some(Ok(Message::Text(t)))) = tokio::time::timeout_at(until, self.ws.next()).await {
            self.seen.push(serde_json::from_str(&t).unwrap());
        }
    }
    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = format!("c{}", rand_id());
        self.send(json!({ "type": "call", "id": id, "method": method, "params": params })).await;
        loop {
            let m = self.next().await.expect("no result");
            if m["type"] == json!("result") && m["id"] == json!(id) {
                return m;
            }
            self.seen.push(m);
        }
    }
}

fn rand_id() -> u64 {
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::SeqCst)
}

#[tokio::test]
async fn a_room_from_a_workspace_runs_over_the_channel_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path()).await;
    let src = srv.courseware().await;
    let created = srv.ok("tok-alice", "session.create", json!({ "workspace_id": src, "path_id": "path-intro", "title": "第一讲" })).await;
    let sid = created["session"]["session_id"].as_str().unwrap().to_string();
    let run = created["session"]["workspace_id"].as_str().unwrap().to_string();
    assert_ne!(run, src);
    // the room's links: audience, display, prompter — rights of the session, not of the Workspace (Q14)
    let at = srv.ok("tok-alice", "session.invite", json!({ "session_id": sid, "kind": "at" })).await["invite"]["token"].as_str().unwrap().to_string();
    let dt = srv.ok("tok-alice", "session.invite", json!({ "session_id": sid, "kind": "dt", "config": { "voice": false } })).await["invite"]["token"].as_str().unwrap().to_string();
    let pt = srv.ok("tok-alice", "session.invite", json!({ "session_id": sid, "kind": "pt" })).await["invite"]["token"].as_str().unwrap().to_string();
    // bob had no grant on the source Workspace and still has none: the link is for the session only
    assert!(srv.rpc("tok-bob", "ws.get_info", json!({ "workspace_id": src })).await["ok"] == json!(false));
    let mut stage = srv.channel(&sid, Some("tok-alice"), None, "presenter", "laptop").await;
    assert_eq!(stage.seen[0]["role"], "presenter");
    assert_eq!(stage.seen[0]["controller"], "laptop");
    let mut bob = srv.channel(&sid, Some("tok-bob"), Some(&at), "audience", "bob-phone").await;
    assert_eq!(bob.seen[0]["role"], "audience");
    let mut guest = srv.channel(&sid, Some(GUEST), Some(&at), "audience", "guest").await;
    assert_eq!(guest.seen[0]["level"], "guest");
    let mut display = srv.channel(&sid, None, Some(&dt), "display", "wall").await;
    assert_eq!(display.seen[0]["role"], "display");
    // the presenter sees the audience arrive; the audience does not
    stage.event("presence.join").await;
    // start, then step: confirmed by the service, delivered to everyone in the room
    let r = stage.call("session.start", json!({})).await;
    assert_eq!(r["result"]["state"], "live", "{r}");
    let r = stage.call("session.command", json!({ "command": { "command_id": "n1", "kind": "next" } })).await;
    assert_eq!(r["result"]["stage"]["index"], 1, "{r}");
    for ch in [&mut bob, &mut guest, &mut display] {
        let ev = ch.event("stage").await;
        assert_eq!(ev["data"]["stage"]["index"], 1);
    }
    // a second presenter terminal observes until it takes over (Q05); the prompter link always commands
    let mut tablet = srv.channel(&sid, Some("tok-alice"), None, "presenter", "tablet").await;
    let r = tablet.call("session.command", json!({ "command": { "command_id": "t1", "kind": "next" } })).await;
    assert_eq!(r["error"]["sub_code"], "NOT_CONTROLLER", "{r}");
    let r = tablet.call("session.takeover", json!({})).await;
    assert_eq!(r["result"]["controller"], "tablet");
    let r = tablet.call("session.command", json!({ "command": { "command_id": "t2", "kind": "next" } })).await;
    assert_eq!(r["result"]["stage"]["index"], 2, "{r}");
    let r = stage.call("session.command", json!({ "command": { "command_id": "n2", "kind": "next" } })).await;
    assert_eq!(r["error"]["sub_code"], "NOT_CONTROLLER");
    let r = srv.raw(None, "session.command", json!({ "session_id": sid, "link": pt, "command": { "command_id": "p1", "kind": "prev" } })).await.unwrap();
    assert_eq!(r["stage"]["index"], 1, "{r}");
    // the prompter reads the notes; the display link reads the run Workspace's public content under its own subject
    let notes = srv.raw(Some(&pt), "session.notes", json!({ "session_id": sid })).await.unwrap();
    assert!(notes["notes"]["frame1"].as_str().unwrap().contains("开场"), "{notes}");
    let frame = srv.raw(Some(&dt), "doc.read", json!({ "workspace_id": run, "entity_id": "frame1" })).await.unwrap();
    assert_eq!(frame["ok"], true, "{frame}");
    assert!(frame.to_string().find("开场").is_none());
    assert_eq!(srv.raw(Some(&dt), "session.notes", json!({ "session_id": sid })).await.unwrap()["ok"], false);
    assert_eq!(srv.raw(Some(&dt), "doc.read", json!({ "workspace_id": src, "entity_id": "frame1" })).await.unwrap()["ok"], false);

    // a claim task with one slot: of five concurrent claimants exactly one gets it (P4-04)
    for who in ["carol", "dave", "erin", "frank"] {
        srv.ok(&format!("tok-{who}"), "session.join", json!({ "session_id": sid, "link": at, "name": who })).await;
    }
    let t = tablet.call("session.task", json!({ "op": "create", "command_id": "k1", "task": { "title": "抢答", "mode": "claim", "slots": 1 } })).await;
    let task = t["result"]["task_id"].as_str().unwrap().to_string();
    let mut claims = Vec::new();
    for who in ["bob", "carol", "dave", "erin", "frank"] {
        let (http, base, task) = (srv.http.clone(), srv.base.clone(), task.clone());
        let sid = sid.clone();
        claims.push(tokio::spawn(async move {
            let v: Value = http.post(&base).json(&json!({ "method": "session.claim", "params": { "session_id": sid, "task_id": task }, "sys": [1, format!("tok-{who}")] }))
                .send().await.unwrap().json().await.unwrap();
            (who, v["result"].clone())
        }));
    }
    let mut won = Vec::new();
    for c in claims {
        let (who, r) = c.await.unwrap();
        if r["ok"] == json!(true) {
            won.push(who);
        } else {
            assert_eq!(r["error"]["sub_code"], "CLAIM_FULL", "{r}");
        }
    }
    assert_eq!(won.len(), 1, "{won:?}");
    // the winner answers; the guest of a room without guest interaction cannot (Q03)
    let winner = won[0];
    let r = srv.rpc(&format!("tok-{winner}"), "session.respond", json!({ "session_id": sid, "task_id": task, "content": { "choice": "B" }, "command_id": "a1" })).await;
    assert_eq!(r["ok"], true, "{r}");
    let r = guest.call("session.react", json!({ "kind": "danmaku", "text": "hi" })).await;
    assert_eq!(r["error"]["sub_code"], "GUEST_NOT_ALLOWED");
    // nobody else's channel carries the private answer
    bob.drain(500).await;
    let leaked = |ch: &Channel| ch.seen.iter().any(|m| m.to_string().contains("\"choice\""));
    if winner != "bob" {
        assert!(!leaked(&bob));
    }
    assert!(!leaked(&guest));
    display.drain(200).await;
    assert!(!leaked(&display));
    // the long-poll fallback (Q04) carries the same timeline
    let w = srv.rpc("tok-bob", "session.watch", json!({ "session_id": sid, "after": 0, "timeout_ms": 1000 })).await;
    assert!(w["events"].as_array().unwrap().iter().any(|e| e["type"] == json!("stage")), "{w}");
    drop((stage, bob, guest, display, tablet));

    // a restart (P4-13): the session and its stage are on disk, the audience reconnects and continues
    let (port, path) = (srv.port, dir.path().to_path_buf());
    drop(srv);
    let _ = port;
    let srv = Server::start(&path).await;
    let got = srv.ok("tok-bob", "session.get", json!({ "session_id": sid })).await;
    assert_eq!(got["state"], "live");
    assert_eq!(got["stage"]["index"], 1);
    let mut bob = srv.channel(&sid, Some("tok-bob"), None, "audience", "bob-phone").await;
    let mut stage = srv.channel(&sid, Some("tok-alice"), None, "presenter", "tablet").await;
    let r = stage.call("session.command", json!({ "command": { "command_id": "n9", "kind": "first" } })).await;
    assert_eq!(r["result"]["stage"]["index"], 0, "{r}");
    // the reconnection replays the timeline first (hello `after: 0`), then the new step
    loop {
        let ev = bob.event("stage").await;
        if ev["command_id"] == json!("n9") {
            assert_eq!(ev["data"]["stage"]["index"], 0);
            break;
        }
    }
    // end without saving, then the owner saves (P4-11 shape: separate steps)
    let r = stage.call("session.end", json!({})).await;
    assert_eq!(r["result"]["state"], "ended");
    let r = stage.call("session.save", json!({})).await;
    assert_eq!(r["result"]["state"], "archived", "{r}");
    // the replay baseline a participant reads has no notes
    let replay = srv.ok("tok-bob", "session.replay", json!({ "session_id": sid })).await;
    assert!(!replay["baseline"].to_string().contains("开场"));
    assert!(replay["baseline"]["entities"].as_array().unwrap().len() > 5);
    // opening the replay is not recorded: no one comes or goes after the end
    drop((bob, stage));
    let mut watcher = srv.channel(&sid, Some("tok-bob"), None, "audience", "bob-replay").await;
    watcher.drain(200).await;
    drop(watcher);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let events = srv.ok("tok-alice", "session.events", json!({ "session_id": sid, "after": 0 })).await;
    let events = events["events"].as_array().unwrap();
    let end = events.iter().position(|e| e["type"] == json!("session.end")).unwrap();
    assert!(!events[end..].iter().any(|e| e["type"].as_str().unwrap().starts_with("presence.")), "{events:?}");
    let events = srv.ok("tok-bob", "session.events", json!({ "session_id": sid, "after": 0 })).await;
    assert!(events["events"].as_array().unwrap().iter().filter(|e| e["type"] == json!("stage")).count() >= 3);
    let posted = srv.ok("tok-bob", "session.post", json!({ "session_id": sid, "kind": "moment", "body": "这里没听懂", "t_rel_ms": 1200 })).await;
    assert!(posted["post_id"].is_string());
    let rated = srv.ok("tok-bob", "session.post", json!({ "session_id": sid, "kind": "rating", "score": 5 })).await;
    assert!(rated["post_id"].is_string());
    let carol_view = srv.ok("tok-carol", "session.list_posts", json!({ "session_id": sid })).await;
    assert_eq!(carol_view["posts"].as_array().unwrap().len(), 1, "ratings are the presenter's: {carol_view}");
    let alice_view = srv.ok("tok-alice", "session.list_posts", json!({ "session_id": sid })).await;
    assert_eq!(alice_view["posts"].as_array().unwrap().len(), 2);
    // the sessions of the courseware (历次演示)
    let list = srv.ok("tok-alice", "session.list", json!({ "workspace_id": src })).await;
    assert_eq!(list["sessions"].as_array().unwrap().len(), 1);
    // my sessions across Workspaces (我的公开课): the owner presenting, a participant; someone who never joined has none
    let mine = srv.ok("tok-alice", "session.list_mine", json!({})).await;
    let mine = mine["sessions"].as_array().unwrap();
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(mine[0]["session_id"], json!(sid));
    assert_eq!(mine[0]["my_role"], "presenter");
    let host = format!("http://aiws-{}.localhost:4120/", src.strip_prefix("ws_").unwrap());
    let url = mine[0]["url"].as_str().unwrap();
    assert!(url.starts_with(&host) && url.ends_with(&format!("?session={sid}")), "{url}");
    let bobs = srv.ok("tok-bob", "session.list_mine", json!({})).await;
    assert_eq!(bobs["sessions"][0]["my_role"], "audience", "{bobs}");
    assert!(bobs["sessions"][0].get("baseline").is_none(), "{bobs}");
    let other = srv.ok("tok-carol", "session.list_mine", json!({})).await;
    assert_eq!(other["sessions"].as_array().unwrap().len(), 1, "carol joined by link: {other}");
    let r = srv.rpc("tok-erin", "session.list_mine", json!({ "limit": 0 })).await;
    assert_eq!(r["ok"], true, "{r}");
}
