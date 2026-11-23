//! Data sources on a real Workspace (AI Native 余下任务 §7.2): a pending source answers DATA_PENDING to SQL; a refresh
//! writes its data version, the same content again writes nothing; the last attempt and the freshness follow the
//! clock; a wish's write is not guarded by the sources it read (B7-2).

mod common;
use aiworkspace_store::source::Fetched;
use aiworkspace_store::wish::readset::{guarded, ReadSet};
use common::*;
use serde_json::json;

#[test]
fn a_source_from_pending_to_valid_to_expired() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "q", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "quotes",
          "payload": { "fields": [{ "field_id": "symbol", "name": "Symbol", "type": "text" }],
                       "source": { "kind": "zone", "path": "/home/alice/q.json", "def": { "meaning": "symbols", "primary_key": ["symbol"], "freshness": { "max_age_s": 60 } } } } },
        { "op": "entity.create", "entity_id": "own", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "own",
          "payload": { "fields": [{ "field_id": "n", "name": "N", "type": "number" }] } },
    ]));
    assert_eq!(sql(&ws, &alice(), "SELECT * FROM quotes").unwrap_err().sub, Some("DATA_PENDING"));
    // B7-2: a wish's write is guarded by the inputs people edit, not by the sources
    let mut reads = ReadSet::default();
    ws.read_ctx(|ctx| {
        reads.table(ctx, "quotes", None).unwrap();
        reads.table(ctx, "own", None).unwrap();
    });
    let inputs = ws.read_ctx(|ctx| reads.to_inputs(ctx)).unwrap();
    let kept = ws.read_ctx(|ctx| guarded(ctx, &inputs));
    assert!(!kept.is_empty() && kept.iter().all(|i| i["entity_id"] == "own"), "{kept:?}");
    // a refresh: the data and its version in one Commit; the same again writes nothing
    let fetched = Fetched::Table { fields: vec![json!({ "field_id": "symbol", "name": "Symbol", "type": "text" })],
                                   records: vec![json!({ "record_id": "k1", "values": { "symbol": "AAPL" } })] };
    let run = ws.source_refresh_start(&alice(), "quotes", "r1").unwrap()["run_id"].as_str().unwrap().to_string();
    let done = ws.source_apply(&run, &fetched, 10, None, &json!({ "program": { "name": "q", "version": "1" } })).unwrap();
    assert_eq!(done["result"]["status"], "accepted", "{done}");
    let m = meta(&ws, &alice(), "quotes");
    assert_eq!((m["data"]["state"].as_str(), m["data"]["version"]["records"].as_u64(), m["data"]["attempt"]["status"].as_str(), m["data"]["expired"].as_bool()),
               (Some("valid"), Some(1), Some("succeeded"), Some(false)), "{m}");
    let head = ws.head_seq;
    env.advance(30_000);
    let run = ws.source_refresh_start(&alice(), "quotes", "r2").unwrap()["run_id"].as_str().unwrap().to_string();
    assert_eq!(ws.source_apply(&run, &fetched, 10, None, &json!({ "program": { "name": "q", "version": "1" } })).unwrap()["result"]["status"], "unchanged");
    assert_eq!(ws.head_seq, head);
    // the last successful check counts for freshness; a minute past it, the data is expired
    env.advance(45_000);
    assert_eq!(meta(&ws, &alice(), "quotes")["data"]["expired"], false);
    env.advance(30_000);
    assert_eq!(meta(&ws, &alice(), "quotes")["data"]["expired"], true);
    let run = ws.source_refresh_start(&alice(), "quotes", "r3").unwrap()["run_id"].as_str().unwrap().to_string();
    ws.source_fail(&run, &aiworkspace_core::WsError::new(aiworkspace_core::Code::DependencyUnavailable, "gone")).unwrap();
    let m = meta(&ws, &alice(), "quotes");
    assert_eq!((m["data"]["attempt"]["status"].as_str(), m["data"]["attempt"]["error"]["detail"].as_str(), m["data"]["state"].as_str()), (Some("failed"), Some("gone"), Some("valid")), "{m}");
    let undo = json!({ "epoch": ws.epoch, "commit_id": done["result"]["commit_id"], "idempotency_key": "undo-zone-file" });
    let undone = ws.undo(&alice(), &undo);
    assert_eq!(undone["status"], "accepted", "{undone}");
}

fn contacts_source(auto: bool) -> serde_json::Value {
    let mut source = json!({ "kind": "builtin", "provider": "contacts", "def": { "meaning": "contacts", "primary_key": ["did"] } });
    if auto { source["refresh"] = json!({ "interval_s": 900 }); }
    source
}

fn contact_table(ws: &mut aiworkspace_store::Workspace, source: Option<&serde_json::Value>) -> u64 {
    let mut payload = json!({});
    if let Some(source) = source { payload["source"] = source.clone(); }
    ok(ws, &alice(), json!([{ "op": "entity.create", "entity_id": "contacts", "type_id": "buckyos.table-source",
        "parent_id": "data", "order_key": "a", "name": "contacts", "payload": payload }]))["seq"].as_u64().unwrap()
}

fn contacts(names: &[(&str, &str)]) -> Fetched {
    let fields = ["did", "name", "tags", "friend_state", "contact_ref"].map(|id| json!({
        "field_id": id, "name": id, "type": if id == "tags" { "json" } else { "text" }
    })).into_iter().collect();
    let records = names.iter().map(|(did, name)| json!({ "record_id": did, "values": { "did": did, "name": name,
        "tags": ["test"], "friend_state": "none", "contact_ref": format!("/record/{did}") } })).collect();
    Fetched::Table { fields, records }
}

fn done(ws: &mut aiworkspace_store::Workspace, id: &str, data: &Fetched) {
    let run = ws.source_apply(id, data, 32, None, &json!({})).unwrap();
    assert_eq!(run["state"], "succeeded", "{run}");
}

#[test]
fn contacts_require_local_authority_and_refresh_the_bound_owner() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "contacts", None).unwrap()["workspace_id"].as_str().unwrap().to_owned();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let auto = contacts_source(true);
    let rev = contact_table(&mut ws, Some(&auto));
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["binding"], "needs_rebind");
    assert!(ws.due_sources().unwrap().is_empty());
    assert!(ws.source_refresh_start(&alice(), "contacts", "forged").is_err());
    ws.grant(&alice(), "bob", Some("contacts"), &caps(&["read", "run"])).unwrap();
    assert!(ws.bind_contacts(&bob(), "contacts", "did:alice", &auto, false, rev, "bind").is_err());
    let first = ws.bind_contacts(&alice(), "contacts", "did:alice", &auto, false, rev, "bind").unwrap();
    let first = first["run_id"].as_str().unwrap();
    assert_eq!(ws.source_refresh_start(&bob(), "contacts", "also") .unwrap()["run_id"], first);
    let rows = contacts(&[("a","Same"), ("b","Same")]);
    done(&mut ws, first, &rows);
    assert_eq!(ws.source_refresh_start(&bob(), "contacts", "also").unwrap()["run_id"], first);
    let next = ws.source_refresh_start(&bob(), "contacts", "second").unwrap();
    let next = next["run_id"].as_str().unwrap();
    assert_eq!(ws.contact_read_context(next).unwrap(), ("alice".to_string(), "did:alice".to_string(), vec!["a".to_string(), "b".to_string()]));
    done(&mut ws, next, &contacts(&[]));
    assert_eq!(meta(&ws, &bob(), "contacts")["data"]["version"]["records"], 0);
    let next = ws.source_refresh_start(&bob(), "contacts", "back").unwrap();
    done(&mut ws, next["run_id"].as_str().unwrap(), &contacts(&[("a","Changed")]));
    let value = ws.read(&bob(), "contacts", Some(&json!({ "kind":"table_cell", "field_id":"name", "record_id":"a" }))).unwrap();
    assert_eq!(value["content"]["value"], "Changed", "{value}");
    let stale = ws.source_refresh_start(&bob(), "contacts", "stale").unwrap();
    let rev = meta(&ws, &alice(), "contacts")["version"]["content_rev"].as_u64().unwrap();
    let manual = contacts_source(false);
    ws.failpoint = aiworkspace_store::FailPoint::parse("before_txn");
    assert!(ws.bind_contacts(&alice(), "contacts", "did:alice", &manual, false, rev, "switch").is_err());
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["binding"], "bound");
    ws.failpoint = aiworkspace_store::FailPoint::parse("after_commit");
    for _ in 0..2 { assert_eq!(ws.bind_contacts(&alice(), "contacts", "did:alice", &manual, false, rev, "switch").unwrap()["configured"], true); }
    assert!(ws.due_sources().unwrap().is_empty());
    assert!(ws.source_apply(stale["run_id"].as_str().unwrap(), &rows, 32, None, &json!({})).is_err());
    let stale_id = stale["run_id"].as_str().unwrap();
    ws.invalidate_contact_run_binding(stale_id).unwrap();
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["binding"], "bound");
    drop(ws);
    let copy = env.svc.clone_workspace(&alice(), &id).unwrap();
    let copy = env.svc.workspace(copy["workspace_id"].as_str().unwrap()).unwrap();
    let mut copy = copy.lock().unwrap();
    assert_eq!(meta(&copy, &alice(), "contacts")["data"]["binding"], "needs_rebind");
    assert!(copy.source_refresh_start(&alice(), "contacts", "copied").is_err());
}

#[test]
fn one_shot_survives_restart_and_rebind_commits_only_against_confirmed_content() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "import", None).unwrap()["workspace_id"].as_str().unwrap().to_owned();
    let source = contacts_source(false);
    let first = {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let rev = contact_table(&mut ws, Some(&source));
        let r = ws.bind_contacts(&alice(), "contacts", "did:alice", &source, true, rev, "once").unwrap();
        assert_eq!(meta(&ws, &alice(), "contacts")["data"]["localize_after_refresh"], true);
        ws.source_run_set(r["run_id"].as_str().unwrap(), "running", None).unwrap();
        r["run_id"].as_str().unwrap().to_owned()
    };
    let env = env.reopen();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert_eq!(ws.resume_contact_runs().unwrap(), vec![first.clone()]);
    assert!(ws.resume_contact_runs().unwrap().is_empty());
    let rows = contacts(&[("a", "Alice")]);
    done(&mut ws, &first, &rows);
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["mode"], "local");
    let rev = meta(&ws, &alice(), "contacts")["version"]["content_rev"].as_u64().unwrap();
    let r = ws.bind_contacts(&alice(), "contacts", "did:alice", &source, false, rev, "rebind").unwrap();
    ok(&mut ws, &alice(), json!([{ "op":"table.set_values","source_id":"contacts","values":[{"record_id":"a","field_id":"name","value":"Local","expect":{"rev":rev}}] }]));
    assert!(ws.source_apply(r["run_id"].as_str().unwrap(), &rows, 32, None, &json!({})).is_err());
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["mode"], "local");
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["attempt"]["status"], "failed");
    let v = ws.read(&alice(), "contacts", Some(&json!({ "kind":"table_cell","record_id":"a","field_id":"name" }))).unwrap();
    assert_eq!(v["content"]["value"], "Local");
    let rev = meta(&ws, &alice(), "contacts")["version"]["content_rev"].as_u64().unwrap();
    let r = ws.bind_contacts(&alice(), "contacts", "did:alice", &source, false, rev, "retry").unwrap();
    ws.source_fail(r["run_id"].as_str().unwrap(), &aiworkspace_core::WsError::io("upstream")).unwrap();
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["attempt"]["error"]["detail"], "upstream");
    let r = ws.source_refresh_start(&alice(), "contacts", "read-again").unwrap();
    done(&mut ws, r["run_id"].as_str().unwrap(), &rows);
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["binding"], "bound");
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["mode"], "synced");
}

#[test]
fn reconnect_replaces_local_field_definitions_and_rows_atomically() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "local", None).unwrap()["workspace_id"].as_str().unwrap().to_owned();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    contact_table(&mut ws, None);
    ok(&mut ws, &alice(), json!([
        {"op":"table.add_field","source_id":"contacts","field":{"field_id":"old","name":"Former title","type":"text"}},
        {"op":"table.add_field","source_id":"contacts","field":{"field_id":"name","name":"Counter","type":"number"}},
        {"op":"entity.set_keys","entity_id":"contacts","keys":[{"key":"title_field_id","value":"old","expect":{"rev":0}}]},
        {"op":"table.insert_records","source_id":"contacts","records":[{"record_id":"a","values":{"old":"mine","name":101}},{"record_id":"other","values":{"old":"mine too"}}]}
    ]));
    let rev = meta(&ws, &alice(), "contacts")["version"]["content_rev"].as_u64().unwrap();
    let source = contacts_source(false);
    let r = ws.bind_contacts(&alice(), "contacts", "did:alice", &source, false, rev, "reconnect").unwrap();
    let mut invalid = contacts(&[("a","Alice")]);
    if let Fetched::Table { records, .. } = &mut invalid { records[0]["values"]["name"] = json!({}); }
    assert_eq!(ws.source_apply(r["run_id"].as_str().unwrap(), &invalid, 30, None, &json!({})).unwrap()["state"], "failed");
    assert_eq!(meta(&ws, &alice(), "contacts")["version"]["content_rev"], rev);
    let old = ws.read(&alice(), "contacts", Some(&json!({"kind":"table_cell","record_id":"a","field_id":"name"}))).unwrap();
    assert_eq!(old["content"]["value"], 101);
    let r = ws.source_refresh_start(&alice(), "contacts", "fixed").unwrap();
    done(&mut ws, r["run_id"].as_str().unwrap(), &contacts(&[("a","Alice")]));
    let v = ws.read(&alice(), "contacts", Some(&json!({"kind":"table_cell","record_id":"a","field_id":"name"}))).unwrap();
    assert_eq!(v["content"]["value"], "Alice");
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["version"]["records"], 1);
}

#[test]
fn large_contacts_snapshot_feeds_sql_and_wish_and_restore_requires_consent() {
    use aiworkspace_store::wish::context::{Stage, StageKind, StageOptions};

    let env = env();
    let id = env.svc.create_workspace(&alice(), "directory", None).unwrap()["workspace_id"].as_str().unwrap().to_owned();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let source = contacts_source(true);
    let rev = contact_table(&mut ws, Some(&source));
    let first = ws.bind_contacts(&alice(), "contacts", "did:alice", &source, false, rev, "bind").unwrap();
    let ids: Vec<String> = (0..1201).map(|n| format!("person{n:04}")).collect();
    let rows: Vec<_> = ids.iter().map(|id| (id.as_str(), "Same name")).collect();
    done(&mut ws, first["run_id"].as_str().unwrap(), &contacts(&rows));
    assert_eq!(sql(&ws, &alice(), "SELECT COUNT(*) FROM contacts").unwrap()["rows"], json!([[1201]]));
    assert_eq!(sql(&ws, &alice(), "SELECT tags FROM contacts WHERE did = 'person1200'").unwrap()["rows"], json!([["[\"test\"]"]]));
    ok(&mut ws, &alice(), json!([{
        "op": "entity.create", "entity_id": "directory-wish", "type_id": "buckyos.wish", "parent_id": "data", "order_key": "b",
        "payload": { "title": "directory", "prompt": "count contacts", "executor": "xllm", "output_mode": "overwrite",
                     "inputs": [{ "entity_id": "contacts", "name": "directory" }] }
    }]));
    let stage_dir = tempfile::tempdir().unwrap();
    let opts = StageOptions {
        kind: StageKind::Code, location: json!({}), request: json!({}), catalog: json!([]), controls: json!({ "controls": [] }),
        map_budget_chars: 4000, materialize: true, manifest: None,
    };
    let stage = Stage::open(ws.wish_snapshot(&alice()).unwrap(), "directory-wish", "contact-wish-run", stage_dir.path(), &opts).unwrap();
    let materialized = std::fs::read_to_string(stage_dir.path().join("context/entities/contacts/rows.jsonl")).unwrap();
    let data: Vec<serde_json::Value> = materialized.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(data.len(), 1201);
    let last = data.iter().find(|r| r["v"]["did"] == "person1200").unwrap();
    assert_eq!(last["v"]["tags"], json!(["test"]));
    drop(stage);
    let pkg = ws.export(&alice(), "share", true, None).unwrap();
    let file = fetched(&ws.objects, &pkg);
    let old_epoch = ws.epoch.clone();
    let stale = ws.source_refresh_start(&alice(), "contacts", "before-restore").unwrap();
    drop(ws);
    drop(h);
    env.svc.import(&alice(), &file, "restore", true).unwrap();
    let h = env.svc.workspace(&id).unwrap();
    let mut restored = h.lock().unwrap();
    assert_ne!(old_epoch, restored.epoch);
    assert!(restored.contact_run_is_current(stale["run_id"].as_str().unwrap()).is_err());
    assert_eq!(meta(&restored, &alice(), "contacts")["data"]["binding"], "needs_rebind");
    assert!(restored.due_sources().unwrap().is_empty());
    assert!(restored.source_refresh_start(&alice(), "contacts", "try-again").is_err());
    assert_eq!(sql(&restored, &alice(), "SELECT COUNT(*) FROM contacts").unwrap()["rows"], json!([[1201]]));
}

#[test]
fn undo_respects_contacts_content_source_ownership_and_partial_plans() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "undo", None).unwrap()["workspace_id"].as_str().unwrap().to_owned();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let source = contacts_source(false);
    let rev = contact_table(&mut ws, Some(&source));
    let first = ws.bind_contacts(&alice(), "contacts", "did:alice", &source, true, rev, "once").unwrap();
    done(&mut ws, first["run_id"].as_str().unwrap(), &contacts(&[("a","Original")]));
    let cell = json!({"kind":"table_cell", "record_id":"a", "field_id":"name"});
    let cell_rev = ws.read(&alice(), "contacts", Some(&cell)).unwrap()["content"]["rev"].clone();
    let edit = ok(&mut ws, &alice(), json!([
        {"op":"table.add_field","source_id":"contacts","field":{"field_id":"temporary","name":"Temporary","type":"text"}},
        {"op":"table.set_values","source_id":"contacts","values":[{"record_id":"a","field_id":"name","value":"Current","expect":{"rev":cell_rev}}]},
        {"op":"entity.set_keys","entity_id":"contacts","keys":[{"key":"description","value":"directory","expect":{"rev":0}}]}
    ]));
    let rev = edit["seq"].as_u64().unwrap();
    let run = ws.bind_contacts(&alice(), "contacts", "did:alice", &source, false, rev, "reconnect").unwrap();
    let refreshed = ws.source_apply(run["run_id"].as_str().unwrap(), &contacts(&[("a","Current"), ("b","Second")]), 32, None, &json!({})).unwrap();
    assert_eq!(refreshed["state"], "succeeded", "{refreshed}");
    let epoch = ws.epoch.clone();
    let undo = |id: &serde_json::Value, key: &str| json!({"epoch":epoch, "commit_id":id, "idempotency_key":key});
    let version = meta(&ws, &alice(), "contacts")["data"]["version"].clone();
    let denied = ws.undo(&alice(), &undo(&refreshed["result"]["commit_id"], "undo-refresh"));
    assert_eq!(denied["sub_code"], "SOURCE_READ_ONLY", "{denied}");
    let blocked = denied["blocked"].as_array().unwrap();
    for operation in ["table.restore_field", "table.delete_records"] {
        assert!(blocked.iter().any(|b| b["operation"]["op"] == operation && b["reason"]["sub_code"] == "SOURCE_READ_ONLY"), "{denied}");
    }
    let mut request = undo(&edit["commit_id"], "undo-local-edit");
    let plan = ws.undo(&alice(), &request);
    assert_eq!(plan["sub_code"], "SOURCE_READ_ONLY", "{plan}");
    assert_eq!(plan["undoable"].as_array().unwrap().len(), 1);
    request["mode"] = json!("partial");
    request["plan_digest"] = plan["plan_digest"].clone();
    assert_eq!(ws.undo(&alice(), &request)["status"], "accepted");
    assert_eq!(ws.read(&alice(), "contacts", Some(&cell)).unwrap()["content"]["value"], "Current");
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["version"], version);
    let entity = ws.read(&alice(), "contacts", None).unwrap();
    assert!(entity["content"]["payload"].get("description").is_none());
    let unbind = ok(&mut ws, &alice(), json!([{"op":"entity.unset_keys","entity_id":"contacts","keys":[{"key":"source","expect":{"rev":entity["content"]["key_revs"]["source"]}}]}]));
    ws.grant(&alice(), "bob", None, &caps(&["read","manage","update"])).unwrap();
    let denied = ws.undo(&bob(), &undo(&unbind["commit_id"], "manager"));
    assert_eq!(denied["code"], "PERMISSION_DENIED", "{denied}");
    let permitted = ws.undo(&alice(), &undo(&unbind["commit_id"], "owner"));
    assert_eq!(permitted["status"], "accepted", "{permitted}");
    assert_eq!(meta(&ws, &alice(), "contacts")["data"]["binding"], "needs_rebind");
    assert!(ws.source_refresh_start(&alice(), "contacts", "refresh").is_err());
}
