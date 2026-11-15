//! `reflow.<zone>` (Agent Browser Protocol §5.4): the signed-in user's reflow
//! library, the bucket `user/control-panel/<user>/reflow` as control-panel
//! declares it, served as pages. RBAC is the real model and default policy;
//! tokens use the test key of tests/ndm.rs.

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
const ZONE: &str = "test.zone";
const SITE: &str = "www-example-com";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// The groups the scheduler gives users, services and the Apps Agents run in.
async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = "g, owner, admin\ng, alice, users\ng, bob, users\ng, system:control-panel, system\ng, system:nfs-server, frame\n\
            g, app:writer, agent\ng, app:writer, agent_runtime\ng, app:writer, agent_read_write\n\
            g, app:reader, agent\ng, app:reader, agent_runtime\ng, app:reader, agent_read_only\n";
        let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, groups);
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim()).await.unwrap();
    })
    .await;
}

fn token(sub: &str, kind: TokenPrincipalKind, target: &str) -> String {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(sub.into()),
        appid: None,
        sudo: false,
        extra: Default::default(),
    };
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(&mut token, &AuthTarget::from_canonical_key(target).unwrap(), TokenUse::Session).unwrap();
    token.generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap()).unwrap()
}

/// The user in the Desktop.
fn desktop(user: &str) -> String {
    token(user, TokenPrincipalKind::User, "system:control-panel")
}

/// What `/sso_callback` plants on `reflow.<zone>`: the Desktop session delegated to nfs-server.
fn signed_in(user: &str) -> String {
    token(user, TokenPrincipalKind::User, "system:nfs-server")
}

/// The session an Agent's commands run with: its App, for the App's owner.
fn agent(owner: &str, app: &str) -> String {
    token(owner, TokenPrincipalKind::App, &format!("app:{app}@{owner}"))
}

struct Server {
    base: String,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn start() -> Server {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("exports/home");
    std::fs::create_dir_all(&home).unwrap();
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: home }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.share.zone_host = Some(ZONE.into());
    config.bucket_specs = serde_json::from_value(json!({
        (CONTROL_PANEL_SERVICE_NAME): {"buckets": control_panel_bucket_decls(), "storage_quota_mib": CONTROL_PANEL_STORAGE_QUOTA_MIB}
    }))
    .unwrap();
    let state = nfs_server::state::AppState::new(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::server::build_router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    Server { base, http, _tmp: tmp }
}

fn library(user: &str) -> String {
    format!("user/{CONTROL_PANEL_SERVICE_NAME}/{user}/{CONTROL_PANEL_REFLOW_BUCKET}")
}

impl Server {
    /// The `/bucket/v1` data plane on the Zone host.
    fn data(&self, method: Method, user: &str, key: &str, auth: &str) -> reqwest::RequestBuilder {
        self.http.request(method, format!("{}/bucket/v1/{}/{key}", self.base, library(user))).bearer_auth(auth)
    }

    async fn put(&self, user: &str, key: &str, content_type: Option<&str>, body: &'static str, auth: &str) -> StatusCode {
        let mut req = self.data(Method::PUT, user, key, auth).body(body);
        if let Some(t) = content_type {
            req = req.header("content-type", t);
        }
        req.send().await.unwrap().status()
    }

    /// A request the gateway forwarded from `reflow.<zone>`.
    fn on(&self, host: &str, method: Method, path: &str, session: Option<&str>) -> reqwest::RequestBuilder {
        let req = self.http.request(method, format!("{}{path}", self.base)).header("x-forwarded-host", host);
        match session {
            Some(t) => req.header("cookie", format!("buckyos_session_token={t}")),
            None => req,
        }
    }

    fn page(&self, path: &str, session: Option<&str>) -> reqwest::RequestBuilder {
        self.on(&format!("reflow.{ZONE}"), Method::GET, path, session)
    }

    async fn status(&self, path: &str, session: Option<&str>) -> StatusCode {
        self.page(path, session).send().await.unwrap().status()
    }

    /// The path exactly as written: clients normalize `..` away before sending.
    async fn raw_status(&self, path: &str, session: &str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(self.base.trim_start_matches("http://")).await.unwrap();
        let request = format!("GET {path} HTTP/1.1\r\nHost: reflow.{ZONE}\r\nCookie: buckyos_session_token={session}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response.split_whitespace().nth(1).unwrap().parse().unwrap()
    }
}

const ALICE_PAGE: &str = "<!doctype html><title>alice</title><script type=module src=assets/app.js></script>";
const BOB_PAGE: &str = "<!doctype html><title>bob</title>";

async fn fill(srv: &Server) {
    let alice = desktop("alice");
    for (key, content_type, body) in [
        ("www-example-com/index.html", Some("text/html; charset=utf-8"), ALICE_PAGE),
        ("www-example-com/assets/app.js", None, "document.title += '!'"),
        ("www-example-com/results/0123456789abcdef.json", Some("application/json"), r#"{"data":{"title":"T"}}"#),
        ("www-example-com/fallback/0123456789abcdef.html", Some("application/octet-stream"), "<p>fallback</p>"),
    ] {
        assert_eq!(srv.put("alice", key, content_type, body, &alice).await, StatusCode::CREATED, "{key}");
    }
    let put = srv.put("bob", "www-example-com/index.html", Some("text/html; charset=utf-8"), BOB_PAGE, &desktop("bob")).await;
    assert_eq!(put, StatusCode::CREATED);
}

#[tokio::test]
async fn pages_are_the_signed_in_users_library_served_as_what_they_are() {
    let srv = start().await;
    fill(&srv).await;
    let alice = signed_in("alice");

    // A directory is its index.html: shown in place, with its own type, free to run.
    let page = srv.page("/www-example-com/?url=https%3A%2F%2Fwww.example.com%2F&page=0123456789abcdef", Some(&alice)).send().await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let headers = page.headers().clone();
    assert_eq!(headers["content-type"], "text/html; charset=utf-8");
    assert!(!headers.contains_key("content-disposition"), "{headers:?}");
    assert!(!headers.contains_key("content-security-policy"), "{headers:?}");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["cache-control"], "private, no-cache");
    assert_eq!(page.text().await.unwrap(), ALICE_PAGE);

    // What the page loads: a script without a declared type, a result, a fallback page stored as bytes.
    for (path, content_type) in [
        ("/www-example-com/index.html", "text/html; charset=utf-8"),
        ("/www-example-com/assets/app.js", "text/javascript"),
        ("/www-example-com/results/0123456789abcdef.json", "application/json"),
        ("/www-example-com/fallback/0123456789abcdef.html", "text/html; charset=utf-8"),
    ] {
        let r = srv.page(path, Some(&alice)).send().await.unwrap();
        assert_eq!((r.status(), r.headers()["content-type"].to_str().unwrap()), (StatusCode::OK, content_type), "{path}");
        assert!(!r.headers().contains_key("content-disposition") && !r.headers().contains_key("content-security-policy"), "{path}");
    }

    // The same bytes on the data plane stay a download that cannot run.
    let download = srv.data(Method::GET, "alice", "www-example-com/index.html", &desktop("alice")).send().await.unwrap();
    assert_eq!(download.headers()["content-type"], "application/octet-stream");
    assert!(download.headers()["content-disposition"].to_str().unwrap().starts_with("attachment"));
    assert!(download.headers()["content-security-policy"].to_str().unwrap().starts_with("sandbox"));

    // `/<site>` is the directory `/<site>/`.
    let redirect = srv.page("/www-example-com?url=https%3A%2F%2Fwww.example.com%2F", Some(&alice)).send().await.unwrap();
    assert_eq!(redirect.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(redirect.headers()["location"], "/www-example-com/?url=https%3A%2F%2Fwww.example.com%2F");

    // HEAD, revalidation and ranges are those of the bucket.
    let head = srv.on(&format!("reflow.{ZONE}"), Method::HEAD, "/www-example-com/", Some(&alice)).send().await.unwrap();
    assert_eq!((head.status(), head.headers()["content-length"].to_str().unwrap()), (StatusCode::OK, ALICE_PAGE.len().to_string().as_str()));
    let etag = head.headers()["etag"].clone();
    assert_eq!(head.text().await.unwrap(), "");
    let fresh = srv.page("/www-example-com/", Some(&alice)).header("if-none-match", etag).send().await.unwrap();
    assert_eq!(fresh.status(), StatusCode::NOT_MODIFIED);
    let range = srv.page("/www-example-com/", Some(&alice)).header("range", "bytes=0-8").send().await.unwrap();
    assert_eq!((range.status(), range.text().await.unwrap().as_str()), (StatusCode::PARTIAL_CONTENT, "<!doctype"));

    // The gateway's dash form of the host, and a Bearer instead of the cookie.
    assert_eq!(srv.on(&format!("reflow-{ZONE}"), Method::GET, "/www-example-com/", Some(&alice)).send().await.unwrap().status(), StatusCode::OK);
    let bearer = srv.page("/www-example-com/", None).bearer_auth(&alice).send().await.unwrap();
    assert_eq!(bearer.status(), StatusCode::OK);

    // Missing keys, and a user without a library.
    assert_eq!(srv.status("/www-example-com/nothing.js", Some(&alice)).await, StatusCode::NOT_FOUND);
    assert_eq!(srv.status("/other-site/", Some(&alice)).await, StatusCode::NOT_FOUND);
    assert_eq!(srv.status("/", Some(&alice)).await, StatusCode::NOT_FOUND);
    assert_eq!(srv.status("/www-example-com/", Some(&signed_in("owner"))).await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn each_user_sees_their_own_library_and_nobody_else_is_signed_in() {
    let srv = start().await;
    fill(&srv).await;

    // The same address is another library for another user.
    let bob = srv.page("/www-example-com/", Some(&signed_in("bob"))).send().await.unwrap();
    assert_eq!((bob.status(), bob.text().await.unwrap().as_str()), (StatusCode::OK, BOB_PAGE));
    assert_eq!(srv.status("/www-example-com/assets/app.js", Some(&signed_in("bob"))).await, StatusCode::NOT_FOUND);

    // Only the session this host was given signs a user in: not the Desktop's own, not an
    // App's or an Agent's for the same user, not a broken one.
    assert_eq!(srv.status("/www-example-com/", None).await, StatusCode::UNAUTHORIZED);
    for session in [
        desktop("alice"),
        desktop("owner"),
        token("alice", TokenPrincipalKind::User, "app:writer@alice"),
        agent("alice", "writer"),
        token("alice", TokenPrincipalKind::System, "system:nfs-server"),
        format!("{}x", signed_in("alice")),
    ] {
        assert_eq!(srv.status("/www-example-com/", Some(&session)).await, StatusCode::UNAUTHORIZED);
        let bearer = srv.page("/www-example-com/", None).bearer_auth(&session).send().await.unwrap();
        assert_eq!(bearer.status(), StatusCode::UNAUTHORIZED);
    }
    let refused = srv.page("/www-example-com/", None).send().await.unwrap();
    assert_eq!(refused.headers()["cache-control"], "no-store");
}

#[tokio::test]
async fn the_host_is_nothing_but_the_library() {
    let srv = start().await;
    fill(&srv).await;
    let alice = signed_in("alice");
    let host = format!("reflow.{ZONE}");

    // Paths never leave the library, however they are written.
    assert_eq!(srv.raw_status("/www-example-com/index.html", &alice).await, 200);
    for path in [
        "/www-example-com/../www-example-com/index.html",
        "/www-example-com/%2e%2e/www-example-com/index.html",
        "/www-example-com/..%2findex.html",
        "/../../user/control-panel/bob/reflow/www-example-com/index.html",
        "/www-example-com//index.html",
        "//www-example-com/index.html",
        "/www-example-com/./index.html",
        "/www-example-com/a%5c..%5cindex.html",
    ] {
        assert_eq!(srv.raw_status(path, &alice).await, 404, "{path}");
    }

    // The file, bucket and S3 APIs of this service do not answer on this host.
    let desktop = desktop("alice");
    for (method, path) in [
        (Method::POST, "/nfs/v1/hello"),
        (Method::POST, "/kapi/nfs-server"),
        (Method::PUT, "/bucket/v1/user/control-panel/alice/reflow/www-example-com/index.html"),
        (Method::DELETE, "/www-example-com/index.html"),
    ] {
        let r = srv.on(&host, method.clone(), path, Some(&alice)).bearer_auth(&desktop).body("{}").send().await.unwrap();
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
    }
    let key = format!("/bucket/v1/{}/www-example-com/index.html", library("alice"));
    assert_eq!(srv.on(&host, Method::GET, &key, Some(&alice)).bearer_auth(&desktop).send().await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_eq!(srv.status("/www-example-com/index.html", Some(&alice)).await, StatusCode::OK, "nothing was changed");

    // On the Zone host the library has no pages.
    let zone = srv.on(ZONE, Method::GET, "/www-example-com/", Some(&alice)).send().await.unwrap();
    assert_eq!(zone.status(), StatusCode::NOT_FOUND);
}

/// The origin runs page code, so what it holds must not be worth stealing: the session signs a user in
/// on `reflow.<zone>` and is nobody on the file, bucket and upload APIs — the Zone owner's included.
#[tokio::test]
async fn the_session_of_this_host_opens_nothing_else() {
    let srv = start().await;
    fill(&srv).await;
    let hello = json!({"args": {"versions": ["nfsp/0"]}});
    let key = format!("/bucket/v1/{}/www-example-com/index.html", library("alice"));
    let rpc = |token: &str| json!({"method": "bucket.list", "params": {}, "sys": [1, token]});

    for user in ["alice", "owner"] {
        let page = signed_in(user);
        // As a Bearer it is refused.
        let files = srv.http.post(format!("{}/nfs/v1/hello", srv.base)).bearer_auth(&page).json(&hello).send().await.unwrap();
        assert_eq!(files.status(), StatusCode::FORBIDDEN, "{user}");
        assert_eq!(files.json::<Value>().await.unwrap()["error"]["reason"], "unauthenticated");
        let data = srv.http.get(format!("{}{key}", srv.base)).bearer_auth(&page).send().await.unwrap();
        assert_eq!(data.status(), StatusCode::FORBIDDEN, "{user}");
        let write = srv.http.put(format!("{}{key}", srv.base)).bearer_auth(&page).body("x").send().await.unwrap();
        assert_eq!(write.status(), StatusCode::FORBIDDEN, "{user}");
        let listed: Value = srv.http.post(format!("{}/kapi/nfs-server", srv.base)).json(&rpc(&page)).send().await.unwrap().json().await.unwrap();
        assert_eq!(listed["code"], "permission_denied", "{listed}");
        let upload = srv.http.get(format!("{}/ndm/v1/objects/lookup?obj_id=x", srv.base)).bearer_auth(&page).send().await.unwrap();
        assert_eq!(upload.status(), StatusCode::UNAUTHORIZED, "{user}");

        // As a cookie (a request the gateway routed by path) it is an anonymous request.
        let cookie = format!("buckyos_session_token={page}");
        let files: Value = srv.http.post(format!("{}/nfs/v1/hello", srv.base)).header("cookie", &cookie).json(&hello).send().await.unwrap().json().await.unwrap();
        assert_eq!(files["result"]["identity"]["role"], "anonymous", "{files}");
        let data = srv.http.get(format!("{}{key}", srv.base)).header("cookie", &cookie).send().await.unwrap();
        assert_eq!(data.status(), StatusCode::FORBIDDEN, "{user}");
    }

    // The Desktop's own session is what these APIs know.
    let desktop = desktop("alice");
    let files: Value = srv.http.post(format!("{}/nfs/v1/hello", srv.base)).bearer_auth(&desktop).json(&hello).send().await.unwrap().json().await.unwrap();
    assert_eq!(files["result"]["identity"], json!({"user_id": "alice", "role": "user"}), "{files}");
    assert_eq!(srv.http.get(format!("{}{key}", srv.base)).bearer_auth(&desktop).send().await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn agents_write_the_library_by_their_permission_mode() {
    let srv = start().await;
    let (writer, reader) = (agent("alice", "writer"), agent("alice", "reader"));
    let key = "www-example-com/index.html";

    assert_eq!(srv.put("alice", key, Some("text/html; charset=utf-8"), ALICE_PAGE, &writer).await, StatusCode::CREATED);
    let page = srv.page("/www-example-com/", Some(&signed_in("alice"))).send().await.unwrap();
    assert_eq!((page.status(), page.text().await.unwrap().as_str()), (StatusCode::OK, ALICE_PAGE));

    // A read-only Agent reads the library and cannot change it.
    let read = srv.data(Method::GET, "alice", key, &reader).send().await.unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let refused = srv.data(Method::PUT, "alice", key, &reader).body(BOB_PAGE).send().await.unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(refused.json::<Value>().await.unwrap()["error"], "permission_denied");
    assert_eq!(srv.data(Method::DELETE, "alice", key, &reader).send().await.unwrap().status(), StatusCode::FORBIDDEN);

    // An Agent works for its owner only.
    assert_eq!(srv.put("bob", key, None, BOB_PAGE, &writer).await, StatusCode::FORBIDDEN);

    assert_eq!(srv.data(Method::DELETE, "alice", key, &writer).send().await.unwrap().status(), StatusCode::OK);
    assert_eq!(srv.status("/www-example-com/", Some(&signed_in("alice"))).await, StatusCode::NOT_FOUND);
}

/// The Agent's `reflow` command against this service, then the pages it made.
/// Needs Deno on PATH and the built SDK (`src/rootfs/libexec/buckyos-tool/dist`, from `buckyos-build.py`):
/// `cargo test -p nfs_server --test reflow -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_reflow_command_fills_the_library_this_host_serves() {
    let srv = start().await;
    let port = srv.base.rsplit(':').next().unwrap().to_string();
    let tool = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/buckyos-agent");
    let run = |app: &'static str, args: Vec<String>| {
        let (tool, port, session) = (tool.clone(), port.clone(), agent("alice", app));
        async move {
            tokio::task::spawn_blocking(move || {
                let output = std::process::Command::new("deno")
                    .args(["run", "-A", "--config"])
                    .arg(tool.join("deno.json"))
                    .arg(tool.join("aicc-tool.ts"))
                    .arg("reflow")
                    .args(&args)
                    .env("BUCKYOS_APPCLIENT_SESSION_TOKEN", session)
                    .env("BUCKYOS_HOST_GATEWAY", "127.0.0.1")
                    .env("BUCKYOS_NODE_GATEWAY_PORT", port)
                    .env("BUCKYOS_OWNER_USER_ID", "alice")
                    .env("BUCKYOS_APP_ID", app)
                    .env("OPENDAN_AGENT_DID", format!("did:bns:{app}.alice"))
                    .output()
                    .expect("deno");
                let stdout = String::from_utf8_lossy(&output.stdout);
                let result: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("{e}: {stdout} {}", String::from_utf8_lossy(&output.stderr)));
                (output.status.code().unwrap(), result)
            })
            .await
            .unwrap()
        }
    };
    let s = |v: &str| v.to_string();

    let work = tempfile::tempdir().unwrap();
    let package = work.path().join("pkg");
    std::fs::create_dir_all(package.join("assets")).unwrap();
    let manifest = json!({
        "schema": "buckyos.reflow/1", "site": SITE,
        "match": [{"host": "www.example.com", "path": "/news/*"}],
        "refresh": {"script": "refresh.mjs", "timeout_s": 60}
    });
    std::fs::write(package.join("manifest.json"), manifest.to_string()).unwrap();
    std::fs::write(package.join("refresh.mjs"), "export default async ({ page }) => ({ title: await page.title() })").unwrap();
    std::fs::write(package.join("index.html"), ALICE_PAGE).unwrap();
    std::fs::write(package.join("assets/app.js"), "document.title += '!'").unwrap();
    std::fs::write(work.path().join("data.json"), r#"{"title":"T"}"#).unwrap();
    std::fs::write(work.path().join("page.html"), "<p>fallback</p>").unwrap();
    let dir = package.to_str().unwrap().to_string();
    let url = "https://www.example.com/news/1?a=1#top";

    // The site has no package yet: what the user asked for in the App is kept beside it.
    let wish = r#"{"schema":"buckyos.reflow.preference/1","preference":"只看标题、日期和正文，手机单栏","updated_at":1794500000000}"#;
    assert_eq!(srv.put("alice", "www-example-com/preference.json", Some("application/json"), wish, &desktop("alice")).await, StatusCode::CREATED);
    let (code, asked) = run("reader", vec![s("match"), s(url)]).await;
    assert_eq!((code, asked["detail"]["preference"].as_str().unwrap()), (0, "只看标题、日期和正文，手机单栏"), "{asked}");
    assert!(asked["detail"]["package"].is_null(), "{asked}");

    // A read-only Agent is told what to do instead.
    let (code, refused) = run("reader", vec![s("put"), s(SITE), dir.clone()]).await;
    assert_eq!((code, refused["title"].as_str().unwrap()), (2, "reflow => permission_denied"), "{refused}");
    let summary = refused["summary"].as_str().unwrap();
    assert!(summary.contains(&dir) && summary.contains("ask the user to import it"), "{summary}");

    let (code, put) = run("writer", vec![s("put"), s(SITE), dir.clone()]).await;
    assert_eq!((code, &put["detail"]["files"]), (0, &json!(["assets/app.js", "index.html", "manifest.json", "refresh.mjs"])), "{put}");
    assert_eq!(put["detail"]["manifest"]["author"], json!({"kind": "agent", "id": "did:bns:writer.alice"}));
    // The manifest stated no preference: the user's went into it, and its file gave way to the package.
    assert_eq!(put["detail"]["manifest"]["preference"], "只看标题、日期和正文，手机单栏", "{put}");
    assert_eq!(put["detail"]["removed"], json!(["preference.json"]), "{put}");
    let gone = srv.data(Method::GET, "alice", "www-example-com/preference.json", &desktop("alice")).send().await.unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    let (code, matched) = run("reader", vec![s("match"), s(url)]).await;
    assert_eq!((code, matched["detail"]["site"].as_str().unwrap()), (0, SITE), "{matched}");
    assert!(matched["detail"]["package"]["updated_at"].as_u64().unwrap() > 0 && matched["detail"]["view"].is_null(), "{matched}");
    assert_eq!(matched["detail"]["package"]["preference"], matched["detail"]["preference"], "{matched}");

    let (code, kept) = run("writer", vec![s("result"), s("put"), s(url), work.path().join("data.json").to_str().unwrap().to_string()]).await;
    assert_eq!(code, 0, "{kept}");
    let view = kept["detail"]["state"]["view"].as_str().unwrap().to_string();
    let page_id = kept["detail"]["state"]["page"].as_str().unwrap().to_string();
    assert_eq!(view, format!("{SITE}/?url=https%3A%2F%2Fwww.example.com%2Fnews%2F1%3Fa%3D1&page={page_id}"));

    // What the App then loads in its frame.
    let alice = signed_in("alice");
    let page = srv.page(&format!("/{view}"), Some(&alice)).send().await.unwrap();
    assert_eq!((page.status(), page.headers()["content-type"].to_str().unwrap()), (StatusCode::OK, "text/html; charset=utf-8"));
    assert_eq!(page.text().await.unwrap(), ALICE_PAGE);
    let script = srv.page("/www-example-com/assets/app.js", Some(&alice)).send().await.unwrap();
    assert_eq!(script.headers()["content-type"], "text/javascript; charset=utf-8");
    let result: Value = srv.page(&format!("/{SITE}/results/{page_id}.json"), Some(&alice)).send().await.unwrap().json().await.unwrap();
    assert_eq!((result["schema"].as_str().unwrap(), &result["data"]), ("buckyos.reflow.result/1", &json!({"title": "T"})));
    assert_eq!((result["source_url"].as_str().unwrap(), result["made_by"].as_str().unwrap()), ("https://www.example.com/news/1?a=1", "did:bns:writer.alice"));

    // A page no package covers shows its fallback page.
    let other = "https://www.example.com/about";
    let (code, fallback) = run("writer", vec![s("result"), s("put"), s(other), work.path().join("page.html").to_str().unwrap().to_string()]).await;
    assert_eq!(code, 0, "{fallback}");
    let view = fallback["detail"]["state"]["view"].as_str().unwrap().to_string();
    let page = srv.page(&format!("/{view}"), Some(&alice)).send().await.unwrap();
    assert_eq!((page.status(), page.headers()["content-type"].to_str().unwrap()), (StatusCode::OK, "text/html; charset=utf-8"));

    let (code, listed) = run("reader", vec![s("ls")]).await;
    assert_eq!((code, listed["detail"]["sites"][0]["site"].as_str().unwrap()), (0, SITE), "{listed}");
    let copy = work.path().join("copy");
    let (code, got) = run("reader", vec![s("get"), s(SITE), copy.to_str().unwrap().to_string()]).await;
    assert_eq!((code, got["detail"]["files"].as_array().unwrap().len()), (0, 4), "{got}");
    assert_eq!(std::fs::read_to_string(copy.join("assets/app.js")).unwrap(), "document.title += '!'");

    let (code, removed) = run("writer", vec![s("rm"), s(SITE)]).await;
    assert_eq!((code, removed["detail"]["removed"].as_u64().unwrap()), (0, 7), "{removed}");
    assert_eq!(srv.status("/www-example-com/", Some(&alice)).await, StatusCode::NOT_FOUND);
}
