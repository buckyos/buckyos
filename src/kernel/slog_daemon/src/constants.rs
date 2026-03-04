//! Shared constants for `slog_daemon`.
//!
//! Keep all daemon-level constants in one place so runtime behavior can be
//! tuned centrally and later mapped to the BuckYOS config system.

/// Service name used by local logger directory and exclusion filters.
pub const SERVICE_NAME: &str = "slog_daemon";

/// Environment key for overriding the logical node id carried in upload payloads.
pub const SLOG_NODE_ID_ENV_KEY: &str = "SLOG_NODE_ID";

/// Environment key for overriding upload endpoint, e.g. `http://host:8089/logs`.
pub const SLOG_SERVER_ENDPOINT_ENV_KEY: &str = "SLOG_SERVER_ENDPOINT";

/// Environment key for overriding daemon log root directory.
pub const SLOG_LOG_DIR_ENV_KEY: &str = "SLOG_LOG_DIR";

/// Environment key for upload HTTP timeout (seconds).
pub const SLOG_UPLOAD_TIMEOUT_SECS_ENV_KEY: &str = "SLOG_UPLOAD_TIMEOUT_SECS";

/// Default node id when no external config is provided.
pub const DEFAULT_NODE_ID: &str = "node-001";

/// Default upload endpoint when no external config is provided.
pub const DEFAULT_SERVER_ENDPOINT: &str = "http://127.0.0.1:8089/logs";

/// Default HTTP timeout for upload requests.
pub const DEFAULT_UPLOAD_TIMEOUT_SECS: u64 = 10;

/// How often daemon rescans log root for service directories.
pub const UPDATE_DIR_INTERVAL_SECS: u64 = 60;

/// Max number of records pulled from all services in one read cycle.
pub const READ_RECORD_BATCH_SIZE: usize = 100;

/// Max records read from one service in a single read cycle.
///
/// This prevents a single hot service from consuming the whole batch and
/// starving other services for long periods.
pub const READ_RECORD_PER_SERVICE_QUOTA: usize = 10;

/// Base polling interval for read loop when not saturated.
pub const READ_RECORD_INTERVAL_MILLIS: u64 = 1000;

/// Initial backoff after one upload failure for a specific service.
pub const INITIAL_UPLOAD_FAILED_RETRY_INTERVAL_SECS: u64 = 2;

/// Upper bound of exponential backoff after repeated failures.
pub const MAX_UPLOAD_FAILED_RETRY_INTERVAL_SECS: u64 = 60 * 2;
