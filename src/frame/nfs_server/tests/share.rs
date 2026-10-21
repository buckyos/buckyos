//! Share module over real HTTP (design §5 M1): snapshot shares in a temporary
//! named store, the `/s/{token}` data plane, policy changes, app handoffs,
//! sweeper expiry and statistics. Tokens are signed like verify-hub session
//! tokens (test key, same as tests/acl.rs).

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::{AppState, SharedState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

fn token_for(user: &str, kind: TokenPrincipalKind, target: AuthTarget) -> String {
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
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(&mut token, &target, TokenUse::Session).unwrap();
    token.generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap()).unwrap()
}

fn user_token(user: &str) -> String {
    token_for(
        user,
        TokenPrincipalKind::User,
        AuthTarget::system(SystemServiceId::parse(CONTROL_PANEL_SERVICE_NAME).unwrap()),
    )
}

struct Server {
    base: String,
    data: PathBuf,
    state: SharedState,
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
    for d in ["home/alice/docs/sub", "home/alice/docs/.git", "home/alice/docs/node_modules/x", "home/bob", "srv/publish/site", "srv/library"] {
        std::fs::create_dir_all(data.join(d)).unwrap();
    }
    std::fs::write(data.join("home/alice/report.txt"), b"hello share world").unwrap();
    std::fs::write(data.join("home/alice/photo.jpg"), b"\xff\xd8\xff\xe0fake-jpeg").unwrap();
    std::fs::write(data.join("home/alice/paper.pdf"), b"%PDF-1.4 fake").unwrap();
    std::fs::write(data.join("home/alice/page.html"), b"<script>alert(1)</script>").unwrap();
    std::fs::write(data.join("home/alice/docs/a.txt"), b"aaa").unwrap();
    std::fs::write(data.join("home/alice/docs/sub/b.txt"), b"bbbb").unwrap();
    std::fs::write(data.join("home/alice/docs/.env"), b"SECRET=1").unwrap();
    std::fs::write(data.join("home/alice/docs/id.pem"), b"key").unwrap();
    std::fs::write(data.join("home/alice/docs/.git/HEAD"), b"ref").unwrap();
    std::fs::write(data.join("home/alice/docs/node_modules/x/i.js"), b"js").unwrap();
    std::fs::write(data.join("home/bob/secret.txt"), b"bob secret").unwrap();
    std::fs::write(data.join("srv/publish/index.html"), b"<h1>hi</h1>").unwrap();
    std::fs::write(data.join("srv/publish/site/app.js"), b"1").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hostname", data.join("srv/publish/escape")).unwrap();
        std::os::unix::fs::symlink(data.join("srv/publish/index.html"), data.join("srv/publish/inside")).unwrap();
        std::os::unix::fs::symlink(data.join("home/bob/secret.txt"), data.join("home/alice/docs/link.txt")).unwrap();
    }
    let mut config = ServerConfig::new(
        tmp.path().join("fsdb"),
        vec![
            ExportRoot { id: "home".into(), path: data.join("home") },
            ExportRoot { id: "srv".into(), path: data.join("srv") },
        ],
    );
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.share.public_base_url = Some("https://public.test.zone".into());
    config.share.zone_host = Some("test.zone".into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    config.share.base_url = Some(base.clone());
    tune(&mut config);
    let state = AppState::new(config).unwrap();
    let app = nfs_server::server::build_router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, data, state, _tmp: tmp }
}

impl Server {
    async fn client_with(&self, token: Option<String>) -> Client {
        let mut c = Client {
            base: self.base.clone(),
            http: reqwest::Client::new(),
            token,
            session: String::new(),
            seq: AtomicU64::new(1),
        };
        let hello = c.post("hello", json!({"args": {"versions": ["nfsp/0"]}})).await;
        assert_eq!(hello["ok"], true, "{}", hello);
        assert!(hello["result"]["features"].as_array().unwrap().iter().any(|f| f == "share"));
        c.session = hello["result"]["session"].as_str().unwrap().to_string();
        c
    }

    async fn client(&self, user: &str) -> Client {
        self.client_with(Some(user_token(user))).await
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().get(format!("{}{}", self.base, path))
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

    async fn call(&self, method: &str, args: Value) -> Value {
        let mut body = json!({"session": self.session, "args": args});
        if nfs_server::share::rpc::WRITE_METHODS.contains(&method) {
            body["seq"] = json!(self.seq.fetch_add(1, Ordering::SeqCst));
        }
        self.post(method, body).await
    }

    async fn ok(&self, method: &str, args: Value) -> Value {
        let v = self.call(method, args).await;
        assert_eq!(v["ok"], true, "{}: {}", method, v);
        v["result"].clone()
    }

    async fn err(&self, method: &str, args: Value) -> Value {
        let v = self.call(method, args).await;
        assert_eq!(v["ok"], false, "{} should fail: {}", method, v);
        v["error"].clone()
    }

    async fn create(&self, path: &str, policy: Value) -> Value {
        let key = uuid::Uuid::new_v4().to_string();
        let op = self.ok("share.create", json!({"source": {"path": path}, "policy": policy, "idempotency_key": key})).await;
        assert_eq!(op["state"], "completed", "{}", op);
        op
    }
}

fn token_of(url: &str) -> String {
    url.rsplit("/s/").next().unwrap().to_string()
}

#[tokio::test]
async fn file_share_lifecycle() {
    let srv = start().await;
    let alice = srv.client("alice").await;

    let caps = alice.ok("share.capabilities", json!({})).await;
    assert_eq!(caps["blocked"], json!([]));
    assert_eq!(caps["default_expiry"], 7 * 86400);

    let plan = alice.ok("share.plan", json!({"source": {"path": "home/alice/report.txt"}})).await;
    assert_eq!(plan["source"]["kind"], "file");
    assert_eq!(plan["estimate"]["total_size"], 17);
    assert_eq!(plan["existing"], json!([]));

    let op = alice.create("home/alice/report.txt", json!({"access": "holder"})).await;
    let url = op["url"].as_str().unwrap().to_string();
    assert!(url.starts_with(&format!("{}/s/", srv.base)));
    let share = &op["share"];
    assert_eq!(share["status"], "active");
    assert!(share["obj_id"].as_str().unwrap().starts_with("cyfile:"));
    let expires = share["expires_at"].as_i64().unwrap();
    assert!((expires - buckyos_kit::buckyos_get_unix_timestamp() as i64 - 7 * 86400).abs() < 10, "default is seven days");
    let token = token_of(&url);

    // Visitor page, meta and content.
    let page = srv.get(&format!("/s/{token}")).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(page.headers()["referrer-policy"], "no-referrer");
    assert!(page.text().await.unwrap().contains("<title>report.txt</title>"));
    let meta: Value = srv.get(&format!("/s/{token}/meta")).send().await.unwrap().json().await.unwrap();
    assert_eq!(meta["kind"], "file");
    assert_eq!(meta["file"]["mime"], "text/plain");
    let body = srv.get(&format!("/s/{token}/d")).send().await.unwrap();
    assert_eq!(body.headers()["content-type"], "text/plain; charset=utf-8");
    assert!(body.headers()["content-disposition"].to_str().unwrap().starts_with("inline"));
    assert_eq!(body.text().await.unwrap(), "hello share world");
    let dl = srv.get(&format!("/s/{token}/d?download=1")).send().await.unwrap();
    assert!(dl.headers()["content-disposition"].to_str().unwrap().starts_with("attachment"));
    let part = srv.get(&format!("/s/{token}/d")).header("Range", "bytes=6-10").send().await.unwrap();
    assert_eq!(part.status(), 206);
    assert_eq!(part.headers()["content-range"], "bytes 6-10/17");
    assert_eq!(part.text().await.unwrap(), "share");

    // The snapshot does not follow later edits of the source.
    std::fs::write(srv.data.join("home/alice/report.txt"), b"changed!").unwrap();
    assert_eq!(srv.get(&format!("/s/{token}/d")).send().await.unwrap().text().await.unwrap(), "hello share world");

    // Owner sees the link again; existing shares show up in plan and access_urls.
    let got = alice.ok("share.get", json!({"share_id": share["share_id"]})).await;
    assert_eq!(got["url"], url);
    assert_eq!(got["audit"][0]["action"], "create");
    assert!(got["retention"][0]["pin_owner"].as_str().unwrap().starts_with("share:"));
    std::fs::write(srv.data.join("home/alice/report.txt"), b"hello share world").unwrap();
    let plan = alice.ok("share.plan", json!({"source": {"path": "home/alice/report.txt"}})).await;
    assert_eq!(plan["existing"].as_array().unwrap().len(), 1);
    let info = alice
        .post("stat", json!({"session": alice.session, "at": {"path": "home/alice/report.txt"}, "want": ["access"]}))
        .await;
    let urls = info["result"]["access_urls"].as_array().unwrap();
    assert!(urls.iter().any(|u| u["kind"] == "signed" && u["url"] == url), "{:?}", urls);

    // Others cannot see or manage it.
    let bob = srv.client("bob").await;
    assert_eq!(bob.err("share.get", json!({"share_id": share["share_id"]})).await["code"], "NOT_FOUND");
    assert_eq!(bob.ok("share.list", json!({})).await["items"], json!([]));
    assert_eq!(alice.ok("share.list", json!({})).await["items"].as_array().unwrap().len(), 1);

    // Revision conflict, then stop: the link becomes an ordinary 404.
    let conflict = alice.err("share.stop", json!({"share_id": share["share_id"], "revision": 99})).await;
    assert_eq!(conflict["reason"], "revision_conflict");
    assert_eq!(conflict["current"]["revision"], 1);
    let stopped = alice.ok("share.stop", json!({"share_id": share["share_id"], "revision": 1})).await;
    assert_eq!(stopped["status"], "stopped");
    for path in [format!("/s/{token}"), format!("/s/{token}/meta"), format!("/s/{token}/d")] {
        assert_eq!(srv.get(&path).send().await.unwrap().status(), 404, "{path}");
    }
    let unknown = srv.get("/s/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").send().await.unwrap();
    assert_eq!(unknown.status(), 404);
    let got = alice.ok("share.get", json!({"share_id": share["share_id"]})).await;
    assert!(got["retention"][0]["released_at"].is_i64());
    assert_eq!(got["audit"][0]["action"], "stop");
}

#[tokio::test]
async fn password_unlock_and_rotation() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let op = alice.create("home/alice/report.txt", json!({"access": "password", "password": "s3cret", "expires_at": null})).await;
    let token = token_of(op["url"].as_str().unwrap());
    assert!(op["share"]["expires_at"].is_null());
    assert_eq!(op["share"]["has_password"], true);

    let page = srv.get(&format!("/s/{token}")).send().await.unwrap().text().await.unwrap();
    assert!(page.contains("\"locked\":true") && !page.contains("report.txt"), "locked page hides the title");
    assert_eq!(srv.get(&format!("/s/{token}/meta")).send().await.unwrap().status(), 401);
    assert_eq!(srv.get(&format!("/s/{token}/d")).send().await.unwrap().status(), 401);
    let http = reqwest::Client::new();
    let unlock = |pw: &'static str| {
        http.post(format!("{}/s/{token}/unlock", srv.base)).json(&json!({"password": pw})).send()
    };
    assert_eq!(unlock("wrong").await.unwrap().status(), 403);
    let ok = unlock("s3cret").await.unwrap();
    assert_eq!(ok.status(), 200);
    let set_cookie = ok.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(set_cookie.contains(&format!("Path=/s/{token}")) && set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
    let cookie = set_cookie.split(';').next().unwrap().to_string();
    let with_cookie = srv.get(&format!("/s/{token}/d")).header("Cookie", &cookie).send().await.unwrap();
    assert_eq!(with_cookie.text().await.unwrap(), "hello share world");

    // Changing the password invalidates earlier unlocks at once (T05).
    let share_id = op["share"]["share_id"].clone();
    let upd = alice.ok("share.update", json!({"share_id": share_id, "revision": 1, "changes": {"password": "n3w"}})).await;
    assert_eq!(upd["revision"], 2);
    assert_eq!(srv.get(&format!("/s/{token}/meta")).header("Cookie", &cookie).send().await.unwrap().status(), 401);

    // Removing the password opens the link to holders again.
    alice.ok("share.update", json!({"share_id": share_id, "revision": 2, "changes": {"password": null}})).await;
    assert_eq!(srv.get(&format!("/s/{token}/meta")).send().await.unwrap().status(), 200);

    // Download can be turned off; preview stays.
    alice.ok("share.update", json!({"share_id": share_id, "revision": 3, "changes": {"allow_download": false}})).await;
    assert_eq!(srv.get(&format!("/s/{token}/d?download=1")).send().await.unwrap().status(), 403);
    assert_eq!(srv.get(&format!("/s/{token}/d")).send().await.unwrap().status(), 200);

    // Unlock attempts are rate limited per token.
    alice.ok("share.update", json!({"share_id": share_id, "revision": 4, "changes": {"password": "again"}})).await;
    // Concurrent, so all attempts fall into one minute even when hashing is slow.
    let statuses = futures::future::join_all((0..12).map(|_| unlock("nope"))).await;
    assert!(statuses.into_iter().any(|r| r.unwrap().status() == 429));
}

#[tokio::test]
async fn folder_share_excludes_and_lists() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let plan = alice.ok("share.plan", json!({"source": {"path": "home/alice/docs"}})).await;
    let excluded: Vec<(String, String)> = plan["estimate"]["excluded"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["path"].as_str().unwrap().to_string(), e["reason"].as_str().unwrap().to_string()))
        .collect();
    for (path, why) in [(".env", "secret"), ("id.pem", "secret"), (".git/", "excluded_by_default"), ("node_modules/", "excluded_by_default")] {
        assert!(excluded.contains(&(path.to_string(), why.to_string())), "{path}: {:?}", excluded);
    }
    #[cfg(unix)]
    assert!(excluded.contains(&("link.txt".to_string(), "symlink".to_string())));
    assert_eq!(plan["estimate"]["item_count"], 2);
    assert_eq!(plan["estimate"]["total_size"], 7);

    let op = alice.create("home/alice/docs", json!({"access": "holder"})).await;
    assert!(op["share"]["obj_id"].as_str().unwrap().starts_with("cydir:"));
    let token = token_of(op["url"].as_str().unwrap());
    let meta: Value = srv.get(&format!("/s/{token}/meta")).send().await.unwrap().json().await.unwrap();
    assert_eq!(meta["kind"], "folder");
    let names: Vec<&str> = meta["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["sub", "a.txt"]);
    assert_eq!(meta["items"][0]["size"], 4);
    let sub: Value = srv.get(&format!("/s/{token}/meta?p=sub")).send().await.unwrap().json().await.unwrap();
    assert_eq!(sub["items"][0]["name"], "b.txt");
    assert_eq!(srv.get(&format!("/s/{token}/d/sub/b.txt")).send().await.unwrap().text().await.unwrap(), "bbbb");
    assert_eq!(srv.get(&format!("/s/{token}/d/.env")).send().await.unwrap().status(), 404);
    assert_eq!(srv.get(&format!("/s/{token}/d/sub/..%2Fa.txt")).send().await.unwrap().status(), 404);
    assert_eq!(srv.get(&format!("/s/{token}/d")).send().await.unwrap().status(), 404, "a folder has no content");
}

#[tokio::test]
async fn permissions_and_live_caps() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let denied = alice
        .err("share.create", json!({"source": {"path": "home/bob/secret.txt"}, "policy": {"access": "holder"}, "idempotency_key": "k1"}))
        .await;
    assert_eq!(denied["code"], "PERMISSION_DENIED");
    assert_eq!(denied["reason"], "source_not_readable");
    assert_eq!(alice.err("share.plan", json!({"source": {"path": "home"}})).await["code"], "PERMISSION_DENIED");

    // Zone services and node devices (the CLI on a node) act for the zone owner.
    let device = srv
        .client_with(Some(token_for(
            "ood1",
            TokenPrincipalKind::Device,
            AuthTarget::system(SystemServiceId::parse(CONTROL_PANEL_SERVICE_NAME).unwrap()),
        )))
        .await;
    let op = device
        .ok("share.create", json!({"source": {"path": "home/alice/docs/a.txt"}, "policy": {"access": "holder"}, "idempotency_key": "dev-1", "source_hint": "cli"}))
        .await;
    assert_eq!(op["share"]["owner_user_id"], "owner");
    let owner = srv.client("owner").await;
    let got = owner.ok("share.get", json!({"share_id": op["share_id"]})).await;
    assert_eq!(got["audit"][0]["actor_user_id"], "system");
    assert_eq!(got["audit"][0]["source"], "cli");

    // Anonymous callers cannot use share.*.
    let anon = srv.client_with(None).await;
    assert_eq!(anon.err("share.list", json!({})).await["reason"], "unauthenticated");

    // A live-subtree cap from the low-level `grant` never opens /s/* (design §3.5).
    let seq = alice.seq.fetch_add(1, Ordering::SeqCst);
    let mut seq_body = json!({"session": alice.session, "seq": seq, "args": {"subtree": {"path": "home/alice/docs"}}});
    seq_body["args"]["ops"] = json!(["read", "list"]);
    let granted = alice.post("grant", seq_body).await;
    assert_eq!(granted["ok"], true, "{}", granted);
    let live = granted["result"]["token"].as_str().unwrap();
    assert_eq!(srv.get(&format!("/s/{live}")).send().await.unwrap().status(), 404);

    // App sessions act for their user on share.* only (D13).
    let app = srv
        .client_with(Some(token_for(
            "alice",
            TokenPrincipalKind::App,
            AuthTarget::from_canonical_key("app:jarvis@alice").unwrap(),
        )))
        .await;
    let op = app.create("home/alice/report.txt", json!({"access": "holder"})).await;
    assert_eq!(op["share"]["created_by_app"], "app:jarvis@alice");
    let got = alice.ok("share.get", json!({"share_id": op["share_id"]})).await;
    assert_eq!(got["audit"][0]["actor_app_id"], "app:jarvis@alice");
    assert_eq!(got["audit"][0]["source"], "agent");
    let listing = app.call("list", json!({})).await;
    assert_eq!(listing["ok"], false);
    assert_eq!(listing["error"]["reason"], "delegated_session");
    let read = app.err("share.plan", json!({"source": {"path": "home/bob/secret.txt"}})).await;
    assert_eq!(read["reason"], "source_not_readable");
}

#[tokio::test]
async fn idempotency_and_plan_staleness() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let args = json!({"source": {"path": "home/alice/report.txt"}, "policy": {"access": "holder", "expires_at": null}, "idempotency_key": "same"});
    let first = alice.ok("share.create", args.clone()).await;
    let again = alice.ok("share.create", args.clone()).await;
    assert_eq!(first["operation_id"], again["operation_id"]);
    assert_eq!(first["share_id"], again["share_id"]);
    assert_eq!(alice.ok("share.list", json!({})).await["items"].as_array().unwrap().len(), 1);
    let mut other = args.clone();
    other["policy"]["access"] = json!("password");
    other["policy"]["password"] = json!("x");
    assert_eq!(alice.err("share.create", other).await["reason"], "idempotency_mismatch");

    let plan = alice.ok("share.plan", json!({"source": {"path": "home/alice/docs"}})).await;
    std::fs::write(srv.data.join("home/alice/docs/new.txt"), b"new").unwrap();
    let stale = alice
        .err(
            "share.create",
            json!({"source": {"path": "home/alice/docs"}, "policy": {"access": "holder"}, "idempotency_key": "k2", "plan_fingerprint": plan["plan_fingerprint"]}),
        )
        .await;
    assert_eq!(stale["reason"], "plan_stale");
}

#[tokio::test]
async fn background_snapshot_and_chunk_list() {
    let srv = start_with(|c| c.share.sync_threshold = 1024).await;
    // Larger than one 32 MiB chunk: stored as a chunk list.
    let big: Vec<u8> = (0..(33 * 1024 * 1024u32)).map(|i| (i % 251) as u8).collect();
    std::fs::write(srv.data.join("home/alice/big.bin"), &big).unwrap();
    let alice = srv.client("alice").await;
    let op = alice
        .ok("share.create", json!({"source": {"path": "home/alice/big.bin"}, "policy": {"access": "holder"}, "idempotency_key": "big"}))
        .await;
    assert_eq!(op["state"], "running");
    let mut done = Value::Null;
    for _ in 0..300 {
        let cur = alice.ok("share.operation", json!({"operation_id": op["operation_id"]})).await;
        if cur["state"] != "running" {
            done = cur;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(done["state"], "completed", "{}", done);
    assert_eq!(done["progress"]["done_bytes"], big.len());
    let token = token_of(done["url"].as_str().unwrap());
    let boundary = 32 * 1024 * 1024u64;
    let range = format!("bytes={}-{}", boundary - 3, boundary + 2);
    let part = srv.get(&format!("/s/{token}/d")).header("Range", range).send().await.unwrap();
    assert_eq!(part.status(), 206);
    let bytes = part.bytes().await.unwrap();
    assert_eq!(&bytes[..], &big[(boundary - 3) as usize..(boundary + 3) as usize]);
    let full = srv.get(&format!("/s/{token}/d")).send().await.unwrap().bytes().await.unwrap();
    assert_eq!(full.len(), big.len());
    assert!(full[..] == big[..]);
}

#[tokio::test]
async fn expiry_sweeper_and_stats() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let now = buckyos_kit::buckyos_get_unix_timestamp() as i64;
    let op = alice.create("home/alice/photo.jpg", json!({"access": "holder", "expires_at": now + 2})).await;
    let token = token_of(op["url"].as_str().unwrap());
    let share_id = op["share_id"].clone();

    // Two page views from one visitor, one from another; one download.
    for ua in ["ua-1", "ua-1", "ua-2"] {
        let r = srv.get(&format!("/s/{token}")).header("User-Agent", ua).send().await.unwrap();
        assert_eq!(r.status(), 200);
    }
    let img = srv.get(&format!("/s/{token}/d")).header("User-Agent", "ua-1").send().await.unwrap();
    assert_eq!(img.headers()["content-type"], "image/jpeg");
    let _ = img.bytes().await.unwrap();
    // The inline preview above is not a download; a saved copy is.
    let saved = srv.get(&format!("/s/{token}/d?download=1")).header("User-Agent", "ua-1").send().await.unwrap();
    let _ = saved.bytes().await.unwrap();
    srv.state.share.flush_stats(&srv.state.db);
    let stats = alice.ok("share.stats", json!({"share_id": share_id, "granularity": "hour"})).await;
    assert_eq!(stats["totals"]["views"], 3);
    assert_eq!(stats["totals"]["downloads"], 1);
    assert_eq!(stats["totals"]["visitors"], 2);
    assert!(stats["totals"]["bytes"].as_i64().unwrap() > 0);
    assert_eq!(stats["unavailable"], json!(["public", "messagehub", "homestation"]));
    let all = alice.ok("share.stats", json!({})).await;
    assert_eq!(all["totals"]["views"], 3);
    assert_eq!(all["summary"]["active"], 1);
    assert_eq!(all["summary"]["expiring_soon"], 1);

    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(srv.get(&format!("/s/{token}")).send().await.unwrap().status(), 404, "expiry is checked per request");
    assert_eq!(alice.ok("share.get", json!({"share_id": share_id})).await["status"], "expired");
    nfs_server::share::sweep_once(&srv.state).await;
    let got = alice.ok("share.get", json!({"share_id": share_id})).await;
    assert_eq!(got["status"], "expired");
    assert!(got["retention"][0]["released_at"].is_i64());
    assert_eq!(got["audit"][0]["action"], "expire");
    assert_eq!(got["audit"][0]["source"], "system");
    let upd = alice.err("share.update", json!({"share_id": share_id, "revision": got["revision"], "changes": {"title": "x"}})).await;
    assert_eq!(upd["reason"], "share_not_active");
}

#[tokio::test]
async fn app_handoffs() {
    let srv = start().await;
    let alice = srv.client("alice").await;

    // An image fits HomeStation as an attachment.
    let plan = alice.ok("share.plan", json!({"intent": "app_handoff", "source": {"path": "home/alice/photo.jpg"}, "target": "homestation"})).await;
    assert_eq!(plan["target_fit"]["mode"], "attachment");
    let op = alice
        .ok("share.app.handoff", json!({"source": {"path": "home/alice/photo.jpg"}, "target_app": "homestation", "idempotency_key": "h1"}))
        .await;
    let h = &op["handoff"];
    assert_eq!(h["mode"], "attachment");
    assert_eq!(h["kind"], "image");
    assert_eq!(h["launch"]["appId"], "homestation");
    let att = &h["launch"]["payload"]["compose"]["attachments"][0];
    assert_eq!(att["object"], h["obj_id"]);
    assert_eq!(att["status"], "uploaded");
    assert_eq!(op["share"]["kind"], "app_delivery");
    assert_eq!(op["share"]["delivery_state"], "handed_off");
    let store = srv.state.share.store(&srv.state.config).await.unwrap();
    let file_id = ndn_lib::ObjId::new(h["obj_id"].as_str().unwrap()).unwrap();
    let body = store.get_object(&file_id).await.unwrap();
    let file: ndn_lib::FileObject = serde_json::from_str(&body).unwrap();
    assert!(!file.content.starts_with("clist"), "HomeStation needs a single chunk");
    assert_eq!(file.meta["mime"], "image/jpeg");

    // Reports: newer facts win, stale ones are refused.
    let share_id = h["share_id"].clone();
    let t = buckyos_kit::buckyos_get_unix_timestamp() as i64 * 1000;
    alice
        .ok("share.app.report", json!({"share_id": share_id, "delivery_state": "confirmed", "business_ref": {"entry": "e1"}, "observed_at": t}))
        .await;
    let stale = alice.err("share.app.report", json!({"share_id": share_id, "delivery_state": "canceled", "observed_at": t - 1000})).await;
    assert_eq!(stale["reason"], "stale_report");
    let got = alice.ok("share.get", json!({"share_id": share_id})).await;
    assert_eq!(got["delivery_state"], "confirmed");
    assert_eq!(got["business_ref"]["entry"], "e1");
    let bob = srv.client("bob").await;
    assert_eq!(bob.err("share.app.report", json!({"share_id": share_id, "delivery_state": "failed"})).await["code"], "NOT_FOUND");

    // A PDF does not fit HomeStation: a non-expiring holder link goes in the post (D14).
    let op = alice
        .ok("share.app.handoff", json!({"source": {"path": "home/alice/paper.pdf"}, "target_app": "homestation", "idempotency_key": "h2"}))
        .await;
    let h = &op["handoff"];
    assert_eq!(h["mode"], "link");
    let url = h["url"].as_str().unwrap();
    assert_eq!(h["launch"]["payload"]["compose"]["link"]["url"], url);
    let link = alice.ok("share.get", json!({"share_id": h["link_share_id"]})).await;
    assert_eq!(link["kind"], "link");
    assert!(link["expires_at"].is_null());
    assert_eq!(link["related_share_id"], h["share_id"]);
    let pdf = srv.get(&format!("/s/{}/d", token_of(url))).send().await.unwrap();
    assert_eq!(pdf.headers()["content-type"], "application/pdf");

    // Closing the composer without posting releases both shares.
    alice.ok("share.app.report", json!({"share_id": h["share_id"], "delivery_state": "canceled"})).await;
    assert_eq!(alice.ok("share.get", json!({"share_id": h["share_id"]})).await["status"], "stopped");
    assert_eq!(alice.ok("share.get", json!({"share_id": h["link_share_id"]})).await["status"], "stopped");
    assert_eq!(srv.get(&format!("/s/{}", token_of(url))).send().await.unwrap().status(), 404);

    // A posted link stays a link (D17): stopping the link share ends it even though the post was confirmed.
    let op = alice
        .ok("share.app.handoff", json!({"source": {"path": "home/alice/paper.pdf"}, "target_app": "homestation", "idempotency_key": "h3"}))
        .await;
    let h = &op["handoff"];
    let url = h["url"].as_str().unwrap();
    alice.ok("share.app.report", json!({"share_id": h["share_id"], "delivery_state": "confirmed", "business_ref": {"entry": "e3"}})).await;
    assert_eq!(srv.get(&format!("/s/{}", token_of(url))).send().await.unwrap().status(), 200);
    let link = alice.ok("share.get", json!({"share_id": h["link_share_id"]})).await;
    alice.ok("share.stop", json!({"share_id": h["link_share_id"], "revision": link["revision"]})).await;
    assert_eq!(srv.get(&format!("/s/{}", token_of(url))).send().await.unwrap().status(), 404);
    assert_eq!(alice.ok("share.get", json!({"share_id": h["share_id"]})).await["delivery_state"], "confirmed");

    // MessageHub takes files as data_obj attachments, folders as links.
    let op = alice
        .ok("share.app.handoff", json!({"source": {"path": "home/alice/report.txt"}, "target_app": "messagehub", "idempotency_key": "m1"}))
        .await;
    assert_eq!(op["handoff"]["launch"]["payload"]["share"]["objId"], op["handoff"]["obj_id"]);
    let op = alice
        .ok("share.app.handoff", json!({"source": {"path": "home/alice/docs"}, "target_app": "messagehub", "idempotency_key": "m2"}))
        .await;
    assert_eq!(op["handoff"]["mode"], "link");
    assert!(op["handoff"]["launch"]["payload"]["share"]["url"].is_string());
    assert_eq!(
        alice.err("share.app.handoff", json!({"source": {"path": "home/alice/report.txt"}, "target_app": "mail", "idempotency_key": "x"})).await["reason"],
        "target_unsupported"
    );
}

#[tokio::test]
async fn html_never_renders_and_list_filters() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let op = alice.create("home/alice/page.html", json!({"access": "holder"})).await;
    let token = token_of(op["url"].as_str().unwrap());
    let r = srv.get(&format!("/s/{token}/d")).send().await.unwrap();
    assert_eq!(r.headers()["content-type"], "application/octet-stream");
    assert!(r.headers()["content-disposition"].to_str().unwrap().starts_with("attachment"));
    assert!(r.headers()["content-security-policy"].to_str().unwrap().starts_with("sandbox"));

    alice.create("home/alice/report.txt", json!({"access": "holder"})).await;
    alice.ok("share.app.handoff", json!({"source": {"path": "home/alice/photo.jpg"}, "target_app": "messagehub", "idempotency_key": "m"})).await;
    let links = alice.ok("share.list", json!({"kind": "link"})).await;
    assert_eq!(links["items"].as_array().unwrap().len(), 2);
    let apps = alice.ok("share.list", json!({"kind": "app_delivery"})).await;
    assert_eq!(apps["items"][0]["target_app"], "messagehub");
    let found = alice.ok("share.list", json!({"q": "page"})).await;
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    let paged = alice.ok("share.list", json!({"limit": 2})).await;
    assert_eq!(paged["items"].as_array().unwrap().len(), 2);
    let rest = alice.ok("share.list", json!({"limit": 2, "cursor": paged["next_cursor"]})).await;
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert!(rest["next_cursor"].is_null());

    let share_id = found["items"][0]["share_id"].clone();
    let hidden = alice.ok("share.hide", json!({"share_id": share_id, "hidden": true})).await;
    assert_eq!(hidden["hidden"], true);
    assert_eq!(alice.ok("share.list", json!({"q": "page"})).await["items"], json!([]));
    assert_eq!(alice.ok("share.list", json!({"q": "page", "include_hidden": true})).await["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn public_projection_and_host_guard() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let listed = alice.ok("share.public.list", json!({"refresh": true})).await;
    assert_eq!(listed["scan_ok"], true);
    let entries = listed["entries"].as_array().unwrap();
    let find = |p: &str| entries.iter().find(|e| e["rel_path"] == p).cloned().unwrap_or(Value::Null);
    assert_eq!(find("index.html")["url"], "https://public.test.zone/index.html");
    assert_eq!(find("site/app.js")["entry_kind"], "file");
    #[cfg(unix)]
    {
        assert_eq!(find("escape")["link_ok"], false, "links leaving srv/publish|library are flagged");
        assert_eq!(find("inside")["link_ok"], true);
    }
    std::fs::remove_file(srv.data.join("srv/publish/site/app.js")).unwrap();
    let again = alice.ok("share.public.list", json!({"refresh": true})).await;
    let gone = again["entries"].as_array().unwrap().iter().find(|e| e["rel_path"] == "site/app.js").unwrap().clone();
    assert!(gone["missing_since"].is_i64());

    // Files under srv/publish get a `public` access URL.
    let owner = srv.client("owner").await;
    let info = owner
        .post("stat", json!({"session": owner.session, "at": {"path": "srv/publish/index.html"}, "want": ["access"]}))
        .await;
    let urls = info["result"]["access_urls"].as_array().unwrap();
    assert!(urls.iter().any(|u| u["kind"] == "public" && u["url"] == "https://public.test.zone/index.html"), "{:?}", urls);
    let plan = owner.ok("share.plan", json!({"source": {"path": "srv/publish/index.html"}})).await;
    assert_eq!(plan["source_public"]["in_publish_dir"], true);
    assert!(plan["warnings"].as_array().unwrap().iter().any(|w| w == "already_public"));

    // On share.<zone>, only /s/* answers.
    let blocked = reqwest::Client::new()
        .post(format!("{}/nfs/v1/hello", srv.base))
        .header("X-Forwarded-Host", "share.test.zone")
        .json(&json!({"args": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 404);
}

async fn local_store(state: &SharedState) -> std::sync::Arc<tokio::sync::Mutex<named_store::NamedStore>> {
    let mgr = state.share.store(&state.config).await.unwrap();
    let id = mgr.get_store_ids().await.pop().unwrap();
    mgr.get_store(&id).await.unwrap()
}

async fn content_chunk(state: &SharedState, file: &ndn_lib::ObjId) -> ndn_lib::ObjId {
    let mgr = state.share.store(&state.config).await.unwrap();
    let body: Value = serde_json::from_str(&mgr.get_object(file).await.unwrap()).unwrap();
    ndn_lib::ObjId::new(body["content"].as_str().unwrap()).unwrap()
}

/// One GC pass as a deployment would run it: deliver pending pin cascades, then evict.
async fn run_gc(state: &SharedState) -> named_store::GcReport {
    let store = local_store(state).await;
    let store = store.lock().await;
    loop {
        let batch = store.fetch_outbox_ready(100).await.unwrap();
        if batch.is_empty() {
            break;
        }
        for entry in batch {
            store.apply_edge(entry.msg.clone()).await.unwrap();
            store.complete_outbox_entry(entry.seq).await.unwrap();
        }
    }
    store.gc_round(0).await.unwrap()
}

#[tokio::test]
async fn unpin_failure_keeps_stopping_until_sweeper_releases() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let first = alice.create("home/alice/report.txt", json!({"access": "holder"})).await;
    let second = alice.create("home/alice/report.txt", json!({"access": "holder"})).await;
    let obj_id = first["share"]["obj_id"].as_str().unwrap().to_string();
    assert_eq!(second["share"]["obj_id"], obj_id.as_str(), "same content, same snapshot");
    let (a, b) = (first["share_id"].as_str().unwrap().to_string(), second["share_id"].as_str().unwrap().to_string());
    let (ta, tb) = (token_of(first["url"].as_str().unwrap()), token_of(second["url"].as_str().unwrap()));
    let oid = ndn_lib::ObjId::new(&obj_id).unwrap();
    let pin_of = |id: &str| format!("share:{id}");

    // Another writer holds the named store database: unpin fails with "database is locked".
    let blocker = rusqlite::Connection::open(srv.data.parent().unwrap().join("store/named_store.db")).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();

    let stopped = alice.ok("share.stop", json!({"share_id": a, "revision": 1})).await;
    assert_eq!(stopped["status"], "stopping");
    for path in [format!("/s/{ta}"), format!("/s/{ta}/meta"), format!("/s/{ta}/d")] {
        assert_eq!(srv.get(&path).send().await.unwrap().status(), 404, "visitors lose access at once: {path}");
    }
    nfs_server::share::sweep_once(&srv.state).await;
    let got = alice.ok("share.get", json!({"share_id": a})).await;
    assert_eq!(got["status"], "stopping", "a failed retry keeps the share stopping");
    assert!(got["retention"][0]["released_at"].is_null());
    let mgr = srv.state.share.store(&srv.state.config).await.unwrap();
    assert!(mgr.anchor_state(&oid, &pin_of(&a)).await.is_ok(), "pin still held");

    // Storage recovers; a restarted server's sweeper finishes the stop.
    blocker.execute_batch("ROLLBACK").unwrap();
    drop(blocker);
    let restarted = AppState::new(srv.state.config.clone()).unwrap();
    nfs_server::share::sweep_once(&restarted).await;
    nfs_server::share::sweep_once(&restarted).await;
    let got = alice.ok("share.get", json!({"share_id": a})).await;
    assert_eq!(got["status"], "stopped");
    assert!(got["retention"][0]["released_at"].is_i64());
    assert_eq!(got["audit"].as_array().unwrap().iter().filter(|e| e["action"] == "stop").count(), 1);
    let mgr = restarted.share.store(&restarted.config).await.unwrap();
    assert!(matches!(mgr.anchor_state(&oid, &pin_of(&a)).await, Err(ndn_lib::NdnError::NotFound(_))));
    assert!(mgr.anchor_state(&oid, &pin_of(&b)).await.is_ok(), "the other share keeps its own pin");

    // GC with the remaining share's pin: the snapshot and its chunk stay protected.
    let report = run_gc(&restarted).await;
    assert_eq!(report.evicted_objects, 0, "{report:?}");
    let chunk = content_chunk(&restarted, &oid).await;
    assert_eq!(mgr.debug_dump_expand_state(&oid).await.unwrap().eviction_class, 2);
    assert_eq!(mgr.debug_dump_expand_state(&chunk).await.unwrap().eviction_class, 1);
    let body = srv.get(&format!("/s/{tb}/d")).send().await.unwrap();
    assert_eq!(body.text().await.unwrap(), "hello share world");

    // Once the last share stops, the snapshot is collected and its chunk loses protection.
    assert_eq!(alice.ok("share.stop", json!({"share_id": b, "revision": 1})).await["status"], "stopped");
    assert_eq!(run_gc(&restarted).await.evicted_objects, 1);
    assert!(!mgr.is_object_exist(&oid).await.unwrap());
    assert_eq!(mgr.debug_dump_expand_state(&chunk).await.unwrap().eviction_class, 0);
}
