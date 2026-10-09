//! Resource Preparation (§8.3): make a candidate's referenced objects and data available by
//! ObjId from any holder, checked by hash. "Local" and "depends on a reachable source" stay
//! distinct (§8.3, A06).

use crate::error::{HsError, HsResult};
use crate::ingress::Arrival;
use crate::objects::{get_object, has_object};
use crate::protocol::*;
use crate::{now_ms, Station};
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

impl Station {
    /// Holders worth asking for objects referenced by `obj_id`: its publisher, then whoever
    /// pushed it or whose stream carried it.
    async fn holders_of(&self, obj_id: &str) -> HsResult<Vec<String>> {
        let id = obj_id.to_string();
        let (publisher, paths) = self
            .db
            .call(move |c| {
                let publisher: Option<String> = c
                    .query_row("SELECT publisher FROM feed_index WHERE obj_id=?1", [&id], |r| r.get(0))
                    .ok();
                let paths: Option<String> = c.query_row("SELECT source_paths FROM candidates WHERE obj_id=?1", [&id], |r| r.get(0)).ok();
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
        if crate::objects::load_local(&self.db, self.chunks.as_ref(), obj_id).await?.is_some() {
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

    async fn prepare_file(&self, file_id: &str, holders: &[String]) -> HsResult<ResourceState> {
        if !self.ensure_object(file_id, holders).await? {
            return Ok(ResourceState::Unavailable);
        }
        let id = file_id.to_string();
        let Some(file) = self.db.call(move |c| get_object(c, &id)).await? else { return Ok(ResourceState::Unavailable) };
        let Some(content) = file.body.get("content").and_then(Value::as_str).map(str::to_string) else { return Ok(ResourceState::Local) };
        if content.is_empty() {
            return Ok(ResourceState::Local);
        }
        if !crate::objects::is_chunk_id(&content) {
            return Ok(ResourceState::Reachable);
        }
        if self.chunks.has_chunk(&content).await {
            return Ok(ResourceState::Local);
        }
        let size = file.body.get("size").and_then(Value::as_u64).unwrap_or(0);
        if !self.cfg.prefetch || size > self.cfg.max_prefetch_bytes {
            return Ok(ResourceState::Reachable);
        }
        for holder in holders {
            match self.fetch_chunk_from(holder, &content, self.cfg.max_prefetch_bytes).await {
                Ok(data) => {
                    self.chunks.put_chunk(&content, data).await?;
                    return Ok(ResourceState::Local);
                }
                Err(e) => log::debug!("chunk {content} not available from {holder}: {e}"),
            }
        }
        Ok(ResourceState::Unavailable)
    }

    /// Prepare everything a Feed Object fixes as content (wrapped object, media, cover).
    pub async fn prepare_resources(&self, obj_id: &str) -> HsResult<ResourceState> {
        let id = obj_id.to_string();
        let Some(feed) = self.db.call(move |c| crate::objects::get_feed(c, &id)).await? else { return Ok(ResourceState::Unavailable) };
        let holders = self.holders_of(obj_id).await?;
        let mut state = ResourceState::Local;
        if let Some(wrapped) = &feed.wraps {
            let wrapped = normalize_obj_id(wrapped);
            if obj_type_of(&wrapped).as_deref() == Some(OBJ_TYPE_FEED) {
                if !self.ensure_object(&wrapped, &holders).await? {
                    return Ok(ResourceState::Unavailable);
                }
                let inner_state = Box::pin(self.prepare_inner(&wrapped, &holders)).await?;
                state = state.worst(inner_state);
            } else {
                state = state.worst(self.prepare_file(&wrapped, &holders).await?);
            }
        }
        if let Some(content) = &feed.content {
            for media in &content.media {
                state = state.worst(self.prepare_file(&normalize_obj_id(&media.object), &holders).await?);
            }
            if let Some(cover) = &content.cover {
                let cover_state = self.prepare_file(&normalize_obj_id(cover), &holders).await?;
                if cover_state == ResourceState::Unavailable && state == ResourceState::Local {
                    state = ResourceState::Reachable;
                }
            }
        }
        Ok(state)
    }

    /// A wrapped Feed Object (repost): its own parts, from its publisher or the wrapper's holders.
    async fn prepare_inner(&self, inner_id: &str, outer_holders: &[String]) -> HsResult<ResourceState> {
        let id = inner_id.to_string();
        let Some(inner) = self.db.call(move |c| crate::objects::get_feed(c, &id)).await? else { return Ok(ResourceState::Unavailable) };
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
            if obj_type_of(&part).as_deref() == Some(OBJ_TYPE_FILE) {
                state = state.worst(self.prepare_file(&part, &holders).await?);
            }
        }
        Ok(state)
    }

    /// Content of a file the owner is looking at but that was not prepared yet (e.g. a followed
    /// candidate): fetch the chunk from a publisher of an object referencing it, check, keep.
    pub async fn fetch_content_on_demand(&self, file_id: &str) -> HsResult<Option<Vec<u8>>> {
        let id = normalize_obj_id(file_id);
        let id2 = id.clone();
        let (file, referencing): (Option<crate::objects::StoredObject>, Vec<String>) = self
            .db
            .call(move |c| {
                let file = get_object(c, &id2)?;
                let mut stmt = c.prepare("SELECT obj_id FROM objects WHERE obj_type=?1 AND instr(body, ?2)>0 LIMIT 8")?;
                let rows = stmt.query_map(rusqlite::params![OBJ_TYPE_FEED, id2], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok((file, rows))
            })
            .await?;
        let Some(content) = file.and_then(|f| f.body.get("content").and_then(Value::as_str).map(str::to_string)) else { return Ok(None) };
        if !crate::objects::is_chunk_id(&content) {
            return Ok(None);
        }
        let mut holders = Vec::new();
        for feed in referencing {
            for h in self.holders_of(&feed).await? {
                if !holders.contains(&h) {
                    holders.push(h);
                }
            }
        }
        for holder in holders {
            if let Ok(data) = self.fetch_chunk_from(&holder, &content, self.cfg.max_prefetch_bytes).await {
                self.chunks.put_chunk(&content, data.clone()).await?;
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    pub async fn set_resource_state(&self, obj_id: &str, state: ResourceState) -> HsResult<()> {
        let id = obj_id.to_string();
        self.db
            .call(move |c| {
                c.execute("UPDATE reading SET resources=?2 WHERE obj_id=?1", params![id, state.as_str()])?;
                c.execute("UPDATE candidates SET resources=?2, updated_at=?3 WHERE obj_id=?1", params![id, state.as_str(), now_ms()])?;
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
        let file = crate::objects::load_local(&self.db, self.chunks.as_ref(), &wrapped).await?;
        match (self.text_of(&wrapped).await?, file) {
            (Some(text), Some(file)) => Ok(serde_json::json!({ "state": "ready", "markdown": text, "file": crate::projection::file_view(&file.body) })),
            (None, Some(_)) => Ok(serde_json::json!({ "state": "unavailable", "reason": "not_text" })),
            _ => Ok(serde_json::json!({ "state": "unavailable", "reason": "no_known_source" })),
        }
    }
}
