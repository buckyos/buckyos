//! `tree.clone` (扩展控件 P1, Q17): a Block subtree copied as one operation — new ids from the seed, own data cloned
//! and rebound, shared data kept shared, connectors copied only when both ends are inside, rich text forked as a new
//! lineage, the same result planned by the service and by an offline replica, undone as one.

use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::plan::{clone_id, plan_commit};
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn para(id: &str, text: &str) -> Value {
    json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] })
}

fn block(id: &str, parent: &str, key: &str, x: f64, payload: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": key,
            "placement": { "x": x, "y": 0, "w": 100, "h": 50 }, "payload": payload })
}

/// Surface `s`: group `g` holding a note with its own rich text, a table view of shared data, and a connector
/// between the two; a connector from the note to a Block outside the group.
fn setup() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "s-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s" } },
        { "op": "entity.create", "entity_id": "s", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s-content" } },
        { "op": "entity.create", "entity_id": "shared", "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": "a", "name": "shared",
          "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }] } },
    ]));
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "g", "type_id": TYPE_CONTAINER, "parent_id": "s", "order_key": "a",
          "placement": { "x": 0, "y": 0, "w": 400, "h": 200 }, "payload": { "kind": "group", "title": "G" } },
        { "op": "entity.create", "entity_id": "note-doc", "type_id": TYPE_RICHTEXT, "parent_id": "s-content", "order_key": "a",
          "payload": { "content": { "type": "doc", "content": [para("p1", "原文")] } } },
        block("note", "g", "a", 0.0, json!({ "view": { "type": "richtext" }, "source_ref": { "entity_id": "note-doc" } })),
        block("tbl", "g", "b", 200.0, json!({ "view": { "type": "table" }, "source_ref": { "entity_id": "shared" } })),
        block("out", "s", "b", 600.0, json!({ "view": { "type": "shape" }, "title": "outside" })),
    ]));
    let end = |id: &str| json!({ "entity_id": id, "anchor": { "kind": "named", "id": "e" } });
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "line-in", "type_id": TYPE_CELL, "parent_id": "g", "order_key": "c", "placement": { "x": 100, "y": 25, "w": 100, "h": 1 },
          "payload": { "view": { "type": "connector" }, "start": end("note"), "end": end("tbl") } },
        { "op": "entity.create", "entity_id": "line-out", "type_id": TYPE_CELL, "parent_id": "g", "order_key": "d", "placement": { "x": 100, "y": 25, "w": 500, "h": 1 },
          "payload": { "view": { "type": "connector" }, "start": end("note"), "end": end("out") } },
    ]));
    ws
}

fn clone_op(seed: &str) -> Value {
    json!([{ "op": "tree.clone", "source_id": "g", "parent_id": "s", "placement": { "x": 0, "y": 300, "w": 400, "h": 200 }, "id_seed": seed }])
}

#[test]
fn a_subtree_is_copied_with_its_own_data_and_inner_lines() {
    let mut ws = setup();
    let seq = ws.ok(clone_op("t1"));
    let id = |old: &str| clone_id("t1", old);
    let (g2, note2, tbl2, doc2) = (id("g"), id("note"), id("tbl"), id("note-doc"));
    // the structure, with the root at its new place
    assert_eq!(ws.store.edges[&g2].parent_id, "s");
    assert_eq!(ws.store.edges[&g2].placement, Some(json!({ "x": 0, "y": 300, "w": 400, "h": 200 })));
    assert_eq!((ws.store.edges[&note2].parent_id.as_str(), ws.store.edges[&tbl2].parent_id.as_str()), (g2.as_str(), g2.as_str()));
    assert_eq!(ws.store.edges[&note2].placement, ws.store.edges["note"].placement);
    // own data is cloned into the Surface's content folder, rebound and owned by the copy; shared data stays shared
    assert_eq!(ws.store.entities[&note2].payload["source_ref"]["entity_id"], doc2.as_str());
    assert_eq!(ws.store.edges[&doc2].parent_id, "s-content");
    assert!(ws.store.refs.iter().any(|r| r.kind == "owns" && r.src_entity_id == note2 && r.dst_entity_id == doc2));
    assert_eq!(ws.store.entities[&tbl2].payload["source_ref"]["entity_id"], "shared");
    assert!(!ws.store.entities.contains_key(&id("shared")));
    // the copy's rich text is its own lineage: editing it leaves the original as it was
    assert_ne!(ws.store.richtexts[&doc2].meta.lineage_id, ws.store.richtexts["note-doc"].meta.lineage_id);
    let hash = ws.store.richtexts[&doc2].meta.block_index["p1"].hash.clone();
    ws.ok(json!([{ "op": "richtext.replace_block", "entity_id": doc2, "block_id": "p1", "node": para("p1", "改过"), "expect": { "hash": hash } }]));
    assert_eq!(ws.store.richtexts["note-doc"].meta.ast["content"][0]["content"][0]["text"], "原文");
    // the inner line follows the copies, the one crossing the border is left out
    let line2 = id("line-in");
    assert_eq!(ws.store.entities[&line2].payload["start"]["entity_id"], note2.as_str());
    assert_eq!(ws.store.entities[&line2].payload["end"]["entity_id"], tbl2.as_str());
    assert!(!ws.store.entities.contains_key(&id("line-out")));
    // the history records the ids
    let recorded = &ws.history[(seq - 1) as usize][0].op;
    assert_eq!(recorded["new_ids"]["g"], g2.as_str());
    assert_eq!(recorded["new_ids"]["note-doc"], doc2.as_str());
    // undone as one: the copies and their own data go, the originals stay
    let seq = seq;
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    for gone in [&g2, &note2, &tbl2, &doc2, &line2] {
        assert!(!ws.store.entities[gone].alive(), "{gone}");
    }
    assert!(ws.store.entities["note-doc"].alive() && ws.store.entities["g"].alive());
}

#[test]
fn the_same_request_plans_the_same_copy_everywhere() {
    // the service and an offline replica start from the same state and get the same rows
    let (a, b) = (setup(), setup());
    let req = MemWorkspace::request(clone_op("seed-1"));
    let env = a.env("alice", "human", None, false);
    let access = Access::full("alice");
    let plan = |ws: &MemWorkspace| {
        let p = plan_commit(&ws.store, &req, &env, &access).map_err(|f| f.to_json()).unwrap();
        let op = p.ops[0].op.clone();
        let mut ids: Vec<String> = p.overlay.into_changes().entities.keys().cloned().collect();
        ids.sort();
        (op, ids)
    };
    assert_eq!(plan(&a), plan(&b));
    // without a seed the ids derive from the Commit's seq and the op's index
    let mut c = setup();
    let seq = c.ok(json!([{ "op": "tree.clone", "source_id": "note", "parent_id": "s" }]));
    assert!(c.store.entities.contains_key(&clone_id(&format!("{seq}.0"), "note")));
}

#[test]
fn clones_land_where_their_parent_allows() {
    let mut ws = setup();
    // into a composite the root needs a slot
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "d-board", "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": "z", "name": "did:bns:alice#board",
          "payload": { "def_id": "board", "kind": "composite", "title": "Board", "manifest_version": 2, "ident": { "author_did": "did:bns:alice", "name": "board" },
                       "composition": { "layout": { "algorithm": "stack" }, "slots": { "cards": {} } } } },
        { "op": "entity.create", "entity_id": "b1", "type_id": TYPE_CONTAINER, "parent_id": "s", "order_key": "z", "placement": { "x": 0, "y": 600, "w": "auto", "h": "auto" },
          "payload": { "kind": "composite", "def_ref": { "entity_id": "d-board" } } },
    ]));
    let into = |slot: Value| json!([{ "op": "tree.clone", "source_id": "out", "parent_id": "b1", "placement": { "x": "auto", "y": "auto", "w": 100, "h": 50 }, "slot": slot, "id_seed": "b" }]);
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.clone", "source_id": "out", "parent_id": "b1", "placement": { "x": "auto", "y": "auto", "w": 100, "h": 50 }, "id_seed": "b" }]))), "INVALID_SCHEMA");
    ws.ok(into(json!("cards")));
    // a copy of the composite keeps sharing its definition and copies its children
    let seq = ws.ok(json!([{ "op": "tree.clone", "source_id": "b1", "parent_id": "s", "placement": { "x": 500, "y": 600, "w": "auto", "h": "auto" }, "id_seed": "c" }]));
    let b2 = clone_id("c", "b1");
    assert_eq!(ws.store.entities[&b2].payload["def_ref"]["entity_id"], "d-board");
    let child = clone_id("c", &clone_id("b", "out"));
    assert_eq!(ws.store.edges[&child].parent_id, b2);
    assert_eq!(ws.store.entities[&child].payload["slot"], "cards");
    assert!(seq > 0);
    // what cannot be copied this way
    for (source, parent) in [("s", "s"), ("shared", "s"), ("note", DATA_ID)] {
        let f = ws.fail(json!([{ "op": "tree.clone", "source_id": source, "parent_id": parent, "id_seed": "x" }]));
        assert!(matches!(code(&f), "INVALID_OPERATION"), "{source} → {parent}: {f}");
    }
    // the same seed twice collides
    let f = ws.fail(into(json!("cards")));
    assert_eq!(f["errors"][0]["sub_code"], "ID_CONFLICT", "{f}");
}
