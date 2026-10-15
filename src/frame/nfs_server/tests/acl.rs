//! Authentication + path ACLs over real HTTP: tokens are signed exactly like
//! verify-hub session tokens (test key), the server runs in TestKey mode.

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, ServerConfig};
use nfs_server::state::AppState;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

fn token(user: &str) -> String {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(user.into()),
        appid: None,
        sudo: false,
        extra: Default::default(),
    };
    bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
    bind_token_target(
        &mut token,
        &AuthTarget::system(SystemServiceId::parse(CONTROL_PANEL_SERVICE_NAME).unwrap()),
        TokenUse::Session,
    )
    .unwrap();
    token.generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap()).unwrap()
}

struct Server {
    base: String,
    data: PathBuf,
    _tmp: tempfile::TempDir,
}

struct Client {
    base: String,
    http: reqwest::Client,
    token: Option<String>,
    session: String,
    seq: AtomicU64,
}

async fn start() -> Server {
    start_with(|_| {}).await
}

async fn start_with(tune: impl FnOnce(&mut ServerConfig)) -> Server {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("exports");
    for d in ["home/alice/docs", "home/bob/share/private", "home/bob/share/pub", "srv/library"] {
        std::fs::create_dir_all(data.join(d)).unwrap();
    }
    std::fs::write(data.join("home/alice/docs/a.txt"), b"alice").unwrap();
    std::fs::write(data.join("home/bob/secret.txt"), b"bob secret").unwrap();
    std::fs::write(data.join("home/bob/share/s.txt"), b"shared").unwrap();
    std::fs::write(data.join("home/bob/share/private/p.txt"), b"private").unwrap();
    std::fs::write(data.join("home/bob/share/pub/photo.jpg"), b"jpeg").unwrap();
    let mut config = ServerConfig::new(
        tmp.path().join("fsdb"),
        vec![
            ExportRoot { id: "home".into(), path: data.join("home") },
            ExportRoot { id: "srv".into(), path: data.join("srv") },
        ],
    );
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    tune(&mut config);
    let app = nfs_server::server::build_router(AppState::new(config).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, data, _tmp: tmp }
}

impl Server {
    async fn client(&self, user: Option<&str>) -> Client {
        let mut c = Client {
            base: self.base.clone(),
            http: reqwest::Client::new(),
            token: user.map(token),
            session: String::new(),
            seq: AtomicU64::new(1),
        };
        let hello = c.post("hello", json!({"args": {"versions": ["nfsp/0"]}})).await;
        assert_eq!(hello["ok"], true, "{}", hello);
        c.session = hello["result"]["session"].as_str().unwrap().to_string();
        c
    }
}

impl Client {
    async fn post(&self, method: &str, body: Value) -> Value {
        let mut req = self.http.post(format!("{}/nfs/v1/{}", self.base, method)).json(&body);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        req.send().await.unwrap().json().await.unwrap()
    }

    async fn call(&self, method: &str, mut body: Value) -> Value {
        body["session"] = json!(self.session);
        self.post(method, body).await
    }

    async fn write(&self, method: &str, mut body: Value) -> Value {
        body["seq"] = json!(self.seq.fetch_add(1, Ordering::SeqCst));
        self.call(method, body).await
    }

    async fn ok(&self, method: &str, body: Value) -> Value {
        let v = self.call(method, body).await;
        assert_eq!(v["ok"], true, "{}: {}", method, v);
        v["result"].clone()
    }

    async fn ok_write(&self, method: &str, body: Value) -> Value {
        let v = self.write(method, body).await;
        assert_eq!(v["ok"], true, "{}: {}", method, v);
        v["result"].clone()
    }

    async fn names(&self, path: &str) -> Vec<String> {
        let r = self.ok("list", json!({"at": {"path": path}})).await;
        r["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
    }

    async fn code(&self, method: &str, body: Value) -> String {
        let v = if method == "list" || method == "resolve" || method == "get_acl" || method == "search" {
            self.call(method, body).await
        } else {
            self.write(method, body).await
        };
        assert_eq!(v["ok"], false, "{} unexpectedly ok: {}", method, v);
        v["error"]["code"].as_str().unwrap().to_string()
    }

    async fn set_acl(&self, path: &str, inherit: bool, entries: Value) -> Value {
        self.write("set_acl", json!({"at": {"path": path}, "args": {"inherit": inherit, "entries": entries}})).await
    }

    async fn node_id(&self, path: &str) -> String {
        let r = self.ok("resolve", json!({"at": {"path": path}})).await;
        r["ref"]["node_id"].as_str().unwrap().to_string()
    }
}

async fn read_status(base: &str, node_id: &str, auth: Option<(&str, &str)>) -> u16 {
    let mut req = reqwest::Client::new().get(format!("{}/nfs/v1/read/{}", base, node_id));
    if let Some((kind, value)) = auth {
        req = req.header(kind, value);
    }
    req.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn users_see_only_their_home_and_grants() {
    let s = start().await;
    let owner = s.client(Some("owner")).await;
    let alice = s.client(Some("alice")).await;
    let bob = s.client(Some("bob")).await;

    assert_eq!(owner.names("/").await, vec!["home", "srv"]);
    assert_eq!(owner.names("/home").await, vec!["alice", "bob"]);
    assert_eq!(alice.names("/").await, vec!["home"]);
    assert_eq!(alice.names("/home").await, vec!["alice"]);
    assert_eq!(alice.names("/home/alice/docs").await, vec!["a.txt"]);
    // Forbidden and missing look the same.
    assert_eq!(alice.code("resolve", json!({"at": {"path": "/home/bob/secret.txt"}})).await, "PERMISSION_DENIED");
    assert_eq!(alice.code("resolve", json!({"at": {"path": "/home/bob/nope"}})).await, "PERMISSION_DENIED");
    assert_eq!(alice.code("list", json!({"at": {"path": "/srv"}})).await, "PERMISSION_DENIED");
    let secret = owner.node_id("/home/bob/secret.txt").await;
    let alice_token = format!("Bearer {}", token("alice"));
    assert_eq!(read_status(&s.base, &secret, Some(("Authorization", &alice_token))).await, 403);
    assert_eq!(read_status(&s.base, &secret, None).await, 403);
    let owner_token = format!("Bearer {}", token("owner"));
    assert_eq!(read_status(&s.base, &secret, Some(("Authorization", &owner_token))).await, 200);

    // Bob shares a folder read-only with alice.
    let r = bob.set_acl("/home/bob/share", true, json!([{"principal": "user:alice", "level": "read"}])).await;
    assert_eq!(r["ok"], true, "{}", r);
    assert_eq!(alice.names("/home").await, vec!["alice", "bob"]);
    assert_eq!(alice.names("/home/bob").await, vec!["share"]);
    assert_eq!(alice.names("/home/bob/share").await, vec!["private", "pub", "s.txt"]);
    let listing = alice.ok("list", json!({"at": {"path": "/home/bob"}})).await;
    assert_eq!(listing["container"]["access_level"], "traverse");
    let shared = alice.ok("resolve", json!({"at": {"path": "/home/bob/share/s.txt"}})).await;
    assert_eq!(shared["access_level"], "read");
    assert_eq!(shared["capabilities"]["accepts_content"], false);
    let cookie = format!("buckyos_session_token={}", token("alice"));
    let s_txt = shared["ref"]["node_id"].as_str().unwrap().to_string();
    assert_eq!(read_status(&s.base, &s_txt, Some(("Cookie", &cookie))).await, 200);
    assert_eq!(alice.code("mkdir", json!({"at": {"path": "/home/bob/share"}, "args": {"name": "x"}})).await, "PERMISSION_DENIED");
    assert_eq!(alice.code("set_acl", json!({"at": {"path": "/home/bob/share"}, "args": {"entries": []}})).await, "PERMISSION_DENIED");

    // Read access only shows the caller's own level, not who else has access.
    let acl = alice.ok("get_acl", json!({"at": {"path": "/home/bob/share"}})).await;
    assert_eq!(acl["access_level"], "read");
    assert!(acl.get("entries").is_none());
    let acl = bob.ok("get_acl", json!({"at": {"path": "/home/bob/share/s.txt"}})).await;
    assert_eq!(acl["can_manage"], true);
    assert_eq!(acl["inherited"][0]["principal"], "user:alice");
    assert_eq!(acl["inherited"][0]["from"], "cyfs:///home/bob/share");

    // Upgrade to write; a protected subfolder stops inheritance.
    bob.set_acl("/home/bob/share", true, json!([{"principal": "user:alice", "level": "write"}])).await;
    bob.set_acl("/home/bob/share/private", false, json!([])).await;
    assert_eq!(alice.names("/home/bob/share").await, vec!["pub", "s.txt"]);
    alice.ok_write("mkdir", json!({"at": {"path": "/home/bob/share"}, "args": {"name": "from-alice"}})).await;
    assert!(s.data.join("home/bob/share/from-alice").is_dir());
    // Write inside a share does not allow deleting the share itself, nor a
    // folder that contains a protected subtree.
    assert_eq!(alice.code("delete", json!({"at": {"path": "/home/bob"}, "args": {"name": "share", "recursive": true}})).await, "PERMISSION_DENIED");
    assert_eq!(
        alice.code("mkdir", json!({"at": {"path": "/home/bob/share/private"}, "args": {"name": "y"}})).await,
        "PERMISSION_DENIED"
    );

    // ACL records follow a protocol move.
    bob.ok_write(
        "move",
        json!({"args": {
            "from": {"parent_ref": bob.ok("resolve", json!({"at": {"path": "/home/bob"}})).await["ref"], "name": "share"},
            "to": {"parent_ref": bob.ok("resolve", json!({"at": {"path": "/home/bob"}})).await["ref"], "name": "shared"},
        }}),
    )
    .await;
    assert_eq!(alice.names("/home/bob").await, vec!["shared"]);
    let all = owner.ok("list_acls", json!({})).await;
    let paths: Vec<&str> = all["records"].as_array().unwrap().iter().map(|r| r["path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec!["cyfs:///home/bob/shared", "cyfs:///home/bob/shared/private"]);
    assert_eq!(alice.ok("list_acls", json!({})).await["records"].as_array().unwrap().len(), 0);
    assert_eq!(bob.ok("list_acls", json!({})).await["records"].as_array().unwrap().len(), 2);

    // Search stays inside what the caller can see.
    let hits = alice.ok("search", json!({"args": {"q": "txt"}})).await;
    let mut found: Vec<&str> = hits["hits"].as_array().unwrap().iter().map(|h| h["canonical_path"].as_str().unwrap()).collect();
    found.sort();
    assert_eq!(found, vec!["cyfs:///home/alice/docs/a.txt", "cyfs:///home/bob/shared/s.txt"]);
}

#[tokio::test]
async fn public_paths_need_no_credentials() {
    let s = start().await;
    let bob = s.client(Some("bob")).await;
    let anon = s.client(None).await;
    assert_eq!(anon.code("list", json!({"at": {"path": "/"}})).await, "PERMISSION_DENIED");
    let r = bob.set_acl("/home/bob/share/pub", true, json!([{"principal": "public", "level": "write"}])).await;
    assert_eq!(r["error"]["code"], "INVALID_ARGUMENT");
    bob.set_acl("/home/bob/share/pub", true, json!([{"principal": "public", "level": "read"}])).await;
    assert_eq!(anon.names("/").await, vec!["home"]);
    assert_eq!(anon.names("/home/bob/share/pub").await, vec!["photo.jpg"]);
    let photo = anon.node_id("/home/bob/share/pub/photo.jpg").await;
    assert_eq!(read_status(&s.base, &photo, None).await, 200);
    // A stale cookie falls back to anonymous instead of failing public reads.
    assert_eq!(read_status(&s.base, &photo, Some(("Cookie", "buckyos_session_token=garbage"))).await, 200);
    assert_eq!(anon.code("resolve", json!({"at": {"path": "/home/bob/share/s.txt"}})).await, "PERMISSION_DENIED");
    assert_eq!(anon.code("mkdir", json!({"at": {"path": "/home/bob/share/pub"}, "args": {"name": "x"}})).await, "PERMISSION_DENIED");
    // zone_users covers every signed-in user but not anonymous callers.
    bob.set_acl("/home/bob/share", true, json!([{"principal": "zone_users", "level": "read"}])).await;
    let carol = s.client(Some("carol")).await;
    assert_eq!(carol.names("/home/bob/share").await.len(), 3);
    assert_eq!(anon.names("/home/bob/share").await, vec!["pub"]);
    assert_eq!(anon.code("resolve", json!({"at": {"path": "/home/bob/share/private"}})).await, "PERMISSION_DENIED");
}

#[tokio::test]
async fn sessions_tokens_and_side_channels() {
    let s = start().await;
    let alice = s.client(Some("alice")).await;
    let bob = s.client(Some("bob")).await;

    // A session only serves the principal that said hello.
    let mut stolen = s.client(Some("alice")).await;
    stolen.session = bob.session.clone();
    let v = stolen.call("list", json!({"at": {"path": "/home"}})).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED");
    assert_eq!(v["error"]["reason"], "session_principal_changed");

    // An explicit bad token is an error, not a silent downgrade.
    let mut forged = s.client(None).await;
    forged.token = Some("not-a-token".into());
    let v = forged.call("list", json!({"at": {"path": "/"}})).await;
    assert_eq!(v["error"]["reason"], "unauthenticated");

    // Content dedup cannot reveal what other users store.
    let owner = s.client(Some("owner")).await;
    let secret = std::fs::read(s.data.join("home/bob/secret.txt")).unwrap();
    let hash = format!("sha256:{}", hex_sha256(&secret));
    // Index bob's content by committing it once as bob.
    let parent = bob.ok("resolve", json!({"at": {"path": "/home/bob"}})).await;
    let ow = bob.ok_write("open_write", json!({"args": {"parent_ref": parent["ref"], "name": "copy.txt", "size": secret.len()}})).await;
    let fb = ow["fb_handle"].as_str().unwrap();
    // Another principal may not push chunks into bob's upload.
    let resp = reqwest::Client::new()
        .patch(format!("{}/nfs/v1/uploads/{}", s.base, fb))
        .bearer_auth(token("alice"))
        .header("Upload-Offset", "0")
        .body(secret.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let resp = reqwest::Client::new()
        .patch(format!("{}/nfs/v1/uploads/{}", s.base, fb))
        .bearer_auth(token("bob"))
        .header("Upload-Offset", "0")
        .body(secret.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 204);
    bob.ok_write(
        "commit_file",
        json!({"args": {"parent_ref": parent["ref"], "name": "copy.txt", "fb_handle": fb, "lease_id": ow["lease"]["lease_id"]}}),
    )
    .await;
    let probe = alice.ok("probe", json!({"args": {"digests": [{"hash": hash, "size": secret.len()}]}})).await;
    assert_eq!(probe["missing"].as_array().unwrap().len(), 1);
    let probe = owner.ok("probe", json!({"args": {"digests": [{"hash": hash, "size": secret.len()}]}})).await;
    assert_eq!(probe["missing"].as_array().unwrap().len(), 0);
    let home = alice.ok("resolve", json!({"at": {"path": "/home/alice"}})).await;
    let v = alice.write("commit_file", json!({"args": {"parent_ref": home["ref"], "name": "stolen.txt", "hash": hash}})).await;
    assert_eq!(v["error"]["code"], "NEED_PULL");

    // Collections belong to their creator.
    let c = alice.ok_write("create_collection", json!({"args": {"title": "Mine", "collection_id": "col_alice"}})).await;
    assert_eq!(c["access_level"], "manage");
    let v = bob.call("open_collection", json!({"args": {"collection_id": "col_alice"}})).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED");
    // A reference never leaks a target the reader cannot see.
    let doc = alice.ok("resolve", json!({"at": {"path": "/home/alice/docs/a.txt"}})).await;
    alice
        .ok_write("collection_patch", json!({"args": {"ref": c["ref"], "ops": [{"add_ref": {"target_ref": doc["ref"]}}]}}))
        .await;
    let v = alice.write("collection_patch", json!({"args": {"ref": c["ref"], "ops": [{"add_ref": {"target_ref": owner.ok("resolve", json!({"at": {"path": "/home/bob/secret.txt"}})).await["ref"]}}]}})).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED");
}

fn hex_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

#[tokio::test]
async fn watch_events_respect_acls() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let s = start().await;
    let alice = s.client(Some("alice")).await;
    let bob = s.client(Some("bob")).await;
    let addr = s.base.trim_start_matches("http://").to_string();

    // Unauthenticated or foreign watchers are refused outright.
    let resp = reqwest::Client::new()
        .get(format!("{}/nfs/v1/watch?session={}", s.base, alice.session))
        .bearer_auth(token("bob"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);

    let mut conn = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let req = format!(
        "GET /nfs/v1/watch?session={} HTTP/1.1\r\nHost: t\r\nAccept: text/event-stream\r\nAuthorization: Bearer {}\r\n\r\n",
        alice.session,
        token("alice")
    );
    conn.write_all(req.as_bytes()).await.unwrap();
    let mut seen = String::new();
    let mut read_until = async |needle: &str| {
        let mut chunk = [0u8; 4096];
        while !seen.contains(needle) {
            let n = tokio::time::timeout(std::time::Duration::from_secs(5), conn.read(&mut chunk))
                .await
                .expect("SSE read timed out")
                .unwrap();
            assert!(n > 0, "SSE closed early");
            seen.push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
        seen.clone()
    };
    read_until("event: resync").await;
    bob.ok_write("mkdir", json!({"at": {"path": "/home/bob"}, "args": {"name": "bob-hidden"}})).await;
    alice.ok_write("mkdir", json!({"at": {"path": "/home/alice"}, "args": {"name": "alice-marker"}})).await;
    let text = read_until("alice-marker").await;
    assert!(!text.contains("bob-hidden"), "leaked: {}", text);
}

#[tokio::test]
async fn copy_cannot_read_or_write_outside_grants() {
    // The ACL gate runs before any TaskMgr call, so an unreachable endpoint is fine.
    let s = start_with(|c| {
        c.copy_test = Some(NfsCopyTestConfig {
            task_mgr_url: "http://127.0.0.1:9/kapi/task-manager".into(),
            service_token: String::new(),
            public_key: PUBLIC.into(),
        })
    })
    .await;
    let owner = s.client(Some("owner")).await;
    let alice = s.client(Some("alice")).await;
    let secret = owner.ok("resolve", json!({"at": {"path": "/home/bob/secret.txt"}})).await;
    let mine = alice.ok("resolve", json!({"at": {"path": "/home/alice/docs"}})).await;
    let doc = alice.ok("resolve", json!({"at": {"path": "/home/alice/docs/a.txt"}})).await;
    let bob_home = owner.ok("resolve", json!({"at": {"path": "/home/bob"}})).await;
    let submit = |source: &Value, dest: &Value| {
        json!({"args": {"idempotency_key": "k", "input": {
            "sources": [{"source_ref": source["copy_ref"], "name": "x", "source_path": "/x"}],
            "destination_ref": dest["copy_ref"], "conflict": "ask", "retry_of": null,
        }}})
    };
    let v = alice.write("copy_submit", submit(&secret, &mine)).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED", "{}", v);
    let v = alice.write("copy_submit", submit(&doc, &bob_home)).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED", "{}", v);
    // Allowed input passes the gate and only then reaches TaskMgr.
    let v = alice.write("copy_submit", submit(&doc, &mine)).await;
    assert_ne!(v["error"]["code"], "PERMISSION_DENIED", "{}", v);
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_cannot_carry_grants_elsewhere() {
    let s = start().await;
    let alice = s.client(Some("alice")).await;
    std::os::unix::fs::symlink("../bob", s.data.join("home/alice/bob-link")).unwrap();
    std::os::unix::fs::symlink("../bob/secret.txt", s.data.join("home/alice/secret-link.txt")).unwrap();
    std::os::unix::fs::symlink("docs/a.txt", s.data.join("home/alice/own-link.txt")).unwrap();
    // A symlinked directory on the way is refused even inside one's own home.
    let v = alice.call("resolve", json!({"at": {"path": "/home/alice/bob-link/secret.txt"}})).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED", "{}", v);
    assert_eq!(v["error"]["reason"], "symlink");
    // A file symlink is a visible node, but reading follows it to its real path.
    let alice_token = format!("Bearer {}", token("alice"));
    let leak = alice.node_id("/home/alice/secret-link.txt").await;
    assert_eq!(read_status(&s.base, &leak, Some(("Authorization", &alice_token))).await, 403);
    let own = alice.node_id("/home/alice/own-link.txt").await;
    assert_eq!(read_status(&s.base, &own, Some(("Authorization", &alice_token))).await, 200);
    let owner_token = format!("Bearer {}", token("owner"));
    assert_eq!(read_status(&s.base, &leak, Some(("Authorization", &owner_token))).await, 200);
}

#[tokio::test]
async fn per_file_write_grant_saves_but_cannot_create() {
    let s = start().await;
    let alice = s.client(Some("alice")).await;
    let bob = s.client(Some("bob")).await;
    bob.set_acl("/home/bob/secret.txt", true, json!([{"principal": "user:alice", "level": "write"}])).await;
    assert_eq!(alice.names("/home/bob").await, vec!["secret.txt"]);
    let file = alice.ok("resolve", json!({"at": {"path": "/home/bob/secret.txt"}})).await;
    let parent = alice.ok("resolve", json!({"at": {"path": "/home/bob"}})).await;
    let content = b"edited by alice";
    let ow = alice.ok_write("open_write", json!({"args": {"ref": file["ref"], "size": content.len()}})).await;
    let fb = ow["fb_handle"].as_str().unwrap();
    let resp = reqwest::Client::new()
        .patch(format!("{}/nfs/v1/uploads/{}", s.base, fb))
        .bearer_auth(token("alice"))
        .header("Upload-Offset", "0")
        .body(content.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 204);
    alice
        .ok_write(
            "commit_file",
            json!({"args": {"parent_ref": parent["ref"], "name": "secret.txt", "fb_handle": fb, "lease_id": ow["lease"]["lease_id"], "overwrite": true}}),
        )
        .await;
    assert_eq!(std::fs::read(s.data.join("home/bob/secret.txt")).unwrap(), content);
    let v = alice.write("open_write", json!({"args": {"parent_ref": parent["ref"], "name": "new.txt", "size": 1}})).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED", "{}", v);
}

#[tokio::test]
async fn records_of_items_removed_outside_do_not_reattach() {
    let s = start().await;
    let bob = s.client(Some("bob")).await;
    let anon = s.client(None).await;
    bob.set_acl("/home/bob/share/pub", true, json!([{"principal": "public", "level": "read"}])).await;
    assert_eq!(anon.names("/home/bob/share/pub").await, vec!["photo.jpg"]);
    std::fs::remove_dir_all(s.data.join("home/bob/share/pub")).unwrap();
    let records = s.client(Some("owner")).await.ok("list_acls", json!({})).await;
    assert_eq!(records["records"][0]["missing"], true);
    bob.ok_write("mkdir", json!({"at": {"path": "/home/bob/share"}, "args": {"name": "pub"}})).await;
    assert_eq!(anon.code("list", json!({"at": {"path": "/home/bob/share/pub"}})).await, "PERMISSION_DENIED");
    assert_eq!(s.client(Some("owner")).await.ok("list_acls", json!({})).await["records"].as_array().unwrap().len(), 0);
}
