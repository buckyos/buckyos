//! Extension controls over the real service (notepads/aiworkspace-extension-p0-todo.md M2): `ext.build` from
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
