use crate::reader::{FlushReadPosError, LogDirReader};
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn new_temp_root(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "buckyos/slog_pipeline_tests/{}_{}_{}",
        prefix,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn make_record(service: &str, time: u64, content: &str) -> SystemLogRecord {
    SystemLogRecord {
        level: LogLevel::Info,
        target: service.to_string(),
        time,
        file: Some("pipeline_service_churn_test.rs".to_string()),
        line: Some(1),
        content: content.to_string(),
    }
}

fn prepare_service_logs(
    log_root: &Path,
    service: &str,
    records: &[SystemLogRecord],
) -> Result<PathBuf, String> {
    let service_dir = log_root.join(service);
    std::fs::create_dir_all(&service_dir).map_err(|e| {
        format!(
            "failed to create service log dir {}: {}",
            service_dir.display(),
            e
        )
    })?;

    let meta = LogMeta::open(&service_dir)?;
    let file_name = format!("{}.1.log", service);
    meta.append_new_file(&file_name)
        .map_err(|e| format!("append_new_file failed: {}", e))?;

    let mut content = String::new();
    for record in records {
        content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }
    let log_file = service_dir.join(&file_name);
    std::fs::write(&log_file, &content)
        .map_err(|e| format!("failed to write log file {}: {}", log_file.display(), e))?;
    meta.update_current_write_index(content.len() as u64)
        .map_err(|e| format!("update_current_write_index failed: {}", e))?;
    Ok(service_dir)
}

#[test]
fn test_log_dir_reader_stays_stable_under_service_churn() {
    let root = new_temp_root("service_churn");
    let reader = LogDirReader::open(&root, vec![]).unwrap();

    let rounds = 12usize;
    for round in 0..rounds {
        let service = format!("svc_churn_{}", round);
        let service_records = vec![make_record(
            &service,
            1722001700000 + round as u64,
            &format!("churn-record-{}", round),
        )];

        let service_dir = prepare_service_logs(&root, &service, &service_records).unwrap();
        reader.update_dir().unwrap();

        let mut seen = false;
        for _ in 0..5 {
            let items = reader.try_read_records(100).unwrap();
            if let Some(item) = items.iter().find(|item| item.id == service) {
                assert_eq!(item.records.len(), service_records.len());
                reader.flush_read_pos(&service).unwrap();
                seen = true;
                break;
            }
        }
        assert!(
            seen,
            "expected to discover churn service {} after update_dir",
            service
        );

        std::fs::remove_dir_all(&service_dir).unwrap();
        reader.update_dir().unwrap();

        let err = reader.flush_read_pos(&service).unwrap_err();
        assert!(matches!(err, FlushReadPosError::NotFound { .. }));

        let items_after = reader.try_read_records(100).unwrap();
        assert!(
            items_after.iter().all(|item| item.id != service),
            "removed service {} should not appear in later reads",
            service
        );
    }

    let stable_service = "svc_stable_after_churn";
    let stable_records = vec![
        make_record(stable_service, 1722001710001, "stable-1"),
        make_record(stable_service, 1722001710002, "stable-2"),
    ];
    let _stable_dir = prepare_service_logs(&root, stable_service, &stable_records).unwrap();
    reader.update_dir().unwrap();

    let mut saw_stable = false;
    for _ in 0..6 {
        let items = reader.try_read_records(100).unwrap();
        if let Some(item) = items.iter().find(|item| item.id == stable_service) {
            assert_eq!(item.records.len(), stable_records.len());
            reader.flush_read_pos(stable_service).unwrap();
            saw_stable = true;
            break;
        }
    }
    assert!(
        saw_stable,
        "expected stable service to be readable after churn"
    );

    std::fs::remove_dir_all(&root).unwrap();
}
