//! What the home page and the shell's configuration read (UI 框架 PRD §4.6, §9; K4, K6, K7): a Workspace's shared
//! settings, when it last changed, whether it is public, its cover, and the presentation sessions a caller may open
//! without a link (the readers of each session's permission object, Q19).

use crate::docdb::{db_err, meta_get};
use crate::session::registry::{SessionRow, ARCHIVED, ENDED, EXPIRED, LIVE, PREPARED};
use crate::session::{session_json, Role};
use crate::workspace::Caller;
use crate::{Service, Workspace};
use aiworkspace_core::access::{Access, Cap};
use aiworkspace_core::model::{EntityRow, ReadCtx, ROOT_ID, SCOPE_SHARED, SURFACES_ID, TYPE_ASSET, TYPE_CELL, TYPE_CONTAINER};
use aiworkspace_core::value::{format_utc_ms, reference_entity_id, reference_is_local};
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

/// Pictures a cover may be: what the asset route serves as themselves (an SVG it serves as bytes: never a cover).
const COVER_MEDIA_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif", "image/bmp"];
/// A picture larger than this is not a cover: the home page downloads each one whole.
const COVER_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// How many Blocks one cover looks at, at most (a Workspace of many canvases answers `ws.list` in time).
const COVER_SCAN_LIMIT: usize = 2_000;

impl Workspace {
    /// The cover of the home page's card (UI 框架 PRD §9.2, K6), a stopgap until canvases are pictured: on the canvas
    /// changed last — then the others, most recently changed first, ties in tree order — the first Block (in the canvas's
    /// own order) showing an asset that `access` reads, the Block too. A picture shows itself (at most 8 MiB); any other
    /// file its poster (a video frame, a PDF's first page, an album cover). `{ workspace_id, object_id, media_type,
    /// surface_id, entity_id }` — the bytes come from the asset route like every asset's — or null: the card draws its
    /// placeholder.
    pub fn cover(&self, access: &Access) -> WsResult<Value> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut surfaces = Vec::new();
        surfaces_in_order(&ctx, SURFACES_ID, &mut surfaces, 0)?;
        let changed = self.surface_changes()?;
        // stable: equal changes keep the tree order
        surfaces.sort_by_key(|id| std::cmp::Reverse(changed.get(id).copied().unwrap_or(0)));
        self.cover_on(access, &surfaces)
    }

    /// The cover among `surfaces`, in that order (a session's: the canvases of its path, [`Service::session_cover`]).
    pub fn cover_on(&self, access: &Access, surfaces: &[String]) -> WsResult<Value> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut budget = COVER_SCAN_LIMIT;
        for id in surfaces {
            let Some(surface) = ctx.entity(id)? else { continue };
            if !surface.alive() || surface.scope != SCOPE_SHARED || !access.can_read(&ctx, &surface)? {
                continue;
            }
            if let Some(mut picture) = first_picture(&ctx, access, id, &mut budget)? {
                picture["workspace_id"] = json!(self.workspace_id);
                picture["surface_id"] = json!(id);
                return Ok(picture);
            }
            if budget == 0 {
                break;
            }
        }
        Ok(Value::Null)
    }

    /// The last change on each canvas: the highest version, by Commit seq, of the canvas and of everything below it
    /// (its Blocks and their places; a deletion is a change). Moving the canvas itself is not one.
    fn surface_changes(&self) -> WsResult<HashMap<String, u64>> {
        let mut st = self
            .doc
            .prepare_cached(
                "WITH RECURSIVE under(surface_id, id) AS ( \
                   SELECT entity_id, entity_id FROM entities \
                    WHERE type_id = ?1 AND deleted_seq IS NULL AND json_extract(payload_json, '$.kind') = 'surface' \
                   UNION ALL \
                   SELECT under.surface_id, t.child_id FROM tree_edges t JOIN under ON t.parent_id = under.id) \
                 SELECT under.surface_id, MAX(MAX(e.meta_rev, e.content_rev, e.life_rev, COALESCE(t.struct_rev, 0))) \
                   FROM under JOIN entities e ON e.entity_id = under.id \
                   LEFT JOIN tree_edges t ON t.child_id = under.id AND under.id <> under.surface_id \
                  GROUP BY under.surface_id",
            )
            .map_err(db_err)?;
        let rows = st.query_map([TYPE_CONTAINER], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).map_err(db_err)?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, seq) = row.map_err(db_err)?;
            out.insert(id, seq.max(0) as u64);
        }
        Ok(out)
    }
}

/// The canvases under `parent` (through canvas folders, K1) in tree order.
fn surfaces_in_order(ctx: &dyn ReadCtx, parent: &str, out: &mut Vec<String>, depth: usize) -> WsResult<()> {
    if depth > 64 {
        return Ok(());
    }
    for edge in ctx.children(parent)? {
        let Some(e) = ctx.entity(&edge.child_id)? else { continue };
        if !e.alive() || e.type_id != TYPE_CONTAINER {
            continue;
        }
        match e.payload.get("kind").and_then(Value::as_str) {
            Some("surface") => out.push(e.entity_id),
            Some("folder") => surfaces_in_order(ctx, &e.entity_id, out, depth + 1)?,
            _ => {}
        }
    }
    Ok(())
}

/// The first Block below `parent`, depth first in its order, showing a picture `access` reads (see [`Workspace::cover`]):
/// `{ object_id, media_type, entity_id }` (the asset's id).
fn first_picture(ctx: &dyn ReadCtx, access: &Access, parent: &str, budget: &mut usize) -> WsResult<Option<Value>> {
    for edge in ctx.children(parent)? {
        if *budget == 0 {
            return Ok(None);
        }
        *budget -= 1;
        let Some(cell) = ctx.entity(&edge.child_id)? else { continue };
        if !cell.alive() || cell.type_id != TYPE_CELL {
            continue;
        }
        let source = cell.payload.get("source_ref").filter(|r| reference_is_local(r)).and_then(reference_entity_id);
        if let Some(asset_id) = source {
            if let Some(picture) = picture_of(ctx, access, &cell, asset_id)? {
                return Ok(Some(picture));
            }
        }
        if let Some(found) = first_picture(ctx, access, &cell.entity_id, budget)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// What a Block showing asset `asset_id` gives a cover: the picture itself, else the asset's poster — when `access`
/// reads both the Block and the asset and the bytes are here.
fn picture_of(ctx: &dyn ReadCtx, access: &Access, cell: &EntityRow, asset_id: &str) -> WsResult<Option<Value>> {
    let Some(asset) = ctx.entity(asset_id)? else { return Ok(None) };
    if !asset.alive() || asset.type_id != TYPE_ASSET || asset.scope != SCOPE_SHARED {
        return Ok(None);
    }
    if !access.can_read(ctx, cell)? || !access.can_read(ctx, &asset)? {
        return Ok(None);
    }
    let stored = |key: &str| -> WsResult<Option<aiworkspace_core::model::AssetInfo>> {
        match asset.payload.get(key).and_then(Value::as_str) {
            Some(id) => ctx.asset(id),
            None => Ok(None),
        }
    };
    let picture = |info: &aiworkspace_core::model::AssetInfo| COVER_MEDIA_TYPES.contains(&info.media_type.as_str());
    let chosen = match stored("object_id")? {
        Some(info) if picture(&info) && info.size <= COVER_MAX_BYTES => Some(info),
        _ => stored("poster")?.filter(picture),
    };
    Ok(chosen.map(|info| json!({ "object_id": info.object_id, "media_type": info.media_type, "entity_id": asset.entity_id })))
}

/// The run Workspaces' logical group holding those who came in without a link as readers of the session's permission
/// object (see [`Service::may_open_unlinked`]): it reads the run Workspace, like a link's group.
pub const SOURCE_READERS_GROUP: &str = "source-readers";

/// The entity of the source Workspace whose readers may open the session without a link (UI 框架 PRD Q19), by id: what
/// `session.create` chose — `policy.readers_of`. A session without one lets nobody in without a link (Q33).
pub fn readers_of(s: &SessionRow) -> String {
    s.policy.get("readers_of").and_then(Value::as_str).unwrap_or_default().to_string()
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
    /// before — or member), whether it presents (`presenting`), when it was last there (`opened_at`) and its `cover`
    /// ([`Self::session_cover`]). The server adds `online` and `url`.
    pub fn session_list_open(&self, caller: &Caller, limit: usize) -> WsResult<Value> {
        let mut rows = Vec::new();
        if caller.is_guest() {
            return Ok(json!({ "ok": true, "sessions": [] }));
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
            rows.push((s.state == LIVE, s.started_ms.unwrap_or(s.created_ms), v, s));
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        // the covers of what is listed only
        let out: Vec<Value> = rows
            .into_iter()
            .take(limit)
            .map(|(_, _, mut v, s)| {
                v["cover"] = self.session_cover(&s, caller);
                v
            })
            .collect();
        Ok(json!({ "ok": true, "sessions": out }))
    }

    /// The cover of a public lecture's card (UI 框架 PRD §9.2, K6), the same stopgap as a Workspace's
    /// ([`Workspace::cover`]): on the canvases of its presentation path, in the order of the steps, the first Block
    /// showing a picture or a poster — in the run Workspace (what the stage shows) when the caller reads it there, else
    /// in the courseware as the caller reads it now; null when neither gives one.
    pub fn session_cover(&self, s: &SessionRow, caller: &Caller) -> Value {
        for id in [&s.workspace_id, &s.source_workspace_id] {
            let Ok(handle) = self.workspace(id) else { continue };
            let ws = handle.lock().unwrap();
            let Ok(access) = ws.require_ws_any(caller) else { continue };
            let mut surfaces: Vec<String> = Vec::new();
            for step in ws.path_steps(&s.path_id).unwrap_or_default() {
                if let Some(surface) = step["surface_id"].as_str() {
                    if !surfaces.iter().any(|known| known == surface) {
                        surfaces.push(surface.to_string());
                    }
                }
            }
            match ws.cover_on(&access, &surfaces) {
                Ok(cover) if !cover.is_null() => return cover,
                _ => {}
            }
        }
        Value::Null
    }
}
