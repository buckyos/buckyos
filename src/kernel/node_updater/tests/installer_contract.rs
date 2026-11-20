use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

const UPDATER: &str = env!("CARGO_BIN_EXE_node_updater");

fn invoke(root: &Path, security: &Path, build: &str, output: &Path) -> (bool, Value) {
    let status = Command::new(UPDATER)
        .arg("migrate")
        .arg("--root")
        .arg(root)
        .arg("--target-build")
        .arg(build)
        .args(["--mode", "silent", "--profile", "full"])
        .arg("--security-root")
        .arg(security)
        .arg("--result")
        .arg(output)
        .status()
        .unwrap();
    (
        status.success(),
        serde_json::from_slice(&fs::read(output).unwrap()).unwrap(),
    )
}

#[test]
fn same_build_retry_and_prepare_failure_through_the_installer_entry_point() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("installation");
    let security = dir.path().join("external-security");
    let result = dir.path().join("result.json");
    fs::create_dir_all(root.join("etc")).unwrap();
    let build = String::from_utf8(
        Command::new(UPDATER)
            .arg("--version")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let build = build.trim();
    let value = b"{ \"schema\": \"buckyos.start_config.v1\", \"test_seed\": \"preserved byte for byte\" }\n";
    let source = root.join("etc/start_config.json");
    for invalid in [b"{}".as_slice(), br#"{"schema":"buckyos.start_config.v0"}"#] {
        fs::write(&source, invalid).unwrap();
        let (ok, report) = invoke(&root, &security, build, &result);
        assert!(!ok);
        assert_eq!(report["status"], "unsupported_schema");
        assert_eq!(report["recovery"], "unchanged");
        assert_eq!(fs::read(&source).unwrap(), invalid);
    }
    fs::write(&source, value).unwrap();
    let (ok, report) = invoke(&root, &security, "wrong-build", &result);
    assert!(!ok);
    assert_eq!(report["status"], "migration_failed");
    assert_eq!(report["target_build"], build);

    #[cfg(unix)]
    {
        let outside = dir.path().join("another-directory");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &security).unwrap();
        let (ok, report) = invoke(&root, &security, build, &result);
        assert!(!ok, "{report}");
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
        fs::remove_file(&security).unwrap();
    }

    // A malformed later data area prevents installation of the security-root move.
    fs::create_dir_all(root.join("local/node_daemon/executor")).unwrap();
    let (ok, report) = invoke(&root, &security, build, &result);
    assert!(!ok);
    assert_eq!(report["phase"], "prepare");
    assert_eq!(report["recovery"], "unchanged");
    assert_eq!(fs::read(&source).unwrap(), value);
    assert!(!security.exists());

    fs::remove_dir(root.join("local/node_daemon/executor")).unwrap();
    // A choice is mandatory when the external root already holds another configuration.
    fs::create_dir(&security).unwrap();
    let active = br#"{"schema":"buckyos.start_config.v1","test_seed":"other"}"#;
    fs::write(security.join("start_config.json"), active).unwrap();
    let (ok, report) = invoke(&root, &security, build, &result);
    assert!(!ok);
    assert_eq!(report["recovery"], "unchanged");
    assert!(report["reason"].as_str().unwrap().contains("archive"));
    assert_eq!(fs::read(&source).unwrap(), value);
    assert_eq!(
        fs::read(security.join("start_config.json")).unwrap(),
        active
    );
    fs::remove_file(security.join("start_config.json")).unwrap();
    let (ok, report) = invoke(&root, &security, build, &result);
    assert!(ok, "{report}");
    assert_eq!(report["status"], "success");
    assert_eq!(report["changed_areas"], 2);
    assert!(!source.exists());
    assert_eq!(fs::read(security.join("start_config.json")).unwrap(), value);
    let (ok, report) = invoke(&root, &security, build, &result);
    assert!(ok, "{report}");
    assert_eq!(report["changed_areas"], 0);
    assert!(!root.join("data").exists());

    let active_path = security.join("start_config.json");
    fs::write(&active_path, b"{}").unwrap();
    let (ok, report) = invoke(&root, &security, build, &result);
    assert!(!ok);
    assert_eq!(report["status"], "unsupported_schema");
    assert_eq!(report["recovery"], "unchanged");
    assert_eq!(fs::read(active_path).unwrap(), b"{}");

    let identity: Value = serde_json::from_slice(
        &Command::new(UPDATER)
            .arg("--identity")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(identity["target_build"], build);
    assert_eq!(identity["schemas"]["msg-center"], "13");
}

#[test]
fn record_install_marks_only_successful_deployment_with_the_installed_build() {
    use buckyos_api::completed_install::{
        read, runtime_build, target_build, COMPLETED_INSTALL_FILE,
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("node");
    let etc = root.join("etc");
    fs::create_dir_all(&etc).unwrap();
    let record = |build| {
        Command::new(UPDATER)
            .arg("record-install")
            .arg("--root")
            .arg(&root)
            .arg("--target-build")
            .arg(build)
            .status()
            .unwrap()
            .success()
    };
    let path = root.join(COMPLETED_INSTALL_FILE);
    assert!(read(&root).unwrap().is_none());
    assert!(!record("wrong-build"));
    assert!(!path.exists());

    let prior = br#"{"schema":"buckyos.completed_install.v1","target_build":"0.5+build10","runtime_build":"0.5+build10.old","completed_at":41}"#;
    fs::write(&path, prior).unwrap();
    for action in ["preflight", "migrate"] {
        let output = Command::new(UPDATER)
            .arg(action)
            .arg("--root")
            .arg(&root)
            .args([
                "--target-build",
                "wrong-build",
                "--mode",
                "silent",
                "--profile",
                "full",
            ])
            .arg("--security-root")
            .arg(root.join("security"))
            .arg("--result")
            .arg(dir.path().join("result"))
            .status()
            .unwrap();
        assert!(!output.success());
        assert_eq!(fs::read(&path).unwrap(), prior);
    }
    let result = dir.path().join("result");
    let (ok, report) = invoke(&root, &root.join("security"), target_build(), &result);
    assert!(ok);
    assert_eq!(report["changed_areas"], 0);
    assert_eq!(fs::read(&path).unwrap(), prior);
    assert!(record(target_build()));
    let receipt = read(&root).unwrap().unwrap();
    assert_eq!(receipt.target_build, target_build());
    assert_eq!(receipt.runtime_build, runtime_build());
    assert!(receipt.completed_at > 41);
    assert!(!root.join("local").exists());
    assert!(record(target_build()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o644);
        let original = dir.path().join("original");
        fs::write(&original, prior).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&original, &path).unwrap();
        assert!(read(&root).is_err());
        assert!(!record(target_build()));
        assert_eq!(fs::read(&original).unwrap(), prior);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&etc).unwrap();
        symlink(dir.path(), &etc).unwrap();
        assert!(read(&root).is_err());
        assert!(!record(target_build()));
    }
}

#[test]
fn validates_current_system_config_and_wal_backed_messages_without_rewriting_them() {
    use buckyos_api::{BuckyOSInfo, BUCKYOS_INFO_KEY, MSG_CENTER_RDB_SCHEMA_SQLITE};
    use rusqlite::Connection;

    let dir = tempdir().unwrap();
    let root = dir.path().join("node");
    let config = root.join("local/system_config");
    {
        drop(system_config::sled_provider::SledStore::open(&config).unwrap());
        let store = sled::open(&config).unwrap();
        let info = BuckyOSInfo::from_runtime("0.7.0+build261101", "beta", "amd64-linux", 1);
        store
            .insert(BUCKYOS_INFO_KEY, serde_json::to_vec(&info).unwrap())
            .unwrap();
        store
            .insert(b"boot/config", b"signed.payload.unchanged")
            .unwrap();
        store.flush().unwrap();
    }

    // Take a quiescent copy of the main file and the committed WAL. Closing
    // the fixture connection first would checkpoint it and miss this case.
    let fixture = dir.path().join("fixture.db");
    let writer = Connection::open(&fixture).unwrap();
    for ddl in [
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=13",
        MSG_CENTER_RDB_SCHEMA_SQLITE,
        include_str!("../../../frame/msg_center/src/groups.sql"),
        include_str!("../../../frame/msg_center/src/notify.sql"),
    ] {
        writer.execute_batch(ddl).unwrap();
    }
    writer
        .execute("INSERT INTO group_states VALUES('g','history')", [])
        .unwrap();
    let db = root.join("data/msg-center/msg-center-main.db");
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    fs::copy(&fixture, &db).unwrap();
    fs::copy(
        dir.path().join("fixture.db-wal"),
        db.with_extension("db-wal"),
    )
    .unwrap();
    drop(writer);

    let build = String::from_utf8(
        Command::new(UPDATER)
            .arg("--version")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let result = dir.path().join("result.json");
    let (ok, report) = invoke(&root, &root.join("security"), build.trim(), &result);
    assert!(ok, "{report}");
    assert_eq!(report["source_build"], "0.7.0+build261101");
    assert_eq!(report["changed_areas"], 0);
    {
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            13
        );
        assert_eq!(
            conn.query_row("SELECT state_json FROM group_states", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "history"
        );
        let store = sled::open(&config).unwrap();
        assert_eq!(
            store.get("boot/config").unwrap().unwrap().as_ref(),
            b"signed.payload.unchanged"
        );
        assert_eq!(
            store
                .get(system_config::sled_provider::SledStore::SCHEMA_KEY)
                .unwrap()
                .unwrap()
                .as_ref(),
            b"1"
        );
    }
    let (ok, report) = invoke(&root, &root.join("security"), build.trim(), &result);
    assert!(ok, "{report}");
    assert_eq!(report["changed_areas"], 0);
}
