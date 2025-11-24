#![allow(dead_code)]

mod client;
mod read_manager;
mod reader;
mod upload;

#[cfg(test)]
mod test;

#[macro_use]
extern crate log;

use crate::client::LogDaemonClient;

const SERVICE_NAME: &str = "slog_daemon";

#[tokio::main]
async fn main() {
    let node_id = "node-001".to_string();
    let service_endpoint = "http://127.0.0.1:8089/logs".to_string();
    let log_dir = slog::get_buckyos_log_root_dir();

    // Init slog daemon own logs, output to file and console
    let daemon_log_dir = log_dir.join(SERVICE_NAME);
    std::fs::create_dir_all(&daemon_log_dir).unwrap();
    slog::SystemLoggerBuilder::new(
        &daemon_log_dir,
        SERVICE_NAME,
        slog::SystemLoggerCategory::Service,
    )
    .level("debug")
    .console("debug")
    .file(true)
    .build()
    .unwrap()
    .start();

    // Specify excluded services
    // We should not upload logs from slog_daemon itself and slog_server
    let excluded = vec![SERVICE_NAME.to_string(), "slog_server".to_string()];
    let _client = match LogDaemonClient::new(node_id, service_endpoint, &log_dir, excluded) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create LogDaemonClient: {}", e);
            return;
        }
    };

    // Sleep indefinitely (or implement actual logic)
    tokio::signal::ctrl_c().await.unwrap();

    info!("Log daemon client exiting.");
}
