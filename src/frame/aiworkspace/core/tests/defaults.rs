//! Field defaults (UI 框架 PRD §7.3): what a new record holds when its insert does not give a field — set and cleared
//! with `table.add_field` / `table.update_field`, filled in by `table.insert_records` for every writer, converted by
//! `table.migrate_field` and kept in step with `table.delete_option`.

use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::plan::plan_commit;
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::value::FieldDef;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn detail(v: &Value) -> String {
    v["errors"][0]["detail"].as_str().unwrap_or("").to_string()
}

fn opts() -> Value {
    json!([{ "option_id": "a", "label": "甲" }, { "option_id": "b", "label": "乙" }, { "option_id": "c", "label": "丙" }])
}

fn table() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([{ "op": "entity.create", "entity_id": "t", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b",
        "payload": { "fields": [
            { "field_id": "title", "name": "Title", "type": "text" },
            { "field_id": "state", "name": "State", "type": "select", "options": opts(), "default": "a" },
            { "field_id": "tags", "name": "Tags", "type": "multi_select", "options": opts(), "default": ["c", "a"] },
            { "field_id": "price", "name": "Price", "type": "decimal", "scale": 2, "default": "9.5" },
            { "field_id": "done", "name": "Done", "type": "boolean", "required": true, "default": false },
            { "field_id": "note", "name": "Note", "type": "text", "nullable": true, "default": "-" }] } }]));
    ws
}

fn field(ws: &MemWorkspace, id: &str) -> FieldRow {
    ws.store.fields[&("t".to_string(), id.to_string())].clone()
}

fn record(ws: &MemWorkspace, id: &str) -> RecordRow {
    ws.store.records[&("t".to_string(), id.to_string())].clone()
}

fn insert(id: &str, values: Value) -> Value {
    json!([{ "op": "table.insert_records", "source_id": "t", "records": [{ "record_id": id, "values": values }] }])
}

fn update(ws: &MemWorkspace, id: &str, changes: Value) -> Value {
    json!([{ "op": "table.update_field", "source_id": "t", "field_id": id, "changes": changes, "expect": { "rev": field(ws, id).def_rev } }])
}

fn def(v: Value) -> Result<FieldDef, String> {
    FieldDef::from_value(&v).map_err(|e| e.detail)
}

#[test]
fn a_default_is_a_canonical_value_of_the_type() {
    let d = |extra: Value| {
        let mut f = json!({ "field_id": "f", "name": "F" });
        f.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        def(f)
    };
    assert_eq!(d(json!({ "type": "decimal", "scale": 2, "default": "9.5" })).unwrap().default, Some(json!("9.50")));
    assert_eq!(d(json!({ "type": "multi_select", "options": opts(), "default": ["c", "a", "c"] })).unwrap().default, Some(json!(["a", "c"])));
    assert_eq!(d(json!({ "type": "currency", "currency": "CNY", "default": "12" })).unwrap().default, Some(json!({ "amount": "12.00", "currency": "CNY" })));
    assert_eq!(d(json!({ "type": "datetime", "default": "2026-11-14T08:00:00+08:00" })).unwrap().default, Some(json!("2026-11-14T00:00:00.000Z")));
    assert_eq!(d(json!({ "type": "json", "default": { "x": 1 } })).unwrap().default, Some(json!({ "x": 1 })));
    assert_eq!(d(json!({ "type": "text" })).unwrap().default, None);
    // the definition is stored with its canonical default
    assert_eq!(d(json!({ "type": "number", "default": 3.0 })).unwrap().to_value()["default"], json!(3));
    for (extra, why) in [
        (json!({ "type": "number", "default": "3" }), "default: field f: expected number"),
        (json!({ "type": "select", "options": opts(), "default": "z" }), "unknown option"),
        (json!({ "type": "multi_select", "options": opts(), "max_items": 1, "default": ["a", "b"] }), "at most 1 options"),
        (json!({ "type": "date", "default": "2026-02-30" }), "YYYY-MM-DD"),
        (json!({ "type": "text", "nullable": true, "default": null }), ""),
        (json!({ "type": "number", "default": { "$formula": { "lang": "aiws", "version": "1", "expr": "1+1" } } }), "not a formula"),
        (json!({ "type": "text", "submitter": true, "default": "x" }), "submitter field has no default"),
        (json!({ "type": "text", "unique": true, "default": "x" }), "unique field has no default"),
        (json!({ "type": "object_ref", "default": { "entity_id": "t" } }), "object_ref field has no default"),
    ] {
        match d(extra.clone()) {
            // `"default": null` is no default at all (serde reads it as absent)
            Ok(f) if why.is_empty() => assert_eq!(f.default, None, "{extra}"),
            Ok(_) => panic!("{extra} must be refused"),
            Err(e) => assert!(e.contains(why), "{extra}: {e}"),
        }
    }
    // a record's properties take none
    let mut ws = MemWorkspace::new();
    let f = ws.fail(json!([{ "op": "entity.create", "entity_id": "r", "type_id": TYPE_RECORD, "parent_id": "data", "order_key": "b",
        "payload": { "schema": { "properties": [{ "key": "k", "name": "K", "type": "text", "default": "x" }] } } }]));
    assert!(detail(&f).contains("default are not supported on record properties"), "{f}");
}

#[test]
fn an_insert_fills_the_fields_it_does_not_give() {
    let mut ws = table();
    ws.ok(insert("r1", json!({ "title": "one" })));
    let r = record(&ws, "r1");
    assert_eq!(Value::Object(r.values.clone()), json!({ "title": "one", "state": "a", "tags": ["a", "c"], "price": "9.50", "done": false, "note": "-" }));
    assert!(r.revs.keys().any(|k| k == "price"), "a filled value has its version");
    assert_eq!(field(&ws, "price").values_rev, ws.head_seq, "a filled field's values changed");
    // what is given wins; an explicit null stays null where the field takes it; a required field with a default may be left out
    ws.ok(insert("r2", json!({ "state": "b", "tags": ["b"], "note": null })));
    let r = record(&ws, "r2");
    assert_eq!((&r.values["state"], &r.values["tags"], &r.values["note"], &r.values["done"]), (&json!("b"), &json!(["b"]), &Value::Null, &json!(false)));
    assert!(!r.values.contains_key("title"), "a field without a default stays unset");
    // later writes are ordinary: the default is not put back
    let rev = record(&ws, "r2").value_rev("state");
    ws.ok(json!([{ "op": "table.unset_values", "source_id": "t", "values": [{ "record_id": "r2", "field_id": "state", "expect": { "rev": rev } }] }]));
    assert!(!record(&ws, "r2").values.contains_key("state"));
    // undo and redo of the insert keep the record as it was written
    let seq = ws.ok(insert("r3", json!({})));
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert!(!record(&ws, "r3").alive());
    ws.undo(ws.head_seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(record(&ws, "r3").values["price"], json!("9.50"));
    // an internal write (a package import, undo, replay) keeps what was recorded
    let env = ws.env("alice", "human", None, true);
    ws.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(insert("r4", json!({ "done": true })))).map_err(|f| f.to_json()).unwrap();
    assert_eq!(Value::Object(record(&ws, "r4").values.clone()), json!({ "done": true }));
    // the normalized operation (the Commit's log) carries the filled values
    let env = ws.env("alice", "human", None, false);
    let (alice, req) = (Access::full("alice"), MemWorkspace::request(insert("r5", json!({ "title": "five" }))));
    let plan = plan_commit(&ws.store, &req, &env, &alice).map_err(|f| f.to_json()).unwrap();
    assert_eq!(plan.ops[0].op["records"][0]["values"]["price"], json!("9.50"));
}

#[test]
fn update_field_sets_and_clears_the_default() {
    let mut ws = table();
    ws.ok(insert("r1", json!({})));
    let seq = ws.ok(update(&ws, "price", json!({ "default": "12" })));
    assert_eq!(field(&ws, "price").def.default, Some(json!("12.00")));
    assert_eq!(record(&ws, "r1").values["price"], json!("9.50"), "stored values are never changed");
    assert_ne!(field(&ws, "price").type_rev, seq, "a default does not tighten the type");
    ws.ok(insert("r2", json!({})));
    assert_eq!(record(&ws, "r2").values["price"], json!("12.00"));
    ws.ok(update(&ws, "price", json!({ "default": null })));
    assert_eq!(field(&ws, "price").def.default, None);
    ws.ok(insert("r3", json!({})));
    assert!(!record(&ws, "r3").values.contains_key("price"));
    // undo puts the previous default back
    ws.undo(ws.head_seq - 1).map_err(|f| f.to_json()).unwrap();
    assert_eq!(field(&ws, "price").def.default, Some(json!("12.00")));
    // a default the field does not take, or one the changed definition no longer takes
    assert!(detail(&ws.fail(update(&ws, "price", json!({ "default": "1.234" })))).contains("default"));
    assert!(detail(&ws.fail(update(&ws, "title", json!({ "default": "x", "unique": true })))).contains("unique field has no default"));
    assert!(detail(&ws.fail(update(&ws, "tags", json!({ "max_items": 1 })))).contains("at most 1 options"));
    assert!(detail(&ws.fail(update(&ws, "note", json!({ "default": 3 })))).contains("expected string"));
    ws.ok(update(&ws, "title", json!({ "default": "untitled", "name": "Name" })));
    assert_eq!(field(&ws, "title").def.default, Some(json!("untitled")));
}

#[test]
fn add_field_takes_a_default_for_new_records_only() {
    let mut ws = table();
    ws.ok(insert("r1", json!({})));
    ws.ok(json!([{ "op": "table.add_field", "source_id": "t", "field": { "field_id": "n", "name": "N", "type": "number", "default": 7 } }]));
    assert!(!record(&ws, "r1").values.contains_key("n"), "existing records get a value only from backfill");
    ws.ok(insert("r2", json!({})));
    assert_eq!(record(&ws, "r2").values["n"], json!(7));
    let f = ws.fail(json!([{ "op": "table.add_field", "source_id": "t", "field": { "field_id": "m", "name": "M", "type": "number", "default": "7" } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
}

#[test]
fn a_migration_converts_the_default_like_a_value() {
    let mut ws = table();
    let migrate = |ws: &MemWorkspace, fid: &str, to: Value, on_failure: &str| {
        json!([{ "op": "table.migrate_field", "source_id": "t", "field_id": fid, "expect": { "rev": field(ws, fid).def_rev }, "to": to, "on_failure": on_failure }])
    };
    // select → text: the option's label
    ws.ok(migrate(&ws, "state", json!({ "type": "text" }), "reject"));
    assert_eq!(field(&ws, "state").def.default, Some(json!("甲")));
    ws.ok(insert("r1", json!({})));
    assert_eq!(record(&ws, "r1").values["state"], json!("甲"));
    // decimal → currency
    ws.ok(migrate(&ws, "price", json!({ "type": "currency", "currency": "USD" }), "reject"));
    assert_eq!(field(&ws, "price").def.default, Some(json!({ "amount": "9.50", "currency": "USD" })));
    // the report says what becomes of it; one that does not convert fails the migration, or is removed with `unset`
    let rev = record(&ws, "r1").value_rev("note");
    ws.ok(json!([{ "op": "table.unset_values", "source_id": "t", "values": [{ "record_id": "r1", "field_id": "note", "expect": { "rev": rev } }] }]));
    ws.ok(update(&ws, "note", json!({ "default": "soon" })));
    let ops = migrate(&ws, "note", json!({ "type": "date" }), "reject");
    let env = ws.env("alice", "human", None, false);
    let refused = plan_commit(&ws.store, &MemWorkspace::request(ops.clone()), &env, &Access::full("alice")).err().unwrap().to_json();
    assert!(detail(&refused).contains("the default cannot be converted"), "{refused}");
    assert_eq!(refused["errors"][0]["data"]["report"]["default"], json!({ "value": "soon", "converted": null, "convertible": false }));
    ws.ok(migrate(&ws, "note", json!({ "type": "date" }), "unset"));
    assert_eq!(field(&ws, "note").def.default, None);
    // undo of a migration puts the definition back, its default too
    let mut ws = table();
    let seq = ws.ok(migrate(&ws, "state", json!({ "type": "text" }), "reject"));
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(field(&ws, "state").def.default, Some(json!("a")));
}

#[test]
fn deleting_an_option_keeps_the_default_in_step() {
    let delete = |ws: &MemWorkspace, fid: &str, oid: &str, on_values: Value| {
        json!([{ "op": "table.delete_option", "source_id": "t", "field_id": fid, "option_id": oid, "on_values": on_values, "expect": { "rev": field(ws, fid).def_rev } }])
    };
    let mut ws = table();
    // the default is a use: reject_if_used keeps the option
    let f = ws.fail(delete(&ws, "state", "a", json!("reject_if_used")));
    assert!(detail(&f).contains("is the field's default"), "{f}");
    assert_eq!(f["errors"][0]["data"], json!({ "used_by": 0, "default": true }));
    ws.ok(delete(&ws, "state", "b", json!("reject_if_used")));
    // unset removes it from the default; replace_with puts the replacement there
    let seq = ws.ok(delete(&ws, "state", "a", json!("unset")));
    assert_eq!(field(&ws, "state").def.default, None);
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(field(&ws, "state").def.default, Some(json!("a")));
    assert!(field(&ws, "state").def.has_option("a"));
    ws.ok(delete(&ws, "state", "a", json!({ "replace_with": "c" })));
    assert_eq!(field(&ws, "state").def.default, Some(json!("c")));
    ws.ok(delete(&ws, "tags", "c", json!({ "replace_with": "b" })));
    assert_eq!(field(&ws, "tags").def.default, Some(json!(["a", "b"])));
    ws.ok(delete(&ws, "tags", "b", json!("unset")));
    let seq = ws.ok(delete(&ws, "tags", "a", json!("unset")));
    assert_eq!(field(&ws, "tags").def.default, None, "an emptied list is no default");
    ws.undo(seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(field(&ws, "tags").def.default, Some(json!(["a"])));
}
