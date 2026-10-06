//! Store-level acceptance tests against real SQLite files (V02–V05, V08–V14,
//! V19–V22). Each test names the design-doc acceptance items it covers.

mod common;
use aiworkspace_store::workspace::{CommitOpts, FailAt, FailPoint};
use common::*;
use serde_json::{json, Value};

/// V01: ids computed by the WASM-safe core equal ndn-lib's for the same input.
#[test]
fn v01_object_ids_match_ndn_lib() {
    use aiworkspace_core::canonical::*;
    let samples = [
        json!({ "ws_type": "buckyos.table-source", "schema_version": 1, "content": { "b": [1, 2.5, "x"], "a": null, "数": "e\u{301}" } }),
        json!({ "\u{20ac}": 1, "\r": 2, "\u{1f600}": 3, "1": { "z": true, "a": 1e21 } }),
        json!({}),
    ];
    for v in samples {
        let (id, text) = named_object(OBJ_TYPE_JSON, &v).unwrap();
        let (nid, ntext) = ndn_lib::build_named_object_by_json(OBJ_TYPE_JSON, &v);
        assert_eq!(text, ntext);
        assert_eq!(id, nid.to_string());
        assert_eq!(ndn_lib::ObjId::new(&id).unwrap(), nid, "the JSON form is type:hex");
    }
    for data in [&b""[..], b"hello", &vec![7u8; 70_000]] {
        let ours = chunk_id(data);
        let hash = <sha2::Sha256 as sha2::Digest>::digest(data);
        let theirs = ndn_lib::ChunkId::from_mix256_result(data.len() as u64, &hash).to_string();
        assert_eq!(ours, theirs);
        let (fid, ftext) = file_object(data.len() as u64, &ours).unwrap();
        let mut f = ndn_lib::FileObject::new(String::new(), data.len() as u64, ours.clone());
        f.content_obj.create_time = 0;
        f.content_obj.last_update_time = 0;
        let (nid, ntext) = ndn_lib::build_named_object_by_json(OBJ_TYPE_FILE, &serde_json::to_value(&f).unwrap());
        assert_eq!(ftext, ntext);
        assert_eq!(fid, nid.to_string());
    }
}

/// The fixture replays through the service interface and the reference index
/// equals a full rebuild.
#[test]
fn fixture_replays_and_reads_back() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    let outline = ws.outline(&alice()).unwrap();
    assert_eq!(outline["entities"].as_array().unwrap().len(), 17);
    let t = ws.read(&alice(), "tasks", None).unwrap();
    assert_eq!(t["content"]["record_count"], 5);
    assert_eq!(t["content"]["fields"].as_array().unwrap().len(), 6);
    let rec = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_record", "record_id": "task-42" }))).unwrap();
    assert_eq!(rec["content"]["values"]["budget"], "1200.00");
    assert_eq!(rec["content"]["body_ref"]["entity_id"], "task-42-details");
    let notes = ws.read(&alice(), "notes", None).unwrap();
    assert_eq!(notes["content"]["content"]["content"][0]["content"][0]["text"], "本周项目进展");
    assert_eq!(ws.read(&alice(), "diagram", None).unwrap()["content"]["availability"], "available");
    assert_eq!(ws.read(&alice(), "note-budget", None).unwrap()["content"]["anchor"]["state"], "resolved");
    assert_eq!(ws.resolve(&alice(), None, Some("/data/任务")).unwrap()["reference"]["entity_id"], "tasks");
    let refs = ws.verify_refs().unwrap();
    assert_eq!(refs["ok"], true, "{refs}");
    assert!(refs["count"].as_u64().unwrap() >= 9);
    // same source, two views: shared records, independent configuration
    let all = ws.query(&alice(), &json!({ "view_id": "cell-all-tasks" }), &env.svc.sources).unwrap();
    let open = ws.query(&alice(), &json!({ "view_id": "cell-open-tasks" }), &env.svc.sources).unwrap();
    assert_eq!(all["total"], 5);
    let ids: Vec<&str> = open["rows"].as_array().unwrap().iter().map(|r| r["record_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["task-41", "task-42", "task-44", "task-43"], "due asc, empty last");
}

/// V02 + V08: renames, moves and reordering keep stable references and content ObjectIds.
#[test]
fn v02_v08_identity_survives_rename_move_sort() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let before = ws.checkpoint(&alice()).unwrap();
    let tasks = ws.read(&alice(), "tasks", None).unwrap();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.rename", "entity_id": "tasks", "name": "任务清单", "expect": { "rev": tasks["meta_rev"] } },
        { "op": "tree.place", "entity_id": "tasks", "order_key": "zz" },
        { "op": "entity.create", "entity_id": "grp", "type_id": "buckyos.container", "parent_id": "surface-main", "order_key": "k", "payload": { "kind": "group" } },
        { "op": "tree.move", "entity_id": "cell-diagram", "new_parent_id": "grp", "order_key": "a" }
    ]));
    let after = ws.checkpoint(&alice()).unwrap();
    assert_ne!(before.content_root, after.content_root, "structure index changed");
    for (e, obj) in &before.objects {
        if e != "root" {
            assert_eq!(after.objects.get(e), Some(obj), "content object of {e} must not change on rename/move");
        }
    }
    // field rename, a new row and a saved sort: selectors and the annotation still hit the same cell
    let f = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_field", "field_id": "budget" }))).unwrap();
    ok(&mut ws, &alice(), json!([
        { "op": "table.update_field", "source_id": "tasks", "field_id": "budget", "changes": { "name": "预算（元）" }, "expect": { "rev": f["content"]["def_rev"] } },
        { "op": "table.insert_records", "source_id": "tasks", "records": [{ "record_id": "task-00", "values": { "title": "插到最前" } }] }
    ]));
    let cell = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_cell", "record_id": "task-42", "field_id": "budget" }))).unwrap();
    assert_eq!(cell["content"]["value"], "1200.00");
    let note = ws.read(&alice(), "note-budget", None).unwrap();
    assert_eq!(note["content"]["anchor"]["state"], "resolved");
    assert_eq!(note["content"]["payload"]["target"]["selector"]["record_id"], "task-42");
    assert_eq!(ws.resolve(&alice(), Some(&json!({ "entity_id": "tasks" })), None).unwrap()["type_id"], "buckyos.table-source");
    // fixed_revision resolves the immutable content even after the entity changed
    let fixed = ws.resolve(&alice(), Some(&json!({ "entity_id": "tasks", "version": { "mode": "fixed_revision", "object_id": before.objects["tasks"] } })), None).unwrap();
    assert_eq!(fixed["object"]["content"]["records"]["count"], 5);
    let ch = ws.resolve(&alice(), Some(&json!({ "entity_id": "tasks", "version": { "mode": "published_channel", "channel": "stable" } })), None).unwrap_err();
    assert_eq!(ch.code.as_str(), "UNSUPPORTED_VERSION", "never downgraded to live head");
}

/// V10: tree + rich text + table in one batch; a failure anywhere (including
/// inside the storage transaction) leaves data, history and CRDT untouched.
#[test]
fn v10_mixed_batch_atomic_with_fault_injection() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let snap = |ws: &aiworkspace_store::Workspace| {
        (ws.head_seq, ws.get_collab_state(&alice(), "notes").unwrap()["snapshot"].clone(), ws.checkpoint_ro(), ws.get_changes(&alice(), &ws.epoch, 0, 500, false).unwrap()["changes"].as_array().unwrap().len())
    };
    let before = snap(&ws);
    let batch = |last_value: &str, rev: u64| json!([
        { "op": "tree.place", "entity_id": "cell-notes", "order_key": "e5" },
        { "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "after": "n-title" },
          "blocks": [{ "type": "paragraph", "attrs": { "block_id": "n-new" }, "content": [{ "type": "text", "text": "批次内新增" }] }] },
        { "op": "table.set_values", "source_id": "tasks", "values": [{ "record_id": "task-42", "field_id": "status", "value": last_value, "expect": { "rev": rev } }] }
    ]);
    let rev = cell_rev(&ws, &alice(), "task-42", "status");
    let r = commit(&mut ws, &alice(), batch("option-nope", rev));
    assert_eq!(r["status"], "rejected");
    assert_eq!(r["errors"][0]["op_index"], 2);
    assert_eq!(snap(&ws), before, "validation failure: nothing changed");
    for at in [FailAt::BeforeTxn, FailAt::InTxn] {
        ws.failpoint = Some(FailPoint { at, abort: false, countdown: 1 });
        let r = commit(&mut ws, &alice(), batch("option-done", rev));
        assert_eq!(code(&r), "STORAGE_IO_ERROR", "{at:?}");
        assert_eq!(snap(&ws), before, "{at:?}: no data, no history, no broadcast, CRDT untouched");
    }
    let r = ok(&mut ws, &alice(), batch("option-done", rev));
    assert_eq!(r["server_ops"][0]["entity_id"], "notes");
    assert_eq!(ws.head_seq, before.0 + 1);
    assert_eq!(ws.verify_refs().unwrap()["ok"], true);
}

/// V11: independent fields both commit; the same field on a stale base conflicts; late writes never resurrect.
#[test]
fn v11_conflicts() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let (r_status, r_budget) = (cell_rev(&ws, &alice(), "task-42", "status"), cell_rev(&ws, &alice(), "task-42", "budget"));
    ok(&mut ws, &alice(), set_cell("task-42", "status", json!("option-done"), r_status));
    ok(&mut ws, &alice(), set_cell("task-42", "budget", json!("1500"), r_budget));
    let r = commit(&mut ws, &alice(), set_cell("task-42", "budget", json!("1"), r_budget));
    assert_eq!((r["status"].as_str(), code(&r)), (Some("conflict"), "REVISION_CONFLICT"));
    assert_eq!(r["conflicts"][0]["current_value"], "1500.00");
    let rec = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_record", "record_id": "task-41" }))).unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.delete_records", "source_id": "tasks", "records": [{ "record_id": "task-41", "expect": { "rev": rec["content"]["rev"] } }] }]));
    let r = commit(&mut ws, &alice(), set_cell("task-41", "owner", json!("迟到"), 2));
    assert_eq!(code(&r), "TARGET_DELETED");
    assert_eq!(ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_record", "record_id": "task-41" }))).unwrap_err().code.as_str(), "TARGET_DELETED");
    // a 10k-row style single-cell write touches exactly one record row
    let rv = cell_rev(&ws, &alice(), "task-44", "owner");
    ok(&mut ws, &alice(), set_cell("task-44", "owner", json!("钱"), rv));
    assert_eq!(ws.last_stats.records, 1);
}

/// V12: a commit whose response was lost is applied once; same key + other payload is refused.
#[test]
fn v12_idempotency_and_unknown_result() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let mut req = request(&ws, json!([{ "op": "entity.create", "entity_id": "grp-1", "type_id": "buckyos.container",
        "parent_id": "surface-main", "order_key": "m", "payload": { "kind": "group" } }]));
    // durable, then the "response is lost"
    ws.failpoint = Some(FailPoint { at: FailAt::AfterCommit, abort: false, countdown: 1 });
    let lost = ws.commit(&req, &alice(), &CommitOpts::default());
    assert_eq!(code(&lost), "STORAGE_IO_ERROR");
    drop(ws);
    env.svc.close(&id); // as after a crash: memory state is rebuilt from the database
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let key = req["idempotency_key"].as_str().unwrap().to_string();
    let found = ws.get_submission(&alice(), &ws.epoch.clone(), &key).unwrap();
    assert_eq!(found["status"], "accepted");
    let seq = ws.head_seq;
    let again = ws.commit(&req, &alice(), &CommitOpts::default());
    assert_eq!((again["status"].as_str(), again["replayed"].as_bool()), (Some("accepted"), Some(true)), "{again}");
    assert_eq!(ws.head_seq, seq, "no second effect, and no ID_CONFLICT against itself");
    // session_id and message are not part of the digest
    req["message"] = json!("retry");
    assert_eq!(ws.commit(&req, &alice(), &CommitOpts::default())["replayed"], true);
    req["operations"][0]["order_key"] = json!("n");
    assert_eq!(code(&ws.commit(&req, &alice(), &CommitOpts::default())), "IDEMPOTENCY_MISMATCH");
    // rejected/conflict results are not recorded: the same key is evaluated again
    let bad = request(&ws, set_cell("task-42", "status", json!("option-nope"), 2));
    assert_eq!(ws.commit(&bad, &alice(), &CommitOpts::default())["status"], "rejected");
    assert_eq!(ws.get_submission(&alice(), &ws.epoch.clone(), bad["idempotency_key"].as_str().unwrap()).unwrap()["status"], "not_found");
    // another principal's key space is separate
    assert_eq!(ws.get_submission(&bob(), &ws.epoch.clone(), &key).unwrap_err().code.as_str(), "NOT_FOUND");
}

/// V13: compensation never overwrites later changes of others; redo = undo of the undo.
#[test]
fn v13_undo_redo() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "bob", None, &["read".into(), "update".into()]).unwrap();
    let epoch = ws.epoch.clone();
    let undo = |ws: &mut aiworkspace_store::Workspace, who: &aiworkspace_store::Caller, commit_id: &Value, key: &str, extra: Value| {
        let mut req = json!({ "epoch": epoch, "commit_id": commit_id, "idempotency_key": key });
        for (k, v) in extra.as_object().into_iter().flatten() {
            req[k] = v.clone();
        }
        ws.undo(who, &req)
    };
    let (r_owner, r_budget) = (cell_rev(&ws, &alice(), "task-43", "owner"), cell_rev(&ws, &alice(), "task-43", "budget"));
    let c1 = ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "tasks", "values": [
        { "record_id": "task-43", "field_id": "owner", "value": "孙", "expect": { "rev": r_owner } },
        { "record_id": "task-43", "field_id": "budget", "value": "77", "expect": { "rev": r_budget } }] }]));
    let u1 = undo(&mut ws, &alice(), &c1["commit_id"], "u1", json!({}));
    assert_eq!(u1["status"], "accepted", "{u1}");
    let rec = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_record", "record_id": "task-43" }))).unwrap();
    assert_eq!(rec["content"]["values"]["owner"], "赵");
    assert!(rec["content"]["values"].get("budget").is_none(), "previously unset → unset again");
    // redo is the undo of the compensating commit
    let redo = undo(&mut ws, &alice(), &u1["commit_id"], "r1", json!({}));
    assert_eq!(redo["status"], "accepted");
    assert_eq!(cell_value(&ws, "task-43", "budget"), json!("77.00"));
    // bob changes one of the two cells; alice's undo now conflicts on that cell only
    let rv = cell_rev(&ws, &bob(), "task-43", "owner");
    ok(&mut ws, &bob(), set_cell("task-43", "owner", json!("bob 改的"), rv));
    let blocked = undo(&mut ws, &alice(), &redo["commit_id"], "u2", json!({}));
    assert_eq!(blocked["status"], "conflict", "{blocked}");
    assert_eq!(blocked["undoable"].as_array().unwrap().len() + blocked["blocked"].as_array().unwrap().len(), 2);
    assert_eq!(cell_value(&ws, "task-43", "owner"), json!("bob 改的"), "not overwritten");
    // partial undo needs the digest of the plan the caller was shown
    assert_eq!(code(&undo(&mut ws, &alice(), &redo["commit_id"], "u3", json!({ "mode": "partial" }))), "INVALID_OPERATION");
    let part = undo(&mut ws, &alice(), &redo["commit_id"], "u3", json!({ "mode": "partial", "plan_digest": blocked["plan_digest"] }));
    assert_eq!(part["status"], "accepted", "{part}");
    assert_eq!(cell_value(&ws, "task-43", "owner"), json!("bob 改的"));
    // bob may not undo alice's commit without manage; editor-session text updates are not compensable
    assert_eq!(code(&undo(&mut ws, &bob(), &c1["commit_id"], "u4", json!({}))), "PERMISSION_DENIED");
}

fn cell_value(ws: &aiworkspace_store::Workspace, record: &str, field: &str) -> Value {
    ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_cell", "record_id": record, "field_id": field }))).unwrap()["content"]["value"].clone()
}

/// V19: committed changes are observable in order, deduplicable, permission-filtered, and classified.
#[test]
fn v19_change_stream() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "bob", None, &["read".into(), "comment".into()]).unwrap();
    let epoch = ws.epoch.clone();
    let head = ws.head_seq;
    let cell = ws.read(&alice(), "cell-open-tasks", None).unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "cell-open-tasks",
        "keys": [{ "key": "title", "value": "待办", "expect": { "rev": cell["content"]["key_revs"]["title"] } }] }]));
    ok(&mut ws, &alice(), json!([{ "op": "tree.place", "entity_id": "cell-open-tasks", "order_key": "g5" }]));
    let rv = cell_rev(&ws, &alice(), "task-42", "due");
    ok(&mut ws, &alice(), set_cell("task-42", "due", json!("2026-10-25"), rv));
    ok(&mut ws, &bob(), json!([{ "op": "entity.create", "entity_id": "bob-note", "type_id": "buckyos.annotation", "parent_id": "data",
        "order_key": "q", "scope": "personal", "payload": { "target": { "entity_id": "notes" }, "kind": "note", "body": "bob 的私人笔记" } }]));
    let changes = ws.get_changes(&alice(), &epoch, head, 100, true).unwrap();
    let evs = changes["changes"].as_array().unwrap();
    assert_eq!(evs.iter().map(|e| e["seq"].as_u64().unwrap()).collect::<Vec<_>>(), (head + 1..=head + 4).collect::<Vec<_>>());
    let class = |i: usize| evs[i]["touched"][0]["change"].as_str().unwrap_or("").to_string();
    assert_eq!((class(0).as_str(), class(1).as_str(), class(2).as_str()), ("view", "moved", "value"));
    use aiworkspace_core::plan::Touched;
    assert!(!Touched::invalidates_data(&class(0)) && !Touched::invalidates_data(&class(1)) && Touched::invalidates_data(&class(2)));
    assert!(evs[0]["idempotency_key"].is_string(), "authors get their own key back");
    // bob's personal annotation: alice sees an empty event with the seq only
    assert_eq!(evs[3], json!({ "seq": head + 4 }));
    let for_bob = ws.get_changes(&bob(), &epoch, head, 100, true).unwrap();
    assert!(for_bob["changes"][0].get("idempotency_key").is_none(), "other people's keys are not visible");
    assert_eq!(for_bob["changes"][3]["ops"][0]["entity_id"], "bob-note");
    // doc.list_annotations: by target and by page, personal ones only for their owner
    let listed = |caller: aiworkspace_store::workspace::Caller, parent: Option<&str>| -> Vec<String> {
        let v = ws.list_annotations(&caller, &["notes".into(), "tasks".into()], parent).unwrap();
        v["annotations"].as_array().unwrap().iter().map(|a| a["entity_id"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(listed(alice(), None), ["note-budget"]);
    assert_eq!(listed(bob(), Some("data")), ["note-budget", "bob-note"]);
    let v = ws.list_annotations(&bob(), &[], Some("data")).unwrap();
    assert_eq!(v["annotations"][1]["content"]["anchor"], json!({ "state": "resolved", "level": "target" }));
    assert_eq!(ws.get_changes(&alice(), "ep_other", 0, 10, true).unwrap_err().code.as_str(), "EPOCH_MISMATCH");
    // paging: `more` until the head is reached
    let page = ws.get_changes(&alice(), &epoch, 0, 2, false).unwrap();
    assert_eq!((page["changes"].as_array().unwrap().len(), page["more"].as_bool()), (2, Some(true)));
}

/// V21: reads are authorized and projected; append-only can add but neither read nor change.
#[test]
fn v21_permissions() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let carol = aiworkspace_store::Caller::user("carol");
    let dave = aiworkspace_store::Caller::user("dave");
    assert_eq!(ws.read(&carol, "tasks", None).unwrap_err().code.as_str(), "NOT_FOUND", "no grant: the workspace does not exist");
    ws.grant(&alice(), "carol", None, &["read".into()]).unwrap();
    assert!(ws.read(&carol, "tasks", None).is_ok());
    let rv = cell_rev(&ws, &carol, "task-42", "owner");
    assert_eq!(code(&commit(&mut ws, &carol, set_cell("task-42", "owner", json!("x"), rv))), "PERMISSION_DENIED");
    // append-only on the table
    ws.grant(&alice(), "dave", Some("tasks"), &["append".into()]).unwrap();
    let r = ok(&mut ws, &dave, json!([{ "op": "table.insert_records", "source_id": "tasks", "records": [{ "record_id": "task-d1", "values": { "title": "dave 追加" } }] }]));
    assert_eq!(r["touched"].as_array().unwrap().len(), 1);
    assert_eq!(r["touched"][0]["selector"]["record_id"], "task-d1");
    assert_eq!(ws.read(&dave, "tasks", None).unwrap_err().code.as_str(), "NOT_FOUND");
    assert!(ws.query(&dave, &json!({ "source_id": "tasks" }), &env.svc.sources).is_err());
    assert_eq!(code(&commit(&mut ws, &dave, set_cell("task-d1", "title", json!("改"), 0))), "PERMISSION_DENIED");
    let stream = ws.get_changes(&dave, &ws.epoch.clone(), 0, 100, true).unwrap();
    assert!(stream["changes"].as_array().unwrap().iter().all(|e| e.get("ops").is_none()), "no content events without read");
    assert_eq!(ws.replica_bootstrap(&dave).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    // subtree grant: read only what lies under the folder (data lives in the data tree, phase two §4.5)
    ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "grp", "type_id": "buckyos.container", "parent_id": "data", "order_key": "k", "payload": { "kind": "folder" } },
        { "op": "tree.move", "entity_id": "project-info", "new_parent_id": "grp", "order_key": "a" }]));
    let erin = aiworkspace_store::Caller::user("erin");
    ws.grant(&alice(), "erin", Some("grp"), &["read".into()]).unwrap();
    assert!(ws.read(&erin, "project-info", None).is_ok());
    assert_eq!(ws.read(&erin, "tasks", None).unwrap_err().code.as_str(), "NOT_FOUND");
    assert_eq!(ws.grant(&carol, "carol", None, &["manage".into()]).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    ws.revoke(&alice(), "carol", None).unwrap();
    assert_eq!(ws.read(&carol, "tasks", None).unwrap_err().code.as_str(), "NOT_FOUND");
}

/// §2.11: write locks.
#[test]
fn write_locks() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "bob", None, &["read".into(), "update".into(), "structure".into()]).unwrap();
    let meta = ws.read(&alice(), "notes", None).unwrap()["meta_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_write_policy", "entity_id": "notes", "policy": "lock_required", "expect": { "rev": meta } }]));
    let edit = |tag: &str| json!([{ "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "end": true },
        "blocks": [{ "type": "paragraph", "attrs": { "block_id": tag }, "content": [{ "type": "text", "text": tag }] }] }]);
    assert_eq!(code(&commit(&mut ws, &alice(), edit("l1"))), "LOCK_REQUIRED");
    let got = ws.lock_acquire(&alice(), &["notes".into()], "s1").unwrap();
    let lock_id = got["locks"][0]["lock_id"].as_str().unwrap().to_string();
    assert_eq!(ws.lock_acquire(&bob(), &["notes".into()], "s1").unwrap_err().code.as_str(), "LOCK_HELD");
    assert_eq!(ws.lock_acquire(&alice(), &["notes".into()], "s2").unwrap_err().code.as_str(), "LOCK_HELD", "another tab of the same user competes");
    assert_eq!(code(&commit(&mut ws, &bob(), edit("l2"))), "LOCK_HELD");
    ok(&mut ws, &alice(), edit("l1"));
    assert_eq!(ws.read(&bob(), "notes", None).unwrap()["lock_holder"]["principal"], "alice");
    // renaming/moving the entity and annotating it are not content writes
    ok(&mut ws, &bob(), json!([{ "op": "tree.place", "entity_id": "notes", "order_key": "e3" }]));
    // lease expiry: bob takes over; alice's late commit is LOCK_LOST and not applied
    env.advance(61_000);
    assert_eq!(ws.lock_renew(&alice(), &[lock_id.clone()]).unwrap_err().code.as_str(), "LOCK_LOST");
    ws.lock_acquire(&bob(), &["notes".into()], "s1").unwrap();
    assert_eq!(code(&commit(&mut ws, &alice(), edit("l3"))), "LOCK_LOST");
    ok(&mut ws, &bob(), edit("l4"));
    // manager breaks the lock
    ws.lock_break(&alice(), "notes").unwrap();
    assert_eq!(code(&commit(&mut ws, &bob(), edit("l5"))), "LOCK_LOST");
    // idle release: held but never written for 10 minutes
    let l = ws.lock_acquire(&alice(), &["notes".into()], "s1").unwrap()["locks"][0]["lock_id"].as_str().unwrap().to_string();
    for _ in 0..19 {
        env.advance(30_000);
        ws.lock_renew(&alice(), &[l.clone()]).unwrap();
    }
    env.advance(40_000);
    let lost = ws.lock_renew(&alice(), &[l]).unwrap_err();
    assert_eq!((lost.code.as_str(), lost.data.unwrap()["lost"][0]["reason"].as_str().map(str::to_string)), ("LOCK_LOST", Some("idle".to_string())));
    // a locked container protects membership of its direct children, not their content
    let page_meta = ws.read(&alice(), "surface-main", None).unwrap()["meta_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_write_policy", "entity_id": "surface-main", "policy": "lock_required", "expect": { "rev": page_meta } }]));
    assert_eq!(code(&commit(&mut ws, &bob(), json!([{ "op": "tree.place", "entity_id": "cell-info", "order_key": "h5" }]))), "LOCK_REQUIRED");
    let rv = cell_rev(&ws, &bob(), "task-42", "owner");
    ok(&mut ws, &bob(), set_cell("task-42", "owner", json!("内容不受容器锁影响"), rv));
    assert_eq!(ws.lock_list(&bob(), None).unwrap()["locks"].as_array().unwrap().len(), 0);
}

/// R17 / V08: a view whose filter references a deleted field is reported and refused, never silently widened.
#[test]
fn dangling_view_configuration() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let cell_before = ws.read(&alice(), "cell-open-tasks", None).unwrap();
    assert_eq!(cell_before["content"]["diagnostics"], json!([]));
    let f = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_field", "field_id": "status" }))).unwrap();
    let del = ok(&mut ws, &alice(), json!([{ "op": "table.delete_field", "source_id": "tasks", "field_id": "status", "expect": { "rev": f["content"]["def_rev"] } }]));
    // the response names the views whose configuration now dangles
    let views: Vec<&str> = del["touched"].as_array().unwrap().iter().filter(|t| t["change"] == "view").map(|t| t["entity_id"].as_str().unwrap()).collect();
    assert!(views.contains(&"cell-open-tasks") && views.contains(&"cell-all-tasks"), "{views:?}");
    // the backend did not rewrite anyone's view configuration
    let cell = ws.read(&alice(), "cell-open-tasks", None).unwrap();
    assert_eq!(cell["content"]["payload"], cell_before["content"]["payload"]);
    assert_eq!(cell["content_rev"], cell_before["content_rev"]);
    let diag = cell["content"]["diagnostics"].as_array().unwrap();
    assert!(diag.iter().any(|d| d["key"] == "filter" && d["code"] == "FIELD_DELETED" && d["field_id"] == "status"), "{diag:?}");
    assert!(diag.iter().any(|d| d["key"] == "fields" && d["field_id"] == "status"));
    // a broken filter refuses the query: ignoring it would show more rows than the view allows
    let e = ws.query(&alice(), &json!({ "view_id": "cell-open-tasks" }), &env.svc.sources).unwrap_err();
    assert_eq!((e.code.as_str(), e.sub), ("INVALID_OPERATION", Some("VIEW_BROKEN")));
    // dangling columns do not change the row set: skipped and reported
    let all = ws.query(&alice(), &json!({ "view_id": "cell-all-tasks" }), &env.svc.sources).unwrap();
    assert_eq!(all["total"], 5);
    assert!(all["diagnostics"].as_array().unwrap().iter().any(|d| d["key"] == "fields" && d["field_id"] == "status"));
    assert!(all["rows"][0]["values"].get("status").is_none(), "residual values of a deleted field are unreadable");
    // one click removes the dead condition: an ordinary set_keys
    // (not done here) — instead undo the deletion: the untouched configuration is valid again
    let undo_req = json!({ "epoch": ws.epoch, "commit_id": del["commit_id"], "idempotency_key": "undo-del-field" });
    let u = ws.undo(&alice(), &undo_req);
    assert_eq!(u["status"], "accepted", "{u}");
    assert_eq!(ws.read(&alice(), "cell-open-tasks", None).unwrap()["content"]["diagnostics"], json!([]));
    assert_eq!(ws.query(&alice(), &json!({ "view_id": "cell-open-tasks" }), &env.svc.sources).unwrap()["total"], 4);
}

/// V03: illegal structure and oversized batches are rejected as a whole.
#[test]
fn v03_structure_and_limits() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let head = ws.head_seq;
    let grp = |id: &str, parent: &str| json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.container", "parent_id": parent, "order_key": "t", "payload": { "kind": "group" } });
    // dangling parent, illegal child type, bad id, bad order key, a cycle within one batch
    assert_eq!(code(&commit(&mut ws, &alice(), json!([grp("ok-1", "surface-main"), grp("bad-1", "no-such-parent")]))), "NOT_FOUND");
    assert_eq!(commit(&mut ws, &alice(), json!([grp("x-1", "tasks")]))["sub_code"], "CHILD_NOT_ALLOWED");
    assert_eq!(code(&commit(&mut ws, &alice(), json!([grp("Bad ID", "surface-main")]))), "INVALID_OPERATION");
    assert_eq!(commit(&mut ws, &alice(), json!([grp("a-1", "surface-main"), grp("a-2", "a-1"),
        { "op": "tree.move", "entity_id": "a-1", "new_parent_id": "a-2", "order_key": "a" }]))["sub_code"], "TREE_CYCLE");
    assert_eq!(code(&commit(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "c-1", "type_id": "buckyos.cell", "parent_id": "surface-main", "order_key": "u",
        "payload": { "source_ref": { "entity_id": "ghost" }, "view": { "type": "table" } } }]))), "REFERENCE_BROKEN");
    ws.limits.max_ops = 50;
    let many: Vec<Value> = (0..51).map(|i| grp(&format!("m-{i}"), "surface-main")).collect();
    assert_eq!(code(&commit(&mut ws, &alice(), json!(many))), "LIMIT_EXCEEDED");
    assert_eq!(ws.head_seq, head, "none of the rejected batches left anything behind");
    assert!(ws.read(&alice(), "ok-1", None).is_err());
    // deleting a referenced source returns the referrers
    let life = ws.read(&alice(), "project-info", None).unwrap()["life_rev"].clone();
    let r = commit(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "project-info", "expect": { "rev": life } }]));
    assert_eq!((code(&r), r["errors"][0]["data"]["referrers"][0]["entity_id"].as_str()), ("REFERENCE_BROKEN", Some("cell-info")));
    // ... unless the referrer goes in the same batch
    let cell_life = ws.read(&alice(), "cell-info", None).unwrap()["life_rev"].clone();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.delete", "entity_id": "cell-info", "expect": { "rev": cell_life } },
        { "op": "entity.delete", "entity_id": "project-info", "expect": { "rev": life } }]));
    assert_eq!(ws.verify_refs().unwrap()["ok"], true);
}
