//! Phase two kernel contracts (M0): two trees, Surfaces with their content folders, placement
//! without `z`, free notes, wishes, Block definitions, dependency records, freshness and relations,
//! and the delete pre-check that names only readable referrers.

use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::freshness::{entity_freshness, relations};
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}
fn sub(v: &Value) -> &str {
    v["sub_code"].as_str().unwrap_or("")
}

fn surface(id: &str, mode: &str) -> Vec<Value> {
    vec![
        json!({ "op": "entity.create", "entity_id": format!("{id}-content"), "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
                "payload": { "kind": "folder", "system": "surface_content", "surface_id": id, "title": id } }),
        json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a", "name": id,
                "payload": { "kind": "surface", "layout": { "mode": mode }, "title": id, "content_folder_id": format!("{id}-content") } }),
    ]
}

fn table(id: &str, parent: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_TABLE, "parent_id": parent, "order_key": "b", "name": id,
            "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }, { "field_id": "n", "name": "数", "type": "number" }] } })
}

fn record(id: &str, parent: &str, owner: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_RECORD, "parent_id": parent, "order_key": "c",
            "payload": { "schema": { "properties": [{ "key": "owner", "name": "负责人", "type": "text" }] }, "props": { "owner": owner } } })
}

fn cell(id: &str, parent: &str, source: Option<&str>, view: &str, placement: Value) -> Value {
    let mut payload = json!({ "view": { "type": view } });
    if let Some(s) = source {
        payload["source_ref"] = json!({ "entity_id": s });
    }
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": "d", "placement": placement, "payload": payload })
}

#[test]
fn every_selector_of_the_frozen_catalogue_resolves_to_a_version_cell() {
    use aiworkspace_core::plan::resolve_cell;
    let mut ws = MemWorkspace::new();
    let mut ops = surface("s1", "free");
    ops.push(table("t1", DATA_ID));
    ops.push(json!({ "op": "table.insert_records", "source_id": "t1", "records": [
        { "record_id": "r1", "values": { "title": "a", "n": 1 } }, { "record_id": "r2", "values": { "title": "b", "n": 2 } }] }));
    let mut view = cell("blk", "s1", Some("t1"), "table", json!({ "x": 0, "y": 0, "w": 100, "h": 100 }));
    view["payload"]["filter"] = json!({ "op": "cmp", "field_id": "n", "operator": "gt", "value": 1 });
    ops.push(view);
    ws.ok(Value::Array(ops));
    let resolve = |ws: &MemWorkspace, id: &str, sel: Value| resolve_cell(&ws.store, &json!({ "entity_id": id, "selector": sel }));
    let cells: Vec<(&str, Value)> = vec![
        ("t1", json!({ "kind": "entity" })),
        ("blk", json!({ "kind": "doc_key", "key": "filter" })),
        ("t1", json!({ "kind": "table_members" })),
        ("t1", json!({ "kind": "table_field", "field_id": "n" })),
        ("t1", json!({ "kind": "table_field_type", "field_id": "n" })),
        ("t1", json!({ "kind": "table_field_values", "field_id": "n" })),
        ("t1", json!({ "kind": "table_record", "record_id": "r1" })),
        ("t1", json!({ "kind": "table_cell", "record_id": "r1", "field_id": "n" })),
        ("s1-content", json!({ "kind": "tree_children" })),
        ("blk", json!({ "kind": "tree_edge" })),
        ("t1", json!({ "kind": "table_view", "cell_id": "blk" })),
        ("t1", json!({ "kind": "table_query", "fields": ["n"] })),
        ("t1", json!({ "kind": "table_query" })),
    ];
    let before: Vec<Value> = cells.iter().map(|(id, sel)| resolve(&ws, id, sel.clone()).unwrap_or_else(|e| panic!("{sel}: {e}"))).collect();
    // a new row changes membership, the view and every query; the view's own keys stay
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "t1", "records": [{ "record_id": "r3", "values": { "title": "c", "n": 3 } }] }]));
    let after: Vec<Value> = cells.iter().map(|(id, sel)| resolve(&ws, id, sel.clone()).unwrap()).collect();
    let changed: Vec<&str> = cells.iter().zip(before.iter().zip(after.iter())).filter(|(_, (b, a))| b != a).map(|((_, s), _)| s["kind"].as_str().unwrap()).collect();
    assert_eq!(changed, ["entity", "table_members", "table_field_values", "table_view", "table_query", "table_query"]);
    // a value of `n` changes the column, its cell, the view and the queries that read `n`
    let rev = ws.store.records[&("t1".to_string(), "r1".to_string())].value_rev("n");
    ws.ok(json!([{ "op": "table.set_values", "source_id": "t1", "values": [{ "record_id": "r1", "field_id": "n", "value": 9, "expect": { "rev": rev } }] }]));
    let again: Vec<Value> = cells.iter().map(|(id, sel)| resolve(&ws, id, sel.clone()).unwrap()).collect();
    let changed: Vec<&str> = cells.iter().zip(after.iter().zip(again.iter())).filter(|(_, (b, a))| b != a).map(|((_, s), _)| s["kind"].as_str().unwrap()).collect();
    assert_eq!(changed, ["entity", "table_field_values", "table_record", "table_cell", "table_view", "table_query", "table_query"]);
    // the view's configuration is part of the view token only
    let frev = ws.store.entities["blk"].key_rev("filter");
    ws.ok(json!([{ "op": "entity.unset_keys", "entity_id": "blk", "keys": [{ "key": "filter", "expect": { "rev": frev } }] }]));
    let third: Vec<Value> = cells.iter().map(|(id, sel)| resolve(&ws, id, sel.clone()).unwrap()).collect();
    let changed: Vec<&str> = cells.iter().zip(again.iter().zip(third.iter())).filter(|(_, (b, a))| b != a).map(|((_, s), _)| s["kind"].as_str().unwrap()).collect();
    assert_eq!(changed, ["doc_key", "table_view"]);
    // a view of another table, a missing field and an unknown kind are refused
    assert!(resolve(&ws, "blk", json!({ "kind": "table_view", "cell_id": "blk" })).is_err());
    assert_eq!(resolve(&ws, "t1", json!({ "kind": "table_query", "fields": ["nope"] })).unwrap_err().code, aiworkspace_core::Code::NotFound);
    assert_eq!(resolve(&ws, "t1", json!({ "kind": "folder_member", "name": "x" })).unwrap_err().code, aiworkspace_core::Code::InvalidOperation, "a table has no members");
    assert_eq!(resolve(&ws, "t1", json!({ "kind": "no_such" })).unwrap_err().code, aiworkspace_core::Code::MissingExtension);
}

#[test]
fn own_data_follows_its_block_and_shared_data_never_does() {
    use aiworkspace_core::testkit::MemWorkspace as W;
    let folder = |id: &str| json!({ "op": "entity.create", "entity_id": format!("{id}-content"), "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
        "payload": { "kind": "folder", "system": "surface_content", "surface_id": id, "title": id } });
    let surf = |id: &str| json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": id, "name": id,
        "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": id, "content_folder_id": format!("{id}-content") } });
    let mut ws = W::new();
    ws.ok(json!([folder("s1"), surf("s1"), folder("s2"), surf("s2"), table("shared", DATA_ID)]));
    // a Block created with its data on the canvas owns it; a Block over data-tree data only binds it
    ws.ok(json!([table("own", "s1-content"), cell("b-own", "s1", Some("own"), "table", json!({ "x": 0, "y": 0, "w": 10, "h": 10 })),
                 cell("b-shared", "s1", Some("shared"), "table", json!({ "x": 20, "y": 0, "w": 10, "h": 10 }))]));
    let kinds = |ws: &W, src: &str| -> Vec<String> { let mut k: Vec<String> = ws.store.refs.iter().filter(|r| r.src_entity_id == src).map(|r| r.kind.clone()).collect(); k.sort(); k };
    assert_eq!(kinds(&ws, "b-own"), ["bind", "owns"]);
    assert_eq!(kinds(&ws, "b-shared"), ["bind"]);
    // a Block declaring shared data never owns what it was created with
    ws.ok(json!([table("declared", "s1-content"), { "op": "entity.create", "entity_id": "b-declared", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "e",
        "placement": { "x": 40, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "declared" }, "data_ownership": "shared" } }]));
    assert_eq!(kinds(&ws, "b-declared"), ["bind"]);
    // deleting the shared Block leaves the data; deleting the own Block cascades to its data (in the log)
    let rev = |ws: &W, id: &str| ws.store.entities[id].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "b-shared", "expect": { "rev": rev(&ws, "b-shared") } }]));
    assert!(ws.store.entities["shared"].alive());
    let seq = ws.ok(json!([{ "op": "entity.delete", "entity_id": "b-own", "expect": { "rev": rev(&ws, "b-own") } }]));
    assert!(!ws.store.entities["own"].alive(), "own data goes with its Block");
    assert_eq!(ws.history[(seq - 1) as usize][0].op["cascade"][0]["entity_id"], "own");
    // undo brings both back
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert!(ws.store.entities["own"].alive() && ws.store.entities["b-own"].alive());
    // a cross-canvas copy binds the same own data: it survives the first Block and goes with the last one
    ws.ok(json!([cell("b-own-2", "s2", Some("own"), "table", json!({ "x": 0, "y": 0, "w": 10, "h": 10 }))]));
    assert_eq!(kinds(&ws, "b-own-2"), ["bind"]);
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "b-own", "expect": { "rev": rev(&ws, "b-own") } }]));
    assert!(ws.store.entities["own"].alive(), "still shown elsewhere");
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "b-own-2", "expect": { "rev": rev(&ws, "b-own-2") } }]));
    assert!(!ws.store.entities["own"].alive(), "the last Block takes the data with it");
    // data.pin turns own data into shared data
    ws.ok(json!([table("pinned", "s1-content"), cell("b-pinned", "s1", Some("pinned"), "table", json!({ "x": 0, "y": 50, "w": 10, "h": 10 }))]));
    assert_eq!(kinds(&ws, "b-pinned"), ["bind", "owns"]);
    let seq = ws.ok(json!([{ "op": "data.pin", "entity_id": "pinned" }]));
    assert_eq!(kinds(&ws, "b-pinned"), ["bind"]);
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(kinds(&ws, "b-pinned"), ["bind", "owns"], "undo restores the ownership");
    ws.ok(json!([{ "op": "data.pin", "entity_id": "pinned" }]));
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "b-pinned", "expect": { "rev": rev(&ws, "b-pinned") } }]));
    assert!(ws.store.entities["pinned"].alive(), "pinned data never follows a Block");
}

#[test]
fn entity_clone_copies_data_and_a_same_canvas_copy_owns_its_clone() {
    use aiworkspace_core::plan::resolve_cell;
    let mut ws = MemWorkspace::new();
    let mut ops = surface("s1", "free");
    ops.push(table("t", "s1-content"));
    ops.push(json!({ "op": "table.insert_records", "source_id": "t", "records": [{ "record_id": "r1", "values": { "title": "a", "n": 1 } }, { "record_id": "r2", "values": { "title": "b", "n": 2 } }] }));
    ops.push(cell("b1", "s1", Some("t"), "table", json!({ "x": 0, "y": 0, "w": 10, "h": 10 })));
    ws.ok(Value::Array(ops));
    // the same-canvas copy: clone the data and bind the copy to the clone, in one Commit
    let seq = ws.ok(json!([
        { "op": "entity.clone", "entity_id": "t", "new_entity_id": "t2", "parent_id": "s1-content", "order_key": "c", "name": "t copy" },
        cell("b2", "s1", Some("t2"), "table", json!({ "x": 30, "y": 0, "w": 10, "h": 10 }))
    ]));
    let t2 = &ws.store.entities["t2"];
    assert_eq!((t2.type_id.as_str(), t2.name.as_deref()), (TYPE_TABLE, Some("t copy")));
    assert_eq!(ws.store.fields.keys().filter(|(s, _)| s == "t2").count(), 2);
    assert_eq!(ws.store.records.keys().filter(|(s, _)| s == "t2").count(), 2);
    assert_eq!(ws.store.records[&("t2".to_string(), "r2".to_string())].values["n"], 2);
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "b2" && r.kind == "owns" && r.dst_entity_id == "t2"));
    assert_eq!(resolve_cell(&ws.store, &json!({ "entity_id": "t2", "selector": { "kind": "table_members" } })).unwrap(), json!(seq));
    // the clone is independent: a value in the original does not touch the copy
    let rev = ws.store.records[&("t".to_string(), "r1".to_string())].value_rev("n");
    ws.ok(json!([{ "op": "table.set_values", "source_id": "t", "values": [{ "record_id": "r1", "field_id": "n", "value": 9, "expect": { "rev": rev } }] }]));
    assert_eq!(ws.store.records[&("t2".to_string(), "r1".to_string())].values["n"], 1);
    // undo removes the clone with its Block
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert!(!ws.store.entities["t2"].alive() && !ws.store.entities["b2"].alive());
    // Blocks are not cloned this way
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.clone", "entity_id": "b1", "new_entity_id": "b3", "parent_id": "s1", "order_key": "z" }]))), "INVALID_OPERATION");
}

#[test]
fn an_inline_link_never_blocks_deleting_its_target() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([table("t", DATA_ID), record("rec", DATA_ID, "x"),
        { "op": "entity.create", "entity_id": "doc", "type_id": TYPE_RICHTEXT, "parent_id": DATA_ID, "order_key": "d",
          "payload": { "content": { "type": "doc", "content": [
            { "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "see " }, { "type": "object_link", "attrs": { "ref": { "entity_id": "rec" }, "label": "the record" } }] }] } } }]));
    let link = ws.store.refs.iter().find(|r| r.src_entity_id == "doc").unwrap();
    assert_eq!((link.kind.as_str(), link.mode(), link.integrity()), ("object_link", "live", "weak"));
    let rev = ws.store.entities["rec"].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "rec", "expect": { "rev": rev } }]));
    assert!(!ws.store.entities["rec"].alive(), "a linked record can be deleted; the link is dead");
}

#[test]
fn opaque_entities_and_formulas_are_kept_verbatim_and_schema_checks_run_only_at_admission() {
    use aiworkspace_core::access::Access;
    use aiworkspace_core::plan::{plan_commit, PlanEnv};
    use aiworkspace_core::testkit::MemWorkspace as W;
    let mut ws = W::new();
    ws.ok(json!([table("t", DATA_ID)]));
    // a formula sits in a typed cell, is read back as given, and counts as empty for uniqueness
    let seq = ws.ok(json!([{ "op": "table.insert_records", "source_id": "t", "records": [
        { "record_id": "r1", "values": { "title": "a", "n": { "$formula": { "lang": "aiws", "version": "1", "expr": "1 + 1" } } } },
        { "record_id": "r2", "values": { "title": "b", "n": { "$formula": { "lang": "aiws", "version": "1", "expr": "1 + 1" } } } }] }]));
    assert_eq!(ws.store.records[&("t".to_string(), "r1".to_string())].values["n"]["$formula"]["expr"], "1 + 1");
    assert_eq!(code(&ws.fail(json!([{ "op": "table.set_values", "source_id": "t", "values": [{ "record_id": "r1", "field_id": "n", "value": { "$formula": { "lang": "aiws", "version": "1" } }, "expect": { "rev": seq } }] }]))), "INVALID_SCHEMA");
    // admission refuses a Viewport out of its Schema; the same operation replays (schema_checks off) as an opaque row
    let bad = json!([{ "op": "entity.create", "entity_id": "vp", "type_id": TYPE_VIEWPORT, "parent_id": SHOWS_ID, "order_key": "a",
        "payload": { "surface_ref": { "entity_id": "nope" }, "center": { "x": 0, "y": 0 }, "zoom": 99 } }]);
    assert_eq!(code(&ws.fail(bad.clone())), "INVALID_SCHEMA");
    let mut env = ws.env("alice", "human", None, true);
    env.schema_checks = false;
    env.replay = true;
    env.import = true;
    let req = W::request(bad);
    let access = Access::full("alice");
    let plan = plan_commit(&ws.store, &req, &env, &access).map_err(|f| f.to_json()).unwrap();
    assert!(plan.overlay.entities["vp"].alive());
    // a newer Schema version of a known type and a declared Schema id are stored as opaque objects (internal callers)
    let env = ws.env("alice", "import", None, true);
    let req = W::request(json!([{ "op": "entity.create", "entity_id": "scene", "type_id": "example.scene", "schema_version": 3, "schema_id": "example.scene@3",
        "parent_id": DATA_ID, "order_key": "s", "payload": { "instances": "tbl", "anything": [1, 2] } }]));
    ws.commit_with(&Access::full("alice"), &env, &req).map_err(|f| f.to_json()).unwrap();
    let scene = &ws.store.entities["scene"];
    assert_eq!((scene.schema_version, scene.schema_id.as_deref(), scene.opaque()), (3, Some("example.scene@3"), true));
    assert_eq!(aiworkspace_core::read::envelope(&ws.store, &Access::full("alice"), scene).unwrap()["schema_id"], "example.scene@3");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "scene2", "type_id": "example.scene", "parent_id": DATA_ID, "order_key": "s2", "payload": {} }]))), "MISSING_EXTENSION");
}

#[test]
fn a_resource_directory_is_a_value_container_isomorphic_with_a_dir_object() {
    use aiworkspace_core::canonical::{chunk_id, dir_object, file_object, DirMember};
    use aiworkspace_core::materialize::{dir_object_of, ObjectSink};
    use aiworkspace_core::plan::resolve_cell;
    use aiworkspace_core::types::dir_payload_from_dir_object;
    struct Sink(Vec<(String, String)>);
    impl ObjectSink for Sink {
        fn put_object(&mut self, t: &str, text: &str) -> aiworkspace_core::WsResult<String> {
            let (id, _) = aiworkspace_core::canonical::named_object(t, &serde_json::from_str(text).unwrap())?;
            self.0.push((t.to_string(), id.clone()));
            Ok(id)
        }
        fn put_file(&mut self, bytes: &[u8]) -> aiworkspace_core::WsResult<String> {
            Ok(file_object(bytes.len() as u64, &chunk_id(bytes))?.0)
        }
    }
    let mut ws = MemWorkspace::new();
    let (fid, _) = file_object(5, &chunk_id(b"hello")).unwrap();
    ws.store.assets.insert(fid.clone(), AssetInfo { object_id: fid.clone(), media_type: "text/plain".into(), size: 5 });
    // members by name: a file (media type and size from the object) and a child directory
    let seq = ws.ok(json!([
        { "op": "entity.create", "entity_id": "sub", "type_id": TYPE_DIR, "parent_id": DATA_ID, "order_key": "a", "name": "sub", "payload": { "members": {} } },
        { "op": "entity.create", "entity_id": "d", "type_id": TYPE_DIR, "parent_id": DATA_ID, "order_key": "b", "name": "d",
          "payload": { "members": { "hello.txt": { "kind": "file", "object_id": fid }, "sub": { "kind": "dir", "entity_id": "sub" } }, "meta": { "note": "x" } } }]));
    let d = &ws.store.entities["d"];
    assert_eq!(d.payload["members"]["hello.txt"]["size"], 5);
    assert_eq!((d.key_rev("m:hello.txt"), d.key_rev("m:sub")), (seq, seq));
    let kinds: Vec<(String, String)> = ws.store.refs.iter().filter(|r| r.src_entity_id == "d").map(|r| (r.kind.clone(), r.dst_entity_id.clone() + &r.dst_object_id)).collect();
    assert!(kinds.contains(&("asset".to_string(), fid.clone())) && kinds.contains(&("member".to_string(), "sub".to_string())), "{kinds:?}");
    // the member selector is a version cell; a bad name, a cycle and a missing child are refused
    assert_eq!(resolve_cell(&ws.store, &json!({ "entity_id": "d", "selector": { "kind": "folder_member", "name": "sub" } })).unwrap(), json!(seq));
    assert_eq!(code(&ws.fail(json!([{ "op": "dir.set_member", "entity_id": "d", "name": "a/b", "member": { "kind": "file", "object_id": fid }, "expect": { "rev": 0 } }]))), "INVALID_SCHEMA");
    assert_eq!(sub(&ws.fail(json!([{ "op": "dir.set_member", "entity_id": "sub", "name": "up", "member": { "kind": "dir", "entity_id": "d" }, "expect": { "rev": 0 } }]))), "TREE_CYCLE");
    assert_eq!(code(&ws.fail(json!([{ "op": "dir.set_member", "entity_id": "d", "name": "ghost", "member": { "kind": "dir", "entity_id": "nope" }, "expect": { "rev": 0 } }]))), "REFERENCE_BROKEN");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "d", "keys": [{ "key": "members", "value": {}, "expect": { "rev": seq } }] }]))), "INVALID_OPERATION");
    // the child directory is kept alive by the member reference
    let rev = ws.store.entities["sub"].life_rev;
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": "sub", "expect": { "rev": rev } }]))), "REFERENCE_BROKEN");
    // materialization: the same members give the same cydir id, computed as cyfs-ndn does
    let mut sink = Sink(Vec::new());
    let (id1, total) = dir_object_of(&ws.store, &ws.store.entities["d"], &mut sink, 0).unwrap();
    let (sub_id, _) = dir_object(&[]).unwrap();
    let (expect, _) = dir_object(&[DirMember { name: "hello.txt".into(), object_id: fid.clone(), size: 5, is_file: true }, DirMember { name: "sub".into(), object_id: sub_id.clone(), size: 0, is_file: false }]).unwrap();
    assert_eq!((id1.as_str(), total), (expect.as_str(), 5));
    assert!(sink.0.iter().all(|(t, _)| t == "cydir") && sink.0.len() == 2);
    let (id2, _) = dir_object_of(&ws.store, &ws.store.entities["d"], &mut Sink(Vec::new()), 0).unwrap();
    assert_eq!(id1, id2);
    // a DirObject imports back as a directory payload (children first)
    let mut dirs = std::collections::BTreeMap::new();
    dirs.insert(sub_id.clone(), "sub".to_string());
    let payload = dir_payload_from_dir_object(&json!({ "create_time": 0, "last_update_time": 0, "total_size": 5, "file_count": 1, "file_size": 5,
        "body": { "hello.txt": { "obj_type": "cyfile", "body": { "content": chunk_id(b"hello"), "create_time": 0, "last_update_time": 0, "size": 5 } }, "sub": sub_id } }), &dirs).unwrap();
    assert_eq!(payload["members"]["hello.txt"]["object_id"], fid);
    assert_eq!(payload["members"]["sub"]["entity_id"], "sub");
    // a value container: removing and re-adding a name are two cells; replace swaps the map; undo restores
    let seq2 = ws.ok(json!([{ "op": "dir.remove_member", "entity_id": "d", "name": "hello.txt", "expect": { "rev": seq } }]));
    assert!(resolve_cell(&ws.store, &json!({ "entity_id": "d", "selector": { "kind": "folder_member", "name": "hello.txt" } })).is_err());
    assert_eq!(code(&ws.fail(json!([{ "op": "dir.set_member", "entity_id": "d", "name": "hello.txt", "member": { "kind": "file", "object_id": fid }, "expect": { "rev": 0 } }]))), "REVISION_CONFLICT");
    ws.undo(seq2).map_err(|f| f.to_json()).unwrap();
    assert_eq!(ws.store.entities["d"].payload["members"]["hello.txt"]["object_id"], fid);
    let rev = ws.store.entities["d"].content_rev;
    let seq3 = ws.ok(json!([{ "op": "dir.replace", "entity_id": "d", "members": { "only.txt": { "kind": "file", "object_id": fid } }, "expect": { "rev": rev } }]));
    assert_eq!(ws.store.entities["d"].payload["members"].as_object().unwrap().len(), 1);
    assert!(!ws.store.refs.iter().any(|r| r.src_entity_id == "d" && r.kind == "member"), "the child member reference went with the map");
    ws.undo(seq3).map_err(|f| f.to_json()).unwrap();
    assert_eq!(ws.store.entities["d"].payload["members"]["sub"]["entity_id"], "sub");
    // the limit
    let many: serde_json::Map<String, Value> = (0..aiworkspace_core::types::MAX_DIR_MEMBERS + 1).map(|i| (format!("f{i}"), json!({ "kind": "file", "object_id": fid }))).collect();
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "big", "type_id": TYPE_DIR, "parent_id": DATA_ID, "order_key": "z", "payload": { "members": many } }]))), "LIMIT_EXCEEDED");
}

#[test]
fn two_trees_and_system_nodes() {
    let mut ws = MemWorkspace::new();
    // the system nodes exist from the start and are untouchable
    for id in [ROOT_ID, DATA_ID, SURFACES_ID, CANVAS_CONTENT_ID] {
        assert!(ws.store.entities[id].alive(), "{id}");
        assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": id, "expect": { "rev": 0 } }]))), "INVALID_OPERATION");
        assert_eq!(code(&ws.fail(json!([{ "op": "entity.rename", "entity_id": id, "name": "x", "expect": { "rev": 0 } }]))), "INVALID_OPERATION");
    }
    assert_eq!(ws.store.edges[CANVAS_CONTENT_ID].parent_id, DATA_ID);
    // a surface needs its content folder under canvas-content; nothing but surfaces goes under `surfaces`
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "s-bad", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
        "payload": { "kind": "surface" } }]))), "INVALID_SCHEMA");
    assert_eq!(sub(&ws.fail(json!([{ "op": "entity.create", "entity_id": "f-bad", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
        "payload": { "kind": "folder" } }]))), "CHILD_NOT_ALLOWED");
    assert_eq!(sub(&ws.fail(json!([{ "op": "entity.create", "entity_id": "p-bad", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
        "payload": { "kind": "page" } }]))), "CHILD_NOT_ALLOWED");
    ws.ok(json!(surface("s1", "free")));
    assert_eq!(ws.store.entities["s1"].payload["content_folder_id"], "s1-content");
    // data goes into the data tree, Blocks into the BlockTree — never the other way round
    ws.ok(json!([table("t1", DATA_ID), record("r1", DATA_ID, "林")]));
    assert_eq!(sub(&ws.fail(json!([table("t-bad", "s1")]))), "CHILD_NOT_ALLOWED");
    assert_eq!(sub(&ws.fail(json!([cell("c-bad", DATA_ID, Some("t1"), "table", Value::Null)]))), "CHILD_NOT_ALLOWED");
    assert_eq!(sub(&ws.fail(json!([{ "op": "entity.create", "entity_id": "g-bad", "type_id": TYPE_CONTAINER, "parent_id": DATA_ID, "order_key": "a", "payload": { "kind": "group" } }]))), "CHILD_NOT_ALLOWED");
    // folders nest in the data tree; groups nest in a Surface; canvas content lives in the Surface's folder
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "folder-a", "type_id": TYPE_CONTAINER, "parent_id": DATA_ID, "order_key": "c", "payload": { "kind": "folder", "title": "调研" } },
        { "op": "entity.create", "entity_id": "g1", "type_id": TYPE_CONTAINER, "parent_id": "s1", "order_key": "a", "placement": { "x": 10, "y": 10, "w": 400, "h": 300 }, "payload": { "kind": "group" } },
        record("r2", "s1-content", "王"),
        cell("c1", "g1", Some("t1"), "table", json!({ "x": 0, "y": 0, "w": 300, "h": 200 })),
        cell("c2", "s1", Some("r2"), "record", json!({ "x": 500, "y": 20, "w": 200, "h": 120 })),
    ]));
    // placement has no z any more; stacking is the order key
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.place", "entity_id": "c2", "placement": { "x": 1, "y": 2, "w": 3, "h": 4, "z": 1 } }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "tree.place", "entity_id": "c2", "order_key": "zz", "placement": { "x": 1, "y": 2, "w": 3, "h": 4 } }]));
    assert_eq!(ws.store.edges["c2"].placement, Some(json!({ "x": 1, "y": 2, "w": 3, "h": 4 })));
    // moving a Block across Surfaces keeps its id and binding; moving it into the data tree is refused
    ws.ok(json!(surface("s2", "free").into_iter().map(|mut op| { op["order_key"] = json!("b"); op }).collect::<Vec<_>>()));
    ws.ok(json!([{ "op": "tree.move", "entity_id": "c2", "new_parent_id": "s2", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 3, "h": 4 } }]));
    assert_eq!(ws.store.edges["c2"].parent_id, "s2");
    assert_eq!(sub(&ws.fail(json!([{ "op": "tree.move", "entity_id": "c2", "new_parent_id": DATA_ID, "order_key": "a" }]))), "CHILD_NOT_ALLOWED");
    // a free note: an annotation without target, placed in the data tree
    ws.ok(json!([{ "op": "entity.create", "entity_id": "note-free", "type_id": TYPE_ANNOTATION, "parent_id": "s1-content", "order_key": "n",
        "payload": { "kind": "note", "body": "便签" } }]));
    assert!(ws.store.entities["note-free"].payload.get("target").is_none());
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "note-bad", "type_id": TYPE_ANNOTATION, "parent_id": "s1-content", "order_key": "n",
        "payload": { "kind": "note", "body": "x", "range": { "kind": "richtext_text" } } }]))), "INVALID_SCHEMA");
    let read = aiworkspace_core::read::read(&ws.store, &Access::full("alice"), "note-free", None).unwrap();
    assert_eq!(read["content"]["anchor"]["state"], "resolved");
    assert_eq!(read["content"]["anchor"]["level"], "none");
    // the outline carries what the two trees need: kinds, system roles, renderer ids and sources
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    let find = |id: &str| outline.iter().find(|e| e["entity_id"] == id).cloned().unwrap();
    assert_eq!(find(CANVAS_CONTENT_ID)["system"], "canvas_content");
    assert_eq!(find("s1")["content_folder_id"], "s1-content");
    assert_eq!(find("c1")["view_type"], "table");
    assert_eq!(find("c1")["source_id"], "t1");
    assert!(find("s1").get("icon").is_none());
}

#[test]
fn surface_icon_is_a_shared_preset_id() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "s1", "keys": [{ "key": "icon", "value": "rocket", "expect": { "rev": 0 } }] }]));
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    assert_eq!(outline.iter().find(|e| e["entity_id"] == "s1").unwrap()["icon"], "rocket");
    let rev = aiworkspace_core::read::read(&ws.store, &Access::full("alice"), "s1", None).unwrap()["content"]["key_revs"]["icon"].clone();
    // only a preset-shaped id, and only on a Surface
    for value in [json!("Rocket"), json!("a b"), json!(""), json!(3), json!("x".repeat(33))] {
        assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "s1", "keys": [{ "key": "icon", "value": value, "expect": { "rev": rev } }] }]))), "INVALID_SCHEMA");
    }
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "f-icon", "type_id": TYPE_CONTAINER, "parent_id": DATA_ID, "order_key": "a",
        "payload": { "kind": "folder", "icon": "rocket" } }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "entity.unset_keys", "entity_id": "s1", "keys": [{ "key": "icon", "expect": { "rev": rev } }] }]));
    assert!(ws.store.entities["s1"].payload.get("icon").is_none());
}

/// draw.io 导入 TODO §2.5: a Surface's page settings live in `config` (an object the client checks, ≤ 64 KiB),
/// on Surfaces only.
#[test]
fn surface_config_holds_page_settings() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "s1", "keys": [{ "key": "config", "value": { "background": "#fffbe6", "page_size": { "w": 827, "h": 1169 } }, "expect": { "rev": 0 } }] }]));
    let read = aiworkspace_core::read::read(&ws.store, &Access::full("alice"), "s1", None).unwrap();
    assert_eq!(read["content"]["payload"]["config"]["background"], "#fffbe6");
    let rev = read["content"]["key_revs"]["config"].clone();
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "s1", "keys": [{ "key": "config", "value": "#fff", "expect": { "rev": rev } }] }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "s1", "keys": [{ "key": "config", "value": { "x": "y".repeat(70_000) }, "expect": { "rev": rev } }] }]))), "LIMIT_EXCEEDED");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "f-config", "type_id": TYPE_CONTAINER, "parent_id": DATA_ID, "order_key": "a",
        "payload": { "kind": "folder", "config": {} } }]))), "INVALID_SCHEMA");
}

/// 标准对象的交互改进 §7.1–§7.2: `placement.rotation` is layout (same operations, same capability),
/// `locked` is a boolean flag of Blocks and groups that the core checks but never enforces.
#[test]
fn rotation_is_layout_and_locked_is_a_flag() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([table("t1", DATA_ID), cell("c1", "s1", Some("t1"), "table", json!({ "x": 0, "y": 0, "w": 300, "h": 200, "rotation": 30 }))]));
    let placement = |ws: &MemWorkspace, id: &str| {
        let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
        outline.iter().find(|e| e["entity_id"] == id).unwrap()["placement"].clone()
    };
    assert_eq!(placement(&ws, "c1")["rotation"], 30);
    ws.ok(json!([{ "op": "tree.place", "entity_id": "c1", "placement": { "x": 10, "y": 0, "w": 300, "h": 200, "rotation": 359.5 } }]));
    assert_eq!(placement(&ws, "c1")["rotation"], 359.5);
    for rotation in [json!(360), json!(-1), json!("90"), json!(null)] {
        assert_eq!(code(&ws.fail(json!([{ "op": "tree.place", "entity_id": "c1", "placement": { "x": 0, "y": 0, "w": 300, "h": 200, "rotation": rotation } }]))), "INVALID_SCHEMA");
    }
    // a group carries the rotation of its children relative to itself, like x / y
    ws.ok(json!([{ "op": "entity.create", "entity_id": "g1", "type_id": TYPE_CONTAINER, "parent_id": "s1", "order_key": "e",
        "placement": { "x": 0, "y": 0, "w": 400, "h": 300, "rotation": 90 }, "payload": { "kind": "group", "layout": { "mode": "free" }, "locked": true } }]));
    ws.ok(json!([{ "op": "tree.move", "entity_id": "c1", "new_parent_id": "g1", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 300, "h": 200, "rotation": 15 } }]));
    assert_eq!(placement(&ws, "c1")["rotation"], 15);
    // locked: projected into the outline, boolean only, Blocks and groups only, never enforced
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "c1", "keys": [{ "key": "locked", "value": true, "expect": { "rev": 0 } }] }]));
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    assert_eq!(outline.iter().find(|e| e["entity_id"] == "c1").unwrap()["locked"], true);
    assert_eq!(outline.iter().find(|e| e["entity_id"] == "g1").unwrap()["locked"], true);
    let rev = aiworkspace_core::read::read(&ws.store, &Access::full("alice"), "c1", None).unwrap()["content"]["key_revs"]["locked"].clone();
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "c1", "keys": [{ "key": "locked", "value": "yes", "expect": { "rev": rev } }] }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "s1", "keys": [{ "key": "locked", "value": true, "expect": { "rev": 0 } }] }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "tree.place", "entity_id": "c1", "placement": { "x": 5, "y": 5, "w": 300, "h": 200 } }]));
    ws.ok(json!([{ "op": "entity.unset_keys", "entity_id": "c1", "keys": [{ "key": "locked", "expect": { "rev": rev } }] }]));
    assert!(ws.store.entities["c1"].payload.get("locked").is_none());
}

#[test]
fn cells_without_source_and_block_definitions() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([table("t1", DATA_ID)]));
    // built-in views still need a source of the right type
    assert_eq!(code(&ws.fail(json!([cell("c-nosrc", "s1", None, "table", Value::Null)]))), "INVALID_SCHEMA");
    // any other renderer id is accepted without source (frame, shape…) or with a known data type
    ws.ok(json!([cell("frame-1", "s1", None, "frame", json!({ "x": 0, "y": 0, "w": 800, "h": 600 })), cell("chart-1", "s1", Some("t1"), "acme.bar-chart", json!({ "x": 0, "y": 0, "w": 300, "h": 200 }))]));
    assert_eq!(code(&ws.fail(json!([cell("c-bad", "s1", Some("s1"), "acme.bar-chart", Value::Null)]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([cell("c-bad", "s1", Some("t1"), "Not Valid", Value::Null)]))), "INVALID_SCHEMA");
    // a definition entity, referenced by a cell: the reference blocks deletion of the definition
    ws.ok(json!([{ "op": "entity.create", "entity_id": "def-kpi", "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": "e", "name": "kpi",
        "payload": { "def_id": "acme.kpi", "version": 2, "kind": "declarative", "title": "指标", "accepts": [TYPE_TABLE],
                     "declarative": { "bindings": [{ "field": "n", "as": "value" }] }, "default_size": { "w": 200, "h": 120 } } }]));
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "def-bad", "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": "e",
        "payload": { "def_id": "acme.kpi", "kind": "html" } }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "entity.create", "entity_id": "kpi-1", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "f",
        "placement": { "x": 0, "y": 0, "w": 200, "h": 120 },
        "payload": { "view": { "type": "acme.kpi", "version": 2 }, "source_ref": { "entity_id": "t1" }, "def_ref": { "entity_id": "def-kpi" }, "config": { "field": "n" } } }]));
    let life = ws.store.entities["def-kpi"].life_rev;
    let f = ws.fail(json!([{ "op": "entity.delete", "entity_id": "def-kpi", "expect": { "rev": life } }]));
    assert_eq!(code(&f), "REFERENCE_BROKEN");
    assert_eq!(f["errors"][0]["data"]["referrers"][0]["entity_id"], "kpi-1");
    // a cell's config is bounded
    let big = "x".repeat(70_000);
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "kpi-1", "keys": [{ "key": "config", "value": { "blob": big }, "expect": { "rev": ws.head_seq } }] }]))), "LIMIT_EXCEEDED");
}

fn analysis(prompt: &str, context: &str) -> Value {
    json!({ "schema_version": "wish.analysis.v2", "status": "ready", "prompt": prompt, "context_prompt": context,
            "output_contract": { "results": [{ "name": "摘要", "type": "richtext", "approach": "direct" }] }, "checks": [], "blockers": [], "warnings": [] })
}

fn wish(id: &str, parent: &str, inputs: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_WISH, "parent_id": parent, "order_key": "w", "name": id,
            "payload": { "title": "许愿格", "prompt": "总结表格", "executor": "mock", "inputs": inputs,
                         "output": { "container_id": parent, "name": "摘要", "type": TYPE_RICHTEXT }, "output_mode": "overwrite" } })
}

#[test]
fn wishes_dependency_records_freshness_and_relations() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([table("t1", DATA_ID), record("r1", DATA_ID, "林")]));
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "t1", "records": [{ "record_id": "row-1", "values": { "title": "a", "n": 1 } }] }]));
    // a wish declares its inputs; they are indexed (kind input) but do not block deletion
    ws.ok(json!([wish("w1", "s1-content", json!([{ "entity_id": "t1", "name": "rows" }, { "entity_id": "r1", "name": "owner" }]))]));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "w1" && r.kind == "input" && r.dst_entity_id == "t1"));
    assert_eq!(code(&ws.fail(json!([wish("w-bad", "s1-content", json!([{ "entity_id": "ghost" }]))]))), "REFERENCE_BROKEN");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "w-bad", "type_id": TYPE_WISH, "parent_id": "s1-content", "order_key": "w",
        "payload": { "prompt": "x", "executor": "gpt" } }]))), "INVALID_SCHEMA");
    // the analysis records which prompt it was derived from: editing the prompt flags "needs re-analysis"
    let a = ws.store.entities["w1"].key_rev("analysis");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "analysis", "value": analysis("总结表格", "读取 rows 全部行与 owner 后总结"), "expect": { "rev": a } }] }]));
    let alice = Access::full("alice");
    assert_eq!(entity_freshness(&ws.store, &alice, "w1").unwrap()["needs_analysis"], false);
    let pr = ws.store.entities["w1"].key_rev("prompt");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "prompt", "value": "总结表格并给建议", "expect": { "rev": pr } }] }]));
    assert_eq!(entity_freshness(&ws.store, &alice, "w1").unwrap()["needs_analysis"], true);

    // the application: result content + Block + dependency record in one commit, guarded by the read set
    let members = ws.store.entities["t1"].key_rev(MEMBERS_KEY);
    let n_values = ws.store.fields[&("t1".into(), "n".into())].values_rev;
    let owner_rev = ws.store.entities["r1"].key_rev("p:owner");
    let read_set = json!([
        { "entity_id": "t1", "selector": { "kind": "table_members" }, "version": { "mode": "follow", "rev": members } },
        { "entity_id": "t1", "selector": { "kind": "table_field_values", "field_id": "n" }, "version": { "mode": "follow", "rev": n_values } },
        { "entity_id": "r1", "selector": { "kind": "doc_key", "key": "p:owner" }, "version": { "mode": "fixed", "rev": owner_rev } }
    ]);
    let derived = json!({ "wish_id": "w1", "run_id": "run-1", "executor": "mock", "simulated": true, "inputs": read_set });
    let apply = ws.ok(json!([
        { "op": "entity.create", "entity_id": "sum-1", "type_id": TYPE_RICHTEXT, "parent_id": "s1-content", "order_key": "x", "name": "摘要",
          "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "共 1 行（模拟）" }] }] } } },
        { "op": "entity.set_derived", "entity_id": "sum-1", "derived": derived },
        cell("c-sum", "s1", Some("sum-1"), "richtext", json!({ "x": 0, "y": 0, "w": 300, "h": 200 })),
        { "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "last_run", "value": { "run_id": "run-1", "state": "succeeded", "read_set": read_set, "produced": ["sum-1"],
            "result_bindings": { "results": { "摘要": { "type": "richtext", "entity_id": "sum-1", "approach": "direct" } } } }, "expect": { "rev": 0 } }] }
    ]));
    let d = ws.store.entities["sum-1"].derived.clone().unwrap();
    assert_eq!(d["generated_rev"], json!(apply));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "sum-1" && r.kind == "derived" && r.dst_entity_id == "t1"));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "sum-1" && r.kind == "produced" && r.dst_entity_id == "w1"));
    assert_eq!(ws.history.last().unwrap()[1].touched[0].change, "derived");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_derived", "entity_id": "c-sum", "derived": derived }]))), "INVALID_OPERATION");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_derived", "entity_id": "sum-1", "derived": { "wish_id": "w1" } }]))), "INVALID_SCHEMA");

    // fresh right after application; the wish agrees
    let f = entity_freshness(&ws.store, &alice, "sum-1").unwrap();
    assert_eq!(f["status"], "current");
    assert_eq!(f["manual_modified"], false);
    assert_eq!(entity_freshness(&ws.store, &alice, "w1").unwrap()["status"], "current");
    // layout and unrelated values do not stale it
    ws.ok(json!([{ "op": "tree.place", "entity_id": "c-sum", "placement": { "x": 50, "y": 50, "w": 300, "h": 200 } }]));
    let title_rev = ws.store.records[&("t1".into(), "row-1".into())].value_rev("title");
    ws.ok(json!([{ "op": "table.set_values", "source_id": "t1", "values": [{ "record_id": "row-1", "field_id": "title", "value": "b", "expect": { "rev": title_rev } }] }]));
    assert_eq!(entity_freshness(&ws.store, &alice, "sum-1").unwrap()["status"], "current");
    // a fixed input with a newer version is reported, not stale
    let owner_rev = ws.store.entities["r1"].key_rev("p:owner");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "r1", "keys": [{ "key": "p:owner", "value": "王", "expect": { "rev": owner_rev } }] }]));
    let f = entity_freshness(&ws.store, &alice, "sum-1").unwrap();
    assert_eq!(f["status"], "current");
    assert_eq!(f["inputs"][2]["newer"], true);
    // a followed input changed: stale, naming the input
    let n_rev = ws.store.records[&("t1".into(), "row-1".into())].value_rev("n");
    ws.ok(json!([{ "op": "table.set_values", "source_id": "t1", "values": [{ "record_id": "row-1", "field_id": "n", "value": 2, "expect": { "rev": n_rev } }] }]));
    let f = entity_freshness(&ws.store, &alice, "sum-1").unwrap();
    assert_eq!(f["status"], "stale");
    assert_eq!(f["changed_inputs"][0]["entity_id"], "t1");
    assert_eq!(f["changed_inputs"][0]["selector"]["field_id"], "n");
    assert_eq!(entity_freshness(&ws.store, &alice, "w1").unwrap()["status"], "stale");
    // the same stale result seen through an upstream dependency: upstream_stale, not current
    let derived2 = json!({ "wish_id": "w1", "run_id": "run-2", "executor": "mock",
                           "inputs": [{ "entity_id": "sum-1", "version": { "mode": "follow", "rev": ws.store.entities["sum-1"].content_rev } }] });
    ws.ok(json!([record("r-down", "s1-content", "x"), { "op": "entity.set_derived", "entity_id": "r-down", "derived": derived2 }]));
    let f = entity_freshness(&ws.store, &alice, "r-down").unwrap();
    assert_eq!(f["status"], "upstream_stale");
    assert_eq!(f["upstream"][0]["entity_id"], "sum-1");
    // a manual edit of the result after generation is detected
    let rt = ws.store.richtexts["sum-1"].clone();
    let hash = rt.meta.block_index["p1"].hash.clone();
    ws.ok(json!([{ "op": "richtext.replace_block", "entity_id": "sum-1", "block_id": "p1", "expect": { "hash": hash },
        "node": { "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "人工改过" }] } }]));
    assert_eq!(entity_freshness(&ws.store, &alice, "sum-1").unwrap()["manual_modified"], true);
    // the content moved on, so r-down's followed input changed too: now directly stale
    assert_eq!(entity_freshness(&ws.store, &alice, "r-down").unwrap()["status"], "stale");
    // a deleted input: unavailable (the input reference never blocked the deletion)
    let life = ws.store.entities["r1"].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "r1", "expect": { "rev": life } }]));
    let f = entity_freshness(&ws.store, &alice, "sum-1").unwrap();
    assert_eq!(f["status"], "unavailable");
    assert_eq!(f["inputs"][2]["reason"], "deleted");
    // unreadable input: unknown, with nothing about it leaked
    let carol = Access { principal: "carol".into(), ws_caps: CapSet::NONE, scoped: [("s1-content".to_string(), CapSet(Cap::Read as u16)), ("s1".to_string(), CapSet(Cap::Read as u16))].into() };
    let f = entity_freshness(&ws.store, &carol, "sum-1").unwrap();
    assert_eq!(f["status"], "unknown");
    assert_eq!(f["inputs"][0]["readable"], false);
    assert!(f["inputs"][0].get("name").is_none() && f["inputs"][0].get("current").is_none());
    // relations: outgoing with readable targets, incoming filtered, hidden ones only as a flag
    let r = relations(&ws.store, &alice, "t1").unwrap();
    assert!(r["incoming"].as_array().unwrap().iter().any(|l| l["kind"] == "input" && l["entity_id"] == "w1"));
    assert!(r["incoming"].as_array().unwrap().iter().any(|l| l["kind"] == "derived" && l["entity_id"] == "sum-1"));
    assert_eq!(r["hidden_incoming"], false);
    let r = relations(&ws.store, &carol, "sum-1").unwrap();
    let to_t1 = r["outgoing"].as_array().unwrap().iter().find(|l| l["entity_id"] == "t1").unwrap();
    assert_eq!(to_t1["readable"], false);
    assert!(to_t1.get("target").is_none());
    assert_eq!(r["produced"].as_array().map(Vec::len), Some(0));
    assert!(r["blocks"].as_array().unwrap().iter().any(|b| b["entity_id"] == "c-sum"));
    let r = relations(&ws.store, &alice, "w1").unwrap();
    assert!(r["produced"].as_array().unwrap().iter().any(|p| p["entity_id"] == "sum-1"));
    // a wish cannot depend on itself through its own result: refused at the structural level by the UI; the kernel terminates the walk
    assert_eq!(entity_freshness(&ws.store, &alice, "r-down").unwrap()["upstream"].as_array().is_some(), true);
}

#[test]
fn delete_precheck_names_only_readable_referrers() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([table("t1", DATA_ID), cell("c1", "s1", Some("t1"), "table", json!({ "x": 0, "y": 0, "w": 1, "h": 1 }))]));
    // bob reads the data tree but not the Surface holding the Block that references the table
    let bob = Access { principal: "bob".into(), ws_caps: CapSet(Cap::Delete as u16), scoped: [(DATA_ID.to_string(), CapSet(Cap::Read as u16 | Cap::Delete as u16))].into() };
    let env = ws.env("bob", "human", None, false);
    let life = ws.store.entities["t1"].life_rev;
    let f = ws.commit_with(&bob, &env, &MemWorkspace::request(json!([{ "op": "entity.delete", "entity_id": "t1", "expect": { "rev": life } }]))).err().unwrap().to_json();
    assert_eq!(code(&f), "REFERENCE_BROKEN");
    assert_eq!(f["errors"][0]["data"]["referrers"].as_array().map(Vec::len), Some(0));
    assert_eq!(f["errors"][0]["data"]["hidden_referrers"], true);
    assert!(!f.to_string().contains("c1"), "{f}");
    let f = ws.fail(json!([{ "op": "entity.delete", "entity_id": "t1", "expect": { "rev": life } }]));
    assert_eq!(f["errors"][0]["data"]["referrers"][0]["entity_id"], "c1");
    assert_eq!(f["errors"][0]["data"]["hidden_referrers"], false);
}

#[test]
fn packages_carry_dependency_records_and_refuse_old_formats() {
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
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([table("t1", DATA_ID), record("r1", "s1-content", "林"),
        { "op": "entity.set_derived", "entity_id": "r1", "derived": { "wish_id": "t1", "run_id": "run-9", "executor": "mock", "inputs": [{ "entity_id": "t1", "version": { "mode": "follow", "rev": 1 } }] } },
        cell("c1", "s1", Some("r1"), "record", json!({ "x": 1, "y": 2, "w": 3, "h": 4 }))]));
    let mut sink = Mem::default();
    let m = materialize(&ws.store, &mut sink, &|_| Ok(true), &mut |_, _| None).unwrap();
    let plan = load_ops(&sink, &m.content_root, &|_| None).unwrap();
    // system nodes are parents only; the record's dependency record and the Block's placement travel along
    assert!(plan.ops.iter().all(|op| !is_system_id(op["entity_id"].as_str().unwrap_or("")) || op["op"] == "entity.set_keys"));
    let derived = plan.ops.iter().find(|op| op["op"] == "entity.set_derived" && op["entity_id"] == "r1").expect("derived op");
    assert_eq!(derived["derived"]["run_id"], "run-9");
    let cell_op = plan.ops.iter().find(|op| op["entity_id"] == "c1").unwrap();
    assert_eq!(cell_op["placement"], json!({ "x": 1, "y": 2, "w": 3, "h": 4 }));
    // replaying into a fresh Workspace reproduces the content root
    let mut fresh = MemWorkspace::new();
    let env = fresh.env("alice", "import", None, true);
    let mut env = env;
    env.import = true;
    let req = MemWorkspace::request(Value::Array(plan.ops.clone()));
    fresh.commit_with(&Access::full("alice"), &env, &req).ok().expect("import");
    assert_eq!(fresh.store.entities["r1"].derived.as_ref().unwrap()["run_id"], "run-9");
    let mut sink2 = Mem::default();
    assert_eq!(materialize(&fresh.store, &mut sink2, &|_| Ok(true), &mut |_, _| None).unwrap().content_root, m.content_root);
    // an older format is refused with a clear version error, never migrated
    let root = aiworkspace_core::canonical::parse_strict(&sink.get_object(&m.content_root).unwrap()).unwrap();
    let mut old = root.clone();
    old["format_version"] = json!("0.1");
    let (old_id, text) = aiworkspace_core::canonical::named_object(aiworkspace_core::canonical::OBJ_TYPE_JSON, &old).unwrap();
    sink.0.insert(old_id.clone(), text);
    assert_eq!(load_ops(&sink, &old_id, &|_| None).err().map(|e| e.code.as_str().to_string()), Some("UNSUPPORTED_VERSION".to_string()));
}

/// 许愿格 v0.2 (§4, §6.2, §9.4, §12.1): input names and selectors, the core-computed analysis basis,
/// configuration staleness, the current result group, the program asset, Block bindings and the
/// folder-membership version cell.
#[test]
fn wish_v2_basis_config_groups_bindings() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!(surface("s1", "free")));
    ws.ok(json!([table("t1", DATA_ID), cell("view-1", "s1", Some("t1"), "table", json!({ "x": 0, "y": 0, "w": 300, "h": 200 }))]));
    let alice = Access::full("alice");
    // inputs: names are identifiers and unique; a table view must be a view of that table
    ws.ok(json!([wish("w1", "s1-content", json!([{ "entity_id": "t1", "name": "sales", "selector": { "kind": "table_view", "cell_id": "view-1" } }]))]));
    assert_eq!(code(&ws.fail(json!([wish("w2", "s1-content", json!([{ "entity_id": "t1", "name": "a" }, { "entity_id": "t1", "name": "a" }]))]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([wish("w2", "s1-content", json!([{ "entity_id": "t1", "name": "销售" }]))]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([wish("w2", "s1-content", json!([{ "entity_id": "t1", "selector": { "kind": "table_view", "cell_id": "s1" } }]))]))), "INVALID_OPERATION");
    assert_eq!(code(&ws.fail(json!([wish("w2", "s1-content", json!([{ "entity_id": "t1", "selector": { "kind": "table_cell", "record_id": "r", "field_id": "n" } }]))]))), "INVALID_SCHEMA");
    // the basis is computed by the core: a writer's digest is ignored
    let mut a = analysis("总结表格", "读取 sales 后总结");
    a["basis"] = json!({ "digest": "forged" });
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [
        { "key": "analysis", "value": a, "expect": { "rev": 0 } },
        { "key": "knowledge", "value": "销售额含税", "expect": { "rev": 0 } }] }]));
    assert_ne!(ws.store.entities["w1"].payload["analysis"]["basis"]["digest"], "forged");
    let f = entity_freshness(&ws.store, &alice, "w1").unwrap();
    assert_eq!(f["needs_analysis"], false);
    assert_eq!(f["status"], "none");
    // knowledge changes the meaning: re-analysis needed; output mode does not
    let om = ws.store.entities["w1"].key_rev("output_mode");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "output_mode", "value": "new", "expect": { "rev": om } }] }]));
    assert_eq!(entity_freshness(&ws.store, &alice, "w1").unwrap()["needs_analysis"], false);
    let kr = ws.store.entities["w1"].key_rev("knowledge");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "knowledge", "value": "销售额不含税", "expect": { "rev": kr } }] }]));
    assert_eq!(entity_freshness(&ws.store, &alice, "w1").unwrap()["needs_analysis"], true);
    // re-writing the analysis (as the host does when it appends inputs) refreshes the basis
    let ar = ws.store.entities["w1"].key_rev("analysis");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "analysis", "value": analysis("总结表格", "读取 sales 后总结"), "expect": { "rev": ar } }] }]));
    assert_eq!(entity_freshness(&ws.store, &alice, "w1").unwrap()["needs_analysis"], false);
    // a program needs its uploaded source
    let program = json!({ "language": "js", "api_version": 2, "source": "not an object id", "digest": "b".repeat(64), "produces": ["摘要"] });
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "program", "value": program, "expect": { "rev": 0 } }] }]))), "INVALID_SCHEMA");
    let object_id = "mix256:".to_string() + &"c".repeat(64);
    let program = json!({ "language": "js", "api_version": 2, "source": object_id, "digest": "b".repeat(64), "produces": ["摘要"] });
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "program", "value": program, "expect": { "rev": 0 } }] }]))), "DEPENDENCY_UNAVAILABLE");
    ws.store.assets.insert(object_id.clone(), AssetInfo { object_id: object_id.clone(), media_type: "text/plain".into(), size: 10 });

    // two result groups: the wish follows the current one (result_bindings), not every group it ever produced
    let members = ws.store.entities["t1"].key_rev(MEMBERS_KEY);
    let read_set = json!([{ "entity_id": "t1", "selector": { "kind": "table_members" }, "version": { "mode": "follow", "rev": members } }]);
    let config = aiworkspace_core::wish::config_digest(&ws.store.entities["w1"].payload);
    let derived = |run: &str| json!({ "wish_id": "w1", "run_id": run, "executor": "xllm", "inputs": read_set, "config_digest": config, "approach": "direct", "result_key": "摘要" });
    let doc = |id: &str| json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_RICHTEXT, "parent_id": "s1-content", "order_key": "x",
        "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "p1" } }] } } });
    // the old group was generated before a knowledge change (another configuration)
    let mut old = derived("run-old");
    old["config_digest"] = json!("0".repeat(32));
    ws.ok(json!([doc("old"), { "op": "entity.set_derived", "entity_id": "old", "derived": old }]));
    let lr = ws.store.entities["w1"].key_rev("last_run");
    ws.ok(json!([doc("new"), { "op": "entity.set_derived", "entity_id": "new", "derived": derived("run-new") },
        { "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "last_run", "value": { "run_id": "run-new", "read_set": read_set, "config_digest": config,
            "result_bindings": { "results": { "摘要": { "type": "richtext", "entity_id": "new", "approach": "direct" } } } }, "expect": { "rev": lr } }] }]));
    assert_eq!(entity_freshness(&ws.store, &alice, "old").unwrap()["config_changed"], true);
    assert_eq!(entity_freshness(&ws.store, &alice, "old").unwrap()["status"], "stale");
    let f = entity_freshness(&ws.store, &alice, "w1").unwrap();
    assert_eq!(f["status"], "current", "{f}");
    assert_eq!(f["results"][0]["entity_id"], "new");
    // a new configuration (the program) makes the current group stale through its recorded digest
    let pr = ws.store.entities["w1"].key_rev("program");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "program", "value": program, "expect": { "rev": pr } }] }]));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "w1" && r.kind == "asset" && r.dst_object_id == object_id));
    let f = entity_freshness(&ws.store, &alice, "w1").unwrap();
    assert_eq!((f["status"].as_str(), f["config_changed"].as_bool(), f["needs_analysis"].as_bool()), (Some("stale"), Some(true), Some(false)));
    assert_eq!(entity_freshness(&ws.store, &alice, "new").unwrap()["config_changed"], true);

    // Block bindings: indexed like source_ref (they block deleting the data), names are identifiers
    ws.ok(json!([{ "op": "entity.create", "entity_id": "html-1", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "h", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 },
        "payload": { "view": { "type": "acme.dash" }, "bindings": { "orders": { "entity_id": "t1" }, "summary": { "entity_id": "new" } } } }]));
    assert!(ws.store.refs.iter().any(|r| r.src_entity_id == "html-1" && r.kind == "bind" && r.dst_entity_id == "new"));
    let life = ws.store.entities["new"].life_rev;
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": "new", "expect": { "rev": life } }]))), "REFERENCE_BROKEN");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "html-1", "keys": [{ "key": "bindings", "value": { "source": { "entity_id": "t1" } }, "expect": { "rev": ws.head_seq } }] }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "html-1", "keys": [{ "key": "bindings", "value": { "x": { "entity_id": "ghost" } }, "expect": { "rev": ws.head_seq } }] }]))), "REFERENCE_BROKEN");

    // folder membership is a version cell: a new member changes it, a moved Block does not
    let cell_of = |ws: &MemWorkspace| aiworkspace_core::plan::resolve_cell(&ws.store, &json!({ "entity_id": "s1-content", "selector": { "kind": "tree_children" } })).unwrap();
    let before = cell_of(&ws);
    ws.ok(json!([{ "op": "tree.place", "entity_id": "view-1", "placement": { "x": 9, "y": 9, "w": 300, "h": 200 } }]));
    assert_eq!(cell_of(&ws), before);
    ws.ok(json!([record("r-new", "s1-content", "王")]));
    assert_ne!(cell_of(&ws), before);
}
