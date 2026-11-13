//! The photo vote (控件机制 TODO §0.4, G20, G21): records owned by their submitter, `multi_select` `max_items`, and
//! `doc.query` groups that count each option of a list, ordered by count and cut to the first N.

use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::filter::{group_params, run_query, QuerySpec};
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn detail(v: &Value) -> String {
    v["errors"][0]["detail"].as_str().unwrap_or("").to_string()
}

fn photos() -> Value {
    json!((1..=8).map(|i| json!({ "option_id": format!("p{i}"), "label": format!("P{i}") })).collect::<Vec<_>>())
}

fn votes() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([{ "op": "entity.create", "entity_id": "votes", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b",
        "payload": { "fields": [
            { "field_id": "who", "name": "Who", "type": "text", "submitter": true },
            { "field_id": "picks", "name": "Picks", "type": "multi_select", "max_items": 3, "options": photos() },
            { "field_id": "note", "name": "Note", "type": "text" }] } }]));
    ws
}

fn voter(name: &str) -> Access {
    Access::granted(name, CapSet(Cap::Read as u16 | Cap::Append as u16), [])
}

fn run(ws: &mut MemWorkspace, who: &Access, internal: bool, ops: Value) -> Result<u64, Value> {
    let env = ws.env(&who.principal, "human", None, internal);
    ws.commit_with(who, &env, &MemWorkspace::request(ops)).map_err(|f| f.to_json())
}

/// Like [`run`], with operations built from the state they apply to.
fn by(ws: &mut MemWorkspace, who: &Access, build: impl FnOnce(&MemWorkspace) -> Value) -> Result<u64, Value> {
    let ops = build(ws);
    run(ws, who, false, ops)
}

fn insert(id: &str, values: Value) -> Value {
    json!([{ "op": "table.insert_records", "source_id": "votes", "records": [{ "record_id": id, "values": values }] }])
}

fn record(ws: &MemWorkspace, id: &str) -> RecordRow {
    ws.store.records[&("votes".to_string(), id.to_string())].clone()
}

fn set(ws: &MemWorkspace, id: &str, field: &str, value: Value) -> Value {
    let rev = record(ws, id).value_rev(field);
    json!([{ "op": "table.set_values", "source_id": "votes", "values": [{ "record_id": id, "field_id": field, "value": value, "expect": { "rev": rev } }] }])
}

#[test]
fn a_submitter_field_is_text_unique_and_one_per_table() {
    let mut ws = MemWorkspace::new();
    let table = |id: &str, fields: Value| json!([{ "op": "entity.create", "entity_id": id, "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b", "payload": { "fields": fields } }]);
    let f = ws.fail(table("t1", json!([{ "field_id": "n", "name": "N", "type": "number", "submitter": true }])));
    assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
    let f = ws.fail(table("t2", json!([{ "field_id": "a", "name": "A", "type": "text", "submitter": true }, { "field_id": "b", "name": "B", "type": "text", "submitter": true }])));
    assert_eq!((code(&f), detail(&f).contains("at most one submitter")), ("INVALID_SCHEMA", true), "{f}");
    for bad in [json!({ "field_id": "t", "name": "T", "type": "text", "max_items": 2 }), json!({ "field_id": "m", "name": "M", "type": "multi_select", "max_items": 0, "options": photos() }),
                json!({ "field_id": "m", "name": "M", "type": "multi_select", "max_items": 1001, "options": photos() })] {
        assert_eq!(code(&ws.fail(table("t3", json!([bad])))), "INVALID_SCHEMA");
    }
    // a record property has no table to be unique in
    let f = ws.fail(json!([{ "op": "entity.create", "entity_id": "rec", "type_id": TYPE_RECORD, "parent_id": "data", "order_key": "c",
        "payload": { "schema": { "properties": [{ "key": "who", "name": "Who", "type": "text", "submitter": true }] }, "props": {} } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
}

#[test]
fn each_voter_owns_one_record_the_service_fills_in() {
    let mut ws = votes();
    let (bob, carol) = (voter("bob"), voter("carol"));
    run(&mut ws, &bob, false, insert("b1", json!({ "picks": ["p1", "p2", "p3"] }))).expect("bob votes");
    assert_eq!(record(&ws, "b1").values["who"], "bob");
    // the submitter is the service's to write, and one record per submitter
    let f = run(&mut ws, &bob, false, insert("b2", json!({ "picks": ["p4"] }))).unwrap_err();
    assert_eq!((code(&f), f["sub_code"].as_str(), &f["errors"][0]["data"]["record_id"]), ("INVALID_OPERATION", Some("SUBMITTER_EXISTS"), &json!("b1")), "{f}");
    let f = run(&mut ws, &carol, false, insert("c0", json!({ "who": "bob", "picks": ["p4"] }))).unwrap_err();
    assert_eq!((code(&f), detail(&f)), ("INVALID_OPERATION", "the submitter is filled by the service".to_string()), "{f}");
    let f = run(&mut ws, &carol, false, json!([{ "op": "table.insert_records", "source_id": "votes", "records": [
        { "record_id": "c1", "values": { "picks": ["p1"] } }, { "record_id": "c2", "values": { "picks": ["p2"] } }] }])).unwrap_err();
    assert_eq!(f["sub_code"], "SUBMITTER_EXISTS", "two records in one insert: {f}");
    // at most three photos
    let f = run(&mut ws, &carol, false, insert("c1", json!({ "picks": ["p1", "p2", "p3", "p4"] }))).unwrap_err();
    assert_eq!((code(&f), detail(&f).contains("at most 3 options")), ("INVALID_SCHEMA", true), "{f}");
    run(&mut ws, &carol, false, insert("c1", json!({ "picks": ["p1", "p4"] }))).expect("carol votes");
    // bob changes his own vote, never carol's, never the submitter
    by(&mut ws, &bob, |ws| set(ws, "b1", "picks", json!(["p1", "p4", "p5"]))).expect("bob changes his vote");
    assert_eq!(record(&ws, "b1").values["picks"], json!(["p1", "p4", "p5"]));
    assert_eq!(code(&by(&mut ws, &bob, |ws| set(ws, "b1", "picks", json!(["p1", "p2", "p3", "p4"]))).unwrap_err()), "INVALID_SCHEMA");
    assert_eq!(code(&by(&mut ws, &bob, |ws| set(ws, "c1", "picks", json!(["p1"]))).unwrap_err()), "PERMISSION_DENIED");
    assert_eq!(code(&by(&mut ws, &bob, |ws| set(ws, "b1", "who", json!("carol"))).unwrap_err()), "INVALID_OPERATION");
    let unset = |id: &str, field: &str, rev: u64| json!([{ "op": "table.unset_values", "source_id": "votes", "values": [{ "record_id": id, "field_id": field, "expect": { "rev": rev } }] }]);
    assert_eq!(code(&by(&mut ws, &bob, |ws| unset("b1", "who", record(ws, "b1").value_rev("who"))).unwrap_err()), "INVALID_OPERATION");
    assert_eq!(code(&by(&mut ws, &bob, |ws| unset("c1", "picks", record(ws, "c1").value_rev("picks"))).unwrap_err()), "PERMISSION_DENIED");
    by(&mut ws, &bob, |ws| unset("b1", "picks", record(ws, "b1").value_rev("picks"))).expect("bob withdraws his picks");
    // delete: his own only; afterwards he may vote again
    let del = |id: &str, rev: u64| json!([{ "op": "table.delete_records", "source_id": "votes", "records": [{ "record_id": id, "expect": { "rev": rev } }] }]);
    assert_eq!(code(&by(&mut ws, &bob, |ws| del("c1", record(ws, "c1").rev)).unwrap_err()), "PERMISSION_DENIED");
    by(&mut ws, &bob, |ws| del("b1", record(ws, "b1").rev)).expect("bob deletes his vote");
    run(&mut ws, &bob, false, insert("b3", json!({ "picks": ["p2"] }))).expect("bob votes again");
    // `update` edits anyone's other fields; nobody writes the submitter; the owner submits like anyone else
    let alice = Access::full("alice");
    by(&mut ws, &alice, |ws| set(ws, "c1", "note", json!("seen"))).expect("an editor edits other fields");
    assert_eq!(code(&by(&mut ws, &alice, |ws| set(ws, "c1", "who", json!("alice"))).unwrap_err()), "INVALID_OPERATION");
    run(&mut ws, &alice, false, insert("a1", json!({ "picks": ["p3"] }))).expect("alice votes");
    assert_eq!(record(&ws, "a1").values["who"], "alice");
    assert_eq!(run(&mut ws, &alice, false, insert("a2", json!({}))).unwrap_err()["sub_code"], "SUBMITTER_EXISTS");
    // undo, import and replay keep what was recorded (and uniqueness still holds there)
    run(&mut ws, &alice, true, insert("d1", json!({ "who": "dave", "picks": ["p5"] }))).expect("an import keeps the submitter");
    assert_eq!(record(&ws, "d1").values["who"], "dave");
    assert_eq!(run(&mut ws, &alice, true, insert("d2", json!({ "who": "dave" }))).unwrap_err()["sub_code"], "UNIQUE_VIOLATION");
    // read alone writes nothing; append alone on a table without a submitter still edits nothing
    let reader = Access::granted("erin", CapSet(Cap::Read as u16), []);
    assert_eq!(code(&run(&mut ws, &reader, false, insert("e1", json!({}))).unwrap_err()), "PERMISSION_DENIED");
    run(&mut ws, &alice, false, json!([{ "op": "entity.create", "entity_id": "plain", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "c",
        "payload": { "fields": [{ "field_id": "t", "name": "T", "type": "text" }] } }])).unwrap();
    run(&mut ws, &bob, false, json!([{ "op": "table.insert_records", "source_id": "plain", "records": [{ "record_id": "x", "values": { "t": "a" } }] }])).unwrap();
    let rev = ws.store.records[&("plain".to_string(), "x".to_string())].value_rev("t");
    let f = run(&mut ws, &bob, false, json!([{ "op": "table.set_values", "source_id": "plain", "values": [{ "record_id": "x", "field_id": "t", "value": "b", "expect": { "rev": rev } }] }])).unwrap_err();
    assert_eq!(code(&f), "PERMISSION_DENIED");
}

#[test]
fn the_submitter_flag_changes_only_on_an_empty_table() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([{ "op": "entity.create", "entity_id": "votes", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b",
        "payload": { "fields": [{ "field_id": "who", "name": "Who", "type": "text" }, { "field_id": "n", "name": "N", "type": "text" }] } }]));
    let def_rev = |ws: &MemWorkspace, f: &str| ws.store.fields[&("votes".to_string(), f.to_string())].def_rev;
    let turn = |ws: &MemWorkspace, f: &str, on: bool| json!([{ "op": "table.update_field", "source_id": "votes", "field_id": f, "changes": { "submitter": on }, "expect": { "rev": def_rev(ws, f) } }]);
    ws.ok(turn(&ws, "who", true));
    assert_eq!(code(&ws.fail(turn(&ws, "n", true))), "INVALID_SCHEMA", "one submitter per table");
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "votes", "records": [{ "record_id": "r1", "values": {} }] }]));
    assert_eq!(ws.store.records[&("votes".to_string(), "r1".to_string())].values["who"], "alice");
    let f = ws.fail(turn(&ws, "who", false));
    assert_eq!((f["status"].as_str(), code(&f)), (Some("conflict"), "SCHEMA_CONFLICT"), "{f}");
    let f = ws.fail(json!([{ "op": "table.add_field", "source_id": "votes", "field": { "field_id": "w2", "name": "W2", "type": "text", "submitter": true } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
    let rev = ws.store.fields[&("votes".to_string(), "who".to_string())].def_rev;
    let rec = ws.store.records[&("votes".to_string(), "r1".to_string())].rev;
    ws.ok(json!([{ "op": "table.delete_records", "source_id": "votes", "records": [{ "record_id": "r1", "expect": { "rev": rec } }] }]));
    ws.ok(json!([{ "op": "table.update_field", "source_id": "votes", "field_id": "who", "changes": { "submitter": false }, "expect": { "rev": rev } }]));
    // a new submitter field on a table with records would own none of them
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "votes", "records": [{ "record_id": "r2", "values": {} }] }]));
    let f = ws.fail(json!([{ "op": "table.add_field", "source_id": "votes", "field": { "field_id": "w2", "name": "W2", "type": "text", "submitter": true } }]));
    assert_eq!(code(&f), "SCHEMA_CONFLICT", "{f}");
}

#[test]
fn lowering_max_items_rescans_the_records() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([{ "op": "entity.create", "entity_id": "votes", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b",
        "payload": { "fields": [{ "field_id": "picks", "name": "Picks", "type": "multi_select", "options": photos() }] } }]));
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "votes", "records": [
        { "record_id": "r1", "values": { "picks": ["p1", "p2", "p3", "p4"] } }, { "record_id": "r2", "values": { "picks": ["p1"] } }] }]));
    let limit = |ws: &MemWorkspace, n: Value| {
        let rev = ws.store.fields[&("votes".to_string(), "picks".to_string())].def_rev;
        json!([{ "op": "table.update_field", "source_id": "votes", "field_id": "picks", "changes": { "max_items": n }, "expect": { "rev": rev } }])
    };
    let f = ws.fail(limit(&ws, json!(3)));
    assert_eq!((code(&f), &f["errors"][0]["data"]["violating_records"]), ("INVALID_OPERATION", &json!(["r1"])), "{f}");
    assert_eq!(code(&ws.fail(limit(&ws, json!(0)))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(limit(&ws, json!("3")))), "INVALID_OPERATION");
    let type_rev = ws.store.fields[&("votes".to_string(), "picks".to_string())].type_rev;
    let seq = ws.ok(limit(&ws, json!(4)));
    assert_eq!(ws.store.fields[&("votes".to_string(), "picks".to_string())].type_rev, seq, "tightening bumps the type");
    assert_ne!(type_rev, seq);
    ws.ok(limit(&ws, json!(6)));
    ws.ok(limit(&ws, Value::Null));
    assert_eq!(ws.store.fields[&("votes".to_string(), "picks".to_string())].def.max_items, None);
    // undo puts the previous limit back
    ws.undo(ws.head_seq).map_err(|f| f.to_json()).unwrap();
    assert_eq!(ws.store.fields[&("votes".to_string(), "picks".to_string())].def.max_items, Some(6));
}

#[test]
fn groups_count_each_option_of_a_list_by_count() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([{ "op": "entity.create", "entity_id": "votes", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b",
        "payload": { "fields": [
            { "field_id": "picks", "name": "Picks", "type": "multi_select", "options": photos() },
            { "field_id": "ext", "name": "Ext", "type": "json", "nullable": true }] } }]));
    let rows = [json!({ "picks": ["p1", "p2"] }), json!({ "picks": ["p1"] }), json!({ "picks": [] }), json!({}), json!({ "picks": ["p2", "p3"] }), json!({ "picks": ["p1"] })];
    let records: Vec<Value> = rows.iter().enumerate().map(|(i, v)| json!({ "record_id": format!("r{i}"), "values": v })).collect();
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "votes", "records": records }]));
    let src = ws.store.entities["votes"].clone();
    let groups = |params: Value| -> Result<Value, String> {
        let mut spec = QuerySpec { group: params["group"].as_str().map(str::to_string), limit: 1, ..Default::default() };
        group_params(&params, &mut spec).map_err(|e| e.code.as_str().to_string())?;
        run_query(&ws.store, &src, &spec).map(|r| r["groups"].clone()).map_err(|e| e.code.as_str().to_string())
    };
    let pairs = |g: Value| g.as_array().unwrap().iter().map(|x| (x["key"].as_str().unwrap().to_string(), x["count"].as_u64().unwrap())).collect::<Vec<_>>();
    let p = |k: &str, n: u64| (k.to_string(), n);
    // without ordering: by key, as before; empty and absent lists under ""
    assert_eq!(pairs(groups(json!({ "group": "picks" })).unwrap()), vec![p("", 2), p("p1", 3), p("p2", 2), p("p3", 1)]);
    // by count, ties by key; then the first N
    assert_eq!(pairs(groups(json!({ "group": "picks", "group_order": "count_desc" })).unwrap()), vec![p("p1", 3), p("", 2), p("p2", 2), p("p3", 1)]);
    assert_eq!(pairs(groups(json!({ "group": "picks", "group_order": "count_desc", "group_limit": 2 })).unwrap()), vec![p("p1", 3), p("", 2)]);
    assert_eq!(pairs(groups(json!({ "group": "picks", "group_limit": 1 })).unwrap()), vec![p("", 2)]);
    // a filter narrows what is counted
    let mut spec = QuerySpec { group: Some("picks".into()), group_by_count: true, group_limit: Some(3), limit: 1, ..Default::default() };
    spec.filter = Some(json!({ "op": "cmp", "field_id": "picks", "operator": "is_not_empty" }));
    let g = run_query(&ws.store, &src, &spec).unwrap()["groups"].clone();
    assert_eq!(pairs(g), vec![p("p1", 3), p("p2", 2), p("p3", 1)]);
    // parameters and group types
    assert_eq!(groups(json!({ "group": "picks", "group_order": "count_asc" })).unwrap_err(), "INVALID_OPERATION");
    assert_eq!(groups(json!({ "group": "picks", "group_limit": 0 })).unwrap_err(), "LIMIT_EXCEEDED");
    assert_eq!(groups(json!({ "group": "picks", "group_limit": 1001 })).unwrap_err(), "LIMIT_EXCEEDED");
    assert_eq!(groups(json!({ "group": "ext" })).unwrap_err(), "INVALID_OPERATION");
}
