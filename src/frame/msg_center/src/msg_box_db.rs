/*!
 * Storage for the msg-center: mailbox records, the delivery queue and
 * ui-session state.
 *
 * beta2.2 split the legacy single `msg_records` table into two tables that
 * mirror the frozen data model (`doc/message_hub/Message Center.md` §2/§6):
 *
 *   mailbox_records   MailboxRecord — an owner's reference to one MsgObject
 *                     (INBOX / SENT / GROUP_INBOX / REQUEST_BOX, RecipientState)
 *   delivery_records  DeliveryRecord — the DELIVERY_QUEUE, keyed by executor
 *                     `transport_did` (DeliveryState, retries, results)
 *
 * Uses an `sqlx::AnyPool` so the backend (sqlite / postgres) is driven by the
 * zone's rdb instance config; the pool itself is `Send + Sync + Clone` and
 * manages its own locking. Every mailbox row carries an `owner` column so a
 * single database holds mailboxes for every zone user.
 */

use buckyos_api::{
    get_rdb_instance, msg_center_default_rdb_instance_config, DeliveryEnvelope, DeliveryError,
    DeliveryRecord, DeliveryState, IngressContext, MailboxKind, MailboxRecord, RdbBackend,
    RecipientState, UiSessionStateEntry, MSG_CENTER_RDB_INSTANCE_ID,
    MSG_CENTER_RDB_SCHEMA_POSTGRES, MSG_CENTER_RDB_SCHEMA_SQLITE, MSG_CENTER_SERVICE_NAME,
};
use kRPC::RPCErrors;
use log::info;
use name_lib::DID;
use ndn_lib::{MsgObject, ObjId};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};
use std::sync::{Arc, Once};

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

/// Per-session aggregate used by `list_sessions`.
#[derive(Debug, Clone)]
pub struct SessionIndexEntry {
    pub session_id: String,
    pub updated_at_ms: u64,
    pub unread_count: u64,
}

#[derive(Clone, Debug)]
pub struct MsgBoxDbMgr {
    inner: Arc<MsgBoxDbInner>,
}

#[derive(Debug)]
struct MsgBoxDbInner {
    pool: AnyPool,
    backend: RdbBackend,
}

const MAILBOX_COLUMNS: &str = r#"
    owner,
    record_id,
    box_kind,
    msg_id,
    msg_kind,
    msg_from,
    msg_to,
    state,
    session_id,
    sort_key,
    tags_json,
    ingress_json,
    created_at_ms,
    updated_at_ms
"#;

const DELIVERY_COLUMNS: &str = r#"
    delivery_id,
    transport_did,
    msg_id,
    target_did,
    envelope_json,
    state,
    attempts,
    next_retry_at_ms,
    external_msg_id,
    delivered_at_ms,
    last_error_json,
    created_at_ms,
    updated_at_ms
"#;

impl MsgBoxDbMgr {
    /// Open a pool against `connection`. `schema` is the DDL to apply (usually
    /// what the service spec carried for the chosen backend); an empty / None
    /// value means "use the compile-time default for `backend`".
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> std::result::Result<Self, RPCErrors> {
        ensure_any_drivers_installed();
        let mut opts = AnyPoolOptions::new().max_connections(8);
        if backend == RdbBackend::Sqlite {
            opts = opts.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON;").await?;
                    conn.execute("PRAGMA journal_mode = WAL;").await?;
                    Ok(())
                })
            });
        }
        let pool = opts.connect(connection).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "open msg-center db at {} failed: {}",
                connection, error
            ))
        })?;
        let inner = Arc::new(MsgBoxDbInner { pool, backend });
        let mgr = Self { inner };
        mgr.apply_schema(schema).await?;
        Ok(mgr)
    }

    /// Resolve the msg-center rdb instance from the service spec and open a
    /// pool against it. This is the production entry point.
    pub async fn open_from_service_spec() -> std::result::Result<Self, RPCErrors> {
        let instance = get_rdb_instance(MSG_CENTER_SERVICE_NAME, None, MSG_CENTER_RDB_INSTANCE_ID)
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("resolve msg-center rdb instance failed: {}", error))
            })?;
        info!("msg_box_db.open {}", instance.connection);
        Self::open(
            &instance.connection,
            instance.backend,
            instance.schema.as_deref(),
        )
        .await
    }

    /// Test / fallback entry: build a default instance config (sqlite) against
    /// the given connection string. The schema DDL comes from the compiled-in
    /// default for the chosen backend.
    #[allow(dead_code)]
    pub async fn open_default_sqlite(connection: &str) -> std::result::Result<Self, RPCErrors> {
        let cfg = msg_center_default_rdb_instance_config();
        let schema = cfg.schema.get(&RdbBackend::Sqlite).cloned();
        Self::open(connection, RdbBackend::Sqlite, schema.as_deref()).await
    }

    pub(crate) fn pool(&self) -> &AnyPool {
        &self.inner.pool
    }

    pub(crate) fn backend(&self) -> RdbBackend {
        self.inner.backend
    }

    async fn apply_schema(&self, override_ddl: Option<&str>) -> std::result::Result<(), RPCErrors> {
        let ddl: &str =
            override_ddl
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(match self.backend() {
                    RdbBackend::Sqlite => MSG_CENTER_RDB_SCHEMA_SQLITE,
                    RdbBackend::Postgres => MSG_CENTER_RDB_SCHEMA_POSTGRES,
                });
        for statement in split_sql_statements(ddl) {
            self.pool().execute(statement.as_str()).await.map_err(|e| {
                RPCErrors::ReasonError(format!("apply msg-center schema failed: {}", e))
            })?;
        }
        Ok(())
    }

    /// Translate `?` placeholders into `$N` form for postgres.
    fn render_sql(&self, sql: &str) -> String {
        match self.backend() {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    // ------------------------------------------------------------------
    // Mailbox records
    // ------------------------------------------------------------------

    pub async fn upsert_record(
        &self,
        record: &MailboxRecord,
    ) -> std::result::Result<(), RPCErrors> {
        self.upsert_record_with_msg(record, None).await
    }

    pub async fn upsert_record_with_msg(
        &self,
        record: &MailboxRecord,
        msg: Option<&MsgObject>,
    ) -> std::result::Result<(), RPCErrors> {
        self.upsert_record_row(record, msg).await?;
        self.touch_message(&record.owner, &record.msg_id, record.created_at_ms)
            .await?;
        Ok(())
    }

    async fn upsert_record_row(
        &self,
        record: &MailboxRecord,
        msg: Option<&MsgObject>,
    ) -> std::result::Result<(), RPCErrors> {
        let tags_json = serde_json::to_string(&record.tags).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode tags of mailbox record {}: {}",
                record.record_id, error
            ))
        })?;
        let ingress_json =
            encode_optional_json(record.ingress.as_ref(), &record.record_id, "ingress")?;
        let msg_from = Some(
            msg.map(|obj| obj.from.clone())
                .unwrap_or_else(|| record.from.clone())
                .to_string(),
        );
        let msg_to = Some(
            msg.and_then(|obj| obj.to.first().cloned())
                .unwrap_or_else(|| record.to.clone())
                .to_string(),
        );
        let msg_kind = msg.map(|obj| obj.kind).unwrap_or(record.msg_kind);

        let sql = self.render_sql(&format!(
            r#"
INSERT INTO mailbox_records ({MAILBOX_COLUMNS})
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(owner, record_id) DO UPDATE SET
    box_kind = excluded.box_kind,
    msg_id = excluded.msg_id,
    msg_kind = COALESCE(excluded.msg_kind, mailbox_records.msg_kind),
    msg_from = COALESCE(excluded.msg_from, mailbox_records.msg_from),
    msg_to = COALESCE(excluded.msg_to, mailbox_records.msg_to),
    state = excluded.state,
    session_id = COALESCE(excluded.session_id, mailbox_records.session_id),
    sort_key = excluded.sort_key,
    tags_json = excluded.tags_json,
    ingress_json = COALESCE(excluded.ingress_json, mailbox_records.ingress_json),
    created_at_ms = excluded.created_at_ms,
    updated_at_ms = excluded.updated_at_ms
"#
        ));

        sqlx::query(&sql)
            .bind(record.owner.to_string())
            .bind(record.record_id.clone())
            .bind(mailbox_kind_name(&record.box_kind).to_string())
            .bind(record.msg_id.to_string())
            .bind(Some(msg_obj_kind_name(&msg_kind).to_string()))
            .bind(msg_from)
            .bind(msg_to)
            .bind(recipient_state_name(&record.state).to_string())
            .bind(record.session_id.clone())
            .bind(to_sql_i64(record.sort_key))
            .bind(tags_json)
            .bind(ingress_json)
            .bind(to_sql_i64(record.created_at_ms))
            .bind(to_sql_i64(record.updated_at_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to upsert mailbox record {}: {}",
                    record.record_id, error
                ))
            })?;
        Ok(())
    }

    pub async fn get_record(
        &self,
        owner: &DID,
        record_id: &str,
    ) -> std::result::Result<Option<MailboxRecord>, RPCErrors> {
        let sql = self.render_sql(&format!(
            "SELECT {MAILBOX_COLUMNS} FROM mailbox_records WHERE owner = ? AND record_id = ?"
        ));

        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(record_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query mailbox record {}: {}",
                    record_id, error
                ))
            })?;

        row.as_ref().map(row_to_mailbox_record).transpose()
    }

    pub async fn list_records(
        &self,
        owner: &DID,
        box_kind: &MailboxKind,
        state_filter: Option<&[RecipientState]>,
        descending: bool,
    ) -> std::result::Result<Vec<MailboxRecord>, RPCErrors> {
        let order_clause = if descending {
            "ORDER BY sort_key DESC, record_id DESC"
        } else {
            "ORDER BY sort_key ASC, record_id ASC"
        };
        let sql = self.render_sql(&format!(
            "SELECT {MAILBOX_COLUMNS} FROM mailbox_records WHERE owner = ? AND box_kind = ? {order_clause}"
        ));

        let rows = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(mailbox_kind_name(box_kind).to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to list mailbox records: {}", error))
            })?;

        let mut records = Vec::with_capacity(rows.len());
        for row in &rows {
            let record = row_to_mailbox_record(row)?;
            if !recipient_state_matches(state_filter, &record.state) {
                continue;
            }
            records.push(record);
        }
        Ok(records)
    }

    /// Cursor-paged records of one `(owner, session_id)` timeline across all
    /// mailbox kinds. DELETED records are hidden from the projection.
    pub async fn list_session_records(
        &self,
        owner: &DID,
        session_id: &str,
        limit: usize,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<&str>,
        descending: bool,
    ) -> std::result::Result<Vec<MailboxRecord>, RPCErrors> {
        let (order_clause, cursor_clause) = if descending {
            (
                "ORDER BY sort_key DESC, record_id DESC",
                "AND (sort_key < ? OR (sort_key = ? AND record_id < ?))",
            )
        } else {
            (
                "ORDER BY sort_key ASC, record_id ASC",
                "AND (sort_key > ? OR (sort_key = ? AND record_id > ?))",
            )
        };
        let cursor_sql = if cursor_sort_key.is_some() {
            cursor_clause
        } else {
            ""
        };
        let sql = self.render_sql(&format!(
            r#"
SELECT {MAILBOX_COLUMNS} FROM mailbox_records
WHERE owner = ? AND session_id = ? AND state != 'DELETED'
{cursor_sql}
{order_clause}
LIMIT ?
"#
        ));

        let mut query = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session_id.to_string());
        if let Some(sort_key) = cursor_sort_key {
            let record_id = cursor_record_id.unwrap_or("").to_string();
            query = query
                .bind(to_sql_i64(sort_key))
                .bind(to_sql_i64(sort_key))
                .bind(record_id);
        }
        let rows = query
            .bind(limit as i64)
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to list session records for {}: {}",
                    session_id, error
                ))
            })?;

        rows.iter().map(row_to_mailbox_record).collect()
    }

    /// Session index for `list_sessions`: per session id, last activity and
    /// unread count, newest first, cursor on (updated_at_ms, session_id).
    pub async fn list_session_index(
        &self,
        owner: &DID,
        limit: usize,
        cursor_updated_at_ms: Option<u64>,
        cursor_session_id: Option<&str>,
    ) -> std::result::Result<Vec<SessionIndexEntry>, RPCErrors> {
        let cursor_sql = if cursor_updated_at_ms.is_some() {
            "HAVING (MAX(updated_at_ms) < ? OR (MAX(updated_at_ms) = ? AND session_id < ?))"
        } else {
            ""
        };
        let sql = self.render_sql(&format!(
            r#"
SELECT
    session_id,
    MAX(updated_at_ms) AS last_updated_at_ms,
    SUM(CASE WHEN state = 'UNREAD' THEN 1 ELSE 0 END) AS unread_count
FROM mailbox_records
WHERE owner = ? AND session_id IS NOT NULL AND state != 'DELETED'
GROUP BY session_id
{cursor_sql}
ORDER BY last_updated_at_ms DESC, session_id DESC
LIMIT ?
"#
        ));

        let mut query = sqlx::query(&sql).bind(owner.to_string());
        if let Some(cursor) = cursor_updated_at_ms {
            let session_id = cursor_session_id.unwrap_or("").to_string();
            query = query
                .bind(to_sql_i64(cursor))
                .bind(to_sql_i64(cursor))
                .bind(session_id);
        }
        let rows = query
            .bind(limit as i64)
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to list session index: {}", error))
            })?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let session_id: String = row
                .try_get("session_id")
                .map_err(|e| decode_err("session_id", &e))?;
            let updated_at_ms: i64 = row
                .try_get("last_updated_at_ms")
                .map_err(|e| decode_err("last_updated_at_ms", &e))?;
            let unread_count: i64 = row
                .try_get("unread_count")
                .map_err(|e| decode_err("unread_count", &e))?;
            entries.push(SessionIndexEntry {
                session_id,
                updated_at_ms: updated_at_ms.max(0) as u64,
                unread_count: unread_count.max(0) as u64,
            });
        }
        Ok(entries)
    }

    // ------------------------------------------------------------------
    // Delivery queue
    // ------------------------------------------------------------------

    pub async fn upsert_delivery(
        &self,
        record: &DeliveryRecord,
    ) -> std::result::Result<(), RPCErrors> {
        let envelope_json = serde_json::to_string(&record.envelope).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode delivery envelope {}: {}",
                record.delivery_id, error
            ))
        })?;
        let last_error_json =
            encode_optional_json(record.last_error.as_ref(), &record.delivery_id, "last_error")?;

        let sql = self.render_sql(&format!(
            r#"
INSERT INTO delivery_records ({DELIVERY_COLUMNS})
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(delivery_id) DO UPDATE SET
    state = excluded.state,
    attempts = excluded.attempts,
    next_retry_at_ms = excluded.next_retry_at_ms,
    external_msg_id = excluded.external_msg_id,
    delivered_at_ms = excluded.delivered_at_ms,
    last_error_json = excluded.last_error_json,
    updated_at_ms = excluded.updated_at_ms
"#
        ));

        sqlx::query(&sql)
            .bind(record.delivery_id.clone())
            .bind(record.envelope.transport_did.to_string())
            .bind(record.envelope.msg_id.to_string())
            .bind(record.envelope.target_did.to_string())
            .bind(envelope_json)
            .bind(delivery_state_name(&record.state).to_string())
            .bind(record.attempts as i64)
            .bind(record.next_retry_at_ms.map(to_sql_i64))
            .bind(record.external_msg_id.clone())
            .bind(record.delivered_at_ms.map(to_sql_i64))
            .bind(last_error_json)
            .bind(to_sql_i64(record.created_at_ms))
            .bind(to_sql_i64(record.updated_at_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to upsert delivery record {}: {}",
                    record.delivery_id, error
                ))
            })?;
        Ok(())
    }

    /// Insert-if-absent for `post_send`: an existing delivery (idempotent
    /// resubmission) keeps its current state/attempts untouched.
    pub async fn create_delivery_if_absent(
        &self,
        record: &DeliveryRecord,
    ) -> std::result::Result<(), RPCErrors> {
        if self.get_delivery(&record.delivery_id).await?.is_some() {
            return Ok(());
        }
        self.upsert_delivery(record).await
    }

    pub async fn get_delivery(
        &self,
        delivery_id: &str,
    ) -> std::result::Result<Option<DeliveryRecord>, RPCErrors> {
        let sql = self.render_sql(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM delivery_records WHERE delivery_id = ?"
        ));
        let row = sqlx::query(&sql)
            .bind(delivery_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query delivery record {}: {}",
                    delivery_id, error
                ))
            })?;
        row.as_ref().map(row_to_delivery_record).transpose()
    }

    /// Reclaim SENDING rows whose lease expired (executor crash): back to WAIT
    /// with attempts+1 and a duplicate-risk marker.
    pub async fn reclaim_stale_sending(
        &self,
        transport_did: &DID,
        lease_deadline_ms: u64,
        now_ms: u64,
    ) -> std::result::Result<u64, RPCErrors> {
        let stale_error = serde_json::to_string(&DeliveryError {
            error_code: Some("lease_expired".to_string()),
            message: "SENDING lease expired; reclaimed by sweep".to_string(),
            retryable: true,
            duplicate_risk: true,
        })
        .map_err(|error| {
            RPCErrors::ReasonError(format!("failed to encode stale delivery error: {}", error))
        })?;
        let sql = self.render_sql(
            r#"
UPDATE delivery_records
SET state = 'WAIT', attempts = attempts + 1, last_error_json = ?, updated_at_ms = ?
WHERE transport_did = ? AND state = 'SENDING' AND updated_at_ms < ?
"#,
        );
        let result = sqlx::query(&sql)
            .bind(stale_error)
            .bind(to_sql_i64(now_ms))
            .bind(transport_did.to_string())
            .bind(to_sql_i64(lease_deadline_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to reclaim stale deliveries: {}", error))
            })?;
        Ok(result.rows_affected())
    }

    /// Take the next due WAIT delivery of an executor. When `lock_on_take` the
    /// row is CAS-moved to SENDING (owner-crash-safe via the lease sweep).
    pub async fn take_next_delivery(
        &self,
        transport_did: &DID,
        now_ms: u64,
        lock_on_take: bool,
    ) -> std::result::Result<Option<DeliveryRecord>, RPCErrors> {
        let sql = self.render_sql(&format!(
            r#"
SELECT {DELIVERY_COLUMNS} FROM delivery_records
WHERE transport_did = ? AND state = 'WAIT'
  AND (next_retry_at_ms IS NULL OR next_retry_at_ms <= ?)
ORDER BY created_at_ms ASC, delivery_id ASC
LIMIT 1
"#
        ));
        let row = sqlx::query(&sql)
            .bind(transport_did.to_string())
            .bind(to_sql_i64(now_ms))
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to take next delivery: {}", error))
            })?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut record = row_to_delivery_record(&row)?;
        if !lock_on_take {
            return Ok(Some(record));
        }

        let cas_sql = self.render_sql(
            "UPDATE delivery_records SET state = 'SENDING', updated_at_ms = ? \
             WHERE delivery_id = ? AND state = 'WAIT'",
        );
        let cas = sqlx::query(&cas_sql)
            .bind(to_sql_i64(now_ms))
            .bind(record.delivery_id.clone())
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to lock delivery record: {}", error))
            })?;
        if cas.rows_affected() == 0 {
            // Lost the race to a concurrent taker; report empty this round.
            return Ok(None);
        }
        record.state = DeliveryState::Sending;
        record.updated_at_ms = now_ms;
        Ok(Some(record))
    }

    /// All delivery records referencing one message (outbound aggregation).
    pub async fn list_deliveries_for_msg(
        &self,
        msg_id: &ObjId,
    ) -> std::result::Result<Vec<DeliveryRecord>, RPCErrors> {
        let sql = self.render_sql(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM delivery_records WHERE msg_id = ? \
             ORDER BY target_did ASC, delivery_id ASC"
        ));
        let rows = sqlx::query(&sql)
            .bind(msg_id.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to list deliveries for msg {}: {}",
                    msg_id.to_string(),
                    error
                ))
            })?;
        rows.iter().map(row_to_delivery_record).collect()
    }

    // ------------------------------------------------------------------
    // Message refs + ui session state (unchanged model)
    // ------------------------------------------------------------------

    pub async fn touch_message(
        &self,
        owner: &DID,
        msg_id: &ObjId,
        created_at_ms: u64,
    ) -> std::result::Result<(), RPCErrors> {
        let sql = self.render_sql(
            "INSERT INTO msg_refs(owner, msg_id, created_at_ms) VALUES(?, ?, ?)
             ON CONFLICT(owner, msg_id) DO NOTHING",
        );
        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(msg_id.to_string())
            .bind(to_sql_i64(created_at_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to persist message ref {}: {}",
                    msg_id.to_string(),
                    error
                ))
            })?;
        Ok(())
    }

    pub async fn has_message(
        &self,
        owner: &DID,
        msg_id: &ObjId,
    ) -> std::result::Result<bool, RPCErrors> {
        let sql = self.render_sql("SELECT 1 FROM msg_refs WHERE owner = ? AND msg_id = ? LIMIT 1");
        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(msg_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query message ref {}: {}",
                    msg_id.to_string(),
                    error
                ))
            })?;
        Ok(row.is_some())
    }

    pub async fn upsert_ui_session_state(
        &self,
        session_id: &str,
        key: &str,
        value: &Value,
        updated_at_ms: u64,
    ) -> std::result::Result<UiSessionStateEntry, RPCErrors> {
        let value_json = serde_json::to_string(value).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode ui session state {}:{}: {}",
                session_id, key, error
            ))
        })?;
        let sql = self.render_sql(
            r#"
INSERT INTO ui_session_states (
    session_id,
    state_key,
    value_json,
    updated_at_ms
) VALUES (?, ?, ?, ?)
ON CONFLICT(session_id, state_key) DO UPDATE SET
    value_json = excluded.value_json,
    updated_at_ms = excluded.updated_at_ms
"#,
        );

        sqlx::query(&sql)
            .bind(session_id.to_string())
            .bind(key.to_string())
            .bind(value_json)
            .bind(to_sql_i64(updated_at_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to upsert ui session state {}:{}: {}",
                    session_id, key, error
                ))
            })?;

        Ok(UiSessionStateEntry {
            session_id: session_id.to_string(),
            key: key.to_string(),
            value: value.clone(),
            updated_at_ms,
        })
    }

    pub async fn get_ui_session_state(
        &self,
        session_id: &str,
        key: &str,
    ) -> std::result::Result<Option<UiSessionStateEntry>, RPCErrors> {
        let sql = self.render_sql(
            r#"
SELECT session_id, state_key, value_json, updated_at_ms
FROM ui_session_states
WHERE session_id = ? AND state_key = ?
"#,
        );
        let row = sqlx::query(&sql)
            .bind(session_id.to_string())
            .bind(key.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query ui session state {}:{}: {}",
                    session_id, key, error
                ))
            })?;

        row.as_ref().map(decode_ui_session_state_row).transpose()
    }

    pub async fn list_ui_session_state(
        &self,
        session_id: &str,
    ) -> std::result::Result<Vec<UiSessionStateEntry>, RPCErrors> {
        let sql = self.render_sql(
            r#"
SELECT session_id, state_key, value_json, updated_at_ms
FROM ui_session_states
WHERE session_id = ?
ORDER BY state_key ASC
"#,
        );
        let rows = sqlx::query(&sql)
            .bind(session_id.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to list ui session state {}: {}",
                    session_id, error
                ))
            })?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            entries.push(decode_ui_session_state_row(row)?);
        }
        Ok(entries)
    }
}

fn decode_err(field: &str, err: &sqlx::Error) -> RPCErrors {
    RPCErrors::ReasonError(format!("failed to decode column {}: {}", field, err))
}

fn decode_ui_session_state_row(
    row: &AnyRow,
) -> std::result::Result<UiSessionStateEntry, RPCErrors> {
    let session_id: String = row
        .try_get("session_id")
        .map_err(|e| decode_err("session_id", &e))?;
    let key: String = row
        .try_get("state_key")
        .map_err(|e| decode_err("state_key", &e))?;
    let value_json: String = row
        .try_get("value_json")
        .map_err(|e| decode_err("value_json", &e))?;
    let updated_at_ms: i64 = row
        .try_get("updated_at_ms")
        .map_err(|e| decode_err("updated_at_ms", &e))?;
    let value = serde_json::from_str(&value_json).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "failed to parse ui session state {}:{}: {}",
            session_id, key, error
        ))
    })?;
    let updated_at_ms = from_sql_i64(updated_at_ms, "updated_at_ms", &session_id)?;

    Ok(UiSessionStateEntry {
        session_id,
        key,
        value,
        updated_at_ms,
    })
}

fn row_to_mailbox_record(row: &AnyRow) -> std::result::Result<MailboxRecord, RPCErrors> {
    let record_id: String = row
        .try_get("record_id")
        .map_err(|e| decode_err("record_id", &e))?;
    let owner_raw: String = row.try_get("owner").map_err(|e| decode_err("owner", &e))?;
    let owner = DID::from_str(&owner_raw).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "invalid owner for mailbox record {}: {}",
            record_id, error
        ))
    })?;
    let box_kind_raw: String = row
        .try_get("box_kind")
        .map_err(|e| decode_err("box_kind", &e))?;
    let box_kind = mailbox_kind_from_name(&box_kind_raw)?;
    let msg_id_raw: String = row
        .try_get("msg_id")
        .map_err(|e| decode_err("msg_id", &e))?;
    let msg_id = parse_obj_id(&msg_id_raw, &record_id)?;
    let msg_kind_raw: Option<String> = row
        .try_get("msg_kind")
        .map_err(|e| decode_err("msg_kind", &e))?;
    let msg_kind = parse_msg_obj_kind(msg_kind_raw.as_deref(), &box_kind);
    let state_raw: String = row.try_get("state").map_err(|e| decode_err("state", &e))?;
    let state = recipient_state_from_name(&state_raw)?;
    let msg_from_raw: Option<String> = row
        .try_get("msg_from")
        .map_err(|e| decode_err("msg_from", &e))?;
    let msg_to_raw: Option<String> = row
        .try_get("msg_to")
        .map_err(|e| decode_err("msg_to", &e))?;
    let session_id: Option<String> = row
        .try_get("session_id")
        .map_err(|e| decode_err("session_id", &e))?;
    let sort_key: i64 = row
        .try_get("sort_key")
        .map_err(|e| decode_err("sort_key", &e))?;
    let tags_json: String = row
        .try_get("tags_json")
        .map_err(|e| decode_err("tags_json", &e))?;
    let ingress_json: Option<String> = row
        .try_get("ingress_json")
        .map_err(|e| decode_err("ingress_json", &e))?;
    let created_at_ms: i64 = row
        .try_get("created_at_ms")
        .map_err(|e| decode_err("created_at_ms", &e))?;
    let updated_at_ms: i64 = row
        .try_get("updated_at_ms")
        .map_err(|e| decode_err("updated_at_ms", &e))?;

    let from = parse_record_did(msg_from_raw.as_deref(), &owner);
    let to_fallback = match box_kind {
        MailboxKind::Inbox | MailboxKind::GroupInbox | MailboxKind::RequestBox => owner.clone(),
        MailboxKind::Sent => from.clone(),
    };
    let to = parse_record_did(msg_to_raw.as_deref(), &to_fallback);
    let tags: Vec<String> = parse_json(&tags_json, &record_id, "tags_json")?;
    let ingress: Option<IngressContext> =
        parse_optional_json(ingress_json.as_deref(), &record_id, "ingress_json")?;

    Ok(MailboxRecord {
        record_id: record_id.clone(),
        owner,
        box_kind,
        msg_id,
        msg_kind,
        state,
        from,
        from_name: None,
        to,
        session_id,
        sort_key: from_sql_i64(sort_key, "sort_key", &record_id)?,
        tags,
        ingress,
        created_at_ms: from_sql_i64(created_at_ms, "created_at_ms", &record_id)?,
        updated_at_ms: from_sql_i64(updated_at_ms, "updated_at_ms", &record_id)?,
    })
}

fn row_to_delivery_record(row: &AnyRow) -> std::result::Result<DeliveryRecord, RPCErrors> {
    let delivery_id: String = row
        .try_get("delivery_id")
        .map_err(|e| decode_err("delivery_id", &e))?;
    let envelope_json: String = row
        .try_get("envelope_json")
        .map_err(|e| decode_err("envelope_json", &e))?;
    let envelope: DeliveryEnvelope = parse_json(&envelope_json, &delivery_id, "envelope_json")?;
    let state_raw: String = row.try_get("state").map_err(|e| decode_err("state", &e))?;
    let state = delivery_state_from_name(&state_raw)?;
    let attempts: i64 = row
        .try_get("attempts")
        .map_err(|e| decode_err("attempts", &e))?;
    let next_retry_at_ms: Option<i64> = row
        .try_get("next_retry_at_ms")
        .map_err(|e| decode_err("next_retry_at_ms", &e))?;
    let external_msg_id: Option<String> = row
        .try_get("external_msg_id")
        .map_err(|e| decode_err("external_msg_id", &e))?;
    let delivered_at_ms: Option<i64> = row
        .try_get("delivered_at_ms")
        .map_err(|e| decode_err("delivered_at_ms", &e))?;
    let last_error_json: Option<String> = row
        .try_get("last_error_json")
        .map_err(|e| decode_err("last_error_json", &e))?;
    let created_at_ms: i64 = row
        .try_get("created_at_ms")
        .map_err(|e| decode_err("created_at_ms", &e))?;
    let updated_at_ms: i64 = row
        .try_get("updated_at_ms")
        .map_err(|e| decode_err("updated_at_ms", &e))?;

    let last_error: Option<DeliveryError> =
        parse_optional_json(last_error_json.as_deref(), &delivery_id, "last_error_json")?;

    Ok(DeliveryRecord {
        delivery_id: delivery_id.clone(),
        envelope,
        state,
        attempts: attempts.clamp(0, u32::MAX as i64) as u32,
        next_retry_at_ms: next_retry_at_ms
            .map(|v| from_sql_i64(v, "next_retry_at_ms", &delivery_id))
            .transpose()?,
        external_msg_id,
        delivered_at_ms: delivered_at_ms
            .map(|v| from_sql_i64(v, "delivered_at_ms", &delivery_id))
            .transpose()?,
        last_error,
        created_at_ms: from_sql_i64(created_at_ms, "created_at_ms", &delivery_id)?,
        updated_at_ms: from_sql_i64(updated_at_ms, "updated_at_ms", &delivery_id)?,
    })
}

fn parse_record_did(raw: Option<&str>, fallback: &DID) -> DID {
    let parsed = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            DID::from_str(value).ok().or_else(|| {
                serde_json::from_str::<Vec<String>>(value)
                    .ok()
                    .and_then(|values| {
                        values.into_iter().find_map(|entry| {
                            let normalized = entry.trim();
                            if normalized.is_empty() {
                                None
                            } else {
                                DID::from_str(normalized).ok()
                            }
                        })
                    })
            })
        });

    parsed.unwrap_or_else(|| fallback.clone())
}

fn parse_obj_id(raw: &str, record_id: &str) -> std::result::Result<ObjId, RPCErrors> {
    serde_json::from_value::<ObjId>(Value::String(raw.to_string())).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "invalid msg_id for record {}: {}",
            record_id, error
        ))
    })
}

fn msg_obj_kind_name(kind: &ndn_lib::MsgObjKind) -> &'static str {
    match kind {
        ndn_lib::MsgObjKind::Chat => "chat",
        ndn_lib::MsgObjKind::GroupMsg => "group_msg",
        ndn_lib::MsgObjKind::Deliver => "deliver",
        ndn_lib::MsgObjKind::Notify => "notify",
        ndn_lib::MsgObjKind::Event => "event",
        ndn_lib::MsgObjKind::Operation => "operation",
    }
}

fn parse_msg_obj_kind(raw: Option<&str>, box_kind: &MailboxKind) -> ndn_lib::MsgObjKind {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    match normalized.as_deref() {
        Some("chat") => ndn_lib::MsgObjKind::Chat,
        Some("group_msg") => ndn_lib::MsgObjKind::GroupMsg,
        Some("deliver") => ndn_lib::MsgObjKind::Deliver,
        Some("notify") => ndn_lib::MsgObjKind::Notify,
        Some("event") => ndn_lib::MsgObjKind::Event,
        Some("operation") => ndn_lib::MsgObjKind::Operation,
        _ => match box_kind {
            MailboxKind::GroupInbox => ndn_lib::MsgObjKind::GroupMsg,
            _ => ndn_lib::MsgObjKind::Chat,
        },
    }
}

fn parse_json<T: DeserializeOwned>(
    raw: &str,
    record_id: &str,
    field: &str,
) -> std::result::Result<T, RPCErrors> {
    serde_json::from_str(raw).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "failed to parse {} for record {}: {}",
            field, record_id, error
        ))
    })
}

fn parse_optional_json<T: DeserializeOwned>(
    raw: Option<&str>,
    record_id: &str,
    field: &str,
) -> std::result::Result<Option<T>, RPCErrors> {
    match raw {
        Some(raw) => parse_json(raw, record_id, field).map(Some),
        None => Ok(None),
    }
}

fn encode_optional_json<T: serde::Serialize>(
    value: Option<&T>,
    record_id: &str,
    field: &str,
) -> std::result::Result<Option<String>, RPCErrors> {
    match value {
        Some(value) => serde_json::to_string(value).map(Some).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode {} for record {}: {}",
                field, record_id, error
            ))
        }),
        None => Ok(None),
    }
}

pub(crate) fn mailbox_kind_name(box_kind: &MailboxKind) -> &'static str {
    match box_kind {
        MailboxKind::Inbox => "INBOX",
        MailboxKind::Sent => "SENT",
        MailboxKind::GroupInbox => "GROUP_INBOX",
        MailboxKind::RequestBox => "REQUEST_BOX",
    }
}

fn mailbox_kind_from_name(raw: &str) -> std::result::Result<MailboxKind, RPCErrors> {
    match raw {
        "INBOX" => Ok(MailboxKind::Inbox),
        "SENT" => Ok(MailboxKind::Sent),
        "GROUP_INBOX" => Ok(MailboxKind::GroupInbox),
        "REQUEST_BOX" => Ok(MailboxKind::RequestBox),
        _ => Err(RPCErrors::ReasonError(format!(
            "invalid mailbox kind '{}', expected one of INBOX/SENT/GROUP_INBOX/REQUEST_BOX",
            raw
        ))),
    }
}

fn recipient_state_name(state: &RecipientState) -> &'static str {
    match state {
        RecipientState::Unread => "UNREAD",
        RecipientState::Reading => "READING",
        RecipientState::Read => "READ",
        RecipientState::Archived => "ARCHIVED",
        RecipientState::Deleted => "DELETED",
    }
}

fn recipient_state_from_name(raw: &str) -> std::result::Result<RecipientState, RPCErrors> {
    match raw {
        "UNREAD" => Ok(RecipientState::Unread),
        "READING" => Ok(RecipientState::Reading),
        "READ" => Ok(RecipientState::Read),
        "ARCHIVED" => Ok(RecipientState::Archived),
        "DELETED" => Ok(RecipientState::Deleted),
        _ => Err(RPCErrors::ReasonError(format!(
            "invalid recipient state '{}', expected UNREAD/READING/READ/ARCHIVED/DELETED",
            raw
        ))),
    }
}

fn delivery_state_name(state: &DeliveryState) -> &'static str {
    match state {
        DeliveryState::Wait => "WAIT",
        DeliveryState::Sending => "SENDING",
        DeliveryState::Sent => "SENT",
        DeliveryState::Failed => "FAILED",
        DeliveryState::Dead => "DEAD",
    }
}

fn delivery_state_from_name(raw: &str) -> std::result::Result<DeliveryState, RPCErrors> {
    match raw {
        "WAIT" => Ok(DeliveryState::Wait),
        "SENDING" => Ok(DeliveryState::Sending),
        "SENT" => Ok(DeliveryState::Sent),
        "FAILED" => Ok(DeliveryState::Failed),
        "DEAD" => Ok(DeliveryState::Dead),
        _ => Err(RPCErrors::ReasonError(format!(
            "invalid delivery state '{}', expected WAIT/SENDING/SENT/FAILED/DEAD",
            raw
        ))),
    }
}

fn recipient_state_matches(filter: Option<&[RecipientState]>, state: &RecipientState) -> bool {
    match filter {
        None => true,
        Some(filters) if filters.is_empty() => true,
        Some(filters) => filters.iter().any(|item| item == state),
    }
}

fn to_sql_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn from_sql_i64(value: i64, field: &str, record_id: &str) -> std::result::Result<u64, RPCErrors> {
    if value < 0 {
        return Err(RPCErrors::ReasonError(format!(
            "invalid {} for record {}: negative value {}",
            field, record_id, value
        )));
    }
    Ok(value as u64)
}

fn rewrite_placeholders_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut idx = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    for ch in sql.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '?' if !in_single && !in_double => {
                idx += 1;
                out.push('$');
                out.push_str(&idx.to_string());
            }
            _ => out.push(ch),
        }
    }
    out
}

fn split_sql_statements(ddl: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for ch in ddl.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                buf.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                buf.push(ch);
            }
            ';' if !in_single && !in_double => {
                let trimmed = buf.trim();
                if !trimmed.is_empty() {
                    stmts.push(trimmed.to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        stmts.push(trimmed.to_string());
    }
    stmts
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::TransportKind;
    use tempfile::tempdir;

    async fn setup_test_mgr() -> (MsgBoxDbMgr, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("msg-box.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
        let mgr = MsgBoxDbMgr::open_default_sqlite(&conn).await.unwrap();
        (mgr, temp_dir)
    }

    fn sample_owner() -> DID {
        DID::from_str("did:bns:alice").unwrap()
    }

    fn sample_msg_id(seed: char) -> ObjId {
        serde_json::from_value(Value::String(format!(
            "mobjchat:{}",
            seed.to_string().repeat(40)
        )))
        .unwrap()
    }

    fn sample_record(owner: &DID, box_kind: MailboxKind, suffix: &str) -> MailboxRecord {
        let record_id = format!(
            "{}|{}|{}",
            owner.to_string(),
            mailbox_kind_name(&box_kind),
            suffix
        );
        MailboxRecord {
            record_id,
            owner: owner.clone(),
            box_kind,
            msg_id: sample_msg_id('a'),
            msg_kind: ndn_lib::MsgObjKind::Chat,
            state: RecipientState::Unread,
            from: owner.clone(),
            from_name: None,
            to: owner.clone(),
            session_id: Some("topic-1".to_string()),
            sort_key: 1_700_000_000_000,
            tags: vec!["tag".to_string()],
            ingress: None,
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
        }
    }

    fn sample_delivery(transport: &DID, target: &DID, seed: char) -> DeliveryRecord {
        let msg_id = sample_msg_id(seed);
        DeliveryRecord {
            delivery_id: format!("dlv-{}-{}", seed, target.to_string()),
            envelope: DeliveryEnvelope {
                msg_id,
                target_did: target.clone(),
                transport_did: transport.clone(),
                transport: TransportKind::Tunnel {
                    platform: "telegram".to_string(),
                    tunnel_instance_id: "tg-main-tunnel".to_string(),
                },
                address: None,
            },
            state: DeliveryState::Wait,
            attempts: 0,
            next_retry_at_ms: None,
            external_msg_id: None,
            delivered_at_ms: None,
            last_error: None,
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn open_and_upsert_record_roundtrip() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let owner = sample_owner();
        let record = sample_record(&owner, MailboxKind::Inbox, "one");
        mgr.upsert_record(&record).await.unwrap();
        let got = mgr.get_record(&owner, &record.record_id).await.unwrap();
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.record_id, record.record_id);
        assert_eq!(got.box_kind, MailboxKind::Inbox);
        assert_eq!(got.state, RecipientState::Unread);
        assert_eq!(got.session_id.as_deref(), Some("topic-1"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_records_and_state_filter() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let owner = sample_owner();
        let r1 = sample_record(&owner, MailboxKind::Inbox, "one");
        let mut r2 = sample_record(&owner, MailboxKind::Inbox, "two");
        r2.state = RecipientState::Read;
        mgr.upsert_record(&r1).await.unwrap();
        mgr.upsert_record(&r2).await.unwrap();
        let all = mgr
            .list_records(&owner, &MailboxKind::Inbox, None, true)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        let unread = mgr
            .list_records(
                &owner,
                &MailboxKind::Inbox,
                Some(&[RecipientState::Unread]),
                true,
            )
            .await
            .unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].state, RecipientState::Unread);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_index_groups_by_session() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let owner = sample_owner();
        let mut r1 = sample_record(&owner, MailboxKind::Inbox, "one");
        r1.session_id = Some("s-a".to_string());
        let mut r2 = sample_record(&owner, MailboxKind::Sent, "two");
        r2.session_id = Some("s-a".to_string());
        r2.state = RecipientState::Read;
        r2.updated_at_ms += 10;
        let mut r3 = sample_record(&owner, MailboxKind::Inbox, "three");
        r3.session_id = Some("s-b".to_string());
        r3.updated_at_ms += 20;
        for r in [&r1, &r2, &r3] {
            mgr.upsert_record(r).await.unwrap();
        }

        let index = mgr.list_session_index(&owner, 10, None, None).await.unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(index[0].session_id, "s-b");
        assert_eq!(index[1].session_id, "s-a");
        assert_eq!(index[1].unread_count, 1);

        let page = mgr
            .list_session_records(&owner, "s-a", 10, None, None, true)
            .await
            .unwrap();
        assert_eq!(page.len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delivery_take_and_reclaim() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let transport = DID::from_str("did:bns:tg-tunnel").unwrap();
        let target = DID::from_str("did:msgtunnel:1.user.tg-main-tunnel").unwrap();
        let record = sample_delivery(&transport, &target, 'c');
        mgr.create_delivery_if_absent(&record).await.unwrap();
        // Idempotent re-create keeps the row.
        mgr.create_delivery_if_absent(&record).await.unwrap();

        let taken = mgr
            .take_next_delivery(&transport, 1_700_000_000_100, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(taken.state, DeliveryState::Sending);

        // Nothing else to take.
        assert!(mgr
            .take_next_delivery(&transport, 1_700_000_000_100, true)
            .await
            .unwrap()
            .is_none());

        // Lease expiry reclaims the SENDING row back to WAIT.
        let reclaimed = mgr
            .reclaim_stale_sending(&transport, 1_700_000_000_200, 1_700_000_000_200)
            .await
            .unwrap();
        assert_eq!(reclaimed, 1);
        let again = mgr
            .take_next_delivery(&transport, 1_700_000_000_300, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again.attempts, 1);
        assert!(again.last_error.is_none() || again.last_error.unwrap().duplicate_risk);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn touch_message_and_has_message() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let owner = sample_owner();
        let msg_id = sample_msg_id('b');
        assert!(!mgr.has_message(&owner, &msg_id).await.unwrap());
        mgr.touch_message(&owner, &msg_id, 1_700_000_000_000)
            .await
            .unwrap();
        assert!(mgr.has_message(&owner, &msg_id).await.unwrap());
        // Idempotent.
        mgr.touch_message(&owner, &msg_id, 1_700_000_000_001)
            .await
            .unwrap();
    }

    #[test]
    fn rewrite_placeholders_handles_quotes() {
        assert_eq!(
            rewrite_placeholders_to_dollar("SELECT ? FROM t WHERE s = '?' AND x = ?"),
            "SELECT $1 FROM t WHERE s = '?' AND x = $2"
        );
    }

    #[test]
    fn split_sql_statements_basic() {
        let ddl = "CREATE TABLE a(x INT); CREATE TABLE b(y TEXT);";
        let stmts = split_sql_statements(ddl);
        assert_eq!(stmts.len(), 2);
    }
}
