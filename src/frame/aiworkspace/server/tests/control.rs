//! Block-controls over kRPC (脑图 TODO M3): the catalogue, a mind map imported from a Markdown outline and from OPML
//! (one commit as the caller, nodes laid out by the control) and exported back — against the real process over HTTP.

#[path = "common/nfs.rs"]
mod nfs;

use aiworkspace_server::auth::StaticTokens;
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{build_router, AppState, Limits};
use aiworkspace_store::Service;
use serde_json::{json, Value};
use std::sync::Arc;

struct Srv {
    base: String,
    http: reqwest::Client,
    _nfs: nfs::Nfs,
}

impl Srv {
    async fn start(dir: &std::path::Path) -> Srv {
        let nfs = nfs::Nfs::start(dir).await;
        let svc = Service::open(&dir.join("data"), nfs.objects().await).unwrap();
        let tokens: StaticTokens = serde_json::from_value(json!({ "tokens": { "tok-alice": { "principal": "alice" } } })).unwrap();
        let state = AppState::new(svc, Arc::new(tokens), Limits::default(), None, WishConfig::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await.unwrap() });
        Srv { base: format!("http://{addr}/kapi/aiworkspace"), http: reqwest::Client::new(), _nfs: nfs }
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [1, "tok-alice"] })).send().await.unwrap().json().await.unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn import_and_export_a_mind_map() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Srv::start(dir.path()).await;
    let listed = srv.rpc("control.list", json!({})).await;
    assert!(listed["controls"].as_array().unwrap().iter().any(|c| c["kind"] == "mindmap"));
    assert!(srv.rpc("control.describe", json!({ "kind": "mindmap" })).await["control"]["agent"].as_str().unwrap().contains("\"auto\""));

    let ws = srv.rpc("ws.create", json!({ "title": "controls" })).await;
    let id = ws["workspace_id"].as_str().unwrap().to_string();
    let info = srv.rpc("ws.get_info", json!({ "workspace_id": id })).await;
    let commit = srv.rpc("doc.commit", json!({ "protocol_version": "1.1", "workspace_id": id, "epoch": info["epoch"], "idempotency_key": "setup", "operations": [
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "s" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "s", "content_folder_id": "sf-content" } },
    ] })).await;
    assert_eq!(commit["status"], "accepted", "{commit}");

    let md = "- Launch\n  - **Market**\n    - interviews\n  - Product\n";
    let imported = srv.rpc("control.import", json!({ "workspace_id": id, "parent_id": "sf", "format": "markdown", "text": md, "at": { "x": 100, "y": 80 } })).await;
    assert_eq!(imported["status"], "accepted", "{imported}");
    assert_eq!(imported["nodes"], 4);
    let container = imported["container_id"].as_str().unwrap().to_string();
    let outline = srv.rpc("doc.outline", json!({ "workspace_id": id })).await;
    let entities = outline["entities"].as_array().unwrap();
    let map = entities.iter().find(|e| e["entity_id"] == container.as_str()).unwrap();
    assert_eq!((map["kind"].as_str(), map["title"].as_str()), (Some("mindmap"), Some("Launch")));
    assert_eq!(map["placement"], json!({ "x": 100.0, "y": 80.0, "w": "auto", "h": "auto" }));
    let market = entities.iter().find(|e| e["title"] == "**Market**").unwrap();
    assert_eq!(market["placement"], json!({ "x": "auto", "y": "auto", "w": "auto", "h": "auto" }));
    assert_eq!(market["view_type"], "mindnode");

    let exported = srv.rpc("control.export", json!({ "workspace_id": id, "entity_id": container })).await;
    assert_eq!(exported["text"], md);
    let opml = srv.rpc("control.export", json!({ "workspace_id": id, "entity_id": container, "format": "opml" })).await;
    let again = srv.rpc("control.import", json!({ "workspace_id": id, "parent_id": "sf", "format": "opml", "text": opml["text"] })).await;
    assert_eq!(again["status"], "accepted", "{again}");
    let back = srv.rpc("control.export", json!({ "workspace_id": id, "entity_id": again["container_id"] })).await;
    assert_eq!(back["text"], md, "OPML carries the same tree");

    // not a control; an empty outline; an unknown format
    let refused: Value = srv.http.post(&srv.base).json(&json!({ "method": "control.export", "params": { "workspace_id": id, "entity_id": "sf" }, "sys": [2, "tok-alice"] })).send().await.unwrap().json().await.unwrap();
    assert!(refused.get("error").is_some() || refused["result"]["ok"] == false, "{refused}");
    let empty: Value = srv.http.post(&srv.base).json(&json!({ "method": "control.import", "params": { "workspace_id": id, "parent_id": "sf", "text": "" }, "sys": [3, "tok-alice"] })).send().await.unwrap().json().await.unwrap();
    assert!(empty.get("error").is_some() || empty["result"]["ok"] == false, "{empty}");
}
