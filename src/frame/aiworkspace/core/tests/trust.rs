//! An HTML definition's trust (权限需求 R37, Q32): code a run writes (origin `program`) needs no `manage`, but it is
//! trusted only when the run's identity manages the definition; a manager vouches later. Replaying the normalized
//! history reaches the same trust without judging anybody's permissions.

use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::model::*;
use aiworkspace_core::plan::{plan_commit, PlanEnv};
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn editor() -> Access {
    let caps = [Cap::Read, Cap::Comment, Cap::Append, Cap::Update, Cap::Delete, Cap::Structure].into_iter().fold(CapSet::NONE, |s, c| s.union(CapSet::of(c)));
    Access::granted("carol", caps, [])
}

fn page(id: &str, html: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_BLOCK_DEF, "parent_id": DATA_ID, "order_key": "p", "name": id,
            "payload": { "def_id": "acme.page", "version": 1, "kind": "html", "title": "看板", "html": { "html": html, "css": "", "js": "", "api_version": 2 } } })
}

fn new_code(ws: &MemWorkspace, id: &str, html: &str) -> Value {
    let rev = ws.store.entities[id].key_revs["html"];
    json!({ "op": "entity.set_keys", "entity_id": id, "keys": [{ "key": "html", "value": { "html": html, "css": "", "js": "", "api_version": 2 }, "expect": { "rev": rev } }] })
}

/// Commit `op` as `who` with `origin`; the writer and origin are kept for the replay.
fn commit(ws: &mut MemWorkspace, log: &mut Vec<(String, String)>, who: &str, access: &Access, origin: &str, op: Value) -> Result<u64, Value> {
    let env = ws.env(who, origin, (origin == "program").then_some("run_x"), false);
    let r = ws.commit_with(access, &env, &MemWorkspace::request(json!([op]))).map_err(|f| f.to_json());
    if r.is_ok() {
        log.push((who.to_string(), origin.to_string()));
    }
    r
}

fn trusted_by(ws: &MemWorkspace, id: &str) -> Option<String> {
    ws.store.entities[id].payload.get("trusted").and_then(|t| t["by"].as_str()).map(str::to_string)
}

#[test]
fn a_runs_page_is_trusted_only_as_its_identity_may_vouch_and_a_replay_agrees() {
    let mut ws = MemWorkspace::new();
    let mut log = Vec::new();
    let owner = Access::full("alice");
    // an editor's run writes its page: written, not trusted
    commit(&mut ws, &mut log, "carol", &editor(), "program", page("pg", "<div id=a></div>")).unwrap();
    assert_eq!(trusted_by(&ws, "pg"), None);
    // by hand, the editor may not write code at all
    let op = new_code(&ws, "pg", "<div id=x></div>");
    let refused = commit(&mut ws, &mut log, "carol", &editor(), "human", op).unwrap_err();
    assert_eq!(refused["code"], "PERMISSION_DENIED", "{refused}");
    // the owner's run: trusted as it is written; history carries the vouch
    let op = new_code(&ws, "pg", "<div id=b></div>");
    commit(&mut ws, &mut log, "alice", &owner, "program", op).unwrap();
    assert_eq!(trusted_by(&ws, "pg").as_deref(), Some("alice"));
    let keys: Vec<&str> = ws.history.last().unwrap()[0].op["keys"].as_array().unwrap().iter().filter_map(|k| k["key"].as_str()).collect();
    assert!(keys.contains(&"trusted"), "{keys:?}");
    // the editor's next page drops the trust; a manager vouches for it by hand
    let op = new_code(&ws, "pg", "<div id=c></div>");
    commit(&mut ws, &mut log, "carol", &editor(), "program", op).unwrap();
    assert_eq!(trusted_by(&ws, "pg"), None);
    let rev = ws.store.entities["pg"].key_revs.get("trusted").copied().unwrap_or(0);
    let vouch = json!({ "op": "entity.set_keys", "entity_id": "pg", "keys": [{ "key": "trusted", "value": true, "expect": { "rev": rev } }] });
    assert_eq!(commit(&mut ws, &mut log, "carol", &editor(), "human", vouch.clone()).unwrap_err()["code"], "PERMISSION_DENIED");
    commit(&mut ws, &mut log, "alice", &owner, "human", vouch).unwrap();
    assert_eq!(trusted_by(&ws, "pg").as_deref(), Some("alice"));
    let op = new_code(&ws, "pg", "<div id=d></div>");
    commit(&mut ws, &mut log, "carol", &editor(), "program", op).unwrap();
    // the owner's run creates a page: trusted from the start
    commit(&mut ws, &mut log, "alice", &owner, "program", page("pg2", "<div id=e></div>")).unwrap();
    assert_eq!(trusted_by(&ws, "pg2").as_deref(), Some("alice"));

    // replaying the normalized history, with nobody's permissions, reaches the same pages
    let mut replica = MemWorkspace::new();
    let history = ws.history.clone();
    for (ops, (who, origin)) in history.iter().zip(&log) {
        let req = MemWorkspace::request(Value::Array(ops.iter().map(|o| o.op.clone()).collect()));
        let env = PlanEnv { replay: true, internal: true, import: true, schema_checks: false, ..replica.env(who, origin, (origin == "program").then_some("run_x"), false) };
        let access = Access::full(who);
        let plan = plan_commit(&replica.store, &req, &env, &access).map_err(|f| f.to_json()).unwrap();
        replica.store.apply(plan.overlay.into_changes());
        replica.head_seq = env.seq;
    }
    for id in ["pg", "pg2"] {
        assert_eq!(replica.store.entities[id].payload, ws.store.entities[id].payload, "{id}");
    }
}
