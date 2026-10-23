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
