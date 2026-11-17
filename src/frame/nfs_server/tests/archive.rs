//! Archive browsing over real HTTP (notepads/archive-preview-files-todo.md):
//! path sources (`<file>/@/archive/...`), object sources (chat attachments as
//! `ObjectRef { inner_path: "/archive/..." }`), member reads (range, stream,
//! materialize), passwords, read-only rules, STALE and name encodings.

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::{AppState, SharedState};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use zip::unstable::write::FileOptionsExt;
use zip::write::SimpleFileOptions;

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = "g, owner, admin\ng, alice, users\ng, bob, users\ng, system:control-panel, system\n";
        let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, groups);
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim()).await.unwrap();
    })
    .await;
}

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
    bind_token_target(&mut token, &AuthTarget::system(SystemServiceId::parse(CONTROL_PANEL_SERVICE_NAME).unwrap()), TokenUse::Session).unwrap();
    token.generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap()).unwrap()
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
    token: String,
    session: String,
    seq: AtomicU64,
    lang: Option<&'static str>,
}

async fn start() -> Server {
    start_with(|_| {}).await
}

async fn start_with(tune: impl FnOnce(&mut ServerConfig)) -> Server {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("exports");
    for d in ["home/alice/docs", "home/bob"] {
        std::fs::create_dir_all(data.join(d)).unwrap();
    }
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: data.join("home") }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    tune(&mut config);
    let state = AppState::new(config).unwrap();
    let app = nfs_server::server::build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, data, state, _tmp: tmp }
}

impl Server {
    async fn client(&self, user: &str) -> Client {
        self.client_lang(user, None).await
    }

    async fn client_lang(&self, user: &str, lang: Option<&'static str>) -> Client {
        let mut c = Client { base: self.base.clone(), http: reqwest::Client::new(), token: token(user), session: String::new(), seq: AtomicU64::new(1), lang };
        let hello = c.post("hello", json!({"args": {"versions": ["nfsp/0"]}})).await;
        assert_eq!(hello["ok"], true, "{hello}");
        c.session = hello["result"]["session"].as_str().unwrap().to_string();
        c
    }

    fn write_zip(&self, rel: &str, bytes: &[u8]) {
        let p = self.data.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

impl Client {
    async fn post(&self, method: &str, body: Value) -> Value {
        let mut req = self.http.post(format!("{}/nfs/v1/{}", self.base, method)).json(&body).bearer_auth(&self.token);
        if let Some(l) = self.lang {
            req = req.header("Accept-Language", l);
        }
        req.send().await.unwrap().json().await.unwrap()
    }

    async fn call(&self, method: &str, mut body: Value) -> Value {
        body["session"] = json!(self.session);
        if method == "mkdir" || method == "delete" {
            body["seq"] = json!(self.seq.fetch_add(1, Ordering::SeqCst));
        }
        self.post(method, body).await
    }

    async fn ok(&self, method: &str, body: Value) -> Value {
        let v = self.call(method, body).await;
        assert_eq!(v["ok"], true, "{method}: {v}");
        v["result"].clone()
    }

    async fn err(&self, method: &str, body: Value) -> Value {
        let v = self.call(method, body).await;
        assert_eq!(v["ok"], false, "{method} should fail: {v}");
        v["error"].clone()
    }

    async fn read(&self, url: &str, range: Option<&str>) -> (u16, Vec<u8>, reqwest::header::HeaderMap) {
        let mut req = self.http.get(format!("{}{}", self.base, url)).bearer_auth(&self.token);
        if let Some(r) = range {
            req = req.header("Range", r);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        (status, resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default(), headers)
    }
}

fn read_url(info: &Value) -> String {
    info["access_urls"].as_array().unwrap().iter().find(|u| u["kind"] == "read").unwrap()["url"].as_str().unwrap().to_string()
}

fn big_text() -> Vec<u8> {
    (0..300_000u32).map(|i| b"abcdefghijklmnopqrstuvwxyz0123456789\n"[(i % 37) as usize]).collect()
}

fn zip_of(entries: &[(&str, &[u8], zip::CompressionMethod)], encrypt: Option<(&str, bool)>) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data, method) in entries {
        if name.ends_with('/') {
            w.add_directory(name.trim_end_matches('/'), SimpleFileOptions::default()).unwrap();
            continue;
        }
        let mut options = SimpleFileOptions::default().compression_method(*method);
        if let Some((pw, aes)) = encrypt {
            options = if aes { options.with_aes_encryption(zip::AesMode::Aes256, pw) } else { options.with_deprecated_encryption(pw.as_bytes()) };
        }
        w.start_file(*name, options).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// Rewrites an ASCII placeholder name to raw bytes (no UTF-8 flag): what a
/// CJK Windows zip tool writes.
fn rename_raw(mut bytes: Vec<u8>, placeholder: &[u8], raw: &[u8]) -> Vec<u8> {
    assert_eq!(placeholder.len(), raw.len());
    let mut i = 0;
    while i + placeholder.len() <= bytes.len() {
        if &bytes[i..i + placeholder.len()] == placeholder {
            bytes[i..i + placeholder.len()].copy_from_slice(raw);
            i += placeholder.len();
        } else {
            i += 1;
        }
    }
    bytes
}

fn sample() -> Vec<u8> {
    let inner = zip_of(&[("x.txt", b"inner", zip::CompressionMethod::Stored)], None);
    zip_of(
        &[
            ("docs/", b"", zip::CompressionMethod::Stored),
            ("docs/a.txt", b"hello archive", zip::CompressionMethod::Stored),
            ("docs/big.txt", &big_text(), zip::CompressionMethod::Deflated),
            ("@/content/n.txt", b"named at", zip::CompressionMethod::Stored),
            ("inner.zip", &inner, zip::CompressionMethod::Stored),
            ("b.txt", b"bee", zip::CompressionMethod::Deflated),
        ],
        None,
    )
}

#[tokio::test]
async fn path_source_browse_and_read() {
    let srv = start().await;
    srv.write_zip("home/alice/docs/a.zip", &sample());
    let alice = srv.client("alice").await;
    let hello = alice.post("hello", json!({})).await;
    assert_eq!(hello["result"]["archive"]["available"], true);
    assert!(hello["result"]["archive"]["formats"][0]["methods"].as_array().unwrap().iter().any(|m| m == "deflate"));

    let root = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive"}, "want": ["base", "ident", "access"]})).await;
    assert_eq!(root["kind"], "dir");
    assert_eq!(root["name"], "a.zip");
    assert_eq!(root["capabilities"]["remove_semantics"], "none");
    assert_eq!(root["capabilities"]["accepts_content"], false);
    assert_eq!(root["attrs"]["archive"]["format"], "zip");
    assert_eq!(root["attrs"]["archive"]["entries"], 6);
    assert_eq!(root["attrs"]["archive"]["hidden_entries"], 0);
    assert_eq!(root["attrs"]["archive"]["name_encoding"]["encoding"], "UTF-8");
    assert!(root["node_id"].as_str().unwrap().starts_with("na_"));

    let list = alice.ok("list", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive"}, "want": ["base", "ident"]})).await;
    let names: Vec<&str> = list["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["@", "b.txt", "docs", "inner.zip"]);
    assert_eq!(list["container"]["attrs"]["archive"]["entries"], 6);
    let docs = &list["entries"][2];
    assert_eq!(docs["canonical_path"], "cyfs:///home/alice/docs/a.zip/@/archive/docs");
    assert_eq!(docs["target"]["kind"], "dir");

    let page = alice.ok("list", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive"}, "args": {"limit": 1}})).await;
    assert_eq!(page["truncated"], true);
    let next = alice.ok("list", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive"}, "args": {"limit": 10, "cursor": page["next_cursor"]}})).await;
    assert_eq!(next["entries"].as_array().unwrap().len(), 3);

    let by_ref = alice.ok("list", json!({"at": {"ref": docs["target"]["ref"]}, "want": ["base", "access", "ident"]})).await;
    let files: Vec<&str> = by_ref["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(files, vec!["a.txt", "big.txt"]);
    let a = by_ref["entries"][0]["target"]["attrs"].clone();
    assert_eq!(a["size"], 13);
    assert_eq!(a["method"], "stored");
    assert_eq!(a["encrypted"], false);

    // stored member: any range straight from the outer file
    let a_info = alice.ok("stat", json!({"at": {"path": "/home/alice/docs/a.zip/@/archive/docs/a.txt"}, "want": ["base", "access", "ident"]})).await;
    let url = read_url(&a_info);
    assert!(url.starts_with("/nfs/v1/read/na_"));
    let (status, body, _) = alice.read(&url, None).await;
    assert_eq!((status, body.as_slice()), (200, &b"hello archive"[..]));
    let (status, body, headers) = alice.read(&url, Some("bytes=6-")).await;
    assert_eq!((status, body.as_slice()), (206, &b"archive"[..]));
    assert_eq!(headers["content-range"], "bytes 6-12/13");
    assert_eq!(alice.read(&url, Some("bytes=13-")).await.0, 416);

    // deflate member: stream from 0, materialize for a later start
    let big = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive/docs/big.txt"}, "want": ["access", "base"]})).await;
    let url = read_url(&big);
    let expect = big_text();
    let (status, body, headers) = alice.read(&url, None).await;
    assert_eq!(status, 200);
    assert_eq!(headers["accept-ranges"], "bytes");
    assert_eq!(body, expect);
    let (status, body, _) = alice.read(&url, Some("bytes=0-99")).await;
    assert_eq!((status, body.as_slice()), (206, &expect[..100]));
    let (status, body, headers) = alice.read(&url, Some("bytes=-1000")).await;
    assert_eq!(status, 206);
    assert_eq!(body, &expect[expect.len() - 1000..]);
    assert_eq!(headers["content-range"], format!("bytes {}-{}/{}", expect.len() - 1000, expect.len() - 1, expect.len()));

    // a real member named `@`, reserved branches, nesting
    let at_dir = alice.ok("list", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive/@/content"}})).await;
    assert_eq!(at_dir["entries"][0]["name"], "n.txt");
    for bad in ["cyfs:///home/alice/docs/a.zip/@", "cyfs:///home/alice/docs/a.zip/@/content"] {
        assert_eq!(alice.err("stat", json!({"at": {"uri": bad}})).await["code"], "NOT_FOUND", "{bad}");
    }
    let nested = alice.err("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive/inner.zip/@/archive"}})).await;
    assert_eq!((nested["code"].as_str(), nested["reason"].as_str()), (Some("UNSUPPORTED"), Some("nested_archive")));
    assert_eq!(alice.err("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive/nope.txt"}})).await["code"], "NOT_FOUND");
    let walked = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive"}, "args": {"name": "b.txt"}})).await;
    assert_eq!(walked["kind"], "file");

    // read-only
    let denied = alice.err("mkdir", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive/docs"}, "args": {"name": "x"}})).await;
    assert_eq!((denied["code"].as_str(), denied["reason"].as_str()), (Some("PERMISSION_DENIED"), Some("archive_read_only")));
    let denied = alice.err("delete", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive/docs"}, "args": {"name": "a.txt"}})).await;
    assert_eq!(denied["reason"], "archive_read_only");

    // the archive file itself is unchanged; a directory really named `@` still works
    std::fs::create_dir_all(srv.data.join("home/alice/docs/@/sub")).unwrap();
    std::fs::write(srv.data.join("home/alice/docs/@/sub/f.txt"), b"plain").unwrap();
    assert_eq!(alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/@/sub/f.txt"}})).await["kind"], "file");
    assert_eq!(alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip"}})).await["kind"], "file");

    // others need read on the archive file
    let bob = srv.client("bob").await;
    assert_eq!(bob.err("list", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive"}})).await["code"], "PERMISSION_DENIED");
    assert_eq!(bob.read(&url, None).await.0, 403);

    // replaced outer: old handles are STALE, new resolution sees the new archive
    let tmp = srv.data.join("home/alice/docs/new.zip");
    std::fs::write(&tmp, zip_of(&[("docs/a.txt", b"replaced", zip::CompressionMethod::Stored)], None)).unwrap();
    std::fs::rename(&tmp, srv.data.join("home/alice/docs/a.zip")).unwrap();
    let (status, body, _) = alice.read(&read_url(&a_info), None).await;
    assert_eq!(status, 410, "{}", String::from_utf8_lossy(&body));
    assert_eq!(alice.err("list", json!({"at": {"ref": docs["target"]["ref"]}})).await["code"], "STALE");
    let fresh = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/a.zip/@/archive/docs/a.txt"}, "want": ["access"]})).await;
    assert_eq!(alice.read(&read_url(&fresh), None).await.1, b"replaced");
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_outer_is_unsupported() {
    let srv = start().await;
    srv.write_zip("home/alice/docs/a.zip", &sample());
    std::os::unix::fs::symlink(srv.data.join("home/alice/docs/a.zip"), srv.data.join("home/alice/docs/link.zip")).unwrap();
    let alice = srv.client("alice").await;
    let e = alice.err("stat", json!({"at": {"uri": "cyfs:///home/alice/docs/link.zip/@/archive"}})).await;
    assert_eq!((e["code"].as_str(), e["reason"].as_str()), (Some("UNSUPPORTED"), Some("symlink")));
}

#[tokio::test]
async fn corrupt_archive_is_reported() {
    let srv = start().await;
    srv.write_zip("home/alice/docs/bad.zip", b"PK\x03\x04 this is not really a zip archive");
    let alice = srv.client("alice").await;
    assert_eq!(alice.err("list", json!({"at": {"uri": "cyfs:///home/alice/docs/bad.zip/@/archive"}})).await["code"], "ARCHIVE_CORRUPT");
}

#[tokio::test]
async fn cjk_names_follow_detection_and_preference() {
    let srv = start().await;
    let (gbk, _, _) = encoding_rs::GBK.encode("报告.txt");
    let long_names = ["项目资料/会议纪要.docx", "项目资料/财务报表汇总.xlsx"];
    let short = rename_raw(zip_of(&[("XXXX.txt", b"short", zip::CompressionMethod::Stored)], None), b"XXXX.txt", &gbk);
    srv.write_zip("home/alice/short.zip", &short);
    let mut long = zip_of(&[("PPPPPPPP/QQQQQQQQ.docx", b"1", zip::CompressionMethod::Stored), ("PPPPPPPP/RRRRRRRRRRRR.xlsx", b"2", zip::CompressionMethod::Stored)], None);
    for (placeholder, real) in [("PPPPPPPP", "项目资料"), ("QQQQQQQQ", "会议纪要"), ("RRRRRRRRRRRR", "财务报表汇总")] {
        long = rename_raw(long, placeholder.as_bytes(), &encoding_rs::GBK.encode(real).0);
    }
    srv.write_zip("home/alice/long.zip", &long);

    let zh = srv.client_lang("alice", Some("zh-CN,zh;q=0.9,en;q=0.8")).await;
    let root = zh.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/short.zip/@/archive"}})).await;
    assert_eq!(root["attrs"]["archive"]["name_encoding"], json!({"encoding": "GBK", "by": "preference"}));
    let list = zh.ok("list", json!({"at": {"uri": "cyfs:///home/alice/short.zip/@/archive"}})).await;
    assert_eq!(list["entries"][0]["name"], "报告.txt");

    // another requester sees the fallback but the GBK path still opens
    let en = srv.client("alice").await;
    let root = en.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/short.zip/@/archive"}})).await;
    assert_eq!(root["attrs"]["archive"]["name_encoding"]["by"], "fallback");
    let member = en.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/short.zip/@/archive/报告.txt"}, "want": ["access"]})).await;
    assert_eq!(en.read(&read_url(&member), None).await.1, b"short");

    let root = en.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/long.zip/@/archive"}})).await;
    assert_eq!(root["attrs"]["archive"]["name_encoding"], json!({"encoding": "GBK", "by": "detected"}));
    for path in long_names {
        en.ok("stat", json!({"at": {"uri": format!("cyfs:///home/alice/long.zip/@/archive/{path}")}})).await;
    }
}

#[tokio::test]
async fn passwords_unlock_remember_and_forget() {
    let srv = start().await;
    for (name, aes) in [("aes.zip", true), ("crypto.zip", false)] {
        srv.write_zip(&format!("home/alice/{name}"), &zip_of(&[("secret.txt", &big_text(), zip::CompressionMethod::Deflated)], Some(("s3cret", aes))));
        let alice = srv.client("alice").await;
        let at = json!({"uri": format!("cyfs:///home/alice/{name}/@/archive")});
        let root = alice.ok("stat", json!({"at": at})).await;
        assert_eq!(root["attrs"]["archive"]["encrypted_entries"], 1);
        let member = alice.ok("stat", json!({"at": {"uri": format!("cyfs:///home/alice/{name}/@/archive/secret.txt")}, "want": ["access", "base"]})).await;
        assert_eq!(member["attrs"]["encrypted"], true);
        let url = read_url(&member);
        let (status, body, _) = alice.read(&url, None).await;
        assert_eq!(status, 403);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "PASSWORD_REQUIRED");
        assert_eq!(alice.err("archive.unlock", json!({"at": at, "args": {"password": "wrong-one"}})).await["code"], "PASSWORD_INVALID", "{name}");
        assert_eq!(alice.ok("archive.unlock", json!({"at": at, "args": {"password": "s3cret"}})).await["ok"], true);
        let (status, body, _) = alice.read(&url, None).await;
        assert_eq!(status, 200);
        assert_eq!(body, big_text());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // remembered: a new session of the same user reads without unlocking
        let again = srv.client("alice").await;
        let (status, body, _) = again.read(&url, Some("bytes=1000-1999")).await;
        assert_eq!(status, 206, "{name}: {}", String::from_utf8_lossy(&body));
        assert_eq!(body, &big_text()[1000..2000]);

        // never shared with another user
        let bob_url = url.clone();
        std::fs::create_dir_all(srv.data.join("home/bob")).unwrap();
        std::fs::copy(srv.data.join(format!("home/alice/{name}")), srv.data.join(format!("home/bob/{name}"))).unwrap();
        let bob = srv.client("bob").await;
        let bob_member = bob.ok("stat", json!({"at": {"uri": format!("cyfs:///home/bob/{name}/@/archive/secret.txt")}, "want": ["access"]})).await;
        assert_eq!(bob.read(&read_url(&bob_member), None).await.0, 403);
        assert_eq!(bob.read(&bob_url, None).await.0, 403);

        // forget this archive
        assert_eq!(again.ok("archive.forget", json!({"at": at})).await["forgotten"], 1);
        let fresh = srv.client("alice").await;
        assert_eq!(fresh.read(&url, Some("bytes=5-9")).await.0, 403);
    }
    let alice = srv.client("alice").await;
    alice.ok("archive.forget", json!({"args": {"all": true}})).await;
}

async fn put_object(srv: &Server, name: &str, bytes: &[u8]) -> String {
    let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(bytes).unwrap();
    let (id, body) = ndn_lib::NamedObject::gen_obj_id(&ndn_lib::FileObject::new(name.into(), bytes.len() as u64, chunk.to_string()));
    let store = srv.state.share.store(&srv.state.config).await.unwrap().clone();
    store.put_chunk(&chunk, bytes).await.unwrap();
    store.put_object(&id, &body).await.unwrap();
    id.to_string()
}

async fn hold(srv: &Server, user: &str, obj: &str) {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {}", token(user)).parse().unwrap());
    let p = nfs_server::ndm::principal(&srv.state, &headers).await.unwrap();
    nfs_server::ndm::record_holding(&srv.state, &p, &ndn_lib::ObjId::new(obj).unwrap(), "granted");
}

#[tokio::test]
async fn object_source_is_a_fixed_reference() {
    let srv = start().await;
    let obj = put_object(&srv, "photos.zip", &sample()).await;
    hold(&srv, "alice", &obj).await;
    let alice = srv.client("alice").await;
    let root_ref = json!({"type": "object", "obj_id": obj, "inner_path": "/archive"});
    let root = alice.ok("stat", json!({"at": {"ref": root_ref}, "want": ["base", "ident"]})).await;
    assert_eq!(root["name"], "photos.zip");
    assert_eq!(root["ref"], root_ref);
    assert!(root["node_id"].as_str().unwrap().starts_with("no_"));
    let list = alice.ok("list", json!({"at": {"ref": {"type": "object", "obj_id": obj, "inner_path": "/archive/docs"}}, "want": ["base", "access", "ident"]})).await;
    let a = list["entries"].as_array().unwrap().iter().find(|e| e["name"] == "a.txt").unwrap().clone();
    assert_eq!(a["target"]["ref"], json!({"type": "object", "obj_id": obj, "inner_path": "/archive/docs/a.txt"}));
    assert!(a.get("canonical_path").is_none());
    let etag = a["target"]["attrs"]["etag"].as_str().unwrap();
    assert!(etag.starts_with(&obj));
    let url = read_url(&a["target"]["attrs"]);
    assert!(url.starts_with("/nfs/v1/read/no_"));
    let (status, body, headers) = alice.read(&url, None).await;
    assert_eq!((status, body.as_slice()), (200, &b"hello archive"[..]));
    assert!(headers["etag"].to_str().unwrap().starts_with('"'));
    let big = alice.ok("stat", json!({"at": {"ref": {"type": "object", "obj_id": obj, "inner_path": "/archive/docs/big.txt"}}, "want": ["access"]})).await;
    assert_eq!(alice.read(&read_url(&big), Some("bytes=10-19")).await.1, &big_text()[10..20]);

    // not a holder: denied, cached index or not
    let bob = srv.client("bob").await;
    assert_eq!(bob.err("list", json!({"at": {"ref": root_ref}})).await["code"], "PERMISSION_DENIED");
    assert_eq!(bob.read(&url, None).await.0, 403);
    let owner = srv.client("owner").await;
    owner.ok("list", json!({"at": {"ref": root_ref}})).await;

    // other inner paths stay unsupported; writes are refused
    assert_eq!(alice.err("stat", json!({"at": {"ref": {"type": "object", "obj_id": obj, "inner_path": "/content"}}})).await["code"], "UNSUPPORTED");
    assert_eq!(alice.err("stat", json!({"at": {"ref": {"type": "object", "obj_id": obj}}})).await["code"], "UNSUPPORTED");
    let denied = alice.err("mkdir", json!({"at": {"ref": root_ref}, "args": {"name": "x"}})).await;
    assert_eq!(denied["reason"], "archive_read_only");
}

#[tokio::test]
async fn one_remembered_password_for_chat_and_zone_copies() {
    let srv = start().await;
    let bytes = zip_of(&[("secret.txt", b"top secret", zip::CompressionMethod::Deflated)], Some(("pw-1", true)));
    srv.write_zip("home/alice/s.zip", &bytes);
    let obj = put_object(&srv, "s.zip", &bytes).await;
    hold(&srv, "alice", &obj).await;
    let alice = srv.client("alice").await;
    alice.ok("archive.unlock", json!({"at": {"uri": "cyfs:///home/alice/s.zip/@/archive"}, "args": {"password": "pw-1"}})).await;
    let zone = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/s.zip/@/archive/secret.txt"}, "want": ["access"]})).await;
    assert_eq!(alice.read(&read_url(&zone), None).await.1, b"top secret");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let chat = alice.ok("stat", json!({"at": {"ref": {"type": "object", "obj_id": obj, "inner_path": "/archive/secret.txt"}}, "want": ["access"]})).await;
    let (status, body, _) = alice.read(&read_url(&chat), None).await;
    assert_eq!((status, body.as_slice()), (200, &b"top secret"[..]), "{}", String::from_utf8_lossy(&body));
}

// ---------- more formats (archive-preview-more-formats-todo.md) ----------

fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_mtime(1_700_000_000);
        h.set_mode(0o644);
        if let Some((link, target)) = name.split_once("->") {
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            b.append_link(&mut h, link, target).unwrap();
        } else if name.ends_with('/') {
            h.set_entry_type(tar::EntryType::Directory);
            h.set_size(0);
            b.append_data(&mut h, name, std::io::empty()).unwrap();
        } else {
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(data.len() as u64);
            b.append_data(&mut h, name, *data).unwrap();
        }
    }
    b.into_inner().unwrap()
}

fn compress(kind: &str, bytes: &[u8]) -> Vec<u8> {
    match kind {
        "gz" => {
            let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            w.write_all(bytes).unwrap();
            w.finish().unwrap()
        }
        "xz" => {
            let mut w = xz2::write::XzEncoder::new(Vec::new(), 6);
            w.write_all(bytes).unwrap();
            w.finish().unwrap()
        }
        "bz2" => {
            let mut w = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
            w.write_all(bytes).unwrap();
            w.finish().unwrap()
        }
        "zst" => zstd::encode_all(bytes, 3).unwrap(),
        _ => bytes.to_vec(),
    }
}

fn tar_sample() -> Vec<u8> {
    tar_of(&[("./", b""), ("./docs/", b""), ("./docs/a.txt", b"hello tar"), ("./docs/big.txt", &big_text()), ("./link->docs/a.txt", b""), ("./top.txt", b"top")])
}

async fn names_at(c: &Client, uri: &str) -> Vec<String> {
    let list = c.ok("list", json!({"at": {"uri": uri}, "want": ["base"]})).await;
    list["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn tar_family_browse_and_read() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let hello = alice.post("hello", json!({})).await;
    let formats = hello["result"]["archive"]["formats"].as_array().unwrap().clone();
    let tar_ext = formats.iter().find(|f| f["format"] == "tar").unwrap()["extensions"].clone();
    assert!(tar_ext.as_array().unwrap().iter().any(|e| e == "tar.gz"));
    assert!(formats.iter().any(|f| f["format"] == "7z"));
    for (file, kind, compression) in [("t.tar", "", None), ("t.tar.gz", "gz", Some("gzip")), ("t.tgz", "gz", Some("gzip")), ("t.tar.xz", "xz", Some("xz")), ("t.tar.bz2", "bz2", Some("bzip2")), ("t.tar.zst", "zst", Some("zstd"))] {
        srv.write_zip(&format!("home/alice/{file}"), &compress(kind, &tar_sample()));
        let root_uri = format!("cyfs:///home/alice/{file}/@/archive");
        let root = alice.ok("stat", json!({"at": {"uri": root_uri}, "want": ["base"]})).await;
        let summary = &root["attrs"]["archive"];
        assert_eq!(summary["format"], "tar", "{file}");
        assert_eq!(summary["compression"].as_str(), compression, "{file}");
        assert_eq!(summary["hidden_entries"], 1, "{file}: the symlink");
        assert_eq!(names_at(&alice, &root_uri).await, vec!["docs", "top.txt"], "{file}");
        let a = alice.ok("stat", json!({"at": {"uri": format!("{root_uri}/docs/a.txt")}, "want": ["base", "access", "ident"]})).await;
        assert_eq!(a["attrs"]["method"], "stored");
        assert_eq!(a["size"], 9);
        let url = read_url(&a);
        assert_eq!(alice.read(&url, None).await.1, b"hello tar");
        let (status, body, _) = alice.read(&url, Some("bytes=6-")).await;
        assert_eq!((status, body.as_slice()), (206, &b"tar"[..]), "{file}");
        let big = alice.ok("stat", json!({"at": {"uri": format!("{root_uri}/docs/big.txt")}, "want": ["access"]})).await;
        let (status, body, _) = alice.read(&read_url(&big), Some("bytes=200000-200099")).await;
        assert_eq!((status, body.as_slice()), (206, &big_text()[200_000..200_100]), "{file}");
    }
    // decompressed once: a second index of the same outer reuses the cached tar
    let unpacked = srv._tmp.path().join("fsdb/archive/unpacked");
    let files = std::fs::read_dir(&unpacked).unwrap().filter(|e| !e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".json")).count();
    assert_eq!(files, 5);
}

#[tokio::test]
async fn single_compressed_files_are_one_member_archives() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let mut gz = flate2::GzBuilder::new().filename("server.log").write(Vec::new(), flate2::Compression::default());
    gz.write_all(b"line one\nline two\n").unwrap();
    srv.write_zip("home/alice/today.gz", &gz.finish().unwrap());
    srv.write_zip("home/alice/dump.sql.zst", &compress("zst", b"select 1;"));
    assert_eq!(names_at(&alice, "cyfs:///home/alice/today.gz/@/archive").await, vec!["server.log"]);
    let root = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/today.gz/@/archive"}, "want": ["base"]})).await;
    assert_eq!(root["attrs"]["archive"]["format"], "compressed");
    let m = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/today.gz/@/archive/server.log"}, "want": ["access", "base"]})).await;
    assert_eq!(alice.read(&read_url(&m), None).await.1, b"line one\nline two\n");
    assert_eq!(names_at(&alice, "cyfs:///home/alice/dump.sql.zst/@/archive").await, vec!["dump.sql"]);
}

#[tokio::test]
async fn decompressing_past_the_limit_is_refused() {
    let srv = start_with(|c| c.archive.max_materialize_bytes = 100_000).await;
    srv.write_zip("home/alice/big.tar.gz", &compress("gz", &tar_of(&[("big.txt", &big_text())])));
    let alice = srv.client("alice").await;
    let e = alice.err("list", json!({"at": {"uri": "cyfs:///home/alice/big.tar.gz/@/archive"}})).await;
    assert_eq!(e["code"], "QUOTA_EXCEEDED", "{e}");
    let leftovers: Vec<_> = std::fs::read_dir(srv._tmp.path().join("fsdb/archive/unpacked")).unwrap().collect();
    assert!(leftovers.is_empty(), "no partial file stays behind");
}

#[tokio::test]
async fn unsupported_formats_say_so() {
    let srv = start().await;
    srv.write_zip("home/alice/old.rar", b"Rar!\x1a\x07\x01\x00 the rest of a rar file");
    srv.write_zip("home/alice/plain.txt", b"not an archive at all, just some words");
    let alice = srv.client("alice").await;
    let e = alice.err("stat", json!({"at": {"uri": "cyfs:///home/alice/old.rar/@/archive"}})).await;
    assert_eq!((e["code"].as_str(), e["reason"].as_str(), e["format"].as_str()), (Some("UNSUPPORTED"), Some("format"), Some("rar")), "{e}");
    let e = alice.err("stat", json!({"at": {"uri": "cyfs:///home/alice/plain.txt/@/archive"}})).await;
    assert_eq!((e["code"].as_str(), e["reason"].as_str()), (Some("UNSUPPORTED"), Some("format")), "{e}");
}

fn sevenz_of(entries: &[(&str, &[u8])], solid: bool, password: Option<&str>, encrypt_header: bool, copy: bool) -> Vec<u8> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password};
    let mut w = ArchiveWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
    let mut methods: Vec<EncoderConfiguration> = Vec::new();
    if let Some(p) = password {
        methods.push(AesEncoderOptions::new(Password::new(p)).into());
    }
    methods.push(if copy { EncoderMethod::COPY.into() } else { Lzma2Options::from_level(1).into() });
    w.set_content_methods(methods);
    w.set_encrypt_header(encrypt_header);
    for (n, _) in entries.iter().filter(|(n, _)| n.ends_with('/')) {
        w.push_archive_entry::<&[u8]>(ArchiveEntry::new_directory(n.trim_end_matches('/')), None).unwrap();
    }
    let files: Vec<&(&str, &[u8])> = entries.iter().filter(|(n, _)| !n.ends_with('/')).collect();
    if solid {
        let list = files.iter().map(|(n, _)| ArchiveEntry::new_file(n)).collect();
        w.push_archive_entries(list, files.iter().map(|(_, d)| (*d).into()).collect()).unwrap();
    } else {
        for (n, d) in files {
            w.push_archive_entry(ArchiveEntry::new_file(n), Some(*d)).unwrap();
        }
    }
    w.finish().unwrap().into_inner()
}

fn sevenz_sample() -> Vec<(&'static str, Vec<u8>)> {
    vec![("docs/", Vec::new()), ("docs/a.txt", b"hello 7z".to_vec()), ("docs/big.txt", big_text()), ("empty.txt", Vec::new()), ("z-last.txt", b"the last one".to_vec())]
}

fn sevenz_sample_of(solid: bool, password: Option<&str>, encrypt_header: bool, copy: bool) -> Vec<u8> {
    let s = sevenz_sample();
    let refs: Vec<(&str, &[u8])> = s.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    sevenz_of(&refs, solid, password, encrypt_header, copy)
}

#[tokio::test]
async fn sevenz_solid_plain_and_copy_blocks() {
    let srv = start().await;
    let alice = srv.client("alice").await;
    let cached = || std::fs::read_dir(srv._tmp.path().join("fsdb/archive/members")).unwrap().count();
    let mut added = Vec::new();
    for (file, solid, copy) in [("solid.7z", true, false), ("plain.7z", false, false), ("copy.7z", true, true)] {
        let before = cached();
        srv.write_zip(&format!("home/alice/{file}"), &sevenz_sample_of(solid, None, false, copy));
        let root_uri = format!("cyfs:///home/alice/{file}/@/archive");
        let root = alice.ok("stat", json!({"at": {"uri": root_uri}, "want": ["base"]})).await;
        let summary = &root["attrs"]["archive"];
        assert_eq!(summary["format"], "7z");
        assert_eq!(summary["header_encrypted"], false);
        assert_eq!(summary["name_encoding"]["by"], "format");
        assert_eq!(names_at(&alice, &root_uri).await, vec!["docs", "empty.txt", "z-last.txt"], "{file}");
        for (path, want) in [("docs/a.txt", b"hello 7z".to_vec()), ("z-last.txt", b"the last one".to_vec()), ("empty.txt", Vec::new()), ("docs/big.txt", big_text())] {
            let m = alice.ok("stat", json!({"at": {"uri": format!("{root_uri}/{path}")}, "want": ["access", "base"]})).await;
            let url = read_url(&m);
            let (status, body, _) = alice.read(&url, None).await;
            assert_eq!((status, body.len()), (200, want.len()), "{file} {path}: {}", String::from_utf8_lossy(&body[..body.len().min(200)]));
            assert_eq!(body, want, "{file} {path}");
            if want.len() > 10 {
                let (status, body, _) = alice.read(&url, Some("bytes=3-9")).await;
                assert_eq!((status, body.as_slice()), (206, &want[3..10]), "{file} {path}");
            }
        }
        added.push(cached() - before);
    }
    // solid: the block's 3 files with data, extracted once; plain: the 2 members read from an offset
    // (materialized); copy: byte ranges of the outer, nothing cached
    assert_eq!(added, vec![3, 2, 0]);
}

#[tokio::test]
async fn sevenz_encrypted_data_needs_a_password_to_read() {
    let srv = start().await;
    srv.write_zip("home/alice/enc.7z", &sevenz_sample_of(true, Some("pw-7z"), false, false));
    let alice = srv.client("alice").await;
    let at = json!({"uri": "cyfs:///home/alice/enc.7z/@/archive"});
    let root = alice.ok("stat", json!({"at": at})).await;
    assert_eq!(root["attrs"]["archive"]["encrypted_entries"], 3);
    let m = alice.ok("stat", json!({"at": {"uri": "cyfs:///home/alice/enc.7z/@/archive/z-last.txt"}, "want": ["access", "base"]})).await;
    assert_eq!(m["attrs"]["encrypted"], true);
    let url = read_url(&m);
    assert_eq!(alice.read(&url, None).await.0, 403);
    assert_eq!(alice.err("archive.unlock", json!({"at": at, "args": {"password": "nope"}})).await["code"], "PASSWORD_INVALID");
    alice.ok("archive.unlock", json!({"at": at, "args": {"password": "pw-7z"}})).await;
    let (status, body, _) = alice.read(&url, None).await;
    assert_eq!((status, body.as_slice()), (200, &b"the last one"[..]), "{}", String::from_utf8_lossy(&body));
    let bob = srv.client("bob").await;
    assert_eq!(bob.read(&url, None).await.0, 403);
}

#[tokio::test]
async fn sevenz_encrypted_header_needs_a_password_to_list() {
    let srv = start().await;
    srv.write_zip("home/alice/hidden.7z", &sevenz_sample_of(true, Some("pw-h"), true, false));
    let alice = srv.client("alice").await;
    let root_uri = "cyfs:///home/alice/hidden.7z/@/archive";
    let at = json!({"uri": root_uri});
    assert_eq!(alice.err("list", json!({"at": at})).await["code"], "PASSWORD_REQUIRED");
    assert_eq!(alice.err("stat", json!({"at": at})).await["code"], "PASSWORD_REQUIRED");
    assert_eq!(alice.err("stat", json!({"at": {"uri": format!("{root_uri}/docs")}})).await["code"], "PASSWORD_REQUIRED");
    assert_eq!(alice.err("archive.unlock", json!({"at": at, "args": {"password": "wrong"}})).await["code"], "PASSWORD_INVALID");
    alice.ok("archive.unlock", json!({"at": at, "args": {"password": "pw-h"}})).await;
    assert_eq!(names_at(&alice, root_uri).await, vec!["docs", "empty.txt", "z-last.txt"]);
    let root = alice.ok("stat", json!({"at": at})).await;
    assert_eq!(root["attrs"]["archive"]["header_encrypted"], true);
    let m = alice.ok("stat", json!({"at": {"uri": format!("{root_uri}/docs/a.txt")}, "want": ["access"]})).await;
    assert_eq!(alice.read(&read_url(&m), None).await.1, b"hello 7z");

    // remembered for alice's next session; never visible to bob
    let again = srv.client("alice").await;
    assert_eq!(names_at(&again, root_uri).await, vec!["docs", "empty.txt", "z-last.txt"]);
    std::fs::copy(srv.data.join("home/alice/hidden.7z"), srv.data.join("home/bob/hidden.7z")).unwrap();
    let bob = srv.client("bob").await;
    assert_eq!(bob.err("list", json!({"at": {"uri": "cyfs:///home/bob/hidden.7z/@/archive"}})).await["code"], "PASSWORD_REQUIRED");

    again.ok("archive.forget", json!({"at": at})).await;
    let fresh = srv.client("alice").await;
    assert_eq!(fresh.err("list", json!({"at": at})).await["code"], "PASSWORD_REQUIRED");

    // forgetting every password closes what was opened with them too
    fresh.ok("archive.unlock", json!({"at": at, "args": {"password": "pw-h"}})).await;
    fresh.ok("list", json!({"at": at})).await;
    fresh.ok("archive.forget", json!({"args": {"all": true}})).await;
    assert_eq!(srv.client("alice").await.err("list", json!({"at": at})).await["code"], "PASSWORD_REQUIRED");
}

#[tokio::test]
async fn sevenz_and_tar_attachments_open_as_objects() {
    let srv = start().await;
    let seven = put_object(&srv, "chat.7z", &sevenz_sample_of(true, None, false, false)).await;
    let targz = put_object(&srv, "chat.tar.gz", &compress("gz", &tar_sample())).await;
    let alice = srv.client("alice").await;
    for (obj, member, want) in [(&seven, "/archive/z-last.txt", &b"the last one"[..]), (&targz, "/archive/docs/a.txt", &b"hello tar"[..])] {
        hold(&srv, "alice", obj).await;
        let m = alice.ok("stat", json!({"at": {"ref": {"type": "object", "obj_id": obj, "inner_path": member}}, "want": ["access", "ident"]})).await;
        let (status, body, headers) = alice.read(&read_url(&m), None).await;
        assert_eq!((status, body.as_slice()), (200, want));
        assert!(headers["etag"].to_str().unwrap().starts_with('"'));
    }
    let bob = srv.client("bob").await;
    assert_eq!(bob.err("list", json!({"at": {"ref": {"type": "object", "obj_id": seven, "inner_path": "/archive"}}})).await["code"], "PERMISSION_DENIED");
}
