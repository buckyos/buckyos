//! Workspace names (控件机制 TODO G9): fixed at creation, unique among one owner's Workspaces, `ws.ensure` creates one
//! Workspace however many callers race for a name, and a deleted Workspace frees its name.

mod common;

use common::*;
use serde_json::json;

#[test]
fn ensure_creates_once_and_names_are_per_owner() {
    let env = env();
    let first = env.svc.ensure_workspace(&alice(), "schedule", None).unwrap();
    assert_eq!((first["created"].clone(), first["title"].clone(), first["name"].clone()), (json!(true), json!("schedule"), json!("schedule")));
    let id = first["workspace_id"].as_str().unwrap().to_string();
    let again = env.svc.ensure_workspace(&alice(), "schedule", Some("ignored")).unwrap();
    assert_eq!((again["created"].clone(), again["workspace_id"].clone(), again["title"].clone()), (json!(false), json!(id), json!("schedule")));
    assert_eq!(again["epoch"], first["epoch"]);
    // another owner's name is another Workspace
    let bobs = env.svc.ensure_workspace(&bob(), "schedule", Some("Bob's")).unwrap();
    assert_ne!(bobs["workspace_id"], json!(id));
    assert_eq!(bobs["title"], "Bob's");
    // ws.create with a used name conflicts; the title stays free
    let e = env.svc.create_workspace_with(&alice(), "another", None, Some("schedule")).unwrap_err().to_json();
    assert_eq!((code(&e), e["sub_code"].as_str()), ("INVALID_OPERATION", Some("NAME_CONFLICT")), "{e}");
    assert_eq!(e["data"]["workspace_id"], json!(id));
    let named = env.svc.create_workspace_with(&alice(), "schedule", None, Some("notes.v2")).unwrap();
    assert_eq!(named["name"], "notes.v2");
    let long = "x".repeat(65);
    for bad in ["", "Schedule", "-x", ".x", "a b", "日程", long.as_str()] {
        let e = env.svc.ensure_workspace(&alice(), bad, None).unwrap_err().to_json();
        assert_eq!(code(&e), "INVALID_OPERATION", "{bad:?}");
    }
    assert!(env.svc.ensure_workspace(&alice(), &format!("0{}", "x".repeat(63)), None).is_ok());
    // listed and described with the name; a Workspace without one has none
    let plain = env.svc.create_workspace(&alice(), "plain", None).unwrap();
    assert!(plain.get("name").is_none());
    let list = env.svc.list_workspaces(&alice()).unwrap();
    let name_of = |id: &serde_json::Value| list["workspaces"].as_array().unwrap().iter().find(|w| w["workspace_id"] == *id).unwrap().get("name").cloned();
    assert_eq!(name_of(&json!(id)), Some(json!("schedule")));
    assert_eq!(name_of(&plain["workspace_id"]), None);
    let info = env.svc.workspace(&id).unwrap().lock().unwrap().info(&alice()).unwrap();
    assert_eq!(info["name"], "schedule");
    // the name survives a restart (the index is rebuilt from the folders)
    let env = env.reopen();
    let e = env.svc.create_workspace_with(&alice(), "x", None, Some("schedule")).unwrap_err().to_json();
    assert_eq!(e["sub_code"], "NAME_CONFLICT");
    assert_eq!(env.svc.ensure_workspace(&alice(), "schedule", None).unwrap()["workspace_id"], json!(id));
}

#[test]
fn concurrent_ensure_makes_one_workspace() {
    let env = env();
    let ids: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8).map(|_| s.spawn(|| env.svc.ensure_workspace(&alice(), "schedule", None).unwrap())).collect();
        handles.into_iter().map(|h| h.join().unwrap()["workspace_id"].as_str().unwrap().to_string()).collect()
    });
    assert!(ids.iter().all(|i| *i == ids[0]), "{ids:?}");
    assert_eq!(env.svc.list_workspaces(&alice()).unwrap()["workspaces"].as_array().unwrap().len(), 1);
}

#[test]
fn delete_frees_the_name_and_copies_carry_none() {
    let env = env();
    let first = env.svc.ensure_workspace(&alice(), "schedule", None).unwrap();
    let id = first["workspace_id"].as_str().unwrap().to_string();
    // a fork is a new Workspace without a name
    let forked = env.svc.fork(&alice(), &id).unwrap();
    let fork_info = env.svc.workspace(forked["workspace_id"].as_str().unwrap()).unwrap().lock().unwrap().info(&alice()).unwrap();
    assert!(fork_info.get("name").is_none(), "{fork_info}");
    env.svc.delete_workspace(&alice(), &id).unwrap();
    let second = env.svc.ensure_workspace(&alice(), "schedule", None).unwrap();
    assert_eq!(second["created"], true);
    assert_ne!(second["workspace_id"], json!(id));
    // and again after a restart, with the old folder in the trash
    let env = env.reopen();
    assert_eq!(env.svc.ensure_workspace(&alice(), "schedule", None).unwrap()["workspace_id"], second["workspace_id"]);
}

#[test]
fn packages_carry_no_name() {
    let env = env();
    let id = env.svc.ensure_workspace(&alice(), "schedule", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let pkg = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let r = ws.export(&alice(), "personal_backup", true, None).unwrap();
        fetched(&ws.objects, &r)
    };
    let name = |env: &Env, id: &str| env.svc.workspace(id).unwrap().lock().unwrap().name().unwrap();
    let new = env.svc.import(&alice(), &pkg, "new", false).unwrap();
    assert_eq!(name(&env, new["workspace_id"].as_str().unwrap()), None);
    // a restore over the Workspace keeps its name
    env.svc.import(&alice(), &pkg, "restore", true).unwrap();
    assert_eq!(name(&env, &id).as_deref(), Some("schedule"));
    assert_eq!(env.svc.ensure_workspace(&alice(), "schedule", None).unwrap()["workspace_id"], json!(id));
    // a show clone is a stage of it, not a second Workspace of that name
    let clone = env.svc.clone_for_show(&id, "sh_1", "2099-01-01T00:00:00.000Z").unwrap();
    assert_eq!(name(&env, &clone), None);
    // a restore that creates the Workspace again has none
    env.svc.delete_workspace(&alice(), &id).unwrap();
    env.svc.import(&alice(), &pkg, "restore", false).unwrap();
    assert_eq!(name(&env, &id), None);
    assert_ne!(env.svc.ensure_workspace(&alice(), "schedule", None).unwrap()["workspace_id"], json!(id));
}
