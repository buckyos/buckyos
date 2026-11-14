//! What the home page and the shell's configuration read (UI 框架 PRD §4.6, §9; K4, K6, K7): a Workspace's shared
//! settings, when it last changed, whether it is public, and the presentation sessions a caller may open without a link.

use crate::docdb::{db_err, meta_get};
use crate::session::registry::{SessionRow, ARCHIVED, ENDED, EXPIRED, LIVE, PREPARED};
use crate::session::{session_json, Role};
use crate::workspace::Caller;
use crate::{Service, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::model::{ReadCtx, ROOT_ID};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::WsResult;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::collections::HashMap;

impl Workspace {
    /// The Workspace's shared settings (K4): the root's `config` — part of the document (committed, undoable, carried by
    /// clones, packages and templates), written by `manage` on `/` — and the version a conditional write expects.
    pub fn settings(&self) -> WsResult<(Value, u64)> {
        let now = self.now();
        let root = self.ctx(&now).entity(ROOT_ID)?;
        Ok(match root {
            Some(r) => (r.payload.get("config").cloned().filter(Value::is_object).unwrap_or_else(|| json!({})), r.key_rev("config")),
            None => (json!({}), 0),
        })
    }

    /// When the content last changed (the last accepted Commit), or when the Workspace was created.
    pub fn changed_at(&self) -> WsResult<Option<String>> {
        let last: Option<String> = self.doc.query_row("SELECT accepted_at FROM commits ORDER BY seq DESC LIMIT 1", [], |r| r.get(0)).optional().map_err(db_err)?;
        Ok(match last {
            Some(at) => Some(at),
            None => meta_get(&self.doc, "created_at")?,
        })
    }

    /// Whether anything here is reachable without being named (§9.2 “已公开”): anonymous access is on and something is
    /// granted to `@guest`, or an entity is published by path (`ws.publish`).
    pub fn public_state(&self) -> WsResult<(bool, bool)> {
        let data = self.acl_data()?;
        let anonymous = data.anonymous && data.has_entries(crate::acl::SUBJECT_GUEST);
        let published: bool = self.local.query_row("SELECT 1 FROM publications WHERE current = 1 LIMIT 1", [], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
        Ok((anonymous, published))
    }
}

/// The run Workspaces' logical group holding the source readers who came in without a link (see
/// [`Service::source_reader_may_open`]): it reads the run Workspace, like a link's group.
pub const SOURCE_READERS_GROUP: &str = "source-readers";

impl Service {
    /// Whether `caller` may open session `s` without a link as a reader of its courseware (K7, 待确认): a person (not a
    /// guest or a link) who may enter the source Workspace in their own right (`ws.list` would list it) and holds `read`
    /// on its `/`, while the room is open to new audience and not expired. They come in as audience.
    pub fn source_reader_may_open(&self, s: &SessionRow, caller: &Caller) -> WsResult<bool> {
        Ok(s.alive() && s.access == "open" && self.reads_courseware(&s.source_workspace_id, caller))
    }

    /// A person (not a guest, a link or a run) whom the Workspace names in its own right (`ws.list` lists it) with
    /// `read` on `/`.
    fn reads_courseware(&self, workspace_id: &str, caller: &Caller) -> bool {
        if caller.is_guest() || caller.kind.as_deref() == Some("run") {
            return false;
        }
        let Ok(handle) = self.workspace(workspace_id) else { return false };
        let ws = handle.lock().unwrap();
        matches!(ws.listed(caller), Ok(Some(a)) if a.ws_caps.has(Cap::Read))
    }

    /// `session.list_open { limit? }` (所有公开课, UI 框架 PRD §9.1, K7): the sessions the caller may open without a link,
    /// across Workspaces — those it owns, presents or joined (expired ones included, to say so), and those of courseware
    /// it reads (see [`Self::source_reader_may_open`]). Live ones first, the rest by start (or creation) time, newest
    /// first. Each says how the caller would come in (`my_role`: presenter, owner, audience — joined before — or
    /// member), whether it presents (`presenting`), when it was last there (`opened_at`) and `cover` (not generated yet:
    /// null). The server adds `online` and `url`.
    pub fn session_list_open(&self, caller: &Caller, limit: usize) -> WsResult<Value> {
        let mut rows = Vec::new();
        if caller.is_guest() {
            return Ok(json!({ "ok": true, "sessions": rows }));
        }
        let all = self.sessions.in_states(&[PREPARED, LIVE, ENDED, ARCHIVED, EXPIRED])?;
        // whether the caller reads each source Workspace, judged once per source
        let mut reads: HashMap<String, bool> = HashMap::new();
        for s in all {
            let participant = self.sessions.participant(&s.session_id, &caller.principal)?;
            let (role, my_role) = if s.presenter == caller.principal {
                (Role::Presenter, "presenter")
            } else if s.owner == caller.principal {
                (Role::Owner, "owner")
            } else if participant.is_some() {
                (Role::Participant, "audience")
            } else {
                if !s.alive() || s.access != "open" {
                    continue;
                }
                let source = s.source_workspace_id.clone();
                if !*reads.entry(source).or_insert_with_key(|id| self.reads_courseware(id, caller)) {
                    continue;
                }
                (Role::Participant, "member")
            };
            let mut v = session_json(&s, Some(role));
            v["my_role"] = json!(my_role);
            v["presenting"] = json!(s.presenter == caller.principal);
            v["opened_at"] = json!(participant.map(|p| format_utc_ms(p.last_seen_ms)));
            v["cover"] = Value::Null;
            rows.push((s.state == LIVE, s.started_ms.unwrap_or(s.created_ms), v));
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let out: Vec<Value> = rows.into_iter().take(limit).map(|(_, _, v)| v).collect();
        Ok(json!({ "ok": true, "sessions": out }))
    }
}
