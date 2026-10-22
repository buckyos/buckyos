//! Session families (contract §7, D-6): one SQLite row per family, never a token.
//! A family re-signs its current pair from the stored jti / exp / claims, so a grace
//! answer after a restart is the same pair the client already holds.

use ::kRPC::{RPCErrors, RPCSessionToken};
use buckyos_api::{
    bind_token_auth_method, bind_token_ceiling, bind_token_external, AuthMethod, AuthTarget,
    SessionCeiling, SessionScope, SessionSource, TokenPrincipalKind, UserType,
};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// The rotated-away refresh token is still answered with the current pair for this long,
/// so pages of one host refreshing over the same cookie at once do not revoke their family.
pub const REFRESH_GRACE_SECONDS: u64 = 10;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS families (
  family_key TEXT PRIMARY KEY,
  subject TEXT NOT NULL,
  principal_kind TEXT NOT NULL,
  target TEXT NOT NULL,
  session_id TEXT NOT NULL,
  claims TEXT NOT NULL,
  source TEXT,
  parent_key TEXT,
  absolute_exp INTEGER,
  created_at INTEGER NOT NULL,
  refresh_jti TEXT NOT NULL,
  refresh_exp INTEGER NOT NULL,
  session_jti TEXT NOT NULL,
  session_exp INTEGER NOT NULL,
  prev_refresh_jti TEXT,
  prev_rotated_at INTEGER
);
CREATE INDEX IF NOT EXISTS families_subject ON families(subject);
CREATE INDEX IF NOT EXISTS families_parent ON families(parent_key);
";

const COLUMNS: &str = "family_key, subject, principal_kind, target, session_id, claims, source, \
     parent_key, absolute_exp, created_at, refresh_jti, refresh_exp, session_jti, session_exp, \
     prev_refresh_jti, prev_rotated_at";

/// Signed claims of every pair a family issues; refresh and delegation only narrow them.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct FamilyClaims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<AuthMethod>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub external: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restricted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<SessionScope>,
}

impl FamilyClaims {
    pub fn account(auth_method: AuthMethod, user_type: UserType) -> Self {
        Self::with_ceiling(
            auth_method,
            false,
            &SessionCeiling::for_user_type(user_type),
        )
    }

    pub fn external(auth_method: AuthMethod) -> Self {
        Self::with_ceiling(auth_method, true, &SessionCeiling::external())
    }

    fn with_ceiling(auth_method: AuthMethod, external: bool, ceiling: &SessionCeiling) -> Self {
        Self {
            auth_method: Some(auth_method),
            external,
            restricted: ceiling.restricted,
            scopes: ceiling.scopes.iter().copied().collect(),
        }
    }

    pub fn ceiling(&self) -> SessionCeiling {
        if self.restricted {
            SessionCeiling::restricted(self.scopes.iter().copied())
        } else {
            SessionCeiling::unrestricted()
        }
    }

    pub fn narrowed(&self, ceiling: &SessionCeiling) -> Self {
        let ceiling = self.ceiling().intersect(ceiling);
        Self {
            auth_method: self.auth_method,
            external: self.external,
            restricted: ceiling.restricted,
            scopes: ceiling.scopes.into_iter().collect(),
        }
    }

    pub fn bind(&self, token: &mut RPCSessionToken) {
        if let Some(auth_method) = self.auth_method {
            bind_token_auth_method(token, auth_method);
        }
        if self.external {
            bind_token_external(token);
        }
        bind_token_ceiling(token, &self.ceiling());
    }
}

/// Whom and what a family's pairs speak for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSubject {
    pub subject: String,
    pub principal_kind: TokenPrincipalKind,
    pub target: AuthTarget,
    pub session_id: u64,
    pub claims: FamilyClaims,
}

impl SessionSubject {
    pub fn family_key(&self) -> String {
        family_key(&self.subject, &self.target, self.session_id)
    }
}

pub fn family_key(subject: &str, target: &AuthTarget, session_id: u64) -> String {
    format!("{}_{}_{}", subject, target.canonical_key(), session_id)
}

/// Identifiers and expiry of the current pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairStamp {
    pub session_jti: String,
    pub session_exp: u64,
    pub refresh_jti: String,
    pub refresh_exp: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotatedRefresh {
    pub jti: String,
    pub rotated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyRecord {
    pub session: SessionSubject,
    pub stamp: PairStamp,
    pub source: Option<SessionSource>,
    /// The family a restricted delegation depends on.
    pub parent_key: Option<String>,
    /// DID federation: no pair outlives this (Q-04).
    pub absolute_exp: Option<u64>,
    pub created_at: u64,
    pub previous: Option<RotatedRefresh>,
}

impl FamilyRecord {
    pub fn key(&self) -> String {
        self.session.family_key()
    }

    pub fn is_live(&self, now: u64) -> bool {
        self.stamp.refresh_exp > now && self.absolute_exp.map_or(true, |exp| exp > now)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshPresentation {
    Current,
    Grace,
    Missing,
    Reused,
}

/// The family's current refresh jti, or the one it replaced within the grace window;
/// anything else is a reuse.
pub fn classify_refresh_presentation(
    family: Option<&FamilyRecord>,
    jti: &str,
    now: u64,
) -> RefreshPresentation {
    let Some(family) = family else {
        return RefreshPresentation::Missing;
    };
    if family.stamp.refresh_jti == jti {
        return RefreshPresentation::Current;
    }
    match family.previous.as_ref() {
        Some(previous)
            if previous.jti == jti
                && now.saturating_sub(previous.rotated_at) <= REFRESH_GRACE_SECONDS =>
        {
            RefreshPresentation::Grace
        }
        _ => RefreshPresentation::Reused,
    }
}

#[derive(Debug)]
pub enum RefreshCommit {
    Committed,
    /// A concurrent rotation won; answer with its pair.
    Grace(FamilyRecord),
    LoggedOut,
    /// The family was revoked.
    Reused,
}

pub struct FamilyStore {
    conn: Mutex<Connection>,
}

fn db_error(error: rusqlite::Error) -> RPCErrors {
    RPCErrors::ReasonError(format!("session family store: {error}"))
}

fn conversion_error(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, Type::Text, message.into())
}

fn to_sql_u64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn json_column<T: serde::de::DeserializeOwned>(row: &Row, column: usize) -> rusqlite::Result<T> {
    let raw: String = row.get(column)?;
    serde_json::from_str(&raw).map_err(|error| conversion_error(column, error.to_string()))
}

fn read_record(row: &Row) -> rusqlite::Result<FamilyRecord> {
    let principal_kind: TokenPrincipalKind = {
        let raw: String = row.get(2)?;
        serde_json::from_value(serde_json::Value::String(raw))
            .map_err(|error| conversion_error(2, error.to_string()))?
    };
    let target = {
        let raw: String = row.get(3)?;
        AuthTarget::from_canonical_key(&raw).map_err(|error| conversion_error(3, error))?
    };
    let session_id = {
        let raw: String = row.get(4)?;
        raw.parse::<u64>()
            .map_err(|error| conversion_error(4, error.to_string()))?
    };
    let source = match row.get::<_, Option<String>>(6)? {
        Some(raw) => Some(
            serde_json::from_str(&raw).map_err(|error| conversion_error(6, error.to_string()))?,
        ),
        None => None,
    };
    let previous = match (
        row.get::<_, Option<String>>(14)?,
        row.get::<_, Option<i64>>(15)?,
    ) {
        (Some(jti), Some(rotated_at)) => Some(RotatedRefresh {
            jti,
            rotated_at: rotated_at as u64,
        }),
        _ => None,
    };
    Ok(FamilyRecord {
        session: SessionSubject {
            subject: row.get(1)?,
            principal_kind,
            target,
            session_id,
            claims: json_column(row, 5)?,
        },
        stamp: PairStamp {
            refresh_jti: row.get(10)?,
            refresh_exp: row.get::<_, i64>(11)? as u64,
            session_jti: row.get(12)?,
            session_exp: row.get::<_, i64>(13)? as u64,
        },
        source,
        parent_key: row.get(7)?,
        absolute_exp: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
        created_at: row.get::<_, i64>(9)? as u64,
        previous,
    })
}

fn select_record(conn: &Connection, key: &str) -> rusqlite::Result<Option<FamilyRecord>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM families WHERE family_key = ?1"),
        params![key],
        read_record,
    )
    .optional()
}

fn write_record(conn: &Connection, record: &FamilyRecord) -> rusqlite::Result<()> {
    let claims = serde_json::to_string(&record.session.claims)
        .map_err(|error| conversion_error(5, error.to_string()))?;
    let source = record
        .source
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| conversion_error(6, error.to_string()))?;
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO families ({COLUMNS}) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"
        ),
        params![
            record.key(),
            record.session.subject,
            record.session.principal_kind.as_str(),
            record.session.target.canonical_key(),
            record.session.session_id.to_string(),
            claims,
            source,
            record.parent_key,
            record.absolute_exp.map(to_sql_u64),
            to_sql_u64(record.created_at),
            record.stamp.refresh_jti,
            to_sql_u64(record.stamp.refresh_exp),
            record.stamp.session_jti,
            to_sql_u64(record.stamp.session_exp),
            record
                .previous
                .as_ref()
                .map(|previous| previous.jti.clone()),
            record
                .previous
                .as_ref()
                .map(|previous| to_sql_u64(previous.rotated_at)),
        ],
    )?;
    Ok(())
}

/// Delete a family and every restricted delegation that depends on it.
fn delete_tree(tx: &Transaction, key: &str) -> rusqlite::Result<u32> {
    let mut removed = 0;
    let mut pending = vec![key.to_string()];
    while let Some(key) = pending.pop() {
        let children = {
            let mut stmt = tx.prepare("SELECT family_key FROM families WHERE parent_key = ?1")?;
            let rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        removed += tx.execute("DELETE FROM families WHERE family_key = ?1", params![key])? as u32;
        pending.extend(children.into_iter().filter(|child| *child != key));
    }
    Ok(removed)
}

impl FamilyStore {
    pub fn open(path: &Path) -> Result<Self, RPCErrors> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "create session family dir {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let conn = Connection::open(path).map_err(db_error)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(db_error)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, RPCErrors> {
        Self::init(Connection::open_in_memory().map_err(db_error)?)
    }

    fn init(conn: Connection) -> Result<Self, RPCErrors> {
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        conn.execute_batch(SCHEMA).map_err(db_error)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn write<T>(
        &self,
        action: impl FnOnce(&Transaction) -> rusqlite::Result<T>,
    ) -> Result<T, RPCErrors> {
        let mut conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let result = action(&tx).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }

    pub fn get(&self, key: &str) -> Result<Option<FamilyRecord>, RPCErrors> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        select_record(&conn, key).map_err(db_error)
    }

    pub fn is_live(&self, key: &str, now: u64) -> Result<bool, RPCErrors> {
        Ok(self
            .get(key)?
            .map(|record| record.is_live(now))
            .unwrap_or(false))
    }

    pub fn insert(&self, record: &FamilyRecord) -> Result<(), RPCErrors> {
        self.write(|tx| write_record(tx, record))
    }

    /// Insert a delegated family only while its source is live, in one transaction, so nothing
    /// is delegated from a session once its logout committed.
    pub fn insert_delegated(
        &self,
        source_key: &str,
        record: &FamilyRecord,
        now: u64,
    ) -> Result<bool, RPCErrors> {
        self.write(|tx| {
            let live = select_record(tx, source_key)?
                .map(|source| source.is_live(now))
                .unwrap_or(false);
            if live {
                write_record(tx, record)?;
            }
            Ok(live)
        })
    }

    /// Compare-and-replace of the current pair: commits only while `old_jti` is still current,
    /// so a logout or a competing rotation that happened meanwhile is never overwritten.
    pub fn rotate(
        &self,
        key: &str,
        old_jti: &str,
        next: &PairStamp,
        claims: &FamilyClaims,
        now: u64,
    ) -> Result<RefreshCommit, RPCErrors> {
        let claims = serde_json::to_string(claims)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        self.write(|tx| {
            let changed = tx.execute(
                "UPDATE families SET prev_refresh_jti = refresh_jti, prev_rotated_at = ?1, \
                 refresh_jti = ?2, refresh_exp = ?3, session_jti = ?4, session_exp = ?5, claims = ?6 \
                 WHERE family_key = ?7 AND refresh_jti = ?8",
                params![
                    to_sql_u64(now),
                    next.refresh_jti,
                    to_sql_u64(next.refresh_exp),
                    next.session_jti,
                    to_sql_u64(next.session_exp),
                    claims,
                    key,
                    old_jti,
                ],
            )?;
            if changed == 1 {
                return Ok(RefreshCommit::Committed);
            }
            let current = select_record(tx, key)?;
            Ok(
                match classify_refresh_presentation(current.as_ref(), old_jti, now) {
                    RefreshPresentation::Grace => RefreshCommit::Grace(current.unwrap()),
                    RefreshPresentation::Missing => RefreshCommit::LoggedOut,
                    RefreshPresentation::Current | RefreshPresentation::Reused => {
                        delete_tree(tx, key)?;
                        RefreshCommit::Reused
                    }
                },
            )
        })
    }

    pub fn update_claims(&self, key: &str, claims: &FamilyClaims) -> Result<(), RPCErrors> {
        let claims = serde_json::to_string(claims)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        self.write(|tx| {
            tx.execute(
                "UPDATE families SET claims = ?1 WHERE family_key = ?2",
                params![claims, key],
            )?;
            Ok(())
        })
    }

    /// Revoke a family together with its dependent delegations; returns the rows removed.
    pub fn remove(&self, key: &str) -> Result<u32, RPCErrors> {
        self.write(|tx| delete_tree(tx, key))
    }

    /// Revoke the subject's families `matches` selects, except `keep` and its delegations.
    pub fn revoke_matching(
        &self,
        subject: &str,
        keep: Option<&str>,
        matches: impl Fn(&FamilyRecord) -> bool,
    ) -> Result<u32, RPCErrors> {
        self.write(|tx| {
            let records = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT {COLUMNS} FROM families WHERE subject = ?1"
                ))?;
                let rows = stmt.query_map(params![subject], read_record)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut removed = 0;
            for record in records {
                let key = record.key();
                if keep == Some(key.as_str())
                    || (keep.is_some() && record.parent_key.as_deref() == keep)
                    || !matches(&record)
                {
                    continue;
                }
                removed += delete_tree(tx, &key)?;
            }
            Ok(removed)
        })
    }

    /// Drop families whose refresh or absolute expiry passed, then delegations left without
    /// their parent.
    pub fn gc(&self, now: u64) -> Result<u32, RPCErrors> {
        self.write(|tx| {
            let mut removed = tx.execute(
                "DELETE FROM families WHERE refresh_exp <= ?1 \
                 OR (absolute_exp IS NOT NULL AND absolute_exp <= ?1)",
                params![to_sql_u64(now)],
            )? as u32;
            loop {
                let orphans = tx.execute(
                    "DELETE FROM families WHERE parent_key IS NOT NULL \
                     AND parent_key NOT IN (SELECT family_key FROM families)",
                    [],
                )? as u32;
                if orphans == 0 {
                    break;
                }
                removed += orphans;
            }
            Ok(removed)
        })
    }

    #[cfg(test)]
    pub fn backdate_rotation(&self, key: &str, seconds: u64) -> Result<(), RPCErrors> {
        self.write(|tx| {
            tx.execute(
                "UPDATE families SET prev_rotated_at = prev_rotated_at - ?1 WHERE family_key = ?2",
                params![to_sql_u64(seconds), key],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "verify-hub-{name}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        dir.join("sessions.db")
    }

    fn stamp(refresh_jti: &str) -> PairStamp {
        PairStamp {
            session_jti: format!("s-{refresh_jti}"),
            session_exp: NOW + 900,
            refresh_jti: refresh_jti.to_string(),
            refresh_exp: NOW + 3600,
        }
    }

    fn record(subject: &str, session_id: u64, claims: FamilyClaims) -> FamilyRecord {
        FamilyRecord {
            session: SessionSubject {
                subject: subject.to_string(),
                principal_kind: TokenPrincipalKind::User,
                target: AuthTarget::system("control-panel".parse().unwrap()),
                session_id,
                claims,
            },
            stamp: stamp("r1"),
            source: None,
            parent_key: None,
            absolute_exp: None,
            created_at: NOW,
            previous: None,
        }
    }

    fn invite_family(subject: &str, session_id: u64, invite_id: &str) -> FamilyRecord {
        let mut family = record(
            subject,
            session_id,
            FamilyClaims::account(AuthMethod::ExclusiveInvite, UserType::Limited),
        );
        family.source = Some(SessionSource::ExclusiveInvite {
            invite_id: invite_id.to_string(),
            version: 1,
        });
        family
    }

    #[test]
    fn records_round_trip_and_survive_reopen() {
        let path = temp_db("reopen");
        let mut family = invite_family("acme", 7, "ei-1");
        family.absolute_exp = Some(NOW + 100);
        family.previous = Some(RotatedRefresh {
            jti: "r0".into(),
            rotated_at: NOW - 1,
        });
        {
            let store = FamilyStore::open(&path).unwrap();
            store.insert(&family).unwrap();
            assert_eq!(store.get(&family.key()).unwrap(), Some(family.clone()));
        }
        let store = FamilyStore::open(&path).unwrap();
        let loaded = store.get(&family.key()).unwrap().unwrap();
        assert_eq!(loaded, family);
        assert_eq!(
            loaded.session.claims.ceiling(),
            SessionCeiling::for_user_type(UserType::Limited)
        );
        assert!(store.is_live(&family.key(), NOW).unwrap());
        assert!(!store.is_live(&family.key(), NOW + 100).unwrap());
        assert_eq!(store.remove(&family.key()).unwrap(), 1);
        assert_eq!(store.get(&family.key()).unwrap(), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn rotation_is_compare_and_replace_with_grace_and_reuse() {
        let store = FamilyStore::open_in_memory().unwrap();
        let family = record("alice", 1, FamilyClaims::default());
        let key = family.key();
        store.insert(&family).unwrap();
        let claims = FamilyClaims::default();

        assert!(matches!(
            store
                .rotate(&key, "r1", &stamp("r2"), &claims, NOW)
                .unwrap(),
            RefreshCommit::Committed
        ));
        let rotated = store.get(&key).unwrap().unwrap();
        assert_eq!(rotated.stamp, stamp("r2"));
        assert_eq!(rotated.previous.as_ref().unwrap().jti, "r1");
        assert_eq!(
            classify_refresh_presentation(Some(&rotated), "r1", NOW + REFRESH_GRACE_SECONDS),
            RefreshPresentation::Grace
        );
        assert_eq!(
            classify_refresh_presentation(Some(&rotated), "r1", NOW + REFRESH_GRACE_SECONDS + 1),
            RefreshPresentation::Reused
        );

        match store
            .rotate(&key, "r1", &stamp("r2b"), &claims, NOW)
            .unwrap()
        {
            RefreshCommit::Grace(current) => assert_eq!(current.stamp.refresh_jti, "r2"),
            other => panic!("expected grace, got {other:?}"),
        }
        assert_eq!(store.get(&key).unwrap().unwrap().stamp.refresh_jti, "r2");

        assert!(matches!(
            store
                .rotate(&key, "r0", &stamp("r3"), &claims, NOW)
                .unwrap(),
            RefreshCommit::Reused
        ));
        assert_eq!(store.get(&key).unwrap(), None);
        assert!(matches!(
            store
                .rotate(&key, "r2", &stamp("r4"), &claims, NOW)
                .unwrap(),
            RefreshCommit::LoggedOut
        ));
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn rotation_persists_narrowed_claims() {
        let store = FamilyStore::open_in_memory().unwrap();
        let family = record(
            "bob",
            2,
            FamilyClaims::account(AuthMethod::Password, UserType::User),
        );
        store.insert(&family).unwrap();
        let narrowed = family
            .session
            .claims
            .narrowed(&SessionCeiling::for_user_type(UserType::Limited));
        assert!(narrowed.restricted);
        store
            .rotate(&family.key(), "r1", &stamp("r2"), &narrowed, NOW)
            .unwrap();
        assert_eq!(
            store.get(&family.key()).unwrap().unwrap().session.claims,
            narrowed
        );
        // A later upgrade never widens what was stored.
        assert_eq!(
            narrowed.narrowed(&SessionCeiling::for_user_type(UserType::User)),
            narrowed
        );
    }

    #[test]
    fn delegated_families_die_with_their_parent() {
        let store = FamilyStore::open_in_memory().unwrap();
        let parent = invite_family("acme", 1, "ei-1");
        let mut child = invite_family("acme", 2, "ei-1");
        child.session.target = AuthTarget::app("notes@bob".parse().unwrap());
        child.parent_key = Some(parent.key());

        assert!(!store.insert_delegated(&parent.key(), &child, NOW).unwrap());
        store.insert(&parent).unwrap();
        assert!(!store
            .insert_delegated(&parent.key(), &child, NOW + 3600)
            .unwrap());
        assert!(store.insert_delegated(&parent.key(), &child, NOW).unwrap());
        assert_eq!(store.remove(&parent.key()).unwrap(), 2);
        assert_eq!(store.get(&child.key()).unwrap(), None);

        store.insert(&parent).unwrap();
        store.insert(&child).unwrap();
        let mut expired = parent.clone();
        expired.stamp.refresh_exp = NOW;
        store.insert(&expired).unwrap();
        assert_eq!(store.gc(NOW).unwrap(), 2);
        assert_eq!(store.get(&child.key()).unwrap(), None);
    }

    #[test]
    fn revoke_matching_keeps_the_current_family_and_other_methods() {
        let store = FamilyStore::open_in_memory().unwrap();
        let current = invite_family("acme", 1, "ei-1");
        let other_invite = invite_family("acme", 2, "ei-2");
        let password = record(
            "acme",
            3,
            FamilyClaims::account(AuthMethod::Password, UserType::Limited),
        );
        let stranger = invite_family("someone", 4, "ei-1");
        let mut current_child = invite_family("acme", 5, "ei-1");
        current_child.parent_key = Some(current.key());
        for family in [
            &current,
            &other_invite,
            &password,
            &stranger,
            &current_child,
        ] {
            store.insert(family).unwrap();
        }

        let revoked = store
            .revoke_matching("acme", Some(&current.key()), |family| {
                family.session.claims.auth_method == Some(AuthMethod::ExclusiveInvite)
            })
            .unwrap();
        assert_eq!(revoked, 1);
        assert!(store.get(&current.key()).unwrap().is_some());
        assert!(store.get(&current_child.key()).unwrap().is_some());
        assert!(store.get(&other_invite.key()).unwrap().is_none());
        assert!(store.get(&password.key()).unwrap().is_some());
        assert!(store.get(&stranger.key()).unwrap().is_some());
    }
}
