//! One path for every entry point (核心 API 调整 R5, R14; design draft §2, §5): `/data/销售`, `/surfaces/看板/text-3`,
//! `~<entity_id>/…`. A path is a walk of names from the root; every step is judged by the caller's `can_read`, and an
//! entity the caller cannot read is reported exactly like one that does not exist. `doc.resolve`, `doc.list`,
//! `doc.find`, the object paths, the stage tools and the CLI all use this module.
//!
//! Names are unique among the live siblings that share a namespace (`naming::same_namespace`). A caller who sees
//! several entities of one name under one parent (a shared one and its own personal one, or a reviewer seeing several
//! participants') resolves the name to one of them: shared first, then the caller's own scope, then sibling order. The
//! others are *shadowed*: their path starts at `~<id>`.

use crate::access::Access;
use crate::canonical::sha256_hex;
use crate::error::{Code, WsError, WsResult};
use crate::model::*;
use base64::Engine;
use serde_json::{json, Value};

pub const LIST_DEFAULT: usize = 100;
pub const LIST_MAX: usize = 1000;
pub const FIND_DEFAULT: usize = 50;
pub const FIND_MAX: usize = 200;
pub const FIND_MAX_DEPTH: usize = 16;
/// Entities one `find` call visits at most before it stops with `complete: false`.
pub const FIND_VISIT_BUDGET: usize = 20_000;

/// The short type name of an entity (filters, `meta.type`): `table`, `richtext`, `record`, `annotation`, `block`,
/// `container`, `asset`, `dir`, `wish`, `def`, `viewport`, `path`, `task`, `response`, `result`; an unknown type
/// is its `type_id`.
pub fn short_type(type_id: &str) -> &str {
    match type_id {
        TYPE_TABLE => "table",
        TYPE_RICHTEXT => "richtext",
        TYPE_RECORD => "record",
        TYPE_ANNOTATION => "annotation",
        TYPE_CELL => "block",
        TYPE_CONTAINER => "container",
        TYPE_ASSET => "asset",
        TYPE_DIR => "dir",
        TYPE_WISH => "wish",
        TYPE_BLOCK_DEF => "def",
        TYPE_VIEWPORT => "viewport",
        TYPE_SHOW_PATH => "path",
        TYPE_TASK => "task",
        TYPE_RESPONSE => "response",
        TYPE_RESULT => "result",
        t => t,
    }
}

/// What `find` and `grep` leave out unless `type` names them.
pub const DEFAULT_EXCLUDED: &[&str] = &["annotation", "viewport", "path", "task", "response", "result"];

/// Keep / drop entity types: short names, full `type_id`s, or `container:<kind>`.
#[derive(Debug, Clone, Default)]
pub struct TypeFilter {
    pub keep: Vec<String>,
    pub drop: Vec<String>,
}

fn type_list(v: Option<&Value>, key: &str) -> WsResult<Vec<String>> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()),
        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| WsError::invalid_op(format!("{key} must be strings")))).collect(),
        Some(_) => Err(WsError::invalid_op(format!("{key} must be a string or an array of strings"))),
    }
}

impl TypeFilter {
    /// `type` / `not_type` of a request; `defaults` applies [`DEFAULT_EXCLUDED`] when `type` is not given.
    pub fn parse(params: &Value, defaults: bool) -> WsResult<TypeFilter> {
        let keep = type_list(params.get("type"), "type")?;
        let mut drop = type_list(params.get("not_type"), "not_type")?;
        if defaults && keep.is_empty() {
            drop.extend(DEFAULT_EXCLUDED.iter().map(|s| s.to_string()));
        }
        Ok(TypeFilter { keep, drop })
    }

    fn one(spec: &str, e: &EntityRow) -> bool {
        let short = short_type(&e.type_id);
        if spec == short || spec == e.type_id {
            return true;
        }
        match spec.strip_prefix("container:") {
            Some(kind) => e.type_id == TYPE_CONTAINER && e.payload.get("kind").and_then(Value::as_str) == Some(kind),
            None => false,
        }
    }

    pub fn matches(&self, e: &EntityRow) -> bool {
        (self.keep.is_empty() || self.keep.iter().any(|s| Self::one(s, e))) && !self.drop.iter().any(|s| Self::one(s, e))
    }
}

fn not_found(what: &str) -> WsError {
    WsError::not_found(format!("{what} not found"))
}

/// Preference among visible siblings of one name: shared, the caller's own scope, the rest.
fn class(access: &Access, e: &EntityRow) -> u8 {
    if e.scope == SCOPE_SHARED {
        0
    } else if e.owner() == Some(access.principal.as_str()) || e.participant() == Some(access.principal.as_str()) {
        1
    } else {
        2
    }
}

/// One child as a caller sees it.
pub struct Child {
    pub entity: EntityRow,
    pub readable: bool,
    /// Another visible sibling takes its name: its path starts at `~<id>`.
    pub shadowed: bool,
}

/// The live children of `parent` the caller's scope sees (readable or not), in sibling order, with shadowing.
pub fn visible_children(ctx: &dyn ReadCtx, access: &Access, parent: &str) -> WsResult<Vec<Child>> {
    let mut out: Vec<Child> = Vec::new();
    for edge in ctx.children(parent)? {
        let Some(e) = ctx.entity(&edge.child_id)? else { continue };
        if !e.alive() || !access.sees_scope(&e) {
            continue;
        }
        let readable = access.can_read(ctx, &e)?;
        out.push(Child { entity: e, readable, shadowed: false });
    }
    // the winner of each name: the best class, then the first in sibling order
    let mut winner: std::collections::BTreeMap<String, (u8, usize)> = std::collections::BTreeMap::new();
    for (i, c) in out.iter().enumerate() {
        if let Some(n) = &c.entity.name {
            let k = (class(access, &c.entity), i);
            winner.entry(n.clone()).and_modify(|w| *w = (*w).min(k)).or_insert(k);
        }
    }
    for (i, c) in out.iter_mut().enumerate() {
        c.shadowed = match &c.entity.name {
            Some(n) => winner.get(n).is_some_and(|w| w.1 != i),
            None => true,
        };
    }
    Ok(out)
}

/// Is `e` (under `parent`) reached by its name?
fn named_step(ctx: &dyn ReadCtx, access: &Access, e: &EntityRow, parent: &str) -> WsResult<bool> {
    let Some(name) = &e.name else { return Ok(false) };
    if e.scope == SCOPE_SHARED {
        return Ok(true);
    }
    let kids = visible_children(ctx, access, parent)?;
    Ok(kids.iter().any(|c| c.entity.entity_id == e.entity_id && !c.shadowed) && !name.is_empty())
}

/// A path, or an `entity_id` (anything not starting with `/` or `~`), to the entity it names. Every step must be
/// readable and live; anything else is `NOT_FOUND`.
pub fn resolve(ctx: &dyn ReadCtx, access: &Access, at: &str) -> WsResult<String> {
    let readable_live = |id: &str| -> WsResult<bool> {
        Ok(match ctx.entity(id)? {
            Some(e) => e.alive() && access.can_read(ctx, &e)?,
            None => false,
        })
    };
    if !at.starts_with('/') && !at.starts_with('~') {
        return if readable_live(at)? { Ok(at.to_string()) } else { Err(not_found(&format!("entity {at}"))) };
    }
    let mut segs = at.split('/').filter(|s| !s.is_empty()).peekable();
    let mut cur = match segs.peek() {
        Some(s) if s.starts_with('~') => {
            let id = s[1..].to_string();
            segs.next();
            if id.is_empty() || !readable_live(&id)? {
                return Err(not_found(&format!("entity {id}")));
            }
            id
        }
        _ => {
            if !readable_live(ROOT_ID)? && segs.peek().is_none() {
                return Err(not_found("path /"));
            }
            ROOT_ID.to_string()
        }
    };
    for seg in segs {
        if seg == "." {
            continue;
        }
        let kids = visible_children(ctx, access, &cur)?;
        let hit = kids.iter().find(|c| c.entity.name.as_deref() == Some(seg) && !c.shadowed);
        match hit {
            Some(c) if c.readable => cur = c.entity.entity_id.clone(),
            _ => return Err(not_found(&format!("path segment {seg:?}"))),
        }
    }
    Ok(cur)
}

/// The path of `id` as the caller may print it: from the root when every ancestor is readable, else from the highest
/// readable ancestor as `~<id>/…` (an unreadable ancestor's name and id never appear). A shadowed or unnamed entity
/// starts its own path (`~<id>`).
pub fn path_of(ctx: &dyn ReadCtx, access: &Access, id: &str) -> WsResult<String> {
    if id == ROOT_ID {
        return Ok("/".into());
    }
    let mut segs: Vec<String> = Vec::new();
    let mut cur = ctx.entity(id)?.ok_or_else(|| not_found(&format!("entity {id}")))?;
    for _ in 0..4096 {
        let Some(edge) = ctx.edge(&cur.entity_id)? else { break };
        let parent = ctx.entity(&edge.parent_id)?;
        let parent_ok = match &parent {
            Some(p) => p.alive() && access.can_read(ctx, p)?,
            None => false,
        };
        if !parent_ok || !named_step(ctx, access, &cur, &edge.parent_id)? {
            segs.reverse();
            let tail = if segs.is_empty() { String::new() } else { format!("/{}", segs.join("/")) };
            return Ok(format!("~{}{tail}", cur.entity_id));
        }
        segs.push(cur.name.clone().unwrap_or_default());
        if edge.parent_id == ROOT_ID {
            break;
        }
        cur = parent.expect("checked above");
    }
    segs.reverse();
    Ok(format!("/{}", segs.join("/")))
}

/// The path of a child of a node whose path is `parent_path`.
pub fn child_path(parent_path: &str, c: &Child) -> String {
    match (&c.entity.name, c.shadowed) {
        (Some(n), false) if parent_path == "/" => format!("/{n}"),
        (Some(n), false) => format!("{parent_path}/{n}"),
        _ => format!("~{}", c.entity.entity_id),
    }
}

fn encode_cursor(v: &Value) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string())
}

/// A cursor of this module: bound to `head_seq` and to the request it continues (`digest`).
pub fn decode_cursor(cursor: Option<&str>, head_seq: u64, digest: &str) -> WsResult<Option<Value>> {
    let Some(c) = cursor else { return Ok(None) };
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(c)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .ok_or_else(|| WsError::invalid_op("bad cursor"))?;
    if raw["d"] != json!(digest) {
        return Err(WsError::invalid_op("the cursor belongs to another request"));
    }
    if raw["h"] != json!(head_seq) {
        return Err(WsError::new(Code::SnapshotExpired, "the Workspace changed since this cursor; start again").with_data(json!({ "head_seq": head_seq })));
    }
    Ok(Some(raw))
}

/// The request a cursor continues, without its paging keys.
pub fn request_digest(params: &Value, keys: &[&str]) -> String {
    let picked: serde_json::Map<String, Value> = keys.iter().filter_map(|k| params.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
    sha256_hex(Value::Object(picked).to_string().as_bytes())[..16].to_string()
}

pub fn new_cursor(head_seq: u64, digest: &str, rest: Value) -> String {
    let mut v = rest;
    v["h"] = json!(head_seq);
    v["d"] = json!(digest);
    encode_cursor(&v)
}

fn bounded(params: &Value, key: &str, default: usize, max: usize) -> WsResult<usize> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_u64().filter(|n| *n >= 1 && *n as usize <= max).map(|n| n as usize).ok_or_else(|| WsError::limit(format!("{key} must be 1..{max}"))),
    }
}

/// One row of `ls` / `find`: identity, place and what reading it gives, never its content.
pub fn item(ctx: &dyn ReadCtx, access: &Access, c: &Child, path: &str) -> WsResult<Value> {
    let e = &c.entity;
    let mut children = 0u64;
    for k in ctx.children(&e.entity_id)? {
        if let Some(x) = ctx.entity(&k.child_id)? {
            if x.alive() && access.can_read(ctx, &x)? {
                children += 1;
            }
        }
    }
    let body = crate::body::describe(ctx, e)?;
    let mut v = json!({
        "entity_id": e.entity_id, "name": e.name, "path": path, "type": short_type(&e.type_id), "type_id": e.type_id,
        "title": crate::naming::title_source(&e.type_id, &e.payload), "children": children, "content_rev": e.content_rev,
        "body": { "state": body.state, "format": body.format },
    });
    if e.type_id == TYPE_CONTAINER {
        v["kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
    }
    if e.type_id == TYPE_CELL {
        v["kind"] = e.payload.get("view").and_then(|x| x.get("type")).cloned().unwrap_or(Value::Null);
    }
    if let Some(s) = &e.schema_id {
        v["schema_id"] = json!(s);
    }
    if e.scope != SCOPE_SHARED {
        v["scope"] = json!(e.scope);
    }
    if c.shadowed {
        v["shadowed"] = json!(true);
    }
    Ok(v)
}

/// `doc.list { at?, type?, not_type?, schema?, limit?, cursor? }`: the readable direct children of one node.
pub fn list(ctx: &dyn ReadCtx, access: &Access, params: &Value, head_seq: u64) -> WsResult<Value> {
    let at = params.get("at").and_then(Value::as_str).unwrap_or("/");
    let filter = TypeFilter::parse(params, false)?;
    let schema = params.get("schema").and_then(Value::as_str);
    let limit = bounded(params, "limit", LIST_DEFAULT, LIST_MAX)?;
    let digest = request_digest(params, &["at", "type", "not_type", "schema"]);
    let skip = decode_cursor(params.get("cursor").and_then(Value::as_str), head_seq, &digest)?.map_or(0, |c| c["n"].as_u64().unwrap_or(0) as usize);
    let id = resolve(ctx, access, at)?;
    let at_path = path_of(ctx, access, &id)?;
    let mut items = Vec::new();
    let mut n = 0usize;
    let mut more = false;
    for c in visible_children(ctx, access, &id)? {
        if !c.readable || !filter.matches(&c.entity) || schema.is_some_and(|s| c.entity.schema_id.as_deref() != Some(s)) {
            continue;
        }
        n += 1;
        if n <= skip {
            continue;
        }
        if items.len() == limit {
            more = true;
            break;
        }
        items.push(item(ctx, access, &c, &child_path(&at_path, &c))?);
    }
    let mut out = json!({ "at": { "entity_id": id, "path": at_path }, "items": items, "head_seq": head_seq });
    if more {
        out["next_cursor"] = json!(new_cursor(head_seq, &digest, json!({ "n": skip + limit })));
    }
    Ok(out)
}

/// `doc.find { at?, depth?, name?, type?, not_type?, schema?, limit?, cursor? }`: a pre-order walk under `at` (not
/// into what the caller cannot read), keeping entities whose name or title contains `name` (case-insensitive, literal).
/// A call visits at most [`FIND_VISIT_BUDGET`] entities; a walk it did not finish says `complete: false` and gives a
/// cursor (the visit count, bound to `head_seq`).
pub fn find(ctx: &dyn ReadCtx, access: &Access, params: &Value, head_seq: u64) -> WsResult<Value> {
    let at = params.get("at").and_then(Value::as_str).unwrap_or("/");
    let filter = TypeFilter::parse(params, true)?;
    let schema = params.get("schema").and_then(Value::as_str);
    let limit = bounded(params, "limit", FIND_DEFAULT, FIND_MAX)?;
    let depth = bounded(params, "depth", FIND_MAX_DEPTH, FIND_MAX_DEPTH)?;
    let needle = match params.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() && s.chars().count() <= 256 => Some(s.to_lowercase()),
        Some(_) => return Err(WsError::invalid_op("name must be a string of 1..256 characters")),
    };
    let digest = request_digest(params, &["at", "depth", "name", "type", "not_type", "schema"]);
    let skip = decode_cursor(params.get("cursor").and_then(Value::as_str), head_seq, &digest)?.map_or(0, |c| c["n"].as_u64().unwrap_or(0) as usize);
    let id = resolve(ctx, access, at)?;
    let at_path = path_of(ctx, access, &id)?;
    let mut items = Vec::new();
    let mut visited = 0usize;
    let push_children = |stack: &mut Vec<(Child, String, usize)>, parent: &str, parent_path: &str, level: usize| -> WsResult<()> {
        let kids: Vec<Child> = visible_children(ctx, access, parent)?.into_iter().filter(|c| c.readable).collect();
        for c in kids.into_iter().rev() {
            let path = child_path(parent_path, &c);
            stack.push((c, path, level));
        }
        Ok(())
    };
    let mut stack: Vec<(Child, String, usize)> = Vec::new();
    push_children(&mut stack, &id, &at_path, 1)?;
    let mut stopped = false;
    while let Some((c, path, level)) = stack.pop() {
        if visited >= skip && (visited - skip >= FIND_VISIT_BUDGET || items.len() == limit) {
            stopped = true;
            break;
        }
        visited += 1;
        if visited > skip {
            let e = &c.entity;
            let title = crate::naming::title_source(&e.type_id, &e.payload);
            let matched = match &needle {
                None => Some("type"),
                Some(n) if e.name.as_deref().is_some_and(|x| x.to_lowercase().contains(n.as_str())) => Some("name"),
                Some(n) if title.as_deref().is_some_and(|x| x.to_lowercase().contains(n.as_str())) => Some("title"),
                Some(_) => None,
            };
            if let Some(m) = matched.filter(|_| filter.matches(e) && schema.is_none_or(|s| e.schema_id.as_deref() == Some(s))) {
                let mut v = item(ctx, access, &c, &path)?;
                if needle.is_some() {
                    v["match"] = json!(m);
                }
                items.push(v);
            }
        }
        if level < depth {
            push_children(&mut stack, &c.entity.entity_id, &path, level + 1)?;
        }
    }
    // a page that ended exactly at the walk's end has nothing more to give
    let complete = !stopped;
    let mut out = json!({ "at": { "entity_id": id, "path": at_path }, "items": items, "complete": complete, "visited": visited.saturating_sub(skip), "head_seq": head_seq });
    if !complete {
        out["next_cursor"] = json!(new_cursor(head_seq, &digest, json!({ "n": visited })));
    }
    Ok(out)
}
