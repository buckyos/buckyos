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
    env.svc.session_set_access(&mut closed, &presenter, &alice(), &json!({ "access": "closed" })).unwrap();
    assert!(titles(&carol).is_empty());
    assert!(env.svc.session_actor(&env.svc.sessions.require(&sid).unwrap(), &carol, None).unwrap().is_none());
    assert_eq!(titles(&bob()).len(), 1, "who joined keeps it");
    // the presenter flag is the caller's own
    let other = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第二讲", "presenter": "bob" })).unwrap();
    let mine = env.svc.session_list_open(&bob(), 100).unwrap();
    let second = mine["sessions"].as_array().unwrap().iter().find(|s| s["session_id"] == other["session"]["session_id"]).unwrap();
    assert_eq!((&second["my_role"], &second["presenting"]), (&json!("presenter"), &json!(true)));
}

/// Q19: a session's permission object is the Surface of its path's first step unless named; its readers (by path
/// inheritance, deny first) open the room without a link; it is bound by id, so renaming or moving the Surface keeps
/// it; the owner or the presenter may choose another object; one that is gone admits nobody.
#[test]
fn the_readers_of_a_sessions_permission_object_open_it_without_a_link() {
    let env = env();
    let src = courseware(&env);
    let created = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第一讲" })).unwrap();
    let sid = created["session"]["session_id"].as_str().unwrap().to_string();
    // the default: the canvas of the first step (frame1 is on 发布会), by id, named for the presenter
    assert_eq!(created["session"]["policy"]["readers_of"], "s-launch");
    assert_eq!((&created["session"]["readers_of"]["path"], &created["session"]["readers_of"]["title"], &created["session"]["readers_of"]["alive"]), (&json!("/surfaces/发布会"), &json!("发布会"), &json!(true)));
    let row = || env.svc.sessions.require(&sid).unwrap();
    let opens = |who: &Caller| env.svc.session_actor(&row(), who, None).unwrap().is_some_and(|a| a.role.as_str() == "audience");
    let listed = |who: &Caller| env.svc.session_list_open(who, 100).unwrap()["sessions"].as_array().unwrap().iter().any(|s| s["session_id"] == json!(sid));
    let carol = Caller::user("carol");
    let handle = env.svc.workspace(&src).unwrap();
    {
        let mut ws = handle.lock().unwrap();
        // bob reads only the canvas being presented, carol only the other one
        ws.grant(&alice(), "bob", Some("s-launch"), &caps(&["read"])).unwrap();
        ws.grant(&alice(), "carol", Some("s-work"), &caps(&["read"])).unwrap();
    }
    assert!(opens(&bob()) && listed(&bob()), "a reader of the presented canvas");
    assert!(!opens(&carol) && !listed(&carol), "a reader of another canvas only");
    // inherited from above, and refused by a deny on the way down
    let dave = Caller::user("dave");
    {
        let mut ws = handle.lock().unwrap();
        ws.grant(&alice(), "dave", Some("/surfaces"), &caps(&["read"])).unwrap();
    }
    assert!(opens(&dave) && listed(&dave), "read inherited from the canvases' folder");
    handle.lock().unwrap().acl_set(&alice(), &json!([{ "subject": "dave", "resource": "s-launch", "deny": ["read"] }])).unwrap();
    assert!(!opens(&dave) && !listed(&dave), "a deny on the canvas wins");
    // renamed and moved into a canvas folder: the same object, the same readers
    {
        let mut ws = handle.lock().unwrap();
        let life = ws.read(&alice(), "s-launch", None).unwrap()["life_rev"].as_u64().unwrap();
        ok(&mut ws, &alice(), json!([
            { "op": "entity.create", "entity_id": "cf-a", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "y", "payload": { "kind": "folder", "title": "归档" } },
            { "op": "entity.rename", "entity_id": "s-launch", "name": "发布会-改", "expect": { "rev": life } },
            { "op": "tree.move", "entity_id": "s-launch", "new_parent_id": "cf-a", "order_key": "a" },
        ]));
    }
    assert!(opens(&bob()) && listed(&bob()), "bob still reads it after the move");
    assert_eq!(env.svc.readers_of_json(&row())["path"], "/surfaces/归档/发布会-改");
    // the owner or the presenter may choose another object (a path or an id the caller reads there)
    let mut s = row();
    let owner = env.svc.require_actor(&s, &alice(), None).unwrap();
    let changed = env.svc.session_set_access(&mut s, &owner, &alice(), &json!({ "readers_of": "/surfaces/总结" })).unwrap();
    assert_eq!((&changed["policy"]["readers_of"], &changed["readers_of"]["title"]), (&json!("s-work"), &json!("总结")));
    assert!(opens(&carol) && listed(&carol), "carol reads the new object");
    assert!(!opens(&bob()) && !listed(&bob()), "bob does not");
    // the whole Workspace is an object too
    env.svc.session_set_access(&mut s, &owner, &alice(), &json!({ "readers_of": "/" })).unwrap();
    assert_eq!(env.svc.readers_of_json(&row())["path"], "/");
    assert!(!opens(&carol), "carol reads one canvas, not the Workspace");
    // who joined keeps the room; nobody else may choose; an object the caller cannot read is not found
    env.svc.session_set_access(&mut s, &owner, &alice(), &json!({ "readers_of": "s-launch" })).unwrap();
    env.svc.session_join(&row(), &bob(), None, Some("Bob")).unwrap();
    let audience = env.svc.require_actor(&row(), &bob(), None).unwrap();
    let mut s = row();
    assert_eq!(env.svc.session_set_access(&mut s, &audience, &bob(), &json!({ "readers_of": "/" })).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    let other = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第二讲", "presenter": "bob" })).unwrap();
    let mut s2 = env.svc.sessions.require(other["session"]["session_id"].as_str().unwrap()).unwrap();
    let presenter = env.svc.require_actor(&s2, &bob(), None).unwrap();
    assert_eq!(env.svc.session_set_access(&mut s2, &presenter, &bob(), &json!({ "readers_of": "s-work" })).unwrap_err().code.as_str(), "NOT_FOUND");
    assert_eq!(env.svc.session_set_access(&mut s2, &presenter, &bob(), &json!({ "readers_of": "s-launch" })).unwrap()["policy"]["readers_of"], "s-launch");
    assert!(env.svc.session_create(&bob(), &json!({ "workspace_id": src, "path_id": "path-intro", "readers_of": 7 })).is_err());
    // named at creation; deleted later: nobody comes in without a link, those who joined stay
    {
        let mut ws = handle.lock().unwrap();
        ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "d-class", "type_id": "buckyos.container", "parent_id": "data", "order_key": "zz", "name": "班级", "payload": { "kind": "folder", "title": "班级" } }]));
        ws.grant(&alice(), "carol", Some("d-class"), &caps(&["read"])).unwrap();
    }
    let third = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第三讲", "readers_of": "/data/班级" })).unwrap();
    let tid = third["session"]["session_id"].as_str().unwrap().to_string();
    assert_eq!(third["session"]["policy"]["readers_of"], "d-class");
    let third_row = || env.svc.sessions.require(&tid).unwrap();
    assert!(env.svc.session_actor(&third_row(), &carol, None).unwrap().is_some());
    {
        let mut ws = handle.lock().unwrap();
        let life = ws.read(&alice(), "d-class", None).unwrap()["life_rev"].as_u64().unwrap();
        ok(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "d-class", "expect": { "rev": life } }]));
    }
    assert!(env.svc.session_actor(&third_row(), &carol, None).unwrap().is_none());
    assert_eq!(env.svc.readers_of_json(&third_row())["alive"], false);
    assert!(opens(&bob()), "who joined comes back");
}

/// Bytes the service takes for a PNG picture (it sniffs the magic), distinct per `tag`.
fn png(tag: &str) -> Vec<u8> {
    [&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..], tag.as_bytes()].concat()
}

/// An asset `id` of `bytes` in folder `folder`, shown by Block `cell` on canvas `surface` at `order` (and its poster).
fn asset_block(ws: &mut aiworkspace_store::Workspace, bytes: &[u8], poster: Option<&[u8]>, id: &str, folder: &str, surface: &str, order: &str) {
    let object = ws.stage_asset(&alice(), bytes).unwrap()["object_id"].as_str().unwrap().to_string();
    let mut payload = json!({ "object_id": object, "file_name": format!("{id}.bin") });
    if let Some(p) = poster {
        payload["poster"] = ws.stage_asset(&alice(), p).unwrap()["object_id"].clone();
    }
    ok(ws, &alice(), json!([
        { "op": "entity.create", "entity_id": id, "type_id": "buckyos.asset-ref", "parent_id": folder, "order_key": order, "payload": payload },
        { "op": "entity.create", "entity_id": format!("c-{id}"), "type_id": "buckyos.cell", "parent_id": surface, "order_key": order,
          "placement": { "x": 0, "y": 0, "w": 100, "h": 100 }, "payload": { "view": { "type": "asset", "version": 1 }, "source_ref": { "entity_id": id } } },
    ]));
}

fn set_cover(ws: &mut aiworkspace_store::Workspace, who: &Caller, cover: Value) -> Value {
    commit(ws, who, json!([{ "op": "entity.set_props", "entity_id": "root", "key": "config", "props": { "cover": cover } }]))
}

fn listed_cover(env: &Env, who: &Caller, id: &str) -> Option<Value> {
    let list = env.svc.list_workspaces(who).unwrap();
    list["workspaces"].as_array().unwrap().iter().find(|w| w["workspace_id"] == json!(id)).map(|w| w["cover"].clone())
}

/// Q37 (UI 框架 PRD §9.2): a card has the cover its managers chose in the Workspace's settings — nothing is taken from
/// the canvases —, every one listed reads it and its bytes, a clone carries it, removing it is an undoable setting.
#[test]
fn the_workspace_cover_is_the_picture_its_managers_chose_in_the_settings() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "封面", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let carol = Caller::user("carol");
    let picture = {
        let mut ws = handle.lock().unwrap();
        ok(&mut ws, &alice(), json!([
            { "op": "entity.create", "entity_id": "one-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "one", "title": "一" } },
            { "op": "entity.create", "entity_id": "one", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "一", "content_folder_id": "one-content" } },
        ]));
        // a picture on a canvas is no cover
        asset_block(&mut ws, &png("on-canvas"), None, "pic", "one-content", "one", "a");
        ws.stage_asset(&alice(), &png("cover")).unwrap()["object_id"].as_str().unwrap().to_string()
    };
    assert_eq!(listed_cover(&env, &alice(), &id), Some(Value::Null), "nothing chosen: the placeholder");
    {
        let mut ws = handle.lock().unwrap();
        // what is no picture, too large, or not uploaded is refused; a non-manager cannot choose one
        let text = ws.stage_asset(&alice(), b"just some text").unwrap()["object_id"].as_str().unwrap().to_string();
        assert_eq!(code(&set_cover(&mut ws, &alice(), json!({ "object_id": text }))), "INVALID_SCHEMA");
        let big = ws.stage_asset(&alice(), &[&png("big")[..], &vec![0u8; 8 * 1024 * 1024]].concat()).unwrap()["object_id"].as_str().unwrap().to_string();
        assert_eq!(code(&set_cover(&mut ws, &alice(), json!({ "object_id": big }))), "LIMIT_EXCEEDED");
        let unknown = "cyfile:0000000000000000000000000000000000000000000000000000000000000000";
        assert_eq!(code(&set_cover(&mut ws, &alice(), json!({ "object_id": unknown }))), "DEPENDENCY_UNAVAILABLE");
        assert_eq!(code(&set_cover(&mut ws, &alice(), json!({ "object_id": picture, "title": "x" }))), "INVALID_SCHEMA");
        ws.grant(&alice(), "bob", None, &caps(&["read", "update", "structure"])).unwrap();
        assert_eq!(code(&set_cover(&mut ws, &bob(), json!({ "object_id": picture }))), "PERMISSION_DENIED");
        // the owner chooses it; the media type and size come from the verified content
        let r = set_cover(&mut ws, &alice(), json!({ "object_id": picture, "media_type": "image/gif", "size": 1 }));
        assert_eq!(r["status"], "accepted", "{r}");
        assert_eq!(ws.settings().unwrap().0["cover"], json!({ "object_id": picture, "media_type": "image/png", "size": png("cover").len() }));
        // carol reads one canvas only: listed, she reads the settings and so the cover
        ws.grant(&alice(), "carol", Some("one"), &caps(&["read"])).unwrap();
        assert!(ws.can_read_asset(&carol, &picture).unwrap());
        assert!(ws.can_read_asset(&Caller::user("dave"), &picture).is_err(), "who cannot enter reads nothing");
    }
    let mine = json!({ "workspace_id": id, "object_id": picture, "media_type": "image/png" });
    assert_eq!(listed_cover(&env, &alice(), &id), Some(mine.clone()));
    assert_eq!(listed_cover(&env, &bob(), &id), Some(mine.clone()));
    assert_eq!(listed_cover(&env, &carol, &id), Some(mine.clone()));
    // a clone carries it, with its bytes
    let cloned = env.svc.clone_workspace(&alice(), &id).unwrap()["workspace_id"].as_str().unwrap().to_string();
    assert_eq!(listed_cover(&env, &alice(), &cloned), Some(json!({ "workspace_id": cloned, "object_id": picture, "media_type": "image/png" })));
    assert!(env.svc.workspace(&cloned).unwrap().lock().unwrap().can_read_asset(&alice(), &picture).unwrap());
    // removed: no cover, its bytes go with the reference; the removal is undone like any setting
    {
        let mut ws = handle.lock().unwrap();
        let r = set_cover(&mut ws, &alice(), Value::Null);
        assert_eq!(r["status"], "accepted", "{r}");
        assert!(ws.settings().unwrap().0.get("cover").is_none());
        assert!(!ws.can_read_asset(&carol, &picture).unwrap());
        let req = json!({ "epoch": ws.epoch, "commit_id": r["commit_id"], "idempotency_key": "undo-cover", "session_id": "s1" });
        assert_eq!(ws.undo(&alice(), &req)["status"], "accepted");
    }
    assert_eq!(listed_cover(&env, &carol, &id), Some(mine));
}

/// Q37 for a public lecture (UI 框架 PRD §9.2): its courseware's cover, for whoever reads the courseware's settings;
/// null for one who does not (a participant who no longer may enter the courseware).
#[test]
fn a_sessions_cover_is_its_coursewares_for_whoever_reads_the_coursewares_settings() {
    let env = env();
    let src = courseware(&env);
    let created = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第一讲" })).unwrap();
    let sid = created["session"]["session_id"].as_str().unwrap().to_string();
    let row = env.svc.sessions.require(&sid).unwrap();
    let cover = |who: &Caller| -> Value {
        env.svc.session_list_open(who, 100).unwrap()["sessions"].as_array().unwrap().iter().find(|s| s["session_id"] == json!(sid)).map(|s| s["cover"].clone()).unwrap_or(json!("not listed"))
    };
    let handle = env.svc.workspace(&src).unwrap();
    {
        let mut ws = handle.lock().unwrap();
        // a picture on the presented canvas is no cover
        asset_block(&mut ws, &png("stage"), None, "stage-pic", "s-launch-content", "s-launch", "z");
    }
    assert_eq!(cover(&alice()), Value::Null);
    let picture = {
        let mut ws = handle.lock().unwrap();
        let picture = ws.stage_asset(&alice(), &png("course")).unwrap()["object_id"].as_str().unwrap().to_string();
        assert_eq!(set_cover(&mut ws, &alice(), json!({ "object_id": picture }))["status"], "accepted");
        ws.grant(&alice(), "bob", Some("s-launch"), &caps(&["read"])).unwrap();
        picture
    };
    let course = json!({ "workspace_id": src, "object_id": picture, "media_type": "image/png" });
    assert_eq!(cover(&alice()), course, "the presenter: the courseware's, not the run Workspace's");
    assert_eq!(cover(&bob()), course, "a reader of the permission object reads the courseware's settings");
    // bob joins, then loses the courseware: still listed (he joined), without a cover
    env.svc.session_join(&row, &bob(), None, Some("Bob")).unwrap();
    handle.lock().unwrap().revoke(&alice(), "bob", Some("s-launch")).unwrap();
    assert_eq!(cover(&bob()), Value::Null);
}

/// Q37: the cover is a setting, so it travels as the settings do — a self-contained package and a template bring the
/// picture along (the new Workspace's card has it); a package without its assets names it, and where its bytes are
/// absent the card has none.
#[test]
fn the_cover_travels_with_packages_and_templates() {
    let env = env();
    let src = courseware(&env);
    let handle = env.svc.workspace(&src).unwrap();
    let picture = {
        let mut ws = handle.lock().unwrap();
        let picture = ws.stage_asset(&alice(), &png("travels")).unwrap()["object_id"].as_str().unwrap().to_string();
        assert_eq!(set_cover(&mut ws, &alice(), json!({ "object_id": picture }))["status"], "accepted");
        picture
    };
    let carried = |env: &Env, id: &str| -> Value {
        let cover = listed_cover(env, &alice(), id).unwrap();
        if !cover.is_null() {
            assert!(env.svc.workspace(id).unwrap().lock().unwrap().can_read_asset(&alice(), &picture).unwrap());
        }
        cover
    };
    let (whole, bare) = {
        let mut ws = handle.lock().unwrap();
        let whole = ws.export(&alice(), "share", true, None).unwrap();
        let bare = ws.export(&alice(), "share", false, None).unwrap();
        assert!(bare["manifest"]["missing"].as_array().unwrap().iter().any(|m| m["id"] == json!(picture)), "{bare}");
        (fetched(&ws.objects, &whole), fetched(&ws.objects, &bare))
    };
    let other = common::env();
    let id = other.svc.import(&alice(), &whole, "new", false).unwrap()["workspace_id"].as_str().unwrap().to_string();
    assert_eq!(carried(&other, &id), json!({ "workspace_id": id, "object_id": picture, "media_type": "image/png" }));
    let third = common::env();
    let id = third.svc.import(&alice(), &bare, "new", false).unwrap()["workspace_id"].as_str().unwrap().to_string();
    assert_eq!(carried(&third, &id), Value::Null, "the bytes stayed behind: no cover");
    assert_eq!(third.svc.workspace(&id).unwrap().lock().unwrap().settings().unwrap().0["cover"]["object_id"], json!(picture), "the setting is kept");
    // a template of one canvas: its root is the Workspace's, settings and all
    let (item, files) = {
        let ws = handle.lock().unwrap();
        let (item, _, files, _) = ws.ext_export_template(&alice(), &["s-launch".to_string()], &[], &json!({ "id": "launch", "title": "发布会" })).unwrap();
        (item, files.into_iter().filter_map(|(k, v)| k.strip_prefix("templates/launch/").map(|r| (r.to_string(), v))).collect())
    };
    let tf = aiworkspace_store::template::parse_template(&item, files).unwrap();
    let fourth = common::env();
    let id = fourth.svc.create_from_template(&alice(), None, &tf, &Default::default()).unwrap()["workspace_id"].as_str().unwrap().to_string();
    assert_eq!(carried(&fourth, &id), json!({ "workspace_id": id, "object_id": picture, "media_type": "image/png" }));
}
