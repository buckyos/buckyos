//! Archive passwords (archive TODO T5, D6): a password that passed the quick
//! check is pending in the session; the first complete read whose CRC / HMAC
//! checks out makes it verified, and stores it per (user, archive sha256)
//! once the outer hash is known. Plaintext in filedb is a known risk (§7).

use crate::error::NfsResult;
use crate::filedb::FileDb;
use rusqlite::params;
use std::collections::HashMap;
use std::sync::Mutex;

pub const SCHEMA: &str = include_str!("password.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Verified,
    Stored,
    Pending,
}

#[derive(Debug, Default)]
struct Known {
    pending: Vec<String>,
    verified: Vec<String>,
}

/// Per (user, outer version key), in memory.
#[derive(Debug, Default)]
pub struct Sessions {
    map: Mutex<HashMap<(String, String), Known>>,
    unlocking: Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

impl Sessions {
    /// One unlock at a time per archive and user (concurrent ones queue).
    pub fn unlock_lock(&self, user: &str, key: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        self.unlocking.lock().unwrap().entry(format!("{user}\0{key}")).or_default().clone()
    }

    pub fn add_pending(&self, user: &str, key: &str, password: &str) {
        let mut map = self.map.lock().unwrap();
        let known = map.entry((user.to_string(), key.to_string())).or_default();
        if !known.verified.iter().any(|p| p == password) && !known.pending.iter().any(|p| p == password) {
            known.pending.push(password.to_string());
        }
    }

    pub fn session_candidates(&self, user: &str, key: &str) -> (Vec<String>, Vec<String>) {
        let map = self.map.lock().unwrap();
        map.get(&(user.to_string(), key.to_string())).map(|k| (k.verified.clone(), k.pending.clone())).unwrap_or_default()
    }

    pub fn promote(&self, user: &str, key: &str, password: &str) {
        let mut map = self.map.lock().unwrap();
        let known = map.entry((user.to_string(), key.to_string())).or_default();
        known.pending.retain(|p| p != password);
        if !known.verified.iter().any(|p| p == password) {
            known.verified.push(password.to_string());
        }
    }

    pub fn drop_pending(&self, user: &str, key: &str, password: &str) {
        if let Some(known) = self.map.lock().unwrap().get_mut(&(user.to_string(), key.to_string())) {
            known.pending.retain(|p| p != password);
        }
    }

    pub fn forget(&self, user: &str, key: Option<&str>) {
        self.map.lock().unwrap().retain(|(u, k), _| u != user || key.is_some_and(|key| key != k));
    }
}

pub fn stored(db: &FileDb, user: &str, sha256: &str) -> NfsResult<Vec<String>> {
    let conn = db.lock();
    let mut stmt = conn.prepare("SELECT password FROM archive_passwords WHERE user_id=?1 AND archive_sha256=?2 ORDER BY verified_at DESC")?;
    let rows = stmt.query_map(params![user, sha256], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// Whether `user` keeps any password at all (before hashing an outer to look one up).
pub fn has_any(db: &FileDb, user: &str) -> bool {
    db.lock().query_row("SELECT 1 FROM archive_passwords WHERE user_id=?1 LIMIT 1", params![user], |_| Ok(())).is_ok()
}

pub fn store(db: &FileDb, user: &str, sha256: &str, password: &str) -> NfsResult<()> {
    db.lock().execute(
        "INSERT INTO archive_passwords(user_id, archive_sha256, password, verified_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(user_id, archive_sha256, password) DO UPDATE SET verified_at=excluded.verified_at",
        params![user, sha256, password, crate::fsutil::unix_now()],
    )?;
    Ok(())
}

pub fn forget(db: &FileDb, user: &str, sha256: Option<&str>) -> NfsResult<usize> {
    let conn = db.lock();
    Ok(match sha256 {
        Some(sha) => conn.execute("DELETE FROM archive_passwords WHERE user_id=?1 AND archive_sha256=?2", params![user, sha])?,
        None => conn.execute("DELETE FROM archive_passwords WHERE user_id=?1", params![user])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_and_store_are_per_user() {
        let s = Sessions::default();
        s.add_pending("alice", "k", "pw");
        assert_eq!(s.session_candidates("alice", "k"), (vec![], vec!["pw".to_string()]));
        assert_eq!(s.session_candidates("bob", "k"), (vec![], vec![]));
        s.promote("alice", "k", "pw");
        assert_eq!(s.session_candidates("alice", "k"), (vec!["pw".to_string()], vec![]));
        s.add_pending("alice", "k", "other");
        s.drop_pending("alice", "k", "other");
        assert_eq!(s.session_candidates("alice", "k").1.len(), 0);
        s.forget("alice", Some("k"));
        assert_eq!(s.session_candidates("alice", "k"), (vec![], vec![]));

        let db = FileDb::open_memory().unwrap();
        store(&db, "alice", "aa", "pw").unwrap();
        store(&db, "alice", "bb", "pw2").unwrap();
        assert_eq!(stored(&db, "alice", "aa").unwrap(), vec!["pw".to_string()]);
        assert!(stored(&db, "bob", "aa").unwrap().is_empty());
        assert_eq!(forget(&db, "alice", Some("aa")).unwrap(), 1);
        assert_eq!(forget(&db, "alice", None).unwrap(), 1);
    }
}
