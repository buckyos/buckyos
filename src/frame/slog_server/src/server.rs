use crate::storage::{LogQueryRequest, LogRecords, LogStorageRef};
use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

const DEFAULT_QUERY_LIMIT: usize = 200;
const MAX_QUERY_LIMIT: usize = 2000;
const MAX_QUERY_SCAN: usize = 20_000;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogResponseMessage {
    pub ret: i32,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct LogQueryHttpRequest {
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub start_time: Option<u64>,
    #[serde(default)]
    pub end_time: Option<u64>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QueryLogRecord {
    pub node: String,
    pub service: String,
    pub log: slog::SystemLogRecord,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QueryPageInfo {
    pub offset: usize,
    pub limit: usize,
    pub returned: usize,
    pub has_more: bool,
    pub next_offset: Option<usize>,
    pub sort: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogQueryData {
    pub records: Vec<QueryLogRecord>,
    pub page: QueryPageInfo,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogQueryResponseMessage {
    pub ret: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<LogQueryData>,
}

#[derive(Debug, Clone)]
struct NormalizedLogQueryRequest {
    node: Option<String>,
    service: Option<String>,
    level: Option<slog::LogLevel>,
    start_time: Option<u64>,
    end_time: Option<u64>,
    offset: usize,
    limit: usize,
    fetch_limit: usize,
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

fn trim_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn parse_level_filter(value: Option<String>) -> Result<Option<slog::LogLevel>, String> {
    let Some(raw) = trim_optional_string(value) else {
        return Ok(None);
    };

    let normalized = raw.to_ascii_lowercase();
    match normalized.as_str() {
        "off" => return Ok(Some(slog::LogLevel::Off)),
        "error" => return Ok(Some(slog::LogLevel::Error)),
        "warn" | "warning" => return Ok(Some(slog::LogLevel::Warn)),
        "info" => return Ok(Some(slog::LogLevel::Info)),
        "debug" => return Ok(Some(slog::LogLevel::Debug)),
        "trace" => return Ok(Some(slog::LogLevel::Trace)),
        _ => {}
    }

    let level_num = normalized.parse::<u32>().map_err(|_| {
        format!(
            "invalid level '{}': expected off/error/warn/info/debug/trace or 0-5",
            raw
        )
    })?;
    let level = slog::LogLevel::try_from(level_num)
        .map_err(|e| format!("invalid level '{}': {} (allowed range: 0..=5)", raw, e))?;
    Ok(Some(level))
}

fn normalize_log_query_request(
    request: LogQueryHttpRequest,
) -> Result<NormalizedLogQueryRequest, String> {
    let node = trim_optional_string(request.node);
    let service = trim_optional_string(request.service);
    let level = parse_level_filter(request.level)?;

    if let (Some(start_time), Some(end_time)) = (request.start_time, request.end_time) {
        if start_time > end_time {
            return Err(format!(
                "invalid time range: start_time {} is greater than end_time {}",
                start_time, end_time
            ));
        }
    }

    let offset = request.offset.unwrap_or(0);
    let requested_limit = request.limit.unwrap_or(DEFAULT_QUERY_LIMIT);
    if requested_limit == 0 {
        return Err("invalid limit: must be greater than 0".to_string());
    }
    let limit = requested_limit.min(MAX_QUERY_LIMIT);

    let scan_window = offset
        .checked_add(limit)
        .ok_or_else(|| "invalid pagination: offset + limit overflow".to_string())?;
    if scan_window > MAX_QUERY_SCAN {
        return Err(format!(
            "invalid pagination: offset + limit must be <= {}",
            MAX_QUERY_SCAN
        ));
    }

    let fetch_limit = scan_window
        .checked_add(1)
        .ok_or_else(|| "invalid pagination: fetch limit overflow".to_string())?;

    Ok(NormalizedLogQueryRequest {
        node,
        service,
        level,
        start_time: request.start_time,
        end_time: request.end_time,
        offset,
        limit,
        fetch_limit,
    })
}

fn flatten_query_result(records: Vec<LogRecords>) -> Vec<QueryLogRecord> {
    let mut flattened = Vec::new();
    for group in records {
        let node = group.node;
        let service = group.service;
        for log in group.logs {
            flattened.push(QueryLogRecord {
                node: node.clone(),
                service: service.clone(),
                log,
            });
        }
    }
    flattened
}

fn stable_sort_query_records(records: &mut [QueryLogRecord]) {
    records.sort_by(|left, right| {
        right
            .log
            .time
            .cmp(&left.log.time)
            .then_with(|| left.node.cmp(&right.node))
            .then_with(|| left.service.cmp(&right.service))
            .then_with(|| (left.log.level as usize).cmp(&(right.log.level as usize)))
            .then_with(|| left.log.target.cmp(&right.log.target))
            .then_with(|| left.log.file.cmp(&right.log.file))
            .then_with(|| left.log.line.cmp(&right.log.line))
            .then_with(|| left.log.content.cmp(&right.log.content))
    });
}

async fn handle_query_logs(
    storage: LogStorageRef,
    request: LogQueryHttpRequest,
) -> (StatusCode, Json<LogQueryResponseMessage>) {
    let normalized = match normalize_log_query_request(request) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(LogQueryResponseMessage {
                    ret: 1,
                    message: e,
                    data: None,
                }),
            );
        }
    };

    info!(
        "Received log query request: node={:?}, service={:?}, level={:?}, start_time={:?}, end_time={:?}, offset={}, limit={}",
        normalized.node,
        normalized.service,
        normalized.level,
        normalized.start_time,
        normalized.end_time,
        normalized.offset,
        normalized.limit
    );

    let query_request = LogQueryRequest {
        node: normalized.node.clone(),
        service: normalized.service.clone(),
        level: normalized.level,
        start_time: normalized.start_time,
        end_time: normalized.end_time,
        limit: Some(normalized.fetch_limit),
    };

    let queried = match storage.query_logs(query_request).await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(LogQueryResponseMessage {
                    ret: 1,
                    message: format!("Failed to query logs: {}", e),
                    data: None,
                }),
            );
        }
    };

    let mut records = flatten_query_result(queried);
    stable_sort_query_records(&mut records);

    let fetched_len = records.len();
    let start = normalized.offset.min(fetched_len);
    let end = normalized
        .offset
        .saturating_add(normalized.limit)
        .min(fetched_len);
    let page_records = if start < end {
        records[start..end].to_vec()
    } else {
        Vec::new()
    };
    let has_more = fetched_len > end;
    let next_offset = has_more.then_some(end);

    (
        StatusCode::OK,
        Json(LogQueryResponseMessage {
            ret: 0,
            message: "Logs queried successfully".to_string(),
            data: Some(LogQueryData {
                records: page_records,
                page: QueryPageInfo {
                    offset: normalized.offset,
                    limit: normalized.limit,
                    returned: end.saturating_sub(start),
                    has_more,
                    next_offset,
                    sort: "time_desc,node_asc,service_asc,level_asc,target_asc,file_asc,line_asc,content_asc".to_string(),
                },
            }),
        }),
    )
}

impl LogHttpServer {
    pub fn new(storage: LogStorageRef) -> Self {
        Self { storage }
    }

    pub async fn run(&self, addr: &str) -> Result<(), String> {
        let append_storage = self.storage.clone();
        let query_get_storage = self.storage.clone();
        let query_post_storage = self.storage.clone();
        let app = Router::new()
            .route(
                "/logs",
                post(move |log_records: Json<LogRecords>| {
                    let storage = append_storage.clone();
                    async move { handle_append_logs(storage, log_records.0).await }
                }),
            )
            .route(
                "/query",
                get(move |request: Query<LogQueryHttpRequest>| {
                    let storage = query_get_storage.clone();
                    async move { handle_query_logs(storage, request.0).await }
                })
                .post(move |request: Json<LogQueryHttpRequest>| {
                    let storage = query_post_storage.clone();
                    async move { handle_query_logs(storage, request.0).await }
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
    use crate::storage::LogStorage;
    use slog::{LogLevel, SystemLogRecord};
    use std::sync::{Arc, Mutex};

    struct MockStorage {
        append_result: Result<(), String>,
        query_result: Result<Vec<LogRecords>, String>,
        captured_query: Arc<Mutex<Option<LogQueryRequest>>>,
    }

    #[async_trait::async_trait]
    impl LogStorage for MockStorage {
        async fn append_logs(&self, _records: LogRecords) -> Result<(), String> {
            self.append_result.clone()
        }

        async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
            *self.captured_query.lock().unwrap() = Some(request);
            self.query_result.clone()
        }
    }

    fn test_log(time: u64, level: LogLevel, content: &str) -> SystemLogRecord {
        SystemLogRecord {
            level,
            target: "test-target".to_string(),
            time,
            file: Some("test.rs".to_string()),
            line: Some(1),
            content: content.to_string(),
        }
    }

    fn make_storage(
        append_result: Result<(), String>,
        query_result: Result<Vec<LogRecords>, String>,
    ) -> (LogStorageRef, Arc<Mutex<Option<LogQueryRequest>>>) {
        let captured_query = Arc::new(Mutex::new(None));
        let storage: LogStorageRef = Arc::new(Box::new(MockStorage {
            append_result,
            query_result,
            captured_query: captured_query.clone(),
        }));
        (storage, captured_query)
    }

    #[tokio::test]
    async fn test_handle_append_logs_returns_ok_when_append_succeeds() {
        let (storage, _) = make_storage(Ok(()), Ok(vec![]));
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
        let (storage, _) = make_storage(Err("db write failed".to_string()), Ok(vec![]));
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

    #[tokio::test]
    async fn test_handle_query_logs_returns_paginated_stable_sorted_records() {
        let (storage, captured_query) = make_storage(
            Ok(()),
            Ok(vec![
                LogRecords {
                    node: "node-b".to_string(),
                    service: "svc-z".to_string(),
                    batch_id: None,
                    record_ids: vec![],
                    logs: vec![test_log(1000, LogLevel::Warn, "content-b")],
                },
                LogRecords {
                    node: "node-a".to_string(),
                    service: "svc-a".to_string(),
                    batch_id: None,
                    record_ids: vec![],
                    logs: vec![
                        test_log(1000, LogLevel::Warn, "content-a"),
                        test_log(900, LogLevel::Info, "content-old"),
                    ],
                },
            ]),
        );

        let (status, body) = handle_query_logs(
            storage,
            LogQueryHttpRequest {
                node: Some("node-a".to_string()),
                service: Some("svc-a".to_string()),
                level: Some("warn".to_string()),
                start_time: Some(100),
                end_time: Some(2000),
                offset: Some(1),
                limit: Some(2),
            },
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.ret, 0);
        let data = body.data.as_ref().expect("query data should exist");
        assert_eq!(data.records.len(), 2);
        assert_eq!(data.records[0].node, "node-b");
        assert_eq!(data.records[0].service, "svc-z");
        assert_eq!(data.records[0].log.content, "content-b");
        assert_eq!(data.records[1].node, "node-a");
        assert_eq!(data.records[1].log.content, "content-old");
        assert_eq!(data.page.offset, 1);
        assert_eq!(data.page.limit, 2);
        assert_eq!(data.page.returned, 2);
        assert!(!data.page.has_more);
        assert_eq!(data.page.next_offset, None);

        let forwarded = captured_query
            .lock()
            .unwrap()
            .clone()
            .expect("forwarded query should be captured");
        assert_eq!(forwarded.node.as_deref(), Some("node-a"));
        assert_eq!(forwarded.service.as_deref(), Some("svc-a"));
        assert_eq!(forwarded.level, Some(LogLevel::Warn));
        assert_eq!(forwarded.start_time, Some(100));
        assert_eq!(forwarded.end_time, Some(2000));
        assert_eq!(forwarded.limit, Some(4));
    }

    #[tokio::test]
    async fn test_handle_query_logs_rejects_invalid_level() {
        let (storage, _) = make_storage(Ok(()), Ok(vec![]));
        let (status, body) = handle_query_logs(
            storage,
            LogQueryHttpRequest {
                level: Some("bad-level".to_string()),
                ..Default::default()
            },
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.ret, 1);
        assert!(body.message.contains("invalid level"));
    }

    #[tokio::test]
    async fn test_handle_query_logs_rejects_invalid_time_range() {
        let (storage, _) = make_storage(Ok(()), Ok(vec![]));
        let (status, body) = handle_query_logs(
            storage,
            LogQueryHttpRequest {
                start_time: Some(200),
                end_time: Some(100),
                ..Default::default()
            },
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.ret, 1);
        assert!(body.message.contains("invalid time range"));
    }

    #[tokio::test]
    async fn test_handle_query_logs_returns_500_when_storage_query_fails() {
        let (storage, _) = make_storage(Ok(()), Err("db query failed".to_string()));
        let (status, body) = handle_query_logs(storage, LogQueryHttpRequest::default()).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.ret, 1);
        assert!(body.message.contains("Failed to query logs"));
    }
}
