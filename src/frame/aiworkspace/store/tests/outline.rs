//! `doc.outline` for a caller granted only part of the Workspace (UI 框架 PRD §4.1.1 Q5, §15 #4, §18.3): a grant on a
//! canvas folder, on one canvas, on `/data/销售`, and a denial on an ancestor. What it may read hangs under its unreadable
//! ancestors as path nodes, which give no name and stay unreadable everywhere else.

mod common;
use aiworkspace_core::Code;
use aiworkspace_store::workspace::{Caller, Workspace};
use common::*;
use serde_json::{json, Value};

fn set(ws: &mut Workspace, changes: Value) {
    ws.acl_set(&alice(), &changes).unwrap();
}

/// `(entity_id, path node)` of `who`'s outline, in order.
fn outline(ws: &Workspace, who: &Caller) -> Vec<(String, bool)> {
    ws.outline(who).unwrap()["entities"].as_array().unwrap().iter().map(|e| (e["entity_id"].as_str().unwrap().to_string(), e["path_only"] == json!(true))).collect()
}

fn has(rows: &[(String, bool)], id: &str) -> Option<bool> {
    rows.iter().find(|(x, _)| x == id).map(|(_, p)| *p)
}

/// The project fixture plus canvas folders `/surfaces/项目A/子目录` (canvases 看板, 周报) and `/surfaces/私人` (日记,
/// 账本), and `/data/销售` holding a table.
fn workspace(env: &Env) -> String {
    let id = project(env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let folder = |id: &str, parent: &str, key: &str, title: &str| json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.container", "parent_id": parent, "order_key": key, "payload": { "kind": "folder", "title": title } });
    let mut ops = vec![
        folder("fa", "surfaces", "c", "项目A"),
        folder("fsub", "fa", "b", "子目录"),
        folder("fp", "surfaces", "d", "私人"),
        folder("sales", "data", "s", "销售"),
        json!({ "op": "entity.create", "entity_id": "sales-t", "type_id": "buckyos.table-source", "parent_id": "sales", "order_key": "a", "name": "订单", "payload": { "fields": [{ "field_id": "c", "name": "客户", "type": "text" }] } }),
    ];
    for (id, parent, key, title) in [("sa", "fa", "a", "看板"), ("sb", "fsub", "a", "周报"), ("sc", "fp", "a", "日记"), ("sd", "fp", "b", "账本")] {
        ops.push(json!({ "op": "entity.create", "entity_id": format!("{id}-content"), "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": format!("m{key}{id}"), "payload": { "kind": "folder", "system": "surface_content", "surface_id": id, "title": title } }));
        ops.push(json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.container", "parent_id": parent, "order_key": key, "payload": { "kind": "surface", "layout": { "mode": "flow" }, "content_folder_id": format!("{id}-content"), "title": title } }));
    }
    ok(&mut ws, &alice(), Value::Array(ops));
    id
}

#[test]
fn a_canvas_folder_grant_shows_its_canvases_under_path_nodes() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    // before: nothing (the walk used to stop at the unreadable root)
    set(&mut ws, json!([{ "subject": "bob", "resource": "/surfaces/项目A", "allow": ["read"] }]));
    let rows = outline(&ws, &bob());
    let ids: Vec<&str> = rows.iter().map(|(x, _)| x.as_str()).collect();
    assert_eq!(ids, ["root", "surfaces", "fa", "sa", "fsub", "sb"]);
    assert_eq!(rows.iter().filter(|(_, p)| *p).count(), 2, "root and surfaces only: {rows:?}");
    let entities = ws.outline(&bob()).unwrap()["entities"].clone();
    let surfaces = entities.as_array().unwrap().iter().find(|e| e["entity_id"] == "surfaces").unwrap().clone();
    let mut keys: Vec<&String> = surfaces.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["entity_id", "kind", "order_key", "parent_id", "path_only", "type_id"], "no name, title, capabilities or revisions");
    // the path nodes stay unreadable: reads, listings and paths go on as before
    assert_eq!(ws.read(&bob(), "surfaces", None).unwrap_err().code, Code::NotFound);
    assert_eq!(ws.list(&bob(), &json!({ "at": "/surfaces" })).unwrap_err().code, Code::NotFound);
    assert_eq!(ws.resolve(&bob(), None, Some("/surfaces/项目A")).unwrap_err().code, Code::NotFound);
    assert_eq!(ws.resolve(&bob(), None, Some("~fa/子目录/周报")).unwrap()["path"], json!("~fa/子目录/周报"));
    // moved into the folder (§15 #4): the canvas follows the folder's grant into the outline; moved out, it leaves
    let life = ws.read(&alice(), "sc", None).unwrap()["struct_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "tree.move", "entity_id": "sc", "new_parent_id": "fsub", "order_key": "b", "expect": { "rev": life } }]));
    assert_eq!(has(&outline(&ws, &bob()), "sc"), Some(false));
    let life = ws.read(&alice(), "sc", None).unwrap()["struct_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "tree.move", "entity_id": "sc", "new_parent_id": "fp", "order_key": "a", "expect": { "rev": life } }]));
    assert_eq!(has(&outline(&ws, &bob()), "sc"), None);
    assert_eq!(has(&outline(&ws, &bob()), "fp"), None, "私人 holds nothing bob reads");
}

#[test]
fn one_canvas_and_one_data_folder() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    set(&mut ws, json!([{ "subject": "bob", "resource": "/surfaces/项目A/子目录/周报", "allow": ["read"] }]));
    let rows = outline(&ws, &bob());
    assert_eq!(rows, [("root", true), ("surfaces", true), ("fa", true), ("fsub", true), ("sb", false)].map(|(a, b)| (a.to_string(), b)));
    let fa = ws.outline(&bob()).unwrap()["entities"].as_array().unwrap().iter().find(|e| e["entity_id"] == "fa").unwrap().clone();
    assert!(fa.get("name").is_none() && fa.get("title").is_none(), "an unreadable folder is not named: {fa}");
    // the canvas's path starts at itself, as the outline lets the client print it
    assert_eq!(ws.read_as(&bob(), &json!({ "entity_id": "sb", "as": "meta" })).unwrap()["path"], json!("~sb"));

    let carol = Caller::user("carol");
    set(&mut ws, json!([{ "subject": "carol", "resource": "/data/销售", "allow": ["read"] }]));
    let rows = outline(&ws, &carol);
    assert_eq!(rows, [("root", true), ("data", true), ("sales", false), ("sales-t", false)].map(|(a, b)| (a.to_string(), b)));
}

#[test]
fn a_denied_ancestor_with_an_allowance_below() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    // bob reads the Workspace but not 私人, except its canvas 日记; 项目A is denied with nothing allowed below
    set(&mut ws, json!([
        { "subject": "bob", "resource": "/", "allow": ["read"] },
        { "subject": "@all", "resource": "/surfaces/私人", "deny": ["read"] },
        { "subject": "bob", "resource": "/surfaces/私人/日记", "allow": ["read"] },
        { "subject": "bob", "resource": "/surfaces/项目A", "deny": ["read"] },
    ]));
    let rows = outline(&ws, &bob());
    assert_eq!(rows.iter().filter(|(_, p)| *p).map(|(x, _)| x.as_str()).collect::<Vec<_>>(), ["fp"]);
    assert_eq!((has(&rows, "sc"), has(&rows, "sd")), (Some(false), None));
    assert_eq!((has(&rows, "fa"), has(&rows, "sa"), has(&rows, "fsub")), (None, None, None));
    assert_eq!((has(&rows, "tasks"), has(&rows, "surface-main")), (Some(false), Some(false)));
    let at = |id: &str| rows.iter().position(|(x, _)| x == id).unwrap();
    assert!(at("surfaces") < at("fp") && at("fp") < at("sc"));
}
