//! Extension controls over the real service (notepads/done/aiworkspace-extension-p0-todo.md M2): `ext.build` from
//! objects the caller may read (here: bucket uploads to the in-process nfs-server), git-like base revisions,
//! compatibility with instances, exports, the library (install from a Workspace or a `.pikg`, import, bundles).

#[path = "common/wishkit.rs"]
mod wishkit;
use aiworkspace_core::canonical::sha256_hex;
use base64::Engine;
use serde_json::{json, Value};
use std::io::Write;
use wishkit::*;

const ALICE: &str = "did:bns:alice";

fn component(name: &str, extra: Value) -> Value {
    let mut c = json!({ "manifest_version": 1, "ident": { "author_did": ALICE, "name": name }, "kind": "declarative", "title": "KPI card",
                        "accepts": ["buckyos.table-source", "buckyos.record"], "allow_no_source": true, "default_size": { "w": 240, "h": 120 },
                        "config_schema": { "type": "object", "properties": { "label": { "type": "string", "default": "Total" } } },
                        "bindings": { "sales": { "accepts": ["buckyos.table-source"], "title": "Sales" } },
                        "entry": "declarative.json", "catalog": { "group": "metrics", "description": "One number" }, "agent": "AGENT.md" });
    for (k, v) in extra.as_object().unwrap() {
        c[k] = v.clone();
    }
    c
}

fn decl(label: &str) -> Value {
    json!({ "layout": "stack", "items": [{ "kind": "metric", "label": label, "bind": "sales.sum:amount", "format": "number" }] })
}

/// Upload a source directory as alice's objects and answer `[{ name, obj_id }]`.
async fn upload(srv: &Srv, dir: &str, files: &[(&str, String)]) -> Value {
    let mut out = Vec::new();
    for (name, text) in files {
        let put = srv.nfs.put("alice", "assets", &format!("src/{dir}/{name}"), text.as_bytes().to_vec()).await;
        out.push(json!({ "name": name, "obj_id": put["obj_id"] }));
    }
    json!(out)
}

fn kpi_files(c: &Value, label: &str) -> Vec<(&'static str, String)> {
    vec![("component.json", serde_json::to_string_pretty(c).unwrap()), ("declarative.json", decl(label).to_string()), ("AGENT.md", "# KPI\nBind `sales`.".into())]
}

async fn workspace(srv: &mut Srv, title: &str) -> String {
    srv.ws = String::new();
    let ws = srv.rpc("ws.create", json!({ "title": title })).await["workspace_id"].as_str().unwrap().to_string();
    srv.ws = ws.clone();
    ws
}

#[test]
fn build_rebuild_conflicts_and_instances() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        let ws = workspace(&mut srv, "Controls").await;
        // a broken component.json: nothing is committed and the diagnostic points at the line
        let head = srv.rpc("ws.get_info", json!({})).await["head_seq"].clone();
        let broken = upload(&srv, "broken", &[("component.json", "{\n  \"manifest_version\": 1,\n  oops\n}".into())]).await;
        let r = srv.rpc("ext.build", json!({ "files": broken })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED", "{r}");
        let d = &r["error"]["data"]["diagnostics"][0];
        assert_eq!((d["file"].as_str(), d["line"].as_u64()), (Some("component.json"), Some(3)), "{r}");
        assert_eq!(srv.rpc("ws.get_info", json!({})).await["head_seq"], head);
        // schema problems are listed together
        let bad = component("kpi", json!({ "config_schema": { "type": "object", "pattern": "x" }, "deps": ["x"], "surprise": 1 }));
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "bad", &kpi_files(&bad, "Total")).await })).await;
        let messages: Vec<String> = r["error"]["data"]["diagnostics"].as_array().unwrap().iter().map(|d| d["message"].as_str().unwrap().to_string()).collect();
        assert!(messages.iter().any(|m| m.contains("pattern")) && messages.iter().any(|m| m.contains("deps")) && messages.iter().any(|m| m.contains("surprise")), "{messages:?}");

        // the first build: folder, source snapshot, definition and build record in one Commit
        let c = component("kpi", json!({}));
        let files = upload(&srv, "kpi", &kpi_files(&c, "Total")).await;
        let dry = srv.rpc("ext.build", json!({ "files": files, "dry_run": true })).await;
        assert_eq!(dry["status"], "checked", "{dry}");
        let b = srv.rpc("ext.build", json!({ "files": files })).await;
        assert_eq!((b["status"].as_str(), b["action"].as_str(), b["version"].as_u64()), (Some("accepted"), Some("created"), Some(1)), "{b}");
        let def = b["def_entity_id"].as_str().unwrap().to_string();
        let read = srv.rpc("doc.read", json!({ "entity_id": def })).await;
        let payload = &read["content"]["payload"];
        assert_eq!(payload["def_id"], "kpi");
        assert_eq!(payload["declarative"]["items"][0]["bind"], "sales.sum:amount");
        assert_eq!(payload["agent"], "# KPI\nBind `sales`.");
        assert_eq!(payload["source_dir"]["entity_id"], b["dir_entity_id"]);
        assert_eq!(read["derived"]["build"]["builder_version"], "aiws-ext-build/1", "{read}");
        let outline = srv.rpc("doc.outline", json!({})).await;
        let e = outline["entities"].as_array().unwrap().iter().find(|e| e["entity_id"] == def).unwrap().clone();
        assert_eq!((e["ident"].as_str(), e["name"].as_str()), (Some("did:bns:alice#kpi"), Some("did:bns:alice#kpi")), "{e}");
        let folder = outline["entities"].as_array().unwrap().iter().find(|x| x["name"] == "extensions").unwrap();
        assert_eq!(folder["parent_id"], "data");

        // same files again: nothing to commit
        let base = json!({ "def_rev": b["def_rev"], "dir_rev": b["dir_rev"] });
        let same = srv.rpc("ext.build", json!({ "files": files, "expect": base })).await;
        assert_eq!(same["unchanged"], true, "{same}");
        // a change without a base, then with a stale one: refused, git-like
        let files2 = upload(&srv, "kpi2", &kpi_files(&c, "Revenue")).await;
        let r = srv.rpc("ext.build", json!({ "files": files2 })).await;
        assert_eq!((r["error"]["code"].as_str(), r["error"]["sub_code"].as_str()), (Some("REVISION_CONFLICT"), Some("BUILD_BASE_REQUIRED")), "{r}");
        let r = srv.rpc("ext.build", json!({ "files": files2, "expect": { "def_rev": 99, "dir_rev": 99 } })).await;
        assert_eq!(r["error"]["sub_code"], "BUILD_BASE_STALE", "{r}");
        let b2 = srv.rpc("ext.build", json!({ "files": files2, "expect": base })).await;
        assert_eq!((b2["action"].as_str(), b2["version"].as_u64()), (Some("rebuilt"), Some(2)), "{b2}");
        assert_eq!(b2["def_entity_id"], json!(def));
        assert_eq!(srv.rpc("doc.read", json!({ "entity_id": def })).await["content"]["payload"]["declarative"]["items"][0]["label"], "Revenue");

        // an instance holds the contract: narrowing it fails the build and names the instance
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "s" } },
            { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "s", "content_folder_id": "sf-content" } },
            { "op": "entity.create", "entity_id": "cell-1", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 240, "h": 120 },
              "payload": { "view": { "type": "declarative" }, "def_ref": { "entity_id": def } } },
        ])).await;
        let mut narrow = c.clone();
        narrow["bindings"]["sales"]["required"] = json!(true);
        let base2 = json!({ "def_rev": b2["def_rev"], "dir_rev": b2["dir_rev"] });
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi3", &kpi_files(&narrow, "Revenue")).await, "expect": base2 })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED", "{r}");
        assert_eq!(r["error"]["data"]["diagnostics"][0]["instances"][0]["entity_id"], "cell-1", "{r}");
        // an html control from inline bytes (a client of another Zone), in the same folder
        let html = json!({ "manifest_version": 1, "ident": { "author_did": ALICE, "name": "clock" }, "kind": "html", "title": "Clock", "allow_no_source": true,
                           "entry": { "html": "view.html", "css": "style.css" } });
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        let h = srv.rpc("ext.build", json!({ "files": [{ "name": "component.json", "base64": b64(&html.to_string()) }, { "name": "view.html", "base64": b64("<div id=t></div>") },
                                                         { "name": "style.css", "base64": b64("#t{color:red}") }] })).await;
        assert_eq!(h["status"], "accepted", "{h}");
        let hp = srv.rpc("doc.read", json!({ "entity_id": h["def_entity_id"] })).await["content"]["payload"].clone();
        assert_eq!((hp["html"]["html"].as_str(), hp["html"]["css"].as_str(), hp["html"]["api_version"].as_u64()), (Some("<div id=t></div>"), Some("#t{color:red}"), Some(2)));
        // the source snapshot cannot be deleted under its definition
        let dir = b["dir_entity_id"].as_str().unwrap();
        let life = srv.rpc("doc.read", json!({ "entity_id": dir })).await["life_rev"].clone();
        let del = srv.commit_raw(json!([{ "op": "entity.delete", "entity_id": dir, "expect": { "rev": life } }])).await;
        assert_eq!(del["code"], "REFERENCE_BROKEN", "{del}");
        let _ = ws;
    })
}

#[test]
fn export_bundle_library_and_imports() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        let ws1 = workspace(&mut srv, "Dev").await;
        let c = component("kpi", json!({}));
        let b = srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi", &kpi_files(&c, "Total")).await })).await;
        let def = b["def_entity_id"].as_str().unwrap().to_string();
        // a definition never built from source (a template's): pulled as synthesized files
        srv.commit(json!([{ "op": "entity.create", "entity_id": "def-inline", "type_id": "buckyos.block-def", "parent_id": "data", "order_key": "z", "name": "inline",
                            "payload": { "def_id": "sample.kpi-card", "kind": "declarative", "title": "Inline KPI", "declarative": { "items": [] } } }])).await;
        let pull = srv.rpc("ext.export", json!({ "entity_ids": ["def-inline"], "purpose": "pull" })).await;
        let names: Vec<&str> = pull["controls"][0]["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["component.json", "declarative.json"], "{pull}");
        assert!(pull["controls"][0]["files"][0]["text"].as_str().unwrap().contains(&format!("did:aiws:{ws1}")));

        // export: the manifest names each control with the digest of its files
        let ex = srv.rpc("ext.export", json!({ "entity_ids": [def] })).await;
        let control = &ex["manifest"]["controls"][0];
        assert_eq!((control["ident"].as_str(), control["dir"].as_str(), control["version"].as_u64()), (Some("did:bns:alice#kpi"), Some("controls/kpi/"), Some(1)), "{ex}");
        let mut pairs: Vec<(String, String)> = Vec::new();
        for f in ex["controls"][0]["files"].as_array().unwrap() {
            let bytes = srv.asset_bytes(&ws1, f["obj_id"].as_str().unwrap()).await;
            pairs.push((f["name"].as_str().unwrap().to_string(), sha256_hex(&bytes)));
        }
        pairs.sort();
        let digest = format!("sha256:{}", sha256_hex(aiworkspace_core::canonical::canonical_json(&json!(pairs)).unwrap().as_bytes()));
        assert_eq!(control["digest"], json!(digest));
        assert_eq!(ex["manifest"]["workspace_templates"], json!([]));

        // global: straight into alice's library; a second time is satisfied, another content at that version refused
        let inst = srv.rpc("ext.install", json!({ "from": { "workspace_id": ws1, "entity_ids": [def] }, "package": { "name": "acme-kpi", "version": "1.0.0" } })).await;
        assert_eq!((inst["action"].as_str(), inst["pkg_id"].as_str()), (Some("installed"), Some("acme-kpi")), "{inst}");
        assert_eq!(srv.rpc("ext.install", json!({ "from": { "workspace_id": ws1, "entity_ids": [def] }, "package": { "name": "acme-kpi", "version": "1.0.0" } })).await["action"], "satisfied");
        let base = json!({ "def_rev": b["def_rev"], "dir_rev": b["dir_rev"] });
        srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi-b", &kpi_files(&c, "Changed")).await, "expect": base })).await;
        let again = srv.rpc("ext.install", json!({ "from": { "workspace_id": ws1, "entity_ids": [def] }, "package": { "name": "acme-kpi", "version": "1.0.0" } })).await;
        assert_eq!(again["error"]["code"], "LIBRARY_PACKAGE_INVALID", "{again}");
        let lib = srv.rpc("ext.list_library", json!({})).await;
        assert_eq!(lib["packages"][0]["controls"][0]["ident"], "did:bns:alice#kpi", "{lib}");

        // import into another Workspace: one Commit, provenance recorded; again: satisfied
        let ws2 = workspace(&mut srv, "Use").await;
        let im = srv.rpc("ext.import", json!({ "items": [{ "pkg_id": "acme-kpi", "ident": "did:bns:alice#kpi" }] })).await;
        assert_eq!((im["status"].as_str(), im["controls"][0]["action"].as_str()), (Some("accepted"), Some("created")), "{im}");
        let def2 = im["controls"][0]["def_entity_id"].as_str().unwrap().to_string();
        let p2 = srv.rpc("doc.read", json!({ "entity_id": def2 })).await["content"]["payload"].clone();
        assert_eq!((p2["provenance"]["pkg_id"].as_str(), p2["provenance"]["version"].as_str()), (Some("acme-kpi"), Some("1.0.0")));
        assert_eq!(p2["declarative"]["items"][0]["label"], "Total");
        let im2 = srv.rpc("ext.import", json!({ "items": [{ "pkg_id": "acme-kpi", "ident": "did:bns:alice#kpi" }] })).await;
        assert_eq!(im2["controls"][0]["action"], "satisfied", "{im2}");

        // across Zones: the bundle's bytes built into a Workspace with provenance (what a client relays)
        let bundle = srv.rpc("ext.library_bundle", json!({ "pkg_id": "acme-kpi" })).await;
        let c0 = &bundle["controls"][0];
        let files: Vec<Value> = c0["files"].as_array().unwrap().iter().map(|f| json!({ "name": f["name"], "base64": f["base64"] })).collect();
        let _ws3 = workspace(&mut srv, "Elsewhere").await;
        let relayed = srv.rpc("ext.build", json!({ "files": files, "provenance": c0["provenance"] })).await;
        assert_eq!(relayed["action"], "created", "{relayed}");
        let relayed2 = srv.rpc("ext.build", json!({ "files": files, "provenance": c0["provenance"] })).await;
        assert_eq!(relayed2["action"], "satisfied", "{relayed2}");

        assert_eq!(srv.rpc("ext.uninstall", json!({ "pkg_id": "acme-kpi" })).await["removed"], "acme-kpi");
        assert_eq!(srv.rpc("ext.list_library", json!({})).await["packages"], json!([]));
        // the Workspaces keep their copies
        srv.ws = ws2;
        assert_eq!(srv.rpc("doc.read", json!({ "entity_id": def2 })).await["content"]["payload"]["def_id"], "kpi");
    })
}

/// An `aiws` PIKG as the CLI packs it: APPDOC.json, PACKAGE_META.json, `aiws.tar.gz` (the bundle).
fn pikg(files: &[(&str, Vec<u8>)], tamper: bool) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, path, bytes.as_slice()).unwrap();
    }
    let tar = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).unwrap();
    let payload = gz.finish().unwrap();
    let digest = format!("sha256:{}", sha256_hex(&payload));
    let app_doc = json!({ "schema_version": 1, "doc_type": "app", "did": "did:bns:kpi-pack", "version": "2.0.0", "app_type": "aiws", "show_name": "KPI pack",
                          "owner": ALICE, "controller": ALICE, "author": ALICE, "selector_type": "single", "categories": ["aiws"],
                          "pkg_list": { "aiws": { "pkg_objid": "pkg:00" } } });
    let meta = json!({ "@schema": "buckyos.pikg.package-meta.v1", "app_doc_id": "appdoc:00", "package_objects": {},
                       "content_index": { digest.clone(): { "sub_pkg_name": "aiws", "path": "aiws.tar.gz", "format": "tar.gz", "size": payload.len(), "digest": digest } } });
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in [("APPDOC.json", app_doc.to_string().into_bytes()), ("PACKAGE_META.json", meta.to_string().into_bytes()),
                          ("aiws.tar.gz", if tamper { let mut p = payload.clone(); p.push(0); p } else { payload })] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn install_a_pikg_and_refuse_a_tampered_one() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        workspace(&mut srv, "Pkg").await;
        let c = component("gauge", json!({}));
        let mut files: Vec<(String, Vec<u8>)> = kpi_files(&c, "Gauge").into_iter().map(|(n, t)| (format!("controls/gauge/{n}"), t.into_bytes())).collect();
        let pairs: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.strip_prefix("controls/gauge/").unwrap(), b.as_slice())).collect();
        let digest = aiworkspace_store::ext::content_digest(pairs);
        let manifest = json!({ "manifest_version": 1, "purpose": "aiws", "host_api": { "declarative": 1 }, "workspace_templates": [],
                               "controls": [{ "ident": "did:bns:alice#gauge", "name": "gauge", "kind": "declarative", "version": 1, "digest": digest, "dir": "controls/gauge/", "title": "Gauge" }] });
        files.push(("aiws-manifest.json".into(), manifest.to_string().into_bytes()));
        let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
        let good = srv.nfs.put("alice", "exports", "kpi.pikg", pikg(&refs, false)).await;
        let r = srv.rpc("ext.install", json!({ "pikg": { "obj_id": good["obj_id"] } })).await;
        assert_eq!((r["action"].as_str(), r["pkg_id"].as_str(), r["version"].as_str()), (Some("installed"), Some("kpi-pack.bns.did"), Some("2.0.0")), "{r}");
        let im = srv.rpc("ext.import", json!({ "items": [{ "pkg_id": "kpi-pack.bns.did", "ident": "did:bns:alice#gauge" }] })).await;
        assert_eq!(im["controls"][0]["action"], "created", "{im}");
        // a package bound into alice's own exports bucket installs as well, and is consumed
        let bound = srv.nfs.put("alice", "exports", "aiws/kpi.pikg", pikg(&refs, false)).await;
        let again = srv.rpc("ext.install", json!({ "pikg": { "bucket_id": bound["bucket_id"], "key": bound["key"] } })).await;
        assert_eq!(again["action"], "satisfied", "{again}");
        assert!(srv.nfs.bound("alice", "exports", "aiws/kpi.pikg").await.is_none());
        let foreign = srv.rpc("ext.install", json!({ "pikg": { "bucket_id": "user/aiworkspace/bob/exports", "key": "x" } })).await;
        assert_eq!(foreign["error"]["code"], "PERMISSION_DENIED", "{foreign}");
        // tampered payload, or a file changed behind the manifest's digest
        let bad = srv.nfs.put("alice", "exports", "bad.pikg", pikg(&refs, true)).await;
        assert_eq!(srv.rpc("ext.install", json!({ "pikg": { "obj_id": bad["obj_id"] } })).await["error"]["code"], "LIBRARY_PACKAGE_INVALID");
        let mut changed = refs.clone();
        changed[1].1 = decl("Hacked").to_string().into_bytes();
        let bad2 = srv.nfs.put("alice", "exports", "bad2.pikg", pikg(&changed, false)).await;
        let r = srv.rpc("ext.install", json!({ "pikg": { "obj_id": bad2["obj_id"] } })).await;
        assert_eq!(r["error"]["code"], "LIBRARY_PACKAGE_INVALID", "{r}");
        assert!(r["error"]["detail"].as_str().unwrap().contains("digest"), "{r}");
        // installer mode is the App Installer's
        let r = srv.rpc("ext.install", json!({ "pikg": { "obj_id": good["obj_id"] }, "mode": "installer", "owner": "bob" })).await;
        assert_eq!(r["error"]["code"], "PERMISSION_DENIED", "{r}");
    })
}

/// A composite board over `card` controls (P1): its component.json and composition.json.
fn board_files(composition: Value, extra: Value) -> Vec<(&'static str, String)> {
    let mut c = json!({ "manifest_version": 2, "ident": { "author_did": ALICE, "name": "board" }, "kind": "composite", "title": "Board",
                        "config_schema": { "type": "object", "properties": { "title": { "type": "string", "default": "Today" }, "gap": { "type": "integer", "default": 8 } } },
                        "entry": "composition.json", "deps": ["did:bns:alice#card"] });
    for (k, v) in extra.as_object().unwrap() {
        c[k] = v.clone();
    }
    vec![("component.json", c.to_string()), ("composition.json", composition.to_string())]
}

fn composition() -> Value {
    json!({ "layout": { "algorithm": "stack", "direction": "column", "spacing": "{{config.gap}}", "padding": 12 },
            "slots": { "header": { "max": 1, "accepts": ["richtext"], "placement": "top" }, "cards": { "accepts": ["cell:did:bns:alice#card", "shape"] } },
            "chrome": { "background": "#f6f7fb", "radius": 12, "title": { "text": "{{config.title}}", "slot": "header" }, "empty": "Drop cards here" } })
}

fn diag_text(r: &Value) -> String {
    r["error"]["data"]["diagnostics"].to_string()
}

#[test]
fn composite_controls_with_dependencies() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        let ws1 = workspace(&mut srv, "Boards").await;
        // the board needs the card first
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "board0", &board_files(composition(), json!({}))).await })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED", "{r}");
        assert!(r["error"]["data"]["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "DEP_MISSING"), "{r}");
        let card = srv.rpc("ext.build", json!({ "files": upload(&srv, "card", &kpi_files(&component("card", json!({})), "Card")).await })).await;
        let card_id = card["def_entity_id"].as_str().unwrap_or_else(|| panic!("{card}")).to_string();
        // what composition.json may say
        for (comp, extra, needle) in [
            (json!({ "layout": { "algorithm": "stack" }, "slots": {}, "events": {} }), json!({}), "events"),
            (json!({ "layout": { "algorithm": "grid" }, "slots": {} }), json!({}), "layout.algorithm"),
            (json!({ "layout": { "algorithm": "fixed" }, "slots": { "a": { "placement": "stack" } } }), json!({}), "fixed"),
            (json!({ "layout": { "algorithm": "stack", "spacing": "{{config.nope}}" }, "slots": {} }), json!({}), "config.nope"),
            (json!({ "layout": { "algorithm": "stack" }, "slots": { "Bad": {} } }), json!({}), "slot name"),
            (json!({ "layout": { "algorithm": "stack" }, "slots": { "a": { "accepts": ["cell:did:bns:bob#other"] } } }), json!({}), "cell:did:bns:bob#other"),
            (composition(), json!({ "manifest_version": 1 }), "manifest_version 2"),
            (composition(), json!({ "accepts": ["buckyos.record"] }), "container of Blocks"),
            (composition(), json!({ "deps": ["did:bns:alice#board"] }), "itself"),
        ] {
            let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "board-bad", &board_files(comp.clone(), extra.clone())).await })).await;
            assert_eq!(r["error"]["code"], "BUILD_FAILED", "{comp} {extra}: {r}");
            assert!(diag_text(&r).contains(needle), "{needle}: {r}");
        }
        let b = srv.rpc("ext.build", json!({ "files": upload(&srv, "board", &board_files(composition(), json!({}))).await })).await;
        assert_eq!(b["status"], "accepted", "{b}");
        let board_id = b["def_entity_id"].as_str().unwrap().to_string();
        let payload = srv.rpc("doc.read", json!({ "entity_id": board_id })).await["content"]["payload"].clone();
        assert_eq!((payload["kind"].as_str(), payload["manifest_version"].as_u64()), (Some("composite"), Some(2)));
        assert_eq!(payload["deps"], json!([{ "ident": "did:bns:alice#card", "entity_id": card_id }]));
        assert_eq!(payload["composition"]["slots"]["cards"]["accepts"][0], "cell:did:bns:alice#card");

        // an instance: a composite container with a slotted card
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "s" } },
            { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "s", "content_folder_id": "sf-content" } },
            { "op": "entity.create", "entity_id": "board-1", "type_id": "buckyos.container", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": "auto", "h": "auto" },
              "payload": { "kind": "composite", "def_ref": { "entity_id": board_id }, "config": { "title": "Sprint", "gap": 4 } } },
            { "op": "entity.create", "entity_id": "card-1", "type_id": "buckyos.cell", "parent_id": "board-1", "order_key": "a", "placement": { "x": "auto", "y": "auto", "w": 240, "h": 120 },
              "payload": { "view": { "type": "declarative" }, "def_ref": { "entity_id": card_id }, "slot": "cards" } },
        ])).await;
        // narrowing the card names the board that builds on it, and the card's instance; nothing is committed
        let mut narrow = component("card", json!({}));
        narrow["bindings"]["sales"]["required"] = json!(true);
        let base = json!({ "def_rev": card["def_rev"], "dir_rev": card["dir_rev"] });
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "card-n", &kpi_files(&narrow, "Card")).await, "expect": base })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED", "{r}");
        let diags = r["error"]["data"]["diagnostics"].as_array().unwrap();
        assert!(diags.iter().any(|d| d["file"] == "did:bns:alice#board" && d["dependents"][0]["def_entity_id"] == json!(board_id)), "{r}");
        assert!(diags.iter().any(|d| d["instances"][0]["entity_id"] == "card-1"), "{r}");
        // the board may change its layout and look freely, but not strand the Blocks of its instances
        let bbase = json!({ "def_rev": b["def_rev"], "dir_rev": b["dir_rev"] });
        let mut tighter = composition();
        tighter["slots"].as_object_mut().unwrap().remove("cards");
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "board-n", &board_files(tighter, json!({}))).await, "expect": bbase })).await;
        assert!(diag_text(&r).contains("slot cards, which the control no longer has") && diag_text(&r).contains("board-1"), "{r}");
        let mut row = composition();
        row["layout"] = json!({ "algorithm": "stack", "direction": "row", "spacing": 24 });
        let b2 = srv.rpc("ext.build", json!({ "files": upload(&srv, "board-row", &board_files(row, json!({}))).await, "expect": bbase })).await;
        assert_eq!((b2["action"].as_str(), b2["version"].as_u64()), (Some("rebuilt"), Some(2)), "{b2}");
        let kids = srv.rpc("doc.read", json!({ "entity_id": "card-1" })).await;
        assert_eq!(kids["content"]["payload"]["slot"], "cards", "the instance's Blocks are untouched");
        // the card cannot be deleted while the board builds on it
        let life = srv.rpc("doc.read", json!({ "entity_id": card_id })).await["life_rev"].clone();
        assert_eq!(srv.commit_raw(json!([{ "op": "entity.delete", "entity_id": card_id, "expect": { "rev": life } }])).await["code"], "REFERENCE_BROKEN");

        // a package export carries the card once; a pull does not
        let ex = srv.rpc("ext.export", json!({ "entity_ids": [board_id, card_id, "board-1"] })).await;
        let idents: Vec<&str> = ex["manifest"]["controls"].as_array().unwrap().iter().map(|c| c["ident"].as_str().unwrap()).collect();
        assert_eq!(idents.len(), 2, "{ex}");
        let board_entry = ex["manifest"]["controls"].as_array().unwrap().iter().find(|c| c["kind"] == "composite").unwrap();
        assert_eq!(board_entry["deps"], json!(["did:bns:alice#card"]));
        assert_eq!(ex["manifest"]["manifest_version"], 3);
        let only = srv.rpc("ext.export", json!({ "entity_ids": [board_id] })).await;
        assert_eq!(only["manifest"]["controls"].as_array().unwrap().len(), 2, "the closure follows def_dep");
        let pull = srv.rpc("ext.export", json!({ "entity_ids": [board_id], "purpose": "pull" })).await;
        assert_eq!(pull["controls"].as_array().unwrap().len(), 1);

        // into the library, then into a fresh Workspace: asking for the board brings the card, card first
        let inst = srv.rpc("ext.install", json!({ "from": { "workspace_id": ws1, "entity_ids": [board_id] }, "package": { "name": "boards", "version": "1.0.0" } })).await;
        assert_eq!(inst["action"], "installed", "{inst}");
        let lib = srv.rpc("ext.list_library", json!({})).await;
        let listed = lib["packages"][0]["controls"].as_array().unwrap().iter().find(|c| c["ident"] == "did:bns:alice#board").unwrap().clone();
        assert_eq!(listed["deps"], json!(["did:bns:alice#card"]), "{lib}");
        workspace(&mut srv, "Fresh").await;
        let im = srv.rpc("ext.import", json!({ "items": [{ "pkg_id": "boards", "ident": "did:bns:alice#board" }] })).await;
        assert_eq!(im["status"], "accepted", "{im}");
        let order: Vec<&str> = im["controls"].as_array().unwrap().iter().map(|c| c["ident"].as_str().unwrap()).collect();
        assert_eq!(order, ["did:bns:alice#card", "did:bns:alice#board"]);
        let new_card = im["controls"][0]["def_entity_id"].as_str().unwrap().to_string();
        let nb = srv.rpc("doc.read", json!({ "entity_id": im["controls"][1]["def_entity_id"] })).await["content"]["payload"].clone();
        assert_eq!(nb["deps"][0]["entity_id"], json!(new_card));
        // a Workspace that has its own card keeps it: the board builds on what is there
        workspace(&mut srv, "Own card").await;
        let own = srv.rpc("ext.build", json!({ "files": upload(&srv, "card-own", &kpi_files(&component("card", json!({})), "Mine")).await })).await;
        let im = srv.rpc("ext.import", json!({ "items": [{ "pkg_id": "boards", "ident": "did:bns:alice#board" }] })).await;
        assert_eq!((im["controls"][0]["action"].as_str(), im["controls"][1]["action"].as_str()), (Some("satisfied"), Some("created")), "{im}");
        assert_eq!(im["controls"][0]["def_entity_id"], own["def_entity_id"]);

        // ext.pack: the service writes the .pikg into alice's exports bucket; the same bundle packs the same bytes,
        // and the package installs (read_pikg + verify_bundle) like one the CLI made
        srv.ws = ws1.clone();
        let p1 = srv.rpc("ext.pack", json!({ "entity_ids": [board_id], "package": { "name": "boards", "version": "2.0.0", "title": "Boards" } })).await;
        assert_eq!((p1["key"].as_str(), p1["pkg_id"].as_str(), p1["app_did"].as_str()), (Some("boards-2.0.0.pikg"), Some("boards.alice.bns.did"), Some("did:bns:boards.alice")), "{p1}");
        assert!(p1["app_doc_object_id"].as_str().unwrap().starts_with("appdoc:"), "{p1}");
        let p2 = srv.rpc("ext.pack", json!({ "entity_ids": [board_id, card_id], "package": { "name": "boards", "version": "2.0.0", "title": "Boards" } })).await;
        assert_eq!(p1["digest"], p2["digest"], "deterministic");
        let r = srv.rpc("ext.install", json!({ "pikg": { "bucket_id": p2["bucket_id"], "key": p2["key"] }, "mode": "dev" })).await;
        assert_eq!((r["action"].as_str(), r["pkg_id"].as_str(), r["version"].as_str()), (Some("installed"), Some("boards.alice.bns.did"), Some("2.0.0")), "{r}");
        assert_eq!(r["controls"].as_array().unwrap().len(), 2);
        let bad = srv.rpc("ext.pack", json!({ "entity_ids": [card_id], "package": { "name": "Bad Name" } })).await;
        assert_eq!(bad["error"]["code"], "INVALID_OPERATION", "{bad}");
        // a package that leaves out what a control builds on is refused
        let files: Vec<(String, Vec<u8>)> = board_files(composition(), json!({})).into_iter().map(|(n, t)| (format!("controls/board/{n}"), t.into_bytes())).collect();
        let digest = aiworkspace_store::ext::content_digest(files.iter().map(|(p, b)| (p.strip_prefix("controls/board/").unwrap(), b.as_slice())));
        let manifest = json!({ "manifest_version": 2, "purpose": "aiws", "workspace_templates": [],
                               "controls": [{ "ident": "did:bns:alice#board", "name": "board", "kind": "composite", "version": 1, "digest": digest, "dir": "controls/board/", "deps": ["did:bns:alice#card"] }] });
        let mut refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
        let m = manifest.to_string().into_bytes();
        refs.push(("aiws-manifest.json", m));
        let open = srv.nfs.put("alice", "exports", "open.pikg", pikg(&refs, false)).await;
        let r = srv.rpc("ext.install", json!({ "pikg": { "obj_id": open["obj_id"] } })).await;
        assert_eq!(r["error"]["code"], "LIBRARY_PACKAGE_INVALID", "{r}");
        assert!(r["error"]["detail"].as_str().unwrap().contains("does not carry"), "{r}");
    })
}

fn card_v3(events: Value) -> Vec<(&'static str, String)> {
    let c = json!({ "manifest_version": 3, "ident": { "author_did": ALICE, "name": "card" }, "kind": "html", "host_api": 3, "title": "Card", "accepts": ["buckyos.record"],
                    "allow_no_source": true, "events": events, "entry": { "html": "view.html", "js": "behavior.js" } });
    vec![("component.json", c.to_string()), ("view.html", "<button id=d>Done</button>".into()), ("behavior.js", "d.onclick=()=>aiws.emit('completed',{title:'x'});aiws.ready()".into())]
}

fn surface_ops() -> Value {
    json!([
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "s" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "s", "content_folder_id": "sf-content" } },
    ])
}

#[test]
fn events_buttons_bubbling_and_migrations() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        workspace(&mut srv, "Events").await;
        let completed = json!({ "completed": { "title": "Done", "params": { "title": { "type": "string" }, "task": { "type": "entity" } } } });
        // events need manifest_version 3, and so does host API 3
        let mut v2 = card_v3(completed.clone());
        v2[0].1 = v2[0].1.replace("\"manifest_version\":3", "\"manifest_version\":2");
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "card-v2", &v2).await })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED", "{r}");
        assert!(diag_text(&r).contains("manifest_version 3"), "{r}");
        let card = srv.rpc("ext.build", json!({ "files": upload(&srv, "card", &card_v3(completed.clone())).await })).await;
        assert_eq!(card["action"], "created", "{card}");
        let card_id = card["def_entity_id"].as_str().unwrap().to_string();
        let payload = srv.rpc("doc.read", json!({ "entity_id": card_id })).await["content"]["payload"].clone();
        assert_eq!((payload["html"]["api_version"].as_u64(), payload["events"]["completed"]["title"].as_str()), (Some(3), Some("Done")), "{payload}");
        let outline = srv.rpc("doc.outline", json!({})).await;
        let e = outline["entities"].as_array().unwrap().iter().find(|e| e["entity_id"] == json!(card_id)).unwrap().clone();
        assert_eq!(e["def_events"], json!(["completed"]));

        // a declarative button raises a declared event only
        let mut kpi = component("kpi", json!({ "manifest_version": 3, "events": { "refresh": {} } }));
        let button = |emit: &str| json!({ "items": [{ "kind": "button", "label": "Refresh", "emit": emit }] }).to_string();
        let files = |c: &Value, emit: &str| vec![("component.json", c.to_string()), ("declarative.json", button(emit)), ("AGENT.md", "#".to_string())];
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi-b1", &files(&kpi, "reload")).await, "dry_run": true })).await;
        assert!(r["error"]["data"]["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "EVENT_UNDECLARED"), "{r}");
        assert_eq!(srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi-b2", &files(&kpi, "refresh")).await, "dry_run": true })).await["status"], "checked");
        kpi["manifest_version"] = json!(2);
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi-b3", &files(&kpi, "refresh")).await, "dry_run": true })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED");

        // a composite re-raises a child's event with its params; a slot whose controls do not raise it is refused
        let mut comp = composition();
        comp["events"] = json!({ "card_done": { "from": { "slot": "cards", "event": "completed" } } });
        let board = srv.rpc("ext.build", json!({ "files": upload(&srv, "board", &board_files(comp.clone(), json!({ "manifest_version": 3 }))).await })).await;
        assert_eq!(board["action"], "created", "{board}");
        let board_id = board["def_entity_id"].as_str().unwrap().to_string();
        let bp = srv.rpc("doc.read", json!({ "entity_id": board_id })).await["content"]["payload"].clone();
        assert_eq!(bp["events"]["card_done"]["from"], json!({ "slot": "cards", "event": "completed" }));
        assert_eq!(bp["events"]["card_done"]["params"]["title"]["type"], "string", "{bp}");
        let mut bad = comp.clone();
        bad["events"] = json!({ "x": { "from": { "slot": "header", "event": "completed" } } });
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "board-x", &board_files(bad, json!({ "manifest_version": 3 }))).await, "force": true })).await;
        assert!(r["error"]["data"]["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "EVENT_UNDECLARED"), "{r}");
        // grid (P3): columns required
        let mut grid = comp.clone();
        grid["layout"] = json!({ "algorithm": "grid", "columns": 3, "gap": "{{config.gap}}" });
        grid["slots"]["header"]["placement"] = json!("stack");
        assert_eq!(srv.rpc("ext.build", json!({ "files": upload(&srv, "board-g", &board_files(grid.clone(), json!({ "manifest_version": 3 }))).await, "dry_run": true, "force": true })).await["status"], "checked");
        grid["layout"] = json!({ "algorithm": "grid" });
        assert_eq!(srv.rpc("ext.build", json!({ "files": upload(&srv, "board-g2", &board_files(grid, json!({ "manifest_version": 3 }))).await, "dry_run": true, "force": true })).await["error"]["code"], "BUILD_FAILED");

        // instances: a board with a card that binds the event, both written per property
        srv.commit(surface_ops()).await;
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "board-1", "type_id": "buckyos.container", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": "auto", "h": "auto" },
              "payload": { "kind": "composite", "def_ref": { "entity_id": board_id }, "config": { "title": "Sprint" } } },
            { "op": "entity.create", "entity_id": "card-1", "type_id": "buckyos.cell", "parent_id": "board-1", "order_key": "a", "placement": { "x": "auto", "y": "auto", "w": 240, "h": 120 },
              "payload": { "view": { "type": "html" }, "def_ref": { "entity_id": card_id }, "slot": "cards" } },
            { "op": "entity.set_props", "entity_id": "card-1", "key": "on", "props": { "completed": { "kind": "notify", "text": "{{event.title}} done" } } },
            { "op": "entity.set_props", "entity_id": "board-1", "key": "on", "props": { "card_done": { "kind": "notify", "text": "a card" } } },
        ])).await;
        let card_on = srv.rpc("doc.read", json!({ "entity_id": "card-1" })).await;
        assert_eq!(card_on["content"]["payload"]["on"]["completed"]["kind"], "notify", "{card_on}");
        // the card stops carrying `title`: the instance reading {{event.title}} is named, and so is the board building on it
        let cbase = |r: &Value| json!({ "def_rev": r["def_rev"], "dir_rev": r["dir_rev"] });
        let narrowed = json!({ "completed": { "title": "Done", "params": { "task": { "type": "entity" } } } });
        let r = srv.rpc("ext.build", json!({ "files": upload(&srv, "card-n", &card_v3(narrowed)).await, "expect": cbase(&card) })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED", "{r}");
        assert!(r["error"]["data"]["diagnostics"].as_array().unwrap().iter().any(|d| d["instances"][0]["entity_id"] == "card-1"), "{r}");
        // the board moves its cards to a new slot: without a migration its instance strands them; with one they move
        let mut moved = comp.clone();
        moved["slots"] = json!({ "header": { "max": 1, "accepts": ["richtext"], "placement": "top" }, "items": { "accepts": ["cell:did:bns:alice#card"] } });
        moved["events"] = json!({ "card_done": { "from": { "slot": "items", "event": "completed" } } });
        let bfiles = upload(&srv, "board-m", &board_files(moved, json!({ "manifest_version": 3 }))).await;
        let r = srv.rpc("ext.build", json!({ "files": bfiles, "expect": cbase(&board) })).await;
        assert!(diag_text(&r).contains("card-1") && diag_text(&r).contains("slot cards"), "{r}");
        let migration = json!({ "slots": { "rename": { "cards": "items" } }, "config": { "rename": { "title": "heading" }, "fill": { "gap": 4 } } });
        let dry = srv.rpc("ext.build", json!({ "files": bfiles, "expect": cbase(&board), "migration": migration, "dry_run": true })).await;
        let plan = dry["plan"].as_array().unwrap();
        assert_eq!(plan.len(), 1, "{dry}");
        assert_eq!(plan[0]["instance"], "board-1");
        let ok = srv.rpc("ext.build", json!({ "files": bfiles, "expect": cbase(&board), "migration": migration })).await;
        assert_eq!((ok["action"].as_str(), ok["migrated"].clone()), (Some("rebuilt"), json!(["board-1"])), "{ok}");
        let b1 = srv.rpc("doc.read", json!({ "entity_id": "board-1" })).await["content"]["payload"].clone();
        assert_eq!(b1["config"], json!({ "heading": "Sprint", "gap": 4 }), "{b1}");
        assert_eq!(b1["on"]["card_done"]["kind"], "notify", "an instance keeps its own handlers");
        assert_eq!(srv.rpc("doc.read", json!({ "entity_id": "card-1" })).await["content"]["payload"]["slot"], "items");
        let r = srv.rpc("doc.read", json!({ "entity_id": "board-1" })).await;
        assert!(r["content"]["key_revs"]["p:config.heading"].as_u64().is_some(), "{r}");
    })
}

#[test]
fn upgrades_from_the_library() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        let ws1 = workspace(&mut srv, "Dev").await;
        let c = component("kpi", json!({}));
        let b = srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi", &kpi_files(&c, "Total")).await })).await;
        let dev_def = b["def_entity_id"].as_str().unwrap().to_string();
        let install = |v: &str| json!({ "from": { "workspace_id": ws1, "entity_ids": [dev_def] }, "package": { "name": "acme-kpi", "version": v } });
        assert_eq!(srv.rpc("ext.install", install("1.0.0")).await["action"], "installed");

        // in use elsewhere: an instance with its own override and binding
        let _ws2 = workspace(&mut srv, "Use").await;
        let im = srv.rpc("ext.import", json!({ "items": [{ "pkg_id": "acme-kpi", "ident": "did:bns:alice#kpi" }] })).await;
        let def = im["controls"][0]["def_entity_id"].as_str().unwrap().to_string();
        srv.commit(surface_ops()).await;
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "sales", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "s", "payload": { "fields": [{ "field_id": "amount", "name": "amount", "type": "number" }] } },
            { "op": "entity.create", "entity_id": "cell-1", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 240, "h": 120 },
              "payload": { "view": { "type": "declarative" }, "def_ref": { "entity_id": def }, "config": { "label": "Mine" }, "bindings": { "sales": { "entity_id": "sales" } } } },
        ])).await;
        let item = json!([{ "def_entity_id": def }]);
        assert_eq!(srv.rpc("ext.upgrade", json!({ "items": item })).await["controls"][0]["action"], "unchanged");

        // 2.0.0 renames the binding: refused without a migration, planned and applied with one
        srv.ws = ws1.clone();
        let mut c2 = component("kpi", json!({}));
        c2["bindings"] = json!({ "orders": { "accepts": ["buckyos.table-source"] } });
        let base = json!({ "def_rev": b["def_rev"], "dir_rev": b["dir_rev"] });
        assert_eq!(srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi2", &kpi_files(&c2, "Orders")).await, "expect": base })).await["action"], "rebuilt");
        assert_eq!(srv.rpc("ext.install", install("2.0.0")).await["action"], "replaced");
        srv.ws = _ws2.clone();
        let r = srv.rpc("ext.upgrade", json!({ "items": item })).await;
        assert_eq!(r["error"]["code"], "BUILD_FAILED", "{r}");
        assert!(diag_text(&r).contains("binds sales"), "{r}");
        let migration = json!({ "did:bns:alice#kpi": { "bindings": { "rename": { "sales": "orders" } } } });
        let dry = srv.rpc("ext.upgrade", json!({ "items": item, "migration": migration, "dry_run": true })).await;
        assert_eq!(dry["plan"][0]["ops"][0]["props"], json!({ "sales": null, "orders": { "entity_id": "sales" } }), "{dry}");
        let up = srv.rpc("ext.upgrade", json!({ "items": item, "migration": migration })).await;
        let ctl = &up["controls"][0];
        assert_eq!((ctl["action"].as_str(), ctl["from"].as_str(), ctl["to"].as_str()), (Some("upgraded"), Some("1.0.0"), Some("2.0.0")), "{up}");
        let cell = srv.rpc("doc.read", json!({ "entity_id": "cell-1" })).await["content"]["payload"].clone();
        assert_eq!((cell["config"]["label"].as_str(), cell["bindings"]["orders"]["entity_id"].as_str()), (Some("Mine"), Some("sales")), "{cell}");
        let p = srv.rpc("doc.read", json!({ "entity_id": def })).await["content"]["payload"].clone();
        assert_eq!((p["provenance"]["version"].as_str(), p["declarative"]["items"][0]["label"].as_str()), (Some("2.0.0"), Some("Orders")));
        assert_eq!(srv.rpc("ext.upgrade", json!({ "items": item })).await["controls"][0]["action"], "unchanged");

        // a local build after the import: the next upgrade lists what would be replaced, force replaces it
        let pulled = srv.rpc("ext.export", json!({ "entity_ids": [def], "purpose": "pull" })).await;
        let lbase = json!({ "def_rev": pulled["controls"][0]["def_rev"], "dir_rev": pulled["controls"][0]["dir_rev"] });
        let mut local = kpi_files(&c2, "Orders");
        local.push(("notes.md", "mine".into()));
        assert_eq!(srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi-local", &local).await, "expect": lbase })).await["action"], "rebuilt");
        srv.ws = ws1.clone();
        srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi3", &kpi_files(&c2, "Orders 3")).await, "force": true })).await;
        assert_eq!(srv.rpc("ext.install", install("3.0.0")).await["action"], "replaced");
        srv.ws = _ws2.clone();
        let r = srv.rpc("ext.upgrade", json!({ "items": item })).await;
        assert_eq!((r["error"]["code"].as_str(), r["error"]["sub_code"].as_str()), (Some("INVALID_OPERATION"), Some("EXT_LOCAL_CHANGES")), "{r}");
        assert_eq!(r["error"]["data"]["files"]["removed"], json!(["notes.md"]), "{r}");
        assert_eq!(srv.rpc("ext.upgrade", json!({ "items": item, "force": true })).await["controls"][0]["action"], "upgraded");
        // a plain import of an identity held from another version answers the library's version
        let _ws3 = workspace(&mut srv, "Third").await;
        srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi-own", &kpi_files(&c2, "Own")).await })).await;
        let r = srv.rpc("ext.import", json!({ "items": [{ "pkg_id": "acme-kpi", "ident": "did:bns:alice#kpi" }] })).await;
        assert_eq!((r["error"]["sub_code"].as_str(), r["error"]["data"]["library_version"].as_str()), (Some("EXT_IDENT_EXISTS"), Some("3.0.0")), "{r}");
    })
}

#[test]
fn workspace_templates_from_snapshot_to_new_workspaces() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        let ws1 = workspace(&mut srv, "Team Tasks").await;
        let c = component("kpi", json!({}));
        let kpi = srv.rpc("ext.build", json!({ "files": upload(&srv, "kpi", &kpi_files(&c, "Total")).await })).await;
        let def = kpi["def_entity_id"].as_str().unwrap().to_string();
        srv.commit(surface_ops()).await;
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "tasks", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "t",
              "payload": { "fields": [{ "field_id": "title", "name": "title", "type": "text" }] } },
            { "op": "table.insert_records", "source_id": "tasks", "records": [{ "record_id": "r1", "values": { "title": "first" } }] },
            { "op": "entity.create", "entity_id": "notes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "u",
              "payload": { "fields": [{ "field_id": "text", "name": "text", "type": "text" }] } },
            { "op": "table.insert_records", "source_id": "notes", "records": [{ "record_id": "n1", "values": { "text": "Welcome, {{params.team_name}}" } }] },
            { "op": "entity.create", "entity_id": "kpi-1", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 240, "h": 120 },
              "payload": { "view": { "type": "declarative" }, "def_ref": { "entity_id": def }, "title": "{{params.team_name}} board", "config": { "label": "{{params.team_name}} tasks" }, "bindings": { "sales": { "entity_id": "tasks" } } } },
            { "op": "entity.create", "entity_id": "notes-1", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "b", "placement": { "x": 0, "y": 200, "w": 240, "h": 120 },
              "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "notes" } } },
            // not part of the template: another Surface's content
            { "op": "entity.create", "entity_id": "other", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "v", "payload": { "fields": [{ "field_id": "x", "name": "x", "type": "text" }] } },
        ])).await;
        let spec = json!({ "id": "team-board", "title": "Team board", "params": { "team_name": { "type": "string", "title": "Team", "default": "Team" } }, "reset": { "tables": ["tasks"] } });
        let ex = srv.rpc("ext.export_template", json!({ "surfaces": ["sf"], "template": spec })).await;
        assert_eq!(ex["template"]["requires"]["controls"], json!(["did:bns:alice#kpi"]), "{ex}");
        assert_eq!(ex["controls"], json!([def]));
        let paths: Vec<&str> = ex["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert!(paths.contains(&"templates/team-board/snapshot.json") && paths.iter().any(|p| p.starts_with("templates/team-board/objects/")), "{paths:?}");
        let bad = srv.rpc("ext.export_template", json!({ "surfaces": ["sf"], "template": { "id": "x", "title": "X", "reset": { "tables": ["other"] } } })).await;
        assert_eq!(bad["error"]["code"], "INVALID_OPERATION", "{bad}");

        // into the library: no Workspace is created; the control it needs comes along
        let count = |v: &Value| v["workspaces"].as_array().map_or(0, Vec::len);
        let before = count(&srv.rpc("ws.list", json!({})).await);
        let inst = srv.rpc("ext.install", json!({ "from": { "workspace_id": ws1, "templates": [{ "surfaces": ["sf"], "template": spec }] }, "package": { "name": "team-kit", "version": "1.0.0" } })).await;
        assert_eq!((inst["action"].as_str(), inst["templates"].clone()), (Some("installed"), json!(["team-board"])), "{inst}");
        assert_eq!(count(&srv.rpc("ws.list", json!({})).await), before, "installing creates no Workspace");
        let lib = srv.rpc("ext.list_library", json!({})).await;
        let pkg = lib["packages"].as_array().unwrap().iter().find(|p| p["pkg_id"] == "team-kit").unwrap().clone();
        assert_eq!((pkg["templates"][0]["id"].as_str(), pkg["controls"][0]["ident"].as_str(), pkg["manifest_version"].as_i64()), (Some("team-board"), Some("did:bns:alice#kpi"), Some(3)), "{pkg}");

        // two creations, two independent Workspaces; parameters replaced, the reset table empty, the instance bound
        let a = srv.rpc("ext.create_from_template", json!({ "pkg_id": "team-kit", "template_id": "team-board", "params": { "team_name": "Red" } })).await;
        let b = srv.rpc("ext.create_from_template", json!({ "pkg_id": "team-kit", "template_id": "team-board", "title": "Blue team" })).await;
        let (wa, wb) = (a["workspace_id"].as_str().unwrap_or_else(|| panic!("{a}")).to_string(), b["workspace_id"].as_str().unwrap_or_else(|| panic!("{b}")).to_string());
        assert_ne!(wa, wb, "{a} {b}");
        assert_eq!(count(&srv.rpc("ws.list", json!({})).await), before + 2);
        srv.ws = wa.clone();
        let cell = srv.rpc("doc.read", json!({ "entity_id": "kpi-1" })).await["content"]["payload"].clone();
        assert_eq!((cell["config"]["label"].as_str(), cell["title"].as_str()), (Some("Red tasks"), Some("Red board")), "{cell}");
        assert_eq!(srv.rpc("doc.read", json!({ "entity_id": def })).await["content"]["payload"]["ident"]["name"], "kpi", "the definition came with the snapshot");
        assert_eq!(srv.rpc("doc.query", json!({ "source_id": "tasks" })).await["rows"], json!([]), "reset");
        assert_eq!(srv.rpc("doc.query", json!({ "source_id": "notes" })).await["rows"][0]["values"]["text"], "Welcome, Red");
        assert_eq!(srv.rpc("doc.read", json!({ "entity_id": "other" })).await["error"]["code"], "NOT_FOUND", "only what the Surface needs");
        srv.commit(json!([{ "op": "entity.set_props", "entity_id": "kpi-1", "key": "config", "props": { "label": "changed" } }])).await;
        srv.ws = wb.clone();
        assert_eq!(srv.rpc("ws.get_info", json!({})).await["title"], "Blue team");
        assert_eq!(srv.rpc("doc.read", json!({ "entity_id": "kpi-1" })).await["content"]["payload"]["config"]["label"], "Team tasks", "defaults, and independent of the first");

        // ext.pack carries the template too; the library checks it like a control
        srv.ws = ws1.clone();
        let packed = srv.rpc("ext.pack", json!({ "entity_ids": [def], "templates": [{ "surfaces": ["sf"], "template": spec }], "package": { "name": "team-kit-pikg", "version": "1.0.0" } })).await;
        assert!(packed["key"].as_str().is_some(), "{packed}");
        let r = srv.rpc("ext.install", json!({ "pikg": { "bucket_id": packed["bucket_id"], "key": packed["key"] } })).await;
        assert_eq!(r["templates"], json!(["team-board"]), "{r}");
    })
}
