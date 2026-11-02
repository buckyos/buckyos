//! The session registry (第四期规划 §3.1, TODO I1): `sessions.sqlite` in the service data folder. One row per
//! presentation session — its state machine, owner and presenter, policy, baseline and archive — plus its invite
//! links, its participants and what people post after the class. The run Workspace (`purpose=session`) holds the
//! content, the events and the stage; this file only knows where they are.

use crate::docdb::db_err;
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
  session_id          TEXT PRIMARY KEY,
  source_workspace_id TEXT NOT NULL,
  workspace_id        TEXT NOT NULL,
  path_id             TEXT NOT NULL,
  title               TEXT NOT NULL,
  owner               TEXT NOT NULL,
  presenter           TEXT NOT NULL,
  state               TEXT NOT NULL,
  access              TEXT NOT NULL DEFAULT 'open',
  posts               TEXT NOT NULL DEFAULT 'open',
  policy_json         TEXT NOT NULL,
  baseline_json       TEXT NOT NULL,
  archive_json        TEXT,
  controller          TEXT,
  created_ms          INTEGER NOT NULL,
  started_ms          INTEGER,
  ended_ms            INTEGER,
  saved_ms            INTEGER,
  expires_ms          INTEGER,
  deleted_ms          INTEGER
);
CREATE INDEX IF NOT EXISTS sessions_source ON sessions(source_workspace_id);
CREATE INDEX IF NOT EXISTS sessions_run ON sessions(workspace_id);
CREATE TABLE IF NOT EXISTS session_invites (
  invite_id   TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL,
  kind        TEXT NOT NULL,
  token       TEXT NOT NULL UNIQUE,
  label       TEXT,
  config_json TEXT NOT NULL DEFAULT '{}',
  created_by  TEXT NOT NULL,
  created_ms  INTEGER NOT NULL,
  revoked_ms  INTEGER
);
CREATE INDEX IF NOT EXISTS session_invites_session ON session_invites(session_id);
CREATE TABLE IF NOT EXISTS session_participants (
  session_id   TEXT NOT NULL,
  principal    TEXT NOT NULL,
  class        TEXT NOT NULL,
  level        TEXT NOT NULL,
  name         TEXT NOT NULL,
  role         TEXT NOT NULL,
  via_invite   TEXT,
  joined_ms    INTEGER NOT NULL,
  last_seen_ms INTEGER NOT NULL,
  online       INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (session_id, principal)
);
CREATE TABLE IF NOT EXISTS session_posts (
  post_id     TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL,
  kind        TEXT NOT NULL,
  author      TEXT NOT NULL,
  author_name TEXT,
  body        TEXT,
  score       INTEGER,
  t_rel_ms    INTEGER,
  created_ms  INTEGER NOT NULL,
  deleted_ms  INTEGER
);
CREATE INDEX IF NOT EXISTS session_posts_session ON session_posts(session_id, created_ms);
"#;

/// Session states (§3.2): `prepared → live → ended → archived`; an ended session nobody saved is `expired` once
/// its retention passed (its run Workspace is gone, the row stays).
pub const PREPARED: &str = "prepared";
pub const LIVE: &str = "live";
pub const ENDED: &str = "ended";
pub const ARCHIVED: &str = "archived";
pub const EXPIRED: &str = "expired";

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_id: String,
    pub source_workspace_id: String,
    pub workspace_id: String,
    pub path_id: String,
    pub title: String,
    pub owner: String,
    pub presenter: String,
    pub state: String,
    pub access: String,
    pub posts: String,
    pub policy: Value,
    pub baseline: Value,
    pub archive: Option<Value>,
    pub controller: Option<String>,
    pub created_ms: i64,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub saved_ms: Option<i64>,
    pub expires_ms: Option<i64>,
    pub deleted_ms: Option<i64>,
}

const SESSION_COLS: &str = "session_id, source_workspace_id, workspace_id, path_id, title, owner, presenter, state, access, posts, policy_json, \
                            baseline_json, archive_json, controller, created_ms, started_ms, ended_ms, saved_ms, expires_ms, deleted_ms";

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn session_row(r: &Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        session_id: r.get(0)?,
        source_workspace_id: r.get(1)?,
        workspace_id: r.get(2)?,
        path_id: r.get(3)?,
        title: r.get(4)?,
        owner: r.get(5)?,
        presenter: r.get(6)?,
        state: r.get(7)?,
        access: r.get(8)?,
        posts: r.get(9)?,
        policy: parse(&r.get::<_, String>(10)?),
        baseline: parse(&r.get::<_, String>(11)?),
        archive: r.get::<_, Option<String>>(12)?.map(|s| parse(&s)),
        controller: r.get(13)?,
        created_ms: r.get(14)?,
        started_ms: r.get(15)?,
        ended_ms: r.get(16)?,
        saved_ms: r.get(17)?,
        expires_ms: r.get(18)?,
        deleted_ms: r.get(19)?,
    })
}

impl SessionRow {
    pub fn alive(&self) -> bool {
        self.deleted_ms.is_none() && self.state != EXPIRED
    }
    /// Whether participants may still write: answers, claims, reactions (§3.2: ending stops live tasks).
    pub fn is_live(&self) -> bool {
        self.state == LIVE && self.deleted_ms.is_none()
    }
    pub fn guest_interact(&self) -> bool {
        self.policy["guest_interact"] == json!(true)
    }
}

#[derive(Debug, Clone)]
pub struct InviteRow {
    pub invite_id: String,
    pub session_id: String,
    pub kind: String,
    pub token: String,
    pub label: Option<String>,
    pub config: Value,
    pub created_by: String,
    pub created_ms: i64,
    pub revoked_ms: Option<i64>,
}

fn invite_row(r: &Row) -> rusqlite::Result<InviteRow> {
    Ok(InviteRow {
        invite_id: r.get(0)?,
        session_id: r.get(1)?,
        kind: r.get(2)?,
        token: r.get(3)?,
        label: r.get(4)?,
        config: parse(&r.get::<_, String>(5)?),
        created_by: r.get(6)?,
        created_ms: r.get(7)?,
        revoked_ms: r.get(8)?,
    })
}

impl InviteRow {
    pub fn to_json(&self) -> Value {
        json!({ "invite_id": self.invite_id, "kind": self.kind, "token": self.token, "label": self.label, "config": self.config,
                "created_by": self.created_by, "created_at": aiworkspace_core::value::format_utc_ms(self.created_ms),
                "revoked": self.revoked_ms.is_some() })
    }
}

#[derive(Debug, Clone)]
pub struct ParticipantRow {
    pub principal: String,
    pub class: String,
    pub level: String,
    pub name: String,
    pub role: String,
    pub via_invite: Option<String>,
    pub joined_ms: i64,
    pub last_seen_ms: i64,
    pub online: bool,
}

pub struct Sessions {
    conn: Mutex<Connection>,
}

impl Sessions {
    pub fn open(path: &Path) -> WsResult<Sessions> {
        let conn = Connection::open(path).map_err(db_err)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
        conn.pragma_update(None, "synchronous", "FULL").map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
        conn.execute_batch(DDL).map_err(db_err)?;
        Ok(Sessions { conn: Mutex::new(conn) })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> WsResult<T> {
        let conn = self.conn.lock().map_err(|_| WsError::io("session registry lock poisoned"))?;
        f(&conn).map_err(db_err)
    }

    pub fn insert(&self, s: &SessionRow) -> WsResult<()> {
        self.with(|c| {
            c.execute(
                &format!("INSERT INTO sessions ({SESSION_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)"),
                params![
                    s.session_id, s.source_workspace_id, s.workspace_id, s.path_id, s.title, s.owner, s.presenter, s.state, s.access, s.posts,
                    s.policy.to_string(), s.baseline.to_string(), s.archive.as_ref().map(Value::to_string), s.controller, s.created_ms,
                    s.started_ms, s.ended_ms, s.saved_ms, s.expires_ms, s.deleted_ms
                ],
            )
            .map(|_| ())
        })
    }

    pub fn get(&self, session_id: &str) -> WsResult<Option<SessionRow>> {
        self.with(|c| c.query_row(&format!("SELECT {SESSION_COLS} FROM sessions WHERE session_id = ?1"), [session_id], session_row).optional())
    }

    /// The session, or `NOT_FOUND` / `SESSION_ENDED`-style refusals for what callers may no longer reach.
    pub fn require(&self, session_id: &str) -> WsResult<SessionRow> {
        match self.get(session_id)? {
            Some(s) if s.deleted_ms.is_none() => Ok(s),
            _ => Err(WsError::sub(Code::NotFound, "SESSION_NOT_FOUND", "session not found")),
        }
    }

    pub fn by_run_workspace(&self, workspace_id: &str) -> WsResult<Option<SessionRow>> {
        self.with(|c| c.query_row(&format!("SELECT {SESSION_COLS} FROM sessions WHERE workspace_id = ?1"), [workspace_id], session_row).optional())
    }

    pub fn list_for_source(&self, source_workspace_id: &str) -> WsResult<Vec<SessionRow>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {SESSION_COLS} FROM sessions WHERE source_workspace_id = ?1 AND deleted_ms IS NULL ORDER BY created_ms DESC"))?;
            let rows = st.query_map([source_workspace_id], session_row)?.collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
    }

    /// The sessions `principal` owns, presents or joined, across Workspaces, newest first.
    pub fn list_for_principal(&self, principal: &str, limit: usize) -> WsResult<Vec<SessionRow>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {SESSION_COLS} FROM sessions s WHERE deleted_ms IS NULL AND (owner = ?1 OR presenter = ?1 OR EXISTS \
                 (SELECT 1 FROM session_participants p WHERE p.session_id = s.session_id AND p.principal = ?1)) ORDER BY created_ms DESC LIMIT ?2"
            ))?;
            let rows = st.query_map(params![principal, limit as i64], session_row)?.collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
    }

    /// Sessions in one of `states` (the sweeper and the start-up recovery).
    pub fn in_states(&self, states: &[&str]) -> WsResult<Vec<SessionRow>> {
        let list = states.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {SESSION_COLS} FROM sessions WHERE deleted_ms IS NULL AND state IN ({list})"))?;
            let rows = st.query_map([], session_row)?.collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
    }

    /// Write back the mutable columns of `s`.
    pub fn update(&self, s: &SessionRow) -> WsResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE sessions SET title = ?2, presenter = ?3, state = ?4, access = ?5, posts = ?6, policy_json = ?7, baseline_json = ?8, archive_json = ?9, \
                 controller = ?10, started_ms = ?11, ended_ms = ?12, saved_ms = ?13, expires_ms = ?14, deleted_ms = ?15 WHERE session_id = ?1",
                params![
                    s.session_id, s.title, s.presenter, s.state, s.access, s.posts, s.policy.to_string(), s.baseline.to_string(),
                    s.archive.as_ref().map(Value::to_string), s.controller, s.started_ms, s.ended_ms, s.saved_ms, s.expires_ms, s.deleted_ms
                ],
            )
            .map(|_| ())
        })
    }

    // ---- invites ----

    pub fn add_invite(&self, i: &InviteRow) -> WsResult<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO session_invites (invite_id, session_id, kind, token, label, config_json, created_by, created_ms, revoked_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![i.invite_id, i.session_id, i.kind, i.token, i.label, i.config.to_string(), i.created_by, i.created_ms, i.revoked_ms],
            )
            .map(|_| ())
        })
    }

    pub fn invites(&self, session_id: &str) -> WsResult<Vec<InviteRow>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT invite_id, session_id, kind, token, label, config_json, created_by, created_ms, revoked_ms FROM session_invites WHERE session_id = ?1 ORDER BY created_ms",
            )?;
            let rows = st.query_map([session_id], invite_row)?.collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
    }

    /// The invite a token names, while it is not revoked.
    pub fn invite_by_token(&self, token: &str) -> WsResult<Option<InviteRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT invite_id, session_id, kind, token, label, config_json, created_by, created_ms, revoked_ms FROM session_invites WHERE token = ?1 AND revoked_ms IS NULL",
                [token],
                invite_row,
            )
            .optional()
        })
    }

    pub fn revoke_invite(&self, session_id: &str, invite_id: &str, now_ms: i64) -> WsResult<bool> {
        self.with(|c| {
            c.execute("UPDATE session_invites SET revoked_ms = ?3 WHERE session_id = ?1 AND invite_id = ?2 AND revoked_ms IS NULL", params![session_id, invite_id, now_ms]).map(|n| n > 0)
        })
    }

    // ---- participants ----

    pub fn upsert_participant(&self, session_id: &str, p: &ParticipantRow) -> WsResult<bool> {
        self.with(|c| {
            let existed: bool = c
                .query_row("SELECT 1 FROM session_participants WHERE session_id = ?1 AND principal = ?2", params![session_id, p.principal], |_| Ok(true))
                .optional()?
                .unwrap_or(false);
            c.execute(
                "INSERT INTO session_participants (session_id, principal, class, level, name, role, via_invite, joined_ms, last_seen_ms, online) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(session_id, principal) DO UPDATE SET \
                 name = excluded.name, role = excluded.role, last_seen_ms = excluded.last_seen_ms, online = excluded.online, \
                 via_invite = COALESCE(session_participants.via_invite, excluded.via_invite)",
                params![session_id, p.principal, p.class, p.level, p.name, p.role, p.via_invite, p.joined_ms, p.last_seen_ms, p.online],
            )?;
            Ok(!existed)
        })
    }

    pub fn participant(&self, session_id: &str, principal: &str) -> WsResult<Option<ParticipantRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT principal, class, level, name, role, via_invite, joined_ms, last_seen_ms, online FROM session_participants WHERE session_id = ?1 AND principal = ?2",
                params![session_id, principal],
                participant_row,
            )
            .optional()
        })
    }

    pub fn participants(&self, session_id: &str) -> WsResult<Vec<ParticipantRow>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT principal, class, level, name, role, via_invite, joined_ms, last_seen_ms, online FROM session_participants WHERE session_id = ?1 ORDER BY joined_ms, principal",
            )?;
            let rows = st.query_map([session_id], participant_row)?.collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
    }

    pub fn set_presence(&self, session_id: &str, principal: &str, online: bool, now_ms: i64) -> WsResult<()> {
        self.with(|c| {
            c.execute("UPDATE session_participants SET online = ?3, last_seen_ms = ?4 WHERE session_id = ?1 AND principal = ?2", params![session_id, principal, online, now_ms]).map(|_| ())
        })
    }

    /// Everyone offline: what a restarted service starts from (no connection survived it).
    pub fn all_offline(&self) -> WsResult<()> {
        self.with(|c| c.execute("UPDATE session_participants SET online = 0", []).map(|_| ()))
    }

    /// Participants who came in through `invite_id` (its revocation takes their grants back).
    pub fn participants_via(&self, session_id: &str, invite_id: &str) -> WsResult<Vec<String>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT principal FROM session_participants WHERE session_id = ?1 AND via_invite = ?2")?;
            let rows = st.query_map(params![session_id, invite_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>();
            rows
        })
    }

    // ---- posts (§8: after the class) ----

    #[allow(clippy::too_many_arguments)]
    pub fn add_post(&self, post_id: &str, session_id: &str, kind: &str, author: &str, author_name: Option<&str>, body: Option<&str>, score: Option<i64>, t_rel_ms: Option<i64>, now_ms: i64) -> WsResult<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO session_posts (post_id, session_id, kind, author, author_name, body, score, t_rel_ms, created_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![post_id, session_id, kind, author, author_name, body, score, t_rel_ms, now_ms],
            )
            .map(|_| ())
        })
    }

    pub fn posts(&self, session_id: &str) -> WsResult<Vec<Value>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT post_id, kind, author, author_name, body, score, t_rel_ms, created_ms FROM session_posts WHERE session_id = ?1 AND deleted_ms IS NULL ORDER BY created_ms, post_id",
            )?;
            let rows = st
                .query_map([session_id], |r| {
                    Ok(json!({ "post_id": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "author": r.get::<_, String>(2)?,
                               "author_name": r.get::<_, Option<String>>(3)?, "body": r.get::<_, Option<String>>(4)?, "score": r.get::<_, Option<i64>>(5)?,
                               "t_rel_ms": r.get::<_, Option<i64>>(6)?, "created_at": aiworkspace_core::value::format_utc_ms(r.get::<_, i64>(7)?) }))
                })?
                .collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
    }

    pub fn post_author(&self, session_id: &str, post_id: &str) -> WsResult<Option<String>> {
        self.with(|c| c.query_row("SELECT author FROM session_posts WHERE session_id = ?1 AND post_id = ?2 AND deleted_ms IS NULL", params![session_id, post_id], |r| r.get(0)).optional())
    }

    pub fn delete_post(&self, session_id: &str, post_id: &str, now_ms: i64) -> WsResult<bool> {
        self.with(|c| c.execute("UPDATE session_posts SET deleted_ms = ?3 WHERE session_id = ?1 AND post_id = ?2 AND deleted_ms IS NULL", params![session_id, post_id, now_ms]).map(|n| n > 0))
    }
}

fn participant_row(r: &Row) -> rusqlite::Result<ParticipantRow> {
    Ok(ParticipantRow {
        principal: r.get(0)?,
        class: r.get(1)?,
        level: r.get(2)?,
        name: r.get(3)?,
        role: r.get(4)?,
        via_invite: r.get(5)?,
        joined_ms: r.get(6)?,
        last_seen_ms: r.get(7)?,
        online: r.get(8)?,
    })
}
