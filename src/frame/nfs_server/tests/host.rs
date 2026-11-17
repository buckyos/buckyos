//! The host realm: this computer's own file tree on BuckyOS Desktop Lite,
//! reachable only by the zone owner through a BuckyOS sudo session
//! (product/bucky_file/nfs_server.md §14). Tokens are signed like
//! verify-hub tokens with the test key of tests/acl.rs.

use base64::Engine;
use ndn_lib::NamedObject;
use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, HostRealm, NamedStoreSource, ServerConfig};
use nfs_server::state::{AppState, SharedState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = "g, owner, admin\ng, alice, users\ng, system:control-panel, system\n";
        let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, groups);
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim()).await.unwrap();
    })
    .await;
}

fn now() -> u64 {
    buckyos_kit::buckyos_get_unix_timestamp()
}

struct Tok<'a> {
    sub: &'a str,
    sudo: bool,
    aud: Option<&'a str>,
    exp: u64,
}

impl Tok<'_> {
    fn sign(&self) -> String {
        let mut token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: None,
            aud: self.aud.map(String::from),
            exp: Some(self.exp),
            iss: Some(VERIFY_HUB_UNIQUE_ID.into()),
            jti: None,
            sub: Some(self.sub.into()),
            appid: None,
            sudo: self.sudo,
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
}

fn normal(user: &str) -> String {
    Tok { sub: user, sudo: false, aud: None, exp: now() + 3600 }.sign()
}

fn sudo(user: &str) -> String {
    Tok { sub: user, sudo: true, aud: Some("nfs-server"), exp: now() + 1800 }.sign()
}

struct Server {
    base: String,
    state: SharedState,
    computer: PathBuf,
    home: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn start_with(tune: impl FnOnce(&mut ServerConfig)) -> Server {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let computer = tmp.path().join("computer");
    let home = tmp.path().join("exports/home");
    for d in ["Users/alice/docs", "etc", "Volumes"] {
        std::fs::create_dir_all(computer.join(d)).unwrap();
    }
    std::fs::create_dir_all(home.join("owner/inbox")).unwrap();
    std::fs::write(home.join("owner/inbox/zone.txt"), b"from the zone").unwrap();
    std::fs::write(computer.join("Users/alice/My Doc.txt"), b"hello computer").unwrap();
    std::fs::write(computer.join("Users/alice/docs/a.txt"), b"aaa").unwrap();
    std::fs::write(computer.join("etc/hosts"), b"127.0.0.1 localhost").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("docs", computer.join("Users/alice/link-dir")).unwrap();
        std::os::unix::fs::symlink("My Doc.txt", computer.join("Users/alice/link-file.txt")).unwrap();
        std::os::unix::fs::symlink("nowhere", computer.join("Users/alice/broken")).unwrap();
    }
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: home.clone() }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.host = Some(HostRealm::new(computer.clone(), "Test Mac".into()));
    config.copy_test = Some(NfsCopyTestConfig {
        task_mgr_url: "http://127.0.0.1:9/kapi/task-manager".into(),
        service_token: String::new(),
        public_key: PUBLIC.into(),
    });
    tune(&mut config);
    let state = AppState::new(config).unwrap();
    let app = nfs_server::server::build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, state, computer, home, _tmp: tmp }
}

async fn start() -> Server {
    start_with(|_| {}).await
}

struct Client {
    base: String,
    http: reqwest::Client,
    token: Option<String>,
    session: String,
    hello: Value,
    seq: AtomicU64,
}

impl Server {
    async fn client(&self, token: Option<String>) -> Client {
        let mut c = Client {
            base: self.base.clone(),
            http: reqwest::Client::new(),
            token,
            session: String::new(),
            hello: Value::Null,
            seq: AtomicU64::new(1),
        };
        let hello = c.post("hello", json!({"args": {"versions": ["nfsp/0"]}})).await;
        assert_eq!(hello["ok"], true, "{}", hello);
        c.session = hello["result"]["session"].as_str().unwrap().to_string();
        c.hello = hello["result"].clone();
        c
    }

    fn count(&self, sql: &str) -> i64 {
        rusqlite::Connection::open(self.state.config.db_path()).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
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
        if !matches!(method, "list" | "resolve" | "stat" | "batch" | "get_acl" | "search" | "repr" | "get_repr" | "get_meta") {
            body["seq"] = json!(self.seq.fetch_add(1, Ordering::SeqCst));
        }
        self.post(method, body).await
    }

    async fn ok(&self, method: &str, body: Value) -> Value {
        let v = self.call(method, body).await;
        assert_eq!(v["ok"], true, "{}: {}", method, v);
        v["result"].clone()
    }

    async fn code(&self, method: &str, body: Value) -> String {
        let v = self.call(method, body).await;
        assert_eq!(v["ok"], false, "{} unexpectedly ok: {}", method, v);
        v["error"]["code"].as_str().unwrap().to_string()
    }

    async fn list(&self, path: &str) -> Value {
        self.ok("list", json!({"at": {"realm": "host", "path": path}, "want": ["base", "access"]})).await
    }

    async fn stat(&self, path: &str) -> Value {
        self.ok("stat", json!({"at": {"realm": "host", "path": path}, "want": ["base", "ident", "access"]})).await
    }

    async fn host_code(&self, path: &str) -> String {
        self.code("list", json!({"at": {"realm": "host", "path": path}})).await
    }
}

fn entry<'a>(listing: &'a Value, name: &str) -> &'a Value {
    listing["entries"].as_array().unwrap().iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("no {name} in {listing}"))
}

fn names(listing: &Value) -> Vec<String> {
    listing["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
}

fn url_of(info: &Value, kind: &str) -> Option<String> {
    info["access_urls"].as_array()?.iter().find(|u| u["kind"] == kind).and_then(|u| u["url"].as_str()).map(String::from)
}

async fn get(base: &str, url: &str, auth: Option<(&str, String)>) -> reqwest::Response {
    let mut req = reqwest::Client::new().get(format!("{}{}", base, url));
    if let Some((name, value)) = auth {
        req = req.header(name, value);
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn realm_is_advertised_to_the_zone_owner_only() {
    let s = start().await;
    let owner = s.client(Some(normal("owner"))).await;
    assert_eq!(owner.hello["realms"], json!([{"id": "dfs", "writable": true}, {"id": "host", "writable": true, "label": "Test Mac"}]));
    assert!(owner.hello.get("sudo_exp").is_none());
    let unlocked = s.client(Some(sudo("owner"))).await;
    assert!(unlocked.hello["sudo_exp"].as_u64().unwrap() > now());
    let alice = s.client(Some(normal("alice"))).await;
    assert_eq!(alice.hello["realms"], json!([{"id": "dfs", "writable": true}]));
    let alice_sudo = s.client(Some(sudo("alice"))).await;
    assert_eq!(alice_sudo.hello["realms"].as_array().unwrap().len(), 1);
    assert!(alice_sudo.hello.get("sudo_exp").is_none());

    let plain = start_with(|c| c.host = None).await;
    let owner = plain.client(Some(sudo("owner"))).await;
    assert_eq!(owner.hello["realms"], json!([{"id": "dfs", "writable": true}]));
    assert_eq!(owner.host_code("/").await, "NOT_FOUND");

    let open = start_with(|c| c.auth = AuthMode::Disabled).await;
    let anyone = open.client(None).await;
    assert_eq!(anyone.hello["realms"], json!([{"id": "dfs", "writable": true}]));
    assert_eq!(anyone.host_code("/").await, "PERMISSION_DENIED");
}

#[tokio::test]
async fn host_access_needs_a_sudo_session_of_the_owner() {
    let s = start().await;
    let ok = s.client(Some(sudo("owner"))).await;
    assert_eq!(names(&ok.list("/").await), vec!["Users", "Volumes", "etc"]);

    // A normal session, a sudo token on that session, or a sudo session with the wrong kind of token.
    let owner = s.client(Some(normal("owner"))).await;
    assert_eq!(owner.host_code("/").await, "SUDO_REQUIRED");
    assert_eq!(owner.host_code("/Users").await, "SUDO_REQUIRED");
    let mut reused = s.client(Some(normal("owner"))).await;
    reused.token = Some(sudo("owner"));
    assert_eq!(reused.host_code("/").await, "SUDO_REQUIRED");
    for (sudo_flag, aud) in [(false, Some("nfs-server")), (true, None), (true, Some("system-config"))] {
        let c = s.client(Some(Tok { sub: "owner", sudo: sudo_flag, aud, exp: now() + 600 }.sign())).await;
        assert_eq!(c.host_code("/").await, "SUDO_REQUIRED", "sudo={sudo_flag} aud={aud:?}");
        assert!(c.hello.get("sudo_exp").is_none());
    }
    // Inside the verifier's leeway but past `exp`.
    let late = s.client(Some(Tok { sub: "owner", sudo: true, aud: Some("nfs-server"), exp: now() - 5 }.sign())).await;
    assert_eq!(late.host_code("/").await, "SUDO_REQUIRED");
    // Cookies never grant.
    let cookie = reqwest::Client::new();
    let hello: Value = cookie
        .post(format!("{}/nfs/v1/hello", s.base))
        .header("Cookie", format!("buckyos_session_token={}", sudo("owner")))
        .json(&json!({"args": {}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let listed: Value = cookie
        .post(format!("{}/nfs/v1/list", s.base))
        .header("Cookie", format!("buckyos_session_token={}", sudo("owner")))
        .json(&json!({"session": hello["result"]["session"], "at": {"realm": "host", "path": "/"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["error"]["code"], "SUDO_REQUIRED", "{listed}");
    // Other users: the realm is not theirs, sudo or not.
    let alice = s.client(Some(sudo("alice"))).await;
    assert_eq!(alice.host_code("/").await, "PERMISSION_DENIED");
    assert_eq!(alice.code("stat", json!({"at": {"realm": "host", "path": "/etc/hosts"}})).await, "PERMISSION_DENIED");
    // Refs minted under a grant are useless without one.
    let hosts = ok.stat("/etc/hosts").await;
    assert_eq!(owner.code("stat", json!({"at": {"ref": hosts["ref"]}})).await, "SUDO_REQUIRED");
    assert_eq!(alice.code("stat", json!({"at": {"ref": hosts["ref"]}})).await, "PERMISSION_DENIED");
    assert_eq!(
        owner.code("batch", json!({"args": {"start": {"ref": hosts["ref"]}, "ops": [{"m": "stat"}]}})).await,
        "SUDO_REQUIRED"
    );
    // DFS sessions of the owner keep working on the Zone.
    assert_eq!(owner.ok("list", json!({"at": {"path": "/home/owner"}})).await["entries"][0]["name"], "inbox");
    assert_eq!(ok.ok("list", json!({"at": {"path": "/home/owner"}})).await["entries"][0]["name"], "inbox");

    // The session ends with its token.
    let short = s.client(Some(Tok { sub: "owner", sudo: true, aud: Some("nfs-server"), exp: now() + 2 }.sign())).await;
    assert_eq!(names(&short.list("/etc").await), vec!["hosts"]);
    let read = url_of(&short.stat("/etc/hosts").await, "read").unwrap();
    assert_eq!(get(&s.base, &read, Some(("Authorization", format!("Bearer {}", normal("owner"))))).await.status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
    assert_eq!(short.host_code("/etc").await, "SUDO_REQUIRED");
    let expired = get(&s.base, &read, Some(("Authorization", format!("Bearer {}", normal("owner"))))).await;
    assert_eq!(expired.status(), 403);
    assert_eq!(expired.json::<Value>().await.unwrap()["error"]["code"], "SUDO_REQUIRED");
    // A fresh token does not revive the old session; a new hello does.
    let mut stale = s.client(Some(Tok { sub: "owner", sudo: true, aud: Some("nfs-server"), exp: now() + 2 }.sign())).await;
    tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
    stale.token = Some(sudo("owner"));
    assert_eq!(stale.host_code("/").await, "SUDO_REQUIRED");
    // A token past the verifier's leeway on a known sudo session: still SUDO_REQUIRED.
    let mut gone = s.client(Some(sudo("owner"))).await;
    gone.token = Some(Tok { sub: "owner", sudo: true, aud: Some("nfs-server"), exp: now() - 600 }.sign());
    assert_eq!(gone.host_code("/").await, "SUDO_REQUIRED");
}

#[cfg(unix)]
#[tokio::test]
async fn listing_follows_symlinks_and_paths_are_file_uris() {
    let s = start().await;
    let c = s.client(Some(sudo("owner"))).await;
    let root = c.list("/").await;
    assert_eq!(root["container"]["access_level"], "manage");
    assert_eq!(url_of(&entry(&root, "Users")["target"]["attrs"], "fs").as_deref(), Some("file:///Users"));
    let alice = c.list("/Users/alice").await;
    assert_eq!(names(&alice), vec!["My Doc.txt", "broken", "docs", "link-dir", "link-file.txt"]);
    let link_dir = entry(&alice, "link-dir");
    assert_eq!(link_dir["target"]["kind"], "dir");
    assert_eq!(link_dir["target"]["attrs"]["flags"], json!(["symlink"]));
    assert_eq!(link_dir["target"]["access_level"], "manage");
    let link_file = entry(&alice, "link-file.txt");
    assert_eq!(link_file["target"]["kind"], "file");
    assert_eq!(link_file["target"]["attrs"]["flags"], json!(["symlink"]));
    assert_eq!(link_file["target"]["attrs"]["size"], 14);
    assert_eq!(entry(&alice, "broken")["target"]["kind"], "symlink");
    assert_eq!(entry(&alice, "docs")["target"]["attrs"]["flags"], json!([]));
    // Navigating through a symlinked directory, by path and by ref.
    assert_eq!(names(&c.list("/Users/alice/link-dir").await), vec!["a.txt"]);
    let by_ref = c.ok("list", json!({"at": {"ref": link_dir["target"]["ref"]}})).await;
    assert_eq!(names(&by_ref), vec!["a.txt"]);
    let walked = c.ok("batch", json!({"args": {"start": {"realm": "host", "path": "/Users"}, "ops": [{"m": "walk", "args": {"names": ["alice", "link-dir"]}}, {"m": "list"}]}})).await;
    assert_eq!(walked["results"][1]["result"]["entries"][0]["name"], "a.txt", "{walked}");
    let doc = c.stat("/Users/alice/My Doc.txt").await;
    assert_eq!(doc["kind"], "file");
    assert_eq!(doc["locations"], json!([{"url": "file:///Users/alice/My%20Doc.txt", "via": "direct"}]));
    assert_eq!(url_of(&doc, "fs").as_deref(), Some("file:///Users/alice/My%20Doc.txt"));
    assert!(doc["copy_ref"]["node_id"].as_str().unwrap().starts_with("nh_"));
    assert!(doc.get("obj_id").is_none());
    let read = url_of(&doc, "read").unwrap();
    assert!(read.starts_with("/nfs/v1/read/nh_") && read.contains("?hx="), "{read}");

    // Paths stay inside the computer tree, and the Zone never reaches it.
    for bad in ["/Users/../etc", "/..", "../etc", "/Users/alice/docs/../../.."] {
        assert_eq!(c.host_code(bad).await, "INVALID_ARGUMENT", "{bad}");
    }
    assert_eq!(c.code("list", json!({"at": {"path": "/@host"}})).await, "NOT_FOUND");
    assert_eq!(c.code("list", json!({"at": {"path": "/@host/etc"}})).await, "NOT_FOUND");
    assert_eq!(c.code("list", json!({"at": {"uri": "cyfs:///@host/etc"}})).await, "NOT_FOUND");
    assert_eq!(c.code("list", json!({"at": {"realm": "device", "path": "/"}})).await, "NOT_FOUND");
    let zone_root = c.ok("list", json!({"at": {"path": "/"}})).await;
    assert_eq!(names(&zone_root), vec!["home"]);
    let walk = c.ok("batch", json!({"args": {"start": {"path": "/"}, "ops": [{"m": "walk", "args": {"name": "@host"}}]}})).await;
    assert_eq!(walk["results"][0]["error"]["code"], "NOT_FOUND", "{walk}");
    assert_eq!(c.code("mkdir", json!({"at": {"path": "/@host/new"}})).await, "NOT_FOUND");
    assert!(!s.computer.join("new").exists());
}

#[tokio::test]
async fn reads_need_the_capability_url_and_the_owner() {
    let s = start().await;
    let c = s.client(Some(sudo("owner"))).await;
    let doc = c.stat("/Users/alice/My Doc.txt").await;
    let read = url_of(&doc, "read").unwrap();
    let node_id = doc["ref"]["node_id"].as_str().unwrap().to_string();
    let owner_bearer = || Some(("Authorization", format!("Bearer {}", normal("owner"))));
    let owner_cookie = || Some(("Cookie", format!("buckyos_session_token={}", normal("owner"))));

    let r = get(&s.base, &read, owner_bearer()).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"hello computer");
    let r = get(&s.base, &read, owner_cookie()).await;
    assert_eq!(r.status(), 200);
    let r = get(&s.base, &format!("{}&download=1&name=saved.txt", read), owner_cookie()).await;
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-disposition"].to_str().unwrap().contains("saved.txt"));
    let r = get(&s.base, &read, Some(("Range", "bytes=6-".to_string()))).await;
    assert_eq!(r.status(), 403, "anonymous");
    let r = get(&s.base, &read, Some(("Authorization", format!("Bearer {}", normal("alice"))))).await;
    assert_eq!(r.status(), 403);

    let bare = format!("/nfs/v1/read/{}", node_id);
    let r = get(&s.base, &bare, owner_bearer()).await;
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], "SUDO_REQUIRED");
    let r = get(&s.base, &bare, Some(("Authorization", format!("Bearer {}", sudo("owner"))))).await;
    assert_eq!(r.status(), 403, "a sudo bearer is no capability");
    let exp: u64 = read.split("hx=").nth(1).unwrap().split('.').next().unwrap().parse().unwrap();
    let mac = read.split('.').next_back().unwrap();
    let tampered = format!("{}?hx={}.{}", bare, exp + 3600, mac);
    assert_eq!(get(&s.base, &tampered, owner_bearer()).await.status(), 403);
    let past = format!("{}?hx={}", bare, s.state.handles.sign_read(&node_id, now() - 1));
    let r = get(&s.base, &past, owner_bearer()).await;
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], "SUDO_REQUIRED");
    let other = c.stat("/etc/hosts").await;
    let swapped = format!("/nfs/v1/read/{}?hx={}", other["ref"]["node_id"].as_str().unwrap(), read.split("hx=").nth(1).unwrap());
    assert_eq!(get(&s.base, &swapped, owner_bearer()).await.status(), 403);

    // A normal session sees no read URL at all.
    let owner = s.client(Some(normal("owner"))).await;
    let dfs = owner.ok("stat", json!({"at": {"path": "/home/owner/inbox/zone.txt"}, "want": ["access"]})).await;
    assert!(url_of(&dfs, "read").unwrap().starts_with("/nfs/v1/read/"));
    assert!(!url_of(&dfs, "read").unwrap().contains("hx="));
}

#[cfg(unix)]
#[tokio::test]
async fn unreadable_and_special_entries() {
    use std::os::unix::fs::PermissionsExt;
    let s = start().await;
    let locked = s.computer.join("Users/locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("inside.txt"), b"x").unwrap();
    std::fs::write(s.computer.join("Users/secret.txt"), b"x").unwrap();
    let fifo = std::ffi::CString::new(s.computer.join("Users/pipe").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    let root = unsafe { libc::geteuid() } == 0;
    if !root {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::set_permissions(s.computer.join("Users/secret.txt"), std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    let c = s.client(Some(sudo("owner"))).await;
    let users = c.list("/Users").await;
    let pipe = entry(&users, "pipe");
    assert_eq!(pipe["target"]["kind"], "file");
    assert_eq!(pipe["target"]["access_level"], "none");
    assert_eq!(pipe["target"]["attrs"]["flags"], json!(["special"]));
    assert!(url_of(&pipe["target"]["attrs"], "read").is_none());
    let info = c.stat("/Users/pipe").await;
    let forged = format!("/nfs/v1/read/{}?hx={}", info["ref"]["node_id"].as_str().unwrap(), s.state.handles.sign_read(info["ref"]["node_id"].as_str().unwrap(), now() + 600));
    let r = get(&s.base, &forged, Some(("Authorization", format!("Bearer {}", normal("owner"))))).await;
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["reason"], "special_file");
    let thumb = c.ok("repr", json!({"at": {"realm": "host", "path": "/Users/pipe"}, "args": {"purpose": "thumbnail"}})).await;
    assert_eq!(thumb["state"], "unsupported", "{thumb}");
    if root {
        return;
    }
    assert_eq!(entry(&users, "locked")["target"]["kind"], "dir");
    assert_eq!(entry(&users, "locked")["target"]["access_level"], "none");
    assert_eq!(entry(&users, "secret.txt")["target"]["access_level"], "none");
    assert_eq!(c.host_code("/Users/locked").await, "PERMISSION_DENIED");
    assert_eq!(c.code("stat", json!({"at": {"realm": "host", "path": "/Users/locked/inside.txt"}})).await, "PERMISSION_DENIED");
    let secret = c.stat("/Users/secret.txt").await;
    assert_eq!(secret["access_level"], "none");
    let r = get(&s.base, &url_of(&secret, "read").unwrap(), Some(("Authorization", format!("Bearer {}", normal("owner"))))).await;
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], "PERMISSION_DENIED");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
}

async fn tus_upload(c: &Client, fb_url: &str, data: &[u8]) {
    let r = c
        .http
        .patch(format!("{}{}", c.base, fb_url))
        .bearer_auth(c.token.as_ref().unwrap())
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Offset", "0")
        .header("Content-Type", "application/offset+octet-stream")
        .body(data.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn mutations_stay_out_of_filedb_and_realms_do_not_mix() {
    let s = start().await;
    let c = s.client(Some(sudo("owner"))).await;
    let owner = s.client(Some(normal("owner"))).await;
    let alice_dir = c.stat("/Users/alice").await;

    let made = c.ok("mkdir", json!({"at": {"ref": alice_dir["ref"]}, "args": {"name": "Projects"}})).await;
    assert_eq!(made["existed"], false);
    assert!(s.computer.join("Users/alice/Projects").is_dir());
    c.ok("mkdir", json!({"at": {"realm": "host", "path": "/Users/alice/Projects/2026/q4"}})).await;
    assert!(s.computer.join("Users/alice/Projects/2026/q4").is_dir());
    assert_eq!(owner.code("mkdir", json!({"at": {"realm": "host", "path": "/Users/x"}})).await, "SUDO_REQUIRED");
    assert_eq!(owner.code("mkdir", json!({"at": {"ref": alice_dir["ref"]}, "args": {"name": "y"}})).await, "SUDO_REQUIRED");

    // Upload through open_write + tus + commit_file, then overwrite.
    let projects = c.stat("/Users/alice/Projects").await;
    for (round, body) in [(0, b"first".as_slice()), (1, b"second version".as_slice())] {
        let opened = c.ok("open_write", json!({"args": {"parent_ref": projects["ref"], "name": "notes.txt", "size": body.len()}})).await;
        assert_eq!(opened["target"]["path"], "file:///Users/alice/Projects/notes.txt");
        tus_upload(&c, opened["upload_url"].as_str().unwrap(), body).await;
        let committed = c.ok("commit_file", json!({"args": {"parent_ref": projects["ref"], "name": "notes.txt", "fb_handle": opened["fb_handle"], "lease_id": opened["lease"]["lease_id"], "overwrite": round == 1}})).await;
        assert!(committed["ref"]["node_id"].as_str().unwrap().starts_with("nh_"));
        assert_eq!(std::fs::read(s.computer.join("Users/alice/Projects/notes.txt")).unwrap(), body);
    }

    // Rename inside the computer tree; a move into the Zone is a copy + delete for the client.
    let moved = c.ok("move", json!({"args": {"from": {"parent_ref": projects["ref"], "name": "notes.txt"}, "to": {"parent_ref": projects["ref"], "name": "renamed.txt"}}})).await;
    assert!(moved["from_revision"].is_string());
    assert!(s.computer.join("Users/alice/Projects/renamed.txt").is_file());
    let inbox = c.ok("stat", json!({"at": {"path": "/home/owner/inbox"}})).await;
    let v = c.call("move", json!({"args": {"from": {"parent_ref": projects["ref"], "name": "renamed.txt"}, "to": {"parent_ref": inbox["ref"], "name": "renamed.txt"}}})).await;
    assert_eq!(v["error"]["code"], "UNSUPPORTED", "{v}");
    assert_eq!(v["error"]["reason"], "cross_realm");
    let v = c.call("move", json!({"args": {"from": {"parent_ref": inbox["ref"], "name": "zone.txt"}, "to": {"parent_ref": projects["ref"], "name": "zone.txt"}}})).await;
    assert_eq!(v["error"]["reason"], "cross_realm", "{v}");
    assert!(s.home.join("owner/inbox/zone.txt").is_file());
    assert!(s.computer.join("Users/alice/Projects/renamed.txt").is_file());

    // DFS-only abilities answer UNSUPPORTED.
    let renamed = c.stat("/Users/alice/Projects/renamed.txt").await;
    let at_renamed = json!({"realm": "host", "path": "/Users/alice/Projects/renamed.txt"});
    assert_eq!(c.code("get_acl", json!({"at": at_renamed})).await, "UNSUPPORTED");
    assert_eq!(c.code("set_acl", json!({"at": at_renamed, "args": {"entries": [{"principal": "zone_users", "level": "read"}]}})).await, "UNSUPPORTED");
    assert_eq!(c.code("set_acl", json!({"at": {"path": "/@host/Users"}, "args": {"entries": []}})).await, "NOT_FOUND");
    assert_eq!(c.code("set_meta", json!({"args": {"ref": renamed["ref"], "records": [{"ns": "user", "key": "k", "value": 1}]}})).await, "UNSUPPORTED");
    assert_eq!(c.code("bind_ref", json!({"args": {"parent_ref": projects["ref"], "name": "r", "target_ref": inbox["ref"]}})).await, "UNSUPPORTED");
    assert_eq!(c.code("bind_ref", json!({"args": {"parent_ref": inbox["ref"], "name": "r", "target_ref": renamed["ref"]}})).await, "UNSUPPORTED");
    assert_eq!(c.code("grant", json!({"args": {"ref": renamed["ref"]}})).await, "UNSUPPORTED");
    assert_eq!(c.code("search", json!({"args": {"q": "renamed", "scope": {"realm": "host", "path": "/Users"}}})).await, "UNSUPPORTED");
    let shared = c.call("share.plan", json!({"args": {"source": at_renamed}})).await;
    assert_eq!(shared["error"]["code"], "UNSUPPORTED", "{shared}");
    assert_eq!(shared["error"]["reason"], "source_unsupported");
    assert!(c.ok("get_meta", json!({"args": {"ref": renamed["ref"]}})).await["records"].as_array().unwrap().is_empty());

    // Delete, recursive and not.
    std::fs::write(s.computer.join("Users/alice/Projects/2026/q4/keep.txt"), b"k").unwrap();
    let year = c.stat("/Users/alice/Projects/2026").await;
    assert_eq!(c.code("delete", json!({"args": {"parent_ref": year["ref"], "name": "q4"}})).await, "NOT_EMPTY");
    assert_eq!(c.code("delete", json!({"args": {"parent_ref": year["ref"], "name": "nope"}})).await, "NOT_FOUND");
    c.ok("delete", json!({"args": {"parent_ref": projects["ref"], "name": "2026", "recursive": true}})).await;
    c.ok("delete", json!({"args": {"parent_ref": projects["ref"], "name": "renamed.txt"}})).await;
    assert!(!s.computer.join("Users/alice/Projects/2026").exists());
    assert!(!s.computer.join("Users/alice/Projects/renamed.txt").exists());
    assert_eq!(owner.code("delete", json!({"args": {"parent_ref": projects["ref"], "name": "x"}})).await, "SUDO_REQUIRED");

    for table in ["anchors", "content_index", "acls"] {
        assert_eq!(s.count(&format!("SELECT COUNT(*) FROM {table} WHERE root_id='@host'")), 0, "{table}");
    }
    assert_eq!(s.count("SELECT COUNT(*) FROM namespace_bindings"), 0);
    assert_eq!(s.count("SELECT COUNT(*) FROM grants"), 0);
}

#[tokio::test]
async fn committed_objects_land_on_the_computer() {
    let s = start().await;
    let token = sudo("owner");
    let c = s.client(Some(token.clone())).await;
    let http = reqwest::Client::new();
    let data = b"uploaded through NDM with the sudo token".to_vec();
    let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(&data).unwrap().to_string();
    let (file_id, file_json) = ndn_lib::FileObject::new("up.txt".into(), data.len() as u64, chunk.clone()).gen_obj_id();
    let b64 = |v: &str| base64::engine::general_purpose::STANDARD.encode(v);
    let meta = format!(
        "app_id {},logical_path {},file_name {},file_hash {},chunk_index {},chunk_hash {}",
        b64("default"),
        b64(&format!("default/{chunk}")),
        b64("up.txt"),
        b64(&file_id.to_string()),
        b64("0"),
        b64(&chunk)
    );
    let created = http
        .post(format!("{}/ndm/v1/uploads", s.base))
        .bearer_auth(&token)
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Length", data.len().to_string())
        .header("Upload-Metadata", meta)
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201, "{:?}", created.text().await);
    let location = created.headers()["location"].to_str().unwrap().to_string();
    let patched = http
        .patch(format!("{}{}", s.base, location))
        .bearer_auth(&token)
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Offset", "0")
        .header("Content-Type", "application/offset+octet-stream")
        .body(data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(patched.status(), 204);
    let put = http
        .post(format!("{}/ndm/v1/store/put_object", s.base))
        .bearer_auth(&token)
        .json(&json!({"obj_id": file_id.to_string(), "obj_data": file_json}))
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success(), "{:?}", put.text().await);

    let committed = c.ok("commit_object", json!({"at": {"realm": "host", "path": "/Users/alice"}, "args": {"name": "up.txt", "obj_id": file_id.to_string()}})).await;
    assert_eq!(committed["kind"], "file");
    assert_eq!(std::fs::read(s.computer.join("Users/alice/up.txt")).unwrap(), data);
    assert!(std::fs::read_dir(s.computer.join("Users/alice")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".nfs_")));
    let owner = s.client(Some(normal("owner"))).await;
    let v = owner.call("commit_object", json!({"at": {"realm": "host", "path": "/Users/alice"}, "args": {"name": "again.txt", "obj_id": file_id.to_string()}})).await;
    assert_eq!(v["error"]["code"], "SUDO_REQUIRED", "{v}");
    assert_eq!(s.count("SELECT COUNT(*) FROM content_index WHERE root_id='@host'"), 0);
    assert_eq!(s.count("SELECT COUNT(*) FROM anchors WHERE root_id='@host'"), 0);
}

#[tokio::test]
async fn copies_across_realms_are_authorized_under_the_grant() {
    let s = start().await;
    let c = s.client(Some(sudo("owner"))).await;
    let owner = s.client(Some(normal("owner"))).await;
    let doc = c.stat("/Users/alice/My Doc.txt").await;
    let host_dir = c.stat("/Users/alice/docs").await;
    let zone_file = c.ok("stat", json!({"at": {"path": "/home/owner/inbox/zone.txt"}})).await;
    let zone_dir = c.ok("stat", json!({"at": {"path": "/home/owner/inbox"}})).await;
    let submit = |source: &Value, path: &str, dest: &Value| {
        json!({"args": {"idempotency_key": format!("k-{}", path), "input": {
            "sources": [{"source_ref": source["copy_ref"], "name": "x", "source_path": path}],
            "destination_ref": dest["copy_ref"], "conflict": "ask", "retry_of": null,
        }}})
    };
    let to_zone = submit(&doc, "/@host/Users/alice/My Doc.txt", &zone_dir);
    let to_host = submit(&zone_file, "/home/owner/inbox/zone.txt", &host_dir);
    // Past the gate the unreachable TaskMgr answers.
    for body in [&to_zone, &to_host] {
        let v = c.call("copy_submit", body.clone()).await;
        assert_eq!(v["error"]["code"], "INTERNAL", "{v}");
        assert!(v["error"]["message"].as_str().unwrap().contains("task-manager"), "{v}");
    }
    let mut normal_session = s.client(Some(normal("owner"))).await;
    for body in [&to_zone, &to_host] {
        assert_eq!(normal_session.call("copy_submit", body.clone()).await["error"]["code"], "SUDO_REQUIRED");
        assert_eq!(owner.call("copy_submit", body.clone()).await["error"]["code"], "SUDO_REQUIRED");
    }
    normal_session.token = Some(sudo("owner"));
    assert_eq!(normal_session.call("copy_submit", to_zone.clone()).await["error"]["code"], "SUDO_REQUIRED");
    let alice = s.client(Some(sudo("alice"))).await;
    assert_eq!(alice.call("copy_submit", to_zone).await["error"]["code"], "PERMISSION_DENIED");
}

#[tokio::test]
async fn thumbnails_of_computer_files() {
    if !["ffmpeg", "ffprobe"].iter().all(|t| std::process::Command::new(t).arg("-version").output().is_ok_and(|o| o.status.success())) {
        return;
    }
    let s = start().await;
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/h264_aac.mov");
    std::fs::copy(fixture, s.computer.join("Users/alice/clip.mov")).unwrap();
    let c = s.client(Some(sudo("owner"))).await;
    let mut record = c.ok("repr", json!({"at": {"realm": "host", "path": "/Users/alice/clip.mov"}, "args": {"purpose": "thumbnail"}})).await;
    for _ in 0..240 {
        if record["state"] != "processing" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        record = c.ok("get_repr", json!({"args": {"work_key": record["work_key"]}})).await;
    }
    assert_eq!(record["state"], "completed", "{record}");
    let image = get(&s.base, record["result"]["read_url"].as_str().unwrap(), None).await;
    assert_eq!(image.status(), 200);
    let owner = s.client(Some(normal("owner"))).await;
    let v = owner.call("repr", json!({"at": {"realm": "host", "path": "/Users/alice/clip.mov"}, "args": {"purpose": "thumbnail"}})).await;
    assert_eq!(v["error"]["code"], "SUDO_REQUIRED", "{v}");
}
