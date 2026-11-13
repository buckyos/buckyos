//! Hybrid SSO (PRE-03, SSO-08, SSO-13) on the permission model (权限需求 §4.2, R5–R8): logical groups whose members
//! are principals, `*` (unrestricted accounts only), `group:` and `contact:`; who may own Workspaces; what `ws.list`
//! shows; share entries that lead into a group (guests, joins, closing, expiry, limits).

mod common;
use aiworkspace_store::entries::{parse_entry_methods, split_entry_id, ENTRY_DID_LOGIN, ENTRY_GUEST, ENTRY_PASSWORD, ENTRY_SELF_REGISTER};
use aiworkspace_store::workspace::{Caller, CallerClass, Workspace};
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

/// A logical group holding `members`, allowed `allow` on the Workspace.
fn group(ws: &mut Workspace, id: &str, members: &[&str], allow: &[&str]) {
    ws.acl_set_group(&alice(), id, None, None).unwrap();
    if !members.is_empty() {
        ws.acl_members(&alice(), id, &strings(members), &[]).unwrap();
    }
    if !allow.is_empty() {
        ws.acl_set(&alice(), &json!([{ "subject": format!("@{id}"), "resource": "/", "allow": allow }])).unwrap();
    }
}

#[test]
fn star_is_for_unrestricted_accounts_only() {
    let env = env();
    let id = new_ws(&env, "星号");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    group(&mut ws, "zone", &["*"], &["read", "comment"]);
    assert_eq!(caps(&ws, &bob()), json!(["read", "comment"]));
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    for who in [limited("acme-c1"), Caller::external("did:bns:carol"), guest.clone(), Caller::service("some-app")] {
        not_found(&ws, &who);
    }
    // an entry naming the principal itself works for every class: a Limited account by user id, an External by DID
    ws.acl_set(&alice(), &json!([{ "subject": "acme-c1", "allow": ["read"] }, { "subject": "did:bns:carol", "allow": ["read", "comment"] }])).unwrap();
    assert_eq!(caps(&ws, &limited("acme-c1")), json!(["read"]));
    assert_eq!(caps(&ws, &Caller::external("did:bns:carol")), json!(["read", "comment"]));
    // an account whose DID is that DID is not the External: a DID subject names the External only
    let mut account = Caller::user("carol");
    account.class = CallerClass::Account { restricted: false, did: Some("did:bns:carol".into()), groups: vec![] };
    assert_eq!(caps(&ws, &account), json!(["read", "comment"]), "carol (an account) reaches it through the `*` group only");
    // `*`, `group:`, `contact:` and `public` are members, never subjects (R7)
    for bad in ["*", "group:customers", "contact:did:bns:alice#vip", "public", "@owner", "@nope"] {
        let e = ws.acl_set(&alice(), &json!([{ "subject": bad, "allow": ["read"] }])).unwrap_err();
        assert!(matches!(e.code, Code::InvalidOperation | Code::NotFound), "{bad}: {e}");
    }
    // what a restricted caller learns of the permissions: its own entries, never the group's names
    let mine = ws.acl_query(&limited("acme-c1"), &json!({ "by": "subject" }), None).unwrap();
    assert_eq!(mine["entries"].as_array().unwrap().len(), 1);
    assert_eq!(mine["judgment"]["groups"], json!([]));
    assert_eq!(ws.acl_groups(&limited("acme-c1")).unwrap()["groups"], json!([]));
}

#[test]
fn group_and_contact_members() {
    let env = env();
    let id = new_ws(&env, "组与联系人");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    group(&mut ws, "customers", &["group:customers"], &["read"]);
    // a contact group is named only by its owner: the member's owner DID is the caller's own
    let owner_alice = with_did(alice(), "did:bns:alice");
    ws.acl_set_group(&alice(), "vip", None, None).unwrap();
    ws.acl_members(&owner_alice, "vip", &strings(&["contact:did:bns:alice#vip"]), &[]).unwrap();
    ws.acl_set(&alice(), &json!([{ "subject": "@vip", "allow": ["read", "comment"] }])).unwrap();
    ws.acl_set(&alice(), &json!([{ "subject": "some-app", "allow": ["read"] }])).unwrap();
    for (who, member) in [(alice(), "contact:did:bns:alice#vip"), (owner_alice.clone(), "contact:did:bns:bob#friends")] {
        assert_eq!(ws.acl_members(&who, "vip", &strings(&[member]), &[]).unwrap_err().code, Code::InvalidOperation, "{} {member}", who.principal);
    }
    for bad in ["group:", "contact:alice#vip", "contact:did:bns:alice", "contact:did:bns:alice#", "has space", "@customers", "public"] {
        assert_eq!(ws.acl_members(&alice(), "vip", &strings(&[bad]), &[]).unwrap_err().code, Code::InvalidOperation, "{bad}");
    }
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
    // ... also when another group already lets it in, since the unknown one could refuse something (§4.6)
    let mut unsure_member = in_groups(limited("acme-c1"), &["customers"]);
    unsure_member.unconfirmed.insert("contact:did:bns:alice#vip".into());
    assert_eq!(reason(&ws.info(&unsure_member).unwrap_err()), Some(Deny::AuthorizationUnavailable));
    // ... unless that group holds no entry at all
    ws.acl_set(&alice(), &json!([{ "subject": "@vip", "allow": [] }])).unwrap();
    assert_eq!(caps(&ws, &unsure_member), json!(["read"]));
    ws.acl_set(&alice(), &json!([{ "subject": "@vip", "allow": ["read", "comment"] }])).unwrap();
    // ... and never for the owner
    let mut owner = alice();
    owner.unconfirmed.insert("contact:did:bns:alice#vip".into());
    assert!(ws.info(&owner).is_ok());

    // a member sees the groups it belongs to and how, never the member lists
    let groups = ws.acl_groups(&in_groups(limited("acme-c1"), &["customers"])).unwrap();
    assert_eq!(groups["groups"].as_array().unwrap().iter().map(|g| g["group_id"].clone()).collect::<Vec<_>>(), vec![json!("customers")]);
    assert!(groups["groups"][0].get("members").is_none());
    let mine = ws.acl_query(&in_groups(limited("acme-c1"), &["customers"]), &json!({ "by": "subject" }), None).unwrap();
    assert_eq!(mine["judgment"]["groups"][0]["via"][0]["member"], "group:customers");
    // a manager sees everything
    assert_eq!(ws.acl_groups(&alice()).unwrap()["groups"].as_array().unwrap().len(), 2);
    // groups are not members of groups; deleting one takes its entries and members along
    ws.acl_delete_group(&alice(), "vip").unwrap();
    not_found(&ws, &ext);
    assert_eq!(ws.dynamic_subjects().unwrap(), vec!["group:customers"]);
}

#[test]
fn only_unrestricted_accounts_and_services_own_workspaces() {
    let env = env();
    let id = new_ws(&env, "源");
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.acl_set(&alice(), &json!([{ "subject": "acme-c1", "allow": ["read", "export"] }, { "subject": "did:bns:carol", "allow": ["read", "export"] }])).unwrap();
        ws.acl_set_anonymous(&alice(), true).unwrap();
        ws.acl_set(&alice(), &json!([{ "subject": "@guest", "allow": ["read", "export"] }])).unwrap();
    }
    let refused = |e: WsError| assert_eq!((e.code, e.denial), (Code::PermissionDenied, Some(Deny::SessionKindNotAllowed)), "{e}");
    for who in [limited("acme-c1"), Caller::external("did:bns:carol"), Caller::guest(GUEST_TOKEN).unwrap()] {
        refused(env.svc.create_workspace(&who, "x", None).unwrap_err());
        refused(env.svc.clone_workspace(&who, &id).unwrap_err());
        refused(aiworkspace_store::service::require_may_own(&who).unwrap_err());
    }
    assert!(env.svc.create_workspace(&Caller::service("did:bns:jarvis"), "agent's", None).is_ok());
    assert!(env.svc.clone_workspace(&bob(), &id).is_ok(), "bob is in @guest and may export");
}

#[test]
fn listing_shows_what_names_the_caller() {
    let env = env();
    let named = new_ws(&env, "点名");
    let grouped = new_ws(&env, "按组");
    let open = new_ws(&env, "公开");
    let everyone = new_ws(&env, "全员");
    let with = |id: &str, f: &dyn Fn(&mut Workspace)| f(&mut env.svc.workspace(id).unwrap().lock().unwrap());
    with(&named, &|ws| {
        ws.acl_set(&alice(), &json!([{ "subject": "acme-c1", "allow": ["read"] }])).unwrap();
    });
    with(&grouped, &|ws| group(ws, "customers", &["group:customers"], &["read"]));
    with(&open, &|ws| {
        ws.acl_set_anonymous(&alice(), true).unwrap();
        ws.acl_set(&alice(), &json!([{ "subject": "@guest", "allow": ["read"] }])).unwrap();
    });
    with(&everyone, &|ws| group(ws, "zone", &["*"], &["read"]));
    let listed = |who: &Caller| -> Vec<String> {
        let mut v: Vec<String> = env.svc.list_workspaces(who).unwrap()["workspaces"].as_array().unwrap().iter().map(|w| w["title"].as_str().unwrap().to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(listed(&in_groups(limited("acme-c1"), &["customers"])), vec!["按组", "点名"]);
    assert_eq!(listed(&Caller::external("did:bns:carol")), Vec::<String>::new());
    assert_eq!(listed(&Caller::guest(GUEST_TOKEN).unwrap()), Vec::<String>::new());
    assert_eq!(listed(&bob()), vec!["全员"], "never one bob may only enter anonymously (R8)");
    assert_eq!(listed(&alice()), vec!["全员", "公开", "按组", "点名"]);
    assert_eq!(env.svc.dynamic_subjects().unwrap(), vec!["group:customers"]);
    // the open one stays reachable by its link
    assert_eq!(caps(&env.svc.workspace(&open).unwrap().lock().unwrap(), &limited("acme-c1")), json!(["read"]));
}

fn entry_local(v: &Value) -> String {
    split_entry_id(v["entry"]["entry_id"].as_str().unwrap()).unwrap().1.to_string()
}

#[test]
fn guests_come_in_only_through_an_entry() {
    let env = env();
    let id = new_ws(&env, "入口");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    group(&mut ws, "visitors", &[], &["read", "comment"]);
    ws.acl_set(&alice(), &json!([{ "subject": "bob", "allow": ["read", "comment"] }])).unwrap();
    // manage only; a group without manage or run; at least one way in; the group exists
    assert_eq!(ws.create_entry(&bob(), "visitors", ENTRY_GUEST, None, None).unwrap_err().code, Code::PermissionDenied);
    assert_eq!(ws.create_entry(&alice(), "visitors", 0, None, None).unwrap_err().code, Code::InvalidOperation);
    assert_eq!(ws.create_entry(&alice(), "visitors", ENTRY_GUEST, Some(T0 / 1000 - 1), None).unwrap_err().code, Code::InvalidOperation);
    assert_eq!(ws.create_entry(&alice(), "nobody", ENTRY_GUEST, None, None).unwrap_err().code, Code::NotFound);
    group(&mut ws, "runners", &[], &["read", "run"]);
    assert_eq!(ws.create_entry(&alice(), "runners", ENTRY_GUEST, None, None).unwrap_err().code, Code::InvalidOperation);

    not_found(&ws, &guest);
    let created = ws.create_entry(&alice(), "visitors", ENTRY_GUEST, Some(T0 / 1000 + 3600), None).unwrap();
    let entry = &created["entry"];
    assert_eq!(entry["state"], "active");
    assert_eq!(entry["group_id"], "visitors");
    assert_eq!(entry["methods"], json!({ "guest": true, "password": false, "self_register": false, "did_login": false }));
    assert!(entry["entry_id"].as_str().unwrap().starts_with(&format!("{id}~se_")), "{entry}");
    let local = entry_local(&created);
    // S2: knowing the Workspace id gives nothing; presenting the entry joins its group
    not_found(&ws, &guest);
    let joined = ws.join_entry(&guest, &local, |_| Ok(())).unwrap();
    assert_eq!((joined["group_id"].clone(), joined["joined"].clone()), (json!("visitors"), json!(true)));
    assert_eq!(caps(&ws, &guest), json!(["read", "comment"]));
    let other_guest = Caller::guest("gt_YWl3cy1ndWVzdC10b2tlbi1maXh0dXJlLTMyYnl0ZXQ").unwrap();
    not_found(&ws, &other_guest);
    // the entry target may now never hold manage or run (R18)
    assert_eq!(ws.acl_set(&alice(), &json!([{ "subject": "@visitors", "allow": ["read", "run"] }])).unwrap_err().code, Code::InvalidOperation);
    assert_eq!(ws.list_entries(&alice()).unwrap()["entries"][0]["joined"], json!(1));
    assert_eq!(ws.list_entries(&bob()).unwrap_err().code, Code::PermissionDenied);

    // closing stops new joins; with remove_members those who came through it leave too
    assert_eq!(ws.close_entry(&bob(), &local, false).unwrap_err().code, Code::PermissionDenied);
    assert_eq!(ws.close_entry(&alice(), &local, false).unwrap()["entry"]["state"], "closed");
    assert_eq!(caps(&ws, &guest), json!(["read", "comment"]), "a member stays until removed");
    assert_eq!(reason(&ws.join_entry(&other_guest, &local, |_| Ok(())).unwrap_err()), Some(Deny::EntryUnavailable));
    assert_eq!(ws.close_entry(&alice(), &local, true).unwrap()["removed_members"], json!(1));
    not_found(&ws, &guest);
    assert_eq!(ws.close_entry(&alice(), "se_aaaaaaaaaaaaaaaaaaaaaaaaaa", false).unwrap_err().code, Code::NotFound);

    // an entry that expires stops admitting by itself; one with a limit admits that many
    let timed = ws.create_entry(&alice(), "visitors", ENTRY_GUEST, Some(T0 / 1000 + 60), Some(1)).unwrap();
    ws.join_entry(&guest, &entry_local(&timed), |_| Ok(())).unwrap();
    assert_eq!(reason(&ws.join_entry(&other_guest, &entry_local(&timed), |_| Ok(())).unwrap_err()), Some(Deny::EntryUnavailable));
    drop(ws);
    env.advance(61_000);
    let mut ws = h.lock().unwrap();
    assert_eq!(ws.entry(&entry_local(&timed)).unwrap().unwrap().state.as_str(), "expired");
    assert_eq!(reason(&ws.join_entry(&other_guest, &entry_local(&timed), |_| Ok(())).unwrap_err()), Some(Deny::EntryUnavailable));
}

#[test]
fn joining_an_entry_makes_a_member() {
    let env = env();
    let id = new_ws(&env, "加入");
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    group(&mut ws, "readers", &[], &["read", "comment"]);
    let created = ws.create_entry(&alice(), "readers", parse_entry_methods(&json!({ "self_register": true })).unwrap(), None, None).unwrap();
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
    assert_eq!(joined["joined"], json!(true));
    assert_eq!(caps(&ws, &customer), json!(["read", "comment"]));
    let again = ws.join_entry(&customer, &local, admit).unwrap();
    assert_eq!(again["joined"], json!(false), "joining again changes nothing");
    let members = ws.acl_groups(&alice()).unwrap()["groups"][0]["members"].clone();
    assert_eq!(members[0]["member"], "acme-c1");
    assert_eq!(members[0]["via_entry"], created["entry"]["entry_id"]);
    // an External joins with its DID; a guest needs an entry open to guests; a service does not join
    let ext = Caller::external("did:bns:carol");
    ws.join_entry(&ext, &local, admit).unwrap();
    assert_eq!(caps(&ws, &ext), json!(["read", "comment"]));
    assert_eq!(reason(&ws.join_entry(&Caller::guest(GUEST_TOKEN).unwrap(), &local, admit).unwrap_err()), Some(Deny::EntryUnavailable));
    assert_eq!(reason(&ws.join_entry(&Caller::service("some-app"), &local, admit).unwrap_err()), Some(Deny::SessionKindNotAllowed));
    // a member's own entries still count, and nobody becomes the owner
    ws.acl_set(&alice(), &json!([{ "subject": "bob", "allow": ["export"] }])).unwrap();
    ws.join_entry(&bob(), &local, admit).unwrap();
    assert_eq!(caps(&ws, &bob()), json!(["read", "comment", "export"]));
    assert_eq!(ws.owner().unwrap().as_deref(), Some("alice"));
    assert_eq!(ws.acl_set(&customer, &json!([{ "subject": "x", "allow": ["read"] }])).unwrap_err().code, Code::PermissionDenied);

    // closing the entry keeps the members unless asked, and stops new joins
    ws.close_entry(&alice(), &local, false).unwrap();
    assert_eq!(caps(&ws, &customer), json!(["read", "comment"]));
    let e = ws.join_entry(&limited("acme-c2"), &local, admit).unwrap_err();
    assert_eq!((e.code, reason(&e)), (Code::NotFound, Some(Deny::EntryUnavailable)));
    ws.acl_members(&alice(), "readers", &[], &strings(&["acme-c1"])).unwrap();
    not_found(&ws, &customer);

    // an entry open to guests lets in a signed-in session it does not otherwise admit
    let open = ws.create_entry(&alice(), "readers", ENTRY_GUEST, None, None).unwrap();
    let refuse = |_: u8| Err(WsError::refused(Code::PermissionDenied, Deny::EntryUnavailable, "no such way in"));
    assert_eq!(ws.join_entry(&customer, &entry_local(&open), refuse).unwrap()["joined"], json!(true));
    assert_eq!(caps(&ws, &customer), json!(["read", "comment"]));
}

#[test]
fn entry_ids_and_methods() {
    assert_eq!(parse_entry_methods(&json!({ "guest": true, "did_login": true })).unwrap(), ENTRY_GUEST | ENTRY_DID_LOGIN);
    assert!(parse_entry_methods(&json!({ "wallet": true })).is_err());
    assert!(parse_entry_methods(&json!({ "guest": "yes" })).is_err());
    assert!(split_entry_id("ws_2611a0b3~se_bbbbbbbbbbbbbbbbbbbbbbbbbb").is_some());
    for bad in ["ws_2611a0b3", "se_bbbbbbbbbbbbbbbbbbbbbbbbbb", "ws_x~se_y", "ws_2611a0b3~ws_bbbbbbbbbbbbbbbbbbbbbbbbbb"] {
        assert!(split_entry_id(bad).is_none(), "{bad}");
    }
}

#[test]
fn restore_keeps_permissions_and_entries() {
    let env = env();
    let id = new_ws(&env, "恢复");
    let guest = Caller::guest(GUEST_TOKEN).unwrap();
    let local = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        group(&mut ws, "visitors", &[], &["read"]);
        let local = entry_local(&ws.create_entry(&alice(), "visitors", ENTRY_GUEST, None, None).unwrap());
        ws.join_entry(&guest, &local, |_| Ok(())).unwrap();
        local
    };
    let pkg = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let r = ws.export(&alice(), "personal_backup", true, None).unwrap();
        fetched(&ws.objects, &r)
    };
    assert_eq!(env.svc.import(&bob(), &pkg, "restore", true).unwrap_err().code, Code::PermissionDenied, "a restore over a Workspace is the owner's (R17)");
    env.svc.import(&alice(), &pkg, "restore", true).unwrap();
    let h = env.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    assert_eq!(ws.entry(&local).unwrap().unwrap().state.as_str(), "active");
    assert_eq!(caps(&ws, &guest), json!(["read"]));
    assert!(ws.acl_audit(&alice(), 0, 100).unwrap()["items"].as_array().unwrap().iter().any(|i| i["action"] == "entry.join"), "the record moves over too");
}
