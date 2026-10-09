//! UI projections (§5.7 层次, §17.4): immutable objects + private state → the card model of the
//! Desktop app (`datamodel/types.ts`). Nothing here is ever written back into an object.

use crate::audience::{AudienceSpec, Reader, ReaderRelations};
use crate::error::HsResult;
use crate::objects::{get_feed, get_object};
use crate::protocol::*;
use crate::publish::{entry_versions, get_entry, get_head};
use crate::settings::UserSettings;
use crate::{hue_of, Station};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone)]
pub struct ViewCtx {
    pub owner: String,
    pub owner_name: String,
    /// did → (name, kind)
    pub names: HashMap<String, (String, String)>,
    pub reader: Reader,
    pub rel: ReaderRelations,
    pub settings: UserSettings,
    pub collector: bool,
}

impl ViewCtx {
    pub fn is_owner(&self) -> bool {
        self.reader == Reader::Owner
    }
}

impl Station {
    pub async fn identity_names(&self) -> HashMap<String, String> {
        self.view_ctx(Reader::Owner).await.names.into_iter().map(|(k, v)| (k, v.0)).collect()
    }

    pub async fn view_ctx(&self, reader: Reader) -> ViewCtx {
        let settings = self.settings().await.unwrap_or_default();
        let owner_name = if settings.profile.name.is_empty() { self.cfg.owner_name.clone() } else { settings.profile.name.clone() };
        let mut names: HashMap<String, (String, String)> = HashMap::new();
        for c in self.contacts.list().await {
            names.insert(c.did.clone(), (c.name.clone(), "person".into()));
        }
        let sources: Vec<(String, String)> = self
            .db
            .call(|c| {
                let mut stmt = c.prepare("SELECT did, name FROM sources WHERE did IS NOT NULL")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap_or_default();
        for (did, name) in sources {
            names.entry(did).or_insert((name, "person".into()));
        }
        for collector in &settings.collectors {
            names.entry(collector.did.clone()).or_insert((collector.name.clone(), "site".into()));
        }
        names.insert(self.cfg.owner.clone(), (owner_name.clone(), "self".into()));
        let rel = self.reader_relations(&reader).await;
        ViewCtx { owner: self.cfg.owner.clone(), owner_name, names, reader, rel, settings, collector: self.cfg.collector }
    }
}

pub fn identity_view(ctx: &ViewCtx, did: &str) -> Value {
    let (name, kind) = ctx
        .names
        .get(did)
        .cloned()
        .unwrap_or_else(|| (did.trim_start_matches("did:").split_once(':').map(|x| x.1).unwrap_or(did).to_string(), "person".into()));
    json!({ "did": did, "name": name, "kind": kind, "hue": hue_of(did) })
}

/// Optional fields are omitted rather than sent as `null` (the UI types use `?:`).
pub fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, without_nulls(v))).collect()),
        Value::Array(items) => Value::Array(items.into_iter().map(without_nulls).collect()),
        other => other,
    }
}

/// The Desktop's FileObject view: `{ kind: 'file', name, meta: { mime, size, ... } }`.
pub fn file_view(body: &Value) -> Value {
    let mut meta = json!({
        "mime": body.get("mime").and_then(Value::as_str).unwrap_or("application/octet-stream"),
        "size": body.get("size").and_then(Value::as_u64).unwrap_or(0),
    });
    for key in ["width", "height", "duration_ms"] {
        if let Some(v) = body.get(key) {
            meta[key] = v.clone();
        }
    }
    json!({ "kind": "file", "name": body.get("name").and_then(Value::as_str).unwrap_or(""), "meta": meta })
}

fn local_file(conn: &Connection, id: &str) -> HsResult<Option<Value>> {
    Ok(get_object(conn, id)?.filter(|o| o.obj_type == OBJ_TYPE_FILE).map(|o| file_view(&o.body)))
}

pub fn entry_state(conn: &Connection, obj_id: &str, feed: &FeedObject) -> HsResult<Option<Value>> {
    let Some(entry) = &feed.entry else { return Ok(None) };
    let Some(head) = get_head(conn, entry)? else { return Ok(None) };
    let versions = entry_versions(conn, entry)?;
    let version = versions.iter().position(|v| v == obj_id).map(|i| i + 1).unwrap_or(1);
    let state = if head.conflict_obj_id.is_some() { "conflict" } else { head.state.as_str() };
    Ok(Some(json!({
        "entry": entry,
        "seq": head.seq,
        "state": state,
        "currentObjId": head.current,
        "isLatest": head.state == HeadState::Active && head.current.as_deref() == Some(obj_id),
        "version": version,
        "versionCount": versions.len().max(1),
    })))
}

pub fn tier_spec(tier: &str) -> AudienceSpec {
    match tier {
        "followers" => AudienceSpec::Followers,
        "friends" => AudienceSpec::Friends,
        "group" => AudienceSpec::Group { group_id: String::new() },
        "dids" => AudienceSpec::Dids { dids: vec![] },
        _ => AudienceSpec::Public,
    }
}

/// Own entries show their full audience; others only reveal the tier (§4.5).
fn audience_view(conn: &Connection, ctx: &ViewCtx, feed: &FeedObject) -> HsResult<Value> {
    let Some(entry) = &feed.entry else { return Ok(json!({ "spec": { "kind": "public" }, "restricted": false })) };
    if let Some(row) = get_entry(conn, entry)? {
        let spec = if ctx.is_owner() { row.audience.clone() } else { tier_spec(row.audience.tier()) };
        return Ok(json!({ "spec": spec, "restricted": !row.audience.is_public() }));
    }
    let row: Option<(i64, Option<String>)> = conn.query_row("SELECT restricted, tier FROM heads WHERE entry=?1", [entry], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let (restricted, tier) = row.map(|(r, t)| (r != 0, t)).unwrap_or((false, None));
    let spec = match (restricted, tier.as_deref()) {
        (false, _) => AudienceSpec::Public,
        (true, Some(t)) if t != "public" => tier_spec(t),
        (true, _) => AudienceSpec::Friends,
    };
    Ok(json!({ "spec": spec, "restricted": restricted }))
}

pub fn readable(conn: &Connection, ctx: &ViewCtx, obj_id: &str) -> HsResult<bool> {
    if ctx.is_owner() {
        return Ok(get_object(conn, obj_id)?.is_some());
    }
    crate::stream::readable_by_grants(conn, obj_id, &ctx.reader, &ctx.rel, ctx.collector)
}

fn withdrawn(conn: &Connection, feed: &FeedObject) -> HsResult<bool> {
    Ok(match &feed.entry {
        Some(e) => get_head(conn, e)?.is_some_and(|h| h.state == HeadState::Withdrawn),
        None => false,
    })
}

pub fn item_view(conn: &Connection, ctx: &ViewCtx, obj_id: &str, depth: u8) -> HsResult<Option<Value>> {
    let obj_id = normalize_obj_id(obj_id);
    let Some(stored) = get_object(conn, &obj_id)? else { return Ok(None) };
    let Ok(feed) = serde_json::from_value::<FeedObject>(stored.body.clone()) else { return Ok(None) };
    let content_type = match feed.comment_type {
        Some(CommentType::Like) | Some(CommentType::Bookmark) | Some(CommentType::Dislike) => "reaction".to_string(),
        Some(_) => "comment".to_string(),
        None => feed.content.as_ref().map_or("text", |c| c.content_type.as_str()).to_string(),
    };
    let target = match feed.comment_type {
        Some(_) => feed.comment_target(),
        None => feed.wraps.as_deref().map(normalize_obj_id).filter(|w| obj_type_of(w).as_deref() == Some(OBJ_TYPE_FEED)),
    };
    let embedded = match &target {
        Some(target) => {
            let relation = match feed.comment_type {
                Some(CommentType::Repost) => "repost",
                Some(CommentType::Quote) => "quote",
                Some(_) => "comment_on",
                None => "wraps",
            };
            let inner = get_feed(conn, target)?;
            let unavailable: Option<String> = conn
                .query_row("SELECT resources FROM candidates WHERE obj_id=?1", [&obj_id], |r| r.get(0))
                .optional()?;
            let view = match inner {
                None if unavailable.as_deref() == Some("unavailable") => json!({ "relation": relation, "visibility": "not_visible" }),
                None => json!({ "relation": relation, "visibility": "missing" }),
                Some(_) if !readable(conn, ctx, target)? => json!({ "relation": relation, "visibility": "not_visible" }),
                Some(inner) if withdrawn(conn, &inner)? => json!({ "relation": relation, "visibility": "withdrawn" }),
                Some(_) => {
                    let item = if depth < 1 { item_view(conn, ctx, target, depth + 1)? } else { None };
                    json!({ "relation": relation, "visibility": "visible", "item": item })
                }
            };
            Some(view)
        }
        None => None,
    };
    let resolve = |part: &MediaPart| -> HsResult<Value> {
        let id = normalize_obj_id(&part.object);
        Ok(json!({ "object": id, "alt": part.alt, "file": local_file(conn, &id)? }))
    };
    let media = feed.content.as_ref().map(|c| c.media.iter().map(resolve).collect::<HsResult<Vec<_>>>()).transpose()?.unwrap_or_default();
    let cover = match feed.content.as_ref().and_then(|c| c.cover.clone()) {
        Some(cover) => Some(resolve(&MediaPart { object: cover, alt: None })?),
        None => None,
    };
    let wrapped_file = match &feed.wraps {
        Some(w) if obj_type_of(w).as_deref() == Some(OBJ_TYPE_FILE) => local_file(conn, &normalize_obj_id(w))?,
        _ => None,
    };
    let verification = if feed.source.is_some() && !stored.local_only {
        "wrapper_only"
    } else if stored.verified {
        "verified"
    } else {
        "unverified"
    };
    let host = feed.source.as_ref().and_then(|s| url::Url::parse(&s.original_url).ok()).and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()));
    Ok(Some(json!({
        "objId": obj_id,
        "object": feed,
        "contentType": content_type,
        "publisher": identity_view(ctx, &feed.publisher),
        "originalAuthor": feed.source.as_ref().and_then(|s| s.original_author.clone()),
        "capturedFrom": host,
        "isCapture": feed.source.is_some(),
        "isPrivateCapture": stored.local_only && feed.entry.is_none(),
        "verification": verification,
        "entry": entry_state(conn, &obj_id, &feed)?,
        "audience": audience_view(conn, ctx, &feed)?,
        "media": media,
        "cover": cover,
        "wrappedFile": wrapped_file,
        "embedded": embedded,
        "category": feed.publication_category,
        "createdAt": feed.iat * 1000,
        "isOwn": feed.publisher == ctx.owner,
    })))
}

fn delivery_state(conn: &Connection, entry: &str) -> HsResult<&'static str> {
    let (total, pending, failed, accepted): (i64, i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(CASE WHEN state='pending' THEN 1 END), COUNT(CASE WHEN state IN ('failed','rejected') THEN 1 END),
                COUNT(CASE WHEN state='accepted' THEN 1 END) FROM outbox WHERE entry=?1",
        [entry],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    Ok(if total == 0 {
        "none"
    } else if pending > 0 {
        "delivering"
    } else if failed > 0 && accepted < total {
        "partially_failed"
    } else {
        "delivered"
    })
}

fn flag(conn: &Connection, entry: Option<String>, default_visibility: &str) -> HsResult<Value> {
    let Some(entry) = entry else { return Ok(json!({ "on": false, "visibility": default_visibility, "delivery": "none" })) };
    let head = get_head(conn, &entry)?;
    let row = get_entry(conn, &entry)?;
    let on = head.as_ref().is_some_and(|h| h.state == HeadState::Active);
    let visibility = match &row {
        Some(r) if !r.audience.is_public() => "author_only",
        Some(_) => "public",
        None => default_visibility,
    };
    Ok(json!({
        "on": on,
        "visibility": visibility,
        "delivery": delivery_state(conn, &entry)?,
        "entry": entry,
        "seq": head.map(|h| h.seq),
    }))
}

pub fn personal_state(conn: &Connection, obj_id: &str) -> HsResult<Value> {
    let row: Option<(Option<String>, Option<String>, i64, Option<String>, i64, i64)> = conn
        .query_row(
            "SELECT like_entry, repost_entry, bookmark_on, bookmark_entry, read_later, dislike FROM personal WHERE obj_id=?1",
            [obj_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let (like, repost, bookmark_on, bookmark_entry, read_later, dislike) = row.unwrap_or((None, None, 0, None, 0, 0));
    let mut bookmark = flag(conn, bookmark_entry.clone(), "private")?;
    let public_on = bookmark["on"].as_bool().unwrap_or(false);
    bookmark["on"] = json!(bookmark_on != 0);
    if !public_on {
        bookmark["visibility"] = json!("private");
        if bookmark_entry.is_none() {
            bookmark["delivery"] = json!("none");
        }
    }
    Ok(json!({
        "like": flag(conn, like, "public")?,
        "bookmark": bookmark,
        "repost": flag(conn, repost, "public")?,
        "readLater": read_later != 0,
        "dislike": dislike != 0,
    }))
}

/// Why a card cannot be reposted (§4.5, §12.5).
pub fn repost_block(conn: &Connection, ctx: &ViewCtx, item: &Value) -> HsResult<Option<&'static str>> {
    let _ = (conn, ctx);
    if item["contentType"] == "reaction" {
        return Ok(Some("reaction"));
    }
    if item["isPrivateCapture"].as_bool().unwrap_or(false) {
        return Ok(Some("private_capture"));
    }
    if item["entry"]["state"] == "withdrawn" {
        return Ok(Some("withdrawn"));
    }
    if item["audience"]["restricted"].as_bool().unwrap_or(false) {
        return Ok(Some("restricted"));
    }
    if item["embedded"]["visibility"] == "not_visible" {
        return Ok(Some("not_visible"));
    }
    Ok(None)
}

pub fn resources_of(conn: &Connection, obj_id: &str) -> HsResult<String> {
    let reading: Option<String> = conn.query_row("SELECT resources FROM reading WHERE obj_id=?1", [obj_id], |r| r.get(0)).optional()?;
    if let Some(r) = reading {
        return Ok(r);
    }
    let candidate: Option<String> = conn.query_row("SELECT resources FROM candidates WHERE obj_id=?1", [obj_id], |r| r.get(0)).optional()?;
    Ok(match candidate.as_deref() {
        Some("unknown") | None => "local".into(),
        Some(r) => r.into(),
    })
}

pub fn card_view(conn: &Connection, ctx: &ViewCtx, obj_id: &str, reading: Option<Value>) -> HsResult<Option<Value>> {
    let obj_id = normalize_obj_id(obj_id);
    if !readable(conn, ctx, &obj_id)? {
        return Ok(None);
    }
    let Some(item) = item_view(conn, ctx, &obj_id, 0)? else { return Ok(None) };
    let block = repost_block(conn, ctx, &item)?;
    let shared_as: Option<String> = conn.query_row("SELECT shared_id FROM capture_shares WHERE capture_id=?1", [&obj_id], |r| r.get(0)).optional()?;
    let stats = crate::comments::stats(conn, ctx, &obj_id, "local")?;
    Ok(Some(without_nulls(json!({
        "item": item,
        "reading": if ctx.is_owner() { reading } else { None },
        "personal": if ctx.is_owner() { Some(personal_state(conn, &obj_id)?) } else { None },
        "stats": stats,
        "resources": resources_of(conn, &obj_id)?,
        "canRepost": block.is_none(),
        "repostBlockedReason": block,
        "sharedAs": shared_as,
    }))))
}

impl Station {
    pub async fn card(&self, obj_id: &str, reader: Reader) -> HsResult<Option<Value>> {
        let ctx = self.view_ctx(reader).await;
        let reading = if ctx.is_owner() { self.reading_entry(&normalize_obj_id(obj_id)).await? } else { None };
        let id = obj_id.to_string();
        self.db.call(move |c| card_view(c, &ctx, &id, reading)).await
    }

    pub async fn cards(&self, obj_ids: Vec<String>, reader: Reader) -> HsResult<Vec<Value>> {
        let mut out = Vec::new();
        for id in obj_ids {
            if let Some(card) = self.card(&id, reader.clone()).await? {
                out.push(card);
            }
        }
        Ok(out)
    }
}
