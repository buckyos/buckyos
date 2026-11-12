//! Bindings that name several data (notepads/aiworkspace-control-on-bind-todo.md §2.3, Q2): a binding declared `many`
//! holds a list of references; each item is a `bind` edge, data created with the control is owned by it, the outline
//! lists the ids, a copy remaps every item. A definition declares `schema_id` and `many` on its bindings.

use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::types::check_def_bindings;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn calendar_table(id: &str, parent: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_TABLE, "schema_id": "buckyos.calendar@1", "parent_id": parent, "order_key": format!("t{id}"),
            "payload": { "title_field_id": "title", "fields": [
                { "field_id": "title", "name": "Title", "type": "text" }, { "field_id": "all_day", "name": "All day", "type": "boolean" },
                { "field_id": "start", "name": "Start", "type": "datetime" }, { "field_id": "end", "name": "End", "type": "datetime" },
                { "field_id": "start_date", "name": "Start date", "type": "date" }, { "field_id": "end_date", "name": "End date", "type": "date" },
                { "field_id": "status", "name": "Status", "type": "select", "options": [] }, { "field_id": "ext", "name": "Ext", "type": "json" } ] } })
}

fn setup() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "s-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s" } },
        { "op": "entity.create", "entity_id": "s", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s-content" } },
        calendar_table("home", DATA_ID),
    ]));
    ws
}

fn calendar(id: &str, bindings: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": "s", "order_key": format!("c{id}"),
            "placement": { "x": 0, "y": 0, "w": 600, "h": 400 }, "payload": { "kind": "calendar", "layout": { "mode": "data" }, "bindings": bindings } })
}

fn refs(ws: &MemWorkspace, src: &str, kind: &str) -> Vec<String> {
    let mut out: Vec<String> = ws.store.refs.iter().filter(|r| r.src_entity_id == src && r.kind == kind).map(|r| r.dst_entity_id.clone()).collect();
    out.sort();
    out
}

#[test]
fn a_many_binding_holds_a_list_each_item_bound_the_new_ones_owned() {
    let mut ws = setup();
    // a calendar created with a new calendar table, also showing one of the data tree
    ws.ok(json!([calendar_table("work", "s-content"), calendar("cal", json!({ "events": [{ "entity_id": "work" }, { "entity_id": "home" }] }))]));
    assert_eq!(refs(&ws, "cal", "bind"), ["home", "work"]);
    assert_eq!(refs(&ws, "cal", "owns"), ["work"], "only what was created with it, on the canvas");
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    let env = outline.iter().find(|e| e["entity_id"] == "cal").unwrap();
    assert_eq!(env["bindings"], json!({ "events": ["work", "home"] }), "the outline lists the ids in order");

    // per-binding writes replace the list; an empty list unbinds
    ws.ok(json!([{ "op": "entity.set_props", "entity_id": "cal", "key": "bindings", "props": { "events": [{ "entity_id": "home" }] } }]));
    assert_eq!(refs(&ws, "cal", "bind"), ["home"]);
    ws.ok(json!([{ "op": "entity.set_props", "entity_id": "cal", "key": "bindings", "props": { "events": [] } }]));
    assert!(ws.store.entity("cal").unwrap().unwrap().payload.get("bindings").and_then(|b| b.get("events")).is_none());
}

#[test]
fn a_list_is_checked_item_by_item() {
    let mut ws = setup();
    let twice = ws.fail(json!([calendar("c1", json!({ "events": [{ "entity_id": "home" }, { "entity_id": "home" }] }))]));
    assert_eq!(code(&twice), "INVALID_SCHEMA", "{twice}");
    let missing = ws.fail(json!([calendar("c2", json!({ "events": [{ "entity_id": "home" }, { "entity_id": "nowhere" }] }))]));
    assert_ne!(code(&missing), "", "{missing}");
    let items: Vec<Value> = (0..33).map(|i| json!({ "entity_id": format!("t{i}") })).collect();
    let many = ws.fail(json!([calendar("c3", json!({ "events": items }))]));
    assert_eq!(code(&many), "LIMIT_EXCEEDED", "{many}");
    let shape = ws.fail(json!([calendar("c4", json!({ "events": ["home"] }))]));
    assert_eq!(code(&shape), "INVALID_SCHEMA", "{shape}");
}

#[test]
fn a_copy_remaps_every_item_of_a_list() {
    let mut ws = setup();
    ws.ok(json!([calendar_table("work", "s-content"), calendar("cal", json!({ "events": [{ "entity_id": "work" }, { "entity_id": "home" }] }))]));
    ws.ok(json!([{ "op": "tree.clone", "source_id": "cal", "parent_id": "s", "content": "private", "id_seed": "x" }]));
    let copies: Vec<_> = ws.store.entities.values().filter(|e| e.alive() && e.payload.get("kind") == Some(&json!("calendar")) && e.entity_id != "cal").collect();
    assert_eq!(copies.len(), 1);
    let events = copies[0].payload["bindings"]["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_ne!(events[0]["entity_id"], json!("work"), "the owned table was copied with it: its copy is bound");
    assert_eq!(events[1]["entity_id"], json!("home"), "data of the data tree stays shared");
}

#[test]
fn a_definition_declares_schema_id_and_many() {
    assert!(check_def_bindings(&json!({ "events": { "accepts": ["buckyos.table-source"], "schema_id": "buckyos.calendar@1", "required": true, "many": true } })).is_ok());
    assert!(check_def_bindings(&json!({ "events": { "many": "yes" } })).is_err());
    assert!(check_def_bindings(&json!({ "events": { "schema_id": "" } })).is_err());
    assert!(check_def_bindings(&json!({ "events": { "multiple": true } })).is_err());
}
