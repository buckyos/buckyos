//! Extension controls P0 (notepads/aiworkspace-extension-p0-todo.md M1): the definition's new keys, the
//! `def_source` reference to its source snapshot, identities as sibling names, the build record, and old payloads.

use aiworkspace_core::access::Access;
use aiworkspace_core::canonical::{chunk_id, file_object};
use aiworkspace_core::model::*;
use aiworkspace_core::read::outline_extras;
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::types::ident_string;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn file(ws: &mut MemWorkspace, bytes: &[u8]) -> String {
    let (id, _) = file_object(bytes.len() as u64, &chunk_id(bytes)).unwrap();
    ws.store.assets.insert(id.clone(), AssetInfo { object_id: id.clone(), media_type: "text/plain".into(), size: bytes.len() as u64 });
    id
}

const ALICE: &str = "did:bns:alice";

fn def_payload(name: &str, extra: Value) -> Value {
    let mut p = json!({ "def_id": name, "version": 1, "kind": "declarative", "title": "KPI", "declarative": { "items": [] },
                        "ident": { "author_did": ALICE, "name": name }, "manifest_version": 1 });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

fn create_def(id: &str, name: &str, payload: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": "m", "name": name, "payload": payload })
}

#[test]
fn identities_are_did_and_name() {
    assert_eq!(ident_string(&json!({ "author_did": ALICE, "name": "kpi-card" })).unwrap(), "did:bns:alice#kpi-card");
    for bad in [json!({ "author_did": "alice", "name": "kpi" }), json!({ "author_did": "did:bns:a/b", "name": "kpi" }), json!({ "author_did": ALICE, "name": "KPI" }),
                json!({ "author_did": ALICE, "name": "kpi", "x": 1 }), json!({ "author_did": format!("did:bns:{}", "a".repeat(120)), "name": "kpi" })] {
        assert!(ident_string(&bad).is_err(), "{bad}");
    }
}

#[test]
fn new_keys_are_checked_and_the_source_directory_is_kept_alive() {
    let mut ws = MemWorkspace::new();
    let f = file(&mut ws, b"{}");
    ws.ok(json!([{ "op": "entity.create", "entity_id": "src1", "type_id": TYPE_DIR, "parent_id": DATA_ID, "order_key": "a",
                   "payload": { "members": { "component.json": { "kind": "file", "object_id": f } } } }]));
    let ident = "did:bns:alice#kpi";
    ws.ok(json!([create_def("d1", ident, def_payload("kpi", json!({
        "source_dir": { "entity_id": "src1" },
        "bindings": { "sales": { "accepts": [TYPE_TABLE], "required": true, "title": "Sales" }, "goal": { "accepts": [TYPE_RECORD] } },
        "catalog": { "group": "metrics", "icon": "chart-bar", "description": "One number" },
        "agent": "# KPI\nBind `sales`.",
        "provenance": { "pkg_id": "acme-kpi", "version": "1.2.0", "digest": format!("sha256:{}", "a".repeat(64)) },
        "config_schema": { "type": "object", "properties": { "field": { "type": "string" } }, "required": ["field"] },
    })))]));
    // the outline names the control
    let mut env = json!({});
    outline_extras(&mut env, &ws.store.entities["d1"]);
    assert_eq!((env["ident"].as_str(), env["manifest_version"].as_u64(), env["provenance_pkg"].as_str(), env["source_dir_id"].as_str()),
               (Some(ident), Some(1), Some("acme-kpi"), Some("src1")));
    // the source snapshot is a strong reference: the directory cannot go while the definition names it
    let life = ws.store.entities["src1"].life_rev;
    let fail = ws.fail(json!([{ "op": "entity.delete", "entity_id": "src1", "expect": { "rev": life } }]));
    assert_eq!(code(&fail), "REFERENCE_BROKEN", "{fail}");
    assert_eq!(fail["errors"][0]["data"]["referrers"][0]["entity_id"], "d1");
    // …and must be a live directory when written
    let mut bad = def_payload("kpi2", json!({ "source_dir": { "entity_id": "nope" } }));
    assert_eq!(code(&ws.fail(json!([create_def("d2", "did:bns:alice#kpi2", bad.clone())]))), "REFERENCE_BROKEN");
    bad["source_dir"] = json!({ "entity_id": "d1" });
    assert_eq!(code(&ws.fail(json!([create_def("d2", "did:bns:alice#kpi2", bad)]))), "INVALID_SCHEMA");
    // shapes
    for (extra, expected) in [
        (json!({ "ident": { "author_did": ALICE, "name": "other" } }), "INVALID_SCHEMA"),
        (json!({ "manifest_version": 9 }), "UNSUPPORTED_VERSION"),
        (json!({ "bindings": { "Bad-Name": {} } }), "INVALID_SCHEMA"),
        (json!({ "bindings": { "x": { "accepts": ["buckyos.nope"] } } }), "INVALID_SCHEMA"),
        (json!({ "catalog": { "icon": "Bad Icon" } }), "INVALID_SCHEMA"),
        (json!({ "provenance": { "digest": "md5:1" } }), "INVALID_SCHEMA"),
        (json!({ "agent": "x".repeat(70_000) }), "INVALID_SCHEMA"),
        (json!({ "source_dir": { "entity_id": "src1", "x": 1 } }), "INVALID_SCHEMA"),
        (json!({ "surprise": true }), "INVALID_SCHEMA"),
    ] {
        let f = ws.fail(json!([create_def("d3", "did:bns:alice#kpi3", def_payload("kpi3", extra.clone()))]));
        assert_eq!(code(&f), expected, "{extra}: {f}");
    }
    // an identity is the definition's name: a second definition of it in the same folder is a name conflict
    let f = ws.fail(json!([create_def("d4", ident, def_payload("kpi", json!({})))]));
    assert_eq!(f["errors"][0]["sub_code"], "NAME_CONFLICT", "{f}");
    // switching the source: the old directory may go once nothing names it
    ws.ok(json!([{ "op": "entity.create", "entity_id": "src2", "type_id": TYPE_DIR, "parent_id": DATA_ID, "order_key": "b", "payload": { "members": {} } }]));
    let rev = ws.store.entities["d1"].key_rev("source_dir");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "d1", "keys": [{ "key": "source_dir", "value": { "entity_id": "src2" }, "expect": { "rev": rev } }] }]));
    let life = ws.store.entities["src1"].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "src1", "expect": { "rev": life } }]));
}

#[test]
fn the_build_record_is_written_by_builds_only() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([create_def("d1", "did:bns:alice#kpi", def_payload("kpi", json!({})))]));
    let record = json!({ "build": { "input_digest": format!("sha256:{}", "0".repeat(64)), "builder_version": "aiws-ext-build/1", "built_at": "2026-10-30T00:00:00.000Z" } });
    let op = json!([{ "op": "entity.set_derived", "entity_id": "d1", "derived": record }]);
    // a person cannot claim a build
    assert_eq!(code(&ws.fail(op.clone())), "INVALID_OPERATION");
    // a build Commit can
    let env = ws.env("alice", "build", None, false);
    if let Err(f) = ws.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(op)) {
        panic!("{}", f.to_json());
    }
    let d = ws.store.entities["d1"].derived.clone().unwrap();
    assert_eq!(d["build"]["builder_version"], "aiws-ext-build/1");
    assert!(d.get("wish_id").is_none());
    // malformed records are refused even there
    let env = ws.env("alice", "build", None, false);
    let bad = json!([{ "op": "entity.set_derived", "entity_id": "d1", "derived": { "build": { "input_digest": "x" } } }]);
    assert!(ws.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(bad)).is_err());
    let env = ws.env("alice", "build", None, false);
    let mixed = json!([{ "op": "entity.set_derived", "entity_id": "d1", "derived": { "build": record["build"], "wish_id": "w1" } }]);
    assert!(ws.commit_with(&Access::full("alice"), &env, &MemWorkspace::request(mixed)).is_err());
}

#[test]
fn definitions_without_the_new_keys_still_work() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([{ "op": "entity.create", "entity_id": "def-kpi", "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": "e", "name": "kpi",
                   "payload": { "def_id": "sample.kpi-card", "kind": "declarative", "title": "KPI", "declarative": { "items": [] } } }]));
    let rev = ws.store.entities["def-kpi"].key_rev("title");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "def-kpi", "keys": [{ "key": "title", "value": "KPI 2", "expect": { "rev": rev } }] }]));
    let mut env = json!({});
    outline_extras(&mut env, &ws.store.entities["def-kpi"]);
    assert_eq!(env["def_id"], "sample.kpi-card");
    assert!(env.get("ident").is_none());
}
