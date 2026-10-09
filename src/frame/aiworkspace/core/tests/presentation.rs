//! Presentation contract (BuckyOS AI Workspace 第三期规划 §4–§6, M0): Frame presentation properties, Viewports,
//! presentation paths in the `shows` system folder, their references (never blocking), dangling steps, the
//! outline projection and stripping speaker notes from a package.

use aiworkspace_core::access::Access;
use aiworkspace_core::materialize::{materialize, materialize_with, without_notes, ObjectSink};
use aiworkspace_core::model::*;
use aiworkspace_core::plan::selector_string;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("../../fixtures/presentation/commits.json");

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

/// The M0 fixture replayed: commit N gets seq N.
fn fixture() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    let f: Value = serde_json::from_str(FIXTURE).unwrap();
    for c in f["commits"].as_array().unwrap() {
        ws.ok(c["operations"].clone());
    }
    ws
}

fn set(ws: &MemWorkspace, id: &str, key: &str, value: Value) -> Value {
    let rev = ws.store.entities[id].key_rev(key);
    json!([{ "op": "entity.set_keys", "entity_id": id, "keys": [{ "key": key, "value": value, "expect": { "rev": rev } }] }])
}

fn steps(ws: &MemWorkspace, path: &str) -> Vec<Value> {
    ws.store.entities[path].payload["steps"].as_array().unwrap().clone()
}

fn outline_of(ws: &MemWorkspace, id: &str) -> Value {
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    outline.into_iter().find(|e| e["entity_id"] == id).unwrap()
}

#[test]
fn fixture_replays_with_five_mixed_steps_and_a_guide() {
    let ws = fixture();
    let ids: Vec<String> = steps(&ws, "path-intro").iter().map(|s| s["target"]["entity_id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids, ["frame1", "frame2", "viewport1", "viewport2", "frame3"]);
    assert_eq!(ws.store.entities["path-guide"].payload["purpose"], "guide");
    // the shows folder is a system node under data, before the canvas content
    assert_eq!(ws.store.edges[SHOWS_ID].parent_id, DATA_ID);
    assert_eq!(ws.store.entities[SHOWS_ID].payload["system"], "shows");
    // every step is a non-blocking `show_target` edge keyed by its step id
    let edges: Vec<_> = ws.store.refs.iter().filter(|r| r.src_entity_id == "path-intro").collect();
    assert_eq!(edges.len(), 5);
    assert!(edges.iter().all(|r| r.kind == "show_target" && !r.blocks_delete()));
    assert!(edges.iter().any(|r| r.src_selector == selector_string(&json!({ "kind": "step", "id": "s3" })) && r.dst_entity_id == "viewport1"));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "viewport1" && r.kind == "show_surface" && r.dst_entity_id == "s-launch"));
}

#[test]
fn outline_carries_what_a_show_needs_first() {
    let ws = fixture();
    let v = outline_of(&ws, "viewport2");
    assert_eq!(v["viewport"], json!({ "surface_id": "s-launch", "center": { "x": 350, "y": 1450 }, "zoom": 2 }));
    assert_eq!(v["title"], "参数区特写");
    let p = outline_of(&ws, "path-intro");
    assert_eq!(p["show_path"], json!({ "purpose": "presentation", "stage": { "w": 1920, "h": 1080 }, "steps": 5 }));
    assert_eq!(outline_of(&ws, "note-live")["live"], true);
    assert!(outline_of(&ws, "frame1").get("live").is_none());
    // notes are content, never in the outline
    assert!(!outline_of(&ws, "frame1").to_string().contains("开场"));
}

#[test]
fn presentation_keys_are_checked_per_block() {
    let mut ws = fixture();
    // frame only: background, notes, caption
    let r = ws.fail(set(&ws, "shape-cover", "presentation", json!({ "notes": "x" })));
    assert_eq!(code(&r), "INVALID_SCHEMA", "{r}");
    // any Block may be operable on stage
    ws.ok(set(&ws, "shape-cover", "presentation", json!({ "live": true })));
    for bad in [json!({ "background": "red" }), json!({ "live": "yes" }), json!({ "speed": 1 }), json!("x")] {
        let r = ws.fail(set(&ws, "frame1", "presentation", bad.clone()));
        assert_eq!(code(&r), "INVALID_SCHEMA", "{bad}: {r}");
    }
    let r = ws.fail(set(&ws, "frame1", "presentation", json!({ "notes": "长".repeat(6000) })));
    assert_eq!(code(&r), "LIMIT_EXCEEDED", "{r}");
    ws.ok(set(&ws, "frame1", "presentation", json!({ "background": "#123", "notes": "多行\n备注", "caption": "说明" })));
    // null removes the whole key
    ws.ok(set(&ws, "frame1", "presentation", Value::Null));
    assert!(ws.store.entities["frame1"].payload.get("presentation").is_none());
}

#[test]
fn shows_folder_holds_only_presentation_entities() {
    let mut ws = fixture();
    let vp = |id: &str, parent: &str| json!([{ "op": "entity.create", "entity_id": id, "type_id": TYPE_VIEWPORT, "parent_id": parent, "order_key": "x",
        "payload": { "surface_ref": { "entity_id": "s-launch" }, "center": { "x": 0, "y": 0 }, "zoom": 1 } }]);
    let r = ws.fail(vp("v-data", DATA_ID));
    assert_eq!(r["sub_code"], "CHILD_NOT_ALLOWED", "{r}");
    let r = ws.fail(json!([{ "op": "entity.create", "entity_id": "t-x", "type_id": TYPE_TABLE, "parent_id": SHOWS_ID, "order_key": "x", "payload": {} }]));
    assert_eq!(r["sub_code"], "CHILD_NOT_ALLOWED", "{r}");
    ws.ok(vp("v-ok", SHOWS_ID));
    // the system folder itself stays put
    let r = ws.fail(json!([{ "op": "entity.delete", "entity_id": SHOWS_ID, "expect": { "rev": 0 } }]));
    assert_eq!(code(&r), "INVALID_OPERATION", "{r}");
}

#[test]
fn viewport_shape_and_surface() {
    let mut ws = fixture();
    let create = |payload: Value| json!([{ "op": "entity.create", "entity_id": "v-new", "type_id": TYPE_VIEWPORT, "parent_id": SHOWS_ID, "order_key": "x", "payload": payload }]);
    for (payload, want) in [
        (json!({ "surface_ref": { "entity_id": "s-launch" }, "center": { "x": 0, "y": 0 }, "zoom": 5 }), "INVALID_SCHEMA"),
        (json!({ "surface_ref": { "entity_id": "s-launch" }, "center": { "x": 0 }, "zoom": 1 }), "INVALID_SCHEMA"),
        (json!({ "surface_ref": { "entity_id": "s-launch" }, "center": { "x": 0, "y": 0 }, "zoom": 1, "rect": {} }), "INVALID_SCHEMA"),
        (json!({ "surface_ref": { "entity_id": "nope" }, "center": { "x": 0, "y": 0 }, "zoom": 1 }), "REFERENCE_BROKEN"),
        (json!({ "surface_ref": { "entity_id": "frame1" }, "center": { "x": 0, "y": 0 }, "zoom": 1 }), "INVALID_SCHEMA"),
    ] {
        let r = ws.fail(create(payload.clone()));
        assert_eq!(code(&r), want, "{payload}: {r}");
    }
    // a flow page has no camera: no Viewport there
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "flow-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "c", "payload": { "kind": "folder", "system": "surface_content" } },
        { "op": "entity.create", "entity_id": "flow", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "c", "payload": { "kind": "surface", "layout": { "mode": "flow" }, "content_folder_id": "flow-content" } },
    ]));
    let r = ws.fail(create(json!({ "surface_ref": { "entity_id": "flow" }, "center": { "x": 0, "y": 0 }, "zoom": 1 })));
    assert_eq!(code(&r), "INVALID_OPERATION", "{r}");
}

#[test]
fn steps_are_judged_when_written_and_kept_when_their_target_goes() {
    let mut ws = fixture();
    let mut s = steps(&ws, "path-intro");
    // duplicate ids, wrong target types, unknown keys
    for bad in [
        json!({ "id": "s1", "target": { "kind": "frame", "entity_id": "frame1" } }),
        json!({ "id": "s9", "target": { "kind": "frame", "entity_id": "shape-cover" } }),
        json!({ "id": "s9", "target": { "kind": "viewport", "entity_id": "frame1" } }),
        json!({ "id": "s9", "target": { "kind": "frame", "entity_id": "frame1" }, "transition": "spin" }),
        json!({ "id": "s9", "target": { "kind": "frame", "entity_id": "frame1" }, "zoom": 2 }),
    ] {
        let mut next = s.clone();
        next.push(bad.clone());
        let r = ws.fail(set(&ws, "path-intro", "steps", json!(next)));
        assert_eq!(code(&r), "INVALID_SCHEMA", "{bad}: {r}");
    }
    // the same Frame twice: two steps
    s.push(json!({ "id": "s6", "target": { "kind": "frame", "entity_id": "frame1" }, "title": "回到封面" }));
    ws.ok(set(&ws, "path-intro", "steps", json!(s)));
    assert_eq!(ws.store.refs.iter().filter(|r| r.src_entity_id == "path-intro" && r.dst_entity_id == "frame1").count(), 2);
    // deleting a target is not blocked; the step stays as a dangling item and the path stays editable
    let rev = ws.store.entities["viewport2"].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "viewport2", "expect": { "rev": rev } }]));
    let rev = ws.store.entities["frame3"].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "frame3", "expect": { "rev": rev } }]));
    let mut s = steps(&ws, "path-intro");
    s.swap(0, 1);
    s[2]["enabled"] = json!(false);
    ws.ok(set(&ws, "path-intro", "steps", json!(s.clone())));
    // retargeting a dangling step is judged like a new one
    let i = s.iter().position(|x| x["id"] == "s4").unwrap();
    s[i]["target"] = json!({ "kind": "viewport", "entity_id": "frame2" });
    let r = ws.fail(set(&ws, "path-intro", "steps", json!(s.clone())));
    assert_eq!(code(&r), "INVALID_SCHEMA", "{r}");
    s[i]["target"] = json!({ "kind": "viewport", "entity_id": "viewport1" });
    ws.ok(set(&ws, "path-intro", "steps", json!(s)));
}

#[test]
fn a_frame_step_needs_a_free_surface_and_paths_check_their_stage() {
    let mut ws = fixture();
    let path = |stage: Value| json!([{ "op": "entity.create", "entity_id": "p-new", "type_id": TYPE_SHOW_PATH, "parent_id": SHOWS_ID, "order_key": "x",
        "payload": { "purpose": "presentation", "stage": stage, "steps": [] } }]);
    for stage in [json!({ "w": 0, "h": 1080 }), json!({ "w": 1920 }), json!({ "w": 1920, "h": 1080, "d": 1 })] {
        let r = ws.fail(path(stage.clone()));
        assert_eq!(code(&r), "INVALID_SCHEMA", "{stage}: {r}");
    }
    let r = ws.fail(json!([{ "op": "entity.create", "entity_id": "p-new", "type_id": TYPE_SHOW_PATH, "parent_id": SHOWS_ID, "order_key": "x",
        "payload": { "purpose": "tour", "stage": { "w": 1920, "h": 1080 }, "steps": [] } }]));
    assert_eq!(code(&r), "INVALID_SCHEMA", "{r}");
    ws.ok(path(json!({ "w": 1440, "h": 1080 })));
}

#[test]
fn undo_restores_steps_and_their_references() {
    let mut ws = fixture();
    let mut s = steps(&ws, "path-intro");
    s.truncate(2);
    let seq = ws.ok(set(&ws, "path-intro", "steps", json!(s)));
    assert_eq!(ws.store.refs.iter().filter(|r| r.src_entity_id == "path-intro").count(), 2);
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(steps(&ws, "path-intro").len(), 5);
    assert_eq!(ws.store.refs.iter().filter(|r| r.src_entity_id == "path-intro").count(), 5);
}

#[test]
fn notes_can_be_left_out_of_a_package() {
    let ws = fixture();
    assert!(without_notes(&ws.store.entities["frame1"]).is_some());
    assert!(without_notes(&ws.store.entities["viewport1"]).is_some());
    assert!(without_notes(&ws.store.entities["viewport2"]).is_none(), "no notes, nothing to strip");
    #[derive(Default)]
    struct Mem(std::collections::BTreeMap<String, String>);
    impl ObjectSink for Mem {
        fn put_object(&mut self, t: &str, c: &str) -> aiworkspace_core::WsResult<String> {
            let id = aiworkspace_core::canonical::build_obj_id(t, c);
            self.0.insert(id.clone(), c.to_string());
            Ok(id)
        }
        fn put_file(&mut self, b: &[u8]) -> aiworkspace_core::WsResult<String> {
            let (id, text) = aiworkspace_core::canonical::file_object(b.len() as u64, &aiworkspace_core::canonical::chunk_id(b))?;
            self.0.insert(id.clone(), text);
            Ok(id)
        }
    }
    let mut sink = Mem::default();
    let full = materialize(&ws.store, &mut sink, &|_| Ok(true), &mut |_, _| None).unwrap();
    let shared = materialize_with(&ws.store, &mut sink, &|_| Ok(true), &mut |_, _| None, true).unwrap();
    assert_ne!(full.content_root, shared.content_root);
    // captions are public and stay; only the notes go
    for id in ["frame1", "viewport1"] {
        let text = &sink.0[&shared.objects[id]];
        assert!(!text.contains("开场") && !text.contains("演示参数区"), "{id}: {text}");
        assert!(text.contains("caption"), "{id}: {text}");
    }
    assert_eq!(full.objects["path-intro"], shared.objects["path-intro"]);
}
