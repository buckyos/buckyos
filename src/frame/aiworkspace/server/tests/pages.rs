//! The Workspace's own origin (内核架构改进 T4-8, `notepads/aiworkspace-subdomain-todo.md` Q3/Q4): on `aiws-<id>.<zone>`
//! the service serves the AI Workspace page from the Desktop build, its files, publications by name, and sends a
//! Surface named without its id to the canonical `?id=` address; without a web directory the page is 503.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::json;
use wishkit::*;

fn web_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("aiws.html"), "<!doctype html><title>aiws page</title>").unwrap();
    std::fs::write(dir.path().join("index.html"), "<!doctype html><title>desktop</title>").unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/app-1234.js"), "console.log(1)").unwrap();
    std::fs::write(dir.path().join("sw.js"), "// worker").unwrap();
    dir
}

#[test]
fn the_workspace_host_serves_the_page_its_files_and_canonical_surface_links() {
    run_test(|| async move {
    let web = web_dir();
    let limits = aiworkspace_server::Limits { public_base: Some("https://aiws-{workspace_id}.zone.invalid".into()), web_dir: Some(web.path().to_path_buf()), ..Default::default() };
    let mut srv = Srv::start_with_limits(tempfile::tempdir().unwrap(), model(), json!({ "tok-alice": { "principal": "alice" } }), limits).await;
    let ws = srv.rpc("ws.create", json!({ "title": "Sales 2026" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.ws = ws.clone();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "sf-overview-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf-overview", "title": "总览 Overview" } },
        { "op": "entity.create", "entity_id": "sf-overview", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "总览 Overview", "content_folder_id": "sf-overview-content" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "orders", "payload": { "fields": [{ "field_id": "m", "name": "m", "type": "text" }] } },
    ])).await;
    srv.rpc("ws.publish", json!({ "entity_id": "orders", "name": "orders" })).await;
    let label = ws.strip_prefix("ws_").unwrap();
    let host = format!("aiws-{label}.zone.invalid");
    let origin = srv.base.trim_end_matches("/kapi/aiworkspace").to_string();
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let get = |path: &str, accept: &str| {
        let (http, origin, host) = (http.clone(), origin.clone(), host.clone());
        let path = path.to_string();
        let accept = accept.to_string();
        async move { http.get(format!("{origin}{path}")).header("x-forwarded-host", host).header("accept", accept).send().await.unwrap() }
    };
    let html = "text/html,application/xhtml+xml";

    // the page, wherever a person lands
    for path in ["/", "/sales-2026", "/anything", "/sales-2026/overview?id=sf-overview", "/sales-2026/orders?id=sf-overview", "/x/y?id=nope&block=b1"] {
        let r = get(path, html).await;
        assert_eq!(r.status(), 200, "{path}");
        assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/html"), "{path}");
        assert_eq!(r.text().await.unwrap(), "<!doctype html><title>aiws page</title>", "{path}");
    }
    // a Surface named without its id: the canonical address, the rest of the query kept
    let r = get("/sales-2026/overview", html).await;
    assert_eq!(r.status(), 307);
    assert_eq!(r.headers()["location"], "/sales-2026/overview?id=sf%2Doverview");
    let r = get("/sales-2026/sf-overview?block=b1", html).await;
    assert_eq!(r.headers()["location"], "/sales-2026/sf-overview?id=sf%2Doverview&block=b1");
    // a publication by name answers the publication, as JSON or as a page for a browser; with `id` it is the page
    let r = get("/sales-2026/orders", "application/json").await;
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!((v["name"].as_str(), v["workspace_id"].as_str()), (Some("orders"), Some(ws.as_str())), "{v}");
    let r = get("/sales-2026/orders", html).await;
    assert_ne!(r.text().await.unwrap(), "<!doctype html><title>aiws page</title>", "a publication's own prerendered page");
    // the Desktop build's files, as they are; the service's API namespace and deeper paths are not pages
    let r = get("/assets/app-1234.js", "*/*").await;
    assert_eq!((r.status().as_u16(), r.headers()["content-type"].to_str().unwrap(), r.headers()["cache-control"].to_str().unwrap()), (200, "text/javascript; charset=utf-8", "public, max-age=31536000, immutable"));
    assert_eq!(get("/sw.js", "*/*").await.text().await.unwrap(), "// worker");
    assert_eq!(get("/index.html", html).await.text().await.unwrap(), "<!doctype html><title>desktop</title>");
    // (a client normalises `%2E%2E` dot segments away before sending; a backslash segment is refused by the service)
    for path in ["/kapi/aiworkspace/nothing", "/a/b/c", "/assets/..%5Caiws.html", "/assets/missing.js/x"] {
        assert_eq!(get(path, html).await.status(), 404, "{path}");
    }
    // a request that is not for a Workspace host is no page either
    assert_eq!(http.get(format!("{origin}/sales-2026")).header("x-forwarded-host", "zone.invalid").send().await.unwrap().status(), 404);
    // the entry page names the Workspace's page on its own origin
    let created = srv.rpc("ws.create_entry", json!({ "capabilities": ["read"], "methods": { "guest": true } })).await;
    let described = srv.rpc("entry.describe", json!({ "entry_id": created["entry"]["entry_id"] })).await;
    assert_eq!(described["resource"]["path"], json!(format!("https://{host}/sales-2026")));
    });
}

#[test]
fn without_page_files_the_host_answers_unavailable() {
    run_test(|| async move {
    let limits = aiworkspace_server::Limits { public_base: Some("https://aiws-{workspace_id}.zone.invalid".into()), ..Default::default() };
    let mut srv = Srv::start_with_limits(tempfile::tempdir().unwrap(), model(), json!({ "tok-alice": { "principal": "alice" } }), limits).await;
    let ws = srv.rpc("ws.create", json!({ "title": "Home" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.ws = ws.clone();
    srv.commit(json!([{ "op": "entity.create", "entity_id": "d", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "d", "payload": { "fields": [{ "field_id": "m", "name": "m", "type": "text" }] } }])).await;
    let origin = srv.base.trim_end_matches("/kapi/aiworkspace").to_string();
    let host = format!("aiws-{}.zone.invalid", ws.strip_prefix("ws_").unwrap());
    let r = srv.http.get(format!("{origin}/")).header("x-forwarded-host", &host).send().await.unwrap();
    assert_eq!(r.status(), 503);
    // a title that is a path the gateway routes elsewhere (`/home/*` → HomeStation) is not used as the slug
    assert_eq!(srv.rpc("ws.publish", json!({ "entity_id": "d", "name": "d" })).await["path"], json!("/workspace/d"));
    });
}
