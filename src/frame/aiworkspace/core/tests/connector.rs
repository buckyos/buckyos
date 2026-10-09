//! Connector contract (连接线方案 §4, §6, §8.3, C1): a Cell with `view.type = connector`, its payload
//! keys and placement box, endpoint bindings, references that never block deletion, the outline
//! projection, and import keeping dangling endpoints.

use aiworkspace_core::access::{Access, CapSet};
use aiworkspace_core::freshness::relations;
use aiworkspace_core::model::*;
use aiworkspace_core::plan::selector_string;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn surface(id: &str, mode: &str) -> Vec<Value> {
    vec![
        json!({ "op": "entity.create", "entity_id": format!("{id}-content"), "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
                "payload": { "kind": "folder", "system": "surface_content", "surface_id": id, "title": id } }),
        json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a", "name": id,
                "payload": { "kind": "surface", "layout": { "mode": mode }, "title": id, "content_folder_id": format!("{id}-content") } }),
    ]
}

fn frame(id: &str, parent: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": "c",
            "placement": { "x": 0, "y": 0, "w": 100, "h": 60 }, "payload": { "view": { "type": "frame" } } })
}

fn group(id: &str, parent: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": parent, "order_key": "b",
            "placement": { "x": 0, "y": 0, "w": 400, "h": 300 }, "payload": { "kind": "group" } })
}

fn line(id: &str, parent: &str, placement: Value, extra: Value) -> Value {
    let mut payload = json!({ "view": { "type": "connector", "version": 1 } });
    for (k, v) in extra.as_object().into_iter().flatten() {
        payload[k] = v.clone();
    }
    let mut op = json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": "m", "payload": payload });
    if !placement.is_null() {
        op["placement"] = placement;
    }
    op
}

fn named(id: &str, anchor: &str) -> Value {
    json!({ "entity_id": id, "anchor": { "kind": "named", "id": anchor } })
}

fn bound(id: &str) -> Value {
    named(id, "n")
}

fn box_(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "x": x, "y": y, "w": w, "h": h })
}

fn set(id: &str, key: &str, value: Value, ws: &MemWorkspace) -> Value {
    let rev = ws.store.entities[id].key_rev(key);
    json!([{ "op": "entity.set_keys", "entity_id": id, "keys": [{ "key": key, "value": value, "expect": { "rev": rev } }] }])
}

fn outline_of(ws: &MemWorkspace, id: &str) -> Value {
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    outline.into_iter().find(|e| e["entity_id"] == id).unwrap()
}

/// s1 (free): a, g1 { b }; s2 (free): c; s3 (flow): d; data: t1.
fn setup() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!(surface("s2", "free")));
    ws.ok(json!(surface("s3", "flow")));
    ws.ok(json!([frame("a", "s1"), group("g1", "s1"), frame("b", "g1"), frame("c", "s2"),
        { "op": "entity.create", "entity_id": "d", "type_id": TYPE_CELL, "parent_id": "s3", "order_key": "c", "payload": { "view": { "type": "frame" } } },
        { "op": "entity.create", "entity_id": "t1", "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": "b", "name": "t1",
          "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }] } }]));
    ws
}

#[test]
fn free_lines_and_their_box() {
    let mut ws = setup();
    // a horizontal free line: the box may have zero height
    ws.ok(json!([line("l1", "s1", box_(10.0, 20.0, 100.0, 0.0), json!({}))]));
    ws.ok(json!([line("l2", "s1", box_(10.0, 20.0, 0.0, 80.0), json!({ "start": null, "end": null }))]));
    // two coordinate ends never collapse into a point; a bound end may coincide with the free one
    assert_eq!(code(&ws.fail(json!([line("bad", "s1", box_(10.0, 20.0, 0.0, 0.0), json!({}))]))), "INVALID_SCHEMA");
    ws.ok(json!([line("l3", "s1", box_(10.0, 20.0, 0.0, 0.0), json!({ "start": bound("a") }))]));
    // no rotation, no negative or missing size, and the box is mandatory
    for p in [json!({ "x": 0, "y": 0, "w": 10, "h": 10, "rotation": 0 }), json!({ "x": 0, "y": 0, "w": -1, "h": 10 }), json!({ "x": 0, "y": 0, "w": 10 })] {
        assert_eq!(code(&ws.fail(json!([line("bad", "s1", p.clone(), json!({}))]))), "INVALID_SCHEMA", "{p}");
    }
    assert_eq!(code(&ws.fail(json!([line("bad", "s1", Value::Null, json!({}))]))), "INVALID_SCHEMA");
    let mut null_placement = line("bad", "s1", Value::Null, json!({}));
    null_placement["placement"] = Value::Null;
    assert_eq!(code(&ws.fail(json!([null_placement]))), "INVALID_SCHEMA");
    // tree.place: placement null is refused, order_key alone is fine, a vertical box is fine
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.place", "entity_id": "l1", "placement": null }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.place", "entity_id": "l1", "placement": box_(0.0, 0.0, 0.0, 0.0) }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.place", "entity_id": "l1", "placement": { "x": 0, "y": 0, "w": 5, "h": 5, "rotation": 90 } }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "tree.place", "entity_id": "l1", "order_key": "zz" }]));
    let placed = ws.ok(json!([{ "op": "tree.place", "entity_id": "l1", "placement": box_(1.0, 2.0, 0.0, 50.0) }]));
    assert_eq!(ws.store.edges["l1"].placement, Some(box_(1.0, 2.0, 0.0, 50.0)));
    // the inverse puts the old box back
    ws.undo(placed).ok().expect("undo place");
    assert_eq!(ws.store.edges["l1"].placement, Some(box_(10.0, 20.0, 100.0, 0.0)));
    // tree.move needs the box too (into a group on the same canvas, or into a flow page: data is kept there)
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.move", "entity_id": "l1", "new_parent_id": "g1", "order_key": "m" }]))), "INVALID_SCHEMA");
    let moved = ws.ok(json!([{ "op": "tree.move", "entity_id": "l1", "new_parent_id": "g1", "order_key": "m", "placement": box_(0.0, 0.0, 30.0, 0.0) }]));
    ws.ok(json!([{ "op": "tree.move", "entity_id": "l2", "new_parent_id": "s3", "order_key": "m", "placement": box_(0.0, 0.0, 30.0, 0.0) }]));
    ws.undo(moved).ok().expect("undo move");
    assert_eq!((ws.store.edges["l1"].parent_id.as_str(), ws.store.edges["l1"].placement.clone()), ("s1", Some(box_(10.0, 20.0, 100.0, 0.0))));
    // ordinary Blocks keep their rules: no zero size
    let mut flat = frame("bad", "s1");
    flat["placement"] = box_(0.0, 0.0, 100.0, 0.0);
    assert_eq!(code(&ws.fail(json!([flat]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.place", "entity_id": "a", "placement": box_(0.0, 0.0, 0.0, 10.0) }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "tree.place", "entity_id": "a", "placement": { "x": 0, "y": 0, "w": 100, "h": 60, "rotation": 45 } }]));
}

#[test]
fn undo_restores_a_box_written_while_bound() {
    let mut ws = setup();
    // two bound ends may share a stored position; unbinding both and writing a real box in one commit
    // must stay undoable even though the inverse puts the point back before the ends are re-bound
    ws.ok(json!([line("l1", "s1", box_(5.0, 5.0, 0.0, 0.0), json!({ "start": bound("a"), "end": bound("b") }))]));
    let s = ws.store.entities["l1"].key_rev("start");
    let unbind = ws.ok(json!([
        { "op": "entity.set_keys", "entity_id": "l1", "keys": [{ "key": "start", "value": null, "expect": { "rev": s } }, { "key": "end", "value": null, "expect": { "rev": s } }] },
        { "op": "tree.place", "entity_id": "l1", "placement": box_(5.0, 5.0, 40.0, 0.0) }]));
    ws.undo(unbind).ok().expect("undo");
    assert_eq!(ws.store.entities["l1"].payload["start"], bound("a"));
    assert_eq!(ws.store.edges["l1"].placement, Some(box_(5.0, 5.0, 0.0, 0.0)));
}

#[test]
fn connector_keys_shapes_and_view_type() {
    let mut ws = setup();
    // connector keys belong to connectors only
    for (k, v) in [("start", Value::Null), ("route", json!("elbow")), ("flip", json!({})), ("controls", json!([])), ("label", json!({ "t": 0.5 })), ("end", bound("a"))] {
        let mut op = frame("bad", "s1");
        op["payload"][k] = v;
        let f = ws.fail(json!([op]));
        assert_eq!(code(&f), "INVALID_SCHEMA", "{k}");
        assert!(f["errors"][0]["detail"].as_str().unwrap().contains("only valid on connectors"), "{f}");
    }
    assert_eq!(code(&ws.fail(set("a", "route", json!("curve"), &ws))), "INVALID_SCHEMA");
    // a connector shows no data and runs no definition
    for (k, v) in [("source_ref", json!({ "entity_id": "t1" })), ("bindings", json!({ "rows": { "entity_id": "t1" } })), ("def_ref", json!({ "entity_id": "t1" })), ("filter", json!({}))] {
        assert_eq!(code(&ws.fail(json!([line("bad", "s1", box_(0.0, 0.0, 10.0, 10.0), json!({ k: v }))]))), "INVALID_SCHEMA", "{k}");
    }
    // the full shape, as the design example writes it
    ws.ok(json!([line("l1", "s1", box_(320.0, 180.0, 320.0, 60.0), json!({
        "title": "下一步", "locked": false, "flip": { "h": true, "v": false },
        "start": named("a", "e"), "end": null, "route": "elbow",
        "controls": [{ "u": 0.5, "v": 0 }, { "u": 0.5, "v": 1, "dx": -12.5, "dy": 0 }],
        "label": { "t": 0.5, "offset": 0 },
        "config": { "stroke": "#64748b", "width": 2, "dash": "solid", "start_cap": "none", "end_cap": "arrow" } }))]));
    // strict shapes
    let many: Vec<Value> = (0..65).map(|_| json!({ "u": 0, "v": 0 })).collect();
    assert_eq!(code(&ws.fail(set("l1", "controls", json!(many), &ws))), "LIMIT_EXCEEDED");
    let max: Vec<Value> = (0..64).map(|i| json!({ "u": 0, "v": i })).collect();
    ws.ok(set("l1", "controls", json!(max), &ws));
    // any anchor id of the allowed alphabet: the target's renderer says what it means
    for end in [named("a", "nne"), named("a", "port:in-2"), named("a", &"x".repeat(64))] {
        ws.ok(set("l1", "start", end, &ws));
    }
    for (k, v) in [
        ("flip", json!({ "h": true, "x": true })),
        ("flip", json!({ "h": "yes" })),
        ("flip", json!(true)),
        ("start", json!({ "entity_id": "a" })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named", "id": "n" }, "extra": 1 })),
        ("start", json!({ "entity_id": "A B", "anchor": { "kind": "named", "id": "n" } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "auto" } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "point", "x": 1, "y": 0.5 } })),
        ("start", json!({ "entity_id": "a", "anchor": "n" })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "port", "port": 1 } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named" } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named", "id": "" } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named", "id": "n e" } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named", "id": "上" } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named", "id": "x".repeat(65) } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named", "id": 3 } })),
        ("start", json!({ "entity_id": "a", "anchor": { "kind": "named", "id": "n", "x": 0 } })),
        ("end", json!("a")),
        ("route", json!("zigzag")),
        ("route", Value::Null),
        ("controls", Value::Null),
        ("controls", json!([{ "u": 0 }])),
        ("controls", json!([{ "u": 0, "v": 1, "dz": 0 }])),
        ("controls", json!([{ "u": "0", "v": 1 }])),
        ("label", json!({ "t": 2 })),
        ("label", json!({ "offset": 1 })),
        ("label", json!({ "t": 0.5, "offset": "a" })),
        ("label", json!({ "t": 0.5, "side": "left" })),
        ("label", Value::Null),
    ] {
        assert_eq!(code(&ws.fail(set("l1", k, v.clone(), &ws))), "INVALID_SCHEMA", "{k}: {v}");
    }
    // a line never turns into a box, nor a box into a line; the version may change
    let f = ws.fail(set("l1", "view", json!({ "type": "frame" }), &ws));
    assert_eq!((code(&f), f["errors"][0]["detail"].as_str().unwrap()), ("INVALID_OPERATION", "view.type cannot change to or from connector"));
    assert_eq!(code(&ws.fail(set("a", "view", json!({ "type": "connector" }), &ws))), "INVALID_OPERATION");
    ws.ok(set("a", "view", json!({ "type": "shape" }), &ws));
    ws.ok(set("l1", "view", json!({ "type": "connector" }), &ws));
    // unset keys fall back to their defaults
    let rev = ws.store.entities["l1"].key_rev("route");
    ws.ok(json!([{ "op": "entity.unset_keys", "entity_id": "l1", "keys": [{ "key": "route", "expect": { "rev": rev } }] }]));
}

#[test]
fn bindings_stay_on_one_free_canvas() {
    let mut ws = setup();
    // a Block and a group on the same canvas, including the connector's own ancestor group
    ws.ok(json!([line("l1", "g1", box_(0.0, 0.0, 50.0, 50.0), json!({ "start": bound("g1"), "end": named("b", "n") }))]));
    ws.ok(json!([line("l2", "s1", box_(0.0, 0.0, 50.0, 50.0), json!({ "start": bound("a"), "end": bound("g1") }))]));
    // targets created earlier in the same batch are visible
    ws.ok(json!([frame("e", "g1"), line("l3", "s1", box_(0.0, 0.0, 50.0, 50.0), json!({ "start": bound("e") }))]));
    // refused targets: itself, another line, data, a Surface, another canvas, missing and deleted ones
    let refuse = |ws: &mut MemWorkspace, end: Value| ws.fail(json!([line("bad", "s1", box_(0.0, 0.0, 50.0, 50.0), json!({ "start": end }))]));
    for (target, want) in [("bad", "INVALID_SCHEMA"), ("l2", "INVALID_SCHEMA"), ("t1", "INVALID_SCHEMA"), ("s1", "INVALID_SCHEMA"), ("c", "INVALID_SCHEMA"), ("ghost", "REFERENCE_BROKEN")] {
        assert_eq!(code(&refuse(&mut ws, bound(target))), want, "{target}");
    }
    assert_eq!(code(&ws.fail(set("l2", "end", bound("l2"), &ws))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(set("l2", "end", bound("l1"), &ws))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(set("l2", "end", bound("s1-content"), &ws))), "INVALID_SCHEMA");
    // a deleted target: does not block deletion, cannot be newly bound, and the dangling end stays editable
    let life = ws.store.entities["b"].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "b", "expect": { "rev": life } }]));
    assert_eq!(code(&refuse(&mut ws, bound("b"))), "REFERENCE_BROKEN");
    ws.ok(set("l1", "route", json!("curve"), &ws));
    ws.ok(set("l1", "start", bound("a"), &ws));
    ws.ok(set("l1", "label", json!({ "t": 0.25 }), &ws));
    assert_eq!(ws.store.entities["l1"].payload["end"], named("b", "n"));
    assert_eq!(code(&ws.fail(set("l1", "end", named("b", "s"), &ws))), "REFERENCE_BROKEN");
    // creating on a flow page is refused; a line moved there keeps its data but gets no new bindings
    let f = ws.fail(json!([line("bad", "s3", box_(0.0, 0.0, 50.0, 50.0), json!({}))]));
    assert_eq!(code(&f), "INVALID_OPERATION", "{f}");
    ws.ok(json!([{ "op": "tree.move", "entity_id": "l2", "new_parent_id": "s3", "order_key": "m", "placement": box_(0.0, 0.0, 50.0, 50.0) }]));
    assert_eq!(code(&ws.fail(set("l2", "end", bound("d"), &ws))), "INVALID_OPERATION");
    ws.ok(set("l2", "route", json!("elbow"), &ws));
    // an unreadable target reads as missing: an id does not bypass the read check
    let dave = Access { principal: "dave".into(), ws_caps: CapSet::NONE, scoped: [("g1".to_string(), CapSet::ALL)].into() };
    let env = ws.env("dave", "human", None, false);
    let req = |end: &str| MemWorkspace::request(json!([line("dl", "g1", box_(0.0, 0.0, 10.0, 10.0), json!({ "start": bound("e"), "end": bound(end) }))]));
    assert_eq!(code(&ws.commit_with(&dave, &env, &req("a")).err().unwrap().to_json()), "REFERENCE_BROKEN");
    ws.commit_with(&dave, &env, &req("g1")).ok().expect("readable targets");
}

#[test]
fn self_loops_need_two_anchors_and_a_route() {
    let mut ws = setup();
    let make = |ws: &mut MemWorkspace, extra: Value| ws.commit(json!([line("loop", "s1", box_(0.0, 0.0, 40.0, 40.0), extra)]));
    for extra in [
        json!({ "start": named("a", "e"), "end": named("a", "e"), "route": "elbow" }),
        json!({ "start": named("a", "e"), "end": named("a", "n") }),
        json!({ "start": named("a", "e"), "end": named("a", "n"), "route": "straight" }),
    ] {
        assert_eq!(code(&make(&mut ws, extra.clone()).err().expect("refused").to_json()), "INVALID_SCHEMA", "{extra}");
    }
    make(&mut ws, json!({ "start": named("a", "e"), "end": named("a", "n"), "route": "elbow" })).ok().expect("self loop");
    assert_eq!(code(&ws.fail(set("loop", "route", json!("straight"), &ws))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(set("loop", "end", named("a", "e"), &ws))), "INVALID_SCHEMA");
    ws.ok(set("loop", "route", json!("curve"), &ws));
    ws.ok(set("loop", "end", named("a", "nne"), &ws));
    ws.ok(set("loop", "end", bound("g1"), &ws));
}

#[test]
fn endpoint_references_relations_and_outline() {
    let mut ws = setup();
    ws.ok(json!([line("l1", "s1", box_(0.0, 0.0, 50.0, 0.0), json!({})),
        line("l2", "g1", box_(0.0, 0.0, 50.0, 50.0), json!({ "start": named("a", "e"), "end": bound("b"), "route": "elbow", "controls": [{ "u": 0.5, "v": 0 }] }))]));
    let refs: Vec<&RefEdge> = ws.store.refs.iter().filter(|r| r.src_entity_id == "l2").collect();
    assert_eq!(refs.len(), 2);
    for (end, target) in [("start", "a"), ("end", "b")] {
        let s = selector_string(&json!({ "kind": "connector_end", "end": end }));
        assert!(refs.iter().any(|r| r.kind == "connector_endpoint" && r.src_selector == s && r.dst_entity_id == target), "{end}");
        assert!(!refs[0].blocks_delete());
    }
    assert!(!ws.store.refs.iter().any(|r| r.src_entity_id == "l1"));
    // relations of the target list the line as an incoming, non-blocking reference
    let rel = relations(&ws.store, &Access::full("alice"), "a").unwrap();
    let incoming = rel["incoming"].as_array().unwrap();
    assert_eq!(incoming.len(), 1, "{rel}");
    assert_eq!((incoming[0]["kind"].as_str(), incoming[0]["entity_id"].as_str()), (Some("connector_endpoint"), Some("l2")));
    assert_eq!(incoming[0]["blocks_delete"], false);
    assert_eq!(incoming[0]["source"]["view_type"], "connector");
    assert_eq!(rel["blocks"], json!([]));
    // a line takes no part in freshness
    assert_eq!(rel["freshness"]["status"], "none");
    // rebinding moves the reference; unbinding drops it
    ws.ok(set("l2", "end", bound("g1"), &ws));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "l2" && r.dst_entity_id == "g1"));
    assert!(!ws.store.refs.iter().any(|r| r.src_entity_id == "l2" && r.dst_entity_id == "b"));
    // deleting a bound target is accepted (the line keeps its stored end) and restoring it re-binds
    let life = ws.store.entities["a"].life_rev;
    let del = ws.ok(json!([{ "op": "entity.delete", "entity_id": "a", "expect": { "rev": life } }]));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "l2" && r.dst_entity_id == "a"));
    ws.undo(del).ok().expect("restore");
    assert!(ws.store.entities["a"].alive());
    // outline: the geometry, with start / end always present, absent keys omitted, config not projected
    let o = outline_of(&ws, "l2");
    assert_eq!(o["view_type"], "connector");
    assert_eq!(o["connector"], json!({ "start": named("a", "e"), "end": bound("g1"), "route": "elbow", "controls": [{ "u": 0.5, "v": 0 }] }));
    assert_eq!(outline_of(&ws, "l1")["connector"], json!({ "start": null, "end": null }));
    assert_eq!(outline_of(&ws, "l1")["placement"], box_(0.0, 0.0, 50.0, 0.0));
    ws.ok(set("l1", "flip", json!({ "h": true }), &ws));
    ws.ok(set("l1", "label", json!({ "t": 0.5, "offset": 4 }), &ws));
    ws.ok(set("l1", "config", json!({ "end_cap": "arrow" }), &ws));
    assert_eq!(outline_of(&ws, "l1")["connector"], json!({ "start": null, "end": null, "flip": { "h": true }, "label": { "t": 0.5, "offset": 4 } }));
    assert!(outline_of(&ws, "a").get("connector").is_none());
}

#[test]
fn import_keeps_dangling_endpoints() {
    use aiworkspace_core::materialize::{load_ops, materialize, ObjectSink, ObjectSource};
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Mem(BTreeMap<String, String>, BTreeMap<String, Vec<u8>>);
    impl ObjectSink for Mem {
        fn put_object(&mut self, t: &str, c: &str) -> aiworkspace_core::WsResult<String> {
            let id = aiworkspace_core::canonical::build_obj_id(t, c);
            self.0.insert(id.clone(), c.to_string());
            Ok(id)
        }
        fn put_file(&mut self, b: &[u8]) -> aiworkspace_core::WsResult<String> {
            let chunk = aiworkspace_core::canonical::chunk_id(b);
            let (id, text) = aiworkspace_core::canonical::file_object(b.len() as u64, &chunk)?;
            self.0.insert(id.clone(), text);
            self.1.insert(id.clone(), b.to_vec());
            Ok(id)
        }
    }
    impl ObjectSource for Mem {
        fn get_object(&self, id: &str) -> aiworkspace_core::WsResult<String> {
            self.0.get(id).cloned().ok_or_else(|| aiworkspace_core::WsError::not_found(id))
        }
        fn get_file(&self, id: &str) -> aiworkspace_core::WsResult<Vec<u8>> {
            self.1.get(id).cloned().ok_or_else(|| aiworkspace_core::WsError::not_found(id))
        }
    }
    let mut ws = setup();
    ws.ok(json!([line("l1", "s1", box_(0.0, 0.0, 50.0, 50.0), json!({ "start": bound("a"), "end": named("b", "w"), "route": "curve" }))]));
    // the end's target is left out of the package: the line travels with its dangling end
    let mut sink = Mem::default();
    let m = materialize(&ws.store, &mut sink, &|e: &EntityRow| Ok(e.entity_id != "b"), &mut |_, _| None).unwrap();
    let plan = load_ops(&sink, &m.content_root, &|_| None).unwrap();
    let mut fresh = MemWorkspace::new();
    let mut env = fresh.env("alice", "import", None, true);
    env.import = true;
    fresh.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(Value::Array(plan.ops.clone()))).ok().expect("import");
    assert_eq!(fresh.store.entities["l1"].payload["end"], named("b", "w"));
    assert!(fresh.store.entities.get("b").is_none());
    assert!(fresh.store.refs.iter().any(|r| r.src_entity_id == "l1" && r.kind == "connector_endpoint" && r.dst_entity_id == "b"));
    // replay of what a backend accepted: a line on a flow page, bound across canvases, is kept verbatim
    let mut env = ws.env("alice", "human", None, true);
    env.import = true;
    env.replay = true;
    ws.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(json!([line("l9", "s3", box_(0.0, 0.0, 5.0, 5.0), json!({ "start": bound("c"), "end": bound("ghost") }))])))
        .ok()
        .expect("replay");
    assert_eq!(ws.store.entities["l9"].payload["end"], bound("ghost"));
    // shapes are still checked there
    let bad = MemWorkspace::request(json!([line("l10", "s1", box_(0.0, 0.0, 5.0, 5.0), json!({ "route": "zigzag" }))]));
    assert_eq!(code(&ws.commit_with(&Access::full("alice"), &env, &bad).err().unwrap().to_json()), "INVALID_SCHEMA");
}
