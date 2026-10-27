//! Resource Preparation (§8.3): make a candidate's referenced objects and data available by
//! ObjId from any holder, checked by hash. "Local" and "depends on a reachable source" stay
//! distinct (§8.3, A06).

use crate::error::{HsError, HsResult};
use crate::ingress::Arrival;
use crate::objects::{bucket_err, canonical, get_object, has_object};
use crate::protocol::*;
use crate::{now_ms, Station};
use buckyos_api::bucket_client::Origin;
use buckyos_api::{HOMESTATION_BUCKET_FOLLOWING, HOMESTATION_BUCKET_FOLLOWING_ENTRIES};
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    Local,
    Reachable,
    Preparing,
    Unavailable,
}

impl ResourceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Reachable => "reachable",
            Self::Preparing => "preparing",
            Self::Unavailable => "unavailable",
        }
    }
    pub fn parse(v: &str) -> Self {
        match v {
            "local" => Self::Local,
            "reachable" => Self::Reachable,
            "unavailable" => Self::Unavailable,
            _ => Self::Preparing,
        }
    }
    fn worst(self, other: Self) -> Self {
        let rank = |s: Self| match s {
            Self::Local => 0,
            Self::Reachable => 1,
            Self::Preparing => 2,
            Self::Unavailable => 3,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

/// Where prepared content of another publisher's object is kept: its key in
/// `following-entries` (its files go under `<key>/<file id>` in `following`) and its origin.
pub(crate) struct Following {
    key: String,
    origin: Origin,
}

impl Station {
    /// Holders worth asking for objects referenced by `obj_id`: its publisher, then whoever
    /// pushed it or whose stream carried it.
    async fn holders_of(&self, obj_id: &str) -> HsResult<Vec<String>> {
        let id = obj_id.to_string();
        let (publisher, paths) = self
            .db
            .call(move |c| {
                let publisher: Option<String> = c
                    .query_row("SELECT publisher FROM feed_index WHERE obj_id=?1 AND owner=?2", [&id, c.owner], |r| r.get(0))
                    .ok();
                let paths: Option<String> =
                    c.query_row("SELECT source_paths FROM candidates WHERE obj_id=?1 AND owner=?2", [&id, c.owner], |r| r.get(0)).ok();
                Ok((publisher, paths))
            })
            .await?;
        let mut holders = Vec::new();
        if let Some(p) = publisher {
            holders.push(p);
        }
        for path in serde_json::from_str::<Vec<Value>>(&paths.unwrap_or_default()).unwrap_or_default() {
            for key in ["sender", "collector"] {
                if let Some(d) = path.get(key).and_then(Value::as_str) {
                    holders.push(d.to_string());
                }
            }
        }
        holders.retain(|h| h != &self.cfg.owner);
        holders.dedup();
        Ok(holders)
    }

    async fn ensure_object(&self, obj_id: &str, holders: &[String]) -> HsResult<bool> {
        let id = obj_id.to_string();
        if self.db.call(move |c| has_object(c, &id)).await? {
            return Ok(true);
        }
        if crate::objects::load_local(&self.db, &self.store, obj_id).await?.is_some() {
            return Ok(true);
        }
        for holder in holders {
            match self.fetch_object_from(holder, obj_id, Arrival::Fetch).await {
                Ok(()) => return Ok(true),
                Err(e) => log::debug!("{obj_id} not available from {holder}: {e}"),
            }
        }
        Ok(false)
    }

    /// Fetches what a file of another publisher's post needs into the zone named store and
    /// binds it in `following` under the post (ambient: the descriptor is kept, the bytes are a
    /// cache). Without `following` (the owner's own objects) nothing is fetched into the store:
    /// what is not local stays reachable by ObjId.
    async fn prepare_file(&self, file_id: &str, holders: &[String], following: Option<&Following>) -> HsResult<ResourceState> {
        let mut pending = vec![file_id.to_string()];
        let mut seen = std::collections::HashSet::new();
        let mut state = ResourceState::Local;
        let mut fetched_bytes = 0u64;
        while let Some(id) = pending.pop() {
            if !seen.insert(id.clone()) { continue; }
            if seen.len() > 100_000 { return Err(crate::error::bad("attachment tree is too large")); }
            if crate::objects::is_chunk_id(&id) {
                if self.store.has_chunk(&id).await { continue; }
                let size = ndn_lib::ChunkId::new(&id).ok().and_then(|chunk| chunk.get_length()).unwrap_or(self.cfg.max_prefetch_bytes);
                if following.is_none() || !self.cfg.prefetch || fetched_bytes.saturating_add(size) > self.cfg.max_prefetch_bytes {
                    state = state.worst(ResourceState::Reachable);
                    continue;
                }
                let mut stored = false;
                for holder in holders {
                    if let Ok(data) = self.fetch_chunk_from(holder, &id, self.cfg.max_prefetch_bytes).await {
                        fetched_bytes += data.len() as u64;
                        self.store.put_fetched_chunk(&id, &data).await?;
                        stored = true;
                        break;
                    }
                }
                if !stored { state = state.worst(ResourceState::Unavailable); }
                continue;
            }
            if !self.ensure_object(&id, holders).await? { return Ok(ResourceState::Unavailable); }
            let lookup = id.clone();
            let Some(object) = self.db.call(move |c| get_object(c, &lookup)).await? else { return Ok(ResourceState::Unavailable) };
            let body = canonical(&object.body);
            if following.is_some() {
                self.store.put_fetched_object(&id, &body).await?;
            }
            pending.extend(crate::objects::object_dependencies(&id, &body)?);
        }
        if let Some(following) = following {
            let key = format!("{}/{file_id}", following.key);
            let bucket = self.bucket(HOMESTATION_BUCKET_FOLLOWING)?;
            if let Err(e) = self.store.bind(&bucket, &key, file_id, None, Some(following.origin.clone())).await {
                log::warn!("prepared {file_id} not bound in {bucket}: {e}");
            }
        }
        let root = file_id.to_string();
        self.db.call(move |c| {
            let entries = c
                .prepare("SELECT entry FROM object_grants WHERE obj_id=?1 AND owner=?2")?
                .query_map([&root, c.owner], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for entry in entries { crate::publish::grant(c, &root, &entry, 0)?; }
            Ok(())
        }).await?;
        Ok(state)
    }

    /// What another publisher's Feed Object is kept under in `following-entries` and its
    /// files under in `following`; none for the owner's own objects.
    fn following(&self, feed: &FeedObject, key: String) -> Option<Following> {
        if feed.publisher == self.cfg.owner {
            return None;
        }
        let source = feed.entry.clone().filter(|e| e.starts_with("cyfs://") || e.starts_with("did:")).unwrap_or_else(|| feed.publisher.clone());
        Some(Following { key, origin: Origin { author: feed.publisher.clone(), source, collected_at: now_ms(), proof: None } })
    }

    async fn bind_following_object(&self, obj_id: &str, following: &Following) {
        let id = obj_id.to_string();
        let body = match self.db.call(move |c| get_object(c, &id)).await {
            Ok(Some(obj)) => canonical(&obj.body),
            Ok(None) => return,
            Err(e) => return log::warn!("{obj_id} not bound in following: {e}"),
        };
        let bound = match self.bucket(HOMESTATION_BUCKET_FOLLOWING_ENTRIES) {
            Ok(bucket) => self.store.put_bound_object(&bucket, &following.key, obj_id, &body, Some(following.origin.clone())).await,
            Err(e) => Err(e),
        };
        if let Err(e) = bound {
            log::warn!("{obj_id} not bound in following: {e}");
        }
    }

    /// Objects that left the reading list, expired as candidates or were unsaved stop holding
    /// what was prepared for them, unless they are still read, pending or saved.
    pub(crate) async fn release_following(&self, obj_ids: Vec<String>) {
        if obj_ids.is_empty() {
            return;
        }
        let released = self
            .db
            .call(move |c| {
                let mut out = Vec::new();
                for id in obj_ids {
                    let kept: bool = c.query_row(
                        "SELECT EXISTS(SELECT 1 FROM reading WHERE obj_id=?1 AND owner=?2)
                           OR EXISTS(SELECT 1 FROM candidates WHERE obj_id=?1 AND owner=?2 AND selection!='admitted')
                           OR EXISTS(SELECT 1 FROM personal WHERE obj_id=?1 AND owner=?2 AND (bookmark_on=1 OR read_later=1))",
                        [&id, c.owner],
                        |r| r.get(0),
                    )?;
                    if !kept {
                        out.push(id);
                    }
                }
                Ok(out)
            })
            .await;
        let released = match released {
            Ok(ids) => ids,
            Err(e) => return log::warn!("following release check failed: {e}"),
        };
        for id in released {
            if let Err(e) = self.unbind_following(&id).await {
                log::warn!("following of {id} not released: {e}");
            }
        }
    }

    async fn unbind_following(&self, obj_id: &str) -> HsResult<()> {
        let client = self.store.buckets().await?;
        for name in [HOMESTATION_BUCKET_FOLLOWING_ENTRIES, HOMESTATION_BUCKET_FOLLOWING] {
            let bucket = self.bucket(name)?;
            let nested = format!("{obj_id}/");
            for key in self.bound_keys(&bucket, obj_id).await? {
                if key == obj_id || key.starts_with(&nested) {
                    client.delete(&bucket, &key, None).await.map_err(bucket_err)?;
                }
            }
        }
        Ok(())
    }

    /// Prepare everything a Feed Object fixes as content (wrapped object, media, cover).
    pub async fn prepare_resources(&self, obj_id: &str) -> HsResult<ResourceState> {
        let id = obj_id.to_string();
        let Some(feed) = self.db.call(move |c| crate::objects::get_feed(c, &id)).await? else { return Ok(ResourceState::Unavailable) };
        let holders = self.holders_of(obj_id).await?;
        let following = self.following(&feed, normalize_obj_id(obj_id));
        if let Some(following) = &following {
            self.bind_following_object(obj_id, following).await;
        }
        let following = following.as_ref();
        let mut state = ResourceState::Local;
        if let Some(wrapped) = &feed.wraps {
            let wrapped = normalize_obj_id(wrapped);
            if obj_type_of(&wrapped).as_deref() == Some(OBJ_TYPE_FEED) {
                if !self.ensure_object(&wrapped, &holders).await? {
                    return Ok(ResourceState::Unavailable);
                }
                let inner_state = Box::pin(self.prepare_inner(&wrapped, &holders, following)).await?;
                state = state.worst(inner_state);
            } else {
                state = state.worst(self.prepare_file(&wrapped, &holders, following).await?);
            }
        }
        if let Some(content) = &feed.content {
            for media in &content.media {
                state = state.worst(self.prepare_file(&normalize_obj_id(&media.object), &holders, following).await?);
            }
            if let Some(cover) = &content.cover {
                let cover_state = self.prepare_file(&normalize_obj_id(cover), &holders, following).await?;
                if cover_state == ResourceState::Unavailable && state == ResourceState::Local {
                    state = ResourceState::Reachable;
                }
            }
        }
        Ok(state)
    }

    /// A wrapped Feed Object (repost): its own parts, from its publisher or the wrapper's holders,
    /// kept under the wrapper (`<wrapper>/<inner>` in `following-entries`).
    async fn prepare_inner(&self, inner_id: &str, outer_holders: &[String], outer: Option<&Following>) -> HsResult<ResourceState> {
        let id = inner_id.to_string();
        let Some(inner) = self.db.call(move |c| crate::objects::get_feed(c, &id)).await? else { return Ok(ResourceState::Unavailable) };
        let following = outer.and_then(|outer| self.following(&inner, format!("{}/{inner_id}", outer.key)));
        if let Some(following) = &following {
            self.bind_following_object(inner_id, following).await;
        }
        let files = outer.map(|outer| Following { key: outer.key.clone(), origin: following.as_ref().map_or_else(|| outer.origin.clone(), |f| f.origin.clone()) });
        let mut holders = vec![inner.publisher.clone()];
        holders.extend(outer_holders.iter().cloned());
        holders.retain(|h| h != &self.cfg.owner);
        holders.dedup();
        if let Some(entry) = &inner.entry {
            let entry = entry.clone();
            let known = {
                let e = entry.clone();
                self.db.call(move |c| crate::publish::get_head(c, &e)).await?.is_some()
            };
            if !known {
                if let Err(e) = self.resolve_remote_entry(&entry).await {
                    log::debug!("entry {entry} unresolved: {e}");
                }
            }
        }
        let mut state = ResourceState::Local;
        for part in inner.content_parts() {
            if matches!(obj_type_of(&part).as_deref(), Some(OBJ_TYPE_FILE | OBJ_TYPE_DIR)) {
                state = state.worst(self.prepare_file(&part, &holders, files.as_ref()).await?);
            }
        }
        Ok(state)
    }

    /// Content of a file the owner is looking at but that was not prepared yet (e.g. a followed
    /// candidate): fetch the chunk from a publisher of an object referencing it, check, keep it
    /// in `following` under that object.
    pub async fn fetch_content_on_demand(&self, file_id: &str) -> HsResult<Option<Vec<u8>>> {
        let id = normalize_obj_id(file_id);
        let id2 = id.clone();
        let (file, referencing): (Option<crate::objects::StoredObject>, Vec<(String, FeedObject)>) = self
            .db
            .call(move |c| {
                let file = get_object(c, &id2)?;
                let mut stmt = c.prepare("SELECT obj_id FROM objects WHERE obj_type=?1 AND instr(body, ?2)>0 AND owner=?3 LIMIT 8")?;
                let rows = stmt.query_map(params![OBJ_TYPE_FEED, id2, c.owner], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                let mut feeds = Vec::new();
                for row in rows {
                    if let Some(feed) = crate::objects::get_feed(c, &row)? {
                        feeds.push((row, feed));
                    }
                }
                Ok((file, feeds))
            })
            .await?;
        let Some(file) = file else { return Ok(None) };
        let Some(content) = file.body.get("content").and_then(Value::as_str).map(str::to_string) else { return Ok(None) };
        if !crate::objects::is_chunk_id(&content) {
            return Ok(None);
        }
        let Some(following) = referencing.iter().find_map(|(feed_id, feed)| self.following(feed, feed_id.clone())) else { return Ok(None) };
        let mut holders = Vec::new();
        for (feed, _) in &referencing {
            for h in self.holders_of(feed).await? {
                if !holders.contains(&h) {
                    holders.push(h);
                }
            }
        }
        for holder in holders {
            if let Ok(data) = self.fetch_chunk_from(&holder, &content, self.cfg.max_prefetch_bytes).await {
                self.store.put_fetched_chunk(&content, &data).await?;
                self.store.put_fetched_object(&id, &canonical(&file.body)).await?;
                let bucket = self.bucket(HOMESTATION_BUCKET_FOLLOWING)?;
                if let Err(e) = self.store.bind(&bucket, &format!("{}/{id}", following.key), &id, None, Some(following.origin.clone())).await {
                    log::warn!("fetched {id} not bound in {bucket}: {e}");
                }
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    pub async fn set_resource_state(&self, obj_id: &str, state: ResourceState) -> HsResult<()> {
        let id = obj_id.to_string();
        self.db
            .call(move |c| {
                c.execute("UPDATE reading SET resources=?2 WHERE obj_id=?1 AND owner=?3", params![id, state.as_str(), c.owner])?;
                c.execute(
                    "UPDATE candidates SET resources=?2, updated_at=?3 WHERE obj_id=?1 AND owner=?4",
                    params![id, state.as_str(), now_ms(), c.owner],
                )?;
                Ok(())
            })
            .await?;
        self.bump(&["reading", "candidates"]);
        Ok(())
    }

    pub async fn retry_resources(&self, obj_id: &str) -> HsResult<ResourceState> {
        let obj_id = normalize_obj_id(obj_id);
        self.set_resource_state(&obj_id, ResourceState::Preparing).await?;
        let state = self.prepare_resources(&obj_id).await.unwrap_or(ResourceState::Unavailable);
        self.set_resource_state(&obj_id, state).await?;
        Ok(state)
    }

    /// Body of a wrapped text/markdown file for the Reader (§9.4).
    pub async fn wrapped_body(&self, obj_id: &str) -> HsResult<Value> {
        let id = normalize_obj_id(obj_id);
        let id2 = id.clone();
        let feed = self.db.call(move |c| crate::objects::get_feed(c, &id2)).await?.ok_or_else(|| HsError::NotFound("no such object".into()))?;
        let Some(wrapped) = feed.wraps.as_deref().map(normalize_obj_id) else {
            return Ok(serde_json::json!({ "state": "unavailable", "reason": "not_wrapped" }));
        };
        if self.text_of(&wrapped).await?.is_none() {
            let state = self.prepare_resources(&id).await.unwrap_or(ResourceState::Unavailable);
            if state == ResourceState::Unavailable {
                return Ok(serde_json::json!({ "state": "unavailable", "reason": "no_known_source" }));
            }
        }
        let file = crate::objects::load_local(&self.db, &self.store, &wrapped).await?;
        match (self.text_of(&wrapped).await?, file) {
            (Some(text), Some(file)) => Ok(serde_json::json!({ "state": "ready", "markdown": text, "file": crate::projection::file_view(&file.body) })),
            (None, Some(_)) => Ok(serde_json::json!({ "state": "unavailable", "reason": "not_text" })),
            _ => Ok(serde_json::json!({ "state": "unavailable", "reason": "no_known_source" })),
        }
    }
}
