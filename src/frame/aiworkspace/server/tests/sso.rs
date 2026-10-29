//! Hybrid SSO against the real process (standalone identities from tokens.json): restricted classes (a Limited
//! account, an External DID) never match `*` and own nothing, `group:` / `contact:` grants, share entries
//! (`entry.describe` without a session, guests, `ws.join_entry` with method and Zone checks), and the D-4 denial on
//! the wire.

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
        for (method, params) in [("ws.create", json!({ "title": "x" })), ("ws.list_subjects", json!({})), ("ws.import", json!({ "bucket_id": "user/aiworkspace/acme-c1/exports", "key": "import/x.bcanvas", "semantics": "new" })), ("ws.fork", wsp.clone())] {
            let r = srv.rpc(tok, method, params).await;
            assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "session_kind_not_allowed"), "{tok} {method}: {r}");
            assert_eq!(r["error"]["denial"]["adjust"]["party"], "product");
            assert_eq!((r["error"]["denial"]["service"].as_str(), r["error"]["denial"]["method"].as_str()), (Some("aiworkspace"), Some(method)));
        }
    }
    assert_eq!(srv.rpc("tok-app", "ws.create", json!({ "title": "a service may" })).await["ok"], json!(true));

    // `*` reaches ordinary accounts only; for a restricted caller "missing" and "not shared" give one neutral answer
    srv.rpc("tok-alice", "ws.grant", json!({ "workspace_id": id, "subject": "*", "capabilities": ["read"] })).await;
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
    let r = srv.rpc("tok-bob", "ws.grant", json!({ "workspace_id": id, "subject": "bob", "capabilities": ["manage"] })).await;
    assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "resource_grant_missing"));

    // system groups and contact groups
    srv.rpc("tok-alice", "ws.grant", json!({ "workspace_id": id, "subject": "group:customers", "capabilities": ["read"] })).await;
    srv.rpc("tok-alice", "ws.grant", json!({ "workspace_id": id, "subject": "contact:did:bns:alice#vip", "capabilities": ["read", "comment"] })).await;
    // someone else's contact group cannot be named (it would let a manager probe it)
    let r = srv.rpc("tok-alice", "ws.grant", json!({ "workspace_id": id, "subject": "contact:did:bns:bob#friends", "capabilities": ["read"] })).await;
    assert_eq!(code(&r), "INVALID_OPERATION", "{r}");
    assert_eq!(srv.rpc("tok-lim", "ws.get_info", wsp.clone()).await["capabilities"], json!(["read"]));
    assert_eq!(srv.rpc("tok-lim2", "ws.get_info", wsp.clone()).await["capabilities"], json!(["read", "comment"]));
    let r = srv.rpc("tok-flaky", "ws.get_info", wsp.clone()).await;
    assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "authorization_unavailable"), "{r}");
    let listed = |v: Value| v["workspaces"].as_array().unwrap().iter().map(|w| w["workspace_id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(listed(srv.rpc("tok-lim", "ws.list", json!({})).await), vec![id.clone()]);
    assert_eq!(listed(srv.rpc("tok-ext", "ws.list", json!({})).await), Vec::<String>::new());

    // a share entry: methods as created, public description without a session
    let r = srv.rpc("tok-bob", "ws.create_entry", json!({ "workspace_id": id, "capabilities": ["read"], "methods": { "guest": true } })).await;
    assert_eq!((code(&r), reason(&r)), ("PERMISSION_DENIED", "resource_grant_missing"), "manage is needed");
    let created = srv.rpc("tok-alice", "ws.create_entry", json!({ "workspace_id": id, "capabilities": ["read", "comment", "append"],
                                                                   "methods": { "guest": true, "password": true, "did_login": true } })).await;
    let entry_id = created["entry"]["entry_id"].as_str().unwrap().to_string();
    assert!(entry_id.starts_with(&format!("{id}~se_")), "{created}");
    let described = srv.raw(None, "entry.describe", json!({ "entry_id": entry_id })).await["result"].clone();
    assert_eq!(described["state"], "active", "{described}");
    // the Workspace's page on its own origin (T4-8): `https://aiws-<id without ws_>.<zone>/<title slug>`
    let page = format!("https://aiws-{}.zone.invalid/dj", id.strip_prefix("ws_").unwrap());
    assert_eq!(described["resource"], json!({ "kind": "workspace", "title": "DJ 电台", "owner_name": "alice", "path": page }));
    assert_eq!(described["methods"], json!({ "guest": true, "password": true, "self_register": false, "did_login": true }));
    let parsed: buckyos_api::EntryDescription = serde_json::from_value(described.clone()).unwrap();
    assert_eq!(parsed.state, buckyos_api::EntryState::Active);
    for bad in ["nope", "ws_aaaaaaaaaaaaaaaaaaaaaaaaaa~se_aaaaaaaaaaaaaaaaaaaaaaaaaa"] {
        assert_eq!(srv.raw(None, "entry.describe", json!({ "entry_id": bad })).await["result"], json!({ "ok": true, "state": "unavailable", "methods": { "guest": false, "password": false, "self_register": false, "did_login": false } }));
    }
    assert_eq!(srv.rpc("tok-alice", "ws.list_entries", wsp.clone()).await["entries"].as_array().unwrap().len(), 1);

    // an entry open to guests lets anyone in at once — a guest, an External (the `*` grant does not apply to them)
    for tok in [GUEST, "tok-ext"] {
        assert_eq!(srv.rpc(tok, "ws.get_info", wsp.clone()).await["capabilities"], json!(["read", "append", "comment"]), "{tok}");
    }
    // a signed-in visitor joins: a local account by password (exclusive invite sessions count as local) gets a grant;
    // the DID way is turned off by the Zone, so the External uses the entry as a guest would (no grant); a guest
    // does not join
    let joined = srv.rpc("tok-lim", "ws.join_entry", json!({ "entry_id": entry_id })).await;
    assert_eq!((joined["capabilities"].clone(), joined["path"].clone(), joined["granted"].clone()), (json!(["read", "append", "comment"]), json!(page), json!(true)), "{joined}");
    assert_eq!(srv.rpc("tok-lim", "ws.get_info", wsp.clone()).await["capabilities"], json!(["read", "append", "comment"]));
    let as_guest = srv.rpc("tok-ext", "ws.join_entry", json!({ "entry_id": entry_id })).await;
    assert_eq!((as_guest["granted"].clone(), as_guest["capabilities"].clone()), (json!(false), json!(["read", "append", "comment"])), "{as_guest}");
    use buckyos_api::{AdjustLocation, AdjustParty, DenyReason};
    assert_eq!(krpc_denial(&srv.raw(Some(GUEST), "ws.join_entry", json!({ "entry_id": entry_id })).await).reason, DenyReason::SessionKindNotAllowed);
    let did_only = srv.rpc("tok-alice", "ws.create_entry", json!({ "workspace_id": id, "capabilities": ["read"], "methods": { "did_login": true } })).await;
    let d = krpc_denial(&srv.raw(Some("tok-ext"), "ws.join_entry", json!({ "entry_id": did_only["entry"]["entry_id"] })).await);
    assert_eq!((d.reason, d.adjust.party, d.adjust.location), (DenyReason::ZoneMethodDisabled, AdjustParty::Admin, Some(AdjustLocation::CpSsoSettings)));
    assert_eq!(d.method.as_deref(), Some("ws.join_entry"));
    let password_only = srv.rpc("tok-alice", "ws.create_entry", json!({ "workspace_id": id, "capabilities": ["read"], "methods": { "password": true } })).await;
    let d = krpc_denial(&srv.raw(Some("tok-ext"), "ws.join_entry", json!({ "entry_id": password_only["entry"]["entry_id"] })).await);
    assert_eq!(d.reason, DenyReason::EntryUnavailable);
    assert_eq!(krpc_denial(&srv.raw(Some("tok-lim"), "ws.join_entry", json!({ "entry_id": "garbage" })).await).reason, DenyReason::EntryUnavailable);
    let grants = srv.rpc("tok-alice", "ws.list_grants", wsp.clone()).await;
    let row = grants["grants"].as_array().unwrap().iter().find(|g| g["subject"] == "acme-c1").cloned().unwrap();
    assert_eq!(row["via_entry"], json!(entry_id));

    // closing: the description becomes unavailable, guests are out, joins stop, joined grants stay
    let r = srv.rpc("tok-alice", "ws.close_entry", json!({ "workspace_id": id, "entry_id": entry_id })).await;
    assert_eq!(r["entry"]["state"], "closed");
    assert_eq!(srv.raw(None, "entry.describe", json!({ "entry_id": entry_id })).await["result"]["state"], "unavailable");
    for tok in [GUEST, "tok-ext"] {
        assert_eq!(reason(&srv.rpc(tok, "ws.get_info", wsp.clone()).await), "resource_grant_missing", "{tok}");
    }
    assert_eq!(krpc_denial(&srv.raw(Some("tok-lim2"), "ws.join_entry", json!({ "entry_id": entry_id })).await).reason, DenyReason::EntryUnavailable);
    assert_eq!(srv.rpc("tok-lim", "ws.get_info", wsp.clone()).await["capabilities"], json!(["read", "append", "comment"]));
    let r = srv.rpc("tok-alice", "ws.close_entry", json!({ "workspace_id": "ws_aaaaaaaaaaaaaaaaaaaaaaaaaa", "entry_id": entry_id })).await;
    assert_eq!(code(&r), "NOT_FOUND", "the workspace named beside the entry must be its own");

    // the HTTP routes answer with the same denial
    let asset = srv.http.get(format!("{}/asset/{}/{}", srv.base, id, "x")).bearer_auth("tok-ext").send().await.unwrap();
    assert_eq!(asset.status(), 404);
    let body: Value = asset.json().await.unwrap();
    assert_eq!(body["error"]["denial"]["reason"], "resource_grant_missing", "{body}");
}
