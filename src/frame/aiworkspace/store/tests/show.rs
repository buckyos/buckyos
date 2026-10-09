//! Non-public shows in the store (第三期规划 §8.1, §8.3): the show lock closes a Workspace for document writes
//! from every session, expires without renewal, and the temporary clone is a full copy that shares objects,
//! keeps grants, wish runs and ids, never shows up in the list and is removed when the service starts again.

mod common;

use aiworkspace_store::show::{is_clone_dir, ShowLock};
use common::*;
use serde_json::json;

const LEASE: i64 = 60_000;

fn lock(env: &Env, id: &str, show: &str) {
    let now = env.clock.load(std::sync::atomic::Ordering::SeqCst);
    env.svc
        .show_lock(id, ShowLock { show_id: show.into(), principal: "alice".into(), started_at: "2026-10-04T08:00:00.000Z".into(), expires_ms: now + LEASE })
        .unwrap();
}

#[test]
fn a_locked_workspace_refuses_every_write_until_the_lease_ends() {
    let env = env();
    let id = project(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.grant(&alice(), "bob", None, &["read".into(), "update".into(), "structure".into()]).unwrap();
    }
    lock(&env, &id, "sh_a");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let rev = cell_rev(&ws, &bob(), "task-42", "budget");
    for who in [alice(), bob()] {
        let r = commit(&mut ws, &who, set_cell("task-42", "budget", json!("9.00"), rev));
        assert_eq!(r["status"], "rejected", "{r}");
        assert_eq!(code(&r), "SHOW_LOCKED");
        assert_eq!(r["retryable"], true);
        assert_eq!(r["errors"][0]["data"]["presenter"], "alice");
        assert_eq!(r["errors"][0]["data"]["show_id"], "sh_a");
    }
    // the user's own work state is not a document write
    ws.set_user_state(&bob(), &json!({ "mode": "canvas" })).unwrap();
    assert_eq!(ws.info(&bob()).unwrap()["show_lock"]["presenter"], "alice");
    // a second show cannot take the same Workspace
    drop(ws);
    let r = env.svc.show_lock(&id, ShowLock { show_id: "sh_b".into(), principal: "bob".into(), started_at: String::new(), expires_ms: i64::MAX }).unwrap_err();
    assert_eq!(r.code.as_str(), "SHOW_LOCKED");
    // renewed, it holds; not renewed, it lapses (a crashed stage)
    env.advance(LEASE - 1000);
    env.svc.show_renew(&id, "sh_a", env.clock.load(std::sync::atomic::Ordering::SeqCst) + LEASE);
    env.advance(LEASE - 1000);
    let mut ws = h.lock().unwrap();
    assert_eq!(code(&commit(&mut ws, &bob(), set_cell("task-42", "budget", json!("9.00"), rev))), "SHOW_LOCKED");
    env.advance(2000);
    ok(&mut ws, &bob(), set_cell("task-42", "budget", json!("9.00"), rev));
    assert!(ws.info(&bob()).unwrap()["show_lock"].is_null());
    drop(ws);
    // releasing is per show
    lock(&env, &id, "sh_c");
    env.svc.show_unlock(&id, "sh_other");
    assert!(env.svc.workspace(&id).unwrap().lock().unwrap().show_lock().is_some());
    env.svc.show_unlock(&id, "sh_c");
    assert!(env.svc.workspace(&id).unwrap().lock().unwrap().show_lock().is_none());
}

#[test]
fn a_clone_is_a_private_full_copy_that_shares_objects() {
    let env = env();
    let id = project(&env);
    let objects_before = std::fs::read_dir(env.dir.path().join("objects/objects")).unwrap().count();
    let (head, epoch) = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.grant(&alice(), "bob", None, &["read".into()]).unwrap();
        ws.set_user_state(&alice(), &json!({ "mode": "canvas" })).unwrap();
        (ws.head_seq, ws.epoch.clone())
    };
    lock(&env, &id, "sh_x");
    let clone = env.svc.clone_for_show(&id, "sh_x", "2026-10-04T08:01:00.000Z").unwrap();
    assert_ne!(clone, id);
    assert!(is_clone_dir(&env.dir.path().join("workspaces").join(&clone)));
    // no bytes copied: assets, snapshots and programs stay where they are
    assert_eq!(std::fs::read_dir(env.dir.path().join("objects/objects")).unwrap().count(), objects_before);
    {
        let h = env.svc.workspace(&clone).unwrap();
        let mut ws = h.lock().unwrap();
        let info = ws.info(&alice()).unwrap();
        assert_eq!((info["head_seq"].as_u64().unwrap(), info["epoch"].as_str().unwrap()), (head, epoch.as_str()));
        assert_eq!(info["purpose"], "show");
        // same ids, same content, same grants; no user state
        assert_eq!(ws.read(&bob(), "tasks", None).unwrap()["content"]["record_count"], 5);
        assert_eq!(ws.read(&alice(), "diagram", None).unwrap()["content"]["availability"], "available");
        assert!(ws.get_user_state(&alice()).unwrap()["entries"].as_object().unwrap().is_empty());
        // the clone is writable while its original is locked, and the writes stay there
        let rev = cell_rev(&ws, &alice(), "task-42", "budget");
        ok(&mut ws, &alice(), set_cell("task-42", "budget", json!("1.00"), rev));
        let r = ws.replica_bootstrap(&alice()).unwrap_err();
        assert_eq!(r.code.as_str(), "INVALID_OPERATION");
    }
    let listed = env.svc.list_workspaces(&alice()).unwrap();
    assert_eq!(listed["workspaces"].as_array().unwrap().len(), 1, "{listed}");
    {
        let h = env.svc.workspace(&id).unwrap();
        let ws = h.lock().unwrap();
        assert_eq!(ws.head_seq, head);
        let budget = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_cell", "record_id": "task-42", "field_id": "budget" }))).unwrap();
        assert_eq!(budget["content"]["value"], "1200.00");
    }
    // only clones can be dropped this way
    assert_eq!(env.svc.drop_clone(&id).unwrap_err().code.as_str(), "INVALID_OPERATION");
    // a restarted service ends every show: the lock is gone and the clone is removed
    let env = env.reopen();
    assert!(!env.dir.path().join("workspaces").join(&clone).exists());
    assert!(env.svc.workspace(&id).unwrap().lock().unwrap().show_lock().is_none());
}

#[test]
fn dropping_a_clone_removes_it() {
    let env = env();
    let id = project(&env);
    let clone = env.svc.clone_for_show(&id, "sh_y", "").unwrap();
    env.svc.workspace(&clone).unwrap();
    env.svc.drop_clone(&clone).unwrap();
    assert!(!env.dir.path().join("workspaces").join(&clone).exists());
    assert_eq!(env.svc.workspace(&clone).err().unwrap().code.as_str(), "NOT_FOUND");
}
