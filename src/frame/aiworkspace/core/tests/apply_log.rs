//! The pure-application log (内核架构改进 D20): accepted Commits are applied from their recorded row deltas,
//! never re-planned. `fixtures/vectors/apply-log.json` holds the deltas of a scripted history and the digest
//! of the state they produce; replaying them through `rows_to_changes` alone — no planner — must give that
//! digest, whatever the planner has become since. Regenerate with `AIWS_REGEN=1 cargo test -p
//! aiworkspace-core --test apply_log`.

use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::plan::{plan_commit, CommitRequest, Limits, PlanEnv};
use aiworkspace_core::replica::{changes_to_rows, rows_to_changes, state_digest};
use serde_json::{json, Value};

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/vectors/apply-log.json");

fn para(id: &str, text: &str) -> Value {
    json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] })
}

/// The scripted history: every primitive is touched at least once.
fn script() -> Vec<Value> {
    vec![
        json!([
            { "op": "entity.create", "entity_id": "s1-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
              "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s1", "title": "s1" } },
            { "op": "entity.create", "entity_id": "s1", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a", "name": "s1",
              "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "s1", "content_folder_id": "s1-content" } },
            { "op": "entity.create", "entity_id": "t", "type_id": TYPE_TABLE, "parent_id": DATA_ID, "order_key": "b", "name": "t",
              "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }, { "field_id": "n", "name": "数", "type": "number" }] } },
            { "op": "table.insert_records", "source_id": "t", "records": [{ "record_id": "r1", "values": { "title": "a", "n": 1 } }, { "record_id": "r2", "values": { "title": "b", "n": 2 } }] }
        ]),
        json!([
            { "op": "entity.create", "entity_id": "doc", "type_id": TYPE_RICHTEXT, "parent_id": DATA_ID, "order_key": "c", "name": "doc",
              "payload": { "content": { "type": "doc", "content": [para("p1", "第一段")] } } },
            { "op": "entity.create", "entity_id": "own", "type_id": TYPE_TABLE, "parent_id": "s1-content", "order_key": "a", "name": "own",
              "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }] } },
            { "op": "entity.create", "entity_id": "b1", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 },
              "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "own" } } }
        ]),
        json!([
            { "op": "richtext.insert_blocks", "entity_id": "doc", "position": { "end": true }, "blocks": [para("p2", "第二段")] },
            { "op": "table.set_values", "source_id": "t", "values": [{ "record_id": "r1", "field_id": "n", "value": 10, "expect": { "rev": 1 } }] },
            { "op": "table.add_field", "source_id": "t", "field": { "field_id": "k", "name": "键", "type": "select", "options": [{ "option_id": "o1", "label": "一" }] } }
        ]),
        json!([
            { "op": "table.delete_records", "source_id": "t", "records": [{ "record_id": "r2", "expect": { "rev": 1 } }] },
            { "op": "entity.rename", "entity_id": "t", "name": "renamed", "expect": { "rev": 1 } },
            { "op": "tree.place", "entity_id": "b1", "placement": { "x": 5, "y": 5, "w": 10, "h": 10 } }
        ]),
        json!([{ "op": "data.pin", "entity_id": "own" }]),
        json!([{ "op": "entity.delete", "entity_id": "b1", "expect": { "rev": 2 } }]),
        json!([{ "op": "entity.restore", "entity_id": "b1", "expect": { "rev": 6 } }]),
    ]
}

fn env(seq: u64) -> PlanEnv {
    PlanEnv { seq, principal: "alice".into(), origin: "human".into(), run_id: None, now: "2026-10-23T00:00:00.000Z".into(), internal: false, import: false, schema_checks: true, replay: false, peer_id: 7, limits: Limits::default() }
}

#[test]
fn recorded_deltas_replay_without_the_planner() {
    // the planner's side: plan every step, record its delta
    let mut planned = MemStore::with_root();
    let mut deltas = Vec::new();
    for (i, ops) in script().into_iter().enumerate() {
        let req = CommitRequest::parse(&json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": "ws", "epoch": "ep", "idempotency_key": format!("k{i}"), "operations": ops })).unwrap();
        let access = Access::full("alice");
        let e = env(i as u64 + 1);
        let plan = plan_commit(&planned, &req, &e, &access).map_err(|f| f.to_json()).unwrap();
        let changes = plan.overlay.into_changes();
        deltas.push(changes_to_rows(&changes).unwrap());
        planned.apply(changes);
    }
    let digest = state_digest(&planned).unwrap();
    // the replica's side: apply the deltas only
    let mut applied = MemStore::with_root();
    for d in &deltas {
        let c = rows_to_changes(&applied, d).unwrap();
        applied.apply(c);
    }
    assert_eq!(state_digest(&applied).unwrap(), digest, "rows reproduce the planned state");
    assert_eq!(applied.richtexts["doc"].meta.ast["content"].as_array().unwrap().len(), 2, "the rich text advanced by update bytes");
    assert!(applied.refs.iter().any(|r| r.src_entity_id == "b1" && r.kind == "bind") && !applied.refs.iter().any(|r| r.kind == "owns"), "refs applied, owns pinned away");
    let recorded = json!({ "description": "Row deltas of core/tests/apply_log.rs's scripted history and the digest of the state they produce (D20). Applied without a planner; regenerate with AIWS_REGEN=1.", "deltas": deltas, "digest": digest });
    if std::env::var("AIWS_REGEN").is_ok() {
        std::fs::write(PATH, serde_json::to_string_pretty(&recorded).unwrap() + "\n").unwrap();
        return;
    }
    // the recorded vector of an older planner still applies to the same digest
    let file: Value = serde_json::from_str(&std::fs::read_to_string(PATH).expect("fixtures/vectors/apply-log.json (AIWS_REGEN=1 records it)")).unwrap();
    let mut old = MemStore::with_root();
    for d in file["deltas"].as_array().unwrap() {
        let c = rows_to_changes(&old, d).unwrap();
        old.apply(c);
    }
    assert_eq!(state_digest(&old).unwrap(), file["digest"], "the recorded deltas no longer reproduce their digest");
    assert_eq!(file["digest"], digest, "the planner now writes a different state for the same history; if intended, re-record with AIWS_REGEN=1");
}
