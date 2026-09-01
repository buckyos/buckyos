//! In-memory session state: hello sessions, the seq exactly-once replay window
//! (NFSP §6.3), and file write leases with fencing seq (Ops_v3 I4).
//!
//! All of this is deliberately volatile (nfs_server.md §3.1): a restart drops
//! sessions and leases; clients recover through SEQ_OUT_OF_WINDOW / STALE /
//! watch resync — the same paths they must implement anyway.

use crate::error::{ErrorCode, NfsError, NfsResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct SessionMgr {
    sessions: Mutex<HashMap<String, Session>>,
    replay_window: u64,
}

struct Session {
    /// seq → cached response body (write ops only)
    replay: HashMap<u64, Value>,
    max_seq: u64,
    #[allow(dead_code)]
    created: Instant,
}

#[derive(Debug)]
pub enum SeqDisposition {
    /// Execute the op; cache the response under this seq.
    Execute,
    /// Duplicate retry: return this cached response verbatim.
    Replay(Value),
}

impl SessionMgr {
    pub fn new(replay_window: u64) -> Self {
        SessionMgr { sessions: Mutex::new(HashMap::new()), replay_window }
    }

    pub fn create(&self) -> String {
        let id = format!("sess_{}", uuid::Uuid::new_v4().simple());
        self.sessions.lock().unwrap().insert(
            id.clone(),
            Session { replay: HashMap::new(), max_seq: 0, created: Instant::now() },
        );
        id
    }

    pub fn exists(&self, id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(id)
    }

    pub fn remove(&self, id: &str) -> bool {
        self.sessions.lock().unwrap().remove(id).is_some()
    }

    /// Replay check for a write op. Reads don't consume seq.
    pub fn check_seq(&self, session: &str, seq: u64) -> NfsResult<SeqDisposition> {
        let mut map = self.sessions.lock().unwrap();
        let s = map
            .get_mut(session)
            .ok_or_else(|| NfsError::new(ErrorCode::PermissionDenied, "unknown session"))?;
        if let Some(cached) = s.replay.get(&seq) {
            return Ok(SeqDisposition::Replay(cached.clone()));
        }
        let min_allowed = s.max_seq.saturating_sub(self.replay_window);
        if seq < min_allowed {
            return Err(NfsError::new(
                ErrorCode::SeqOutOfWindow,
                format!("seq {} below replay window (min {})", seq, min_allowed),
            ));
        }
        Ok(SeqDisposition::Execute)
    }

    pub fn record_seq(&self, session: &str, seq: u64, response: Value) {
        let mut map = self.sessions.lock().unwrap();
        if let Some(s) = map.get_mut(session) {
            if seq > s.max_seq {
                s.max_seq = seq;
            }
            s.replay.insert(seq, response);
            // Evict entries that fell out of the window.
            let min_keep = s.max_seq.saturating_sub(self.replay_window);
            if s.replay.len() as u64 > self.replay_window * 2 {
                s.replay.retain(|k, _| *k >= min_keep);
            }
        }
    }
}

// ---------- leases ----------

#[derive(Debug, Clone)]
pub struct Lease {
    pub lease_id: String,
    pub session: String,
    /// Fencing seq: strictly increasing across all leases ever granted.
    pub seq: u64,
    pub expires: Instant,
    /// (size, mtime) of the pre-existing target at open_write time; None if the
    /// target did not exist. Used for the bypass-writer conflict re-check at
    /// commit (nfs_server.md §4.2: 旁路编辑在 commit_file 时显式冲突).
    pub base: Option<(u64, i64)>,
    /// Upload staging handle bound to this lease.
    pub fb_handle: String,
}

pub struct LeaseMgr {
    /// key: "{root}\u{0}{rel}" of the target file path
    leases: Mutex<HashMap<String, Lease>>,
    fencing: AtomicU64,
    ttl: Duration,
}

pub fn lease_key(root: &str, rel: &str) -> String {
    format!("{}\u{0}{}", root, rel)
}

impl LeaseMgr {
    pub fn new(ttl: Duration) -> Self {
        LeaseMgr { leases: Mutex::new(HashMap::new()), fencing: AtomicU64::new(1), ttl }
    }

    /// Acquires (or re-acquires, for the same session) the write lease.
    pub fn acquire(
        &self,
        root: &str,
        rel: &str,
        session: &str,
        base: Option<(u64, i64)>,
        fb_handle: String,
    ) -> NfsResult<Lease> {
        let key = lease_key(root, rel);
        let mut map = self.leases.lock().unwrap();
        let now = Instant::now();
        if let Some(existing) = map.get(&key) {
            if existing.expires > now && existing.session != session {
                return Err(NfsError::new(
                    ErrorCode::LeaseConflict,
                    format!("write lease held by another session for '{}'", rel),
                )
                .with("holder_session", Value::String(existing.session.clone())));
            }
        }
        let lease = Lease {
            lease_id: format!("l_{}", uuid::Uuid::new_v4().simple()),
            session: session.to_string(),
            seq: self.fencing.fetch_add(1, Ordering::SeqCst),
            expires: now + self.ttl,
            base,
            fb_handle,
        };
        map.insert(key, lease.clone());
        Ok(lease)
    }

    pub fn get(&self, root: &str, rel: &str) -> Option<Lease> {
        let map = self.leases.lock().unwrap();
        map.get(&lease_key(root, rel)).filter(|l| l.expires > Instant::now()).cloned()
    }

    /// Validates that (session, lease_id) still holds the lease and refreshes it.
    pub fn validate(&self, root: &str, rel: &str, session: &str, lease_id: &str) -> NfsResult<Lease> {
        let mut map = self.leases.lock().unwrap();
        let key = lease_key(root, rel);
        match map.get_mut(&key) {
            Some(l) if l.lease_id == lease_id && l.session == session => {
                if l.expires <= Instant::now() {
                    map.remove(&key);
                    return Err(NfsError::new(ErrorCode::LeaseConflict, "lease expired"));
                }
                l.expires = Instant::now() + self.ttl;
                Ok(l.clone())
            }
            Some(l) => Err(NfsError::new(
                ErrorCode::LeaseConflict,
                "lease lost (superseded by another writer)",
            )
            .with("holder_session", Value::String(l.session.clone()))),
            None => Err(NfsError::new(ErrorCode::LeaseConflict, "no active lease")),
        }
    }

    pub fn release(&self, root: &str, rel: &str, lease_id: &str) {
        let mut map = self.leases.lock().unwrap();
        let key = lease_key(root, rel);
        if map.get(&key).map(|l| l.lease_id == lease_id).unwrap_or(false) {
            map.remove(&key);
        }
    }

    pub fn release_session(&self, session: &str) -> Vec<Lease> {
        let mut map = self.leases.lock().unwrap();
        let dropped: Vec<Lease> =
            map.values().filter(|l| l.session == session).cloned().collect();
        map.retain(|_, l| l.session != session);
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_window() {
        let m = SessionMgr::new(4);
        let s = m.create();
        assert!(m.exists(&s));
        match m.check_seq(&s, 1).unwrap() {
            SeqDisposition::Execute => {}
            _ => panic!("expected execute"),
        }
        m.record_seq(&s, 1, serde_json::json!({"r":1}));
        // Duplicate → replay.
        match m.check_seq(&s, 1).unwrap() {
            SeqDisposition::Replay(v) => assert_eq!(v["r"], 1),
            _ => panic!("expected replay"),
        }
        for i in 2..=10 {
            m.record_seq(&s, i, serde_json::json!({"r": i}));
        }
        // Far below the window → SEQ_OUT_OF_WINDOW.
        let err = m.check_seq(&s, 2).unwrap_err();
        assert_eq!(err.code, ErrorCode::SeqOutOfWindow);
        // Unknown session.
        assert!(m.check_seq("sess_nope", 1).is_err());
        assert!(m.remove(&s));
        assert!(!m.exists(&s));
    }

    #[test]
    fn lease_conflict_and_fencing() {
        let m = LeaseMgr::new(Duration::from_secs(60));
        let l1 = m.acquire("home", "a.txt", "sess_a", None, "fb1".into()).unwrap();
        let err = m.acquire("home", "a.txt", "sess_b", None, "fb2".into()).unwrap_err();
        assert_eq!(err.code, ErrorCode::LeaseConflict);
        // Same session may re-open; fencing seq advances.
        let l2 = m.acquire("home", "a.txt", "sess_a", None, "fb3".into()).unwrap();
        assert!(l2.seq > l1.seq);
        // Old lease id no longer validates.
        assert!(m.validate("home", "a.txt", "sess_a", &l1.lease_id).is_err());
        assert!(m.validate("home", "a.txt", "sess_a", &l2.lease_id).is_ok());
        m.release("home", "a.txt", &l2.lease_id);
        assert!(m.get("home", "a.txt").is_none());
        // Different paths don't conflict.
        m.acquire("home", "b.txt", "sess_a", None, "fb4".into()).unwrap();
        m.acquire("home", "c.txt", "sess_b", None, "fb5".into()).unwrap();
    }

    #[test]
    fn lease_expiry() {
        let m = LeaseMgr::new(Duration::from_millis(0));
        let l = m.acquire("home", "x", "s1", None, "fb".into()).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        // Expired lease can be taken by another session.
        assert!(m.acquire("home", "x", "s2", None, "fb2".into()).is_ok());
        let _ = l;
    }

    #[test]
    fn release_session_drops_all() {
        let m = LeaseMgr::new(Duration::from_secs(60));
        m.acquire("home", "1", "s1", None, "f1".into()).unwrap();
        m.acquire("home", "2", "s1", None, "f2".into()).unwrap();
        m.acquire("home", "3", "s2", None, "f3".into()).unwrap();
        let dropped = m.release_session("s1");
        assert_eq!(dropped.len(), 2);
        assert!(m.get("home", "1").is_none());
        assert!(m.get("home", "3").is_some());
    }
}
