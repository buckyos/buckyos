//! Owner-scoped session metadata storage (`Message Center.md` §5.5 / §5.8):
//!
//!   owner_sessions            lifecycle (active / archived), delete watermark
//!                             and explicit registration of empty sessions
//!   owner_ui_session_states   `(owner, session_id, key)` UI preferences
//!
//! Both tables are keyed by `owner`, so the same `session_id` used by two
//! owners never shares state. The session projection (`list_sessions` /
//! `list_session`) applies the watermark from `owner_sessions` so records the
//! owner deleted stay hidden even when the same message is replayed.

use crate::msg_box_db::{
    decode_err, decode_ui_session_state_row, row_to_mailbox_record, to_sql_i64, MsgBoxDbMgr,
};
use buckyos_api::{MailboxRecord, OwnerSessionState, SessionLifecycle, UiSessionStateEntry};
use kRPC::RPCErrors;
use name_lib::DID;
use serde_json::Value;
use sqlx::any::AnyRow;
use sqlx::Row;

/// Per-session aggregate of the visible (post-watermark) records.
#[derive(Debug, Clone)]
pub struct VisibleSessionIndexEntry {
    pub session_id: String,
    pub updated_at_ms: u64,
    pub last_activity_ms: u64,
    pub unread_count: u64,
    pub request_count: u64,
}

/// Delete watermark: records at or before `(sort_key, record_id)` are hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteWatermark {
    pub sort_key: u64,
    pub record_id: String,
}

impl DeleteWatermark {
    pub fn from_state(state: &OwnerSessionState) -> Option<Self> {
        state.delete_watermark_sort_key.map(|sort_key| Self {
            sort_key,
            record_id: state.delete_watermark_record_id.clone().unwrap_or_default(),
        })
    }

    /// `true` when `record` is newer than the watermark and therefore visible.
    pub fn allows(&self, record: &MailboxRecord) -> bool {
        record.sort_key > self.sort_key
            || (record.sort_key == self.sort_key && record.record_id > self.record_id)
    }
}

const WATERMARK_SQL: &str = r#"(s.delete_watermark_sort_key IS NULL
    OR r.sort_key > s.delete_watermark_sort_key
    OR (r.sort_key = s.delete_watermark_sort_key AND r.record_id > COALESCE(s.delete_watermark_record_id, '')))"#;

fn lifecycle_name(lifecycle: SessionLifecycle) -> &'static str {
    match lifecycle {
        SessionLifecycle::Active => "active",
        SessionLifecycle::Archived => "archived",
    }
}

fn parse_lifecycle(raw: &str) -> SessionLifecycle {
    match raw.trim().to_ascii_lowercase().as_str() {
        "archived" => SessionLifecycle::Archived,
        _ => SessionLifecycle::Active,
    }
}

fn optional_u64(row: &AnyRow, field: &str) -> std::result::Result<Option<u64>, RPCErrors> {
    let value: Option<i64> = row.try_get(field).map_err(|e| decode_err(field, &e))?;
    Ok(value.filter(|v| *v >= 0).map(|v| v as u64))
}

fn optional_string(row: &AnyRow, field: &str) -> std::result::Result<Option<String>, RPCErrors> {
    let value: Option<String> = row.try_get(field).map_err(|e| decode_err(field, &e))?;
    Ok(value.filter(|v| !v.is_empty()))
}

fn row_to_owner_session(row: &AnyRow) -> std::result::Result<OwnerSessionState, RPCErrors> {
    let owner: String = row.try_get("owner").map_err(|e| decode_err("owner", &e))?;
    let owner = DID::from_str(&owner)
        .map_err(|e| RPCErrors::ReasonError(format!("invalid owner DID {}: {}", owner, e)))?;
    let session_id: String = row
        .try_get("session_id")
        .map_err(|e| decode_err("session_id", &e))?;
    let lifecycle: String = row
        .try_get("lifecycle")
        .map_err(|e| decode_err("lifecycle", &e))?;
    let registered: i64 = row
        .try_get("registered")
        .map_err(|e| decode_err("registered", &e))?;
    let peer_did = optional_string(row, "peer_did")?
        .map(|value| {
            DID::from_str(&value)
                .map_err(|e| RPCErrors::ReasonError(format!("invalid peer DID {}: {}", value, e)))
        })
        .transpose()?;
    let binding = optional_string(row, "binding_json")?
        .map(|value| {
            serde_json::from_str::<Value>(&value).map_err(|e| {
                RPCErrors::ReasonError(format!("invalid binding json for {}: {}", session_id, e))
            })
        })
        .transpose()?;
    let created_at_ms: i64 = row
        .try_get("created_at_ms")
        .map_err(|e| decode_err("created_at_ms", &e))?;
    let updated_at_ms: i64 = row
        .try_get("updated_at_ms")
        .map_err(|e| decode_err("updated_at_ms", &e))?;
    Ok(OwnerSessionState {
        owner,
        session_id,
        lifecycle: parse_lifecycle(&lifecycle),
        registered: registered != 0,
        origin: optional_string(row, "origin")?,
        peer_did,
        binding,
        title: optional_string(row, "title")?,
        archived_at_ms: optional_u64(row, "archived_at_ms")?,
        delete_watermark_sort_key: optional_u64(row, "delete_watermark_sort_key")?,
        delete_watermark_record_id: optional_string(row, "delete_watermark_record_id")?,
        deleted_at_ms: optional_u64(row, "deleted_at_ms")?,
        created_at_ms: created_at_ms.max(0) as u64,
        updated_at_ms: updated_at_ms.max(0) as u64,
    })
}

const OWNER_SESSION_COLUMNS: &str = r#"
    owner, session_id, lifecycle, registered, origin, peer_did, binding_json, title,
    archived_at_ms, delete_watermark_sort_key, delete_watermark_record_id, deleted_at_ms,
    created_at_ms, updated_at_ms
"#;

impl MsgBoxDbMgr {
    pub async fn get_owner_session(
        &self,
        owner: &DID,
        session_id: &str,
    ) -> std::result::Result<Option<OwnerSessionState>, RPCErrors> {
        let sql = self.render_sql(&format!(
            "SELECT {OWNER_SESSION_COLUMNS} FROM owner_sessions WHERE owner = ? AND session_id = ?"
        ));
        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query owner session {}/{}: {}",
                    owner.to_string(),
                    session_id,
                    error
                ))
            })?;
        row.as_ref().map(row_to_owner_session).transpose()
    }

    pub async fn list_owner_sessions(
        &self,
        owner: &DID,
    ) -> std::result::Result<Vec<OwnerSessionState>, RPCErrors> {
        let sql = self.render_sql(&format!(
            "SELECT {OWNER_SESSION_COLUMNS} FROM owner_sessions WHERE owner = ? ORDER BY session_id ASC"
        ));
        let rows = sqlx::query(&sql)
            .bind(owner.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to list owner sessions of {}: {}",
                    owner.to_string(),
                    error
                ))
            })?;
        rows.iter().map(row_to_owner_session).collect()
    }

    pub async fn upsert_owner_session(
        &self,
        state: &OwnerSessionState,
    ) -> std::result::Result<(), RPCErrors> {
        let binding_json = state
            .binding
            .as_ref()
            .map(|value| {
                serde_json::to_string(value).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "failed to encode binding of session {}: {}",
                        state.session_id, error
                    ))
                })
            })
            .transpose()?;
        let sql = self.render_sql(&format!(
            r#"
INSERT INTO owner_sessions ({OWNER_SESSION_COLUMNS})
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(owner, session_id) DO UPDATE SET
    lifecycle = excluded.lifecycle,
    registered = excluded.registered,
    origin = excluded.origin,
    peer_did = excluded.peer_did,
    binding_json = excluded.binding_json,
    title = excluded.title,
    archived_at_ms = excluded.archived_at_ms,
    delete_watermark_sort_key = excluded.delete_watermark_sort_key,
    delete_watermark_record_id = excluded.delete_watermark_record_id,
    deleted_at_ms = excluded.deleted_at_ms,
    created_at_ms = owner_sessions.created_at_ms,
    updated_at_ms = excluded.updated_at_ms
"#
        ));
        sqlx::query(&sql)
            .bind(state.owner.to_string())
            .bind(state.session_id.clone())
            .bind(lifecycle_name(state.lifecycle).to_string())
            .bind(if state.registered { 1i64 } else { 0i64 })
            .bind(state.origin.clone())
            .bind(state.peer_did.as_ref().map(|did| did.to_string()))
            .bind(binding_json)
            .bind(state.title.clone())
            .bind(state.archived_at_ms.map(to_sql_i64))
            .bind(state.delete_watermark_sort_key.map(to_sql_i64))
            .bind(state.delete_watermark_record_id.clone())
            .bind(state.deleted_at_ms.map(to_sql_i64))
            .bind(to_sql_i64(state.created_at_ms))
            .bind(to_sql_i64(state.updated_at_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to upsert owner session {}/{}: {}",
                    state.owner.to_string(),
                    state.session_id,
                    error
                ))
            })?;
        Ok(())
    }

    /// Newest visible record key of one session, used to place the delete
    /// watermark after every record that exists at deletion time.
    pub async fn session_max_record_key(
        &self,
        owner: &DID,
        session_id: &str,
    ) -> std::result::Result<Option<(u64, String)>, RPCErrors> {
        let sql = self.render_sql(
            r#"
SELECT sort_key, record_id FROM mailbox_records
WHERE owner = ? AND session_id = ?
ORDER BY sort_key DESC, record_id DESC
LIMIT 1
"#,
        );
        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query max record of session {}: {}",
                    session_id, error
                ))
            })?;
        row.map(|row| {
            let sort_key: i64 = row
                .try_get("sort_key")
                .map_err(|e| decode_err("sort_key", &e))?;
            let record_id: String = row
                .try_get("record_id")
                .map_err(|e| decode_err("record_id", &e))?;
            Ok((sort_key.max(0) as u64, record_id))
        })
        .transpose()
    }

    /// Session aggregates of every visible record (after the owner's delete
    /// watermark). Ordering / cursor / lifecycle filtering are applied by the
    /// caller after merging with registered empty sessions.
    pub async fn list_visible_session_index(
        &self,
        owner: &DID,
    ) -> std::result::Result<Vec<VisibleSessionIndexEntry>, RPCErrors> {
        let sql = self.render_sql(&format!(
            r#"
SELECT
    r.session_id AS session_id,
    MAX(r.updated_at_ms) AS last_updated_at_ms,
    MAX(CASE WHEN r.msg_kind IN ('chat', 'group_msg', 'deliver') THEN r.sort_key ELSE 0 END) AS last_activity_ms,
    SUM(CASE WHEN r.state = 'UNREAD' THEN 1 ELSE 0 END) AS unread_count,
    SUM(CASE WHEN r.box_kind = 'REQUEST_BOX' THEN 1 ELSE 0 END) AS request_count
FROM mailbox_records r
LEFT JOIN owner_sessions s ON s.owner = r.owner AND s.session_id = r.session_id
WHERE r.owner = ? AND r.session_id IS NOT NULL AND r.state != 'DELETED'
  AND {WATERMARK_SQL}
GROUP BY r.session_id
"#
        ));
        let rows = sqlx::query(&sql)
            .bind(owner.to_string())
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
            let last_activity_ms: i64 = row
                .try_get("last_activity_ms")
                .map_err(|e| decode_err("last_activity_ms", &e))?;
            let unread_count: i64 = row
                .try_get("unread_count")
                .map_err(|e| decode_err("unread_count", &e))?;
            let request_count: i64 = row
                .try_get("request_count")
                .map_err(|e| decode_err("request_count", &e))?;
            entries.push(VisibleSessionIndexEntry {
                session_id,
                updated_at_ms: updated_at_ms.max(0) as u64,
                last_activity_ms: last_activity_ms.max(0) as u64,
                unread_count: unread_count.max(0) as u64,
                request_count: request_count.max(0) as u64,
            });
        }
        Ok(entries)
    }

    /// `list_session_records` restricted to records newer than the owner's
    /// delete watermark.
    pub async fn list_visible_session_records(
        &self,
        owner: &DID,
        session_id: &str,
        limit: usize,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<&str>,
        descending: bool,
        watermark: Option<&DeleteWatermark>,
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
        let watermark_sql = if watermark.is_some() {
            "AND (sort_key > ? OR (sort_key = ? AND record_id > ?))"
        } else {
            ""
        };
        let sql = self.render_sql(&format!(
            r#"
SELECT {} FROM mailbox_records
WHERE owner = ? AND session_id = ? AND state != 'DELETED'
{cursor_sql}
{watermark_sql}
{order_clause}
LIMIT ?
"#,
            crate::msg_box_db::MAILBOX_COLUMNS
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
        if let Some(watermark) = watermark {
            query = query
                .bind(to_sql_i64(watermark.sort_key))
                .bind(to_sql_i64(watermark.sort_key))
                .bind(watermark.record_id.clone());
        }
        let rows = query
            .bind(limit as i64)
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to list visible session records for {}: {}",
                    session_id, error
                ))
            })?;
        rows.iter().map(row_to_mailbox_record).collect()
    }

    // ------------------------------------------------------------------
    // Owner-scoped UI session state
    // ------------------------------------------------------------------

    pub async fn upsert_owner_ui_session_state(
        &self,
        owner: &DID,
        session_id: &str,
        key: &str,
        value: &Value,
        updated_at_ms: u64,
    ) -> std::result::Result<UiSessionStateEntry, RPCErrors> {
        let value_json = serde_json::to_string(value).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode owner ui session state {}:{}: {}",
                session_id, key, error
            ))
        })?;
        let sql = self.render_sql(
            r#"
INSERT INTO owner_ui_session_states (owner, session_id, state_key, value_json, updated_at_ms)
VALUES (?, ?, ?, ?, ?)
ON CONFLICT(owner, session_id, state_key) DO UPDATE SET
    value_json = excluded.value_json,
    updated_at_ms = excluded.updated_at_ms
"#,
        );
        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session_id.to_string())
            .bind(key.to_string())
            .bind(value_json)
            .bind(to_sql_i64(updated_at_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to upsert owner ui session state {}:{}: {}",
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

    pub async fn get_owner_ui_session_state(
        &self,
        owner: &DID,
        session_id: &str,
        key: &str,
    ) -> std::result::Result<Option<UiSessionStateEntry>, RPCErrors> {
        let sql = self.render_sql(
            r#"
SELECT session_id, state_key, value_json, updated_at_ms
FROM owner_ui_session_states
WHERE owner = ? AND session_id = ? AND state_key = ?
"#,
        );
        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session_id.to_string())
            .bind(key.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query owner ui session state {}:{}: {}",
                    session_id, key, error
                ))
            })?;
        row.as_ref().map(decode_ui_session_state_row).transpose()
    }

    pub async fn list_owner_ui_session_state(
        &self,
        owner: &DID,
        session_id: &str,
    ) -> std::result::Result<Vec<UiSessionStateEntry>, RPCErrors> {
        let sql = self.render_sql(
            r#"
SELECT session_id, state_key, value_json, updated_at_ms
FROM owner_ui_session_states
WHERE owner = ? AND session_id = ?
ORDER BY state_key ASC
"#,
        );
        let rows = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session_id.to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to list owner ui session state {}: {}",
                    session_id, error
                ))
            })?;
        rows.iter().map(decode_ui_session_state_row).collect()
    }

    pub async fn delete_owner_ui_session_states(
        &self,
        owner: &DID,
        session_id: &str,
    ) -> std::result::Result<(), RPCErrors> {
        let sql = self
            .render_sql("DELETE FROM owner_ui_session_states WHERE owner = ? AND session_id = ?");
        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(session_id.to_string())
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to delete owner ui session state {}: {}",
                    session_id, error
                ))
            })?;
        Ok(())
    }
}
