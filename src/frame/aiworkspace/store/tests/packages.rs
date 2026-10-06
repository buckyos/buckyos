//! Snapshots, packages, fork, privacy, assets, unknown content and URL tables
//! (V04, V05, V14, V20, V22, V24).

mod common;
use aiworkspace_store::urlsource::GeneratedSource;
use aiworkspace_store::workspace::Caller;
use aiworkspace_store::Service;
use common::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn export(env: &Env, id: &str, who: &Caller, mode: &str, self_contained: bool) -> (PathBuf, Value) {
    let h = env.svc.workspace(id).unwrap();
    let mut ws = h.lock().unwrap();
    let r = ws.export(who, mode, self_contained).unwrap();
    (PathBuf::from(r["path"].as_str().unwrap()), r["manifest"].clone())
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn unzip_all(path: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut all = Vec::new();
    for i in 0..zip.len() {
        zip.by_index(i).unwrap().read_to_end(&mut all).unwrap();
    }
    all
}

/// V04: export at an explicit checkpoint → import elsewhere → same ContentRoot; damaged packages change nothing.
#[test]
fn v04_export_import_roundtrip() {
    let env = env();
    let id = project(&env);
    let (pkg, manifest) = export(&env, &id, &alice(), "share", true);
    assert_eq!(manifest["self_contained"], true);
    assert_eq!(manifest["missing"], json!([]));
    assert!(manifest["types"].as_array().unwrap().len() >= 6);
    // edits after the checkpoint are not in the package
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let rv = cell_rev(&ws, &alice(), "task-42", "owner");
        ok(&mut ws, &alice(), set_cell("task-42", "owner", json!("导出之后"), rv));
    }
    // a different deployment
    let other = common::env();
    let r = other.svc.import(&bob(), &pkg, "restore", false).unwrap();
    assert_eq!(r["workspace_id"], json!(id));
    assert_eq!(r["content_root"], manifest["content_root"]);
    assert_eq!(r["head_seq"], 1, "history is not carried over");
    let h = other.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    assert_eq!(ws.checkpoint(&bob()).unwrap().content_root, manifest["content_root"].as_str().unwrap());
    assert_eq!(ws.read(&bob(), "tasks", Some(&json!({ "kind": "table_cell", "record_id": "task-42", "field_id": "owner" }))).unwrap()["content"]["value"], "林");
    assert_eq!(ws.read(&bob(), "diagram", None).unwrap()["content"]["availability"], "available");
    assert_eq!(ws.read(&bob(), "notes", None).unwrap()["content"]["content"]["content"][3]["attrs"]["ref"]["entity_id"], "cell-open-tasks");
    assert_eq!(ws.verify_refs().unwrap()["ok"], true);
    assert_eq!(ws.query(&bob(), &json!({ "view_id": "cell-open-tasks" }), &other.svc.sources).unwrap()["total"], 4);
    let epoch_imported = ws.epoch.clone();
    drop(ws);
    // the importer owns the copy; opening a package grants nothing on the original
    assert!(other.svc.workspace(&id).unwrap().lock().unwrap().read(&alice(), "tasks", None).is_err());
    // restoring over an existing Workspace: explicit replace + manage on the existing one
    assert_eq!(other.svc.import(&bob(), &pkg, "restore", false).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    assert_eq!(other.svc.import(&alice(), &pkg, "restore", true).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    let again = other.svc.import(&bob(), &pkg, "restore", true).unwrap();
    assert_ne!(again["epoch"], json!(epoch_imported), "a restore starts a new epoch");
    let h = other.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    assert_eq!(ws.get_changes(&bob(), &epoch_imported, 0, 10, false).unwrap_err().code.as_str(), "EPOCH_MISMATCH");
    assert!(ws.read(&bob(), "tasks", None).is_ok(), "grants of the replaced workspace are kept");
    drop(ws);
    assert_eq!(other.svc.import(&bob(), &pkg, "merge", false).unwrap_err().code.as_str(), "INVALID_OPERATION");

    // damaged package: a well-formed zip in which one object no longer hashes to its name
    let third = common::env();
    let bad = third.dir.path().join("bad.bcanvas");
    {
        use std::io::{Read, Write};
        let mut src = zip::ZipArchive::new(std::fs::File::open(&pkg).unwrap()).unwrap();
        let mut dst = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
        let mut tampered = false;
        for i in 0..src.len() {
            let mut e = src.by_index(i).unwrap();
            let name = e.name().to_string();
            let mut data = Vec::new();
            e.read_to_end(&mut data).unwrap();
            if !tampered && name.starts_with("objects/") && name.ends_with(".jobj") && contains(&data, "buckyos.table-source") {
                data = String::from_utf8(data).unwrap().replace("项目任务", "篡改任务").into_bytes();
                tampered = true;
            }
            dst.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            dst.write_all(&data).unwrap();
        }
        assert!(tampered);
        dst.finish().unwrap();
    }
    assert!(third.svc.import(&bob(), &bad, "restore", false).is_err());
    assert_eq!(third.svc.list_workspaces(&bob()).unwrap()["workspaces"], json!([]));
}

/// V05: a Fork is independent, has the same content root at birth, and carries no session state.
#[test]
fn v05_fork() {
    let env = env();
    let id = project(&env);
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.grant(&alice(), "bob", None, &["read".into(), "comment".into(), "export".into()]).unwrap();
        ok(&mut ws, &bob(), json!([{ "op": "entity.create", "entity_id": "bob-private", "type_id": "buckyos.annotation", "parent_id": "data",
            "order_key": "r", "scope": "personal", "payload": { "target": { "entity_id": "tasks" }, "kind": "note", "body": "私人" } }]));
    }
    let source_root = env.svc.workspace(&id).unwrap().lock().unwrap().checkpoint(&alice()).unwrap().content_root;
    let f = env.svc.fork(&alice(), &id).unwrap();
    let fid = f["workspace_id"].as_str().unwrap().to_string();
    assert_ne!(fid, id);
    assert_eq!(f["content_root"], json!(source_root), "internal references carry no workspace_id, so nothing is rewritten");
    let (lineage_src, lineage_fork) = {
        let a = env.svc.workspace(&id).unwrap().lock().unwrap().get_collab_state(&alice(), "notes").unwrap()["lineage_id"].clone();
        let b = env.svc.workspace(&fid).unwrap().lock().unwrap().get_collab_state(&alice(), "notes").unwrap()["lineage_id"].clone();
        (a, b)
    };
    assert_ne!(lineage_src, lineage_fork, "a fork starts new collaboration lineages");
    let h = env.svc.workspace(&fid).unwrap();
    let mut fork = h.lock().unwrap();
    assert_eq!(fork.info(&alice()).unwrap()["forked_from"]["workspace_id"], json!(id));
    assert!(fork.read(&bob(), "tasks", None).is_err(), "grants are not copied");
    assert!(fork.read(&alice(), "bob-private", None).is_err(), "personal entities are not copied");
    assert_eq!(fork.list_grants(&alice()).unwrap()["grants"].as_array().unwrap().len(), 1);
    assert_eq!(fork.read(&alice(), "note-budget", None).unwrap()["content"]["anchor"]["state"], "resolved");
    // diverge
    let rv = cell_rev(&fork, &alice(), "task-42", "owner");
    ok(&mut fork, &alice(), set_cell("task-42", "owner", json!("fork 里改的"), rv));
    assert_ne!(fork.checkpoint(&alice()).unwrap().content_root, source_root);
    drop(fork);
    let h = env.svc.workspace(&id).unwrap();
    let mut src = h.lock().unwrap();
    assert_eq!(src.checkpoint(&alice()).unwrap().content_root, source_root, "the source is untouched");
    assert_eq!(src.read(&alice(), "tasks", Some(&json!({ "kind": "table_cell", "record_id": "task-42", "field_id": "owner" }))).unwrap()["content"]["value"], "林");
    // bob (no manage, has export) may fork what he can read
    drop(src);
    assert!(env.svc.fork(&bob(), &id).is_ok());
    assert_eq!(env.svc.fork(&Caller::user("mallory"), &id).unwrap_err().code.as_str(), "NOT_FOUND");
}

/// V14: share packages and replicas carry no private drafts, no deleted text, no other people's personal content.
#[test]
fn v14_privacy_of_exports_and_replicas() {
    let env = env();
    let id = project(&env);
    let secret = "绝密-中间稿-SECRET-7f3k";
    let private = "bob-private-note-91xz";
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ws.grant(&alice(), "bob", None, &["read".into(), "comment".into(), "export".into()]).unwrap();
        // text that was shared and later deleted
        let idx = ws.read(&alice(), "notes", None).unwrap()["content"]["blocks"]["n-end"]["hash"].clone();
        let r = ok(&mut ws, &alice(), json!([{ "op": "richtext.replace_block", "entity_id": "notes", "block_id": "n-end", "expect": { "hash": idx },
            "node": { "type": "paragraph", "attrs": { "block_id": "n-end" }, "content": [{ "type": "text", "text": secret }] } }]));
        assert_eq!(r["status"], "accepted");
        let idx = ws.read(&alice(), "notes", None).unwrap()["content"]["blocks"]["n-end"].clone();
        ok(&mut ws, &alice(), json!([{ "op": "richtext.delete_blocks", "entity_id": "notes",
            "blocks": [{ "block_id": "n-end", "expect": { "hash": idx["hash"], "struct_rev": idx["struct_rev"] } }] }]));
        // bob's personal annotation: created, edited, deleted
        ok(&mut ws, &bob(), json!([{ "op": "entity.create", "entity_id": "bob-note", "type_id": "buckyos.annotation", "parent_id": "data",
            "order_key": "r", "scope": "personal", "payload": { "target": { "entity_id": "tasks" }, "kind": "note", "body": private } }]));
        let n = ws.read(&bob(), "bob-note", None).unwrap();
        ok(&mut ws, &bob(), json!([{ "op": "entity.set_keys", "entity_id": "bob-note",
            "keys": [{ "key": "body", "value": format!("{private}-v2"), "expect": { "rev": n["content"]["key_revs"]["body"] } }] }]));
        let n = ws.read(&bob(), "bob-note", None).unwrap();
        ok(&mut ws, &bob(), json!([{ "op": "entity.delete", "entity_id": "bob-note", "expect": { "rev": n["life_rev"] } }]));
        // the history really holds both secrets (so the tests below are meaningful)
        let raw = std::fs::read(ws.dir.join("doc.sqlite")).unwrap();
        let wal = std::fs::read(ws.dir.join("doc.sqlite-wal")).unwrap_or_default();
        assert!(contains(&raw, private) || contains(&wal, private));
    }
    // share export: content only, rebuilt lineage → the deleted text is gone
    let (pkg, manifest) = export(&env, &id, &alice(), "share", true);
    let all = unzip_all(&pkg);
    assert!(!contains(&all, secret), "deleted text must not travel in a share package");
    assert!(!contains(&all, private));
    assert_eq!(manifest["collab"], json!({}));
    // a personal backup keeps the original CRDT history (and says so through its mode)
    {
        let h = env.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "alice-note", "type_id": "buckyos.annotation", "parent_id": "data",
            "order_key": "s", "scope": "personal", "payload": { "target": { "entity_id": "notes", "selector": { "kind": "richtext_block", "block_id": "n-intro" } },
            "kind": "highlight", "body": "alice 自己的批注" } }]));
    }
    let (backup, bm) = export(&env, &id, &alice(), "personal_backup", true);
    assert_eq!(bm["personal_entities"], 1);
    {
        // restoring it brings back the owner's personal layer, as personal scope again
        let other = common::env();
        other.svc.import(&alice(), &backup, "restore", false).unwrap();
        let h = other.svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let n = ws.read(&alice(), "alice-note", None).unwrap();
        assert_eq!((n["scope"].as_str(), n["content"]["anchor"]["state"].as_str()), (Some("personal"), Some("resolved")));
        ws.grant(&alice(), "bob", None, &["read".into()]).unwrap();
        assert!(ws.read(&bob(), "alice-note", None).is_err());
    }
    let share_again = export(&env, &id, &alice(), "share", true).0;
    assert!(!contains(&unzip_all(&share_again), "alice 自己的批注"), "personal entities never enter a share package");
    assert_eq!(bm["export_mode"], "personal_backup");
    assert!(bm["collab"]["notes"]["lineage_id"].is_string());
    assert!(!contains(&unzip_all(&backup), private), "other people's personal entities never leave");
    // replica for alice: built from an allow-list, so bob's note is nowhere in the bytes, in any version
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let rep = ws.replica_bootstrap(&alice()).unwrap();
    let bytes = std::fs::read(rep["path"].as_str().unwrap()).unwrap();
    assert!(!contains(&bytes, private), "replica must not contain another principal's personal content");
    assert!(contains(&bytes, "完成第一期架构验证"));
    let conn = rusqlite::Connection::open(rep["path"].as_str().unwrap()).unwrap();
    let n = |t: &str| conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!((n("commits"), n("commit_ops"), n("richtext_updates"), n("snapshots"), n("entity_versions")), (0, 0, 0, 0, 0));
    assert!(n("entities") >= 13 && n("table_records") == 5);
    // residual values of a deleted field do not reach replicas either
    let f = ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_field", "field_id": "owner" }))).unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.delete_field", "source_id": "tasks", "field_id": "owner", "expect": { "rev": f["content"]["def_rev"] } }]));
    let rep2 = ws.replica_bootstrap(&alice()).unwrap();
    assert!(!contains(&std::fs::read(rep2["path"].as_str().unwrap()).unwrap(), "\"owner\":\"赵\""));
    assert!(ws.read(&alice(), "tasks", Some(&json!({ "kind": "table_record", "record_id": "task-42" }))).unwrap()["content"]["values"].get("owner").is_none());
    assert!(!contains(&unzip_all(&PathBuf::from(ws.export(&alice(), "share", true).unwrap()["path"].as_str().unwrap())), "\"owner\":\"赵\""));
}

/// V20: unknown types / versions are preserved, read-only, and never silently dropped.
#[test]
fn v20_unknown_content_is_preserved() {
    let env = env();
    let id = project(&env);
    let dir = env.svc.workspace(&id).unwrap().lock().unwrap().dir.clone();
    env.svc.close(&id);
    {
        // as written by a newer build: an extension type and a newer schema version
        let conn = rusqlite::Connection::open(dir.join("doc.sqlite")).unwrap();
        conn.execute_batch(
            "INSERT INTO entities (entity_id,type_id,schema_version,payload_json,key_revs_json,created_seq,meta_rev,content_rev,life_rev)
               VALUES ('chart-1','acme.chart',3,'{\"series\":[1,2,3],\"future\":{\"x\":null}}','{}',6,6,6,6);
             INSERT INTO tree_edges VALUES ('chart-1','data','p',NULL,6);
             INSERT INTO entities (entity_id,type_id,schema_version,payload_json,key_revs_json,created_seq,meta_rev,content_rev,life_rev)
               VALUES ('rec-v9','buckyos.record',9,'{\"shape\":\"unknown\"}','{}',6,6,6,6);
             INSERT INTO tree_edges VALUES ('rec-v9','data','q',NULL,6);",
        )
        .unwrap();
    }
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let c = ws.read(&alice(), "chart-1", None).unwrap();
    assert_eq!((c["degraded"].as_str(), &c["content"]["payload"]["series"]), (Some("MISSING_EXTENSION"), &json!([1, 2, 3])));
    assert_eq!(ws.read(&alice(), "rec-v9", None).unwrap()["degraded"], "UNSUPPORTED_VERSION");
    assert_eq!(code(&commit(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "chart-1", "keys": [{ "key": "series", "value": [], "expect": { "rev": 0 } }] }]))), "MISSING_EXTENSION");
    assert_eq!(code(&commit(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "rec-v9", "keys": [{ "key": "shape", "value": 1, "expect": { "rev": 0 } }] }]))), "UNSUPPORTED_VERSION");
    assert_eq!(code(&commit(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "c2", "type_id": "acme.chart", "parent_id": "data", "order_key": "s" }]))), "MISSING_EXTENSION");
    ok(&mut ws, &alice(), json!([{ "op": "tree.place", "entity_id": "chart-1", "order_key": "p5" }]));
    // export → import keeps the raw payload byte-for-byte in meaning
    let pkg = PathBuf::from(ws.export(&alice(), "share", true).unwrap()["path"].as_str().unwrap());
    drop(ws);
    let other = common::env();
    let r = other.svc.import(&alice(), &pkg, "new", false).unwrap();
    let h2 = other.svc.workspace(r["workspace_id"].as_str().unwrap()).unwrap();
    let ws2 = h2.lock().unwrap();
    assert_eq!(ws2.read(&alice(), "chart-1", None).unwrap()["content"]["payload"], json!({ "series": [1, 2, 3], "future": { "x": null } }));
    assert_eq!(ws2.read(&alice(), "rec-v9", None).unwrap()["schema_version"], 9);
    drop(ws2);
    // an unknown storage version is refused and the file is left exactly as it was
    env.svc.close(&id);
    {
        let conn = rusqlite::Connection::open(dir.join("doc.sqlite")).unwrap();
        conn.execute("UPDATE workspace_meta SET value = '99' WHERE key = 'storage_schema_version'", []).unwrap();
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").ok();
    }
    let before = std::fs::read(dir.join("doc.sqlite")).unwrap();
    assert_eq!(env.svc.workspace(&id).err().unwrap().code.as_str(), "UNSUPPORTED_VERSION");
    assert_eq!(std::fs::read(dir.join("doc.sqlite")).unwrap(), before);
}

/// V22: no dangling asset references; history keeps old assets retained; damage is explained.
#[test]
fn v22_assets() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let asset = |id: &str, obj: &str| json!([{ "op": "entity.create", "entity_id": id, "type_id": "buckyos.asset-ref", "parent_id": "data",
        "order_key": "u", "payload": { "object_id": obj, "media_type": "text/html", "size": 1 } }]);
    assert_eq!(code(&commit(&mut ws, &alice(), asset("a-none", "cyfile:00ff"))), "DEPENDENCY_UNAVAILABLE");
    assert!(ws.read(&alice(), "a-none", None).is_err(), "no dangling reference was created");
    // uploaded but the commit fails → staged only; after the TTL it is listed as unretained
    let orphan = ws.stage_asset(&alice(), b"orphan bytes").unwrap()["object_id"].as_str().unwrap().to_string();
    assert_eq!(commit(&mut ws, &alice(), json!([asset("a-1", &orphan)[0].clone(), { "op": "entity.rename", "entity_id": "ghost", "name": "x", "expect": { "rev": 0 } }]))["status"], "rejected");
    assert!(ws.read(&alice(), "a-1", None).is_err());
    assert_eq!(ws.list_unretained(&alice()).unwrap()["unretained"], json!([]), "still leased");
    // media type and size come from the verified bytes, not from the client
    let png = ws.stage_asset(&alice(), &[0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
    ok(&mut ws, &alice(), asset("a-2", png["object_id"].as_str().unwrap()));
    let a2 = ws.read(&alice(), "a-2", None).unwrap();
    assert_eq!((&a2["content"]["payload"]["media_type"], &a2["content"]["payload"]["size"]), (&json!("image/png"), &json!(7)));
    // replacing the image: the old object stays retained because history references it
    let newer = ws.stage_asset(&alice(), b"%PDF-new").unwrap()["object_id"].as_str().unwrap().to_string();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "a-2",
        "keys": [{ "key": "object_id", "value": newer, "expect": { "rev": a2["content"]["key_revs"]["object_id"] } }] }]));
    assert_eq!(ws.read(&alice(), "a-2", None).unwrap()["content"]["payload"]["media_type"], "application/pdf");
    // replacing the diagram: stale image dimensions go, derived metadata gets a new version
    let d = ws.read(&alice(), "diagram", None).unwrap();
    let other = ws.stage_asset(&alice(), b"plain text now").unwrap()["object_id"].as_str().unwrap().to_string();
    let r = ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "diagram",
        "keys": [{ "key": "object_id", "value": other, "expect": { "rev": d["content"]["key_revs"]["object_id"] } }] }]));
    let d2 = ws.read(&alice(), "diagram", None).unwrap();
    assert!(d2["content"]["payload"].get("image").is_none());
    assert_eq!((&d2["content"]["payload"]["media_type"], &d2["content"]["key_revs"]["size"]), (&json!("text/plain"), &r["seq"]));
    let undo_req = json!({ "epoch": ws.epoch, "commit_id": r["commit_id"], "idempotency_key": "undo-replace" });
    assert_eq!(ws.undo(&alice(), &undo_req)["status"], "accepted");
    assert_eq!(ws.read(&alice(), "diagram", None).unwrap()["content"]["payload"]["object_id"], d["content"]["payload"]["object_id"]);
    env.advance(25 * 3600 * 1000);
    let un = ws.list_unretained(&alice()).unwrap();
    let ids: Vec<&str> = un["unretained"].as_array().unwrap().iter().map(|u| u["object_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![orphan.as_str()], "only the never-referenced upload is collectable");
    assert_eq!(code(&commit(&mut ws, &alice(), asset("a-3", &orphan))), "DEPENDENCY_UNAVAILABLE", "an expired lease cannot be referenced");
    // tamper with the stored bytes → corrupt; remove them → missing; the document still opens
    let obj = ws.read(&alice(), "diagram", None).unwrap()["content"]["payload"]["object_id"].as_str().unwrap().to_string();
    let chunk_path = ws.objects.chunk_path(&ws.objects.file_chunk(&obj).unwrap()).unwrap();
    std::fs::write(&chunk_path, b"tampered").unwrap();
    assert_eq!(ws.read(&alice(), "diagram", None).unwrap()["content"]["availability"], "corrupt");
    std::fs::remove_file(&chunk_path).unwrap();
    assert_eq!(ws.read(&alice(), "diagram", None).unwrap()["content"]["availability"], "missing");
    // a self-contained export reports what it could not include instead of pretending
    let m = ws.export(&alice(), "share", true).unwrap()["manifest"].clone();
    assert_eq!(m["self_contained"], false);
    assert_eq!(m["missing"][0]["id"], json!(obj));
    let pkg = PathBuf::from(ws.export(&alice(), "share", true).unwrap()["path"].as_str().unwrap());
    drop(ws);
    let other = common::env();
    let r = other.svc.import(&alice(), &pkg, "new", false).unwrap();
    let h2 = other.svc.workspace(r["workspace_id"].as_str().unwrap()).unwrap();
    assert_eq!(h2.lock().unwrap().read(&alice(), "diagram", None).unwrap()["content"]["availability"], "missing");
}

fn with_source(env: &mut Env) -> Arc<GeneratedSource> {
    let src = Arc::new(GeneratedSource::default());
    env.svc.sources.register("fixture", src.clone());
    src
}

fn url_table(ws: &mut aiworkspace_store::Workspace, id: &str, url: &str, query: Value) {
    ok(ws, &alice(), json!([{ "op": "entity.create", "entity_id": id, "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "v",
        "payload": { "data_mode": "url_query", "title_field_id": "event_id",
            "source_ref": { "kind": "url_query", "source_url": url, "query": query, "version": { "mode": "live_head" }, "consistency": "best_effort" },
            "fields": [ { "field_id": "event_id", "name": "事件", "type": "text" }, { "field_id": "created_at", "name": "时间", "type": "datetime" },
                        { "field_id": "amount", "name": "金额", "type": "decimal", "scale": 2 }, { "field_id": "project_id", "name": "项目", "type": "text" } ] } }]));
}

/// V24: a big table addressed by URL + query, without ObjectIds.
#[test]
fn v24_url_query_table() {
    let mut env = env();
    let src = with_source(&mut env);
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ws.grant(&alice(), "bob", None, &["read".into()]).unwrap();
    let url = "fixture://events?rows=1000000&row_key=1&snapshot=1&deny=bob";
    url_table(&mut ws, "events", url, json!({ "fields": ["event_id", "created_at", "amount", "project_id"] }));
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "cell-events", "type_id": "buckyos.cell", "parent_id": "surface-main", "order_key": "w",
        "payload": { "source_ref": { "entity_id": "events" }, "view": { "type": "table" }, "title": "项目 42 的事件",
                     "filter": { "op": "cmp", "field_id": "project_id", "operator": "eq", "value": "project-42" } } }]));
    assert_eq!(src.rows_generated.load(Ordering::Relaxed), 0, "saving the definition and the view fetched nothing");
    // on-demand paging through the view; only the requested page is produced
    let p1 = ws.query(&alice(), &json!({ "view_id": "cell-events", "limit": 20 }), &env.svc.sources).unwrap();
    assert_eq!(p1["rows"].as_array().unwrap().len(), 20);
    assert_eq!(p1["rows"][0]["values"]["project_id"], "project-42");
    assert_eq!(p1["rows"][0]["record_id"], "ev-000000042");
    assert_eq!((p1["data_mode"].as_str(), p1["consistency"].as_str(), p1["source_revision"].as_str()), (Some("url_query"), Some("snapshot"), Some("snap-1000000")));
    assert_eq!(src.rows_generated.load(Ordering::Relaxed), 20);
    let p2 = ws.query(&alice(), &json!({ "view_id": "cell-events", "limit": 20, "cursor": p1["next_cursor"], "source_revision": p1["source_revision"], "consistency": "snapshot" }), &env.svc.sources).unwrap();
    assert_eq!(p2["rows"][0]["record_id"], "ev-000001042");
    assert!(src.rows_generated.load(Ordering::Relaxed) < 100, "never a full fetch of the million rows");
    // cache isolation: same URL, different query / different principal
    let again = ws.query(&alice(), &json!({ "view_id": "cell-events", "limit": 20, "cursor": p1["next_cursor"], "source_revision": p1["source_revision"] }), &env.svc.sources).unwrap();
    assert_eq!(again["from_cache"], true);
    let other_q = ws.query(&alice(), &json!({ "source_id": "events", "limit": 20, "cursor": p1["next_cursor"], "source_revision": p1["source_revision"],
        "filter": { "op": "cmp", "field_id": "project_id", "operator": "eq", "value": "project-7" } }), &env.svc.sources).unwrap();
    assert!(other_q.get("from_cache").is_none());
    assert_eq!(other_q["rows"][0]["values"]["project_id"], "project-7");
    assert_eq!(ws.query(&bob(), &json!({ "view_id": "cell-events", "limit": 20, "cursor": p1["next_cursor"], "source_revision": p1["source_revision"] }), &env.svc.sources)
        .unwrap_err().code.as_str(), "PERMISSION_DENIED", "another principal never gets alice's cached page");
    // filters the source cannot push down are refused, not dropped
    assert_eq!(ws.query(&alice(), &json!({ "source_id": "events", "filter": { "op": "cmp", "field_id": "amount", "operator": "gt", "value": "5" } }), &env.svc.sources)
        .unwrap_err().code.as_str(), "INVALID_OPERATION");
    // read-only in this phase; the definition itself is ordinary versioned document state
    assert_eq!(commit(&mut ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "events", "records": [{ "record_id": "x", "values": {} }] }]))["sub_code"], "SOURCE_READ_ONLY");
    let ev = ws.read(&alice(), "events", None).unwrap();
    let rev0 = ev["content_rev"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "events", "keys": [{ "key": "description", "value": "事件流", "expect": { "rev": 0 } }] }]));
    assert_ne!(ws.read(&alice(), "events", None).unwrap()["content_rev"], rev0);
    // a source without snapshots or row keys does not get those abilities by wishful thinking
    url_table(&mut ws, "events-live", "fixture://live?rows=500&row_key=0&snapshot=0", json!({}));
    let caps = ws.source_capabilities(&alice(), "events-live", &env.svc.sources).unwrap();
    assert_eq!((&caps["capabilities"]["consistent_paging"], &caps["capabilities"]["stable_row_key"]), (&json!(false), &json!(false)));
    let live = ws.query(&alice(), &json!({ "source_id": "events-live", "limit": 5 }), &env.svc.sources).unwrap();
    assert_eq!(live["consistency"], "best_effort");
    assert!(live["rows"][0].get("record_id").is_none(), "page positions are never passed off as record ids");
    assert!(live.get("source_revision").is_none());
    assert_eq!(ws.query(&alice(), &json!({ "source_id": "events-live", "limit": 5, "consistency": "snapshot" }), &env.svc.sources)
        .unwrap_err().code.as_str(), "DEPENDENCY_UNAVAILABLE");
    // unknown scheme: the definition is kept, the source is reported unavailable, nothing is fetched
    url_table(&mut ws, "events-http", "https://data.example.com/datasets/events", json!({}));
    assert_eq!(ws.source_capabilities(&alice(), "events-http", &env.svc.sources).unwrap()["available"], false);
    assert_eq!(ws.query(&alice(), &json!({ "source_id": "events-http" }), &env.svc.sources).unwrap_err().code.as_str(), "DEPENDENCY_UNAVAILABLE");
    assert!(commit(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "bad-url", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "x",
        "payload": { "data_mode": "url_query", "source_ref": { "kind": "url_query", "source_url": "https://user:pw@host/x" } } }]))["status"] == "rejected", "credentials never go into a document");

    // export keeps the definition, not the data; import does not go online
    let before = src.queries.load(Ordering::Relaxed);
    let exported = ws.export(&alice(), "share", true).unwrap();
    let m = &exported["manifest"];
    assert_eq!(m["self_contained"], false, "external rows are not inside the package");
    assert_eq!(m["external_sources"].as_array().unwrap().len(), 3);
    assert_eq!(m["external_sources"][0]["slice_included"], false);
    let pkg = PathBuf::from(exported["path"].as_str().unwrap());
    drop(ws);
    let mut other = common::env();
    let src2 = with_source(&mut other);
    let r = other.svc.import(&alice(), &pkg, "new", false).unwrap();
    assert_eq!(r["content_root"], m["content_root"], "definition round-trips exactly");
    assert_eq!((src.queries.load(Ordering::Relaxed), src2.queries.load(Ordering::Relaxed)), (before, 0), "neither export nor import queried the source");
    let h2 = other.svc.workspace(r["workspace_id"].as_str().unwrap()).unwrap();
    let mut ws2 = h2.lock().unwrap();
    assert_eq!(ws2.read(&alice(), "events", None).unwrap()["content"]["payload"]["source_ref"]["source_url"], url);
    assert_eq!(ws2.query(&alice(), &json!({ "view_id": "cell-events", "limit": 3 }), &other.svc.sources).unwrap()["rows"][0]["record_id"], "ev-000000042");
    // offline: content that was never fetched is not presented as available
    src2.offline.store(true, Ordering::Relaxed);
    assert_eq!(ws2.query(&alice(), &json!({ "view_id": "cell-events", "limit": 3, "cursor": "999" }), &other.svc.sources).unwrap_err().code.as_str(), "DEPENDENCY_UNAVAILABLE");
    src2.offline.store(false, Ordering::Relaxed);
    // materialize just the chosen slice into an embedded table: that slice gets a content identity
    let page = ws2.query(&alice(), &json!({ "view_id": "cell-events", "limit": 3 }), &other.svc.sources).unwrap();
    let records: Vec<Value> = page["rows"].as_array().unwrap().iter().map(|r| json!({ "record_id": r["record_id"], "values": r["values"] })).collect();
    ok(&mut ws2, &alice(), json!([
        { "op": "entity.create", "entity_id": "events-slice", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "y",
          "payload": { "description": format!("slice of {url} @ {}", page["source_revision"]), "fields": [
              { "field_id": "event_id", "name": "事件", "type": "text" }, { "field_id": "created_at", "name": "时间", "type": "datetime" },
              { "field_id": "amount", "name": "金额", "type": "decimal", "scale": 2 }, { "field_id": "project_id", "name": "项目", "type": "text" }] } },
        { "op": "table.insert_records", "source_id": "events-slice", "records": records }]));
    let cp = ws2.checkpoint(&alice()).unwrap();
    assert!(cp.objects["events-slice"].starts_with("jobj:"));
    assert_eq!(cp.unresolved.len(), 3, "the URL dependencies are still reported as not fixed");
    let _ = Service::open; // keep the import used on all cfgs
}
