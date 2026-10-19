//! Share tables in filedb (design §3.3). Credentials stay in `grants`
//! (`cap_id`, `token_hash`, `expires_at`, `revoked`); everything else about a
//! share — the result record, its immutable content version, link policy,
//! retention pins, app deliveries, audit and access statistics — lives here.

use crate::error::{NfsError, NfsResult};
use crate::filedb::FileDb;
use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS share_operations(
  operation_id TEXT PRIMARY KEY,
  intent TEXT NOT NULL,
  owner_user_id TEXT NOT NULL,
  actor_user_id TEXT NOT NULL,
  actor_app_id TEXT,
  input_json TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  task_id TEXT,
  share_id TEXT,
  state TEXT NOT NULL,
  error_json TEXT,
  progress_json TEXT,
  result_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(owner_user_id, idempotency_key)
);
CREATE TABLE IF NOT EXISTS share_versions(
  obj_id TEXT PRIMARY KEY,
  obj_type TEXT NOT NULL,
  root_name TEXT NOT NULL,
  mime TEXT,
  total_size INTEGER NOT NULL,
  item_count INTEGER NOT NULL,
  source_path TEXT NOT NULL,
  source_mtime INTEGER,
  source_size INTEGER,
  excluded_json TEXT,
  prepared_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS shares(
  share_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  owner_user_id TEXT NOT NULL,
  created_by_app TEXT,
  title TEXT NOT NULL,
  obj_id TEXT NOT NULL REFERENCES share_versions(obj_id),
  source_path TEXT NOT NULL,
  cap_id TEXT,
  related_share_id TEXT,
  status TEXT NOT NULL,
  status_reason TEXT,
  hidden INTEGER NOT NULL DEFAULT 0,
  revision INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  stopped_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_shares_owner ON shares(owner_user_id, status);
CREATE INDEX IF NOT EXISTS idx_shares_source ON shares(source_path);
CREATE TABLE IF NOT EXISTS share_link_policy(
  share_id TEXT PRIMARY KEY REFERENCES shares(share_id),
  access TEXT NOT NULL,
  password_hash TEXT,
  password_rev INTEGER NOT NULL DEFAULT 0,
  allow_download INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS share_retentions(
  share_id TEXT NOT NULL REFERENCES shares(share_id),
  obj_id TEXT NOT NULL,
  pin_owner TEXT NOT NULL,
  pinned_at INTEGER NOT NULL,
  released_at INTEGER,
  PRIMARY KEY(share_id, obj_id)
);
CREATE TABLE IF NOT EXISTS share_deliveries(
  share_id TEXT PRIMARY KEY REFERENCES shares(share_id),
  target_app TEXT NOT NULL,
  handoff_at INTEGER NOT NULL,
  delivery_state TEXT NOT NULL,
  business_ref_json TEXT,
  reported_at INTEGER,
  error TEXT
);
CREATE TABLE IF NOT EXISTS share_audit(
  audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
  share_id TEXT NOT NULL,
  action TEXT NOT NULL,
  revision_before INTEGER,
  revision_after INTEGER,
  actor_user_id TEXT NOT NULL,
  actor_app_id TEXT,
  source TEXT NOT NULL,
  reason TEXT,
  outcome TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_share_audit ON share_audit(share_id, audit_id);
CREATE TABLE IF NOT EXISTS share_access_log(
  log_id INTEGER PRIMARY KEY AUTOINCREMENT,
  share_id TEXT NOT NULL,
  at INTEGER NOT NULL,
  kind TEXT NOT NULL,
  status INTEGER NOT NULL,
  bytes INTEGER NOT NULL DEFAULT 0,
  visitor TEXT,
  inner_path TEXT
);
CREATE INDEX IF NOT EXISTS idx_access_share_at ON share_access_log(share_id, at);
CREATE INDEX IF NOT EXISTS idx_access_visitor ON share_access_log(share_id, visitor, at);
CREATE TABLE IF NOT EXISTS share_access_stats(
  share_id TEXT NOT NULL,
  bucket INTEGER NOT NULL,
  views INTEGER NOT NULL DEFAULT 0,
  downloads INTEGER NOT NULL DEFAULT 0,
  bytes INTEGER NOT NULL DEFAULT 0,
  visitors INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(share_id, bucket)
);
CREATE TABLE IF NOT EXISTS public_observations(
  rel_path TEXT PRIMARY KEY,
  entry_kind TEXT NOT NULL,
  link_target TEXT,
  link_ok INTEGER,
  size INTEGER,
  mtime INTEGER,
  first_seen_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  missing_since INTEGER
);
"#;

#[derive(Debug, Clone)]
pub struct ShareRow {
    pub share_id: String,
    pub kind: String,
    pub owner_user_id: String,
    pub created_by_app: Option<String>,
    pub title: String,
    pub obj_id: String,
    pub source_path: String,
    pub cap_id: Option<String>,
    pub related_share_id: Option<String>,
    pub status: String,
    pub status_reason: Option<String>,
    pub hidden: bool,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub stopped_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct VersionRow {
    pub obj_id: String,
    pub obj_type: String,
    pub root_name: String,
    pub mime: Option<String>,
    pub total_size: i64,
    pub item_count: i64,
    pub source_path: String,
    pub source_mtime: Option<i64>,
    pub source_size: Option<i64>,
    pub excluded: Value,
    pub prepared_at: i64,
}

#[derive(Debug, Clone)]
pub struct PolicyRow {
    pub share_id: String,
    pub access: String,
    pub password_hash: Option<String>,
    pub password_rev: i64,
    pub allow_download: bool,
}

#[derive(Debug, Clone)]
pub struct OperationRow {
    pub operation_id: String,
    pub intent: String,
    pub owner_user_id: String,
    pub actor_user_id: String,
    pub actor_app_id: Option<String>,
    pub input: Value,
    pub idempotency_key: String,
    pub task_id: Option<String>,
    pub share_id: Option<String>,
    pub state: String,
    pub error: Option<Value>,
    pub progress: Option<Value>,
    pub result: Option<Value>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct DeliveryRow {
    pub share_id: String,
    pub target_app: String,
    pub handoff_at: i64,
    pub delivery_state: String,
    pub business_ref: Option<Value>,
    pub reported_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RetentionRow {
    pub obj_id: String,
    pub pin_owner: String,
    pub pinned_at: i64,
    pub released_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CapRow {
    pub cap_id: String,
    pub subtree: String,
    pub expires_at: Option<i64>,
    pub revoked: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditRow {
    pub action: String,
    pub revision_before: Option<i64>,
    pub revision_after: Option<i64>,
    pub actor_user_id: String,
    pub actor_app_id: Option<String>,
    pub source: String,
    pub reason: Option<String>,
    pub outcome: String,
    pub at: i64,
}

/// One audit entry (who did what to which share, with which result).
pub struct AuditEntry<'a> {
    pub share_id: &'a str,
    pub action: &'a str,
    pub revision_before: Option<i64>,
    pub revision_after: Option<i64>,
    pub actor_user_id: &'a str,
    pub actor_app_id: Option<&'a str>,
    pub source: &'a str,
    pub reason: Option<&'a str>,
    pub outcome: &'a str,
    pub at: i64,
}

/// Everything `share.create` / `share.app.handoff` writes for one share, in one transaction.
pub struct NewShare<'a> {
    pub share: &'a ShareRow,
    /// (cap_id, token_hash, subtree, expires_at) for link shares.
    pub grant: Option<(&'a str, &'a str, &'a str, Option<i64>)>,
    pub policy: Option<&'a PolicyRow>,
    pub delivery: Option<&'a DeliveryRow>,
    pub pin_owner: &'a str,
    pub audit: AuditEntry<'a>,
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    /// None = every owner (admin listings only).
    pub owner: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub source_path: Option<String>,
    pub obj_id: Option<String>,
    pub q: Option<String>,
    pub include_hidden: bool,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StatsPoint {
    pub t: i64,
    pub views: i64,
    pub downloads: i64,
    pub bytes: i64,
    pub visitors: i64,
}

/// A data-plane event waiting in the stats buffer (stats.rs).
#[derive(Debug, Clone)]
pub struct AccessEvent {
    pub share_id: String,
    pub at: i64,
    pub kind: &'static str,
    pub status: u16,
    pub bytes: u64,
    pub visitor: Option<String>,
    pub inner_path: Option<String>,
    /// Counts as a download: an attachment response from the start of the file.
    pub download_start: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PublicObservation {
    pub rel_path: String,
    pub entry_kind: String,
    pub link_target: Option<String>,
    pub link_ok: Option<bool>,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub missing_since: Option<i64>,
}

const SHARE_COLS: &str = "share_id, kind, owner_user_id, created_by_app, title, obj_id, source_path, cap_id, related_share_id, status, status_reason, hidden, revision, created_at, updated_at, stopped_at";
const OP_COLS: &str = "operation_id, intent, owner_user_id, actor_user_id, actor_app_id, input_json, idempotency_key, task_id, share_id, state, error_json, progress_json, result_json, created_at, updated_at";

fn share_row(r: &Row) -> rusqlite::Result<ShareRow> {
    Ok(ShareRow {
        share_id: r.get(0)?,
        kind: r.get(1)?,
        owner_user_id: r.get(2)?,
        created_by_app: r.get(3)?,
        title: r.get(4)?,
        obj_id: r.get(5)?,
        source_path: r.get(6)?,
        cap_id: r.get(7)?,
        related_share_id: r.get(8)?,
        status: r.get(9)?,
        status_reason: r.get(10)?,
        hidden: r.get::<_, i64>(11)? != 0,
        revision: r.get(12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
        stopped_at: r.get(15)?,
    })
}

fn json_col(r: &Row, i: usize) -> rusqlite::Result<Option<Value>> {
    Ok(r.get::<_, Option<String>>(i)?.and_then(|s| serde_json::from_str(&s).ok()))
}

fn op_row(r: &Row) -> rusqlite::Result<OperationRow> {
    Ok(OperationRow {
        operation_id: r.get(0)?,
        intent: r.get(1)?,
        owner_user_id: r.get(2)?,
        actor_user_id: r.get(3)?,
        actor_app_id: r.get(4)?,
        input: json_col(r, 5)?.unwrap_or(Value::Null),
        idempotency_key: r.get(6)?,
        task_id: r.get(7)?,
        share_id: r.get(8)?,
        state: r.get(9)?,
        error: json_col(r, 10)?,
        progress: json_col(r, 11)?,
        result: json_col(r, 12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
    })
}

pub(crate) fn insert_audit(conn: &rusqlite::Connection, a: &AuditEntry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO share_audit(share_id, action, revision_before, revision_after, actor_user_id, actor_app_id, source, reason, outcome, at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![a.share_id, a.action, a.revision_before, a.revision_after, a.actor_user_id, a.actor_app_id, a.source, a.reason, a.outcome, a.at],
    )?;
    Ok(())
}

pub fn hour_bucket(at: i64) -> i64 {
    at - at.rem_euclid(3600)
}

pub fn day_start(at: i64) -> i64 {
    at - at.rem_euclid(86400)
}

impl FileDb {
    // ---------- versions ----------

    pub fn share_version_get(&self, obj_id: &str) -> NfsResult<Option<VersionRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT obj_id, obj_type, root_name, mime, total_size, item_count, source_path, source_mtime, source_size, excluded_json, prepared_at
                 FROM share_versions WHERE obj_id=?1",
                params![obj_id],
                |r| {
                    Ok(VersionRow {
                        obj_id: r.get(0)?,
                        obj_type: r.get(1)?,
                        root_name: r.get(2)?,
                        mime: r.get(3)?,
                        total_size: r.get(4)?,
                        item_count: r.get(5)?,
                        source_path: r.get(6)?,
                        source_mtime: r.get(7)?,
                        source_size: r.get(8)?,
                        excluded: json_col(r, 9)?.unwrap_or(Value::Array(vec![])),
                        prepared_at: r.get(10)?,
                    })
                },
            )
            .optional()?)
    }

    /// First snapshot of an object wins: the same obj_id is the same content.
    pub fn share_version_put(&self, v: &VersionRow) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR IGNORE INTO share_versions(obj_id, obj_type, root_name, mime, total_size, item_count, source_path, source_mtime, source_size, excluded_json, prepared_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                v.obj_id, v.obj_type, v.root_name, v.mime, v.total_size, v.item_count, v.source_path,
                v.source_mtime, v.source_size, v.excluded.to_string(), v.prepared_at
            ],
        )?;
        Ok(())
    }

    // ---------- shares ----------

    pub fn share_create(&self, n: NewShare) -> NfsResult<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let s = n.share;
        if let Some((cap_id, token_hash, subtree, expires_at)) = n.grant {
            tx.execute(
                "INSERT INTO grants(cap_id, token_hash, subtree, ops_json, audience, expires_at, max_uses, created_at)
                 VALUES(?1,?2,?3,'[\"read\",\"list\"]',NULL,?4,NULL,?5)",
                params![cap_id, token_hash, subtree, expires_at, s.created_at],
            )?;
        }
        tx.execute(
            &format!("INSERT INTO shares({SHARE_COLS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"),
            params![
                s.share_id, s.kind, s.owner_user_id, s.created_by_app, s.title, s.obj_id, s.source_path,
                s.cap_id, s.related_share_id, s.status, s.status_reason, s.hidden as i64, s.revision,
                s.created_at, s.updated_at, s.stopped_at
            ],
        )?;
        if let Some(p) = n.policy {
            tx.execute(
                "INSERT INTO share_link_policy(share_id, access, password_hash, password_rev, allow_download) VALUES(?1,?2,?3,?4,?5)",
                params![p.share_id, p.access, p.password_hash, p.password_rev, p.allow_download as i64],
            )?;
        }
        if let Some(d) = n.delivery {
            tx.execute(
                "INSERT INTO share_deliveries(share_id, target_app, handoff_at, delivery_state, business_ref_json, reported_at, error)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    d.share_id, d.target_app, d.handoff_at, d.delivery_state,
                    d.business_ref.as_ref().map(|v| v.to_string()), d.reported_at, d.error
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO share_retentions(share_id, obj_id, pin_owner, pinned_at, released_at) VALUES(?1,?2,?3,?4,NULL)",
            params![s.share_id, s.obj_id, n.pin_owner, s.created_at],
        )?;
        insert_audit(&tx, &n.audit)?;
        tx.commit()?;
        Ok(())
    }

    pub fn share_get(&self, share_id: &str) -> NfsResult<Option<ShareRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(&format!("SELECT {SHARE_COLS} FROM shares WHERE share_id=?1"), params![share_id], share_row)
            .optional()?)
    }

    pub fn share_by_cap(&self, cap_id: &str) -> NfsResult<Option<ShareRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(&format!("SELECT {SHARE_COLS} FROM shares WHERE cap_id=?1"), params![cap_id], share_row)
            .optional()?)
    }

    pub fn share_list(&self, f: &ListFilter) -> NfsResult<Vec<ShareRow>> {
        let mut sql = format!("SELECT {SHARE_COLS} FROM shares WHERE 1=1");
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        let mut push = |sql: &mut String, clause: &str, v: rusqlite::types::Value| {
            args.push(v);
            sql.push_str(&clause.replace('?', &format!("?{}", args.len())));
        };
        if let Some(o) = &f.owner {
            push(&mut sql, " AND owner_user_id=?", o.clone().into());
        }
        if let Some(k) = &f.kind {
            push(&mut sql, " AND kind=?", k.clone().into());
        }
        if let Some(st) = &f.status {
            push(&mut sql, " AND status=?", st.clone().into());
        }
        if let Some(p) = &f.source_path {
            push(&mut sql, " AND source_path=?", p.clone().into());
        }
        if let Some(o) = &f.obj_id {
            push(&mut sql, " AND obj_id=?", o.clone().into());
        }
        if let Some(q) = f.q.as_ref().filter(|q| !q.is_empty()) {
            let like = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
            push(&mut sql, " AND (title LIKE ? ESCAPE '\\'", like.clone().into());
            push(&mut sql, " OR source_path LIKE ? ESCAPE '\\')", like.into());
        }
        if !f.include_hidden {
            sql.push_str(" AND hidden=0");
        }
        sql.push_str(" ORDER BY created_at DESC, share_id DESC");
        push(&mut sql, " LIMIT ?", f.limit.into());
        push(&mut sql, " OFFSET ?", f.offset.into());
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), share_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Optimistic update: runs `apply` inside a transaction when `expected` is
    /// the current revision; Err(current row) otherwise.
    pub fn share_update<T>(
        &self,
        share_id: &str,
        expected: i64,
        apply: impl FnOnce(&rusqlite::Transaction, &ShareRow) -> NfsResult<T>,
    ) -> NfsResult<Result<T, ShareRow>> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let current = tx
            .query_row(&format!("SELECT {SHARE_COLS} FROM shares WHERE share_id=?1"), params![share_id], share_row)
            .optional()?
            .ok_or_else(|| crate::error::not_found(format!("share '{}' not found", share_id)))?;
        if current.revision != expected {
            return Ok(Err(current));
        }
        let out = apply(&tx, &current)?;
        tx.commit()?;
        Ok(Ok(out))
    }

    /// Unconditional status change (sweeper, stop completion), with an audit row.
    pub fn share_set_status(
        &self,
        share_id: &str,
        status: &str,
        reason: Option<&str>,
        bump: bool,
        audit: Option<AuditEntry>,
    ) -> NfsResult<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let now = crate::fsutil::unix_now();
        let stopped = matches!(status, "stopped" | "expired" | "failed").then_some(now);
        tx.execute(
            "UPDATE shares SET status=?2, status_reason=COALESCE(?3, status_reason), revision=revision+?4, updated_at=?5,
             stopped_at=COALESCE(stopped_at, ?6) WHERE share_id=?1",
            params![share_id, status, reason, bump as i64, now, stopped],
        )?;
        if let Some(a) = audit {
            insert_audit(&tx, &a)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn share_policy(&self, share_id: &str) -> NfsResult<Option<PolicyRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT share_id, access, password_hash, password_rev, allow_download FROM share_link_policy WHERE share_id=?1",
                params![share_id],
                |r| {
                    Ok(PolicyRow {
                        share_id: r.get(0)?,
                        access: r.get(1)?,
                        password_hash: r.get(2)?,
                        password_rev: r.get(3)?,
                        allow_download: r.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()?)
    }

    pub fn share_cap(&self, cap_id: &str) -> NfsResult<Option<CapRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT cap_id, subtree, expires_at, revoked FROM grants WHERE cap_id=?1",
                params![cap_id],
                |r| Ok(CapRow { cap_id: r.get(0)?, subtree: r.get(1)?, expires_at: r.get(2)?, revoked: r.get::<_, i64>(3)? != 0 }),
            )
            .optional()?)
    }

    /// Link shares whose credential has expired but are still marked active.
    pub fn shares_expired_active(&self, now: i64) -> NfsResult<Vec<ShareRow>> {
        let conn = self.lock();
        let cols = SHARE_COLS.split(", ").map(|c| format!("s.{c}")).collect::<Vec<_>>().join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT {cols} FROM shares s JOIN grants g ON g.cap_id=s.cap_id
             WHERE s.status='active' AND g.expires_at IS NOT NULL AND g.expires_at<=?1"
        ))?;
        let rows = stmt.query_map(params![now], share_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn shares_in_status(&self, status: &str) -> NfsResult<Vec<ShareRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!("SELECT {SHARE_COLS} FROM shares WHERE status=?1"))?;
        let rows = stmt.query_map(params![status], share_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn share_count_expiring(&self, owner: Option<&str>, now: i64, within: i64) -> NfsResult<(i64, i64)> {
        let conn = self.lock();
        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM shares WHERE status='active' AND (?1 IS NULL OR owner_user_id=?1)",
            params![owner],
            |r| r.get(0),
        )?;
        let expiring: i64 = conn.query_row(
            "SELECT COUNT(*) FROM shares s JOIN grants g ON g.cap_id=s.cap_id
             WHERE s.status='active' AND (?1 IS NULL OR s.owner_user_id=?1) AND g.expires_at IS NOT NULL AND g.expires_at>?2 AND g.expires_at<=?3",
            params![owner, now, now + within],
            |r| r.get(0),
        )?;
        Ok((active, expiring))
    }

    // ---------- retention ----------

    pub fn share_retentions(&self, share_id: &str) -> NfsResult<Vec<RetentionRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT obj_id, pin_owner, pinned_at, released_at FROM share_retentions WHERE share_id=?1 ORDER BY pinned_at",
        )?;
        let rows = stmt.query_map(params![share_id], |r| {
            Ok(RetentionRow { obj_id: r.get(0)?, pin_owner: r.get(1)?, pinned_at: r.get(2)?, released_at: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn share_retention_release(&self, share_id: &str, now: i64) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE share_retentions SET released_at=?2 WHERE share_id=?1 AND released_at IS NULL",
            params![share_id, now],
        )?;
        Ok(())
    }

    // ---------- deliveries ----------

    pub fn share_delivery(&self, share_id: &str) -> NfsResult<Option<DeliveryRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT share_id, target_app, handoff_at, delivery_state, business_ref_json, reported_at, error FROM share_deliveries WHERE share_id=?1",
                params![share_id],
                |r| {
                    Ok(DeliveryRow {
                        share_id: r.get(0)?,
                        target_app: r.get(1)?,
                        handoff_at: r.get(2)?,
                        delivery_state: r.get(3)?,
                        business_ref: json_col(r, 4)?,
                        reported_at: r.get(5)?,
                        error: r.get(6)?,
                    })
                },
            )
            .optional()?)
    }

    /// Records an app's report unless a newer fact is already stored (by
    /// observation time, milliseconds). Returns false for a stale report.
    pub fn share_delivery_report(
        &self,
        share_id: &str,
        state: &str,
        business_ref: Option<&Value>,
        error: Option<&str>,
        observed_at: i64,
        audit: AuditEntry,
    ) -> NfsResult<bool> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE share_deliveries SET delivery_state=?2, business_ref_json=COALESCE(?3, business_ref_json), error=?4, reported_at=?5
             WHERE share_id=?1 AND (reported_at IS NULL OR reported_at<=?5)",
            params![share_id, state, business_ref.map(|v| v.to_string()), error, observed_at],
        )?;
        if n == 0 {
            return Ok(false);
        }
        insert_audit(&tx, &audit)?;
        tx.commit()?;
        Ok(true)
    }

    // ---------- audit ----------

    pub fn share_audit_add(&self, a: AuditEntry) -> NfsResult<()> {
        let conn = self.lock();
        insert_audit(&conn, &a)?;
        Ok(())
    }

    pub fn share_audit(&self, share_id: &str, limit: i64) -> NfsResult<Vec<AuditRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT action, revision_before, revision_after, actor_user_id, actor_app_id, source, reason, outcome, at
             FROM share_audit WHERE share_id=?1 ORDER BY audit_id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![share_id, limit], |r| {
            Ok(AuditRow {
                action: r.get(0)?,
                revision_before: r.get(1)?,
                revision_after: r.get(2)?,
                actor_user_id: r.get(3)?,
                actor_app_id: r.get(4)?,
                source: r.get(5)?,
                reason: r.get(6)?,
                outcome: r.get(7)?,
                at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ---------- operations ----------

    pub fn share_operation(&self, operation_id: &str) -> NfsResult<Option<OperationRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(&format!("SELECT {OP_COLS} FROM share_operations WHERE operation_id=?1"), params![operation_id], op_row)
            .optional()?)
    }

    pub fn share_operation_by_key(&self, owner: &str, key: &str) -> NfsResult<Option<OperationRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {OP_COLS} FROM share_operations WHERE owner_user_id=?1 AND idempotency_key=?2"),
                params![owner, key],
                op_row,
            )
            .optional()?)
    }

    /// Inserts a new operation; Ok(false) when (owner, key) already exists.
    pub fn share_operation_insert(&self, op: &OperationRow) -> NfsResult<bool> {
        let conn = self.lock();
        let n = conn.execute(
            &format!("INSERT OR IGNORE INTO share_operations({OP_COLS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"),
            params![
                op.operation_id, op.intent, op.owner_user_id, op.actor_user_id, op.actor_app_id, op.input.to_string(),
                op.idempotency_key, op.task_id, op.share_id, op.state, op.error.as_ref().map(|v| v.to_string()),
                op.progress.as_ref().map(|v| v.to_string()), op.result.as_ref().map(|v| v.to_string()),
                op.created_at, op.updated_at
            ],
        )?;
        Ok(n > 0)
    }

    pub fn share_operation_update(&self, op: &OperationRow) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE share_operations SET task_id=?2, share_id=?3, state=?4, error_json=?5, progress_json=?6, result_json=?7, updated_at=?8
             WHERE operation_id=?1",
            params![
                op.operation_id, op.task_id, op.share_id, op.state, op.error.as_ref().map(|v| v.to_string()),
                op.progress.as_ref().map(|v| v.to_string()), op.result.as_ref().map(|v| v.to_string()),
                crate::fsutil::unix_now()
            ],
        )?;
        Ok(())
    }

    pub fn share_operations_unfinished(&self) -> NfsResult<Vec<OperationRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {OP_COLS} FROM share_operations WHERE state IN ('preparing','running') ORDER BY created_at"
        ))?;
        let rows = stmt.query_map([], op_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ---------- access statistics ----------

    /// Appends buffered data-plane events and folds them into hourly buckets.
    /// A visitor counts once per share per day (first event of the day).
    pub fn share_access_flush(&self, events: &[AccessEvent]) -> NfsResult<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for e in events {
            let new_visitor = match &e.visitor {
                Some(v) if matches!(e.kind, "page" | "download") => tx
                    .query_row(
                        "SELECT 1 FROM share_access_log WHERE share_id=?1 AND visitor=?2 AND at>=?3 AND kind IN ('page','download') LIMIT 1",
                        params![e.share_id, v, day_start(e.at)],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?
                    .is_none(),
                _ => false,
            };
            tx.execute(
                "INSERT INTO share_access_log(share_id, at, kind, status, bytes, visitor, inner_path) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![e.share_id, e.at, e.kind, e.status, e.bytes as i64, e.visitor, e.inner_path],
            )?;
            let views = (e.kind == "page" && e.status < 400) as i64;
            let downloads = (e.kind == "download" && e.download_start && e.status < 400) as i64;
            tx.execute(
                "INSERT INTO share_access_stats(share_id, bucket, views, downloads, bytes, visitors) VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(share_id, bucket) DO UPDATE SET views=views+?3, downloads=downloads+?4, bytes=bytes+?5, visitors=visitors+?6",
                params![e.share_id, hour_bucket(e.at), views, downloads, e.bytes as i64, new_visitor as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Hourly or daily series over the given shares (empty buckets omitted).
    pub fn share_stats_series(&self, share_ids: &[String], from: i64, to: i64, daily: bool) -> NfsResult<Vec<StatsPoint>> {
        if share_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let marks = (0..share_ids.len()).map(|i| format!("?{}", i + 3)).collect::<Vec<_>>().join(",");
        let group = if daily { "bucket - (bucket % 86400)" } else { "bucket" };
        let sql = format!(
            "SELECT {group} AS t, SUM(views), SUM(downloads), SUM(bytes), SUM(visitors) FROM share_access_stats
             WHERE bucket>=?1 AND bucket<?2 AND share_id IN ({marks}) GROUP BY t ORDER BY t"
        );
        let mut args: Vec<rusqlite::types::Value> = vec![from.into(), to.into()];
        args.extend(share_ids.iter().map(|s| rusqlite::types::Value::from(s.clone())));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok(StatsPoint { t: r.get(0)?, views: r.get(1)?, downloads: r.get(2)?, bytes: r.get(3)?, visitors: r.get(4)? })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// All-time totals for one share, plus the last access time.
    pub fn share_stats_total(&self, share_id: &str) -> NfsResult<(StatsPoint, Option<i64>)> {
        let conn = self.lock();
        let p = conn.query_row(
            "SELECT COALESCE(SUM(views),0), COALESCE(SUM(downloads),0), COALESCE(SUM(bytes),0), COALESCE(SUM(visitors),0)
             FROM share_access_stats WHERE share_id=?1",
            params![share_id],
            |r| Ok(StatsPoint { t: 0, views: r.get(0)?, downloads: r.get(1)?, bytes: r.get(2)?, visitors: r.get(3)? }),
        )?;
        let last: Option<i64> =
            conn.query_row("SELECT MAX(at) FROM share_access_log WHERE share_id=?1", params![share_id], |r| r.get(0))?;
        Ok((p, last))
    }

    pub fn share_access_prune(&self, before: i64) -> NfsResult<usize> {
        let conn = self.lock();
        Ok(conn.execute("DELETE FROM share_access_log WHERE at<?1", params![before])?)
    }

    // ---------- public observations ----------

    pub fn public_observations(&self) -> NfsResult<Vec<PublicObservation>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT rel_path, entry_kind, link_target, link_ok, size, mtime, first_seen_at, last_seen_at, missing_since
             FROM public_observations ORDER BY rel_path",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PublicObservation {
                rel_path: r.get(0)?,
                entry_kind: r.get(1)?,
                link_target: r.get(2)?,
                link_ok: r.get::<_, Option<i64>>(3)?.map(|v| v != 0),
                size: r.get(4)?,
                mtime: r.get(5)?,
                first_seen_at: r.get(6)?,
                last_seen_at: r.get(7)?,
                missing_since: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Replaces the observation set with one scan: present entries are
    /// upserted, absent ones get `missing_since` (kept for 7 days, then dropped).
    pub fn public_observations_apply(&self, seen: &[PublicObservation], now: i64) -> NfsResult<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("CREATE TEMP TABLE IF NOT EXISTS public_seen(rel_path TEXT PRIMARY KEY)", [])?;
        tx.execute("DELETE FROM public_seen", [])?;
        for o in seen {
            tx.execute(
                "INSERT INTO public_observations(rel_path, entry_kind, link_target, link_ok, size, mtime, first_seen_at, last_seen_at, missing_since)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?7,NULL)
                 ON CONFLICT(rel_path) DO UPDATE SET entry_kind=?2, link_target=?3, link_ok=?4, size=?5, mtime=?6, last_seen_at=?7, missing_since=NULL",
                params![o.rel_path, o.entry_kind, o.link_target, o.link_ok.map(|b| b as i64), o.size, o.mtime, now],
            )?;
            tx.execute("INSERT OR IGNORE INTO public_seen(rel_path) VALUES(?1)", params![o.rel_path])?;
        }
        tx.execute(
            "UPDATE public_observations SET missing_since=?1 WHERE missing_since IS NULL AND rel_path NOT IN (SELECT rel_path FROM public_seen)",
            params![now],
        )?;
        tx.execute("DELETE FROM public_observations WHERE missing_since IS NOT NULL AND missing_since<?1", params![now - 7 * 86400])?;
        tx.commit()?;
        Ok(())
    }
}

/// Errors surfaced by `share.*` carry a stable business `reason` next to the NFSP code.
pub fn reason(code: crate::error::ErrorCode, reason: &str, msg: impl Into<String>) -> NfsError {
    NfsError::new(code, msg).with("reason", serde_json::json!(reason))
}
