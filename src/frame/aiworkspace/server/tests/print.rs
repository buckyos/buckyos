//! `print.package` over kRPC (打印 TODO T7): the package of a canvas lands in the owner's `exports` bucket and is handed
//! to the caller's App session (NDM holdings at nfs-server); the renderer comes from the Desktop build the service
//! serves (`--web`), and a service without one says so.

#[path = "common/nfs.rs"]
mod nfs;

use aiworkspace_server::auth::StaticTokens;
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{build_router, AppState, Limits};
use aiworkspace_store::Service;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::Arc;

const PRESENTATION: &str = include_str!("../../fixtures/presentation/commits.json");
const APP: &str = "app:control-panel@alice";

struct Srv {
    base: String,
    http: reqwest::Client,
    nfs: nfs::Nfs,
}

impl Srv {
    async fn start(dir: &std::path::Path, web: Option<std::path::PathBuf>) -> Srv {
        let nfs = nfs::Nfs::start(dir).await;
        let svc = Service::open(&dir.join("data"), nfs.objects().await).unwrap();
        let tokens: StaticTokens = serde_json::from_value(json!({ "tokens": {
            "tok-alice": { "principal": "alice", "app_id": APP }, "tok-bob": { "principal": "bob", "app_id": "app:control-panel@bob" } } }))
        .unwrap();
        let state = AppState::new(svc, Arc::new(tokens), Limits { web_dir: web, ..Limits::default() }, None, WishConfig::default());
        let _ = state.ndm_gateway.set(nfs.base.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await.unwrap() });
        Srv { base: format!("http://{addr}/kapi/aiworkspace"), http: reqwest::Client::new(), nfs }
    }

    async fn call(&self, token: &str, method: &str, params: Value) -> Value {
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [1, token] })).send().await.unwrap().json().await.unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    /// A Workspace of alice's with the presentation fixture.
    async fn courseware(&self) -> String {
        let created = self.call("tok-alice", "ws.create", json!({ "title": "课件" })).await;
        let (ws, epoch) = (created["workspace_id"].as_str().unwrap().to_string(), created["epoch"].as_str().unwrap().to_string());
        let fx: Value = serde_json::from_str(PRESENTATION).unwrap();
        for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
            let r = self.call("tok-alice", "doc.commit", json!({ "protocol_version": "1.2", "workspace_id": ws, "epoch": epoch,
                "idempotency_key": format!("fx/{i}"), "operations": c["operations"] })).await;
            assert_eq!(r["status"], "accepted", "{r}");
        }
        ws
    }
}

fn params(ws: &str) -> Value {
    json!({ "workspace_id": ws, "surface_id": "s-launch", "path_id": "path-intro", "paper": { "name": "16:9", "width_mm": 297, "height_mm": 167.06 },
            "date": "2026-11-16", "locale": "zh-CN" })
}

/// A Desktop build holding a print renderer.
fn web_dir(dir: &std::path::Path) -> std::path::PathBuf {
    let web = dir.join("web");
    std::fs::create_dir_all(web.join("aiws-print").join("fonts")).unwrap();
    std::fs::write(web.join("aiws-print").join("index.html"), "<!doctype html><script src=\"data.js\"></script><script src=\"print.js\"></script>").unwrap();
    std::fs::write(web.join("aiws-print").join("print.js"), "window.__AIWS_PRINT_READY__ = true;").unwrap();
    std::fs::write(web.join("aiws-print").join("fonts").join("x.css"), "").unwrap();
    std::fs::write(web.join("aiws-print").join("data.js"), "stale").unwrap();
    web
}

#[tokio::test(flavor = "multi_thread")]
async fn a_print_package_is_handed_to_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Srv::start(dir.path(), Some(web_dir(dir.path()))).await;
    let ws = srv.courseware().await;
    let r = srv.call("tok-alice", "print.package", params(&ws)).await;
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!((r["job"]["mode"].as_str(), r["job"]["title"].as_str(), r["job"]["paper"]["height_mm"].as_f64()), (Some("free"), Some("发布会"), Some(167.06)));
    assert_eq!(r["issues"], json!([]));
    let obj = r["obj_id"].as_str().unwrap();
    // the caller's App session holds it: its own `media.page` run may read it
    let holder = nfs_server::ndm::Principal::new("alice".into(), "app:control-panel".into(), APP.into(), None);
    assert!(nfs_server::ndm::holds(&srv.nfs.state, &holder, obj));
    let other = nfs_server::ndm::Principal::new("bob".into(), "app:control-panel".into(), "app:control-panel@bob".into(), None);
    assert!(!nfs_server::ndm::holds(&srv.nfs.state, &other, obj));
    // the zip: the renderer's files (its stale data.js replaced), the data, the assets
    let bytes = srv.nfs.objects().await;
    let data = aiworkspace_core::materialize::ObjectSource::get_file(bytes.as_ref(), obj).unwrap();
    assert_eq!(data.len() as u64, r["size"].as_u64().unwrap());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
    let mut names: Vec<String> = zip.file_names().map(str::to_string).collect();
    names.sort();
    assert_eq!(names, vec!["data.js", "fonts/x.css", "index.html", "print.js"]);
    let mut js = String::new();
    zip.by_name("data.js").unwrap().read_to_string(&mut js).unwrap();
    let package: Value = serde_json::from_str(js.strip_prefix("window.__AIWS_PRINT__ = ").unwrap().trim_end().trim_end_matches(';')).unwrap();
    assert_eq!(package["job"], r["job"]);
    assert_eq!(package["principal"], "alice");
}

#[tokio::test(flavor = "multi_thread")]
async fn printing_without_a_renderer_or_without_export_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Srv::start(dir.path(), None).await;
    let ws = srv.courseware().await;
    let r = srv.call("tok-alice", "print.package", params(&ws)).await;
    assert_eq!((r["error"]["code"].as_str(), r["error"]["sub_code"].as_str()), (Some("DEPENDENCY_UNAVAILABLE"), Some("PRINT_RENDERER_MISSING")), "{r}");
    let dir2 = tempfile::tempdir().unwrap();
    let srv = Srv::start(dir2.path(), Some(web_dir(dir2.path()))).await;
    let ws = srv.courseware().await;
    // not a member: the Workspace does not exist for bob
    let r = srv.call("tok-bob", "print.package", params(&ws)).await;
    assert_eq!(r["error"]["code"], "NOT_FOUND", "{r}");
    let grant = json!({ "workspace_id": ws, "changes": [{ "subject": "bob", "resource": "/", "allow": ["read"] }] });
    assert_eq!(srv.call("tok-alice", "acl.set", grant).await["ok"], true);
    let r = srv.call("tok-bob", "print.package", params(&ws)).await;
    assert_eq!((r["error"]["code"].as_str(), r["error"]["denial"]["reason"].as_str()), (Some("PERMISSION_DENIED"), Some("resource_grant_missing")), "{r}");
    let r = srv.call("tok-alice", "print.package", json!({ "workspace_id": ws, "surface_id": "s-launch", "paper": { "name": "A4", "width_mm": 210, "height_mm": 297 }, "date": "d", "locale": "en" })).await;
    assert_eq!((r["error"]["code"].as_str(), r["error"]["sub_code"].as_str()), (Some("INVALID_OPERATION"), Some("PRINT_PATH_REQUIRED")), "{r}");
    let mut bad = params(&ws);
    bad["paper"]["width_mm"] = json!(-1);
    assert_eq!(srv.call("tok-alice", "print.package", bad).await["error"]["code"], "INVALID_OPERATION");
    let mut frame = params(&ws);
    frame["frame_id"] = json!("frame1");
    let r = srv.call("tok-alice", "print.package", frame).await;
    assert_eq!(r["error"]["sub_code"], "PRINT_FRAME_UNUSABLE", "{r}");
}
