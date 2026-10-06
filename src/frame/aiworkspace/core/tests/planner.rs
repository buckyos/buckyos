//! Planner behavior on the in-memory store: tree rules, version cells,
//! tables, rich text, atomic mixed batches and compensation.

use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

/// One Surface (flow layout) with its canvas content folder: the phase-two shape of the sample page.
fn page(ws: &mut MemWorkspace) {
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "surface-main-content", "type_id": TYPE_CONTAINER, "parent_id": "canvas-content", "order_key": "a",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "surface-main", "title": "项目工作区" } },
        { "op": "entity.create", "entity_id": "surface-main", "type_id": TYPE_CONTAINER, "parent_id": "surfaces", "order_key": "a", "name": "项目工作区",
          "payload": { "kind": "surface", "layout": { "mode": "flow" }, "title": "项目工作区", "content_folder_id": "surface-main-content" } }
    ]));
}

fn tasks(ws: &mut MemWorkspace) {
    page(ws);
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "tasks", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b", "name": "任务",
          "payload": { "title_field_id": "title", "fields": [
            { "field_id": "title", "name": "任务", "type": "text", "required": true },
            { "field_id": "status", "name": "状态", "type": "select", "options": [
                { "option_id": "option-open", "label": "进行中" }, { "option_id": "option-done", "label": "完成" }] },
            { "field_id": "budget", "name": "预算", "type": "decimal", "scale": 2 },
            { "field_id": "due", "name": "截止日期", "type": "date" },
            { "field_id": "code", "name": "编号", "type": "text", "unique": true }] } },
        { "op": "table.insert_records", "source_id": "tasks", "records": [
            { "record_id": "task-41", "values": { "title": "盘点", "status": "option-done", "budget": "300", "code": "T41" } },
            { "record_id": "task-42", "values": { "title": "完成第一期架构验证", "status": "option-open", "budget": "1200.00", "due": "2026-10-20" } }] }
    ]));
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}
fn sub(v: &Value) -> &str {
    v["sub_code"].as_str().unwrap_or("")
}

#[test]
fn tree_rules_and_auto_merge() {
    let mut ws = MemWorkspace::new();
    page(&mut ws);
    let grp = |id: &str, parent: &str, key: &str| json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER,
        "parent_id": parent, "order_key": key, "payload": { "kind": "group" } });
    ws.ok(json!([grp("g-a", "surface-main", "a"), grp("g-b", "surface-main", "b"), grp("g-c", "g-a", "a")]));
    // root only holds the system nodes; a group belongs to a BlockTree, not to the data tree; ids are never reused; the root is fixed
    assert_eq!(sub(&ws.fail(json!([grp("g-x", "root", "c")]))), "CHILD_NOT_ALLOWED");
    assert_eq!(sub(&ws.fail(json!([grp("g-x", "data", "c")]))), "CHILD_NOT_ALLOWED");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": "data", "expect": { "rev": 0 } }]))), "INVALID_OPERATION");
    assert_eq!(sub(&ws.fail(json!([grp("g-a", "surface-main", "c")]))), "ID_CONFLICT");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": "root", "expect": { "rev": 0 } }]))), "INVALID_OPERATION");
    assert_eq!(code(&ws.fail(json!([grp("g-y", "nope", "c")]))), "NOT_FOUND");
    // two movers of the same node: both accepted, the later one wins
    let s1 = ws.ok(json!([{ "op": "tree.move", "entity_id": "g-c", "new_parent_id": "g-b", "order_key": "a" }]));
    let s2 = ws.ok(json!([{ "op": "tree.move", "entity_id": "g-c", "new_parent_id": "surface-main", "order_key": "c" }]));
    assert_eq!(ws.store.edges["g-c"].parent_id, "surface-main");
    assert_eq!(ws.store.edges["g-c"].struct_rev, s2);
    // undoing the earlier move must not drag the node back over the later move
    let f = ws.undo(s1).err().expect("conflict").to_json();
    assert_eq!(f["status"], "conflict");
    assert_eq!(f["conflicts"][0]["expected_rev"], json!(s1));
    // concurrent cycle: the first is accepted, the second is rejected as a cycle
    ws.ok(json!([{ "op": "tree.move", "entity_id": "g-a", "new_parent_id": "g-b", "order_key": "a" }]));
    assert_eq!(sub(&ws.fail(json!([{ "op": "tree.move", "entity_id": "g-b", "new_parent_id": "g-a", "order_key": "a" }]))), "TREE_CYCLE");
    // equal order keys still give a total order
    ws.ok(json!([grp("g-d", "surface-main", "c")]));
    let order: Vec<String> = ws.store.children("surface-main").unwrap().into_iter().map(|e| e.child_id).collect();
    assert_eq!(order, vec!["g-b", "g-c", "g-d"]);
    // names are unique among siblings, also when moving in
    ws.ok(json!([{ "op": "entity.rename", "entity_id": "g-c", "name": "同名", "expect": { "rev": 2 } }]));
    ws.ok(json!([grp("g-e", "g-b", "b")]));
    let rev = ws.store.entities["g-e"].meta_rev;
    ws.ok(json!([{ "op": "entity.rename", "entity_id": "g-e", "name": "同名", "expect": { "rev": rev } }]));
    assert_eq!(sub(&ws.fail(json!([{ "op": "tree.move", "entity_id": "g-e", "new_parent_id": "surface-main", "order_key": "d" }]))), "NAME_CONFLICT");
    // delete: children must be listed; a node moved in meanwhile is a conflict, not a silent victim
    let life = ws.store.entities["g-b"].life_rev;
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": "g-b", "expect": { "rev": life } }]))), "INVALID_OPERATION");
    let f = ws.fail(json!([{ "op": "entity.delete", "entity_id": "g-b", "subtree": { "delete": ["g-a"] }, "expect": { "rev": life } }]));
    assert_eq!(code(&f), "REVISION_CONFLICT");
    assert!(f["conflicts"][0]["detail"].as_str().unwrap().contains("g-e"));
    let del = ws.ok(json!([{ "op": "entity.delete", "entity_id": "g-b", "subtree": { "delete": ["g-a", "g-e", "gone"] }, "expect": { "rev": life } }]));
    assert!(!ws.store.entities["g-a"].alive() && !ws.store.entities["g-e"].alive());
    // moving a deleted node or into a deleted parent never resurrects anything
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.move", "entity_id": "g-a", "new_parent_id": "surface-main", "order_key": "x" }]))), "TARGET_DELETED");
    assert_eq!(code(&ws.fail(json!([{ "op": "tree.move", "entity_id": "g-d", "new_parent_id": "g-b", "order_key": "x" }]))), "TARGET_DELETED");
    // undo of the subtree delete restores the same set
    ws.undo(del).ok().expect("undo delete");
    assert!(ws.store.entities["g-b"].alive() && ws.store.entities["g-a"].alive() && ws.store.entities["g-e"].alive());
}

#[test]
fn cells_conflict_per_field_and_detect_aba() {
    let mut ws = MemWorkspace::new();
    tasks(&mut ws);
    let base = ws.head_seq;
    let set = |field: &str, value: Value, rev: u64| json!([{ "op": "table.set_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": field, "value": value, "expect": { "rev": rev } }] }]);
    // different fields of one record, both based on the same old state: both succeed
    ws.ok(set("status", json!("option-done"), base));
    let s_budget = ws.ok(set("budget", json!("1500"), base));
    assert_eq!(ws.store.records[&("tasks".into(), "task-42".into())].values["budget"], json!("1500.00"));
    // same field on a stale base: explicit conflict carrying the current value
    let f = ws.fail(set("budget", json!("9"), base));
    assert_eq!(code(&f), "REVISION_CONFLICT");
    assert_eq!(f["conflicts"][0]["current_value"], json!("1500.00"));
    assert_eq!(f["conflicts"][0]["current_rev"], json!(s_budget));
    // changed away and back: the value equals the old one but the write is still stale
    let s2 = ws.ok(set("budget", json!("1200.00"), s_budget));
    assert_eq!(code(&ws.fail(set("budget", json!("1"), base))), "REVISION_CONFLICT");
    assert!(s2 > s_budget);
    // all conflicts of a batch are reported together
    let f = ws.fail(json!([{ "op": "table.set_values", "source_id": "tasks", "values": [
        { "record_id": "task-42", "field_id": "budget", "value": "1", "expect": { "rev": 1 } },
        { "record_id": "task-42", "field_id": "status", "value": "option-open", "expect": { "rev": 1 } },
        { "record_id": "task-41", "field_id": "title", "value": "x", "expect": { "rev": base } }] }]));
    assert_eq!(f["conflicts"].as_array().unwrap().len(), 2);
    assert_eq!(ws.store.records[&("tasks".into(), "task-41".into())].values["title"], json!("盘点"), "nothing applied");
    // missing expect is an error, never last-writer-wins
    assert_eq!(code(&ws.fail(json!([{ "op": "table.set_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": "budget", "value": "1" }] }]))), "INVALID_OPERATION");
    // late write after delete: TARGET_DELETED, the record stays deleted
    let rec_rev = ws.store.records[&("tasks".into(), "task-41".into())].rev;
    ws.ok(json!([{ "op": "table.delete_records", "source_id": "tasks", "records": [{ "record_id": "task-41", "expect": { "rev": rec_rev } }] }]));
    assert_eq!(code(&ws.fail(set("title", json!("x"), base).as_array().map(|a| {
        let mut o = a[0].clone();
        o["values"][0]["record_id"] = json!("task-41");
        json!([o])
    }).unwrap())), "TARGET_DELETED");
    assert!(!ws.store.records[&("tasks".into(), "task-41".into())].alive());
}

#[test]
fn three_kinds_of_empty_and_constraints() {
    let mut ws = MemWorkspace::new();
    tasks(&mut ws);
    let rec = |ws: &MemWorkspace| ws.store.records[&("tasks".into(), "task-42".into())].clone();
    // required: cannot be unset; non-nullable: null rejected; "" is an ordinary text value
    assert_eq!(code(&ws.fail(json!([{ "op": "table.unset_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": "title", "expect": { "rev": 2 } }] }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "table.set_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": "due", "value": null, "expect": { "rev": 2 } }] }]))), "INVALID_SCHEMA");
    ws.ok(json!([{ "op": "table.set_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": "title", "value": "", "expect": { "rev": 2 } }] }]));
    assert_eq!(rec(&ws).values["title"], json!(""));
    let s = ws.ok(json!([{ "op": "table.unset_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": "due", "expect": { "rev": 2 } }] }]));
    assert!(!rec(&ws).values.contains_key("due"));
    assert_eq!(rec(&ws).revs["due"], s, "the cell's version still advances");
    ws.undo(s).ok().expect("undo unset");
    assert_eq!(rec(&ws).values["due"], json!("2026-10-20"));
    // unique is checked at the commit boundary across records
    let f = ws.fail(json!([{ "op": "table.insert_records", "source_id": "tasks",
        "records": [{ "record_id": "task-43", "values": { "title": "dup", "code": "T41" } }] }]));
    assert_eq!(sub(&f), "UNIQUE_VIOLATION");
    assert_eq!(sub(&ws.fail(json!([{ "op": "table.insert_records", "source_id": "tasks",
        "records": [{ "record_id": "task-42", "values": { "title": "again" } }] }]))), "ID_CONFLICT");
    assert_eq!(code(&ws.fail(json!([{ "op": "table.insert_records", "source_id": "tasks",
        "records": [{ "record_id": "task-44", "values": { "status": "option-open" } }] }]))), "INVALID_SCHEMA");
    // writes based on a stale field type are schema conflicts; renames do not invalidate them
    let f_status = ws.store.fields[&("tasks".into(), "status".into())].clone();
    ws.ok(json!([{ "op": "table.update_field", "source_id": "tasks", "field_id": "status",
        "changes": { "name": "进度" }, "expect": { "rev": f_status.def_rev } }]));
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "tasks", "field_type_revs": { "status": f_status.type_rev },
        "records": [{ "record_id": "task-45", "values": { "title": "ok", "status": "option-open" } }] }]));
    let def_rev = ws.store.fields[&("tasks".into(), "status".into())].def_rev;
    ws.ok(json!([{ "op": "table.delete_option", "source_id": "tasks", "field_id": "status", "option_id": "option-done",
        "on_values": "unset", "expect": { "rev": def_rev } }]));
    let f = ws.fail(json!([{ "op": "table.insert_records", "source_id": "tasks", "field_type_revs": { "status": f_status.type_rev },
        "records": [{ "record_id": "task-46", "values": { "title": "late", "status": "option-open" } }] }]));
    assert_eq!(code(&f), "SCHEMA_CONFLICT");
}

#[test]
fn field_delete_restore_and_migration() {
    let mut ws = MemWorkspace::new();
    tasks(&mut ws);
    let key = ("tasks".to_string(), "task-42".to_string());
    let def_rev = ws.store.fields[&("tasks".into(), "budget".into())].def_rev;
    let del = ws.ok(json!([{ "op": "table.delete_field", "source_id": "tasks", "field_id": "budget", "expect": { "rev": def_rev } }]));
    // residual values stay in storage (for lossless undo) but the field is gone for writers
    assert_eq!(ws.store.records[&key].values["budget"], json!("1200.00"));
    assert_eq!(code(&ws.fail(json!([{ "op": "table.set_values", "source_id": "tasks",
        "values": [{ "record_id": "task-42", "field_id": "budget", "value": "1", "expect": { "rev": 2 } }] }]))), "TARGET_DELETED");
    assert_eq!(sub(&ws.fail(json!([{ "op": "table.add_field", "source_id": "tasks",
        "field": { "field_id": "budget", "name": "新预算", "type": "text" } }]))), "ID_CONFLICT");
    assert_eq!(code(&ws.fail(json!([{ "op": "table.restore_field", "source_id": "tasks", "field_id": "budget", "expect": { "rev": del } }]))),
        "INVALID_OPERATION", "internal op is not callable from outside");
    ws.undo(del).ok().expect("restore field");
    let f = ws.store.fields[&("tasks".into(), "budget".into())].clone();
    assert!(f.alive() && f.type_rev == ws.head_seq);
    assert_eq!(ws.store.records[&key].revs["budget"], 2, "cell versions are untouched by delete/restore");
    assert_eq!(code(&ws.fail(json!([{ "op": "table.delete_field", "source_id": "tasks", "field_id": "title", "expect": { "rev": 2 } }]))), "INVALID_OPERATION");

    // migration: text → date, strict parsing, all-or-nothing
    ws.ok(json!([{ "op": "table.add_field", "source_id": "tasks", "field": { "field_id": "due_text", "name": "截止文本", "type": "text" } }]));
    let add = ws.head_seq;
    ws.ok(json!([{ "op": "table.set_values", "source_id": "tasks", "values": [
        { "record_id": "task-41", "field_id": "due_text", "value": "2026-11-01", "expect": { "rev": 0 } },
        { "record_id": "task-42", "field_id": "due_text", "value": "next friday", "expect": { "rev": 0 } }] }]));
    let before = ws.store.clone();
    let mig = json!({ "op": "table.migrate_field", "source_id": "tasks", "field_id": "due_text", "expect": { "rev": add }, "to": { "type": "date" } });
    let f = ws.fail(json!([mig]));
    assert_eq!(code(&f), "INVALID_OPERATION");
    assert_eq!(f["errors"][0]["data"]["report"]["failing"]["count"], 1);
    assert_eq!(f["errors"][0]["data"]["report"]["failing"]["sample"][0]["record_id"], "task-42");
    assert_eq!(ws.store.records, before.records, "no half-migrated table");
    assert_eq!(ws.store.fields, before.fields);
    let mut lenient = mig.clone();
    lenient["on_failure"] = json!("unset");
    let m = ws.ok(json!([lenient]));
    assert_eq!(ws.store.fields[&("tasks".into(), "due_text".into())].def.ty.as_str(), "date");
    assert!(!ws.store.records[&key].values.contains_key("due_text"));
    assert_eq!(code(&ws.fail(json!([{ "op": "table.set_values", "source_id": "tasks", "field_type_revs": { "due_text": add },
        "values": [{ "record_id": "task-42", "field_id": "due_text", "value": "2026-12-01", "expect": { "rev": m } }] }]))), "SCHEMA_CONFLICT");
    assert_eq!(sub(&ws.fail(json!([{ "op": "table.migrate_field", "source_id": "tasks", "field_id": "due_text",
        "expect": { "rev": m }, "to": { "type": "boolean" } }]))), "MIGRATION_UNSUPPORTED");
    ws.undo(m).ok().expect("undo migration");
    assert_eq!(ws.store.records[&key].values["due_text"], json!("next friday"));
    assert_eq!(ws.store.fields[&("tasks".into(), "due_text".into())].def.ty.as_str(), "text");
}

fn para(id: &str, text: &str) -> Value {
    json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] })
}

#[test]
fn mixed_batch_is_atomic_and_richtext_blocks_use_tokens() {
    let mut ws = MemWorkspace::new();
    tasks(&mut ws);
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "cell-open", "type_id": TYPE_CELL, "parent_id": "surface-main", "order_key": "c",
          "payload": { "source_ref": { "entity_id": "tasks" }, "view": { "type": "table" }, "title": "未完成任务",
                       "filter": { "op": "cmp", "field_id": "status", "operator": "ne", "value": "option-done" } } },
        { "op": "entity.create", "entity_id": "notes", "type_id": TYPE_RICHTEXT, "parent_id": "data", "order_key": "d",
          "payload": { "content": { "type": "doc", "content": [
              para("intro", "项目说明"),
              { "type": "object_embed", "attrs": { "block_id": "emb", "ref": { "entity_id": "cell-open" } } }] } } }
    ]));
    let created = ws.head_seq;
    let rt = ws.store.richtexts["notes"].clone();
    let snapshot_before = aiworkspace_core::richtext::export_snapshot(&rt.doc).unwrap();
    let state_before = ws.store.clone();
    // tree + rich text + table in one batch, the last step fails → nothing at all happened
    let f = ws.fail(json!([
        { "op": "tree.place", "entity_id": "notes", "order_key": "e" },
        { "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "after": "intro" }, "blocks": [para("p2", "新增段落")] },
        { "op": "table.set_values", "source_id": "tasks",
          "values": [{ "record_id": "task-42", "field_id": "status", "value": "option-nope", "expect": { "rev": 2 } }] }
    ]));
    assert_eq!(f["status"], "rejected");
    assert_eq!(f["errors"][0]["op_index"], 2);
    assert_eq!(ws.head_seq, created);
    assert_eq!(ws.store.entities, state_before.entities);
    assert_eq!(ws.store.edges, state_before.edges);
    assert_eq!(aiworkspace_core::richtext::export_snapshot(&ws.store.richtexts["notes"].doc).unwrap(), snapshot_before,
        "authoritative CRDT untouched by the rejected candidate");
    // the same batch with a valid last step commits everything under one seq
    let s = ws.ok(json!([
        { "op": "tree.place", "entity_id": "notes", "order_key": "e" },
        { "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "after": "intro" }, "blocks": [para("p2", "新增段落")] },
        { "op": "table.set_values", "source_id": "tasks",
          "values": [{ "record_id": "task-42", "field_id": "status", "value": "option-done", "expect": { "rev": 2 } }] }
    ]));
    let idx = ws.store.richtexts["notes"].meta.block_index.clone();
    assert_eq!(idx["p2"].struct_rev, s);
    assert_eq!(idx["intro"].struct_rev, created, "neighbours of an insert did not move");
    assert!(ws.history.last().unwrap()[1].op["server_update"].is_string());
    // block tokens: stale hash conflicts and returns the current block
    let f = ws.fail(json!([{ "op": "richtext.replace_block", "entity_id": "notes", "block_id": "p2",
        "expect": { "hash": "0000" }, "node": para("p2", "x") }]));
    assert_eq!(code(&f), "REVISION_CONFLICT");
    assert_eq!(f["conflicts"][0]["current_value"]["node"]["content"][0]["text"], "新增段落");
    let rep = ws.ok(json!([{ "op": "richtext.replace_block", "entity_id": "notes", "block_id": "p2",
        "expect": { "hash": idx["p2"].hash }, "node": { "type": "heading", "attrs": { "block_id": "p2", "level": 2 },
        "content": [{ "type": "text", "text": "改成标题" }] } }]));
    // duplicate block ids and dangling embeds are rejected, state unchanged
    assert_eq!(code(&ws.fail(json!([{ "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "end": true }, "blocks": [para("intro", "dup")] }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "end": true },
        "blocks": [{ "type": "object_embed", "attrs": { "block_id": "e2", "ref": { "entity_id": "tasks" } } }] }]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([{ "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "end": true },
        "blocks": [{ "type": "object_embed", "attrs": { "block_id": "e2", "ref": { "entity_id": "ghost" } } }] }]))), "REFERENCE_BROKEN");
    // the embedded cell and its table are protected by the reference index
    let life = ws.store.entities["cell-open"].life_rev;
    let f = ws.fail(json!([{ "op": "entity.delete", "entity_id": "cell-open", "expect": { "rev": life } }]));
    assert_eq!(code(&f), "REFERENCE_BROKEN");
    assert_eq!(f["errors"][0]["data"]["referrers"][0]["entity_id"], "notes");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.delete", "entity_id": "tasks", "expect": { "rev": 2 } }]))), "REFERENCE_BROKEN");
    // compensation of block ops: replace back, then the mixed batch as a whole
    ws.undo(rep).ok().expect("undo replace");
    assert_eq!(ws.store.richtexts["notes"].meta.ast["content"][1]["content"][0]["text"], "新增段落");
    ws.undo(s).ok().expect("undo mixed batch");
    assert!(!ws.store.richtexts["notes"].meta.block_index.contains_key("p2"));
    assert_eq!(ws.store.records[&("tasks".into(), "task-42".into())].values["status"], json!("option-open"));
    assert_eq!(ws.store.edges["notes"].order_key, "d");
}

#[test]
fn keyed_documents_and_views() {
    let mut ws = MemWorkspace::new();
    tasks(&mut ws);
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "project-info", "type_id": TYPE_RECORD, "parent_id": "data", "order_key": "c",
          "payload": { "schema": { "properties": [
              { "key": "owner", "name": "负责人", "type": "text", "required": true },
              { "key": "budget", "name": "预算", "type": "decimal", "scale": 2 }] },
            "props": { "owner": "林", "budget": "1200" } } },
        { "op": "entity.create", "entity_id": "cell-all", "type_id": TYPE_CELL, "parent_id": "surface-main", "order_key": "d",
          "payload": { "source_ref": { "entity_id": "tasks" }, "view": { "type": "table" }, "title": "全部任务" } }
    ]));
    let s = ws.head_seq;
    assert_eq!(ws.store.entities["project-info"].payload["p:budget"], json!("1200.00"));
    // independent top-level keys do not conflict; the same key does
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "project-info", "keys": [{ "key": "p:owner", "value": "王", "expect": { "rev": s } }] }]));
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "project-info", "keys": [{ "key": "p:budget", "value": "9", "expect": { "rev": s } }] }]));
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "project-info", "keys": [{ "key": "p:owner", "value": "赵", "expect": { "rev": s } }] }]))), "REVISION_CONFLICT");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "project-info", "keys": [{ "key": "p:nope", "value": 1, "expect": { "rev": 0 } }] }]))), "INVALID_SCHEMA");
    // saving a view: filter and width are separate cells; the source's content_rev is untouched
    let src_rev = ws.store.entities["tasks"].content_rev;
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "cell-all", "keys": [
        { "key": "filter", "value": { "op": "cmp", "field_id": "status", "operator": "ne", "value": "option-done" }, "expect": { "rev": 0 } }] }]));
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "cell-all", "keys": [
        { "key": "fields", "value": [{ "field_id": "title", "width": 320 }], "expect": { "rev": 0 } }] }]));
    assert_eq!(ws.store.entities["tasks"].content_rev, src_rev);
    assert_eq!(ws.history.last().unwrap()[0].touched[0].change, "view");
    // operator/type matrix is enforced when a view is saved
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "cell-all", "keys": [
        { "key": "filter", "value": { "op": "cmp", "field_id": "title", "operator": "lt", "value": "a" }, "expect": { "rev": ws.head_seq - 1 } }] }]))), "INVALID_OPERATION");
    // a cell cannot bind a source of the wrong type
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.create", "entity_id": "cell-bad", "type_id": TYPE_CELL, "parent_id": "surface-main",
        "order_key": "e", "payload": { "source_ref": { "entity_id": "project-info" }, "view": { "type": "table" } } }]))), "INVALID_SCHEMA");
}

#[test]
fn permissions_append_only_and_personal_scope() {
    let mut ws = MemWorkspace::new();
    tasks(&mut ws);
    let bob = Access { principal: "bob".into(), ws_caps: CapSet(Cap::Append as u16), scoped: Default::default() };
    let env = ws.env("bob", "human", None, false);
    // append-only: may insert, may not update or delete
    let req = MemWorkspace::request(json!([{ "op": "table.insert_records", "source_id": "tasks",
        "records": [{ "record_id": "task-b1", "values": { "title": "bob 的任务" } }] }]));
    ws.commit_with(&bob, &env, &req).ok().expect("append");
    let env = ws.env("bob", "human", None, false);
    let req = MemWorkspace::request(json!([{ "op": "table.set_values", "source_id": "tasks",
        "values": [{ "record_id": "task-b1", "field_id": "title", "value": "x", "expect": { "rev": 3 } }] }]));
    assert_eq!(code(&ws.commit_with(&bob, &env, &req).err().unwrap().to_json()), "PERMISSION_DENIED");
    // read-only principal with comment: can annotate a cell, cannot touch data
    let carol = Access { principal: "carol".into(), ws_caps: CapSet(Cap::Read as u16 | Cap::Comment as u16), scoped: Default::default() };
    let env = ws.env("carol", "human", None, false);
    let req = MemWorkspace::request(json!([{ "op": "entity.create", "entity_id": "note-budget", "type_id": TYPE_ANNOTATION,
        "parent_id": "data", "order_key": "z", "scope": "personal",
        "payload": { "target": { "entity_id": "tasks", "selector": { "kind": "table_cell", "record_id": "task-42", "field_id": "budget" } },
                     "kind": "note", "body": "请核对预算来源" } }]));
    ws.commit_with(&carol, &env, &req).ok().expect("annotate");
    assert_eq!(ws.store.entities["note-budget"].scope, "user:carol");
    assert_eq!(ws.store.entities["note-budget"].payload["author"], "carol");
    // a personal annotation does not exist for anyone else
    let alice = Access::full("alice");
    let env = ws.env("alice", "human", None, false);
    let req = MemWorkspace::request(json!([{ "op": "entity.set_keys", "entity_id": "note-budget",
        "keys": [{ "key": "body", "value": "hijack", "expect": { "rev": 4 } }] }]));
    assert_eq!(code(&ws.commit_with(&alice, &env, &req).err().unwrap().to_json()), "NOT_FOUND");
    // anchors do not block deletion and never re-attach: the note falls back to its table
    let rev = ws.store.records[&("tasks".into(), "task-42".into())].rev;
    ws.ok(json!([{ "op": "table.delete_records", "source_id": "tasks", "records": [{ "record_id": "task-42", "expect": { "rev": rev } }] }]));
    let note = &ws.store.entities["note-budget"].payload;
    let a = aiworkspace_core::anchor::resolve(&ws.store, note).unwrap();
    assert_eq!((a.state, a.level), (aiworkspace_core::anchor::State::Degraded, aiworkspace_core::anchor::Level::Entity));
    assert_eq!(note["target"]["selector"]["record_id"], "task-42");
}
