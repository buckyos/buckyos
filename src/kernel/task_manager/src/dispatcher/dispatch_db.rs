//! Task Dispatch Center 2.0 durable store (doc §11/§15.8).
//!
//! Owns the dispatcher's independent RDB: dispatch records with a stable
//! queue order, write-ahead DeliveryAttempts, versioned runner
//! registrations, operation routes, instance leases and the persisted
//! round-robin cursor. No SQL joins into the Task Core store.

use crate::task_store::{rewrite_placeholders_to_dollar, split_sql_statements};
use buckyos_api::*;
use kRPC::{RPCErrors, Result};
use log::*;
use serde_json::Value;
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};
use std::sync::Once;

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

fn db_err(err: sqlx::Error) -> RPCErrors {
    RPCErrors::ReasonError(format!("dispatch store error: {}", err))
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => {
            let message = db.message().to_ascii_lowercase();
            message.contains("unique") || message.contains("duplicate")
        }
        _ => false,
    }
}

pub struct DispatchDb {
    pool: AnyPool,
    backend: RdbBackend,
}

impl DispatchDb {
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> std::result::Result<Self, String> {
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

    pub async fn open_from_service_spec() -> std::result::Result<Self, String> {
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

    async fn apply_schema(
        &self,
        override_ddl: Option<&str>,
    ) -> std::result::Result<(), sqlx::Error> {
        let ddl: &str =
            override_ddl
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(match self.backend {
                    RdbBackend::Sqlite => TASK_DISPATCHER_RDB_SCHEMA_SQLITE,
                    RdbBackend::Postgres => TASK_DISPATCHER_RDB_SCHEMA_POSTGRES,
                });
        for statement in split_sql_statements(ddl) {
            self.pool.execute(statement.as_str()).await?;
        }
        // beta2.2 no-compat: a v2 store (dispatch_record.operation column)
        // is dropped and rebuilt on the v3 layout.
        let v2 = match self.backend {
            RdbBackend::Sqlite => sqlx::query(
                "SELECT 1 FROM pragma_table_info('dispatch_record') WHERE name = 'operation'",
            )
            .fetch_optional(&self.pool)
            .await?
            .is_some(),
            RdbBackend::Postgres => sqlx::query(
                "SELECT 1 FROM information_schema.columns WHERE table_name = 'dispatch_record' AND column_name = 'operation'",
            )
            .fetch_optional(&self.pool)
            .await?
            .is_some(),
        };
        if v2 {
            warn!(
                "dispatch_db: 1.x schema detected; rebuilding as dispatcher schema v3 (no-compat)"
            );
            for table in [
                "delivery_attempt",
                "dispatch_record",
                "runner_registration",
                "operation_route",
                "target_instance",
                "dispatch_cursor",
                "dispatch_approval",
                "dispatch_event",
                "target_registration",
            ] {
                let sql = format!("DROP TABLE IF EXISTS {}", table);
                self.pool.execute(sql.as_str()).await?;
            }
            let ddl = match self.backend {
                RdbBackend::Sqlite => TASK_DISPATCHER_RDB_SCHEMA_SQLITE,
                RdbBackend::Postgres => TASK_DISPATCHER_RDB_SCHEMA_POSTGRES,
            };
            for statement in split_sql_statements(ddl) {
                self.pool.execute(statement.as_str()).await?;
            }
        }
        Ok(())
    }

    fn render_sql(&self, sql: &str) -> String {
        match self.backend {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    // -----------------------------------------------------------------
    // Dispatch records
    // -----------------------------------------------------------------

    /// Insert a fresh record in `CreatingTask`. Returns the existing record
    /// on an idempotency-key replay.
    pub async fn insert_record(
        &self,
        record: &DispatchRecord,
        idempotency_key: &str,
    ) -> Result<Option<DispatchRecord>> {
        let sql = self.render_sql(
            "INSERT INTO dispatch_record (
                dispatch_id, idempotency_key, requested_by_user, requested_by_app,
                schema_id, schema_version, requested_target_id, target_id,
                target_selection_json, registration_revision, delivery_policy_json,
                status, task_id, input_json, input_digest, auth_json, priority,
                ready_at, attempt_count, reject_reason, approval_json, message,
                expires_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        let insert = sqlx::query(&sql)
            .bind(&record.dispatch_id)
            .bind(idempotency_key)
            .bind(&record.auth.requested_by_user)
            .bind(&record.auth.requested_by_app)
            .bind(&record.schema_id)
            .bind(record.schema_version as i64)
            .bind(record.requested_target_id.clone())
            .bind(&record.target_id)
            .bind(serde_json::to_string(&record.target_selection).unwrap_or_default())
            .bind(record.registration_revision as i64)
            .bind(serde_json::to_string(&record.delivery_policy).unwrap_or_default())
            .bind(record.status.to_string())
            .bind(record.task_id.clone())
            .bind(record.input.to_string())
            .bind(&record.auth.input_digest)
            .bind(serde_json::to_string(&record.auth).unwrap_or_default())
            .bind(record.priority)
            .bind(record.ready_at as i64)
            .bind(record.attempt_count as i64)
            .bind(record.reject_reason.map(|r| r.to_string()))
            .bind(
                record
                    .approval
                    .as_ref()
                    .map(|a| serde_json::to_string(a).unwrap_or_default()),
            )
            .bind(record.message.clone())
            .bind(record.expires_at.map(|v| v as i64))
            .bind(record.created_at as i64)
            .bind(record.updated_at as i64)
            .execute(&self.pool)
            .await;
        match insert {
            Ok(_) => Ok(None),
            Err(err) if is_unique_violation(&err) => {
                let existing = self
                    .get_record_by_idempotency(
                        &record.auth.requested_by_user,
                        &record.auth.requested_by_app,
                        idempotency_key,
                    )
                    .await?;
                Ok(existing)
            }
            Err(err) => Err(db_err(err)),
        }
    }

    pub async fn get_record_by_idempotency(
        &self,
        user: &str,
        app: &str,
        idempotency_key: &str,
    ) -> Result<Option<DispatchRecord>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_record WHERE requested_by_user = ? AND requested_by_app = ? AND idempotency_key = ?",
        );
        let row = sqlx::query(&sql)
            .bind(user)
            .bind(app)
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.map(record_from_row).transpose().map_err(db_err)
    }

    pub async fn get_record(&self, dispatch_id: &str) -> Result<Option<DispatchRecord>> {
        let sql = self.render_sql("SELECT * FROM dispatch_record WHERE dispatch_id = ?");
        let row = sqlx::query(&sql)
            .bind(dispatch_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.map(record_from_row).transpose().map_err(db_err)
    }

    pub async fn get_record_by_task(&self, task_id: &str) -> Result<Option<DispatchRecord>> {
        let sql = self.render_sql("SELECT * FROM dispatch_record WHERE task_id = ?");
        let row = sqlx::query(&sql)
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.map(record_from_row).transpose().map_err(db_err)
    }

    /// Guarded status/state transition. `expected_status` implements the
    /// dispatcher's own CAS: recovery and the evaluation loop can race, only
    /// one transition wins.
    pub async fn update_record_state(
        &self,
        dispatch_id: &str,
        expected_status: DispatchStatus,
        update: RecordStateUpdate,
    ) -> Result<bool> {
        let now = crate::task_store::now_ms();
        let sql = self.render_sql(
            "UPDATE dispatch_record SET
                status = ?,
                task_id = COALESCE(?, task_id),
                ready_at = COALESCE(?, ready_at),
                attempt_count = COALESCE(?, attempt_count),
                reject_reason = COALESCE(?, reject_reason),
                approval_json = COALESCE(?, approval_json),
                message = COALESCE(?, message),
                updated_at = ?
            WHERE dispatch_id = ? AND status = ?",
        );
        let result = sqlx::query(&sql)
            .bind(update.new_status.to_string())
            .bind(update.task_id)
            .bind(update.ready_at.map(|v| v as i64))
            .bind(update.attempt_count.map(|v| v as i64))
            .bind(update.reject_reason.map(|r| r.to_string()))
            .bind(
                update
                    .approval
                    .as_ref()
                    .map(|a| serde_json::to_string(a).unwrap_or_default()),
            )
            .bind(update.message)
            .bind(now as i64)
            .bind(dispatch_id)
            .bind(expected_status.to_string())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected() > 0)
    }

    /// Stable queue order (doc §11.2):
    /// `(priority DESC, ready_at ASC, created_at ASC, dispatch_id ASC)`.
    pub async fn list_due_assignable(&self, now: u64, limit: u32) -> Result<Vec<DispatchRecord>> {
        let sql = self.render_sql(
            "SELECT * FROM dispatch_record
             WHERE status IN ('Queued', 'WaitingForTarget') AND ready_at <= ?
             ORDER BY priority DESC, ready_at ASC, created_at ASC, dispatch_id ASC
             LIMIT ?",
        );
        let rows = sqlx::query(&sql)
            .bind(now as i64)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|row| record_from_row(row).map_err(db_err))
            .collect()
    }

    pub async fn list_records_in_status(
        &self,
        statuses: &[DispatchStatus],
    ) -> Result<Vec<DispatchRecord>> {
        let mut records = Vec::new();
        for status in statuses {
            let sql = self.render_sql(
                "SELECT * FROM dispatch_record WHERE status = ? ORDER BY priority DESC, ready_at ASC, created_at ASC, dispatch_id ASC",
            );
            let rows = sqlx::query(&sql)
                .bind(status.to_string())
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
            for row in rows {
                records.push(record_from_row(row).map_err(db_err)?);
            }
        }
        Ok(records)
    }

    pub async fn list_records(
        &self,
        status: Option<DispatchStatus>,
        target_id: Option<&str>,
        schema_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DispatchRecord>> {
        let mut sql = String::from("SELECT * FROM dispatch_record");
        let mut conditions = Vec::new();
        let mut binds: Vec<String> = Vec::new();
        if let Some(status) = status {
            conditions.push("status = ?");
            binds.push(status.to_string());
        }
        if let Some(target_id) = target_id {
            conditions.push("target_id = ?");
            binds.push(target_id.to_string());
        }
        if let Some(schema_id) = schema_id {
            conditions.push("schema_id = ?");
            binds.push(schema_id.to_string());
        }
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC, dispatch_id ASC LIMIT ");
        sql.push_str(&limit.clamp(1, 500).to_string());
        let sql = self.render_sql(&sql);
        let mut query = sqlx::query(&sql);
        for bind in binds {
            query = query.bind(bind);
        }
        let rows = query.fetch_all(&self.pool).await.map_err(db_err)?;
        rows.into_iter()
            .map(|row| record_from_row(row).map_err(db_err))
            .collect()
    }

    /// Non-terminal records whose public task may carry a pending cancel.
    pub async fn list_open_records(&self) -> Result<Vec<DispatchRecord>> {
        self.list_records_in_status(&[
            DispatchStatus::CreatingTask,
            DispatchStatus::PendingApproval,
            DispatchStatus::Queued,
            DispatchStatus::WaitingForTarget,
            DispatchStatus::Offering,
            DispatchStatus::Binding,
            DispatchStatus::Activating,
        ])
        .await
    }

    // -----------------------------------------------------------------
    // Delivery attempts (write-ahead journal)
    // -----------------------------------------------------------------

    pub async fn insert_attempt(&self, attempt: &DeliveryAttempt) -> Result<()> {
        let sql = self.render_sql(
            "INSERT INTO delivery_attempt (
                dispatch_id, attempt_no, delivery_id, lease_epoch, target_id,
                instance_id, endpoint, stage, outcome, outcome_detail,
                reservation_token, runner_epoch, deadline_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(&attempt.dispatch_id)
            .bind(attempt.attempt_no as i64)
            .bind(&attempt.delivery_id)
            .bind(attempt.lease_epoch as i64)
            .bind(&attempt.target_id)
            .bind(&attempt.instance_id)
            .bind(&attempt.endpoint)
            .bind(attempt.stage.to_string())
            .bind(attempt.outcome.map(|o| o.to_string()))
            .bind(attempt.outcome_detail.clone())
            .bind(attempt.reservation_token.clone())
            .bind(attempt.runner_epoch.map(|v| v as i64))
            .bind(attempt.deadline_at as i64)
            .bind(attempt.created_at as i64)
            .bind(attempt.updated_at as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn update_attempt(
        &self,
        dispatch_id: &str,
        attempt_no: u32,
        stage: DeliveryStage,
        outcome: Option<DeliveryOutcome>,
        outcome_detail: Option<String>,
        reservation_token: Option<String>,
        runner_epoch: Option<u64>,
    ) -> Result<()> {
        let now = crate::task_store::now_ms();
        let sql = self.render_sql(
            "UPDATE delivery_attempt SET
                stage = ?,
                outcome = ?,
                outcome_detail = COALESCE(?, outcome_detail),
                reservation_token = COALESCE(?, reservation_token),
                runner_epoch = COALESCE(?, runner_epoch),
                updated_at = ?
            WHERE dispatch_id = ? AND attempt_no = ?",
        );
        sqlx::query(&sql)
            .bind(stage.to_string())
            .bind(outcome.map(|o| o.to_string()))
            .bind(outcome_detail)
            .bind(reservation_token)
            .bind(runner_epoch.map(|v| v as i64))
            .bind(now as i64)
            .bind(dispatch_id)
            .bind(attempt_no as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn latest_attempt(&self, dispatch_id: &str) -> Result<Option<DeliveryAttempt>> {
        let sql = self.render_sql(
            "SELECT * FROM delivery_attempt WHERE dispatch_id = ? ORDER BY attempt_no DESC LIMIT 1",
        );
        let row = sqlx::query(&sql)
            .bind(dispatch_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.map(attempt_from_row).transpose().map_err(db_err)
    }

    // -----------------------------------------------------------------
    // Runner registrations & routes
    // -----------------------------------------------------------------

    /// Upsert a registration; every accepted write increments the revision.
    pub async fn upsert_registration(
        &self,
        mut registration: TargetRegistration,
    ) -> Result<TargetRegistration> {
        let now = crate::task_store::now_ms();
        let existing = self.get_registration(&registration.target_id).await?;
        registration.registration_revision = existing
            .as_ref()
            .map(|r| r.registration_revision + 1)
            .unwrap_or(1);
        let registration_json = serde_json::to_string(&registration)
            .map_err(|e| RPCErrors::ReasonError(format!("serialize registration: {}", e)))?;
        if existing.is_some() {
            let sql = self.render_sql(
                "UPDATE runner_registration SET owner_user_id = ?, owner_app_id = ?, registration_revision = ?, registration_json = ?, enabled = ?, updated_at = ? WHERE target_id = ?",
            );
            sqlx::query(&sql)
                .bind(&registration.owner_user_id)
                .bind(&registration.owner_app_id)
                .bind(registration.registration_revision as i64)
                .bind(&registration_json)
                .bind(if registration.enabled { 1i64 } else { 0i64 })
                .bind(now as i64)
                .bind(&registration.target_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        } else {
            let sql = self.render_sql(
                "INSERT INTO runner_registration (target_id, owner_user_id, owner_app_id, registration_revision, registration_json, enabled, last_lease_epoch, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 0, ?)",
            );
            sqlx::query(&sql)
                .bind(&registration.target_id)
                .bind(&registration.owner_user_id)
                .bind(&registration.owner_app_id)
                .bind(registration.registration_revision as i64)
                .bind(&registration_json)
                .bind(if registration.enabled { 1i64 } else { 0i64 })
                .bind(now as i64)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        Ok(registration)
    }

    pub async fn get_registration(&self, target_id: &str) -> Result<Option<TargetRegistration>> {
        let sql = self.render_sql(
            "SELECT registration_json, enabled FROM runner_registration WHERE target_id = ?",
        );
        let row = sqlx::query(&sql)
            .bind(target_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(row) = row else { return Ok(None) };
        let registration_json: String = row.try_get("registration_json").map_err(db_err)?;
        let enabled: i64 = row.try_get("enabled").map_err(db_err)?;
        let mut registration: TargetRegistration = serde_json::from_str(&registration_json)
            .map_err(|e| RPCErrors::ReasonError(format!("parse registration: {}", e)))?;
        registration.enabled = enabled != 0;
        Ok(Some(registration))
    }

    pub async fn list_registrations(&self) -> Result<Vec<TargetRegistration>> {
        let sql = self.render_sql(
            "SELECT registration_json, enabled FROM runner_registration ORDER BY target_id",
        );
        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        let mut registrations = Vec::new();
        for row in rows {
            let registration_json: String = row.try_get("registration_json").map_err(db_err)?;
            let enabled: i64 = row.try_get("enabled").map_err(db_err)?;
            let mut registration: TargetRegistration = serde_json::from_str(&registration_json)
                .map_err(|e| RPCErrors::ReasonError(format!("parse registration: {}", e)))?;
            registration.enabled = enabled != 0;
            registrations.push(registration);
        }
        Ok(registrations)
    }

    pub async fn set_registration_enabled(&self, target_id: &str, enabled: bool) -> Result<bool> {
        let now = crate::task_store::now_ms();
        let sql = self.render_sql(
            "UPDATE runner_registration SET enabled = ?, updated_at = ? WHERE target_id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(if enabled { 1i64 } else { 0i64 })
            .bind(now as i64)
            .bind(target_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected() > 0)
    }

    /// Allocate the next per-target lease epoch (strictly increasing,
    /// durable across restarts).
    pub async fn next_lease_epoch(&self, target_id: &str) -> Result<u64> {
        let now = crate::task_store::now_ms();
        let sql = self.render_sql(
            "UPDATE runner_registration SET last_lease_epoch = last_lease_epoch + 1, updated_at = ? WHERE target_id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(now as i64)
            .bind(target_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(RPCErrors::ReasonError(format!(
                "{}: {}",
                DISPATCH_ERR_TARGET_NOT_REGISTERED, target_id
            )));
        }
        let sql =
            self.render_sql("SELECT last_lease_epoch FROM runner_registration WHERE target_id = ?");
        let row = sqlx::query(&sql)
            .bind(target_id)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        let epoch: i64 = row.try_get("last_lease_epoch").map_err(db_err)?;
        Ok(epoch.max(0) as u64)
    }

    pub async fn set_operation_route(
        &self,
        schema_id: &str,
        default_target_id: &str,
    ) -> Result<OperationRoute> {
        let now = crate::task_store::now_ms();
        let existing = self.get_operation_route(schema_id).await?;
        let revision = existing.map(|r| r.revision + 1).unwrap_or(1);
        let sql = self.render_sql(
            "INSERT INTO operation_route (schema_id, default_target_id, revision, enabled, updated_at) VALUES (?, ?, ?, 1, ?)
             ON CONFLICT(schema_id) DO UPDATE SET default_target_id = ?, revision = ?, enabled = 1, updated_at = ?",
        );
        sqlx::query(&sql)
            .bind(schema_id)
            .bind(default_target_id)
            .bind(revision as i64)
            .bind(now as i64)
            .bind(default_target_id)
            .bind(revision as i64)
            .bind(now as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(OperationRoute {
            schema_id: schema_id.to_string(),
            default_target_id: default_target_id.to_string(),
            revision,
            enabled: true,
        })
    }

    pub async fn disable_operation_route(&self, schema_id: &str) -> Result<()> {
        let now = crate::task_store::now_ms();
        let sql = self.render_sql(
            "UPDATE operation_route SET enabled = 0, revision = revision + 1, updated_at = ? WHERE schema_id = ?",
        );
        sqlx::query(&sql)
            .bind(now as i64)
            .bind(schema_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn get_operation_route(&self, schema_id: &str) -> Result<Option<OperationRoute>> {
        let sql = self.render_sql("SELECT * FROM operation_route WHERE schema_id = ?");
        let row = sqlx::query(&sql)
            .bind(schema_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(row) = row else { return Ok(None) };
        route_from_row(row).map(Some).map_err(db_err)
    }

    pub async fn list_operation_routes(&self) -> Result<Vec<OperationRoute>> {
        let sql = self.render_sql("SELECT * FROM operation_route ORDER BY schema_id");
        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|row| route_from_row(row).map_err(db_err))
            .collect()
    }

    // -----------------------------------------------------------------
    // Target instances (leases) & round-robin cursor
    // -----------------------------------------------------------------

    pub async fn upsert_instance(&self, instance: &TargetInstance) -> Result<()> {
        let now = crate::task_store::now_ms();
        let sql = self.render_sql(
            "INSERT INTO target_instance (target_id, instance_id, endpoint, lease_epoch, lease_expires_at, capacity, available_capacity, attached_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(target_id, instance_id) DO UPDATE SET endpoint = ?, lease_epoch = ?, lease_expires_at = ?, capacity = ?, available_capacity = ?, attached_at = ?",
        );
        sqlx::query(&sql)
            .bind(&instance.target_id)
            .bind(&instance.instance_id)
            .bind(&instance.endpoint)
            .bind(instance.lease_epoch as i64)
            .bind(instance.lease_expires_at as i64)
            .bind(instance.capacity as i64)
            .bind(instance.available_capacity as i64)
            .bind(now as i64)
            .bind(&instance.endpoint)
            .bind(instance.lease_epoch as i64)
            .bind(instance.lease_expires_at as i64)
            .bind(instance.capacity as i64)
            .bind(instance.available_capacity as i64)
            .bind(now as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn get_instance(
        &self,
        target_id: &str,
        instance_id: &str,
    ) -> Result<Option<TargetInstance>> {
        let sql = self
            .render_sql("SELECT * FROM target_instance WHERE target_id = ? AND instance_id = ?");
        let row = sqlx::query(&sql)
            .bind(target_id)
            .bind(instance_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.map(instance_from_row).transpose().map_err(db_err)
    }

    /// Live instances of a target, stable-ordered by instance id.
    pub async fn live_instances(&self, target_id: &str, now: u64) -> Result<Vec<TargetInstance>> {
        let sql = self.render_sql(
            "SELECT * FROM target_instance WHERE target_id = ? AND lease_expires_at > ? ORDER BY instance_id ASC",
        );
        let rows = sqlx::query(&sql)
            .bind(target_id)
            .bind(now as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|row| instance_from_row(row).map_err(db_err))
            .collect()
    }

    pub async fn remove_instance(&self, target_id: &str, instance_id: &str) -> Result<()> {
        let sql =
            self.render_sql("DELETE FROM target_instance WHERE target_id = ? AND instance_id = ?");
        sqlx::query(&sql)
            .bind(target_id)
            .bind(instance_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn update_instance_lease(
        &self,
        target_id: &str,
        instance_id: &str,
        lease_epoch: u64,
        lease_expires_at: u64,
        available_capacity: Option<u32>,
    ) -> Result<bool> {
        let sql = self.render_sql(
            "UPDATE target_instance SET lease_expires_at = ?, available_capacity = COALESCE(?, available_capacity)
             WHERE target_id = ? AND instance_id = ? AND lease_epoch = ?",
        );
        let result = sqlx::query(&sql)
            .bind(lease_expires_at as i64)
            .bind(available_capacity.map(|v| v as i64))
            .bind(target_id)
            .bind(instance_id)
            .bind(lease_epoch as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected() > 0)
    }

    /// Persisted RoundRobin cursor (doc §11.2 determinism requirement).
    pub async fn get_cursor(&self, target_id: &str) -> Result<Option<String>> {
        let sql =
            self.render_sql("SELECT cursor_instance_id FROM dispatch_cursor WHERE target_id = ?");
        let row = sqlx::query(&sql)
            .bind(target_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        match row {
            Some(row) => Ok(row.try_get("cursor_instance_id").map_err(db_err)?),
            None => Ok(None),
        }
    }

    pub async fn set_cursor(&self, target_id: &str, instance_id: &str) -> Result<()> {
        let now = crate::task_store::now_ms();
        let sql = self.render_sql(
            "INSERT INTO dispatch_cursor (target_id, cursor_instance_id, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(target_id) DO UPDATE SET cursor_instance_id = ?, updated_at = ?",
        );
        sqlx::query(&sql)
            .bind(target_id)
            .bind(instance_id)
            .bind(now as i64)
            .bind(instance_id)
            .bind(now as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

/// Field bundle for a guarded record transition.
pub struct RecordStateUpdate {
    pub new_status: DispatchStatus,
    pub task_id: Option<String>,
    pub ready_at: Option<u64>,
    pub attempt_count: Option<u32>,
    pub reject_reason: Option<DispatchRejectReason>,
    pub approval: Option<DispatchApproval>,
    pub message: Option<String>,
}

impl RecordStateUpdate {
    pub fn to_status(new_status: DispatchStatus) -> Self {
        Self {
            new_status,
            task_id: None,
            ready_at: None,
            attempt_count: None,
            reject_reason: None,
            approval: None,
            message: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Row decoding
// ---------------------------------------------------------------------------

fn record_from_row(row: AnyRow) -> std::result::Result<DispatchRecord, sqlx::Error> {
    let dispatch_id: String = row.try_get("dispatch_id")?;
    let schema_id: String = row.try_get("schema_id")?;
    let schema_version: i64 = row.try_get("schema_version")?;
    let requested_target_id: Option<String> = row.try_get("requested_target_id")?;
    let target_id: String = row.try_get("target_id")?;
    let target_selection_json: String = row.try_get("target_selection_json")?;
    let registration_revision: i64 = row.try_get("registration_revision")?;
    let delivery_policy_json: String = row.try_get("delivery_policy_json")?;
    let status: String = row.try_get("status")?;
    let task_id: Option<String> = row.try_get("task_id")?;
    let input_json: String = row.try_get("input_json")?;
    let auth_json: String = row.try_get("auth_json")?;
    let priority: i64 = row.try_get("priority")?;
    let ready_at: i64 = row.try_get("ready_at")?;
    let attempt_count: i64 = row.try_get("attempt_count")?;
    let reject_reason: Option<String> = row.try_get("reject_reason")?;
    let approval_json: Option<String> = row.try_get("approval_json")?;
    let message: Option<String> = row.try_get("message")?;
    let expires_at: Option<i64> = row.try_get("expires_at")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(DispatchRecord {
        dispatch_id,
        requested_target_id,
        target_id,
        target_selection: serde_json::from_str(&target_selection_json)
            .unwrap_or(TargetSelection::Explicit),
        schema_id,
        schema_version: schema_version.max(0) as u32,
        registration_revision: registration_revision.max(0) as u64,
        delivery_policy: serde_json::from_str(&delivery_policy_json).unwrap_or_default(),
        status: DispatchStatus::from_str(&status).unwrap_or(DispatchStatus::Failed),
        task_id,
        input: serde_json::from_str(&input_json).unwrap_or(Value::Null),
        auth: serde_json::from_str(&auth_json).unwrap_or(DispatchAuthEnvelope {
            requested_by_user: String::new(),
            requested_by_app: String::new(),
            on_behalf_of: String::new(),
            zone_trusted_caller: false,
            workflow_ref: None,
            input_digest: String::new(),
            created_at: 0,
            expires_at: None,
        }),
        priority,
        ready_at: ready_at.max(0) as u64,
        attempt_count: attempt_count.max(0) as u32,
        reject_reason: reject_reason.and_then(|r| DispatchRejectReason::from_str(&r).ok()),
        approval: approval_json.and_then(|a| serde_json::from_str(&a).ok()),
        message,
        expires_at: expires_at.map(|v| v.max(0) as u64),
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
    })
}

fn attempt_from_row(row: AnyRow) -> std::result::Result<DeliveryAttempt, sqlx::Error> {
    let dispatch_id: String = row.try_get("dispatch_id")?;
    let attempt_no: i64 = row.try_get("attempt_no")?;
    let delivery_id: String = row.try_get("delivery_id")?;
    let lease_epoch: i64 = row.try_get("lease_epoch")?;
    let target_id: String = row.try_get("target_id")?;
    let instance_id: String = row.try_get("instance_id")?;
    let endpoint: String = row.try_get("endpoint")?;
    let stage: String = row.try_get("stage")?;
    let outcome: Option<String> = row.try_get("outcome")?;
    let outcome_detail: Option<String> = row.try_get("outcome_detail")?;
    let reservation_token: Option<String> = row.try_get("reservation_token")?;
    let runner_epoch: Option<i64> = row.try_get("runner_epoch")?;
    let deadline_at: i64 = row.try_get("deadline_at")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(DeliveryAttempt {
        dispatch_id,
        attempt_no: attempt_no.max(0) as u32,
        delivery_id,
        lease_epoch: lease_epoch.max(0) as u64,
        target_id,
        instance_id,
        endpoint,
        stage: stage.parse().unwrap_or(DeliveryStage::Offer),
        outcome: outcome.and_then(|o| o.parse().ok()),
        outcome_detail,
        reservation_token,
        runner_epoch: runner_epoch.map(|v| v.max(0) as u64),
        deadline_at: deadline_at.max(0) as u64,
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
    })
}

fn route_from_row(row: AnyRow) -> std::result::Result<OperationRoute, sqlx::Error> {
    let schema_id: String = row.try_get("schema_id")?;
    let default_target_id: String = row.try_get("default_target_id")?;
    let revision: i64 = row.try_get("revision")?;
    let enabled: i64 = row.try_get("enabled")?;
    Ok(OperationRoute {
        schema_id,
        default_target_id,
        revision: revision.max(0) as u64,
        enabled: enabled != 0,
    })
}

fn instance_from_row(row: AnyRow) -> std::result::Result<TargetInstance, sqlx::Error> {
    let target_id: String = row.try_get("target_id")?;
    let instance_id: String = row.try_get("instance_id")?;
    let endpoint: String = row.try_get("endpoint")?;
    let lease_epoch: i64 = row.try_get("lease_epoch")?;
    let lease_expires_at: i64 = row.try_get("lease_expires_at")?;
    let capacity: i64 = row.try_get("capacity")?;
    let available_capacity: i64 = row.try_get("available_capacity")?;
    Ok(TargetInstance {
        target_id,
        instance_id,
        endpoint,
        lease_epoch: lease_epoch.max(0) as u64,
        lease_expires_at: lease_expires_at.max(0) as u64,
        capacity: capacity.max(0) as u32,
        available_capacity: available_capacity.max(0) as u32,
    })
}
