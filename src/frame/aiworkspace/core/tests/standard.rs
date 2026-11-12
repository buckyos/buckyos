//! The standard business tables (`schemas/standard-tables.md`, 控件机制 TODO G8–G10): the record rules judge each
//! record once on the final values of the Commit, the table keeps its standard fields, and replay runs none of it.

use aiworkspace_core::access::Access;
use aiworkspace_core::filter::{run_query, QuerySpec};
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn detail(v: &Value) -> String {
    v["errors"][0]["detail"].as_str().unwrap_or("").to_string()
}

fn calendar_fields() -> Value {
    json!([
        { "field_id": "title", "name": "Title", "type": "text", "required": true },
        { "field_id": "all_day", "name": "All day", "type": "boolean", "nullable": true },
        { "field_id": "start", "name": "Start", "type": "datetime", "nullable": true },
        { "field_id": "end", "name": "End", "type": "datetime", "nullable": true },
        { "field_id": "start_date", "name": "Start date", "type": "date", "nullable": true },
        { "field_id": "end_date", "name": "End date", "type": "date", "nullable": true },
        { "field_id": "status", "name": "Status", "type": "select", "nullable": true, "options": [
            { "option_id": "confirmed", "label": "Confirmed" }, { "option_id": "tentative", "label": "Tentative" }, { "option_id": "cancelled", "label": "Cancelled" }] },
        { "field_id": "ext", "name": "Ext", "type": "json", "nullable": true },
        { "field_id": "note", "name": "Note", "type": "text" }
    ])
}

fn todo_fields() -> Value {
    json!([
        { "field_id": "title", "name": "Title", "type": "text", "required": true },
        { "field_id": "status", "name": "Status", "type": "select", "required": true, "options": [
            { "option_id": "todo", "label": "To do" }, { "option_id": "doing", "label": "Doing" }, { "option_id": "done", "label": "Done" }] },
        { "field_id": "due", "name": "Due", "type": "date", "nullable": true },
        { "field_id": "order", "name": "Order", "type": "text", "nullable": true },
        { "field_id": "done_at", "name": "Done at", "type": "datetime", "nullable": true },
        { "field_id": "ext", "name": "Ext", "type": "json", "nullable": true }
    ])
}

fn table(id: &str, schema_id: &str, fields: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_TABLE, "schema_id": schema_id, "parent_id": "data", "order_key": "b",
            "payload": { "title_field_id": "title", "fields": fields } })
}

fn insert(source: &str, id: &str, values: Value) -> Value {
    json!({ "op": "table.insert_records", "source_id": source, "records": [{ "record_id": id, "values": values }] })
}

fn calendar() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([table("cal", "buckyos.calendar@1", calendar_fields())]));
    ws
}

/// `table.set_values` of `(field, value)` on one record, each item expecting the cell's current rev.
fn set(ws: &MemWorkspace, source: &str, record: &str, values: &[(&str, Value)]) -> Value {
    let rec = &ws.store.records[&(source.to_string(), record.to_string())];
    let items: Vec<Value> = values.iter().map(|(f, v)| json!({ "record_id": record, "field_id": f, "value": v, "expect": { "rev": rec.value_rev(f) } })).collect();
    json!({ "op": "table.set_values", "source_id": source, "values": items })
}

fn unset(ws: &MemWorkspace, source: &str, record: &str, fields: &[&str]) -> Value {
    let rec = &ws.store.records[&(source.to_string(), record.to_string())];
    let items: Vec<Value> = fields.iter().map(|f| json!({ "record_id": record, "field_id": f, "expect": { "rev": rec.value_rev(f) } })).collect();
    json!({ "op": "table.unset_values", "source_id": source, "values": items })
}

fn value(ws: &MemWorkspace, source: &str, record: &str, field: &str) -> Option<Value> {
    ws.store.records[&(source.to_string(), record.to_string())].values.get(field).cloned()
}

#[test]
fn a_standard_table_needs_its_fields() {
    let mut ws = MemWorkspace::new();
    // every standard field, in its type (a select's options may differ); extra fields are free
    let mut fields = calendar_fields();
    fields.as_array_mut().unwrap().retain(|f| f["field_id"] != "end");
    let f = ws.fail(json!([table("cal", "buckyos.calendar@1", fields)]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
    assert!(detail(&f).contains("end"), "{f}");
    let mut fields = todo_fields();
    fields[2] = json!({ "field_id": "due", "name": "Due", "type": "datetime", "nullable": true });
    let f = ws.fail(json!([table("todo", "buckyos.todo@1", fields)]));
    assert!(detail(&f).contains("due must be date"), "{f}");
    let mut t = table("todo", "buckyos.todo@1", todo_fields());
    t["payload"].as_object_mut().unwrap().remove("title_field_id");
    let f = ws.fail(json!([t]));
    assert!(detail(&f).contains("title_field_id must be title"), "{f}");
    let mut fields = todo_fields();
    fields[1]["options"] = json!([{ "option_id": "open", "label": "Open" }, { "option_id": "done", "label": "Done" }]);
    ws.ok(json!([table("todo", "buckyos.todo@1", fields)]));
    // a table without a known schema_id has no rules
    ws.ok(json!([table("other", "example.thing@1", json!([{ "field_id": "title", "name": "T", "type": "text" }]))]));

    // a standard field is neither deleted nor retyped; other fields are free
    let mut ws = calendar();
    let f = ws.fail(json!([{ "op": "table.delete_field", "source_id": "cal", "field_id": "end", "expect": { "rev": 1 } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
    assert!(detail(&f).contains("cannot be deleted"), "{f}");
    let f = ws.fail(json!([{ "op": "table.migrate_field", "source_id": "cal", "field_id": "start", "expect": { "rev": 1 }, "to": { "type": "text" } }]));
    assert!(detail(&f).contains("stays datetime"), "{f}");
    let f = ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "cal", "keys": [{ "key": "title_field_id", "value": "note", "expect": { "rev": 1 } }] }]));
    assert!(detail(&f).contains("title_field_id must be title"), "{f}");
    ws.ok(json!([{ "op": "table.delete_field", "source_id": "cal", "field_id": "note", "expect": { "rev": 1 } }]));
}

#[test]
fn timed_and_all_day_events() {
    let mut ws = calendar();
    ws.ok(json!([insert("cal", "e1", json!({ "title": "standup", "start": "2026-11-12T09:00:00+08:00", "end": "2026-11-12T09:15:00+08:00" }))]));
    // end before start
    let f = ws.fail(json!([insert("cal", "e2", json!({ "title": "x", "start": "2026-11-12T10:00:00Z", "end": "2026-11-12T09:00:00Z" }))]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
    assert!(detail(&f).contains("end is before start"), "{f}");
    assert_eq!(f["errors"][0]["data"]["field_id"], "end");
    assert_eq!(f["errors"][0]["data"]["record_id"], "e2");
    let f = ws.fail(json!([set(&ws, "cal", "e1", &[("start", json!("2026-11-12T10:00:00+08:00"))])]));
    assert!(detail(&f).contains("end is before start"), "{f}");
    // moved together in one operation: judged on the final values
    ws.ok(json!([set(&ws, "cal", "e1", &[("start", json!("2026-11-12T10:00:00+08:00")), ("end", json!("2026-11-12T10:15:00+08:00"))])]));
    // a timed event needs its start and has no dates
    let f = ws.fail(json!([insert("cal", "e3", json!({ "title": "x", "end": "2026-11-12T09:00:00Z" }))]));
    assert!(detail(&f).contains("start is required on a timed event"), "{f}");
    let f = ws.fail(json!([insert("cal", "e3", json!({ "title": "x", "start": "2026-11-12T09:00:00Z", "start_date": "2026-11-12" }))]));
    assert!(detail(&f).contains("start_date must be absent"), "{f}");
    // an all-day event uses dates only
    let f = ws.fail(json!([insert("cal", "d1", json!({ "title": "trip", "all_day": true, "start": "2026-11-12T09:00:00Z", "start_date": "2026-11-12" }))]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
    assert!(detail(&f).contains("start must be absent on an all-day event"), "{f}");
    let f = ws.fail(json!([insert("cal", "d1", json!({ "title": "trip", "all_day": true }))]));
    assert!(detail(&f).contains("start_date is required on an all-day event"), "{f}");
    let f = ws.fail(json!([insert("cal", "d1", json!({ "title": "trip", "all_day": true, "start_date": "2026-11-14", "end_date": "2026-11-12" }))]));
    assert!(detail(&f).contains("end_date is before start_date"), "{f}");
    ws.ok(json!([insert("cal", "d1", json!({ "title": "trip", "all_day": true, "start_date": "2026-11-12", "end_date": "2026-11-14" }))]));
    // null is absence
    ws.ok(json!([insert("cal", "d2", json!({ "title": "holiday", "all_day": true, "start_date": "2026-11-12", "start": null, "end": null }))]));

    // timed → all-day over two operations of one Commit (the first alone breaks the rule), and its undo
    let before = (value(&ws, "cal", "e1", "start"), value(&ws, "cal", "e1", "end"));
    let seq = ws.ok(json!([
        set(&ws, "cal", "e1", &[("all_day", json!(true)), ("start_date", json!("2026-11-12"))]),
        unset(&ws, "cal", "e1", &["start", "end"])
    ]));
    let f = ws.fail(json!([set(&ws, "cal", "e1", &[("all_day", json!(false))])]));
    assert!(detail(&f).contains("start_date must be absent on a timed event"), "{f}");
    assert_eq!(f["errors"][0]["op_index"], 0);
    ws.undo(seq).unwrap_or_else(|e| panic!("undo: {}", e.to_json()));
    assert_eq!((value(&ws, "cal", "e1", "start"), value(&ws, "cal", "e1", "end")), before);
    assert_eq!(value(&ws, "cal", "e1", "start_date"), None);

    // the failing record's last operation is named
    let f = ws.fail(json!([
        insert("cal", "e4", json!({ "title": "x", "start": "2026-11-12T09:00:00Z" })),
        { "op": "table.set_values", "source_id": "cal", "values": [{ "record_id": "e4", "field_id": "end", "value": "2026-11-12T08:00:00Z", "expect": { "rev": 0 } }] }
    ]));
    assert_eq!(f["errors"][0]["op_index"], 1, "{f}");
    // a deleted record is not judged
    ws.ok(json!([{ "op": "table.delete_records", "source_id": "cal", "records": [{ "record_id": "d2", "expect": { "rev": ws.store.records[&("cal".to_string(), "d2".to_string())].rev } }] }]));
}

#[test]
fn recurrence_in_ext() {
    let mut ws = calendar();
    let event = |ext: Value| json!({ "title": "x", "start": "2026-11-12T09:00:00Z", "end": "2026-11-12T10:00:00Z", "ext": ext });
    ws.ok(json!([insert("cal", "s1", event(json!({ "rrule": "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "exdate": ["2026-11-13T09:00:00Z"], "tz": "Asia/Shanghai",
        "reminders": [{ "before_min": 10 }, { "before_min": 1440, "channel": "msg" }], "busy": true, "color": "#3366ff", "location": "Room 1", "anything": { "kept": [1] } })))]));
    let f = ws.fail(json!([insert("cal", "s2", event(json!({ "rrule": "FREQ=HOURLY" })))]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
    assert!(detail(&f).contains("ext.rrule: FREQ=HOURLY"), "{f}");
    let f = ws.fail(json!([insert("cal", "s2", event(json!({ "rrule": "FREQ=DAILY;BYHOUR=9" })))]));
    assert!(detail(&f).contains("BYHOUR"), "{f}");
    // an override of one occurrence is not a series
    let f = ws.fail(json!([insert("cal", "o1", event(json!({ "rrule": "FREQ=DAILY", "recurrence_id": { "master": "s1", "original": "2026-11-16T09:00:00Z" } })))]));
    assert!(detail(&f).contains("recurrence_id"), "{f}");
    ws.ok(json!([insert("cal", "o1", event(json!({ "recurrence_id": { "master": "s1", "original": "2026-11-16T09:00:00Z" } })))]));
    // the shapes of the keys with a meaning
    for (ext, part) in [
        (json!({ "exdate": ["2026-11-13"] }), "ext.exdate"),
        (json!({ "tz": "Asia Shanghai" }), "ext.tz"),
        (json!({ "color": "blue" }), "ext.color"),
        (json!({ "busy": "yes" }), "ext.busy"),
        (json!({ "reminders": [{ "before_min": 40321 }] }), "ext.reminders"),
        (json!({ "rule_text": "x".repeat(257) }), "ext.rule_text"),
        (json!({ "occurrences": ["soon"] }), "ext.occurrences"),
        (json!({ "covered_until": "2026-13-01" }), "ext.covered_until"),
        (json!({ "maintenance": "ok" }), "ext.maintenance"),
        (json!({ "recurrence_id": { "master": "s1" } }), "ext.recurrence_id"),
    ] {
        let f = ws.fail(json!([insert("cal", "s3", event(ext.clone()))]));
        assert!(detail(&f).contains(part), "{ext}: {f}");
    }
    // an all-day series excludes dates
    ws.ok(json!([insert("cal", "a1", json!({ "title": "x", "all_day": true, "start_date": "2026-11-12", "ext": { "rrule": "FREQ=YEARLY", "exdate": ["2027-11-12"] } }))]));
    // json values are compared only for emptiness
    let src = ws.store.entities["cal"].clone();
    let q = |filter: Value| run_query(&ws.store, &src, &QuerySpec { filter: Some(filter), limit: 100, ..Default::default() });
    assert_eq!(q(json!({ "op": "cmp", "field_id": "ext", "operator": "is_not_empty" })).unwrap()["rows"].as_array().unwrap().len(), 3);
    let e = q(json!({ "op": "cmp", "field_id": "ext", "operator": "eq", "value": {} })).unwrap_err();
    assert_eq!(e.code.as_str(), "INVALID_OPERATION");
    let e = run_query(&ws.store, &src, &QuerySpec { sorts: vec![json!({ "field_id": "ext", "direction": "asc" })], limit: 10, ..Default::default() }).unwrap_err();
    assert_eq!(e.code.as_str(), "INVALID_OPERATION");
}

#[test]
fn todo_records() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([table("todo", "buckyos.todo@1", todo_fields())]));
    ws.ok(json!([insert("todo", "t1", json!({ "title": "buy milk", "status": "todo", "order": "a1", "ext": { "priority": "high", "tags": ["home"], "source": { "from": "chat" } } }))]));
    let f = ws.fail(json!([insert("todo", "t2", json!({ "title": "x", "status": "todo", "order": "" }))]));
    assert!(detail(&f).contains("order"), "{f}");
    let f = ws.fail(json!([insert("todo", "t2", json!({ "title": "x", "status": "todo", "ext": { "priority": "asap" } }))]));
    assert!(detail(&f).contains("ext.priority"), "{f}");
    let f = ws.fail(json!([insert("todo", "t2", json!({ "title": "x", "status": "todo", "ext": { "tags": ["x".repeat(65)] } }))]));
    assert!(detail(&f).contains("ext.tags"), "{f}");
    let f = ws.fail(json!([insert("todo", "t2", json!({ "title": "x", "status": "todo", "ext": { "$formula": { "lang": "aiws", "version": "1", "expr": "1" } } }))]));
    assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
    // moving a card writes its own order key and status
    ws.ok(json!([set(&ws, "todo", "t1", &[("status", json!("done")), ("order", json!("a2")), ("done_at", json!("2026-11-12T08:00:00Z"))])]));
}

#[test]
fn replay_runs_no_standard_rule() {
    let mut ws = calendar();
    ws.ok(json!([insert("cal", "e1", json!({ "title": "x", "start": "2026-11-12T09:00:00Z" }))]));
    let replay = |ws: &mut MemWorkspace, ops: Value| {
        let mut env = ws.env("alice", "human", None, false);
        env.schema_checks = false;
        ws.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(ops)).map_err(|f| f.to_json())
    };
    let bad = json!([set(&ws, "cal", "e1", &[("end", json!("2026-11-12T08:00:00Z")), ("start_date", json!("2026-11-12"))])]);
    assert_eq!(code(&ws.fail(bad.clone())), "INVALID_SCHEMA");
    replay(&mut ws, bad).expect("replay keeps an accepted record whatever the business rules say");
    replay(&mut ws, json!([insert("cal", "e2", json!({ "title": "x", "all_day": true, "start": "2026-11-12T09:00:00Z", "ext": { "rrule": "FREQ=HOURLY" } }))])).unwrap();
    replay(&mut ws, json!([{ "op": "table.delete_field", "source_id": "cal", "field_id": "end", "expect": { "rev": 1 } }])).unwrap();
    replay(&mut ws, json!([table("cal2", "buckyos.calendar@1", json!([{ "field_id": "title", "name": "T", "type": "text" }]))])).unwrap();
}
