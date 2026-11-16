//! Data sources (AI Native 余下任务 §7.2, B0): a Source may say what its data is (`def`), a Zone source reads the
//! owner's home and names its primary key, a source without a data version is pending (no valid dependency: a
//! definition, never an empty table), only the refresh writes the data version, only the owner changes a Source, and a
//! table with a `source.def` registers a user schema the other tables carrying it must follow.

use aiworkspace_core::access::{Access, CapSet};
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::{body, freshness, meta, search};
use serde_json::{json, Value};

fn full() -> Access {
    Access::full("alice")
}

fn fields() -> Value {
    json!([{ "field_id": "symbol", "name": "Symbol", "type": "text" }, { "field_id": "price", "name": "Price", "type": "number" }])
}

fn zone_source() -> Value {
    json!({ "kind": "zone", "path": "/home/alice/sync/quotes.json", "refresh": { "interval_s": 900 },
            "def": { "meaning": "Daily closing prices", "primary_key": ["symbol"], "freshness": { "max_age_s": 3600 }, "limit": 100 } })
}

fn create(id: &str, key: &str, payload: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": key, "payload": payload })
}

fn sub(f: &Value) -> &str {
    f["errors"][0]["sub_code"].as_str().or_else(|| f["errors"][0]["code"].as_str()).unwrap_or("")
}

fn detail(f: &Value) -> String {
    f["errors"][0]["detail"].as_str().unwrap_or("").to_string()
}

/// Commit as the refresh does (origin `source`, internal, as the owner).
fn refresh(ws: &mut MemWorkspace, ops: Value) {
    let env = ws.env("alice", "source", None, true);
    ws.commit_with(&full(), &env, &MemWorkspace::request(ops)).unwrap_or_else(|f| panic!("{}", f.to_json()));
}

#[test]
fn source_definitions_and_zone_paths() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([create("quotes", "a", json!({ "fields": fields(), "source": zone_source() }))]));
    let bad = |source: Value| json!([create("x", "b", json!({ "fields": fields(), "source": source }))]);
    for (source, why) in [
        (json!({ "kind": "zone", "path": "/home/alice/../bob/q.json", "def": { "meaning": "m", "primary_key": ["symbol"] } }), "without . or .."),
        (json!({ "kind": "zone", "path": "home/alice/q.json", "def": { "meaning": "m", "primary_key": ["symbol"] } }), "absolute Zone path"),
        (json!({ "kind": "zone", "path": "/data/alice/q.json", "def": { "meaning": "m", "primary_key": ["symbol"] } }), "owner's home"),
        (json!({ "kind": "zone", "path": "/home/alice/q.json", "url": "https://example.invalid/", "def": { "meaning": "m", "primary_key": ["symbol"] } }), "zone source has"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "refresh": { "interval_s": 600 } }), "at least 900"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "def": { "origin": "x" } }), "meaning"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "def": { "meaning": "m", "primary_key": ["a", "b", "c", "d", "e"] } }), "1 to 4"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "def": { "meaning": "m", "primary_key": ["a", "a"] } }), "distinct"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "def": { "meaning": "m", "limit": 0 } }), "limit"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "def": { "meaning": "m", "freshness": { "max_age_s": 0 } } }), "freshness"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "def": { "meaning": "m", "schema": {} } }), "unknown key"),
    ] {
        let f = ws.fail(bad(source.clone()));
        assert!(detail(&f).contains(why), "{source}: {f}");
    }
    // a session feed carries no definition
    assert!(detail(&ws.fail(bad(json!({ "kind": "session", "feed": "tasks", "def": { "meaning": "m" } })))).contains("session source"));
}

#[test]
fn a_source_without_a_data_version_is_pending() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([create("quotes", "a", json!({ "fields": fields(), "source": zone_source() })), create("own", "b", json!({ "fields": fields() }))]));
    let e = ws.store.entity("quotes").unwrap().unwrap();
    let m = meta::meta(&ws.store, &full(), &e, None).unwrap();
    assert_eq!((m["data"]["mode"].as_str(), m["data"]["state"].as_str(), m["body"]["state"].as_str()), (Some("synced"), Some("pending"), Some("pending")), "{m}");
    assert_eq!(m["data"]["source"]["path"], "/home/alice/sync/quotes.json");
    assert_eq!(m["data"]["def"]["primary_key"], json!(["symbol"]));
    assert!(m["data"]["version"].is_null() && m["data"]["source"].get("refreshed_at").is_none());
    // the body is the definition, never an empty table
    let b = body::read_body(&ws.store, &full(), &e, &json!({}), None).unwrap();
    let text = b.to_string();
    assert!(text.contains("Daily closing prices") && text.contains("field Price: number") && text.contains("primary key: symbol"), "{b}");
    // a text query covers its name and definition only, and says it skipped the rows
    let never = || false;
    let q = search::text_query(&ws.store, &full(), &json!({ "text": "Daily closing" }), ws.head_seq, None, &never).unwrap();
    assert!(q["coverage"]["skipped"].as_array().unwrap().iter().any(|s| s["entity_id"] == "quotes" && s["reason"] == "pending"), "{q}");
    // a local table has no data state
    assert!(meta::meta(&ws.store, &full(), &ws.store.entity("own").unwrap().unwrap(), None).unwrap()["data"].get("state").is_none());

    // the data version is the refresh's: nobody else writes it, and with it the source is valid
    let version = json!({ "synced_at": "2026-11-15T08:01:12.000Z", "business_time": "2026-11-15T08:00:00Z", "program": { "name": "quotes", "version": "3" }, "records": 0 });
    let f = ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "quotes", "keys": [{ "key": "data_version", "value": version, "expect": { "rev": 0 } }] }]));
    assert_eq!(sub(&f), "SOURCE_READ_ONLY", "{f}");
    let f = ws.fail(json!([{ "op": "entity.set_keys", "entity_id": "own", "keys": [{ "key": "data_version", "value": version, "expect": { "rev": 0 } }] }]));
    assert!(detail(&f).contains("entity with a Source"), "{f}");
    refresh(&mut ws, json!([{ "op": "entity.set_keys", "entity_id": "quotes", "keys": [{ "key": "data_version", "value": version, "expect": { "rev": 0 } }] }]));
    let e = ws.store.entity("quotes").unwrap().unwrap();
    let m = meta::meta(&ws.store, &full(), &e, None).unwrap();
    assert_eq!((m["data"]["state"].as_str(), m["body"]["state"].as_str(), m["data"]["version"]["program"]["version"].as_str()), (Some("valid"), Some("full"), Some("3")), "{m}");
    let f = MemWorkspace::request(json!([{ "op": "entity.set_keys", "entity_id": "quotes", "keys": [{ "key": "data_version", "value": { "records": 1 }, "expect": "any" }] }]));
    let env = ws.env("alice", "source", None, true);
    assert!(ws.commit_with(&full(), &env, &f).is_err(), "a data version names when it was synced");
}

#[test]
fn a_wish_on_a_pending_source_waits_for_data() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([create("quotes", "a", json!({ "fields": fields(), "source": zone_source() }))]));
    ws.ok(json!([{ "op": "entity.create", "entity_id": "w", "type_id": TYPE_WISH, "parent_id": DATA_ID, "order_key": "w",
                   "payload": { "prompt": "top movers", "executor": "xllm", "inputs": [{ "entity_id": "quotes", "name": "quotes", "label": "Quotes" }] } }]));
    let f = freshness::entity_freshness(&ws.store, &full(), "w").unwrap();
    assert_eq!(f["input_problems"], json!([{ "entity_id": "quotes", "reason": "pending", "input": "quotes", "label": "Quotes" }]), "{f}");
    assert_eq!(freshness::pending_inputs(&ws.store, &ws.store.entity("w").unwrap().unwrap().payload).unwrap().len(), 1);
    // with an analysis and a macro in place, the main state is waiting for data
    let mut f = f.clone();
    f["analysis_status"] = json!("ready");
    f["needs_analysis"] = json!(false);
    f["status"] = json!("none");
    f["needs_coding"] = Value::Null;
    assert_eq!(freshness::main_status(&f, &json!({}))["main"], "waiting_data");
}

#[test]
fn only_the_owner_changes_a_source() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([create("t", "a", json!({ "fields": fields() }))]));
    let editor = Access::uniform("bob", CapSet::ALL);
    let as_bob = |ws: &mut MemWorkspace, ops: Value| {
        let env = ws.env("bob", "human", None, false);
        ws.commit_with(&editor, &env, &MemWorkspace::request(ops)).map_err(|f| f.to_json())
    };
    let set = json!([{ "op": "entity.set_keys", "entity_id": "t", "keys": [{ "key": "source", "value": { "kind": "url", "url": "https://example.invalid/t.csv" }, "expect": { "rev": 0 } }] }]);
    let f = as_bob(&mut ws, set.clone()).unwrap_err();
    assert_eq!(f["errors"][0]["code"], "PERMISSION_DENIED", "{f}");
    let f = as_bob(&mut ws, json!([create("u", "b", json!({ "fields": fields(), "source": { "kind": "url", "url": "https://example.invalid/u.csv" } }))])).unwrap_err();
    assert_eq!(f["errors"][0]["code"], "PERMISSION_DENIED", "{f}");
    ws.ok(set);
    let f = as_bob(&mut ws, json!([{ "op": "entity.unset_keys", "entity_id": "t", "keys": [{ "key": "source", "expect": "any" }] }])).unwrap_err();
    assert_eq!(f["errors"][0]["code"], "PERMISSION_DENIED", "{f}");
    // the presentation keys stay an editor's
    as_bob(&mut ws, json!([{ "op": "entity.set_keys", "entity_id": "t", "keys": [{ "key": "description", "value": "prices", "expect": { "rev": 0 } }] }])).unwrap();
}

#[test]
fn a_table_with_a_definition_registers_its_schema() {
    let mut ws = MemWorkspace::new();
    let with_schema = |id: &str, key: &str, schema: &str, payload: Value| {
        let mut op = create(id, key, payload);
        op["schema_id"] = json!(schema);
        op
    };
    // a buckyos. id is a built-in one; the id has a shape
    assert!(detail(&ws.fail(json!([with_schema("q", "a", "buckyos.quotes@1", json!({ "fields": fields(), "source": zone_source() }))]))).contains("built-in"));
    assert!(detail(&ws.fail(json!([with_schema("q", "a", "quotes@1", json!({ "fields": fields(), "source": zone_source() }))]))).contains("<namespace>.<name>@<major>"));
    // the first table carrying it with a source.def defines it
    ws.ok(json!([with_schema("q", "a", "acme.quotes@1", json!({ "fields": fields(), "source": zone_source() }))]));
    let f = ws.fail(json!([with_schema("out", "b", "acme.quotes@1", json!({ "fields": [{ "field_id": "symbol", "name": "Symbol", "type": "text" }] }))]));
    assert!(detail(&f).contains("needs the field price"), "{f}");
    let f = ws.fail(json!([with_schema("out", "b", "acme.quotes@1", json!({ "fields": [{ "field_id": "symbol", "name": "S", "type": "text" }, { "field_id": "price", "name": "P", "type": "text" }] }))]));
    assert!(detail(&f).contains("must be number"), "{f}");
    let mut more = fields();
    more.as_array_mut().unwrap().push(json!({ "field_id": "rank", "name": "Rank", "type": "number" }));
    ws.ok(json!([with_schema("out", "b", "acme.quotes@1", json!({ "fields": more }))]));
    // a follower keeps the schema's fields; its own fields and the definer's are free
    let rev = ws.store.field("out", "price").unwrap().unwrap().def_rev;
    let f = ws.fail(json!([{ "op": "table.delete_field", "source_id": "out", "field_id": "price", "expect": { "rev": rev } }]));
    assert!(detail(&f).contains("field of acme.quotes@1"), "{f}");
    let rev = ws.store.field("out", "rank").unwrap().unwrap().def_rev;
    ws.ok(json!([{ "op": "table.delete_field", "source_id": "out", "field_id": "rank", "expect": { "rev": rev } }]));
}

#[test]
fn contract_results_and_blockers() {
    use aiworkspace_core::wish::{check_analysis, check_contract_result};
    check_contract_result(&json!({ "name": "todo", "type": "table", "schema_id": "buckyos.todo@1" })).unwrap();
    check_contract_result(&json!({ "name": "q", "type": "table", "schema_id": "acme.quotes@1" })).unwrap();
    assert!(check_contract_result(&json!({ "name": "n", "type": "richtext", "schema_id": "acme.quotes@1" })).is_err());
    assert!(check_contract_result(&json!({ "name": "q", "type": "table", "schema_id": "quotes" })).is_err());
    let analysis = |blockers: Value| json!({ "schema_version": "wish.analysis.v3", "status": "ready", "prompt": "p", "context_prompt": "c",
        "output_contract": { "results": [] }, "checks": [], "blockers": blockers, "warnings": [] });
    // an analysis that needs data from outside fails (its blockers are never written); the shape is still checked
    let wanted = json!([{ "code": "external_data", "message": "no quotes here", "wanted": { "meaning": "daily prices", "origin": "an exchange", "freshness": "daily",
                                                                                         "fields": [{ "name": "symbol", "type": "text" }] } }]);
    assert!(check_analysis(&analysis(wanted)).unwrap_err().detail.contains("blockers cannot be ready"));
    let bad = json!([{ "code": "external_data", "message": "m", "wanted": { "fields": [] } }]);
    assert!(check_analysis(&analysis(bad)).unwrap_err().detail.contains("wanted"));
}

#[test]
fn a_table_view_may_show_its_first_rows() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([create("t", "a", json!({ "fields": fields() })),
        { "op": "entity.create", "entity_id": "s1-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s1" } },
        { "op": "entity.create", "entity_id": "s1", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s1-content" } }]));
    let cell = |limit: Value| json!([{ "op": "entity.create", "entity_id": "v", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 100, "h": 100 },
                                       "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "t" }, "row_limit": limit } }]);
    assert!(detail(&ws.fail(cell(json!(0)))).contains("row_limit"));
    ws.ok(cell(json!(100)));
    let m = meta::meta(&ws.store, &full(), &ws.store.entity("v").unwrap().unwrap(), None).unwrap();
    assert_eq!(m["shows"]["row_limit"], 100, "{m}");
}

#[test]
fn a_zone_source_may_name_the_sync_task_that_writes_it() {
    let mut ws = MemWorkspace::new();
    let out = "/home/alice/agents/bot.example/sync/quotes/out.json";
    let synced = |path: &str, sync: Value| json!({ "kind": "zone", "path": path, "sync": sync, "def": { "meaning": "Daily closing prices", "primary_key": ["symbol"] } });
    ws.ok(json!([create("quotes", "a", json!({ "fields": fields(), "source": synced(out, json!({ "agent_id": "bot.example", "task_id": "quotes" })) }))]));
    let m = meta::meta(&ws.store, &full(), &ws.store.entity("quotes").unwrap().unwrap(), None).unwrap();
    assert_eq!(m["data"]["sync"], json!({ "agent_id": "bot.example", "task_id": "quotes" }), "{m}");
    ws.ok(json!([create("plain", "b", json!({ "fields": fields(), "source": zone_source() }))]));
    assert!(meta::meta(&ws.store, &full(), &ws.store.entity("plain").unwrap().unwrap(), None).unwrap()["data"]["sync"].is_null());
    let bad = |source: Value| json!([create("x", "c", json!({ "fields": fields(), "source": source }))]);
    for (source, why) in [
        (synced("/home/alice/sync/quotes.json", json!({ "agent_id": "bot.example", "task_id": "quotes" })), "agents/bot.example/sync/quotes/out.json"),
        (synced("/home/alice/agents/bot.example/sync/other/out.json", json!({ "agent_id": "bot.example", "task_id": "quotes" })), "source.path must be"),
        (synced("/home/alice/agents/other.example/sync/quotes/out.json", json!({ "agent_id": "bot.example", "task_id": "quotes" })), "source.path must be"),
        (synced(out, json!({ "agent_id": "bot.example" })), "{ agent_id, task_id }"),
        (synced(out, json!({ "agent_id": "bot.example", "task_id": "quotes", "secret": "x" })), "agent_id and task_id only"),
        (json!({ "kind": "url", "url": "https://example.invalid/q.csv", "sync": { "agent_id": "bot.example", "task_id": "quotes" } }), "belongs to a zone source"),
    ] {
        let f = ws.fail(bad(source.clone()));
        assert!(detail(&f).contains(why), "{source}: {f}");
    }
    // it is part of the Source: the owner's alone
    let editor = Access::uniform("bob", CapSet::ALL);
    let env = ws.env("bob", "human", None, false);
    let unset = MemWorkspace::request(json!([{ "op": "entity.set_keys", "entity_id": "quotes", "keys": [{ "key": "source", "value": synced(out, json!({ "agent_id": "bot.example", "task_id": "quotes2" })), "expect": "any" }] }]));
    assert_eq!(ws.commit_with(&editor, &env, &unset).unwrap_err().to_json()["errors"][0]["code"], "PERMISSION_DENIED");
}
