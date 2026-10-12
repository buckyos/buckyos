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
