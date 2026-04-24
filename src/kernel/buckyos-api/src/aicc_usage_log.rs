/*!
 * AICC usage-log schema, data types, and query DSL.
 *
 * The actual sqlx-backed store lives inside the aicc service itself (see
 * `src/frame/aicc/src/aicc_usage_log_db.rs`) and mirrors the layout used by
 * other service rdb instances (msg-center, task-manager). This module only
 * carries:
 *
 * - the instance id + schema DDL that the scheduler drops into
 *   `services/aicc/spec.install_config.rdb_instances`
 * - the row struct (`AiccUsageEvent`) shared between the writer and reader
 * - the query DSL (`QueryUsageRequest` / `QueryUsageResponse`) so callers do
 *   not have to hand-roll SQL to read the log.
 */

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rdb_mgr::{RdbBackend, RdbInstanceConfig};

/// Logical name of the aicc usage-log rdb instance. The scheduler writes this
/// into `services/aicc/spec` and the aicc service resolves it at start via
/// `get_rdb_instance`.
pub const AICC_USAGE_LOG_RDB_INSTANCE_ID: &str = "aicc-usage-log";

/// Version of the usage-log schema. Bump whenever the DDL changes.
pub const AICC_USAGE_LOG_RDB_SCHEMA_VERSION: u64 = 1;

/// Sqlite DDL for the usage-log database. The only required table in v1 is
/// `aicc_usage_event`; summary tables can be added later when SQL aggregation
/// becomes necessary.
pub const AICC_USAGE_LOG_RDB_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS aicc_usage_event (
    event_id              TEXT PRIMARY KEY,
    tenant_id             TEXT NOT NULL,
    caller_app_id         TEXT,
    task_id               TEXT NOT NULL,
    idempotency_key       TEXT,
    capability            TEXT NOT NULL,
    request_model         TEXT NOT NULL,
    provider_model        TEXT NOT NULL,
    input_tokens          INTEGER,
    output_tokens         INTEGER,
    total_tokens          INTEGER,
    request_units         INTEGER,
    usage_json            TEXT NOT NULL,
    finance_snapshot_json TEXT,
    created_at_ms         INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_time
    ON aicc_usage_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_time
    ON aicc_usage_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_model_time
    ON aicc_usage_event(provider_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_request_model_time
    ON aicc_usage_event(request_model, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_task
    ON aicc_usage_event(tenant_id, task_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_idem
    ON aicc_usage_event(tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
"#;

/// Postgres DDL mirroring the sqlite schema above.
pub const AICC_USAGE_LOG_RDB_SCHEMA_POSTGRES: &str = r#"
CREATE TABLE IF NOT EXISTS aicc_usage_event (
    event_id              TEXT PRIMARY KEY,
    tenant_id             TEXT NOT NULL,
    caller_app_id         TEXT,
    task_id               TEXT NOT NULL,
    idempotency_key       TEXT,
    capability            TEXT NOT NULL,
    request_model         TEXT NOT NULL,
    provider_model        TEXT NOT NULL,
    input_tokens          BIGINT,
    output_tokens         BIGINT,
    total_tokens          BIGINT,
    request_units         BIGINT,
    usage_json            TEXT NOT NULL,
    finance_snapshot_json TEXT,
    created_at_ms         BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_time
    ON aicc_usage_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_time
    ON aicc_usage_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_model_time
    ON aicc_usage_event(provider_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_request_model_time
    ON aicc_usage_event(request_model, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_task
    ON aicc_usage_event(tenant_id, task_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_idem
    ON aicc_usage_event(tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
"#;

/// Default rdb-instance config for the aicc usage-log. The scheduler drops
/// this into `install_config.rdb_instances` when bootstrapping the service.
pub fn aicc_usage_log_default_rdb_instance_config() -> RdbInstanceConfig {
    let mut schema = HashMap::new();
    schema.insert(
        RdbBackend::Sqlite,
        AICC_USAGE_LOG_RDB_SCHEMA_SQLITE.to_string(),
    );
    schema.insert(
        RdbBackend::Postgres,
        AICC_USAGE_LOG_RDB_SCHEMA_POSTGRES.to_string(),
    );
    RdbInstanceConfig {
        backend: RdbBackend::Sqlite,
        version: AICC_USAGE_LOG_RDB_SCHEMA_VERSION,
        schema,
        // Empty -> rdb_mgr generates `sqlite://$appdata/aicc-usage-log.db` at
        // resolve time.
        connection: String::new(),
    }
}

/// One durable row in `aicc_usage_event`.
///
/// Token columns are flattened copies of `usage_json` so SQL aggregation can
/// work without parsing JSON. `request_units` is the generic fallback for
/// non-token providers; future extensions (image count, audio seconds, ...)
/// should add their own top-level columns as the schema version bumps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiccUsageEvent {
    pub event_id: String,
    pub tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_app_id: Option<String>,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub capability: String,
    pub request_model: String,
    pub provider_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_units: Option<u64>,
    pub usage_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finance_snapshot_json: Option<Value>,
    pub created_at_ms: i64,
}

/// Time-range selector for `query_usage`.
///
/// `Explicit` is the general form. The shortcuts are resolved server-side
/// relative to the current clock so callers can write `last_1d` / `last_7d`
/// without worrying about clock skew.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageQueryTimeRange {
    Last1d,
    Last7d,
    Last30d,
    Explicit {
        start_time_ms: i64,
        end_time_ms: i64,
    },
}

/// Optional `WHERE` filters. Every field is independent; omitted fields mean
/// "no filter on this dimension".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageQueryFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Group dimensions supported by `query_usage`. Multiple values produce a
/// multi-dimensional grouping (think `GROUP BY a, b`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum UsageQueryGroup {
    ProviderModel,
    RequestModel,
    Capability,
    CallerAppId,
    TenantId,
}

impl UsageQueryGroup {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::ProviderModel => "provider_model",
            Self::RequestModel => "request_model",
            Self::Capability => "capability",
            Self::CallerAppId => "caller_app_id",
            Self::TenantId => "tenant_id",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum UsageQueryBucket {
    Hour,
    Day,
}

impl UsageQueryBucket {
    pub fn span_ms(self) -> i64 {
        match self {
            Self::Hour => 60 * 60 * 1000,
            Self::Day => 24 * 60 * 60 * 1000,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageQueryOutputMode {
    Summary,
    Events,
    SummaryAndEvents,
}

impl Default for UsageQueryOutputMode {
    fn default() -> Self {
        Self::Summary
    }
}

/// The general query interface mandated by the requirements doc (section 7).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryUsageRequest {
    pub time_range: UsageQueryTimeRange,
    #[serde(default)]
    pub filters: UsageQueryFilters,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_by: Vec<UsageQueryGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_bucket: Option<UsageQueryBucket>,
    #[serde(default)]
    pub output_mode: UsageQueryOutputMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Aggregated counts / totals. Units beyond tokens go into `request_units`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageAggregate {
    pub total_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub request_units: u64,
}

/// One row of a grouped query result. `group` holds `dimension → value`
/// pairs, e.g. `{"provider_model": "gpt4.openai"}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageGroupedRow {
    pub group: HashMap<String, String>,
    pub aggregate: UsageAggregate,
}

/// One row of a time-bucketed result. When a grouping is also set, the same
/// dimension map appears on every bucket row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageBucketedRow {
    pub bucket_start_ms: i64,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub group: HashMap<String, String>,
    pub aggregate: UsageAggregate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct QueryUsageResponse {
    pub total: UsageAggregate,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grouped: Vec<UsageGroupedRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buckets: Vec<UsageBucketedRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<AiccUsageEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
