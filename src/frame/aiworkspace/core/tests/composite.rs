//! Extension controls P1 (notepads/aiworkspace-extension-p1-todo.md M2): composite definitions and their instances
//! (`buckyos.container { kind: composite, def_ref }` with slotted children), definition → definition dependencies
//! (`def_dep`, `REFERENCE_CYCLE`, only builds write `deps`), and the split of the validators (Q15): replay runs the
//! primitive and reference checks only.

use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::plan::{plan_commit, PlanEnv};
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::types::schema_validations;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn sub(v: &Value) -> &str {
    v["sub_code"].as_str().unwrap_or("")
}

const ALICE: &str = "did:bns:alice";

fn composition(algorithm: &str) -> Value {
    json!({ "layout": { "algorithm": algorithm, "direction": "column", "spacing": "{{config.gap}}" },
            "slots": { "header": { "max": 1, "accepts": ["richtext"], "placement": "top" }, "cards": { "accepts": ["cell:did:bns:alice#card", "shape"] } },
            "chrome": { "background": "{{config.background}}", "title": { "text": "{{config.title}}" } } })
}

fn def(id: &str, name: &str, key: &str, kind: &str, extra: Value) -> Value {
    let mut p = json!({ "def_id": name, "kind": kind, "title": name, "ident": { "author_did": ALICE, "name": name }, "manifest_version": 2 });
    match kind {
        "composite" => p["composition"] = composition("stack"),
        _ => p["declarative"] = json!({ "items": [] }),
    }
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": key, "name": format!("{ALICE}#{name}"), "payload": p })
}

fn as_build(ws: &mut MemWorkspace, ops: Value) -> Result<u64, Value> {
    let env = ws.env("alice", "build", None, false);
    ws.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(ops)).map_err(|f| f.to_json())
}

/// A free Surface `s` with its content folder, a card definition and a board (composite) depending on it.
fn setup() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "s-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s" } },
        { "op": "entity.create", "entity_id": "s", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s-content" } },
        def("d-card", "card", "a", "declarative", json!({})),
    ]));
    as_build(&mut ws, json!([def("d-board", "board", "b", "composite", json!({ "deps": [{ "ident": "did:bns:alice#card", "entity_id": "d-card" }] }))])).unwrap();
    ws
}

fn board(id: &str, extra: Value) -> Value {
    let mut payload = json!({ "kind": "composite", "def_ref": { "entity_id": "d-board" }, "config": { "title": "Today", "gap": 8 } });
    for (k, v) in extra.as_object().unwrap() {
        payload[k] = v.clone();
    }
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": "s", "order_key": "m",
            "placement": { "x": 10, "y": 10, "w": "auto", "h": "auto" }, "payload": payload })
}

fn card(id: &str, parent: &str, key: &str, slot: Option<&str>, placement: Value) -> Value {
    let mut payload = json!({ "view": { "type": "card" }, "def_ref": { "entity_id": "d-card" }, "title": id });
    if let Some(s) = slot {
        payload["slot"] = json!(s);
    }
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": key, "placement": placement, "payload": payload })
}

fn laid_out() -> Value {
    json!({ "x": "auto", "y": "auto", "w": 200, "h": 80 })
}

#[test]
fn composite_definitions_keep_their_shape() {
    let mut ws = setup();
    let d = &ws.store.entities["d-board"];
    assert_eq!(d.payload["kind"], "composite");
    let mut env = json!({});
    aiworkspace_core::read::outline_extras(&mut env, d);
    assert_eq!(env["deps"], json!(["did:bns:alice#card"]), "the outline names what it builds on");
    assert_eq!(env["composition"]["layout"]["algorithm"], "stack", "instances are laid out from the outline alone");
    assert_eq!(env["config_defaults"], json!({}));
    for (extra, expected) in [
        (json!({ "composition": { "slots": { "Bad": {} } } }), "INVALID_SCHEMA"),
        (json!({ "composition": { "slots": {}, "events": {} } }), "INVALID_SCHEMA"),
        (json!({ "composition": { "layout": {} } }), "INVALID_SCHEMA"),
        (json!({ "composition": { "slots": (0..17).map(|i| (format!("s{i}"), json!({}))).collect::<serde_json::Map<String, Value>>() } }), "LIMIT_EXCEEDED"),
        (json!({ "manifest_version": 3 }), "UNSUPPORTED_VERSION"),
    ] {
        let f = as_build(&mut ws, json!([def("d-x", "x", "c", "composite", extra.clone())])).unwrap_err();
        assert_eq!(code(&f), expected, "{extra}: {f}");
    }
    // a composite has no body of its own, a render definition no composition
    let mut no_comp = def("d-x", "x", "c", "composite", json!({}));
    no_comp["payload"].as_object_mut().unwrap().remove("composition");
    assert_eq!(code(&ws.fail(json!([no_comp]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([def("d-x", "x", "c", "declarative", json!({ "composition": composition("stack") }))]))), "INVALID_SCHEMA");
}

#[test]
fn only_builds_write_deps_and_dependencies_never_close_a_cycle() {
    let mut ws = setup();
    // a person cannot claim a dependency
    let f = ws.fail(json!([def("d-y", "y", "c", "composite", json!({ "deps": [{ "ident": "did:bns:alice#card", "entity_id": "d-card" }] }))]));
    assert_eq!(code(&f), "INVALID_OPERATION", "{f}");
    let rev = ws.store.entities["d-board"].key_rev("deps");
    let f = ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "d-board", "keys": [{ "key": "deps", "value": [], "expect": { "rev": rev } }] }]));
    assert_eq!(code(&f), "INVALID_OPERATION", "{f}");
    // but may edit what a definition shows without touching them
    let rev = ws.store.entities["d-board"].key_rev("title");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "d-board", "keys": [{ "key": "title", "value": "Board", "expect": { "rev": rev } }] }]));
    // the dependency is strong: the card cannot go while the board builds on it
    let life = ws.store.entities["d-card"].life_rev;
    let f = ws.fail(json!([{ "op": "entity.delete", "entity_id": "d-card", "expect": { "rev": life } }]));
    assert_eq!(code(&f), "REFERENCE_BROKEN", "{f}");
    assert_eq!(f["errors"][0]["data"]["referrers"][0]["kind"], "def_dep");
    // shapes and targets
    for (deps, expected) in [
        (json!([{ "ident": "did:bns:alice#card", "entity_id": "nope" }]), "REFERENCE_BROKEN"),
        (json!([{ "ident": "card", "entity_id": "d-card" }]), "INVALID_SCHEMA"),
        (json!([{ "ident": "did:bns:alice#card", "entity_id": "s" }]), "INVALID_SCHEMA"),
        (json!([{ "ident": "did:bns:alice#card", "entity_id": "d-card" }, { "ident": "did:bns:alice#card", "entity_id": "d-card" }]), "INVALID_SCHEMA"),
    ] {
        let f = as_build(&mut ws, json!([def("d-y", "y", "c", "composite", json!({ "deps": deps.clone() }))])).unwrap_err();
        assert_eq!(code(&f), expected, "{deps}: {f}");
    }
    // card → board would close board → card
    let rev = ws.store.entities["d-card"].key_rev("deps");
    let f = as_build(&mut ws, json!([{ "op": "entity.set_keys", "entity_id": "d-card", "keys": [{ "key": "deps", "value": [{ "ident": "did:bns:alice#board", "entity_id": "d-board" }], "expect": { "rev": rev } }] }])).unwrap_err();
    assert_eq!(code(&f), "REFERENCE_CYCLE", "{f}");
    assert_eq!(f["errors"][0]["data"]["cycle"], json!(["d-card", "d-board", "d-card"]));
    let rev = ws.store.entities["d-board"].key_rev("deps");
    let f = as_build(&mut ws, json!([{ "op": "entity.set_keys", "entity_id": "d-board", "keys": [{ "key": "deps", "value": [{ "ident": "did:bns:alice#board", "entity_id": "d-board" }], "expect": { "rev": rev } }] }])).unwrap_err();
    assert_eq!(code(&f), "REFERENCE_CYCLE", "{f}");
}

#[test]
fn a_composite_instance_names_its_definition_and_slots_its_children() {
    let mut ws = setup();
    ws.ok(json!([board("b1", json!({})), card("c1", "b1", "a", Some("cards"), laid_out()), card("c2", "b1", "b", Some("cards"), laid_out())]));
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    let b1 = outline.iter().find(|e| e["entity_id"] == "b1").unwrap();
    assert_eq!((b1["kind"].as_str(), b1["layout"].clone(), b1["def_id"].as_str()), (Some("composite"), json!({ "mode": "def" }), Some("d-board")));
    assert_eq!(outline.iter().find(|e| e["entity_id"] == "c2").unwrap()["slot"], "cards", "a slot holds several Blocks");
    // the definition is a strong reference of the instance
    let life = ws.store.entities["d-board"].life_rev;
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": "d-board", "expect": { "rev": life } }]))), "REFERENCE_BROKEN");
    // children carry a slot of the definition
    assert_eq!(code(&ws.fail(json!([card("c3", "b1", "c", None, laid_out())]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([card("c3", "b1", "c", Some("footer"), laid_out())]))), "INVALID_SCHEMA");
    // a group fills a slot too; a Cell in a composite stays a leaf
    ws.ok(json!([{ "op": "entity.create", "entity_id": "g1", "type_id": TYPE_CONTAINER, "parent_id": "b1", "order_key": "c", "placement": { "x": "auto", "y": "auto", "w": 100, "h": 100 },
                   "payload": { "kind": "group", "slot": "header" } }]));
    assert_eq!(sub(&ws.fail(json!([card("c4", "c1", "a", Some("cards"), laid_out())]))), "CHILD_NOT_ALLOWED");
    // the instance's definition is fixed, and must be a composite one
    let rev = ws.store.entities["b1"].key_rev("def_ref");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "b1", "keys": [{ "key": "def_ref", "value": { "entity_id": "d-card" }, "expect": { "rev": rev } }] }]))), "INVALID_OPERATION");
    let mut wrong = board("b2", json!({}));
    wrong["payload"]["def_ref"] = json!({ "entity_id": "d-card" });
    assert_eq!(code(&ws.fail(json!([wrong]))), "INVALID_SCHEMA");
    let mut none = board("b2", json!({}));
    none["payload"].as_object_mut().unwrap().remove("def_ref");
    assert_eq!(code(&ws.fail(json!([none]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([board("b2", json!({ "layout": { "mode": "free" } }))]))), "INVALID_SCHEMA");
    // only a composite names a definition
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "g2", "type_id": TYPE_CONTAINER, "parent_id": "s", "order_key": "z",
                                       "payload": { "kind": "group", "def_ref": { "entity_id": "d-board" } } }]))), "INVALID_SCHEMA");
    // moving a Block in or out rewrites its slot in the same step
    ws.ok(json!([card("free", "s", "x", None, json!({ "x": 0, "y": 0, "w": 200, "h": 80 }))]));
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.move", "entity_id": "free", "new_parent_id": "b1", "order_key": "d", "placement": laid_out() }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "tree.move", "entity_id": "free", "new_parent_id": "b1", "order_key": "d", "placement": laid_out(), "slot": "cards" }]));
    assert_eq!(ws.store.entities["free"].payload["slot"], "cards");
    let seq = ws.ok(json!([{ "op": "tree.move", "entity_id": "c1", "new_parent_id": "s", "order_key": "y", "placement": { "x": 0, "y": 0, "w": 200, "h": 80 }, "slot": null }]));
    assert!(ws.store.entities["c1"].payload.get("slot").is_none());
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(ws.store.entities["c1"].payload["slot"], "cards", "undo puts the slot back with the place");
    assert_eq!(ws.store.edges["c1"].parent_id, "b1");
}

#[test]
fn a_fixed_composite_never_lays_its_children_out() {
    let mut ws = setup();
    let mut fixed = def("d-fixed", "fixed", "c", "composite", json!({}));
    fixed["payload"]["composition"] = json!({ "layout": { "algorithm": "fixed" }, "slots": { "area": { "placement": "free" } } });
    ws.ok(json!([fixed]));
    let mut b = board("bf", json!({}));
    b["payload"]["def_ref"] = json!({ "entity_id": "d-fixed" });
    ws.ok(json!([b]));
    assert_eq!(code(&ws.fail(json!([card("f1", "bf", "a", Some("area"), laid_out())]))), "INVALID_SCHEMA");
    ws.ok(json!([card("f1", "bf", "a", Some("area"), json!({ "x": 4, "y": 4, "w": 200, "h": 80 }))]));
}

#[test]
fn replay_runs_no_business_schema() {
    // an operation the Schema refuses (a Viewport out of range) but whose primitives hold
    let mut ws = setup();
    let ops = json!([{ "op": "entity.create", "entity_id": "vp", "type_id": TYPE_VIEWPORT, "parent_id": SHOWS_ID, "order_key": "a",
                       "payload": { "surface_ref": { "entity_id": "s" }, "center": { "x": 0, "y": 0 }, "zoom": 99 } },
                     board("b1", json!({ "config": { "gap": "not a number" } })), card("c1", "b1", "a", Some("cards"), laid_out())]);
    assert_eq!(code(&ws.fail(ops.clone())), "INVALID_SCHEMA", "admission judges the Schema");
    let replay = PlanEnv { schema_checks: false, ..ws.env("alice", "human", None, false) };
    let access = Access::full("alice");
    let before = schema_validations();
    let req = MemWorkspace::request(ops);
    let plan = plan_commit(&ws.store, &req, &replay, &access).map_err(|f| f.to_json()).unwrap();
    assert_eq!(schema_validations(), before, "replay never enters validate_schema");
    assert!(plan.overlay.into_changes().entities.contains_key("vp"));
    // primitives and references still hold there: a composite child without a slot is refused on replay too
    let bad = MemWorkspace::request(json!([board("b2", json!({})), card("c9", "b2", "a", None, laid_out())]));
    assert!(plan_commit(&ws.store, &bad, &replay, &access).is_err());
    assert_eq!(schema_validations(), before);
    // admission does enter it
    ws.ok(json!([board("b3", json!({}))]));
    assert!(schema_validations() > before);
}
