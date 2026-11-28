//! Durable user policies, price list and Credit ledger
//! (doc/aicc/aicc_user_policy.md §5).
//!
//! Admission never writes. Settlement writes one charge per upstream
//! attempt, moves the user's balance and adds the charge to the counters the
//! attempt was admitted under, all in one transaction. Every write
//! transaction starts with a write so that SQLite takes its write lock before
//! reading.

use super::*;
use crate::policy::pricing::StoredPrice;
use crate::policy::{ChargePlan, CounterScope, PriceList};
use buckyos_api::{
    format_credit_micro, AiccPolicy, CreditCharge, CreditChargeStatus, CreditEntry,
    CreditEntryKind, CreditLedgerItem, PolicyAuditRecord, PolicyBody, PolicySource,
    PolicySubject,
};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const POLICY_SCHEMA: &str = include_str!("policy.sql");

const POLICY_SEEDED_KEY: &str = "policy_seeded";
const POLICY_REVISION_KEY: &str = "policy_revision";
const PRICE_REVISION_KEY: &str = "price_revision";

/// Tables of schema version 3 the 3 → 4 upgrade removes; their rules,
/// token accounts and session route history are not migrated.
pub(super) const V3_TABLES: [&str; 7] = [
    "aicc_session_route_history",
    "aicc_quota_rule",
    "aicc_quota_counter",
    "aicc_quota_account",
    "aicc_quota_grant",
    "aicc_quota_attempt",
    "aicc_visibility_policy",
];

static POLICY_ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn new_policy_id(prefix: &str, salt: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = POLICY_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let digest = Sha256::digest(format!("{nanos}:{sequence}:{salt}").as_bytes());
    format!(
        "{prefix}{}",
        digest[..10]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[derive(Debug)]
pub(crate) enum PolicyWriteError {
    NotFound,
    VersionConflict { expected: u64, actual: u64 },
    /// Same idempotency id with different parameters.
    IdempotencyConflict,
    Storage(StorageError),
}

impl From<StorageError> for PolicyWriteError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for PolicyWriteError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::Database(error))
    }
}

impl From<serde_json::Error> for PolicyWriteError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(StorageError::Json(error))
    }
}

/// The current window of one counter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CounterState {
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub used_credit_micro: i64,
    pub used_tokens: i64,
}

impl CounterState {
    /// The window is open at `now_ms`; an expired window counts as unused.
    pub(crate) fn active_at(&self, now_ms: i64) -> bool {
        now_ms >= self.window_start_ms && now_ms < self.window_end_ms
    }
}

/// What settlement adds for one attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChargeOutcome {
    pub credit_micro: i64,
    pub tokens: u64,
    pub status: CreditChargeStatus,
}

/// A credit entry application request (recharge or adjustment).
pub(crate) struct CreditEntryDraft<'a> {
    pub user_id: &'a str,
    pub request_id: &'a str,
    pub kind: CreditEntryKind,
    pub amount_micro: i64,
    pub note: Option<&'a str>,
    pub external_ref: Option<&'a Value>,
    pub operator: &'a str,
    pub now_ms: i64,
}

fn kind_name(kind: CreditEntryKind) -> &'static str {
    match kind {
        CreditEntryKind::Recharge => "recharge",
        CreditEntryKind::Adjust => "adjust",
    }
}

fn policy_from_row(row: &AnyRow) -> StorageResult<AiccPolicy> {
    let source: String = row.try_get("source")?;
    let subject: String = row.try_get("subject")?;
    let body: String = row.try_get("body_json")?;
    Ok(AiccPolicy {
        policy_id: row.try_get("policy_id")?,
        source: PolicySource::parse(&source)
            .ok_or_else(|| StorageError::InvalidRecord(format!("policy source {source}")))?,
        subject: PolicySubject::parse(&subject)
            .ok_or_else(|| StorageError::InvalidRecord(format!("policy subject {subject}")))?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        body: serde_json::from_str(&body)?,
        version: from_i64(row.try_get("version")?)?,
        created_by: row.try_get("created_by")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_by: row.try_get("updated_by")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn entry_from_row(row: &AnyRow) -> StorageResult<CreditEntry> {
    let kind: String = row.try_get("kind")?;
    let external: Option<String> = row.try_get("external_ref_json")?;
    Ok(CreditEntry {
        entry_id: row.try_get("entry_id")?,
        user_id: row.try_get("user_id")?,
        kind: match kind.as_str() {
            "recharge" => CreditEntryKind::Recharge,
            "adjust" => CreditEntryKind::Adjust,
            other => return Err(StorageError::InvalidRecord(format!("credit entry kind {other}"))),
        },
        amount: format_credit_micro(row.try_get("amount_micro")?),
        balance_after: format_credit_micro(row.try_get("balance_after_micro")?),
        recharge_id: Some(row.try_get("request_id")?),
        external_ref: external.map(|value| serde_json::from_str(&value)).transpose()?,
        operator: row.try_get("operator")?,
        note: row.try_get("note")?,
        created_at_ms: row.try_get("created_at_ms")?,
    })
}

fn charge_from_row(row: &AnyRow) -> StorageResult<CreditCharge> {
    let status: String = row.try_get("status")?;
    Ok(CreditCharge {
        attempt_id: row.try_get("attempt_id")?,
        task_id: row.try_get("task_id")?,
        user_id: row.try_get("user_id")?,
        app_id: row.try_get("app_id")?,
        operator_id: row.try_get("operator_id")?,
        provider_instance_name: row.try_get("provider_instance_name")?,
        exact_model: row.try_get("exact_model")?,
        admitted_at_ms: row.try_get("admitted_at_ms")?,
        tokens: from_i64(row.try_get("tokens")?)?,
        amount: format_credit_micro(row.try_get("credit_micro")?),
        status: CreditChargeStatus::parse(&status)
            .ok_or_else(|| StorageError::InvalidRecord(format!("charge status {status}")))?,
        settled_at_ms: row.try_get("settled_at_ms")?,
    })
}

fn counter_from_row(row: &AnyRow) -> StorageResult<CounterState> {
    Ok(CounterState {
        window_start_ms: row.try_get("window_start_ms")?,
        window_end_ms: row.try_get("window_end_ms")?,
        used_credit_micro: row.try_get("used_credit_micro")?,
        used_tokens: row.try_get("used_tokens")?,
    })
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
    /// The default policies of a new Zone (§3.1): `type:admin` and
    /// `type:user` see everything for free. Written once; afterwards they are
    /// ordinary policies.
    pub(super) async fn seed_default_policies(&self) -> StorageResult<()> {
        let mut tx = self.pool.begin().await?;
        let insert = self.sql(
            "INSERT INTO aicc_state (state_key,int_value,updated_at_ms) VALUES (?,1,0) ON CONFLICT(state_key) DO NOTHING",
        );
        let seeded = sqlx::query(&insert)
            .bind(POLICY_SEEDED_KEY)
            .execute(&mut *tx)
            .await?;
        if seeded.rows_affected() == 1 {
            let body = serde_json::to_string(&PolicyBody {
                all_providers: Some(true),
                price_coefficient: Some(0.0),
                ..PolicyBody::default()
            })?;
            let sql = self.sql(
                "INSERT INTO aicc_policy (policy_id,source,subject,body_json,enabled,version,
                 created_by,created_at_ms,updated_by,updated_at_ms)
                 VALUES (?,'admin',?,?,1,1,'system',0,'system',0) ON CONFLICT DO NOTHING",
            );
            for subject in ["type:admin", "type:user"] {
                sqlx::query(&sql)
                    .bind(new_policy_id("pol_", subject))
                    .bind(subject)
                    .bind(&body)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    async fn bump_state(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        key: &str,
        now_ms: i64,
    ) -> StorageResult<u64> {
        let bump = self.sql(
            "INSERT INTO aicc_state (state_key,int_value,updated_at_ms) VALUES (?,1,?)
             ON CONFLICT(state_key) DO UPDATE SET int_value=aicc_state.int_value+1, updated_at_ms=excluded.updated_at_ms",
        );
        sqlx::query(&bump)
            .bind(key)
            .bind(now_ms)
            .execute(&mut **tx)
            .await?;
        let read = self.sql("SELECT int_value FROM aicc_state WHERE state_key=?");
        let value: i64 = sqlx::query_scalar(&read)
            .bind(key)
            .fetch_one(&mut **tx)
            .await?;
        from_i64(value)
    }

    async fn state_value(&self, key: &str) -> StorageResult<u64> {
        let sql = self.sql("SELECT int_value FROM aicc_state WHERE state_key=?");
        let value: Option<i64> = sqlx::query_scalar(&sql)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        from_i64(value.unwrap_or(0))
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
            .bind(new_policy_id("qx_", target_id))
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

    pub(crate) async fn policy_audit(&self, target_id: &str) -> StorageResult<Vec<PolicyAuditRecord>> {
        let sql = self.sql(
            "SELECT * FROM aicc_quota_audit WHERE target_id=? ORDER BY created_at_ms DESC LIMIT 200",
        );
        let rows = sqlx::query(&sql).bind(target_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                let before: Option<String> = row.try_get("before_json")?;
                let after: Option<String> = row.try_get("after_json")?;
                Ok(PolicyAuditRecord {
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

    pub(crate) async fn policies(&self) -> StorageResult<Vec<AiccPolicy>> {
        sqlx::query("SELECT * FROM aicc_policy ORDER BY created_at_ms, subject, source")
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(policy_from_row)
            .collect()
    }

    pub(crate) async fn policy(&self, policy_id: &str) -> StorageResult<Option<AiccPolicy>> {
        let sql = self.sql("SELECT * FROM aicc_policy WHERE policy_id=?");
        sqlx::query(&sql)
            .bind(policy_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(policy_from_row)
            .transpose()
    }

    /// Create the policy of `(source, subject)` or replace it when it is
    /// still at `expected_version`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn upsert_policy(
        &self,
        source: PolicySource,
        subject: &PolicySubject,
        enabled: bool,
        body: &PolicyBody,
        expected_version: Option<u64>,
        operator: &str,
        now_ms: i64,
    ) -> Result<AiccPolicy, PolicyWriteError> {
        let mut tx = self.pool.begin().await?;
        self.bump_state(&mut tx, POLICY_REVISION_KEY, now_ms).await?;
        let read = self.sql("SELECT * FROM aicc_policy WHERE source=? AND subject=?");
        let existing = sqlx::query(&read)
            .bind(source.as_str())
            .bind(subject.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .as_ref()
            .map(policy_from_row)
            .transpose()?;
        let body_json = serde_json::to_string(body)?;
        let policy = match existing {
            Some(existing) => {
                if expected_version != Some(existing.version) {
                    tx.rollback().await?;
                    return Err(PolicyWriteError::VersionConflict {
                        expected: expected_version.unwrap_or(0),
                        actual: existing.version,
                    });
                }
                let update = self.sql(
                    "UPDATE aicc_policy SET body_json=?, enabled=?, version=version+1, updated_by=?,
                     updated_at_ms=? WHERE policy_id=? AND version=?",
                );
                sqlx::query(&update)
                    .bind(&body_json)
                    .bind(i64::from(enabled))
                    .bind(operator)
                    .bind(now_ms)
                    .bind(&existing.policy_id)
                    .bind(to_i64(existing.version)?)
                    .execute(&mut *tx)
                    .await?;
                let next = AiccPolicy {
                    enabled,
                    body: body.clone(),
                    version: existing.version + 1,
                    updated_by: operator.to_string(),
                    updated_at_ms: now_ms,
                    ..existing.clone()
                };
                self.audit_tx(
                    &mut tx,
                    "policy",
                    &existing.policy_id,
                    "update",
                    operator,
                    Some(serde_json::to_value(&existing)?),
                    Some(serde_json::to_value(&next)?),
                    now_ms,
                )
                .await?;
                next
            }
            None => {
                if expected_version.is_some() {
                    tx.rollback().await?;
                    return Err(PolicyWriteError::NotFound);
                }
                let policy = AiccPolicy {
                    policy_id: new_policy_id("pol_", &subject.to_string()),
                    source,
                    subject: subject.clone(),
                    enabled,
                    body: body.clone(),
                    version: 1,
                    created_by: operator.to_string(),
                    created_at_ms: now_ms,
                    updated_by: operator.to_string(),
                    updated_at_ms: now_ms,
                };
                let insert = self.sql(
                    "INSERT INTO aicc_policy (policy_id,source,subject,body_json,enabled,version,
                     created_by,created_at_ms,updated_by,updated_at_ms) VALUES (?,?,?,?,?,1,?,?,?,?)",
                );
                sqlx::query(&insert)
                    .bind(&policy.policy_id)
                    .bind(source.as_str())
                    .bind(subject.to_string())
                    .bind(&body_json)
                    .bind(i64::from(enabled))
                    .bind(operator)
                    .bind(now_ms)
                    .bind(operator)
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await?;
                self.audit_tx(
                    &mut tx,
                    "policy",
                    &policy.policy_id,
                    "create",
                    operator,
                    None,
                    Some(serde_json::to_value(&policy)?),
                    now_ms,
                )
                .await?;
                policy
            }
        };
        tx.commit().await?;
        Ok(policy)
    }

    pub(crate) async fn delete_policy(
        &self,
        policy_id: &str,
        expected_version: u64,
        operator: &str,
        now_ms: i64,
    ) -> Result<(), PolicyWriteError> {
        let mut tx = self.pool.begin().await?;
        self.bump_state(&mut tx, POLICY_REVISION_KEY, now_ms).await?;
        let read = self.sql("SELECT * FROM aicc_policy WHERE policy_id=?");
        let Some(existing) = sqlx::query(&read)
            .bind(policy_id)
            .fetch_optional(&mut *tx)
            .await?
            .as_ref()
            .map(policy_from_row)
            .transpose()?
        else {
            tx.rollback().await?;
            return Err(PolicyWriteError::NotFound);
        };
        if existing.version != expected_version {
            tx.rollback().await?;
            return Err(PolicyWriteError::VersionConflict {
                expected: expected_version,
                actual: existing.version,
            });
        }
        let delete = self.sql("DELETE FROM aicc_policy WHERE policy_id=?");
        sqlx::query(&delete).bind(policy_id).execute(&mut *tx).await?;
        self.audit_tx(
            &mut tx,
            "policy",
            policy_id,
            "delete",
            operator,
            Some(serde_json::to_value(&existing)?),
            None,
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn price_list(&self) -> StorageResult<PriceList> {
        let revision = self.state_value(PRICE_REVISION_KEY).await?;
        let mut entries = BTreeMap::new();
        for row in sqlx::query("SELECT * FROM aicc_price_entry")
            .fetch_all(&self.pool)
            .await?
        {
            let pricing: String = row.try_get("pricing_json")?;
            let entry = StoredPrice {
                entry_key: row.try_get("entry_key")?,
                pricing: serde_json::from_str(&pricing)?,
                note: row.try_get("note")?,
                updated_by: row.try_get("updated_by")?,
                updated_at_ms: row.try_get("updated_at_ms")?,
            };
            entries.insert(entry.entry_key.clone(), entry);
        }
        let mut credit_rates = BTreeMap::new();
        for row in sqlx::query("SELECT currency,credit_per_unit FROM aicc_price_rate")
            .fetch_all(&self.pool)
            .await?
        {
            credit_rates.insert(row.try_get("currency")?, row.try_get("credit_per_unit")?);
        }
        Ok(PriceList {
            revision,
            entries,
            credit_rates,
        })
    }

    /// Apply price list changes when it is still at `expected_revision`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn update_prices(
        &self,
        expected_revision: u64,
        upserts: &[StoredPrice],
        deletes: &[String],
        credit_rates: Option<&BTreeMap<String, f64>>,
        operator: &str,
        now_ms: i64,
    ) -> Result<u64, PolicyWriteError> {
        let mut tx = self.pool.begin().await?;
        let revision = self.bump_state(&mut tx, PRICE_REVISION_KEY, now_ms).await?;
        if revision != expected_revision.saturating_add(1) {
            tx.rollback().await?;
            return Err(PolicyWriteError::VersionConflict {
                expected: expected_revision,
                actual: revision.saturating_sub(1),
            });
        }
        let delete = self.sql("DELETE FROM aicc_price_entry WHERE entry_key=?");
        for key in deletes {
            sqlx::query(&delete).bind(key).execute(&mut *tx).await?;
        }
        let upsert = self.sql(
            "INSERT INTO aicc_price_entry (entry_key,pricing_json,note,updated_by,updated_at_ms)
             VALUES (?,?,?,?,?) ON CONFLICT(entry_key) DO UPDATE SET pricing_json=excluded.pricing_json,
             note=excluded.note, updated_by=excluded.updated_by, updated_at_ms=excluded.updated_at_ms",
        );
        for entry in upserts {
            sqlx::query(&upsert)
                .bind(&entry.entry_key)
                .bind(serde_json::to_string(&entry.pricing)?)
                .bind(&entry.note)
                .bind(operator)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(rates) = credit_rates {
            sqlx::query("DELETE FROM aicc_price_rate").execute(&mut *tx).await?;
            let insert = self.sql("INSERT INTO aicc_price_rate (currency,credit_per_unit) VALUES (?,?)");
            for (currency, rate) in rates {
                sqlx::query(&insert)
                    .bind(currency)
                    .bind(*rate)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        self.audit_tx(
            &mut tx,
            "price",
            "price_list",
            "update",
            operator,
            None,
            Some(serde_json::json!({
                "upserts": upserts.iter().map(|entry| &entry.entry_key).collect::<Vec<_>>(),
                "deletes": deletes,
                "credit_rates": credit_rates,
            })),
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(revision)
    }

    /// Balance in micro-Credit and when it last changed; 0 without an account.
    pub(crate) async fn credit_balance(&self, user_id: &str) -> StorageResult<(i64, i64)> {
        let sql = self.sql("SELECT balance_micro,updated_at_ms FROM aicc_credit_account WHERE user_id=?");
        Ok(sqlx::query(&sql)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| -> StorageResult<(i64, i64)> {
                Ok((row.try_get("balance_micro")?, row.try_get("updated_at_ms")?))
            })
            .transpose()?
            .unwrap_or((0, 0)))
    }

    pub(crate) async fn credit_balances(&self) -> StorageResult<BTreeMap<String, (i64, i64)>> {
        let mut balances = BTreeMap::new();
        for row in sqlx::query("SELECT user_id,balance_micro,updated_at_ms FROM aicc_credit_account")
            .fetch_all(&self.pool)
            .await?
        {
            balances.insert(
                row.try_get("user_id")?,
                (row.try_get("balance_micro")?, row.try_get("updated_at_ms")?),
            );
        }
        Ok(balances)
    }

    /// Lock (creating it at 0) the account row of a user; the first write of
    /// every balance changing transaction.
    async fn lock_account(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        user_id: &str,
        now_ms: i64,
    ) -> StorageResult<()> {
        let sql = self.sql(
            "INSERT INTO aicc_credit_account (user_id,balance_micro,version,updated_at_ms) VALUES (?,0,0,?)
             ON CONFLICT(user_id) DO UPDATE SET version=aicc_credit_account.version",
        );
        sqlx::query(&sql)
            .bind(user_id)
            .bind(now_ms)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn add_balance(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        user_id: &str,
        delta: i64,
        now_ms: i64,
    ) -> StorageResult<i64> {
        let update = self.sql(
            "UPDATE aicc_credit_account SET balance_micro=balance_micro+?, version=version+1, updated_at_ms=?
             WHERE user_id=?",
        );
        sqlx::query(&update)
            .bind(delta)
            .bind(now_ms)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
        let read = self.sql("SELECT balance_micro FROM aicc_credit_account WHERE user_id=?");
        Ok(sqlx::query_scalar(&read)
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await?)
    }

    /// Recharge or adjust once per request id: the same id with the same
    /// parameters returns the original entry, with different ones it fails.
    pub(crate) async fn apply_credit_entry(
        &self,
        draft: CreditEntryDraft<'_>,
    ) -> Result<(CreditEntry, i64, bool), PolicyWriteError> {
        let mut tx = self.pool.begin().await?;
        self.lock_account(&mut tx, draft.user_id, draft.now_ms).await?;
        let existing = self.sql("SELECT * FROM aicc_credit_entry WHERE request_id=?");
        if let Some(row) = sqlx::query(&existing)
            .bind(draft.request_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            let entry = entry_from_row(&row)?;
            let amount: i64 = row.try_get("amount_micro")?;
            tx.rollback().await?;
            let same = entry.user_id == draft.user_id
                && entry.kind == draft.kind
                && amount == draft.amount_micro
                && entry.note.as_deref() == draft.note
                && entry.external_ref.as_ref() == draft.external_ref;
            if !same {
                return Err(PolicyWriteError::IdempotencyConflict);
            }
            let (balance, _) = self.credit_balance(draft.user_id).await?;
            return Ok((entry, balance, true));
        }
        let balance = self
            .add_balance(&mut tx, draft.user_id, draft.amount_micro, draft.now_ms)
            .await?;
        let entry_id = new_policy_id("ce_", draft.request_id);
        let insert = self.sql(
            "INSERT INTO aicc_credit_entry (entry_id,request_id,user_id,kind,amount_micro,balance_after_micro,
             external_ref_json,operator,note,created_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?)",
        );
        let inserted = sqlx::query(&insert)
            .bind(&entry_id)
            .bind(draft.request_id)
            .bind(draft.user_id)
            .bind(kind_name(draft.kind))
            .bind(draft.amount_micro)
            .bind(balance)
            .bind(draft.external_ref.map(Value::to_string))
            .bind(draft.operator)
            .bind(draft.note)
            .bind(draft.now_ms)
            .execute(&mut *tx)
            .await;
        if let Err(error) = inserted {
            // Another user's entry took the id between the read and the insert.
            tx.rollback().await?;
            return match self.credit_entry(draft.request_id).await? {
                Some(_) => Err(PolicyWriteError::IdempotencyConflict),
                None => Err(PolicyWriteError::Storage(StorageError::Database(error))),
            };
        }
        self.audit_tx(
            &mut tx,
            "credit",
            draft.user_id,
            kind_name(draft.kind),
            draft.operator,
            None,
            Some(serde_json::json!({
                "request_id": draft.request_id,
                "amount": format_credit_micro(draft.amount_micro),
                "note": draft.note,
            })),
            draft.now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok((
            CreditEntry {
                entry_id,
                user_id: draft.user_id.to_string(),
                kind: draft.kind,
                amount: format_credit_micro(draft.amount_micro),
                balance_after: format_credit_micro(balance),
                recharge_id: Some(draft.request_id.to_string()),
                external_ref: draft.external_ref.cloned(),
                operator: draft.operator.to_string(),
                note: draft.note.map(str::to_owned),
                created_at_ms: draft.now_ms,
            },
            balance,
            false,
        ))
    }

    pub(crate) async fn credit_entry(&self, request_id: &str) -> StorageResult<Option<CreditEntry>> {
        let sql = self.sql("SELECT * FROM aicc_credit_entry WHERE request_id=?");
        sqlx::query(&sql)
            .bind(request_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(entry_from_row)
            .transpose()
    }

    /// Entries and charges of a user, newest first; the cursor is the time
    /// and id of the last item returned.
    pub(crate) async fn credit_ledger(
        &self,
        user_id: &str,
        start_ms: i64,
        end_ms: i64,
        cursor: Option<&str>,
        page: usize,
    ) -> StorageResult<(Vec<CreditLedgerItem>, Option<String>)> {
        let cursor = cursor.map(decode_cursor).transpose()?;
        let (before_ms, before_id) = cursor.unwrap_or((i64::MAX, String::from("\u{10ffff}")));
        let entries = self.sql(
            "SELECT * FROM aicc_credit_entry WHERE user_id=? AND created_at_ms>=? AND created_at_ms<?
             AND (created_at_ms<? OR (created_at_ms=? AND entry_id<?))
             ORDER BY created_at_ms DESC, entry_id DESC LIMIT ?",
        );
        let charges = self.sql(
            "SELECT * FROM aicc_credit_charge WHERE user_id=? AND settled_at_ms>=? AND settled_at_ms<?
             AND (settled_at_ms<? OR (settled_at_ms=? AND attempt_id<?))
             ORDER BY settled_at_ms DESC, attempt_id DESC LIMIT ?",
        );
        let mut items = Vec::new();
        for row in sqlx::query(&entries)
            .bind(user_id)
            .bind(start_ms)
            .bind(end_ms)
            .bind(before_ms)
            .bind(before_ms)
            .bind(&before_id)
            .bind((page + 1) as i64)
            .fetch_all(&self.pool)
            .await?
        {
            let entry = entry_from_row(&row)?;
            items.push((entry.created_at_ms, entry.entry_id.clone(), CreditLedgerItem::Entry(entry)));
        }
        for row in sqlx::query(&charges)
            .bind(user_id)
            .bind(start_ms)
            .bind(end_ms)
            .bind(before_ms)
            .bind(before_ms)
            .bind(&before_id)
            .bind((page + 1) as i64)
            .fetch_all(&self.pool)
            .await?
        {
            let charge = charge_from_row(&row)?;
            items.push((charge.settled_at_ms, charge.attempt_id.clone(), CreditLedgerItem::Charge(charge)));
        }
        items.sort_by(|left, right| (right.0, &right.1).cmp(&(left.0, &left.1)));
        let next = (items.len() > page).then(|| encode_cursor(items[page - 1].0, &items[page - 1].1));
        Ok((items.into_iter().take(page).map(|(_, _, item)| item).collect(), next))
    }

    #[cfg(test)]
    pub(crate) async fn credit_charge(&self, attempt_id: &str) -> StorageResult<Option<CreditCharge>> {
        let sql = self.sql("SELECT * FROM aicc_credit_charge WHERE attempt_id=?");
        sqlx::query(&sql)
            .bind(attempt_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(charge_from_row)
            .transpose()
    }

    /// Current windows of the counters named by `keys`.
    pub(crate) async fn counters(&self, keys: &BTreeSet<String>) -> StorageResult<BTreeMap<String, CounterState>> {
        if keys.is_empty() {
            return Ok(BTreeMap::new());
        }
        let marks = vec!["?"; keys.len()].join(",");
        let sql = self.sql(&format!(
            "SELECT * FROM aicc_quota_counter WHERE scope_key IN ({marks})"
        ));
        let rows = bind_all!(sqlx::query(&sql), keys).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| Ok((row.try_get::<String, _>("scope_key")?, counter_from_row(row)?)))
            .collect()
    }

    /// Settle an attempt once: insert its charge, move the balance and add
    /// it to the counters of its admission time. A charge recorded with an
    /// unknown usage is completed once when the real usage arrives. Returns
    /// the charge as stored.
    pub(crate) async fn settle_charge(
        &self,
        plan: &ChargePlan,
        outcome: ChargeOutcome,
        usage_event_id: Option<&str>,
        now_ms: i64,
    ) -> StorageResult<ChargeOutcome> {
        let mut tx = self.pool.begin().await?;
        self.lock_account(&mut tx, &plan.user_id, now_ms).await?;
        let read = self.sql("SELECT credit_micro,tokens,status FROM aicc_credit_charge WHERE attempt_id=?");
        let existing = sqlx::query(&read)
            .bind(&plan.attempt_id)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(row) = existing {
            let status: String = row.try_get("status")?;
            let stored = ChargeOutcome {
                credit_micro: row.try_get("credit_micro")?,
                tokens: from_i64(row.try_get("tokens")?)?,
                status: CreditChargeStatus::parse(&status)
                    .ok_or_else(|| StorageError::InvalidRecord(format!("charge status {status}")))?,
            };
            if stored.status != CreditChargeStatus::UnknownUsage
                || outcome.status == CreditChargeStatus::UnknownUsage
            {
                tx.rollback().await?;
                return Ok(stored);
            }
            let update = self.sql(
                "UPDATE aicc_credit_charge SET credit_micro=?, tokens=?, status=?, usage_event_id=?,
                 settled_at_ms=? WHERE attempt_id=? AND status='unknown_usage'",
            );
            sqlx::query(&update)
                .bind(outcome.credit_micro)
                .bind(to_i64(outcome.tokens)?)
                .bind(outcome.status.as_str())
                .bind(usage_event_id)
                .bind(now_ms)
                .bind(&plan.attempt_id)
                .execute(&mut *tx)
                .await?;
        } else {
            let insert = self.sql(
                "INSERT INTO aicc_credit_charge (attempt_id,task_id,usage_event_id,user_id,app_id,operator_id,
                 provider_instance_name,exact_model,admitted_at_ms,tokens,credit_micro,status,pricing_json,
                 settled_at_ms) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            );
            sqlx::query(&insert)
                .bind(&plan.attempt_id)
                .bind(&plan.task_id)
                .bind(usage_event_id)
                .bind(&plan.user_id)
                .bind(&plan.app_id)
                .bind(&plan.operator_id)
                .bind(&plan.provider_instance_name)
                .bind(&plan.exact_model)
                .bind(plan.admitted_at_ms)
                .bind(to_i64(outcome.tokens)?)
                .bind(outcome.credit_micro)
                .bind(outcome.status.as_str())
                .bind(serde_json::to_string(&plan.price)?)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
        }
        if outcome.credit_micro != 0 {
            self.add_balance(&mut tx, &plan.user_id, -outcome.credit_micro, now_ms)
                .await?;
        }
        let tokens = if plan.token_metered {
            to_i64(outcome.tokens)?
        } else {
            0
        };
        if outcome.credit_micro != 0 || tokens != 0 {
            for scope in &plan.counters {
                self.add_to_counter(&mut tx, scope, plan.admitted_at_ms, outcome.credit_micro, tokens, now_ms)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(outcome)
    }

    /// A window starts at the first charge after the previous one ended; a
    /// charge admitted before the current window started counts in none.
    async fn add_to_counter(
        &self,
        tx: &mut sqlx::Transaction<'_, Any>,
        scope: &CounterScope,
        admitted_at_ms: i64,
        credit_micro: i64,
        tokens: i64,
        now_ms: i64,
    ) -> StorageResult<()> {
        let key = scope.key();
        let length = scope.period.length_ms();
        let insert = self.sql(
            "INSERT INTO aicc_quota_counter (scope_key,scope_json,window_start_ms,window_end_ms,
             used_credit_micro,used_tokens,updated_at_ms) VALUES (?,?,?,?,0,0,?) ON CONFLICT(scope_key) DO NOTHING",
        );
        sqlx::query(&insert)
            .bind(&key)
            .bind(serde_json::to_string(scope)?)
            .bind(admitted_at_ms)
            .bind(admitted_at_ms.saturating_add(length))
            .bind(now_ms)
            .execute(&mut **tx)
            .await?;
        let read = self.sql("SELECT * FROM aicc_quota_counter WHERE scope_key=?");
        let row = sqlx::query(&read).bind(&key).fetch_one(&mut **tx).await?;
        let state = counter_from_row(&row)?;
        let (start, end, base_credit, base_tokens) = if admitted_at_ms < state.window_start_ms {
            return Ok(());
        } else if admitted_at_ms < state.window_end_ms {
            (
                state.window_start_ms,
                state.window_end_ms,
                state.used_credit_micro,
                state.used_tokens,
            )
        } else {
            (admitted_at_ms, admitted_at_ms.saturating_add(length), 0, 0)
        };
        let update = self.sql(
            "UPDATE aicc_quota_counter SET window_start_ms=?, window_end_ms=?, used_credit_micro=?,
             used_tokens=?, updated_at_ms=? WHERE scope_key=?",
        );
        sqlx::query(&update)
            .bind(start)
            .bind(end)
            .bind(base_credit.saturating_add(credit_micro))
            .bind(base_tokens.saturating_add(tokens))
            .bind(now_ms)
            .bind(&key)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Apps already seen in a user's usage (an App the user may limit).
    pub(crate) async fn usage_apps(&self, user_id: &str) -> StorageResult<BTreeSet<String>> {
        let sql = self.sql(
            "SELECT DISTINCT caller_app_id FROM aicc_usage_event WHERE user_id=? AND caller_app_id IS NOT NULL",
        );
        let apps: Vec<Option<String>> = sqlx::query_scalar(&sql)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(apps.into_iter().flatten().filter(|app| !app.is_empty()).collect())
    }
}

#[cfg(test)]
mod tests;
