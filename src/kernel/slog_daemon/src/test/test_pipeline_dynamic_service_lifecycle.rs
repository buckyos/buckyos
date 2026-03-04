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
        file: Some("pipeline_dynamic_lifecycle_test.rs".to_string()),
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
fn test_dynamic_service_lifecycle_add_and_remove() {
    let root = new_temp_root("dynamic_service_lifecycle");
    let service_a = "svc_lifecycle_a";
    let service_b = "svc_lifecycle_b";

    let records_a = vec![
        make_record(service_a, 1722000800001, "lifecycle-a-1"),
        make_record(service_a, 1722000800002, "lifecycle-a-2"),
    ];
    let records_b = vec![
        make_record(service_b, 1722000801001, "lifecycle-b-1"),
        make_record(service_b, 1722000801002, "lifecycle-b-2"),
    ];

    let service_a_dir = prepare_service_logs(&root, service_a, &records_a).unwrap();

    let reader = LogDirReader::open(&root, vec![]).unwrap();
    let items_a = reader.try_read_records(100).unwrap();
    let a_item = items_a
        .iter()
        .find(|item| item.id == service_a)
        .expect("expected records from service_a");
    assert_eq!(a_item.records.len(), records_a.len());
    reader.flush_read_pos(service_a).unwrap();

    let _service_b_dir = prepare_service_logs(&root, service_b, &records_b).unwrap();
    reader.update_dir().unwrap();

    let mut saw_b = false;
    for _ in 0..5 {
        let items = reader.try_read_records(100).unwrap();
        if let Some(item) = items.iter().find(|item| item.id == service_b) {
            assert_eq!(item.records.len(), records_b.len());
            reader.flush_read_pos(service_b).unwrap();
            saw_b = true;
            break;
        }
    }
    assert!(
        saw_b,
        "expected service_b to be discovered after update_dir"
    );

    std::fs::remove_dir_all(&service_a_dir).unwrap();
    reader.update_dir().unwrap();

    let err = reader.flush_read_pos(service_a).unwrap_err();
    assert!(matches!(err, FlushReadPosError::NotFound { .. }));

    std::fs::remove_dir_all(&root).unwrap();
}
