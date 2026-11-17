//! The print package (打印 TODO §3.2, §3.4, T5, T7–T9): what one print of a canvas carries — fixed at one head, the
//! caller's readable shared rows of the canvas (or of every canvas a presentation path prints), without speaker notes,
//! comments or what the caller may not read or export — and that the WASM Replica builds a Workspace from those rows.

mod common;

use aiworkspace_core::access::Access;
use aiworkspace_core::materialize::ObjectSource;
use aiworkspace_core::replica::{load_rows, Replica};
use aiworkspace_store::print::{PrintContents, PrintRequest};
use aiworkspace_store::workspace::Workspace;
use common::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Read;

const PRESENTATION: &str = include_str!("../../fixtures/presentation/commits.json");

fn png(tag: &str) -> Vec<u8> {
    [&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..], tag.as_bytes()].concat()
}

/// A Workspace of alice's with the presentation fixture (two free canvases, the five-step mixed path, a guide).
fn courseware(env: &Env) -> String {
    let id = env.svc.create_workspace(&alice(), "课件", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    let fx: Value = serde_json::from_str(PRESENTATION).unwrap();
    for c in fx["commits"].as_array().unwrap() {
        ok(&mut ws, &alice(), c["operations"].clone());
    }
    id
}

/// A flow canvas `flow1` ("文档") with a rich text, a table (one deleted field), a sticky note, a comment and the Block
/// showing it, a secret shape, a picture, a video with its poster and an SVG drawing.
fn document(ws: &mut Workspace) {
    let para = |id: &str, text: &str| json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] });
    let cell = |id: &str, order: &str, payload: Value| json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.cell", "parent_id": "flow1", "order_key": order, "payload": payload });
    let data = |id: &str, ty: &str, order: &str, payload: Value| json!({ "op": "entity.create", "entity_id": id, "type_id": ty, "parent_id": "f-content", "order_key": order, "payload": payload });
    let picture = ws.stage_asset(&alice(), &png("picture")).unwrap()["object_id"].as_str().unwrap().to_string();
    let video = ws.stage_asset(&alice(), b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2mp41 frames").unwrap()["object_id"].as_str().unwrap().to_string();
    let poster = ws.stage_asset(&alice(), &png("poster")).unwrap()["object_id"].as_str().unwrap().to_string();
    let drawing = ws.stage_asset(&alice(), b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"/>").unwrap()["object_id"].as_str().unwrap().to_string();
    ok(ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "f-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "z",
          "payload": { "kind": "folder", "title": "文档", "system": "surface_content", "surface_id": "flow1" } },
        { "op": "entity.create", "entity_id": "flow1", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "z",
          "payload": { "kind": "surface", "layout": { "mode": "flow" }, "title": "文档", "content_folder_id": "f-content" } },
        data("doc1", "buckyos.richtext", "a", json!({ "content": { "type": "doc", "content": [para("p1", "第一段正文")] } })),
        data("t1", "buckyos.table-source", "b", json!({ "title_field_id": "name", "fields": [
            { "field_id": "name", "name": "名称", "type": "text" }, { "field_id": "gone", "name": "旧列", "type": "text" }] })),
        { "op": "table.insert_records", "source_id": "t1", "records": [{ "record_id": "r1", "values": { "name": "甲行", "gone": "旧列的值" } }] },
        data("sticky1", "buckyos.annotation", "c", json!({ "kind": "note", "body": "便签的话" })),
        data("cmt1", "buckyos.annotation", "d", json!({ "target": { "entity_id": "doc1" }, "kind": "note", "body": "批注的话" })),
        data("pic", "buckyos.asset-ref", "e", json!({ "object_id": picture })),
        data("clip", "buckyos.asset-ref", "f", json!({ "object_id": video, "poster": poster })),
        data("drawing", "buckyos.asset-ref", "g", json!({ "object_id": drawing })),
        cell("c-doc1", "a", json!({ "view": { "type": "richtext" }, "source_ref": { "entity_id": "doc1" } })),
        cell("c-t1", "b", json!({ "view": { "type": "table" }, "source_ref": { "entity_id": "t1" } })),
        cell("c-sticky1", "c", json!({ "view": { "type": "note", "version": 1 }, "source_ref": { "entity_id": "sticky1" } })),
        cell("c-cmt1", "d", json!({ "view": { "type": "note", "version": 1 }, "source_ref": { "entity_id": "cmt1" } })),
        cell("c-secret", "e", json!({ "view": { "type": "shape", "version": 1 }, "title": "机密标题" })),
        cell("c-pic", "f", json!({ "view": { "type": "asset", "version": 1 }, "source_ref": { "entity_id": "pic" } })),
        cell("c-clip", "g", json!({ "view": { "type": "asset", "version": 1 }, "source_ref": { "entity_id": "clip" } })),
        cell("c-drawing", "h", json!({ "view": { "type": "asset", "version": 1 }, "source_ref": { "entity_id": "drawing" } })),
    ]));
    let f = ws.read(&alice(), "t1", Some(&json!({ "kind": "table_field", "field_id": "gone" }))).unwrap();
    ok(ws, &alice(), json!([{ "op": "table.delete_field", "source_id": "t1", "field_id": "gone", "expect": { "rev": f["content"]["def_rev"] } }]));
}

/// An A4 portrait PDF Frame `pf` ("公司信纸") on `s-work`, its footer showing the page number.
fn pdf_frame(ws: &mut Workspace) {
    ok(ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "pf", "type_id": "buckyos.container", "parent_id": "s-work", "order_key": "m", "placement": { "x": 40, "y": 40, "w": 595, "h": 842 },
          "payload": { "kind": "pdf-frame", "title": "公司信纸", "config": { "page": "A", "orientation": "portrait", "header": 56, "footer": 56, "margin": 56 } } },
        { "op": "entity.create", "entity_id": "pf-h", "type_id": "buckyos.container", "parent_id": "pf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 595, "h": 56 }, "payload": { "kind": "group", "slot": "header" } },
        { "op": "entity.create", "entity_id": "pf-f", "type_id": "buckyos.container", "parent_id": "pf", "order_key": "b", "placement": { "x": 0, "y": 786, "w": 595, "h": 56 }, "payload": { "kind": "group", "slot": "footer" } },
        { "op": "entity.create", "entity_id": "pn", "type_id": "buckyos.cell", "parent_id": "pf-f", "order_key": "a", "placement": { "x": 248, "y": 16, "w": 100, "h": 24 },
          "payload": { "view": { "type": "label" }, "title": "{page} / {pages}", "config": { "align": "center", "font_size": 10 } } },
    ]));
}

fn request(surface: &str, path: Option<&str>) -> PrintRequest {
    PrintRequest::parse(&json!({ "surface_id": surface, "path_id": path, "paper": { "name": "A4", "width_mm": 210, "height_mm": 297 },
                                 "date": "2026-11-16", "locale": "zh-CN" }))
    .unwrap()
}

fn ids(contents: &PrintContents, table: &str, key: &str) -> BTreeSet<String> {
    contents.package["rows"][table].as_array().unwrap().iter().map(|r| r[key].as_str().unwrap().to_string()).collect()
}

fn codes(contents: &PrintContents) -> Vec<(String, String)> {
    contents.package["issues"].as_array().unwrap().iter().map(|i| (i["code"].as_str().unwrap().to_string(), i["step_id"].as_str().unwrap_or("").to_string())).collect()
}

/// The rows build a WASM Replica (the print renderer's starting point): its outline and reads.
fn replica(contents: &PrintContents) -> Replica {
    let p = &contents.package;
    let store = load_rows(&p["rows"]).unwrap();
    Replica::new(p["principal"].as_str().unwrap(), p["workspace_id"].as_str().unwrap(), p["epoch"].as_str().unwrap(), store, p["head_seq"].as_u64().unwrap(), 7)
}

fn outline_ids(r: &Replica) -> BTreeSet<String> {
    aiworkspace_core::read::outline(&r.working, &Access::full(&r.principal)).unwrap().iter().map(|e| e["entity_id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_free_canvas_prints_its_path_across_canvases_without_notes() {
    let env = env();
    let id = courseware(&env);
    let handle = env.svc.workspace(&id).unwrap();
    let ws = handle.lock().unwrap();
    let c = ws.print_contents(&alice(), &request("s-launch", Some("path-intro"))).unwrap();
    let job = &c.package["job"];
    assert_eq!((job["mode"].as_str(), job["title"].as_str(), job["path_id"].as_str()), (Some("free"), Some("发布会"), Some("path-intro")));
    assert_eq!(c.package["format"], "buckyos.aiws-print");
    assert_eq!(c.package["head_seq"], json!(ws.head_seq));
    assert_eq!(codes(&c), Vec::<(String, String)>::new(), "{}", c.package["issues"]);
    let entities = ids(&c, "entities", "entity_id");
    // both canvases of the mixed path, the path, its Viewports, the trees above them; never the guide
    for e in ["s-launch", "s-work", "frame1", "frame2", "frame3", "shape-thanks", "shape-extra", "viewport1", "viewport2", "path-intro", "root", "surfaces", "shows", "data", "note-problem", "d-note-problem"] {
        assert!(entities.contains(e), "{e} missing: {entities:?}");
    }
    assert!(!entities.contains("path-guide"));
    // speaker notes stay behind, captions travel
    let text = c.package.to_string();
    for notes in ["开场：介绍我们是谁", "讲痛点", "总结三点", "演示参数区"] {
        assert!(!text.contains(notes), "{notes} travelled");
    }
    assert!(text.contains("欢迎：这是演示的第一页。"));
    let r = replica(&c);
    let outline = outline_ids(&r);
    assert!(["frame1", "frame3", "viewport1", "path-intro", "s-work"].iter().all(|e| outline.contains(*e)), "{outline:?}");
    let frame = aiworkspace_core::read::read(&r.working, &Access::full("alice"), "frame1", None).unwrap();
    assert!(frame["content"]["payload"]["presentation"].get("notes").is_none(), "{frame}");
    assert_eq!(frame["content"]["payload"]["presentation"]["caption"], "欢迎：这是演示的第一页。");
    let path = aiworkspace_core::read::read(&r.working, &Access::full("alice"), "path-intro", None).unwrap();
    assert_eq!(path["content"]["payload"]["steps"].as_array().unwrap().len(), 5);
}

#[test]
fn a_free_canvas_needs_a_presentation_path_through_it() {
    let env = env();
    let id = courseware(&env);
    let handle = env.svc.workspace(&id).unwrap();
    let ws = handle.lock().unwrap();
    let sub = |r: Result<PrintContents, aiworkspace_core::WsError>| {
        let e = r.err().expect("refused");
        (e.code.as_str(), e.sub.unwrap_or(""), e.data.as_ref().map(|d| d["reason"].as_str().unwrap_or("").to_string()).unwrap_or_default())
    };
    assert_eq!(sub(ws.print_contents(&alice(), &request("s-launch", None))), ("INVALID_OPERATION", "PRINT_PATH_REQUIRED", String::new()));
    assert_eq!(sub(ws.print_contents(&alice(), &request("s-launch", Some("path-guide")))), ("INVALID_OPERATION", "PRINT_PATH_UNUSABLE", "guide".into()));
    assert_eq!(sub(ws.print_contents(&alice(), &request("s-launch", Some("viewport1")))), ("INVALID_OPERATION", "PRINT_PATH_UNUSABLE", "not_a_path".into()));
    assert_eq!(sub(ws.print_contents(&alice(), &request("s-launch", Some("nope")))).0, "NOT_FOUND");
    drop(ws);
    // a path that never stops on this canvas does not print it
    let mut ws = handle.lock().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "path-work", "type_id": "buckyos.show-path", "parent_id": "shows", "order_key": "e",
        "payload": { "purpose": "presentation", "stage": { "w": 1920, "h": 1080 }, "steps": [{ "id": "w1", "target": { "kind": "frame", "entity_id": "frame3" } }] } }]));
    assert_eq!(sub(ws.print_contents(&alice(), &request("s-launch", Some("path-work")))), ("INVALID_OPERATION", "PRINT_PATH_UNUSABLE", "not_on_surface".into()));
    // a disabled step and a dangling one are skipped and reported
    let path = ws.read(&alice(), "path-intro", None).unwrap();
    let mut steps = path["content"]["payload"]["steps"].clone();
    steps[1]["enabled"] = json!(false);
    steps.as_array_mut().unwrap().push(json!({ "id": "s6", "target": { "kind": "frame", "entity_id": "frame2" } }));
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "path-intro", "keys": [{ "key": "steps", "value": steps, "expect": { "rev": path["content"]["key_revs"]["steps"] } }] }]));
    let frame2 = ws.read(&alice(), "frame2", None).unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "frame2", "expect": { "rev": frame2["life_rev"] } }]));
    let c = ws.print_contents(&alice(), &request("s-launch", Some("path-intro"))).unwrap();
    let issues = c.package["issues"].as_array().unwrap();
    assert_eq!(issues.iter().map(|i| (i["step_id"].as_str().unwrap(), i["detail"].as_str().unwrap())).collect::<Vec<_>>(), vec![("s2", "disabled"), ("s6", "missing")]);
}

#[test]
fn printing_needs_export_and_leaves_out_what_the_caller_may_not_read_or_export() {
    let env = env();
    let id = courseware(&env);
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    ws.grant(&alice(), "bob", Some("s-launch"), &caps(&["read"])).unwrap();
    ws.grant(&alice(), "bob", Some("shows"), &caps(&["read"])).unwrap();
    let e = ws.print_contents(&bob(), &request("s-launch", Some("path-intro"))).err().unwrap();
    assert_eq!(e.code.as_str(), "PERMISSION_DENIED");
    ws.grant(&alice(), "bob", Some("s-launch"), &caps(&["read", "export"])).unwrap();
    // the sticky notes' data lives in the canvas's content folder: unread, they are counted
    let c = ws.print_contents(&bob(), &request("s-launch", Some("path-intro"))).unwrap();
    let unreadable = c.package["issues"].as_array().unwrap().iter().find(|i| i["code"] == "content_unreadable").unwrap();
    assert_eq!((unreadable["surface_id"].as_str(), unreadable["detail"].as_str()), (Some("s-launch"), Some("2")));
    assert!(!c.package.to_string().contains("数据散落在各处"));
    ws.grant(&alice(), "bob", Some("s-launch-content"), &caps(&["read"])).unwrap();
    // the other canvas of the path: unreadable, then readable but not exportable — its step is left out either way
    let c = ws.print_contents(&bob(), &request("s-launch", Some("path-intro"))).unwrap();
    assert_eq!(codes(&c), vec![("surface_unreadable".to_string(), "s5".to_string())]);
    assert!(c.package["issues"][0].get("surface_id").is_none(), "an unreadable canvas is not named");
    ws.grant(&alice(), "bob", Some("s-work"), &caps(&["read"])).unwrap();
    let c = ws.print_contents(&bob(), &request("s-launch", Some("path-intro"))).unwrap();
    assert_eq!(codes(&c), vec![("surface_not_exportable".to_string(), "s5".to_string())]);
    let entities = ids(&c, "entities", "entity_id");
    assert!(entities.contains("frame1") && !entities.contains("frame3") && !entities.contains("s-work") && !entities.contains("shape-thanks"));
    // above what bob may read: the tree's nodes as ids and kinds only
    let rows = c.package["rows"]["entities"].as_array().unwrap();
    let root = rows.iter().find(|r| r["entity_id"] == "root").unwrap();
    assert!(root["name"].is_null() && root["payload_json"] == json!("{\"kind\":\"root\"}"), "{root}");
    // a Block bob may not read is not in the package, only counted
    ws.acl_set(&alice(), &json!([{ "subject": "bob", "resource": "shape-cover", "allow": [], "deny": ["read"] }])).unwrap();
    let c = ws.print_contents(&bob(), &request("s-launch", Some("path-intro"))).unwrap();
    assert!(!ids(&c, "entities", "entity_id").contains("shape-cover"));
    assert!(!c.package.to_string().contains("BuckyOS AI Workspace\""), "the unreadable shape's title travelled");
    let unreadable = c.package["issues"].as_array().unwrap().iter().find(|i| i["code"] == "content_unreadable").unwrap();
    assert_eq!((unreadable["surface_id"].as_str(), unreadable["detail"].as_str()), (Some("s-launch"), Some("1")));
    let r = replica(&c);
    assert!(outline_ids(&r).contains("frame1"));
}

#[test]
fn a_flow_canvas_carries_its_content_without_comments_and_with_its_assets() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "文档库", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    document(&mut ws);
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_props", "entity_id": "root", "key": "config", "props": { "fonts": { "serif": " Noto Serif CJK SC ", "sans": "", "mono": 3 } } }]));
    assert_eq!(ws.print_contents(&alice(), &request("flow1", Some("path-intro"))).err().unwrap().code.as_str(), "INVALID_OPERATION");
    ws.grant(&alice(), "bob", Some("flow1"), &caps(&["read", "export"])).unwrap();
    ws.grant(&alice(), "bob", Some("f-content"), &caps(&["read"])).unwrap();
    ws.acl_set(&alice(), &json!([{ "subject": "bob", "resource": "c-secret", "allow": [], "deny": ["read"] }])).unwrap();
    let c = ws.print_contents(&bob(), &request("flow1", None)).unwrap();
    assert_eq!(c.package["job"]["mode"], "flow");
    assert_eq!(c.package["job"]["fonts"], json!({ "serif": "Noto Serif CJK SC" }));
    let entities = ids(&c, "entities", "entity_id");
    for e in ["flow1", "c-doc1", "doc1", "c-t1", "t1", "c-sticky1", "sticky1", "pic", "c-pic", "clip", "c-clip", "drawing", "c-drawing"] {
        assert!(entities.contains(e), "{e} missing: {entities:?}");
    }
    // comments and the Blocks showing them, and what bob may not read, stay behind
    for e in ["cmt1", "c-cmt1", "c-secret"] {
        assert!(!entities.contains(e), "{e} travelled");
    }
    let text = c.package.to_string();
    assert!(!text.contains("批注的话") && !text.contains("机密标题") && !text.contains("旧列的值"), "{text}");
    assert!(text.contains("便签的话") && text.contains("甲行"));
    assert_eq!(ids(&c, "table_fields", "field_id"), BTreeSet::from(["name".to_string()]));
    assert_eq!(ids(&c, "richtext_states", "entity_id"), BTreeSet::from(["doc1".to_string()]));
    // the picture and the video's poster travel; the video itself does not
    let issues = c.package["issues"].as_array().unwrap();
    let omitted = issues.iter().find(|i| i["code"] == "asset_omitted").unwrap();
    assert_eq!((omitted["entity_id"].as_str(), omitted["detail"].as_str()), (Some("clip"), Some("video/mp4")));
    assert_eq!(c.assets.len(), 3, "{:?}", c.assets);
    assert_eq!(ids(&c, "assets", "object_id").len(), 4);
    // each file carries the extension of its type: Chromium types file:// by it (an SVG is never sniffed)
    let mut exts: Vec<&str> = c.assets.iter().map(|(_, p)| p.rsplit('.').next().unwrap()).collect();
    exts.sort();
    assert_eq!(exts, vec!["png", "png", "svg"]);
    assert!(c.assets.iter().all(|(o, p)| p.starts_with(&format!("assets/{}.", aiworkspace_core::canonical::obj_id_to_filename(o).unwrap()))));
    let r = replica(&c);
    let outline = outline_ids(&r);
    assert!(outline.contains("c-doc1") && outline.contains("t1") && !outline.contains("c-cmt1"));
    let doc = aiworkspace_core::read::read(&r.working, &Access::full("bob"), "doc1", None).unwrap();
    assert!(doc.to_string().contains("第一段正文"), "{doc}");
    let rec = aiworkspace_core::read::read(&r.working, &Access::full("bob"), "t1", Some(&json!({ "kind": "table_record", "record_id": "r1" }))).unwrap();
    assert!(rec.to_string().contains("甲行"), "{rec}");
}

#[test]
fn the_package_is_a_zip_with_the_renderer_the_data_and_the_assets() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "文档库", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    document(&mut ws);
    let head = ws.head_seq;
    let renderer = vec![("index.html".to_string(), b"<!doctype html><script src=\"data.js\"></script>".to_vec()), ("print.js".to_string(), b"void 0".to_vec())];
    let built = ws.print_package(&alice(), &request("flow1", None), &renderer).unwrap();
    assert_eq!(ws.head_seq, head, "printing writes nothing");
    // the same head and job: the same bytes (`media.page` keys its cache on the content)
    let again = ws.print_package(&alice(), &request("flow1", None), &renderer).unwrap();
    assert_eq!(ws.objects.get_file(&again.obj_id).unwrap(), ws.objects.get_file(&built.obj_id).unwrap());
    assert_ne!(again.key, built.key);
    assert!(built.key.starts_with(&format!("{id}/print/")) && built.key.ends_with(".zip"));
    assert_eq!(env.bound("alice", "exports", &built.key).as_deref(), Some(built.obj_id.as_str()));
    let bytes = ws.objects.get_file(&built.obj_id).unwrap();
    assert_eq!(bytes.len() as u64, built.size);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: BTreeSet<String> = zip.file_names().map(str::to_string).collect();
    assert!(names.contains("index.html") && names.contains("print.js") && names.contains("data.js"), "{names:?}");
    let mut data = String::new();
    zip.by_name("data.js").unwrap().read_to_string(&mut data).unwrap();
    let json = data.strip_prefix("window.__AIWS_PRINT__ = ").unwrap().trim_end().strip_suffix(';').unwrap();
    let package: Value = serde_json::from_str(json).unwrap();
    assert_eq!(package["job"], built.job);
    let assets = package["assets"].as_object().unwrap();
    assert_eq!(assets.len(), 3);
    assert!(assets.values().any(|p| p.as_str().unwrap().ends_with(".svg")));
    for (obj, path) in assets {
        let mut file = Vec::new();
        zip.by_name(path.as_str().unwrap()).unwrap().read_to_end(&mut file).unwrap();
        assert_eq!(file, ws.objects.get_file(obj).unwrap());
        assert!(names.contains(path.as_str().unwrap()));
    }
    assert!(names.iter().filter(|n| n.starts_with("assets/")).count() == 3);
}

#[test]
fn a_pdf_frame_on_another_canvas_travels_with_its_bands_only() {
    let env = env();
    let id = courseware(&env);
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    document(&mut ws);
    pdf_frame(&mut ws);
    let mut req = request("flow1", None);
    req.frame_id = Some("pf".into());
    let c = ws.print_contents(&alice(), &req).unwrap();
    assert_eq!(c.package["job"]["frame_id"], "pf");
    let entities = ids(&c, "entities", "entity_id");
    for e in ["pf", "pf-h", "pf-f", "pn", "s-work", "flow1"] {
        assert!(entities.contains(e), "{e} missing: {entities:?}");
    }
    assert!(!entities.contains("frame3") && !entities.contains("shape-thanks"), "the frame's canvas does not print");
    let r = replica(&c);
    assert!(outline_ids(&r).contains("pn"));
    // a PDF Frame is decoration: reading it is enough (export is judged on the printed canvas)
    ws.grant(&alice(), "bob", Some("flow1"), &caps(&["read", "export"])).unwrap();
    assert_eq!(ws.print_contents(&bob(), &req).err().unwrap().code.as_str(), "NOT_FOUND");
    ws.grant(&alice(), "bob", Some("pf"), &caps(&["read"])).unwrap();
    let c = ws.print_contents(&bob(), &req).unwrap();
    assert!(ids(&c, "entities", "entity_id").contains("pn"));
    req.frame_id = Some("frame3".into());
    assert_eq!(ws.print_contents(&alice(), &req).err().unwrap().sub, Some("PRINT_FRAME_UNUSABLE"));
}

/// Sample packages for the print renderer (P3): `AIWS_PRINT_SAMPLE_DIR=<dir> [AIWS_PRINT_RENDERER=<desktop dist>/aiws-print]
/// cargo test -p aiworkspace-store --test print sample_packages -- --ignored` writes `flow.zip` (the document canvas),
/// `flow-frame.zip` (the same on A5 with the PDF Frame `pf`) and `free.zip` (the presentation fixture along `path-intro`), each
/// also unpacked beside it. Without a renderer the package carries a placeholder `index.html`.
#[test]
#[ignore]
fn sample_packages() {
    let out = std::path::PathBuf::from(std::env::var("AIWS_PRINT_SAMPLE_DIR").expect("AIWS_PRINT_SAMPLE_DIR"));
    let renderer = match std::env::var("AIWS_PRINT_RENDERER") {
        Ok(dir) => aiworkspace_store::print::renderer_files(Some(std::path::Path::new(&dir))).unwrap(),
        Err(_) => vec![("index.html".to_string(), b"<!doctype html><meta charset=\"utf-8\"><script src=\"data.js\"></script><pre id=\"o\"></pre><script>o.textContent = JSON.stringify(window.__AIWS_PRINT__.job, null, 2)</script>".to_vec())],
    };
    let env = env();
    let id = courseware(&env);
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    document(&mut ws);
    pdf_frame(&mut ws);
    let paper = |name: &str, w: f64, h: f64| json!({ "name": name, "width_mm": w, "height_mm": h });
    let samples = [("flow", "flow1", None, None, paper("A4", 210.0, 297.0)), ("flow-frame", "flow1", None, Some("pf"), paper("A5", 148.0, 210.0)),
                   ("free", "s-launch", Some("path-intro"), None, paper("16:9", 297.0, 167.06))];
    for (name, surface, path, frame, paper) in samples {
        let req = PrintRequest::parse(&json!({ "surface_id": surface, "path_id": path, "frame_id": frame, "paper": paper, "date": "2026-11-16", "locale": "zh-CN" })).unwrap();
        let built = ws.print_package(&alice(), &req, &renderer).unwrap();
        let bytes = ws.objects.get_file(&built.obj_id).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(format!("{name}.zip")), &bytes).unwrap();
        let unpacked = out.join(name);
        let _ = std::fs::remove_dir_all(&unpacked);
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap().extract(&unpacked).unwrap();
        eprintln!("{name}: {} ({} bytes), issues {}", out.join(format!("{name}.zip")).display(), built.size, built.issues);
    }
}
