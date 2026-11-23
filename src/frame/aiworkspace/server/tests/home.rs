//! The UI framework's service interfaces over HTTP (UI 框架 PRD §4.6, §9; K4, K6, K7): personal preferences shared by
//! every Workspace and kept apart per person, the Workspace's shared settings in `ws.get_info`, the open record behind
//! `ws.list.opened_at`, `session.list_open` with a courseware reader who never joined opening the room without a link,
//! and the cover its managers choose (Q37) with its bytes on the asset route for whoever reads the settings.

#[path = "common/nfs.rs"]
mod nfs;

use nfs::Nfs;
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
    async fn start(dir: &std::path::Path, nfs: &Nfs) -> Server {
        let auth = dir.join("tokens.json");
        std::fs::write(&auth, json!({ "tokens": {
            "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" },
            "tok-a1": { "principal": "did:agent:one", "class": "agent", "owner": "alice", "app_id": "one" },
            "tok-a2": { "principal": "did:agent:two", "class": "agent", "owner": "alice", "app_id": "two" },
            "tok-reader": { "principal": "did:agent:reader", "class": "agent", "owner": "alice", "agent_read_only": true }
        } }).to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--public-base").arg("http://aiws-{workspace_id}.localhost:4120")
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
    let nfs = Nfs::start(dir.path()).await;
    let srv = Server::start(dir.path(), &nfs).await;
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
    // the permission object (Q19): the presenter sees it named and may choose another over kRPC; the audience is not told
    let own = srv.ok("tok-alice", "session.get", json!({ "session_id": sid })).await;
    assert_eq!((&own["readers_of"]["entity_id"], &own["readers_of"]["path"]), (&json!("s-launch"), &json!("/surfaces/发布会")), "{own}");
    let set = srv.ok("tok-alice", "session.set_access", json!({ "session_id": sid, "readers_of": "/surfaces/总结" })).await;
    assert_eq!(set["readers_of"]["entity_id"], "s-work", "{set}");
    assert!(srv.ok("tok-bob", "session.get", json!({ "session_id": sid })).await.get("readers_of").is_none());
}

/// Q37 over HTTP: no cover until a manager chooses one in the settings (a picture on a canvas is none); a reader of one
/// canvas lists it and fetches its bytes from the asset route, so does a reader of the courseware's lecture; nobody
/// else gets them; a clone carries the picture.
#[tokio::test(flavor = "multi_thread")]
async fn the_cover_is_a_setting_and_whoever_reads_the_settings_gets_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let nfs = Nfs::start(dir.path()).await;
    let srv = Server::start(dir.path(), &nfs).await;
    let ws = srv.ok("tok-alice", "ws.create", json!({ "title": "有封面" })).await;
    let (id, epoch) = (ws["workspace_id"].as_str().unwrap().to_string(), ws["epoch"].as_str().unwrap().to_string());
    let fx: Value = serde_json::from_str(FIXTURE).unwrap();
    for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
        srv.ok("tok-alice", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": format!("fx/{i}"), "operations": c["operations"] })).await;
    }
    let cover_of = |list: &Value, ws: &str| list["workspaces"].as_array().unwrap().iter().find(|w| w["workspace_id"] == json!(ws)).map(|w| w["cover"].clone());
    assert_eq!(cover_of(&srv.ok("tok-alice", "ws.list", json!({})).await, &id), Some(Value::Null));
    let png = [&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..], b"cover over http"].concat();
    let bound = nfs.put("alice", "assets", "upload/cover.png", png.clone()).await;
    let staged = srv.ok("tok-alice", "asset.import_object", json!({ "workspace_id": id, "bucket_id": bound["bucket_id"], "key": bound["key"] })).await;
    let object = staged["object_id"].as_str().unwrap().to_string();
    let set = |who: &'static str, key: &'static str, cover: Value| {
        let (srv, id, epoch) = (&srv, id.clone(), epoch.clone());
        async move {
            srv.rpc(who, "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": key,
                "operations": [{ "op": "entity.set_props", "entity_id": "root", "key": "config", "props": { "cover": cover } }] })).await
        }
    };
    // bob reads one canvas: no manager, his choice is refused
    srv.ok("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "bob", "resource": "s-launch", "allow": ["read"] }] })).await;
    let refused = set("tok-bob", "bob-cover", json!({ "object_id": object })).await;
    assert_eq!(refused["status"], "rejected", "{refused}");
    let accepted = set("tok-alice", "cover", json!({ "object_id": object })).await;
    assert_eq!(accepted["status"], "accepted", "{accepted}");
    let expected = json!({ "workspace_id": id, "object_id": object, "media_type": "image/png" });
    assert_eq!(cover_of(&srv.ok("tok-bob", "ws.list", json!({})).await, &id), Some(expected.clone()));
    assert_eq!(srv.ok("tok-bob", "ws.get_info", json!({ "workspace_id": id })).await["settings"]["cover"], json!({ "object_id": object, "media_type": "image/png", "size": png.len() }));
    let fetch = |token: &'static str, ws: String| {
        let (http, base, object) = (srv.http.clone(), srv.base.clone(), object.clone());
        async move { http.get(format!("{base}/asset/{ws}/{object}")).bearer_auth(token).send().await.unwrap() }
    };
    let got = fetch("tok-bob", id.clone()).await;
    assert!(got.status().is_success(), "{}", got.status());
    assert_eq!(got.headers()["content-type"], "image/png");
    assert_eq!(got.bytes().await.unwrap().to_vec(), png);
    assert!(!fetch(GUEST, id.clone()).await.status().is_success(), "a guest does not enter this Workspace");
    // the lecture's card: its courseware's cover, for bob who reads its permission object
    let created = srv.ok("tok-alice", "session.create", json!({ "workspace_id": id, "path_id": "path-intro", "title": "第一讲" })).await;
    let sid = created["session"]["session_id"].as_str().unwrap().to_string();
    let lectures = srv.ok("tok-bob", "session.list_open", json!({})).await;
    let lecture = lectures["sessions"].as_array().unwrap().iter().find(|s| s["session_id"] == json!(sid)).cloned().unwrap_or_else(|| panic!("{lectures}"));
    assert_eq!(lecture["cover"], expected);
    // a clone carries it, its bytes readable there
    let cloned = srv.ok("tok-alice", "ws.clone", json!({ "workspace_id": id })).await["workspace_id"].as_str().unwrap().to_string();
    assert_eq!(cover_of(&srv.ok("tok-alice", "ws.list", json!({})).await, &cloned), Some(json!({ "workspace_id": cloned, "object_id": object, "media_type": "image/png" })));
    assert!(fetch("tok-alice", cloned).await.status().is_success());
    // removed: no cover, and bob no longer gets the bytes
    assert_eq!(set("tok-alice", "uncover", Value::Null).await["status"], "accepted");
    assert_eq!(cover_of(&srv.ok("tok-bob", "ws.list", json!({})).await, &id), Some(Value::Null));
    assert!(!fetch("tok-bob", id.clone()).await.status().is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn human_and_two_agent_sessions_share_the_owners_named_workspace_after_restart() {
    for first in ["tok-alice", "tok-a1"] {
        let dir = tempfile::tempdir().unwrap();
        let nfs = Nfs::start(dir.path()).await;
        let srv = Server::start(dir.path(), &nfs).await;
        let params = || json!({ "name": "schedule" });
        assert_eq!(srv.ok(first, "ws.lookup", params()).await["found"], false);
        assert_eq!(srv.rpc("tok-reader", "ws.ensure", params()).await["error"]["code"], "PERMISSION_DENIED");
        assert_eq!(srv.ok("tok-reader", "ws.lookup", params()).await["found"], false);
        let created = srv.ok(first, "ws.ensure", params()).await;
        assert_eq!(created["created"], true);
        let id = &created["workspace_id"];
        let epoch = &created["epoch"];
        let table = json!([{ "op": "entity.create", "entity_id": "calendar", "type_id": "buckyos.table-source", "name": "calendar",
            "parent_id": "data", "order_key": "a", "payload": { "fields": [{ "field_id": "title", "name": "Title", "type": "text" }] } }]);
        srv.ok(first, "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": "init", "operations": table })).await;
        let actors = [("tok-alice", "alice", "r0"), ("tok-a1", "did:agent:one", "r1"), ("tok-a2", "did:agent:two", "r2")];
        assert_eq!(srv.ok("tok-reader", "ws.lookup", params()).await["workspace_id"], *id);
        let denied = srv.rpc("tok-reader", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch,
            "idempotency_key": "denied", "operations": [{ "op": "table.insert_records", "source_id": "calendar", "records": [
                { "record_id": "denied", "values": { "title": "denied" } }] }] })).await;
        assert_eq!(denied["status"], "rejected", "{denied}");
        let server = &srv;
        let requests = actors.iter().map(|&(token, principal, row)| async move {
            assert_eq!(&server.ok(token, "ws.ensure", params()).await["workspace_id"], id);
            let info = server.ok(token, "ws.get_info", json!({ "workspace_id": id })).await;
            assert_eq!(info["principal"], principal, "{info}");
            assert_eq!(info["owned_by_caller_user"], true);
            server.ok(token, "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": row,
                "operations": [{ "op": "table.insert_records", "source_id": "calendar", "records": [{ "record_id": row, "values": { "title": row } }] }] })).await;
        });
        futures_util::future::join_all(requests).await;
        assert_ne!(&srv.ok("tok-bob", "ws.ensure", params()).await["workspace_id"], id);
        drop(srv);
        let restarted = Server::start(dir.path(), &nfs).await;
        for (token, _, _) in actors {
            assert_eq!(&restarted.ok(token, "ws.lookup", params()).await["workspace_id"], id);
            let rows = restarted.ok(token, "doc.query", json!({ "workspace_id": id, "sql": "SELECT _id FROM \"/data/calendar\" ORDER BY _id" })).await;
            assert_eq!(rows["rows"], json!([["r0"], ["r1"], ["r2"]]));
        }
    }
}
