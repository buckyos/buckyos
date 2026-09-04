/*!
 * AICC usage-log schema, data types, and query DSL.
 *
 * The actual sqlx-backed store lives inside the aicc service itself (see
 * `src/frame/aicc/src/aicc_usage_log_db.rs`) and mirrors the layout used by
 * other service rdb instances (msg-center, task-manager). This module only
 * carries:
 *
 * - the instance id + schema DDL that the scheduler drops into
 *   `services/aicc/spec.spec_config.rdb_instances`
 * - the row struct (`AiccUsageEvent`) shared between the writer and reader
 * - the query DSL (`QueryUsageRequest` / `QueryUsageResponse`) so callers do
 *   not have to hand-roll SQL to read the log.
 */

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::aicc_client::{ai_methods, AiUsage, Money, RouteTrace};
use crate::rdb_mgr::{RdbBackend, RdbInstanceConfig, RdbPartition};

/// Logical name of the aicc usage-log rdb instance. The scheduler writes this
/// into `services/aicc/spec` and the aicc service resolves it at start via
/// `get_rdb_instance`.
pub const AICC_USAGE_LOG_RDB_INSTANCE_ID: &str = "aicc-usage-log";

/// Version of the usage-log schema. Bump whenever the DDL changes.
pub const AICC_USAGE_LOG_RDB_SCHEMA_VERSION: u64 = 5;

/// Sqlite DDL for the usage-log database. The only required table in v1 is
/// `aicc_usage_event`; summary tables can be added later when SQL aggregation
/// becomes necessary.
pub const AICC_USAGE_LOG_RDB_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS aicc_usage_event (
    event_id              TEXT PRIMARY KEY,
    tenant_id             TEXT NOT NULL,
    user_id               TEXT NOT NULL,
    caller_app_id         TEXT,
    task_id               TEXT NOT NULL,
    idempotency_key       TEXT,
    method                TEXT NOT NULL,
    capability            TEXT NOT NULL,
    request_model         TEXT NOT NULL,
    provider_instance_name TEXT NOT NULL,
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
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_user_time
    ON aicc_usage_event(user_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_method_time
    ON aicc_usage_event(method, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_provider_instance_time
    ON aicc_usage_event(provider_instance_name, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_model_time
    ON aicc_usage_event(provider_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_request_model_time
    ON aicc_usage_event(request_model, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_task
    ON aicc_usage_event(tenant_id, task_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_idem
    ON aicc_usage_event(tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE TABLE IF NOT EXISTS aicc_route_trace (
    trace_id                 TEXT PRIMARY KEY,
    tenant_id                TEXT NOT NULL,
    caller_app_id            TEXT,
    task_id                  TEXT NOT NULL,
    request_model            TEXT NOT NULL,
    selected_exact_model     TEXT,
    provider_instance_name   TEXT,
    api_type                 TEXT NOT NULL,
    route_trace_json         TEXT NOT NULL,
    created_at_ms            INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_time
    ON aicc_route_trace(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_tenant_time
    ON aicc_route_trace(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_request_model_time
    ON aicc_route_trace(request_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_task_time
    ON aicc_route_trace(task_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_video_continuation_source (
    tenant_id                 TEXT NOT NULL,
    content_id                TEXT NOT NULL,
    source_task_id            TEXT NOT NULL,
    created_at_ms             INTEGER NOT NULL,
    PRIMARY KEY (tenant_id, content_id)
);
CREATE INDEX IF NOT EXISTS idx_aicc_video_continuation_source_task
    ON aicc_video_continuation_source(source_task_id);
"#;

/// Postgres DDL mirroring the sqlite schema above.
pub const AICC_USAGE_LOG_RDB_SCHEMA_POSTGRES: &str = r#"
CREATE TABLE IF NOT EXISTS aicc_usage_event (
    event_id              TEXT PRIMARY KEY,
    tenant_id             TEXT NOT NULL,
    user_id               TEXT NOT NULL,
    caller_app_id         TEXT,
    task_id               TEXT NOT NULL,
    idempotency_key       TEXT,
    method                TEXT NOT NULL,
    capability            TEXT NOT NULL,
    request_model         TEXT NOT NULL,
    provider_instance_name TEXT NOT NULL,
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
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_user_time
    ON aicc_usage_event(user_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_method_time
    ON aicc_usage_event(method, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_provider_instance_time
    ON aicc_usage_event(provider_instance_name, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_model_time
    ON aicc_usage_event(provider_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_request_model_time
    ON aicc_usage_event(request_model, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_task
    ON aicc_usage_event(tenant_id, task_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_idem
    ON aicc_usage_event(tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE TABLE IF NOT EXISTS aicc_route_trace (
    trace_id                 TEXT PRIMARY KEY,
    tenant_id                TEXT NOT NULL,
    caller_app_id            TEXT,
    task_id                  TEXT NOT NULL,
    request_model            TEXT NOT NULL,
    selected_exact_model     TEXT,
    provider_instance_name   TEXT,
    api_type                 TEXT NOT NULL,
    route_trace_json         TEXT NOT NULL,
    created_at_ms            BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_time
    ON aicc_route_trace(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_tenant_time
    ON aicc_route_trace(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_request_model_time
    ON aicc_route_trace(request_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_task_time
    ON aicc_route_trace(task_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_video_continuation_source (
    tenant_id                 TEXT NOT NULL,
    content_id                TEXT NOT NULL,
    source_task_id            TEXT NOT NULL,
    created_at_ms             BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, content_id)
);
CREATE INDEX IF NOT EXISTS idx_aicc_video_continuation_source_task
    ON aicc_video_continuation_source(source_task_id);
"#;

/// Default rdb-instance config for the aicc usage-log. The scheduler drops
/// this into `spec_config.rdb_instances` when bootstrapping the service.
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
        connection: String::new(),
        partitions: vec![RdbPartition::UserData],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiccVideoContinuationSource {
    pub tenant_id: String,
    pub content_id: String,
    pub source_task_id: String,
    pub created_at_ms: i64,
}

/// One durable row in `aicc_usage_event`.
///
/// Token columns are flattened copies of `usage_json` so SQL aggregation can
/// work without parsing JSON. `request_units` is the generic fallback for
/// non-token providers; future extensions (image count, audio seconds, ...)
/// should add their own top-level columns as the schema version bumps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiccUsageEvent {
    pub event_id: String,
    pub tenant_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_app_id: Option<String>,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Canonical typed inference method from [`ai_methods`].
    #[serde(
        serialize_with = "serialize_canonical_typed_method",
        deserialize_with = "deserialize_canonical_typed_method"
    )]
    pub method: String,
    pub capability: String,
    pub request_model: String,
    pub provider_instance_name: String,
    pub provider_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_units: Option<u64>,
    pub usage_json: AiUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finance_snapshot_json: Option<Value>,
    pub created_at_ms: i64,
}

fn serialize_canonical_typed_method<S>(method: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if !ai_methods::is_ai_method(method) {
        return Err(serde::ser::Error::custom(format!(
            "usage method `{method}` is not a canonical typed method"
        )));
    }
    serializer.serialize_str(method)
}

fn deserialize_canonical_typed_method<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let method = String::deserialize(deserializer)?;
    if !ai_methods::is_ai_method(&method) {
        return Err(serde::de::Error::custom(format!(
            "usage method `{method}` is not a canonical typed method"
        )));
    }
    Ok(method)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiccRouteTraceEvent {
    pub trace_id: String,
    pub tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_app_id: Option<String>,
    pub task_id: String,
    pub request_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_exact_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_instance_name: Option<String>,
    pub api_type: String,
    pub route_trace_json: RouteTrace,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QueryRouteTraceRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_instance_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_exact_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scheduler_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

impl QueryRouteTraceRequest {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct QueryRouteTraceResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<u64>,
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
    /// A half-open time interval: `[start_time_ms, end_time_ms)`.
    Explicit {
        start_time_ms: i64,
        end_time_ms: i64,
    },
}

/// Optional `WHERE` filters. Every field is independent; omitted fields mean
/// "no filter on this dimension".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UsageQueryFilters {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenant_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caller_app_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_app_query: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_model_query: Option<String>,
    /// Matches the persisted `provider_instance_name` column directly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_instance_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_instance_query: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub idempotency_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
}

/// Group dimensions supported by `query_usage`. Multiple values produce a
/// multi-dimensional grouping (think `GROUP BY a, b`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum UsageQueryGroup {
    ProviderModel,
    ProviderInstanceName,
    RequestModel,
    Method,
    Capability,
    CallerAppId,
    UserId,
    TenantId,
}

impl UsageQueryGroup {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::ProviderModel => "provider_model",
            Self::ProviderInstanceName => "provider_instance_name",
            Self::RequestModel => "request_model",
            Self::Method => "method",
            Self::Capability => "capability",
            Self::CallerAppId => "caller_app_id",
            Self::UserId => "user_id",
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageQueryOutputMode {
    #[default]
    Summary,
    Events,
    SummaryAndEvents,
}

/// The general query interface mandated by the requirements doc (section 7).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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

impl QueryUsageRequest {
    pub fn new(time_range: UsageQueryTimeRange) -> Self {
        Self {
            time_range,
            filters: UsageQueryFilters::default(),
            group_by: Vec::new(),
            time_bucket: None,
            output_mode: UsageQueryOutputMode::default(),
            limit: None,
            cursor: None,
        }
    }
}

/// Aggregated counts and totals.
///
/// `finance_totals` contains one subtotal per normalized currency, sorted by
/// currency. `finance_complete` is true when every event has a valid finance
/// snapshot; mixed currencies do not make an otherwise valid aggregate
/// incomplete. Consumers may use valid subtotals from an incomplete aggregate,
/// but must not treat them as the complete cost of all events.
/// `consumed_request_units` counts every successful completion as at least one
/// unit: `max(request_units.unwrap_or(1), 1)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UsageAggregate {
    pub total_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub consumed_request_units: u64,
    pub finance_totals: Vec<Money>,
    pub finance_complete: bool,
}

impl Default for UsageAggregate {
    fn default() -> Self {
        Self {
            total_requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            consumed_request_units: 0,
            finance_totals: Vec::new(),
            finance_complete: true,
        }
    }
}

/// One row of a grouped query result. `group` holds `dimension → value`
/// pairs, e.g. `{"provider_model": "gpt4.openai"}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageGroupedRow {
    pub group: HashMap<String, String>,
    pub aggregate: UsageAggregate,
}

/// One row of a time-bucketed result. When a grouping is also set, the same
/// dimension map appears on every bucket row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn usage_event() -> AiccUsageEvent {
        AiccUsageEvent {
            event_id: "event-1".to_string(),
            tenant_id: "tenant-1".to_string(),
            user_id: "user-1".to_string(),
            caller_app_id: Some("app-1".to_string()),
            task_id: "task-1".to_string(),
            idempotency_key: Some("idem-1".to_string()),
            method: ai_methods::CHAT_COMPLETIONS_CREATE.to_string(),
            capability: "chat".to_string(),
            request_model: "smart".to_string(),
            provider_instance_name: "openai-main".to_string(),
            provider_model: "gpt-5.openai-main".to_string(),
            input_tokens: Some(10),
            output_tokens: Some(4),
            total_tokens: Some(14),
            request_units: None,
            usage_json: AiUsage {
                input_tokens: Some(10),
                output_tokens: Some(4),
                total_tokens: Some(14),
                request_units: None,
            },
            finance_snapshot_json: None,
            created_at_ms: 1_750_000_000_000,
        }
    }

    #[test]
    fn usage_event_serde_round_trip_preserves_identity_dimensions() {
        let event = usage_event();
        let value = serde_json::to_value(&event).expect("serialize usage event");

        assert_eq!(value["user_id"], "user-1");
        assert_eq!(value["method"], ai_methods::CHAT_COMPLETIONS_CREATE);
        assert_eq!(value["provider_instance_name"], "openai-main");
        assert_eq!(
            serde_json::from_value::<AiccUsageEvent>(value).expect("deserialize usage event"),
            event
        );
    }

    #[test]
    fn usage_event_rejects_noncanonical_method_and_unknown_fields() {
        let mut invalid_method =
            serde_json::to_value(usage_event()).expect("serialize usage event");
        invalid_method["method"] = json!("provider.list");
        assert!(serde_json::from_value::<AiccUsageEvent>(invalid_method).is_err());

        let mut invalid_event = usage_event();
        invalid_event.method = "provider.list".to_string();
        assert!(serde_json::to_value(invalid_event).is_err());

        let mut unknown_field = serde_json::to_value(usage_event()).expect("serialize usage event");
        unknown_field["unexpected"] = json!(true);
        assert!(serde_json::from_value::<AiccUsageEvent>(unknown_field).is_err());
    }

    #[test]
    fn usage_query_filters_round_trip_and_reject_unknown_fields() {
        let filters = UsageQueryFilters {
            user_ids: vec!["user-1".to_string()],
            methods: vec![ai_methods::EMBEDDING_TEXT.to_string()],
            provider_instance_names: vec!["openai-main".to_string()],
            ..Default::default()
        };
        let value = serde_json::to_value(&filters).expect("serialize usage filters");
        assert_eq!(
            serde_json::from_value::<UsageQueryFilters>(value).expect("deserialize usage filters"),
            filters
        );

        assert!(serde_json::from_value::<UsageQueryFilters>(json!({
            "user_ids": ["user-1"],
            "unknown": true
        }))
        .is_err());
    }

    #[test]
    fn usage_query_group_keys_and_wire_names_are_canonical() {
        let cases = [
            (
                UsageQueryGroup::ProviderInstanceName,
                "provider_instance_name",
            ),
            (UsageQueryGroup::Method, "method"),
            (UsageQueryGroup::UserId, "user_id"),
        ];

        for (group, expected) in cases {
            assert_eq!(group.as_key(), expected);
            assert_eq!(
                serde_json::to_value(group).expect("serialize group"),
                expected
            );
            assert_eq!(
                serde_json::from_value::<UsageQueryGroup>(json!(expected))
                    .expect("deserialize group"),
                group
            );
        }
    }

    #[test]
    fn explicit_time_range_keeps_half_open_bounds_on_wire() {
        let range = UsageQueryTimeRange::Explicit {
            start_time_ms: 100,
            end_time_ms: 200,
        };
        assert_eq!(
            serde_json::to_value(range).expect("serialize time range"),
            json!({
                "kind": "explicit",
                "start_time_ms": 100,
                "end_time_ms": 200
            })
        );
    }

    #[test]
    fn usage_schema_v5_contains_identity_columns_and_indexes() {
        assert_eq!(AICC_USAGE_LOG_RDB_SCHEMA_VERSION, 5);
        for ddl in [
            AICC_USAGE_LOG_RDB_SCHEMA_SQLITE,
            AICC_USAGE_LOG_RDB_SCHEMA_POSTGRES,
        ] {
            for column in [
                "user_id               TEXT NOT NULL",
                "method                TEXT NOT NULL",
                "provider_instance_name TEXT NOT NULL",
            ] {
                assert!(ddl.contains(column), "missing column: {column}");
            }
            for index in [
                "idx_aicc_usage_event_user_time",
                "idx_aicc_usage_event_method_time",
                "idx_aicc_usage_event_provider_instance_time",
            ] {
                assert!(ddl.contains(index), "missing index: {index}");
            }
        }
    }

    #[test]
    fn usage_aggregate_default_is_complete_zero_event_contract() {
        let aggregate = UsageAggregate::default();
        assert_eq!(aggregate.total_requests, 0);
        assert_eq!(aggregate.consumed_request_units, 0);
        assert!(aggregate.finance_totals.is_empty());
        assert!(aggregate.finance_complete);

        let value = serde_json::to_value(&aggregate).expect("serialize aggregate");
        assert_eq!(
            value,
            json!({
                "total_requests": 0,
                "input_tokens": 0,
                "output_tokens": 0,
                "total_tokens": 0,
                "consumed_request_units": 0,
                "finance_totals": [],
                "finance_complete": true
            })
        );
        assert_eq!(
            serde_json::from_value::<UsageAggregate>(value).expect("deserialize aggregate"),
            aggregate
        );
    }

    #[test]
    fn usage_aggregate_exposes_sorted_multi_currency_and_incomplete_contracts() {
        let complete = UsageAggregate {
            total_requests: 2,
            consumed_request_units: 2,
            finance_totals: vec![Money::new(0.25, "EUR"), Money::new(0.5, "USD")],
            finance_complete: true,
            ..Default::default()
        };
        let incomplete = UsageAggregate {
            total_requests: 2,
            consumed_request_units: 3,
            finance_totals: vec![Money::new(0.5, "USD")],
            finance_complete: false,
            ..Default::default()
        };

        let complete_value = serde_json::to_value(&complete).expect("serialize aggregate");
        assert_eq!(
            complete_value["finance_totals"],
            json!([
                {"amount": 0.25, "currency": "EUR"},
                {"amount": 0.5, "currency": "USD"}
            ])
        );

        for aggregate in [complete, incomplete] {
            let value = serde_json::to_value(&aggregate).expect("serialize aggregate");
            assert_eq!(
                serde_json::from_value::<UsageAggregate>(value).expect("deserialize aggregate"),
                aggregate
            );
        }

        assert!(serde_json::from_value::<UsageAggregate>(json!({
            "total_requests": 1,
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0,
            "request_units": 1,
            "finance_amount": 0.5,
            "finance_currency": "USD",
            "finance_complete": true
        }))
        .is_err());
    }
}
