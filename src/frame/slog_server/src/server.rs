use crate::storage::{LogRecords, LogStorageRef};
use axum::{Router, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogResponseMessage {
    pub ret: i32,
    pub message: String,
}

pub struct LogHttpServer {
    storage: LogStorageRef,
}

impl LogHttpServer {
    pub fn new(storage: LogStorageRef) -> Self {
        Self { storage }
    }

    pub async fn run(&self, addr: &str) -> Result<(), String> {
        let storage = self.storage.clone();
        let app = Router::new().route(
            "/logs",
            post(move |log_records: axum::Json<LogRecords>| {
                let storage = storage.clone();
                async move {
                    match storage.append_logs(log_records.0).await {
                        Ok(_) => axum::Json(LogResponseMessage {
                            ret: 0,
                            message: "Logs stored successfully".to_string(),
                        }),
                        Err(e) => axum::Json(LogResponseMessage {
                            ret: 1,
                            message: format!("Failed to store logs: {}", e),
                        }),
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            let msg = format!("Failed to bind to address {}: {}", addr, e);
            error!("{}", msg);
            msg
        })?;

        match axum::serve(listener, app).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = format!("HTTP server error: {}", e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }
}
