//! Presence and the work log (UI 框架 PRD §4.2, §4.5, §12, K2, K3) in the store: whose groups a reader sees and on which
//! Surface; the timeline of Commits (with messages), wish runs, permission changes, sessions and shows, newest first,
//! filtered by what each reader may read now, by “mine” and by “Agents and macros”, in pages that join up.

mod common;

use aiworkspace_store::presence::PresenceInput;
use aiworkspace_store::session::registry::SessionRow;
use aiworkspace_store::worklog::{Query, ShowEvent};
use aiworkspace_store::workspace::{Caller, CommitOpts, Workspace};
use common::*;
use serde_json::{json, Value};

const PRESENTATION: &str = include_str!("../../fixtures/presentation/commits.json");

fn folder(id: &str, parent: &str, order: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.container", "parent_id": parent, "order_key": order, "name": id,
            "payload": { "kind": "folder", "title": id } })
}

fn surface(id: &str, order: &str) -> Value {
    json!([
        { "op": "entity.create", "entity_id": format!("{id}-content"), "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": order,
          "name": format!("{id}-content"), "payload": { "kind": "folder", "system": "surface_content", "surface_id": id, "title": id } },
        { "op": "entity.create", "entity_id": id, "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": order, "name": id,
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": id, "content_folder_id": format!("{id}-content") } },
    ])
}

/// One Commit with a message, a second later than the one before.
fn said(env: &Env, ws: &mut Workspace, who: &Caller, message: &str, ops: Value) -> Value {
    env.advance(1_000);
    let mut req = request(ws, ops);
    req["message"] = json!(message);
    let r = ws.commit(&req, who, &CommitOpts::default());
    assert_eq!(r["status"], "accepted", "{r}");
    r
}

/// Set a folder's title (the rev it expects read first).
fn retitle(ws: &Workspace, id: &str, value: &str) -> Value {
    let rev = ws.read(&alice(), id, None).unwrap()["content"]["key_revs"]["title"].as_u64().unwrap_or(0);
    json!([{ "op": "entity.set_keys", "entity_id": id, "keys": [{ "key": "title", "value": value, "expect": { "rev": rev } }] }])
}

fn new_ws(env: &Env) -> String {
    env.svc.create_workspace(&alice(), "协作", None).unwrap()["workspace_id"].as_str().unwrap().to_string()
}

fn log(env: &Env, ws: &str, who: &Caller, p: Value) -> Value {
    env.svc.work_log(who, ws, &Query::parse(&p).unwrap(), Vec::new()).unwrap()
}

fn messages(page: &Value) -> Vec<String> {
    page["entries"].as_array().unwrap().iter().filter(|e| e["kind"] == "commit").map(|e| e["message"].as_str().unwrap_or("").to_string()).collect()
}

fn ids(page: &Value) -> Vec<String> {
    page["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_string()).collect()
}

/// alice's Workspace: two canvases, a folder on the first, a secret folder bob may not read, a Commit of bob's.
fn setup(env: &Env) -> String {
    let id = new_ws(env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    said(env, &mut ws, &alice(), "建画布 s1", surface("s1", "a"));
    said(env, &mut ws, &alice(), "建画布 s2", surface("s2", "b"));
    ws.acl_set(&alice(), &json!([{ "subject": "bob", "resource": "/", "allow": ["read", "update", "append", "structure", "delete"] }])).unwrap();
    said(env, &mut ws, &alice(), "加文件夹", json!([folder("plan", "s1-content", "a")]));
    said(env, &mut ws, &bob(), "bob 的文件夹", json!([folder("bobs", "data", "c")]));
    said(env, &mut ws, &alice(), "机密", json!([folder("secret", "data", "d")]));
    ws.acl_set(&alice(), &json!([{ "subject": "bob", "resource": "secret", "deny": ["read"] }])).unwrap();
    let ops = retitle(&ws, "secret", "x");
    said(env, &mut ws, &alice(), "改机密", ops);
    id
}

#[test]
fn the_log_is_newest_first_with_messages_and_what_the_reader_may_read() {
    let env = env();
    let id = setup(&env);
    let all = log(&env, &id, &alice(), json!({}));
    assert_eq!(messages(&all), ["改机密", "机密", "bob 的文件夹", "加文件夹", "建画布 s2", "建画布 s1"], "{all}");
    let keys: Vec<String> = all["entries"].as_array().unwrap().iter().map(|e| format!("{}|{}", e["at"].as_str().unwrap(), e["id"].as_str().unwrap())).collect();
    assert!(keys.windows(2).all(|w| w[0] > w[1]), "strictly newest first: {keys:?}");
    assert_eq!(all["more"], false);
    // a Commit carries who, what kind of writer, and the object to locate (with its canvas)
    let plan = all["entries"].as_array().unwrap().iter().find(|e| e["message"] == "加文件夹").unwrap();
    assert_eq!((plan["actor"].as_str(), plan["actor_kind"].as_str(), plan["kind"].as_str(), plan["mine"].as_bool()), (Some("alice"), Some("person"), Some("commit"), Some(true)));
    assert_eq!((plan["target"]["entity_id"].as_str(), plan["target"]["surface_id"].as_str()), (Some("plan"), Some("s1")), "{plan}");
    let s1 = all["entries"].as_array().unwrap().iter().find(|e| e["message"] == "建画布 s1").unwrap();
    assert_eq!(s1["target"]["surface_id"], "s1", "a canvas locates as itself: {s1}");
    // the managers see the permission changes, in the same timeline
    let acl: Vec<&Value> = all["entries"].as_array().unwrap().iter().filter(|e| e["kind"] == "acl").collect();
    assert_eq!(acl.len(), 2, "{all}");
    assert_eq!(acl[0]["action"], "acl.set");
    // bob: nothing touching only the secret, no permission record
    let bobs = log(&env, &id, &bob(), json!({}));
    assert_eq!(messages(&bobs), ["bob 的文件夹", "加文件夹", "建画布 s2", "建画布 s1"], "{bobs}");
    assert!(bobs["entries"].as_array().unwrap().iter().all(|e| e["kind"] != "acl"), "{bobs}");
    assert!(bobs["entries"].as_array().unwrap().iter().all(|e| e["mine"] == json!(e["actor"] == "bob")));
    // mine
    let mine = log(&env, &id, &bob(), json!({ "filter": "mine" }));
    assert_eq!(messages(&mine), ["bob 的文件夹"]);
    let mine = log(&env, &id, &alice(), json!({ "filter": "mine" }));
    assert_eq!(messages(&mine).len(), 5);
    assert!(mine["entries"].as_array().unwrap().iter().all(|e| e["actor"] == "alice"));
    // nothing an Agent or a macro did yet
    assert_eq!(log(&env, &id, &alice(), json!({ "filter": "automation" }))["entries"], json!([]));
    assert!(Query::parse(&json!({ "filter": "everything" })).is_err());
    assert!(Query::parse(&json!({ "before": "nonsense" })).is_err());
    // someone who cannot enter learns nothing
    assert!(env.svc.work_log(&Caller::user("carol"), &id, &Query::parse(&json!({})).unwrap(), Vec::new()).is_err());
    // an undo is the undoer's (its Commit is internal, origin `system`)
    let commit_id = bobs["entries"].as_array().unwrap().iter().find(|e| e["message"] == "bob 的文件夹").unwrap()["commit_id"].as_str().unwrap().to_string();
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        env.advance(1_000);
        let req = json!({ "epoch": ws.epoch, "commit_id": commit_id, "idempotency_key": "undo-1", "session_id": "s1" });
        let r = ws.undo(&bob(), &req);
        assert_eq!(r["status"], "accepted", "{r}");
    }
    let undo = log(&env, &id, &bob(), json!({ "limit": 1 }))["entries"][0].clone();
    assert_eq!((undo["undoes"].as_str(), undo["actor"].as_str(), undo["actor_kind"].as_str(), undo["mine"].as_bool()), (Some(commit_id.as_str()), Some("bob"), Some("person"), Some(true)), "{undo}");
}

#[test]
fn pages_join_up_without_gaps_or_repeats() {
    let env = env();
    let id = setup(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        for i in 0..5 {
            let ops = retitle(&ws, "bobs", &format!("t{i}"));
            said(&env, &mut ws, &bob(), &format!("第 {i} 次"), ops);
        }
    }
    for who in [alice(), bob()] {
        let whole = ids(&log(&env, &id, &who, json!({ "limit": 200 })));
        let mut paged = Vec::new();
        let mut before: Option<String> = None;
        for _ in 0..20 {
            let page = log(&env, &id, &who, json!({ "limit": 3, "before": before }));
            assert!(page["entries"].as_array().unwrap().len() <= 3);
            paged.extend(ids(&page));
            if page["more"] == false {
                assert!(page["next_before"].is_null());
                break;
            }
            before = page["next_before"].as_str().map(str::to_string);
        }
        assert_eq!(paged, whole, "{}", who.principal);
    }
}

#[test]
fn a_long_run_of_unreadable_commits_ends_the_page_early_with_a_cursor() {
    let env = env();
    let id = setup(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        for i in 0..1_100 {
            let ops = retitle(&ws, "secret", &format!("s{i}"));
            let r = commit(&mut ws, &alice(), ops);
            assert_eq!(r["status"], "accepted");
        }
    }
    let first = log(&env, &id, &bob(), json!({}));
    assert_eq!(first["entries"], json!([]), "the first thousand rows were all unreadable");
    assert_eq!(first["more"], true);
    let next = log(&env, &id, &bob(), json!({ "before": first["next_before"] }));
    assert_eq!(messages(&next), ["bob 的文件夹", "加文件夹", "建画布 s2", "建画布 s1"], "{next}");
}

#[test]
fn runs_start_and_end_in_the_log_as_proc_get_shows_them() {
    let env = env();
    let id = setup(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        env.advance(1_000);
        // a run of a wish shown on s1 (the folder `plan` stands for the wish: the log looks at its readability only)
        let (run, _) = ws.wish_run_create(&bob(), "wish.mock@1", &json!({ "wish_id": "plan", "stage": "run" }), "k1", &json!(null)).unwrap();
        env.advance(1_000);
        ws.wish_run_set(&run, Some("running"), None, None, None, None).unwrap();
        env.advance(1_000);
        ws.wish_run_set(&run, Some("failed"), None, None, None, Some(&json!({ "error": { "code": "PROGRAM_FAILED", "detail": "x" } }))).unwrap();
        env.advance(5_000);
        // a later update of the run does not move its end
        ws.wish_run_set(&run, None, None, None, None, Some(&json!({ "note": 1 }))).unwrap();
        // alice's run of the secret: bob sees nothing of it
        env.advance(1_000);
        ws.wish_run_create(&alice(), "wish.mock@1", &json!({ "wish_id": "secret", "stage": "run" }), "k2", &json!(null)).unwrap();
    }
    let runs = |who: &Caller, filter: &str| -> Vec<Value> {
        log(&env, &id, who, json!({ "filter": filter }))["entries"].as_array().unwrap().iter().filter(|e| e["kind"] == "run").cloned().collect()
    };
    let bobs = runs(&bob(), "all");
    assert_eq!(bobs.len(), 2, "{bobs:?}");
    assert_eq!((bobs[0]["phase"].as_str(), bobs[0]["state"].as_str(), bobs[0]["error_code"].as_str()), (Some("end"), Some("failed"), Some("PROGRAM_FAILED")));
    assert_eq!((bobs[1]["phase"].as_str(), bobs[1]["actor"].as_str(), bobs[1]["actor_kind"].as_str()), (Some("start"), Some("bob"), Some("macro")));
    // a run of the Mock executor runs no macro (UI 框架 PRD Q27): the log says so
    assert!(bobs.iter().all(|r| r["simulated"] == json!(true)), "{bobs:?}");
    assert_eq!(bobs[0]["target"]["surface_id"], "s1", "{}", bobs[0]);
    let ended = bobs[0]["at"].as_str().unwrap();
    let started = bobs[1]["at"].as_str().unwrap();
    assert_eq!(ended, aiworkspace_core::value::format_utc_ms(T0 + 9_000), "the end is when the state last changed (started {started})");
    // the runs are the automation filter's; alice (who may update the wish) sees bob's and her own
    assert_eq!(runs(&bob(), "automation").len(), 2);
    assert_eq!(log(&env, &id, &bob(), json!({ "filter": "automation" }))["entries"].as_array().unwrap().len(), 2, "only runs: no Commit was a macro's");
    assert_eq!(runs(&alice(), "all").len(), 3);
    assert_eq!(runs(&alice(), "mine").len(), 1);
    // someone who may only read the wish sees its own runs only
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.acl_set(&alice(), &json!([{ "subject": "carol", "resource": "/", "allow": ["read", "run"] }])).unwrap();
    }
    assert_eq!(runs(&Caller::user("carol"), "all").len(), 0);
}

#[test]
fn sessions_and_shows_start_and_end_in_the_log() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "课件", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let fx: Value = serde_json::from_str(PRESENTATION).unwrap();
        for c in fx["commits"].as_array().unwrap() {
            ok(&mut ws, &alice(), c["operations"].clone());
        }
        ws.acl_set(&alice(), &json!([{ "subject": "bob", "resource": "/", "allow": ["read"] }])).unwrap();
    }
    env.advance(1_000);
    let v = env.svc.session_create(&alice(), &json!({ "workspace_id": id, "path_id": "path-intro" })).unwrap();
    let mut s: SessionRow = env.svc.sessions.require(v["session"]["session_id"].as_str().unwrap()).unwrap();
    let actor = env.svc.require_actor(&s, &alice(), None).unwrap();
    env.advance(1_000);
    env.svc.session_start(&mut s, &actor).unwrap();
    env.advance(1_000);
    env.svc.session_end(&mut s, &actor, 60_000).unwrap();
    let shows = vec![
        ShowEvent { show_id: "sh_1".into(), presenter: "bob".into(), path_id: "path-intro".into(), at_ms: T0 + 10_000, end: false },
        ShowEvent { show_id: "sh_1".into(), presenter: "bob".into(), path_id: "path-intro".into(), at_ms: T0 + 11_000, end: true },
    ];
    let q = Query::parse(&json!({})).unwrap();
    let kinds = |who: &Caller, f: &str| -> Vec<(String, String)> {
        let q = Query::parse(&json!({ "filter": f })).unwrap();
        env.svc.work_log(who, &id, &q, shows.clone()).unwrap()["entries"].as_array().unwrap().iter()
            .filter(|e| matches!(e["kind"].as_str(), Some("session" | "show")))
            .map(|e| (e["kind"].as_str().unwrap().to_string(), e["phase"].as_str().unwrap().to_string())).collect()
    };
    let pairs = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    assert_eq!(kinds(&alice(), "all"), pairs(&[("show", "end"), ("show", "start"), ("session", "end"), ("session", "start")]));
    let page = env.svc.work_log(&alice(), &id, &q, shows.clone()).unwrap();
    let start = page["entries"].as_array().unwrap().iter().find(|e| e["kind"] == "session" && e["phase"] == "start").unwrap();
    assert_eq!((start["actor"].as_str(), start["target"]["entity_id"].as_str(), start["session_id"].as_str()), (Some("alice"), Some("path-intro"), Some(s.session_id.as_str())));
    // bob reads the path (sees the show) but has nothing to do with the session
    assert_eq!(kinds(&bob(), "all"), pairs(&[("show", "end"), ("show", "start")]));
    assert_eq!(kinds(&bob(), "mine"), pairs(&[("show", "end"), ("show", "start")]));
    assert_eq!(kinds(&alice(), "mine"), pairs(&[("session", "end"), ("session", "start")]));
    assert!(kinds(&alice(), "automation").is_empty());
}

#[test]
fn presence_shows_groups_to_managers_only_and_surfaces_the_reader_may_read() {
    let env = env();
    let id = setup(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.acl_set_group(&alice(), "editors", Some("编辑组"), None).unwrap();
    ws.acl_members(&alice(), "editors", &["bob".to_string()], &[]).unwrap();
    ws.acl_set(&alice(), &json!([{ "subject": "bob", "resource": "s2", "deny": ["read"] }])).unwrap();
    // what a window may report
    assert_eq!(ws.readable_surface(&bob(), "s1").unwrap().as_deref(), Some("s1"));
    assert_eq!(ws.readable_surface(&bob(), "s2").unwrap(), None, "a canvas it cannot read");
    assert_eq!(ws.readable_surface(&bob(), "plan").unwrap(), None, "not a canvas");
    assert_eq!(ws.readable_surface(&bob(), "nothing").unwrap(), None);
    let (a, b) = (alice(), bob());
    let people = [PresenceInput { caller: &a, surface_id: Some("s2") }, PresenceInput { caller: &b, surface_id: Some("s1") }];
    // the owner manages: everyone's groups
    let seen = ws.presence_view(&alice(), &people).unwrap();
    assert_eq!((seen[0]["self"].as_bool(), seen[0]["owner"].as_bool(), seen[0]["surface_id"].as_str()), (Some(true), Some(true), Some("s2")));
    assert_eq!(seen[1]["groups"], json!([{ "group_id": "editors", "title": "编辑组" }]));
    assert_eq!((seen[1]["owner"].as_bool(), seen[1]["surface_id"].as_str()), (Some(false), Some("s1")));
    // bob: his own groups, not alice's; alice's canvas is one he cannot read
    let seen = ws.presence_view(&bob(), &people).unwrap();
    assert!(seen[0].get("owner").is_none() && seen[0].get("groups").is_none(), "{}", seen[0]);
    assert!(seen[0]["surface_id"].is_null(), "{}", seen[0]);
    assert_eq!(seen[1]["groups"], json!([{ "group_id": "editors", "title": "编辑组" }]));
    assert_eq!(seen[1]["self"], true);
}
