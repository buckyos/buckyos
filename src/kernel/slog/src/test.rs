use crate::*;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const SERVICE_NAME: &str = "test_slog_service";

#[test]
#[ignore = "manual integration test"]
fn test_main() {
    // Start new thread to read logs
    std::thread::spawn(|| {
        test_read();
    });

    test_write();
}

#[test]
#[ignore = "manual integration test"]
fn test_write() {
    let log_root_dir = get_buckyos_log_root_dir();
    std::fs::create_dir_all(&log_root_dir).unwrap();

    let log_dir = log_root_dir.join(SERVICE_NAME);
    std::fs::create_dir_all(&log_dir).unwrap();
    println!("Log directory: {:?}", log_dir);

    // Create file log target

    let logger =
        SystemLoggerBuilder::new(&log_root_dir, SERVICE_NAME, SystemLoggerCategory::Service)
            .level("info")
            .console("debug")
            .enable_file_with_upload()
            .unwrap()
            .build()
            .unwrap();
    logger.start();

    log::info!("This is an info log.");
    log::debug!("This is a debug log.");

    for index in 0..7 {
        log::info!("Info log message number {}", index);
        log::debug!("Debug log message number {}", index);

        // std::thread::sleep(std::time::Duration::from_secs(1));
    }

    std::thread::sleep(std::time::Duration::from_secs(2));
}

fn test_read() {
    let log_root_dir = get_buckyos_log_root_dir();
    let log_dir = log_root_dir.join(SERVICE_NAME);

    let reader = FileLogReader::open(&log_dir).unwrap();

    // Write logs to a separate file
    let target_file = log_dir.join(format!("{}_copy.log", SERVICE_NAME));
    let mut target_file = std::fs::File::create(&target_file).unwrap();

    loop {
        let records = reader.try_read_next_records(100).unwrap();
        if !records.is_empty() {
            info!("Read {} log records", records.len());
            for record in &records {
                let s = SystemLogRecordLineFormatter::format_record(record);
                println!("read record: {}", s);
                target_file.write_all(s.as_bytes()).unwrap();
            }

            target_file.flush().unwrap();
            reader.flush_read_index().unwrap();
        }

        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

fn new_temp_log_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "buckyos/slog_tests/{}_{}_{}",
        prefix,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_get_last_sealed_file_returns_none_when_no_sealed_file() {
    let log_dir = new_temp_log_dir("meta_none");
    let meta = LogMeta::open(&log_dir).unwrap();

    let ret = meta.get_last_sealed_file().unwrap();
    assert!(ret.is_none());

    std::fs::remove_dir_all(&log_dir).unwrap();
}

#[test]
fn test_get_last_sealed_file_field_mapping() {
    let log_dir = new_temp_log_dir("meta_mapping");
    let meta = LogMeta::open(&log_dir).unwrap();

    meta.append_new_file("service.1.log").unwrap();
    meta.update_current_write_index(123).unwrap();
    meta.seal_current_write_file().unwrap();
    meta.update_current_read_index(45).unwrap();

    let file = meta.get_last_sealed_file().unwrap().unwrap();
    assert_eq!(file.name, "service.1.log");
    assert_eq!(file.write_index, 123);
    assert!(file.is_sealed);
    assert_eq!(file.read_index, 45);
    assert!(!file.is_read_complete);

    std::fs::remove_dir_all(&log_dir).unwrap();
}

#[test]
fn test_get_last_sealed_file_returns_latest_sealed_file() {
    let log_dir = new_temp_log_dir("meta_latest");
    let meta = LogMeta::open(&log_dir).unwrap();

    meta.append_new_file("service.1.log").unwrap();
    meta.update_current_write_index(1).unwrap();
    meta.seal_current_write_file().unwrap();

    meta.append_new_file("service.2.log").unwrap();
    meta.update_current_write_index(2).unwrap();
    meta.seal_current_write_file().unwrap();

    let file = meta.get_last_sealed_file().unwrap().unwrap();
    assert_eq!(file.name, "service.2.log");
    assert_eq!(file.write_index, 2);
    assert!(file.is_sealed);

    std::fs::remove_dir_all(&log_dir).unwrap();
}
