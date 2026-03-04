use crate::storage::{LogRecords, LogStorageRef};
use axum::{Json, Router, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogResponseMessage {
    pub ret: i32,
    pub message: String,
}

pub struct LogHttpServer {
    storage: LogStorageRef,
}

async fn handle_append_logs(
    storage: LogStorageRef,
    records: LogRecords,
) -> (StatusCode, Json<LogResponseMessage>) {
    info!(
        "Received log records: node {}, service: {}, count {}",
        records.node,
        records.service,
        records.logs.len()
    );

    match storage.append_logs(records).await {
        Ok(_) => (
            StatusCode::OK,
            Json(LogResponseMessage {
                ret: 0,
                message: "Logs stored successfully".to_string(),
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LogResponseMessage {
                ret: 1,
                message: format!("Failed to store logs: {}", e),
            }),
        ),
    }
}

impl LogHttpServer {
    pub fn new(storage: LogStorageRef) -> Self {
        Self { storage }
    }

    pub async fn run(&self, addr: &str) -> Result<(), String> {
        let storage = self.storage.clone();
        let app = Router::new().route(
            "/logs",
            post(move |log_records: Json<LogRecords>| {
                let storage = storage.clone();
                async move { handle_append_logs(storage, log_records.0).await }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{LogQueryRequest, LogStorage};
    use std::sync::Arc;

    struct MockStorage {
        append_result: Result<(), String>,
    }

    #[async_trait::async_trait]
    impl LogStorage for MockStorage {
        async fn append_logs(&self, _records: LogRecords) -> Result<(), String> {
            self.append_result.clone()
        }

        async fn query_logs(&self, _request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_handle_append_logs_returns_ok_when_append_succeeds() {
        let storage: LogStorageRef = Arc::new(Box::new(MockStorage {
            append_result: Ok(()),
        }));
        let (status, body) = handle_append_logs(
            storage,
            LogRecords {
                node: "node-1".to_string(),
                service: "svc-a".to_string(),
                batch_id: None,
                record_ids: vec![],
                logs: vec![],
            },
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.ret, 0);
    }

    #[tokio::test]
    async fn test_handle_append_logs_returns_500_when_append_fails() {
        let storage: LogStorageRef = Arc::new(Box::new(MockStorage {
            append_result: Err("db write failed".to_string()),
        }));
        let (status, body) = handle_append_logs(
            storage,
            LogRecords {
                node: "node-1".to_string(),
                service: "svc-a".to_string(),
                batch_id: None,
                record_ids: vec![],
                logs: vec![],
            },
        )
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.ret, 1);
        assert!(body.message.contains("Failed to store logs"));
    }
}
