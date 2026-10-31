//! Presentation sessions in the store (第四期规划 §3–§8, P4-01/02/04/05/09/13/14/17): the run Workspace is a whole
//! clone cut back to the clone point with the notes moved to the presenter's scope; links let the audience in with
//! their own identity; the stage is the service's; tasks, claims and answers go through the session methods only,
//! in single Commits that also refresh the feeds and land on the timeline; ending, saving and expiry are separate.

mod common;

use aiworkspace_core::access::Access;
use aiworkspace_store::session::registry::SessionRow;
use aiworkspace_store::session::{feed_sample, Actor, NewEvent, Role};
use aiworkspace_store::workspace::Caller;
use common::*;
use serde_json::{json, Value};

const PRESENTATION: &str = include_str!("../../fixtures/presentation/commits.json");

/// A Workspace owned by alice with the third phase's presentation fixture (`path-intro`, notes on Frames and Viewports).
fn courseware(env: &Env) -> String {
    let id = env.svc.create_workspace(&alice(), "课件", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let handle = env.svc.workspace(&id).unwrap();
    let mut ws = handle.lock().unwrap();
    let fx: Value = serde_json::from_str(PRESENTATION).unwrap();
    for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
        let req = json!({ "protocol_version": "1.0", "workspace_id": ws.workspace_id, "epoch": ws.epoch,
                          "idempotency_key": format!("fixture/{}", i + 1), "operations": c["operations"] });
        let r = ws.commit(&req, &alice(), &Default::default());
        assert_eq!(r["status"], "accepted", "{r}");
    }
    id
}

fn session(env: &Env, ws: &str, policy: Value) -> SessionRow {
    let v = env.svc.session_create(&alice(), &json!({ "workspace_id": ws, "path_id": "path-intro", "policy": policy })).unwrap();
    env.svc.sessions.require(v["session"]["session_id"].as_str().unwrap()).unwrap()
}

fn fresh(env: &Env, s: &SessionRow) -> SessionRow {
    env.svc.sessions.require(&s.session_id).unwrap()
}

fn presenter(env: &Env, s: &SessionRow) -> Actor {
    env.svc.require_actor(s, &alice(), None).unwrap()
}

fn invite(env: &Env, s: &SessionRow, kind: &str) -> aiworkspace_store::session::registry::InviteRow {
    let v = env.svc.session_invite(s, &presenter(env, s), &json!({ "kind": kind })).unwrap();
    env.svc.sessions.invite_by_token(v["invite"]["token"].as_str().unwrap()).unwrap().unwrap()
}

fn join(env: &Env, s: &SessionRow, who: &Caller, inv: &aiworkspace_store::session::registry::InviteRow) -> Actor {
    env.svc.session_join(s, who, Some(inv), Some(who.principal.as_str())).unwrap().0
}

fn guest() -> Caller {
    Caller::guest(&format!("gt_{}", "A".repeat(43))).unwrap()
}

#[test]
fn the_run_workspace_starts_at_the_clone_point_without_notes() {
    let env = env();
    let src = courseware(&env);
    let s = session(&env, &src, json!({}));
    assert_eq!(s.state, "prepared");
    assert_ne!(s.workspace_id, src);
    let h = env.svc.workspace(&s.workspace_id).unwrap();
    let ws = h.lock().unwrap();
    assert_eq!(ws.purpose().as_deref(), Some("session"));
    assert!(ws.is_session_run());
    // history: nothing below the clone point, for anybody
    let point = s.baseline["source"]["seq"].as_u64().unwrap();
    assert_eq!(ws.base_seq(), point);
    let early = ws.get_changes(&alice(), &ws.epoch, 0, 100, true).unwrap_err();
    assert_eq!(early.code.as_str(), "BASE_TOO_OLD");
    // the notes left the shared entities and live in the presenter's table
    let frame = ws.read(&alice(), "frame1", None).unwrap();
    assert!(frame["content"]["payload"]["presentation"].get("notes").is_none(), "{frame}");
    let vp = ws.read(&alice(), "viewport1", None).unwrap();
    assert!(vp["content"]["payload"].get("notes").is_none());
    let notes = s.baseline["notes_entity_id"].as_str().unwrap();
    let table = ws.read(&alice(), notes, None).unwrap();
    assert_eq!(table["scope"], "presenter");
    // not listed among Workspaces, and the source keeps its notes
    drop(ws);
    assert!(!env.svc.list_workspaces(&alice()).unwrap()["workspaces"].as_array().unwrap().iter().any(|w| w["workspace_id"] == json!(s.workspace_id)));
    let src_ws = env.svc.workspace(&src).unwrap();
    let frame = src_ws.lock().unwrap().read(&alice(), "frame1", None).unwrap();
    assert!(frame["content"]["payload"]["presentation"]["notes"].is_string());
}

#[test]
fn links_let_the_audience_in_and_revocation_takes_it_back() {
    let env = env();
    let src = courseware(&env);
    let s = session(&env, &src, json!({}));
    // a stranger without a link sees nothing
    assert_eq!(env.svc.session_join(&s, &bob(), None, None).unwrap_err().sub, Some("SESSION_NOT_FOUND"));
    let at = invite(&env, &s, "at");
    let b = join(&env, &s, &bob(), &at);
    assert_eq!(b.role, Role::Participant);
    assert_eq!(b.level, "verified");
    let g = join(&env, &s, &guest(), &at);
    assert_eq!(g.level, "guest");
    let notes = s.baseline["notes_entity_id"].as_str().unwrap().to_string();
    {
        let h = env.svc.workspace(&s.workspace_id).unwrap();
        let ws = h.lock().unwrap();
        // the audience reads the public content, never the notes table
        ws.read(&bob(), "frame1", None).unwrap();
        assert_eq!(ws.read(&bob(), &notes, None).unwrap_err().code.as_str(), "NOT_FOUND");
        assert_eq!(ws.read(&guest(), &notes, None).unwrap_err().code.as_str(), "NOT_FOUND");
        // nor the clone's history: the baseline a replay starts from carries no notes either
        let baseline = ws.replay_baseline(&b.access()).unwrap();
        assert!(!baseline["entities"].as_array().unwrap().iter().any(|e| e["entity_id"] == json!(notes)));
        assert!(!baseline.to_string().contains("开场：介绍我们是谁"));
    }
    // a display link reads under its own subject, a prompter link acts for the presenter
    let dt = invite(&env, &s, "dt");
    let d = env.svc.session_actor(&s, &guest(), Some(&dt)).unwrap().unwrap();
    assert_eq!(d.role, Role::Display);
    let pt = invite(&env, &s, "pt");
    assert_eq!(env.svc.session_actor(&s, &guest(), Some(&pt)).unwrap().unwrap().role, Role::Prompter);
    // revoking the audience link removes the grants it gave
    env.svc.session_revoke_invite(&s, &presenter(&env, &s), &at.invite_id).unwrap();
    let h = env.svc.workspace(&s.workspace_id).unwrap();
    let ws = h.lock().unwrap();
    assert_eq!(ws.read(&bob(), "frame1", None).unwrap_err().code.as_str(), "NOT_FOUND");
    drop(ws);
    // and a closed room takes nobody new
    let at2 = invite(&env, &s, "at");
    let mut s2 = fresh(&env, &s);
    let pa = presenter(&env, &s2);
    env.svc.session_set_access(&mut s2, &pa, &json!({ "access": "closed" })).unwrap();
    assert_eq!(env.svc.session_join(&s2, &Caller::user("carol"), Some(&at2), None).unwrap_err().sub, Some("SESSION_CLOSED"));
}

#[test]
fn the_stage_is_the_services_and_every_step_is_on_the_timeline() {
    let env = env();
    let src = courseware(&env);
    let mut s = session(&env, &src, json!({}));
    let p = presenter(&env, &s);
    assert_eq!(env.svc.session_command(&s, &p, &json!({ "command_id": "c0", "kind": "next" })).unwrap_err().sub, Some("SESSION_NOT_LIVE"));
    env.svc.session_start(&mut s, &p).unwrap();
    env.advance(1500);
    let r = env.svc.session_command(&s, &p, &json!({ "command_id": "c1", "kind": "next" })).unwrap();
    assert_eq!(r["stage"]["index"], 1);
    assert_eq!(r["event"]["t_rel_ms"], 1500);
    // a resend is the same command
    let again = env.svc.session_command(&s, &p, &json!({ "command_id": "c1", "kind": "next" })).unwrap();
    assert_eq!(again["duplicate"], true);
    assert_eq!(again["stage"]["index"], 1);
    env.svc.session_command(&s, &p, &json!({ "command_id": "c2", "kind": "black" })).unwrap();
    let last = env.svc.session_command(&s, &p, &json!({ "command_id": "c3", "kind": "last" })).unwrap();
    assert_eq!(last["stage"]["black"], true);
    assert_eq!(last["stage"]["index"].as_u64().unwrap() + 1, last["stage"]["count"].as_u64().unwrap());
    // an audience member cannot drive it
    let at = invite(&env, &s, "at");
    let b = join(&env, &s, &bob(), &at);
    assert_eq!(env.svc.session_command(&s, &b, &json!({ "command_id": "x", "kind": "next" })).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    // the timeline: start, three stage changes (no duplicate), all public
    let h = env.svc.workspace(&s.workspace_id).unwrap();
    let ws = h.lock().unwrap();
    let (events, _) = ws.events_after(&b.access(), 0, 100, None).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "stage").count(), 3, "{kinds:?}");
    assert!(kinds.contains(&"session.start"));
    let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn tasks_claims_and_answers_go_through_the_session() {
    let env = env();
    let src = courseware(&env);
    // the courseware binds the feeds at design time (Q20)
    let feeds: Vec<String> = {
        let h = env.svc.workspace(&src).unwrap();
        let mut ws = h.lock().unwrap();
        ["tasks", "my_responses", "responses", "participants", "results"].iter().map(|f| feed_sample(&mut ws, &alice(), f, false).unwrap()["entity_id"].as_str().unwrap().to_string()).collect()
    };
    let mut s = session(&env, &src, json!({}));
    let p = presenter(&env, &s);
    let at = invite(&env, &s, "at");
    let (b, c, d) = (join(&env, &s, &bob(), &at), join(&env, &s, &Caller::user("carol"), &at), join(&env, &s, &Caller::user("dave"), &at));
    let g = join(&env, &s, &guest(), &at);
    env.svc.session_start(&mut s, &p).unwrap();
    // an open task: everyone but the guest (this room keeps guests watching) answers once per version
    let t = env.svc.session_task(&s, &p, &json!({ "op": "create", "command_id": "k1", "task": { "title": "Q1", "mode": "open" } })).unwrap();
    let t1 = t["task_id"].as_str().unwrap().to_string();
    env.svc.session_respond(&s, &b, &json!({ "task_id": t1, "content": "A", "command_id": "r1" })).unwrap();
    let dup = env.svc.session_respond(&s, &b, &json!({ "task_id": t1, "content": "A", "command_id": "r1" })).unwrap();
    assert_eq!(dup["duplicate"], true);
    env.svc.session_respond(&s, &b, &json!({ "task_id": t1, "content": "B", "command_id": "r2" })).unwrap();
    assert_eq!(env.svc.session_respond(&s, &g, &json!({ "task_id": t1, "content": "A", "command_id": "g1" })).unwrap_err().sub, Some("GUEST_NOT_ALLOWED"));
    // nobody but the session writes a response
    {
        let h = env.svc.workspace(&s.workspace_id).unwrap();
        let mut ws = h.lock().unwrap();
        let rid = aiworkspace_store::session::feeds::response_id(&t1, "bob");
        let r = commit(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": rid, "keys": [{ "key": "content", "value": "Z" }] }]));
        assert_eq!(r["sub_code"], "SESSION_ONLY", "{r}");
        // bob reads his own answer; carol does not
        assert_eq!(ws.read(&bob(), &rid, None).unwrap()["content"]["payload"]["content"], "B");
        assert_eq!(ws.read(&Caller::user("carol"), &rid, None).unwrap_err().code.as_str(), "NOT_FOUND");
    }
    // a claim task with two slots: the third claimant is refused, claims are idempotent
    let k = env.svc.session_task(&s, &p, &json!({ "op": "create", "command_id": "k2", "task": { "title": "抢答", "mode": "claim", "slots": 2 } })).unwrap();
    let t2 = k["task_id"].as_str().unwrap().to_string();
    env.svc.session_claim(&s, &b, &json!({ "task_id": t2 })).unwrap();
    assert_eq!(env.svc.session_claim(&s, &b, &json!({ "task_id": t2 })).unwrap()["duplicate"], true);
    env.svc.session_claim(&s, &c, &json!({ "task_id": t2 })).unwrap();
    assert_eq!(env.svc.session_claim(&s, &d, &json!({ "task_id": t2 })).unwrap_err().sub, Some("CLAIM_FULL"));
    assert_eq!(env.svc.session_respond(&s, &d, &json!({ "task_id": t2, "content": "x", "command_id": "d1" })).unwrap_err().sub, Some("NOT_CLAIMED"));
    // a pick: only the assigned answers; revoked, they cannot; the task closes, late answers are refused and recorded
    let a = env.svc.session_task(&s, &p, &json!({ "op": "create", "command_id": "k3", "principals": ["carol"],
                                                    "task": { "title": "点名", "mode": "assigned", "pick": { "seed": 7, "pool": ["bob", "carol", "dave"] } } })).unwrap();
    let t3 = a["task_id"].as_str().unwrap().to_string();
    assert_eq!(env.svc.session_respond(&s, &b, &json!({ "task_id": t3, "content": "x", "command_id": "b3" })).unwrap_err().sub, Some("NOT_ASSIGNED"));
    env.svc.session_task(&s, &p, &json!({ "op": "revoke", "task_id": t3, "principals": ["carol"], "command_id": "k4" })).unwrap();
    assert_eq!(env.svc.session_respond(&s, &c, &json!({ "task_id": t3, "content": "x", "command_id": "c3" })).unwrap_err().sub, Some("ANSWER_REVOKED"));
    env.svc.session_task(&s, &p, &json!({ "op": "assign", "task_id": t3, "principals": ["dave"], "command_id": "k5" })).unwrap();
    env.svc.session_respond(&s, &d, &json!({ "task_id": t3, "content": "C", "command_id": "d3" })).unwrap();
    env.svc.session_task(&s, &p, &json!({ "op": "close", "task_id": t1, "command_id": "k6" })).unwrap();
    assert_eq!(env.svc.session_respond(&s, &c, &json!({ "task_id": t1, "content": "late", "command_id": "c9" })).unwrap_err().sub, Some("TASK_CLOSED"));
    env.svc.session_publish(&s, &p, &json!({ "task_id": t1, "title": "答案", "content": { "answer": "B" }, "command_id": "pub1" })).unwrap();
    // the feeds: tasks public with counts, answers presenter-only, each participant's own answers in its own table
    let h = env.svc.workspace(&s.workspace_id).unwrap();
    let ws = h.lock().unwrap();
    let rows = |who: &Caller, id: &str| -> Vec<Value> {
        let q = ws.query(who, &json!({ "source_id": id, "limit": 100 }), &env.svc.sources).unwrap();
        q["rows"].as_array().cloned().unwrap_or_default()
    };
    let tasks = rows(&bob(), &feeds[0]);
    assert_eq!(tasks.len(), 3, "{tasks:?}");
    let q1 = tasks.iter().find(|r| r["values"]["task_id"] == json!(t1)).unwrap();
    assert_eq!(q1["values"]["answered"], 1);
    assert_eq!(q1["values"]["state"], "closed");
    let mine = feeds_of(&ws, &bob(), "my_responses");
    assert_eq!(mine.len(), 1);
    let my = rows(&bob(), &mine[0]);
    assert!(my.iter().any(|r| r["values"]["task_id"] == json!(t1) && r["values"]["content"] == json!("\"B\"")), "{my:?}");
    assert!(feeds_of(&ws, &Caller::user("carol"), "my_responses").iter().all(|id| *id != mine[0]));
    assert_eq!(ws.read(&bob(), &feeds[2], None).unwrap_err().code.as_str(), "NOT_FOUND");
    let all = rows(&alice(), &feeds[2]);
    assert!(all.len() >= 3);
    let results = rows(&bob(), &feeds[4]);
    assert_eq!(results.len(), 1);
    // the timeline: the late answer is on carol's track, which bob does not see
    let late = |access: &Access| ws.events_after(access, 0, 1000, None).unwrap().0.iter().filter(|e| e["type"] == json!("task.late")).count();
    assert_eq!(late(&c.access()), 1);
    assert_eq!(late(&b.access()), 0);
    assert_eq!(late(&p.access()), 1);
    // every Commit is on the timeline exactly once (the event is written in the Commit's transaction)
    let (events, _) = ws.events_after(&p.access(), 0, 10_000, None).unwrap();
    let commits: Vec<u64> = events.iter().filter(|e| e["type"] == json!("commit")).map(|e| e["commit_seq"].as_u64().unwrap()).collect();
    let mut sorted = commits.clone();
    sorted.dedup();
    assert_eq!(commits, sorted);
    assert_eq!(*commits.last().unwrap(), ws.head_seq);
    assert_eq!(commits.len() as u64, ws.head_seq - s.baseline["source"]["seq"].as_u64().unwrap());
}

fn feeds_of(ws: &aiworkspace_store::Workspace, who: &Caller, feed: &str) -> Vec<String> {
    ws.feed_entities()
        .unwrap()
        .into_iter()
        .filter(|(id, f, _)| f == feed && ws.read(who, id, None).is_ok())
        .filter(|(_, _, p)| p.is_some())
        .map(|(id, ..)| id)
        .collect()
}

#[test]
fn guests_answer_only_where_the_room_lets_them_and_say_so() {
    let env = env();
    let src = courseware(&env);
    let mut s = session(&env, &src, json!({ "guest_interact": true }));
    let p = presenter(&env, &s);
    let at = invite(&env, &s, "at");
    let g = join(&env, &s, &guest(), &at);
    env.svc.session_start(&mut s, &p).unwrap();
    let t = env.svc.session_task(&s, &p, &json!({ "op": "create", "command_id": "k1", "task": { "mode": "open" } })).unwrap();
    let t1 = t["task_id"].as_str().unwrap();
    env.svc.session_respond(&s, &g, &json!({ "task_id": t1, "content": 1, "command_id": "g1" })).unwrap();
    let h = env.svc.workspace(&s.workspace_id).unwrap();
    let ws = h.lock().unwrap();
    let rid = aiworkspace_store::session::feeds::response_id(t1, &g.principal);
    assert_eq!(ws.read(&alice(), &rid, None).unwrap()["content"]["payload"]["level"], "guest");
}

#[test]
fn ending_saving_and_restarting_keep_what_was_confirmed() {
    let env = env();
    let src = courseware(&env);
    let mut s = session(&env, &src, json!({}));
    let p = presenter(&env, &s);
    let at = invite(&env, &s, "at");
    let b = join(&env, &s, &bob(), &at);
    env.svc.session_start(&mut s, &p).unwrap();
    env.svc.session_command(&s, &p, &json!({ "command_id": "c1", "kind": "next" })).unwrap();
    // a presenter who is not the owner cannot save; the owner can, after the end
    let other = env.svc.session_create(&alice(), &json!({ "workspace_id": src, "path_id": "path-intro", "presenter": "bob" })).unwrap();
    let mut o = env.svc.sessions.require(other["session"]["session_id"].as_str().unwrap()).unwrap();
    let bob_presents = env.svc.require_actor(&o, &bob(), None).unwrap();
    assert_eq!(bob_presents.role, Role::Presenter);
    env.svc.session_start(&mut o, &bob_presents).unwrap();
    env.svc.session_end(&mut o, &bob_presents, 1000).unwrap();
    assert_eq!(env.svc.session_save(&mut o, &bob_presents).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    let owner = env.svc.require_actor(&o, &alice(), None).unwrap();
    assert_eq!(owner.role, Role::Owner);
    // two sessions of one courseware are independent (P4-01)
    assert_ne!(o.workspace_id, s.workspace_id);
    // the restart (P4-13): the stage, the events and the participants are on disk
    let env = env.reopen();
    let s = fresh(&env, &s);
    assert_eq!(s.state, "live");
    let h = env.svc.workspace(&s.workspace_id).unwrap();
    let stage = h.lock().unwrap().stage();
    assert_eq!(stage["index"], 1);
    assert!(!env.svc.sessions.participant(&s.session_id, "bob").unwrap().unwrap().online);
    let b = env.svc.require_actor(&s, &bob(), None).unwrap_or(b);
    assert_eq!(b.role, Role::Participant);
    // saved: archived with its manifest; the other, never saved, expires and keeps only its row
    let mut s = s;
    let p = env.svc.require_actor(&s, &alice(), None).unwrap();
    env.advance(5_000);
    env.svc.session_end(&mut s, &p, 1000).unwrap();
    // the session's time stops at the end: someone opening the replay later is put there, the length stays
    env.advance(600_000);
    let (end, late) = env
        .svc
        .with_run(&s, |ws| {
            let (late, _) = ws.append_event(NewEvent { actor: "bob", scope: "presenter", track: "diag", kind: "presence.join", command_id: None, data: json!({}), t_rel_ms: None })?;
            let (events, _) = ws.events_after(&p.access(), 0, 2000, None)?;
            let end = events.iter().find(|e| e["type"] == "session.end").unwrap()["t_rel_ms"].as_i64().unwrap();
            Ok((end, late["t_rel_ms"].as_i64().unwrap()))
        })
        .unwrap();
    assert!(end >= 5_000, "{end}");
    assert_eq!(late, end);
    let saved = env.svc.session_save(&mut s, &p).unwrap();
    assert_eq!(saved["archive"]["duration_ms"], end);
    assert_eq!(saved["state"], "archived");
    assert_eq!(saved["archive"]["format"], "buckyos.ai-workspace/session-archive/1");
    assert!(saved["archive"]["events"]["last"].as_u64().unwrap() > 2);
    let mut o = fresh(&env, &o);
    env.svc.session_expire(&mut o).unwrap();
    let o = env.svc.sessions.get(&o.session_id).unwrap().unwrap();
    assert_eq!(o.state, "expired");
    assert!(env.svc.with_run(&o, |_| Ok(())).is_err());
    // the archived one is still readable
    assert!(env.svc.with_run(&fresh(&env, &s), |ws| Ok(ws.stage())).is_ok());
}
