//! A Commit sent in parts (`doc.stage` + `doc.commit { staged }`): one transaction, one undo unit, one event.

mod common;

use aiworkspace_core::model::MemStore;
use aiworkspace_core::replica::{load_rows, Replica};
use aiworkspace_store::workspace::{Caller, CommitOpts, Workspace};
use common::*;
use serde_json::{json, Value};

fn rec(i: usize, text: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": format!("r-{i}"), "type_id": "buckyos.record", "parent_id": "data", "order_key": format!("k{i:06}1"),
            "payload": { "schema": { "properties": [{ "key": "t", "name": "t", "type": "text" }] }, "props": { "t": text } } })
}

fn recs(range: std::ops::Range<usize>, text: &str) -> Vec<Value> {
    range.map(|i| rec(i, text)).collect()
}

fn stage(ws: &mut Workspace, key: &str, part: u64, ops: &[Value]) -> String {
    let r = try_stage(ws, &alice(), key, part, ops).unwrap();
    assert_eq!(r["operations"], ops.len());
    r["digest"].as_str().unwrap().to_string()
}

fn final_request(ws: &Workspace, key: &str, staged: &[String], ops: &[Value]) -> Value {
    json!({ "protocol_version": "1.0", "workspace_id": ws.workspace_id, "epoch": ws.epoch, "idempotency_key": key, "session_id": "s1",
            "staged": staged, "operations": ops })
}

fn finish(ws: &mut Workspace, who: &Caller, key: &str, staged: &[String], ops: &[Value]) -> Value {
    let req = final_request(ws, key, staged, ops);
    ws.commit(&req, who, &CommitOpts::default())
}

fn try_stage(ws: &mut Workspace, who: &Caller, key: &str, part: u64, ops: &[Value]) -> aiworkspace_core::WsResult<Value> {
    let req = json!({ "workspace_id": ws.workspace_id, "epoch": ws.epoch, "idempotency_key": key, "part": part, "operations": ops });
    ws.stage_commit(who, &req)
}

fn staged_usage(ws: &Workspace) -> u64 {
    ws.usage().unwrap()["staged"].as_u64().unwrap_or(0)
}

fn workspace(env: &Env) -> String {
    env.svc.create_workspace(&alice(), "分块", None).unwrap()["workspace_id"].as_str().unwrap().to_string()
}

#[test]
fn parts_assemble_into_one_commit_and_replay_after_they_are_gone() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let head = ws.head_seq;
    let d0 = stage(&mut ws, "imp/1", 0, &recs(0..3, "a"));
    // the same part again replaces it: a retry is safe
    assert_eq!(stage(&mut ws, "imp/1", 0, &recs(0..3, "a")), d0);
    let d1 = stage(&mut ws, "imp/1", 1, &recs(3..5, "a"));
    assert!(staged_usage(&ws) > 0);
    let req = final_request(&ws, "imp/1", &[d0.clone(), d1.clone()], &recs(5..6, "a"));
    // prepare assembles without consuming
    let dry = ws.prepare(&req, &alice());
    assert_eq!(dry["status"], "ok", "{dry}");
    assert!(staged_usage(&ws) > 0);
    let r = ws.commit(&req, &alice(), &CommitOpts::default());
    assert_eq!((r["status"].as_str(), r["seq"].as_u64()), (Some("accepted"), Some(head + 1)), "{r}");
    assert_eq!(staged_usage(&ws), 0, "the parts are consumed");
    for i in 0..6 {
        assert!(ws.read(&alice(), &format!("r-{i}"), None).is_ok());
    }
    // one event carries every operation, in order
    let changes = ws.get_changes(&alice(), &ws.epoch.clone(), head, 10, true).unwrap();
    let events = changes["changes"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    let ids: Vec<&str> = events[0]["ops"].as_array().unwrap().iter().map(|o| o["entity_id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["r-0", "r-1", "r-2", "r-3", "r-4", "r-5"]);
    // the same final request replays although its parts are gone
    let again = ws.commit(&req, &alice(), &CommitOpts::default());
    assert_eq!((again["replayed"].as_bool(), again["commit_id"].as_str()), (Some(true), r["commit_id"].as_str()), "{again}");
    // the same key naming other parts is another request
    let other = finish(&mut ws, &alice(), "imp/1", &[d1, d0], &recs(5..6, "a"));
    assert_eq!(code(&other), "IDEMPOTENCY_MISMATCH", "{other}");
}

#[test]
fn missing_expired_and_mismatched_parts_are_refused() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let d0 = stage(&mut ws, "k/1", 0, &recs(0..2, "a"));
    // part 1 was never staged
    let r = finish(&mut ws, &alice(), "k/1", &[d0.clone(), d0.clone()], &recs(4..5, "a"));
    assert_eq!((code(&r), r["sub_code"].as_str(), r["retryable"].as_bool()), ("SNAPSHOT_EXPIRED", Some("STAGED_PARTS_MISSING"), Some(true)), "{r}");
    assert_eq!(staged_usage(&ws), 0, "a rejected final commit consumes its parts too");
    // an expired part is a missing part
    let d0 = stage(&mut ws, "k/2", 0, &recs(0..2, "a"));
    env.advance(3_600_001);
    let r = finish(&mut ws, &alice(), "k/2", &[d0], &recs(4..5, "a"));
    assert_eq!(r["sub_code"], "STAGED_PARTS_MISSING", "{r}");
    // the request names a digest the staged part does not have
    let d0 = stage(&mut ws, "k/3", 0, &recs(0..2, "a"));
    stage(&mut ws, "k/3", 0, &recs(2..4, "a"));
    let r = finish(&mut ws, &alice(), "k/3", &[d0], &recs(4..5, "a"));
    assert_eq!(code(&r), "IDEMPOTENCY_MISMATCH", "{r}");
    // parts belong to who staged them
    let d0 = stage(&mut ws, "k/4", 0, &recs(0..2, "a"));
    ws.grant(&alice(), "bob", None, &["read".into(), "update".into(), "structure".into()]).unwrap();
    let r = finish(&mut ws, &bob(), "k/4", &[d0], &recs(4..5, "a"));
    assert_eq!(r["sub_code"], "STAGED_PARTS_MISSING", "{r}");
    assert!(ws.read(&alice(), "r-0", None).is_err(), "nothing was written");
    // someone without access cannot stage
    let e = try_stage(&mut ws, &Caller::user("carol"), "c/1", 0, &recs(0..1, "a")).unwrap_err();
    assert_eq!(e.code.as_str(), "NOT_FOUND");
}

#[test]
fn conflict_consumes_and_limits_apply_to_the_assembled_commit() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ok(&mut ws, &alice(), json!(recs(0..1, "a")));
    // r-0 exists: the assembled Commit is refused as a whole and leaves nothing behind
    let d0 = stage(&mut ws, "c/1", 0, &recs(1..3, "a"));
    let r = finish(&mut ws, &alice(), "c/1", &[d0], &recs(0..1, "a"));
    assert_ne!(r["status"], "accepted", "{r}");
    assert!(ws.read(&alice(), "r-1", None).is_err());
    assert_eq!(staged_usage(&ws), 0);
    // operations: counted over all parts
    ws.limits.max_ops = 4;
    let d0 = stage(&mut ws, "c/2", 0, &recs(1..4, "a"));
    let r = finish(&mut ws, &alice(), "c/2", &[d0], &recs(4..6, "a"));
    assert_eq!(code(&r), "LIMIT_EXCEEDED", "{r}");
    // bytes: the parts of one key may not pass the total, and neither may the assembled request
    ws.limits.max_ops = 50_000;
    ws.limits.max_commit_total_bytes = 4_000;
    let big = "y".repeat(1_500);
    stage(&mut ws, "c/3", 0, &recs(1..3, &big));
    let e = try_stage(&mut ws, &alice(), "c/3", 1, &recs(3..5, &big)).unwrap_err();
    assert_eq!(e.code.as_str(), "LIMIT_EXCEEDED");
    let d0 = stage(&mut ws, "c/4", 0, &recs(1..3, &big));
    let r = finish(&mut ws, &alice(), "c/4", &[d0], &recs(3..4, &big));
    assert_eq!(code(&r), "LIMIT_EXCEEDED", "{r}");
}

#[test]
fn sweep_and_clear_staged_release_the_parts() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    {
        let mut ws = h.lock().unwrap();
        stage(&mut ws, "s/1", 0, &recs(0..2, "a"));
        let held = staged_usage(&ws);
        assert!(held > 0);
        let cleared = ws.clear_staged(&alice()).unwrap();
        assert_eq!((cleared["commit_parts"].as_u64(), cleared["commit_part_bytes"].as_u64()), (Some(1), Some(held)), "{cleared}");
        assert_eq!(staged_usage(&ws), 0);
        stage(&mut ws, "s/2", 0, &recs(0..2, "a"));
    }
    env.svc.sweep().unwrap();
    assert!(staged_usage(&h.lock().unwrap()) > 0, "a fresh part stays");
    env.advance(3_600_001);
    env.svc.sweep().unwrap();
    assert_eq!(staged_usage(&h.lock().unwrap()), 0, "an hour later it is gone");
}

#[test]
fn a_commit_over_one_request_undoes_redoes_and_reaches_a_replica() {
    let env = env();
    let id = workspace(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let b = ws.replica_bootstrap(&alice()).unwrap();
    let conn = rusqlite::Connection::open(fetched(&ws.objects, &b)).unwrap();
    let mut tables = serde_json::Map::new();
    for table in ["entities", "tree_edges", "table_fields", "table_records", "richtext_states", "refs", "assets"] {
        let mut st = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let rows: Vec<Value> = st
            .query_map([], |r| {
                let mut o = serde_json::Map::new();
                for (i, n) in names.iter().enumerate() {
                    let v = match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(x) => json!(x),
                        rusqlite::types::ValueRef::Real(x) => json!(x),
                        rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                        rusqlite::types::ValueRef::Blob(_) => Value::Null,
                    };
                    o.insert(n.clone(), v);
                }
                Ok(Value::Object(o))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        tables.insert(table.to_string(), Value::Array(rows));
    }
    let confirmed: MemStore = load_rows(&Value::Object(tables)).unwrap();
    let mut rep = Replica::new("alice", &ws.workspace_id, &ws.epoch, confirmed, b["head_seq"].as_u64().unwrap(), 7);

    // 3 parts of ~3.3 MiB each: 10 MiB in all, more than one request may carry
    let text = "汉".repeat(1_100);
    let (per, total) = (1_000usize, 3_000usize);
    let head = ws.head_seq;
    let d0 = stage(&mut ws, "big/1", 0, &recs(0..per, &text));
    let d1 = stage(&mut ws, "big/1", 1, &recs(per..2 * per, &text));
    let last = recs(2 * per..total, &text);
    let request_bytes = json!(recs(0..total, &text)).to_string().len();
    assert!(request_bytes > 8 * 1024 * 1024, "{request_bytes}");
    let r = finish(&mut ws, &alice(), "big/1", &[d0, d1], &last);
    assert_eq!(r["status"], "accepted", "{r}");
    assert_eq!(ws.head_seq, head + 1);
    let commit_id = r["commit_id"].as_str().unwrap().to_string();

    // the replica applies the one large event
    let changes = ws.get_changes(&alice(), &rep.epoch.clone(), rep.confirmed_seq, 500, true).unwrap();
    rep.apply_remote(changes["changes"].as_array().unwrap()).unwrap();
    assert_eq!(rep.confirmed_seq, ws.head_seq);
    assert!(rep.confirmed.entities.contains_key("r-2999"));
    // the replica refuses locally what the backend would refuse for its size
    let key = "rep/1";
    let huge = json!({ "protocol_version": "1.0", "workspace_id": rep.workspace_id, "epoch": rep.epoch, "idempotency_key": key, "operations": recs(5_000..5_002, "a") });
    rep.limits.max_commit_total_bytes = 100;
    assert_eq!(code(&rep.submit_local(&huge)), "LIMIT_EXCEEDED");

    // undo: one compensating Commit over 8 MiB; redo: the undo of that one
    let epoch = ws.epoch.clone();
    let undone = ws.undo(&alice(), &json!({ "epoch": epoch, "commit_id": commit_id, "idempotency_key": "undo/1" }));
    assert_eq!(undone["status"], "accepted", "{undone}");
    assert!(ws.read(&alice(), "r-0", None).is_err());
    assert!(ws.read(&alice(), "r-2999", None).is_err());
    let redone = ws.undo(&alice(), &json!({ "epoch": epoch, "commit_id": undone["commit_id"], "idempotency_key": "redo/1" }));
    assert_eq!(redone["status"], "accepted", "{redone}");
    assert!(ws.read(&alice(), "r-0", None).is_ok());
    assert!(ws.read(&alice(), "r-2999", None).is_ok());
    assert_eq!(ws.head_seq, head + 3);

    // a Commit at the limit stays undoable and redoable: its compensating Commits get twice the room
    ws.limits.max_ops = 10;
    ok(&mut ws, &alice(), json!(recs(9_000..9_010, "a")));
    let at_limit = ws.get_changes(&alice(), &epoch, head + 3, 10, false).unwrap()["changes"][0]["commit_id"].clone();
    let undone = ws.undo(&alice(), &json!({ "epoch": epoch, "commit_id": at_limit, "idempotency_key": "undo/2" }));
    assert_eq!(undone["status"], "accepted", "{undone}");
    let redone = ws.undo(&alice(), &json!({ "epoch": epoch, "commit_id": undone["commit_id"], "idempotency_key": "redo/2" }));
    assert_eq!(redone["status"], "accepted", "{redone}");
    assert_eq!(code(&commit(&mut ws, &alice(), json!(recs(9_100..9_111, "a")))), "LIMIT_EXCEEDED");
}
