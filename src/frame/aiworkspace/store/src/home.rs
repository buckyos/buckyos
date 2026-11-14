//! What the home page and the shell's configuration read (UI 框架 PRD §4.6, §9; K4, K6, K7): a Workspace's shared
//! settings, when it last changed, whether it is public, and the presentation sessions a caller may open without a link
//! (the readers of each session's permission object, Q19).

use crate::docdb::{db_err, meta_get};
use crate::session::registry::{SessionRow, ARCHIVED, ENDED, EXPIRED, LIVE, PREPARED};
use crate::session::{session_json, Role};
use crate::workspace::Caller;
use crate::{Service, Workspace};
use aiworkspace_core::access::{Access, Cap};
use aiworkspace_core::model::{EntityRow, ReadCtx, ROOT_ID, SCOPE_SHARED};
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

/// The run Workspaces' logical group holding those who came in without a link as readers of the session's permission
/// object (see [`Service::may_open_unlinked`]): it reads the run Workspace, like a link's group.
pub const SOURCE_READERS_GROUP: &str = "source-readers";

/// The entity of the source Workspace whose readers may open the session without a link (UI 框架 PRD Q19), by id: what
/// `session.create` chose — `policy.readers_of` — or, for a session made before there was one, the Workspace (`/`).
pub fn readers_of(s: &SessionRow) -> String {
    s.policy.get("readers_of").and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(ROOT_ID).to_string()
}

/// The display name of an entity: its title, else its name, else its id.
fn title_of(e: &EntityRow) -> String {
    e.payload.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string).or_else(|| e.name.clone()).unwrap_or_else(|| e.entity_id.clone())
}

impl Workspace {
    /// A permission object as `access` names it — a path or an entity id it may read (`/`: the Workspace itself),
    /// resolved like an entry's resource — by id (`root` for the Workspace).
    pub fn permission_object(&self, access: &Access, resource: &str) -> WsResult<String> {
        let scope = self.resolve_resource(access, resource)?;
        Ok(if scope.is_empty() { ROOT_ID.to_string() } else { scope })
    }

    /// Whether `access` holds `read` on `entity_id` (`root`: the Workspace): by path inheritance, deny first, as every
    /// read is judged. An entity that is deleted, gone or not shared content admits nobody.
    pub fn reads_object(&self, access: &Access, entity_id: &str) -> WsResult<bool> {
        if entity_id == ROOT_ID {
            return Ok(access.ws_caps.has(Cap::Read));
        }
        let now = self.now();
        let ctx = self.ctx(&now);
        match ctx.entity(entity_id)? {
            Some(e) if e.alive() && e.scope == SCOPE_SHARED => access.can_read(&ctx, &e),
            _ => Ok(false),
        }
    }
}

impl Service {
    /// Whether `caller` may open session `s` without a link (K7, Q19): the room is open to new audience and not
    /// expired, and the caller is a person (not a guest, a link or a run) whom the source Workspace names in its own
    /// right (`ws.list` lists it) holding `read` on the session's permission object ([`readers_of`]). They come in as
    /// audience.
    pub fn may_open_unlinked(&self, s: &SessionRow, caller: &Caller) -> WsResult<bool> {
        Ok(s.alive() && s.access == "open" && self.reads_permission_object(&s.source_workspace_id, &readers_of(s), caller))
    }

    /// A person named in its own right by the Workspace with `read` on `entity_id` there.
    fn reads_permission_object(&self, workspace_id: &str, entity_id: &str, caller: &Caller) -> bool {
        if caller.is_guest() || caller.kind.as_deref() == Some("run") {
            return false;
        }
        let Ok(handle) = self.workspace(workspace_id) else { return false };
        let ws = handle.lock().unwrap();
        match ws.listed(caller) {
            Ok(Some(access)) => ws.reads_object(&access, entity_id).unwrap_or(false),
            _ => false,
        }
    }

    /// The session's permission object as its presenter and owner see it (Q19): `{ entity_id, path, title, type_id,
    /// kind, alive }` — `alive: false` when it was deleted or is gone (then nobody comes in without a link until another
    /// is chosen). Named in full: the presenter and the owner read the whole run Workspace anyway.
    pub fn readers_of_json(&self, s: &SessionRow) -> Value {
        let id = readers_of(s);
        let gone = json!({ "entity_id": id, "path": Value::Null, "title": Value::Null, "type_id": Value::Null, "kind": Value::Null, "alive": false });
        let Ok(handle) = self.workspace(&s.source_workspace_id) else { return gone };
        let ws = handle.lock().unwrap();
        if id == ROOT_ID {
            return json!({ "entity_id": ROOT_ID, "path": "/", "title": ws.title(), "type_id": Value::Null, "kind": "workspace", "alive": true });
        }
        let now = ws.now();
        let ctx = ws.ctx(&now);
        match ctx.entity(&id) {
            Ok(Some(e)) if e.alive() => {
                let path = aiworkspace_core::path::path_of(&ctx, &Access::full(&s.owner), &id).unwrap_or_else(|_| format!("~{id}"));
                json!({ "entity_id": id, "path": path, "title": title_of(&e), "type_id": e.type_id, "kind": e.payload.get("kind").cloned().unwrap_or(Value::Null), "alive": true })
            }
            _ => gone,
        }
    }

    /// `session.list_open { limit? }` (所有公开课, UI 框架 PRD §9.1, K7): the sessions the caller may open without a link,
    /// across Workspaces — those it owns, presents or joined (expired ones included, to say so), and those whose
    /// permission object it reads (see [`Self::may_open_unlinked`]). Live ones first, the rest by start (or creation)
    /// time, newest first. Each says how the caller would come in (`my_role`: presenter, owner, audience — joined
    /// before — or member), whether it presents (`presenting`), when it was last there (`opened_at`) and `cover` (not
    /// generated yet: null). The server adds `online` and `url`.
    pub fn session_list_open(&self, caller: &Caller, limit: usize) -> WsResult<Value> {
        let mut rows = Vec::new();
        if caller.is_guest() {
            return Ok(json!({ "ok": true, "sessions": rows }));
        }
        let all = self.sessions.in_states(&[PREPARED, LIVE, ENDED, ARCHIVED, EXPIRED])?;
        // whether the caller reads each permission object, judged once per object
        let mut reads: HashMap<(String, String), bool> = HashMap::new();
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
                let object = (s.source_workspace_id.clone(), readers_of(&s));
                if !*reads.entry(object).or_insert_with_key(|(ws, id)| self.reads_permission_object(ws, id, caller)) {
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
