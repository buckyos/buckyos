//! The published protocol catalogues must describe the code that runs.

use aiworkspace_core::plan::OPERATIONS;
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::Code;
use serde_json::{json, Value};

fn load(name: &str) -> Value {
    let path = format!("{}/../schemas/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn error_codes_file_matches_code() {
    let file = load("error-codes.json");
    let listed: Vec<(String, bool, bool)> = file["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["code"].as_str().unwrap().to_string(), c["retryable"].as_bool().unwrap(), c["status"] == "conflict"))
        .collect();
    let actual: Vec<(String, bool, bool)> = Code::ALL.iter().map(|c| (c.as_str().to_string(), c.retryable(), c.is_conflict())).collect();
    assert_eq!(listed, actual);
}

#[test]
fn operation_catalogue_matches_dispatch() {
    let file = load("operations.json");
    let listed: Vec<(String, String, String, String)> = file["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let s = |k: &str| o[k].as_str().unwrap().to_string();
            (s("op"), s("class"), s("undo"), s("capability"))
        })
        .collect();
    let actual: Vec<(String, String, String, String)> =
        OPERATIONS.iter().map(|(a, b, c, d)| (a.to_string(), b.to_string(), c.to_string(), d.to_string())).collect();
    assert_eq!(listed, actual);
    // the applicable types (核心 API 调整 §4) are the same list in the same order
    let listed_types: Vec<(String, Vec<String>)> = file["operations"].as_array().unwrap().iter()
        .map(|o| (o["op"].as_str().unwrap().to_string(), o["types"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect()))
        .collect();
    let actual_types: Vec<(String, Vec<String>)> = aiworkspace_core::plan::OPERATION_TYPES.iter().map(|(o, t)| (o.to_string(), t.iter().map(|x| x.to_string()).collect())).collect();
    assert_eq!(listed_types, actual_types);
    assert_eq!(OPERATIONS.len(), actual_types.len());
    // every catalogued name is known to the dispatcher; anything else is not
    let mut ws = MemWorkspace::new();
    for (name, class, ..) in OPERATIONS {
        let f = ws.fail(json!([{ "op": name }]));
        let detail = f["errors"][0]["detail"].as_str().unwrap_or("");
        assert!(!detail.contains("unknown operation"), "{name}: {f}");
        if *class == "internal" {
            assert!(detail.contains("internal operation"), "{name} must not be callable from outside: {f}");
        }
    }
    assert!(ws.fail(json!([{ "op": "table.drop_everything" }]))["errors"][0]["detail"].as_str().unwrap().contains("unknown operation"));
}

#[test]
fn reference_schema_and_the_three_version_vocabularies() {
    use aiworkspace_core::value::{canonical_reference, reference_from_canonical};
    let file = load("reference.schema.json");
    assert_eq!(file["required"], json!(["entity_id", "mode"]));
    assert_eq!(file["properties"]["mode"]["enum"], json!(["live", "fixed"]));
    let obj = "mix256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let root = aiworkspace_core::canonical::named_object("jobj", &json!({ "a": 1 })).unwrap().0;
    // the frozen shape round-trips
    let live = canonical_reference(&json!({ "workspace_id": "ws1", "entity_id": "orders", "mode": "live" })).unwrap();
    assert_eq!(live, json!({ "workspace_id": "ws1", "entity_id": "orders", "mode": "live" }));
    let fixed = canonical_reference(&json!({ "entity_id": "orders", "mode": "fixed", "version": { "epoch": "ep1", "seq": 7 } })).unwrap();
    assert_eq!(fixed["version"], json!({ "epoch": "ep1", "seq": 7 }));
    let rooted = canonical_reference(&json!({ "entity_id": "orders", "mode": "fixed", "version": { "content_root": root, "path": "a/b" } })).unwrap();
    assert_eq!(rooted["version"]["path"], "a/b");
    assert!(canonical_reference(&json!({ "entity_id": "orders", "mode": "live", "version": { "epoch": "e", "seq": 1 } })).is_err());
    assert!(canonical_reference(&json!({ "entity_id": "orders", "mode": "fixed" })).is_err());
    assert!(canonical_reference(&json!({ "entity_id": "orders", "mode": "fixed", "version": { "content_root": "nope" } })).is_err());
    // 1. persisted references: live_head / fixed_revision / published_channel
    assert_eq!(canonical_reference(&json!({ "entity_id": "t", "version": { "mode": "live_head" } })).unwrap()["mode"], "live");
    assert_eq!(canonical_reference(&json!({ "entity_id": "t" })).unwrap()["mode"], "live");
    let f = canonical_reference(&json!({ "entity_id": "t", "version": { "mode": "fixed_revision", "object_id": root } })).unwrap();
    assert_eq!((f["mode"].as_str(), f["version"]["content_root"].as_str()), (Some("fixed"), Some(root.as_str())));
    assert_eq!(canonical_reference(&json!({ "entity_id": "t", "version": { "mode": "published_channel", "channel": "main" } })).unwrap()["mode"], "live");
    assert!(canonical_reference(&json!({ "entity_id": "t", "version": { "mode": "fixed_revision", "source_revision": "r9" } })).is_err());
    // 2. wish inputs: follow / fixed
    let w = canonical_reference(&json!({ "entity_id": "sales", "name": "sales", "label": "Orders", "selector": { "kind": "table_query", "fields": ["m"] }, "version": { "mode": "follow", "rev": 3 } })).unwrap();
    assert_eq!((w["mode"].as_str(), w["selector"]["kind"].as_str()), (Some("live"), Some("table_query")));
    assert_eq!(canonical_reference(&json!({ "entity_id": "sales", "version": { "mode": "fixed", "object_id": obj } })).unwrap()["version"]["content_root"], obj);
    assert!(canonical_reference(&json!({ "entity_id": "sales", "version": { "mode": "fixed", "rev": 3 } })).is_err());
    // 3. back to the persisted form: live / content root; { epoch, seq } is not retained yet
    assert_eq!(reference_from_canonical(&live).unwrap()["version"], json!({ "mode": "live_head" }));
    assert_eq!(reference_from_canonical(&rooted).unwrap()["version"]["mode"], "fixed_revision");
    assert_eq!(reference_from_canonical(&fixed).unwrap_err().code, Code::VersionNotRetained);
}

#[test]
fn richtext_schema_file_is_wellformed() {
    let s = aiworkspace_core::richtext::schema();
    assert_eq!(s["id"], "buckyos.richtext.basic.v1");
    for (name, node) in s["nodes"].as_object().unwrap() {
        for item in node.get("content").and_then(Value::as_array).into_iter().flatten() {
            for of in item["of"].as_array().unwrap() {
                let of = of.as_str().unwrap();
                let known = match of.strip_prefix('@') {
                    Some(g) => s["nodes"].as_object().unwrap().values().any(|n| n["group"] == g),
                    None => s["nodes"].get(of).is_some(),
                };
                assert!(known, "{name} refers to unknown {of}");
            }
        }
    }
}

#[test]
fn commit_request_schema_speaks_the_current_protocol() {
    use aiworkspace_core::model::PROTOCOL_VERSION;
    use aiworkspace_core::plan::SELECTOR_KINDS;
    let file = load("commit-request.schema.json");
    let versions = file["properties"]["protocol_version"]["enum"].as_array().unwrap();
    assert!(versions.contains(&json!(PROTOCOL_VERSION)), "{versions:?}");
    assert!(file["$id"].as_str().unwrap().ends_with(PROTOCOL_VERSION));
    let listed: Vec<&str> = file["$defs"]["selector"]["properties"]["kind"]["enum"].as_array().unwrap().iter().map(|k| k.as_str().unwrap()).collect();
    assert_eq!(listed, SELECTOR_KINDS);
    // every listed kind is one the planner resolves (an unknown one would be MISSING_EXTENSION)
    let ws = MemWorkspace::new();
    for kind in SELECTOR_KINDS {
        let e = aiworkspace_core::plan::resolve_cell(&ws.store, &json!({ "entity_id": "root", "selector": { "kind": kind } })).err();
        assert!(e.is_none_or(|e| e.code != Code::MissingExtension), "{kind}");
    }
}

#[test]
fn extension_schemas_match_the_validators() {
    use aiworkspace_core::types::{BLOCK_DEF_KEYS, MANIFEST_VERSION};
    let def = load("block-def.schema.json");
    let mut keys: Vec<&str> = def["properties"].as_object().unwrap().keys().map(String::as_str).collect();
    let mut code_keys = BLOCK_DEF_KEYS.to_vec();
    keys.sort();
    code_keys.sort();
    assert_eq!(keys, code_keys);
    for (name, path) in [("block-def.schema.json", "/properties/manifest_version/maximum"), ("component.schema.json", "/properties/manifest_version/maximum"), ("aiws-manifest.schema.json", "/properties/manifest_version/maximum")] {
        assert_eq!(load(name).pointer(path), Some(&json!(MANIFEST_VERSION)), "{name}");
    }
    // the reference kind the source snapshot uses is in the frozen catalogue
    assert!(aiworkspace_core::model::REF_KINDS.iter().any(|(k, _, i, _)| *k == "def_source" && *i == "strong"));
}

/// The JSON Schema keywords the parameter schemas use: `$ref` (to this file's `$defs`), `type`, `const`, `enum`,
/// `required`, `properties`, `additionalProperties` (false or a schema), `items`, `minItems`, `maxItems`, `minimum`,
/// `maximum`, `minLength`, `maxLength`, `oneOf`, `anyOf`. `pattern` and `propertyNames` are not checked here.
fn conforms(root: &Value, schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r.strip_prefix("#/$defs/").unwrap_or_else(|| panic!("{at}: unsupported $ref {r}"));
        return conforms(root, &root["$defs"][name], v, at);
    }
    let fail = |why: String| Err(format!("{at}: {why} ({v})"));
    if let Some(t) = schema.get("type") {
        let types: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let is = |t: &str| match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_u64() || v.is_i64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            other => panic!("{at}: unsupported type {other}"),
        };
        if !types.iter().any(|t| is(t)) {
            return fail(format!("not of type {t}"));
        }
    }
    if let Some(c) = schema.get("const") {
        if c != v {
            return fail(format!("not {c}"));
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            return fail("not in the enum".into());
        }
    }
    if let (Some(n), Some(x)) = (schema.get("minimum").and_then(Value::as_f64), v.as_f64()) {
        if x < n {
            return fail(format!("below {n}"));
        }
    }
    if let (Some(n), Some(x)) = (schema.get("maximum").and_then(Value::as_f64), v.as_f64()) {
        if x > n {
            return fail(format!("above {n}"));
        }
    }
    if let Some(s) = v.as_str() {
        let len = s.chars().count() as u64;
        if schema.get("minLength").and_then(Value::as_u64).is_some_and(|n| len < n) || schema.get("maxLength").and_then(Value::as_u64).is_some_and(|n| len > n) {
            return fail("length out of range".into());
        }
    }
    if let Some(o) = v.as_object() {
        for k in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
            if !o.contains_key(k.as_str().unwrap()) {
                return fail(format!("{k} missing"));
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        for (k, x) in o {
            match props.and_then(|p| p.get(k)) {
                Some(s) => conforms(root, s, x, &format!("{at}/{k}"))?,
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => return fail(format!("{k} is not allowed")),
                    Some(s) if s.is_object() => conforms(root, s, x, &format!("{at}/{k}"))?,
                    _ => {}
                },
            }
        }
    }
    if let Some(a) = v.as_array() {
        if schema.get("minItems").and_then(Value::as_u64).is_some_and(|n| (a.len() as u64) < n) || schema.get("maxItems").and_then(Value::as_u64).is_some_and(|n| (a.len() as u64) > n) {
            return fail("item count out of range".into());
        }
        if let Some(items) = schema.get("items") {
            for (i, x) in a.iter().enumerate() {
                conforms(root, items, x, &format!("{at}/{i}"))?;
            }
        }
    }
    if let Some(list) = schema.get("oneOf").and_then(Value::as_array) {
        let n = list.iter().filter(|s| conforms(root, s, v, at).is_ok()).count();
        if n != 1 {
            return fail(format!("matches {n} of oneOf"));
        }
    }
    if let Some(list) = schema.get("anyOf").and_then(Value::as_array) {
        if !list.iter().any(|s| conforms(root, s, v, at).is_ok()) {
            return fail("matches none of anyOf".into());
        }
    }
    Ok(())
}

/// The operations the websdk's write helpers build have parameter schemas (核心 API 调整 §4, design §10), and the
/// planner agrees with them: what a helper builds conforms and is accepted, every listed parameter is one the planner
/// knows, and an unlisted one is refused by both.
#[test]
fn helper_operations_have_parameter_schemas_the_planner_agrees_with() {
    use aiworkspace_core::model::{CANVAS_CONTENT_ID, SURFACES_ID, TYPE_CELL, TYPE_CONTAINER, TYPE_RICHTEXT, TYPE_TABLE};
    let schema = load("commit-request.schema.json");
    let catalogue = load("operations.json");
    assert_eq!(catalogue["protocol_version"], aiworkspace_core::model::PROTOCOL_VERSION);
    let params: Vec<(String, Value)> = catalogue["operations"].as_array().unwrap().iter()
        .filter_map(|o| {
            let r = o.get("params")?.as_str().unwrap();
            let name = r.strip_prefix("commit-request.schema.json#/$defs/").unwrap_or_else(|| panic!("{r}"));
            let def = schema["$defs"].get(name).unwrap_or_else(|| panic!("{r} does not exist")).clone();
            assert_eq!(def["properties"]["op"]["const"], o["op"], "{r}");
            Some((o["op"].as_str().unwrap().to_string(), def))
        })
        .collect();
    let mut covered: Vec<&str> = params.iter().map(|(o, _)| o.as_str()).collect();
    covered.sort();
    assert_eq!(covered, ["entity.rename", "entity.set_props", "richtext.delete_blocks", "richtext.insert_blocks", "richtext.replace_block",
        "table.add_field", "table.add_option", "table.delete_records", "table.insert_records", "table.set_values", "table.unset_values", "tree.move"]);

    let para = |id: &str, text: &str| json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] });
    let setup = || {
        let mut ws = MemWorkspace::new();
        ws.ok(json!([
            { "op": "entity.create", "entity_id": "t", "type_id": TYPE_TABLE, "parent_id": "data", "order_key": "a",
              "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" },
                                      { "field_id": "s", "name": "状态", "type": "select", "options": [{ "option_id": "a", "label": "甲" }] }] } },
            { "op": "table.insert_records", "source_id": "t", "records": [{ "record_id": "r1", "values": { "title": "一", "s": "a" } },
                                                                        { "record_id": "r2", "values": { "title": "二" } }] },
            { "op": "entity.create", "entity_id": "d", "type_id": TYPE_RICHTEXT, "parent_id": "data", "order_key": "b",
              "payload": { "content": { "type": "doc", "content": [para("p1", "第一段"), para("p2", "第二段")] } } },
            { "op": "entity.create", "entity_id": "f", "type_id": TYPE_CONTAINER, "parent_id": "data", "order_key": "c", "payload": { "kind": "folder" } },
            { "op": "entity.create", "entity_id": "g", "type_id": TYPE_CONTAINER, "parent_id": "data", "order_key": "d", "payload": { "kind": "folder" } },
            { "op": "entity.create", "entity_id": "s-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
              "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s" } },
            { "op": "entity.create", "entity_id": "s", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
              "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s-content" } },
            { "op": "entity.create", "entity_id": "c", "type_id": TYPE_CELL, "parent_id": "s", "order_key": "m", "placement": { "x": 0, "y": 0, "w": 200, "h": 100 },
              "payload": { "view": { "type": "text" }, "config": { "text": "a" } } }
        ]));
        ws
    };
    // what each helper builds (websdk aiws_write.ts), with the versions it read
    let ws = setup();
    let cell_rev = |r: &str, f: &str| ws.store.records[&("t".to_string(), r.to_string())].value_rev(f);
    let record_rev = |r: &str| ws.store.records[&("t".to_string(), r.to_string())].rev;
    let hash = |b: &str| ws.store.richtexts["d"].meta.block_index[b].hash.clone();
    let built = vec![
        json!({ "op": "table.insert_records", "source_id": "t", "records": [{ "record_id": "r3", "values": { "title": "三", "s": "a" } }] }),
        json!({ "op": "table.set_values", "source_id": "t", "values": [{ "record_id": "r1", "field_id": "title", "value": "壹", "expect": { "rev": cell_rev("r1", "title") } }] }),
        json!({ "op": "table.unset_values", "source_id": "t", "values": [{ "record_id": "r1", "field_id": "s", "expect": { "rev": cell_rev("r1", "s") } }] }),
        json!({ "op": "table.delete_records", "source_id": "t", "records": [{ "record_id": "r2", "expect": { "rev": record_rev("r2") } }] }),
        json!({ "op": "table.add_field", "source_id": "t", "field": { "field_id": "n", "name": "数量", "type": "number" }, "order_key": "x" }),
        json!({ "op": "table.add_option", "source_id": "t", "field_id": "s", "option": { "option_id": "b", "label": "乙" },
                "expect": { "rev": ws.store.fields[&("t".to_string(), "s".to_string())].def_rev } }),
        json!({ "op": "richtext.insert_blocks", "entity_id": "d", "position": { "end": true }, "blocks": [para("p3", "第三段")] }),
        json!({ "op": "richtext.replace_block", "entity_id": "d", "block_id": "p1", "expect": { "hash": hash("p1") }, "node": para("p1", "改过") }),
        json!({ "op": "richtext.delete_blocks", "entity_id": "d", "blocks": [{ "block_id": "p2", "expect": { "hash": hash("p2"), "struct_rev": ws.store.richtexts["d"].meta.block_index["p2"].struct_rev } }] }),
        json!({ "op": "entity.rename", "entity_id": "f", "name": "资料", "expect": { "rev": ws.store.entities["f"].meta_rev } }),
        json!({ "op": "tree.move", "entity_id": "f", "new_parent_id": "g", "order_key": "m" }),
        json!({ "op": "entity.set_props", "entity_id": "c", "key": "config", "props": { "text": "b" }, "expect": { "text": 0 } }),
    ];
    for op in &built {
        let name = op["op"].as_str().unwrap();
        let def = &params.iter().find(|(o, _)| o == name).unwrap_or_else(|| panic!("{name} has no schema")).1;
        conforms(&schema, def, op, name).unwrap();
        let mut ws = setup();
        ws.ok(json!([op]));
        // an unlisted parameter: refused by the schema and by the planner
        let mut extra = op.clone();
        extra["zz_extra"] = json!(1);
        assert!(conforms(&schema, def, &extra, name).is_err(), "{name}");
        let f = setup().fail(json!([extra]));
        assert!(f.to_string().contains("unknown parameter zz_extra"), "{name}: {f}");
        // every listed parameter is one the planner knows
        for k in def["properties"].as_object().unwrap().keys().filter(|k| *k != "op") {
            let mut probe = op.clone();
            if probe.get(k).is_none() {
                probe[k] = json!(null);
            }
            if let Err(f) = setup().commit(json!([probe])) {
                let f = f.to_json().to_string();
                assert!(!f.contains(&format!("unknown parameter {k}")), "{name}: the schema lists {k}, the planner does not know it: {f}");
            }
        }
    }
}
