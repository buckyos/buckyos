//! Block-control contract (脑图对象方案 §4–§6, TODO M0): `mindmap` containers, `"auto"` placement components only
//! where a control lays Blocks out, Blocks under Blocks inside a control, non-text nodes keeping a numeric size,
//! `slot` unique among siblings, the outline projection, deleting and undoing a subtree.

use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("../../fixtures/mindmap/commits.json");

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn sub(v: &Value) -> &str {
    v["sub_code"].as_str().unwrap_or("")
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

fn outline(ws: &MemWorkspace) -> Vec<Value> {
    aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap()
}

fn outline_of(ws: &MemWorkspace, id: &str) -> Value {
    outline(ws).into_iter().find(|e| e["entity_id"] == id).unwrap()
}

const AUTO: &str = "auto";

fn node(id: &str, parent: &str, key: &str, placement: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": key,
            "placement": placement, "payload": { "view": { "type": "mindnode" }, "title": id } })
}

fn auto() -> Value {
    json!({ "x": AUTO, "y": AUTO, "w": AUTO, "h": AUTO })
}

#[test]
fn fixture_is_accepted_and_projected() {
    let ws = fixture();
    let mm = outline_of(&ws, "mm-both");
    assert_eq!(mm["kind"], "mindmap");
    assert_eq!(mm["layout"], json!({ "mode": "tree" }), "the layout is implied by the kind");
    assert_eq!(mm["config"]["direction"], "both", "a control's attributes are projected for the layout");
    assert_eq!(mm["placement"], json!({ "x": 0, "y": 1400, "w": AUTO, "h": AUTO }));
    let n = outline_of(&ws, "both-2-1");
    assert_eq!(n["parent_id"], "both-2", "a node sits under its parent node");
    assert_eq!(n["placement"], auto(), "\"auto\" components are passed through verbatim");
    assert_eq!(n["title"], "二.1");
    assert_eq!(outline_of(&ws, "both-3")["placement"], json!({ "x": 420, "y": -160, "w": AUTO, "h": AUTO }), "a pinned node");
    let side = outline_of(&ws, "both-1");
    assert_eq!((side["side"].as_str(), side["branch_color"].as_str()), (Some("left"), Some("#e8590c")));
    assert_eq!(outline_of(&ws, "slot-bkg")["slot"], "bkg");
    assert!(outline_of(&ws, "min-1").get("slot").is_none_or(Value::is_null));
    assert_eq!(outline_of(&ws, "min-inner")["placement"], auto(), "a nested control is a node of the outer one");
    assert_eq!(outline_of(&ws, "min-frame")["placement"], json!({ "x": AUTO, "y": AUTO, "w": 160, "h": 90 }));
    // a group's config is not projected (only a control's)
    assert!(outline(&ws).iter().filter(|e| e["kind"] == "surface").all(|e| e.get("config").is_none_or(Value::is_null)));
}

#[test]
fn container_shape() {
    let mut ws = fixture();
    let mm = |id: &str, payload: Value| {
        json!([{ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": "s-mind", "order_key": "y",
                 "placement": { "x": 0, "y": 0, "w": AUTO, "h": AUTO }, "payload": payload }])
    };
    assert_eq!(code(&ws.fail(mm("m1", json!({ "kind": "mindmap", "layout": { "mode": "free" } })))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(mm("m1", json!({ "kind": "mindmap", "config": { "direction": "up" } })))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(mm("m1", json!({ "kind": "mindmap", "config": { "spacing": { "sibling": -1 } } })))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(mm("m1", json!({ "kind": "mindmap", "config": { "zoom": 2 } })))), "INVALID_SCHEMA");
    assert_eq!(sub(&ws.fail(mm("m1", json!({ "kind": "carousel" })))), "CHILD_NOT_ALLOWED", "an unknown control kind");
    // a group has no config and no auto size
    assert_eq!(code(&ws.fail(mm("g1", json!({ "kind": "group", "config": {} })))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(mm("g1", json!({ "kind": "group" })))), "INVALID_SCHEMA");
    ws.ok(mm("m1", json!({ "kind": "mindmap", "locked": true, "config": { "edge": "elbow", "branch_colors": ["#4f8df7", "#e8590c"] } })));
    assert_eq!(outline_of(&ws, "m1")["layout"], json!({ "mode": "tree" }));
    // a control's x / y on the canvas are numbers: nothing lays the control itself out there
    let free = json!([{ "op": "entity.create", "entity_id": "m2", "type_id": TYPE_CONTAINER, "parent_id": "s-mind", "order_key": "y",
                        "placement": { "x": AUTO, "y": 0, "w": AUTO, "h": AUTO }, "payload": { "kind": "mindmap" } }]);
    assert_eq!(code(&ws.fail(free)), "INVALID_SCHEMA");
    // the kind of a container never changes
    let rev = ws.store.entities["mm-min"].key_rev("kind");
    let change = json!([{ "op": "entity.set_keys", "entity_id": "mm-min", "keys": [{ "key": "kind", "value": "group", "expect": { "rev": rev } }] }]);
    assert_eq!(code(&ws.fail(change)), "INVALID_OPERATION");
}

#[test]
fn auto_only_inside_a_control() {
    let mut ws = fixture();
    // on the free canvas
    assert_eq!(code(&ws.fail(json!([node("n1", "s-mind", "y", auto())]))), "INVALID_SCHEMA");
    // in a plain group
    ws.ok(json!([{ "op": "entity.create", "entity_id": "g1", "type_id": TYPE_CONTAINER, "parent_id": "s-mind", "order_key": "y",
                   "placement": { "x": 0, "y": 0, "w": 400, "h": 300 }, "payload": { "kind": "group" } }]));
    assert_eq!(code(&ws.fail(json!([node("n1", "g1", "a", json!({ "x": AUTO, "y": 0, "w": 100, "h": 40 }))]))), "INVALID_SCHEMA");
    // under a node: fine at any depth
    ws.ok(json!([node("n1", "min-1-1", "a", auto()), node("n2", "n1", "a", json!({ "x": 10, "y": AUTO, "w": AUTO, "h": 30 }))]));
    // a group inside a control is laid out (x / y) but keeps its size
    ws.ok(json!([{ "op": "entity.create", "entity_id": "g2", "type_id": TYPE_CONTAINER, "parent_id": "min-1", "order_key": "z",
                   "placement": { "x": AUTO, "y": AUTO, "w": 200, "h": 120 }, "payload": { "kind": "group" } }]));
    let g3 = json!([{ "op": "entity.create", "entity_id": "g3", "type_id": TYPE_CONTAINER, "parent_id": "min-1", "order_key": "zz",
                      "placement": { "x": AUTO, "y": AUTO, "w": AUTO, "h": 120 }, "payload": { "kind": "group" } }]);
    assert_eq!(code(&ws.fail(g3)), "INVALID_SCHEMA");
    // inside that group the free layout rules again
    assert_eq!(code(&ws.fail(json!([node("n3", "g2", "a", auto())]))), "INVALID_SCHEMA");
    // anything but "auto" or a number
    assert_eq!(code(&ws.fail(json!([node("n4", "min-1", "z", json!({ "x": "fit", "y": AUTO, "w": AUTO, "h": AUTO }))]))), "INVALID_SCHEMA");
    // tree.place writes "auto" back (reset) and pins again
    let rev = ws.store.edges["both-3"].struct_rev;
    ws.ok(json!([{ "op": "tree.place", "entity_id": "both-3", "placement": auto(), "expect": { "rev": rev } }]));
    assert_eq!(outline_of(&ws, "both-3")["placement"], auto());
    ws.ok(json!([{ "op": "tree.place", "entity_id": "both-3", "placement": { "x": 5, "y": 6, "w": AUTO, "h": AUTO } }]));
}

#[test]
fn non_text_nodes_keep_a_numeric_size() {
    let mut ws = fixture();
    let frame = |placement: Value| {
        json!([{ "op": "entity.create", "entity_id": "f1", "type_id": TYPE_CELL, "parent_id": "min-1", "order_key": "z",
                 "placement": placement, "payload": { "view": { "type": "frame" } } }])
    };
    let e = ws.fail(frame(auto()));
    assert_eq!(code(&e), "INVALID_SCHEMA", "{e}");
    assert_eq!(code(&ws.fail(frame(json!({ "x": AUTO, "y": AUTO, "w": 100, "h": AUTO })))), "INVALID_SCHEMA");
    ws.ok(frame(json!({ "x": AUTO, "y": AUTO, "w": 100, "h": 60 })));
    // a text node turned into another view while its size is auto
    let rev = ws.store.entities["min-1"].key_rev("view");
    let turn = json!([{ "op": "entity.set_keys", "entity_id": "min-1", "keys": [{ "key": "view", "value": { "type": "frame" }, "expect": { "rev": rev } }] }]);
    assert_eq!(code(&ws.fail(turn)), "INVALID_SCHEMA");
    // shapes and rich text size themselves too
    ws.ok(json!([{ "op": "entity.create", "entity_id": "s1", "type_id": TYPE_CELL, "parent_id": "min-1", "order_key": "zz",
                   "placement": auto(), "payload": { "view": { "type": "shape" }, "title": "形状" } }]));
    // tree.place of a non-text node cannot set an auto size either
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.place", "entity_id": "min-frame", "placement": auto() }]))), "INVALID_SCHEMA");
}

#[test]
fn only_nodes_inside_a_control_hold_blocks() {
    let mut ws = fixture();
    ws.ok(json!([{ "op": "entity.create", "entity_id": "g1", "type_id": TYPE_CONTAINER, "parent_id": "s-mind", "order_key": "y",
                   "placement": { "x": 0, "y": 0, "w": 400, "h": 300 }, "payload": { "kind": "group" } },
                 { "op": "entity.create", "entity_id": "leaf", "type_id": TYPE_CELL, "parent_id": "g1", "order_key": "a",
                   "placement": { "x": 0, "y": 0, "w": 100, "h": 40 }, "payload": { "view": { "type": "shape" } } },
                 { "op": "entity.create", "entity_id": "top", "type_id": TYPE_CELL, "parent_id": "s-mind", "order_key": "z",
                   "placement": { "x": 0, "y": 0, "w": 100, "h": 40 }, "payload": { "view": { "type": "shape" } } }]));
    let under = |parent: &str| json!([node("n1", parent, "a", json!({ "x": 0, "y": 0, "w": 100, "h": 40 }))]);
    assert_eq!(sub(&ws.fail(under("leaf"))), "CHILD_NOT_ALLOWED", "a Cell in a group stays a leaf");
    assert_eq!(sub(&ws.fail(under("top"))), "CHILD_NOT_ALLOWED", "a Cell on the canvas stays a leaf");
    // a control under a node (R24) and a node under a control's node
    ws.ok(json!([{ "op": "entity.create", "entity_id": "m2", "type_id": TYPE_CONTAINER, "parent_id": "min-2-1", "order_key": "a",
                   "placement": auto(), "payload": { "kind": "mindmap" } }, node("m2-root", "m2", "a", auto())]));
    // data never enters the BlockTree
    let table = json!([{ "op": "entity.create", "entity_id": "t1", "type_id": TYPE_TABLE, "parent_id": "min-1", "order_key": "z", "payload": { "fields": [] } }]);
    assert_eq!(sub(&ws.fail(table)), "CHILD_NOT_ALLOWED");
    // a connector is drawn on the canvas, not inside a control
    let line = json!([{ "op": "entity.create", "entity_id": "l1", "type_id": TYPE_CELL, "parent_id": "mm-min", "order_key": "z",
                        "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "connector" } } }]);
    assert_eq!(code(&ws.fail(line)), "INVALID_OPERATION");
}

#[test]
fn moving_nodes() {
    let mut ws = fixture();
    // re-parent within the map (placement stays auto) and across maps
    ws.ok(json!([{ "op": "tree.move", "entity_id": "min-1", "new_parent_id": "min-3", "order_key": "b", "placement": auto() }]));
    assert_eq!(outline_of(&ws, "min-1-1")["parent_id"], "min-1", "the subtree moves along");
    ws.ok(json!([{ "op": "tree.move", "entity_id": "min-1", "new_parent_id": "right-root", "order_key": "z", "placement": auto() }]));
    // no cycles
    let cycle = json!([{ "op": "tree.move", "entity_id": "right-root", "new_parent_id": "min-1-1", "order_key": "z", "placement": auto() }]);
    assert_eq!(sub(&ws.fail(cycle)), "TREE_CYCLE");
    // out of the control (R9: the Desktop refuses it; the core refuses what would break the rules)
    let out = |placement: Value| json!([{ "op": "tree.move", "entity_id": "min-2-1", "new_parent_id": "s-mind", "order_key": "z", "placement": placement }]);
    assert_eq!(code(&ws.fail(out(auto()))), "INVALID_SCHEMA", "an auto placement cannot leave the control");
    ws.ok(out(json!({ "x": 0, "y": 0, "w": 120, "h": 40 })));
    // a node with children is no leaf: it cannot leave
    let with_kids = json!([{ "op": "tree.move", "entity_id": "min-1", "new_parent_id": "s-mind", "order_key": "zz", "placement": { "x": 0, "y": 0, "w": 120, "h": 40 } }]);
    assert_eq!(sub(&ws.fail(with_kids)), "CHILD_NOT_ALLOWED");
    // moving the whole control keeps its nodes valid
    ws.ok(json!([{ "op": "tree.place", "entity_id": "mm-min", "placement": { "x": 50, "y": 60, "w": AUTO, "h": AUTO } }]));
}

#[test]
fn slots() {
    let mut ws = fixture();
    let slotted = |id: &str, parent: &str, slot: &str| {
        json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": "z",
                "placement": auto(), "payload": { "view": { "type": "mindnode" }, "slot": slot } })
    };
    let e = ws.fail(json!([slotted("x1", "slot-root", "bkg")]));
    assert_eq!(sub(&e), "SLOT_CONFLICT", "{e}");
    // the same name in another namespace (another parent) is fine
    ws.ok(json!([slotted("x1", "slot-bkg", "bkg")]));
    // two in one commit
    assert_eq!(sub(&ws.fail(json!([slotted("x2", "slot-text", "icon"), slotted("x3", "slot-text", "icon")]))), "SLOT_CONFLICT");
    assert_eq!(code(&ws.fail(json!([slotted("x2", "slot-text", "Bad Name")]))), "INVALID_SCHEMA");
    // outside a control
    let top = json!([{ "op": "entity.create", "entity_id": "x4", "type_id": TYPE_CELL, "parent_id": "s-mind", "order_key": "z",
                       "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "shape" }, "slot": "bkg" } }]);
    assert_eq!(code(&ws.fail(top)), "INVALID_SCHEMA");
    // a move into a namespace where the name is taken
    let clash = json!([{ "op": "tree.move", "entity_id": "x1", "new_parent_id": "slot-root", "order_key": "zz", "placement": auto() }]);
    assert_eq!(sub(&ws.fail(clash)), "SLOT_CONFLICT");
    // renaming the slot of a Block
    let rev = ws.store.entities["slot-text"].key_rev("slot");
    let rename = json!([{ "op": "entity.set_keys", "entity_id": "slot-text", "keys": [{ "key": "slot", "value": "bkg", "expect": { "rev": rev } }] }]);
    assert_eq!(sub(&ws.fail(rename)), "SLOT_CONFLICT");
}

#[test]
fn delete_and_undo_a_subtree() {
    let mut ws = fixture();
    let kids: Vec<Value> = ["min-2-1", "min-2-2", "min-2-3", "min-inner", "inner-root", "inner-1", "inner-2"].iter().map(|s| json!(s)).collect();
    let rev = ws.store.entities["min-2"].life_rev;
    let refused = ws.fail(json!([{ "op": "entity.delete", "entity_id": "min-2", "expect": { "rev": rev } }]));
    assert_eq!(code(&refused), "INVALID_OPERATION", "a node with children needs its subtree listed");
    let seq = ws.ok(json!([{ "op": "entity.delete", "entity_id": "min-2", "subtree": { "delete": kids }, "expect": { "rev": rev } }]));
    assert!(outline(&ws).iter().all(|e| e["entity_id"] != "inner-1"));
    assert!(ws.undo(seq).is_ok());
    assert_eq!(outline_of(&ws, "inner-1")["parent_id"], "inner-root");
    assert_eq!(outline_of(&ws, "min-inner")["placement"], auto());
}

#[test]
fn old_protocol_requests_are_still_understood() {
    assert!(supports_protocol("1.0"));
    assert!(supports_protocol(PROTOCOL_VERSION));
    assert!(!supports_protocol("1.99"));
    assert!(!supports_protocol("2.0"));
    assert!(!supports_protocol("1"));
}
