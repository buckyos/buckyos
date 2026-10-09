//! Phase two store contracts: user work state (not a Commit), addressable versions of generated
//! results with restore plans, grants as seen by non-managers, and the Mock on the new structure.

mod common;
use aiworkspace_store::workspace::{Caller, CommitOpts};
use common::*;
use serde_json::{json, Value};

#[test]
fn user_state_is_per_subject_and_never_a_commit() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let head = ws.head_seq;
    ws.set_user_state(&alice(), &json!({ "mode": "canvas", "viewport:surface-main": { "x": 10, "y": 20, "zoom": 0.5 } })).unwrap();
    let mine = ws.get_user_state(&alice()).unwrap();
    assert_eq!(mine["entries"]["mode"], "canvas");
    assert_eq!(mine["entries"]["viewport:surface-main"]["zoom"], 0.5);
    // nothing in the document moved
    assert_eq!(ws.head_seq, head);
    assert_eq!(ws.get_changes(&alice(), &ws.epoch.clone(), head, 10, false).unwrap()["changes"].as_array().unwrap().len(), 0);
    // another principal has its own entries; a null removes
    ws.grant(&alice(), "bob", None, &["read".into()]).unwrap();
    assert_eq!(ws.get_user_state(&bob()).unwrap()["entries"].as_object().unwrap().len(), 0);
    ws.set_user_state(&alice(), &json!({ "mode": null })).unwrap();
    assert!(ws.get_user_state(&alice()).unwrap()["entries"].get("mode").is_none());
    // no grant at all: the workspace does not exist for the caller
    assert_eq!(ws.get_user_state(&Caller::user("nobody")).unwrap_err().code.as_str(), "NOT_FOUND");
    // a share export carries no user state
    let export = ws.export(&alice(), "share", true, None).unwrap();
    let bytes = std::fs::read(export["path"].as_str().unwrap()).unwrap();
    assert!(!bytes.windows(b"viewport:surface-main".len()).any(|w| w == b"viewport:surface-main"));
}

#[test]
fn generated_results_get_versions_that_can_be_restored() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let members = ws.read(&alice(), "tasks", None).unwrap()["content"]["members_rev"].clone();
    let derived = |run: &str| json!({ "wish_id": "tasks", "run_id": run, "executor": "mock", "simulated": true,
        "inputs": [{ "entity_id": "tasks", "selector": { "kind": "table_members" }, "version": { "mode": "follow", "rev": members } }] });
    // run 1: a record result with its record
    let r1 = ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "kpi", "type_id": "buckyos.record", "parent_id": "surface-main-content", "order_key": "k",
          "payload": { "schema": { "properties": [{ "key": "open", "name": "未完成", "type": "number" }] }, "props": { "open": 4 } } },
        { "op": "entity.set_derived", "entity_id": "kpi", "derived": derived("run-1") }]));
    let seq1 = r1["seq"].as_u64().unwrap();
    // run 2 overwrites in place: same identity, a new version
    let rev = ws.read(&alice(), "kpi", None).unwrap()["content"]["key_revs"]["p:open"].clone();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.set_keys", "entity_id": "kpi", "keys": [{ "key": "p:open", "value": 3, "expect": { "rev": rev } }] },
        { "op": "entity.set_derived", "entity_id": "kpi", "derived": derived("run-2") }]));
    let versions = ws.list_versions(&alice(), "kpi").unwrap();
    let list = versions["versions"].as_array().unwrap();
    assert_eq!(list.len(), 2, "{versions}");
    assert_eq!(list[0]["derived"]["run_id"], "run-2");
    assert_eq!(list[1]["derived"]["run_id"], "run-1");
    assert_eq!(list[1]["content_rev"], json!(seq1));
    assert_eq!(list[0]["kind"], "generated");
    // the read envelope carries the dependency record; freshness is current
    let read = ws.read(&alice(), "kpi", None).unwrap();
    assert_eq!(read["derived"]["run_id"], "run-2");
    assert_eq!(ws.freshness(&alice(), &["kpi".into()]).unwrap()["items"][0]["status"], "current");
    // restoring version 1 is an ordinary commit: content and dependency record come back, undo works
    let plan = ws.restore_version_plan(&alice(), "kpi", seq1).unwrap();
    let ops = plan["operations"].clone();
    assert_eq!(plan["derived"]["run_id"], "run-1");
    let restored = ok(&mut ws, &alice(), ops);
    let read = ws.read(&alice(), "kpi", None).unwrap();
    assert_eq!(read["content"]["props"]["open"], 4);
    assert_eq!(read["derived"]["run_id"], "run-1");
    let epoch = ws.epoch.clone();
    let undo = ws.undo(&alice(), &json!({ "epoch": epoch, "commit_id": restored["commit_id"], "idempotency_key": "undo-restore" }));
    assert_eq!(undo["status"], "accepted", "{undo}");
    let read = ws.read(&alice(), "kpi", None).unwrap();
    assert_eq!(read["content"]["props"]["open"], 3);
    assert_eq!(read["derived"]["run_id"], "run-2");
    // a rich text version restores through block operations
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_derived", "entity_id": "notes", "derived": derived("run-n1") }]));
    let notes_v1 = ws.read(&alice(), "notes", None).unwrap()["content_rev"].as_u64().unwrap();
    let first_block = ws.read(&alice(), "notes", None).unwrap()["content"]["content"]["content"][0]["attrs"]["block_id"].as_str().unwrap().to_string();
    let hash = ws.read(&alice(), "notes", None).unwrap()["content"]["blocks"][&first_block]["hash"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "richtext.replace_block", "entity_id": "notes", "block_id": first_block, "expect": { "hash": hash },
        "node": { "type": "paragraph", "attrs": { "block_id": first_block }, "content": [{ "type": "text", "text": "改掉的标题" }] } }]));
    assert_eq!(ws.freshness(&alice(), &["notes".into()]).unwrap()["items"][0]["manual_modified"], true);
    let plan = ws.restore_version_plan(&alice(), "notes", notes_v1).unwrap();
    assert!(plan["operations"].as_array().unwrap().iter().any(|op| op["op"].as_str().unwrap().starts_with("richtext.")));
    ok(&mut ws, &alice(), plan["operations"].clone());
    assert_eq!(ws.read(&alice(), "notes", None).unwrap()["content"]["content"]["content"][0]["content"][0]["text"], "本周项目进展");
    // tables are not restorable in this phase; unknown versions are not found
    assert_eq!(ws.restore_version_plan(&alice(), "kpi", 999).unwrap_err().code.as_str(), "NOT_FOUND");
    // bob without read sees nothing of it
    ws.grant(&alice(), "bob", Some("data"), &["read".into()]).unwrap();
    assert!(ws.list_versions(&bob(), "kpi").unwrap()["versions"].as_array().unwrap().len() >= 2);
    ws.revoke(&alice(), "bob", Some("data")).unwrap();
    assert_eq!(ws.list_versions(&bob(), "kpi").unwrap_err().code.as_str(), "NOT_FOUND");
}

#[test]
fn grants_as_seen_by_non_managers_and_canvas_permissions() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "bob", None, &["read".into()]).unwrap();
    ws.grant(&alice(), "carol", None, &["read".into(), "comment".into()]).unwrap();
    // a canvas permission is two rows: the Surface and its content folder (phase two §6.3)
    ws.grant(&alice(), "bob", Some("surface-main"), &["structure".into(), "update".into()]).unwrap();
    ws.grant(&alice(), "bob", Some("surface-main-content"), &["structure".into(), "update".into(), "append".into()]).unwrap();
    let all = ws.list_grants(&alice()).unwrap();
    assert_eq!(all["complete"], true);
    assert_eq!(all["grants"].as_array().unwrap().len(), 5);
    // bob sees only his own rows, not carol's
    let mine = ws.list_grants(&bob()).unwrap();
    assert_eq!(mine["complete"], false);
    let subjects: Vec<&str> = mine["grants"].as_array().unwrap().iter().map(|g| g["subject"].as_str().unwrap()).collect();
    assert!(subjects.iter().all(|s| *s == "bob"), "{subjects:?}");
    assert_eq!(subjects.len(), 3);
    // bob can place a Block on the Surface and create data in its content folder, but not touch first-level data
    let life = ws.read(&bob(), "cell-info", None).unwrap()["life_rev"].clone();
    let _ = life;
    assert_eq!(commit(&mut ws, &bob(), json!([{ "op": "tree.place", "entity_id": "cell-info", "order_key": "h7" }]))["status"], "accepted");
    assert_eq!(commit(&mut ws, &bob(), json!([{ "op": "entity.create", "entity_id": "bob-note", "type_id": "buckyos.annotation", "parent_id": "surface-main-content", "order_key": "z",
        "payload": { "kind": "note", "body": "便签" } }]))["status"], "rejected", "annotations need comment, which bob lacks");
    assert_eq!(commit(&mut ws, &bob(), json!([{ "op": "entity.create", "entity_id": "bob-rec", "type_id": "buckyos.record", "parent_id": "surface-main-content", "order_key": "z",
        "payload": { "schema": { "properties": [] }, "props": {} } }]))["status"], "accepted");
    assert_eq!(code(&commit(&mut ws, &bob(), json!([{ "op": "entity.create", "entity_id": "bob-rec2", "type_id": "buckyos.record", "parent_id": "data", "order_key": "z",
        "payload": { "schema": { "properties": [] }, "props": {} } }]))), "PERMISSION_DENIED");
}

#[test]
fn mock_program_writes_data_and_block_into_the_two_trees() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let run = ws.proc_start(&alice(), "mock.task-summary@1", &json!({ "today": "2026-10-13" }), "run-p2").unwrap();
    assert_eq!(run["state"], "waiting_confirmation", "{run}");
    let applied = ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), None).unwrap();
    assert_eq!(applied["state"], "succeeded", "{applied}");
    let outline = ws.outline(&alice()).unwrap();
    let parent = |eid: &str| outline["entities"].as_array().unwrap().iter().find(|e| e["entity_id"] == eid).map(|e| e["parent_id"].as_str().unwrap().to_string());
    assert_eq!(parent("summary").as_deref(), Some("data"));
    assert_eq!(parent("cell-summary").as_deref(), Some("surface-main"));
    let _ = CommitOpts::default();
    let _: Value = Value::Null;
}
