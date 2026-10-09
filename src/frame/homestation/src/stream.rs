//! Reading the owner's Home Feed List (§4.4): display read and change read, filtered by the
//! reader's audience (§4.5); single entry Heads, objects, chunks, author comment views and
//! profile. Items are signed Heads; list responses themselves are not signed.

use crate::audience::{allows, Reader, ReaderRelations};
use crate::error::{HsError, HsResult};
use crate::objects::{get_object, StoredObject};
use crate::protocol::*;
use crate::publish::{entry_versions, get_entry, get_head, EntryKind, EntryRow};
use crate::{now_ms, Station};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamItem {
    pub entry: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Unsigned side information: the entry is not public (§4.5).
    pub restricted: bool,
    pub tier: String,
    pub head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    pub iat: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayPage {
    pub items: Vec<StreamItem>,
    /// Attached objects the reader may read: ObjId → JWT or canonical JSON (§5.5 随附).
    pub objects: BTreeMap<String, String>,
    pub next: Option<String>,
    pub change_cursor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChangeItem {
    pub cursor: i64,
    pub entry: String,
    pub kind: String,
    pub entry_kind: String,
    pub seq: u64,
    pub restricted: bool,
    pub head: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesPage {
    pub changes: Vec<StreamChangeItem>,
    pub objects: BTreeMap<String, String>,
    pub next_cursor: i64,
    pub more: bool,
    /// The cursor predates compaction: re-sync with a display read (§4.4).
    pub resync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentRecord {
    pub comment: String,
    pub comment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub target: String,
    pub comment_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentViewPage {
    pub maintainer: String,
    pub view: String,
    pub target: String,
    pub records: Vec<CommentRecord>,
    pub objects: BTreeMap<String, String>,
    pub as_of: i64,
    pub complete: bool,
    /// The maintainer's own counting: only a claim, never adopted as verified (§15.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicProfile {
    pub did: String,
    pub name: String,
    pub bio: String,
    pub stream: String,
    pub inbox: String,
    pub followers: i64,
    pub following: i64,
    pub posts: i64,
    pub featured: Vec<String>,
    pub collector: bool,
}

/// Kinds a non-owner sees in the display read; reactions and follows only appear in the change read.
pub fn display_kind_matches(row: &EntryRow, kind: Option<&str>, owner: bool) -> bool {
    let k = row.kind;
    match kind.unwrap_or("all") {
        "all" => {
            if owner {
                k != EntryKind::Follow
            } else {
                matches!(k, EntryKind::Post | EntryKind::Comment | EntryKind::Repost | EntryKind::Quote)
            }
        }
        "feed" => (k == EntryKind::Post && row.category.is_none()) || matches!(k, EntryKind::Comment | EntryKind::Repost | EntryKind::Quote),
        "posts" => k == EntryKind::Post && row.category.is_none(),
        "comments" => k == EntryKind::Comment,
        "reposts" => matches!(k, EntryKind::Repost | EntryKind::Quote),
        "reactions" => matches!(k, EntryKind::Like | EntryKind::Bookmark | EntryKind::Dislike),
        "follows" => k == EntryKind::Follow,
        "work" => row.category.as_deref() == Some("work"),
        "product" => row.category.as_deref() == Some("product"),
        _ => false,
    }
}

fn wire(obj: &StoredObject) -> String {
    obj.wire().0
}

pub(crate) fn readable_by_grants(conn: &Connection, obj_id: &str, reader: &Reader, rel: &ReaderRelations, collector: bool) -> HsResult<bool> {
    let obj = get_object(conn, obj_id)?;
    if let Some(obj) = &obj {
        if obj.local_only {
            return Ok(*reader == Reader::Owner);
        }
    }
    if *reader == Reader::Owner {
        return Ok(true);
    }
    let mut stmt = conn.prepare("SELECT entry FROM object_grants WHERE obj_id=?1")?;
    let entries = stmt.query_map([obj_id], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let is_head = obj.as_ref().is_some_and(|o| o.obj_type == OBJ_TYPE_HEAD);
    for entry in entries {
        let Some(row) = get_entry(conn, &entry)? else { continue };
        if !allows(&row.audience, reader, rel) {
            continue;
        }
        if is_head {
            return Ok(true);
        }
        if let Some(head) = get_head(conn, &entry)? {
            if head.state == HeadState::Active {
                return Ok(true);
            }
        }
    }
    if collector {
        let in_index = conn
            .query_row(
                "SELECT 1 FROM collector_index WHERE obj_id=?1
                 UNION SELECT 1 FROM collector_index ci JOIN feed_index f ON f.obj_id=ci.obj_id
                   JOIN heads h ON h.entry=f.entry WHERE h.head_obj_id=?1",
                [obj_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if in_index {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Attach an object and its parts when the reader may read them.
fn attach(conn: &Connection, objects: &mut BTreeMap<String, String>, obj_id: &str, reader: &Reader, rel: &ReaderRelations, collector: bool, depth: u8) -> HsResult<()> {
    let obj_id = normalize_obj_id(obj_id);
    if objects.contains_key(&obj_id) || !readable_by_grants(conn, &obj_id, reader, rel, collector)? {
        return Ok(());
    }
    let Some(obj) = get_object(conn, &obj_id)? else { return Ok(()) };
    objects.insert(obj_id.clone(), wire(&obj));
    if obj.obj_type == OBJ_TYPE_FEED && depth < 2 {
        if let Ok(feed) = serde_json::from_value::<FeedObject>(obj.body.clone()) {
            for part in feed.content_parts() {
                attach(conn, objects, &part, reader, rel, collector, depth + 1)?;
            }
            if let Some(entry) = &feed.entry {
                if feed.publisher != obj.publisher.clone().unwrap_or_default() {
                    return Ok(());
                }
                if depth > 0 {
                    if let Some(head) = get_head(conn, entry)? {
                        attach(conn, objects, &head.head_obj_id, reader, rel, collector, depth + 1)?;
                    }
                }
            }
        }
    }
    Ok(())
}

impl Station {
    pub async fn reader_relations(&self, reader: &Reader) -> ReaderRelations {
        let Reader::Did(did) = reader else { return ReaderRelations::default() };
        let contact = self.contacts.get(did).await;
        let did_s = did.clone();
        let follower = self
            .db
            .call(move |c| Ok(c.query_row("SELECT 1 FROM followers WHERE did=?1 AND state='active'", [did_s], |_| Ok(())).optional()?.is_some()))
            .await
            .unwrap_or(false);
        ReaderRelations {
            follower,
            friend: contact.as_ref().is_some_and(|c| c.friend && !c.blocked),
            groups: contact.map(|c| c.groups).unwrap_or_default(),
        }
    }

    /// Display read: current entries the reader may see, newest declared `iat` first.
    pub async fn read_display(
        &self,
        reader: &Reader,
        kind: Option<String>,
        cursor: Option<String>,
        limit: usize,
        with_objects: bool,
    ) -> HsResult<DisplayPage> {
        let rel = self.reader_relations(reader).await;
        let reader = reader.clone();
        let collector = self.cfg.collector;
        let limit = limit.clamp(1, 100);
        self.db
            .call(move |c| {
                let owner = reader == Reader::Owner;
                let mut stmt = c.prepare(
                    "SELECT p.entry, COALESCE(f.iat, p.created_at/1000) AS t FROM published p
                     JOIN heads h ON h.entry=p.entry
                     LEFT JOIN feed_index f ON f.obj_id=h.current
                     ORDER BY t DESC, p.entry DESC",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                let after = cursor.as_deref().and_then(|c| c.split_once('|')).map(|(t, e)| (t.parse::<i64>().unwrap_or(i64::MAX), e.to_string()));
                let mut items = Vec::new();
                let mut objects = BTreeMap::new();
                let mut next = None;
                for (entry, t) in rows {
                    if let Some((at, ae)) = &after {
                        if (t, entry.as_str()) >= (*at, ae.as_str()) {
                            continue;
                        }
                    }
                    let Some(row) = get_entry(c, &entry)? else { continue };
                    if !display_kind_matches(&row, kind.as_deref(), owner) || !allows(&row.audience, &reader, &rel) {
                        continue;
                    }
                    let Some(head) = get_head(c, &entry)? else { continue };
                    if !owner && head.state == HeadState::Withdrawn {
                        continue;
                    }
                    if items.len() == limit {
                        next = items.last().map(|i: &StreamItem| format!("{}|{}", i.iat, i.entry));
                        break;
                    }
                    let Some(head_obj) = get_object(c, &head.head_obj_id)? else { continue };
                    if with_objects {
                        if let Some(current) = &head.current {
                            attach(c, &mut objects, current, &reader, &rel, collector, 0)?;
                        }
                    }
                    items.push(StreamItem {
                        entry: entry.clone(),
                        kind: row.kind.as_str().into(),
                        category: row.category.clone(),
                        restricted: !row.audience.is_public(),
                        tier: row.audience.tier().into(),
                        head: wire(&head_obj),
                        current: head.current.clone(),
                        iat: t.max(0) as u64,
                    });
                }
                let change_cursor: i64 = c.query_row("SELECT COALESCE(MAX(cursor), 0) FROM stream_changes", [], |r| r.get(0))?;
                Ok(DisplayPage { items, objects, next, change_cursor })
            })
            .await
    }

    /// Change read: Heads changed after `since` (new entries, versions, withdrawals, newly
    /// visible entries). Omissions never mean withdrawal (A68).
    pub async fn read_changes(&self, reader: &Reader, since: i64, limit: usize, with_objects: bool) -> HsResult<ChangesPage> {
        let rel = self.reader_relations(reader).await;
        let reader = reader.clone();
        let collector = self.cfg.collector;
        let limit = limit.clamp(1, 500) as i64;
        self.db
            .call(move |c| {
                let compacted: i64 = crate::db::get_meta(c, "stream_compacted_through")?.and_then(|v| v.parse().ok()).unwrap_or(0);
                let max_cursor: i64 = c.query_row("SELECT COALESCE(MAX(cursor), 0) FROM stream_changes", [], |r| r.get(0))?;
                if since < compacted {
                    return Ok(ChangesPage { changes: vec![], objects: BTreeMap::new(), next_cursor: max_cursor, more: false, resync: true });
                }
                let mut stmt = c.prepare("SELECT cursor, entry, kind, seq FROM stream_changes WHERE cursor>?1 ORDER BY cursor LIMIT ?2")?;
                let rows = stmt
                    .query_map(params![since, limit + 1], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                let more = rows.len() as i64 > limit;
                let mut changes = Vec::new();
                let mut objects = BTreeMap::new();
                let mut next_cursor = since;
                for (cursor, entry, kind, seq) in rows.into_iter().take(limit as usize) {
                    next_cursor = cursor;
                    let Some(row) = get_entry(c, &entry)? else { continue };
                    if !allows(&row.audience, &reader, &rel) {
                        continue;
                    }
                    let head_obj_id: Option<String> = if kind == "head" {
                        c.query_row("SELECT head_obj_id FROM head_history WHERE entry=?1 AND seq=?2", params![entry, seq], |r| r.get(0)).optional()?
                    } else {
                        get_head(c, &entry)?.map(|h| h.head_obj_id)
                    };
                    let Some(head_obj_id) = head_obj_id else { continue };
                    let Some(head_obj) = get_object(c, &head_obj_id)? else { continue };
                    if with_objects {
                        if let Some(current) = head_obj.body.get("current").and_then(Value::as_str) {
                            attach(c, &mut objects, current, &reader, &rel, collector, 0)?;
                        }
                    }
                    changes.push(StreamChangeItem {
                        cursor,
                        entry: entry.clone(),
                        kind,
                        entry_kind: row.kind.as_str().into(),
                        seq: seq as u64,
                        restricted: !row.audience.is_public(),
                        head: wire(&head_obj),
                    });
                }
                Ok(ChangesPage { changes, objects, next_cursor, more, resync: false })
            })
            .await
    }

    /// Drop the change log up to `through`; readers behind it must re-sync (A67).
    pub async fn compact_stream(&self, through: i64) -> HsResult<()> {
        self.db
            .call(move |c| {
                c.execute("DELETE FROM stream_changes WHERE cursor<=?1", [through])?;
                crate::db::set_meta(c, "stream_compacted_through", &through.to_string())
            })
            .await
    }

    /// `GET /home/<ns>/@/<key>`: the current Head of an entry, same answer for "never existed"
    /// and "not visible to you" (E11, A32).
    pub async fn read_entry_head(&self, reader: &Reader, entry: &str) -> HsResult<Option<(String, bool)>> {
        let rel = self.reader_relations(reader).await;
        let reader = reader.clone();
        let entry = entry.to_string();
        self.db
            .call(move |c| {
                let Some(row) = get_entry(c, &entry)? else { return Ok(None) };
                if !allows(&row.audience, &reader, &rel) {
                    return Ok(None);
                }
                let Some(head) = get_head(c, &entry)? else { return Ok(None) };
                Ok(get_object(c, &head.head_obj_id)?.map(|o| (wire(&o), !row.audience.is_public())))
            })
            .await
    }

    pub async fn read_object(&self, reader: &Reader, obj_id: &str) -> HsResult<Option<StoredObject>> {
        let rel = self.reader_relations(reader).await;
        let reader = reader.clone();
        let collector = self.cfg.collector;
        let obj_id = normalize_obj_id(obj_id);
        let found = self
            .db
            .call(move |c| {
                if !readable_by_grants(c, &obj_id, &reader, &rel, collector)? {
                    return Ok(None);
                }
                get_object(c, &obj_id)
            })
            .await?;
        Ok(found)
    }

    pub async fn object_readable(&self, reader: &Reader, obj_id: &str) -> HsResult<bool> {
        let rel = self.reader_relations(reader).await;
        let reader = reader.clone();
        let collector = self.cfg.collector;
        let obj_id = normalize_obj_id(obj_id);
        self.db.call(move |c| readable_by_grants(c, &obj_id, &reader, &rel, collector)).await
    }

    pub async fn read_chunk(&self, reader: &Reader, chunk_id: &str) -> HsResult<Option<Vec<u8>>> {
        if !self.object_readable(reader, chunk_id).await? {
            return Ok(None);
        }
        self.chunks.get_chunk(&normalize_obj_id(chunk_id)).await
    }

    /// Author view (§15.1) of comments on one of the owner's versions, or — on a collector —
    /// the collector view (§15.2). Readable by whoever may read the target.
    pub async fn read_comment_view(&self, reader: &Reader, target: &str, comment_type: Option<String>) -> HsResult<Option<CommentViewPage>> {
        let target = normalize_obj_id(target);
        let rel = self.reader_relations(reader).await;
        let reader = reader.clone();
        let collector = self.cfg.collector;
        let owner = self.cfg.owner.clone();
        let target_readable = self.object_readable(&reader, &target).await?;
        self.db
            .call(move |c| {
                let own: Option<String> = c
                    .query_row(
                        "SELECT v.entry FROM entry_versions v JOIN published p ON p.entry=v.entry WHERE v.obj_id=?1",
                        [&target],
                        |r| r.get(0),
                    )
                    .optional()?;
                let (versions, view) = match &own {
                    Some(entry) => {
                        if !target_readable {
                            return Ok(None);
                        }
                        (entry_versions(c, entry)?, "author")
                    }
                    None if collector => (vec![target.clone()], "collector"),
                    None => return Ok(None),
                };
                let mut records = Vec::new();
                let mut objects = BTreeMap::new();
                let mut seen = std::collections::HashSet::new();
                for version in &versions {
                    let ids: Vec<String> = if view == "author" {
                        let mut stmt = c.prepare("SELECT comment_id FROM author_list WHERE target=?1 AND listed=1 ORDER BY position")?;
                        let rows = stmt.query_map([version], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
                        rows
                    } else {
                        let mut stmt = c.prepare(
                            "SELECT f.obj_id FROM feed_index f JOIN collector_index ci ON ci.obj_id=f.obj_id WHERE f.target=?1 ORDER BY ci.received_at",
                        )?;
                        let rows = stmt.query_map([version], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
                        rows
                    };
                    for id in ids {
                        if !seen.insert(id.clone()) {
                            continue;
                        }
                        let Some(obj) = get_object(c, &id)? else { continue };
                        if obj.local_only {
                            continue;
                        }
                        let Ok(feed) = serde_json::from_value::<FeedObject>(obj.body.clone()) else { continue };
                        let Some(ct) = feed.comment_type else { continue };
                        if let Some(filter) = &comment_type {
                            if ct.as_str() != filter {
                                continue;
                            }
                        }
                        let head = match &feed.entry {
                            Some(entry) => match get_head(c, entry)? {
                                Some(h) => get_object(c, &h.head_obj_id)?.map(|o| wire(&o)),
                                None => None,
                            },
                            None => None,
                        };
                        if let Some(wrapped) = &feed.wraps {
                            if obj.obj_type == OBJ_TYPE_FEED && readable_by_grants(c, wrapped, &reader, &rel, collector)? {
                                if let Some(w) = get_object(c, wrapped)? {
                                    objects.insert(normalize_obj_id(wrapped), wire(&w));
                                }
                            }
                        }
                        records.push(CommentRecord {
                            comment: wire(&obj),
                            comment_id: id.clone(),
                            head,
                            target: version.clone(),
                            comment_type: ct.as_str().into(),
                        });
                    }
                }
                let claimed = if view == "author" {
                    let likes = records.iter().filter(|r| r.comment_type == "like").count();
                    Some(serde_json::json!({ "likes": likes, "source": owner }))
                } else {
                    None
                };
                Ok(Some(CommentViewPage {
                    maintainer: owner,
                    view: view.into(),
                    target,
                    records,
                    objects,
                    as_of: now_ms(),
                    complete: true,
                    claimed,
                }))
            })
            .await
    }

    pub async fn public_profile(&self, reader: &Reader) -> HsResult<PublicProfile> {
        let settings = self.settings().await?;
        let name = if settings.profile.name.is_empty() { self.cfg.owner_name.clone() } else { settings.profile.name.clone() };
        let posts = self.read_display(reader, Some("feed".into()), None, 100, false).await?.items.len() as i64;
        let mut featured = Vec::new();
        for id in &settings.profile.featured {
            if self.object_readable(reader, id).await? {
                featured.push(id.clone());
            }
        }
        let (followers, following) = self
            .db
            .call(|c| {
                let followers: i64 = c.query_row("SELECT COUNT(*) FROM followers WHERE state='active'", [], |r| r.get(0))?;
                let following: i64 =
                    c.query_row("SELECT COUNT(*) FROM sources WHERE kind='person' AND basis!='[]'", [], |r| r.get(0))?;
                Ok((followers, following))
            })
            .await?;
        Ok(PublicProfile {
            did: self.cfg.owner.clone(),
            name,
            bio: settings.profile.bio,
            stream: stream_url(&self.cfg.zone),
            inbox: inbox_target(&self.cfg.zone),
            followers,
            following,
            posts,
            featured,
            collector: self.cfg.collector,
        })
    }

    pub fn entry_for_path(&self, namespace: &str, key: &str) -> HsResult<String> {
        let ns = EntryNamespace::parse(namespace).ok_or_else(|| HsError::NotFound("no such namespace".into()))?;
        if !valid_entry_key(key) {
            return Err(HsError::NotFound("no such entry".into()));
        }
        Ok(entry_url(&self.cfg.zone, ns, key))
    }
}

impl Station {
    /// Collector index (§14.2): recent public posts submitted here, for cold start (A18).
    pub async fn read_collector_recent(&self, since: i64, limit: usize) -> HsResult<DisplayPage> {
        if !self.cfg.collector {
            return Err(HsError::NotFound("not a collector".into()));
        }
        let limit = limit.clamp(1, 100) as i64;
        self.db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT ci.obj_id, ci.received_at, f.entry, f.kind, f.iat FROM collector_index ci
                     JOIN feed_index f ON f.obj_id=ci.obj_id
                     WHERE ci.received_at>?1 AND f.kind='post' ORDER BY ci.received_at LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![since, limit], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut items = Vec::new();
                let mut objects = BTreeMap::new();
                let mut cursor = since;
                for (obj_id, at, entry, kind, iat) in rows {
                    cursor = at;
                    let Some(entry) = entry else { continue };
                    let Some(head) = get_head(c, &entry)? else { continue };
                    if head.state != HeadState::Active || head.current.as_deref() != Some(obj_id.as_str()) || head.restricted {
                        continue;
                    }
                    let Some(head_obj) = get_object(c, &head.head_obj_id)? else { continue };
                    let Some(obj) = get_object(c, &obj_id)? else { continue };
                    objects.insert(obj_id.clone(), wire(&obj));
                    if let Ok(feed) = serde_json::from_value::<FeedObject>(obj.body.clone()) {
                        for part in feed.content_parts() {
                            if let Some(p) = get_object(c, &part)? {
                                if p.obj_type == OBJ_TYPE_FILE {
                                    objects.insert(part.clone(), wire(&p));
                                }
                            }
                        }
                    }
                    items.push(StreamItem {
                        entry,
                        kind,
                        category: None,
                        restricted: false,
                        tier: "public".into(),
                        head: wire(&head_obj),
                        current: Some(obj_id),
                        iat: iat.max(0) as u64,
                    });
                }
                Ok(DisplayPage { items, objects, next: None, change_cursor: cursor })
            })
            .await
    }
}
