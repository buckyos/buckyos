//! Kernel closure (内核架构改进 M2): bindings and their release, history resource retention, quota
//! admission and the maintenance channel, the restart scan, the history floor and show-clone access.

mod common;

use aiworkspace_store::workspace::Caller;
use common::*;
use serde_json::{json, Value};

fn asset_ops(object_id: &str) -> Value {
    json!([{ "op": "entity.create", "entity_id": "pic", "type_id": "buckyos.asset-ref", "parent_id": "data", "order_key": "p", "name": "pic", "payload": { "object_id": object_id } }])
}

#[test]
fn the_sweep_unbinds_what_nothing_retains_any_more() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "回收", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let (staged, cp, pic) = {
        let mut ws = h.lock().unwrap();
        let staged = ws.stage_asset(&alice(), b"a picture").unwrap();
        let oid = staged["object_id"].as_str().unwrap().to_string();
        ok(&mut ws, &alice(), asset_ops(&oid));
        let other = ws.stage_asset(&alice(), b"never referenced").unwrap()["object_id"].as_str().unwrap().to_string();
        let cp = ws.checkpoint(&alice()).unwrap();
        (other, cp, oid)
    };
    let (pic_key, staged_key, snapshot_key) = (format!("{id}/{pic}"), format!("{id}/{staged}"), format!("{id}/snapshot/{}", cp.snapshot_id));
    // nothing is released while it is fresh (an upload may still be about to be referenced)
    let r = env.svc.sweep().unwrap();
    assert_eq!((r["staged"].as_u64(), r["history"].as_u64()), (Some(0), Some(0)), "{r}");
    assert!(env.bound("alice", "assets", &staged_key).is_some());
    // two days later: the staged upload expired (24 h) and nothing references it — its binding goes; the picture
    // and the checkpoint stay bound
    env.advance(2 * 24 * 3_600_000);
    let r = env.svc.sweep().unwrap();
    assert_eq!(r["staged"], 1, "{r}");
    assert!(env.bound("alice", "assets", &staged_key).is_none(), "an expired, unreferenced upload is unbound");
    assert_eq!(env.bound("alice", "assets", &pic_key).as_deref(), Some(pic.as_str()));
    assert!(env.bound("alice", "objects", &snapshot_key).is_some());
    // delete the picture and the checkpoint: the snapshot goes at once, the history-only picture after its retention period
    {
        let mut ws = h.lock().unwrap();
        let rev = ws.read(&alice(), "pic", None).unwrap()["life_rev"].as_u64().unwrap();
        ok(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "pic", "expect": { "rev": rev } }]));
        ws.delete_checkpoint(&alice(), &cp.snapshot_id).unwrap();
    }
    assert!(env.bound("alice", "objects", &snapshot_key).is_none(), "a deleted checkpoint releases its retention set");
    env.advance(2 * 24 * 3_600_000);
    env.svc.sweep().unwrap();
    assert!(env.bound("alice", "assets", &pic_key).is_some(), "inside the retention period the history keeps it");
    env.advance(31 * 24 * 3_600_000);
    let r = env.svc.sweep().unwrap();
    assert_eq!(r["history"], 1, "{r}");
    assert!(env.bound("alice", "assets", &pic_key).is_none(), "after the retention period it is unbound");
    // the undo that would bring the picture back says so
    let mut ws = h.lock().unwrap();
    let changes = ws.get_changes(&alice(), &ws.epoch.clone(), 0, 10, false).unwrap();
    let delete_commit = changes["changes"].as_array().unwrap().iter().rev().find(|c| c["commit_id"].is_string()).unwrap()["commit_id"].clone();
    let epoch = ws.epoch.clone();
    let undone = ws.undo(&alice(), &json!({ "epoch": epoch, "commit_id": delete_commit, "idempotency_key": "undo-pic" }));
    assert_eq!(undone["code"], "RESOURCE_RECLAIMED", "{undone}");
}

#[test]
fn purge_releases_at_once_and_closes_undo() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "清除", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let oid = ws.stage_asset(&alice(), b"doomed").unwrap()["object_id"].as_str().unwrap().to_string();
    ok(&mut ws, &alice(), asset_ops(&oid));
    ws.set_user_state(&alice(), &json!({ "block:pic/draft": 1 }), None).unwrap();
    let rev = ws.read(&alice(), "pic", None).unwrap()["life_rev"].as_u64().unwrap();
    let deleted = ok(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "pic", "expect": { "rev": rev } }]));
    assert!(env.bound("alice", "assets", &format!("{id}/{oid}")).is_some(), "history keeps a deleted entity's asset");
    let purged = ws.purge_deleted(&alice(), None, None).unwrap();
    assert_eq!(purged["purged"], json!(["pic"]));
    assert!(env.bound("alice", "assets", &format!("{id}/{oid}")).is_none(), "purged content is released before any retention period");
    assert!(ws.get_user_state(&alice()).unwrap()["entries"].get("block:pic/draft").is_none());
    let (wid, epoch) = (ws.workspace_id.clone(), ws.epoch.clone());
    assert_eq!(code(&ws.commit(&json!({ "protocol_version": "1.0", "workspace_id": wid, "epoch": epoch, "idempotency_key": "r1",
        "operations": [{ "op": "entity.restore", "entity_id": "pic", "expect": { "rev": deleted["seq"] } }] }), &alice(), &Default::default())), "RESOURCE_RECLAIMED");
}

#[test]
fn quota_admission_and_the_maintenance_channel() {
    let mut env = env();
    env.svc.quota = Some(6_000);
    let id = env.svc.create_workspace(&alice(), "额度", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert_eq!(ws.quota, Some(6_000));
    let big = vec![b'x'; 4_000];
    ws.stage_asset(&alice(), &big).unwrap();
    // the next upload passes the quota; staged bytes are reclaimable, so: release and retry
    let e = ws.stage_asset(&alice(), &big).unwrap_err();
    assert_eq!((e.code.as_str(), e.data.as_ref().unwrap()["reclaimable_bytes"].as_u64()), ("QUOTA_EXCEEDED", Some(4_000)));
    assert!(e.code.retryable());
    // a Commit of that size is refused too, the maintenance methods are not
    let r = commit(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "big", "type_id": "buckyos.record", "parent_id": "data", "order_key": "b",
        "payload": { "schema": { "properties": [{ "key": "t", "name": "t", "type": "text" }] }, "props": { "t": "y".repeat(3_000) } } }]));
    assert_eq!(code(&r), "QUOTA_EXCEEDED", "{r}");
    let cleared = ws.clear_staged(&alice()).unwrap();
    assert_eq!(cleared["bytes"], 4_000);
    assert!(ws.stage_asset(&alice(), &big).is_ok(), "released space admits the upload");
    // with nothing to release the answer is the limit itself
    ws.clear_staged(&alice()).unwrap();
    ws.quota = Some(1);
    let e = ws.stage_asset(&alice(), b"tiny").unwrap_err();
    assert_eq!((e.code.as_str(), e.data.as_ref().unwrap()["reclaimable_bytes"].as_u64()), ("QUOTA_LIMIT", Some(0)));
    assert!(ws.purge_deleted(&alice(), None, None).is_ok(), "maintenance is accepted over quota");
}

#[test]
fn a_restart_marks_owned_runs_interrupted_before_anyone_asks() {
    let env = env();
    let id = project(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let (run_id, _) = ws.wish_run_create(&alice(), "wish.xllm@1", &json!({ "wish_id": "w", "stage": "analyze" }), "k1", &json!({ "mode": "workspace_owner", "user_id": "alice" })).unwrap();
        ws.wish_run_set(&run_id, Some("running"), None, None, None, None).unwrap();
    }
    let env = env.reopen();
    let h = env.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    let runs = ws.wish_runs_list(&alice(), None, 10).unwrap();
    let run = &runs[0];
    assert_eq!((run["state"].as_str(), run["detail"]["error"]["code"].as_str()), (Some("interrupted"), Some("INTERRUPTED")), "{run}");
}

#[test]
fn the_history_floor_refuses_what_lies_below_it() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let epoch = ws.epoch.clone();
    ws.set_base_seq(3).unwrap();
    assert_eq!(ws.info(&alice()).unwrap()["base_seq"], 3);
    assert_eq!(ws.get_changes(&alice(), &epoch, 1, 10, false).unwrap_err().code.as_str(), "BASE_TOO_OLD");
    assert!(ws.get_changes(&alice(), &epoch, 3, 10, false).is_ok());
    // undoing a commit below the floor is refused as well
    let below: Vec<Value> = ws.get_changes(&alice(), &epoch, 0, 100, false).map(|v| v["changes"].as_array().cloned().unwrap_or_default()).unwrap_or_default();
    let _ = below;
    ws.set_base_seq(0).unwrap();
    let early = ws.get_changes(&alice(), &epoch, 0, 2, false).unwrap()["changes"][1]["commit_id"].clone();
    ws.set_base_seq(3).unwrap();
    let undone = ws.undo(&alice(), &json!({ "epoch": epoch, "commit_id": early, "idempotency_key": "u" }));
    assert_eq!(undone["code"], "BASE_TOO_OLD", "{undone}");
}

#[test]
fn a_show_clone_follows_the_source_workspaces_current_grants() {
    let env = env();
    let id = project(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.grant(&alice(), "bob", None, &["read".into()]).unwrap();
    }
    let clone = env.svc.clone_for_show(&id, "sh_1", "2099-01-01T00:00:00.000Z").unwrap();
    {
        let h = env.svc.workspace(&clone).unwrap();
        let ws = h.lock().unwrap();
        assert!(ws.read(&bob(), "tasks", None).is_ok(), "bob reads the clone through the source's grant");
        assert_eq!(ws.read(&Caller::user("carol"), "tasks", None).unwrap_err().code.as_str(), "NOT_FOUND");
    }
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.revoke(&alice(), "bob", None).unwrap();
        assert!(ws.usage().unwrap()["clones"].as_i64().unwrap() > 0, "the clone counts on its source");
    }
    let h = env.svc.workspace(&clone).unwrap();
    let mut ws = h.lock().unwrap();
    assert_eq!(ws.read(&bob(), "tasks", None).unwrap_err().code.as_str(), "NOT_FOUND", "revoking on the source ends access to the clone");
    // the clone reads through the source's bindings; what it binds itself goes with it
    let own = ws.stage_asset(&alice(), b"written during the show").unwrap()["object_id"].as_str().unwrap().to_string();
    let source_asset = ws.read(&alice(), "diagram", None).unwrap()["content"]["payload"]["object_id"].as_str().unwrap().to_string();
    assert_eq!(ws.bucket("assets").unwrap(), "user/aiworkspace/alice/assets");
    drop(ws);
    assert!(env.bound("alice", "assets", &format!("{clone}/{own}")).is_some());
    env.svc.drop_clone(&clone).unwrap();
    assert!(env.bound("alice", "assets", &format!("{clone}/{own}")).is_none());
    assert!(env.bound("alice", "assets", &format!("{id}/{source_asset}")).is_some(), "the source's bindings stay");
    let h = env.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    assert_eq!(ws.usage().unwrap()["clones"].as_i64().unwrap_or(0), 0);
}
