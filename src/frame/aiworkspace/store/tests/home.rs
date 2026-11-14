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

/// K6, the stopgap cover (UI 框架 PRD §9.2): the first Block showing a picture (or a file's poster) on the canvas changed
/// last, as the caller reads it — the Block and the asset both — else null.
#[test]
fn the_workspace_cover_is_the_first_picture_of_the_canvas_changed_last_as_the_caller_reads_it() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "封面", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let cover = |who: &Caller| -> Value {
        let list = env.svc.list_workspaces(who).unwrap();
        list["workspaces"].as_array().unwrap().iter().find(|w| w["workspace_id"] == json!(id)).map(|w| w["cover"].clone()).unwrap_or(Value::Null)
    };
    let handle = env.svc.workspace(&id).unwrap();
    {
        let mut ws = handle.lock().unwrap();
        ok(&mut ws, &alice(), json!([
            { "op": "entity.create", "entity_id": "one-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "one", "title": "一" } },
            { "op": "entity.create", "entity_id": "one", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "一", "content_folder_id": "one-content" } },
            { "op": "entity.create", "entity_id": "two-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "b", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "two", "title": "二" } },
            { "op": "entity.create", "entity_id": "two", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "b", "payload": { "kind": "surface", "layout": { "mode": "flow" }, "title": "二", "content_folder_id": "two-content" } },
        ]));
    }
    assert_eq!(cover(&alice()), Value::Null, "no picture anywhere: the placeholder");
    let picture = {
        let mut ws = handle.lock().unwrap();
        // on the first canvas a file that is no picture and has no poster, then a picture: the picture
        asset_block(&mut ws, b"just some text", None, "doc", "one-content", "one", "a");
        asset_block(&mut ws, &png("one"), None, "pic", "one-content", "one", "b");
        ws.read(&alice(), "pic", None).unwrap()["content"]["payload"]["object_id"].as_str().unwrap().to_string()
    };
    let got = cover(&alice());
    assert_eq!(got, json!({ "workspace_id": id, "object_id": picture, "media_type": "image/png", "surface_id": "one", "entity_id": "pic" }));
    // the second canvas changes later: its video's poster
    let poster = {
        let mut ws = handle.lock().unwrap();
        asset_block(&mut ws, b"\0\0\0\x18ftypmp42\0\0\0\0mp42isomframes", Some(&png("poster")), "film", "two-content", "two", "a");
        ws.read(&alice(), "film", None).unwrap()["content"]["payload"]["poster"].as_str().unwrap().to_string()
    };
    let got = cover(&alice());
    assert_eq!((&got["object_id"], &got["media_type"], &got["surface_id"], &got["entity_id"]), (&json!(poster), &json!("image/png"), &json!("two"), &json!("film")));
    // the first canvas changes again (a Block moves): its picture is back
    {
        let mut ws = handle.lock().unwrap();
        ok(&mut ws, &alice(), json!([{ "op": "tree.move", "entity_id": "c-doc", "new_parent_id": "one", "order_key": "c" }]));
    }
    assert_eq!(cover(&alice())["surface_id"], "one");
    // bob reads the second canvas and its data: his cover comes from there, whatever changed last
    {
        let mut ws = handle.lock().unwrap();
        ws.grant(&alice(), "bob", Some("two"), &caps(&["read"])).unwrap();
        ws.grant(&alice(), "bob", Some("two-content"), &caps(&["read"])).unwrap();
    }
    assert_eq!(cover(&bob())["entity_id"], "film");
    // carol reads the first canvas but not the data on it: no picture she may see
    let carol = Caller::user("carol");
    handle.lock().unwrap().grant(&alice(), "carol", Some("one"), &caps(&["read"])).unwrap();
    assert!(env.svc.list_workspaces(&carol).unwrap()["workspaces"].as_array().unwrap().iter().any(|w| w["workspace_id"] == json!(id)));
    assert_eq!(cover(&carol), Value::Null);
    // the picture's Block deleted: the next one in the canvas's order (none here but the poster elsewhere)
    {
        let mut ws = handle.lock().unwrap();
        let life = ws.read(&alice(), "c-pic", None).unwrap()["life_rev"].as_u64().unwrap();
        ok(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "c-pic", "expect": { "rev": life } }]));
    }
    assert_eq!(cover(&alice())["entity_id"], "film");
}

/// K6 for a public lecture (UI 框架 PRD §9.2): the first picture on the canvases of its path — in the run Workspace for
/// whoever reads it, else in the courseware as the caller reads it there; null otherwise.
#[test]
fn a_sessions_cover_comes_from_the_canvases_of_its_path_as_the_caller_reads_them() {
    let env = env();
    let src = courseware(&env);
    {
        let handle = env.svc.workspace(&src).unwrap();
        let mut ws = handle.lock().unwrap();
        asset_block(&mut ws, &png("stage"), None, "stage-pic", "s-launch-content", "s-launch", "z");
    }
    let created = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "title": "第一讲" })).unwrap();
    let sid = created["session"]["session_id"].as_str().unwrap().to_string();
    let row = env.svc.sessions.require(&sid).unwrap();
    let cover = |who: &Caller| -> Value {
        env.svc.session_list_open(who, 100).unwrap()["sessions"].as_array().unwrap().iter().find(|s| s["session_id"] == json!(sid)).map(|s| s["cover"].clone()).unwrap_or(Value::Null)
    };
    // the presenter reads the run Workspace: the picture there
    let mine = cover(&alice());
    assert_eq!((&mine["workspace_id"], &mine["surface_id"], &mine["entity_id"], &mine["media_type"]), (&json!(row.workspace_id), &json!("s-launch"), &json!("stage-pic"), &json!("image/png")));
    // bob reads the presented canvas of the courseware but not its data: listed, without a cover
    {
        let handle = env.svc.workspace(&src).unwrap();
        handle.lock().unwrap().grant(&alice(), "bob", Some("s-launch"), &caps(&["read"])).unwrap();
    }
    assert_eq!(cover(&bob()), Value::Null);
    // with the canvas's data too: the courseware's picture (he cannot read the run Workspace before joining)
    {
        let handle = env.svc.workspace(&src).unwrap();
        handle.lock().unwrap().grant(&alice(), "bob", Some("s-launch-content"), &caps(&["read"])).unwrap();
    }
    let his = cover(&bob());
    assert_eq!((&his["workspace_id"], &his["entity_id"]), (&json!(src), &json!("stage-pic")));
    // once joined he reads the run Workspace: its picture
    env.svc.session_join(&row, &bob(), None, Some("Bob")).unwrap();
    assert_eq!(cover(&bob())["workspace_id"], json!(row.workspace_id));
}
