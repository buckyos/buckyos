//! Extension controls P2 (notepads/done/aiworkspace-extension-p2p3-todo.md M1, Q28 / Q29): per-property writes of a Block's
//! `config` / `bindings` / `on` (`entity.set_props`, one version cell `p:<key>.<name>` each), the read-set precondition
//! on the definition an instance follows, and the shapes of `on` and of a definition's `events`.

use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::plan::{plan_commit, resolve_cell, PlanEnv};
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::types::schema_validations;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn setup() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "s-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s" } },
        { "op": "entity.create", "entity_id": "s", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s-content" } },
        { "op": "entity.create", "entity_id": "d-card", "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": "a",
          "payload": { "def_id": "card", "kind": "declarative", "title": "Card", "declarative": { "items": [] } } },
        { "op": "entity.create", "entity_id": "t", "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": "b",
          "payload": { "fields": [{ "field_id": "f1", "name": "title", "type": "text" }] } },
        { "op": "entity.create", "entity_id": "c", "type_id": TYPE_CELL, "parent_id": "s", "order_key": "m", "placement": { "x": 0, "y": 0, "w": 200, "h": 100 },
          "payload": { "view": { "type": "card" }, "def_ref": { "entity_id": "d-card" }, "config": { "color": "#000", "text": "a" } } },
    ]));
    ws
}

fn cell(ws: &MemWorkspace, id: &str) -> EntityRow {
    ws.store.entity(id).unwrap().unwrap()
}

fn props(key: &str, props: Value, expect: Value) -> Value {
    json!({ "op": "entity.set_props", "entity_id": "c", "key": key, "props": props, "expect": expect })
}

#[test]
fn two_people_two_properties_both_stay() {
    let mut ws = setup();
    // both start from the same read: neither property was ever written on its own
    let a = ws.ok(json!([props("config", json!({ "color": "#f00" }), json!({ "color": 0 }))]));
    ws.ok(json!([props("config", json!({ "text": "b" }), json!({ "text": 0 }))]));
    let c = cell(&ws, "c");
    assert_eq!(c.payload["config"], json!({ "color": "#f00", "text": "b" }));
    assert_eq!(c.key_rev("p:config.color"), a);
    assert_eq!(resolve_cell(&ws.store, &json!({ "entity_id": "c", "selector": { "kind": "entity_prop", "key": "config", "name": "color" } })).unwrap(), json!(a));
    assert_eq!(resolve_cell(&ws.store, &json!({ "entity_id": "c", "selector": { "kind": "entity_prop", "key": "config", "name": "never" } })).unwrap(), json!(0));
}

#[test]
fn the_same_property_conflicts_per_property() {
    let mut ws = setup();
    let a = ws.ok(json!([props("config", json!({ "color": "#f00" }), json!({ "color": 0 }))]));
    let f = ws.fail(json!([props("config", json!({ "color": "#00f", "text": "z" }), json!({ "color": 0, "text": 0 }))]));
    assert_eq!(f["status"], "conflict");
    assert_eq!(code(&f), "REVISION_CONFLICT");
    let conflicts = f["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{f}");
    assert_eq!(conflicts[0]["selector"], json!({ "kind": "entity_prop", "key": "config", "name": "color" }));
    assert_eq!((conflicts[0]["expected_rev"].clone(), conflicts[0]["current_rev"].clone()), (json!(0), json!(a)));
    assert_eq!(conflicts[0]["current_value"], "#f00");
    // without expect a property merges
    ws.ok(json!([props("config", json!({ "color": "#00f" }), Value::Null)]));
    assert_eq!(cell(&ws, "c").payload["config"]["color"], "#00f");
}

#[test]
fn undo_restores_only_what_it_wrote() {
    let mut ws = setup();
    let a = ws.ok(json!([props("config", json!({ "color": "#f00", "size": 3 }), json!({ "color": 0, "size": 0 }))]));
    ws.ok(json!([props("config", json!({ "text": "b" }), json!({ "text": 0 }))]));
    ws.undo(a).map_err(|f| f.to_json()).unwrap();
    assert_eq!(cell(&ws, "c").payload["config"], json!({ "color": "#000", "text": "b" }));
    // someone changed the same property since: the undo conflicts instead of overwriting them
    let a = ws.ok(json!([props("config", json!({ "color": "#0f0" }), Value::Null)]));
    ws.ok(json!([props("config", json!({ "color": "#fff" }), Value::Null)]));
    assert!(ws.undo(a).is_err());
}

#[test]
fn null_removes_and_an_empty_object_removes_the_key() {
    let mut ws = setup();
    ws.ok(json!([props("config", json!({ "color": null, "text": null }), Value::Null)]));
    let c = cell(&ws, "c");
    assert!(c.payload.get("config").is_none(), "{:?}", c.payload);
    // bindings are normalized and judged as with set_keys
    ws.ok(json!([props("bindings", json!({ "tasks": { "entity_id": "t" } }), json!({ "tasks": 0 }))]));
    assert_eq!(cell(&ws, "c").payload["bindings"]["tasks"]["entity_id"], "t");
    let f = ws.fail(json!([props("bindings", json!({ "other": { "entity_id": "missing" } }), Value::Null)]));
    assert_eq!(code(&f), "REFERENCE_BROKEN", "{f}");
    let f = ws.fail(json!([props("title", json!({ "x": 1 }), Value::Null)]));
    assert_eq!(code(&f), "INVALID_OPERATION");
    let f = ws.fail(json!([props("config", json!({ "a.b": 1 }), Value::Null)]));
    assert_eq!(code(&f), "INVALID_OPERATION");
}

#[test]
fn a_whole_key_write_moves_every_property_cell() {
    let mut ws = setup();
    ws.ok(json!([props("config", json!({ "color": "#f00" }), json!({ "color": 0 }))]));
    let base = cell(&ws, "c").key_rev("p:config.text");
    let rev = cell(&ws, "c").key_rev("config");
    let seq = ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "c", "keys": [{ "key": "config", "value": { "color": "#111", "text": "x", "new": 1 }, "expect": { "rev": rev } }] }]));
    let c = cell(&ws, "c");
    for n in ["color", "text", "new"] {
        assert_eq!(c.key_rev(&format!("p:config.{n}")), seq, "{n}");
    }
    let f = ws.fail(json!([props("config", json!({ "text": "y" }), json!({ "text": base }))]));
    assert_eq!(code(&f), "REVISION_CONFLICT");
}

#[test]
fn an_instance_write_follows_its_definition_revision() {
    let mut ws = setup();
    let def_rev = cell(&ws, "d-card").content_rev;
    let pre = |rev: u64| json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": "ws_test", "epoch": "ep_test", "idempotency_key": "k",
                                 "preconditions": [{ "target": { "entity_id": "d-card" }, "expect": { "rev": rev } }],
                                 "operations": [props("config", json!({ "color": "#f00" }), Value::Null)] });
    let req = aiworkspace_core::plan::CommitRequest::parse(&pre(def_rev)).unwrap();
    let env = ws.env("alice", "human", None, false);
    ws.commit_with(&Access::full("alice"), &env, &req).map_err(|f| f.to_json()).unwrap();
    // the definition is rebuilt meanwhile: the stale form's write is refused and names the definition
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "d-card", "keys": [{ "key": "title", "value": "Card 2", "expect": { "rev": 1 } }] }]));
    let req = aiworkspace_core::plan::CommitRequest::parse(&pre(def_rev)).unwrap();
    let env = ws.env("alice", "human", None, false);
    let f = ws.commit_with(&Access::full("alice"), &env, &req).map_err(|f| f.to_json()).unwrap_err();
    assert_eq!(code(&f), "REVISION_CONFLICT");
    assert_eq!(f["conflicts"][0]["entity_id"], "d-card");
    assert_eq!(f["conflicts"][0]["detail"], "read-set precondition failed");
}

#[test]
fn on_and_events_shapes() {
    let mut ws = setup();
    let on = json!({ "completed": [{ "kind": "write", "target": "source", "what": "recordSet", "args": { "props": { "status": "done" } } }, { "kind": "notify", "text": "{{event.title}}" }] });
    ws.ok(json!([props("on", json!({ "completed": on["completed"] }), json!({ "completed": 0 }))]));
    assert_eq!(cell(&ws, "c").payload["on"], on);
    for bad in [json!({ "Bad": { "kind": "notify" } }), json!({ "x": { "kind": "run" } }), json!({ "x": [] }), json!({ "x": [{ "kind": "notify" }, { "kind": "notify" }, { "kind": "notify" }, { "kind": "notify" }, { "kind": "notify" }] })] {
        let f = ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "c", "keys": [{ "key": "on", "value": bad, "expect": { "rev": cell(&ws, "c").key_rev("on") } }] }]));
        assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
    }
    // only a composite among the containers binds events
    let f = ws.fail(json!([{ "op": "entity.create", "entity_id": "g", "type_id": TYPE_CONTAINER, "parent_id": "s", "order_key": "n",
                              "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "kind": "group", "on": {} } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
    // a definition's events are the build's
    let f = ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "d-card", "keys": [{ "key": "events", "value": { "completed": {} }, "expect": { "rev": 0 } }] }]));
    assert_eq!(code(&f), "INVALID_OPERATION", "{f}");
    let env = ws.env("alice", "build", None, false);
    let req = MemWorkspace::request(json!([{ "op": "entity.set_keys", "entity_id": "d-card", "keys": [{ "key": "events", "value": { "completed": { "title": "Done" } }, "expect": { "rev": 0 } }] }]));
    ws.commit_with(&Access::full("alice"), &env, &req).map_err(|f| f.to_json()).unwrap();
    let env = ws.env("alice", "build", None, false);
    let req = MemWorkspace::request(json!([{ "op": "entity.set_keys", "entity_id": "d-card", "keys": [{ "key": "events", "value": { "Bad": {} }, "expect": { "rev": cell(&ws, "d-card").key_rev("events") } }] }]));
    assert!(ws.commit_with(&Access::full("alice"), &env, &req).is_err());
}

#[test]
fn replay_of_set_props_runs_no_business_schema() {
    let ws = setup();
    let req = MemWorkspace::request(json!([props("config", json!({ "color": "#f00" }), json!({ "color": 0 }))]));
    let replay = PlanEnv { schema_checks: false, ..ws.env("alice", "human", None, false) };
    let before = schema_validations();
    plan_commit(&ws.store, &req, &replay, &Access::full("alice")).map_err(|f| f.to_json()).unwrap();
    assert_eq!(schema_validations(), before, "replay never enters validate_schema");
}
