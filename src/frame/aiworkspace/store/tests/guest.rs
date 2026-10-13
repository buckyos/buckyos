//! Public links: the `public` grant subject applies to everyone, a guest (a principal derived from a
//! browser-made token) reaches nothing else, keeps its own annotations, and owns no workspace.

mod common;
use aiworkspace_store::service::require_signed_in;
use aiworkspace_store::workspace::{Caller, PUBLIC_SUBJECT};
use common::*;
use serde_json::json;

const GUEST_TOKEN: &str = "gt_abcdefghijklmnopqrstuvwxyz";

#[test]
fn guest_principals_come_from_the_token_only() {
    let g = Caller::guest(GUEST_TOKEN).unwrap();
    assert!(g.is_guest());
    assert!(g.principal.starts_with("guest:") && g.principal.len() == "guest:".len() + 26, "{}", g.principal);
    assert_eq!(Caller::guest(GUEST_TOKEN).unwrap().principal, g.principal, "same browser, same guest");
    assert_ne!(Caller::guest("gt_abcdefghijklmnopqrstuvwxy2").unwrap().principal, g.principal);
    assert!(!g.principal.contains("abcdefghij"), "the token itself is never part of the principal");
    for bad in ["gt_short", "pt_abcdefghijklmnopqrstuvwxyz", "tok-alice", ""] {
        assert!(Caller::guest(bad).is_none(), "{bad}");
    }
    assert!(!alice().is_guest());
    assert!(require_signed_in(&g).is_err());
    assert!(require_signed_in(&alice()).is_ok());
}

#[test]
fn a_public_grant_opens_the_workspace_to_guests_and_star_does_not() {
    let env = env();
    let id = project(&env);
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    // nothing yet: the workspace does not exist for the guest, `*` is for signed-in principals only
    assert_eq!(code(&ws.info(&guest).unwrap_err().to_json()), "NOT_FOUND");
    ws.grant(&alice(), "*", None, &["read".into()]).unwrap();
    assert_eq!(code(&ws.info(&guest).unwrap_err().to_json()), "NOT_FOUND");
    assert_eq!(ws.info(&bob()).unwrap()["capabilities"], json!(["read"]));

    // the public link: read + comment for everyone, guests and zone users alike
    ws.grant(&alice(), PUBLIC_SUBJECT, None, &["read".into(), "comment".into()]).unwrap();
    let info = ws.info(&guest).unwrap();
    assert_eq!(info["capabilities"], json!(["read", "comment"]));
    assert_eq!(info["principal"], json!(guest.principal));
    assert_eq!(ws.info(&bob()).unwrap()["capabilities"], json!(["read", "comment"]), "public ∪ *");
    assert!(ws.outline(&guest).unwrap()["entities"].as_array().unwrap().len() > 5);
    // what the guest sees of the grants: the rows that apply to it, no names
    let grants = ws.list_grants(&guest).unwrap();
    assert_eq!(grants["complete"], json!(false));
    let subjects: Vec<&str> = grants["grants"].as_array().unwrap().iter().map(|g| g["subject"].as_str().unwrap()).collect();
    assert_eq!(subjects, vec![PUBLIC_SUBJECT]);

    // a guest annotates (and the annotation names the guest, plus the display name it chose)
    ok(&mut ws, &guest, json!([{ "op": "entity.create", "entity_id": "guest-note", "type_id": "buckyos.annotation", "parent_id": "data",
                                 "order_key": "zz", "payload": { "target": { "entity_id": "tasks" }, "kind": "note", "body": "看起来不错", "author_name": "路人甲" } }]));
    let note = ws.read(&guest, "guest-note", None).unwrap();
    assert_eq!(note["content"]["payload"]["author"], json!(guest.principal));
    assert_eq!(note["content"]["payload"]["author_name"], json!("路人甲"));
    // its own note can go, alice's cannot; no other write is possible
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "owner-note", "type_id": "buckyos.annotation", "parent_id": "data",
                                   "order_key": "zy", "payload": { "target": { "entity_id": "tasks" }, "kind": "note", "body": "owner" } }]));
    let owner_rev = ws.read(&guest, "owner-note", None).unwrap()["life_rev"].clone();
    let r = commit(&mut ws, &guest, json!([{ "op": "entity.delete", "entity_id": "owner-note", "expect": { "rev": owner_rev } }]));
    assert_eq!(r["status"], "rejected", "{r}");
    assert_eq!(code(&r), "PERMISSION_DENIED");
    let r = commit(&mut ws, &guest, json!([{ "op": "entity.set_keys", "entity_id": "tasks", "keys": [{ "key": "title", "value": "x" }] }]));
    assert_eq!(r["status"], "rejected", "{r}");
    ok(&mut ws, &guest, json!([{ "op": "entity.delete", "entity_id": "guest-note", "expect": { "rev": note["life_rev"] } }]));
    assert_eq!(code(&ws.grant(&guest, "bob", None, &["read".into()]).unwrap_err().to_json()), "PERMISSION_DENIED");
    assert_eq!(code(&ws.grant(&alice(), PUBLIC_SUBJECT, None, &["read".into(), "manage".into()]).unwrap_err().to_json()), "INVALID_OPERATION");

    // the link is withdrawn: the guest is out at once, bob keeps `*`
    ws.revoke(&alice(), PUBLIC_SUBJECT, None).unwrap();
    assert_eq!(code(&ws.info(&guest).unwrap_err().to_json()), "NOT_FOUND");
    assert_eq!(ws.info(&bob()).unwrap()["capabilities"], json!(["read"]));
    drop(ws);
    // a guest owns and enumerates nothing
    assert_eq!(env.svc.list_workspaces(&guest).unwrap()["workspaces"], json!([]));
    assert_eq!(code(&env.svc.create_workspace(&guest, "x", None).unwrap_err().to_json()), "PERMISSION_DENIED");
    assert_eq!(code(&env.svc.fork(&guest, &id).unwrap_err().to_json()), "PERMISSION_DENIED");
}

fn caps_of(ws: &aiworkspace_store::workspace::Workspace, who: &Caller, id: &str) -> Vec<String> {
    serde_json::from_value(ws.read(who, id, None).unwrap()["capabilities"].clone()).unwrap()
}

/// A valid order key per x (letters only).
fn key(x: i64) -> String {
    x.to_string().chars().map(|c| (b'a' + c.to_digit(10).unwrap() as u8 + 1) as char).collect()
}

fn note_ops(note: &str, cell: &str, x: i64, body: &str) -> serde_json::Value {
    json!([
        { "op": "entity.create", "entity_id": note, "type_id": "buckyos.annotation", "parent_id": "board-content", "order_key": format!("n{}", key(x)),
          "payload": { "kind": "note", "body": body, "style": { "color": "#fff2cc" }, "author_name": "路人甲" } },
        { "op": "entity.create", "entity_id": cell, "type_id": "buckyos.cell", "parent_id": "board", "order_key": format!("c{}", key(x)),
          "placement": { "x": x, "y": 0, "w": 220, "h": 160 }, "payload": { "view": { "type": "note" }, "source_ref": { "entity_id": note } } },
    ])
}

/// 公开链接便签: with `comment` alone a guest puts notes on a canvas and moves, resizes, deletes and restores
/// its own note Blocks; other Blocks, other people's notes and other kinds of Block stay out of reach.
#[test]
fn a_comment_only_guest_adds_and_handles_its_own_notes() {
    let env = env();
    let id = project(&env);
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "board-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "b",
          "name": "白板", "payload": { "kind": "folder", "title": "白板", "system": "surface_content", "surface_id": "board" } },
        { "op": "entity.create", "entity_id": "board", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "b",
          "name": "白板", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "白板", "content_folder_id": "board-content" } },
        { "op": "entity.create", "entity_id": "frame", "type_id": "buckyos.cell", "parent_id": "board", "order_key": "a",
          "placement": { "x": 0, "y": 400, "w": 600, "h": 400 }, "payload": { "view": { "type": "frame" }, "title": "框" } },
        { "op": "entity.create", "entity_id": "grp", "type_id": "buckyos.container", "parent_id": "board", "order_key": "g",
          "placement": { "x": 800, "y": 0, "w": 400, "h": 400 }, "payload": { "kind": "group", "layout": { "mode": "free" } } },
    ]));
    ok(&mut ws, &alice(), note_ops("alice-note", "alice-cell", 2000, "owner"));
    ws.grant(&alice(), PUBLIC_SUBJECT, None, &["read".into(), "comment".into()]).unwrap();

    // the note and its Block go in one commit
    ok(&mut ws, &guest, note_ops("g-note", "g-cell", 100, "访客便签"));
    let cell = ws.read(&guest, "g-cell", None).unwrap();
    assert_eq!(cell["parent_id"], "board");
    assert_eq!(ws.read(&guest, "g-note", None).unwrap()["content"]["payload"]["author"], json!(guest.principal));
    // envelopes say what the guest may do with its own writing, and only there
    let caps = caps_of(&ws, &guest, "g-cell");
    for c in ["read", "comment", "update", "delete", "structure"] {
        assert!(caps.iter().any(|x| x == c), "{c} in {caps:?}");
    }
    assert_eq!(caps_of(&ws, &guest, "g-note"), vec!["read", "update", "delete", "comment"]);
    assert_eq!(caps_of(&ws, &guest, "alice-cell"), vec!["read", "comment"]);
    assert_eq!(caps_of(&ws, &guest, "alice-note"), vec!["read", "comment"]);
    assert_eq!(caps_of(&ws, &guest, "frame"), vec!["read", "comment"]);

    // move, resize, restack, into a group and out again, lock: all its own Block
    ok(&mut ws, &guest, json!([{ "op": "tree.place", "entity_id": "g-cell", "placement": { "x": 140, "y": 30, "w": 330, "h": 240 } }]));
    ok(&mut ws, &guest, json!([{ "op": "tree.place", "entity_id": "g-cell", "order_key": "zz" }]));
    ok(&mut ws, &guest, json!([{ "op": "tree.move", "entity_id": "g-cell", "new_parent_id": "grp", "order_key": "a", "placement": { "x": 10, "y": 10, "w": 220, "h": 160 } }]));
    assert_eq!(ws.read(&guest, "g-cell", None).unwrap()["parent_id"], "grp");
    ok(&mut ws, &guest, json!([{ "op": "tree.move", "entity_id": "g-cell", "new_parent_id": "board", "order_key": "zz", "placement": { "x": 140, "y": 30, "w": 220, "h": 160 } }]));
    ok(&mut ws, &guest, json!([{ "op": "entity.set_keys", "entity_id": "g-cell", "keys": [{ "key": "locked", "value": true, "expect": { "rev": 0 } }] }]));
    // and its note's text
    let body_rev = ws.read(&guest, "g-note", None).unwrap()["content"]["key_revs"]["body"].clone();
    ok(&mut ws, &guest, json!([{ "op": "entity.set_keys", "entity_id": "g-note", "keys": [{ "key": "body", "value": "改过", "expect": { "rev": body_rev } }] }]));

    let denied = |ws: &mut aiworkspace_store::workspace::Workspace, ops: serde_json::Value| {
        let r = commit(ws, &guest, ops);
        assert_eq!(r["status"], "rejected", "{r}");
        assert_eq!(code(&r), "PERMISSION_DENIED", "{r}");
    };
    // other Blocks, someone else's note, and Blocks of other kinds stay out of reach
    denied(&mut ws, json!([{ "op": "tree.place", "entity_id": "frame", "placement": { "x": 1, "y": 1, "w": 10, "h": 10 } }]));
    denied(&mut ws, json!([{ "op": "tree.place", "entity_id": "alice-cell", "placement": { "x": 1, "y": 1, "w": 10, "h": 10 } }]));
    let rev = ws.read(&guest, "alice-cell", None).unwrap()["life_rev"].clone();
    denied(&mut ws, json!([{ "op": "entity.delete", "entity_id": "alice-cell", "expect": { "rev": rev } }]));
    denied(&mut ws, json!([{ "op": "entity.create", "entity_id": "g-frame", "type_id": "buckyos.cell", "parent_id": "board", "order_key": "f",
                             "placement": { "x": 0, "y": 0, "w": 100, "h": 100 }, "payload": { "view": { "type": "frame" } } }]));
    denied(&mut ws, json!([{ "op": "entity.create", "entity_id": "g-view", "type_id": "buckyos.cell", "parent_id": "board", "order_key": "f",
                             "placement": { "x": 0, "y": 0, "w": 100, "h": 100 }, "payload": { "view": { "type": "note" }, "source_ref": { "entity_id": "alice-note" } } }]));
    denied(&mut ws, json!([{ "op": "entity.create", "entity_id": "g-grp", "type_id": "buckyos.container", "parent_id": "board", "order_key": "f",
                             "placement": { "x": 0, "y": 0, "w": 100, "h": 100 }, "payload": { "kind": "group", "layout": { "mode": "free" } } }]));
    // its own Block cannot be turned into someone else's note or another kind of Block
    let src_rev = ws.read(&guest, "g-cell", None).unwrap()["content"]["key_revs"]["source_ref"].clone();
    denied(&mut ws, json!([{ "op": "entity.set_keys", "entity_id": "g-cell", "keys": [{ "key": "source_ref", "value": { "entity_id": "alice-note" }, "expect": { "rev": src_rev } }] }]));
    denied(&mut ws, json!([{ "op": "entity.set_keys", "entity_id": "g-cell", "keys": [{ "key": "view", "value": { "type": "frame" }, "expect": { "rev": src_rev } }] }]));

    // delete the Block with its note, then undo it the way the client does (restore both)
    let cell_rev = ws.read(&guest, "g-cell", None).unwrap()["life_rev"].clone();
    let note_rev = ws.read(&guest, "g-note", None).unwrap()["life_rev"].clone();
    let r = ok(&mut ws, &guest, json!([{ "op": "entity.delete", "entity_id": "g-cell", "expect": { "rev": cell_rev } },
                                       { "op": "entity.delete", "entity_id": "g-note", "expect": { "rev": note_rev } }]));
    let seq = r["seq"].clone();
    ok(&mut ws, &guest, json!([{ "op": "entity.restore", "entity_id": "g-note", "expect": { "rev": seq } },
                               { "op": "entity.restore", "entity_id": "g-cell", "expect": { "rev": seq } }]));
    assert_eq!(ws.read(&guest, "g-cell", None).unwrap()["deleted"], json!(false));

    // a lock-required canvas does not hold back notes, which never take a lock (like annotations)
    let board_rev = ws.read(&alice(), "board", None).unwrap()["meta_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_write_policy", "entity_id": "board", "policy": "lock_required", "expect": { "rev": board_rev } }]));
    ok(&mut ws, &guest, note_ops("g-note2", "g-cell2", 300, "锁定画布上的便签"));
    ok(&mut ws, &guest, json!([{ "op": "tree.place", "entity_id": "g-cell2", "placement": { "x": 320, "y": 0, "w": 220, "h": 160 } }]));

    // without `comment` (read only) nothing of this is possible
    ws.revoke(&alice(), PUBLIC_SUBJECT, None).unwrap();
    ws.grant(&alice(), PUBLIC_SUBJECT, None, &["read".into()]).unwrap();
    assert_eq!(caps_of(&ws, &guest, "g-cell"), vec!["read"]);
    denied(&mut ws, json!([{ "op": "tree.place", "entity_id": "g-cell", "placement": { "x": 1, "y": 1, "w": 10, "h": 10 } }]));
    denied(&mut ws, note_ops("g-note3", "g-cell3", 500, "x"));
}

/// The same rule holds for a signed-in principal granted `comment` only, and editors keep handling every note.
#[test]
fn comment_only_users_handle_their_notes_and_editors_handle_all() {
    let env = env();
    let id = project(&env);
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "board-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "b",
          "name": "白板", "payload": { "kind": "folder", "title": "白板", "system": "surface_content", "surface_id": "board" } },
        { "op": "entity.create", "entity_id": "board", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "b",
          "name": "白板", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "白板", "content_folder_id": "board-content" } },
    ]));
    ws.grant(&alice(), "bob", None, &["read".into(), "comment".into()]).unwrap();
    ok(&mut ws, &bob(), note_ops("b-note", "b-cell", 100, "bob"));
    ok(&mut ws, &bob(), json!([{ "op": "tree.place", "entity_id": "b-cell", "placement": { "x": 10, "y": 10, "w": 220, "h": 160 } }]));
    // the owner moves and removes bob's note Block as before
    ok(&mut ws, &alice(), json!([{ "op": "tree.place", "entity_id": "b-cell", "placement": { "x": 50, "y": 50, "w": 220, "h": 160 } }]));
    let rev = ws.read(&alice(), "b-cell", None).unwrap()["life_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "b-cell", "expect": { "rev": rev } }]));
}
