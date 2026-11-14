//! What the service keeps per person across Workspaces (UI 框架 PRD §4.6, §9, §12; K4, K6): `users.sqlite` in the
//! service data folder. Each Workspace has its own origin, so the browser's storage cannot carry a preference from one
//! Workspace to another (or to another device); and the home page cannot read what a Workspace page left there.
//!
//! - **Preferences** (`user.get_prefs` / `user.set_prefs`): one JSON object per principal, never part of any
//!   Workspace's document, never exported, cloned or carried by a template. Top-level keys are the unit of a merge.
//! - **Opened** (`ws.get_info { record_open: true }`): when the caller last opened each Workspace, for the home page's
//!   “最近” and `ws.list`'s `opened_at`.

use crate::docdb::db_err;
use crate::workspace::Caller;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{Code, Deny, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS user_prefs (
  principal   TEXT PRIMARY KEY,
  prefs_json  TEXT NOT NULL,
  revision    INTEGER NOT NULL,
  updated_ms  INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS user_opened (
  principal    TEXT NOT NULL,
  workspace_id TEXT NOT NULL,
  opened_ms    INTEGER NOT NULL,
  PRIMARY KEY (principal, workspace_id)
) WITHOUT ROWID;
"#;

/// The whole preference object of one person, canonical JSON.
pub const MAX_PREFS_BYTES: usize = 256 * 1024;
/// Top-level keys one person may keep.
pub const MAX_PREF_KEYS: usize = 500;
/// The length of a top-level key.
pub const MAX_PREF_KEY_LEN: usize = 128;

pub struct UserStore {
    conn: Mutex<Connection>,
}

/// Preferences belong to a person who signs in again as the same principal: an account, an External DID, a service or
/// an Agent. A link's visitor (a guest token, a display or prompter link) has no lasting identity to keep them under.
fn require_person(caller: &Caller) -> WsResult<()> {
    if caller.is_guest() || caller.kind.as_deref() == Some("run") {
        return Err(WsError::refused(Code::PermissionDenied, Deny::SessionKindNotAllowed, "this kind of session keeps no personal preferences"));
    }
    Ok(())
}

fn check_key(k: &str) -> WsResult<()> {
    if k.is_empty() || k.len() > MAX_PREF_KEY_LEN || k.chars().any(char::is_control) {
        return Err(WsError::invalid_op(format!("invalid preference key {k:?} (1–{MAX_PREF_KEY_LEN} bytes, no control characters)")));
    }
    Ok(())
}

impl UserStore {
    pub fn open(path: &Path) -> WsResult<UserStore> {
        let conn = Connection::open(path).map_err(db_err)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
        conn.execute_batch(DDL).map_err(db_err)?;
        Ok(UserStore { conn: Mutex::new(conn) })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> WsResult<T>) -> WsResult<T> {
        let conn = self.conn.lock().map_err(|_| WsError::io("user store lock poisoned"))?;
        f(&conn)
    }

    fn read(c: &Connection, principal: &str) -> WsResult<(Map<String, Value>, i64, Option<i64>)> {
        let row: Option<(String, i64, i64)> = c
            .query_row("SELECT prefs_json, revision, updated_ms FROM user_prefs WHERE principal = ?1", [principal], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()
            .map_err(db_err)?;
        Ok(match row {
            Some((text, rev, at)) => (serde_json::from_str::<Value>(&text).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default(), rev, Some(at)),
            None => (Map::new(), 0, None),
        })
    }

    /// `user.get_prefs`: the caller's preferences (`{}` and revision 0 when it kept none).
    pub fn get_prefs(&self, caller: &Caller) -> WsResult<Value> {
        require_person(caller)?;
        self.with(|c| {
            let (prefs, rev, at) = Self::read(c, &caller.principal)?;
            Ok(json!({ "ok": true, "principal": caller.principal, "prefs": prefs, "revision": rev, "updated_at": at.map(format_utc_ms) }))
        })
    }

    /// `user.set_prefs { prefs? | merge?, expect? }`: `prefs` replaces the whole object; `merge: { key: value | null }`
    /// sets those top-level keys (null removes one) and keeps the rest. `expect` (the revision the writer read) makes the
    /// write conditional: `REVISION_CONFLICT` with the current revision when it moved. Every write bumps the revision.
    pub fn set_prefs(&self, caller: &Caller, p: &Value, now_ms: i64) -> WsResult<Value> {
        require_person(caller)?;
        let whole = p.get("prefs").filter(|v| !v.is_null());
        let merge = p.get("merge").filter(|v| !v.is_null());
        let (whole, merge) = match (whole, merge) {
            (Some(_), Some(_)) => return Err(WsError::invalid_op("give prefs (the whole object) or merge (some keys), not both")),
            (None, None) => return Err(WsError::invalid_op("prefs or merge required")),
            (w, m) => (w.map(|v| v.as_object().ok_or_else(|| WsError::invalid_op("prefs must be an object"))).transpose()?, m.map(|v| v.as_object().ok_or_else(|| WsError::invalid_op("merge must be an object"))).transpose()?),
        };
        let expect = match p.get("expect") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_i64().ok_or_else(|| WsError::invalid_op("expect must be the revision (an integer)"))?),
        };
        self.with(|c| {
            let tx = c.unchecked_transaction().map_err(db_err)?;
            let (mut prefs, rev, _) = Self::read(&tx, &caller.principal)?;
            if let Some(want) = expect.filter(|w| *w != rev) {
                return Err(WsError::new(Code::RevisionConflict, "your preferences changed elsewhere; read them again").with_data(json!({ "expected": want, "current": rev })));
            }
            match (whole, merge) {
                (Some(w), _) => prefs = w.clone(),
                (_, Some(m)) => {
                    for (k, v) in m {
                        if v.is_null() {
                            prefs.remove(k);
                        } else {
                            prefs.insert(k.clone(), v.clone());
                        }
                    }
                }
                _ => unreachable!(),
            }
            for k in prefs.keys() {
                check_key(k)?;
            }
            if prefs.len() > MAX_PREF_KEYS {
                return Err(WsError::limit(format!("at most {MAX_PREF_KEYS} preference keys")));
            }
            let text = Value::Object(prefs.clone()).to_string();
            if text.len() > MAX_PREFS_BYTES {
                return Err(WsError::limit(format!("preferences are limited to {MAX_PREFS_BYTES} bytes")));
            }
            tx.execute(
                "INSERT OR REPLACE INTO user_prefs (principal, prefs_json, revision, updated_ms) VALUES (?1, ?2, ?3, ?4)",
                params![caller.principal, text, rev + 1, now_ms],
            )
            .map_err(db_err)?;
            tx.commit().map_err(db_err)?;
            Ok(json!({ "ok": true, "prefs": prefs, "revision": rev + 1, "updated_at": format_utc_ms(now_ms) }))
        })
    }

    /// The caller opened `workspace_id` now (its Workspace session started). Guests and link visitors are not recorded.
    pub fn record_open(&self, caller: &Caller, workspace_id: &str, now_ms: i64) -> WsResult<()> {
        if require_person(caller).is_err() {
            return Ok(());
        }
        self.with(|c| {
            c.execute(
                "INSERT INTO user_opened (principal, workspace_id, opened_ms) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (principal, workspace_id) DO UPDATE SET opened_ms = excluded.opened_ms",
                params![caller.principal, workspace_id, now_ms],
            )
            .map(|_| ())
            .map_err(db_err)
        })
    }

    /// When `principal` last opened each Workspace (unix ms).
    pub fn opened(&self, principal: &str) -> WsResult<HashMap<String, i64>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT workspace_id, opened_ms FROM user_opened WHERE principal = ?1").map_err(db_err)?;
            let rows = st.query_map([principal], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).map_err(db_err)?;
            rows.collect::<rusqlite::Result<HashMap<_, _>>>().map_err(db_err)
        })
    }
}
