//! Presentation sessions (第四期规划 §3–§8; `notepads/aiworkspace-phase4-todo.md` §1). A session is a row of the
//! registry (`registry.rs`) and a **run Workspace** (`purpose=session`): a whole-database clone of the source taken
//! when the room is created, cut back to the clone point (no earlier history) and with the speaker notes moved into
//! a presenter-scoped table. Everything a session records lives in the run Workspace: its content and Commits, its
//! timeline (`session_events`, written in the Commit's own transaction), the authoritative stage, the feeds Blocks
//! bind to (`feeds.rs`), the tasks, answers and published results.

pub mod feeds;
pub mod registry;

use crate::docdb::{db_err, meta_get, meta_set};
use crate::objects::{bucket_id, ASSETS};
use crate::service::Service;
use crate::source::Fetched;
use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::canonical::sha256_hex;
use aiworkspace_core::id::is_prefixed_id;
use aiworkspace_core::model::*;
use aiworkspace_core::types::session_feed;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{richtext, Code, WsError, WsResult};
use base64::Engine;
use registry::{InviteRow, ParticipantRow, SessionRow, ARCHIVED, ENDED, EXPIRED, LIVE, PREPARED};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// `workspace_meta.purpose` of a session's run Workspace.
pub const PURPOSE_SESSION: &str = "session";
/// Unsaved sessions are kept this long after they end (Q10: a service setting, 7 days by default).
pub const DEFAULT_RETENTION_MS: i64 = 7 * 24 * 3_600_000;
/// Event payloads are bounded: a sample batch, a media state, a reaction.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
/// The capabilities a presenter who is not the owner holds on the run Workspace (everything but `manage`).
const PRESENTER_CAPS: u16 = CapSet::ALL.0 & !(Cap::Manage as u16);

const EVENTS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS session_events (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  t_rel_ms     INTEGER NOT NULL,
  received_ms  INTEGER NOT NULL,
  actor        TEXT NOT NULL,
  scope        TEXT NOT NULL,
  track        TEXT NOT NULL,
  type         TEXT NOT NULL,
  command_id   TEXT,
  schema       INTEGER NOT NULL DEFAULT 1,
  payload_json TEXT NOT NULL,
  commit_seq   INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS session_events_command ON session_events(actor, command_id) WHERE command_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS session_events_type ON session_events(type, seq);
"#;

fn io(e: std::io::Error) -> WsError {
    WsError::io(e.to_string())
}

/// A session Commit the planner did not accept, as an error carrying the commit result.
fn commit_error(result: &Value) -> WsError {
    let code = result["code"].as_str().and_then(Code::parse).unwrap_or(Code::StorageIoError);
    let detail = result["errors"][0]["detail"].as_str().or_else(|| result["conflicts"][0]["detail"].as_str()).unwrap_or("the session write was not accepted");
    WsError::new(code, detail).with_data(json!({ "commit": result }))
}

fn ended() -> WsError {
    WsError::sub(Code::InvalidOperation, "SESSION_NOT_LIVE", "the session is not running")
}

fn s<'a>(p: &'a Value, key: &str) -> WsResult<&'a str> {
    p.get(key).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{key} required")))
}

/// Who acts in a session, as the service established it (never what a request body says).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The session's presenter (the owner too, when they are one).
    Presenter,
    /// The owner who is not the presenter: manages, saves, reads everything, does not control the stage.
    Owner,
    /// A joined audience member.
    Participant,
    /// A public display (`dt_` link): reads the public projection, has no identity.
    Display,
    /// A prompter (`pt_` link): the presenter's remote — commands, notes, the stage.
    Prompter,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Presenter => "presenter",
            Role::Owner => "owner",
            Role::Participant => "audience",
            Role::Display => "display",
            Role::Prompter => "prompter",
        }
    }
    /// Controls the stage and runs tasks.
    pub fn presents(&self) -> bool {
        matches!(self, Role::Presenter | Role::Prompter)
    }
    /// Reads the presenter's data (notes, every answer, presence).
    pub fn reviews(&self) -> bool {
        matches!(self, Role::Presenter | Role::Owner | Role::Prompter)
    }
}

#[derive(Debug, Clone)]
pub struct Actor {
    pub principal: String,
    pub role: Role,
    /// `account | external | guest | display`.
    pub class: String,
    /// `verified | guest` (§4: guests' answers carry their identity level).
    pub level: &'static str,
    pub name: String,
}

impl Actor {
    pub fn is_owner(&self, s: &SessionRow) -> bool {
        self.principal == s.owner && matches!(self.role, Role::Owner | Role::Presenter)
    }
    /// The Access the actor reads the run Workspace with in session methods (the grants say the same for direct reads).
    pub fn access(&self) -> Access {
        let caps = if self.role.reviews() { CapSet::ALL } else { CapSet(Cap::Read as u16) };
        Access { principal: self.principal.clone(), ws_caps: caps, scoped: BTreeMap::new() }
    }
}

/// Room policy (§3.1, Q03, Q13): who interacts and what the room expects.
pub fn normalize_policy(v: Option<&Value>) -> WsResult<Value> {
    let empty = json!({});
    let v = v.filter(|v| !v.is_null()).unwrap_or(&empty);
    let o = v.as_object().ok_or_else(|| WsError::invalid_op("policy must be an object"))?;
    for k in o.keys() {
        if !matches!(k.as_str(), "guest_interact" | "voice" | "expected_audience" | "media_limit_mib" | "show_names") {
            return Err(WsError::invalid_op(format!("unknown policy key {k}")));
        }
    }
    let flag = |k: &str, d: bool| -> WsResult<bool> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(d),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(WsError::invalid_op(format!("policy.{k} must be a boolean"))),
        }
    };
    let count = |k: &str, d: u64, max: u64| -> WsResult<u64> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(d),
            Some(v) => v.as_u64().filter(|n| (1..=max).contains(n)).ok_or_else(|| WsError::invalid_op(format!("policy.{k} must be a count in 1…{max}"))),
        }
    };
    Ok(json!({ "guest_interact": flag("guest_interact", false)?, "voice": flag("voice", true)?, "show_names": flag("show_names", true)?,
               "expected_audience": count("expected_audience", 50, 10_000)?, "media_limit_mib": count("media_limit_mib", 2048, 2048)? }))
}

// ---------------------------------------------------------------------------
// events (§6.2)
// ---------------------------------------------------------------------------

/// What a Commit leaves on a session's timeline: written by `write_commit` inside its transaction.
pub(crate) fn commit_event(tx: &Connection, now_ms: i64, actor: &str, origin: &str, seq: u64, commit_id: &str, touched: &BTreeSet<&str>) -> WsResult<()> {
    let t_rel = t_rel(tx, now_ms)?;
    let track = match origin {
        "session" | "source" => "interaction",
        "system" => "diag",
        _ => "public",
    };
    let payload = json!({ "seq": seq, "commit_id": commit_id, "origin": origin, "touched": touched });
    tx.execute(
        "INSERT INTO session_events (t_rel_ms, received_ms, actor, scope, track, type, schema, payload_json, commit_seq) VALUES (?1, ?2, ?3, 'public', ?4, 'commit', 1, ?5, ?6)",
        params![t_rel, now_ms, actor, track, payload.to_string(), seq],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Demonstration time (§6.2): milliseconds since the session started, by the service's clock; 0 before the start.
fn t_rel(conn: &Connection, now_ms: i64) -> WsResult<i64> {
    let t0: Option<i64> = meta_get(conn, "session_t0")?.and_then(|s| s.parse().ok());
    Ok(t0.map(|t0| (now_ms - t0).max(0)).unwrap_or(0))
}

/// One event to append. `scope`: `public | presenter | participant:<p> | diag`; `track`: the §6.1 track.
pub struct NewEvent<'a> {
    pub actor: &'a str,
    pub scope: &'a str,
    pub track: &'a str,
    pub kind: &'a str,
    pub command_id: Option<&'a str>,
    pub data: Value,
    /// The demonstration time the sample was taken at (converted by the service from the client's clock); `None`: now.
    pub t_rel_ms: Option<i64>,
}

/// May `access` read an event of `scope`?
pub fn event_visible(access: &Access, scope: &str) -> bool {
    match scope {
        "public" => true,
        "presenter" | "diag" => access.reviewer(),
        s => s.strip_prefix(PARTICIPANT_PREFIX).is_some_and(|p| p == access.principal || access.reviewer()),
    }
}

fn event_json(session_id: &str, r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let seq: i64 = r.get(0)?;
    let payload: String = r.get(9)?;
    Ok(json!({ "seq": seq, "event_id": format!("{session_id}:{seq}"), "session_id": session_id, "t_rel_ms": r.get::<_, i64>(1)?,
               "received_at": format_utc_ms(r.get::<_, i64>(2)?), "actor": r.get::<_, String>(3)?, "scope": r.get::<_, String>(4)?,
               "track": r.get::<_, String>(5)?, "type": r.get::<_, String>(6)?, "command_id": r.get::<_, Option<String>>(7)?,
               "schema": r.get::<_, i64>(8)?, "data": serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null),
               "commit_seq": r.get::<_, Option<i64>>(10)? }))
}

const EVENT_COLS: &str = "seq, t_rel_ms, received_ms, actor, scope, track, type, command_id, schema, payload_json, commit_seq";

impl Workspace {
    pub fn is_session_run(&self) -> bool {
        self.session_events
    }

    pub fn session_id(&self) -> Option<String> {
        meta_get(&self.doc, "session_id").ok().flatten()
    }

    /// Demonstration time now.
    pub fn t_rel_now(&self) -> i64 {
        t_rel(&self.doc, (self.clock)()).unwrap_or(0)
    }

    pub fn session_t0(&self) -> Option<i64> {
        meta_get(&self.doc, "session_t0").ok().flatten().and_then(|s| s.parse().ok())
    }

    /// Append one event (its own transaction); a command id seen before from the same actor answers the first event.
    pub fn append_event(&self, ev: NewEvent) -> WsResult<(Value, bool)> {
        if !self.session_events {
            return Err(WsError::invalid_op("not a session workspace"));
        }
        if let Some(cmd) = ev.command_id {
            if let Some(prior) = self.event_by_command(ev.actor, cmd)? {
                return Ok((prior, true));
            }
        }
        let text = ev.data.to_string();
        if text.len() > MAX_EVENT_BYTES {
            return Err(WsError::limit("an event is limited to 64 KiB"));
        }
        let now = (self.clock)();
        let t = match ev.t_rel_ms {
            Some(t) => t.clamp(0, t_rel(&self.doc, now)?.max(0)),
            None => t_rel(&self.doc, now)?,
        };
        self.doc
            .execute(
                "INSERT INTO session_events (t_rel_ms, received_ms, actor, scope, track, type, command_id, schema, payload_json) VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8)",
                params![t, now, ev.actor, ev.scope, ev.track, ev.kind, ev.command_id, text],
            )
            .map_err(db_err)?;
        let seq = self.doc.last_insert_rowid();
        Ok((self.event(seq)?.unwrap_or(Value::Null), false))
    }

    fn event(&self, seq: i64) -> WsResult<Option<Value>> {
        let sid = self.session_id().unwrap_or_default();
        self.doc.query_row(&format!("SELECT {EVENT_COLS} FROM session_events WHERE seq = ?1"), [seq], |r| event_json(&sid, r)).optional().map_err(db_err)
    }

    pub fn event_by_command(&self, actor: &str, command_id: &str) -> WsResult<Option<Value>> {
        let sid = self.session_id().unwrap_or_default();
        self.doc
            .query_row(&format!("SELECT {EVENT_COLS} FROM session_events WHERE actor = ?1 AND command_id = ?2"), params![actor, command_id], |r| event_json(&sid, r))
            .optional()
            .map_err(db_err)
    }

    pub fn last_event_seq(&self) -> u64 {
        self.doc.query_row("SELECT COALESCE(MAX(seq), 0) FROM session_events", [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64
    }

    /// Events after `after`, those `access` may read: by scope, and a Commit's only when it touched something readable
    /// (its `touched` list cut to that).
    pub fn events_after(&self, access: &Access, after: u64, limit: usize, until_t_rel: Option<i64>) -> WsResult<(Vec<Value>, bool)> {
        let sid = self.session_id().unwrap_or_default();
        let mut st = self.doc.prepare_cached(&format!("SELECT {EVENT_COLS} FROM session_events WHERE seq > ?1 ORDER BY seq LIMIT ?2")).map_err(db_err)?;
        let rows: Vec<Value> = st.query_map(params![after as i64, (limit + 1) as i64], |r| event_json(&sid, r)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let more = rows.len() > limit;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for mut ev in rows.into_iter().take(limit) {
            if until_t_rel.is_some_and(|t| ev["t_rel_ms"].as_i64().unwrap_or(0) > t) {
                break;
            }
            if !event_visible(access, ev["scope"].as_str().unwrap_or("")) {
                continue;
            }
            if ev["type"] == json!("commit") {
                let mut seen = Vec::new();
                for id in ev["data"]["touched"].as_array().cloned().unwrap_or_default() {
                    let id = id.as_str().unwrap_or("");
                    if ctx.entity(id)?.is_some_and(|e| access.can_read(&ctx, &e).unwrap_or(false)) {
                        seen.push(json!(id));
                    }
                }
                if seen.is_empty() {
                    continue;
                }
                ev["data"]["touched"] = json!(seen);
            }
            out.push(ev);
        }
        Ok((out, more))
    }

    /// The authoritative stage (Q01-C): what the service last confirmed.
    pub fn stage(&self) -> Value {
        meta_get(&self.doc, "session_stage").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}))
    }

    /// The playable steps of the session's path as the run Workspace has them now: `{ id, kind, target_id, surface_id, title, transition }`.
    pub fn session_steps(&self) -> WsResult<Vec<Value>> {
        let path_id = meta_get(&self.doc, "session_path")?.unwrap_or_default();
        let now = self.now();
        let ctx = self.ctx(&now);
        let Some(path) = ctx.entity(&path_id)?.filter(|e| e.alive()) else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for step in path.payload.get("steps").and_then(Value::as_array).into_iter().flatten() {
            if step.get("enabled") == Some(&json!(false)) {
                continue;
            }
            let target_id = step["target"]["entity_id"].as_str().unwrap_or("");
            let Some(target) = ctx.entity(target_id)?.filter(|e| e.alive()) else { continue };
            let surface = if target.type_id == TYPE_VIEWPORT {
                target.payload.get("surface_ref").and_then(|r| r.get("entity_id")).and_then(Value::as_str).map(str::to_string)
            } else {
                let mut cur = target_id.to_string();
                let mut found = None;
                for _ in 0..256 {
                    let Some(edge) = ctx.edge(&cur)? else { break };
                    if ctx.entity(&edge.parent_id)?.is_some_and(|p| p.payload.get("kind") == Some(&json!("surface"))) {
                        found = Some(edge.parent_id.clone());
                        break;
                    }
                    cur = edge.parent_id;
                }
                found
            };
            let title = step.get("title").and_then(Value::as_str).or_else(|| target.payload.get("title").and_then(Value::as_str)).unwrap_or("");
            out.push(json!({ "id": step["id"], "kind": step["target"]["kind"], "target_id": target_id, "surface_id": surface, "title": title,
                             "transition": step.get("transition").cloned().unwrap_or(json!("auto")), "hide": step.get("hide").cloned().unwrap_or(json!([])) }));
        }
        Ok(out)
    }

    /// The scope of an entity, alive or not (`None`: unknown).
    pub fn entity_scope(&self, id: &str) -> WsResult<Option<String>> {
        self.doc.query_row("SELECT scope FROM entities WHERE entity_id = ?1", [id], |r| r.get(0)).optional().map_err(db_err)
    }

    /// The audience following the presenter (§5.1), kept with the stage without an event of its own.
    pub fn set_followers(&self, stage: &Value) -> WsResult<()> {
        meta_set(&self.doc, "session_stage", &stage.to_string())
    }

    fn set_stage(&self, stage: &Value, event: NewEvent) -> WsResult<Value> {
        let tx = self.doc.unchecked_transaction().map_err(db_err)?;
        meta_set(&tx, "session_stage", &stage.to_string())?;
        tx.commit().map_err(db_err)?;
        Ok(self.append_event(event)?.0)
    }
}

/// The transition a step change takes (§1, third phase §9): what the step says, else fly on the same Surface, fade
/// across Surfaces; the first step and a jump back to the same step fly or cut.
fn transition(from: Option<&Value>, to: &Value) -> &'static str {
    match to["transition"].as_str() {
        Some("fly") => return "fly",
        Some("fade") => return "fade",
        Some("cut") => return "cut",
        _ => {}
    }
    match from {
        Some(f) if f["surface_id"] == to["surface_id"] && !to["surface_id"].is_null() => "fly",
        Some(_) => "fade",
        None => "fade",
    }
}

// ---------------------------------------------------------------------------
// the run Workspace
// ---------------------------------------------------------------------------

/// Every row of the document tables, as `replica::load_rows` reads them (rich texts folded into their snapshot): the
/// baseline a replay starts from (§7.1, TODO I12).
pub fn dump_doc_rows(ws: &Workspace) -> WsResult<Value> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut out = Map::new();
    for table in ["entities", "tree_edges", "refs", "table_fields", "table_records", "richtext_states", "assets"] {
        let mut st = ws.doc.prepare(&format!("SELECT * FROM {table}")).map_err(db_err)?;
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let rows: Vec<Value> = st
            .query_map([], |r| {
                let mut m = Map::new();
                for (i, n) in names.iter().enumerate() {
                    let v = match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(x) => json!(x),
                        rusqlite::types::ValueRef::Real(x) => json!(x),
                        rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                        rusqlite::types::ValueRef::Blob(b) => json!(b64.encode(b)),
                    };
                    m.insert(n.clone(), v);
                }
                Ok(Value::Object(m))
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        out.insert(table.to_string(), Value::Array(rows));
    }
    // a rich text with pending updates: its snapshot as the replica would need it
    let now = ws.now();
    let ctx = ws.ctx(&now);
    if let Some(rows) = out.get_mut("richtext_states").and_then(Value::as_array_mut) {
        for r in rows {
            let id = r["entity_id"].as_str().unwrap_or("").to_string();
            if let Some(rt) = ctx.richtext(&id)? {
                r["snapshot"] = json!(b64.encode(richtext::export_snapshot(&rt.doc)?));
            }
        }
    }
    Ok(Value::Object(out))
}

impl Service {
    /// The run Workspace of a new session (§3.1, Q02): both databases copied with `VACUUM INTO` inside the source's
    /// writer, then — before anyone can open it — the identity rewritten, history cut back to the clone point,
    /// rich texts reduced to shallow snapshots, grants reset to the owner and the presenter, deployment state
    /// dropped and every live asset bound under the run Workspace's own key (an edit or deletion at the source never
    /// takes a resource away from a session). Returns `(id, { workspace_id, epoch, seq })` of the clone point.
    pub fn clone_for_session(&self, source_id: &str, session_id: &str, owner: &str, presenter: &str, path_id: &str) -> WsResult<(String, Value)> {
        let id = random_id("ws_");
        let tmp = self.tmp_dir();
        let built = (|| -> WsResult<Value> {
            std::fs::create_dir_all(&tmp).map_err(io)?;
            let point = {
                let handle = self.workspace(source_id)?;
                let ws = handle.lock().unwrap();
                if ws.purpose().is_some() {
                    return Err(WsError::invalid_op("a show clone or session workspace cannot be presented in a session"));
                }
                for name in ["doc.sqlite", "local.sqlite"] {
                    let conn = if name == "doc.sqlite" { &ws.doc } else { &ws.local };
                    conn.execute("VACUUM INTO ?1", [tmp.join(name).to_string_lossy().as_ref()]).map_err(db_err)?;
                }
                json!({ "workspace_id": source_id, "epoch": ws.epoch, "seq": ws.head_seq, "title": ws.title() })
            };
            let seq = point["seq"].as_u64().unwrap_or(0);
            let doc = Connection::open(tmp.join("doc.sqlite")).map_err(db_err)?;
            for (k, v) in [
                ("workspace_id", id.clone()),
                ("purpose", PURPOSE_SESSION.to_string()),
                ("session_source", source_id.to_string()),
                ("session_id", session_id.to_string()),
                ("session_path", path_id.to_string()),
                ("session_from", point.to_string()),
                ("epoch", random_id("ep_")),
                ("base_seq", seq.to_string()),
                ("peer_id", (rand::random::<u64>() >> 1).to_string()),
            ] {
                meta_set(&doc, k, &v)?;
            }
            for k in ["forked_from", "show_source", "show_id", "show_expires_at"] {
                doc.execute("DELETE FROM workspace_meta WHERE key = ?1", [k]).map_err(db_err)?;
            }
            // the session starts from the content, not from how it was written (§3.1)
            {
                let mut st = doc.prepare("SELECT entity_id, snapshot FROM richtext_states").map_err(db_err)?;
                let states: Vec<(String, Vec<u8>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
                drop(st);
                for (entity_id, snapshot) in states {
                    let mut st = doc.prepare("SELECT update_bytes FROM richtext_updates WHERE entity_id = ?1 ORDER BY seq, idx").map_err(db_err)?;
                    let updates: Vec<Vec<u8>> = st.query_map([&entity_id], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
                    drop(st);
                    let loaded = richtext::load_doc(&snapshot, updates.iter().map(Vec::as_slice))?;
                    let shallow = richtext::export_shallow(&loaded)?;
                    doc.execute("UPDATE richtext_states SET snapshot = ?2, snapshot_seq = ?3 WHERE entity_id = ?1", params![entity_id, shallow, seq]).map_err(db_err)?;
                }
            }
            doc.execute_batch(
                "DELETE FROM richtext_updates; DELETE FROM commit_ops; DELETE FROM commit_deltas; DELETE FROM commits; \
                 DELETE FROM entity_versions; DELETE FROM snapshots; DELETE FROM assets WHERE ended_seq IS NOT NULL;",
            )
            .map_err(db_err)?;
            doc.execute_batch(EVENTS_DDL).map_err(db_err)?;
            // the session's own bindings: the owner's buckets, the run Workspace's key prefix
            let bucket = bucket_id(owner, ASSETS)?;
            let assets: Vec<String> = {
                let mut st = doc.prepare("SELECT object_id FROM assets").map_err(db_err)?;
                let v = st.query_map([], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
                v
            };
            for object_id in assets {
                let key = format!("{id}/{object_id}");
                match self.objects.bind(&bucket, &key, &object_id) {
                    Ok(version) => {
                        doc.execute("UPDATE assets SET bucket_key = ?2, bucket_version = ?3 WHERE object_id = ?1", params![object_id, key, version as i64]).map_err(db_err)?;
                    }
                    Err(e) => {
                        log::warn!("session {session_id}: asset {object_id} could not be bound: {e}");
                        doc.execute("UPDATE assets SET bucket_key = NULL, bucket_version = NULL WHERE object_id = ?1", [&object_id]).map_err(db_err)?;
                    }
                }
            }
            drop(doc);
            let local = Connection::open(tmp.join("local.sqlite")).map_err(db_err)?;
            local
                .execute_batch(
                    "DELETE FROM grants; DELETE FROM share_entries; DELETE FROM publications; DELETE FROM referrers; DELETE FROM locks; \
                     DELETE FROM lock_events; DELETE FROM staged_assets; DELETE FROM user_state; DELETE FROM runs; DELETE FROM local_meta;",
                )
                .map_err(db_err)?;
            local.execute("INSERT INTO grants (subject, scope_entity_id, capabilities) VALUES (?1, '', ?2)", params![owner, CapSet::ALL.0]).map_err(db_err)?;
            if presenter != owner {
                local.execute("INSERT INTO grants (subject, scope_entity_id, capabilities) VALUES (?1, '', ?2)", params![presenter, PRESENTER_CAPS]).map_err(db_err)?;
            }
            local.execute("INSERT INTO local_meta (key, value) VALUES ('owner', ?1)", [owner]).map_err(db_err)?;
            Ok(point)
        })();
        let point = match built {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e);
            }
        };
        std::fs::rename(&tmp, self.ws_dir(&id)).map_err(io)?;
        Ok((id, point))
    }

    /// The registry row of the session whose run Workspace `ws_id` is, if it is one.
    pub fn session_of_run(&self, ws_id: &str) -> WsResult<Option<SessionRow>> {
        self.sessions.by_run_workspace(ws_id)
    }

    /// `session.create { workspace_id, path_id, title?, presenter?, policy? }` (§3.1, §3.2): the caller becomes the owner;
    /// it needs workspace-level `read` and `update` on the source (授课只要求创建本场及 Presenter 所需权限, not the Host).
    pub fn session_create(&self, caller: &Caller, p: &Value) -> WsResult<Value> {
        if caller.is_restricted() || caller.is_guest() {
            return Err(WsError::refused(Code::PermissionDenied, aiworkspace_core::Deny::SessionKindNotAllowed, "this kind of account cannot create a session"));
        }
        let source_id = s(p, "workspace_id")?.to_string();
        let path_id = s(p, "path_id")?.to_string();
        let policy = normalize_policy(p.get("policy"))?;
        let presenter = p.get("presenter").and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(&caller.principal).to_string();
        let title = {
            let handle = self.workspace(&source_id)?;
            let ws = handle.lock().unwrap();
            let access = ws.require_ws(caller, Cap::Read)?;
            if !access.ws_caps.has(Cap::Update) {
                return Err(WsError::denied("creating a session needs update on the workspace"));
            }
            let read = ws.read(caller, &path_id, None)?;
            if read["type_id"] != json!(TYPE_SHOW_PATH) {
                return Err(WsError::invalid_op(format!("{path_id} is not a presentation path")));
            }
            p.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string).or_else(|| read["content"]["payload"]["title"].as_str().map(str::to_string)).unwrap_or_else(|| ws.title())
        };
        if title.chars().count() > 256 {
            return Err(WsError::invalid_op("title too long"));
        }
        let session_id = random_id("se_");
        let (run_id, point) = self.clone_for_session(&source_id, &session_id, &caller.principal, &presenter, &path_id)?;
        let owner = Caller::user(&caller.principal);
        let baseline = {
            let handle = self.workspace(&run_id)?;
            let mut ws = handle.lock().unwrap();
            let notes = strip_notes(&mut ws, &owner)?;
            // the baseline a replay starts from and the version the archive pins (§7.1, §8)
            let rows = dump_doc_rows(&ws)?;
            std::fs::create_dir_all(ws.dir.join("session")).map_err(io)?;
            std::fs::write(ws.dir.join("session").join("baseline.json"), rows.to_string()).map_err(io)?;
            let cp = ws.materialize_for(&Access::full(&caller.principal), "session", &caller.principal)?;
            json!({ "source": point, "workspace_id": run_id, "epoch": ws.epoch, "seq": ws.head_seq, "snapshot_id": cp.snapshot_id,
                    "content_root": cp.content_root, "notes_entity_id": notes })
        };
        let now = (self.clock)();
        let row = SessionRow {
            session_id: session_id.clone(),
            source_workspace_id: source_id,
            workspace_id: run_id,
            path_id,
            title,
            owner: caller.principal.clone(),
            presenter,
            state: PREPARED.into(),
            access: "open".into(),
            posts: "open".into(),
            policy,
            baseline,
            archive: None,
            controller: None,
            created_ms: now,
            started_ms: None,
            ended_ms: None,
            saved_ms: None,
            expires_ms: None,
            deleted_ms: None,
        };
        self.sessions.insert(&row)?;
        log::info!("session {session_id} created on {} by {} (run workspace {})", row.source_workspace_id, caller.principal, row.workspace_id);
        Ok(json!({ "ok": true, "session": session_json(&row, Some(Role::Presenter)) }))
    }

    /// Who `caller` is in `s`: the presenter, the owner, a joined participant, or — through a link — a display or a
    /// prompter. `None`: nobody (a link is needed to join).
    pub fn session_actor(&self, s: &SessionRow, caller: &Caller, via: Option<&InviteRow>) -> WsResult<Option<Actor>> {
        let class = |c: &Caller| match c.class {
            crate::CallerClass::Account { .. } => "account",
            crate::CallerClass::External { .. } => "external",
            crate::CallerClass::Guest => "guest",
            crate::CallerClass::Service => "service",
        };
        if let Some(i) = via.filter(|i| i.session_id == s.session_id && i.revoked_ms.is_none()) {
            match i.kind.as_str() {
                "pt" => return Ok(Some(Actor { principal: s.presenter.clone(), role: Role::Prompter, class: "account".into(), level: "verified", name: s.presenter.clone() })),
                "dt" => return Ok(Some(Actor { principal: display_principal(&i.invite_id), role: Role::Display, class: "display".into(), level: "guest", name: i.label.clone().unwrap_or_else(|| "Display".into()) })),
                _ => {}
            }
        }
        if caller.principal == s.presenter {
            return Ok(Some(Actor { principal: caller.principal.clone(), role: Role::Presenter, class: class(caller).into(), level: "verified", name: caller.principal.clone() }));
        }
        if caller.principal == s.owner {
            return Ok(Some(Actor { principal: caller.principal.clone(), role: Role::Owner, class: class(caller).into(), level: "verified", name: caller.principal.clone() }));
        }
        if let Some(p) = self.sessions.participant(&s.session_id, &caller.principal)? {
            let level = if p.level == "guest" { "guest" } else { "verified" };
            return Ok(Some(Actor { principal: caller.principal.clone(), role: Role::Participant, class: p.class, level, name: p.name }));
        }
        Ok(None)
    }

    /// An actor or the refusal a stranger gets (it cannot tell a missing session from one it was not invited to).
    pub fn require_actor(&self, s: &SessionRow, caller: &Caller, via: Option<&InviteRow>) -> WsResult<Actor> {
        self.session_actor(s, caller, via)?.ok_or_else(|| WsError::sub(Code::NotFound, "SESSION_NOT_FOUND", "session not found, or you were not invited"))
    }

    /// `session.get`: what the caller's role may know of the session.
    pub fn session_get(&self, s: &SessionRow, actor: &Actor) -> WsResult<Value> {
        let mut v = session_json(s, Some(actor.role));
        v["ok"] = json!(true);
        v["role"] = json!(actor.role.as_str());
        v["principal"] = json!(actor.principal);
        v["level"] = json!(actor.level);
        v["owner_actions"] = json!(actor.is_owner(s));
        if s.alive() {
            let handle = self.workspace(&s.workspace_id)?;
            let ws = handle.lock().unwrap();
            v["stage"] = ws.stage();
            v["steps"] = json!(ws.session_steps()?);
            v["head_event"] = json!(ws.last_event_seq());
            v["t_rel_ms"] = json!(ws.t_rel_now());
            if actor.role.reviews() {
                v["notes_entity_id"] = s.baseline["notes_entity_id"].clone();
            }
        }
        if actor.role.reviews() {
            // who came in, and whether they are here (§5.4: the presenter's own view of the room)
            v["participants"] = json!(self.sessions.participants(&s.session_id)?.iter().map(|p| json!({
                "principal": p.principal, "name": p.name, "class": p.class, "level": p.level, "role": p.role, "online": p.online,
                "joined_at": format_utc_ms(p.joined_ms), "last_seen": format_utc_ms(p.last_seen_ms) })).collect::<Vec<_>>());
        }
        Ok(v)
    }

    /// `session.list { workspace_id }` (§8: 作品中的历次演示入口): the sessions of a source Workspace its readers may see.
    pub fn session_list(&self, caller: &Caller, source_id: &str) -> WsResult<Value> {
        let manage = {
            let handle = self.workspace(source_id)?;
            let ws = handle.lock().unwrap();
            ws.require_ws(caller, Cap::Read)?.ws_caps.has(Cap::Manage)
        };
        let mut out = Vec::new();
        for s in self.sessions.list_for_source(source_id)? {
            let mine = s.owner == caller.principal || s.presenter == caller.principal || manage;
            let joined = self.sessions.participant(&s.session_id, &caller.principal)?.is_some();
            if mine || joined {
                let role = if s.presenter == caller.principal { Role::Presenter } else if mine { Role::Owner } else { Role::Participant };
                out.push(session_json(&s, Some(role)));
            }
        }
        Ok(json!({ "ok": true, "sessions": out }))
    }

    /// Run `f` on the session's run Workspace.
    pub fn with_run<T>(&self, s: &SessionRow, f: impl FnOnce(&mut Workspace) -> WsResult<T>) -> WsResult<T> {
        if !s.alive() {
            return Err(WsError::sub(Code::NotFound, "SESSION_EXPIRED", "the session was not saved and has expired"));
        }
        let handle = self.workspace(&s.workspace_id)?;
        let mut ws = handle.lock().unwrap();
        f(&mut ws)
    }

    /// `session.start` (§3.2): recording starts with the demonstration — the clock's zero, every feed activated (sample
    /// rows replaced by the session's own), the first step on stage.
    pub fn session_start(&self, s: &mut SessionRow, actor: &Actor) -> WsResult<Value> {
        if !actor.role.presents() && !actor.is_owner(s) {
            return Err(WsError::denied("only the presenter starts the session"));
        }
        if s.state == LIVE {
            return Ok(json!({ "ok": true, "state": s.state, "already": true }));
        }
        if s.state != PREPARED {
            return Err(WsError::sub(Code::InvalidOperation, "SESSION_STATE", format!("a {} session cannot start again; create another session", s.state)));
        }
        let now = (self.clock)();
        let participants = self.sessions.participants(&s.session_id)?;
        let stage = self.with_run(s, |ws| {
            meta_set(&ws.doc, "session_t0", &now.to_string())?;
            let steps = ws.session_steps()?;
            let first = steps.first().cloned();
            let stage = json!({ "step_id": first.as_ref().map(|f| f["id"].clone()), "index": 0, "count": steps.len(),
                                "title": first.as_ref().map(|f| f["title"].clone()), "surface_id": first.as_ref().map(|f| f["surface_id"].clone()),
                                "black": false, "free": false, "transition": "fade", "nonce": 1, "media": {}, "rect": Value::Null, "updated_ms": now });
            ws.set_stage(&stage, NewEvent { actor: &actor.principal, scope: "public", track: "public", kind: "session.start", command_id: None, data: json!({ "stage": stage }), t_rel_ms: Some(0) })?;
            let owner = Caller::user(&s.owner);
            ws.refresh_feeds(&owner, &actor.principal, &format!("start/{}", s.session_id), &FeedSel::all(), &participants)?;
            Ok(stage)
        })?;
        s.state = LIVE.into();
        s.started_ms = Some(now);
        self.sessions.update(s)?;
        Ok(json!({ "ok": true, "state": s.state, "stage": stage }))
    }

    /// `session.end` (§3.2): live tasks stop; the room keeps its data for the retention period unless it is saved.
    pub fn session_end(&self, s: &mut SessionRow, actor: &Actor, retention_ms: i64) -> WsResult<Value> {
        if !actor.role.presents() && !actor.is_owner(s) {
            return Err(WsError::denied("only the presenter or the owner ends the session"));
        }
        if matches!(s.state.as_str(), ENDED | ARCHIVED | EXPIRED) {
            return Ok(json!({ "ok": true, "state": s.state, "already": true }));
        }
        let now = (self.clock)();
        self.with_run(s, |ws| {
            let owner = Caller::user(&s.owner);
            // open tasks close with the session: nothing more is accepted
            let open: Vec<String> = ws.session_entities(TYPE_TASK)?.into_iter().filter(|t| t.payload.get("state") == Some(&json!("open"))).map(|t| t.entity_id).collect();
            if !open.is_empty() {
                let mut ops = Vec::new();
                let mut overrides = BTreeMap::new();
                for id in &open {
                    let e = ws.entity_row(id)?;
                    let mut p = e.payload.clone();
                    p.insert("state".into(), json!("closed"));
                    p.insert("closed_at".into(), json!(format_utc_ms(now)));
                    ops.push(json!({ "op": "entity.set_keys", "entity_id": id, "keys": [
                        { "key": "state", "value": "closed", "expect": { "rev": e.key_rev("state") } },
                        { "key": "closed_at", "value": format_utc_ms(now), "expect": { "rev": e.key_rev("closed_at") } } ] }));
                    overrides.insert(id.clone(), Override::payload(&e, p));
                }
                ws.session_commit(&owner, &actor.principal, &format!("end/{}", s.session_id), ops, &overrides, &FeedSel::tasks(), &[])?;
            }
            ws.append_event(NewEvent { actor: &actor.principal, scope: "public", track: "public", kind: "session.end", command_id: None, data: json!({}), t_rel_ms: None })?;
            meta_set(&ws.doc, "session_closed", &now.to_string())?;
            Ok(())
        })?;
        s.state = ENDED.into();
        s.ended_ms = Some(now);
        s.expires_ms = Some(now + retention_ms);
        s.controller = None;
        self.sessions.update(s)?;
        Ok(json!({ "ok": true, "state": s.state, "expires_at": format_utc_ms(now + retention_ms) }))
    }

    /// `session.save` (§8, Q12): the owner (or a manager of the source) keeps the session: an archive manifest naming
    /// the baseline, the final snapshot, the event range, the voice segments and what is missing, and a `session`
    /// retention set pinning every object they need — no 30-day history rule applies to it.
    pub fn session_save(&self, s: &mut SessionRow, actor: &Actor) -> WsResult<Value> {
        if !actor.is_owner(s) {
            return Err(WsError::denied("saving the session is the owner's"));
        }
        if s.state == ARCHIVED {
            return Ok(json!({ "ok": true, "state": s.state, "archive": s.archive, "already": true }));
        }
        if s.state != ENDED {
            return Err(WsError::sub(Code::InvalidOperation, "SESSION_STATE", "end the session before saving it"));
        }
        let now = (self.clock)();
        let archive = self.with_run(s, |ws| {
            let cp = ws.materialize_for(&Access::full(&s.owner), "session", &s.owner)?;
            let mut members = BTreeMap::new();
            for root in [s.baseline["content_root"].as_str().unwrap_or(""), cp.content_root.as_str()] {
                if !root.is_empty() {
                    self.objects.content_members(root, &mut members)?;
                }
            }
            let mut st = ws.doc.prepare("SELECT object_id FROM assets").map_err(db_err)?;
            let assets: Vec<String> = st.query_map([], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
            drop(st);
            let mut missing = Vec::new();
            for a in &assets {
                if !self.objects.add_member(a, &mut members)? {
                    missing.push(json!({ "object_id": a }));
                }
            }
            let voice: Vec<Value> = {
                let mut st = ws.doc.prepare("SELECT payload_json FROM session_events WHERE type = 'voice.segment' ORDER BY seq").map_err(db_err)?;
                let v = st.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)?.filter_map(|r| r.ok()).filter_map(|t| serde_json::from_str::<Value>(&t).ok()).collect();
                v
            };
            let gaps: Vec<Value> = {
                let mut st = ws.doc.prepare("SELECT payload_json, t_rel_ms FROM session_events WHERE type IN ('record.gap', 'voice.gap') ORDER BY seq").map_err(db_err)?;
                let v = st
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                    .map_err(db_err)?
                    .filter_map(|r| r.ok())
                    .map(|(t, at)| json!({ "t_rel_ms": at, "data": serde_json::from_str::<Value>(&t).unwrap_or(Value::Null) }))
                    .collect();
                v
            };
            let key = format!("{}/session/{}", ws.workspace_id, s.session_id);
            let (root, version) = self.objects.retain(&ws.bucket(crate::objects::OBJECTS)?, &key, &members)?;
            let complete = missing.is_empty() && gaps.is_empty();
            Ok(json!({ "format": "buckyos.ai-workspace/session-archive/1", "session_id": s.session_id, "baseline": s.baseline,
                       "final": { "snapshot_id": cp.snapshot_id, "content_root": cp.content_root, "seq": cp.seq },
                       "events": { "first": 1, "last": ws.last_event_seq() }, "voice": voice, "gaps": gaps, "missing": missing,
                       "complete": complete, "retention": { "kind": "session", "key": key, "root": root, "version": version },
                       "duration_ms": ws.t_rel_now(), "saved_at": format_utc_ms(now), "access_policy": { "access": s.access, "posts": s.posts } }))
        })?;
        s.state = ARCHIVED.into();
        s.saved_ms = Some(now);
        s.expires_ms = None;
        s.archive = Some(archive.clone());
        self.sessions.update(s)?;
        Ok(json!({ "ok": true, "state": s.state, "archive": archive }))
    }

    /// An ended session nobody saved, past its retention (§3.2): the run Workspace, its events and media go; the row
    /// stays to say "未保存，已过期".
    pub fn session_expire(&self, s: &mut SessionRow) -> WsResult<()> {
        self.drop_run(s)?;
        s.state = EXPIRED.into();
        s.controller = None;
        self.sessions.update(s)
    }

    /// `session.delete` (§8): the owner removes the room; only what this session holds is released.
    pub fn session_delete(&self, s: &mut SessionRow, actor: &Actor) -> WsResult<Value> {
        if !actor.is_owner(s) {
            return Err(WsError::denied("deleting the session is the owner's"));
        }
        self.drop_run(s)?;
        s.deleted_ms = Some((self.clock)());
        s.controller = None;
        self.sessions.update(s)?;
        Ok(json!({ "ok": true }))
    }

    fn drop_run(&self, s: &SessionRow) -> WsResult<()> {
        let dir = self.ws_dir(&s.workspace_id);
        if !dir.exists() {
            return Ok(());
        }
        if let Some(key) = s.archive.as_ref().and_then(|a| a["retention"]["key"].as_str()) {
            if let Ok(bucket) = bucket_id(&s.owner, crate::objects::OBJECTS) {
                let _ = self.objects.unbind(&bucket, key);
            }
        }
        if let Err(e) = self.release_folder(&dir) {
            log::warn!("release bindings of session {}: {e}", s.session_id);
        }
        self.close(&s.workspace_id);
        std::fs::remove_dir_all(&dir).map_err(io)?;
        self.close(&s.workspace_id);
        Ok(())
    }

    /// `session.set_access { access?, posts? }` (§3.2, §8): closing the room to new audience and closing comments are
    /// separate switches.
    pub fn session_set_access(&self, s: &mut SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        if !actor.is_owner(s) && !actor.role.presents() {
            return Err(WsError::denied("the owner or the presenter sets the room's access"));
        }
        if let Some(a) = p.get("access").and_then(Value::as_str) {
            if !matches!(a, "open" | "closed") {
                return Err(WsError::invalid_op("access must be open or closed"));
            }
            s.access = a.into();
        }
        if let Some(a) = p.get("posts").and_then(Value::as_str) {
            if !matches!(a, "open" | "closed") {
                return Err(WsError::invalid_op("posts must be open or closed"));
            }
            s.posts = a.into();
        }
        if let Some(v) = p.get("guest_interact") {
            if !actor.is_owner(s) {
                return Err(WsError::denied("the owner sets whether guests interact"));
            }
            s.policy["guest_interact"] = json!(v.as_bool().ok_or_else(|| WsError::invalid_op("guest_interact must be a boolean"))?);
        }
        self.sessions.update(s)?;
        Ok(json!({ "ok": true, "access": s.access, "posts": s.posts, "policy": s.policy }))
    }

    // ---- invites (Q14) ----

    /// `session.invite { kind: at | dt | pt, label?, config? }`: a link whose rights the session defines, independent
    /// of the Workspace's sharing. `at_`: the audience (joins with the visitor's own identity); `dt_`: a public display
    /// (`config.voice`: whether it plays the voice, default false); `pt_`: a prompter.
    pub fn session_invite(&self, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        if !actor.is_owner(s) && actor.role != Role::Presenter {
            return Err(WsError::denied("the owner or the presenter creates links"));
        }
        let kind = s_kind(p.get("kind").and_then(Value::as_str).unwrap_or("at"))?;
        let config = match p.get("config") {
            None | Some(Value::Null) => json!({}),
            Some(Value::Object(o)) if Value::Object(o.clone()).to_string().len() <= 4096 => Value::Object(o.clone()),
            Some(_) => return Err(WsError::invalid_op("config must be a small object")),
        };
        let label = p.get("label").and_then(Value::as_str).map(|l| l.chars().take(128).collect::<String>());
        let row = InviteRow {
            invite_id: random_id("iv_"),
            session_id: s.session_id.clone(),
            kind: kind.into(),
            token: random_id(&format!("{kind}_")),
            label,
            config,
            created_by: actor.principal.clone(),
            created_ms: (self.clock)(),
            revoked_ms: None,
        };
        self.sessions.add_invite(&row)?;
        if kind == "dt" || kind == "pt" {
            // a display or a prompter reads the public projection of the run Workspace under the link's own subject
            self.with_run(s, |ws| {
                ws.local
                    .execute("INSERT OR REPLACE INTO grants (subject, scope_entity_id, capabilities, via_entry) VALUES (?1, '', ?2, ?3)", params![display_principal(&row.invite_id), Cap::Read as u16, row.invite_id])
                    .map_err(db_err)?;
                Ok(())
            })?;
        }
        Ok(json!({ "ok": true, "invite": row.to_json() }))
    }

    pub fn session_invites(&self, s: &SessionRow, actor: &Actor) -> WsResult<Value> {
        if !actor.is_owner(s) && actor.role != Role::Presenter {
            return Err(WsError::denied("the owner or the presenter lists links"));
        }
        let list: Vec<Value> = self.sessions.invites(&s.session_id)?.iter().map(InviteRow::to_json).collect();
        Ok(json!({ "ok": true, "invites": list }))
    }

    /// `session.revoke_invite { invite_id }`: the link dies and so does every grant it gave (§7.2: later reads follow
    /// the current access).
    pub fn session_revoke_invite(&self, s: &SessionRow, actor: &Actor, invite_id: &str) -> WsResult<Value> {
        if !actor.is_owner(s) && actor.role != Role::Presenter {
            return Err(WsError::denied("the owner or the presenter revokes links"));
        }
        let revoked = self.sessions.revoke_invite(&s.session_id, invite_id, (self.clock)())?;
        if revoked && s.alive() {
            self.with_run(s, |ws| {
                ws.local.execute("DELETE FROM grants WHERE via_entry = ?1", [invite_id]).map_err(db_err)?;
                Ok(())
            })?;
        }
        Ok(json!({ "ok": true, "revoked": revoked }))
    }

    /// `session.join { session_id, invite?, name? }` (§4, Q03, Q14): an audience link lets the caller in with its own
    /// identity — a Zone account, an External DID, or (always allowed to watch) a guest; a joined participant comes
    /// back without the link. Writes the read grant (`via_entry` = the link), the participant row and the
    /// participant's own `my_responses` tables, and records the arrival.
    pub fn session_join(&self, s: &SessionRow, caller: &Caller, invite: Option<&InviteRow>, name: Option<&str>) -> WsResult<(Actor, Value, bool)> {
        if !s.alive() {
            return Err(WsError::sub(Code::NotFound, "SESSION_EXPIRED", "the session was not saved and has expired"));
        }
        let now = (self.clock)();
        if let Some(actor) = self.session_actor(s, caller, invite)? {
            if matches!(actor.role, Role::Display | Role::Prompter) {
                return Ok((actor, json!({}), false));
            }
            let name = name.map(clean_name).filter(|n| !n.is_empty()).unwrap_or(actor.name.clone());
            let row = ParticipantRow { principal: actor.principal.clone(), class: actor.class.clone(), level: actor.level.into(), name: name.clone(),
                                       role: actor.role.as_str().into(), via_invite: None, joined_ms: now, last_seen_ms: now, online: true };
            let first = self.sessions.upsert_participant(&s.session_id, &row)?;
            let my = self.with_run(s, |ws| ws.ensure_my_feeds(&Caller::user(&s.owner), &actor.principal))?;
            return Ok((Actor { name, ..actor }, my, first));
        }
        let Some(inv) = invite.filter(|i| i.kind == "at" && i.session_id == s.session_id && i.revoked_ms.is_none()) else {
            return Err(WsError::sub(Code::NotFound, "SESSION_NOT_FOUND", "session not found, or you were not invited"));
        };
        if s.access != "open" {
            return Err(WsError::sub(Code::PermissionDenied, "SESSION_CLOSED", "the room is closed to new audience"));
        }
        if matches!(caller.class, crate::CallerClass::Service) && !caller.is_agent() {
            return Err(WsError::refused(Code::PermissionDenied, aiworkspace_core::Deny::SessionKindNotAllowed, "a service cannot join a session"));
        }
        let (class, level) = match &caller.class {
            crate::CallerClass::Account { .. } => ("account", "verified"),
            crate::CallerClass::External { .. } => ("external", "verified"),
            crate::CallerClass::Guest => ("guest", "guest"),
            crate::CallerClass::Service => ("service", "verified"),
        };
        let name = name.map(clean_name).filter(|n| !n.is_empty()).unwrap_or_else(|| caller.principal.clone());
        self.with_run(s, |ws| {
            ws.local
                .execute("INSERT OR IGNORE INTO grants (subject, scope_entity_id, capabilities, via_entry) VALUES (?1, '', ?2, ?3)", params![caller.principal, Cap::Read as u16, inv.invite_id])
                .map_err(db_err)?;
            Ok(())
        })?;
        let row = ParticipantRow { principal: caller.principal.clone(), class: class.into(), level: level.into(), name: name.clone(), role: "audience".into(),
                                   via_invite: Some(inv.invite_id.clone()), joined_ms: now, last_seen_ms: now, online: true };
        let first = self.sessions.upsert_participant(&s.session_id, &row)?;
        let my = self.with_run(s, |ws| ws.ensure_my_feeds(&Caller::user(&s.owner), &caller.principal))?;
        let actor = Actor { principal: caller.principal.clone(), role: Role::Participant, class: class.into(), level, name };
        Ok((actor, my, first))
    }
}

/// An order key under the `session` folder: lower-case letters and digits, never ending in `0`.
fn okey(prefix: &str, id: &str) -> String {
    format!("{prefix}{}1", id.to_ascii_lowercase().replace(['_', '-'], ""))
}

fn s_kind(k: &str) -> WsResult<&'static str> {
    match k {
        "at" => Ok("at"),
        "dt" => Ok("dt"),
        "pt" => Ok("pt"),
        _ => Err(WsError::invalid_op("kind must be at, dt or pt")),
    }
}

/// The grant subject and principal of a display or prompter link.
pub fn display_principal(invite_id: &str) -> String {
    format!("link:{invite_id}")
}

fn clean_name(n: &str) -> String {
    n.trim().chars().filter(|c| !c.is_control()).take(64).collect()
}

/// The session as a role sees it: the presenter and the owner see the policy and baseline, others the public part.
pub fn session_json(s: &SessionRow, role: Option<Role>) -> Value {
    let mut v = json!({ "session_id": s.session_id, "title": s.title, "state": s.state, "access": s.access, "posts": s.posts,
                        "workspace_id": s.workspace_id, "source_workspace_id": s.source_workspace_id, "path_id": s.path_id,
                        "owner": s.owner, "presenter": s.presenter, "guest_interact": s.guest_interact(), "voice": s.policy["voice"],
                        "show_names": s.policy["show_names"],
                        "created_at": format_utc_ms(s.created_ms), "started_at": s.started_ms.map(format_utc_ms), "ended_at": s.ended_ms.map(format_utc_ms),
                        "saved_at": s.saved_ms.map(format_utc_ms), "expires_at": s.expires_ms.map(format_utc_ms), "controller": s.controller });
    if role.is_some_and(|r| r.reviews()) {
        v["policy"] = s.policy.clone();
        v["baseline"] = s.baseline.clone();
    }
    if let Some(a) = &s.archive {
        v["archive"] = json!({ "complete": a["complete"], "duration_ms": a["duration_ms"], "missing": a["missing"], "gaps": a["gaps"], "voice": a["voice"],
                               "saved_at": a["saved_at"], "events": a["events"] });
    }
    v
}

/// Speaker notes leave the shared entities (§3.1): Frame `presentation.notes` and Viewport `notes` move into a table
/// in the presenter scope (`session/notes`, one record per target), so no public read, change stream, snapshot or
/// export of the run Workspace carries them. Returns the table's id, `None` when there were no notes.
fn strip_notes(ws: &mut Workspace, owner: &Caller) -> WsResult<Option<String>> {
    let mut ops = Vec::new();
    let mut records = Vec::new();
    {
        let mut st = ws.doc.prepare("SELECT entity_id, type_id, payload_json, key_revs_json FROM entities WHERE deleted_seq IS NULL AND type_id IN (?1, ?2)").map_err(db_err)?;
        let rows: Vec<(String, String, String, String)> =
            st.query_map(params![TYPE_CELL, TYPE_VIEWPORT], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        for (id, ty, payload, revs) in rows {
            let payload: Map<String, Value> = serde_json::from_str(&payload).unwrap_or_default();
            let revs: BTreeMap<String, u64> = serde_json::from_str(&revs).unwrap_or_default();
            if ty == TYPE_VIEWPORT {
                if let Some(n) = payload.get("notes").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                    records.push(json!({ "record_id": id, "values": { "target_id": id, "kind": "viewport", "notes": n } }));
                    ops.push(json!({ "op": "entity.unset_keys", "entity_id": id, "keys": [{ "key": "notes", "expect": { "rev": revs.get("notes").copied().unwrap_or(0) } }] }));
                }
            } else if let Some(pres) = payload.get("presentation").and_then(Value::as_object) {
                if let Some(n) = pres.get("notes").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                    records.push(json!({ "record_id": id, "values": { "target_id": id, "kind": "frame", "notes": n } }));
                    let mut rest = pres.clone();
                    rest.remove("notes");
                    let value = if rest.is_empty() { Value::Null } else { Value::Object(rest) };
                    ops.push(json!({ "op": "entity.set_keys", "entity_id": id, "keys": [{ "key": "presentation", "value": value, "expect": { "rev": revs.get("presentation").copied().unwrap_or(0) } }] }));
                }
            }
        }
    }
    if records.is_empty() {
        return Ok(None);
    }
    let table = random_id("sn_");
    ops.push(json!({ "op": "system.ensure", "id": SESSION_ID }));
    ops.push(json!({ "op": "entity.create", "entity_id": table, "type_id": TYPE_TABLE, "parent_id": SESSION_ID, "order_key": "a", "scope": SCOPE_PRESENTER,
                     "name": "notes", "payload": { "description": "Speaker notes", "fields": [
                         { "field_id": "target_id", "name": "Target", "type": "text" },
                         { "field_id": "kind", "name": "Kind", "type": "text" },
                         { "field_id": "notes", "name": "Notes", "type": "text" } ] } }));
    for chunk in records.chunks(500) {
        ops.push(json!({ "op": "table.insert_records", "source_id": table, "records": chunk }));
    }
    let req = ws.commit_request("session/strip-notes", "speaker notes move to the presenter's scope", ops);
    let result = ws.commit(&req, owner, &CommitOpts { internal: true, ..CommitOpts::default() });
    if result["status"] != json!("accepted") {
        return Err(WsError::io(format!("moving the speaker notes failed: {result}")));
    }
    Ok(Some(table))
}

// ---------------------------------------------------------------------------
// session data: tasks, responses, results, feeds (§5.2, §5.3)
// ---------------------------------------------------------------------------

/// Which feeds a Commit refreshes.
#[derive(Debug, Clone, Default)]
pub struct FeedSel {
    pub all: bool,
    pub names: BTreeSet<&'static str>,
    /// `my_responses` of these participants only (with `all` or the name: everyone's).
    pub mine: BTreeSet<String>,
}

impl FeedSel {
    pub fn all() -> FeedSel {
        FeedSel { all: true, ..FeedSel::default() }
    }
    pub fn of(names: &[&'static str]) -> FeedSel {
        FeedSel { names: names.iter().copied().collect(), ..FeedSel::default() }
    }
    pub fn tasks() -> FeedSel {
        FeedSel::of(&["tasks", "my_responses", "responses"])
    }
    pub fn with_mine(mut self, p: &str) -> FeedSel {
        self.mine.insert(p.to_string());
        self
    }
    fn wants(&self, feed: &str, participant: Option<&str>) -> bool {
        if self.all {
            return true;
        }
        match participant {
            Some(p) => self.mine.contains(p) || (self.names.contains(feed) && self.mine.is_empty()),
            None => self.names.contains(feed),
        }
    }
}

/// A session entity as this Commit will leave it (feeds are computed from the state after the Commit).
#[derive(Debug, Clone)]
pub struct Override {
    pub type_id: String,
    pub scope: String,
    pub payload: Map<String, Value>,
}

impl Override {
    fn payload(e: &EntityRow, payload: Map<String, Value>) -> Override {
        Override { type_id: e.type_id.clone(), scope: e.scope.clone(), payload }
    }
}

fn text_of(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => json!(""),
        Some(Value::String(s)) => json!(s),
        Some(other) => json!(other.to_string()),
    }
}

impl Workspace {
    pub(crate) fn entity_row(&self, id: &str) -> WsResult<EntityRow> {
        let now = self.now();
        self.ctx(&now).entity(id)?.filter(|e| e.alive()).ok_or_else(|| WsError::not_found(format!("{id} not found")))
    }

    /// Alive entities of `type_id` in the `session` folder.
    pub fn session_entities(&self, type_id: &str) -> WsResult<Vec<EntityRow>> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for edge in ctx.children(SESSION_ID)? {
            if let Some(e) = ctx.entity(&edge.child_id)? {
                if e.alive() && e.type_id == type_id {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    /// Every alive feed entity: `(entity_id, feed, participant)`.
    pub fn feed_entities(&self) -> WsResult<Vec<(String, String, Option<String>)>> {
        let mut st = self
            .doc
            .prepare_cached("SELECT entity_id, scope, payload_json FROM entities WHERE deleted_seq IS NULL AND type_id = ?1 AND payload_json LIKE '%\"session\"%'")
            .map_err(db_err)?;
        let rows: Vec<(String, String, String)> = st.query_map([TYPE_TABLE], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut out = Vec::new();
        for (id, scope, payload) in rows {
            let payload: Map<String, Value> = serde_json::from_str(&payload).unwrap_or_default();
            let probe = EntityRow { payload, ..system_container(&id, "folder", None, None) };
            if let Some(feed) = session_feed(&probe) {
                out.push((id, feed.to_string(), scope.strip_prefix(PARTICIPANT_PREFIX).map(str::to_string)));
            }
        }
        Ok(out)
    }

    /// A commit request of this Workspace.
    pub(crate) fn commit_request(&self, idem: &str, message: &str, ops: Vec<Value>) -> Value {
        json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch, "idempotency_key": idem,
                "message": message, "operations": ops })
    }

    /// `ops` plus the feeds `sel` names, recomputed from the state the Commit leaves (`overrides`), as one Commit of origin
    /// `session` written as `actor` under the owner's authority (the session method has decided who may do it).
    #[allow(clippy::too_many_arguments)]
    pub fn session_commit(&mut self, owner: &Caller, actor: &str, idem: &str, mut ops: Vec<Value>, overrides: &BTreeMap<String, Override>, sel: &FeedSel, participants: &[ParticipantRow]) -> WsResult<Value> {
        ops.extend(self.feed_ops(owner, sel, overrides, participants)?);
        if ops.is_empty() {
            return Ok(json!({ "status": "unchanged" }));
        }
        let idem = if idem.len() > 128 { format!("h/{}", &sha256_hex(idem.as_bytes())[..40]) } else { idem.to_string() };
        let req = self.commit_request(&idem, "session", ops);
        let result = self.commit(&req, owner, &CommitOpts { as_session: true, as_principal: Some(actor.to_string()), ..CommitOpts::default() });
        match result["status"].as_str() {
            Some("accepted") => Ok(result),
            _ => Err(commit_error(&result)),
        }
    }

    /// Refresh the feeds `sel` names on their own (presence, reactions, start).
    pub fn refresh_feeds(&mut self, owner: &Caller, actor: &str, idem: &str, sel: &FeedSel, participants: &[ParticipantRow]) -> WsResult<Value> {
        let idem = format!("{idem}/{}", self.head_seq);
        self.session_commit(owner, actor, &idem, Vec::new(), &BTreeMap::new(), sel, participants)
    }

    /// The operations that bring the selected feeds to the state after the Commit.
    fn feed_ops(&mut self, owner: &Caller, sel: &FeedSel, overrides: &BTreeMap<String, Override>, participants: &[ParticipantRow]) -> WsResult<Vec<Value>> {
        let feeds = self.feed_entities()?;
        let wanted: Vec<(String, String, Option<String>)> = feeds.into_iter().filter(|(_, f, p)| sel.wants(f, p.as_deref())).collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        // the session entities after the Commit
        let mut entities: BTreeMap<String, Override> = BTreeMap::new();
        for ty in [TYPE_TASK, TYPE_RESPONSE, TYPE_RESULT] {
            for e in self.session_entities(ty)? {
                entities.insert(e.entity_id.clone(), Override { type_id: e.type_id, scope: e.scope, payload: e.payload });
            }
        }
        for (id, o) in overrides {
            entities.insert(id.clone(), o.clone());
        }
        fn of<'a>(entities: &'a BTreeMap<String, Override>, t: &'a str) -> impl Iterator<Item = (&'a String, &'a Override)> {
            entities.iter().filter(move |(_, o)| o.type_id == t)
        }
        let mut ops = Vec::new();
        for (entity_id, feed, participant) in wanted {
            let public = feed == "participants" && self.entity_scope(&entity_id)?.as_deref() == Some(SCOPE_SHARED);
            let records: Vec<Value> = match feed.as_str() {
                "participants" => participants
                    .iter()
                    .filter(|p| p.role != "display")
                    .map(|p| {
                        let mut values = json!({ "name": p.name, "principal": p.principal, "role": p.role, "level": p.level, "online": p.online,
                                                 "last_seen": format_utc_ms(p.last_seen_ms), "joined_at": format_utc_ms(p.joined_ms) });
                        if public {
                            // the public projection: display name and presence only
                            for k in ["principal", "level", "last_seen"] {
                                values.as_object_mut().unwrap().remove(k);
                            }
                        }
                        json!({ "record_id": feeds::principal_key(&p.principal), "values": values })
                    })
                    .collect(),
                "stage" => {
                    let st = self.stage();
                    if st.get("step_id").is_none() {
                        Vec::new()
                    } else {
                        vec![json!({ "record_id": "stage", "values": {
                            "step_id": text_of(st.get("step_id")), "step_index": st["index"].as_u64().unwrap_or(0) + 1, "step_count": st["count"].as_u64().unwrap_or(0),
                            "step_title": text_of(st.get("title")), "black": st["black"] == json!(true), "free": st["free"] == json!(true),
                            "transition": text_of(st.get("transition")), "media": st.get("media").map(Value::to_string).unwrap_or_default(),
                            "followers": st["followers"].as_u64().unwrap_or(0), "updated_at": st["updated_ms"].as_i64().map(format_utc_ms).unwrap_or_default() } })]
                    }
                }
                "tasks" => of(&entities, TYPE_TASK)
                    .map(|(id, t)| {
                        let answers: Vec<&Override> = of(&entities, TYPE_RESPONSE).map(|(_, r)| r).filter(|r| r.payload.get("task_id").and_then(Value::as_str) == Some(id.as_str())).collect();
                        let answered = answers.iter().filter(|r| r.payload.get("state") == Some(&json!("submitted"))).count();
                        let claimed = answers.iter().filter(|r| matches!(r.payload.get("state").and_then(Value::as_str), Some("claimed" | "submitted"))).count();
                        let picked: Vec<String> = if t.payload.get("show_names") == Some(&json!(false)) {
                            Vec::new()
                        } else {
                            answers.iter().filter(|r| r.payload.get("state") != Some(&json!("invalid"))).filter(|_| t.payload.get("mode") != Some(&json!("open")))
                                .filter_map(|r| r.payload.get("name").and_then(Value::as_str).map(str::to_string)).collect()
                        };
                        json!({ "record_id": id, "values": { "task_id": id, "title": text_of(t.payload.get("title")), "mode": text_of(t.payload.get("mode")),
                                "state": text_of(t.payload.get("state")), "slots": t.payload.get("slots").and_then(Value::as_u64).unwrap_or(0),
                                "claimed": claimed, "answered": answered, "picked": picked.join(", "), "block_id": text_of(t.payload.get("block_id")),
                                "created_at": text_of(t.payload.get("created_at")) } })
                    })
                    .collect(),
                "my_responses" => match participant.as_deref() {
                    // the courseware's shared sample stays empty in a session: each participant reads its own table
                    None => Vec::new(),
                    Some(p) => of(&entities, TYPE_RESPONSE)
                        .filter(|(_, r)| r.payload.get("participant").and_then(Value::as_str) == Some(p))
                        .map(|(_, r)| {
                            let review = r.payload.get("review").filter(|v| v["published"] == json!(true));
                            json!({ "record_id": r.payload.get("task_id").and_then(Value::as_str).unwrap_or(""), "values": {
                                "task_id": text_of(r.payload.get("task_id")), "state": text_of(r.payload.get("state")),
                                "content": r.payload.get("content").map(Value::to_string).unwrap_or_default(), "version": r.payload.get("version").and_then(Value::as_u64).unwrap_or(0),
                                "submitted_at": text_of(r.payload.get("submitted_at")), "verdict": text_of(review.and_then(|v| v.get("verdict"))),
                                "score": review.and_then(|v| v.get("score")).and_then(Value::as_f64).map(Value::from).unwrap_or(Value::Null),
                                "comment": text_of(review.and_then(|v| v.get("comment"))) } })
                        })
                        .map(|mut v| {
                            if v["values"]["score"].is_null() {
                                v["values"].as_object_mut().unwrap().remove("score");
                            }
                            v
                        })
                        .collect(),
                },
                "responses" => of(&entities, TYPE_RESPONSE)
                    .map(|(id, r)| json!({ "record_id": id, "values": {
                        "response_id": id, "task_id": text_of(r.payload.get("task_id")), "participant": text_of(r.payload.get("participant")),
                        "name": text_of(r.payload.get("name")), "level": text_of(r.payload.get("level")), "state": text_of(r.payload.get("state")),
                        "content": r.payload.get("content").map(Value::to_string).unwrap_or_default(), "submitted_at": text_of(r.payload.get("submitted_at")),
                        "late": r.payload.get("late") == Some(&json!(true)) } }))
                    .collect(),
                "results" => of(&entities, TYPE_RESULT)
                    .map(|(id, r)| json!({ "record_id": id, "values": {
                        "task_id": text_of(r.payload.get("task_id")), "title": text_of(r.payload.get("title")),
                        "content": r.payload.get("content").map(Value::to_string).unwrap_or_default(), "published_at": text_of(r.payload.get("published_at")) } }))
                    .collect(),
                "reactions" => self.reaction_rows()?,
                _ => Vec::new(),
            };
            let fetched = Fetched::Table { fields: feeds::fields(&feed), records };
            ops.extend(self.source_ops(owner, &entity_id, &fetched)?);
        }
        Ok(ops)
    }

    /// The recent reactions as feed rows (§5.2: 近期聚合): the last 200, grouped by kind and text.
    fn reaction_rows(&self) -> WsResult<Vec<Value>> {
        let mut st = self.doc.prepare_cached("SELECT payload_json, received_ms FROM session_events WHERE type = 'reaction' ORDER BY seq DESC LIMIT 200").map_err(db_err)?;
        let rows: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut groups: BTreeMap<String, (Value, u64, i64)> = BTreeMap::new();
        for (payload, at) in rows {
            let v: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            let key = format!("{}\n{}", v["kind"].as_str().unwrap_or(""), v["text"].as_str().unwrap_or(""));
            let entry = groups.entry(key).or_insert((v.clone(), 0, at));
            entry.1 += 1;
            entry.2 = entry.2.max(at);
        }
        Ok(groups
            .into_iter()
            .map(|(key, (v, count, at))| {
                let mut values = json!({ "kind": text_of(v.get("kind")), "text": text_of(v.get("text")), "count": count, "at": format_utc_ms(at),
                                         "target_id": text_of(v.get("target_id")) });
                if let Some(x) = v.get("x").and_then(Value::as_f64) {
                    values["x"] = json!(x);
                }
                if let Some(y) = v.get("y").and_then(Value::as_f64) {
                    values["y"] = json!(y);
                }
                json!({ "record_id": format!("r{}", &sha256_hex(key.as_bytes())[..30]), "values": values })
            })
            .collect())
    }

    /// A participant's own `my_responses` tables, one per sample the courseware placed (I6): created on joining, empty
    /// until they answer. Returns `{ <sample id>: <own id> }` for the client's binding redirect.
    pub fn ensure_my_feeds(&mut self, owner: &Caller, principal: &str) -> WsResult<Value> {
        let feeds = self.feed_entities()?;
        let samples: Vec<String> = feeds.iter().filter(|(_, f, p)| f == "my_responses" && p.is_none()).map(|(id, ..)| id.clone()).collect();
        let mut map = Map::new();
        let mut ops = Vec::new();
        for sample in samples {
            let own = feeds::my_feed_id(&sample, principal);
            if !feeds.iter().any(|(id, ..)| *id == own) {
                ops.push(json!({ "op": "entity.create", "entity_id": own, "type_id": TYPE_TABLE, "parent_id": SESSION_ID, "order_key": okey("m", &own[2..12]),
                                 "scope": format!("{PARTICIPANT_PREFIX}{principal}"),
                                 "payload": { "description": feeds::title("my_responses"), "source": { "kind": "session", "feed": "my_responses", "mode": "live" },
                                              "fields": feeds::fields("my_responses") } }));
            }
            map.insert(sample, json!(own));
        }
        if !ops.is_empty() {
            let idem = format!("my-feeds/{}", &sha256_hex(principal.as_bytes())[..24]);
            let req = self.commit_request(&idem, "session: a participant's own answers", ops);
            let result = self.commit(&req, owner, &CommitOpts { as_session: true, as_principal: Some(principal.to_string()), ..CommitOpts::default() });
            if result["status"] != json!("accepted") {
                return Err(commit_error(&result));
            }
            // answers given before (a rejoin after the table was created elsewhere) show up at once
            self.session_commit(owner, principal, &format!("my-feeds-fill/{}/{}", &sha256_hex(principal.as_bytes())[..24], self.head_seq), Vec::new(), &BTreeMap::new(),
                                &FeedSel::default().with_mine(principal), &[])?;
        }
        Ok(Value::Object(map))
    }
}

/// `session.feed_sample { workspace_id, feed }` (Q20): the courseware's sample entity of a feed in the `session` folder,
/// created the first time a Block binds it (fixed fields, example rows, read-only like every Source). One per feed.
pub fn feed_sample(ws: &mut Workspace, caller: &Caller, feed: &str, public: bool) -> WsResult<Value> {
    if !aiworkspace_core::types::SESSION_FEEDS.contains(&feed) {
        return Err(WsError::invalid_op(format!("unknown feed {feed}")));
    }
    let access = ws.require_ws_any(caller)?;
    {
        let now = ws.now();
        if !access.can(&ws.ctx(&now), DATA_ID, Cap::Structure)? {
            return Err(WsError::denied("placing a session feed needs structure on the data tree"));
        }
    }
    // `participants` may also be placed public (Q18: names and presence only, the principals stay with the presenter)
    let scope = if public && feed == "participants" { SCOPE_SHARED } else { feeds::default_scope(feed) };
    for (id, f, p) in ws.feed_entities()? {
        if f == feed && p.is_none() && ws.entity_scope(&id)?.as_deref() == Some(scope) {
            return Ok(json!({ "ok": true, "entity_id": id, "created": false }));
        }
    }
    let id = random_id("sf_");
    let name = if scope == SCOPE_SHARED && feed == "participants" { "participants-public".to_string() } else { feed.to_string() };
    let mut ops = vec![
        json!({ "op": "system.ensure", "id": SESSION_ID }),
        json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_TABLE, "parent_id": SESSION_ID, "order_key": okey("f", &id[3..13]), "scope": scope, "name": name,
                "payload": { "description": feeds::title(feed), "source": { "kind": "session", "feed": feed, "mode": "live" }, "fields": feeds::fields(feed) } }),
    ];
    let samples = feeds::sample_records(feed);
    if !samples.is_empty() {
        ops.push(json!({ "op": "table.insert_records", "source_id": id, "records": samples }));
    }
    let req = ws.commit_request(&random_id("fs_"), &format!("session feed {feed}"), ops);
    let result = ws.commit(&req, caller, &CommitOpts { as_source: true, ..CommitOpts::default() });
    if result["status"] != json!("accepted") {
        return Err(commit_error(&result));
    }
    Ok(json!({ "ok": true, "entity_id": id, "created": true, "seq": result["seq"] }))
}

/// Answers, claims and tasks (§5.2): the kernel's part. Who may answer and how many is decided here, in the run
/// Workspace's single writer — two claims for the last slot are serialized, the second sees it taken.
impl Service {
    fn live(&self, s: &SessionRow) -> WsResult<()> {
        if !s.is_live() {
            return Err(ended());
        }
        Ok(())
    }

    /// `session.task { op: create | update | assign | revoke | close | reopen, task_id?, task?, principals?, command_id }`
    /// (presenter). `assign` authorizes the named participants to answer (a pick: the extension's seed and pool go in
    /// `task.pick`); `revoke` takes it back — what they submitted stays, marked invalid.
    pub fn session_task(&self, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        if !actor.role.presents() {
            return Err(WsError::denied("only the presenter runs tasks"));
        }
        self.live(s)?;
        let op = s_op(p.get("op").and_then(Value::as_str).unwrap_or(""))?;
        let command = p.get("command_id").and_then(Value::as_str).filter(|c| !c.is_empty() && c.len() <= 96).ok_or_else(|| WsError::invalid_op("command_id required"))?;
        let now = format_utc_ms((self.clock)());
        let participants = self.sessions.participants(&s.session_id)?;
        let principals: Vec<String> = p.get("principals").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        for who in &principals {
            match participants.iter().find(|x| x.principal == *who) {
                None => return Err(WsError::invalid_op(format!("{who} has not joined this session"))),
                Some(x) if x.level == "guest" && !s.guest_interact() => return Err(WsError::sub(Code::PermissionDenied, "GUEST_NOT_ALLOWED", format!("{who} is a guest: guests only watch in this room"))),
                _ => {}
            }
        }
        let owner = Caller::user(&s.owner);
        self.with_run(s, |ws| {
            let mut ops = Vec::new();
            let mut overrides = BTreeMap::new();
            let mut sel = FeedSel::tasks();
            let task_id = match op {
                "create" => {
                    let task = p.get("task").and_then(Value::as_object).cloned().unwrap_or_default();
                    let id = random_id("tk_");
                    let mut payload = Map::new();
                    for k in ["title", "question", "block_id", "mode", "slots", "pick", "ext", "show_names"] {
                        if let Some(v) = task.get(k) {
                            payload.insert(k.into(), v.clone());
                        }
                    }
                    payload.entry("mode").or_insert(json!("open"));
                    payload.insert("state".into(), json!("open"));
                    payload.insert("created_at".into(), json!(now));
                    if !principals.is_empty() {
                        payload.insert("assigned".into(), json!(principals));
                    }
                    ops.push(json!({ "op": "system.ensure", "id": SESSION_ID }));
                    ops.push(json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_TASK, "parent_id": SESSION_ID, "order_key": okey("t", &id[3..13]), "payload": payload }));
                    overrides.insert(id.clone(), Override { type_id: TYPE_TASK.into(), scope: SCOPE_SHARED.into(), payload });
                    id
                }
                _ => {
                    let id = s_str(p, "task_id")?.to_string();
                    let t = ws.entity_row(&id).ok().filter(|e| e.type_id == TYPE_TASK).ok_or_else(|| WsError::not_found(format!("task {id} not found")))?;
                    let mut payload = t.payload.clone();
                    let mut keys = Vec::new();
                    let mut set = |k: &str, v: Value, payload: &mut Map<String, Value>| {
                        keys.push(json!({ "key": k, "value": v, "expect": { "rev": t.key_rev(k) } }));
                        payload.insert(k.into(), v);
                    };
                    match op {
                        "update" => {
                            for (k, v) in p.get("task").and_then(Value::as_object).cloned().unwrap_or_default() {
                                if !matches!(k.as_str(), "title" | "question" | "block_id" | "slots" | "pick" | "ext" | "show_names") {
                                    return Err(WsError::invalid_op(format!("task.{k} cannot be updated")));
                                }
                                set(&k, v, &mut payload);
                            }
                        }
                        "assign" => {
                            let mut assigned: Vec<Value> = t.payload.get("assigned").and_then(Value::as_array).cloned().unwrap_or_default();
                            for who in &principals {
                                if !assigned.iter().any(|a| a.as_str() == Some(who)) {
                                    assigned.push(json!(who));
                                }
                            }
                            set("assigned", json!(assigned), &mut payload);
                            if let Some(pick) = p.get("task").and_then(|t| t.get("pick")) {
                                set("pick", pick.clone(), &mut payload);
                            }
                        }
                        "revoke" => {
                            let mut revoked: Vec<Value> = t.payload.get("revoked").and_then(Value::as_array).cloned().unwrap_or_default();
                            for who in &principals {
                                if !revoked.iter().any(|a| a.as_str() == Some(who)) {
                                    revoked.push(json!(who));
                                }
                            }
                            set("revoked", json!(revoked), &mut payload);
                        }
                        "close" => {
                            set("state", json!("closed"), &mut payload);
                            set("closed_at", json!(now), &mut payload);
                        }
                        _ => set("state", json!("open"), &mut payload),
                    }
                    if !keys.is_empty() {
                        ops.push(json!({ "op": "entity.set_keys", "entity_id": id, "keys": keys }));
                    }
                    overrides.insert(id.clone(), Override::payload(&t, payload));
                    id
                }
            };
            // the named participants' answers: authorized (assign / create with principals) or invalidated (revoke)
            for who in &principals {
                let rid = feeds::response_id(&task_id, who);
                let existing = ws.entity_row(&rid).ok();
                let name = participants.iter().find(|x| x.principal == *who).map(|x| x.name.clone()).unwrap_or_else(|| who.clone());
                let level = participants.iter().find(|x| x.principal == *who).map(|x| x.level.clone()).unwrap_or_else(|| "verified".into());
                sel = sel.with_mine(who);
                match (op, existing) {
                    ("revoke", Some(r)) => {
                        let mut payload = r.payload.clone();
                        payload.insert("state".into(), json!("invalid"));
                        ops.push(json!({ "op": "entity.set_keys", "entity_id": rid, "keys": [{ "key": "state", "value": "invalid", "expect": { "rev": r.key_rev("state") } }] }));
                        overrides.insert(rid, Override::payload(&r, payload));
                    }
                    ("revoke", None) => {}
                    (_, Some(r)) if r.payload.get("state") == Some(&json!("invalid")) => {
                        let mut payload = r.payload.clone();
                        payload.insert("state".into(), json!("assigned"));
                        ops.push(json!({ "op": "entity.set_keys", "entity_id": rid, "keys": [{ "key": "state", "value": "assigned", "expect": { "rev": r.key_rev("state") } }] }));
                        overrides.insert(rid, Override::payload(&r, payload));
                    }
                    (_, Some(_)) => {}
                    (_, None) => {
                        let payload: Map<String, Value> = json!({ "task_id": task_id, "participant": who, "name": name, "level": level, "state": "assigned", "version": 0 })
                            .as_object().cloned().unwrap_or_default();
                        ops.push(json!({ "op": "entity.create", "entity_id": rid, "type_id": TYPE_RESPONSE, "parent_id": SESSION_ID, "order_key": okey("r", &rid[2..12]),
                                         "scope": format!("{PARTICIPANT_PREFIX}{who}"), "payload": payload }));
                        overrides.insert(rid, Override { type_id: TYPE_RESPONSE.into(), scope: format!("{PARTICIPANT_PREFIX}{who}"), payload });
                    }
                }
            }
            if op != "create" && principals.is_empty() {
                sel = FeedSel::tasks();
            }
            let result = ws.session_commit(&owner, &actor.principal, &format!("task/{command}"), ops, &overrides, &sel, &[])?;
            Ok(json!({ "ok": true, "task_id": task_id, "seq": result["seq"], "replayed": result["replayed"] }))
        })
    }

    /// `session.claim { task_id, command_id }` (§5.2: 限额抢答, the kernel's atomic primitive): a slot of a `claim` task,
    /// or the refusal `CLAIM_FULL`. Claiming again is the same claim; a disconnection gives nothing back (Q09).
    pub fn session_claim(&self, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        self.live(s)?;
        self.may_interact(s, actor)?;
        let task_id = s_str(p, "task_id")?.to_string();
        let owner = Caller::user(&s.owner);
        let now = format_utc_ms((self.clock)());
        self.with_run(s, |ws| {
            let t = ws.entity_row(&task_id).ok().filter(|e| e.type_id == TYPE_TASK).ok_or_else(|| WsError::not_found(format!("task {task_id} not found")))?;
            if t.payload.get("mode") != Some(&json!("claim")) {
                return Err(WsError::invalid_op("this task is not claimed: it is open to everyone or assigned"));
            }
            if t.payload.get("state") != Some(&json!("open")) {
                return Err(task_closed(ws, actor, &task_id, "claim")?);
            }
            let rid = feeds::response_id(&task_id, &actor.principal);
            if let Ok(r) = ws.entity_row(&rid) {
                let state = r.payload.get("state").and_then(Value::as_str).unwrap_or("");
                return match state {
                    "invalid" => Err(WsError::sub(Code::PermissionDenied, "CLAIM_REVOKED", "your claim was revoked")),
                    _ => Ok(json!({ "ok": true, "response_id": rid, "state": state, "duplicate": true })),
                };
            }
            let slots = t.payload.get("slots").and_then(Value::as_u64).unwrap_or(1);
            let taken = ws.session_entities(TYPE_RESPONSE)?.iter()
                .filter(|r| r.payload.get("task_id").and_then(Value::as_str) == Some(task_id.as_str()))
                .filter(|r| matches!(r.payload.get("state").and_then(Value::as_str), Some("claimed" | "submitted")))
                .count() as u64;
            if taken >= slots {
                ws.append_event(NewEvent { actor: &actor.principal, scope: &format!("{PARTICIPANT_PREFIX}{}", actor.principal), track: "participant", kind: "task.claim_refused",
                                           command_id: None, data: json!({ "task_id": task_id, "reason": "full" }), t_rel_ms: None })?;
                return Err(WsError::sub(Code::InvalidOperation, "CLAIM_FULL", format!("all {slots} slots of this task are taken")).with_data(json!({ "slots": slots })));
            }
            let payload: Map<String, Value> = json!({ "task_id": task_id, "participant": actor.principal, "name": actor.name, "level": actor.level,
                                                      "state": "claimed", "version": 0, "claimed_at": now }).as_object().cloned().unwrap_or_default();
            let ops = vec![json!({ "op": "entity.create", "entity_id": rid, "type_id": TYPE_RESPONSE, "parent_id": SESSION_ID, "order_key": okey("r", &rid[2..12]),
                                   "scope": format!("{PARTICIPANT_PREFIX}{}", actor.principal), "payload": payload })];
            let mut overrides = BTreeMap::new();
            overrides.insert(rid.clone(), Override { type_id: TYPE_RESPONSE.into(), scope: format!("{PARTICIPANT_PREFIX}{}", actor.principal), payload });
            let result = ws.session_commit(&owner, &actor.principal, &format!("claim/{task_id}"), ops, &overrides, &FeedSel::of(&["tasks", "responses"]).with_mine(&actor.principal), &[])?;
            Ok(json!({ "ok": true, "response_id": rid, "state": "claimed", "seq": result["seq"], "slot": taken + 1 }))
        })
    }

    /// `session.respond { task_id, content, command_id }` (§5.2, Q07): the participant's own answer, a new version each
    /// time while the task is open; late answers are refused and recorded (§5.2 默认规则).
    pub fn session_respond(&self, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        self.live(s)?;
        self.may_interact(s, actor)?;
        let task_id = s_str(p, "task_id")?.to_string();
        let command = p.get("command_id").and_then(Value::as_str).filter(|c| !c.is_empty() && c.len() <= 96).ok_or_else(|| WsError::invalid_op("command_id required"))?;
        let content = p.get("content").cloned().unwrap_or(Value::Null);
        if content.to_string().len() > 64 * 1024 {
            return Err(WsError::limit("an answer is limited to 64 KiB"));
        }
        let owner = Caller::user(&s.owner);
        let now = format_utc_ms((self.clock)());
        self.with_run(s, |ws| {
            let t = ws.entity_row(&task_id).ok().filter(|e| e.type_id == TYPE_TASK).ok_or_else(|| WsError::not_found(format!("task {task_id} not found")))?;
            let rid = feeds::response_id(&task_id, &actor.principal);
            let existing = ws.entity_row(&rid).ok();
            if let Some(prior) = ws.commit_by_key(&actor.principal, &format!("respond/{command}"))? {
                return Ok(json!({ "ok": true, "response_id": rid, "seq": prior, "duplicate": true }));
            }
            if t.payload.get("state") != Some(&json!("open")) {
                return Err(task_closed(ws, actor, &task_id, "respond")?);
            }
            let revoked = t.payload.get("revoked").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(actor.principal.as_str())));
            let mode = t.payload.get("mode").and_then(Value::as_str).unwrap_or("open");
            match (&existing, mode) {
                _ if revoked => return Err(WsError::sub(Code::PermissionDenied, "ANSWER_REVOKED", "your turn on this task was revoked")),
                (Some(r), _) if r.payload.get("state") == Some(&json!("invalid")) => return Err(WsError::sub(Code::PermissionDenied, "ANSWER_REVOKED", "your turn on this task was revoked")),
                (None, "assigned") => return Err(WsError::sub(Code::PermissionDenied, "NOT_ASSIGNED", "this task is for the participants the presenter picked")),
                (None, "claim") => return Err(WsError::sub(Code::PermissionDenied, "NOT_CLAIMED", "claim a slot before answering")),
                _ => {}
            }
            let mut ops = Vec::new();
            let mut overrides = BTreeMap::new();
            match existing {
                Some(r) => {
                    let version = r.payload.get("version").and_then(Value::as_u64).unwrap_or(0) + 1;
                    let mut payload = r.payload.clone();
                    payload.insert("content".into(), content.clone());
                    payload.insert("state".into(), json!("submitted"));
                    payload.insert("version".into(), json!(version));
                    payload.insert("submitted_at".into(), json!(now));
                    ops.push(json!({ "op": "entity.set_keys", "entity_id": rid, "keys": [
                        { "key": "content", "value": content, "expect": { "rev": r.key_rev("content") } },
                        { "key": "state", "value": "submitted", "expect": { "rev": r.key_rev("state") } },
                        { "key": "version", "value": version, "expect": { "rev": r.key_rev("version") } },
                        { "key": "submitted_at", "value": now, "expect": { "rev": r.key_rev("submitted_at") } } ] }));
                    overrides.insert(rid.clone(), Override::payload(&r, payload));
                }
                None => {
                    let mut payload: Map<String, Value> = json!({ "task_id": task_id, "participant": actor.principal, "name": actor.name, "level": actor.level,
                                                                  "state": "submitted", "version": 1, "submitted_at": now, "content": content })
                        .as_object().cloned().unwrap_or_default();
                    if let Some(origin) = p.get("origin_session").and_then(Value::as_str) {
                        payload.insert("origin_session".into(), json!(origin));
                    }
                    ops.push(json!({ "op": "entity.create", "entity_id": rid, "type_id": TYPE_RESPONSE, "parent_id": SESSION_ID, "order_key": okey("r", &rid[2..12]),
                                     "scope": format!("{PARTICIPANT_PREFIX}{}", actor.principal), "payload": payload }));
                    overrides.insert(rid.clone(), Override { type_id: TYPE_RESPONSE.into(), scope: format!("{PARTICIPANT_PREFIX}{}", actor.principal), payload });
                }
            }
            let result = ws.session_commit(&owner, &actor.principal, &format!("respond/{command}"), ops, &overrides, &FeedSel::of(&["tasks", "responses"]).with_mine(&actor.principal), &[])?;
            Ok(json!({ "ok": true, "response_id": rid, "seq": result["seq"], "replayed": result["replayed"] }))
        })
    }

    /// `session.review { response_id, review: { verdict?, score?, comment?, published? } }` (presenter, §5.2: 评阅):
    /// private until `published`, then the participant sees it in `my_responses`.
    pub fn session_review(&self, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        if !actor.role.reviews() {
            return Err(WsError::denied("only the presenter reviews answers"));
        }
        let rid = s_str(p, "response_id")?.to_string();
        let review = p.get("review").filter(|v| v.is_object()).cloned().ok_or_else(|| WsError::invalid_op("review must be an object"))?;
        let owner = Caller::user(&s.owner);
        let now = format_utc_ms((self.clock)());
        self.with_run(s, |ws| {
            let r = ws.entity_row(&rid).ok().filter(|e| e.type_id == TYPE_RESPONSE).ok_or_else(|| WsError::not_found(format!("response {rid} not found")))?;
            let who = r.payload.get("participant").and_then(Value::as_str).unwrap_or("").to_string();
            let mut payload = r.payload.clone();
            payload.insert("review".into(), review.clone());
            payload.insert("reviewed_at".into(), json!(now));
            let ops = vec![json!({ "op": "entity.set_keys", "entity_id": rid, "keys": [
                { "key": "review", "value": review, "expect": { "rev": r.key_rev("review") } }, { "key": "reviewed_at", "value": now, "expect": { "rev": r.key_rev("reviewed_at") } } ] })];
            let mut overrides = BTreeMap::new();
            overrides.insert(rid.clone(), Override::payload(&r, payload));
            let idem = format!("review/{rid}/{}", &sha256_hex(p["review"].to_string().as_bytes())[..16]);
            let result = ws.session_commit(&owner, &actor.principal, &idem, ops, &overrides, &FeedSel::default().with_mine(&who), &[])?;
            Ok(json!({ "ok": true, "seq": result["seq"] }))
        })
    }

    /// `session.publish { task_id, title?, content, command_id }` (presenter, §4): a result made public on purpose — a
    /// new entity, never the raw answers or their history.
    pub fn session_publish(&self, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        if !actor.role.presents() {
            return Err(WsError::denied("only the presenter publishes results"));
        }
        if s.state != LIVE && s.state != ENDED {
            return Err(ended());
        }
        let task_id = s_str(p, "task_id")?.to_string();
        let command = p.get("command_id").and_then(Value::as_str).filter(|c| !c.is_empty() && c.len() <= 96).ok_or_else(|| WsError::invalid_op("command_id required"))?;
        let content = p.get("content").cloned().unwrap_or(Value::Null);
        let owner = Caller::user(&s.owner);
        let now = format_utc_ms((self.clock)());
        self.with_run(s, |ws| {
            ws.entity_row(&task_id).ok().filter(|e| e.type_id == TYPE_TASK).ok_or_else(|| WsError::not_found(format!("task {task_id} not found")))?;
            let id = format!("rr{}", &sha256_hex(format!("{task_id}\n{command}").as_bytes())[..30]);
            let mut payload: Map<String, Value> = json!({ "task_id": task_id, "content": content, "published_at": now }).as_object().cloned().unwrap_or_default();
            if let Some(t) = p.get("title").and_then(Value::as_str) {
                payload.insert("title".into(), json!(t));
            }
            let ops = vec![
                json!({ "op": "system.ensure", "id": SESSION_ID }),
                json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_RESULT, "parent_id": SESSION_ID, "order_key": okey("x", &id[2..12]), "payload": payload }),
            ];
            let mut overrides = BTreeMap::new();
            overrides.insert(id.clone(), Override { type_id: TYPE_RESULT.into(), scope: SCOPE_SHARED.into(), payload });
            let result = ws.session_commit(&owner, &actor.principal, &format!("publish/{command}"), ops, &overrides, &FeedSel::of(&["results"]), &[])?;
            Ok(json!({ "ok": true, "result_id": id, "seq": result["seq"], "replayed": result["replayed"] }))
        })
    }

    /// Whether `actor` may answer, claim or react: everyone but guests in a room without guest interaction (Q03).
    pub fn may_interact(&self, s: &SessionRow, actor: &Actor) -> WsResult<()> {
        match actor.role {
            Role::Display => Err(WsError::denied("a display only shows the room")),
            Role::Participant if actor.level == "guest" && !s.guest_interact() => {
                Err(WsError::sub(Code::PermissionDenied, "GUEST_NOT_ALLOWED", "guests watch this room: sign in to take part"))
            }
            _ => Ok(()),
        }
    }

    /// `session.command { command: { command_id, kind, args? }, rect? }` (presenter / prompter, Q01-C): the service runs the
    /// step rules and confirms; the stage feed follows. Kinds: `next prev first last goto black back free media sync`.
    pub fn session_command(&self, s: &SessionRow, actor: &Actor, cmd: &Value) -> WsResult<Value> {
        if !actor.role.presents() {
            return Err(WsError::denied("only the presenter controls the stage"));
        }
        self.live(s)?;
        let id = cmd.get("command_id").and_then(Value::as_str).filter(|c| !c.is_empty() && c.len() <= 128).ok_or_else(|| WsError::invalid_op("command.command_id required"))?;
        let kind = cmd.get("kind").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("command.kind required"))?;
        let args = cmd.get("args").cloned().unwrap_or(json!({}));
        if cmd.to_string().len() > 8 * 1024 {
            return Err(WsError::limit("a command is limited to 8 KiB"));
        }
        let now = (self.clock)();
        let participants = self.sessions.participants(&s.session_id)?;
        let owner = Caller::user(&s.owner);
        self.with_run(s, |ws| {
            if let Some(prior) = ws.event_by_command(&actor.principal, id)? {
                return Ok(json!({ "ok": true, "duplicate": true, "event": prior, "stage": ws.stage() }));
            }
            let steps = ws.session_steps()?;
            let mut stage = ws.stage();
            let index = stage["index"].as_u64().unwrap_or(0) as usize;
            let current = steps.iter().position(|st| Some(&st["id"]) == stage.get("step_id")).unwrap_or(index.min(steps.len().saturating_sub(1)));
            let goto = |stage: &mut Value, target: usize, steps: &[Value]| {
                if steps.is_empty() {
                    return false;
                }
                let target = target.min(steps.len() - 1);
                if target == current && stage["free"] != json!(true) {
                    return false;
                }
                let from = steps.get(current);
                let tr = if target == current { "fly" } else { transition(from, &steps[target]) };
                stage["step_id"] = steps[target]["id"].clone();
                stage["index"] = json!(target);
                stage["count"] = json!(steps.len());
                stage["title"] = steps[target]["title"].clone();
                stage["surface_id"] = steps[target]["surface_id"].clone();
                stage["transition"] = json!(tr);
                stage["free"] = json!(false);
                stage["nonce"] = json!(stage["nonce"].as_u64().unwrap_or(0) + 1);
                true
            };
            let changed = match kind {
                "next" => goto(&mut stage, current + 1, &steps),
                "prev" => goto(&mut stage, current.saturating_sub(1), &steps),
                "first" => goto(&mut stage, 0, &steps),
                "last" => goto(&mut stage, steps.len().saturating_sub(1), &steps),
                "goto" => {
                    let want = args.get("step_id").and_then(Value::as_str).unwrap_or("");
                    match steps.iter().position(|st| st["id"].as_str() == Some(want)) {
                        Some(i) => goto(&mut stage, i, &steps),
                        None => return Err(WsError::invalid_op(format!("step {want} is not on the path"))),
                    }
                }
                "back" => stage["free"] == json!(true) && goto(&mut stage, current, &steps),
                "black" => {
                    let on = args.get("on").and_then(Value::as_bool).unwrap_or(stage["black"] != json!(true));
                    let was = stage["black"] == json!(true);
                    stage["black"] = json!(on);
                    was != on
                }
                "free" => {
                    let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
                    let was = stage["free"] == json!(true);
                    stage["free"] = json!(on);
                    was != on
                }
                "media" | "sync" => {
                    let entity = args.get("entity_id").and_then(Value::as_str).filter(|e| aiworkspace_core::id::is_valid_id(e)).ok_or_else(|| WsError::invalid_op("args.entity_id required"))?;
                    let action = args.get("action").and_then(Value::as_str).unwrap_or(if kind == "sync" { "sync" } else { "" });
                    if !matches!(action, "play" | "pause" | "seek" | "rate" | "ended" | "sync") {
                        return Err(WsError::invalid_op("args.action must be play, pause, seek, rate, ended or sync"));
                    }
                    let pos = args.get("media_pos_ms").and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| WsError::invalid_op("args.media_pos_ms required"))?;
                    let rate = args.get("rate").and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0 && *v <= 16.0).unwrap_or(1.0);
                    let prior = stage["media"][entity].clone();
                    let playing = match action {
                        "play" => true,
                        "pause" | "ended" => false,
                        _ => prior["playing"] == json!(true),
                    };
                    if !stage["media"].is_object() {
                        stage["media"] = json!({});
                    }
                    stage["media"][entity] = json!({ "action": action, "media_pos_ms": pos, "t_rel_ms": ws.t_rel_now(), "rate": rate, "playing": playing });
                    true
                }
                _ => return Err(WsError::invalid_op(format!("unknown command {kind}"))),
            };
            if let Some(rect) = cmd.get("rect").filter(|r| r.is_object()) {
                stage["rect"] = rect.clone();
            }
            stage["updated_ms"] = json!(now);
            let data = json!({ "command": { "kind": kind, "args": args }, "stage": stage, "changed": changed });
            let ev = ws.set_stage(&stage, NewEvent { actor: &actor.principal, scope: "public", track: "public", kind: if kind == "media" || kind == "sync" { "media.state" } else { "stage" },
                                                     command_id: Some(id), data, t_rel_ms: None })?;
            if changed && !matches!(kind, "sync") {
                ws.refresh_feeds(&owner, &actor.principal, &format!("stage/{id}"), &FeedSel::of(&["stage"]), &participants)?;
            }
            Ok(json!({ "ok": true, "duplicate": false, "event": ev, "stage": stage }))
        })
    }

    /// `session.react { kind: feedback | danmaku, text?, target_id?, x?, y?, command_id }` (§5.2): position feedback in content
    /// coordinates or a comment flying over the stage; recorded on the public interaction track.
    pub fn session_react(&self, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        self.live(s)?;
        self.may_interact(s, actor)?;
        let kind = p.get("kind").and_then(Value::as_str).filter(|k| matches!(*k, "feedback" | "danmaku")).ok_or_else(|| WsError::invalid_op("kind must be feedback or danmaku"))?;
        let text: String = p.get("text").and_then(Value::as_str).unwrap_or("").chars().take(200).collect();
        if kind == "danmaku" && text.trim().is_empty() {
            return Err(WsError::invalid_op("a comment needs text"));
        }
        let mut data = json!({ "kind": kind, "text": text, "name": actor.name, "level": actor.level });
        for k in ["x", "y"] {
            if let Some(v) = p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()) {
                data[k] = json!(v);
            }
        }
        for k in ["target_id", "surface_id", "symbol"] {
            if let Some(v) = p.get(k).and_then(Value::as_str).filter(|v| v.len() <= 64) {
                data[k] = json!(v);
            }
        }
        let command = p.get("command_id").and_then(Value::as_str).filter(|c| !c.is_empty() && c.len() <= 96);
        self.with_run(s, |ws| {
            let (ev, dup) = ws.append_event(NewEvent { actor: &actor.principal, scope: "public", track: "reaction", kind: "reaction", command_id: command, data, t_rel_ms: None })?;
            Ok(json!({ "ok": true, "event": ev, "duplicate": dup }))
        })
    }

    /// A sample of a continuous state (Q01-C, Q15): the presenter's pointer, free camera or a stroke in progress; an
    /// audience member's own browsing. Recorded with the demonstration time it was taken at.
    pub fn session_sample(&self, s: &SessionRow, actor: &Actor, kind: &str, data: Value, t_rel_ms: Option<i64>) -> WsResult<Value> {
        self.live(s)?;
        let (scope, track) = match (actor.role.presents(), kind) {
            (true, "pointer" | "camera" | "ink") => ("public".to_string(), "public"),
            (true, "device") => ("diag".to_string(), "voice"),
            (false, "camera" | "buffer") => (format!("{PARTICIPANT_PREFIX}{}", actor.principal), "participant"),
            _ => return Err(WsError::denied(format!("{kind} samples are not yours to send"))),
        };
        self.with_run(s, |ws| Ok(ws.append_event(NewEvent { actor: &actor.principal, scope: &scope, track, kind: &format!("sample.{kind}"), command_id: None, data, t_rel_ms })?.0))
    }
}

fn s_op(op: &str) -> WsResult<&'static str> {
    Ok(match op {
        "create" => "create",
        "update" => "update",
        "assign" => "assign",
        "revoke" => "revoke",
        "close" => "close",
        "reopen" => "reopen",
        _ => return Err(WsError::invalid_op("op must be create, update, assign, revoke, close or reopen")),
    })
}

fn s_str<'a>(p: &'a Value, key: &str) -> WsResult<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|v| aiworkspace_core::id::is_valid_id(v)).ok_or_else(|| WsError::invalid_op(format!("{key} required")))
}

/// A late answer or claim: refused, and the attempt is on the participant's track (§5.2: 迟到提交拒绝并记事件).
fn task_closed(ws: &Workspace, actor: &Actor, task_id: &str, what: &str) -> WsResult<WsError> {
    ws.append_event(NewEvent { actor: &actor.principal, scope: &format!("{PARTICIPANT_PREFIX}{}", actor.principal), track: "participant", kind: "task.late",
                               command_id: None, data: json!({ "task_id": task_id, "attempt": what }), t_rel_ms: None })?;
    Ok(WsError::sub(Code::InvalidOperation, "TASK_CLOSED", "the task is closed: nothing more is accepted"))
}

impl Workspace {
    /// One recorded segment of the voice (§7.3.3): an Ogg Opus file kept as an asset of the run Workspace (an AssetRef
    /// in the `session` folder, so the archive and the retention rules see it) and a `voice.segment` event naming it
    /// with its place on the demonstration time line.
    pub fn session_voice_segment(&mut self, owner: &Caller, actor: &str, bytes: &[u8], index: u64, t_rel_start: i64, t_rel_end: i64, lost_ms: i64) -> WsResult<Value> {
        let staged = self.stage_asset(owner, bytes)?;
        let object_id = staged["object_id"].as_str().unwrap_or("").to_string();
        let id = format!("va{}", &sha256_hex(format!("{object_id}\n{index}").as_bytes())[..30]);
        let ops = vec![
            json!({ "op": "system.ensure", "id": SESSION_ID }),
            json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_ASSET, "parent_id": SESSION_ID, "order_key": okey("v", &format!("{index:08}")),
                    "payload": { "object_id": object_id, "file_name": format!("voice-{index:05}.ogg"), "duration_ms": (t_rel_end - t_rel_start).max(1) } }),
        ];
        self.session_commit(owner, actor, &format!("voice/{object_id}"), ops, &BTreeMap::new(), &FeedSel::default(), &[])?;
        let data = json!({ "segment_id": object_id, "entity_id": id, "index": index, "t_rel_start": t_rel_start, "t_rel_end": t_rel_end, "lost_ms": lost_ms,
                           "media_type": staged["media_type"], "size": bytes.len() });
        let (ev, _) = self.append_event(NewEvent { actor, scope: "public", track: "voice", kind: "voice.segment", command_id: None, data, t_rel_ms: Some(t_rel_start) })?;
        Ok(ev)
    }

    /// The `seq` of a Commit `principal` wrote under `idem_key`, if any.
    pub fn commit_by_key(&self, principal: &str, idem_key: &str) -> WsResult<Option<u64>> {
        self.doc.query_row("SELECT seq FROM commits WHERE principal = ?1 AND idem_key = ?2", params![principal, idem_key], |r| r.get(0)).optional().map_err(db_err)
    }

    /// The baseline rows a replay starts from (§7.1), cut to what `access` may read now (§7.2: a revoked reader reads
    /// nothing more, whatever it could see then).
    pub fn replay_baseline(&self, access: &Access) -> WsResult<Value> {
        let text = std::fs::read_to_string(self.dir.join("session").join("baseline.json")).map_err(io)?;
        let mut rows: Value = serde_json::from_str(&text).map_err(|e| WsError::io(e.to_string()))?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut readable: BTreeMap<String, bool> = BTreeMap::new();
        let mut can_read = |id: &str| -> WsResult<bool> {
            if let Some(v) = readable.get(id) {
                return Ok(*v);
            }
            let ok = match ctx.entity(id)? {
                Some(e) => access.can_read(&ctx, &e)?,
                None => false,
            };
            readable.insert(id.to_string(), ok);
            Ok(ok)
        };
        crate::workspace::filter_rows(&mut rows, &mut can_read)?;
        Ok(rows)
    }
}

/// Is `dir` a session's run Workspace (read without opening a writer)?
pub fn is_session_dir(dir: &Path) -> bool {
    let Ok(conn) = Connection::open_with_flags(dir.join("doc.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return false };
    meta_get(&conn, "purpose").ok().flatten().as_deref() == Some(PURPOSE_SESSION)
}

/// A link token of a session (`at_` / `dt_` / `pt_` + 26 base32).
pub fn is_link_token(t: &str) -> bool {
    ["at_", "dt_", "pt_"].iter().any(|p| is_prefixed_id(p, t))
}
