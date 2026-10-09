//! Rich text collaboration through `richtext.apply_update`, explicit drafts,
//! migration pre-check and the deterministic Mock (V06, V07, V09, V14, V18).

mod common;
use aiworkspace_core::richtext::{self, BlockOp, Position};
use aiworkspace_store::workspace::{Caller, CommitOpts, Workspace};
use base64::Engine;
use common::*;
use loro::LoroDoc;
use serde_json::{json, Value};
use std::path::PathBuf;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// An editor client: a working document forked from the confirmed state.
struct Client {
    doc: LoroDoc,
    lineage: String,
}

impl Client {
    fn open(ws: &Workspace, who: &Caller, entity: &str, peer: u64) -> Client {
        let st = ws.get_collab_state(who, entity).unwrap();
        let doc = richtext::load_doc(&B64.decode(st["snapshot"].as_str().unwrap()).unwrap(), std::iter::empty()).unwrap();
        doc.set_peer_id(peer).unwrap();
        Client { doc, lineage: st["lineage_id"].as_str().unwrap().to_string() }
    }
    /// Make a local edit and return the update bytes an editor would submit.
    fn edit(&self, ops: &[BlockOp]) -> Vec<u8> {
        let vv = self.doc.oplog_vv();
        richtext::apply_block_ops(&self.doc, ops).unwrap();
        richtext::export_updates(&self.doc, &vv).unwrap()
    }
    fn op(&self, entity: &str, update: &[u8]) -> Value {
        json!([{ "op": "richtext.apply_update", "entity_id": entity, "lineage_id": self.lineage, "update": B64.encode(update) }])
    }
}

fn undo(ws: &mut Workspace, commit_id: &Value, key: &str) -> Value {
    let req = json!({ "epoch": ws.epoch, "commit_id": commit_id, "idempotency_key": key });
    ws.undo(&alice(), &req)
}
fn prepare(ws: &mut Workspace, ops: Value) -> Value {
    let req = request(ws, ops);
    ws.prepare(&req, &alice())
}

fn para(id: &str, text: &str) -> Value {
    json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] })
}
fn insert_after(anchor: &str, id: &str, text: &str) -> BlockOp {
    BlockOp::Insert { position: Position::After(anchor.into()), blocks: vec![para(id, text)] }
}
fn ast(ws: &Workspace, entity: &str) -> Value {
    ws.read(&alice(), entity, None).unwrap()["content"]["content"].clone()
}

/// V07: two offline editors converge whatever the arrival order, duplicates are harmless,
/// updates with unknown bases are deferred, and an invalid merge never pollutes the authority.
#[test]
fn v07_concurrent_richtext() {
    let run = |a_first: bool| -> Value {
        let env = env();
        let id = project(&env);
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.grant(&alice(), "bob", None, &["read".into(), "update".into()]).unwrap();
        let a = Client::open(&ws, &alice(), "notes", 101);
        let b = Client::open(&ws, &bob(), "notes", 202);
        let ua = a.edit(&[insert_after("n-title", "a-1", "alice 离线写的一段中文")]);
        let ub = b.edit(&[insert_after("n-title", "b-1", "bob 离线写的另一段")]);
        let (first, second) = if a_first { ((&alice(), &a, &ua), (&bob(), &b, &ub)) } else { ((&bob(), &b, &ub), (&alice(), &a, &ua)) };
        let r1 = ok(&mut ws, first.0, first.1.op("notes", first.2));
        assert!(r1["touched"].as_array().unwrap().iter().any(|t| t["change"] == "text"));
        ok(&mut ws, second.0, second.1.op("notes", second.2));
        // the same bytes again (a retry under a new key): accepted and changes nothing
        let before = ast(&ws, "notes");
        ok(&mut ws, first.0, first.1.op("notes", first.2));
        assert_eq!(ast(&ws, "notes"), before);
        assert_eq!(ws.verify_refs().unwrap()["ok"], true);
        before
    };
    let x = run(true);
    assert_eq!(x, run(false), "arrival order does not change the merged document");
    let ids: Vec<&str> = x["content"].as_array().unwrap().iter().filter_map(|b| b["attrs"]["block_id"].as_str()).collect();
    assert!(ids.contains(&"a-1") && ids.contains(&"b-1"));

    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let a = Client::open(&ws, &alice(), "notes", 101);
    let u1 = a.edit(&[insert_after("n-title", "a-1", "first")]);
    let u2 = a.edit(&[BlockOp::Delete { block_ids: vec!["a-1".into()] }]);
    // u2 depends on u1 which the backend has not seen: deferred, retryable, nothing applied
    let before = (ws.head_seq, ws.get_collab_state(&alice(), "notes").unwrap()["snapshot"].clone());
    let r = commit(&mut ws, &alice(), a.op("notes", &u2));
    assert_eq!((code(&r), r["retryable"].as_bool()), ("BASE_UNKNOWN", Some(true)));
    assert_eq!((ws.head_seq, ws.get_collab_state(&alice(), "notes").unwrap()["snapshot"].clone()), before);
    ok(&mut ws, &alice(), a.op("notes", &u1));
    ok(&mut ws, &alice(), a.op("notes", &u2));
    // wrong lineage, garbage bytes
    let mut wrong = a.op("notes", &u1);
    wrong[0]["lineage_id"] = json!("other");
    assert_eq!(commit(&mut ws, &alice(), wrong)["sub_code"], "LINEAGE_MISMATCH");
    assert_eq!(code(&commit(&mut ws, &alice(), a.op("notes", b"not a loro update"))), "INVALID_OPERATION");
    // two clients concurrently create the same block id: the first wins, the second is rejected, state intact
    let c1 = Client::open(&ws, &alice(), "notes", 301);
    let c2 = Client::open(&ws, &alice(), "notes", 302);
    let (d1, d2) = (c1.edit(&[insert_after("n-title", "dup", "one")]), c2.edit(&[insert_after("n-title", "dup", "two")]));
    ok(&mut ws, &alice(), c1.op("notes", &d1));
    let good = ast(&ws, "notes");
    assert_eq!(code(&commit(&mut ws, &alice(), c2.op("notes", &d2))), "INVALID_SCHEMA");
    assert_eq!(ast(&ws, "notes"), good);
    // editor-session updates are SessionLocal: the backend cannot compensate them
    let last = ws.get_changes(&alice(), &ws.epoch.clone(), ws.head_seq - 1, 1, false).unwrap()["changes"][0]["commit_id"].clone();
    let u = undo(&mut ws, &last, "u-rt");
    assert_eq!(code(&u), "NOT_UNDOABLE");
    // a client's own UndoManager reverts only its own edit, not the merged remote one
    let me = Client::open(&ws, &alice(), "notes", 401);
    let mut undo = loro::UndoManager::new(&me.doc);
    richtext::apply_block_ops(&me.doc, &[insert_after("n-title", "mine", "my edit")]).unwrap();
    let other = Client::open(&ws, &alice(), "notes", 402);
    let remote = other.edit(&[insert_after("n-title", "theirs", "their edit")]);
    me.doc.import(&remote).unwrap();
    assert!(undo.undo().unwrap());
    let after: Vec<String> = richtext::validate_ast(&richtext::decode_ast(&me.doc).unwrap(), &richtext::Limits::default()).unwrap().blocks.into_keys().collect();
    assert!(after.contains(&"theirs".to_string()) && !after.contains(&"mine".to_string()));
}

/// V06: rich text survives restart; a personal-backup restore keeps the lineage so old offline edits still merge.
#[test]
fn v06_richtext_restore_and_lineage() {
    let env = env();
    let id = project(&env);
    let (offline_update, lineage, pkg_backup, pkg_share) = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let a = Client::open(&ws, &alice(), "notes", 101);
        for i in 0..5 {
            let u = a.edit(&[insert_after("n-title", &format!("p-{i}"), &format!("第 {i} 段，含 emoji 😀 与标记"))]);
            ok(&mut ws, &alice(), a.op("notes", &u));
        }
        // a client that went offline now and keeps an unsent edit
        let offline = Client::open(&ws, &alice(), "notes", 777);
        let pending = offline.edit(&[insert_after("p-0", "late", "离线期间写的")]);
        let backup = PathBuf::from(ws.export(&alice(), "personal_backup", true, None).unwrap()["path"].as_str().unwrap());
        let share = PathBuf::from(ws.export(&alice(), "share", true, None).unwrap()["path"].as_str().unwrap());
        (pending, offline.lineage, backup, share)
    };
    // restart: memory is rebuilt from snapshot + updates
    let env = env.reopen();
    let expect = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let a = Client::open(&ws, &alice(), "notes", 102);
        assert_eq!(richtext::decode_ast(&a.doc).unwrap(), ast(&ws, "notes"), "collab state and AST projection agree after restart");
        let u = a.edit(&[insert_after("n-title", "after-restart", "重启后继续编辑")]);
        ok(&mut ws, &alice(), a.op("notes", &u));
        ast(&ws, "notes")["content"].as_array().unwrap().len()
    };
    assert!(expect >= 11);
    // full restore elsewhere: same lineage → the old offline increment still merges
    let other = common::env();
    other.svc.import(&alice(), &pkg_backup, "restore", false).unwrap();
    let h = other.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert_eq!(ws.get_collab_state(&alice(), "notes").unwrap()["lineage_id"], json!(lineage));
    let op = json!([{ "op": "richtext.apply_update", "entity_id": "notes", "lineage_id": lineage, "update": B64.encode(&offline_update) }]);
    ok(&mut ws, &alice(), op.clone());
    assert!(ast(&ws, "notes")["content"].as_array().unwrap().iter().any(|b| b["attrs"]["block_id"] == "late"));
    drop(ws);
    // a share package is content only: new lineage, so that old increment is refused instead of mis-merged
    let third = common::env();
    third.svc.import(&alice(), &pkg_share, "restore", false).unwrap();
    let h = third.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert_ne!(ws.get_collab_state(&alice(), "notes").unwrap()["lineage_id"], json!(lineage));
    assert_eq!(commit(&mut ws, &alice(), op)["sub_code"], "LINEAGE_MISMATCH");
    assert!(ast(&ws, "notes")["content"].as_array().unwrap().iter().any(|b| b["attrs"]["block_id"] == "p-4"));
}

/// Storage compaction folds updates into the snapshot without losing history or bumping `seq`.
#[test]
fn richtext_storage_compaction() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let early = Client::open(&ws, &alice(), "notes", 900);
    let stale = early.edit(&[insert_after("n-title", "from-old-base", "基于很早的状态")]);
    let a = Client::open(&ws, &alice(), "notes", 101);
    for i in 0..205 {
        let u = a.edit(&[insert_after("n-title", &format!("c-{i}"), "x")]);
        ok(&mut ws, &alice(), a.op("notes", &u));
    }
    let conn = rusqlite::Connection::open(ws.dir.join("doc.sqlite")).unwrap();
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM richtext_updates WHERE entity_id = 'notes'", [], |r| r.get(0)).unwrap();
    assert!(n < 200, "updates were folded into the snapshot ({n} left)");
    drop(ws);
    env.svc.close(&id);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert_eq!(ast(&ws, "notes")["content"].as_array().unwrap().len(), 5 + 205);
    // history is complete: an update made against the very first state still merges
    ok(&mut ws, &alice(), early.op("notes", &stale));
}

/// V14 (drafts): four private edits reach the other side as one block-level result.
#[test]
fn v14_explicit_draft_commits_only_the_result() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let base = ws.read(&alice(), "notes", None).unwrap();
    let base_ast = base["content"]["content"].clone();
    let index: aiworkspace_core::model::BlockIndex = serde_json::from_value(
        base["content"]["blocks"].as_object().unwrap().iter()
            .map(|(k, v)| (k.clone(), json!({ "parent": "", "node_type": "", "hash": v["hash"], "struct_rev": v["struct_rev"] }))).collect::<serde_json::Map<_, _>>().into(),
    ).unwrap();
    // private fork; its CRDT history never leaves the client
    let draft = Client::open(&ws, &alice(), "notes", 555);
    for (i, text) in ["草稿一 SECRET-A", "草稿二 SECRET-B", "草稿三 SECRET-C", "最终稿"].iter().enumerate() {
        richtext::apply_block_ops(&draft.doc, &[BlockOp::Replace { block_id: "n-end".into(), node: para("n-end", text) }]).unwrap();
        if i == 1 {
            richtext::apply_block_ops(&draft.doc, &[insert_after("n-end", "tmp", "临时块 SECRET-D")]).unwrap();
        }
        if i == 2 {
            richtext::apply_block_ops(&draft.doc, &[BlockOp::Delete { block_ids: vec!["tmp".into()] }]).unwrap();
        }
    }
    let ops = richtext::diff_blocks("notes", &base_ast, &richtext::decode_ast(&draft.doc).unwrap(), &index).unwrap();
    assert_eq!(ops.len(), 1, "four private edits fold into one replace: {ops:?}");
    let r = ok(&mut ws, &alice(), Value::Array(ops.clone()));
    assert!(r["server_ops"][0]["update"].is_string(), "the client applies the bytes the backend produced");
    // what the peer can ever see: the change stream and the collaboration state
    let stream = ws.get_changes(&alice(), &ws.epoch.clone(), 0, 500, true).unwrap().to_string();
    let collab = B64.decode(ws.get_collab_state(&alice(), "notes").unwrap()["snapshot"].as_str().unwrap()).unwrap();
    for secret in ["SECRET-A", "SECRET-B", "SECRET-C", "SECRET-D"] {
        assert!(!stream.contains(secret));
        assert!(!collab.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
    assert!(stream.contains("最终稿"));
    // someone else changed the same block meanwhile → the draft commit conflicts and the draft is kept by the client
    let r = commit(&mut ws, &alice(), Value::Array(ops));
    assert_eq!((r["status"].as_str(), code(&r)), (Some("conflict"), "REVISION_CONFLICT"));
    assert_eq!(r["conflicts"][0]["current_value"]["node"]["content"][0]["text"], "最终稿");
    // block-level commits are compensable
    let c = ws.get_changes(&alice(), &ws.epoch.clone(), ws.head_seq - 1, 1, false).unwrap()["changes"][0]["commit_id"].clone();
    assert_eq!(undo(&mut ws, &c, "u-d")["status"], "accepted");
    assert_eq!(ast(&ws, "notes"), base_ast);
}

/// V09: migration pre-check through `prepare`, all-or-nothing execution even with a fault inside the transaction.
#[test]
fn v09_migration_precheck_and_atomicity() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.add_field", "source_id": "tasks", "field": { "field_id": "due_text", "name": "截止文本", "type": "text" } }]));
    let add = ws.head_seq;
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "tasks", "values": [
        { "record_id": "task-40", "field_id": "due_text", "value": "2026-11-01", "expect": { "rev": 0 } },
        { "record_id": "task-41", "field_id": "due_text", "value": "下周五", "expect": { "rev": 0 } },
        { "record_id": "task-42", "field_id": "due_text", "value": "2026-12-24", "expect": { "rev": 0 } }] }]));
    let mig = |on_failure: &str| json!([{ "op": "table.migrate_field", "source_id": "tasks", "field_id": "due_text",
        "expect": { "rev": add }, "to": { "type": "date" }, "on_failure": on_failure }]);
    let seq = ws.head_seq;
    let p = prepare(&mut ws, mig("reject"));
    assert_eq!(p["status"], "rejected");
    assert_eq!(p["errors"][0]["data"]["report"], json!({ "total": 5, "convertible": 2, "unset": 2,
        "failing": { "count": 1, "sample": [{ "record_id": "task-41", "value": "下周五" }] } }));
    let p = prepare(&mut ws, mig("unset"));
    assert_eq!((p["status"].as_str(), &p["reports"][0]["report"]["failing"]["count"]), (Some("ok"), &json!(1)));
    assert_eq!(ws.head_seq, seq, "prepare stores nothing");
    let before = ws.checkpoint_ro();
    ws.failpoint = Some(aiworkspace_store::FailPoint { at: aiworkspace_store::FailAt::InTxn, abort: false, countdown: 1 });
    assert_eq!(code(&commit(&mut ws, &alice(), mig("unset"))), "STORAGE_IO_ERROR");
    assert_eq!(ws.checkpoint_ro(), before, "no half-migrated table");
    let m = ok(&mut ws, &alice(), mig("unset"));
    let f = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_field", "field_id": "due_text" }))).unwrap();
    assert_eq!(f["content"]["type"], "date");
    // typed values round-trip exactly; a write based on the old type is recognized
    assert_eq!(code(&commit(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "tasks", "field_type_revs": { "due_text": add },
        "values": [{ "record_id": "task-41", "field_id": "due_text", "value": "2026-10-09", "expect": { "rev": m["seq"] } }] }]))), "SCHEMA_CONFLICT");
    let u = undo(&mut ws, &m["commit_id"], "u-m");
    assert_eq!(u["status"], "accepted", "{u}");
    assert_eq!(ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_cell", "record_id": "task-41", "field_id": "due_text" }))).unwrap()["content"]["value"], "下周五");
}

/// V18: the Mock's candidate goes through the same pipeline; read-set and manual edits are protected.
#[test]
fn v18_controlled_processing() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let params = json!({ "today": "2026-10-13" });
    let start = |ws: &mut Workspace, key: &str, p: &Value| ws.proc_start(&alice(), "mock.task-summary@1", p, key).unwrap();
    let risk = |ws: &Workspace, rec: &str| {
        ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_cell", "record_id": rec, "field_id": "risk" }))).unwrap()["content"].clone()
    };
    // 1. a candidate is data: nothing changed yet
    let seq0 = ws.head_seq;
    let run = start(&mut ws, "r1", &params);
    assert_eq!((run["state"].as_str(), run["simulated"].as_bool()), (Some("waiting_confirmation"), Some(true)));
    assert_eq!(run["prepare"]["status"], "ok");
    assert_eq!(run["inputs"][0]["entity_id"], "tasks");
    assert_eq!(ws.head_seq, seq0);
    assert_eq!(start(&mut ws, "r1", &params)["run_id"], run["run_id"], "proc.start is idempotent");
    // 2. unrelated edits (owner is outside the read set) do not invalidate the candidate
    let rv = cell_rev(&ws, &alice(), "task-41", "owner");
    ok(&mut ws, &alice(), set_cell("task-41", "owner", json!("换人"), rv));
    let applied = ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap();
    assert_eq!(applied["state"], "succeeded", "{applied}");
    assert_eq!(ws.head_seq, seq0 + 2, "one application = one commit");
    assert_eq!(risk(&ws, "task-41")["value"], "option-high");
    assert_eq!(risk(&ws, "task-42")["value"], "option-medium");
    assert_eq!(risk(&ws, "task-44")["value"], "option-low");
    assert_eq!(risk(&ws, "task-43")["is_set"], false, "no due date → not set");
    assert_eq!(risk(&ws, "task-41")["meta"]["derived"]["program"], "mock.task-summary@1");
    assert_eq!(risk(&ws, "task-41")["meta"]["derived"]["run_id"], run["run_id"]);
    let summary = ws.read(&alice(), "summary", None).unwrap();
    assert_eq!(summary["content"]["content"]["content"][1]["content"][0]["text"], "共 5 项任务，未完成 4 项，逾期 1 项。");
    assert!(ws.read(&alice(), "cell-summary", None).is_ok());
    // 3. the whole application is one undo unit
    let commit_id = applied["result"]["commit_id"].clone();
    assert_eq!(undo(&mut ws, &commit_id, "u-run")["status"], "accepted");
    assert_eq!(risk(&ws, "task-41")["is_set"], false);
    assert!(ws.read(&alice(), "summary", None).is_err());
    let redo = ws.get_changes(&alice(), &ws.epoch.clone(), ws.head_seq - 1, 1, false).unwrap()["changes"][0]["commit_id"].clone();
    assert_eq!(undo(&mut ws, &redo, "redo-run")["status"], "accepted");
    // 4. the three read-set violations: changed due, new open task, done → open
    for (n, change) in [
        json!([{ "op": "table.set_values", "source_id": "tasks", "values": [{ "record_id": "task-44", "field_id": "due", "value": "2026-10-01", "expect": { "rev": 2 } }] }]),
        json!([{ "op": "table.insert_records", "source_id": "tasks", "records": [{ "record_id": "task-99", "values": { "title": "新任务", "status": "option-open", "due": "2026-10-01" } }] }]),
        json!([{ "op": "table.set_values", "source_id": "tasks", "values": [{ "record_id": "task-40", "field_id": "status", "value": "option-open", "expect": { "rev": 2 } }] }]),
    ].into_iter().enumerate() {
        let run = start(&mut ws, &format!("stale-{n}"), &json!({ "today": "2026-10-21" }));
        assert_eq!(run["state"], "waiting_confirmation", "{run}");
        ok(&mut ws, &alice(), change);
        let before = ws.head_seq;
        let r = ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap();
        assert_eq!((r["state"].as_str(), code(&r["result"])), (Some("failed"), "REVISION_CONFLICT"), "case {n}: {r}");
        assert_eq!(ws.head_seq, before, "case {n}: the old result stays, nothing was applied");
    }
    // 5. manual override: skipped with a warning; edited after the candidate was built → conflict
    let run = start(&mut ws, "fresh", &json!({ "today": "2026-10-13" }));
    ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap();
    let rv = cell_rev(&ws, &alice(), "task-42", "risk");
    ok(&mut ws, &alice(), set_cell("task-42", "risk", json!("option-low"), rv));
    assert_eq!(risk(&ws, "task-42")["meta"]["manual_override"], true);
    let run = start(&mut ws, "after-override", &json!({ "today": "2026-11-15" }));
    assert_eq!(run["warnings"][0], json!({ "code": "MANUAL_OVERRIDE_SKIPPED", "record_id": "task-42", "field_id": "risk" }));
    // a human rewrites a generated block after the candidate was built
    let h = ws.read(&alice(), "summary", None).unwrap()["content"]["blocks"]["sum-stats"]["hash"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "richtext.replace_block", "entity_id": "summary", "block_id": "sum-stats", "expect": { "hash": h },
        "node": para("sum-stats", "人工改写的统计") }]));
    let r = ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap();
    assert_eq!(code(&r["result"]), "REVISION_CONFLICT");
    assert_eq!(risk(&ws, "task-42")["value"], "option-low", "the human value survives");
    assert_eq!(ws.read(&alice(), "summary", None).unwrap()["content"]["content"]["content"][1]["content"][0]["text"], "人工改写的统计");
    // 6. bad candidates are rejected as a whole by prepare and by commit
    for inject in ["unknown_option", "dangling_ref", "over_limit"] {
        let run = start(&mut ws, &format!("bad-{inject}"), &json!({ "today": "2026-12-01", "inject": inject }));
        assert_eq!(run["prepare"]["status"], "rejected", "{inject}");
        let before = ws.checkpoint_ro();
        let r = ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap();
        assert_eq!(r["result"]["status"], "rejected", "{inject}");
        assert_eq!(ws.checkpoint_ro(), before);
    }
    // 7. cancel: a late application — even a direct commit of the candidate — is refused
    let run = start(&mut ws, "to-cancel", &json!({ "today": "2027-01-01" }));
    let candidate = run["candidate"].clone();
    assert_eq!(ws.proc_cancel(&alice(), run["run_id"].as_str().unwrap()).unwrap()["state"], "cancelled");
    assert_eq!(ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap_err().code.as_str(), "RUN_CANCELLED");
    assert_eq!(code(&ws.commit(&candidate, &alice(), &CommitOpts::default())), "RUN_CANCELLED");
    // cancelling something that already applied says so
    let run = start(&mut ws, "apply-then-cancel", &json!({ "today": "2027-01-01" }));
    ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap();
    let c = ws.proc_cancel(&alice(), run["run_id"].as_str().unwrap()).unwrap();
    assert_eq!((c["already_applied"].as_bool(), c["commit_id"].is_string()), (Some(true), true));
    // 8. the Mock has no way around write locks
    let meta = ws.read(&alice(), "summary", None).unwrap()["meta_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_write_policy", "entity_id": "summary", "policy": "lock_required", "expect": { "rev": meta } }]));
    let rv = cell_rev(&ws, &alice(), "task-43", "due");
    ok(&mut ws, &alice(), set_cell("task-43", "due", json!("2027-06-01"), rv));
    let run = start(&mut ws, "locked", &json!({ "today": "2028-01-01" }));
    assert_eq!(run["state"], "waiting_confirmation", "{run}");
    assert_eq!(code(&ws.proc_apply(&alice(), run["run_id"].as_str().unwrap(), Some("s1")).unwrap()["result"]), "LOCK_REQUIRED");
}
