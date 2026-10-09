//! Pull (§4.4 "Pull 是事实来源"): read followed streams by change cursor, fetch objects and
//! chunks by ObjId from whoever may hold them, resolve remote entries.

use crate::auth::make_reader_proof;
use crate::error::{bad, HsError, HsResult};
use crate::ingress::Arrival;
use crate::protocol::*;
use crate::stream::{ChangesPage, CommentViewPage, DisplayPage};
use crate::{now_ms, Station};
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub heads: usize,
    pub objects: usize,
    pub resynced: bool,
}

impl Station {
    pub async fn origin_for_did(&self, did: &str) -> HsResult<(String, String)> {
        let zone = self.directory.zone_of(did).await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        let origin = self.directory.origin_of_zone(&zone).await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        Ok((zone, origin))
    }

    /// GET a protocol path on another zone, identified by a reader proof.
    pub async fn remote_get(&self, zone: &str, origin: &str, path_and_query: &str) -> HsResult<reqwest::Response> {
        self.remote_get_as(zone, origin, path_and_query, true).await
    }

    /// `as_owner = false` reads as an anonymous visitor (portal previews).
    pub async fn remote_get_as(&self, zone: &str, origin: &str, path_and_query: &str, as_owner: bool) -> HsResult<reqwest::Response> {
        let url = format!("{}{}", origin.trim_end_matches('/'), path_and_query);
        let mut request = self.http.get(&url).header("host", zone).timeout(Duration::from_secs(20));
        if as_owner {
            let proof = make_reader_proof(&self.signer, &self.cfg.owner, zone).map_err(HsError::Internal)?;
            request = request.header("authorization", format!("DID {proof}"));
        }
        request.send().await.map_err(|e| HsError::Unavailable(format!("GET {url}: {e}")))
    }

    async fn remote_json<T: serde::de::DeserializeOwned>(&self, zone: &str, origin: &str, path: &str) -> HsResult<Option<T>> {
        let response = self.remote_get(zone, origin, path).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(HsError::Unavailable(format!("{path}: HTTP {}", response.status())));
        }
        let value = response.json::<T>().await.map_err(|e| HsError::Unavailable(format!("{path}: {e}")))?;
        Ok(Some(value))
    }

    pub(crate) async fn pull_loop(self: std::sync::Arc<Self>) {
        loop {
            if let Err(e) = self.pull_all().await {
                log::warn!("pull pass failed: {e}");
            }
            let _ = tokio::time::timeout(self.cfg.pull_interval, self.wake.pull.notified()).await;
        }
    }

    /// Sync every active source once (persons by change read, feeds by the Spider).
    pub async fn pull_all(&self) -> HsResult<()> {
        let sources: Vec<(String, String, Option<String>)> = self
            .db
            .call(|c| {
                let mut stmt = c.prepare("SELECT id, kind, did FROM sources WHERE paused=0 AND basis!='[]' ORDER BY COALESCE(last_fetch_at, 0)")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        for (id, kind, did) in sources {
            if let Some(did) = &did {
                if self.is_local_principal(did).await {
                    continue;
                }
            }
            let result = if kind == "person" {
                self.sync_person(&id).await.map(|_| ())
            } else if kind == "channel" {
                Ok(())
            } else if self.cfg.spider_enabled {
                self.crawl_source(&id).await.map(|_| ())
            } else {
                Ok(())
            };
            if let Err(e) = result {
                self.record_source_error(&id, &e.to_string()).await?;
            }
        }
        if let Err(e) = self.pull_collectors().await {
            log::debug!("collector pull failed: {e}");
        }
        self.wake.select.notify_one();
        Ok(())
    }

    pub async fn record_source_error(&self, id: &str, error: &str) -> HsResult<()> {
        let id = id.to_string();
        let reason = error.to_string();
        let error = serde_json::json!({ "at": now_ms(), "reason": error }).to_string();
        let now = now_ms();
        let changed = self
            .db
            .call(move |c| {
                let previous: Option<String> = c.query_row("SELECT last_error FROM sources WHERE id=?1", [&id], |r| r.get(0)).unwrap_or(None);
                let same = previous.and_then(|p| serde_json::from_str::<Value>(&p).ok()).and_then(|p| p.get("reason").and_then(Value::as_str).map(str::to_string)) == Some(reason);
                c.execute("UPDATE sources SET last_error=?2, last_fetch_at=?3 WHERE id=?1", params![id, error, now])?;
                Ok(!same)
            })
            .await?;
        if changed {
            self.bump(&["sources"]);
        }
        Ok(())
    }

    /// Change read of one followed person's stream; display read on first sync or re-sync.
    pub async fn sync_person(&self, source_id: &str) -> HsResult<SyncReport> {
        let id = source_id.to_string();
        let (did, name, cursor) = self
            .db
            .call(move |c| {
                Ok(c.query_row("SELECT did, name, cursor FROM sources WHERE id=?1", [id], |r| {
                    Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?))
                })?)
            })
            .await?;
        let did = did.ok_or_else(|| bad("source has no DID"))?;
        let (zone, origin) = self.origin_for_did(&did).await?;
        let arrival = Arrival::Pull { source_id: source_id.to_string(), label: name.clone() };
        let mut report = SyncReport::default();
        let mut cursor = cursor;
        if cursor.is_none() {
            cursor = Some(self.backfill(&zone, &origin, &arrival, &mut report).await?);
        }
        let mut since = cursor.unwrap_or(0);
        for _ in 0..20 {
            let path = format!("/home/feed?mode=changes&since={since}&objects=1");
            let page: ChangesPage = self.remote_json(&zone, &origin, &path).await?.ok_or_else(|| HsError::Unavailable("stream not found".into()))?;
            if page.resync {
                report.resynced = true;
                since = self.backfill(&zone, &origin, &arrival, &mut report).await?.max(page.next_cursor);
                break;
            }
            self.apply_objects(&page.objects, &arrival, &mut report).await;
            for change in &page.changes {
                self.apply_remote_head(&change.head, change.restricted, &did, &arrival, &page.objects, &mut report).await;
            }
            since = page.next_cursor.max(since);
            if !page.more {
                break;
            }
        }
        let id = source_id.to_string();
        let now = now_ms();
        let recovered = self
            .db
            .call(move |c| {
                let had_error: bool = c.query_row("SELECT last_error IS NOT NULL FROM sources WHERE id=?1", [&id], |r| r.get(0)).unwrap_or(false);
                c.execute(
                    "UPDATE sources SET cursor=?2, last_success_at=?3, last_fetch_at=?3, last_error=NULL WHERE id=?1",
                    params![id, since, now],
                )?;
                Ok(had_error)
            })
            .await?;
        if report.heads + report.objects > 0 || recovered {
            self.bump(&["sources", "candidates", "reading", "comments"]);
            self.wake.select.notify_one();
        }
        Ok(report)
    }

    async fn backfill(&self, zone: &str, origin: &str, arrival: &Arrival, report: &mut SyncReport) -> HsResult<i64> {
        let page: DisplayPage = self
            .remote_json(zone, origin, "/home/feed?mode=display&limit=30&objects=1")
            .await?
            .ok_or_else(|| HsError::Unavailable("stream not found".into()))?;
        self.apply_objects(&page.objects, arrival, report).await;
        for item in &page.items {
            self.apply_remote_head(&item.head, item.restricted, "", arrival, &page.objects, report).await;
        }
        Ok(page.change_cursor)
    }

    async fn apply_objects(&self, objects: &std::collections::BTreeMap<String, String>, arrival: &Arrival, report: &mut SyncReport) {
        // Parts first (FileObjects, wrapped objects), Feed Objects of the stream afterwards.
        let mut ordered: Vec<(&String, &String)> = objects.iter().collect();
        ordered.sort_by_key(|(id, _)| obj_type_of(id).as_deref() == Some(OBJ_TYPE_FEED));
        for (id, body) in ordered {
            let known = {
                let id = normalize_obj_id(id);
                self.db.call(move |c| crate::objects::has_object(c, &id)).await.unwrap_or(false)
            };
            // Known objects are ingested again: a new arrival path may make them a candidate.
            let is_head = obj_type_of(id).as_deref() == Some(OBJ_TYPE_HEAD);
            match self.ingest_expected(id, body, if is_head { Arrival::Fetch } else { arrival.clone() }, false).await {
                Ok(()) if !known => report.objects += 1,
                Ok(()) => {}
                Err(e) => log::debug!("attached object {id} refused: {e}"),
            }
        }
    }

    async fn apply_remote_head(
        &self,
        head_jwt: &str,
        restricted: bool,
        expected_publisher: &str,
        arrival: &Arrival,
        objects: &std::collections::BTreeMap<String, String>,
        report: &mut SyncReport,
    ) {
        let verified = match crate::objects::verify_jwt(self.directory.as_ref(), head_jwt, Some(OBJ_TYPE_HEAD)).await {
            Ok(v) => v,
            Err(e) => {
                log::debug!("remote head refused: {e:?}");
                return;
            }
        };
        if !expected_publisher.is_empty() && verified.publisher != expected_publisher {
            return;
        }
        let current = verified.claims.get("current").and_then(Value::as_str).map(normalize_obj_id);
        if let Some(current) = &current {
            let known = {
                let id = current.clone();
                self.db.call(move |c| crate::objects::has_object(c, &id)).await.unwrap_or(false)
            };
            if !known && !objects.contains_key(current) {
                let publisher = verified.publisher.clone();
                if let Err(e) = self.fetch_object_from(&publisher, current, arrival.clone()).await {
                    log::debug!("fetch {current} failed: {e}");
                }
            }
        }
        if let Ok(crate::ingress::HeadApply::New | crate::ingress::HeadApply::Conflict) = self.ingest_head(verified, restricted).await {
            report.heads += 1;
        }
    }

    /// Fetch an object by ObjId from `holder`'s HomeStation and ingest it after checking the id.
    pub async fn fetch_object_from(&self, holder: &str, obj_id: &str, arrival: Arrival) -> HsResult<()> {
        let (zone, origin) = self.origin_for_did(holder).await?;
        let response = self.remote_get(&zone, &origin, &format!("/home/objects/{}", normalize_obj_id(obj_id))).await?;
        if !response.status().is_success() {
            return Err(HsError::NotFound(format!("{obj_id} not readable at {holder} (HTTP {})", response.status())));
        }
        let restricted = response.headers().get(crate::delivery::HEADER_AUDIENCE).and_then(|v| v.to_str().ok()) == Some("restricted");
        let body = response.text().await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        self.ingest_expected(obj_id, &body, arrival, restricted).await
    }

    pub async fn fetch_chunk_from(&self, holder: &str, chunk_id: &str, max_bytes: u64) -> HsResult<Vec<u8>> {
        let (zone, origin) = self.origin_for_did(holder).await?;
        let response = self.remote_get(&zone, &origin, &format!("/home/chunks/{}", normalize_obj_id(chunk_id))).await?;
        if !response.status().is_success() {
            return Err(HsError::NotFound(format!("chunk not readable at {holder} (HTTP {})", response.status())));
        }
        if response.content_length().unwrap_or(0) > max_bytes {
            return Err(HsError::Unavailable("chunk exceeds the prefetch budget".into()));
        }
        let data = response.bytes().await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        if !crate::objects::verify_chunk(chunk_id, &data) {
            return Err(bad("chunk data does not match its id"));
        }
        Ok(data.to_vec())
    }

    /// Resolve an entry of another publisher: read its current Head from the namespace owner.
    pub async fn resolve_remote_entry(&self, entry: &str) -> HsResult<Option<crate::publish::HeadRow>> {
        let parsed = EntryRef::parse(entry).map_err(bad)?;
        let EntryRef::Path { zone, namespace, key } = parsed else {
            return Err(HsError::Unavailable("DID entries resolve through the name service".into()));
        };
        if zone == self.cfg.zone {
            let entry = entry.to_string();
            return self.db.call(move |c| crate::publish::get_head(c, &entry)).await;
        }
        let origin = self.directory.origin_of_zone(&zone).await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        let response = self.remote_get(&zone, &origin, &format!("/home/{}/@/{}", namespace.as_str(), key)).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(HsError::Unavailable(format!("HTTP {}", response.status())));
        }
        let restricted = response.headers().get(crate::delivery::HEADER_AUDIENCE).and_then(|v| v.to_str().ok()) == Some("restricted");
        let body = response.text().await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        let verified = crate::objects::verify_jwt(self.directory.as_ref(), body.trim(), Some(OBJ_TYPE_HEAD)).await.map_err(HsError::from)?;
        if verified.claims.get("entry").and_then(Value::as_str) != Some(entry) {
            return Err(bad("head is for another entry"));
        }
        self.ingest_head(verified, restricted).await?;
        let entry = entry.to_string();
        self.db.call(move |c| crate::publish::get_head(c, &entry)).await
    }

    /// Read another maintainer's comment view on `target` and keep what verifies (§13.3).
    pub async fn pull_comment_view(&self, maintainer: &str, target: &str, as_collector: bool) -> HsResult<Option<CommentViewPage>> {
        let (zone, origin) = self.origin_for_did(maintainer).await?;
        let path = format!("/home/comments?target={}", normalize_obj_id(target));
        let Some(page) = self.remote_json::<CommentViewPage>(&zone, &origin, &path).await? else { return Ok(None) };
        let arrival = if as_collector {
            Arrival::Collector { collector: maintainer.to_string() }
        } else {
            Arrival::AuthorList { author: maintainer.to_string() }
        };
        for (id, body) in &page.objects {
            let _ = self.ingest_expected(id, body, Arrival::Fetch, false).await;
        }
        for record in &page.records {
            if let Err(e) = self.ingest_expected(&record.comment_id, &record.comment, arrival.clone(), false).await {
                log::debug!("comment {} refused: {e}", record.comment_id);
                continue;
            }
            if let Some(head) = &record.head {
                if let Ok(v) = crate::objects::verify_jwt(self.directory.as_ref(), head, Some(OBJ_TYPE_HEAD)).await {
                    let _ = self.ingest_head(v, false).await;
                }
            }
        }
        Ok(Some(page))
    }

    /// Cold start (§14.3): recent public objects of the preset collectors become candidates.
    pub async fn pull_collectors(&self) -> HsResult<usize> {
        let settings = self.settings().await?;
        let mut total = 0;
        for collector in settings.collectors {
            let key = format!("collector_cursor:{}", collector.did);
            let key2 = key.clone();
            let since: i64 = self.db.call(move |c| crate::db::get_meta(c, &key2)).await?.and_then(|v| v.parse().ok()).unwrap_or(0);
            let (zone, origin) = match self.origin_for_did(&collector.did).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let path = format!("/home/index/recent?since={since}&limit=50");
            let page: Option<crate::stream::DisplayPage> = match self.remote_json(&zone, &origin, &path).await {
                Ok(p) => p,
                Err(e) => {
                    log::debug!("collector {} unavailable: {e}", collector.did);
                    continue;
                }
            };
            let Some(page) = page else { continue };
            let arrival = Arrival::Collector { collector: collector.did.clone() };
            let mut report = SyncReport::default();
            self.apply_objects(&page.objects, &arrival, &mut report).await;
            for item in &page.items {
                self.apply_remote_head(&item.head, false, "", &arrival, &page.objects, &mut report).await;
            }
            total += report.objects;
            let next = page.change_cursor;
            self.db.call(move |c| crate::db::set_meta(c, &key, &next.to_string())).await?;
        }
        Ok(total)
    }
}
