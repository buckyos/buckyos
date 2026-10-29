//! Preview Pipeline (`repr` / `get_repr`) with the real ffmpeg / ffprobe on
//! real HEIC / HEVC samples (tests/fixtures/preview, encoded by libheif and
//! x265). Conversion cases skip when ffmpeg / ffprobe are not installed; the
//! "missing converter" case always runs. Tokens use the test key of tests/ndm.rs.

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::AppState;
use reqwest::Method;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = "g, owner, admin\ng, alice, users\ng, bob, users\ng, app:photos, app\ng, system:control-panel, system\n";
        let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, groups);
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim()).await.unwrap();
    })
    .await;
}

fn token(sub: &str, target: &str) -> String {
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
    bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
    bind_token_target(&mut token, &AuthTarget::from_canonical_key(target).unwrap(), TokenUse::Session).unwrap();
    token.generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap()).unwrap()
}

fn ui(name: &str) -> String {
    token(name, "system:control-panel")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview").join(name)
}

fn tool(name: &str) -> Option<PathBuf> {
    nfs_server::preview::tools::find(name, &nfs_server::preview::tools::search_dirs())
}

fn tools_present() -> bool {
    if tool("ffmpeg").is_some() && tool("ffprobe").is_some() {
        return true;
    }
    eprintln!("skipped: ffmpeg/ffprobe not installed");
    false
}

struct Server {
    base: String,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn start(tool_dirs: Option<Vec<PathBuf>>) -> Server {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("exports/home");
    std::fs::create_dir_all(home.join("alice/照片 2026")).unwrap();
    for f in std::fs::read_dir(fixture("")).unwrap() {
        let f = f.unwrap();
        std::fs::copy(f.path(), home.join("alice/照片 2026").join(f.file_name())).unwrap();
    }
    std::fs::write(home.join("alice/照片 2026/broken.mov"), [&b"\0\0\0\x14ftypqt  \0\0\0\0qt  "[..], &[0x42u8; 4096][..]].concat()).unwrap();
    std::fs::write(home.join("alice/notes.pdf"), b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n").unwrap();
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: home }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.preview_tool_dirs = tool_dirs;
    config.bucket_specs = serde_json::from_value(json!({"photos": {"buckets": {"library": {"kind": "data", "data_policy": "documents"}}}})).unwrap();
    let state = AppState::new(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::server::build_router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, http: reqwest::Client::new(), _tmp: tmp }
}

struct Client<'a> {
    srv: &'a Server,
    token: String,
    session: String,
    hello: Value,
}

impl Server {
    async fn client(&self, user: &str) -> Client<'_> {
        let token = ui(user);
        let hello: Value = self.http.post(format!("{}/nfs/v1/hello", self.base)).bearer_auth(&token).json(&json!({"args": {}})).send().await.unwrap().json().await.unwrap();
        assert_eq!(hello["ok"], true, "{hello}");
        Client { srv: self, session: hello["result"]["session"].as_str().unwrap().into(), hello: hello["result"].clone(), token }
    }
}

impl Client<'_> {
    async fn call(&self, method: &str, mut body: Value) -> Value {
        body["session"] = json!(self.session);
        self.srv.http.post(format!("{}/nfs/v1/{method}", self.srv.base)).bearer_auth(&self.token).json(&body).send().await.unwrap().json().await.unwrap()
    }

    async fn repr_at(&self, path: &str, args: Value) -> Value {
        self.call("repr", json!({"at": {"path": path}, "args": args})).await
    }

    /// Ensures, then polls `get_repr` until the work is no longer processing.
    async fn converted(&self, first: Value) -> Value {
        assert_eq!(first["ok"], true, "{first}");
        let mut record = first["result"].clone();
        for _ in 0..240 {
            if record["state"] != "processing" {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            let polled = self.call("get_repr", json!({"args": {"work_key": record["work_key"]}})).await;
            assert_eq!(polled["ok"], true, "{polled}");
            record = polled["result"].clone();
        }
        panic!("conversion did not finish: {record}");
    }

    async fn fetch(&self, read_url: &str) -> Vec<u8> {
        let r = self.srv.http.request(Method::GET, format!("{}{}", self.srv.base, read_url)).send().await.unwrap();
        assert!(r.status().is_success(), "{} {}", r.status(), read_url);
        r.bytes().await.unwrap().to_vec()
    }
}

fn probe_bytes(bytes: &[u8]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out");
    std::fs::write(&path, bytes).unwrap();
    let out = std::process::Command::new(tool("ffprobe").unwrap())
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format"])
        .arg(&path)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

/// RGB of one pixel of a JPEG.
fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 3] {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("in.jpg");
    std::fs::write(&path, bytes).unwrap();
    let out = std::process::Command::new(tool("ffmpeg").unwrap())
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args(["-vf", &format!("format=rgb24,crop=1:1:{x}:{y}"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    [out.stdout[0], out.stdout[1], out.stdout[2]]
}

fn near(c: [u8; 3], want: [u8; 3]) -> bool {
    c.iter().zip(want).all(|(a, b)| (*a as i32 - b as i32).abs() < 60)
}

const DIR: &str = "/home/alice/照片 2026";

#[tokio::test]
async fn missing_converter_is_unsupported_not_a_failure() {
    let empty = tempfile::tempdir().unwrap();
    let srv = start(Some(vec![empty.path().to_path_buf()])).await;
    let alice = srv.client("alice").await;
    let features = alice.hello["features"].as_array().unwrap();
    assert!(!features.iter().any(|f| f == "repr"), "{features:?}");
    let r = alice.repr_at(&format!("{DIR}/portrait_grid.heic"), json!({"purpose": "preview"})).await;
    assert_eq!(r["result"]["state"], "unsupported", "{r}");
    assert_eq!(r["result"]["reason"], "no-pipeline");
    assert!(r["result"]["detail"].as_str().unwrap().contains("ffmpeg and ffprobe not found"), "{r}");
}

#[tokio::test]
async fn heic_portrait_grid_becomes_an_upright_jpeg_and_is_reused() {
    if !tools_present() {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    assert!(alice.hello["features"].as_array().unwrap().iter().any(|f| f == "repr"));
    assert_eq!(alice.hello["repr"]["available"], true, "{}", alice.hello);
    assert!(alice.hello["repr"]["ffmpeg"]["version"].is_string());

    let path = format!("{DIR}/portrait_grid.heic");
    let done = alice.converted(alice.repr_at(&path, json!({"purpose": "preview", "runtime": {"accept": ["image", "video"]}})).await).await;
    assert_eq!(done["state"], "completed", "{done}");
    let result = &done["result"];
    assert_eq!(result["media_type"], "image/jpeg");
    assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(768), Some(1024)), "{result}");
    let jpeg = alice.fetch(result["read_url"].as_str().unwrap()).await;
    let probed = probe_bytes(&jpeg);
    assert_eq!(probed["streams"][0]["width"], 768);
    assert_eq!(probed["streams"][0]["height"], 1024);
    // heif-enc rotated the landscape (red top-left) clockwise: red is top-right now.
    assert!(near(pixel(&jpeg, 760, 8), [255, 0, 0]), "{:?}", pixel(&jpeg, 760, 8));
    assert!(near(pixel(&jpeg, 8, 8), [0, 0, 255]), "{:?}", pixel(&jpeg, 8, 8));
    assert!(near(pixel(&jpeg, 8, 1016), [0, 128, 0]) || near(pixel(&jpeg, 8, 1016), [0, 255, 0]), "{:?}", pixel(&jpeg, 8, 1016));

    // Same content, same work: completed at once, same key, a usable read URL.
    let again = alice.repr_at(&path, json!({"purpose": "preview"})).await;
    assert_eq!(again["result"]["state"], "completed", "{again}");
    assert_eq!(again["result"]["work_key"], done["work_key"]);
    assert_eq!(alice.fetch(again["result"]["result"]["read_url"].as_str().unwrap()).await, jpeg);

    // The read URL needs its token; bob cannot ensure nor query alice's work.
    let bare = done["result"]["read_url"].as_str().unwrap().split('?').next().unwrap().to_string();
    assert!(!srv.http.get(format!("{}{}", srv.base, bare)).send().await.unwrap().status().is_success());
    let bob = srv.client("bob").await;
    let denied = bob.repr_at(&path, json!({"purpose": "preview"})).await;
    assert_eq!(denied["error"]["code"], "PERMISSION_DENIED", "{denied}");
    let query = bob.call("get_repr", json!({"args": {"work_key": done["work_key"]}})).await;
    assert_eq!(query["error"]["code"], "PERMISSION_DENIED", "{query}");

    // Thumbnail: the same pipeline, its own work and size.
    let thumb = alice.converted(alice.repr_at(&path, json!({"purpose": "thumbnail"})).await).await;
    assert_ne!(thumb["work_key"], done["work_key"]);
    let probed = probe_bytes(&alice.fetch(thumb["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!((probed["streams"][0]["width"].as_u64(), probed["streams"][0]["height"].as_u64()), (Some(480), Some(640)));
}

#[tokio::test]
async fn heic_alpha_and_hdr_are_noted() {
    if !tools_present() {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    let alpha = alice.converted(alice.repr_at(&format!("{DIR}/alpha.heic"), json!({})).await).await;
    assert_eq!(alpha["state"], "completed", "{alpha}");
    assert!(alpha["result"]["fidelity"].as_str().unwrap().contains("白底"), "{alpha}");
    let jpeg = alice.fetch(alpha["result"]["read_url"].as_str().unwrap()).await;
    assert!(near(pixel(&jpeg, 4, 4), [255, 255, 255]), "{:?}", pixel(&jpeg, 4, 4));
    assert!(near(pixel(&jpeg, 64, 48), [255, 255, 0]), "{:?}", pixel(&jpeg, 64, 48));

    let hdr = alice.converted(alice.repr_at(&format!("{DIR}/hdr_pq.heic"), json!({})).await).await;
    assert_eq!(hdr["state"], "completed", "{hdr}");
    assert!(hdr["result"]["fidelity"].as_str().unwrap().contains("HDR"), "{hdr}");
}

#[tokio::test]
async fn videos_remux_or_transcode_by_runtime_hint() {
    if !tools_present() {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    let codec_of = |v: &Value, kind: &str| v["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == kind).map(|s| s["codec_name"].as_str().unwrap().to_string());

    let h264 = alice.converted(alice.repr_at(&format!("{DIR}/h264_aac.mov"), json!({"runtime": {"video_codecs": {"h264": true, "hevc": false}}})).await).await;
    assert_eq!(h264["result"]["plan"], "remux", "{h264}");
    let probed = probe_bytes(&alice.fetch(h264["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!(codec_of(&probed, "video").as_deref(), Some("h264"));
    assert_eq!(codec_of(&probed, "audio").as_deref(), Some("aac"));
    assert!(probed["format"]["format_name"].as_str().unwrap().contains("mp4"));

    let hevc_mov = format!("{DIR}/hevc_aac.mov");
    let transcoded = alice.converted(alice.repr_at(&hevc_mov, json!({"runtime": {"video_codecs": {"h264": true, "hevc": false}}})).await).await;
    assert_eq!(transcoded["result"]["plan"], "transcode", "{transcoded}");
    let probed = probe_bytes(&alice.fetch(transcoded["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!(codec_of(&probed, "video").as_deref(), Some("h264"));
    assert_eq!(probed["streams"][0]["pix_fmt"], "yuv420p");
    let duration: f64 = probed["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((duration - 1.0).abs() < 0.2, "{duration}");

    // A Runtime that plays HEVC gets the same bytes remuxed: another work.
    let remuxed = alice.converted(alice.repr_at(&hevc_mov, json!({"runtime": {"video_codecs": {"h264": true, "hevc": true}}})).await).await;
    assert_eq!(remuxed["result"]["plan"], "remux");
    assert_ne!(remuxed["work_key"], transcoded["work_key"]);
    let probed = probe_bytes(&alice.fetch(remuxed["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!(probed["streams"][0]["codec_tag_string"], "hvc1");

    // Portrait: rotation baked into pixels, no display matrix left.
    let portrait = alice.converted(alice.repr_at(&format!("{DIR}/portrait_hevc.mov"), json!({})).await).await;
    let probed = probe_bytes(&alice.fetch(portrait["result"]["read_url"].as_str().unwrap()).await);
    let v = &probed["streams"][0];
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(180), Some(320)), "{v}");
    assert!(v["side_data_list"].as_array().is_none_or(|l| l.iter().all(|d| d["rotation"].as_i64().unwrap_or(0) == 0)), "{v}");

    // HDR HLG → SDR BT.709, noted.
    let hdr = alice.converted(alice.repr_at(&format!("{DIR}/hdr_hlg_hevc.mov"), json!({})).await).await;
    assert!(hdr["result"]["fidelity"].as_str().unwrap().contains("HDR 已映射为 SDR"), "{hdr}");
    let probed = probe_bytes(&alice.fetch(hdr["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!(probed["streams"][0]["color_transfer"], "bt709", "{probed}");

    // A video thumbnail is a small JPEG frame.
    let thumb = alice.converted(alice.repr_at(&hevc_mov, json!({"purpose": "thumbnail"})).await).await;
    assert_eq!(thumb["result"]["media_type"], "image/jpeg");
    let probed = probe_bytes(&alice.fetch(thumb["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!(probed["streams"][0]["width"], 320);
}

#[tokio::test]
async fn failures_are_cached_until_an_explicit_retry() {
    if !tools_present() {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    let broken = format!("{DIR}/broken.mov");
    let failed = alice.converted(alice.repr_at(&broken, json!({})).await).await;
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"]["code"], "CONTENT_CORRUPTED");
    let again = alice.repr_at(&broken, json!({})).await;
    assert_eq!(again["result"]["attempt_id"], failed["attempt_id"], "negative cache");
    let stale = alice.repr_at(&broken, json!({"retry": true, "expected_attempt_id": "att_other"})).await;
    assert_eq!(stale["result"]["attempt_id"], failed["attempt_id"], "CAS on the attempt the caller saw");
    let retried = alice.repr_at(&broken, json!({"retry": true, "expected_attempt_id": failed["attempt_id"]})).await;
    assert_ne!(retried["result"]["attempt_id"], failed["attempt_id"], "{retried}");
    let retried = alice.converted(retried).await;
    assert_eq!(retried["state"], "failed");

    // Planning, not execution: no converter for a PDF, no preview of a folder.
    let pdf = alice.repr_at("/home/alice/notes.pdf", json!({})).await;
    assert_eq!(pdf["result"]["state"], "unsupported", "{pdf}");
    let dir = alice.repr_at(DIR, json!({})).await;
    assert_eq!(dir["result"]["reason"], "not-content", "{dir}");
    let unknown = alice.repr_at(&broken, json!({"target": {"width": 1}})).await;
    assert_eq!(unknown["error"]["code"], "INVALID_ARGUMENT", "unknown params are rejected");
}

#[tokio::test]
async fn bucket_sources_need_read_access_or_a_token() {
    if !tools_present() {
        return;
    }
    let srv = start(None).await;
    let lib = "user/photos/alice/library";
    let put = srv
        .http
        .put(format!("{}/bucket/v1/{lib}/2026/IMG_0001.HEIC", srv.base))
        .bearer_auth(token("alice", "app:photos@alice"))
        .header("content-type", "image/heic")
        .body(std::fs::read(fixture("landscape_grid.heic")).unwrap())
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success(), "{}", put.status());
    let source = json!({"bucket": {"bucket_id": lib, "key": "2026/IMG_0001.HEIC"}});
    let alice = srv.client("alice").await;
    let done = alice.converted(alice.call("repr", json!({"args": {"source": source}})).await).await;
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!((done["result"]["width"].as_u64(), done["result"]["height"].as_u64()), (Some(1024), Some(768)));

    let bob = srv.client("bob").await;
    let denied = bob.call("repr", json!({"args": {"source": source}})).await;
    assert_eq!(denied["error"]["code"], "PERMISSION_DENIED", "a cached result still needs the source's permission: {denied}");
    let issued: Value = srv
        .http
        .post(format!("{}/kapi/nfs-server", srv.base))
        .json(&json!({"method": "bucket.create_access_token", "params": {"bucket_id": lib, "key": "2026/IMG_0001.HEIC", "actions": ["read"], "expires_at": bucket_store::now_ms() + 600_000}, "sys": [1, token("alice", "app:photos@alice")]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access = issued["result"]["token"].as_str().unwrap();
    let shared = bob.call("repr", json!({"args": {"source": {"bucket": {"bucket_id": lib, "key": "2026/IMG_0001.HEIC", "access_token": access}}}})).await;
    assert_eq!(shared["result"]["state"], "completed", "{shared}");
    assert_eq!(shared["result"]["work_key"], done["work_key"], "the cached result is shared once access is proven");
}
