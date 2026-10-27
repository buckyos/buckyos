//! FORMAT 1.0 contracts of the store (内核架构改进 M1): the `run` capability, user state revisions, the
//! history floor, retention roots and the usage ledger, the run's writing identity and the derived origin.

mod common;

use aiworkspace_store::workspace::{Caller, CommitOpts};
use common::*;
use serde_json::{json, Value};

fn table_ops() -> Value {
    json!([{ "op": "entity.create", "entity_id": "t", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "t",
             "payload": { "fields": [{ "field_id": "title", "name": "标题", "type": "text" }, { "field_id": "n", "name": "数", "type": "number" }] } },
           { "op": "table.insert_records", "source_id": "t", "records": [{ "record_id": "r1", "values": { "title": "a", "n": 1 } }] }])
}

#[test]
fn a_run_only_subject_starts_runs_but_cannot_commit() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert!(ws.grant(&alice(), "bob", None, &["read".into(), "run".into()]).is_ok());
    assert!(ws.grant_with(&alice(), "carol", None, &["read".into()], Some("d1"), None).is_err(), "delegation fields need run");
    ws.grant_with(&alice(), "carol", None, &["read".into(), "run".into()], Some("digest-1"), Some(&json!([{ "name": "summary", "parent_id": "data" }]))).unwrap();
    let grants = ws.list_grants(&alice()).unwrap();
    let carol = grants["grants"].as_array().unwrap().iter().find(|g| g["subject"] == "carol").unwrap();
    assert_eq!((carol["task_digest"].as_str(), carol["outputs"][0]["name"].as_str()), (Some("digest-1"), Some("summary")));
    assert_eq!(ws.run_grants(&Caller::user("carol")).unwrap()[0].1.as_deref(), Some("digest-1"));
    // bob writes nothing by hand, but runs the Mock
    let denied = commit(&mut ws, &bob(), json!([{ "op": "entity.rename", "entity_id": "tasks", "name": "x", "expect": { "rev": 1 } }]));
    assert_eq!(code(&denied), "PERMISSION_DENIED", "{denied}");
    let run = ws.proc_start(&bob(), "mock.task-summary@1", &json!({ "today": "2026-10-13" }), "run-bob").unwrap();
    assert_eq!(run["state"], "waiting_confirmation", "{run}");
    // a reader without run cannot
    ws.grant(&alice(), "dave", None, &["read".into()]).unwrap();
    assert_eq!(ws.proc_start(&Caller::user("dave"), "mock.task-summary@1", &json!({ "today": "2026-10-13" }), "run-dave").unwrap_err().code.as_str(), "PERMISSION_DENIED");
    // two users applying the same run meet one accepted record, written as the run's identity (D08)
    let applied = ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), None).unwrap();
    assert_eq!(applied["state"], "succeeded", "{applied}");
    let again = ws.proc_apply(&bob(), run["run_id"].as_str().unwrap(), None).unwrap();
    assert_eq!(again["result"]["commit_id"], applied["result"]["commit_id"]);
    let sub = ws.get_submission(&bob(), &ws.epoch.clone(), &format!("run/{}", run["run_id"].as_str().unwrap())).unwrap();
    assert_eq!(sub["status"], "accepted", "the record is bob's (the run's principal)");
    let changes = ws.get_changes(&alice(), &ws.epoch.clone(), 0, 100, false).unwrap();
    let last = changes["changes"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["origin"].as_str(), last["author"].as_str()), (Some("program"), Some("bob")));
}

#[test]
fn origin_is_derived_by_the_server() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let rev = ws.read(&alice(), "tasks", None).unwrap()["meta_rev"].as_u64().unwrap();
    let mut req = request(&ws, json!([{ "op": "entity.rename", "entity_id": "tasks", "name": "renamed", "expect": { "rev": rev } }]));
    req["origin"] = json!("agent");
    let r = ws.commit(&req, &alice(), &CommitOpts::default());
    assert_eq!(r["status"], "accepted", "{r}");
    let changes = ws.get_changes(&alice(), &ws.epoch.clone(), 0, 100, false).unwrap();
    let last = changes["changes"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["origin"], "human", "a user's token is human whatever the client wrote");
    ws.grant(&alice(), "did:agent:x", None, &["read".into(), "update".into(), "structure".into()]).unwrap();
    let rev = ws.read(&alice(), "tasks", None).unwrap()["meta_rev"].as_u64().unwrap();
    let r = commit(&mut ws, &Caller::agent("did:agent:x"), json!([{ "op": "entity.rename", "entity_id": "tasks", "name": "by agent", "expect": { "rev": rev } }]));
    assert_eq!(r["status"], "accepted", "{r}");
    let changes = ws.get_changes(&alice(), &ws.epoch.clone(), 0, 100, false).unwrap();
    assert_eq!(changes["changes"].as_array().unwrap().last().unwrap()["origin"], "agent");
}

#[test]
fn user_state_revisions_and_block_state_cleanup() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let first = ws.set_user_state(&alice(), &json!({ "panel": { "open": true }, "block:b1/draft": "x" }), None).unwrap();
    assert_eq!(first["revisions"]["panel"], 1);
    let state = ws.get_user_state(&alice()).unwrap();
    assert_eq!((state["entries"]["panel"]["open"].as_bool(), state["revisions"]["panel"].as_i64()), (Some(true), Some(1)));
    // a second client writes with the revision it read: fine; the first one, with its stale revision, conflicts
    let second = ws.set_user_state(&alice(), &json!({ "panel": { "open": false } }), Some(&json!({ "panel": 1 }))).unwrap();
    assert_eq!(second["revisions"]["panel"], 2);
    let stale = ws.set_user_state(&alice(), &json!({ "panel": { "open": true } }), Some(&json!({ "panel": 1 }))).unwrap_err();
    assert_eq!((stale.code.as_str(), stale.data.as_ref().unwrap()["current"].as_i64()), ("REVISION_CONFLICT", Some(2)));
    assert!(ws.set_user_state(&alice(), &json!({ "fresh": 1 }), Some(&json!({ "fresh": 0 }))).is_ok(), "a new key expects 0");
    assert_eq!(ws.clear_block_state(&["b1".into()]).unwrap(), 1);
    assert!(ws.get_user_state(&alice()).unwrap()["entries"].get("block:b1/draft").is_none());
}

#[test]
fn a_fixed_version_resolves_only_while_a_snapshot_retains_it() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "固定", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let created = ok(&mut ws, &alice(), table_ops());
    let seq = created["seq"].as_u64().unwrap();
    let epoch = ws.epoch.clone();
    let at = |seq: u64| json!({ "entity_id": "t", "mode": "fixed", "version": { "epoch": epoch, "seq": seq } });
    // nothing retains seq 1 yet
    assert_eq!(ws.resolve(&alice(), Some(&at(seq)), None).unwrap_err().code.as_str(), "VERSION_NOT_RETAINED");
    let cp = ws.checkpoint(&alice()).unwrap();
    assert_eq!(cp.seq, seq);
    let r = ws.resolve(&alice(), Some(&at(seq)), None).unwrap();
    assert_eq!((r["fixed"].as_bool(), r["snapshot_id"].as_str(), r["object"]["ws_type"].as_str()), (Some(true), Some(cp.snapshot_id.as_str()), Some("buckyos.table-source")));
    assert_eq!(r["canonical"]["version"]["seq"], seq);
    // a later change does not move the fixed version; its own seq is not retained
    ok(&mut ws, &alice(), json!([{ "op": "entity.rename", "entity_id": "t", "name": "later", "expect": { "rev": seq } }]));
    assert_eq!(ws.resolve(&alice(), Some(&at(seq)), None).unwrap()["object_id"], r["object_id"]);
    assert_eq!(ws.resolve(&alice(), Some(&at(seq + 1)), None).unwrap_err().code.as_str(), "VERSION_NOT_RETAINED");
    // releasing the checkpoint's retention (its binding) makes the version unavailable, and an unknown epoch never resolves
    assert!(ws.release_retention("snapshot", &cp.snapshot_id).unwrap());
    assert!(env.bound("alice", "objects", &format!("{id}/snapshot/{}", cp.snapshot_id)).is_none());
    assert_eq!(ws.resolve(&alice(), Some(&at(seq)), None).unwrap_err().code.as_str(), "VERSION_NOT_RETAINED");
    assert_eq!(ws.resolve(&alice(), Some(&json!({ "entity_id": "t", "mode": "fixed", "version": { "epoch": "ep_other", "seq": 1 } })), None).unwrap_err().code.as_str(), "VERSION_NOT_RETAINED");
}

#[test]
fn history_floor_retention_roots_and_usage_ledger() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "格式", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let info = ws.info(&alice()).unwrap();
    assert_eq!((info["base_seq"].as_u64(), info["planner_version"].as_u64(), info["usage"]["total"].as_i64()), (Some(0), Some(1), Some(0)));
    assert!(info["richtext_schema"].as_str().is_some());
    // a Commit moves the content ledger by what it wrote
    let created = ok(&mut ws, &alice(), table_ops());
    let usage = ws.usage().unwrap();
    assert!(usage["content"].as_i64().unwrap() > 0, "{usage}");
    let before = usage["content"].as_i64().unwrap();
    let rev = created["seq"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "t", "values": [{ "record_id": "r1", "field_id": "n", "value": 1234567, "expect": { "rev": rev } }] }]));
    assert!(ws.usage().unwrap()["content"].as_i64().unwrap() > before);
    // a staged upload, a checkpoint and an export are bindings in the owner's buckets
    let staged = ws.stage_asset(&alice(), b"hello").unwrap();
    let object_id = staged["object_id"].as_str().unwrap();
    assert_eq!(env.bound("alice", "assets", &format!("{id}/{object_id}")).as_deref(), Some(object_id));
    assert_eq!(staged["bucket_id"], "user/aiworkspace/alice/assets");
    let cp = ws.checkpoint(&alice()).unwrap();
    assert!(env.bound("alice", "objects", &format!("{id}/snapshot/{}", cp.snapshot_id)).is_some_and(|d| d.starts_with("cydir:")));
    let ex = ws.export(&alice(), "share", true, None).unwrap();
    let key = format!("{id}/export/{}.bcanvas", ex["export_id"].as_str().unwrap());
    assert_eq!((ex["download"]["bucket_id"].as_str(), ex["download"]["key"].as_str()), (Some("user/aiworkspace/alice/exports"), Some(key.as_str())));
    assert!(ex["download"]["token"].as_str().is_some_and(|t| t.starts_with("bkt_")));
    assert_eq!(env.bound("alice", "exports", &key), ex["download"]["obj_id"].as_str().map(str::to_string));
    let usage = ws.usage().unwrap();
    assert!(usage.get("exports").is_none() && usage["staged"].as_i64().unwrap() == 5, "{usage}");
    assert!(ws.release_retention("export", ex["export_id"].as_str().unwrap()).unwrap());
    assert!(env.bound("alice", "exports", &key).is_none());
    assert!(!ws.release_retention("export", ex["export_id"].as_str().unwrap()).unwrap());
    // the floor: nothing below base_seq 0, so every request passes; the code path is the error's
    assert!(ws.get_changes(&alice(), &ws.epoch.clone(), 0, 10, false).is_ok());
}
