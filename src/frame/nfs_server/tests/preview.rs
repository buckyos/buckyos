//! Preview Pipeline (`repr` / `get_repr`) with the real ffmpeg / ffprobe on
//! real HEIC / HEVC samples (tests/fixtures/preview, encoded by libheif and
//! x265) and the real LibreOffice on small Office / ODF documents (`office.*`,
//! saved by LibreOffice). The other samples are generated: stills written by
//! ImageMagick, `raw.dng` an uncompressed Bayer DNG, `clip.*` / `tone.*`
//! encoded by ffmpeg, `book.epub` by pandoc, `sheet.*` and the PDFs by
//! LibreOffice. Conversion cases skip when their tool is not
//! installed; the "missing converter" cases and document sniffing always run.
//! Tokens use the test key of tests/ndm.rs.

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

fn soffice_present() -> bool {
    if tool("soffice").is_some() {
        return true;
    }
    eprintln!("skipped: soffice not installed");
    false
}

fn present(names: &[&str]) -> bool {
    let missing: Vec<&&str> = names.iter().filter(|n| tool(n).is_none()).collect();
    if !missing.is_empty() {
        eprintln!("skipped: {missing:?} not installed");
    }
    missing.is_empty()
}

/// Whether some installed LibreOffice has Calc.
fn calc_present() -> bool {
    let probe = nfs_server::preview::tools::probe(&nfs_server::preview::tools::search_dirs());
    if probe.soffice_for(nfs_server::preview::media::Office::Calc).is_some() {
        return true;
    }
    eprintln!("skipped: no LibreOffice with Calc installed");
    false
}

/// `(width, height)` of an image.
fn dimensions(bytes: &[u8]) -> (u64, u64) {
    let v = probe_bytes(bytes);
    (v["streams"][0]["width"].as_u64().unwrap(), v["streams"][0]["height"].as_u64().unwrap())
}

/// A tool directory holding only `names` (links to the installed binaries that exist).
fn only_tools(names: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in names {
        let Some(found) = tool(name) else { continue };
        let link = dir.path().join(nfs_server::preview::tools::exe_name(name));
        #[cfg(unix)]
        std::os::unix::fs::symlink(&found, &link).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(&found, &link).unwrap();
    }
    dir
}

/// Page objects of a PDF (`/Type/Page`, not `/Type/Pages`).
fn pdf_pages(pdf: &[u8]) -> usize {
    pdf.windows(11).filter(|w| w.starts_with(b"/Type/Page") && w[10] != b's').count()
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
    std::fs::write(home.join("alice/照片 2026/broken.jxl"), [&[0xffu8, 0x0a][..], &[0x42u8; 512][..]].concat()).unwrap();
    std::fs::write(home.join("alice/照片 2026/broken.ipynb"), b"this is not a notebook").unwrap();
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

#[test]
fn documents_are_sniffed_by_content() {
    use nfs_server::preview::media::{sniff, Kind};
    let read = |name: &str| std::fs::read(fixture(name)).unwrap();
    assert_eq!(sniff(&read("office.docx"), None), Some(Kind::Document("docx")));
    assert_eq!(sniff(&read("office.pptx"), None), Some(Kind::Document("pptx")));
    assert_eq!(sniff(&read("office.odt"), None), Some(Kind::Document("odt")));
    assert_eq!(sniff(&read("office.doc"), None), Some(Kind::Document("doc")));
    // The bytes decide, a wrong or missing name does not.
    assert_eq!(sniff(&read("office.pptx"), Some("deck.docx")), Some(Kind::Document("pptx")));
    assert_eq!(sniff(&read("office.docx")[..2048], Some("attachment")), Some(Kind::Document("docx")), "the first entries are enough");
    assert_eq!(sniff(b"{\\rtf1\\ansi\\deff0 {\\fonttbl}}", None), Some(Kind::Document("rtf")));
    assert_eq!(sniff(&read("h264_aac.mov"), Some("clip.docx")), Some(Kind::Video));
    assert_eq!(sniff(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3", Some("notes.docx")), Some(Kind::Pdf));
    for (name, kind) in [
        ("sheet.xls", Kind::Document("xls")),
        ("sheet.ods", Kind::Document("ods")),
        ("book.epub", Kind::Markup("epub")),
        ("raw.dng", Kind::Raster("dng")),
        ("still.jxl", Kind::Raster("jxl")),
        ("still.jp2", Kind::Raster("jp2")),
        ("still.exr", Kind::Raster("exr")),
        ("still.hdr", Kind::Raster("hdr")),
        ("still.dds", Kind::Raster("dds")),
        ("clip.asf", Kind::Video),
        ("tone.wma", Kind::Video),
        ("clip.flv", Kind::Video),
        ("clip.rm", Kind::Video),
        ("tone.aiff", Kind::Audio),
        ("tone.caf", Kind::Audio),
        ("pages.pdf", Kind::Pdf),
    ] {
        assert_eq!(sniff(&read(name), Some("attachment")), Some(kind), "{name}");
    }
    assert_eq!(sniff(&read("still.tga"), Some("still.tga")), Some(Kind::Raster("tga")));
    assert_eq!(sniff(&read("readme.md"), Some("readme.md")), Some(Kind::Markup("gfm")));
}

#[tokio::test]
async fn office_documents_become_pdf() {
    if !soffice_present() {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    let repr = &alice.hello["repr"];
    assert!(repr["soffice"][0]["version"].is_string(), "{repr}");
    assert!(repr["soffice"][0]["components"].as_array().unwrap().iter().any(|c| c == "writer"), "{repr}");
    assert!(repr["profiles"].as_array().unwrap().iter().any(|p| p == "preview-document"), "{repr}");

    for (name, pages) in [("office.docx", 1), ("office.pptx", 2), ("office.odt", 1), ("office.doc", 1)] {
        let path = format!("{DIR}/{name}");
        let done = alice.converted(alice.repr_at(&path, json!({"purpose": "preview", "runtime": {"accept": ["image", "video", "pdf"]}})).await).await;
        assert_eq!(done["state"], "completed", "{name}: {done}");
        let result = &done["result"];
        assert_eq!(result["media_type"], "application/pdf", "{name}: {result}");
        assert_eq!(result["result_type"], "document");
        assert!(result["fidelity"].as_str().unwrap().contains("LibreOffice"), "{result}");
        let pdf = alice.fetch(result["read_url"].as_str().unwrap()).await;
        assert!(pdf.starts_with(b"%PDF-"), "{name}");
        assert_eq!(pdf_pages(&pdf), pages, "{name}");
    }
    let scratch = srv._tmp.path().join("fsdb/preview-tmp");
    assert_eq!(std::fs::read_dir(&scratch).map(|d| d.count()).unwrap_or(0), 0, "attempt and profile directories are removed");

    // Cached: the same work, completed at once, served as a PDF an iframe can embed.
    let docx = format!("{DIR}/office.docx");
    let again = alice.repr_at(&docx, json!({})).await;
    assert_eq!(again["result"]["state"], "completed", "{again}");
    let read_url = again["result"]["result"]["read_url"].as_str().unwrap();
    let head = srv.http.get(format!("{}{}", srv.base, read_url)).send().await.unwrap();
    assert_eq!(head.headers()["content-type"], "application/pdf");

    let no_pdf = alice.repr_at(&docx, json!({"runtime": {"accept": ["image", "video"]}})).await;
    assert_eq!(no_pdf["result"]["reason"], "no-renderer", "{no_pdf}");

    // Named content (a bucket binding): materialized under the real extension.
    let lib = "user/photos/alice/library";
    let put = srv
        .http
        .put(format!("{}/bucket/v1/{lib}/decks/Q3 计划", srv.base))
        .bearer_auth(token("alice", "app:photos@alice"))
        .body(std::fs::read(fixture("office.pptx")).unwrap())
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success(), "{}", put.status());
    let deck = alice.converted(alice.call("repr", json!({"args": {"source": {"bucket": {"bucket_id": lib, "key": "decks/Q3 计划"}}}})).await).await;
    assert_eq!(deck["state"], "completed", "{deck}");
    assert_eq!(pdf_pages(&alice.fetch(deck["result"]["read_url"].as_str().unwrap()).await), 2);

    // An uploaded named object (`source.obj_id`, what AI Workspace's Office import sends).
    use ndn_lib::NamedObject;
    let bytes = std::fs::read(fixture("office.docx")).unwrap();
    let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(&bytes).unwrap();
    let (file_id, file_body) = ndn_lib::FileObject::new("周报.docx".into(), bytes.len() as u64, chunk.to_string()).gen_obj_id();
    let write = srv
        .http
        .put(format!("{}/ndm/proxy/v1/write/chunk/{}", srv.base, chunk.to_string()))
        .bearer_auth(&alice.token)
        .header("NDM-Chunk-Size", bytes.len().to_string())
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(write.status().is_success(), "{}", write.status());
    let manifest = json!({"objects": [{"obj_id": file_id.to_string(), "obj_data": file_body}], "roots": [file_id.to_string()]});
    let finalized: Value = srv.http.post(format!("{}/ndm/v1/import/finalize", srv.base)).bearer_auth(&alice.token).json(&manifest).send().await.unwrap().json().await.unwrap();
    assert_eq!(finalized["ready"], true, "{finalized}");
    let asset = alice.converted(alice.call("repr", json!({"args": {"source": {"obj_id": file_id.to_string()}, "purpose": "preview"}})).await).await;
    assert_eq!(asset["state"], "completed", "{asset}");
    assert_eq!((&asset["result"]["media_type"], &asset["result"]["result_type"]), (&json!("application/pdf"), &json!("document")));
    assert!(alice.fetch(asset["result"]["read_url"].as_str().unwrap()).await.starts_with(b"%PDF-"));
    let bob = srv.client("bob").await;
    let denied = bob.call("repr", json!({"args": {"source": {"obj_id": file_id.to_string()}}})).await;
    assert_eq!(denied["error"]["code"], "PERMISSION_DENIED", "{denied}");
}

#[tokio::test]
async fn missing_soffice_keeps_the_ffmpeg_profiles() {
    let tools = only_tools(&["ffmpeg", "ffprobe"]);
    let srv = start(Some(vec![tools.path().to_path_buf()])).await;
    let alice = srv.client("alice").await;
    let repr = &alice.hello["repr"];
    assert!(!repr["profiles"].as_array().unwrap().iter().any(|p| p == "preview-document"), "{repr}");
    assert!(repr["missing"].as_array().unwrap().iter().any(|m| m == "soffice"), "{repr}");
    assert!(repr.get("soffice").is_none(), "{repr}");
    let docx = alice.repr_at(&format!("{DIR}/office.docx"), json!({})).await;
    assert_eq!(docx["result"]["state"], "unsupported", "{docx}");
    assert_eq!(docx["result"]["reason"], "no-pipeline");
    let detail = docx["result"]["detail"].as_str().unwrap();
    assert!(detail.contains("soffice is not available on this node") && detail.contains("deployment defect"), "{docx}");

    if !tools_present() {
        return;
    }
    assert!(alice.hello["features"].as_array().unwrap().iter().any(|f| f == "repr"));
    assert_eq!(repr["profiles"], json!(["preview-image", "preview-video", "preview-audio", "thumbnail"]));
    assert_eq!(repr["available"], true);
    let heic = alice.converted(alice.repr_at(&format!("{DIR}/landscape_grid.heic"), json!({"purpose": "thumbnail"})).await).await;
    assert_eq!(heic["state"], "completed", "{heic}");
}

#[tokio::test]
async fn soffice_alone_advertises_repr() {
    if !soffice_present() {
        return;
    }
    let tools = only_tools(&["soffice"]);
    let srv = start(Some(vec![tools.path().to_path_buf()])).await;
    let alice = srv.client("alice").await;
    assert!(alice.hello["features"].as_array().unwrap().iter().any(|f| f == "repr"), "{}", alice.hello);
    let repr = &alice.hello["repr"];
    let profiles = repr["profiles"].as_array().unwrap();
    assert!(profiles.iter().all(|p| p == "preview-document" || p == "preview-sheet") && profiles[0] == "preview-document", "{repr}");
    assert!(repr.get("ffmpeg").is_none(), "{repr}");
    let heic = alice.repr_at(&format!("{DIR}/portrait_grid.heic"), json!({})).await;
    assert_eq!(heic["result"]["reason"], "no-pipeline", "{heic}");
    assert!(heic["result"]["detail"].as_str().unwrap().contains("ffmpeg and ffprobe not found"), "{heic}");
    let odt = alice.converted(alice.repr_at(&format!("{DIR}/office.odt"), json!({})).await).await;
    assert_eq!(odt["state"], "completed", "{odt}");
}

#[tokio::test]
async fn each_tool_group_is_advertised_on_its_own() {
    for (names, profiles) in [
        (&["magick"][..], json!(["preview-image", "thumbnail"])),
        (&["pdftoppm", "pdfinfo"][..], json!(["thumbnail"])),
        (&["pandoc"][..], json!(["preview-html"])),
    ] {
        if !present(names) {
            continue;
        }
        let tools = only_tools(names);
        let srv = start(Some(vec![tools.path().to_path_buf()])).await;
        let alice = srv.client("alice").await;
        let repr = &alice.hello["repr"];
        assert_eq!(repr["profiles"], profiles, "{names:?}: {repr}");
        assert_eq!(repr[names[0]]["source"], "system", "{repr}");
        assert!(repr["missing"].as_array().unwrap().iter().any(|m| m == "ffmpeg"), "{repr}");
        let heic = alice.repr_at(&format!("{DIR}/portrait_grid.heic"), json!({})).await;
        assert!(heic["result"]["detail"].as_str().unwrap().contains("ffmpeg and ffprobe not found"), "{heic}");
    }
    // Without pandoc an EPUB has no pipeline; half of poppler is none.
    let tools = only_tools(&["ffmpeg", "ffprobe", "pdftoppm"]);
    let srv = start(Some(vec![tools.path().to_path_buf()])).await;
    let alice = srv.client("alice").await;
    let epub = alice.repr_at(&format!("{DIR}/book.epub"), json!({})).await;
    assert_eq!(epub["result"]["reason"], "no-pipeline", "{epub}");
    assert!(epub["result"]["detail"].as_str().unwrap().contains("pandoc not found"), "{epub}");
    let pdf = alice.repr_at(&format!("{DIR}/pages.pdf"), json!({"purpose": "thumbnail"})).await;
    assert!(pdf["result"]["detail"].as_str().unwrap().contains("pdfinfo not found"), "{pdf}");
    let dng = alice.repr_at(&format!("{DIR}/raw.dng"), json!({})).await;
    assert!(dng["result"]["detail"].as_str().unwrap().contains("magick not found"), "{dng}");
}

#[tokio::test]
async fn stills_go_through_imagemagick() {
    if !present(&["magick", "ffmpeg", "ffprobe"]) {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    for name in ["raw.dng", "still.jxl", "still.jp2", "still.exr", "still.hdr", "still.dds", "still.tga"] {
        let done = alice.converted(alice.repr_at(&format!("{DIR}/{name}"), json!({"purpose": "preview", "runtime": {"accept": ["image"]}})).await).await;
        assert_eq!(done["state"], "completed", "{name}: {done}");
        let result = &done["result"];
        assert_eq!((&result["media_type"], &result["result_type"], &result["plan"]), (&json!("image/jpeg"), &json!("image"), &json!("magick-jpeg")), "{name}");
        assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(96), Some(64)), "{name}: {result}");
        let jpeg = alice.fetch(result["read_url"].as_str().unwrap()).await;
        assert_eq!(dimensions(&jpeg), (96, 64), "{name}");
        let (left, right) = (pixel(&jpeg, 12, 32), pixel(&jpeg, 84, 32));
        assert!(left[0] > 150 && left[2] < 140 && right[2] > 150 && right[0] < 100, "{name}: left {left:?} right {right:?}");
    }
    let thumb = alice.converted(alice.repr_at(&format!("{DIR}/raw.dng"), json!({"purpose": "thumbnail"})).await).await;
    assert_eq!(thumb["state"], "completed", "{thumb}");
    assert_eq!(thumb["result"]["plan"], "magick-jpeg");
    let bad = alice.converted(alice.repr_at(&format!("{DIR}/broken.jxl"), json!({})).await).await;
    assert_eq!(bad["state"], "failed", "{bad}");
    assert_eq!(bad["error"]["code"], "CONVERT_FAILED");
}

#[tokio::test]
async fn more_containers_reach_ffmpeg() {
    if !tools_present() {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    for name in ["clip.asf", "clip.flv", "clip.rm"] {
        let done = alice.converted(alice.repr_at(&format!("{DIR}/{name}"), json!({})).await).await;
        assert_eq!(done["state"], "completed", "{name}: {done}");
        assert_eq!((&done["result"]["result_type"], &done["result"]["plan"]), (&json!("video"), &json!("transcode")), "{name}");
        let thumb = alice.converted(alice.repr_at(&format!("{DIR}/{name}"), json!({"purpose": "thumbnail"})).await).await;
        assert_eq!(thumb["state"], "completed", "{name}: {thumb}");
    }
    for name in ["tone.wma", "tone.aiff", "tone.caf"] {
        let done = alice.converted(alice.repr_at(&format!("{DIR}/{name}"), json!({})).await).await;
        assert_eq!(done["state"], "completed", "{name}: {done}");
        assert_eq!(done["result"]["media_type"], "audio/mp4", "{name}: {done}");
    }
}

#[tokio::test]
async fn markup_becomes_one_html_page() {
    if !present(&["pandoc"]) {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    for (name, needle) in [
        ("book.epub", "第一章"),
        ("notebook.ipynb", "hello"),
        ("readme.md", "<table"),
    ] {
        let done = alice.converted(alice.repr_at(&format!("{DIR}/{name}"), json!({"runtime": {"accept": ["image", "html"]}})).await).await;
        assert_eq!(done["state"], "completed", "{name}: {done}");
        let result = &done["result"];
        assert_eq!((&result["result_type"], &result["plan"], &result["fidelity"]), (&json!("html"), &json!("pandoc-html"), &json!("已转换为网页显示")), "{name}");
        let read_url = result["read_url"].as_str().unwrap();
        let served = srv.http.get(format!("{}{}", srv.base, read_url)).send().await.unwrap();
        assert_ne!(served.headers()["content-type"], "text/html; charset=utf-8", "{name}: the data plane never serves a page to run in its origin");
        assert_eq!(result["media_type"], "text/html; charset=utf-8", "{name}");
        let html = String::from_utf8(alice.fetch(read_url).await).unwrap();
        assert!(html.contains(needle), "{name}: {html}");
        assert!(html.contains(&format!("<title>{name}</title>")) && html.contains("overflow-x: auto"), "{name}");
        assert!(!html.contains("<script") && !html.contains("src=\"http"), "{name}");
    }
    let epub = String::from_utf8(alice.fetch(alice.repr_at(&format!("{DIR}/book.epub"), json!({})).await["result"]["result"]["read_url"].as_str().unwrap()).await).unwrap();
    assert!(epub.contains("src=\"data:image/png;base64,"), "an EPUB's pictures are embedded");

    let md = String::from_utf8(alice.fetch(alice.repr_at(&format!("{DIR}/readme.md"), json!({})).await["result"]["result"]["read_url"].as_str().unwrap()).await).unwrap();
    assert!(md.contains("<math") && md.contains("type=\"checkbox\"") && md.contains("footnote-ref"), "math, task lists and footnotes: {md}");
    let no_html = alice.repr_at(&format!("{DIR}/readme.md"), json!({"runtime": {"accept": ["image", "pdf"]}})).await;
    assert_eq!(no_html["result"]["reason"], "no-renderer", "{no_html}");
    let thumb = alice.repr_at(&format!("{DIR}/readme.md"), json!({"purpose": "thumbnail"})).await;
    assert_eq!(thumb["result"]["reason"], "no-renderer", "{thumb}");
    // Text that only carries the name of a markup format is still read by pandoc, which reports what is wrong.
    let bad = alice.converted(alice.repr_at(&format!("{DIR}/broken.ipynb"), json!({})).await).await;
    assert_eq!(bad["state"], "failed", "{bad}");
}

#[tokio::test]
async fn pdfs_and_documents_have_thumbnails() {
    if !present(&["pdftoppm", "pdfinfo", "ffmpeg", "ffprobe"]) {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    let pdf = format!("{DIR}/pages.pdf");
    let preview = alice.repr_at(&pdf, json!({"purpose": "preview"})).await;
    assert_eq!((&preview["result"]["state"], &preview["result"]["reason"]), (&json!("unsupported"), &json!("no-pipeline")), "{preview}");
    let thumb = alice.converted(alice.repr_at(&pdf, json!({"purpose": "thumbnail", "runtime": {"accept": ["image"]}})).await).await;
    assert_eq!(thumb["state"], "completed", "{thumb}");
    assert_eq!((&thumb["result"]["media_type"], &thumb["result"]["plan"]), (&json!("image/jpeg"), &json!("pdf-page")));
    let (w, h) = dimensions(&alice.fetch(thumb["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!(w.max(h), 640, "{w}x{h}");

    let locked = alice.converted(alice.repr_at(&format!("{DIR}/locked.pdf"), json!({"purpose": "thumbnail"})).await).await;
    assert_eq!((&locked["state"], &locked["reason"]), (&json!("unsupported"), &json!("no-renderer")), "{locked}");
    assert!(locked["detail"].as_str().unwrap().contains("password"), "{locked}");

    // The picture a document carries: ODF, OOXML, an EPUB's cover (shrunk to the thumbnail size).
    for (name, size) in [("office.odt", None), ("thumb.docx", Some((96, 64))), ("book.epub", Some((426, 640)))] {
        let done = alice.converted(alice.repr_at(&format!("{DIR}/{name}"), json!({"purpose": "thumbnail"})).await).await;
        assert_eq!(done["state"], "completed", "{name}: {done}");
        assert_eq!((&done["result"]["plan"], &done["result"]["fidelity"]), (&json!("embedded"), &json!("文档内嵌的缩略图")), "{name}");
        let (w, h) = dimensions(&alice.fetch(done["result"]["read_url"].as_str().unwrap()).await);
        assert!(w <= 640 && h <= 640, "{name}: {w}x{h}");
        if let Some(size) = size {
            assert_eq!((w, h), size, "{name}");
        }
    }

    // None inside, no preview cached: unsupported, and soffice is not started for it.
    let docx = format!("{DIR}/office.docx");
    let none = alice.converted(alice.repr_at(&docx, json!({"purpose": "thumbnail"})).await).await;
    assert_eq!((&none["state"], &none["reason"]), (&json!("unsupported"), &json!("no-renderer")), "{none}");
    let again = alice.repr_at(&docx, json!({"purpose": "thumbnail"})).await;
    assert_eq!(again["result"]["state"], "unsupported", "{again}");
    if !soffice_present() {
        return;
    }
    // Once its preview PDF is cached, the thumbnail is that PDF's first page.
    let preview = alice.converted(alice.repr_at(&docx, json!({})).await).await;
    assert_eq!(preview["state"], "completed", "{preview}");
    let thumb = alice.converted(alice.repr_at(&docx, json!({"purpose": "thumbnail"})).await).await;
    assert_eq!(thumb["state"], "completed", "{thumb}");
    assert_eq!(thumb["result"]["plan"], "pdf-page");
    let (w, h) = dimensions(&alice.fetch(thumb["result"]["read_url"].as_str().unwrap()).await);
    assert_eq!(w.max(h), 640, "{w}x{h}");
}

#[tokio::test]
async fn legacy_spreadsheets_become_xlsx() {
    if !calc_present() {
        return;
    }
    let srv = start(None).await;
    let alice = srv.client("alice").await;
    assert!(alice.hello["repr"]["profiles"].as_array().unwrap().iter().any(|p| p == "preview-sheet"), "{}", alice.hello);
    for name in ["sheet.xls", "sheet.ods"] {
        let done = alice.converted(alice.repr_at(&format!("{DIR}/{name}"), json!({"runtime": {"accept": ["image", "pdf", "sheet"]}})).await).await;
        assert_eq!(done["state"], "completed", "{name}: {done}");
        let result = &done["result"];
        assert_eq!((&result["result_type"], &result["plan"]), (&json!("sheet"), &json!("soffice-xlsx")), "{name}");
        assert_eq!(result["media_type"], "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "{name}");
        let xlsx = alice.fetch(result["read_url"].as_str().unwrap()).await;
        assert_eq!(nfs_server::preview::media::sniff(&xlsx, None), Some(nfs_server::preview::media::Kind::Document("xlsx")), "{name}");
    }
    let no_sheet = alice.repr_at(&format!("{DIR}/sheet.xls"), json!({"runtime": {"accept": ["image", "pdf"]}})).await;
    assert_eq!(no_sheet["result"]["reason"], "no-renderer", "{no_sheet}");
}

/// A LibreOffice program directory with only the named applications' libraries, started through the real one.
#[cfg(target_os = "linux")]
fn partial_office(dir: &Path, modules: &[&str]) -> Option<PathBuf> {
    let real = std::fs::canonicalize(tool("soffice")?).ok()?;
    let program = dir.join("program");
    std::fs::create_dir_all(&program).unwrap();
    std::fs::write(program.join("soffice.bin"), b"").unwrap();
    for m in modules {
        std::fs::write(program.join(format!("lib{m}lo.so")), b"").unwrap();
    }
    let launcher = program.join("soffice");
    std::fs::write(&launcher, format!("#!/bin/sh\nexec '{}' \"$@\"\n", real.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
    Some(program)
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn an_install_without_calc_leaves_sheets_to_the_next_one() {
    if !soffice_present() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let no_calc = partial_office(&tmp.path().join("system"), &["sw", "sd"]).unwrap();
    let srv = start(Some(vec![no_calc.clone()])).await;
    let alice = srv.client("alice").await;
    let repr = &alice.hello["repr"];
    assert_eq!(repr["soffice"][0]["components"], json!(["writer", "impress"]), "{repr}");
    assert_eq!(repr["soffice_missing_components"], json!(["calc"]), "{repr}");
    assert!(!repr["profiles"].as_array().unwrap().iter().any(|p| p == "preview-sheet"), "{repr}");
    let xls = alice.repr_at(&format!("{DIR}/sheet.xls"), json!({})).await;
    assert_eq!((&xls["result"]["state"], &xls["result"]["reason"]), (&json!("unsupported"), &json!("no-pipeline")), "planned, not failed: {xls}");
    let detail = xls["result"]["detail"].as_str().unwrap();
    assert!(detail.contains("has no Calc") && detail.contains("deployment defect"), "{xls}");
    let docx = alice.converted(alice.repr_at(&format!("{DIR}/office.docx"), json!({})).await).await;
    assert_eq!(docx["state"], "completed", "{docx}");

    // A second install (the extension package's place in the lookup order) takes what the first lacks.
    let ext = tmp.path().join("ext/bin");
    std::fs::create_dir_all(&ext).unwrap();
    std::os::unix::fs::symlink(no_calc.join("soffice"), ext.join("soffice.real")).unwrap();
    std::fs::write(ext.join("soffice"), format!("#!/bin/sh\nexec '{}' \"$@\"\n", no_calc.join("soffice").display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(ext.join("soffice"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let srv = start(Some(vec![no_calc.clone(), ext.clone()])).await;
    let alice = srv.client("alice").await;
    let repr = &alice.hello["repr"];
    assert_eq!(repr["soffice"].as_array().unwrap().len(), 2, "{repr}");
    assert_eq!(repr["soffice"][1]["components"], json!(["writer", "calc", "impress"]), "{repr}");
    assert!(repr.get("soffice_missing_components").is_none(), "{repr}");
    assert!(repr["profiles"].as_array().unwrap().iter().any(|p| p == "preview-sheet"), "{repr}");
}

#[tokio::test]
async fn a_large_ole2_file_is_told_by_its_directory() {
    use nfs_server::preview::media::{sniff, Kind};
    // A 3 MiB compound file whose directory sits past the first MiB.
    let sector = 512usize;
    let directory_sector = 5000u32;
    let mut file = vec![0u8; (directory_sector as usize + 2) * sector];
    file[..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    file[30] = 9;
    file[48..52].copy_from_slice(&directory_sector.to_le_bytes());
    let name: Vec<u8> = "Workbook".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let at = (directory_sector as usize + 1) * sector + 128;
    file[at..at + name.len()].copy_from_slice(&name);
    file[at + 64..at + 66].copy_from_slice(&((name.len() + 2) as u16).to_le_bytes());
    assert_eq!(sniff(&file[..1024 * 1024], Some("big.et")), Some(Kind::Document("xls")), "the name still hints");
    assert_eq!(sniff(&file[..1024 * 1024], Some("big.bin")), None);

    let empty = only_tools(&["pandoc"]);
    let srv = start(Some(vec![empty.path().to_path_buf()])).await;
    std::fs::write(srv._tmp.path().join("exports/home/alice/big.bin"), &file).unwrap();
    let alice = srv.client("alice").await;
    if !present(&["pandoc"]) {
        return;
    }
    let planned = alice.repr_at("/home/alice/big.bin", json!({})).await;
    let detail = planned["result"]["detail"].as_str().unwrap();
    assert!(detail.contains("soffice is not available"), "recognized as a spreadsheet, which needs soffice: {planned}");
}

/// A docx whose only picture is a link to `url`.
fn docx_linking_to(url: &str) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let parts = [
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
        ("word/_rels/document.xml.rels", format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{url}" TargetMode="External"/></Relationships>"#)),
        ("word/document.xml", r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body><w:p><w:r><w:t>linked picture</w:t></w:r></w:p><w:p><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="609600"/><wp:docPr id="1" name="p"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="p"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:link="rId5"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="609600"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p></w:body></w:document>"#.to_string()),
    ];
    for (name, body) in parts {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
async fn soffice_does_not_fetch_what_a_document_links_to() {
    if !soffice_present() {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/tracker.png", listener.local_addr().unwrap());
    let srv = start(None).await;
    std::fs::write(srv._tmp.path().join("exports/home/alice/linked.docx"), docx_linking_to(&url)).unwrap();
    let alice = srv.client("alice").await;
    let done = alice.converted(alice.repr_at("/home/alice/linked.docx", json!({})).await).await;
    assert_eq!(done["state"], "completed", "{done}");
    let fetched = tokio::time::timeout(Duration::from_millis(500), listener.accept()).await;
    assert!(fetched.is_err(), "soffice connected to {url}");
}
