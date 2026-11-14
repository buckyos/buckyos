//! What the UI framework stores and the home page reads (UI 框架 PRD §4.6, §9, §12; K4, K6, K7): the Workspace's shared
//! settings (the root's `config`, `manage` to write, carried by a clone), a Surface's toolbar (`structure`), personal
//! preferences across Workspaces, the home page's fields of `ws.list`, and the sessions a caller may open without a link.

mod common;

use aiworkspace_store::workspace::Caller;
use common::*;
use serde_json::{json, Value};

const PRESENTATION: &str = include_str!("../../fixtures/presentation/commits.json");

fn set_settings(ws: &mut aiworkspace_store::Workspace, who: &Caller, value: Value) -> Value {
    let (_, rev) = ws.settings().unwrap();
    commit(ws, who, json!([{ "op": "entity.set_keys", "entity_id": "root", "keys": [{ "key": "config", "value": value, "expect": { "rev": rev } }] }]))
}

#[test]
fn workspace_settings_are_content_written_by_managers_and_carried_by_a_clone() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "设置", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    {
        let mut ws = handle.lock().unwrap();
        assert_eq!(ws.settings().unwrap(), (json!({}), 0));
        let toolbar = json!({ "toolbar": { "zones": { "tools": { "hidden": true, "locked": true } } } });
        let r = set_settings(&mut ws, &alice(), toolbar.clone());
        assert_eq!(r["status"], "accepted", "{r}");
        assert_eq!(ws.settings().unwrap().0, toolbar);
        // an editor of the whole Workspace without `manage` cannot change them; a property write is judged the same
        ws.grant(&alice(), "bob", None, &caps(&["read", "update", "structure"])).unwrap();
        let r = set_settings(&mut ws, &bob(), json!({ "toolbar": {} }));
        assert_eq!(code(&r), "PERMISSION_DENIED", "{r}");
        let r = commit(&mut ws, &bob(), json!([{ "op": "entity.set_props", "entity_id": "root", "key": "config", "props": { "toolbar": null } }]));
        assert_eq!(code(&r), "PERMISSION_DENIED", "{r}");
        // the root's title stays an `update` write
        let r = commit(&mut ws, &bob(), json!([{ "op": "entity.set_keys", "entity_id": "root", "keys": [{ "key": "title", "value": "改名", "expect": "any" }] }]));
        assert_ne!(code(&r), "PERMISSION_DENIED", "{r}");
        // a manager can, and it is an ordinary undoable Commit
        ws.grant(&alice(), "bob", None, &caps(&["read", "update", "structure", "manage"])).unwrap();
        let r = set_settings(&mut ws, &bob(), json!({ "toolbar": { "zones": {} } }));
        assert_eq!(r["status"], "accepted", "{r}");
        let req = json!({ "epoch": ws.epoch, "commit_id": r["commit_id"], "idempotency_key": "undo-settings", "session_id": "s1" });
        let undone = ws.undo(&bob(), &req);
        assert_eq!(undone["status"], "accepted", "{undone}");
        assert_eq!(ws.settings().unwrap().0, toolbar);
        // only an object of at most 64 KiB
        let r = set_settings(&mut ws, &alice(), json!("hidden"));
        assert_eq!(code(&r), "INVALID_SCHEMA", "{r}");
        let r = set_settings(&mut ws, &alice(), json!({ "pad": "x".repeat(70 * 1024) }));
        assert_eq!(code(&r), "LIMIT_EXCEEDED", "{r}");
    }
    // a clone carries them (they are part of the content)
    let cloned = env.svc.clone_workspace(&alice(), &id).unwrap();
    let copy = env.svc.workspace(cloned["workspace_id"].as_str().unwrap()).unwrap();
    assert_eq!(copy.lock().unwrap().settings().unwrap().0["toolbar"]["zones"]["tools"], json!({ "hidden": true, "locked": true }));
}

#[test]
fn a_surface_toolbar_needs_structure_on_the_surface() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "画布", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "free-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "free", "title": "Free" } },
        { "op": "entity.create", "entity_id": "free", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "Free", "content_folder_id": "free-content", "config": { "background": "#ffffff" } } },
    ]));
    ws.grant(&alice(), "bob", None, &caps(&["read", "update"])).unwrap();
    let set = |ws: &mut aiworkspace_store::Workspace, who: &Caller, props: Value| commit(ws, who, json!([{ "op": "entity.set_props", "entity_id": "free", "key": "config", "props": props }]));
    // page settings stay an `update` write; the toolbar is `structure`
    let r = set(&mut ws, &bob(), json!({ "background": "#eeeeee" }));
    assert_eq!(r["status"], "accepted", "{r}");
    let r = set(&mut ws, &bob(), json!({ "toolbar": { "zones": { "tools": { "hidden": true } } } }));
    assert_eq!(code(&r), "PERMISSION_DENIED", "{r}");
    ws.grant(&alice(), "bob", Some("free"), &caps(&["read", "update", "structure"])).unwrap();
    let r = set(&mut ws, &bob(), json!({ "toolbar": { "zones": { "tools": { "hidden": true } } } }));
    assert_eq!(r["status"], "accepted", "{r}");
    let read = ws.read(&alice(), "free", None).unwrap();
    assert_eq!(read["content"]["payload"]["config"], json!({ "background": "#eeeeee", "toolbar": { "zones": { "tools": { "hidden": true } } } }));
}

#[test]
fn personal_preferences_follow_the_person_not_the_workspace() {
    let env = env();
    let users = &env.svc.users;
    let empty = users.get_prefs(&alice()).unwrap();
    assert_eq!((&empty["prefs"], &empty["revision"]), (&json!({}), &json!(0)));
    let r = users.set_prefs(&alice(), &json!({ "merge": { "inspector_dock": "right", "home": { "tab": "recent" } } }), T0).unwrap();
    assert_eq!(r["revision"], 1);
    // a merge keeps the other keys; null removes one
    let r = users.set_prefs(&alice(), &json!({ "merge": { "home": null, "status_expanded": true }, "expect": 1 }), T0 + 1).unwrap();
    assert_eq!(r["prefs"], json!({ "inspector_dock": "right", "status_expanded": true }));
    // a stale revision is a conflict that names the current one
    let e = users.set_prefs(&alice(), &json!({ "merge": { "x": 1 }, "expect": 1 }), T0 + 2).unwrap_err();
    assert_eq!((e.code.as_str(), e.data.as_ref().unwrap()["current"].as_i64()), ("REVISION_CONFLICT", Some(2)));
    // another person is not affected; the whole object can be replaced
    assert_eq!(users.get_prefs(&bob()).unwrap()["prefs"], json!({}));
    users.set_prefs(&bob(), &json!({ "prefs": { "inspector_dock": "left" } }), T0).unwrap();
    assert_eq!(users.get_prefs(&alice()).unwrap()["prefs"]["inspector_dock"], "right");
    // the store survives a restart
    let env = env.reopen();
    assert_eq!(env.svc.users.get_prefs(&alice()).unwrap()["revision"], 2);
    // a guest keeps none; malformed writes are refused
    let guest = Caller::guest(&format!("gt_{}", "A".repeat(43))).unwrap();
    assert_eq!(env.svc.users.get_prefs(&guest).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    assert_eq!(env.svc.users.set_prefs(&alice(), &json!({ "merge": { "": 1 } }), T0).unwrap_err().code.as_str(), "INVALID_OPERATION");
    assert_eq!(env.svc.users.set_prefs(&alice(), &json!({ "prefs": {}, "merge": {} }), T0).unwrap_err().code.as_str(), "INVALID_OPERATION");
    assert_eq!(env.svc.users.set_prefs(&alice(), &json!({ "merge": { "big": "x".repeat(300 * 1024) } }), T0).unwrap_err().code.as_str(), "LIMIT_EXCEEDED");
}

#[test]
fn the_workspace_list_carries_the_home_page_fields() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "首页", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let item = |who: &Caller| -> Value {
        let list = env.svc.list_workspaces(who).unwrap();
        list["workspaces"].as_array().unwrap().iter().find(|w| w["workspace_id"] == json!(id)).cloned().unwrap_or(Value::Null)
    };
    let first = item(&alice());
    assert_eq!((&first["owner_principal"], &first["opened_at"], &first["public"], &first["cover"], &first["live_sessions"]), (&json!("alice"), &Value::Null, &json!(false), &Value::Null, &json!(0)));
    assert!(first["updated_at"].is_string(), "{first}");
    // a Commit moves the modification time; opening records the caller's own time only
    env.advance(60_000);
    {
        let handle = env.svc.workspace(&id).unwrap();
        let mut ws = handle.lock().unwrap();
        ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "note", "type_id": "buckyos.richtext", "parent_id": "data", "order_key": "a", "name": "笔记", "payload": {} }]));
        ws.grant(&alice(), "bob", None, &caps(&["read"])).unwrap();
    }
    env.svc.users.record_open(&alice(), &id, T0 + 90_000).unwrap();
    let after = item(&alice());
    assert!(after["updated_at"].as_str().unwrap() > first["updated_at"].as_str().unwrap(), "{after}");
    assert_eq!(after["opened_at"], "2026-10-04T08:01:30.000Z");
    let for_bob = item(&bob());
    assert_eq!((&for_bob["opened_at"], &for_bob["owner_principal"], &for_bob["capabilities"]), (&Value::Null, &json!("alice"), &json!(["read"])));
    // anonymous access with something granted to `@guest` makes it public
    env.svc.workspace(&id).unwrap().lock().unwrap().acl_set_anonymous(&alice(), true).unwrap();
    assert_eq!(item(&alice())["public"], false, "the switch alone grants nothing");
    env.svc.workspace(&id).unwrap().lock().unwrap().acl_set(&alice(), &json!([{ "subject": "@guest", "resource": "/", "allow": ["read"] }])).unwrap();
    let public = item(&alice());
    assert_eq!((&public["public"], &public["anonymous"], &public["published"]), (&json!(true), &json!(true), &json!(false)));
}

/// A Workspace owned by alice with the third phase's presentation fixture (`path-intro`).
fn courseware(env: &Env) -> String {
    let id = env.svc.create_workspace(&alice(), "课件", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    let fx: Value = serde_json::from_str(PRESENTATION).unwrap();
    for c in fx["commits"].as_array().unwrap() {
        ok(&mut ws, &alice(), c["operations"].clone());
    }
    id
}

#[test]
fn the_public_lecture_catalogue_lists_what_the_caller_may_open_without_a_link() {
    let env = env();
    let src = courseware(&env);
    let created = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第一讲" })).unwrap();
    let sid = created["session"]["session_id"].as_str().unwrap().to_string();
    let titles = |who: &Caller| -> Vec<(String, String, bool)> {
        env.svc.session_list_open(who, 100).unwrap()["sessions"].as_array().unwrap().iter()
            .map(|s| (s["session_id"].as_str().unwrap().to_string(), s["my_role"].as_str().unwrap().to_string(), s["presenting"].as_bool().unwrap())).collect()
    };
    assert_eq!(titles(&alice()), vec![(sid.clone(), "presenter".to_string(), true)]);
    // bob has nothing on the courseware: neither listed nor admitted
    assert!(titles(&bob()).is_empty());
    let row = env.svc.sessions.require(&sid).unwrap();
    assert!(env.svc.session_actor(&row, &bob(), None).unwrap().is_none());
    // as a reader of the courseware he may open it without a link, and has never joined
    env.svc.workspace(&src).unwrap().lock().unwrap().grant(&alice(), "bob", None, &caps(&["read"])).unwrap();
    assert_eq!(titles(&bob()), vec![(sid.clone(), "member".to_string(), false)]);
    let item = &env.svc.session_list_open(&bob(), 100).unwrap()["sessions"][0];
    assert_eq!((&item["opened_at"], &item["cover"], &item["state"]), (&Value::Null, &Value::Null, &json!("prepared")));
    assert!(item.get("baseline").is_none(), "the courseware's name is the presenters' and owners'");
    // joining makes him audience; he reads the run Workspace through the readers' group
    let (actor, _, first) = env.svc.session_join(&row, &bob(), None, Some("Bob")).unwrap();
    assert!(first);
    assert_eq!(actor.role.as_str(), "audience");
    env.svc.workspace(&row.workspace_id).unwrap().lock().unwrap().require_ws_any(&bob()).unwrap();
    assert_eq!(titles(&bob()), vec![(sid.clone(), "audience".to_string(), false)]);
    // a room closed to new audience is not offered to readers who have not joined, nor opened by them
    let carol = Caller::user("carol");
    env.svc.workspace(&src).unwrap().lock().unwrap().grant(&alice(), "carol", None, &caps(&["read"])).unwrap();
    assert_eq!(titles(&carol).len(), 1);
    let mut closed = env.svc.sessions.require(&sid).unwrap();
    let presenter = env.svc.require_actor(&closed, &alice(), None).unwrap();
    env.svc.session_set_access(&mut closed, &presenter, &json!({ "access": "closed" })).unwrap();
    assert!(titles(&carol).is_empty());
    assert!(env.svc.session_actor(&env.svc.sessions.require(&sid).unwrap(), &carol, None).unwrap().is_none());
    assert_eq!(titles(&bob()).len(), 1, "who joined keeps it");
    // the presenter flag is the caller's own
    let other = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第二讲", "presenter": "bob" })).unwrap();
    let mine = env.svc.session_list_open(&bob(), 100).unwrap();
    let second = mine["sessions"].as_array().unwrap().iter().find(|s| s["session_id"] == other["session"]["session_id"]).unwrap();
    assert_eq!((&second["my_role"], &second["presenting"]), (&json!("presenter"), &json!(true)));
}
