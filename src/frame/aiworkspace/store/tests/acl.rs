//! The permission model (权限需求 notepads/aiws-permission-system-prd.md §4, §9) at the store: entries by path that follow
//! their entity, logical groups with system-group members, denials (nearest layer, then tier, then deny wins), `@guest`,
//! `@all`, `@creator`, `@owner`, Agents judged as their owner until configured, private subtrees, queries and
//! explanations, moves that widen visibility, the operation record, the permission revision, inner-path publication
//! readers — and what each kind of caller sees through every read entry (R24): list, find, text query, SQL, path
//! resolution and bodies.

mod common;
use aiworkspace_store::workspace::{Caller, CallerClass, Workspace};
use aiworkspace_core::Code;
use common::*;
use serde_json::{json, Value};

fn in_groups(mut c: Caller, groups: &[&str]) -> Caller {
    if let CallerClass::Account { groups: g, .. } = &mut c.class {
        *g = groups.iter().map(|s| s.to_string()).collect();
    }
    c
}

fn set(ws: &mut Workspace, changes: Value) -> Value {
    ws.acl_set(&alice(), &changes).unwrap()
}

fn group(ws: &mut Workspace, id: &str, members: &[&str]) {
    ws.acl_set_group(&alice(), id, None, None).unwrap();
    if !members.is_empty() {
        ws.acl_members(&alice(), id, &members.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &[]).unwrap();
    }
}

fn caps(ws: &Workspace, who: &Caller, id: Option<&str>) -> Vec<&'static str> {
    ws.caps_on(who, id).unwrap()
}

fn sees(ws: &Workspace, who: &Caller, id: &str) -> bool {
    ws.read(who, id, None).is_ok()
}

fn rev(ws: &Workspace, id: &str) -> Value {
    ws.read(&alice(), id, None).unwrap()["meta_rev"].clone()
}

fn listed(ws: &Workspace, who: &Caller, at: &str) -> Vec<String> {
    ws.list(who, &json!({ "at": at })).map(|v| v["items"].as_array().unwrap().iter().map(|i| i["entity_id"].as_str().unwrap().to_string()).collect()).unwrap_or_default()
}

#[test]
fn groups_paths_and_system_groups() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    group(&mut ws, "teachers", &["group:staff"]);
    set(&mut ws, json!([{ "subject": "@teachers", "resource": "/data/任务", "allow": ["read", "update"] }]));
    let carol = in_groups(Caller::user("carol"), &["staff"]);
    assert_eq!(caps(&ws, &carol, Some("tasks")), ["read", "update"]);
    assert!(caps(&ws, &carol, Some("notes")).is_empty(), "only the table");
    // leaving the system group ends it at the next request
    assert_eq!(ws.info(&Caller::user("carol")).unwrap_err().code, Code::NotFound);
    // the entry follows its entity through a rename and a move; queries print the new path
    let r = rev(&ws, "tasks");
    ok(&mut ws, &alice(), json!([{ "op": "entity.rename", "entity_id": "tasks", "name": "任务清单", "expect": { "rev": r } }]));
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "archive", "type_id": "buckyos.container", "parent_id": "data", "order_key": "z", "name": "归档", "payload": { "kind": "folder" } }]));
    ok(&mut ws, &alice(), json!([{ "op": "tree.move", "entity_id": "tasks", "new_parent_id": "archive", "order_key": "a" }]));
    assert_eq!(caps(&ws, &carol, Some("tasks")), ["read", "update"]);
    let q = ws.acl_query(&alice(), &json!({ "by": "entity", "resource": "/data/归档/任务清单" }), None).unwrap();
    let teachers = q["subjects"].as_array().unwrap().iter().find(|s| s["subject"] == "@teachers").cloned().unwrap();
    assert_eq!((teachers["entries"][0]["source"].as_str(), teachers["entries"][0]["resource"]["path"].as_str()), (Some("direct"), Some("/data/归档/任务清单")));
    // a path through something the manager cannot read is not a target; a personal entity is never one
    assert_eq!(ws.acl_set(&alice(), &json!([{ "subject": "bob", "resource": "/data/nothing", "allow": ["read"] }])).unwrap_err().code, Code::NotFound);
}

#[test]
fn denials_nearest_layer_and_tiers() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    group(&mut ws, "teachers", &["dora"]);
    group(&mut ws, "subs", &["dora"]);
    group(&mut ws, "students", &["sam"]);
    set(&mut ws, json!([
        { "subject": "@teachers", "resource": "/", "allow": ["read"] },
        { "subject": "@teachers", "resource": "/data/任务", "allow": ["update"] },
        { "subject": "@subs", "resource": "/data/任务", "deny": ["update"] },
        { "subject": "@students", "resource": "/", "allow": ["read"] },
        { "subject": "@students", "resource": "/data", "deny": ["read"] },
        { "subject": "@students", "resource": "/data/项目说明", "allow": ["read"] },
    ]));
    let dora = Caller::user("dora");
    // two groups on one layer: the refusal wins
    assert_eq!(caps(&ws, &dora, Some("tasks")), ["read"]);
    // the nearer layer decides: students read the notes, nothing else under /data, the canvas still
    let sam = Caller::user("sam");
    assert!(sees(&ws, &sam, "notes"));
    assert!(!sees(&ws, &sam, "tasks") && !sees(&ws, &sam, "project-info"));
    assert!(sees(&ws, &sam, "surface-main"));
    // a single caller's entry beats a group's refusal on the same layer
    set(&mut ws, json!([{ "subject": "sam", "resource": "/data", "allow": ["read"] }]));
    assert!(sees(&ws, &sam, "tasks"));
    // the explanation names the deciding entry
    let ex = ws.acl_query(&alice(), &json!({ "by": "explain", "resource": "/data/任务", "principal": "dora" }), Some(&dora)).unwrap();
    let update = ex["capabilities"].as_array().unwrap().iter().find(|c| c["capability"] == "update").cloned().unwrap();
    assert_eq!((update["allowed"].as_bool(), update["decided_by"]["subject"].as_str(), update["decided_by"]["kind"].as_str()), (Some(false), Some("@subs"), Some("deny")));
    let groups: Vec<&str> = ex["judgment"]["groups"].as_array().unwrap().iter().map(|g| g["group_id"].as_str().unwrap()).collect();
    assert_eq!(groups, ["subs", "teachers"]);
    // the by-entity query lists the refusal and what it overrides
    let q = ws.acl_query(&alice(), &json!({ "by": "entity", "resource": "/data/任务" }), None).unwrap();
    let subs = q["subjects"].as_array().unwrap().iter().find(|s| s["subject"] == "@subs").cloned().unwrap();
    assert_eq!(subs["entries"][0]["overrides"][0]["subject"], "@teachers");
    let sources: Vec<(String, String)> = q["subjects"].as_array().unwrap().iter().flat_map(|s| s["entries"].as_array().unwrap().iter().map(move |e| (s["subject"].as_str().unwrap().to_string(), e["source"].as_str().unwrap().to_string()))).collect();
    assert!(sources.contains(&("@students".into(), "inherited".into())) && sources.contains(&("@teachers".into(), "workspace".into())), "{sources:?}");
}

#[test]
fn private_subtree_owner_creator_and_all() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    group(&mut ws, "staff", &["bob", "carol"]);
    set(&mut ws, json!([{ "subject": "@all", "resource": "/", "allow": ["read"] }, { "subject": "@staff", "resource": "/", "allow": ["comment"] }]));
    // a private subtree: @all refused read at its root, alice's colleague carol allowed by name
    set(&mut ws, json!([{ "subject": "@all", "resource": "/data/项目说明", "deny": ["read"] }, { "subject": "carol", "resource": "/data/项目说明", "allow": ["read"] }]));
    assert!(!sees(&ws, &bob(), "notes") && sees(&ws, &Caller::user("carol"), "notes") && sees(&ws, &alice(), "notes"));
    assert!(sees(&ws, &bob(), "tasks"));
    // @owner: nothing changes the owner, nothing names it
    assert!(ws.acl_set(&alice(), &json!([{ "subject": "@owner", "deny": ["read"] }])).is_err());
    set(&mut ws, json!([{ "subject": "alice", "resource": "/", "deny": ["read", "manage"] }]));
    assert_eq!(caps(&ws, &alice(), None).len(), 9);
    // managers never give manage (R17)
    set(&mut ws, json!([{ "subject": "dj", "resource": "/", "allow": ["read", "manage", "structure", "update"] }]));
    let dj = Caller::user("dj");
    let e = ws.acl_set(&dj, &json!([{ "subject": "bob", "allow": ["manage"] }])).unwrap_err();
    assert_eq!((e.code, e.sub.as_deref()), (Code::PermissionDenied, Some("OWNER_ONLY")));
    assert_eq!(ws.acl_set(&dj, &json!([{ "subject": "dj", "allow": ["read"] }])).unwrap_err().sub.as_deref(), Some("OWNER_ONLY"), "nor take it");
    ws.acl_set(&dj, &json!([{ "subject": "bob", "resource": "/data/任务", "allow": ["update"] }])).unwrap();
    assert_eq!(ws.acl_members(&dj, "staff", &["erin".into()], &[]).unwrap()["added"], 1);
    // @creator: a homework wall — students add Blocks, change and delete their own, not the others'
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "wall", "type_id": "buckyos.container", "parent_id": "data", "order_key": "w", "name": "作业墙", "payload": { "kind": "folder" } }]));
    group(&mut ws, "students", &["sam", "sue"]);
    set(&mut ws, json!([
        { "subject": "@students", "resource": "/data/作业墙", "allow": ["read", "structure"] },
        { "subject": "@creator", "resource": "/data/作业墙", "allow": ["update", "delete"] },
    ]));
    let (sam, sue) = (Caller::user("sam"), Caller::user("sue"));
    ok(&mut ws, &sam, json!([{ "op": "entity.create", "entity_id": "sam-hw", "type_id": "buckyos.container", "parent_id": "wall", "order_key": "a", "name": "sam", "payload": { "kind": "folder" } }]));
    ok(&mut ws, &sue, json!([{ "op": "entity.create", "entity_id": "sue-hw", "type_id": "buckyos.container", "parent_id": "wall", "order_key": "b", "name": "sue", "payload": { "kind": "folder" } }]));
    assert_eq!(ws.read(&alice(), "sam-hw", None).unwrap()["created_by"], json!("sam"));
    assert!(caps(&ws, &sam, Some("sam-hw")).contains(&"update") && caps(&ws, &sam, Some("sam-hw")).contains(&"delete"));
    assert!(!caps(&ws, &sam, Some("sue-hw")).contains(&"update"), "not another's");
    let life = ws.read(&alice(), "sue-hw", None).unwrap()["life_rev"].clone();
    let r = commit(&mut ws, &sam, json!([{ "op": "entity.delete", "entity_id": "sue-hw", "expect": { "rev": life } }]));
    assert_eq!(code(&r), "PERMISSION_DENIED");
    let life = ws.read(&alice(), "sam-hw", None).unwrap()["life_rev"].clone();
    ok(&mut ws, &sam, json!([{ "op": "entity.delete", "entity_id": "sam-hw", "expect": { "rev": life } }]));
    assert!(ws.acl_set(&alice(), &json!([{ "subject": "@creator", "allow": ["manage"] }])).is_err());
}

#[test]
fn agents_are_their_owner_until_configured() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let agent = Caller::agent_of("did:web:jarvis.test.buckyos.io", "alice");
    assert_eq!(caps(&ws, &agent, None).len(), 9, "judged as alice, the owner");
    assert_eq!(ws.info(&agent).unwrap()["judged_as"], "alice");
    ok(&mut ws, &agent, json!([{ "op": "entity.create", "entity_id": "by-agent", "type_id": "buckyos.container", "parent_id": "data", "order_key": "x", "name": "a", "payload": { "kind": "folder" } }]));
    assert_eq!(ws.read(&alice(), "by-agent", None).unwrap()["created_by"], json!("alice"), "created as its owner");
    // bob's Agent is bob here
    set(&mut ws, json!([{ "subject": "bob", "resource": "/", "allow": ["read"] }]));
    let bobs = Caller::agent_of("did:web:helper.test.buckyos.io", "bob");
    assert_eq!(caps(&ws, &bobs, None), ["read"]);
    // one entry naming the Agent and it is a subject of its own, in the whole Workspace (Q29)
    set(&mut ws, json!([{ "subject": "did:web:jarvis.test.buckyos.io", "resource": "/", "allow": ["read", "update", "structure"] }, { "subject": "did:web:jarvis.test.buckyos.io", "resource": "/data/项目说明", "deny": ["read"] }]));
    assert_eq!(caps(&ws, &agent, None), ["read", "update", "structure"]);
    assert!(!sees(&ws, &agent, "notes"));
    ok(&mut ws, &agent, json!([{ "op": "entity.create", "entity_id": "by-agent-2", "type_id": "buckyos.container", "parent_id": "data", "order_key": "y", "name": "b", "payload": { "kind": "folder" } }]));
    assert_eq!(ws.read(&alice(), "by-agent-2", None).unwrap()["created_by"], json!("did:web:jarvis.test.buckyos.io"));
    // the record names who acted and as whom
    set(&mut ws, json!([{ "subject": "did:web:jarvis.test.buckyos.io", "resource": "/" }, { "subject": "did:web:jarvis.test.buckyos.io", "resource": "/data/项目说明" }]));
    ws.acl_set_anonymous(&agent, false).unwrap();
    let last = ws.acl_audit(&alice(), 0, 1000).unwrap()["items"].as_array().unwrap().last().cloned().unwrap();
    assert_eq!((last["actor"].as_str(), last["judged_as"].as_str()), (Some("did:web:jarvis.test.buckyos.io"), Some("alice")));
    // an Agent creates Workspaces for its owner
    drop(ws);
    let made = env.svc.create_workspace(&agent, "Agent's", None).unwrap();
    assert_eq!(env.svc.workspace(made["workspace_id"].as_str().unwrap()).unwrap().lock().unwrap().owner().unwrap().as_deref(), Some("alice"));
}

#[test]
fn moves_record_widened_visibility_and_changes_bump_the_revision() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let rev0 = ws.acl_rev();
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "secret", "type_id": "buckyos.container", "parent_id": "data", "order_key": "s", "name": "老师", "payload": { "kind": "folder" } }]));
    group(&mut ws, "teachers", &["dora"]);
    group(&mut ws, "everyone", &["*"]);
    set(&mut ws, json!([{ "subject": "@teachers", "resource": "/data/老师", "allow": ["read"] }, { "subject": "@everyone", "resource": "surface-main-content", "allow": ["read"] }]));
    assert!(ws.acl_rev() > rev0);
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "grades", "type_id": "buckyos.container", "parent_id": "secret", "order_key": "a", "name": "成绩", "payload": { "kind": "folder" } }]));
    // moved to the shared canvas folder: it succeeds, and the Commit and the record say who now reaches it
    let r = ok(&mut ws, &alice(), json!([{ "op": "tree.move", "entity_id": "grades", "new_parent_id": "surface-main-content", "order_key": "z" }]));
    let widened = &r["visibility_widened"][0];
    assert_eq!(widened["entity_id"], "grades", "{r}");
    assert!(widened["gained"].as_array().unwrap().iter().any(|g| g["subject"] == "@everyone"), "{r}");
    let items = ws.acl_audit(&alice(), 0, 1000).unwrap()["items"].as_array().unwrap().clone();
    let w = items.iter().find(|i| i["action"] == "visibility.widened").unwrap();
    assert_eq!(w["target"]["entity_id"], "grades");
    let set_item = items.iter().find(|i| i["action"] == "acl.set").unwrap();
    assert_eq!(set_item["actor"], "alice");
    assert!(set_item["target"][0]["resource"]["path"].as_str().unwrap().starts_with('/'), "{set_item}");
    assert!(set_item["after"].is_array());
    // non-managers read no record
    assert_eq!(ws.acl_audit(&bob(), 0, 10).unwrap_err().code, Code::PermissionDenied);
}

/// R24: each kind of caller through every read entry.
#[test]
fn caller_matrix_over_read_entries() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    // everyone who may enter reads, except the task table — a group's entry wins over `@all`'s on the same layer
    group(&mut ws, "readers", &["group:staff"]);
    set(&mut ws, json!([
        { "subject": "@all", "resource": "/", "allow": ["read"] },
        { "subject": "@all", "resource": "/data/任务", "deny": ["read"] },
        { "subject": "@readers", "resource": "/data/任务", "allow": ["read"] },
    ]));
    ws.acl_set_anonymous(&alice(), true).unwrap();
    let staff = in_groups(Caller::user("carol"), &["staff"]);
    let lone = Caller::account("lim", true);
    let guest = Caller::guest("gt_YWl3cy1ndWVzdC10b2tlbi1maXh0dXJlLTMyYnl0ZXM").unwrap();
    let mut unsure = Caller::user("erin");
    unsure.unconfirmed.insert("group:staff".into());
    // (caller, sees tasks, sees notes)
    for (who, tasks, notes) in [(&staff, true, true), (&lone, false, true), (&guest, false, true), (&alice(), true, true)] {
        assert_eq!(sees(ws_ref(&ws), who, "tasks"), tasks, "{} tasks", who.principal);
        assert_eq!(sees(ws_ref(&ws), who, "notes"), notes, "{} notes", who.principal);
        let data = listed(&ws, who, "/data");
        assert_eq!((data.contains(&"tasks".to_string()), data.contains(&"notes".to_string())), (tasks, notes), "{} ls {data:?}", who.principal);
        assert_eq!(ws.resolve(who, None, Some("/data/任务")).is_ok(), tasks, "{} resolve", who.principal);
        assert_eq!(ws.read_as(who, &json!({ "entity_id": "notes", "as": "body" })).is_ok(), notes, "{} body", who.principal);
        assert_eq!(sql(&ws, who, "SELECT COUNT(*) AS n FROM \"/data/任务\"").is_ok(), tasks, "{} sql", who.principal);
        let found = ws.find(who, &json!({ "name": "项目说明" })).map(|v| v["items"].as_array().unwrap().iter().any(|i| i["entity_id"] == "notes")).unwrap_or(false);
        assert_eq!(found, notes, "{} find", who.principal);
    }
    // the unreadable is NOT_FOUND, and a membership that cannot be checked refuses
    assert_eq!(ws.info(&unsure).unwrap_err().denial, Some(aiworkspace_core::Deny::AuthorizationUnavailable));
    // anonymous access off: @guest's entries do nothing, a lone signed-in caller is out too
    ws.acl_set_anonymous(&alice(), false).unwrap();
    for who in [&guest, &lone] {
        assert_eq!(ws.info(who).unwrap_err().code, Code::NotFound, "{}", who.principal);
    }
    // revoked: out at the next request
    set(&mut ws, json!([{ "subject": "@readers", "resource": "/data/任务" }]));
    assert!(!sees(&ws, &staff, "tasks"));
    ws.acl_members(&alice(), "readers", &[], &["group:staff".into()]).unwrap();
    assert_eq!(ws.info(&staff).unwrap_err().code, Code::NotFound);
    // an inner-path publication's reader reads that entity through the publication only
    ws.publish_entity(&alice(), "tasks", Some("tasks")).unwrap();
    let reader = Caller::publication_reader("tasks");
    assert!(sees(&ws, &reader, "tasks") && !sees(&ws, &reader, "notes") && !sees(&ws, &reader, "project-info"));
    assert_eq!(ws.public_content("tasks", false, false).unwrap()["entity_id"], "tasks");
    assert!(ws.public_content("tasks", true, false).unwrap()["content_root"].is_string());
    // not the Workspace for anyone else: the publication is no `@guest`
    ws.acl_set_anonymous(&alice(), true).unwrap();
    assert!(!sees(&ws, &guest, "tasks"));
    assert!(!sees(&ws, &reader, "notes"), "the reader is no member: @all does not reach it");
    // a denial on the publication's subject, then the unpublish, take it away
    set(&mut ws, json!([{ "subject": "@pub:tasks", "resource": "/data/任务", "deny": ["read"] }]));
    assert!(ws.public_content("tasks", false, false).is_err());
    ws.unpublish(&alice(), "tasks").unwrap();
    assert!(ws.public_content("tasks", false, false).is_err());
}

fn ws_ref(ws: &Workspace) -> &Workspace {
    ws
}

#[test]
fn permission_design_travels_with_clones_and_packages() {
    let env = env();
    let id = project(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        group(&mut ws, "teachers", &["dora"]);
        set(&mut ws, json!([
            { "subject": "@teachers", "resource": "/data/任务", "allow": ["read", "update"] },
            { "subject": "@all", "resource": "/data/项目说明", "allow": ["comment"] },
            { "subject": "bob", "resource": "/", "allow": ["read", "export"] },
        ]));
        ws.acl_set_anonymous(&alice(), true).unwrap();
    }
    // a manager's clone: groups and their entries, no members, no single callers, anonymous off (R12)
    let c = env.svc.clone_workspace(&alice(), &id).unwrap();
    assert_eq!(c["permission_design"]["groups"], 1, "{c}");
    let copy = env.svc.workspace(c["workspace_id"].as_str().unwrap()).unwrap();
    let copy = copy.lock().unwrap();
    let groups = copy.acl_groups(&alice()).unwrap();
    assert_eq!((groups["groups"][0]["group_id"].as_str(), groups["groups"][0]["members"].as_array().unwrap().len(), groups["anonymous"].as_bool()), (Some("teachers"), 0, Some(false)));
    let all = copy.acl_query(&alice(), &json!({ "by": "all" }), None).unwrap();
    let subjects: Vec<&str> = all["entries"].as_array().unwrap().iter().map(|e| e["subject"].as_str().unwrap()).collect();
    assert_eq!(subjects.len(), 2);
    assert!(subjects.contains(&"@teachers") && subjects.contains(&"@all") && !subjects.contains(&"bob"));
    assert_eq!(copy.read(&alice(), "tasks", None).unwrap()["created_by"], json!("alice"), "the cloner created the copy's entities (C4)");
    drop(copy);
    // with export only, the copy carries no design
    let b = env.svc.clone_workspace(&bob(), &id).unwrap();
    assert!(b["permission_design"].is_null(), "{b}");
    // a manager's package carries it into a new import
    let pkg = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let r = ws.export(&alice(), "share", true, None).unwrap();
        assert!(r["manifest"]["permission_design"]["groups"].is_array());
        fetched(&ws.objects, &r)
    };
    let imported = env.svc.import(&alice(), &pkg, "new", false).unwrap();
    assert_eq!(imported["permission_design"]["entries"], 2, "{imported}");
}

/// R37: HTML code is a manager's to write or vouch for; whoever does marks the definition trusted.
#[test]
fn html_code_needs_manage_and_is_marked_trusted() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    set(&mut ws, json!([{ "subject": "bob", "resource": "/", "allow": ["read", "update", "structure"] }]));
    let def = |eid: &str| json!([{ "op": "entity.create", "entity_id": eid, "type_id": "buckyos.block-def", "parent_id": "data", "order_key": "d", "name": eid,
        "payload": { "def_id": "acme.card", "version": 1, "kind": "html", "title": "卡片", "html": { "html": "<div>hi</div>", "js": "1", "api_version": 1 } } }]);
    let r = commit(&mut ws, &bob(), def("bobs-card"));
    assert_eq!(code(&r), "PERMISSION_DENIED", "an editor does not write code: {r}");
    // a declarative definition is no code
    ok(&mut ws, &bob(), json!([{ "op": "entity.create", "entity_id": "decl", "type_id": "buckyos.block-def", "parent_id": "data", "order_key": "e", "name": "decl",
        "payload": { "def_id": "acme.kpi", "version": 1, "kind": "declarative", "title": "指标", "declarative": { "bindings": [] } } }]));
    ok(&mut ws, &alice(), def("card"));
    let trusted = ws.read(&alice(), "card", None).unwrap()["content"]["payload"]["trusted"].clone();
    assert_eq!(trusted["by"], "alice", "{trusted}");
    // bob may retitle it (no code), not change its code
    let rev = ws.read(&alice(), "card", None).unwrap()["content"]["key_revs"]["title"].clone();
    ok(&mut ws, &bob(), json!([{ "op": "entity.set_keys", "entity_id": "card", "keys": [{ "key": "title", "value": "新卡片", "expect": { "rev": rev } }] }]));
    assert_eq!(ws.read(&alice(), "card", None).unwrap()["content"]["payload"]["trusted"], trusted, "still trusted");
    let rev = ws.read(&alice(), "card", None).unwrap()["content"]["key_revs"]["html"].clone();
    let r = commit(&mut ws, &bob(), json!([{ "op": "entity.set_keys", "entity_id": "card", "keys": [{ "key": "html", "value": { "html": "<b>x</b>" }, "expect": { "rev": rev } }] }]));
    assert_eq!(code(&r), "PERMISSION_DENIED");
}

/// §4.5: a macro bound with `intersection` never writes beyond the group, whoever runs it — the owner included; `union`
/// gives nothing beyond the triggerer in this phase.
#[test]
fn macro_identity_is_the_trigger_bounded_by_an_intersection_group() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "w1", "type_id": "buckyos.wish", "parent_id": "data", "order_key": "w", "name": "w1",
        "payload": { "title": "许愿格", "prompt": "汇总", "executor": "mock", "output_mode": "overwrite" } }]));
    group(&mut ws, "signup", &[]);
    set(&mut ws, json!([{ "subject": "@signup", "resource": "/data/任务", "allow": ["read", "append"] }]));
    let now_caps = |ws: &Workspace, who: &Caller, at: &str| -> Vec<&'static str> {
        let a = ws.macro_access(who, "w1").unwrap();
        ws.read_ctx(|ctx| a.caps(ctx, at).map(|c| c.names())).unwrap()
    };
    assert_eq!(now_caps(&ws, &alice(), "tasks").len(), 9, "unbound: the triggerer's own");
    ws.acl_bind_macro(&alice(), "w1", Some("signup"), "intersection").unwrap();
    assert_eq!(now_caps(&ws, &alice(), "tasks"), ["read", "append"]);
    assert!(now_caps(&ws, &alice(), "notes").is_empty());
    // union in this phase: no more than the triggerer
    set(&mut ws, json!([{ "subject": "bob", "resource": "/", "allow": ["read", "run"] }]));
    ws.acl_bind_macro(&alice(), "w1", Some("signup"), "union").unwrap();
    assert_eq!(now_caps(&ws, &bob(), "tasks"), ["read", "run"]);
    // changing a bound macro is on the record (Q10)
    let rev = ws.read(&alice(), "w1", None).unwrap()["content"]["key_revs"]["prompt"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "w1", "keys": [{ "key": "prompt", "value": "改了", "expect": { "rev": rev } }] }]));
    assert!(ws.acl_audit(&alice(), 0, 1000).unwrap()["items"].as_array().unwrap().iter().any(|i| i["action"] == "macro.changed" && i["target"]["wish_id"] == "w1"));
    // a macro's group never holds manage: not while bound, and a group holding it is not bound; binding is a manager's
    assert_eq!(ws.acl_set(&alice(), &json!([{ "subject": "@signup", "resource": "/data/项目说明", "allow": ["manage"] }])).unwrap_err().code, Code::InvalidOperation);
    group(&mut ws, "admins", &[]);
    set(&mut ws, json!([{ "subject": "@admins", "resource": "/data/项目说明", "allow": ["manage"] }]));
    assert!(ws.acl_bind_macro(&alice(), "w1", Some("admins"), "union").is_err());
    assert_eq!(ws.acl_bind_macro(&bob(), "w1", None, "union").unwrap_err().code, Code::PermissionDenied);
    // the by-entity query names the macro and who may change it
    let q = ws.acl_query(&alice(), &json!({ "by": "entity", "resource": "/data/任务" }), None).unwrap();
    assert_eq!((q["macros"][0]["group_id"].as_str(), q["macros"][0]["editors"][0].as_str()), (Some("signup"), Some("@owner")), "{q}");
}
