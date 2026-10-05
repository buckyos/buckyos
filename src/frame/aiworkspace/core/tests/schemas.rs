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
