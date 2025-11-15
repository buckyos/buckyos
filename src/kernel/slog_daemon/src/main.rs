mod reader;
mod read_manager;
mod upload;
mod client;

#[macro_use]
extern crate log;

use crate::client::LogDaemonClient;

#[tokio::main]
async fn main() {
    let node_id = "node-001".to_string();
    let service_endpoint = "http://logserver.example.com/upload".to_string();
    let log_dir = std::path::Path::new("/var/log/slog");

    buckyos_kit::init_logging(app_name, is_service);
    let _client = match LogDaemonClient::new(node_id, service_endpoint, log_dir) {
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
