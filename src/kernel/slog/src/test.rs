use crate::*;
use std::io::Write;

const SERVICE_NAME: &str = "test_slog_service";

#[test]
fn test_main() {
    // Start new thread to read logs
    std::thread::spawn(|| {
        test_read();
    });

    test_write();
}

#[test]
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
