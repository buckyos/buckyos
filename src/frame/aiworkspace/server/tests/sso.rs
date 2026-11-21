//! Hybrid SSO against the real process (standalone identities from tokens.json): restricted classes (a Limited
//! account, an External DID) are never in a `*` group and own nothing, `group:` / `contact:` members of logical
//! groups, share entries leading into a group (`entry.describe` without a session, guests, `ws.join_entry` with method
//! and Zone checks), and the D-4 denial on the wire.

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
    async fn start(dir: &std::path::Path) -> Server {
        let auth = dir.join("tokens.json");
        let tokens = json!({
            "tokens": {
                "tok-alice": { "principal": "alice", "did": "did:bns:alice", "auth_method": "password" },
                "tok-bob": { "principal": "bob", "auth_method": "password" },
                "tok-lim": { "principal": "acme-c1", "restricted": true, "groups": ["customers"], "auth_method": "exclusive_invite" },
                "tok-lim2": { "principal": "acme-c2", "restricted": true, "auth_method": "password", "contacts": ["contact:did:bns:alice#vip"] },
                "tok-flaky": { "principal": "acme-c3", "restricted": true, "auth_method": "password", "contacts_unavailable": true },
                "tok-ext": { "principal": "did:bns:carol", "class": "external", "auth_method": "did_federation" },
                "tok-app": { "principal": "some-app", "class": "service" }
            },
            "sso": { "self_register_enabled": true, "did_login_enabled": false }
        });
        std::fs::write(&auth, tokens.to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--public-base").arg("https://aiws-{workspace_id}.zone.invalid")
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

    async fn raw(&self, token: Option<&str>, method: &str, params: Value) -> Value {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let sys = match token {
            Some(t) => json!([seq, t]),
            None => json!([seq]),
        };
        self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": sys })).send().await.unwrap().json().await.unwrap()
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        let v = self.raw(Some(token), method, params).await;
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }
}

fn code(v: &Value) -> &str {
    v["error"]["code"].as_str().unwrap_or("")
}

fn reason(v: &Value) -> &str {
    v["error"]["denial"]["reason"].as_str().unwrap_or("")
}

/// A refusal on the shared kRPC wire (`ws.join_entry`): `{ error, code: access_denied, data: AccessDenial }`.
fn krpc_denial(v: &Value) -> buckyos_api::AccessDenial {
    assert_eq!(v["code"], "access_denied", "{v}");
    assert!(v.get("result").is_none(), "{v}");
    serde_json::from_value(v["data"].clone()).unwrap()
}

#[tokio::test]
async fn restricted_classes_entries_and_denials() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path()).await;
    let ws = srv.rpc("tok-alice", "ws.create", json!({ "title": "DJ 电台" })).await;
    let id = ws["workspace_id"].as_str().unwrap().to_string();
    let wsp = json!({ "workspace_id": id });

    // PRE-03: restricted classes own nothing; the refusal says why (a product boundary, not a missing grant)
    for tok in ["tok-lim", "tok-ext"] {
        for (method, params) in [("ws.create", json!({ "title": "x" })), ("ws.list_subjects", json!({})), ("ws.import", json!({ "bucket_id": "user/aiworkspace/acme-c1/exports", "key": "import/x.bcanvas", "semantics": "new" })), ("ws.clone", wsp.clone())] {
            let r = srv.rpc(tok, method, params).await;
            assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "session_kind_not_allowed"), "{tok} {method}: {r}");
            assert_eq!(r["error"]["denial"]["adjust"]["party"], "product");
            assert_eq!((r["error"]["denial"]["service"].as_str(), r["error"]["denial"]["method"].as_str()), (Some("aiworkspace"), Some(method)));
        }
    }
    assert_eq!(srv.rpc("tok-app", "ws.create", json!({ "title": "a service may" })).await["ok"], json!(true));

    // a group of every ordinary account (`*`) leaves restricted callers out; for them "missing" and "not shared" give one
    // neutral answer
    srv.rpc("tok-alice", "acl.set_group", json!({ "workspace_id": id, "group_id": "zone" })).await;
    srv.rpc("tok-alice", "acl.members", json!({ "workspace_id": id, "group_id": "zone", "add": ["*"] })).await;
    srv.rpc("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "@zone", "allow": ["read"] }] })).await;
    assert_eq!(srv.rpc("tok-bob", "ws.get_info", wsp.clone()).await["capabilities"], json!(["read"]));
    for tok in ["tok-lim", "tok-ext", "tok-app", GUEST] {
        let r = srv.rpc(tok, "ws.get_info", wsp.clone()).await;
        assert_eq!((code(&r), reason(&r)), ("NOT_FOUND", "resource_grant_missing"), "{tok}: {r}");
    }
    let missing = srv.rpc("tok-lim", "ws.get_info", json!({ "workspace_id": "ws_aaaaaaaaaaaaaaaaaaaaaaaaaa" })).await;
    let hidden = srv.rpc("tok-lim", "ws.get_info", wsp.clone()).await;
    assert_eq!(missing["error"], hidden["error"], "a missing workspace and one not shared look the same");
    assert_eq!(hidden["error"]["denial"]["adjust"], json!({ "party": "sharer", "location": "share_panel" }));
    // an ordinary account learns the same category; its detail is the service's own
    let other = srv.rpc("tok-bob", "ws.get_info", json!({ "workspace_id": "ws_aaaaaaaaaaaaaaaaaaaaaaaaaa" })).await;
    assert_eq!((code(&other), reason(&other)), ("NOT_FOUND", "resource_grant_missing"));
    // a reader that may not manage: PERMISSION_DENIED, same reason
    let r = srv.rpc("tok-bob", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "bob", "allow": ["manage"] }] })).await;
    assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "resource_grant_missing"));

    // system groups and contact groups are members of logical groups (权限需求 §4.2)
    srv.rpc("tok-alice", "acl.set_group", json!({ "workspace_id": id, "group_id": "customers" })).await;
    srv.rpc("tok-alice", "acl.members", json!({ "workspace_id": id, "group_id": "customers", "add": ["group:customers"] })).await;
    srv.rpc("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "@customers", "allow": ["read"] }] })).await;
    srv.rpc("tok-alice", "acl.set_group", json!({ "workspace_id": id, "group_id": "vip" })).await;
    srv.rpc("tok-alice", "acl.members", json!({ "workspace_id": id, "group_id": "vip", "add": ["contact:did:bns:alice#vip"] })).await;
    srv.rpc("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "@vip", "allow": ["read", "comment"] }] })).await;
    // someone else's contact group cannot be named (it would let a manager probe it)
    let r = srv.rpc("tok-alice", "acl.members", json!({ "workspace_id": id, "group_id": "vip", "add": ["contact:did:bns:bob#friends"] })).await;
    assert_eq!(code(&r), "INVALID_OPERATION", "{r}");
    assert_eq!(srv.rpc("tok-lim", "ws.get_info", wsp.clone()).await["capabilities"], json!(["read"]));
    assert_eq!(srv.rpc("tok-lim2", "ws.get_info", wsp.clone()).await["capabilities"], json!(["read", "comment"]));
    let r = srv.rpc("tok-flaky", "ws.get_info", wsp.clone()).await;
    assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "authorization_unavailable"), "{r}");
    let listed = |v: Value| v["workspaces"].as_array().unwrap().iter().map(|w| w["workspace_id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(listed(srv.rpc("tok-lim", "ws.list", json!({})).await), vec![id.clone()]);
    assert_eq!(listed(srv.rpc("tok-ext", "ws.list", json!({})).await), Vec::<String>::new());

    // a share entry leads into a group (R8): methods as created, public description without a session
    srv.rpc("tok-alice", "acl.set_group", json!({ "workspace_id": id, "group_id": "visitors" })).await;
    srv.rpc("tok-alice", "acl.set", json!({ "workspace_id": id, "changes": [{ "subject": "@visitors", "allow": ["read", "comment", "append"] }] })).await;
    let r = srv.rpc("tok-bob", "ws.create_entry", json!({ "workspace_id": id, "group_id": "visitors", "methods": { "guest": true } })).await;
    assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "resource_grant_missing"), "manage is needed");
    let legacy = srv.rpc("tok-alice", "ws.create_entry", json!({ "workspace_id": id, "group_id": "visitors", "methods": { "did_login": true } })).await;
    assert_eq!(code(&legacy), "INVALID_OPERATION");
    let created = srv.rpc("tok-alice", "ws.create_entry", json!({ "workspace_id": id, "group_id": "visitors", "methods": { "guest": true } })).await;
    let entry_id = created["entry"]["entry_id"].as_str().unwrap();
    let described = srv.raw(None, "entry.describe", json!({ "entry_id": entry_id })).await["result"].clone();
    assert_eq!(described["state"], "active");
    for token in ["tok-bob","tok-lim","tok-ext"] {
        let failure=srv.raw(Some(token),"ws.join_entry",json!({"entry_id":entry_id})).await;
        assert_eq!(krpc_denial(&failure).reason,buckyos_api::DenyReason::EntryUnavailable);
    }
    let joined=srv.rpc(GUEST,"ws.join_entry",json!({"entry_id":entry_id})).await;
    assert_eq!(joined["joined"],true);
    assert_eq!(srv.rpc(GUEST,"ws.get_info",wsp.clone()).await["capabilities"],json!(["read","append","comment"]));
    let closed=srv.rpc("tok-alice","ws.close_entry",json!({"workspace_id":id,"entry_id":entry_id,"remove_members":true})).await;
    assert_eq!(closed["removed_members"],1);
    assert_eq!(srv.raw(None,"entry.describe",json!({"entry_id":entry_id})).await["result"]["state"],"unavailable");
    let unavailable = srv.raw(Some(GUEST),"ws.join_entry",json!({"entry_id":entry_id})).await;
    assert_eq!(krpc_denial(&unavailable).reason,buckyos_api::DenyReason::EntryUnavailable);
    let bad_callback=srv.raw(None,"share.callback",json!({"operation":"commit","resource_id":entry_id,"callback_secret":"invalid","did":"did:bns:carol"})).await;
    assert_eq!(bad_callback["result"]["ok"],false);

    // the HTTP routes answer with the same denial
    let asset = srv.http.get(format!("{}/asset/{}/{}", srv.base, id, "x")).bearer_auth("tok-ext").send().await.unwrap();
    assert_eq!(asset.status(), 404);
    let body: Value = asset.json().await.unwrap();
    assert_eq!(body["error"]["denial"]["reason"], "resource_grant_missing", "{body}");
}
