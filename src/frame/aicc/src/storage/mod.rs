#![allow(dead_code)]

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use buckyos_api::{
    get_rdb_instance, AiUsage, AiccRouteTraceEvent, AiccUsageEvent, QueryRouteTraceRequest,
    QueryRouteTraceResponse, QueryUsageRequest, QueryUsageResponse, RdbBackend, UsageAggregate,
    UsageBucketedRow, UsageGroupedRow, UsageQueryGroup, UsageQueryOutputMode, UsageQueryTimeRange,
    AICC_USAGE_LOG_RDB_INSTANCE_ID,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};
use std::collections::BTreeMap;
use std::sync::Once;
use thiserror::Error;

const SERVICE_NAME: &str = "aicc";
const INVENTORY_SCHEMA_VERSION: i64 = 1;
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1_000;
static INSTALL_DRIVERS: Once = Once::new();

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS aicc_provider_inventory_lkgs (
 provider_instance_name TEXT PRIMARY KEY, schema_version INTEGER NOT NULL DEFAULT 1,
 provider_profile_id TEXT NOT NULL, protocol_adapter_id TEXT NOT NULL,
 provider_model_list_fingerprint TEXT NOT NULL, metadata_applied_seq BIGINT NOT NULL,
 inventory_revision TEXT, discovered_at_ms BIGINT NOT NULL, snapshot_json TEXT NOT NULL,
 snapshot_sha256 TEXT NOT NULL, created_at_ms BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_provider_inventory_lkgs_updated ON aicc_provider_inventory_lkgs(updated_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_provider_inventory_lkgs_metadata ON aicc_provider_inventory_lkgs(metadata_applied_seq);
CREATE TABLE IF NOT EXISTS aicc_usage_event (
 event_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, caller_app_id TEXT, task_id TEXT NOT NULL,
 idempotency_key TEXT, capability TEXT NOT NULL, request_model TEXT NOT NULL,
 provider_model TEXT NOT NULL, input_tokens BIGINT, output_tokens BIGINT, total_tokens BIGINT,
 request_units BIGINT, usage_json TEXT NOT NULL, finance_snapshot_json TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_time ON aicc_usage_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_time ON aicc_usage_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_model_time ON aicc_usage_event(provider_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_request_model_time ON aicc_usage_event(request_model, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_task ON aicc_usage_event(tenant_id, task_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_idem ON aicc_usage_event(tenant_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE TABLE IF NOT EXISTS aicc_route_trace_event (
 trace_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, caller_app_id TEXT, task_id TEXT NOT NULL,
 request_id TEXT, route_id TEXT, provider_trace_id TEXT, request_model TEXT NOT NULL,
 selected_exact_model TEXT, provider_instance_name TEXT, api_type TEXT NOT NULL,
 scheduler_profile TEXT, outcome TEXT, route_trace_json TEXT NOT NULL, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_time ON aicc_route_trace_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_tenant_time ON aicc_route_trace_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_task_time ON aicc_route_trace_event(task_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_audit_event (
 audit_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, caller_app_id TEXT, event_type TEXT NOT NULL,
 request_id TEXT, task_id TEXT, route_id TEXT, provider_trace_id TEXT,
 provider_instance_name TEXT, exact_model TEXT, data_json TEXT NOT NULL, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_audit_event_time ON aicc_audit_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_audit_event_tenant_time ON aicc_audit_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_audit_event_task_time ON aicc_audit_event(task_id, created_at_ms);
"#;

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid storage record: {0}")]
    InvalidRecord(String),
    #[error("provider completion is missing usage")]
    MissingUsage,
    #[error("invalid cursor")]
    InvalidCursor,
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
pub(crate) type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct InventoryLkgsRecord {
    pub provider_instance_name: String,
    pub schema_version: i64,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub provider_model_list_fingerprint: String,
    pub metadata_applied_seq: u64,
    pub inventory_revision: Option<String>,
    pub discovered_at_ms: i64,
    pub snapshot: Value,
    pub snapshot_sha256: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl InventoryLkgsRecord {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        provider_instance_name: impl Into<String>,
        provider_profile_id: impl Into<String>,
        protocol_adapter_id: impl Into<String>,
        fingerprint: impl Into<String>,
        metadata_applied_seq: u64,
        inventory_revision: Option<String>,
        discovered_at_ms: i64,
        snapshot: Value,
        now_ms: i64,
    ) -> StorageResult<Self> {
        let snapshot_json = serde_json::to_string(&snapshot)?;
        let record = Self {
            provider_instance_name: provider_instance_name.into(),
            schema_version: 1,
            provider_profile_id: provider_profile_id.into(),
            protocol_adapter_id: protocol_adapter_id.into(),
            provider_model_list_fingerprint: fingerprint.into(),
            metadata_applied_seq,
            inventory_revision,
            discovered_at_ms,
            snapshot,
            snapshot_sha256: sha256_hex(snapshot_json.as_bytes()),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> StorageResult<()> {
        if self.provider_instance_name.trim().is_empty()
            || self.provider_profile_id.trim().is_empty()
            || self.protocol_adapter_id.trim().is_empty()
            || self.provider_model_list_fingerprint.trim().is_empty()
        {
            return Err(StorageError::InvalidRecord(
                "inventory identity is empty".into(),
            ));
        }
        if self.schema_version != INVENTORY_SCHEMA_VERSION
            || self.discovered_at_ms < 0
            || self.created_at_ms < 0
            || self.updated_at_ms < 0
        {
            return Err(StorageError::InvalidRecord(
                "invalid inventory schema or timestamp".into(),
            ));
        }
        let json = serde_json::to_string(&self.snapshot)?;
        if sha256_hex(json.as_bytes()) != self.snapshot_sha256 {
            return Err(StorageError::InvalidRecord(
                "inventory digest mismatch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderCompletion {
    pub event_id: String,
    pub tenant_id: String,
    pub caller_app_id: Option<String>,
    pub task_id: String,
    pub idempotency_key: Option<String>,
    pub capability: String,
    pub request_model: String,
    pub provider_model: String,
    pub usage: Option<AiUsage>,
    pub finance_snapshot: Option<Value>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsageWriteOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct RouteTraceRecord {
    pub trace: AiccRouteTraceEvent,
    pub request_id: Option<String>,
    pub route_id: Option<String>,
    pub provider_trace_id: Option<String>,
    pub scheduler_profile: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct AuditEvent {
    pub audit_id: String,
    pub tenant_id: String,
    pub caller_app_id: Option<String>,
    pub event_type: String,
    pub request_id: Option<String>,
    pub task_id: Option<String>,
    pub route_id: Option<String>,
    pub provider_trace_id: Option<String>,
    pub provider_instance_name: Option<String>,
    pub exact_model: Option<String>,
    pub data: Value,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AuditQuery {
    pub tenant_id: String,
    pub event_types: Vec<String>,
    pub request_ids: Vec<String>,
    pub task_ids: Vec<String>,
    pub route_ids: Vec<String>,
    pub provider_trace_ids: Vec<String>,
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AuditQueryResult {
    pub events: Vec<AuditEvent>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RetentionResult {
    pub route_traces_deleted: u64,
    pub audit_events_deleted: u64,
}

pub(crate) struct AiccStorage {
    pool: AnyPool,
    backend: RdbBackend,
}

impl AiccStorage {
    pub(crate) async fn open(connection: &str, backend: RdbBackend) -> StorageResult<Self> {
        INSTALL_DRIVERS.call_once(install_default_drivers);
        let connections = if backend == RdbBackend::Sqlite && connection.contains(":memory:") {
            1
        } else {
            8
        };
        let pool = AnyPoolOptions::new()
            .max_connections(connections)
            .connect(connection)
            .await?;
        let storage = Self { pool, backend };
        for statement in SCHEMA.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            storage.pool.execute(statement).await?;
        }
        Ok(storage)
    }

    pub(crate) async fn open_from_service_spec() -> StorageResult<Self> {
        let instance = get_rdb_instance(SERVICE_NAME, None, AICC_USAGE_LOG_RDB_INSTANCE_ID)
            .await
            .map_err(|e| StorageError::InvalidRecord(e.to_string()))?;
        Self::open(&instance.connection, instance.backend).await
    }

    pub(crate) async fn upsert_inventory(&self, record: &InventoryLkgsRecord) -> StorageResult<()> {
        record.validate()?;
        let sql = self.sql("INSERT INTO aicc_provider_inventory_lkgs
          (provider_instance_name,schema_version,provider_profile_id,protocol_adapter_id,
           provider_model_list_fingerprint,metadata_applied_seq,inventory_revision,discovered_at_ms,
           snapshot_json,snapshot_sha256,created_at_ms,updated_at_ms)
          VALUES (?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(provider_instance_name) DO UPDATE SET
           schema_version=excluded.schema_version,provider_profile_id=excluded.provider_profile_id,
           protocol_adapter_id=excluded.protocol_adapter_id,
           provider_model_list_fingerprint=excluded.provider_model_list_fingerprint,
           metadata_applied_seq=excluded.metadata_applied_seq,inventory_revision=excluded.inventory_revision,
           discovered_at_ms=excluded.discovered_at_ms,snapshot_json=excluded.snapshot_json,
           snapshot_sha256=excluded.snapshot_sha256,updated_at_ms=excluded.updated_at_ms");
        let mut tx = self.pool.begin().await?;
        sqlx::query(&sql)
            .bind(&record.provider_instance_name)
            .bind(record.schema_version)
            .bind(&record.provider_profile_id)
            .bind(&record.protocol_adapter_id)
            .bind(&record.provider_model_list_fingerprint)
            .bind(to_i64(record.metadata_applied_seq)?)
            .bind(&record.inventory_revision)
            .bind(record.discovered_at_ms)
            .bind(serde_json::to_string(&record.snapshot)?)
            .bind(&record.snapshot_sha256)
            .bind(record.created_at_ms)
            .bind(record.updated_at_ms)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn load_inventory(
        &self,
        name: &str,
    ) -> StorageResult<Option<InventoryLkgsRecord>> {
        let sql =
            self.sql("SELECT * FROM aicc_provider_inventory_lkgs WHERE provider_instance_name=?");
        let Some(row) = sqlx::query(&sql)
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };
        match inventory_from_row(row).and_then(|r| {
            r.validate()?;
            Ok(r)
        }) {
            Ok(record) => Ok(Some(record)),
            Err(_) => {
                let sql = self
                    .sql("DELETE FROM aicc_provider_inventory_lkgs WHERE provider_instance_name=?");
                sqlx::query(&sql).bind(name).execute(&self.pool).await?;
                Ok(None)
            }
        }
    }

    pub(crate) async fn list_inventory_behind(
        &self,
        seq: u64,
    ) -> StorageResult<Vec<InventoryLkgsRecord>> {
        let sql = self.sql("SELECT * FROM aicc_provider_inventory_lkgs WHERE metadata_applied_seq<? ORDER BY provider_instance_name");
        let rows = sqlx::query(&sql)
            .bind(to_i64(seq)?)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| inventory_from_row(row).ok())
            .filter(|r| r.validate().is_ok())
            .collect())
    }

    pub(crate) async fn write_provider_completion(
        &self,
        completion: ProviderCompletion,
    ) -> StorageResult<UsageWriteOutcome> {
        let usage = completion.usage.ok_or(StorageError::MissingUsage)?;
        if usage.input_tokens.is_none()
            && usage.output_tokens.is_none()
            && usage.total_tokens.is_none()
            && usage.request_units.is_none()
        {
            return Err(StorageError::MissingUsage);
        }
        let event = AiccUsageEvent {
            event_id: completion.event_id,
            tenant_id: completion.tenant_id,
            caller_app_id: completion.caller_app_id,
            task_id: completion.task_id,
            idempotency_key: completion.idempotency_key,
            capability: completion.capability,
            request_model: completion.request_model,
            provider_model: completion.provider_model,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            request_units: usage.request_units,
            usage_json: usage,
            finance_snapshot_json: completion.finance_snapshot,
            created_at_ms: completion.created_at_ms,
        };
        self.write_usage(&event).await
    }

    async fn write_usage(&self, e: &AiccUsageEvent) -> StorageResult<UsageWriteOutcome> {
        if [
            &e.event_id,
            &e.tenant_id,
            &e.task_id,
            &e.capability,
            &e.request_model,
            &e.provider_model,
        ]
        .iter()
        .any(|v| v.trim().is_empty())
            || e.created_at_ms < 0
        {
            return Err(StorageError::InvalidRecord(
                "usage attribution is incomplete".into(),
            ));
        }
        let sql = self.sql("INSERT INTO aicc_usage_event
          (event_id,tenant_id,caller_app_id,task_id,idempotency_key,capability,request_model,
           provider_model,input_tokens,output_tokens,total_tokens,request_units,usage_json,
           finance_snapshot_json,created_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING");
        let result = sqlx::query(&sql)
            .bind(&e.event_id)
            .bind(&e.tenant_id)
            .bind(&e.caller_app_id)
            .bind(&e.task_id)
            .bind(&e.idempotency_key)
            .bind(&e.capability)
            .bind(&e.request_model)
            .bind(&e.provider_model)
            .bind(opt_i64(e.input_tokens)?)
            .bind(opt_i64(e.output_tokens)?)
            .bind(opt_i64(e.total_tokens)?)
            .bind(opt_i64(e.request_units)?)
            .bind(serde_json::to_string(&e.usage_json)?)
            .bind(
                e.finance_snapshot_json
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
            )
            .bind(e.created_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(if result.rows_affected() == 1 {
            UsageWriteOutcome::Inserted
        } else {
            UsageWriteOutcome::Duplicate
        })
    }

    pub(crate) async fn query_usage(
        &self,
        req: &QueryUsageRequest,
        now_ms: i64,
    ) -> StorageResult<QueryUsageResponse> {
        let (start, end) = time_range(&req.time_range, now_ms)?;
        let sql = self.sql("SELECT * FROM aicc_usage_event WHERE created_at_ms>=? AND created_at_ms<? ORDER BY created_at_ms DESC,event_id DESC");
        let rows = sqlx::query(&sql)
            .bind(start)
            .bind(end)
            .fetch_all(&self.pool)
            .await?;
        let mut events = rows
            .into_iter()
            .map(usage_from_row)
            .collect::<StorageResult<Vec<_>>>()?;
        events.retain(|e| usage_matches(e, &req.filters));
        let total = aggregate(events.iter());
        let grouped = grouped(&events, &req.group_by);
        let buckets = bucketed(&events, &req.group_by, req.time_bucket);
        let mut raw = Vec::new();
        let mut next_cursor = None;
        if matches!(
            req.output_mode,
            UsageQueryOutputMode::Events | UsageQueryOutputMode::SummaryAndEvents
        ) {
            let cursor = req.cursor.as_deref().map(decode_cursor).transpose()?;
            raw = events
                .into_iter()
                .filter(|e| cursor_allows(e.created_at_ms, &e.event_id, cursor.as_ref()))
                .collect();
            let limit = limit(req.limit);
            if raw.len() > limit {
                next_cursor = Some(encode_cursor(
                    raw[limit - 1].created_at_ms,
                    &raw[limit - 1].event_id,
                ));
                raw.truncate(limit);
            }
        }
        Ok(QueryUsageResponse {
            total,
            grouped,
            buckets,
            events: raw,
            next_cursor,
        })
    }

    pub(crate) async fn write_route_trace(&self, r: &RouteTraceRecord) -> StorageResult<()> {
        if r.trace.trace_id.trim().is_empty()
            || r.trace.tenant_id.trim().is_empty()
            || r.trace.task_id.trim().is_empty()
            || r.trace.created_at_ms < 0
        {
            return Err(StorageError::InvalidRecord(
                "trace identity is incomplete".into(),
            ));
        }
        let sql = self.sql("INSERT INTO aicc_route_trace_event
          (trace_id,tenant_id,caller_app_id,task_id,request_id,route_id,provider_trace_id,
           request_model,selected_exact_model,provider_instance_name,api_type,scheduler_profile,
           outcome,route_trace_json,created_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(trace_id) DO NOTHING");
        sqlx::query(&sql)
            .bind(&r.trace.trace_id)
            .bind(&r.trace.tenant_id)
            .bind(&r.trace.caller_app_id)
            .bind(&r.trace.task_id)
            .bind(&r.request_id)
            .bind(&r.route_id)
            .bind(&r.provider_trace_id)
            .bind(&r.trace.request_model)
            .bind(&r.trace.selected_exact_model)
            .bind(&r.trace.provider_instance_name)
            .bind(&r.trace.api_type)
            .bind(&r.scheduler_profile)
            .bind(&r.outcome)
            .bind(serde_json::to_string(&r.trace.route_trace_json)?)
            .bind(r.trace.created_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn query_route_traces(
        &self,
        tenant: &str,
        req: &QueryRouteTraceRequest,
    ) -> StorageResult<QueryRouteTraceResponse> {
        let sql = self.sql("SELECT * FROM aicc_route_trace_event WHERE tenant_id=? ORDER BY created_at_ms DESC,trace_id DESC");
        let rows = sqlx::query(&sql).bind(tenant).fetch_all(&self.pool).await?;
        let cursor = req.cursor.as_deref().map(decode_cursor).transpose()?;
        let mut records = rows
            .into_iter()
            .map(trace_from_row)
            .collect::<StorageResult<Vec<_>>>()?;
        records.retain(|r| trace_matches(r, req));
        let total_count = records.len() as u64;
        records
            .retain(|r| cursor_allows(r.trace.created_at_ms, &r.trace.trace_id, cursor.as_ref()));
        let page_limit = limit(req.limit);
        let next_cursor = (records.len() > page_limit).then(|| {
            encode_cursor(
                records[page_limit - 1].trace.created_at_ms,
                &records[page_limit - 1].trace.trace_id,
            )
        });
        records.truncate(page_limit);
        Ok(QueryRouteTraceResponse {
            traces: records
                .into_iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?,
            next_cursor,
            total_count: Some(total_count),
        })
    }

    pub(crate) async fn write_audit_event(&self, e: &AuditEvent) -> StorageResult<()> {
        if e.audit_id.trim().is_empty()
            || e.tenant_id.trim().is_empty()
            || e.event_type.trim().is_empty()
            || e.created_at_ms < 0
        {
            return Err(StorageError::InvalidRecord(
                "audit identity is incomplete".into(),
            ));
        }
        let sql = self.sql("INSERT INTO aicc_audit_event
          (audit_id,tenant_id,caller_app_id,event_type,request_id,task_id,route_id,provider_trace_id,
           provider_instance_name,exact_model,data_json,created_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
           ON CONFLICT(audit_id) DO NOTHING");
        sqlx::query(&sql)
            .bind(&e.audit_id)
            .bind(&e.tenant_id)
            .bind(&e.caller_app_id)
            .bind(&e.event_type)
            .bind(&e.request_id)
            .bind(&e.task_id)
            .bind(&e.route_id)
            .bind(&e.provider_trace_id)
            .bind(&e.provider_instance_name)
            .bind(&e.exact_model)
            .bind(serde_json::to_string(&e.data)?)
            .bind(e.created_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn query_audit(&self, q: &AuditQuery) -> StorageResult<AuditQueryResult> {
        if q.tenant_id.trim().is_empty() {
            return Err(StorageError::InvalidRecord("audit tenant is empty".into()));
        }
        let sql = self.sql("SELECT * FROM aicc_audit_event WHERE tenant_id=? ORDER BY created_at_ms DESC,audit_id DESC");
        let rows = sqlx::query(&sql)
            .bind(&q.tenant_id)
            .fetch_all(&self.pool)
            .await?;
        let cursor = q.cursor.as_deref().map(decode_cursor).transpose()?;
        let mut events = rows
            .into_iter()
            .map(audit_from_row)
            .collect::<StorageResult<Vec<_>>>()?;
        events.retain(|e| {
            audit_matches(e, q) && cursor_allows(e.created_at_ms, &e.audit_id, cursor.as_ref())
        });
        let page_limit = limit(q.limit);
        let next_cursor = (events.len() > page_limit).then(|| {
            encode_cursor(
                events[page_limit - 1].created_at_ms,
                &events[page_limit - 1].audit_id,
            )
        });
        events.truncate(page_limit);
        Ok(AuditQueryResult {
            events,
            next_cursor,
        })
    }

    pub(crate) async fn enforce_diagnostic_retention(
        &self,
        cutoff_ms: i64,
    ) -> StorageResult<RetentionResult> {
        if cutoff_ms < 0 {
            return Err(StorageError::InvalidRecord(
                "negative retention cutoff".into(),
            ));
        }
        let trace_sql = self.sql("DELETE FROM aicc_route_trace_event WHERE created_at_ms<?");
        let audit_sql = self.sql("DELETE FROM aicc_audit_event WHERE created_at_ms<?");
        let mut tx = self.pool.begin().await?;
        let traces = sqlx::query(&trace_sql)
            .bind(cutoff_ms)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let audits = sqlx::query(&audit_sql)
            .bind(cutoff_ms)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(RetentionResult {
            route_traces_deleted: traces,
            audit_events_deleted: audits,
        })
    }

    fn sql(&self, sql: &str) -> String {
        if self.backend == RdbBackend::Postgres {
            placeholders(sql)
        } else {
            sql.to_string()
        }
    }
}

fn inventory_from_row(row: AnyRow) -> StorageResult<InventoryLkgsRecord> {
    let snapshot: String = row.try_get("snapshot_json")?;
    Ok(InventoryLkgsRecord {
        provider_instance_name: row.try_get("provider_instance_name")?,
        schema_version: row.try_get("schema_version")?,
        provider_profile_id: row.try_get("provider_profile_id")?,
        protocol_adapter_id: row.try_get("protocol_adapter_id")?,
        provider_model_list_fingerprint: row.try_get("provider_model_list_fingerprint")?,
        metadata_applied_seq: from_i64(row.try_get("metadata_applied_seq")?)?,
        inventory_revision: row.try_get("inventory_revision")?,
        discovered_at_ms: row.try_get("discovered_at_ms")?,
        snapshot: serde_json::from_str(&snapshot)?,
        snapshot_sha256: row.try_get("snapshot_sha256")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn usage_from_row(row: AnyRow) -> StorageResult<AiccUsageEvent> {
    let usage: String = row.try_get("usage_json")?;
    let finance: Option<String> = row.try_get("finance_snapshot_json")?;
    Ok(AiccUsageEvent {
        event_id: row.try_get("event_id")?,
        tenant_id: row.try_get("tenant_id")?,
        caller_app_id: row.try_get("caller_app_id")?,
        task_id: row.try_get("task_id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        capability: row.try_get("capability")?,
        request_model: row.try_get("request_model")?,
        provider_model: row.try_get("provider_model")?,
        input_tokens: opt_from_i64(row.try_get("input_tokens")?)?,
        output_tokens: opt_from_i64(row.try_get("output_tokens")?)?,
        total_tokens: opt_from_i64(row.try_get("total_tokens")?)?,
        request_units: opt_from_i64(row.try_get("request_units")?)?,
        usage_json: serde_json::from_str(&usage)?,
        finance_snapshot_json: finance.map(|v| serde_json::from_str(&v)).transpose()?,
        created_at_ms: row.try_get("created_at_ms")?,
    })
}

fn trace_from_row(row: AnyRow) -> StorageResult<RouteTraceRecord> {
    let trace: String = row.try_get("route_trace_json")?;
    Ok(RouteTraceRecord {
        trace: AiccRouteTraceEvent {
            trace_id: row.try_get("trace_id")?,
            tenant_id: row.try_get("tenant_id")?,
            caller_app_id: row.try_get("caller_app_id")?,
            task_id: row.try_get("task_id")?,
            request_model: row.try_get("request_model")?,
            selected_exact_model: row.try_get("selected_exact_model")?,
            provider_instance_name: row.try_get("provider_instance_name")?,
            api_type: row.try_get("api_type")?,
            route_trace_json: serde_json::from_str(&trace)?,
            created_at_ms: row.try_get("created_at_ms")?,
        },
        request_id: row.try_get("request_id")?,
        route_id: row.try_get("route_id")?,
        provider_trace_id: row.try_get("provider_trace_id")?,
        scheduler_profile: row.try_get("scheduler_profile")?,
        outcome: row.try_get("outcome")?,
    })
}

fn audit_from_row(row: AnyRow) -> StorageResult<AuditEvent> {
    let data: String = row.try_get("data_json")?;
    Ok(AuditEvent {
        audit_id: row.try_get("audit_id")?,
        tenant_id: row.try_get("tenant_id")?,
        caller_app_id: row.try_get("caller_app_id")?,
        event_type: row.try_get("event_type")?,
        request_id: row.try_get("request_id")?,
        task_id: row.try_get("task_id")?,
        route_id: row.try_get("route_id")?,
        provider_trace_id: row.try_get("provider_trace_id")?,
        provider_instance_name: row.try_get("provider_instance_name")?,
        exact_model: row.try_get("exact_model")?,
        data: serde_json::from_str(&data)?,
        created_at_ms: row.try_get("created_at_ms")?,
    })
}

fn time_range(range: &UsageQueryTimeRange, now: i64) -> StorageResult<(i64, i64)> {
    let day = 86_400_000i64;
    let value = match range {
        UsageQueryTimeRange::Last1d => (now.saturating_sub(day), now),
        UsageQueryTimeRange::Last7d => (now.saturating_sub(7 * day), now),
        UsageQueryTimeRange::Last30d => (now.saturating_sub(30 * day), now),
        UsageQueryTimeRange::Explicit {
            start_time_ms,
            end_time_ms,
        } => (*start_time_ms, *end_time_ms),
    };
    if value.0 < 0 || value.1 <= value.0 {
        Err(StorageError::InvalidRecord("invalid time range".into()))
    } else {
        Ok(value)
    }
}

fn usage_matches(e: &AiccUsageEvent, f: &buckyos_api::UsageQueryFilters) -> bool {
    exact(&f.tenant_ids, Some(&e.tenant_id))
        && exact(&f.caller_app_ids, e.caller_app_id.as_deref())
        && fuzzy(f.caller_app_query.as_deref(), e.caller_app_id.as_deref())
        && exact(&f.request_models, Some(&e.request_model))
        && exact(&f.provider_models, Some(&e.provider_model))
        && fuzzy(f.provider_model_query.as_deref(), Some(&e.provider_model))
        && exact(
            &f.provider_instance_names,
            provider_instance(&e.provider_model),
        )
        && fuzzy(
            f.provider_instance_query.as_deref(),
            provider_instance(&e.provider_model),
        )
        && exact(&f.capabilities, Some(&e.capability))
        && exact(&f.task_ids, Some(&e.task_id))
        && exact(&f.idempotency_keys, e.idempotency_key.as_deref())
}

fn grouped(events: &[AiccUsageEvent], groups: &[UsageQueryGroup]) -> Vec<UsageGroupedRow> {
    if groups.is_empty() {
        return Vec::new();
    }
    let mut map: BTreeMap<Vec<String>, Vec<&AiccUsageEvent>> = BTreeMap::new();
    for e in events {
        map.entry(group_values(e, groups)).or_default().push(e);
    }
    map.into_iter()
        .map(|(values, events)| UsageGroupedRow {
            group: groups
                .iter()
                .zip(values)
                .map(|(g, v)| (g.as_key().to_string(), v))
                .collect(),
            aggregate: aggregate(events),
        })
        .collect()
}

fn bucketed(
    events: &[AiccUsageEvent],
    groups: &[UsageQueryGroup],
    bucket: Option<buckyos_api::UsageQueryBucket>,
) -> Vec<UsageBucketedRow> {
    let Some(bucket) = bucket else {
        return Vec::new();
    };
    let span = bucket.span_ms();
    let mut map: BTreeMap<(i64, Vec<String>), Vec<&AiccUsageEvent>> = BTreeMap::new();
    for e in events {
        map.entry((
            e.created_at_ms.div_euclid(span) * span,
            group_values(e, groups),
        ))
        .or_default()
        .push(e);
    }
    map.into_iter()
        .map(|((bucket_start_ms, values), events)| UsageBucketedRow {
            bucket_start_ms,
            group: groups
                .iter()
                .zip(values)
                .map(|(g, v)| (g.as_key().to_string(), v))
                .collect(),
            aggregate: aggregate(events),
        })
        .collect()
}

fn group_values(e: &AiccUsageEvent, groups: &[UsageQueryGroup]) -> Vec<String> {
    groups
        .iter()
        .map(|g| match g {
            UsageQueryGroup::ProviderModel => e.provider_model.clone(),
            UsageQueryGroup::RequestModel => e.request_model.clone(),
            UsageQueryGroup::Capability => e.capability.clone(),
            UsageQueryGroup::CallerAppId => e.caller_app_id.clone().unwrap_or_default(),
            UsageQueryGroup::TenantId => e.tenant_id.clone(),
        })
        .collect()
}

fn aggregate<'a>(events: impl IntoIterator<Item = &'a AiccUsageEvent>) -> UsageAggregate {
    let mut a = UsageAggregate::default();
    let mut finance = 0.0;
    let mut currency: Option<&str> = None;
    let mut comparable = true;
    let mut found = false;
    for e in events {
        a.total_requests += 1;
        a.input_tokens += e.input_tokens.unwrap_or(0);
        a.output_tokens += e.output_tokens.unwrap_or(0);
        a.total_tokens += e.total_tokens.unwrap_or(0);
        a.request_units += e.request_units.unwrap_or(0);
        if let Some(v) = &e.finance_snapshot_json {
            match (
                v.get("amount").and_then(Value::as_f64),
                v.get("currency").and_then(Value::as_str),
            ) {
                (Some(amount), Some(unit)) => {
                    found = true;
                    comparable &= currency.is_none_or(|known| known == unit);
                    currency = Some(unit);
                    finance += amount;
                }
                _ => comparable = false,
            }
        }
    }
    if found && comparable {
        a.finance_amount = Some(finance)
    }
    a
}

fn trace_matches(r: &RouteTraceRecord, q: &QueryRouteTraceRequest) -> bool {
    q.start_time_ms.is_none_or(|v| r.trace.created_at_ms >= v)
        && q.end_time_ms.is_none_or(|v| r.trace.created_at_ms < v)
        && exact(&q.task_ids, Some(&r.trace.task_id))
        && exact(&q.request_ids, r.request_id.as_deref())
        && exact(&q.api_types, Some(&r.trace.api_type))
        && exact(
            &q.provider_instance_names,
            r.trace.provider_instance_name.as_deref(),
        )
        && exact(
            &q.selected_exact_models,
            r.trace.selected_exact_model.as_deref(),
        )
        && exact(&q.scheduler_profiles, r.scheduler_profile.as_deref())
        && q.outcome
            .as_deref()
            .is_none_or(|v| r.outcome.as_deref() == Some(v))
        && q.query.as_deref().is_none_or(|query| {
            let query = query.to_lowercase();
            [
                Some(r.trace.trace_id.as_str()),
                Some(r.trace.task_id.as_str()),
                r.request_id.as_deref(),
                r.route_id.as_deref(),
                r.provider_trace_id.as_deref(),
                r.trace.selected_exact_model.as_deref(),
                r.trace.provider_instance_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|v| v.to_lowercase().contains(&query))
        })
}

fn audit_matches(e: &AuditEvent, q: &AuditQuery) -> bool {
    q.start_time_ms.is_none_or(|v| e.created_at_ms >= v)
        && q.end_time_ms.is_none_or(|v| e.created_at_ms < v)
        && exact(&q.event_types, Some(&e.event_type))
        && exact(&q.request_ids, e.request_id.as_deref())
        && exact(&q.task_ids, e.task_id.as_deref())
        && exact(&q.route_ids, e.route_id.as_deref())
        && exact(&q.provider_trace_ids, e.provider_trace_id.as_deref())
}

fn exact(values: &[String], actual: Option<&str>) -> bool {
    values.is_empty() || actual.is_some_and(|a| values.iter().any(|v| v == a))
}
fn fuzzy(query: Option<&str>, actual: Option<&str>) -> bool {
    query.is_none_or(|q| actual.is_some_and(|a| a.to_lowercase().contains(&q.to_lowercase())))
}
fn provider_instance(model: &str) -> Option<&str> {
    model.rsplit_once('@').map(|(_, instance)| instance)
}
fn limit(value: Option<u32>) -> usize {
    value
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT)
}
fn encode_cursor(timestamp: i64, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{timestamp}\0{id}"))
}
fn decode_cursor(value: &str) -> StorageResult<(i64, String)> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| StorageError::InvalidCursor)?;
    let text = String::from_utf8(bytes).map_err(|_| StorageError::InvalidCursor)?;
    let (timestamp, id) = text.split_once('\0').ok_or(StorageError::InvalidCursor)?;
    if id.is_empty() {
        return Err(StorageError::InvalidCursor);
    }
    Ok((
        timestamp.parse().map_err(|_| StorageError::InvalidCursor)?,
        id.into(),
    ))
}
fn cursor_allows(timestamp: i64, id: &str, cursor: Option<&(i64, String)>) -> bool {
    cursor.is_none_or(|(t, i)| timestamp < *t || (timestamp == *t && id < i.as_str()))
}
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}
fn to_i64(value: u64) -> StorageResult<i64> {
    i64::try_from(value).map_err(|_| StorageError::InvalidRecord("integer overflow".into()))
}
fn opt_i64(value: Option<u64>) -> StorageResult<Option<i64>> {
    value.map(to_i64).transpose()
}
fn from_i64(value: i64) -> StorageResult<u64> {
    u64::try_from(value).map_err(|_| StorageError::InvalidRecord("negative integer".into()))
}
fn opt_from_i64(value: Option<i64>) -> StorageResult<Option<u64>> {
    value.map(from_i64).transpose()
}
fn placeholders(sql: &str) -> String {
    let mut index = 0;
    sql.chars()
        .fold(String::with_capacity(sql.len()), |mut out, c| {
            if c == '?' {
                index += 1;
                out.push('$');
                out.push_str(&index.to_string())
            } else {
                out.push(c)
            }
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{RouteTrace, UsageQueryBucket, UsageQueryFilters};
    use serde_json::json;

    async fn db() -> AiccStorage {
        AiccStorage::open("sqlite::memory:", RdbBackend::Sqlite)
            .await
            .unwrap()
    }
    fn completion(id: &str, task: &str, idem: &str, at: i64) -> ProviderCompletion {
        ProviderCompletion {
            event_id: id.into(),
            tenant_id: "tenant-a".into(),
            caller_app_id: Some("app-a".into()),
            task_id: task.into(),
            idempotency_key: Some(idem.into()),
            capability: "llm".into(),
            request_model: "llm.chat".into(),
            provider_model: "gpt-5:reasoning@openai-primary".into(),
            usage: Some(AiUsage {
                input_tokens: Some(10),
                output_tokens: Some(5),
                total_tokens: Some(15),
                request_units: None,
            }),
            finance_snapshot: Some(json!({"amount": 0.25, "currency": "USD"})),
            created_at_ms: at,
        }
    }

    #[tokio::test]
    async fn inventory_round_trip_and_corrupt_row_rebuild() {
        let db = db().await;
        let mut record = InventoryLkgsRecord::new(
            "openai-primary",
            "openai",
            "openai-responses",
            "f1",
            4,
            None,
            10,
            json!({"models":["gpt-5"]}),
            10,
        )
        .unwrap();
        db.upsert_inventory(&record).await.unwrap();
        record.metadata_applied_seq = 5;
        record.updated_at_ms = 20;
        db.upsert_inventory(&record).await.unwrap();
        assert_eq!(
            db.load_inventory("openai-primary")
                .await
                .unwrap()
                .unwrap()
                .metadata_applied_seq,
            5
        );
        sqlx::query("UPDATE aicc_provider_inventory_lkgs SET snapshot_sha256='bad'")
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db.load_inventory("openai-primary").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn usage_is_required_deduplicated_and_queryable() {
        let db = db().await;
        let mut missing = completion("e0", "t0", "i0", 9_000);
        missing.usage = None;
        assert!(matches!(
            db.write_provider_completion(missing).await,
            Err(StorageError::MissingUsage)
        ));
        assert_eq!(
            db.write_provider_completion(completion("e1", "t1", "i1", 9_000))
                .await
                .unwrap(),
            UsageWriteOutcome::Inserted
        );
        assert_eq!(
            db.write_provider_completion(completion("e2", "t1", "i2", 10_000))
                .await
                .unwrap(),
            UsageWriteOutcome::Duplicate
        );
        assert_eq!(
            db.write_provider_completion(completion("e3", "t3", "i1", 10_000))
                .await
                .unwrap(),
            UsageWriteOutcome::Duplicate
        );
        db.write_provider_completion(completion("e4", "t4", "i4", 10_000))
            .await
            .unwrap();
        let request = QueryUsageRequest {
            time_range: UsageQueryTimeRange::Explicit {
                start_time_ms: 1,
                end_time_ms: 20_000,
            },
            filters: UsageQueryFilters::default(),
            group_by: vec![UsageQueryGroup::ProviderModel],
            time_bucket: Some(UsageQueryBucket::Hour),
            output_mode: UsageQueryOutputMode::SummaryAndEvents,
            limit: Some(1),
            cursor: None,
        };
        let first = db.query_usage(&request, 20_000).await.unwrap();
        assert_eq!(first.total.total_requests, 2);
        assert_eq!(first.total.total_tokens, 30);
        assert_eq!(first.total.finance_amount, Some(0.5));
        assert_eq!(first.grouped.len(), 1);
        assert_eq!(first.buckets.len(), 1);
        assert_eq!(first.events.len(), 1);
        assert!(first.next_cursor.is_some());
        let mut next = request;
        next.cursor = first.next_cursor;
        assert_eq!(db.query_usage(&next, 20_000).await.unwrap().events.len(), 1);
    }

    #[tokio::test]
    async fn trace_audit_are_correlated_and_retained() {
        let db = db().await;
        db.write_route_trace(&RouteTraceRecord {
            trace: AiccRouteTraceEvent {
                trace_id: "trace-1".into(),
                tenant_id: "tenant-a".into(),
                caller_app_id: Some("app-a".into()),
                task_id: "task-1".into(),
                request_model: "llm.chat".into(),
                selected_exact_model: Some("gpt-5@openai-primary".into()),
                provider_instance_name: Some("openai-primary".into()),
                api_type: "llm".into(),
                route_trace_json: RouteTrace::default(),
                created_at_ms: 100,
            },
            request_id: Some("request-1".into()),
            route_id: Some("route-1".into()),
            provider_trace_id: Some("provider-1".into()),
            scheduler_profile: Some("balanced".into()),
            outcome: Some("succeeded".into()),
        })
        .await
        .unwrap();
        db.write_audit_event(&AuditEvent {
            audit_id: "audit-1".into(),
            tenant_id: "tenant-a".into(),
            caller_app_id: None,
            event_type: "provider.completed".into(),
            request_id: Some("request-1".into()),
            task_id: Some("task-1".into()),
            route_id: Some("route-1".into()),
            provider_trace_id: Some("provider-1".into()),
            provider_instance_name: Some("openai-primary".into()),
            exact_model: Some("gpt-5@openai-primary".into()),
            data: json!({"status":"succeeded"}),
            created_at_ms: 100,
        })
        .await
        .unwrap();
        let traces = db
            .query_route_traces(
                "tenant-a",
                &QueryRouteTraceRequest {
                    limit: None,
                    cursor: None,
                    start_time_ms: None,
                    end_time_ms: None,
                    task_ids: vec!["task-1".into()],
                    request_ids: vec!["request-1".into()],
                    api_types: vec![],
                    provider_instance_names: vec![],
                    selected_exact_models: vec![],
                    scheduler_profiles: vec![],
                    query: None,
                    outcome: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(traces.total_count, Some(1));
        let audits = db
            .query_audit(&AuditQuery {
                tenant_id: "tenant-a".into(),
                task_ids: vec!["task-1".into()],
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(audits.events.len(), 1);
        let deleted = db.enforce_diagnostic_retention(101).await.unwrap();
        assert_eq!(
            deleted,
            RetentionResult {
                route_traces_deleted: 1,
                audit_events_deleted: 1
            }
        );
    }

    #[test]
    fn postgres_placeholders_are_numbered() {
        assert_eq!(placeholders("a=? AND b=?"), "a=$1 AND b=$2")
    }
}
