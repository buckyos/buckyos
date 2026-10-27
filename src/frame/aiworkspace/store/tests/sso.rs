//! Hybrid SSO (PRE-03, SSO-08, SSO-13): grant matching by caller class (`*` is for unrestricted accounts only),
//! `group:` / `contact:` subjects, who may own Workspaces, and share entries (guests, joins, closing, expiry).

mod common;
use aiworkspace_store::entries::{parse_entry_methods, split_entry_id, ENTRY_DID_LOGIN, ENTRY_GUEST, ENTRY_PASSWORD, ENTRY_SELF_REGISTER};
use aiworkspace_store::workspace::{Caller, CallerClass, Workspace, ANY_AUTHENTICATED_SUBJECT, PUBLIC_SUBJECT};
use aiworkspace_core::{Code, Deny, WsError};
use common::*;
use serde_json::{json, Value};

const GUEST_TOKEN: &str = "gt_YWl3cy1ndWVzdC10b2tlbi1maXh0dXJlLTMyYnl0ZXM";

fn limited(id: &str) -> Caller {
    Caller::account(id, true)
}

fn in_groups(mut c: Caller, groups: &[&str]) -> Caller {
    if let CallerClass::Account { groups: g, .. } = &mut c.class {
        *g = groups.iter().map(|s| s.to_string()).collect();
    }
    c
}

fn with_did(mut c: Caller, did: &str) -> Caller {
    if let CallerClass::Account { did: d, .. } = &mut c.class {
        *d = Some(did.to_string());
    }
    c
}

fn reason(e: &WsError) -> Option<Deny> {
    e.denial
}

fn caps(ws: &Workspace, who: &Caller) -> Value {
    ws.info(who).unwrap()["capabilities"].clone()
}

fn not_found(ws: &Workspace, who: &Caller) {
    assert_eq!(ws.info(who).unwrap_err().code, Code::NotFound, "{} must not reach the workspace", who.principal);
}

fn new_ws(env: &Env, title: &str) -> String {
    env.svc.create_workspace(&alice(), title, None).unwrap()["workspace_id"].as_str().unwrap().to_string()
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn star_is_for_unrestricted_accounts_only() {
    let env = env();
    let id = new_ws(&env, "星号");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), ANY_AUTHENTICATED_SUBJECT, None, &strings(&["read", "comment"])).unwrap();
    assert_eq!(caps(&ws, &bob()), json!(["read", "comment"]));
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    for who in [limited("acme-c1"), Caller::external("did:bns:carol"), guest.clone(), Caller::service("some-app")] {
        not_found(&ws, &who);
    }
    // a grant to the principal itself works for every class: a Limited account by user id, an External by DID
    ws.grant(&alice(), "acme-c1", None, &strings(&["read"])).unwrap();
    ws.grant(&alice(), "did:bns:carol", None, &strings(&["read", "comment"])).unwrap();
    assert_eq!(caps(&ws, &limited("acme-c1")), json!(["read"]));
    assert_eq!(caps(&ws, &Caller::external("did:bns:carol")), json!(["read", "comment"]));
    // an account whose DID is that DID is not the External: a DID subject names the External only
    let mut account = Caller::user("carol");
    account.class = CallerClass::Account { restricted: false, did: Some("did:bns:carol".into()), groups: vec![] };
    assert_eq!(caps(&ws, &account), json!(["read", "comment"]), "carol (an account) reaches it through `*` only");
    // `public` applies to everyone
    ws.grant(&alice(), PUBLIC_SUBJECT, None, &strings(&["read"])).unwrap();
    assert_eq!(caps(&ws, &guest), json!(["read"]));
    assert_eq!(caps(&ws, &Caller::service("some-app")), json!(["read"]));
    // what a restricted caller sees of the grants: its own row and `public`, never `*`
    let grants = ws.list_grants(&limited("acme-c1")).unwrap();
    let subjects: Vec<&str> = grants["grants"].as_array().unwrap().iter().map(|g| g["subject"].as_str().unwrap()).collect();
    assert_eq!(subjects, vec!["acme-c1", PUBLIC_SUBJECT]);
}

#[test]
fn group_and_contact_subjects() {
    let env = env();
    let id = new_ws(&env, "组与联系人");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "group:customers", None, &strings(&["read"])).unwrap();
    // a contact group is named only by its owner: the grant's owner DID is the granter's own
    let owner_alice = with_did(alice(), "did:bns:alice");
    ws.grant(&owner_alice, "contact:did:bns:alice#vip", None, &strings(&["read", "comment"])).unwrap();
    ws.grant(&alice(), "some-app", None, &strings(&["read", "manage"])).unwrap();
    for (who, subject) in [(alice(), "contact:did:bns:alice#vip"), (owner_alice.clone(), "contact:did:bns:bob#friends"), (Caller::service("some-app"), "contact:did:bns:alice#vip")] {
        assert_eq!(ws.grant(&who, subject, None, &strings(&["read"])).unwrap_err().code, Code::InvalidOperation, "{} {subject}", who.principal);
    }
    for bad in ["group:", "contact:alice#vip", "contact:did:bns:alice", "contact:did:bns:alice#", "has space"] {
        assert_eq!(ws.grant(&alice(), bad, None, &strings(&["read"])).unwrap_err().code, Code::InvalidOperation, "{bad}");
    }
    ws.revoke(&alice(), "some-app", None).unwrap();
    assert_eq!(ws.dynamic_subjects().unwrap(), vec!["contact:did:bns:alice#vip", "group:customers"]);

    // system groups: Limited and ordinary accounts alike; never Externals or guests
    assert_eq!(caps(&ws, &in_groups(limited("acme-c1"), &["customers"])), json!(["read"]));
    assert_eq!(caps(&ws, &in_groups(Caller::user("bob"), &["customers", "staff"])), json!(["read"]));
    not_found(&ws, &in_groups(limited("acme-c2"), &["staff"]));
    not_found(&ws, &Caller::external("did:bns:customers"));

    // contact groups: whatever the service confirmed
    let mut ext = Caller::external("did:bns:dora");
    not_found(&ws, &ext);
    ext.contacts.insert("contact:did:bns:alice#vip".into());
    assert_eq!(caps(&ws, &ext), json!(["read", "comment"]));
    // a membership that could not be checked: refused as "authorization unavailable", not as missing
    let mut unsure = Caller::external("did:bns:erin");
    unsure.unconfirmed.insert("contact:did:bns:alice#vip".into());
    let e = ws.info(&unsure).unwrap_err();
    assert_eq!((e.code, reason(&e)), (Code::PermissionDenied, Some(Deny::AuthorizationUnavailable)));
    // ... also when another grant already lets it in, since the unknown one could add more
    let mut unsure_member = in_groups(limited("acme-c1"), &["customers"]);
    unsure_member.unconfirmed.insert("contact:did:bns:alice#vip".into());
    assert_eq!(reason(&ws.info(&unsure_member).unwrap_err()), Some(Deny::AuthorizationUnavailable));
    // ... but not for someone who holds everything anyway (the owner)
    let mut owner = alice();
    owner.unconfirmed.insert("contact:did:bns:alice#vip".into());
    assert!(ws.info(&owner).is_ok());

    // the member sees the rows that let it in, by their subject
    let grants = ws.list_grants(&in_groups(limited("acme-c1"), &["customers"])).unwrap();
    assert_eq!(grants["grants"].as_array().unwrap().iter().map(|g| g["subject"].clone()).collect::<Vec<_>>(), vec![json!("group:customers")]);
}

#[test]
fn only_unrestricted_accounts_and_services_own_workspaces() {
    let env = env();
    let id = new_ws(&env, "源");
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        for who in ["acme-c1", "did:bns:carol"] {
            ws.grant(&alice(), who, None, &strings(&["read", "export"])).unwrap();
        }
        ws.grant(&alice(), PUBLIC_SUBJECT, None, &strings(&["read", "export"])).unwrap();
    }
    let refused = |e: WsError| assert_eq!((e.code, e.denial), (Code::PermissionDenied, Some(Deny::SessionKindNotAllowed)), "{e}");
    for who in [limited("acme-c1"), Caller::external("did:bns:carol"), Caller::guest(GUEST_TOKEN).unwrap()] {
        refused(env.svc.create_workspace(&who, "x", None).unwrap_err());
        refused(env.svc.fork(&who, &id).unwrap_err());
        refused(aiworkspace_store::service::require_may_own(&who).unwrap_err());
    }
    assert!(env.svc.create_workspace(&Caller::service("did:bns:jarvis"), "agent's", None).is_ok());
    assert!(env.svc.fork(&bob(), &id).is_ok());
}

#[test]
fn restricted_callers_list_only_what_was_granted_to_them() {
    let env = env();
    let named = new_ws(&env, "点名");
    let grouped = new_ws(&env, "按组");
    let open = new_ws(&env, "公开");
    let everyone = new_ws(&env, "全员");
    let grant = |id: &str, subject: &str| env.svc.workspace(id).unwrap().lock().unwrap().grant(&alice(), subject, None, &strings(&["read"])).unwrap();
    grant(&named, "acme-c1");
    grant(&grouped, "group:customers");
    grant(&open, PUBLIC_SUBJECT);
    grant(&everyone, ANY_AUTHENTICATED_SUBJECT);
    let listed = |who: &Caller| -> Vec<String> {
        let mut v: Vec<String> = env.svc.list_workspaces(who).unwrap()["workspaces"].as_array().unwrap().iter().map(|w| w["title"].as_str().unwrap().to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(listed(&in_groups(limited("acme-c1"), &["customers"])), vec!["按组", "点名"]);
    assert_eq!(listed(&Caller::external("did:bns:carol")), Vec::<String>::new());
    assert_eq!(listed(&Caller::guest(GUEST_TOKEN).unwrap()), Vec::<String>::new());
    assert_eq!(listed(&bob()), vec!["全员", "公开"], "an ordinary account still lists public and `*` workspaces");
    assert_eq!(env.svc.dynamic_subjects().unwrap(), vec!["group:customers"]);
    // the public one stays reachable by its link
    assert_eq!(caps(&env.svc.workspace(&open).unwrap().lock().unwrap(), &limited("acme-c1")), json!(["read"]));
}

fn entry_local(v: &Value) -> String {
    split_entry_id(v["entry"]["entry_id"].as_str().unwrap()).unwrap().1.to_string()
}

#[test]
fn guest_entries_open_while_active() {
    let env = env();
    let id = new_ws(&env, "入口");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    // manage only; no manage in the template; at least one capability and one way in
    ws.grant(&alice(), "bob", None, &strings(&["read", "comment"])).unwrap();
    assert_eq!(ws.create_entry(&bob(), None, &strings(&["read"]), ENTRY_GUEST, None).unwrap_err().code, Code::PermissionDenied);
    assert_eq!(ws.create_entry(&alice(), None, &strings(&["read", "manage"]), ENTRY_GUEST, None).unwrap_err().code, Code::InvalidOperation);
    assert_eq!(ws.create_entry(&alice(), None, &[], ENTRY_GUEST, None).unwrap_err().code, Code::InvalidOperation);
    assert_eq!(ws.create_entry(&alice(), None, &strings(&["read"]), 0, None).unwrap_err().code, Code::InvalidOperation);
    assert_eq!(ws.create_entry(&alice(), None, &strings(&["read"]), ENTRY_GUEST, Some(T0 / 1000 - 1)).unwrap_err().code, Code::InvalidOperation);

    not_found(&ws, &guest);
    let created = ws.create_entry(&alice(), None, &strings(&["read", "comment"]), ENTRY_GUEST, Some(T0 / 1000 + 3600)).unwrap();
    let entry = &created["entry"];
    assert_eq!(entry["state"], "active");
    assert_eq!(entry["methods"], json!({ "guest": true, "password": false, "self_register": false, "did_login": false }));
    assert!(entry["entry_id"].as_str().unwrap().starts_with(&format!("{id}~se_")), "{entry}");
    let local = entry_local(&created);
    // anyone gets the template of an entry open to guests — guests and signed-in callers of every class alike
    for who in [guest.clone(), limited("acme-c1"), Caller::external("did:bns:carol"), Caller::service("some-app")] {
        assert_eq!(caps(&ws, &who), json!(["read", "comment"]), "{}", who.principal);
    }
    assert_eq!(ws.list_entries(&alice()).unwrap()["entries"].as_array().unwrap().len(), 1);
    assert_eq!(ws.list_entries(&bob()).unwrap_err().code, Code::PermissionDenied);

    // closing takes effect at the next request; closing again is fine
    assert_eq!(ws.close_entry(&bob(), &local).unwrap_err().code, Code::PermissionDenied);
    assert_eq!(ws.close_entry(&alice(), &local).unwrap()["entry"]["state"], "closed");
    not_found(&ws, &guest);
    not_found(&ws, &limited("acme-c1"));
    assert!(ws.close_entry(&alice(), &local).is_ok());
    assert_eq!(ws.close_entry(&alice(), "se_aaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap_err().code, Code::NotFound);

    // an entry that expires stops working by itself
    let timed = ws.create_entry(&alice(), None, &strings(&["read"]), ENTRY_GUEST, Some(T0 / 1000 + 60)).unwrap();
    assert_eq!(caps(&ws, &guest), json!(["read"]));
    drop(ws);
    env.advance(61_000);
    let ws = h.lock().unwrap();
    not_found(&ws, &guest);
    assert_eq!(ws.entry(&entry_local(&timed)).unwrap().unwrap().state.as_str(), "expired");
}

#[test]
fn entries_on_a_subtree_need_manage_there() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "dj", Some("tasks"), &strings(&["read", "append", "manage"])).unwrap();
    let dj = Caller::user("dj");
    assert_eq!(ws.create_entry(&dj, None, &strings(&["read"]), ENTRY_GUEST, None).unwrap_err().code, Code::PermissionDenied);
    let created = ws.create_entry(&dj, Some("tasks"), &strings(&["read", "append"]), ENTRY_GUEST | ENTRY_PASSWORD, None).unwrap();
    assert_eq!(created["entry"]["scope_entity_id"], "tasks");
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    assert!(ws.read(&guest, "tasks", None).is_ok());
    assert_eq!(ws.read(&guest, "project-info", None).unwrap_err().code, Code::NotFound, "only the subtree");
    assert_eq!(ws.list_entries(&dj).unwrap()["entries"].as_array().unwrap().len(), 1);
    assert_eq!(ws.list_entries(&alice()).unwrap()["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn joining_an_entry_grants_its_template() {
    let env = env();
    let id = new_ws(&env, "加入");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let created = ws.create_entry(&alice(), None, &strings(&["read", "comment"]), parse_entry_methods(&json!({ "self_register": true })).unwrap(), None).unwrap();
    assert_eq!(created["entry"]["methods"]["password"], json!(true), "self_register implies password");
    let local = entry_local(&created);
    let admit = |_: u8| Ok(());
    let customer = limited("acme-c1");
    not_found(&ws, &customer);

    // the service decides about the methods; a refusal leaves nothing behind
    let refuse = |bits: u8| {
        assert_eq!(bits, ENTRY_PASSWORD | ENTRY_SELF_REGISTER);
        Err(WsError::refused(Code::PermissionDenied, Deny::ZoneMethodDisabled, "off"))
    };
    assert_eq!(reason(&ws.join_entry(&customer, &local, refuse).unwrap_err()), Some(Deny::ZoneMethodDisabled));
    not_found(&ws, &customer);

    let joined = ws.join_entry(&customer, &local, admit).unwrap();
    assert_eq!((joined["capabilities"].clone(), joined["joined"].clone()), (json!(["read", "comment"]), json!(true)));
    assert_eq!(caps(&ws, &customer), json!(["read", "comment"]));
    let again = ws.join_entry(&customer, &local, admit).unwrap();
    assert_eq!(again["joined"], json!(false), "joining again changes nothing");
    let row = ws.list_grants(&alice()).unwrap()["grants"].as_array().unwrap().iter().find(|g| g["subject"] == "acme-c1").cloned().unwrap();
    assert_eq!(row["via_entry"], created["entry"]["entry_id"]);
    // an External joins with its DID; a guest or a service does not join
    let ext = Caller::external("did:bns:carol");
    ws.join_entry(&ext, &local, admit).unwrap();
    assert_eq!(caps(&ws, &ext), json!(["read", "comment"]));
    for who in [Caller::guest(GUEST_TOKEN).unwrap(), Caller::service("some-app")] {
        assert_eq!(reason(&ws.join_entry(&who, &local, admit).unwrap_err()), Some(Deny::SessionKindNotAllowed));
    }
    // a joined grant merges into one already there, and nobody becomes the owner
    ws.grant(&alice(), "bob", None, &strings(&["read", "export"])).unwrap();
    ws.join_entry(&bob(), &local, admit).unwrap();
    assert_eq!(caps(&ws, &bob()), json!(["read", "comment", "export"]));
    assert_eq!(ws.owner().unwrap().as_deref(), Some("alice"));
    assert_eq!(ws.grant(&customer, "x", None, &strings(&["read"])).unwrap_err().code, Code::PermissionDenied);

    // closing the entry keeps the grants of those who joined (they go with ws.revoke) and stops new joins
    ws.close_entry(&alice(), &local).unwrap();
    assert_eq!(caps(&ws, &customer), json!(["read", "comment"]));
    let e = ws.join_entry(&limited("acme-c2"), &local, admit).unwrap_err();
    assert_eq!((e.code, reason(&e)), (Code::NotFound, Some(Deny::EntryUnavailable)));
    assert_eq!(reason(&ws.join_entry(&customer, "se_aaaaaaaaaaaaaaaaaaaaaaaaaa", admit).unwrap_err()), Some(Deny::EntryUnavailable));
    ws.revoke(&alice(), "acme-c1", None).unwrap();
    not_found(&ws, &customer);

    // an entry open to guests lets in a session it does not admit as anyone else: no grant, gone with the entry
    let open = ws.create_entry(&alice(), None, &strings(&["read"]), ENTRY_GUEST, None).unwrap();
    let open_local = entry_local(&open);
    let refuse = |_: u8| Err(WsError::refused(Code::PermissionDenied, Deny::EntryUnavailable, "no such way in"));
    let r = ws.join_entry(&customer, &open_local, refuse).unwrap();
    assert_eq!((r["granted"].clone(), r["joined"].clone(), r["capabilities"].clone()), (json!(false), json!(false), json!(["read"])));
    assert!(ws.list_grants(&alice()).unwrap()["grants"].as_array().unwrap().iter().all(|g| g["subject"] != "acme-c1"));
    assert_eq!(caps(&ws, &customer), json!(["read"]));
    ws.close_entry(&alice(), &open_local).unwrap();
    not_found(&ws, &customer);
}

#[test]
fn entry_ids_and_methods() {
    assert_eq!(parse_entry_methods(&json!({ "guest": true, "did_login": true })).unwrap(), ENTRY_GUEST | ENTRY_DID_LOGIN);
    assert!(parse_entry_methods(&json!({ "wallet": true })).is_err());
    assert!(parse_entry_methods(&json!({ "guest": "yes" })).is_err());
    assert!(split_entry_id("ws_aaaaaaaaaaaaaaaaaaaaaaaaaa~se_bbbbbbbbbbbbbbbbbbbbbbbbbb").is_some());
    for bad in ["ws_aaaaaaaaaaaaaaaaaaaaaaaaaa", "se_bbbbbbbbbbbbbbbbbbbbbbbbbb", "ws_x~se_y", "ws_aaaaaaaaaaaaaaaaaaaaaaaaaa~ws_bbbbbbbbbbbbbbbbbbbbbbbbbb"] {
        assert!(split_entry_id(bad).is_none(), "{bad}");
    }
}

#[test]
fn restore_keeps_grants_and_entries() {
    let env = env();
    let id = new_ws(&env, "恢复");
    let local = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        entry_local(&ws.create_entry(&alice(), None, &strings(&["read"]), ENTRY_GUEST, None).unwrap())
    };
    let pkg = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let r = ws.export(&alice(), "personal_backup", true, None).unwrap();
        fetched(&ws.objects, &r)
    };
    env.svc.import(&alice(), &pkg, "restore", true).unwrap();
    let h = env.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    assert_eq!(ws.entry(&local).unwrap().unwrap().state.as_str(), "active");
    assert_eq!(caps(&ws, &Caller::guest(GUEST_TOKEN).unwrap()), json!(["read"]));
}
