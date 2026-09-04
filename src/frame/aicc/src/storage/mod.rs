#![allow(dead_code)]

use crate::execution::{
    ExecutionOutput, ExecutionRecord, ExecutionState, ExecutionStore, IdempotencyClaim,
    PinnedProviderTask, UsageCompletion, UsageCompletionPort,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use buckyos_api::{
    ai_methods, get_rdb_instance, AiUsage, AiccError, AiccErrorCode, AiccRouteTraceEvent,
    AiccUsageEvent, Money, QueryRouteTraceRequest, QueryRouteTraceResponse, QueryUsageRequest,
    QueryUsageResponse, RdbBackend, UsageAggregate, UsageBucketedRow, UsageGroupedRow,
    UsageQueryGroup, UsageQueryOutputMode, UsageQueryTimeRange, AICC_USAGE_LOG_RDB_INSTANCE_ID,
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
 event_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL,
 caller_app_id TEXT, task_id TEXT NOT NULL, idempotency_key TEXT, method TEXT NOT NULL,
 capability TEXT NOT NULL, request_model TEXT NOT NULL, provider_instance_name TEXT NOT NULL,
 provider_model TEXT NOT NULL, input_tokens BIGINT, output_tokens BIGINT, total_tokens BIGINT,
 request_units BIGINT, usage_json TEXT NOT NULL, finance_snapshot_json TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_time ON aicc_usage_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_time ON aicc_usage_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_user_time ON aicc_usage_event(user_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_method_time ON aicc_usage_event(method, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_provider_instance_time ON aicc_usage_event(provider_instance_name, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_model_time ON aicc_usage_event(provider_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_request_model_time ON aicc_usage_event(request_model, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_task ON aicc_usage_event(tenant_id, task_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_idem ON aicc_usage_event(tenant_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE TABLE IF NOT EXISTS aicc_execution_record (
 tenant_id TEXT NOT NULL, method TEXT NOT NULL, idempotency_key TEXT NOT NULL,
 task_id TEXT NOT NULL UNIQUE, body_fingerprint TEXT NOT NULL, state TEXT NOT NULL,
 record_json TEXT NOT NULL, created_at_ms BIGINT NOT NULL, expires_at_ms BIGINT NOT NULL,
 PRIMARY KEY (tenant_id, method, idempotency_key));
CREATE INDEX IF NOT EXISTS idx_aicc_execution_record_state ON aicc_execution_record(state, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_execution_record_expiry ON aicc_execution_record(expires_at_ms);
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
    pub user_id: String,
    pub caller_app_id: Option<String>,
    pub task_id: String,
    pub idempotency_key: Option<String>,
    pub method: String,
    pub capability: String,
    pub request_model: String,
    pub provider_instance_name: String,
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
            user_id: completion.user_id,
            caller_app_id: completion.caller_app_id,
            task_id: completion.task_id,
            idempotency_key: completion.idempotency_key,
            method: completion.method,
            capability: completion.capability,
            request_model: completion.request_model,
            provider_instance_name: completion.provider_instance_name,
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
            &e.user_id,
            &e.task_id,
            &e.method,
            &e.capability,
            &e.request_model,
            &e.provider_instance_name,
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
        if !ai_methods::is_ai_method(&e.method) {
            return Err(StorageError::InvalidRecord(
                "usage method is not canonical".into(),
            ));
        }
        let sql = self.sql("INSERT INTO aicc_usage_event
          (event_id,tenant_id,user_id,caller_app_id,task_id,idempotency_key,method,capability,
           request_model,provider_instance_name,provider_model,input_tokens,output_tokens,total_tokens,
           request_units,usage_json,finance_snapshot_json,created_at_ms)
          VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING");
        let result = sqlx::query(&sql)
            .bind(&e.event_id)
            .bind(&e.tenant_id)
            .bind(&e.user_id)
            .bind(&e.caller_app_id)
            .bind(&e.task_id)
            .bind(&e.idempotency_key)
            .bind(&e.method)
            .bind(&e.capability)
            .bind(&e.request_model)
            .bind(&e.provider_instance_name)
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

    async fn claim_execution(&self, initial: &ExecutionRecord) -> StorageResult<IdempotencyClaim> {
        validate_initial_execution(initial)?;
        let record_json = serde_json::to_string(initial)?;
        let sql = self.sql(
            "INSERT INTO aicc_execution_record
             (tenant_id,method,idempotency_key,task_id,body_fingerprint,state,record_json,
              created_at_ms,expires_at_ms) VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING",
        );
        let inserted = sqlx::query(&sql)
            .bind(&initial.scope.tenant_id)
            .bind(&initial.scope.method)
            .bind(&initial.scope.key)
            .bind(&initial.task_id)
            .bind(&initial.body_fingerprint)
            .bind(state_name(initial.state))
            .bind(record_json)
            .bind(to_i64(initial.created_at_ms)?)
            .bind(to_i64(initial.expires_at_ms)?)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1;
        if inserted {
            return Ok(IdempotencyClaim::Created(initial.clone()));
        }
        let Some(existing) = self
            .execution_by_scope(
                &initial.scope.tenant_id,
                &initial.scope.method,
                &initial.scope.key,
            )
            .await?
        else {
            return Err(StorageError::InvalidRecord(
                "execution task ID is already bound to another idempotency scope".into(),
            ));
        };
        Ok(if existing.body_fingerprint == initial.body_fingerprint {
            IdempotencyClaim::Existing(existing)
        } else {
            IdempotencyClaim::Conflict
        })
    }

    async fn execution_by_scope(
        &self,
        tenant_id: &str,
        method: &str,
        key: &str,
    ) -> StorageResult<Option<ExecutionRecord>> {
        let sql = self.sql(
            "SELECT state,record_json FROM aicc_execution_record
             WHERE tenant_id=? AND method=? AND idempotency_key=?",
        );
        sqlx::query(&sql)
            .bind(tenant_id)
            .bind(method)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?
            .map(execution_from_row)
            .transpose()
    }

    async fn execution_by_task(&self, task_id: &str) -> StorageResult<Option<ExecutionRecord>> {
        let sql = self.sql("SELECT state,record_json FROM aicc_execution_record WHERE task_id=?");
        sqlx::query(&sql)
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await?
            .map(execution_from_row)
            .transpose()
    }

    async fn mutate_execution(
        &self,
        task_id: &str,
        mutation: impl FnOnce(&mut ExecutionRecord),
    ) -> StorageResult<bool> {
        let Some(mut record) = self.execution_by_task(task_id).await? else {
            return Err(StorageError::InvalidRecord(
                "execution task does not exist".into(),
            ));
        };
        if execution_is_terminal(record.state) {
            return Ok(false);
        }
        mutation(&mut record);
        let sql = self.sql(
            "UPDATE aicc_execution_record SET state=?,record_json=? WHERE task_id=?
             AND state IN ('submitted','queued','running')",
        );
        Ok(sqlx::query(&sql)
            .bind(state_name(record.state))
            .bind(serde_json::to_string(&record)?)
            .bind(task_id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    async fn recoverable_executions(&self) -> StorageResult<Vec<ExecutionRecord>> {
        let rows = sqlx::query(
            "SELECT state,record_json FROM aicc_execution_record
             WHERE state IN ('submitted','queued','running') ORDER BY created_at_ms,task_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(execution_from_row).collect()
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

#[async_trait]
impl ExecutionStore for AiccStorage {
    async fn claim(&self, initial: ExecutionRecord) -> Result<IdempotencyClaim, AiccError> {
        self.claim_execution(&initial).await.map_err(storage_error)
    }

    async fn get_task(&self, task_id: &str) -> Result<Option<ExecutionRecord>, AiccError> {
        self.execution_by_task(task_id).await.map_err(storage_error)
    }

    async fn set_running(
        &self,
        task_id: &str,
        state: ExecutionState,
        binding: PinnedProviderTask,
    ) -> Result<bool, AiccError> {
        if execution_is_terminal(state) {
            return Err(AiccError::new(
                AiccErrorCode::InternalError,
                "set_running cannot persist a terminal execution state",
            ));
        }
        self.mutate_execution(task_id, |record| {
            record.state = state;
            record.binding = Some(binding);
        })
        .await
        .map_err(storage_error)
    }

    async fn try_complete(
        &self,
        task_id: &str,
        output: ExecutionOutput,
    ) -> Result<bool, AiccError> {
        self.mutate_execution(task_id, |record| {
            record.state = ExecutionState::Succeeded;
            record.output = Some(output);
        })
        .await
        .map_err(storage_error)
    }

    async fn try_fail(&self, task_id: &str, error: AiccError) -> Result<bool, AiccError> {
        self.mutate_execution(task_id, |record| {
            record.state = if error.code == AiccErrorCode::Cancelled {
                ExecutionState::Cancelled
            } else {
                ExecutionState::Failed
            };
            record.error = Some(error);
        })
        .await
        .map_err(storage_error)
    }

    async fn try_cancel(&self, task_id: &str) -> Result<bool, AiccError> {
        self.mutate_execution(task_id, |record| {
            record.state = ExecutionState::Cancelled;
            record.error = Some(AiccError::new(
                AiccErrorCode::Cancelled,
                "task was cancelled",
            ));
        })
        .await
        .map_err(storage_error)
    }

    async fn recoverable(&self) -> Result<Vec<ExecutionRecord>, AiccError> {
        self.recoverable_executions().await.map_err(storage_error)
    }
}

#[async_trait]
impl UsageCompletionPort for AiccStorage {
    async fn write_once(&self, completion: UsageCompletion) -> Result<(), AiccError> {
        self.write_provider_completion(ProviderCompletion {
            event_id: completion.event_id,
            tenant_id: completion.tenant_id,
            user_id: completion.user_id,
            caller_app_id: completion.caller_app_id,
            task_id: completion.task_id,
            idempotency_key: Some(completion.idempotency_key),
            method: completion.method,
            capability: completion.capability,
            request_model: completion.request_model,
            provider_instance_name: completion.provider_instance_name,
            provider_model: completion.provider_model,
            usage: Some(completion.usage),
            finance_snapshot: completion
                .finance_snapshot
                .map(serde_json::to_value)
                .transpose()
                .map_err(|_| {
                    AiccError::new(
                        AiccErrorCode::InternalError,
                        "usage finance snapshot could not be serialized",
                    )
                })?,
            created_at_ms: completion.completed_at_ms,
        })
        .await
        .map(|_| ())
        .map_err(storage_error)
    }
}

fn validate_initial_execution(record: &ExecutionRecord) -> StorageResult<()> {
    if record.usage_event_id.trim().is_empty()
        || record.user_id.trim().is_empty()
        || record.request_model.trim().is_empty()
        || record.body_fingerprint.trim().is_empty()
        || record.task_id.trim().is_empty()
        || record.event_ref.trim().is_empty()
        || record.scope.tenant_id.trim().is_empty()
        || record.scope.method.trim().is_empty()
        || record.scope.key.trim().is_empty()
        || record.expires_at_ms < record.created_at_ms
        || record.state != ExecutionState::Submitted
        || record.binding.is_some()
        || record.output.is_some()
        || record.error.is_some()
    {
        return Err(StorageError::InvalidRecord(
            "initial execution record is invalid".into(),
        ));
    }
    Ok(())
}

fn execution_from_row(row: AnyRow) -> StorageResult<ExecutionRecord> {
    let state: String = row.try_get("state")?;
    let record_json: String = row.try_get("record_json")?;
    let record: ExecutionRecord = serde_json::from_str(&record_json)?;
    if state != state_name(record.state) {
        return Err(StorageError::InvalidRecord(
            "execution state does not match its durable record".into(),
        ));
    }
    Ok(record)
}

fn state_name(state: ExecutionState) -> &'static str {
    match state {
        ExecutionState::Submitted => "submitted",
        ExecutionState::Queued => "queued",
        ExecutionState::Running => "running",
        ExecutionState::Succeeded => "succeeded",
        ExecutionState::Failed => "failed",
        ExecutionState::Cancelled => "cancelled",
    }
}

fn execution_is_terminal(state: ExecutionState) -> bool {
    matches!(
        state,
        ExecutionState::Succeeded | ExecutionState::Failed | ExecutionState::Cancelled
    )
}

fn storage_error(_: StorageError) -> AiccError {
    AiccError::new(
        AiccErrorCode::InternalError,
        "persistent AICC storage operation failed",
    )
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
        user_id: row.try_get("user_id")?,
        caller_app_id: row.try_get("caller_app_id")?,
        task_id: row.try_get("task_id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        method: row.try_get("method")?,
        capability: row.try_get("capability")?,
        request_model: row.try_get("request_model")?,
        provider_instance_name: row.try_get("provider_instance_name")?,
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
        && exact(&f.user_ids, Some(&e.user_id))
        && exact(&f.caller_app_ids, e.caller_app_id.as_deref())
        && fuzzy(f.caller_app_query.as_deref(), e.caller_app_id.as_deref())
        && exact(&f.request_models, Some(&e.request_model))
        && exact(&f.provider_models, Some(&e.provider_model))
        && fuzzy(f.provider_model_query.as_deref(), Some(&e.provider_model))
        && exact(&f.provider_instance_names, Some(&e.provider_instance_name))
        && fuzzy(
            f.provider_instance_query.as_deref(),
            Some(&e.provider_instance_name),
        )
        && exact(&f.capabilities, Some(&e.capability))
        && exact(&f.task_ids, Some(&e.task_id))
        && exact(&f.idempotency_keys, e.idempotency_key.as_deref())
        && exact(&f.methods, Some(&e.method))
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
            UsageQueryGroup::ProviderInstanceName => e.provider_instance_name.clone(),
            UsageQueryGroup::RequestModel => e.request_model.clone(),
            UsageQueryGroup::Method => e.method.clone(),
            UsageQueryGroup::Capability => e.capability.clone(),
            UsageQueryGroup::CallerAppId => e.caller_app_id.clone().unwrap_or_default(),
            UsageQueryGroup::UserId => e.user_id.clone(),
            UsageQueryGroup::TenantId => e.tenant_id.clone(),
        })
        .collect()
}

fn aggregate<'a>(events: impl IntoIterator<Item = &'a AiccUsageEvent>) -> UsageAggregate {
    let mut a = UsageAggregate::default();
    let mut finance = BTreeMap::<String, Option<f64>>::new();
    for e in events {
        a.total_requests += 1;
        a.input_tokens += e.input_tokens.unwrap_or(0);
        a.output_tokens += e.output_tokens.unwrap_or(0);
        a.total_tokens += e.total_tokens.unwrap_or(0);
        a.consumed_request_units += e.request_units.unwrap_or(1).max(1);
        match e.finance_snapshot_json.as_ref().and_then(valid_finance) {
            Some((amount, currency)) => {
                let total = finance.entry(currency).or_insert(Some(0.0));
                if let Some(current) = total {
                    let next = *current + amount;
                    if next.is_finite() {
                        *current = next;
                    } else {
                        *total = None;
                        a.finance_complete = false;
                    }
                }
            }
            None => a.finance_complete = false,
        }
    }
    a.finance_totals = finance
        .into_iter()
        .filter_map(|(currency, amount)| amount.map(|amount| Money::new(amount, currency)))
        .collect();
    a
}

fn valid_finance(value: &Value) -> Option<(f64, String)> {
    let amount = value.get("amount")?.as_f64()?;
    let currency = value.get("currency")?.as_str()?.trim();
    if !amount.is_finite() || amount < 0.0 || currency.is_empty() {
        return None;
    }
    Some((amount, currency.to_ascii_uppercase()))
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
    use crate::execution::{
        NativeTaskResumeDescriptor, PinnedPricingBasis, PinnedPricingSnapshot, ResumeCredential,
        ResumeCredentialKind,
    };
    use buckyos_api::{AiCost, ApiType, RouteTrace, UsageQueryBucket, UsageQueryFilters};
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
            user_id: "user-a".into(),
            caller_app_id: Some("app-a".into()),
            task_id: task.into(),
            idempotency_key: Some(idem.into()),
            method: ai_methods::CHAT_COMPLETIONS_CREATE.into(),
            capability: "llm".into(),
            request_model: "llm.chat".into(),
            provider_instance_name: "openai-primary".into(),
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

    fn usage_event(id: &str, finance: Option<Value>, request_units: Option<u64>) -> AiccUsageEvent {
        let usage = AiUsage {
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            request_units,
        };
        AiccUsageEvent {
            event_id: id.into(),
            tenant_id: "tenant-a".into(),
            user_id: "user-a".into(),
            caller_app_id: Some("app-a".into()),
            task_id: format!("task-{id}"),
            idempotency_key: Some(format!("idem-{id}")),
            method: ai_methods::CHAT_COMPLETIONS_CREATE.into(),
            capability: "llm".into(),
            request_model: "llm.chat".into(),
            provider_instance_name: "openai-primary".into(),
            provider_model: "gpt-5:reasoning@openai-primary".into(),
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            request_units,
            usage_json: usage,
            finance_snapshot_json: finance,
            created_at_ms: 10_000,
        }
    }

    fn execution_record(task_id: &str, key: &str) -> ExecutionRecord {
        ExecutionRecord {
            scope: crate::execution::IdempotencyScope::new(
                "tenant-a",
                ai_methods::CHAT_COMPLETIONS_CREATE,
                key,
            )
            .unwrap(),
            usage_event_id: format!("usage-{task_id}"),
            user_id: "user-a".into(),
            caller_app_id: Some("app-a".into()),
            request_model: "llm.chat".into(),
            body_fingerprint: format!("fingerprint-{task_id}"),
            task_id: task_id.into(),
            event_ref: format!("event-{task_id}"),
            state: ExecutionState::Submitted,
            binding: None,
            output: None,
            error: None,
            created_at_ms: 10_000,
            expires_at_ms: 100_000_000,
        }
    }

    fn provider_binding() -> PinnedProviderTask {
        let reference = "system-config://secrets/aicc/openai-primary/api-key".to_string();
        let fingerprint = sha256_hex(reference.as_bytes())[..16].to_string();
        PinnedProviderTask {
            runtime_generation: 7,
            exact_model: "gpt-5:reasoning@openai-primary".into(),
            provider_model_id: "gpt-5".into(),
            provider_instance_name: "openai-primary".into(),
            protocol_adapter_id: "openai-responses".into(),
            operation: "responses.create".into(),
            api_type: ApiType::Llm,
            remote_task_id: Some("remote-1".into()),
            cancel_supported: true,
            resume: Some(NativeTaskResumeDescriptor {
                base_url: "https://openai-primary.invalid/v1".into(),
                credential: Some(ResumeCredential {
                    reference,
                    kind: ResumeCredentialKind::NamedHeader,
                    header_name: Some("X-Api-Key".into()),
                    fingerprint,
                }),
                resolved_parameters: BTreeMap::from([
                    ("provider_model_id".into(), json!("gpt-5")),
                    ("status_path".into(), json!("/tasks/{task_id}")),
                    ("result_path".into(), json!("/tasks/{task_id}/result")),
                    ("cancel_path".into(), json!("/tasks/{task_id}/cancel")),
                ]),
                request_timeout_ms: 30_000,
                max_request_bytes: 1_048_576,
                max_response_bytes: 8_388_608,
            }),
            pricing: Some(PinnedPricingSnapshot {
                currency: "USD".into(),
                basis: PinnedPricingBasis::Tokens {
                    input_token: Some(0.000_001_25),
                    cache_input_token: Some(0.000_000_125),
                    output_token: Some(0.000_01),
                },
            }),
        }
    }

    fn execution_output() -> ExecutionOutput {
        ExecutionOutput {
            value: json!({"answer": 42}),
            usage: AiUsage::request_units(1),
            artifacts: vec![],
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
    async fn execution_store_persists_claim_binding_and_terminal_cas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aicc.db");
        let connection = format!("sqlite://{}?mode=rwc", path.display());
        let db = AiccStorage::open(&connection, RdbBackend::Sqlite)
            .await
            .unwrap();

        let running = execution_record("task-running", "key-running");
        assert!(matches!(
            db.claim(running.clone()).await.unwrap(),
            IdempotencyClaim::Created(_)
        ));
        assert!(matches!(
            db.claim(running.clone()).await.unwrap(),
            IdempotencyClaim::Existing(existing) if existing == running
        ));
        let mut conflicting = running.clone();
        conflicting.body_fingerprint = "different".into();
        assert!(matches!(
            db.claim(conflicting).await.unwrap(),
            IdempotencyClaim::Conflict
        ));
        assert!(db
            .set_running(
                &running.task_id,
                ExecutionState::Running,
                provider_binding(),
            )
            .await
            .unwrap());
        drop(db);

        let db = AiccStorage::open(&connection, RdbBackend::Sqlite)
            .await
            .unwrap();
        let restored = db.get_task(&running.task_id).await.unwrap().unwrap();
        assert_eq!(restored.state, ExecutionState::Running);
        assert_eq!(restored.binding, Some(provider_binding()));
        let restored_binding = restored.binding.as_ref().unwrap();
        assert_eq!(
            restored_binding.resume.as_ref(),
            provider_binding().resume.as_ref()
        );
        assert_eq!(
            restored_binding.pricing.as_ref(),
            provider_binding().pricing.as_ref()
        );
        let encoded_binding = serde_json::to_string(restored_binding).unwrap();
        assert!(encoded_binding.contains("system-config://secrets/aicc/openai-primary/api-key"));
        assert!(!encoded_binding.contains("plaintext-secret"));
        assert_eq!(db.recoverable().await.unwrap(), vec![restored]);
        assert!(db.try_cancel(&running.task_id).await.unwrap());
        assert!(!db
            .try_complete(&running.task_id, execution_output())
            .await
            .unwrap());

        let completed = execution_record("task-complete", "key-complete");
        db.claim(completed.clone()).await.unwrap();
        assert!(db
            .try_complete(&completed.task_id, execution_output())
            .await
            .unwrap());
        assert_eq!(
            db.get_task(&completed.task_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            ExecutionState::Succeeded
        );

        let failed = execution_record("task-failed", "key-failed");
        db.claim(failed.clone()).await.unwrap();
        assert!(db
            .try_fail(
                &failed.task_id,
                AiccError::new(AiccErrorCode::ProviderError, "provider failed"),
            )
            .await
            .unwrap());
        assert_eq!(
            db.get_task(&failed.task_id).await.unwrap().unwrap().state,
            ExecutionState::Failed
        );

        let raced = execution_record("task-raced", "key-raced");
        db.claim(raced.clone()).await.unwrap();
        let (complete, cancel) = tokio::join!(
            db.try_complete(&raced.task_id, execution_output()),
            db.try_cancel(&raced.task_id)
        );
        assert_ne!(complete.unwrap(), cancel.unwrap());
        assert!(db.recoverable().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn usage_completion_writer_is_durable_and_idempotent() {
        let db = db().await;
        let completion = UsageCompletion {
            event_id: "usage-production".into(),
            tenant_id: "tenant-a".into(),
            user_id: "user-a".into(),
            caller_app_id: Some("app-a".into()),
            task_id: "task-production".into(),
            idempotency_key: "idem-production".into(),
            method: ai_methods::CHAT_COMPLETIONS_CREATE.into(),
            capability: "llm".into(),
            request_model: "llm.chat".into(),
            provider_instance_name: "openai-primary".into(),
            provider_model: "gpt-5:reasoning@openai-primary".into(),
            usage: AiUsage {
                input_tokens: Some(10),
                output_tokens: Some(5),
                total_tokens: Some(15),
                request_units: None,
            },
            finance_snapshot: Some(AiCost {
                amount: 0.25,
                currency: "EUR".into(),
            }),
            completed_at_ms: 10_000,
        };
        db.write_once(completion.clone()).await.unwrap();
        db.write_once(completion).await.unwrap();

        let result = db
            .query_usage(
                &QueryUsageRequest::new(UsageQueryTimeRange::Explicit {
                    start_time_ms: 10_000,
                    end_time_ms: 10_001,
                }),
                10_001,
            )
            .await
            .unwrap();
        assert_eq!(result.total.total_requests, 1);
        assert_eq!(result.total.consumed_request_units, 1);
        assert_eq!(result.total.finance_totals, vec![Money::new(0.25, "EUR")]);
        assert!(result.total.finance_complete);
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
            group_by: vec![
                UsageQueryGroup::ProviderInstanceName,
                UsageQueryGroup::Method,
                UsageQueryGroup::UserId,
            ],
            time_bucket: Some(UsageQueryBucket::Hour),
            output_mode: UsageQueryOutputMode::SummaryAndEvents,
            limit: Some(1),
            cursor: None,
        };
        let first = db.query_usage(&request, 20_000).await.unwrap();
        assert_eq!(first.total.total_requests, 2);
        assert_eq!(first.total.total_tokens, 30);
        assert_eq!(first.total.consumed_request_units, 2);
        assert_eq!(first.total.finance_totals, vec![Money::new(0.5, "USD")]);
        assert!(first.total.finance_complete);
        assert_eq!(first.grouped.len(), 1);
        assert_eq!(first.grouped[0].group["user_id"], "user-a");
        assert_eq!(
            first.grouped[0].group["method"],
            ai_methods::CHAT_COMPLETIONS_CREATE
        );
        assert_eq!(
            first.grouped[0].group["provider_instance_name"],
            "openai-primary"
        );
        assert_eq!(first.grouped[0].aggregate, first.total);
        assert_eq!(first.buckets.len(), 1);
        assert_eq!(first.buckets[0].aggregate, first.total);
        assert_eq!(first.events.len(), 1);
        assert!(first.next_cursor.is_some());
        let mut next = request;
        next.cursor = first.next_cursor;
        assert_eq!(db.query_usage(&next, 20_000).await.unwrap().events.len(), 1);
    }

    #[test]
    fn finance_aggregation_is_currency_explicit_and_fail_closed() {
        let empty = aggregate(std::iter::empty());
        assert_eq!(empty, UsageAggregate::default());

        let complete = [
            usage_event(
                "one",
                Some(json!({"amount": 0.25, "currency": " usd "})),
                None,
            ),
            usage_event(
                "two",
                Some(json!({"amount": 0.25, "currency": "USD"})),
                Some(0),
            ),
            usage_event(
                "three",
                Some(json!({"amount": 0.25, "currency": "usd"})),
                Some(3),
            ),
        ];
        let complete = aggregate(&complete);
        assert_eq!(complete.consumed_request_units, 5);
        assert_eq!(complete.finance_totals, vec![Money::new(0.75, "USD")]);
        assert!(complete.finance_complete);

        let partial = [
            usage_event(
                "priced",
                Some(json!({"amount": 0.25, "currency": "USD"})),
                None,
            ),
            usage_event("missing", None, None),
        ];
        let partial = aggregate(&partial);
        assert_eq!(partial.finance_totals, vec![Money::new(0.25, "USD")]);
        assert!(!partial.finance_complete);

        for (id, finance) in [
            ("negative", json!({"amount": -0.1, "currency": "USD"})),
            ("not-number", json!({"amount": "0.1", "currency": "USD"})),
            ("no-amount", json!({"currency": "USD"})),
            ("no-currency", json!({"amount": 0.1})),
            ("empty-currency", json!({"amount": 0.1, "currency": " "})),
        ] {
            let event = usage_event(id, Some(finance), None);
            let invalid = aggregate([&event]);
            assert!(invalid.finance_totals.is_empty(), "{id}");
            assert!(!invalid.finance_complete, "{id}");
        }

        let mixed = [
            usage_event(
                "dollars",
                Some(json!({"amount": 0.25, "currency": "USD"})),
                None,
            ),
            usage_event(
                "euros",
                Some(json!({"amount": 0.25, "currency": "EUR"})),
                None,
            ),
        ];
        let mixed = aggregate(&mixed);
        assert_eq!(
            mixed.finance_totals,
            vec![Money::new(0.25, "EUR"), Money::new(0.25, "USD")]
        );
        assert!(mixed.finance_complete);

        let overflow = [
            usage_event(
                "huge-one",
                Some(json!({"amount": 1.0e308, "currency": "JPY"})),
                None,
            ),
            usage_event(
                "huge-two",
                Some(json!({"amount": 1.0e308, "currency": "JPY"})),
                None,
            ),
            usage_event(
                "valid-dollars",
                Some(json!({"amount": 0.5, "currency": "USD"})),
                None,
            ),
        ];
        let overflow = aggregate(&overflow);
        assert_eq!(overflow.finance_totals, vec![Money::new(0.5, "USD")]);
        assert!(!overflow.finance_complete);
    }

    #[tokio::test]
    async fn usage_identity_filters_use_half_open_time_range() {
        let db = db().await;
        db.write_provider_completion(completion("start", "task-start", "idem-start", 10_000))
            .await
            .unwrap();
        db.write_provider_completion(completion("end", "task-end", "idem-end", 11_000))
            .await
            .unwrap();
        let request = QueryUsageRequest {
            time_range: UsageQueryTimeRange::Explicit {
                start_time_ms: 10_000,
                end_time_ms: 11_000,
            },
            filters: UsageQueryFilters {
                user_ids: vec!["user-a".into()],
                methods: vec![ai_methods::CHAT_COMPLETIONS_CREATE.into()],
                provider_instance_names: vec!["openai-primary".into()],
                ..UsageQueryFilters::default()
            },
            group_by: vec![],
            time_bucket: None,
            output_mode: UsageQueryOutputMode::Events,
            limit: None,
            cursor: None,
        };
        let response = db.query_usage(&request, 12_000).await.unwrap();
        assert_eq!(response.total.total_requests, 1);
        assert_eq!(response.events[0].event_id, "start");

        let mut invalid = completion("bad", "task-bad", "idem-bad", 12_000);
        invalid.method = "provider.list".into();
        assert!(matches!(
            db.write_provider_completion(invalid).await,
            Err(StorageError::InvalidRecord(_))
        ));
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
