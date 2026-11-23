use super::*;
use buckyos_api::*;
use rusqlite::Connection;
use system_config::sled_provider::SledStore;
use tempfile::tempdir;

fn current_system_config(path: &Path) -> sled::Db {
    drop(SledStore::open(path).unwrap());
    sled::open(path).unwrap()
}

fn run(root: &Path, work: &Path) -> Result<Vec<Change>, Failure> {
    prepare(root, &root.join("security"), work, &|_| Ok(false)).map(|result| result.changes)
}

fn database(root: &Path, path: &str, ddl: &str) -> Connection {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = Connection::open(path).unwrap();
    db.execute_batch(ddl).unwrap();
    db
}
#[test]
fn fresh_install_does_not_create_data_areas() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    assert!(run(root.path(), work.path()).unwrap().is_empty());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn host_account_requires_its_inline_schema() {
    use buckyos_api::host_account::{HostAccountFile, HOST_ACCOUNT_FILE};

    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let etc = root.path().join("etc");
    fs::create_dir(&etc).unwrap();
    let path = etc.join(HOST_ACCOUNT_FILE);
    let sidecar = etc.join("host_account.schema.json");
    let bytes = serde_json::to_vec(&HostAccountFile::V1 {
        account: "alice".into(),
    })
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"schema": "buckyos.host_account.v1", "account": "alice"})
    );
    fs::write(&path, &bytes).unwrap();
    assert!(run(root.path(), work.path()).unwrap().is_empty());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!sidecar.exists());

    fs::write(sidecar, r#"{"schema_version":1}"#).unwrap();
    for invalid in [
        r#"{"account":"alice"}"#,
        r#"{"schema":"buckyos.host_account.v2","account":"alice"}"#,
        r#"{"schema":"buckyos.host_account.v1","account":"alice","extra":true}"#,
        r#"{"schema":"buckyos.host_account.v1","account":"alice","account":"root"}"#,
    ] {
        fs::write(&path, invalid).unwrap();
        let failure = run(root.path(), work.path()).unwrap_err();
        assert!(failure.unsupported && failure.module == "host-account");
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
    }
}

#[test]
fn both_task_partitions_require_the_physical_version_and_full_schema() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let open = |partition: &str| {
        let path = format!("{partition}/task-manager/task-mgr-main.db");
        let db = database(root.path(), &path, TASK_MANAGER_RDB_SCHEMA_SQLITE);
        db.execute(
            "INSERT INTO task_store_meta VALUES ('schema_version',8)",
            [],
        )
        .unwrap();
        db
    };
    drop(open("data"));
    let local = open("local");
    assert!(run(root.path(), work.path()).unwrap().is_empty());
    local
        .execute("UPDATE task_store_meta SET meta_value=7", [])
        .unwrap();
    let err = run(root.path(), work.path()).unwrap_err();
    assert!(err.unsupported && err.reason.contains("7"));
    local
        .execute("UPDATE task_store_meta SET meta_value=8", [])
        .unwrap();
    local
        .execute_batch("ALTER TABLE task ADD COLUMN future TEXT")
        .unwrap();
    assert!(run(root.path(), work.path()).unwrap_err().unsupported);
}

#[test]
fn current_message_store_includes_wal_and_unversioned_store_is_rejected() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let rel = "data/msg-center/msg-center-main.db";
    let db = database(
        root.path(),
        rel,
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0",
    );
    for sql in [
        MSG_CENTER_RDB_SCHEMA_SQLITE,
        include_str!("../../../../frame/msg_center/src/groups.sql"),
        include_str!("../../../../frame/msg_center/src/notify.sql"),
    ] {
        db.execute_batch(sql).unwrap();
    }
    db.execute(
        "INSERT INTO group_states VALUES('g','legacy message body')",
        [],
    )
    .unwrap();
    let src = root.path().join(rel);
    let before = fs::read(&src).unwrap();
    let failed = run(root.path(), work.path()).unwrap_err();
    assert!(failed.unsupported && failed.reason.contains("schema 0"));
    db.pragma_update(None, "user_version", MSG_CENTER_RDB_SCHEMA_VERSION)
        .unwrap();
    assert!(run(root.path(), tempdir().unwrap().path())
        .unwrap()
        .is_empty());
    assert_eq!(before, fs::read(&src).unwrap());
    assert_eq!(
        db.query_row("SELECT state_json FROM group_states", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "legacy message body"
    );
    db.execute_batch("DROP TABLE notifies").unwrap();
    let error = run(root.path(), tempdir().unwrap().path()).unwrap_err();
    assert!(error.unsupported && error.module == "msg-center");
}

#[test]
fn configuration_is_not_applied_if_later_storage_fails() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let path = root.path().join("local/system_config");
    let signed = b"some.jwt.\x00opaque";
    {
        let db = current_system_config(&path);
        db.insert(b"boot/config", signed).unwrap();
        db.flush().unwrap();
    }
    let data = root.path().join("data/.fsdb/filedb.sqlite");
    fs::create_dir_all(data.parent().unwrap()).unwrap();
    fs::write(&data, b"unknown persisted format").unwrap();
    let before = fs::read(path.join("db")).unwrap();
    assert_eq!(
        run(root.path(), work.path()).unwrap_err().module,
        "nfs-metadata"
    );
    assert_eq!(before, fs::read(path.join("db")).unwrap());
    fs::remove_file(data).unwrap();
    let work = tempdir().unwrap();
    assert!(run(root.path(), work.path()).unwrap().is_empty());
    let original = sled::open(path).unwrap();
    assert_eq!(
        original.get(b"boot/config").unwrap().as_deref(),
        Some(signed.as_slice())
    );
    assert!(original.scan_prefix("__meta/revision/").next().is_none());
}

#[test]
fn service_settings_require_the_owners_schema() {
    let root = tempdir().unwrap();
    let path = root.path().join("local/system_config");
    for (key, schema, fields) in [
        (
            "services/aicc/settings",
            "buckyos.aicc.settings.v1",
            serde_json::json!({"providers": []}),
        ),
        (
            "users/alice/samba/settings",
            "buckyos.samba.settings.v1",
            serde_json::json!({"password": "test", "is_enable": true}),
        ),
    ] {
        let mut document = fields;
        for version in [None, Some("unrecognized"), Some(schema)] {
            if let Some(version) = version {
                document["schema"] = serde_json::Value::from(version);
            }
            let db = current_system_config(&path);
            db.insert(key, document.to_string().as_bytes()).unwrap();
            db.flush().unwrap();
            drop(db);
            let result = run(root.path(), tempdir().unwrap().path());
            if version == Some(schema) {
                assert!(result.unwrap().is_empty());
            } else {
                let error = result.unwrap_err();
                assert!(error.unsupported && error.area == key);
            }
        }
    }
}

fn set_config(root: &Path, key: &str, value: &[u8]) {
    let db = current_system_config(&root.join("local/system_config"));
    db.insert(key, value).unwrap();
    db.flush().unwrap();
}

#[test]
fn domain_hosting_requires_its_schema_version() {
    let hosting = |version: serde_json::Value, mode: &str| {
        serde_json::json!({
            "schema_version": version, "mode": mode, "certificate": "upload",
            "domain": "home.example.com",
        })
    };
    let mut unversioned = hosting(1.into(), "sn");
    unversioned.as_object_mut().unwrap().remove("schema_version");
    let mut extended = hosting(1.into(), "sn");
    extended["token"] = "x".into();
    for (value, accepted) in [
        (hosting(1.into(), "sn"), true),
        (hosting(1.into(), "manual"), true),
        (unversioned, false),
        (hosting(0.into(), "sn"), false),
        (hosting(2.into(), "sn"), false),
        (hosting(1.into(), "dns_api"), false),
        (hosting(1.into(), "future"), false),
        (extended, false),
    ] {
        let root = tempdir().unwrap();
        set_config(root.path(), DOMAIN_HOSTING_KEY, value.to_string().as_bytes());
        match run(root.path(), tempdir().unwrap().path()) {
            Ok(changes) => assert!(accepted && changes.is_empty(), "{value}"),
            Err(e) => assert!(
                !accepted
                    && e.unsupported
                    && e.module == "kernel-config"
                    && e.area == DOMAIN_HOSTING_KEY,
                "{value}: {e}"
            ),
        }
    }
}

#[test]
fn only_the_dns_credential_in_use_is_read_and_its_secret_is_not_reported() {
    const TOKEN: &str = "top-secret-token";
    let root = tempdir().unwrap();
    let hosting = |credential: &str| {
        serde_json::json!({
            "schema_version": 1, "mode": "dns_api", "certificate": "acme",
            "domain": "home.example.com", "provider": "cloudflare", "credential_id": credential,
        })
        .to_string()
    };
    let probe = || run(root.path(), tempdir().unwrap().path());
    let security = root.path().join("security");
    let path = cyfs_dns_provider::credential_path(&security, "home-1").unwrap();

    let unused = cyfs_dns_provider::credential_path(&security, "unused").unwrap();
    fs::create_dir_all(unused.parent().unwrap()).unwrap();
    fs::write(&unused, "not a credential").unwrap();
    assert!(probe().unwrap().is_empty());

    set_config(root.path(), DOMAIN_HOSTING_KEY, hosting("home-1").as_bytes());
    assert!(probe().unwrap().is_empty());

    cyfs_dns_provider::save_credential(
        &path,
        &cyfs_dns_provider::DnsCredential {
            provider: "cloudflare".into(),
            fields: BTreeMap::from([("api_token".into(), TOKEN.into())]),
            endpoint: None,
        },
    )
    .unwrap();
    let current = fs::read_to_string(&path).unwrap();
    assert!(probe().unwrap().is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), current);

    for invalid in [
        current.replace("buckyos.dns_credential.v1", "buckyos.dns_credential.v2"),
        current.replace("\"schema\"", "\"other\""),
        current.replace("\"cloudflare\"", "\"retired-service\""),
        current.replace("\"api_token\"", "\"renamed_field\""),
        current[..current.len() - 2].to_string(),
    ] {
        fs::write(&path, &invalid).unwrap();
        let failure = probe().unwrap_err();
        assert!(
            failure.unsupported
                && failure.module == "dns-credential"
                && failure.area == path.display().to_string(),
            "{failure}"
        );
        assert!(!failure.to_string().contains(TOKEN), "{failure}");
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
    }

    #[cfg(unix)]
    {
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&unused, &path).unwrap();
        assert!(probe().unwrap_err().reason.contains("symbolic link"));
    }
    set_config(root.path(), DOMAIN_HOSTING_KEY, hosting("../home-1").as_bytes());
    let failure = probe().unwrap_err();
    assert!(failure.unsupported && failure.area == DOMAIN_HOSTING_KEY, "{failure}");
}

#[test]
fn login_sources_and_account_bindings_are_verified_without_reporting_them() {
    use serde_json::json;
    const ISSUER: &str = "https://accounts.google.com";
    let did = "did:web:alice.google.com";
    let subject_key = oauth_subject_key(ISSUER, "stable-alice");
    let subject = |sub: &str| {
        json!({
            "identity": {
                "source_id": "google", "issuer": ISSUER, "sub": sub, "email": "alice@gmail.com",
                "label": "alice", "did": did, "webhost": "google.com",
            },
            "did": did, "email": "alice@gmail.com",
        })
        .to_string()
    };
    let source = json!({
        "id": "google", "display_name": "Google", "enabled": true, "kind": "google",
        "client_id": "client", "secret_ref": "google-demo",
    });
    let mut future_source = source.clone();
    future_source["future"] = json!(true);
    let subject_path = format!("{OAUTH_SUBJECTS_PREFIX}/{subject_key}");
    let label_path = oauth_label_key(did);
    let binding_path = oauth_binding_key(ISSUER, "stable-alice");
    let native_path = native_login_binding_key(did);
    let source_path = "services/control_panel/user_login_sources/alice/google";
    let retired = json!([{
        "id": "acme", "kind": "oidc", "webhost": "acme.example", "issuer": "https://acme.example",
    }]);
    for (key, valid, invalid) in [
        (
            SSO_SETTINGS_KEY,
            json!({"oauth_sources": [source], "did_login_enabled": false}).to_string(),
            vec![
                json!({"oauth_sources": [future_source]}).to_string(),
                json!({"did_login_enabled": "alice@gmail.com"}).to_string(),
            ],
        ),
        (
            RETIRED_OAUTH_SOURCES_KEY,
            retired.to_string(),
            vec!["{}".into(), json!([{"id": "acme"}]).to_string()],
        ),
        (
            &subject_path,
            subject("stable-alice"),
            vec![subject("another"), json!({"did": did}).to_string()],
        ),
        (
            &label_path,
            subject_key.clone(),
            vec![String::new(), json!(subject_key).to_string()],
        ),
        (
            &binding_path,
            "alice".into(),
            vec![String::new(), "../alice".into()],
        ),
        (
            &native_path,
            json!("alice").to_string(),
            vec!["alice".into(), json!("").to_string()],
        ),
        (
            source_path,
            json!(did).to_string(),
            vec![did.into(), json!("alice@gmail.com").to_string()],
        ),
    ] {
        let root = tempdir().unwrap();
        for invalid in invalid {
            set_config(root.path(), key, invalid.as_bytes());
            let failure = run(root.path(), tempdir().unwrap().path()).unwrap_err();
            assert!(
                failure.unsupported && failure.module == "sso-config" && failure.area == key,
                "{key} {invalid}: {failure}"
            );
            assert!(!failure.reason.contains("alice"), "{failure}");
        }
        set_config(root.path(), key, valid.as_bytes());
        assert!(run(root.path(), tempdir().unwrap().path())
            .unwrap()
            .is_empty());
        let db = sled::open(root.path().join("local/system_config")).unwrap();
        assert_eq!(db.get(key).unwrap().as_deref(), Some(valid.as_bytes()));
    }
}

#[test]
fn non_sqlite_declaration_fails_even_if_instance_was_never_created() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let path = root.path().join("local/system_config");
    {
        let db = current_system_config(&path);
        db.insert(
            b"services/example/spec",
            &br#"{"spec_config":{"rdb_instances":{"mydb":{"backend":"postgres"}}}}"#[..],
        )
        .unwrap();
        db.flush().unwrap();
    }
    let err = run(root.path(), work.path()).unwrap_err();
    assert!(err.unsupported && err.reason.contains("Postgres") && err.reason.contains("mydb"));
    assert!(!root.path().join("data").exists());
}

#[test]
fn workspaces_require_current_explicit_versions_for_both_databases() {
    use aiworkspace_store::schema::{DOC_DDL, LOCAL_ADDED_DDL, LOCAL_DDL};
    let root = tempdir().unwrap();
    let id = "ws_2611abcd";
    let dir = format!("data/aiworkspace/workspaces/{id}");
    let doc = database(root.path(), &format!("{dir}/doc.sqlite"), DOC_DDL);
    for (name, value) in [
        ("workspace_id", id),
        ("format_version", "1.0"),
        ("storage_schema_version", "7"),
    ] {
        doc.execute("INSERT INTO workspace_meta VALUES (?1,?2)", [name, value])
            .unwrap();
    }
    let local = database(root.path(), &format!("{dir}/local.sqlite"), LOCAL_DDL);
    local.execute_batch(LOCAL_ADDED_DDL).unwrap();
    let probe = || run(root.path(), tempdir().unwrap().path());
    assert!(probe().unwrap_err().unsupported);
    local.pragma_update(None, "user_version", 1).unwrap();
    assert!(probe().unwrap().is_empty());
    local
        .execute_batch(aiworkspace_store::entries::REGISTERED_ENTRIES_DDL)
        .unwrap();
    local
        .execute_batch("INSERT INTO entry_acceptances VALUES ('share', 'member', 0, 1)")
        .unwrap();
    assert!(probe().unwrap().is_empty());
    doc.execute_batch(aiworkspace_store::schema::RECORD_ORDER_INDEX)
        .unwrap();
    assert!(probe().unwrap().is_empty());
    doc.execute_batch("DROP INDEX table_records_created; CREATE INDEX table_records_created ON table_records(source_id)").unwrap();
    assert!(probe().unwrap_err().unsupported);
    doc.execute_batch("DROP INDEX table_records_created")
        .unwrap();

    local
        .execute_batch("DROP TABLE registered_entries")
        .unwrap();
    assert!(probe().unwrap_err().unsupported);
    local
        .execute_batch(aiworkspace_store::entries::REGISTERED_ENTRIES_DDL)
        .unwrap();
    local
        .execute_batch("ALTER TABLE entry_acceptances ADD COLUMN extra TEXT")
        .unwrap();
    assert!(probe().unwrap_err().unsupported);
    assert_eq!(
        local
            .query_row(
                "SELECT committed FROM entry_acceptances WHERE member='member'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    local.execute_batch("DROP TABLE entry_acceptances").unwrap();
    assert!(probe().unwrap_err().unsupported);
    local
        .execute_batch(aiworkspace_store::entries::REGISTERED_ENTRIES_DDL)
        .unwrap();
    assert!(probe().unwrap().is_empty());
    for old in ["5", "6", "8"] {
        doc.execute(
            "UPDATE workspace_meta SET value=? WHERE key='storage_schema_version'",
            [old],
        )
        .unwrap();
        assert!(probe().unwrap_err().unsupported);
        assert_eq!(
            doc.query_row(
                "SELECT value FROM workspace_meta WHERE key='storage_schema_version'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            old
        );
    }
}

#[test]
fn malformed_text_editor_data_aborts_after_verified_config_and_bucket_copies() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let sys = root.path().join("local/system_config");
    {
        let db = current_system_config(&sys);
        db.insert(b"boot/config", b"signed text").unwrap();
        db.flush().unwrap();
    }
    let storage = root.path().join("storage");
    fs::create_dir(&storage).unwrap();
    let cfg = storage.join("named_store.json");
    fs::write(
        &cfg,
        br#"{"epoch":1,"stores":[{"store_id":"local","path":"./local","weight":1}]}"#,
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let obj = runtime.block_on(async {
        let ndm = named_store::NamedDataMgr::get_store_mgr(&cfg, &Default::default())
            .await
            .unwrap();
        bucket_store::content::store_bytes(
            &ndm,
            "entry.json",
            br#"{"format":"buckyos.text-editor.recovery/9"}"#,
            Some("application/json"),
        )
        .await
        .unwrap()
        .1
    });
    drop(runtime);
    let index = database(
        root.path(),
        "data/nfs-server/bucket-index.db",
        bucket_store::db::SCHEMA,
    );
    index.execute("INSERT INTO buckets(bucket_id,kind,data_policy,scope,appid,owner,name,created_at) VALUES('r','user','app','user','text-editor.buckyos.bns.did','alice','recovery',0)",[]).unwrap();
    index.execute("INSERT INTO bindings(bucket_id,key,version,is_current,obj_id,created_at,created_by) VALUES('r','save/entry.json',1,1,?1,0,'alice')",[obj.to_string()]).unwrap();
    let err = run(root.path(), work.path()).unwrap_err();
    assert_eq!(err.module, "text-editor", "{err}");
    assert!(err.unsupported, "{err}");
    let db = sled::open(sys).unwrap();
    assert_eq!(
        db.get(SledStore::SCHEMA_KEY).unwrap().as_deref(),
        Some(&b"1"[..])
    );
    assert!(db.scan_prefix("__meta/revision/").next().is_none());
    assert_eq!(
        index
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[path = "../../registry_paths.rs"]
mod registry_paths;

#[test]
fn registry_detects_filename_globs_overlapping_subtrees() {
    use registry_paths::overlaps;
    assert!(overlaps("file/etc/*.json", "file/etc/x.zone_lkgs.json"));
    assert!(overlaps("file/etc/tool-*.json", "file/etc/*-receipt.*"));
    assert!(overlaps(
        "kv/users/*/apps",
        "kv/users/alice/apps/editor/settings"
    ));
    assert!(overlaps("file/etc", "file/etc/future/x"));
    assert!(!overlaps("file/etc/*.json", "file/etc/*.toml"));
    assert!(!overlaps("file/local/kmsg", "sled/local/kmsg"));
}

#[test]
fn an_existing_system_config_requires_its_own_schema() {
    let root = tempdir().unwrap();
    let path = root.path().join("local/system_config");
    for invalid in [None, Some(b"0"), Some(b"2")] {
        {
            let db = sled::open(&path).unwrap();
            db.insert(b"boot/config", b"opaque").unwrap();
            if let Some(version) = invalid {
                db.insert(SledStore::SCHEMA_KEY, version).unwrap();
            }
            db.flush().unwrap();
        }
        let failure = run(root.path(), tempdir().unwrap().path()).unwrap_err();
        assert!(failure.unsupported && failure.module == "system-config");
        assert!(SledStore::open(&path).is_err());
        let db = sled::open(&path).unwrap();
        assert_eq!(
            db.get(SledStore::SCHEMA_KEY).unwrap().as_deref(),
            invalid.map(|v| v.as_slice())
        );
    }
}

#[test]
fn service_specs_select_explicit_partition_connections_for_the_dispatcher() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let id = TASK_DISPATCHER_RDB_INSTANCE_ID;
    let dir = root.path().join("local/system_config");
    let spec = serde_json::json!({"spec_config":{"rdb_instances":{id:{"backend":"sqlite","connection":"sqlite:$partdata/custom.db?mode=rwc"}}}});
    {
        let config = current_system_config(&dir);
        config
            .insert(
                b"services/task-manager/spec",
                serde_json::to_vec(&spec).unwrap(),
            )
            .unwrap();
        config.flush().unwrap();
    }
    let path = "data/task-manager/custom.db";
    let db = database(root.path(), path, TASK_DISPATCHER_RDB_SCHEMA_SQLITE);
    assert!(run(root.path(), work.path()).unwrap_err().unsupported);
    db.pragma_update(None, "user_version", TASK_DISPATCHER_RDB_SCHEMA_VERSION)
        .unwrap();
    assert!(run(root.path(), tempdir().unwrap().path())
        .unwrap()
        .is_empty());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert!(!root.path().join("data/task-dispatcher").exists());
}

#[test]
fn fresh_owner_schemas_match_the_optional_offline_baselines() {
    let root = tempdir().unwrap();
    let work = tempdir().unwrap();
    let kmsg_path = root.path().join("local/kmsg");
    drop(kmsg::sled_msg_queue::SledMsgQueue::new_in_dir(&kmsg_path).unwrap());
    let fs_meta = root.path().join("var/fs_meta/fs_meta.db");
    fs::create_dir_all(fs_meta.parent().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        drop(
            fs_meta::fs_meta_service::FSMetaService::new(fs_meta.to_string_lossy().into_owned())
                .unwrap(),
        )
    });
    drop(rt);
    let index = root.path().join("bin/pkgs/meta_index.db");
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    package_lib::MetaIndexDb::create_or_open(&index).unwrap();
    let changes = run(root.path(), work.path()).unwrap();
    assert!(changes.is_empty());
    let conn = Connection::open(&index).unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    assert!(
        run(root.path(), tempdir().unwrap().path())
            .unwrap_err()
            .unsupported
    );
}

#[test]
fn current_and_invalid_kmsg_databases_are_never_rebased() {
    use kmsg::sled_msg_queue::SledMsgQueue;
    let root = tempdir().unwrap();
    let path = root.path().join("local/kmsg");
    drop(SledMsgQueue::new_in_dir(&path).unwrap());
    assert!(run(root.path(), tempdir().unwrap().path())
        .unwrap()
        .is_empty());
    for invalid in [None, Some(0u64), Some(2)] {
        {
            let db = sled::open(&path).unwrap();
            let meta = db.open_tree("meta").unwrap();
            match invalid {
                Some(v) => meta.insert(b"schema_version", &v.to_be_bytes()).unwrap(),
                None => meta.remove(b"schema_version").unwrap(),
            };
            db.flush().unwrap();
        }
        let failure = run(root.path(), tempdir().unwrap().path()).unwrap_err();
        assert!(failure.unsupported && failure.module == "kmsg");
        assert!(SledMsgQueue::new_in_dir(&path).is_err());
        let db = sled::open(&path).unwrap();
        let raw = db
            .open_tree("meta")
            .unwrap()
            .get(b"schema_version")
            .unwrap();
        assert_eq!(
            raw.as_deref(),
            invalid.map(u64::to_be_bytes).as_ref().map(|v| v.as_slice())
        );
    }
    {
        let db = sled::open(&path).unwrap();
        db.open_tree("meta")
            .unwrap()
            .insert(b"schema_version", &1u64.to_be_bytes())
            .unwrap();
        db.drop_tree("queue_subs").unwrap();
        db.flush().unwrap();
    }
    assert!(
        run(root.path(), tempdir().unwrap().path())
            .unwrap_err()
            .unsupported
    );
    assert!(SledMsgQueue::new_in_dir(&path).is_err());
    assert!(!sled::open(&path)
        .unwrap()
        .tree_names()
        .iter()
        .any(|n| n.as_ref() == b"queue_subs"));
}

#[test]
fn external_storage_authorizes_only_configured_databases_and_requires_the_volume() {
    let root = tempdir().unwrap();
    let other = tempdir().unwrap();
    let work = tempdir().unwrap();
    let external = other.path().join("named");
    fs::create_dir(&external).unwrap();
    let main = external.join("named_store.db");
    drop(named_store::NamedLocalStoreDB::new(main.to_string_lossy().into_owned()).unwrap());
    let cfg = root.path().join("storage/named_store.json");
    fs::create_dir(cfg.parent().unwrap()).unwrap();
    fs::write(
        &cfg,
        serde_json::to_vec(&serde_json::json!({"stores":[{"path":external}]})).unwrap(),
    )
    .unwrap();
    let source = other.path().join("metadata.sqlite");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        drop(
            fs_meta::fs_meta_service::FSMetaService::new(source.to_string_lossy().into_owned())
                .unwrap(),
        )
    });
    drop(rt);
    let fc = root.path().join("etc/fs_daemon.json");
    fs::create_dir(fc.parent().unwrap()).unwrap();
    fs::write(
        &fc,
        serde_json::to_vec(&serde_json::json!({"fs_meta_db_path":source})).unwrap(),
    )
    .unwrap();
    for p in [&source, &main] {
        assert_eq!(
            crate::absolute_root(p).unwrap().as_os_str(),
            p.canonicalize().unwrap().as_os_str()
        );
        assert!(configured_external_db(root.path(), p).unwrap());
        for extension in ["-wal", "-shm", "-journal"] {
            let mut sidecar = p.as_os_str().to_owned();
            sidecar.push(extension);
            assert!(
                configured_external_db(root.path(), Path::new(&sidecar)).unwrap(),
                "{}",
                Path::new(&sidecar).display()
            );
        }
    }
    assert!(!configured_external_db(root.path(), &external).unwrap());
    assert!(!configured_external_db(root.path(), &external.join("neighbor.db")).unwrap());
    assert!(
        configured_external_db(root.path(), &external.join("../named/named_store.db")).is_err()
    );
    let changes = run(root.path(), work.path()).unwrap();
    assert!(changes.is_empty());
    #[cfg(windows)]
    {
        let canonical_root = root.path().canonicalize().unwrap();
        assert!(configured_external_db(&canonical_root, &main.canonicalize().unwrap()).unwrap());
        assert!(configured_external_db(&canonical_root, &main).unwrap());
        assert!(configured_external_db(&canonical_root, &source.canonicalize().unwrap()).unwrap());
        let inside = root.path().join("var/internal-fs-meta.db");
        fs::write(
            &fc,
            serde_json::to_vec(&serde_json::json!({"fs_meta_db_path": inside})).unwrap(),
        )
        .unwrap();
        assert!(!configured_external_db(&canonical_root, &inside).unwrap());
        fs::write(
            &fc,
            serde_json::to_vec(&serde_json::json!({"fs_meta_db_path":source})).unwrap(),
        )
        .unwrap();
    }
    fs::remove_file(&main).unwrap();
    let retry = tempdir().unwrap();
    assert!(run(root.path(), retry.path())
        .unwrap_err()
        .reason
        .contains("mount"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&source, &main).unwrap();
        assert!(configured_external_db(root.path(), &main).is_err());
    }
}

#[test]
fn registered_external_opendan_sessions_are_read_only() {
    use libopendan::protocol::*;
    use libopendan::state::AgentLayout;
    use name_lib::{generate_ed25519_key_pair, AgentDocument, DID};
    use serde_json::json;
    let installation = tempdir().unwrap();
    let volume = tempdir().unwrap();
    let root = installation.path().canonicalize().unwrap();
    let location = volume
        .path()
        .canonicalize()
        .unwrap()
        .join("work-20261101-1");
    let sid = location.file_name().unwrap().to_str().unwrap();

    let owner = "alice";
    let did = DID::new("web", "agent.example.com");
    let (_, key) = generate_ed25519_key_pair();
    let doc = AgentDocument::new(
        did.clone(),
        DID::new("web", "alice.example.com"),
        serde_json::from_value(key).unwrap(),
    );
    let (object_id, _) =
        ndn_lib::build_named_object_by_json("agentdoc", &serde_json::to_value(&doc).unwrap());
    let agent_id = AgentId::from_agent_did(&did).unwrap();
    let spec = AgentSpec {
        schema_version: AGENT_SPEC_SCHEMA_VERSION,
        agent_id: agent_id.clone(),
        agent_did: did.clone(),
        agent_doc: doc,
        agent_doc_object_id: object_id.clone(),
        permission_mode: AgentPermissionMode::ReadWrite,
        binding: AgentServiceBinding {
            schema_version: AGENT_SPEC_SCHEMA_VERSION,
            agent_did: did.clone(),
            agent_doc_object_id: object_id,
            target_app_instance_id: format!("{agent_id}@{owner}").parse().unwrap(),
            service_name: "www".into(),
            generation: 1,
        },
        generation: 1,
    };
    {
        let db = current_system_config(&root.join("local/system_config"));
        db.insert(
            agent_spec_key(owner, &agent_id).as_bytes(),
            serde_json::to_vec(&spec).unwrap(),
        )
        .unwrap();
        db.flush().unwrap();
    }

    let layout = AgentLayout::new(
        root.join("data/home")
            .join(owner)
            .join("agents")
            .join(agent_id.as_str()),
    );
    fs::create_dir_all(layout.sessions_dir()).unwrap();
    let actor = json!({"principal":"app:example@alice"});
    // An ordinary drive path in a registry is valid when the installer root
    // arrived through Windows canonicalize() with a verbatim prefix.
    let registered_location = location.to_str().unwrap();
    #[cfg(windows)]
    let registered_location = registered_location
        .strip_prefix(r"\\?\")
        .unwrap_or(registered_location);
    let entry = json!({"session_id":sid, "location":registered_location, "kind":"work", "created_by":actor, "driver":actor, "status":SessionStatus::created(1)});
    let registry = layout.registry_entry(sid);
    fs::write(&registry, serde_json::to_vec(&entry).unwrap()).unwrap();

    let area = location.join(STATE_DIR);
    fs::create_dir_all(&area).unwrap();
    let write = |name: &str, value: serde_json::Value| {
        fs::write(area.join(name), serde_json::to_vec(&value).unwrap()).unwrap()
    };
    let mut cfg = json!({"schema":SESSION_CONFIG_SCHEMA,"session":{"session_id":sid,"agent_did":did,"kind":"work","created_at_ms":1,"created_by":actor,"driver":actor}});
    write(SESSION_CONFIG_FILE, cfg.clone());
    write(
        STATE_FILE,
        json!(SessionState::initial(
            WorklogBoundary {
                committed_seq: 0,
                committed_bytes: 0
            },
            1
        )),
    );
    write(SUMMARY_FILE, json!(SessionSummary::initial()));
    fs::write(area.join(WORKLOG_FILE), b"uncommitted tail").unwrap();
    let subdir = area.join("unknown");
    fs::create_dir(&subdir).unwrap();
    fs::write(subdir.join("user-file"), b"unowned").unwrap();
    let probe = || run(&root, tempdir().unwrap().path());
    // Snapshot the directory and the full contents, including the uncommitted
    // tail. Probing must never take a lease or call LiveSession recovery.
    let contents = || {
        let names: BTreeSet<_> = fs::read_dir(&area)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let records: Vec<_> = [
            SESSION_CONFIG_FILE,
            STATE_FILE,
            SUMMARY_FILE,
            WORKLOG_FILE,
            "unknown/user-file",
        ]
        .iter()
        .map(|n| fs::read(area.join(n)).unwrap())
        .collect();
        (names, records, fs::read(&registry).unwrap())
    };
    let initial = contents();
    let session = libopendan::SessionDir::open(&location).unwrap();
    assert_eq!(session.config().unwrap().session_id(), sid);
    assert!(probe()
        .unwrap()
        .iter()
        .all(|change| change.target.starts_with(&root)));
    assert_eq!(contents(), initial);
    assert!(!configured_external_db(&root, &area.join(STATE_FILE)).unwrap());

    cfg["schema"] = json!("opendan.session_config/999");
    write(SESSION_CONFIG_FILE, cfg.clone());
    let unknown = contents();
    let error = probe().unwrap_err();
    assert!(
        error.unsupported && error.module == "agent-storage",
        "{error}"
    );
    assert_eq!(contents(), unknown);
    cfg["schema"] = json!(SESSION_CONFIG_SCHEMA);
    write(SESSION_CONFIG_FILE, cfg);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let binding = area.join(BINDING_FILE);
        symlink(&registry, &binding).unwrap();
        assert!(probe().unwrap_err().reason.contains("symbolic link"));
        fs::remove_file(&binding).unwrap();
        fs::create_dir(&binding).unwrap();
        assert!(probe()
            .unwrap_err()
            .reason
            .contains("regular session record"));
        fs::remove_dir(&binding).unwrap();
    }

    fs::rename(&area, location.join("relocated")).unwrap();
    assert!(probe().is_err());
    let mark = layout.unreachable_mark(sid);
    fs::create_dir_all(mark.parent().unwrap()).unwrap();
    fs::write(&mark, b"1700000000000").unwrap();
    assert!(probe().is_ok());
    assert_eq!(fs::read(&mark).unwrap(), b"1700000000000");
    // Joining ".." to a verbatim Windows PathBuf normalizes it immediately.
    // Persist the literal registry string to exercise the reader's check.
    let separator = std::path::MAIN_SEPARATOR;
    let escaped =
        format!("{registered_location}{separator}missing{separator}..{separator}different");
    assert!(Path::new(&escaped)
        .components()
        .any(|c| c == Component::ParentDir));
    let mut moved = entry;
    moved["location"] = json!(escaped);
    fs::write(&registry, serde_json::to_vec(&moved).unwrap()).unwrap();
    assert!(probe().is_err());
}
