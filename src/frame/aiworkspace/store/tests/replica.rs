//! The offline replica engine against a real backend Workspace (kernel side of
//! V15 and V17): local edits while disconnected, rebase on remote changes,
//! unknown results, conflicts, deleted targets and revoked permission — with
//! every local input still recoverable.

mod common;
use aiworkspace_core::materialize::{materialize, ObjectSink};
use aiworkspace_core::model::{EntityRow, MemStore};
use aiworkspace_core::replica::{dump_rows, load_rows, Replica};
use aiworkspace_core::WsResult;
use aiworkspace_store::workspace::{Caller, CommitOpts, Workspace};
use base64::Engine;
use common::*;
use serde_json::{json, Map, Value};

/// Read the replica database the way the browser Worker does: every allowed table as rows.
fn replica_tables(path: &str) -> Value {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut out = Map::new();
    for table in ["entities", "tree_edges", "table_fields", "table_records", "richtext_states", "refs", "assets"] {
        let mut st = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let rows: Vec<Value> = st
            .query_map([], |r| {
                let mut o = Map::new();
                for (i, n) in names.iter().enumerate() {
                    let v = match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(x) => json!(x),
                        rusqlite::types::ValueRef::Real(x) => json!(x),
                        rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                        rusqlite::types::ValueRef::Blob(b) => json!(base64::engine::general_purpose::STANDARD.encode(b)),
                    };
                    o.insert(n.clone(), v);
                }
                Ok(Value::Object(o))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        out.insert(table.to_string(), Value::Array(rows));
    }
    Value::Object(out)
}

/// Computes ids without storing anything: enough to compare content roots.
struct IdSink;
impl ObjectSink for IdSink {
    fn put_object(&mut self, obj_type: &str, canonical: &str) -> WsResult<String> {
        Ok(aiworkspace_core::canonical::build_obj_id(obj_type, canonical))
    }
    fn put_file(&mut self, bytes: &[u8]) -> WsResult<String> {
        let chunk = aiworkspace_core::canonical::chunk_id(bytes);
        Ok(aiworkspace_core::canonical::file_object(bytes.len() as u64, &chunk)?.0)
    }
}

fn root_of(m: &MemStore) -> String {
    materialize(m, &mut IdSink, &|_: &EntityRow| Ok(true), &mut |_, _| None).unwrap().content_root
}

fn bootstrap(ws: &mut Workspace, who: &Caller, peer: u64) -> Replica {
    let b = ws.replica_bootstrap(who).unwrap();
    let confirmed = load_rows(&replica_tables(b["path"].as_str().unwrap())).unwrap();
    Replica::new(&who.principal, &ws.workspace_id, &ws.epoch, confirmed, b["head_seq"].as_u64().unwrap(), peer)
}

fn local(rep: &Replica, key: &str, ops: Value) -> Value {
    json!({ "protocol_version": "0.1", "workspace_id": rep.workspace_id, "epoch": rep.epoch, "idempotency_key": key, "session_id": "s1", "operations": ops })
}

fn working_cell(rep: &Replica, record: &str, field: &str) -> (Option<Value>, u64) {
    let r = &rep.working.records[&("tasks".to_string(), record.to_string())];
    (r.values.get(field).cloned(), r.value_rev(field))
}

/// What the browser Worker does with `take_confirmed_delta`: upsert by primary key, delete references.
fn apply_delta(disk: &mut Value, delta: &Value) {
    let keys: &[(&str, &[&str])] = &[
        ("entities", &["entity_id"]),
        ("tree_edges", &["child_id"]),
        ("table_fields", &["source_id", "field_id"]),
        ("table_records", &["source_id", "record_id"]),
        ("richtext_states", &["entity_id"]),
        ("refs", &["src_entity_id", "src_selector", "kind", "dst_workspace_id", "dst_entity_id", "dst_object_id", "dst_query_json"]),
        ("assets", &["object_id"]),
    ];
    let pk = |row: &Value, cols: &[&str]| cols.iter().map(|c| row[*c].to_string()).collect::<Vec<_>>();
    for (table, cols) in keys {
        let rows = disk[*table].as_array_mut().unwrap();
        if *table == "refs" {
            for gone in delta["refs_deleted"].as_array().unwrap() {
                rows.retain(|r| pk(r, cols) != pk(gone, cols));
            }
        }
        for row in delta[*table].as_array().unwrap() {
            rows.retain(|r| pk(r, cols) != pk(row, cols));
            rows.push(row.clone());
        }
    }
}

/// Pull everything after the confirmed position and rebase.
fn catch_up(ws: &Workspace, who: &Caller, rep: &mut Replica) -> Value {
    let changes = ws.get_changes(who, &rep.epoch.clone(), rep.confirmed_seq, 500, true).unwrap();
    rep.apply_remote(changes["changes"].as_array().unwrap()).unwrap()
}

/// The send loop of §6.4: one queued submission at a time, settle through the change stream.
fn send_all(ws: &mut Workspace, who: &Caller, rep: &mut Replica) -> Vec<(String, String)> {
    let mut outcomes = Vec::new();
    while let Some(req) = rep.next_to_send().unwrap() {
        let key = req["idempotency_key"].as_str().unwrap().to_string();
        rep.mark(&key, "sending", None).unwrap();
        let r = ws.commit(&req, who, &CommitOpts::default());
        let status = r["status"].as_str().unwrap().to_string();
        if status == "accepted" {
            catch_up(ws, who, rep); // the only path that advances the confirmed layer
        } else {
            rep.mark(&key, &status, Some(r)).unwrap();
        }
        outcomes.push((key, status));
    }
    outcomes
}

#[test]
fn offline_edit_rebase_and_send() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "bob", None, &["read".into(), "update".into(), "delete".into(), "structure".into()]).unwrap();
    let mut rep = bootstrap(&mut ws, &alice(), 4242);
    assert_eq!(root_of(&rep.confirmed), ws.checkpoint_ro(), "the replica starts as an exact copy of the readable state");

    // ---- offline: alice edits; nothing reaches the backend ----
    let head = ws.head_seq;
    let (_, rev) = working_cell(&rep, "task-42", "owner");
    assert_eq!(rep.submit_local(&local(&rep, "a/1", set_cell("task-42", "owner", json!("离线改一"), rev)))["status"], "saved_locally");
    // a second edit of the same cell builds on the first one's provisional version
    let (v, prov) = working_cell(&rep, "task-42", "owner");
    assert_eq!((v, prov > head), (Some(json!("离线改一")), true));
    rep.submit_local(&local(&rep, "a/2", set_cell("task-42", "owner", json!("离线改二"), prov)));
    assert_eq!(rep.pending[1].request["operations"][0]["values"][0]["expect"], json!({ "after": "a/1" }), "stored as a dependency, not a provisional number");
    let (_, rb) = working_cell(&rep, "task-44", "budget");
    rep.submit_local(&local(&rep, "a/3", set_cell("task-44", "budget", json!("20"), rb)));
    let (_, r41) = working_cell(&rep, "task-41", "owner");
    rep.submit_local(&local(&rep, "a/4", set_cell("task-41", "owner", json!("给已删记录的修改"), r41)));
    rep.submit_local(&local(&rep, "a/5", json!([
        { "op": "entity.create", "entity_id": "offline-grp", "type_id": "buckyos.container", "parent_id": "page-main", "order_key": "n", "payload": { "kind": "group" } },
        { "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "after": "n-title" },
          "blocks": [{ "type": "paragraph", "attrs": { "block_id": "offline-p" }, "content": [{ "type": "text", "text": "离线写的段落" }] }] }])));
    // invalid local input is refused locally by the same rules and is not queued
    let bad = rep.submit_local(&local(&rep, "a/bad", set_cell("task-42", "status", json!("option-nope"), 2)));
    assert_eq!((bad["status"].as_str(), rep.pending.len()), (Some("rejected"), 5));
    assert_eq!(ws.head_seq, head);
    assert_eq!(root_of(&rep.confirmed), ws.checkpoint_ro(), "local edits never touch the confirmed layer");

    // ---- the replica is closed and reopened: confirmed rows + pending rows are all there is ----
    let saved_rows = dump_rows(&rep.confirmed).unwrap();
    let saved_pending = rep.pending_json();
    let (seq, epoch) = (rep.confirmed_seq, rep.epoch.clone());
    let working_before = root_of(&rep.working);
    drop(rep);
    let mut rep = Replica::new("alice", &id, &epoch, load_rows(&saved_rows).unwrap(), seq, 4243);
    rep.restore_pending(saved_pending.as_array().unwrap());
    assert_eq!(rep.pending.len(), 5);
    assert_eq!(working_cell(&rep, "task-42", "owner").0, Some(json!("离线改二")), "the working view is rebuilt from confirmed + pending");
    assert_eq!(root_of(&rep.working), working_before);

    // ---- meanwhile bob works online ----
    let rv = cell_rev(&ws, &bob(), "task-44", "budget");
    ok(&mut ws, &bob(), set_cell("task-44", "budget", json!("99"), rv)); // collides with a/3
    let rv = cell_rev(&ws, &bob(), "task-43", "owner");
    ok(&mut ws, &bob(), set_cell("task-43", "owner", json!("bob 改的"), rv)); // independent
    let rec = ws.read(&bob(), "tasks", Some(&json!({ "kind": "table_record", "record_id": "task-41" }))).unwrap()["content"]["rev"].clone();
    ok(&mut ws, &bob(), json!([{ "op": "table.delete_records", "source_id": "tasks", "records": [{ "record_id": "task-41", "expect": { "rev": rec } }] }]));

    // ---- reconnect: catch up first, then send ----
    let mut disk = saved_rows.clone();
    assert_eq!(rep.take_confirmed_delta().unwrap()["entities"], json!([]), "nothing to persist before a remote change");
    let r = catch_up(&ws, &alice(), &mut rep);
    let delta = rep.take_confirmed_delta().unwrap();
    assert!(delta["table_records"].as_array().unwrap().len() == 3 && delta["entities"].as_array().unwrap().len() < rep.confirmed.entities.len(), "only changed rows: {delta}");
    apply_delta(&mut disk, &delta);
    assert_eq!(root_of(&load_rows(&disk).unwrap()), ws.checkpoint_ro(), "rows on disk + delta == confirmed layer");
    assert_eq!(r["confirmed_seq"], json!(ws.head_seq));
    assert_eq!(root_of(&rep.confirmed), ws.checkpoint_ro(), "confirmed layer == backend state after replaying the change stream");
    let state = |rep: &Replica, key: &str| rep.pending.iter().find(|p| p.key == key).map(|p| p.state.clone());
    assert_eq!(state(&rep, "a/3").as_deref(), Some("conflict"), "bob changed the same cell");
    assert_eq!(state(&rep, "a/4").as_deref(), Some("conflict"), "its record was deleted meanwhile");
    assert_eq!(rep.pending.iter().find(|p| p.key == "a/4").unwrap().result.as_ref().unwrap()["code"], "TARGET_DELETED");
    assert_eq!(working_cell(&rep, "task-44", "budget").0, Some(json!("99.00")), "the working view shows the accepted remote value");
    assert_eq!(working_cell(&rep, "task-43", "owner").0, Some(json!("bob 改的")));
    assert_eq!(working_cell(&rep, "task-42", "owner").0, Some(json!("离线改二")), "unaffected local edits survive the rebase");
    let outcomes = send_all(&mut ws, &alice(), &mut rep);
    assert_eq!(outcomes, vec![("a/1".to_string(), "accepted".to_string()), ("a/2".to_string(), "accepted".to_string()), ("a/5".to_string(), "accepted".to_string())]);
    // nothing was lost: the two refused inputs are still there to inspect or export
    let kept: Vec<&str> = rep.pending.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(kept, vec!["a/3", "a/4"]);
    assert_eq!(rep.pending[0].request["operations"][0]["values"][0]["value"], "20");
    assert_eq!(root_of(&rep.confirmed), ws.checkpoint_ro());
    // the incremental rows written after every catch-up rebuild exactly the confirmed layer (references included)
    apply_delta(&mut disk, &rep.take_confirmed_delta().unwrap());
    let reloaded = load_rows(&disk).unwrap();
    assert_eq!(root_of(&reloaded), ws.checkpoint_ro());
    assert_eq!(reloaded.refs, rep.confirmed.refs);
    assert_eq!(reloaded.richtexts["notes"].meta.ast, rep.confirmed.richtexts["notes"].meta.ast);
    assert_eq!(root_of(&rep.working), ws.checkpoint_ro(), "no undecided submission is left in the view");
    assert_eq!(cell_rev(&ws, &alice(), "task-42", "owner"), rep.working.records[&("tasks".into(), "task-42".into())].value_rev("owner"));
    // the deleted record was not resurrected by the late edit
    assert!(ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_record", "record_id": "task-41" }))).is_err());
    let notes = ws.read(&alice(), "notes", None).unwrap();
    assert_eq!(notes["content"]["content"]["content"][1]["attrs"]["block_id"], "offline-p");
    assert_eq!(rep.working.richtexts["notes"].meta.ast, notes["content"]["content"]);
}

#[test]
fn unknown_results_revoked_permission_and_epoch_change() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "bob", None, &["read".into(), "update".into()]).unwrap();
    let mut rep = bootstrap(&mut ws, &bob(), 777);
    let (_, rev) = working_cell(&rep, "task-42", "owner");
    rep.submit_local(&local(&rep, "b/1", set_cell("task-42", "owner", json!("bob 离线"), rev)));
    let (_, rev) = working_cell(&rep, "task-43", "owner");
    rep.submit_local(&local(&rep, "b/2", set_cell("task-43", "owner", json!("bob 第二条"), rev)));

    // 1. the commit reaches the backend but the response is lost
    let req = rep.next_to_send().unwrap().unwrap();
    rep.mark("b/1", "sending", None).unwrap();
    assert_eq!(ws.commit(&req, &bob(), &CommitOpts::default())["status"], "accepted");
    rep.mark("b/1", "unknown", None).unwrap();
    assert!(rep.next_to_send().unwrap().is_some(), "an undecided submission does not block independent ones");
    // meanwhile someone else changes the same cell again: an `unknown` row must not be judged by content
    let rv = cell_rev(&ws, &alice(), "task-42", "owner");
    ok(&mut ws, &alice(), set_cell("task-42", "owner", json!("alice 又改"), rv));
    let head = ws.head_seq;
    let r = catch_up(&ws, &bob(), &mut rep);
    assert_eq!(r["settled"], json!(["b/1"]), "recognized by its key in the change stream");
    assert_eq!((rep.pending.len(), ws.head_seq), (1, head), "settled without a second application");
    assert_eq!(working_cell(&rep, "task-42", "owner").0, Some(json!("alice 又改")));

    // 2. the response is lost and the commit never arrived: get_submission says not_found → resend unchanged
    let req = rep.next_to_send().unwrap().unwrap();
    rep.mark("b/2", "unknown", None).unwrap();
    catch_up(&ws, &bob(), &mut rep);
    assert_eq!(rep.pending[0].state, "unknown", "still undecided after catching up");
    assert_eq!(ws.get_submission(&bob(), &rep.epoch, "b/2").unwrap()["status"], "not_found");
    rep.mark("b/2", "queued", None).unwrap();
    assert_eq!(rep.next_to_send().unwrap().unwrap(), req, "same key, same content");

    // 3. permission is revoked while offline: the backend refuses, the input stays
    ws.grant(&alice(), "bob", None, &["read".into()]).unwrap();
    let outcomes = send_all(&mut ws, &bob(), &mut rep);
    assert_eq!(outcomes, vec![("b/2".to_string(), "rejected".to_string())]);
    assert_eq!(rep.pending[0].result.as_ref().unwrap()["code"], "PERMISSION_DENIED");
    assert_eq!(rep.pending[0].request["operations"][0]["values"][0]["value"], "bob 第二条", "recoverable");
    assert_eq!(working_cell(&rep, "task-43", "owner").0, Some(json!("赵")), "and no longer shown as if it were accepted");
    assert_eq!(root_of(&rep.working), root_of(&rep.confirmed));

    // 4. a lock_required object cannot be edited through the replica
    let meta = ws.read(&alice(), "notes", None).unwrap()["meta_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_write_policy", "entity_id": "notes", "policy": "lock_required", "expect": { "rev": meta } }]));
    catch_up(&ws, &bob(), &mut rep);
    let r = rep.submit_local(&local(&rep, "b/3", json!([{ "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "end": true },
        "blocks": [{ "type": "paragraph", "attrs": { "block_id": "x1" } }] }])));
    assert_eq!(r["code"], "LOCK_REQUIRED");

    // 5. the Workspace is restored from a package: the old epoch is dead, nothing is replayed onto the new history
    let pkg = std::path::PathBuf::from(ws.export(&alice(), "share", true).unwrap()["path"].as_str().unwrap());
    drop(ws);
    env.svc.import(&alice(), &pkg, "restore", true).unwrap();
    let h = env.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    assert_eq!(ws.get_changes(&bob(), &rep.epoch, rep.confirmed_seq, 10, true).unwrap_err().code.as_str(), "EPOCH_MISMATCH");
    let req = rep.pending[0].request.clone();
    drop(ws);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert_eq!(ws.commit(&req, &bob(), &CommitOpts::default())["code"], "EPOCH_MISMATCH");
    assert_eq!(rep.pending.len(), 1, "pending input is kept for the user, not discarded");
}

/// A commit accepted by a newer backend (an anchor kind this core does not know) must not stop
/// the replica: it is applied as it is and reads back as `unsupported`.
#[test]
fn replica_keeps_anchors_it_does_not_know() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let mut rep = bootstrap(&mut ws, &alice(), 4343);
    // stand-in for a newer backend: the import path keeps what a strict write would refuse
    let req = json!({ "protocol_version": "0.1", "workspace_id": ws.workspace_id, "epoch": ws.epoch, "idempotency_key": "newer/1",
        "operations": [{ "op": "entity.create", "entity_id": "note-future", "type_id": "buckyos.annotation", "parent_id": "page-main",
            "order_key": "zz", "payload": { "target": { "entity_id": "notes", "selector": { "kind": "richtext_line", "line": 2 } },
                "range": { "kind": "richtext_columns", "from": 1 }, "kind": "note", "body": "来自新版本" } }] });
    let r = ws.commit(&req, &alice(), &CommitOpts { internal: true, import: true, undoes: None });
    assert_eq!(r["status"], "accepted", "{r}");
    catch_up(&ws, &alice(), &mut rep);
    let read = aiworkspace_core::read::read(&rep.working, &aiworkspace_core::access::Access::full("alice"), "note-future", None).unwrap();
    assert_eq!(read["content"]["anchor"], json!({ "state": "unsupported", "level": "entity" }));
}
