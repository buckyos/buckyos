//! Cross-language vectors (V01). The same file is checked by the browser build
//! (WASM facade) in the Desktop tests. Regenerate with `AIWS_REGEN=1 cargo test -p aiworkspace-core --test vectors`.

use aiworkspace_core::canonical::*;
use aiworkspace_core::value::FieldDef;
use serde_json::{json, Value};

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/vectors/object-ids.json");

fn inputs() -> Vec<(&'static str, Value)> {
    vec![
        ("jobj", json!({})),
        ("jobj", json!({ "b": 1, "a": null, "c": [true, false, "x"] })),
        ("jobj", json!({ "ws_type": "buckyos.table-source", "schema_version": 1, "content": { "title_field_id": "title", "records": { "encoding": "jsonl+jcs", "count": 0 } } })),
        // key order is by UTF-16 code units, not by code points
        ("jobj", json!({ "\u{20ac}": "Euro", "\r": "CR", "\u{fb33}": "Hebrew", "1": "One", "\u{1f600}": "Emoji", "\u{80}": "Control", "\u{f6}": "Latin" })),
        // strings are hashed as given: no Unicode normalization (decomposed vs precomposed differ)
        ("jobj", json!({ "text": "e\u{301}" })),
        ("jobj", json!({ "text": "\u{e9}" })),
        ("jobj", json!({ "escapes": "\"\\/\u{8}\u{c}\n\r\t\u{1}\u{7f}", "中文": "值", "emoji": "😀" })),
        ("jobj", json!({ "numbers": [0, 1, -1, 1.5, 0.1, 1e21, 1e-7, 123456789, 9007199254740991u64, -9007199254740991i64, 4.5e-10, 100, 1.0] })),
        // an explicit null and an omitted key are different objects
        ("jobj", json!({ "a": 1 })),
        ("jobj", json!({ "a": 1, "b": null })),
        ("jobj", json!({ "ws_type": "buckyos.richtext", "schema_version": 1, "content": { "editor_schema": "buckyos.richtext.basic.v1",
            "doc": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "本周项目进展" }] }] } } })),
        ("cyfile", json!({ "content": "mix256:05abcd", "create_time": 0, "last_update_time": 0, "size": 5 })),
    ]
}

fn build() -> Value {
    let valid: Vec<Value> = inputs()
        .into_iter()
        .map(|(t, v)| {
            let (id, text) = named_object(t, &v).unwrap();
            json!({ "obj_type": t, "input": serde_json::to_string(&v).unwrap(), "canonical": text, "object_id": id })
        })
        .collect();
    let invalid = json!([
        { "input": "{\"a\":1,\"a\":2}", "reason": "duplicate key" },
        { "input": "{\"n\":9007199254740992}", "reason": "integer beyond 2^53-1" },
        { "input": "{\"n\":-9007199254740992}", "reason": "integer beyond 2^53-1" },
        { "input": "{\"n\":-0.0}", "reason": "negative zero" },
        { "input": "{\"n\":1e999}", "reason": "non-finite number" },
        { "input": "{\"a\":1} trailing", "reason": "trailing data" },
        { "input": "{'a':1}", "reason": "not JSON" }
    ]);
    let chunks: Vec<Value> = [&b""[..], b"hello", "中文 bytes".as_bytes()]
        .iter()
        .map(|d| {
            let c = chunk_id(d);
            json!({ "hex": hex::encode(d), "chunk_id": c, "file_object_id": file_object(d.len() as u64, &c).unwrap().0 })
        })
        .collect();
    let values = json!([
        { "def": { "field_id": "f", "name": "f", "type": "decimal", "scale": 2 }, "input": "001200.5", "output": "1200.50" },
        { "def": { "field_id": "f", "name": "f", "type": "decimal", "scale": 2 }, "input": "-0.00", "output": "0.00" },
        { "def": { "field_id": "f", "name": "f", "type": "decimal", "scale": 2 }, "input": "1.005", "error": "INVALID_SCHEMA" },
        { "def": { "field_id": "f", "name": "f", "type": "number" }, "input": -0.0, "output": 0 },
        { "def": { "field_id": "f", "name": "f", "type": "number" }, "input": 9007199254740992u64, "error": "INVALID_SCHEMA" },
        { "def": { "field_id": "f", "name": "f", "type": "date" }, "input": "2026-02-29", "error": "INVALID_SCHEMA" },
        { "def": { "field_id": "f", "name": "f", "type": "datetime" }, "input": "2026-10-04T16:00:00+08:00", "output": "2026-10-04T08:00:00.000Z" },
        { "def": { "field_id": "f", "name": "f", "type": "datetime" }, "input": "2026-10-04T08:00:00", "error": "INVALID_SCHEMA" },
        { "def": { "field_id": "f", "name": "f", "type": "text" }, "input": "", "output": "" },
        { "def": { "field_id": "f", "name": "f", "type": "text" }, "input": null, "error": "INVALID_SCHEMA" },
        { "def": { "field_id": "f", "name": "f", "type": "text", "nullable": true }, "input": null, "output": null },
        { "def": { "field_id": "f", "name": "f", "type": "multi_select", "options": [{ "option_id": "b", "label": "B" }, { "option_id": "a", "label": "A" }] }, "input": ["b", "a", "b"], "output": ["a", "b"] }
    ]);
    json!({ "description": "V01 vectors: canonical JSON, ObjectIds, chunk ids and value normalization. Checked by Rust and by the WASM build in the browser.",
            "valid": valid, "invalid": invalid, "chunks": chunks, "values": values })
}

#[test]
fn vectors_file_is_current_and_holds() {
    let built = build();
    if std::env::var("AIWS_REGEN").is_ok() {
        std::fs::write(PATH, serde_json::to_string_pretty(&built).unwrap() + "\n").unwrap();
    }
    let file: Value = serde_json::from_str(&std::fs::read_to_string(PATH).expect("run with AIWS_REGEN=1 once")).unwrap();
    assert_eq!(file, built, "fixtures/vectors/object-ids.json is stale");
    for v in file["valid"].as_array().unwrap() {
        let parsed = parse_strict(v["input"].as_str().unwrap()).unwrap();
        let (id, text) = named_object(v["obj_type"].as_str().unwrap(), &parsed).unwrap();
        assert_eq!((id.as_str(), text.as_str()), (v["object_id"].as_str().unwrap(), v["canonical"].as_str().unwrap()));
        verify_named_object(&id, &text).unwrap();
    }
    for v in file["invalid"].as_array().unwrap() {
        let r = parse_strict(v["input"].as_str().unwrap()).and_then(|p| canonical_json(&p));
        assert!(r.is_err(), "{} must be rejected ({})", v["input"], v["reason"]);
    }
    for v in file["values"].as_array().unwrap() {
        let def = FieldDef::from_value(&v["def"]).unwrap();
        match (def.normalize(&v["input"]), v.get("error")) {
            (Ok(out), None) => assert_eq!(out, v["output"], "{v}"),
            (Err(e), Some(code)) => assert_eq!(e.code.as_str(), code.as_str().unwrap(), "{v}"),
            (r, _) => panic!("{v}: {r:?}"),
        }
    }
    // the distinctions the vectors exist for
    let ids: Vec<&str> = file["valid"].as_array().unwrap().iter().map(|v| v["object_id"].as_str().unwrap()).collect();
    assert_ne!(ids[4], ids[5], "no normalization before hashing");
    assert_ne!(ids[8], ids[9], "null is not absence");
}
