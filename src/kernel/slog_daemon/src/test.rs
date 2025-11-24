use crate::client::LogDaemonClient;
use slog::*;

const SERVICE_NAME: &str = "test_slog_daemon";

#[tokio::test]
async fn test_daemon() {
    let node_id = "node-001".to_string();
    let service_endpoint = "http://127.0.0.1:8089/logs".to_string();
    let log_dir = slog::get_buckyos_log_root_dir();

    SystemLoggerBuilder::new(&log_dir, SERVICE_NAME, SystemLoggerCategory::Service)
        .level("debug")
        .console("debug")
        .build()
        .unwrap()
        .start();

    let excluded = vec![];
    let _client = match LogDaemonClient::new(node_id, service_endpoint, &log_dir, excluded) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create LogDaemonClient: {}", e);
            return;
        }
    };

    // Sleep infinitely
    tokio::signal::ctrl_c().await.unwrap();
}

async fn test_service(name: &str) {
    let log_root_dir = get_buckyos_log_root_dir();
    let log_dir = log_root_dir.join(name);
    std::fs::create_dir_all(&log_dir).unwrap();

    // Create file log target
    let target = FileLogTarget::new(
        &log_dir,
        name.to_string(),
        1024 * 1024 * 16, // 16 MB max file size
        1000,             // flush interval ms
    )
    .unwrap();
    let target = Box::new(target) as Box<dyn SystemLogTarget>;

    let logger = SystemLoggerBuilder::new(&log_root_dir, name, SystemLoggerCategory::Service)
        .level("info")
        .console("debug")
        .target(target)
        .build()
        .unwrap();
    logger.start();

    let mut index = 0;
    loop {
        log::info!("Info log message number {}", index);
        log::debug!("Debug log message number {}", index);
        index += 1;

        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
