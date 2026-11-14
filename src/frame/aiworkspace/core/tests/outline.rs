//! The outline of a caller who reads only part of the tree (UI 框架 PRD §4.1.1 Q5, §18.3): what it may read below an
//! entity it may not still hangs in the two trees, under that entity as a path node — id, type, a container's kind,
//! parent and order, nothing else.

use aiworkspace_core::access::{Access, Applied, Cap, CapSet, Tier};
use aiworkspace_core::model::*;
use aiworkspace_core::read::outline;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// `data/销售` (a table, a rich text), `surfaces/项目A` (canvas s1 with a group, canvas folder 子目录 with s2),
/// `surfaces/私人` (canvases s3, s4).
fn ws() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    let folder = |id: &str, parent: &str, key: &str, title: &str| json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": parent, "order_key": key, "payload": { "kind": "folder", "title": title } });
    let canvas = |id: &str, parent: &str, key: &str, title: &str| {
        vec![
            json!({ "op": "entity.create", "entity_id": format!("{id}-content"), "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": key, "payload": { "kind": "folder", "system": "surface_content", "surface_id": id, "title": title } }),
            json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": parent, "order_key": key, "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": format!("{id}-content"), "title": title } }),
            json!({ "op": "entity.create", "entity_id": format!("{id}-b"), "type_id": TYPE_CELL, "parent_id": id, "order_key": "a", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "label" }, "title": format!("{title} 的标签") } }),
        ]
    };
    let mut ops = vec![
        folder("sales", DATA_ID, "a", "销售"),
        json!({ "op": "entity.create", "entity_id": "orders", "type_id": TYPE_TABLE, "parent_id": "sales", "order_key": "a", "name": "订单", "payload": { "fields": [{ "field_id": "c", "name": "客户", "type": "text" }] } }),
        json!({ "op": "entity.create", "entity_id": "doc", "type_id": TYPE_RICHTEXT, "parent_id": DATA_ID, "order_key": "b", "payload": {} }),
        folder("f1", SURFACES_ID, "a", "项目A"),
        folder("f2", "f1", "b", "子目录"),
        folder("f3", SURFACES_ID, "b", "私人"),
    ];
    ops.extend(canvas("s1", "f1", "a", "看板"));
    ops.push(json!({ "op": "entity.create", "entity_id": "g1", "type_id": TYPE_CONTAINER, "parent_id": "s1", "order_key": "b", "placement": { "x": 20, "y": 0, "w": 30, "h": 30 }, "payload": { "kind": "group", "title": "分组" } }));
    ops.push(json!({ "op": "entity.create", "entity_id": "g1-b", "type_id": TYPE_CELL, "parent_id": "g1", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "label" }, "title": "组里" } }));
    ops.extend(canvas("s2", "f2", "b", "周报"));
    ops.extend(canvas("s3", "f3", "c", "日记"));
    ops.extend(canvas("s4", "f3", "d", "账本"));
    ws.ok(Value::Array(ops));
    ws
}

fn read() -> CapSet {
    CapSet::of(Cap::Read)
}

fn single(subject: &str, allow: CapSet, deny: CapSet) -> Applied {
    Applied { subject: subject.into(), tier: Tier::Single, creator: false, allow, deny }
}

/// `(entity_id, is a path node)` in outline order.
fn rows(ws: &MemWorkspace, access: &Access) -> Vec<(String, bool)> {
    outline(&ws.store, access).unwrap().iter().map(|e| (e["entity_id"].as_str().unwrap().to_string(), e["path_only"] == json!(true))).collect()
}

fn ids(rows: &[(String, bool)]) -> Vec<&str> {
    rows.iter().map(|(id, _)| id.as_str()).collect()
}

fn path_nodes(rows: &[(String, bool)]) -> Vec<&str> {
    rows.iter().filter(|(_, p)| *p).map(|(id, _)| id.as_str()).collect()
}

#[test]
fn a_grant_on_a_canvas_folder_hangs_under_path_nodes() {
    let ws = ws();
    let bob = Access::granted("bob", CapSet::NONE, [("f1".to_string(), read())]);
    let r = rows(&ws, &bob);
    assert_eq!(ids(&r), ["root", SURFACES_ID, "f1", "s1", "s1-b", "g1", "g1-b", "f2", "s2", "s2-b"], "tree order, path nodes first");
    assert_eq!(path_nodes(&r), ["root", SURFACES_ID]);
    // a path node places itself and says nothing more: no name, title, content, capabilities or revisions
    let out = outline(&ws.store, &bob).unwrap();
    let surfaces = out.iter().find(|e| e["entity_id"] == SURFACES_ID).unwrap();
    assert_eq!(surfaces, &json!({ "entity_id": SURFACES_ID, "type_id": TYPE_CONTAINER, "kind": "surfaces", "parent_id": ROOT_ID, "order_key": "b", "path_only": true }));
    let root = out.iter().find(|e| e["entity_id"] == ROOT_ID).unwrap();
    assert_eq!(root, &json!({ "entity_id": ROOT_ID, "type_id": TYPE_CONTAINER, "kind": "root", "path_only": true }));
    // what is readable is the usual envelope
    let f1 = out.iter().find(|e| e["entity_id"] == "f1").unwrap();
    assert_eq!((f1["title"].as_str(), f1["name"].as_str(), f1["capabilities"].clone()), (Some("项目A"), Some("项目A"), json!(["read"])));
    assert!(f1.get("path_only").is_none());
}

#[test]
fn a_grant_on_one_canvas_names_no_folder() {
    let ws = ws();
    let bob = Access::granted("bob", CapSet::NONE, [("s1".to_string(), read())]);
    let r = rows(&ws, &bob);
    assert_eq!(ids(&r), ["root", SURFACES_ID, "f1", "s1", "s1-b", "g1", "g1-b"], "子目录 holds nothing bob reads");
    assert_eq!(path_nodes(&r), ["root", SURFACES_ID, "f1"]);
    let out = outline(&ws.store, &bob).unwrap();
    let f1 = out.iter().find(|e| e["entity_id"] == "f1").unwrap();
    assert_eq!(f1, &json!({ "entity_id": "f1", "type_id": TYPE_CONTAINER, "kind": "folder", "parent_id": SURFACES_ID, "order_key": "a", "path_only": true }));
    // the canvas's own data folder is not covered by a grant on the canvas
    assert!(!ids(&r).contains(&"s1-content"));
    // one Block of a group: the canvas and the group are path nodes
    let one = Access::granted("bob", CapSet::NONE, [("g1-b".to_string(), read())]);
    let r = rows(&ws, &one);
    assert_eq!(ids(&r), ["root", SURFACES_ID, "f1", "s1", "g1", "g1-b"]);
    assert_eq!(path_nodes(&r), ["root", SURFACES_ID, "f1", "s1", "g1"]);
}

#[test]
fn a_grant_on_a_data_folder_and_on_a_record_body() {
    let ws = ws();
    let bob = Access::granted("bob", CapSet::NONE, [("sales".to_string(), read())]);
    let r = rows(&ws, &bob);
    assert_eq!(ids(&r), ["root", DATA_ID, "sales", "orders"]);
    assert_eq!(path_nodes(&r), ["root", DATA_ID]);
    // a rich text under a table (a record's body): the table is a path node, without a kind
    let mut ws = ws;
    ws.ok(json!([{ "op": "entity.create", "entity_id": "body", "type_id": TYPE_RICHTEXT, "parent_id": "orders", "order_key": "a", "payload": {} }]));
    let body = Access::granted("bob", CapSet::NONE, [("body".to_string(), read())]);
    let out = outline(&ws.store, &body).unwrap();
    assert_eq!(out.iter().map(|e| e["entity_id"].as_str().unwrap()).collect::<Vec<_>>(), ["root", DATA_ID, "sales", "orders", "body"]);
    assert_eq!(out[3], json!({ "entity_id": "orders", "type_id": TYPE_TABLE, "parent_id": "sales", "order_key": "a", "path_only": true }));
}

#[test]
fn a_denied_ancestor_is_a_path_node_only_above_what_is_allowed() {
    let ws = ws();
    // bob reads the Workspace, not 私人 — except the canvas 日记
    let layers = BTreeMap::from([
        (String::new(), vec![single("bob", read(), CapSet::NONE)]),
        ("f3".to_string(), vec![single("bob", CapSet::NONE, read())]),
        ("s3".to_string(), vec![single("bob", read(), CapSet::NONE)]),
    ]);
    let bob = Access::from_layers("bob", true, layers);
    let r = rows(&ws, &bob);
    assert_eq!(path_nodes(&r), ["f3"]);
    let at = |id: &str| ids(&r).iter().position(|x| *x == id).unwrap();
    assert!(at("f3") < at("s3") && at("s3") < at("s3-b"));
    assert!(!ids(&r).contains(&"s4") && !ids(&r).contains(&"s4-b"), "the rest of 私人 stays out");
    // nothing allowed below a denial: the denied folder is not listed at all
    let layers = BTreeMap::from([(String::new(), vec![single("bob", read(), CapSet::NONE)]), ("f3".to_string(), vec![single("bob", CapSet::NONE, read())])]);
    let r = rows(&ws, &Access::from_layers("bob", true, layers));
    assert!(path_nodes(&r).is_empty() && !ids(&r).contains(&"f3") && ids(&r).contains(&"s1"));
}

#[test]
fn creator_entries_and_full_access() {
    let mut ws = ws();
    // `@creator` may allow anywhere: no shortcut, still only what is readable below decides
    ws.store.entities.get_mut("s4").unwrap().created_by = Some("bob".into());
    let creator = Applied { subject: "@creator".into(), tier: Tier::Group, creator: true, allow: read(), deny: CapSet::NONE };
    let bob = Access::from_layers("bob", true, BTreeMap::from([(String::new(), vec![creator])]));
    let r = rows(&ws, &bob);
    assert_eq!(ids(&r), ["root", SURFACES_ID, "f3", "s4"]);
    assert_eq!(path_nodes(&r), ["root", SURFACES_ID, "f3"]);
    // nothing readable at all: nothing
    assert!(rows(&ws, &Access::granted("bob", CapSet::NONE, [])).is_empty());
    // the owner: every entity, no path node
    let all = rows(&ws, &Access::full("alice"));
    assert!(path_nodes(&all).is_empty() && all.len() == ws.store.entities.len());
}
