//! Durable token quota ledger and visibility policy (doc/aicc/aicc_token_quota.md).
//!
//! Every upstream attempt of a metered call has a row in `aicc_quota_attempt`
//! from admission to settlement. Periodic counters exist only while an
//! enabled rule maintains them and are rebuilt from that ledger whenever a
//! rule starts maintaining one; total accounts are never rebuilt, they live
//! as long as the Zone and keep counting while their rules are disabled or
//! deleted. Every write transaction starts with a write so that SQLite takes
//! its write lock before reading, and every reservation is a conditional
//! update so that concurrent admissions on Postgres cannot overdraw.

use super::*;
use crate::quota::{
    call_scopes, counter_key, denial_details, object_from_parts, object_parts, periodic_state,
    resource_from_parts, resource_parts, scope_key, total_state, CheckedRule, Ledger,
};
use buckyos_api::{
    QuotaAccountView, QuotaAuditRecord, QuotaDenialDetails, QuotaGrantKind, QuotaGrantRecord,
    QuotaKind, QuotaObject, QuotaPendingPeriod, QuotaPeriod, QuotaResource, QuotaRule,
    QuotaRuleSource, QuotaUsageState, VisibilityEntry, VisibilitySubjectPolicy,
};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const QUOTA_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS aicc_quota_rule (
 rule_id TEXT PRIMARY KEY, source TEXT NOT NULL, object_type TEXT NOT NULL,
 user_id TEXT NOT NULL, app_id TEXT NOT NULL, resource_type TEXT NOT NULL, resource_id TEXT NOT NULL,
 scope_key TEXT NOT NULL, kind TEXT NOT NULL, period TEXT, limit_tokens BIGINT, account_id TEXT,
 enabled INTEGER NOT NULL, deleted INTEGER NOT NULL, version BIGINT NOT NULL,
 pending_period TEXT, pending_effective_at_ms BIGINT, pending_conflict INTEGER NOT NULL DEFAULT 0,
 created_by TEXT NOT NULL, created_at_ms BIGINT NOT NULL, updated_by TEXT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_user ON aicc_quota_rule(user_id, deleted);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_app ON aicc_quota_rule(app_id, deleted);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_scope ON aicc_quota_rule(scope_key, deleted);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_pending ON aicc_quota_rule(pending_effective_at_ms);
CREATE TABLE IF NOT EXISTS aicc_quota_counter (
 counter_key TEXT PRIMARY KEY, scope_key TEXT NOT NULL, period TEXT NOT NULL,
 window_start_ms BIGINT NOT NULL, window_end_ms BIGINT NOT NULL,
 settled_tokens BIGINT NOT NULL, reserved_tokens BIGINT NOT NULL, pending_tokens BIGINT NOT NULL,
 updated_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_counter_end ON aicc_quota_counter(window_end_ms);
CREATE TABLE IF NOT EXISTS aicc_quota_account (
 account_id TEXT PRIMARY KEY, scope_key TEXT NOT NULL UNIQUE, object_type TEXT NOT NULL,
 user_id TEXT NOT NULL, app_id TEXT NOT NULL, resource_type TEXT NOT NULL, resource_id TEXT NOT NULL,
 granted_tokens BIGINT NOT NULL, settled_tokens BIGINT NOT NULL, reserved_tokens BIGINT NOT NULL,
 pending_tokens BIGINT NOT NULL, start_at_ms BIGINT NOT NULL, version BIGINT NOT NULL,
 updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_quota_grant (
 grant_id TEXT PRIMARY KEY, account_id TEXT NOT NULL, kind TEXT NOT NULL, amount_tokens BIGINT NOT NULL,
 operator TEXT NOT NULL, note TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_grant_account ON aicc_quota_grant(account_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_quota_attempt (
 attempt_id TEXT PRIMARY KEY, task_id TEXT NOT NULL, user_id TEXT NOT NULL, app_id TEXT NOT NULL,
 operator_id TEXT, provider_instance_name TEXT NOT NULL, exact_model TEXT NOT NULL, method TEXT NOT NULL,
 admitted_at_ms BIGINT NOT NULL, budget_tokens BIGINT NOT NULL, hard_limit INTEGER NOT NULL,
 state TEXT NOT NULL, charged_tokens BIGINT, reconcile_deadline_ms BIGINT NOT NULL,
 settled_at_ms BIGINT, applied_json TEXT NOT NULL, attribution_json TEXT);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_state ON aicc_quota_attempt(state, reconcile_deadline_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_user_time ON aicc_quota_attempt(user_id, admitted_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_app_time ON aicc_quota_attempt(app_id, admitted_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_task ON aicc_quota_attempt(task_id);
CREATE TABLE IF NOT EXISTS aicc_quota_audit (
 audit_id TEXT PRIMARY KEY, target_type TEXT NOT NULL, target_id TEXT NOT NULL, action TEXT NOT NULL,
 operator TEXT NOT NULL, before_json TEXT, after_json TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_audit_target ON aicc_quota_audit(target_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_visibility_policy (
 subject TEXT PRIMARY KEY, entries_json TEXT NOT NULL, updated_by TEXT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_state (
 state_key TEXT PRIMARY KEY, int_value BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL);
"#;

const VISIBILITY_REVISION_KEY: &str = "visibility_revision";
const VISIBILITY_SEEDED_KEY: &str = "visibility_seeded";
/// Settled attempts and periodic counters older than every window are pruned.
const QUOTA_LEDGER_RETENTION_MS: i64 = 8 * 24 * 60 * 60 * 1000;

const STATE_ADMITTING: &str = "admitting";
const STATE_RESERVED: &str = "reserved";
const STATE_PENDING: &str = "pending";
const STATE_SETTLED: &str = "settled";
const STATE_ESTIMATED: &str = "estimated";
const STATE_RELEASED: &str = "released";

static QUOTA_ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn new_quota_id(prefix: &str, salt: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = QUOTA_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let digest = Sha256::digest(format!("{nanos}:{sequence}:{salt}").as_bytes());
    format!(
        "{prefix}{}",
        digest[..10]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// Account id of a scope: stable, so a rebuilt rule finds its old ledger.
pub(crate) fn account_id_for_scope(scope_key: &str) -> String {
    let digest = Sha256::digest(scope_key.as_bytes());
    format!(
        "qa_{}",
        digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// One upstream attempt asking for a reservation.
#[derive(Clone, Debug)]
pub(crate) struct AttemptAdmission {
    pub attempt_id: String,
    pub task_id: String,
    pub user_id: String,
    pub app_id: Option<String>,
    pub operator_id: Option<String>,
    pub provider_instance_name: String,
    pub exact_model: String,
    pub method: String,
    pub budget_tokens: u64,
    pub hard_limit: bool,
    pub now_ms: i64,
    pub reconcile_deadline_ms: i64,
    pub attribution_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AdmitOutcome {
    Admitted,
    /// The attempt id was admitted before (replay or recovery).
    Replayed,
    Denied(QuotaDenialDetails),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettleOutcome {
    /// Real usage: replaces the reservation (and corrects an estimate).
    Actual(u64),
    /// The Provider never accepted the attempt: nothing was consumed.
    Release,
    /// Usage is unknown: keep the reservation until it is reconciled.
    Unknown,
    /// Reconciliation deadline passed: charge the reservation.
    Estimate,
}

#[derive(Debug)]
pub(crate) enum QuotaWriteError {
    NotFound,
    VersionConflict { expected: u64, actual: u64 },
    /// A rule of the same identity exists (its id).
    Duplicate(String),
    /// Same recharge id with different parameters.
    RechargeConflict,
    Invalid(&'static str),
    Storage(StorageError),
}

impl From<StorageError> for QuotaWriteError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for QuotaWriteError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::Database(error))
    }
}

impl From<serde_json::Error> for QuotaWriteError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(StorageError::Json(error))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AccountRecord {
    pub account_id: String,
    pub scope_key: String,
    pub object: QuotaObject,
    pub resource: QuotaResource,
    pub granted: u64,
    pub ledger: Ledger,
    pub start_at_ms: i64,
    pub version: u64,
    pub updated_at_ms: i64,
}

impl AccountRecord {
    pub(crate) fn view(&self, enforced: bool) -> QuotaAccountView {
        let state = total_state(true, self.granted, self.ledger, self.start_at_ms, self.updated_at_ms);
        QuotaAccountView {
            account_id: self.account_id.clone(),
            object: self.object.clone(),
            resource: self.resource.clone(),
            granted_tokens: self.granted,
            settled_tokens: self.ledger.settled,
            reserved_tokens: self.ledger.reserved,
            pending_tokens: self.ledger.pending,
            available_tokens: state.available_tokens,
            debt_tokens: state.debt_tokens,
            start_at_ms: self.start_at_ms,
            version: self.version,
            updated_at_ms: self.updated_at_ms,
            enforced,
        }
    }
}

/// Rule ids a counter or account was charged on behalf of an attempt.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct AppliedTargets {
    #[serde(default)]
    counters: Vec<String>,
    #[serde(default)]
    accounts: Vec<String>,
}

#[derive(Clone, Debug)]
struct AttemptRow {
    state: String,
    budget: u64,
    charged: Option<u64>,
    applied: AppliedTargets,
}

#[derive(Clone, Debug)]
struct StoredRule {
    rule: QuotaRule,
    scope_key: String,
}

fn source_name(source: QuotaRuleSource) -> &'static str {
    match source {
        QuotaRuleSource::Admin => "admin",
        QuotaRuleSource::User => "user",
    }
}

fn kind_name(kind: QuotaKind) -> &'static str {
    match kind {
        QuotaKind::Periodic => "periodic",
        QuotaKind::Total => "total",
    }
}

fn rule_from_row(row: &AnyRow) -> StorageResult<StoredRule> {
    let invalid = || StorageError::InvalidRecord("quota rule row is invalid".into());
    let source = match row.try_get::<String, _>("source")?.as_str() {
        "admin" => QuotaRuleSource::Admin,
        "user" => QuotaRuleSource::User,
        _ => return Err(invalid()),
    };
    let kind = match row.try_get::<String, _>("kind")?.as_str() {
        "periodic" => QuotaKind::Periodic,
        "total" => QuotaKind::Total,
        _ => return Err(invalid()),
    };
    let object_type: String = row.try_get("object_type")?;
    let user_id: String = row.try_get("user_id")?;
    let app_id: String = row.try_get("app_id")?;
    let resource_type: String = row.try_get("resource_type")?;
    let resource_id: String = row.try_get("resource_id")?;
    let period = row
        .try_get::<Option<String>, _>("period")?
        .map(|value| QuotaPeriod::parse(&value).ok_or_else(invalid))
        .transpose()?;
    let pending_period = match (
        row.try_get::<Option<String>, _>("pending_period")?,
        row.try_get::<Option<i64>, _>("pending_effective_at_ms")?,
    ) {
        (Some(period), Some(effective_at_ms)) => Some(QuotaPendingPeriod {
            period: QuotaPeriod::parse(&period).ok_or_else(invalid)?,
            effective_at_ms,
            conflict: row.try_get::<i64, _>("pending_conflict")? != 0,
        }),
        _ => None,
    };
    Ok(StoredRule {
        scope_key: row.try_get("scope_key")?,
        rule: QuotaRule {
            rule_id: row.try_get("rule_id")?,
            source,
            object: object_from_parts(&object_type, &user_id, &app_id).ok_or_else(invalid)?,
            resource: resource_from_parts(&resource_type, &resource_id).ok_or_else(invalid)?,
            kind,
            period,
            limit_tokens: opt_from_i64(row.try_get("limit_tokens")?)?,
            account_id: row.try_get("account_id")?,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            deleted: row.try_get::<i64, _>("deleted")? != 0,
            pending_period,
            version: from_i64(row.try_get("version")?)?,
            created_by: row.try_get("created_by")?,
            created_at_ms: row.try_get("created_at_ms")?,
            updated_by: row.try_get("updated_by")?,
            updated_at_ms: row.try_get("updated_at_ms")?,
        },
    })
}

fn account_from_row(row: &AnyRow) -> StorageResult<AccountRecord> {
    let invalid = || StorageError::InvalidRecord("quota account row is invalid".into());
    let object_type: String = row.try_get("object_type")?;
    let user_id: String = row.try_get("user_id")?;
    let app_id: String = row.try_get("app_id")?;
    let resource_type: String = row.try_get("resource_type")?;
    let resource_id: String = row.try_get("resource_id")?;
    Ok(AccountRecord {
        account_id: row.try_get("account_id")?,
        scope_key: row.try_get("scope_key")?,
        object: object_from_parts(&object_type, &user_id, &app_id).ok_or_else(invalid)?,
        resource: resource_from_parts(&resource_type, &resource_id).ok_or_else(invalid)?,
        granted: from_i64(row.try_get("granted_tokens")?)?,
        ledger: Ledger {
            settled: from_i64(row.try_get("settled_tokens")?)?,
            reserved: from_i64(row.try_get("reserved_tokens")?)?,
            pending: from_i64(row.try_get("pending_tokens")?)?,
        },
        start_at_ms: row.try_get("start_at_ms")?,
        version: from_i64(row.try_get("version")?)?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn grant_from_row(row: &AnyRow) -> StorageResult<QuotaGrantRecord> {
    Ok(QuotaGrantRecord {
        grant_id: row.try_get("grant_id")?,
        account_id: row.try_get("account_id")?,
        kind: match row.try_get::<String, _>("kind")?.as_str() {
            "initial" => QuotaGrantKind::Initial,
            "recharge" => QuotaGrantKind::Recharge,
            _ => {
                return Err(StorageError::InvalidRecord(
                    "quota grant kind is invalid".into(),
                ))
            }
        },
        amount_tokens: from_i64(row.try_get("amount_tokens")?)?,
        operator: row.try_get("operator")?,
        note: row.try_get("note")?,
        created_at_ms: row.try_get("created_at_ms")?,
    })
}

/// `(sql, binds)` restricting `aicc_quota_attempt` (or, with `usage`,
/// `aicc_usage_event`) to the calls of one scope.
fn scope_predicate(object: &QuotaObject, resource: &QuotaResource, usage: bool) -> (String, Vec<String>) {
    let (user_column, app_column, provider_column, model_column) = if usage {
        ("u.user_id", "u.caller_app_id", "u.provider_instance_name", "u.provider_model")
    } else {
        ("user_id", "app_id", "provider_instance_name", "exact_model")
    };
    let mut clauses = Vec::new();
    let mut binds = Vec::new();
    let (_, user_id, app_id) = object_parts(object);
    if !user_id.is_empty() {
        clauses.push(format!("{user_column}=?"));
        binds.push(user_id.to_string());
    }
    if !app_id.is_empty() {
        clauses.push(format!("{app_column}=?"));
        binds.push(app_id.to_string());
    }
    match resource {
        QuotaResource::Provider {
            provider_instance_name,
        } => {
            clauses.push(format!("{provider_column}=?"));
            binds.push(provider_instance_name.clone());
        }
        QuotaResource::Model { exact_model } => {
            clauses.push(format!("{model_column}=?"));
            binds.push(exact_model.clone());
        }
        QuotaResource::All | QuotaResource::Hidden => {}
    }
    (clauses.join(" AND "), binds)
}

macro_rules! bind_all {
    ($query:expr, $binds:expr) => {{
        let mut query = $query;
        for bind in $binds {
            query = query.bind(bind.clone());
        }
        query
    }};
}

impl AiccStorage {
    pub(super) async fn seed_default_visibility(&self) -> StorageResult<()> {
        let mut tx = self.pool.begin().await?;
        let insert = self.sql(
            "INSERT INTO aicc_state (state_key,int_value,updated_at_ms) VALUES (?,1,0) ON CONFLICT(state_key) DO NOTHING",
        );
        let seeded = sqlx::query(&insert)
            .bind(VISIBILITY_SEEDED_KEY)
            .execute(&mut *tx)
            .await?;
        if seeded.rows_affected() == 1 {
            let entries = serde_json::to_string(&vec![VisibilityEntry::AllProviders])?;
            let sql = self.sql(
                "INSERT INTO aicc_visibility_policy (subject,entries_json,updated_by,updated_at_ms)
                 VALUES (?,?,?,0) ON CONFLICT(subject) DO NOTHING",
            );
            for subject in ["type:admin", "type:user"] {
                sqlx::query(&sql)
                    .bind(subject)
                    .bind(&entries)
                    .bind("system")
                    .execute(&mut *tx)
                    .await?;
            }
            let revision = self.sql(
                "INSERT INTO aicc_state (state_key,int_value,updated_at_ms) VALUES (?,1,0)
                 ON CONFLICT(state_key) DO NOTHING",
            );
            sqlx::query(&revision)
                .bind(VISIBILITY_REVISION_KEY)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn visibility_policy(
        &self,
    ) -> StorageResult<(u64, Vec<VisibilitySubjectPolicy>)> {
        let revision = self.state_value(VISIBILITY_REVISION_KEY).await?;
        let rows = sqlx::query("SELECT * FROM aicc_visibility_policy ORDER BY subject")
            .fetch_all(&self.pool)
            .await?;
        let mut subjects = Vec::with_capacity(rows.len());
        for row in rows {
            let entries: String = row.try_get("entries_json")?;
            subjects.push(VisibilitySubjectPolicy {
                subject: row.try_get("subject")?,
                entries: serde_json::from_str(&entries)?,
                updated_by: row.try_get("updated_by")?,
                updated_at_ms: row.try_get("updated_at_ms")?,
            });
        }
        Ok((revision, subjects))
    }

    async fn state_value(&self, key: &str) -> StorageResult<u64> {
        let sql = self.sql("SELECT int_value FROM aicc_state WHERE state_key=?");
        let value: Option<i64> = sqlx::query_scalar(&sql)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        from_i64(value.unwrap_or(0))
    }

    /// Replace the allow items of one subject when the policy is still at
    /// `expected_revision`; returns the new revision.
    pub(crate) async fn update_visibility(
        &self,
        expected_revision: u64,
        subject: &str,
        entries: &[VisibilityEntry],
        operator: &str,
        now_ms: i64,
    ) -> Result<u64, QuotaWriteError> {
        let mut tx = self.pool.begin().await?;
        let bump = self.sql(
            "INSERT INTO aicc_state (state_key,int_value,updated_at_ms) VALUES (?,1,?)
             ON CONFLICT(state_key) DO UPDATE SET int_value=aicc_state.int_value+1, updated_at_ms=excluded.updated_at_ms",
        );
        sqlx::query(&bump)
            .bind(VISIBILITY_REVISION_KEY)
            .bind(now_ms)
            .execute(&mut *tx)
            .await?;
        let read = self.sql("SELECT int_value FROM aicc_state WHERE state_key=?");
        let revision: i64 = sqlx::query_scalar(&read)
            .bind(VISIBILITY_REVISION_KEY)
            .fetch_one(&mut *tx)
            .await?;
        let revision = from_i64(revision)?;
        if revision != expected_revision.saturating_add(1) {
            tx.rollback().await?;
            return Err(QuotaWriteError::VersionConflict {
                expected: expected_revision,
                actual: revision.saturating_sub(1),
            });
        }
        let before_sql = self.sql("SELECT entries_json FROM aicc_visibility_policy WHERE subject=?");
        let before: Option<String> = sqlx::query_scalar(&before_sql)
            .bind(subject)
            .fetch_optional(&mut *tx)
            .await?;
        if entries.is_empty() {
            let sql = self.sql("DELETE FROM aicc_visibility_policy WHERE subject=?");
            sqlx::query(&sql).bind(subject).execute(&mut *tx).await?;
        } else {
            let sql = self.sql(
                "INSERT INTO aicc_visibility_policy (subject,entries_json,updated_by,updated_at_ms)
                 VALUES (?,?,?,?) ON CONFLICT(subject) DO UPDATE SET entries_json=excluded.entries_json,
                 updated_by=excluded.updated_by, updated_at_ms=excluded.updated_at_ms",
            );
            sqlx::query(&sql)
                .bind(subject)
                .bind(serde_json::to_string(entries)?)
                .bind(operator)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
        }
        self.audit_tx(
            &mut tx,
            "visibility",
            subject,
            "update",
            operator,
            before.map(|value| serde_json::from_str(&value)).transpose()?,
            Some(serde_json::to_value(entries)?),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(revision)
    }

    #[allow(clippy::too_many_arguments)]
    async fn audit_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        target_type: &str,
        target_id: &str,
        action: &str,
        operator: &str,
        before: Option<Value>,
        after: Option<Value>,
        now_ms: i64,
    ) -> StorageResult<()> {
        let sql = self.sql(
            "INSERT INTO aicc_quota_audit
             (audit_id,target_type,target_id,action,operator,before_json,after_json,created_at_ms)
             VALUES (?,?,?,?,?,?,?,?)",
        );
        sqlx::query(&sql)
            .bind(new_quota_id("qx_", target_id))
            .bind(target_type)
            .bind(target_id)
            .bind(action)
            .bind(operator)
            .bind(before.map(|value| value.to_string()))
            .bind(after.map(|value| value.to_string()))
            .bind(now_ms)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub(crate) async fn quota_audit(&self, target_ids: &[String]) -> StorageResult<Vec<QuotaAuditRecord>> {
        if target_ids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; target_ids.len()].join(",");
        let sql = self.sql(&format!(
            "SELECT * FROM aicc_quota_audit WHERE target_id IN ({marks}) ORDER BY created_at_ms DESC LIMIT 200"
        ));
        let rows = bind_all!(sqlx::query(&sql), target_ids).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                let before: Option<String> = row.try_get("before_json")?;
                let after: Option<String> = row.try_get("after_json")?;
                Ok(QuotaAuditRecord {
                    audit_id: row.try_get("audit_id")?,
                    target_type: row.try_get("target_type")?,
                    target_id: row.try_get("target_id")?,
                    action: row.try_get("action")?,
                    operator: row.try_get("operator")?,
                    before: before.map(|value| serde_json::from_str(&value)).transpose()?,
                    after: after.map(|value| serde_json::from_str(&value)).transpose()?,
                    created_at_ms: row.try_get("created_at_ms")?,
                })
            })
            .collect()
    }

    pub(crate) async fn quota_rules(&self, include_deleted: bool) -> StorageResult<Vec<QuotaRule>> {
        let sql = if include_deleted {
            "SELECT * FROM aicc_quota_rule ORDER BY created_at_ms"
        } else {
            "SELECT * FROM aicc_quota_rule WHERE deleted=0 ORDER BY created_at_ms"
        };
        sqlx::query(sql)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| rule_from_row(row).map(|stored| stored.rule))
            .collect()
    }

    pub(crate) async fn quota_rule(&self, rule_id: &str) -> StorageResult<Option<QuotaRule>> {
        let sql = self.sql("SELECT * FROM aicc_quota_rule WHERE rule_id=?");
        sqlx::query(&sql)
            .bind(rule_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| rule_from_row(&row).map(|stored| stored.rule))
            .transpose()
    }

    pub(crate) async fn quota_account(&self, account_id: &str) -> StorageResult<Option<AccountRecord>> {
        let sql = self.sql("SELECT * FROM aicc_quota_account WHERE account_id=?");
        sqlx::query(&sql)
            .bind(account_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| account_from_row(&row))
            .transpose()
    }

    pub(crate) async fn quota_account_for_scope(
        &self,
        object: &QuotaObject,
        resource: &QuotaResource,
    ) -> StorageResult<Option<AccountRecord>> {
        let sql = self.sql("SELECT * FROM aicc_quota_account WHERE scope_key=?");
        sqlx::query(&sql)
            .bind(scope_key(object, resource))
            .fetch_optional(&self.pool)
            .await?
            .map(|row| account_from_row(&row))
            .transpose()
    }

    pub(crate) async fn quota_grants(&self, account_id: &str) -> StorageResult<Vec<QuotaGrantRecord>> {
        let sql = self.sql(
            "SELECT * FROM aicc_quota_grant WHERE account_id=? ORDER BY created_at_ms DESC",
        );
        sqlx::query(&sql)
            .bind(account_id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(grant_from_row)
            .collect()
    }

    pub(crate) async fn quota_grant(&self, grant_id: &str) -> StorageResult<Option<QuotaGrantRecord>> {
        let sql = self.sql("SELECT * FROM aicc_quota_grant WHERE grant_id=?");
        sqlx::query(&sql)
            .bind(grant_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| grant_from_row(&row))
            .transpose()
    }

    /// Live state of a rule: its current window (read from the counter, or
    /// summed from the ledger when no enabled rule maintains one) or its account.
    pub(crate) async fn quota_rule_state(
        &self,
        rule: &QuotaRule,
        now_ms: i64,
    ) -> StorageResult<QuotaUsageState> {
        match rule.kind {
            QuotaKind::Periodic => {
                let period = rule.period.unwrap_or(QuotaPeriod::Day);
                let (start, end) = period.window(now_ms);
                let key = counter_key(&scope_key(&rule.object, &rule.resource), period, start);
                let sql = self.sql("SELECT * FROM aicc_quota_counter WHERE counter_key=?");
                let (ledger, updated_at_ms) = match sqlx::query(&sql)
                    .bind(&key)
                    .fetch_optional(&self.pool)
                    .await?
                {
                    Some(row) => (
                        Ledger {
                            settled: from_i64(row.try_get("settled_tokens")?)?,
                            reserved: from_i64(row.try_get("reserved_tokens")?)?,
                            pending: from_i64(row.try_get("pending_tokens")?)?,
                        },
                        row.try_get("updated_at_ms")?,
                    ),
                    None => {
                        let mut conn = self.pool.acquire().await?;
                        (
                            self.sum_window(&mut conn, &rule.object, &rule.resource, start, end)
                                .await?,
                            now_ms,
                        )
                    }
                };
                Ok(periodic_state(rule, period, ledger, now_ms, updated_at_ms))
            }
            QuotaKind::Total => {
                let account = match rule.account_id.as_deref() {
                    Some(account_id) => self.quota_account(account_id).await?,
                    None => None,
                };
                Ok(match account {
                    Some(account) => total_state(
                        rule.enabled && !rule.deleted,
                        account.granted,
                        account.ledger,
                        account.start_at_ms,
                        account.updated_at_ms,
                    ),
                    None => total_state(rule.enabled, 0, Ledger::default(), now_ms, now_ms),
                })
            }
        }
    }

    /// Ledger of the attempts admitted in `[start, end)` within a scope, plus
    /// usage recorded before the ledger existed (no attempt row for its task).
    async fn sum_window(
        &self,
        conn: &mut sqlx::AnyConnection,
        object: &QuotaObject,
        resource: &QuotaResource,
        start: i64,
        end: i64,
    ) -> StorageResult<Ledger> {
        let (predicate, binds) = scope_predicate(object, resource, false);
        let scope = if predicate.is_empty() {
            String::new()
        } else {
            format!(" AND {predicate}")
        };
        let sql = self.sql(&format!(
            "SELECT
               CAST(COALESCE(SUM(CASE WHEN state IN ('settled','estimated') THEN charged_tokens ELSE 0 END),0) AS BIGINT) AS settled,
               CAST(COALESCE(SUM(CASE WHEN state IN ('reserved','pending') THEN budget_tokens ELSE 0 END),0) AS BIGINT) AS reserved,
               CAST(COALESCE(SUM(CASE WHEN state='pending' THEN budget_tokens ELSE 0 END),0) AS BIGINT) AS pending
             FROM aicc_quota_attempt WHERE admitted_at_ms>=? AND admitted_at_ms<?{scope}"
        ));
        let row = bind_all!(sqlx::query(&sql).bind(start).bind(end), &binds)
            .fetch_one(&mut *conn)
            .await?;
        let (usage_predicate, usage_binds) = scope_predicate(object, resource, true);
        let usage_scope = if usage_predicate.is_empty() {
            String::new()
        } else {
            format!(" AND {usage_predicate}")
        };
        let legacy_sql = self.sql(&format!(
            "SELECT CAST(COALESCE(SUM(COALESCE(u.total_tokens, u.input_tokens + u.output_tokens, u.input_tokens, 0)),0) AS BIGINT)
             FROM aicc_usage_event u WHERE u.created_at_ms>=? AND u.created_at_ms<?{usage_scope}
             AND NOT EXISTS (SELECT 1 FROM aicc_quota_attempt a WHERE a.task_id=u.task_id)"
        ));
        let legacy: i64 = bind_all!(sqlx::query_scalar(&legacy_sql).bind(start).bind(end), &usage_binds)
            .fetch_one(&mut *conn)
            .await?;
        Ok(Ledger {
            settled: from_i64(row.try_get::<i64, _>("settled")?)?.saturating_add(from_i64(legacy)?),
            reserved: from_i64(row.try_get::<i64, _>("reserved")?)?,
            pending: from_i64(row.try_get::<i64, _>("pending")?)?,
        })
    }

    /// The counter of a window, created from the ledger when missing. In-flight
    /// attempts of the window are registered on it so their settlement reaches it.
    #[allow(clippy::too_many_arguments)]
    async fn ensure_counter(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        scope: &str,
        object: &QuotaObject,
        resource: &QuotaResource,
        period: QuotaPeriod,
        start: i64,
        end: i64,
        now_ms: i64,
    ) -> StorageResult<Ledger> {
        let key = counter_key(scope, period, start);
        let read = self.sql("SELECT * FROM aicc_quota_counter WHERE counter_key=?");
        if let Some(row) = sqlx::query(&read).bind(&key).fetch_optional(&mut **tx).await? {
            return Ok(Ledger {
                settled: from_i64(row.try_get("settled_tokens")?)?,
                reserved: from_i64(row.try_get("reserved_tokens")?)?,
                pending: from_i64(row.try_get("pending_tokens")?)?,
            });
        }
        let ledger = self.sum_window(&mut **tx, object, resource, start, end).await?;
        let insert = self.sql(
            "INSERT INTO aicc_quota_counter
             (counter_key,scope_key,period,window_start_ms,window_end_ms,settled_tokens,reserved_tokens,pending_tokens,updated_at_ms)
             VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(counter_key) DO NOTHING",
        );
        sqlx::query(&insert)
            .bind(&key)
            .bind(scope)
            .bind(period.as_str())
            .bind(start)
            .bind(end)
            .bind(to_i64(ledger.settled)?)
            .bind(to_i64(ledger.reserved)?)
            .bind(to_i64(ledger.pending)?)
            .bind(now_ms)
            .execute(&mut **tx)
            .await?;
        let (predicate, binds) = scope_predicate(object, resource, false);
        let scope_sql = if predicate.is_empty() {
            String::new()
        } else {
            format!(" AND {predicate}")
        };
        let inflight = self.sql(&format!(
            "SELECT attempt_id, applied_json FROM aicc_quota_attempt
             WHERE state IN ('reserved','pending') AND admitted_at_ms>=? AND admitted_at_ms<?{scope_sql}"
        ));
        let rows = bind_all!(sqlx::query(&inflight).bind(start).bind(end), &binds)
            .fetch_all(&mut **tx)
            .await?;
        let update = self.sql("UPDATE aicc_quota_attempt SET applied_json=? WHERE attempt_id=?");
        for row in rows {
            let attempt_id: String = row.try_get("attempt_id")?;
            let applied: String = row.try_get("applied_json")?;
            let mut applied: AppliedTargets = serde_json::from_str(&applied)?;
            if !applied.counters.contains(&key) {
                applied.counters.push(key.clone());
                sqlx::query(&update)
                    .bind(serde_json::to_string(&applied)?)
                    .bind(&attempt_id)
                    .execute(&mut **tx)
                    .await?;
            }
        }
        Ok(ledger)
    }

    /// Rebuild the current-window counter of a rule that starts maintaining
    /// it, unless another enabled rule of the same scope and period already does.
    async fn refresh_counter(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        rule: &StoredRule,
        now_ms: i64,
    ) -> StorageResult<()> {
        let Some(period) = rule.rule.period else {
            return Ok(());
        };
        if self
            .counter_shared(tx, &rule.scope_key, period, &rule.rule.rule_id)
            .await?
        {
            return Ok(());
        }
        let (start, end) = period.window(now_ms);
        let delete = self.sql("DELETE FROM aicc_quota_counter WHERE counter_key=?");
        sqlx::query(&delete)
            .bind(counter_key(&rule.scope_key, period, start))
            .execute(&mut **tx)
            .await?;
        self.ensure_counter(
            tx,
            &rule.scope_key,
            &rule.rule.object,
            &rule.rule.resource,
            period,
            start,
            end,
            now_ms,
        )
        .await?;
        Ok(())
    }

    /// Drop the current-window counter a rule stops maintaining, so that it
    /// can never be read stale later.
    async fn release_counter(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        scope: &str,
        period: QuotaPeriod,
        rule_id: &str,
        now_ms: i64,
    ) -> StorageResult<()> {
        if self.counter_shared(tx, scope, period, rule_id).await? {
            return Ok(());
        }
        let delete = self.sql("DELETE FROM aicc_quota_counter WHERE counter_key=?");
        sqlx::query(&delete)
            .bind(counter_key(scope, period, period.window(now_ms).0))
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn counter_shared(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        scope: &str,
        period: QuotaPeriod,
        rule_id: &str,
    ) -> StorageResult<bool> {
        let sql = self.sql(
            "SELECT COUNT(*) FROM aicc_quota_rule WHERE scope_key=? AND period=? AND kind='periodic'
             AND enabled=1 AND deleted=0 AND rule_id<>?",
        );
        let count: i64 = sqlx::query_scalar(&sql)
            .bind(scope)
            .bind(period.as_str())
            .bind(rule_id)
            .fetch_one(&mut **tx)
            .await?;
        Ok(count > 0)
    }

    async fn stored_rule_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        rule_id: &str,
    ) -> StorageResult<Option<StoredRule>> {
        let sql = self.sql("SELECT * FROM aicc_quota_rule WHERE rule_id=?");
        sqlx::query(&sql)
            .bind(rule_id)
            .fetch_optional(&mut **tx)
            .await?
            .map(|row| rule_from_row(&row))
            .transpose()
    }

    /// Periodic rule of the same source, scope and period (or total rule of
    /// the same source and scope) that is not deleted.
    async fn duplicate_rule(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        source: QuotaRuleSource,
        scope: &str,
        kind: QuotaKind,
        period: Option<QuotaPeriod>,
        excluding: &str,
    ) -> StorageResult<Option<String>> {
        let sql = self.sql(
            "SELECT rule_id, period, pending_period FROM aicc_quota_rule
             WHERE source=? AND scope_key=? AND kind=? AND deleted=0 AND rule_id<>?",
        );
        let rows = sqlx::query(&sql)
            .bind(source_name(source))
            .bind(scope)
            .bind(kind_name(kind))
            .bind(excluding)
            .fetch_all(&mut **tx)
            .await?;
        for row in rows {
            let rule_id: String = row.try_get("rule_id")?;
            if kind == QuotaKind::Total {
                return Ok(Some(rule_id));
            }
            let existing: Option<String> = row.try_get("period")?;
            let pending: Option<String> = row.try_get("pending_period")?;
            let wanted = period.map(QuotaPeriod::as_str);
            if existing.as_deref() == wanted || (pending.is_some() && pending.as_deref() == wanted) {
                return Ok(Some(rule_id));
            }
        }
        Ok(None)
    }

    /// Insert a rule; a total rule restores the account of its scope or
    /// creates it with `initial_tokens`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_quota_rule(
        &self,
        source: QuotaRuleSource,
        object: &QuotaObject,
        resource: &QuotaResource,
        kind: QuotaKind,
        period: Option<QuotaPeriod>,
        limit_tokens: Option<u64>,
        initial_tokens: u64,
        operator: &str,
        now_ms: i64,
    ) -> Result<QuotaRule, QuotaWriteError> {
        let scope = scope_key(object, resource);
        let mut tx = self.pool.begin().await?;
        let lock = self.sql("UPDATE aicc_quota_rule SET version=version WHERE scope_key=?");
        sqlx::query(&lock).bind(&scope).execute(&mut *tx).await?;
        if let Some(existing) = self
            .duplicate_rule(&mut tx, source, &scope, kind, period, "")
            .await?
        {
            tx.rollback().await?;
            return Err(QuotaWriteError::Duplicate(existing));
        }
        let rule_id = new_quota_id("qr_", &scope);
        let account_id = if kind == QuotaKind::Total {
            let account_id = account_id_for_scope(&scope);
            let (object_type, user_id, app_id) = object_parts(object);
            let (resource_type, resource_id) = resource_parts(resource);
            let insert = self.sql(
                "INSERT INTO aicc_quota_account
                 (account_id,scope_key,object_type,user_id,app_id,resource_type,resource_id,granted_tokens,
                  settled_tokens,reserved_tokens,pending_tokens,start_at_ms,version,updated_at_ms)
                 VALUES (?,?,?,?,?,?,?,?,0,0,0,?,1,?) ON CONFLICT(scope_key) DO NOTHING",
            );
            let created = sqlx::query(&insert)
                .bind(&account_id)
                .bind(&scope)
                .bind(object_type)
                .bind(user_id)
                .bind(app_id)
                .bind(resource_type)
                .bind(resource_id)
                .bind(to_i64(initial_tokens)?)
                .bind(now_ms)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
            if created.rows_affected() == 1 {
                let grant = self.sql(
                    "INSERT INTO aicc_quota_grant (grant_id,account_id,kind,amount_tokens,operator,note,created_at_ms)
                     VALUES (?,?,'initial',?,?,NULL,?)",
                );
                sqlx::query(&grant)
                    .bind(format!("initial:{account_id}"))
                    .bind(&account_id)
                    .bind(to_i64(initial_tokens)?)
                    .bind(operator)
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await?;
            }
            Some(account_id)
        } else {
            None
        };
        let (object_type, user_id, app_id) = object_parts(object);
        let (resource_type, resource_id) = resource_parts(resource);
        let insert = self.sql(
            "INSERT INTO aicc_quota_rule
             (rule_id,source,object_type,user_id,app_id,resource_type,resource_id,scope_key,kind,period,
              limit_tokens,account_id,enabled,deleted,version,pending_period,pending_effective_at_ms,
              pending_conflict,created_by,created_at_ms,updated_by,updated_at_ms)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,1,0,1,NULL,NULL,0,?,?,?,?)",
        );
        sqlx::query(&insert)
            .bind(&rule_id)
            .bind(source_name(source))
            .bind(object_type)
            .bind(user_id)
            .bind(app_id)
            .bind(resource_type)
            .bind(resource_id)
            .bind(&scope)
            .bind(kind_name(kind))
            .bind(period.map(QuotaPeriod::as_str))
            .bind(opt_i64(limit_tokens)?)
            .bind(&account_id)
            .bind(operator)
            .bind(now_ms)
            .bind(operator)
            .bind(now_ms)
            .execute(&mut *tx)
            .await?;
        let stored = self
            .stored_rule_tx(&mut tx, &rule_id)
            .await?
            .ok_or(QuotaWriteError::NotFound)?;
        self.refresh_counter(&mut tx, &stored, now_ms).await?;
        self.audit_tx(
            &mut tx,
            "rule",
            &rule_id,
            "create",
            operator,
            None,
            Some(serde_json::to_value(&stored.rule)?),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(stored.rule)
    }

    /// Change the limit, the enabled state or the pending period of a rule
    /// at `expected_version`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn update_quota_rule(
        &self,
        rule_id: &str,
        expected_version: u64,
        limit_tokens: Option<u64>,
        period: Option<QuotaPeriod>,
        enabled: Option<bool>,
        cancel_pending_period: bool,
        operator: &str,
        now_ms: i64,
    ) -> Result<QuotaRule, QuotaWriteError> {
        let mut tx = self.pool.begin().await?;
        let lock = self.sql("UPDATE aicc_quota_rule SET version=version WHERE rule_id=?");
        sqlx::query(&lock).bind(rule_id).execute(&mut *tx).await?;
        let Some(before) = self.stored_rule_tx(&mut tx, rule_id).await? else {
            tx.rollback().await?;
            return Err(QuotaWriteError::NotFound);
        };
        if before.rule.deleted {
            tx.rollback().await?;
            return Err(QuotaWriteError::NotFound);
        }
        if before.rule.version != expected_version {
            tx.rollback().await?;
            return Err(QuotaWriteError::VersionConflict {
                expected: expected_version,
                actual: before.rule.version,
            });
        }
        let periodic = before.rule.kind == QuotaKind::Periodic;
        if (limit_tokens.is_some() || period.is_some()) && !periodic {
            tx.rollback().await?;
            return Err(QuotaWriteError::Invalid(
                "a total rule has no limit or period; recharge its account instead",
            ));
        }
        let mut pending = before.rule.pending_period.clone();
        if cancel_pending_period {
            pending = None;
        }
        if let Some(target) = period {
            if Some(target) == before.rule.period {
                pending = None;
            } else {
                if let Some(existing) = self
                    .duplicate_rule(
                        &mut tx,
                        before.rule.source,
                        &before.scope_key,
                        QuotaKind::Periodic,
                        Some(target),
                        rule_id,
                    )
                    .await?
                {
                    tx.rollback().await?;
                    return Err(QuotaWriteError::Duplicate(existing));
                }
                let current = before.rule.period.unwrap_or(target);
                pending = Some(QuotaPendingPeriod {
                    period: target,
                    effective_at_ms: current.window(now_ms).1,
                    conflict: false,
                });
            }
        }
        let next_enabled = enabled.unwrap_or(before.rule.enabled);
        let update = self.sql(
            "UPDATE aicc_quota_rule SET limit_tokens=?, enabled=?, pending_period=?, pending_effective_at_ms=?,
             pending_conflict=?, version=version+1, updated_by=?, updated_at_ms=? WHERE rule_id=?",
        );
        sqlx::query(&update)
            .bind(opt_i64(limit_tokens.or(before.rule.limit_tokens))?)
            .bind(i64::from(next_enabled))
            .bind(pending.as_ref().map(|pending| pending.period.as_str()))
            .bind(pending.as_ref().map(|pending| pending.effective_at_ms))
            .bind(i64::from(pending.as_ref().is_some_and(|pending| pending.conflict)))
            .bind(operator)
            .bind(now_ms)
            .bind(rule_id)
            .execute(&mut *tx)
            .await?;
        let after = self
            .stored_rule_tx(&mut tx, rule_id)
            .await?
            .ok_or(QuotaWriteError::NotFound)?;
        if periodic && next_enabled && !before.rule.enabled {
            self.refresh_counter(&mut tx, &after, now_ms).await?;
        }
        if periodic && !next_enabled && before.rule.enabled {
            if let Some(period) = before.rule.period {
                self.release_counter(&mut tx, &before.scope_key, period, rule_id, now_ms)
                    .await?;
            }
        }
        self.audit_tx(
            &mut tx,
            "rule",
            rule_id,
            "update",
            operator,
            Some(serde_json::to_value(&before.rule)?),
            Some(serde_json::to_value(&after.rule)?),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(after.rule)
    }

    pub(crate) async fn delete_quota_rule(
        &self,
        rule_id: &str,
        expected_version: u64,
        operator: &str,
        now_ms: i64,
    ) -> Result<QuotaRule, QuotaWriteError> {
        let mut tx = self.pool.begin().await?;
        let lock = self.sql("UPDATE aicc_quota_rule SET version=version WHERE rule_id=?");
        sqlx::query(&lock).bind(rule_id).execute(&mut *tx).await?;
        let Some(before) = self.stored_rule_tx(&mut tx, rule_id).await? else {
            tx.rollback().await?;
            return Err(QuotaWriteError::NotFound);
        };
        if before.rule.deleted {
            tx.rollback().await?;
            return Err(QuotaWriteError::NotFound);
        }
        if before.rule.version != expected_version {
            tx.rollback().await?;
            return Err(QuotaWriteError::VersionConflict {
                expected: expected_version,
                actual: before.rule.version,
            });
        }
        let update = self.sql(
            "UPDATE aicc_quota_rule SET deleted=1, enabled=0, version=version+1, updated_by=?, updated_at_ms=?
             WHERE rule_id=?",
        );
        sqlx::query(&update)
            .bind(operator)
            .bind(now_ms)
            .bind(rule_id)
            .execute(&mut *tx)
            .await?;
        if before.rule.enabled {
            if let Some(period) = before.rule.period {
                self.release_counter(&mut tx, &before.scope_key, period, rule_id, now_ms)
                    .await?;
            }
        }
        let after = self
            .stored_rule_tx(&mut tx, rule_id)
            .await?
            .ok_or(QuotaWriteError::NotFound)?;
        self.audit_tx(
            &mut tx,
            "rule",
            rule_id,
            "delete",
            operator,
            Some(serde_json::to_value(&before.rule)?),
            None,
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(after.rule)
    }

    /// Credit `amount` to an account once per `recharge_id`.
    pub(crate) async fn recharge_quota_account(
        &self,
        account_id: &str,
        recharge_id: &str,
        amount: u64,
        note: Option<&str>,
        operator: &str,
        now_ms: i64,
    ) -> Result<(QuotaGrantRecord, bool), QuotaWriteError> {
        let mut tx = self.pool.begin().await?;
        let lock = self.sql("UPDATE aicc_quota_account SET version=version WHERE account_id=?");
        if sqlx::query(&lock)
            .bind(account_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 0
        {
            tx.rollback().await?;
            return Err(QuotaWriteError::NotFound);
        }
        let existing = self.sql("SELECT * FROM aicc_quota_grant WHERE grant_id=?");
        if let Some(row) = sqlx::query(&existing)
            .bind(recharge_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            let grant = grant_from_row(&row)?;
            tx.rollback().await?;
            if grant.account_id == account_id
                && grant.kind == QuotaGrantKind::Recharge
                && grant.amount_tokens == amount
                && grant.note.as_deref() == note
            {
                return Ok((grant, true));
            }
            return Err(QuotaWriteError::RechargeConflict);
        }
        let insert = self.sql(
            "INSERT INTO aicc_quota_grant (grant_id,account_id,kind,amount_tokens,operator,note,created_at_ms)
             VALUES (?,?,'recharge',?,?,?,?)",
        );
        sqlx::query(&insert)
            .bind(recharge_id)
            .bind(account_id)
            .bind(to_i64(amount)?)
            .bind(operator)
            .bind(note)
            .bind(now_ms)
            .execute(&mut *tx)
            .await?;
        let credit = self.sql(
            "UPDATE aicc_quota_account SET granted_tokens=granted_tokens+?, version=version+1, updated_at_ms=?
             WHERE account_id=?",
        );
        sqlx::query(&credit)
            .bind(to_i64(amount)?)
            .bind(now_ms)
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        self.audit_tx(
            &mut tx,
            "account",
            account_id,
            "recharge",
            operator,
            None,
            Some(serde_json::json!({"recharge_id": recharge_id, "amount_tokens": amount, "note": note})),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        let grant = QuotaGrantRecord {
            grant_id: recharge_id.to_string(),
            account_id: account_id.to_string(),
            kind: QuotaGrantKind::Recharge,
            amount_tokens: amount,
            operator: operator.to_string(),
            note: note.map(str::to_string),
            created_at_ms: now_ms,
        };
        Ok((grant, false))
    }

    /// Switch the period of rules whose pending change is due, unless the
    /// target period now collides with another rule (§8).
    async fn apply_due_period_switches(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        now_ms: i64,
    ) -> StorageResult<()> {
        let sql = self.sql(
            "SELECT * FROM aicc_quota_rule WHERE pending_effective_at_ms IS NOT NULL
             AND pending_effective_at_ms<=? AND pending_conflict=0 AND deleted=0",
        );
        let due = sqlx::query(&sql).bind(now_ms).fetch_all(&mut **tx).await?;
        for row in due {
            let stored = rule_from_row(&row)?;
            let Some(pending) = stored.rule.pending_period.clone() else {
                continue;
            };
            if self
                .duplicate_rule(
                    tx,
                    stored.rule.source,
                    &stored.scope_key,
                    QuotaKind::Periodic,
                    Some(pending.period),
                    &stored.rule.rule_id,
                )
                .await?
                .is_some()
            {
                let conflict = self.sql(
                    "UPDATE aicc_quota_rule SET pending_conflict=1, version=version+1 WHERE rule_id=?",
                );
                sqlx::query(&conflict)
                    .bind(&stored.rule.rule_id)
                    .execute(&mut **tx)
                    .await?;
                continue;
            }
            let switch = self.sql(
                "UPDATE aicc_quota_rule SET period=?, pending_period=NULL, pending_effective_at_ms=NULL,
                 pending_conflict=0, version=version+1, updated_at_ms=? WHERE rule_id=?",
            );
            sqlx::query(&switch)
                .bind(pending.period.as_str())
                .bind(now_ms)
                .bind(&stored.rule.rule_id)
                .execute(&mut **tx)
                .await?;
            if stored.rule.enabled {
                if let Some(old) = stored.rule.period {
                    self.release_counter(tx, &stored.scope_key, old, &stored.rule.rule_id, now_ms)
                        .await?;
                }
                if let Some(after) = self.stored_rule_tx(tx, &stored.rule.rule_id).await? {
                    self.refresh_counter(tx, &after, now_ms).await?;
                }
            }
            self.audit_tx(
                tx,
                "rule",
                &stored.rule.rule_id,
                "period_switch",
                "system",
                Some(serde_json::json!({"period": stored.rule.period})),
                Some(serde_json::json!({"period": pending.period})),
                now_ms,
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn quota_apply_due_switches(&self, now_ms: i64) -> StorageResult<()> {
        let mut tx = self.pool.begin().await?;
        let lock = self.sql("UPDATE aicc_state SET int_value=int_value WHERE state_key=?");
        sqlx::query(&lock)
            .bind(VISIBILITY_SEEDED_KEY)
            .execute(&mut *tx)
            .await?;
        self.apply_due_period_switches(&mut tx, now_ms).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn rules_for_call(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        user_id: &str,
        app_id: &str,
        provider_instance_name: &str,
        exact_model: &str,
    ) -> StorageResult<Vec<StoredRule>> {
        let sql = self.sql(
            "SELECT * FROM aicc_quota_rule WHERE deleted=0 AND (
               (object_type='user' AND user_id=?) OR (object_type='app' AND app_id=? AND app_id<>'')
               OR (object_type='user_app' AND user_id=? AND app_id=? AND app_id<>''))
             AND (resource_type='all' OR (resource_type='provider' AND resource_id=?)
               OR (resource_type='model' AND resource_id=?))",
        );
        sqlx::query(&sql)
            .bind(user_id)
            .bind(app_id)
            .bind(user_id)
            .bind(app_id)
            .bind(provider_instance_name)
            .bind(exact_model)
            .fetch_all(&mut **tx)
            .await?
            .iter()
            .map(rule_from_row)
            .collect()
    }

    async fn accounts_for_call(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        scopes: &[String],
    ) -> StorageResult<Vec<AccountRecord>> {
        let marks = vec!["?"; scopes.len()].join(",");
        let sql = self.sql(&format!(
            "SELECT * FROM aicc_quota_account WHERE scope_key IN ({marks})"
        ));
        bind_all!(sqlx::query(&sql), scopes)
            .fetch_all(&mut **tx)
            .await?
            .iter()
            .map(account_from_row)
            .collect()
    }

    /// Reserve `budget_tokens` on every rule, window and account of the
    /// call, or on none of them (§7.1 step 4).
    pub(crate) async fn quota_admit(&self, attempt: &AttemptAdmission) -> StorageResult<AdmitOutcome> {
        for _ in 0..3 {
            if let Some(outcome) = self.try_admit(attempt).await? {
                return Ok(outcome);
            }
        }
        Err(StorageError::InvalidRecord(
            "quota reservation kept racing with concurrent admissions".into(),
        ))
    }

    async fn try_admit(&self, a: &AttemptAdmission) -> StorageResult<Option<AdmitOutcome>> {
        let now = a.now_ms;
        let app_id = a.app_id.clone().unwrap_or_default();
        let mut tx = self.pool.begin().await?;
        let insert = self.sql(
            "INSERT INTO aicc_quota_attempt
             (attempt_id,task_id,user_id,app_id,operator_id,provider_instance_name,exact_model,method,
              admitted_at_ms,budget_tokens,hard_limit,state,charged_tokens,reconcile_deadline_ms,settled_at_ms,
              applied_json,attribution_json)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,NULL,?,NULL,'{}',?) ON CONFLICT(attempt_id) DO NOTHING",
        );
        let inserted = sqlx::query(&insert)
            .bind(&a.attempt_id)
            .bind(&a.task_id)
            .bind(&a.user_id)
            .bind(&app_id)
            .bind(&a.operator_id)
            .bind(&a.provider_instance_name)
            .bind(&a.exact_model)
            .bind(&a.method)
            .bind(now)
            .bind(to_i64(a.budget_tokens)?)
            .bind(i64::from(a.hard_limit))
            .bind(STATE_ADMITTING)
            .bind(a.reconcile_deadline_ms)
            .bind(&a.attribution_json)
            .execute(&mut *tx)
            .await?;
        if inserted.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(Some(AdmitOutcome::Replayed));
        }
        self.apply_due_period_switches(&mut tx, now).await?;
        let rules = self
            .rules_for_call(&mut tx, &a.user_id, &app_id, &a.provider_instance_name, &a.exact_model)
            .await?;
        let scopes = call_scopes(
            &a.user_id,
            a.app_id.as_deref(),
            &a.provider_instance_name,
            &a.exact_model,
        )
        .iter()
        .map(|(object, resource)| scope_key(object, resource))
        .collect::<Vec<_>>();
        let accounts = self.accounts_for_call(&mut tx, &scopes).await?;
        let mut counters: BTreeMap<String, u64> = BTreeMap::new();
        let mut checked = Vec::new();
        for stored in rules.iter().filter(|stored| stored.rule.enabled) {
            match stored.rule.kind {
                QuotaKind::Periodic => {
                    let Some(period) = stored.rule.period else {
                        continue;
                    };
                    let (start, end) = period.window(now);
                    let ledger = self
                        .ensure_counter(
                            &mut tx,
                            &stored.scope_key,
                            &stored.rule.object,
                            &stored.rule.resource,
                            period,
                            start,
                            end,
                            now,
                        )
                        .await?;
                    let limit = stored.rule.limit_tokens.unwrap_or(0);
                    let key = counter_key(&stored.scope_key, period, start);
                    let entry = counters.entry(key).or_insert(limit);
                    *entry = (*entry).min(limit);
                    checked.push(CheckedRule {
                        rule: stored.rule.clone(),
                        state: periodic_state(&stored.rule, period, ledger, now, now),
                    });
                }
                QuotaKind::Total => {
                    let account = accounts
                        .iter()
                        .find(|account| account.scope_key == stored.scope_key);
                    let (granted, ledger, start) = account
                        .map(|account| (account.granted, account.ledger, account.start_at_ms))
                        .unwrap_or((0, Ledger::default(), now));
                    checked.push(CheckedRule {
                        rule: stored.rule.clone(),
                        state: total_state(true, granted, ledger, start, now),
                    });
                }
            }
        }
        if let Some(details) = denial_details(
            &checked,
            a.budget_tokens,
            &a.provider_instance_name,
            &a.exact_model,
        ) {
            tx.rollback().await?;
            return Ok(Some(AdmitOutcome::Denied(details)));
        }
        let budget = to_i64(a.budget_tokens)?;
        let reserve_counter = self.sql(
            "UPDATE aicc_quota_counter SET reserved_tokens=reserved_tokens+?, updated_at_ms=?
             WHERE counter_key=? AND settled_tokens+reserved_tokens+?<=?",
        );
        for (key, limit) in &counters {
            let updated = sqlx::query(&reserve_counter)
                .bind(budget)
                .bind(now)
                .bind(key)
                .bind(budget)
                .bind(to_i64(*limit)?)
                .execute(&mut *tx)
                .await?;
            if updated.rows_affected() != 1 {
                tx.rollback().await?;
                return Ok(None);
            }
        }
        let enforced = rules
            .iter()
            .filter(|stored| stored.rule.enabled && stored.rule.kind == QuotaKind::Total)
            .map(|stored| stored.scope_key.clone())
            .collect::<BTreeSet<_>>();
        let reserve_enforced = self.sql(
            "UPDATE aicc_quota_account SET reserved_tokens=reserved_tokens+?, version=version+1, updated_at_ms=?
             WHERE account_id=? AND settled_tokens+reserved_tokens+?<=granted_tokens",
        );
        let reserve_open = self.sql(
            "UPDATE aicc_quota_account SET reserved_tokens=reserved_tokens+?, version=version+1, updated_at_ms=?
             WHERE account_id=?",
        );
        for account in &accounts {
            let updated = if enforced.contains(&account.scope_key) {
                sqlx::query(&reserve_enforced)
                    .bind(budget)
                    .bind(now)
                    .bind(&account.account_id)
                    .bind(budget)
                    .execute(&mut *tx)
                    .await?
            } else {
                sqlx::query(&reserve_open)
                    .bind(budget)
                    .bind(now)
                    .bind(&account.account_id)
                    .execute(&mut *tx)
                    .await?
            };
            if updated.rows_affected() != 1 {
                tx.rollback().await?;
                return Ok(None);
            }
        }
        let applied = AppliedTargets {
            counters: counters.keys().cloned().collect(),
            accounts: accounts
                .iter()
                .map(|account| account.account_id.clone())
                .collect(),
        };
        let reserve = self.sql("UPDATE aicc_quota_attempt SET state=?, applied_json=? WHERE attempt_id=?");
        sqlx::query(&reserve)
            .bind(STATE_RESERVED)
            .bind(serde_json::to_string(&applied)?)
            .bind(&a.attempt_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(AdmitOutcome::Admitted))
    }

    /// Read-only admission check used to choose among route candidates.
    pub(crate) async fn quota_check(
        &self,
        user_id: &str,
        app_id: Option<&str>,
        provider_instance_name: &str,
        exact_model: &str,
        budget_tokens: u64,
        now_ms: i64,
    ) -> StorageResult<Option<QuotaDenialDetails>> {
        let app = app_id.unwrap_or_default();
        let mut tx = self.pool.begin().await?;
        let rules = self
            .rules_for_call(&mut tx, user_id, app, provider_instance_name, exact_model)
            .await?;
        if !rules.iter().any(|stored| stored.rule.enabled) {
            tx.commit().await?;
            return Ok(None);
        }
        let scopes = call_scopes(user_id, app_id, provider_instance_name, exact_model)
            .iter()
            .map(|(object, resource)| scope_key(object, resource))
            .collect::<Vec<_>>();
        let accounts = self.accounts_for_call(&mut tx, &scopes).await?;
        let mut checked = Vec::new();
        for stored in rules.iter().filter(|stored| stored.rule.enabled) {
            match stored.rule.kind {
                QuotaKind::Periodic => {
                    let Some(period) = stored.rule.period else {
                        continue;
                    };
                    let (start, end) = period.window(now_ms);
                    let read = self.sql("SELECT * FROM aicc_quota_counter WHERE counter_key=?");
                    let ledger = match sqlx::query(&read)
                        .bind(counter_key(&stored.scope_key, period, start))
                        .fetch_optional(&mut *tx)
                        .await?
                    {
                        Some(row) => Ledger {
                            settled: from_i64(row.try_get("settled_tokens")?)?,
                            reserved: from_i64(row.try_get("reserved_tokens")?)?,
                            pending: from_i64(row.try_get("pending_tokens")?)?,
                        },
                        None => {
                            self.sum_window(&mut tx, &stored.rule.object, &stored.rule.resource, start, end)
                                .await?
                        }
                    };
                    checked.push(CheckedRule {
                        rule: stored.rule.clone(),
                        state: periodic_state(&stored.rule, period, ledger, now_ms, now_ms),
                    });
                }
                QuotaKind::Total => {
                    let account = accounts
                        .iter()
                        .find(|account| account.scope_key == stored.scope_key);
                    let (granted, ledger, start) = account
                        .map(|account| (account.granted, account.ledger, account.start_at_ms))
                        .unwrap_or((0, Ledger::default(), now_ms));
                    checked.push(CheckedRule {
                        rule: stored.rule.clone(),
                        state: total_state(true, granted, ledger, start, now_ms),
                    });
                }
            }
        }
        tx.commit().await?;
        Ok(denial_details(
            &checked,
            budget_tokens,
            provider_instance_name,
            exact_model,
        ))
    }

    async fn attempt_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        attempt_id: &str,
    ) -> StorageResult<Option<AttemptRow>> {
        let sql = self.sql("SELECT state,budget_tokens,charged_tokens,applied_json FROM aicc_quota_attempt WHERE attempt_id=?");
        let Some(row) = sqlx::query(&sql)
            .bind(attempt_id)
            .fetch_optional(&mut **tx)
            .await?
        else {
            return Ok(None);
        };
        let applied: String = row.try_get("applied_json")?;
        Ok(Some(AttemptRow {
            state: row.try_get("state")?,
            budget: from_i64(row.try_get("budget_tokens")?)?,
            charged: opt_from_i64(row.try_get("charged_tokens")?)?,
            applied: serde_json::from_str(&applied)?,
        }))
    }

    /// Settle an attempt; repeating a settlement changes nothing (§7.2).
    pub(crate) async fn quota_settle(
        &self,
        attempt_id: &str,
        outcome: SettleOutcome,
        now_ms: i64,
    ) -> StorageResult<()> {
        let mut tx = self.pool.begin().await?;
        let lock = self.sql("UPDATE aicc_quota_attempt SET state=state WHERE attempt_id=?");
        sqlx::query(&lock).bind(attempt_id).execute(&mut *tx).await?;
        let Some(attempt) = self.attempt_tx(&mut tx, attempt_id).await? else {
            tx.commit().await?;
            return Ok(());
        };
        let pending = attempt.state == STATE_PENDING;
        // (settled delta, reserved release, pending release, pending add, next state, charged)
        let plan: Option<(i64, u64, u64, u64, &str, Option<u64>)> =
            match (attempt.state.as_str(), outcome) {
                (STATE_RESERVED | STATE_PENDING, SettleOutcome::Actual(tokens)) => Some((
                    to_i64(tokens)?,
                    attempt.budget,
                    if pending { attempt.budget } else { 0 },
                    0,
                    STATE_SETTLED,
                    Some(tokens),
                )),
                (STATE_RESERVED | STATE_PENDING, SettleOutcome::Release) => Some((
                    0,
                    attempt.budget,
                    if pending { attempt.budget } else { 0 },
                    0,
                    STATE_RELEASED,
                    Some(0),
                )),
                (STATE_RESERVED, SettleOutcome::Unknown) => {
                    Some((0, 0, 0, attempt.budget, STATE_PENDING, None))
                }
                (STATE_RESERVED | STATE_PENDING, SettleOutcome::Estimate) => Some((
                    to_i64(attempt.budget)?,
                    attempt.budget,
                    if pending { attempt.budget } else { 0 },
                    0,
                    STATE_ESTIMATED,
                    Some(attempt.budget),
                )),
                (STATE_ESTIMATED, SettleOutcome::Actual(tokens)) => Some((
                    to_i64(tokens)? - to_i64(attempt.charged.unwrap_or(attempt.budget))?,
                    0,
                    0,
                    0,
                    STATE_SETTLED,
                    Some(tokens),
                )),
                _ => None,
            };
        let Some((settled_delta, release, release_pending, add_pending, next_state, charged)) = plan
        else {
            tx.commit().await?;
            return Ok(());
        };
        let release = to_i64(release)?;
        let release_pending = to_i64(release_pending)?;
        let add_pending = to_i64(add_pending)?;
        for table in ["aicc_quota_counter", "aicc_quota_account"] {
            let (key_column, version) = if table == "aicc_quota_counter" {
                ("counter_key", "")
            } else {
                ("account_id", ", version=version+1")
            };
            let sql = self.sql(&format!(
                "UPDATE {table} SET
                   settled_tokens=CASE WHEN settled_tokens+?<0 THEN 0 ELSE settled_tokens+? END,
                   reserved_tokens=CASE WHEN reserved_tokens<? THEN 0 ELSE reserved_tokens-? END,
                   pending_tokens=CASE WHEN pending_tokens-?+?<0 THEN 0 ELSE pending_tokens-?+? END,
                   updated_at_ms=?{version}
                 WHERE {key_column}=?"
            ));
            let keys = if table == "aicc_quota_counter" {
                &attempt.applied.counters
            } else {
                &attempt.applied.accounts
            };
            for key in keys {
                sqlx::query(&sql)
                    .bind(settled_delta)
                    .bind(settled_delta)
                    .bind(release)
                    .bind(release)
                    .bind(release_pending)
                    .bind(add_pending)
                    .bind(release_pending)
                    .bind(add_pending)
                    .bind(now_ms)
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let update = self.sql(
            "UPDATE aicc_quota_attempt SET state=?, charged_tokens=?, settled_at_ms=? WHERE attempt_id=?",
        );
        sqlx::query(&update)
            .bind(next_state)
            .bind(opt_i64(charged)?)
            .bind((next_state != STATE_PENDING).then_some(now_ms))
            .bind(attempt_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Charge the reservation of attempts whose usage never arrived before
    /// their deadline, and prune what no window can still need.
    pub(crate) async fn quota_reconcile(&self, now_ms: i64) -> StorageResult<usize> {
        let sql = self.sql(
            "SELECT attempt_id FROM aicc_quota_attempt WHERE state IN ('reserved','pending')
             AND reconcile_deadline_ms<=? LIMIT 200",
        );
        let expired: Vec<String> = sqlx::query_scalar(&sql)
            .bind(now_ms)
            .fetch_all(&self.pool)
            .await?;
        for attempt_id in &expired {
            self.quota_settle(attempt_id, SettleOutcome::Estimate, now_ms)
                .await?;
        }
        let horizon = now_ms - QUOTA_LEDGER_RETENTION_MS;
        let prune_attempts = self.sql(
            "DELETE FROM aicc_quota_attempt WHERE state IN ('settled','estimated','released')
             AND admitted_at_ms<?",
        );
        sqlx::query(&prune_attempts)
            .bind(horizon)
            .execute(&self.pool)
            .await?;
        let stale_admitting = self.sql(
            "DELETE FROM aicc_quota_attempt WHERE state=? AND admitted_at_ms<?",
        );
        sqlx::query(&stale_admitting)
            .bind(STATE_ADMITTING)
            .bind(now_ms - QUOTA_LEDGER_RETENTION_MS)
            .execute(&self.pool)
            .await?;
        let prune_counters = self.sql("DELETE FROM aicc_quota_counter WHERE window_end_ms<?");
        sqlx::query(&prune_counters)
            .bind(horizon)
            .execute(&self.pool)
            .await?;
        Ok(expired.len())
    }

    /// Business Apps the user's calls were recorded under.
    pub(crate) async fn usage_apps(&self, user_id: &str) -> StorageResult<BTreeSet<String>> {
        let sql = self.sql(
            "SELECT DISTINCT app_id FROM aicc_quota_attempt WHERE user_id=? AND app_id<>''
             UNION SELECT DISTINCT caller_app_id FROM aicc_usage_event WHERE user_id=? AND caller_app_id IS NOT NULL",
        );
        let apps: Vec<String> = sqlx::query_scalar(&sql)
            .bind(user_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(apps.into_iter().filter(|app| !app.is_empty()).collect())
    }

    #[cfg(test)]
    pub(crate) async fn quota_attempt_state(&self, attempt_id: &str) -> StorageResult<Option<(String, Option<u64>)>> {
        let sql = self.sql("SELECT state, charged_tokens FROM aicc_quota_attempt WHERE attempt_id=?");
        sqlx::query(&sql)
            .bind(attempt_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("state")?,
                    opt_from_i64(row.try_get("charged_tokens")?)?,
                ))
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::QuotaStatus;

    const HOUR: i64 = 60 * 60 * 1000;
    /// 2026-10-20 00:00 UTC (a Tuesday): window boundaries are easy to read.
    const T0: i64 = 20_746 * 24 * HOUR;

    async fn db() -> AiccStorage {
        AiccStorage::open("sqlite::memory:", RdbBackend::Sqlite)
            .await
            .unwrap()
    }

    fn user(id: &str) -> QuotaObject {
        QuotaObject::User {
            user_id: id.into(),
        }
    }

    fn user_app(user: &str, app: &str) -> QuotaObject {
        QuotaObject::UserApp {
            user_id: user.into(),
            app_id: app.into(),
        }
    }

    fn model(name: &str) -> QuotaResource {
        QuotaResource::Model {
            exact_model: name.into(),
        }
    }

    fn attempt(id: &str, user: &str, app: &str, model: &str, budget: u64, now: i64) -> AttemptAdmission {
        let provider = model.rsplit_once('@').unwrap().1.to_string();
        AttemptAdmission {
            attempt_id: id.into(),
            task_id: format!("task-{id}"),
            user_id: user.into(),
            app_id: Some(app.into()),
            operator_id: Some(user.into()),
            provider_instance_name: provider,
            exact_model: model.into(),
            method: "chat.completions.create".into(),
            budget_tokens: budget,
            hard_limit: true,
            now_ms: now,
            reconcile_deadline_ms: now + HOUR,
            attribution_json: None,
        }
    }

    async fn periodic(
        db: &AiccStorage,
        source: QuotaRuleSource,
        object: QuotaObject,
        resource: QuotaResource,
        period: QuotaPeriod,
        limit: u64,
        now: i64,
    ) -> QuotaRule {
        db.create_quota_rule(
            source,
            &object,
            &resource,
            QuotaKind::Periodic,
            Some(period),
            Some(limit),
            0,
            "admin",
            now,
        )
        .await
        .unwrap()
    }

    async fn total(db: &AiccStorage, object: QuotaObject, resource: QuotaResource, initial: u64, now: i64) -> QuotaRule {
        db.create_quota_rule(
            QuotaRuleSource::Admin,
            &object,
            &resource,
            QuotaKind::Total,
            None,
            None,
            initial,
            "admin",
            now,
        )
        .await
        .unwrap()
    }

    async fn state(db: &AiccStorage, rule_id: &str, now: i64) -> QuotaUsageState {
        let rule = db.quota_rule(rule_id).await.unwrap().unwrap();
        db.quota_rule_state(&rule, now).await.unwrap()
    }

    #[tokio::test]
    async fn every_matching_rule_is_charged_and_one_shortfall_blocks_all() {
        // Q04: user Provider rule + shared App rule + own App limit + model rule.
        let db = db().await;
        let provider = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::Provider {
            provider_instance_name: "cloud".into(),
        }, QuotaPeriod::Day, 200_000, T0).await;
        let app = periodic(&db, QuotaRuleSource::Admin, QuotaObject::App { app_id: "writer".into() }, QuotaResource::All, QuotaPeriod::Week, 500_000, T0).await;
        let own = periodic(&db, QuotaRuleSource::User, user_app("alice", "writer"), QuotaResource::All, QuotaPeriod::FourHours, 30_000, T0).await;
        let exact = periodic(&db, QuotaRuleSource::Admin, user("alice"), model("m@cloud"), QuotaPeriod::FourHours, 50_000, T0).await;
        let admitted = db.quota_admit(&attempt("a1", "alice", "writer", "m@cloud", 20_000, T0 + 1)).await.unwrap();
        assert_eq!(admitted, AdmitOutcome::Admitted);
        db.quota_settle("a1", SettleOutcome::Actual(12_000), T0 + 2).await.unwrap();
        for rule in [&provider, &app, &own, &exact] {
            assert_eq!(state(&db, &rule.rule_id, T0 + 3).await.settled_tokens, 12_000);
        }
        // Own 4h limit has 18_000 left: a 20_000 budget is refused by it alone.
        let denied = db.quota_admit(&attempt("a2", "alice", "writer", "m@cloud", 20_000, T0 + 4)).await.unwrap();
        let AdmitOutcome::Denied(details) = denied else { panic!("expected denial") };
        assert_eq!(details.blocking.len(), 1);
        assert_eq!(details.blocking[0].rule_id, own.rule_id);
        assert_eq!(details.max_allowed_tokens, 18_000);
        // No partial reservation was left behind.
        assert_eq!(state(&db, &provider.rule_id, T0 + 5).await.reserved_tokens, 0);
        assert_eq!(db.quota_attempt_state("a2").await.unwrap(), None);
        // Bob's calls do not touch Alice's rules but share the App rule.
        db.quota_admit(&attempt("b1", "bob", "writer", "m@cloud", 1_000, T0 + 6)).await.unwrap();
        db.quota_settle("b1", SettleOutcome::Actual(1_000), T0 + 7).await.unwrap();
        assert_eq!(state(&db, &provider.rule_id, T0 + 8).await.settled_tokens, 12_000);
        assert_eq!(state(&db, &app.rule_id, T0 + 8).await.settled_tokens, 13_000);
    }

    #[tokio::test]
    async fn periodic_windows_reset_independently_and_keep_the_admission_window() {
        // Q05/Q08: 4h and weekly rules; an attempt settles into its admission window.
        let db = db().await;
        let short = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::FourHours, 10_000, T0).await;
        let week = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Week, 15_000, T0).await;
        db.quota_admit(&attempt("a1", "alice", "app", "m@cloud", 8_000, T0 + 4 * HOUR - 10)).await.unwrap();
        db.quota_settle("a1", SettleOutcome::Actual(9_000), T0 + 4 * HOUR + 10).await.unwrap();
        assert_eq!(state(&db, &short.rule_id, T0 + 4 * HOUR - 1).await.settled_tokens, 9_000);
        assert_eq!(state(&db, &short.rule_id, T0 + 4 * HOUR).await.settled_tokens, 0);
        assert_eq!(state(&db, &week.rule_id, T0 + 4 * HOUR).await.settled_tokens, 9_000);
        // The 4h window recovered, the weekly one did not.
        let AdmitOutcome::Denied(details) = db.quota_admit(&attempt("a2", "alice", "app", "m@cloud", 7_000, T0 + 5 * HOUR)).await.unwrap() else {
            panic!("weekly rule must block");
        };
        assert_eq!(details.blocking[0].rule_id, week.rule_id);
        assert_eq!(details.reset_at_ms, Some(QuotaPeriod::Week.window(T0).1));
    }

    #[tokio::test]
    async fn new_and_lowered_rules_count_existing_use_and_in_flight_reservations() {
        // Q12: 60_000 used before the rule exists; a new 50_000 rule blocks.
        let db = db().await;
        periodic(&db, QuotaRuleSource::Admin, user("carol"), QuotaResource::All, QuotaPeriod::Week, 1_000_000, T0).await;
        db.quota_admit(&attempt("a1", "alice", "app", "m@cloud", 60_000, T0 + 1)).await.unwrap();
        db.quota_settle("a1", SettleOutcome::Actual(60_000), T0 + 2).await.unwrap();
        db.quota_admit(&attempt("a2", "alice", "app", "m@cloud", 5_000, T0 + 3)).await.unwrap();
        let rule = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Day, 50_000, T0 + 4).await;
        let current = state(&db, &rule.rule_id, T0 + 5).await;
        assert_eq!((current.settled_tokens, current.reserved_tokens), (60_000, 5_000));
        assert_eq!(current.status, QuotaStatus::Exhausted);
        assert!(matches!(
            db.quota_admit(&attempt("a3", "alice", "app", "m@cloud", 1, T0 + 6)).await.unwrap(),
            AdmitOutcome::Denied(_)
        ));
        // The in-flight attempt settles into the counter created after it.
        db.quota_settle("a2", SettleOutcome::Actual(1_000), T0 + 7).await.unwrap();
        let current = state(&db, &rule.rule_id, T0 + 8).await;
        assert_eq!((current.settled_tokens, current.reserved_tokens), (61_000, 0));
        // Raising the limit restores admission without clearing the history.
        db.update_quota_rule(&rule.rule_id, rule.version, Some(100_000), None, None, false, "admin", T0 + 9)
            .await
            .unwrap();
        assert_eq!(
            db.quota_admit(&attempt("a4", "alice", "app", "m@cloud", 30_000, T0 + 10)).await.unwrap(),
            AdmitOutcome::Admitted
        );
    }

    #[tokio::test]
    async fn disabling_and_reenabling_recounts_the_current_window() {
        // Q13: no implicit reset when a rule is disabled, re-enabled or rebuilt.
        let db = db().await;
        let rule = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Day, 50_000, T0).await;
        let disabled = db
            .update_quota_rule(&rule.rule_id, rule.version, None, None, Some(false), false, "admin", T0 + 1)
            .await
            .unwrap();
        db.quota_admit(&attempt("a1", "alice", "app", "m@cloud", 40_000, T0 + 2)).await.unwrap();
        db.quota_settle("a1", SettleOutcome::Actual(40_000), T0 + 3).await.unwrap();
        let enabled = db
            .update_quota_rule(&rule.rule_id, disabled.version, None, None, Some(true), false, "admin", T0 + 4)
            .await
            .unwrap();
        assert_eq!(state(&db, &rule.rule_id, T0 + 5).await.settled_tokens, 40_000);
        db.delete_quota_rule(&rule.rule_id, enabled.version, "admin", T0 + 6).await.unwrap();
        let rebuilt = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Day, 50_000, T0 + 7).await;
        assert_eq!(state(&db, &rebuilt.rule_id, T0 + 8).await.settled_tokens, 40_000);
        // A stale edit is refused.
        assert!(matches!(
            db.update_quota_rule(&rule.rule_id, rule.version, Some(1), None, None, false, "admin", T0 + 9).await,
            Err(QuotaWriteError::NotFound)
        ));
        assert!(matches!(
            db.update_quota_rule(&rebuilt.rule_id, rebuilt.version + 5, Some(1), None, None, false, "admin", T0 + 9).await,
            Err(QuotaWriteError::VersionConflict { .. })
        ));
    }

    #[tokio::test]
    async fn period_changes_wait_for_the_window_end_and_conflicts_keep_the_old_rule() {
        let db = db().await;
        let rule = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::FourHours, 50_000, T0).await;
        let other = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Week, 90_000, T0).await;
        assert!(matches!(
            db.update_quota_rule(&rule.rule_id, rule.version, None, Some(QuotaPeriod::Week), None, false, "admin", T0 + 1).await,
            Err(QuotaWriteError::Duplicate(id)) if id == other.rule_id
        ));
        let pending = db
            .update_quota_rule(&rule.rule_id, rule.version, None, Some(QuotaPeriod::Day), None, false, "admin", T0 + 1)
            .await
            .unwrap();
        assert_eq!(pending.period, Some(QuotaPeriod::FourHours));
        assert_eq!(pending.pending_period.as_ref().unwrap().effective_at_ms, T0 + 4 * HOUR);
        db.quota_apply_due_switches(T0 + 4 * HOUR).await.unwrap();
        let switched = db.quota_rule(&rule.rule_id).await.unwrap().unwrap();
        assert_eq!(switched.period, Some(QuotaPeriod::Day));
        assert!(switched.pending_period.is_none());
    }

    #[tokio::test]
    async fn total_accounts_never_reset_and_recharge_pays_debt_first() {
        // Q28/Q29/Q34.
        let db = db().await;
        let rule = total(&db, user("alice"), QuotaResource::All, 100_000, T0).await;
        let account_id = rule.account_id.clone().unwrap();
        db.quota_admit(&attempt("a1", "alice", "app", "m@cloud", 60_000, T0 + 1)).await.unwrap();
        db.quota_settle("a1", SettleOutcome::Actual(60_000), T0 + 2).await.unwrap();
        db.quota_admit(&attempt("a2", "alice", "app", "m@cloud", 10_000, T0 + 3)).await.unwrap();
        let (_, replayed) = db
            .recharge_quota_account(&account_id, "r-1", 50_000, Some("top up"), "admin", T0 + 4)
            .await
            .unwrap();
        assert!(!replayed);
        let after = state(&db, &rule.rule_id, T0 + 5).await;
        assert_eq!(after.granted_tokens, Some(150_000));
        assert_eq!(after.available_tokens, 80_000);
        assert_eq!(after.reserved_tokens, 10_000);
        assert_eq!(after.reset_at_ms, None);
        // Same id, same parameters: credited once; different parameters: refused.
        let (_, replayed) = db
            .recharge_quota_account(&account_id, "r-1", 50_000, Some("top up"), "admin", T0 + 6)
            .await
            .unwrap();
        assert!(replayed);
        assert!(matches!(
            db.recharge_quota_account(&account_id, "r-1", 60_000, None, "admin", T0 + 6).await,
            Err(QuotaWriteError::RechargeConflict)
        ));
        assert_eq!(state(&db, &rule.rule_id, T0 + 7).await.granted_tokens, Some(150_000));
        // Overspend leaves a debt that the next recharge pays first.
        db.quota_settle("a2", SettleOutcome::Actual(100_000), T0 + 8).await.unwrap();
        let debt = state(&db, &rule.rule_id, T0 + 9).await;
        assert_eq!((debt.available_tokens, debt.debt_tokens), (0, 10_000));
        db.recharge_quota_account(&account_id, "r-2", 5_000, None, "admin", T0 + 10).await.unwrap();
        assert_eq!(state(&db, &rule.rule_id, T0 + 11).await.available_tokens, 0);
        db.recharge_quota_account(&account_id, "r-3", 20_000, None, "admin", T0 + 12).await.unwrap();
        assert_eq!(state(&db, &rule.rule_id, T0 + 13).await.available_tokens, 15_000);
        // Days later nothing came back by itself.
        assert_eq!(state(&db, &rule.rule_id, T0 + 30 * 24 * HOUR).await.available_tokens, 15_000);
    }

    #[tokio::test]
    async fn total_account_starts_now_keeps_counting_while_deleted_and_is_restored() {
        // Q33.
        let db = db().await;
        db.quota_admit(&attempt("before", "alice", "app", "m@cloud", 5_000, T0)).await.unwrap();
        let rule = total(&db, user("alice"), QuotaResource::All, 10_000, T0 + 1).await;
        db.quota_settle("before", SettleOutcome::Actual(5_000), T0 + 2).await.unwrap();
        assert_eq!(state(&db, &rule.rule_id, T0 + 3).await.settled_tokens, 0);
        db.delete_quota_rule(&rule.rule_id, rule.version, "admin", T0 + 4).await.unwrap();
        db.quota_admit(&attempt("during", "alice", "app", "m@cloud", 20_000, T0 + 5)).await.unwrap();
        db.quota_settle("during", SettleOutcome::Actual(20_000), T0 + 6).await.unwrap();
        let rebuilt = total(&db, user("alice"), QuotaResource::All, 99_999, T0 + 7).await;
        assert_eq!(rebuilt.account_id, rule.account_id);
        let restored = state(&db, &rebuilt.rule_id, T0 + 8).await;
        assert_eq!(restored.granted_tokens, Some(10_000));
        assert_eq!(restored.settled_tokens, 20_000);
        assert_eq!(restored.debt_tokens, 10_000);
        assert!(matches!(
            db.quota_admit(&attempt("after", "alice", "app", "m@cloud", 1, T0 + 9)).await.unwrap(),
            AdmitOutcome::Denied(details) if details.reset_at_ms.is_none()
        ));
    }

    #[tokio::test]
    async fn settlement_is_idempotent_and_unknown_usage_is_estimated_then_corrected() {
        // Q08/Q09/Q15.
        let db = db().await;
        let rule = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Week, 100_000, T0).await;
        db.quota_admit(&attempt("a1", "alice", "app", "m@cloud", 8_000, T0)).await.unwrap();
        assert_eq!(
            db.quota_admit(&attempt("a1", "alice", "app", "m@cloud", 8_000, T0)).await.unwrap(),
            AdmitOutcome::Replayed
        );
        db.quota_settle("a1", SettleOutcome::Actual(3_000), T0 + 1).await.unwrap();
        db.quota_settle("a1", SettleOutcome::Actual(3_000), T0 + 2).await.unwrap();
        assert_eq!(state(&db, &rule.rule_id, T0 + 3).await.settled_tokens, 3_000);
        db.quota_admit(&attempt("a2", "alice", "app", "m@cloud", 9_000, T0 + 4)).await.unwrap();
        db.quota_settle("a2", SettleOutcome::Unknown, T0 + 5).await.unwrap();
        let pending = state(&db, &rule.rule_id, T0 + 6).await;
        assert_eq!((pending.reserved_tokens, pending.pending_tokens), (9_000, 9_000));
        assert_eq!(db.quota_reconcile(T0 + 4 + HOUR).await.unwrap(), 1);
        let estimated = state(&db, &rule.rule_id, T0 + 2 * HOUR).await;
        assert_eq!((estimated.settled_tokens, estimated.reserved_tokens, estimated.pending_tokens), (12_000, 0, 0));
        db.quota_settle("a2", SettleOutcome::Actual(4_000), T0 + 3 * HOUR).await.unwrap();
        assert_eq!(state(&db, &rule.rule_id, T0 + 3 * HOUR).await.settled_tokens, 7_000);
        // A released attempt costs nothing.
        db.quota_admit(&attempt("a3", "alice", "app", "m@cloud", 9_000, T0 + 3 * HOUR)).await.unwrap();
        db.quota_settle("a3", SettleOutcome::Release, T0 + 3 * HOUR + 1).await.unwrap();
        let released = state(&db, &rule.rule_id, T0 + 3 * HOUR + 2).await;
        assert_eq!((released.settled_tokens, released.reserved_tokens), (7_000, 0));
    }

    #[tokio::test]
    async fn concurrent_admissions_never_overdraw() {
        // Q07: two 8_000 requests on 10_000 left; at most one is admitted.
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("quota.db").display());
        let db = std::sync::Arc::new(AiccStorage::open(&url, RdbBackend::Sqlite).await.unwrap());
        let rule = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Day, 10_000, T0).await;
        let mut handles = Vec::new();
        for index in 0..6 {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                db.quota_admit(&attempt(&format!("c{index}"), "alice", "app", "m@cloud", 8_000, T0 + 1))
                    .await
                    .unwrap()
            }));
        }
        let mut admitted = 0;
        for handle in handles {
            if handle.await.unwrap() == AdmitOutcome::Admitted {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 1);
        assert_eq!(state(&db, &rule.rule_id, T0 + 2).await.reserved_tokens, 8_000);
    }

    #[tokio::test]
    async fn zero_limit_forbids_and_budget_above_a_whole_period_has_no_reset() {
        // Q16.
        let db = db().await;
        let zero = periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Day, 0, T0).await;
        assert_eq!(state(&db, &zero.rule_id, T0).await.status, QuotaStatus::Forbidden);
        let AdmitOutcome::Denied(details) = db.quota_admit(&attempt("a1", "alice", "app", "m@cloud", 1, T0)).await.unwrap() else {
            panic!("zero limit must block");
        };
        assert_eq!(details.reset_at_ms, None);
        let db = super::tests::db().await;
        periodic(&db, QuotaRuleSource::Admin, user("alice"), QuotaResource::All, QuotaPeriod::Day, 1_000, T0).await;
        let AdmitOutcome::Denied(details) = db.quota_admit(&attempt("a2", "alice", "app", "m@cloud", 5_000, T0)).await.unwrap() else {
            panic!("budget above the period must block");
        };
        assert_eq!(details.max_allowed_tokens, 1_000);
        assert_eq!(details.reset_at_ms, None);
    }

    #[tokio::test]
    async fn default_visibility_is_seeded_once_and_updates_are_versioned() {
        let db = db().await;
        let (revision, policy) = db.visibility_policy().await.unwrap();
        assert_eq!(revision, 1);
        assert_eq!(
            policy.iter().map(|item| item.subject.as_str()).collect::<Vec<_>>(),
            vec!["type:admin", "type:user"]
        );
        let revision = db
            .update_visibility(1, "type:user", &[], "admin", T0)
            .await
            .unwrap();
        assert_eq!(revision, 2);
        assert!(matches!(
            db.update_visibility(1, "type:admin", &[], "admin", T0).await,
            Err(QuotaWriteError::VersionConflict { expected: 1, actual: 2 })
        ));
        db.seed_default_visibility().await.unwrap();
        let (_, policy) = db.visibility_policy().await.unwrap();
        assert_eq!(policy.len(), 1, "a removed default is not restored");
    }
}
