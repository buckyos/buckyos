//! The `currency` value type (表格数据导入 TODO §6): stored with its code, ordered by (code, amount), filtered
//! within one currency, migrated to and from decimal / number / text, and a default currency that only later
//! writes take.

use aiworkspace_core::filter::{run_query, QuerySpec};
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn prices(ws: &mut MemWorkspace) {
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "prices", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "b",
          "payload": { "title_field_id": "item", "fields": [
            { "field_id": "item", "name": "Item", "type": "text" },
            { "field_id": "price", "name": "Price", "type": "currency", "currency": "USD" }] } },
        { "op": "table.insert_records", "source_id": "prices", "records": [
            { "record_id": "r1", "values": { "item": "a", "price": "200" } },
            { "record_id": "r2", "values": { "item": "b", "price": { "amount": "500", "currency": "CNY" } } },
            { "record_id": "r3", "values": { "item": "c", "price": { "amount": 300, "currency": "JPY" } } },
            { "record_id": "r4", "values": { "item": "d", "price": { "amount": "10", "currency": "EUR" } } },
            { "record_id": "r5", "values": { "item": "e", "price": 50 } },
            { "record_id": "r6", "values": { "item": "f" } }] }
    ]));
}

fn ids(result: &Value) -> Vec<String> {
    result["rows"].as_array().unwrap().iter().map(|r| r["record_id"].as_str().unwrap().to_string()).collect()
}

fn query(ws: &MemWorkspace, filter: Option<Value>, sorts: Vec<Value>) -> Value {
    let source = ws.store.entities["prices"].clone();
    run_query(&ws.store, &source, &QuerySpec { filter, sorts, limit: 100, ..Default::default() }).expect("query")
}

#[test]
fn stored_with_code_and_scale() {
    let mut ws = MemWorkspace::new();
    prices(&mut ws);
    let value = |id: &str| ws.store.records[&("prices".to_string(), id.to_string())].values["price"].clone();
    assert_eq!(value("r1"), json!({ "amount": "200.00", "currency": "USD" }));
    assert_eq!(value("r2"), json!({ "amount": "500.00", "currency": "CNY" }));
    assert_eq!(value("r3"), json!({ "amount": "300.00", "currency": "JPY" }));
    // symbols and loose text are refused: resolving them is the importer's job
    for bad in [json!("$12"), json!("USD 12"), json!({ "amount": "1" }), json!({ "amount": "1.234", "currency": "USD" })] {
        let f = ws.fail(json!([{ "op": "table.insert_records", "source_id": "prices", "records": [{ "record_id": "rx", "values": { "price": bad } }] }]));
        assert_eq!(code(&f), "INVALID_SCHEMA", "{bad}");
    }
    // a currency field needs its default currency; unique is not offered
    let f = ws.fail(json!([{ "op": "table.add_field", "source_id": "prices", "field": { "field_id": "p2", "name": "P2", "type": "currency" } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
    let f = ws.fail(json!([{ "op": "table.add_field", "source_id": "prices", "field": { "field_id": "p2", "name": "P2", "type": "currency", "currency": "EUR", "unique": true } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
    // the title field stays text
    let f = ws.fail(json!([{ "op": "entity.create", "entity_id": "t2", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "c",
        "payload": { "title_field_id": "p", "fields": [{ "field_id": "p", "name": "P", "type": "currency", "currency": "USD" }] } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA");
}

#[test]
fn sort_and_filter() {
    let mut ws = MemWorkspace::new();
    prices(&mut ws);
    // grouped by code, then by amount; empty last
    let sorted = query(&ws, None, vec![json!({ "field_id": "price", "direction": "asc" })]);
    assert_eq!(ids(&sorted), ["r2", "r4", "r3", "r5", "r1", "r6"]);
    let desc = query(&ws, None, vec![json!({ "field_id": "price", "direction": "desc" })]);
    assert_eq!(ids(&desc), ["r1", "r5", "r3", "r4", "r2", "r6"]);
    // `> 100` (a bare amount: the field's default currency) only meets amounts in that currency
    let gt = query(&ws, Some(json!({ "op": "cmp", "field_id": "price", "operator": "gt", "value": 100 })), vec![]);
    assert_eq!(ids(&gt), ["r1"]);
    let lt_cny = query(&ws, Some(json!({ "op": "cmp", "field_id": "price", "operator": "lt", "value": { "amount": "1000", "currency": "CNY" } })), vec![]);
    assert_eq!(ids(&lt_cny), ["r2"]);
    let eq = query(&ws, Some(json!({ "op": "cmp", "field_id": "price", "operator": "eq", "value": "200.0" })), vec![]);
    assert_eq!(ids(&eq), ["r1"]);
    let ne = query(&ws, Some(json!({ "op": "cmp", "field_id": "price", "operator": "ne", "value": 200 })), vec![]);
    assert_eq!(ids(&ne), ["r2", "r3", "r4", "r5"]);
    let empty = query(&ws, Some(json!({ "op": "cmp", "field_id": "price", "operator": "is_empty" })), vec![]);
    assert_eq!(ids(&empty), ["r6"]);
    // text operators and grouping are not offered
    let source = ws.store.entities["prices"].clone();
    let bad = run_query(&ws.store, &source, &QuerySpec { filter: Some(json!({ "op": "cmp", "field_id": "price", "operator": "contains", "value": "1" })), limit: 10, ..Default::default() });
    assert!(bad.is_err());
    let grouped = run_query(&ws.store, &source, &QuerySpec { group: Some("price".into()), limit: 10, ..Default::default() });
    assert!(grouped.is_err());
}

#[test]
fn default_currency_and_migrations() {
    let mut ws = MemWorkspace::new();
    prices(&mut ws);
    let key = |id: &str| ("prices".to_string(), id.to_string());
    let field = |ws: &MemWorkspace| ws.store.fields[&("prices".to_string(), "price".to_string())].clone();
    // changing the default currency leaves stored values alone; later bare amounts take the new one
    let rev = field(&ws).def_rev;
    ws.ok(json!([{ "op": "table.update_field", "source_id": "prices", "field_id": "price", "changes": { "currency": "EUR" }, "expect": { "rev": rev } }]));
    assert_eq!(field(&ws).def.currency.as_deref(), Some("EUR"));
    assert_eq!(ws.store.records[&key("r1")].values["price"], json!({ "amount": "200.00", "currency": "USD" }));
    let rev = ws.store.records[&key("r6")].revs.get("price").copied().unwrap_or(0);
    ws.ok(json!([{ "op": "table.set_values", "source_id": "prices", "values": [{ "record_id": "r6", "field_id": "price", "value": "7", "expect": { "rev": rev } }] }]));
    assert_eq!(ws.store.records[&key("r6")].values["price"], json!({ "amount": "7.00", "currency": "EUR" }));
    let rev = field(&ws).def_rev;
    let f = ws.fail(json!([{ "op": "table.update_field", "source_id": "prices", "field_id": "price", "changes": { "currency": "eur" }, "expect": { "rev": rev } }]));
    assert_eq!(code(&f), "INVALID_SCHEMA");

    // currency → text keeps the code; → decimal drops it
    ws.ok(json!([{ "op": "table.migrate_field", "source_id": "prices", "field_id": "price", "expect": { "rev": rev }, "to": { "type": "text" } }]));
    assert_eq!(ws.store.records[&key("r2")].values["price"], json!("CNY 500.00"));
    let mut ws = MemWorkspace::new();
    prices(&mut ws);
    let rev = field(&ws).def_rev;
    ws.ok(json!([{ "op": "table.migrate_field", "source_id": "prices", "field_id": "price", "expect": { "rev": rev }, "to": { "type": "decimal", "scale": 2 } }]));
    assert_eq!(ws.store.records[&key("r2")].values["price"], json!("500.00"));
    // decimal → currency gives every value the target's default currency
    let rev = field(&ws).def_rev;
    ws.ok(json!([{ "op": "table.migrate_field", "source_id": "prices", "field_id": "price", "expect": { "rev": rev }, "to": { "type": "currency", "currency": "GBP", "scale": 0 }, "on_failure": "unset" }]));
    assert_eq!(field(&ws).def.currency.as_deref(), Some("GBP"));
    assert_eq!(ws.store.records[&key("r1")].values["price"], json!({ "amount": "200", "currency": "GBP" }));
    // currency → currency (another scale) keeps the default and each value's own code
    let rev = field(&ws).def_rev;
    ws.ok(json!([{ "op": "table.migrate_field", "source_id": "prices", "field_id": "price", "expect": { "rev": rev }, "to": { "type": "currency", "scale": 2 } }]));
    assert_eq!(field(&ws).def.currency.as_deref(), Some("GBP"));
    assert_eq!(ws.store.records[&key("r1")].values["price"], json!({ "amount": "200.00", "currency": "GBP" }));
    // number → currency
    ws.ok(json!([{ "op": "table.add_field", "source_id": "prices", "field": { "field_id": "n", "name": "N", "type": "number" } }]));
    let add = ws.head_seq;
    ws.ok(json!([{ "op": "table.set_values", "source_id": "prices", "values": [{ "record_id": "r1", "field_id": "n", "value": 12.5, "expect": { "rev": 0 } }] }]));
    ws.ok(json!([{ "op": "table.migrate_field", "source_id": "prices", "field_id": "n", "expect": { "rev": add }, "to": { "type": "currency", "currency": "USD" } }]));
    assert_eq!(ws.store.records[&key("r1")].values["n"], json!({ "amount": "12.50", "currency": "USD" }));
    let rev = ws.store.fields[&("prices".to_string(), "n".to_string())].def_rev;
    let f = ws.fail(json!([{ "op": "table.migrate_field", "source_id": "prices", "field_id": "n", "expect": { "rev": rev }, "to": { "type": "number" } }]));
    assert_eq!(f["errors"][0]["sub_code"], "MIGRATION_UNSUPPORTED");
}
