//! Store for the Task Dispatch Center.
//!
//! Independent rdb instance (`task-dispatcher-main`): no table, schema
//! version or SQL join is shared with the task store. All timestamps are
//! unix milliseconds.

use buckyos_api::{
    get_rdb_instance, DispatchApproval, DispatchAuthEnvelope, DispatchRecord,
    DispatchRejectReason, DispatchStatus, OperationRoute, RdbBackend, TargetInstance,
    TargetRegistration, TargetSelection, TASK_DISPATCHER_RDB_INSTANCE_ID,
    TASK_DISPATCHER_RDB_SCHEMA_POSTGRES, TASK_DISPATCHER_RDB_SCHEMA_SQLITE,
    TASK_MANAGER_SERVICE_NAME,
};
use log::*;
use serde_json::Value;
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};
use std::sync::Once;

use crate::task_db::{rewrite_placeholders_to_dollar, split_sql_statements};

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

pub type DbResult<T> = Result<T, sqlx::Error>;

/// Handle to the dispatcher rdb. Same pooling model as `TaskDb`: the
/// `AnyPool` is internally Arc-backed, so `DispatchDb` is shared via
/// `Arc<DispatchDb>` without an outer lock.
pub struct DispatchDb {
    pool: AnyPool,
    backend: RdbBackend,
}

/// A `dispatch_target` row: the registration JSON is the source of truth,
/// the owner/enabled columns exist for queries and are kept in sync.
#[derive(Debug, Clone)]
pub struct TargetRow {
    pub registration: TargetRegistration,
    pub owner_user_id: String,
    pub owner_app_id: String,
    pub enabled: bool,
}

#[derive(Debug, Default, Clone)]
pub struct RecordFilter {
    pub target_id: Option<String>,
    pub operation: Option<String>,
    pub status: Option<DispatchStatus>,
    pub requested_by_user: Option<String>,
    pub requested_by_app: Option<String>,
    pub on_behalf_of: Option<String>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub limit: u32,
}

impl DispatchDb {
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> Result<Self, String> {
        ensure_any_drivers_installed();
        let mut opts = AnyPoolOptions::new().max_connections(8);
        if backend == RdbBackend::Sqlite {
            opts = opts.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON;").await?;
                    Ok(())
                })
            });
        }
        let pool = opts
            .connect(connection)
            .await
            .map_err(|err| format!("open task-dispatcher db at {}: {}", connection, err))?;
        let db = DispatchDb { pool, backend };
        db.apply_schema(schema)
            .await
            .map_err(|err| format!("apply task-dispatcher schema: {}", err))?;
        Ok(db)
    }

    /// Production entry point. The dispatcher instance rides inside the
    /// task-manager service spec (same deployment unit, independent store).
    pub async fn open_from_service_spec() -> Result<Self, String> {
        let instance = get_rdb_instance(
            TASK_MANAGER_SERVICE_NAME,
            None,
            TASK_DISPATCHER_RDB_INSTANCE_ID,
        )
        .await
        .map_err(|err| format!("resolve task-dispatcher rdb instance failed: {}", err))?;
        info!("dispatch_db.open {}", instance.connection);
        Self::open(
            &instance.connection,
            instance.backend,
            instance.schema.as_deref(),
        )
        .await
    }

    async fn apply_schema(&self, override_ddl: Option<&str>) -> DbResult<()> {
        let ddl: &str = override_ddl
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(match self.backend {
                RdbBackend::Sqlite => TASK_DISPATCHER_RDB_SCHEMA_SQLITE,
                RdbBackend::Postgres => TASK_DISPATCHER_RDB_SCHEMA_POSTGRES,
            });
        for statement in split_sql_statements(ddl) {
            self.pool.execute(statement.as_str()).await?;
        }
        self.migrate_schema().await?;
        Ok(())
    }

    /// In-place migrations for stores created by an older schema (the DDL
    /// above only creates missing tables). Runs unconditionally on open —
    /// also when a stale DDL override still describes the old shape.
    async fn migrate_schema(&self) -> DbResult<()> {
        // v1 -> v2 (M4 approval gate): dispatch_record.approval column.
        self.add_record_column_if_missing("approval", "TEXT").await
    }

    async fn add_record_column_if_missing(&self, column: &str, sql_type: &str) -> DbResult<()> {
        match self.backend {
            RdbBackend::Sqlite => {
                let exists = sqlx::query(
                    "SELECT 1 FROM pragma_table_info('dispatch_record') WHERE name = ?",
                )
                .bind(column.to_string())
                .fetch_optional(self.pool())
                .await?
                .is_some();
                if !exists {
                    let sql = format!("ALTER TABLE dispatch_record ADD COLUMN {} {}", column, sql_type);
                    self.pool.execute(sql.as_str()).await?;
                }
            }
            RdbBackend::Postgres => {
                let sql = format!(
                    "ALTER TABLE dispatch_record ADD COLUMN IF NOT EXISTS {} {}",
                    column, sql_type
                );
                self.pool.execute(sql.as_str()).await?;
            }
        }
        Ok(())
    }

    fn pool(&self) -> &AnyPool {
        &self.pool
    }

    fn render_sql(&self, sql: &str) -> String {
        match self.backend {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    // ------------------------------------------------------------------
    // targets
    // ------------------------------------------------------------------

    pub async fn upsert_target(
        &self,
        registration: &TargetRegistration,
        now_ms: i64,
    ) -> DbResult<()> {
        let registration_json =
            serde_json::to_string(registration).unwrap_or_else(|_| "{}".to_string());
        let sql = self.render_sql(
            "INSERT INTO dispatch_target (
                target_id, owner_user_id, owner_app_id, registration, enabled,
                lease_epoch_seq, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, 0, ?, ?)
            ON CONFLICT(target_id) DO UPDATE SET
                owner_user_id = excluded.owner_user_id,
                owner_app_id = excluded.owner_app_id,
                registration = excluded.registration,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
        );
        sqlx::query(&sql)
            .bind(registration.target_id.clone())
            .bind(registration.owner_user_id.clone())
            .bind(registration.owner_app_id.clone())
            .bind(registration_json)
            .bind(if registration.enabled { 1_i64 } else { 0_i64 })
            .bind(now_ms)
            .bind(now_ms)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn get_target(&self, target_id: &str) -> DbResult<Option<TargetRow>> {
        let sql = self.render_sql("SELECT * FROM dispatch_target WHERE target_id = ?");
        let row = sqlx::query(&sql)
            .bind(target_id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(target_from_row).transpose()
    }

    pub async fn list_targets(&self) -> DbResult<Vec<TargetRow>> {
        let sql = self.render_sql("SELECT * FROM dispatch_target ORDER BY target_id ASC");
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        rows.into_iter().map(target_from_row).collect()
    }

    pub async fn set_target_enabled(
        &self,
        target_id: &str,
        enabled: bool,
        now_ms: i64,
    ) -> DbResult<bool> {
        // The registration JSON is the source of truth — patch it in rust to
        // avoid backend-specific json functions.
        let Some(mut row) = self.get_target(target_id).await? else {
            return Ok(false);
        };
        row.registration.enabled = enabled;
        let registration_json =
            serde_json::to_string(&row.registration).unwrap_or_else(|_| "{}".to_string());
        let sql = self.render_sql(
            "UPDATE dispatch_target SET enabled = ?, registration = ?, updated_at = ? WHERE target_id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(if enabled { 1_i64 } else { 0_i64 })
            .bind(registration_json)
            .bind(now_ms)
            .bind(target_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Per-target strictly-increasing lease epoch (survives instance rows
    /// being deleted on detach/expiry).
    pub async fn next_lease_epoch(&self, target_id: &str) -> DbResult<Option<i64>> {
        let sql = self.render_sql(
            "UPDATE dispatch_target SET lease_epoch_seq = lease_epoch_seq + 1
             WHERE target_id = ? RETURNING lease_epoch_seq",
        );
        let row = sqlx::query(&sql)
            .bind(target_id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(|row| row.try_get::<i64, _>("lease_epoch_seq"))
            .transpose()
    }

    // ------------------------------------------------------------------
    // operation routes
    // ------------------------------------------------------------------

    pub async fn upsert_route(
        &self,
        operation: &str,
        default_target_id: &str,
        now_ms: i64,
    ) -> DbResult<OperationRoute> {
        let sql = self.render_sql(
            "INSERT INTO dispatch_operation_route (
                operation, default_target_id, revision, enabled, created_at, updated_at
            ) VALUES (?, ?, 1, 1, ?, ?)
            ON CONFLICT(operation) DO UPDATE SET
                default_target_id = excluded.default_target_id,
                revision = dispatch_operation_route.revision + 1,
                enabled = 1,
                updated_at = excluded.updated_at
            RETURNING operation, default_target_id, revision, enabled",
        );
        let row = sqlx::query(&sql)
            .bind(operation.to_string())
            .bind(default_target_id.to_string())
            .bind(now_ms)
            .bind(now_ms)
            .fetch_one(self.pool())
            .await?;
        route_from_row(row)
    }

    pub async fn get_route(&self, operation: &str) -> DbResult<Option<OperationRoute>> {
        let sql = self.render_sql(
            "SELECT operation, default_target_id, revision, enabled
             FROM dispatch_operation_route WHERE operation = ?",
        );
        let row = sqlx::query(&sql)
            .bind(operation.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(route_from_row).transpose()
    }

    pub async fn list_routes(&self) -> DbResult<Vec<OperationRoute>> {
        let sql = self.render_sql(
            "SELECT operation, default_target_id, revision, enabled
             FROM dispatch_operation_route ORDER BY operation ASC",
        );
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        rows.into_iter().map(route_from_row).collect()
    }

    pub async fn set_route_enabled(
        &self,
        operation: &str,
        enabled: bool,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_operation_route
             SET enabled = ?, revision = revision + 1, updated_at = ?
             WHERE operation = ?",
        );
        let result = sqlx::query(&sql)
            .bind(if enabled { 1_i64 } else { 0_i64 })
            .bind(now_ms)
            .bind(operation.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // ------------------------------------------------------------------
    // instances
    // ------------------------------------------------------------------

    pub async fn insert_instance(&self, instance: &TargetInstance, now_ms: i64) -> DbResult<()> {
        let sql = self.render_sql(
            "INSERT INTO dispatch_instance (
                target_id, instance_id, lease_epoch, lease_expires_at,
                capacity, available_capacity, attached_at, renewed_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(instance.target_id.clone())
            .bind(instance.instance_id.clone())
            .bind(instance.lease_epoch as i64)
            .bind(instance.lease_expires_at as i64)
            .bind(instance.capacity as i64)
            .bind(instance.available_capacity as i64)
            .bind(now_ms)
            .bind(now_ms)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn get_instance(
        &self,
        target_id: &str,
        instance_id: &str,
    ) -> DbResult<Option<TargetInstance>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_instance WHERE target_id = ? AND instance_id = ?",
        );
        let row = sqlx::query(&sql)
            .bind(target_id.to_string())
            .bind(instance_id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(instance_from_row).transpose()
    }

    pub async fn renew_instance(
        &self,
        target_id: &str,
        instance_id: &str,
        lease_epoch: u64,
        lease_expires_at: i64,
        available_capacity: u32,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_instance
             SET lease_expires_at = ?, available_capacity = ?, renewed_at = ?
             WHERE target_id = ? AND instance_id = ? AND lease_epoch = ?",
        );
        let result = sqlx::query(&sql)
            .bind(lease_expires_at)
            .bind(available_capacity as i64)
            .bind(now_ms)
            .bind(target_id.to_string())
            .bind(instance_id.to_string())
            .bind(lease_epoch as i64)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_instance(&self, target_id: &str, instance_id: &str) -> DbResult<bool> {
        let sql = self
            .render_sql("DELETE FROM dispatch_instance WHERE target_id = ? AND instance_id = ?");
        let result = sqlx::query(&sql)
            .bind(target_id.to_string())
            .bind(instance_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_active_instances(
        &self,
        target_id: &str,
        now_ms: i64,
    ) -> DbResult<Vec<TargetInstance>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_instance
             WHERE target_id = ? AND lease_expires_at > ?
             ORDER BY instance_id ASC",
        );
        let rows = sqlx::query(&sql)
            .bind(target_id.to_string())
            .bind(now_ms)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(instance_from_row).collect()
    }

    /// Drop instances whose lease has expired; returns the affected
    /// `(target_id, instance_id)` pairs so their targets can be re-evaluated.
    pub async fn delete_expired_instances(&self, now_ms: i64) -> DbResult<Vec<(String, String)>> {
        let select_sql = self.render_sql(
            "SELECT target_id, instance_id FROM dispatch_instance WHERE lease_expires_at <= ?",
        );
        let rows = sqlx::query(&select_sql)
            .bind(now_ms)
            .fetch_all(self.pool())
            .await?;
        let mut expired = Vec::with_capacity(rows.len());
        for row in rows {
            let target_id: String = row.try_get("target_id")?;
            let instance_id: String = row.try_get("instance_id")?;
            expired.push((target_id, instance_id));
        }
        if !expired.is_empty() {
            let delete_sql =
                self.render_sql("DELETE FROM dispatch_instance WHERE lease_expires_at <= ?");
            sqlx::query(&delete_sql)
                .bind(now_ms)
                .execute(self.pool())
                .await?;
        }
        Ok(expired)
    }

    // ------------------------------------------------------------------
    // records
    // ------------------------------------------------------------------

    /// `idempotency_key` is deliberately not part of the wire-level
    /// `DispatchRecord` (doc §3.4); it lives in the store for the unique
    /// `(requested_by_user, requested_by_app, idempotency_key)` index.
    pub async fn insert_record(
        &self,
        record: &DispatchRecord,
        idempotency_key: &str,
    ) -> DbResult<()> {
        let sql = self.render_sql(
            "INSERT INTO dispatch_record (
                dispatch_id, requested_target_id, target_id, target_selection,
                operation, status, input, auth, idempotency_key,
                requested_by_user, requested_by_app, on_behalf_of,
                offer_instance_id, offer_lease_expires_at, offer_delivery_count,
                target_task_id, reject_reason, approval, message, expires_at,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(record.dispatch_id.clone())
            .bind(record.requested_target_id.clone())
            .bind(record.target_id.clone())
            .bind(serde_json::to_string(&record.target_selection).unwrap_or_default())
            .bind(record.operation.clone())
            .bind(record.status.to_string())
            .bind(serde_json::to_string(&record.input).unwrap_or_else(|_| "null".to_string()))
            .bind(serde_json::to_string(&record.auth).unwrap_or_else(|_| "{}".to_string()))
            .bind(idempotency_key.to_string())
            .bind(record.auth.requested_by_user.clone())
            .bind(record.auth.requested_by_app.clone())
            .bind(record.auth.on_behalf_of.clone())
            .bind(record.offer_instance_id.clone())
            .bind(record.offer_lease_expires_at.map(|v| v as i64))
            .bind(record.offer_delivery_count as i64)
            .bind(record.target_task_id)
            .bind(record.reject_reason.map(|r| r.to_string()))
            .bind(
                record
                    .approval
                    .as_ref()
                    .and_then(|approval| serde_json::to_string(approval).ok()),
            )
            .bind(record.message.clone())
            .bind(record.auth.expires_at.map(|v| v as i64))
            .bind(record.created_at as i64)
            .bind(record.updated_at as i64)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn get_record(&self, dispatch_id: &str) -> DbResult<Option<DispatchRecord>> {
        let sql = self.render_sql("SELECT * FROM dispatch_record WHERE dispatch_id = ?");
        let row = sqlx::query(&sql)
            .bind(dispatch_id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(record_from_row).transpose()
    }

    pub async fn find_record_by_idempotency_key(
        &self,
        requested_by_user: &str,
        requested_by_app: &str,
        idempotency_key: &str,
    ) -> DbResult<Option<DispatchRecord>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_record
             WHERE requested_by_user = ? AND requested_by_app = ? AND idempotency_key = ?",
        );
        let row = sqlx::query(&sql)
            .bind(requested_by_user.to_string())
            .bind(requested_by_app.to_string())
            .bind(idempotency_key.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(record_from_row).transpose()
    }

    pub async fn list_records(&self, filter: &RecordFilter) -> DbResult<Vec<DispatchRecord>> {
        let mut sql = String::from("SELECT * FROM dispatch_record");
        let mut conditions: Vec<String> = Vec::new();
        enum Param {
            Text(String),
            Int(i64),
        }
        let mut params: Vec<Param> = Vec::new();

        if let Some(target_id) = filter.target_id.as_deref() {
            conditions.push("target_id = ?".to_string());
            params.push(Param::Text(target_id.to_string()));
        }
        if let Some(operation) = filter.operation.as_deref() {
            conditions.push("operation = ?".to_string());
            params.push(Param::Text(operation.to_string()));
        }
        if let Some(status) = filter.status {
            conditions.push("status = ?".to_string());
            params.push(Param::Text(status.to_string()));
        }
        if let Some(user) = filter.requested_by_user.as_deref() {
            conditions.push("requested_by_user = ?".to_string());
            params.push(Param::Text(user.to_string()));
        }
        if let Some(app) = filter.requested_by_app.as_deref() {
            conditions.push("requested_by_app = ?".to_string());
            params.push(Param::Text(app.to_string()));
        }
        if let Some(on_behalf_of) = filter.on_behalf_of.as_deref() {
            conditions.push("on_behalf_of = ?".to_string());
            params.push(Param::Text(on_behalf_of.to_string()));
        }
        if let Some(since) = filter.since_ms {
            conditions.push("created_at >= ?".to_string());
            params.push(Param::Int(since));
        }
        if let Some(until) = filter.until_ms {
            conditions.push("created_at < ?".to_string());
            params.push(Param::Int(until));
        }
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC, dispatch_id DESC LIMIT ");
        sql.push_str(&filter.limit.max(1).to_string());

        let sql = self.render_sql(&sql);
        let mut query = sqlx::query(&sql);
        for param in params {
            query = match param {
                Param::Text(v) => query.bind(v),
                Param::Int(v) => query.bind(v),
            };
        }
        let rows = query.fetch_all(self.pool()).await?;
        rows.into_iter().map(record_from_row).collect()
    }

    /// Records eligible for assignment, oldest first.
    pub async fn list_assignable(
        &self,
        target_id: &str,
        limit: u32,
    ) -> DbResult<Vec<DispatchRecord>> {
        let sql = self.render_sql(&format!(
            "SELECT * FROM dispatch_record
             WHERE target_id = ? AND status IN ('Queued', 'WaitingForTarget')
             ORDER BY created_at ASC, dispatch_id ASC LIMIT {}",
            limit.max(1)
        ));
        let rows = sqlx::query(&sql)
            .bind(target_id.to_string())
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(record_from_row).collect()
    }

    pub async fn count_offered(&self, target_id: &str) -> DbResult<u32> {
        let sql = self.render_sql(
            "SELECT COUNT(*) AS cnt FROM dispatch_record WHERE target_id = ? AND status = 'Offered'",
        );
        let row = sqlx::query(&sql)
            .bind(target_id.to_string())
            .fetch_one(self.pool())
            .await?;
        let count: i64 = row.try_get("cnt")?;
        Ok(count.max(0) as u32)
    }

    pub async fn count_offered_for_instance(
        &self,
        target_id: &str,
        instance_id: &str,
    ) -> DbResult<u32> {
        let sql = self.render_sql(
            "SELECT COUNT(*) AS cnt FROM dispatch_record
             WHERE target_id = ? AND status = 'Offered' AND offer_instance_id = ?",
        );
        let row = sqlx::query(&sql)
            .bind(target_id.to_string())
            .bind(instance_id.to_string())
            .fetch_one(self.pool())
            .await?;
        let count: i64 = row.try_get("cnt")?;
        Ok(count.max(0) as u32)
    }

    /// Conditional transition into `Offered`. Returns false when the record
    /// left the assignable states concurrently.
    pub async fn mark_offered(
        &self,
        dispatch_id: &str,
        instance_id: &str,
        offer_lease_expires_at: i64,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Offered', offer_instance_id = ?, offer_lease_expires_at = ?,
                 offer_delivery_count = offer_delivery_count + 1, updated_at = ?
             WHERE dispatch_id = ? AND status IN ('Queued', 'WaitingForTarget')",
        );
        let result = sqlx::query(&sql)
            .bind(instance_id.to_string())
            .bind(offer_lease_expires_at)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Queued -> WaitingForTarget for everything not assigned this round.
    pub async fn mark_queued_as_waiting(&self, target_id: &str, now_ms: i64) -> DbResult<u64> {
        let sql = self.render_sql(
            "UPDATE dispatch_record SET status = 'WaitingForTarget', updated_at = ?
             WHERE target_id = ? AND status = 'Queued'",
        );
        let result = sqlx::query(&sql)
            .bind(now_ms)
            .bind(target_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }

    /// Accept transition. Allowed from Offered (normal), Queued /
    /// WaitingForTarget (late accept after lease-expiry requeue) and
    /// Uncertain (the target itself is the authoritative business resolver).
    pub async fn mark_accepted(
        &self,
        dispatch_id: &str,
        target_task_id: i64,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Accepted', target_task_id = ?,
                 offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ?
               AND status IN ('Offered', 'Queued', 'WaitingForTarget', 'Uncertain')",
        );
        let result = sqlx::query(&sql)
            .bind(target_task_id)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_rejected(
        &self,
        dispatch_id: &str,
        reason: DispatchRejectReason,
        detail: Option<String>,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Rejected', reject_reason = ?, message = ?,
                 offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ? AND status IN ('Offered', 'Queued', 'WaitingForTarget')",
        );
        let result = sqlx::query(&sql)
            .bind(reason.to_string())
            .bind(detail)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// `PendingApproval` is cancelable too: the submitter may withdraw a
    /// record that is still waiting for a manual release.
    pub async fn mark_canceled(&self, dispatch_id: &str, now_ms: i64) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Canceled', offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ?
               AND status IN ('PendingApproval', 'Queued', 'WaitingForTarget', 'Offered')",
        );
        let result = sqlx::query(&sql)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// The request deadline also covers the approval wait: a record nobody
    /// released in time expires like any other pre-accept record.
    pub async fn mark_expired(
        &self,
        dispatch_id: &str,
        detail: &str,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Expired', message = ?, offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ?
               AND status IN ('PendingApproval', 'Queued', 'WaitingForTarget', 'Offered')",
        );
        let result = sqlx::query(&sql)
            .bind(detail.to_string())
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Offer lease expired, IdempotentAccept contract: back to Queued for
    /// re-evaluation (same dispatch_id, same envelope — handoff recovery,
    /// not a business retry).
    pub async fn requeue_expired_offer(&self, dispatch_id: &str, now_ms: i64) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Queued', offer_instance_id = NULL,
                 offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ? AND status = 'Offered' AND offer_lease_expires_at <= ?",
        );
        let result = sqlx::query(&sql)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .bind(now_ms)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Offer lease expired, no idempotency contract: manual/business
    /// resolution required. Conditional on the lease actually being expired
    /// so a concurrent fresh re-offer is never clobbered.
    pub async fn mark_uncertain_expired(&self, dispatch_id: &str, now_ms: i64) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Uncertain', offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ? AND status = 'Offered' AND offer_lease_expires_at <= ?",
        );
        let result = sqlx::query(&sql)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .bind(now_ms)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Detach-time recovery for a None-contract target: offers parked on the
    /// detached instance become Uncertain regardless of lease age.
    pub async fn mark_uncertain_for_instance(
        &self,
        dispatch_id: &str,
        instance_id: &str,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Uncertain', offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ? AND status = 'Offered' AND offer_instance_id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .bind(instance_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Detach-time recovery for an IdempotentAccept target: recycle the
    /// instance's offers immediately (no need to wait for the lease).
    pub async fn requeue_offer_for_instance(
        &self,
        dispatch_id: &str,
        instance_id: &str,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Queued', offer_instance_id = NULL,
                 offer_lease_expires_at = NULL, updated_at = ?
             WHERE dispatch_id = ? AND status = 'Offered' AND offer_instance_id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .bind(instance_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// In-flight offers currently parked on one instance (any lease age).
    pub async fn list_offered_for_instance(
        &self,
        target_id: &str,
        instance_id: &str,
    ) -> DbResult<Vec<DispatchRecord>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_record
             WHERE target_id = ? AND status = 'Offered' AND offer_instance_id = ?
             ORDER BY created_at ASC, dispatch_id ASC",
        );
        let rows = sqlx::query(&sql)
            .bind(target_id.to_string())
            .bind(instance_id.to_string())
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(record_from_row).collect()
    }

    /// Manual release: PendingApproval -> Queued, recording the decision.
    /// Conditional on the record still being held so approve can never race
    /// past a concurrent cancel/expiry — and never touches the envelope.
    pub async fn approve_pending(
        &self,
        dispatch_id: &str,
        approval: &DispatchApproval,
        now_ms: i64,
    ) -> DbResult<bool> {
        let approval_json = serde_json::to_string(approval).unwrap_or_else(|_| "{}".to_string());
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Queued', approval = ?, updated_at = ?
             WHERE dispatch_id = ? AND status = 'PendingApproval'",
        );
        let result = sqlx::query(&sql)
            .bind(approval_json)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Manual refusal: PendingApproval -> terminal Rejected(approval_denied).
    pub async fn deny_pending(
        &self,
        dispatch_id: &str,
        approval: &DispatchApproval,
        now_ms: i64,
    ) -> DbResult<bool> {
        let approval_json = serde_json::to_string(approval).unwrap_or_else(|_| "{}".to_string());
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = 'Rejected', reject_reason = ?, approval = ?, message = ?, updated_at = ?
             WHERE dispatch_id = ? AND status = 'PendingApproval'",
        );
        let result = sqlx::query(&sql)
            .bind(DispatchRejectReason::ApprovalDenied.to_string())
            .bind(approval_json)
            .bind(approval.note.clone())
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn resolve_uncertain(
        &self,
        dispatch_id: &str,
        target_task_id: Option<i64>,
        now_ms: i64,
    ) -> DbResult<bool> {
        let (to_status, task_binding) = match target_task_id {
            Some(task_id) => ("Accepted", Some(task_id)),
            None => ("Canceled", None),
        };
        let sql = self.render_sql(
            "UPDATE dispatch_record
             SET status = ?, target_task_id = ?, updated_at = ?
             WHERE dispatch_id = ? AND status = 'Uncertain'",
        );
        let result = sqlx::query(&sql)
            .bind(to_status.to_string())
            .bind(task_binding)
            .bind(now_ms)
            .bind(dispatch_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Offers whose lease has run out (candidates for requeue / uncertain).
    pub async fn list_expired_offers(&self, now_ms: i64) -> DbResult<Vec<DispatchRecord>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_record
             WHERE status = 'Offered' AND offer_lease_expires_at IS NOT NULL
               AND offer_lease_expires_at <= ?",
        );
        let rows = sqlx::query(&sql)
            .bind(now_ms)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(record_from_row).collect()
    }

    /// Pre-accept records whose request deadline has passed. `Uncertain` is
    /// deliberately excluded: the target may already own a task, so only
    /// `resolve_uncertain` (or a late target accept) may move it.
    /// `PendingApproval` is included — the deadline covers the approval wait.
    pub async fn list_request_expired(&self, now_ms: i64) -> DbResult<Vec<DispatchRecord>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_record
             WHERE status IN ('PendingApproval', 'Queued', 'WaitingForTarget', 'Offered')
               AND expires_at IS NOT NULL AND expires_at <= ?",
        );
        let rows = sqlx::query(&sql)
            .bind(now_ms)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(record_from_row).collect()
    }

    /// Targets that still have handoffs in flight — used by the startup
    /// recovery scan.
    pub async fn list_targets_with_open_records(&self) -> DbResult<Vec<String>> {
        let sql = self.render_sql(
            "SELECT DISTINCT target_id FROM dispatch_record
             WHERE status IN ('Queued', 'WaitingForTarget', 'Offered')",
        );
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        rows.into_iter()
            .map(|row| row.try_get::<String, _>("target_id"))
            .collect()
    }

    pub async fn has_due_for_instance(
        &self,
        target_id: &str,
        instance_id: &str,
        now_ms: i64,
    ) -> DbResult<bool> {
        let sql = self.render_sql(
            "SELECT 1 AS due FROM dispatch_record
             WHERE target_id = ?
               AND (status IN ('Queued', 'WaitingForTarget')
                    OR (status = 'Offered' AND offer_instance_id = ?
                        AND offer_lease_expires_at > ?))
             LIMIT 1",
        );
        let row = sqlx::query(&sql)
            .bind(target_id.to_string())
            .bind(instance_id.to_string())
            .bind(now_ms)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.is_some())
    }

    /// Offered records assigned to this instance with a live lease.
    /// Idempotent: repeated pulls within the lease return the same batch.
    pub async fn claim_for_instance(
        &self,
        target_id: &str,
        instance_id: &str,
        now_ms: i64,
        max: u32,
    ) -> DbResult<Vec<DispatchRecord>> {
        let sql = self.render_sql(&format!(
            "SELECT * FROM dispatch_record
             WHERE target_id = ? AND status = 'Offered' AND offer_instance_id = ?
               AND offer_lease_expires_at > ?
             ORDER BY created_at ASC, dispatch_id ASC LIMIT {}",
            max.max(1)
        ));
        let rows = sqlx::query(&sql)
            .bind(target_id.to_string())
            .bind(instance_id.to_string())
            .bind(now_ms)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(record_from_row).collect()
    }

    /// Earliest pending deadline across offer leases, request expiries and
    /// instance leases — drives the maintenance timer.
    pub async fn next_deadline_ms(&self) -> DbResult<Option<i64>> {
        let mut deadlines: Vec<i64> = Vec::new();
        let offer_sql = self.render_sql(
            "SELECT MIN(offer_lease_expires_at) AS deadline FROM dispatch_record
             WHERE status = 'Offered' AND offer_lease_expires_at IS NOT NULL",
        );
        if let Some(deadline) = fetch_deadline(self.pool(), &offer_sql).await? {
            deadlines.push(deadline);
        }
        let expires_sql = self.render_sql(
            "SELECT MIN(expires_at) AS deadline FROM dispatch_record
             WHERE status IN ('PendingApproval', 'Queued', 'WaitingForTarget', 'Offered')
               AND expires_at IS NOT NULL",
        );
        if let Some(deadline) = fetch_deadline(self.pool(), &expires_sql).await? {
            deadlines.push(deadline);
        }
        let instance_sql =
            self.render_sql("SELECT MIN(lease_expires_at) AS deadline FROM dispatch_instance");
        if let Some(deadline) = fetch_deadline(self.pool(), &instance_sql).await? {
            deadlines.push(deadline);
        }
        Ok(deadlines.into_iter().min())
    }

    // ------------------------------------------------------------------
    // events
    // ------------------------------------------------------------------

    pub async fn insert_event(
        &self,
        dispatch_id: &str,
        ts_ms: i64,
        from_status: &str,
        to_status: &str,
        instance_id: Option<&str>,
        detail: Option<&str>,
    ) -> DbResult<()> {
        let sql = self.render_sql(
            "INSERT INTO dispatch_event (dispatch_id, ts, from_status, to_status, instance_id, detail)
             VALUES (?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(dispatch_id.to_string())
            .bind(ts_ms)
            .bind(from_status.to_string())
            .bind(to_status.to_string())
            .bind(instance_id.map(|v| v.to_string()))
            .bind(detail.map(|v| v.to_string()))
            .execute(self.pool())
            .await?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn list_events(&self, dispatch_id: &str) -> DbResult<Vec<(String, String)>> {
        let sql = self.render_sql(
            "SELECT from_status, to_status FROM dispatch_event
             WHERE dispatch_id = ? ORDER BY ts ASC, id ASC",
        );
        let rows = sqlx::query(&sql)
            .bind(dispatch_id.to_string())
            .fetch_all(self.pool())
            .await?;
        rows.into_iter()
            .map(|row| {
                let from: String = row.try_get("from_status")?;
                let to: String = row.try_get("to_status")?;
                Ok((from, to))
            })
            .collect()
    }
}

async fn fetch_deadline(pool: &AnyPool, sql: &str) -> DbResult<Option<i64>> {
    let row = sqlx::query(sql).fetch_one(pool).await?;
    // MIN() over an empty set yields NULL — surface as None.
    let deadline: Option<i64> = row.try_get("deadline").ok();
    Ok(deadline)
}

fn target_from_row(row: AnyRow) -> DbResult<TargetRow> {
    let registration_str: String = row.try_get("registration")?;
    let owner_user_id: String = row.try_get("owner_user_id")?;
    let owner_app_id: String = row.try_get("owner_app_id")?;
    let enabled: i64 = row.try_get("enabled")?;
    let registration: TargetRegistration =
        serde_json::from_str(&registration_str).map_err(|err| sqlx::Error::ColumnDecode {
            index: "registration".to_string(),
            source: Box::new(err),
        })?;
    Ok(TargetRow {
        registration,
        owner_user_id,
        owner_app_id,
        enabled: enabled != 0,
    })
}

fn route_from_row(row: AnyRow) -> DbResult<OperationRoute> {
    let operation: String = row.try_get("operation")?;
    let default_target_id: String = row.try_get("default_target_id")?;
    let revision: i64 = row.try_get("revision")?;
    let enabled: i64 = row.try_get("enabled")?;
    Ok(OperationRoute {
        operation,
        default_target_id,
        revision: revision.max(0) as u64,
        enabled: enabled != 0,
    })
}

fn instance_from_row(row: AnyRow) -> DbResult<TargetInstance> {
    let target_id: String = row.try_get("target_id")?;
    let instance_id: String = row.try_get("instance_id")?;
    let lease_epoch: i64 = row.try_get("lease_epoch")?;
    let lease_expires_at: i64 = row.try_get("lease_expires_at")?;
    let capacity: i64 = row.try_get("capacity")?;
    let available_capacity: i64 = row.try_get("available_capacity")?;
    Ok(TargetInstance {
        target_id,
        instance_id,
        lease_epoch: lease_epoch.max(0) as u64,
        lease_expires_at: lease_expires_at.max(0) as u64,
        capacity: capacity.max(0) as u32,
        available_capacity: available_capacity.max(0) as u32,
    })
}

fn record_from_row(row: AnyRow) -> DbResult<DispatchRecord> {
    let dispatch_id: String = row.try_get("dispatch_id")?;
    let requested_target_id: Option<String> = row.try_get("requested_target_id")?;
    let target_id: String = row.try_get("target_id")?;
    let target_selection_str: String = row.try_get("target_selection")?;
    let operation: String = row.try_get("operation")?;
    let status_str: String = row.try_get("status")?;
    let input_str: String = row.try_get("input")?;
    let auth_str: String = row.try_get("auth")?;
    let offer_instance_id: Option<String> = row.try_get("offer_instance_id")?;
    let offer_lease_expires_at: Option<i64> = row.try_get("offer_lease_expires_at")?;
    let offer_delivery_count: i64 = row.try_get("offer_delivery_count")?;
    let target_task_id: Option<i64> = row.try_get("target_task_id")?;
    let reject_reason: Option<String> = row.try_get("reject_reason")?;
    let approval_str: Option<String> = row.try_get("approval")?;
    let message: Option<String> = row.try_get("message")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    let target_selection: TargetSelection = serde_json::from_str(&target_selection_str)
        .map_err(|err| sqlx::Error::ColumnDecode {
            index: "target_selection".to_string(),
            source: Box::new(err),
        })?;
    let status =
        DispatchStatus::from_str(status_str.as_str()).map_err(|err| sqlx::Error::ColumnDecode {
            index: "status".to_string(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        })?;
    let input: Value = serde_json::from_str(&input_str).unwrap_or(Value::Null);
    let auth: DispatchAuthEnvelope =
        serde_json::from_str(&auth_str).map_err(|err| sqlx::Error::ColumnDecode {
            index: "auth".to_string(),
            source: Box::new(err),
        })?;
    let reject_reason = reject_reason
        .as_deref()
        .and_then(|value| DispatchRejectReason::from_str(value).ok());
    let approval: Option<DispatchApproval> = match approval_str.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            Some(serde_json::from_str(value).map_err(|err| sqlx::Error::ColumnDecode {
                index: "approval".to_string(),
                source: Box::new(err),
            })?)
        }
        _ => None,
    };

    Ok(DispatchRecord {
        dispatch_id,
        requested_target_id,
        target_id,
        target_selection,
        operation,
        status,
        input,
        auth,
        offer_instance_id,
        offer_lease_expires_at: offer_lease_expires_at.map(|v| v.max(0) as u64),
        offer_delivery_count: offer_delivery_count.max(0) as u32,
        target_task_id,
        reject_reason,
        approval,
        message,
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
    })
}
