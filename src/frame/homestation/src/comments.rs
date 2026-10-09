//! Comment Sync / Index (§12–§15): every comment and interaction is an independent Feed Object
//! bound to a version; views by maintainer (author, collector, local merge); statistics are
//! derived from verified records only (§15.5).

use crate::error::{HsError, HsResult};
use crate::objects::get_feed;
use crate::projection::{identity_view, item_view, ViewCtx};
use crate::protocol::*;
use crate::publish::{entry_versions, get_head};
use crate::{now_ms, Station};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
struct CommentRow {
    id: String,
    publisher: String,
    comment_type: CommentType,
    target: String,
    entry: Option<String>,
    iat: i64,
}

/// The entry a version belongs to (any publisher), with its known versions.
fn versions_of(conn: &Connection, obj_id: &str) -> HsResult<Vec<String>> {
    let entry: Option<String> = conn
        .query_row("SELECT entry FROM feed_index WHERE obj_id=?1 AND entry_valid=1", [obj_id], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(match entry {
        Some(entry) => {
            let v = entry_versions(conn, &entry)?;
            if v.is_empty() {
                vec![obj_id.to_string()]
            } else {
                v
            }
        }
        None => vec![obj_id.to_string()],
    })
}

fn comments_on(conn: &Connection, targets: &[String]) -> HsResult<Vec<CommentRow>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT f.obj_id, f.publisher, f.comment_type, f.target, f.entry, f.iat, f.entry_valid FROM feed_index f
         JOIN objects o ON o.obj_id=f.obj_id
         WHERE f.target=?1 AND f.kind='comment' AND o.verified=1 AND o.local_only=0",
    )?;
    for target in targets {
        let rows = stmt
            .query_map([target], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (id, publisher, ct, target, entry, iat, valid) in rows {
            let Some(comment_type) = ct.as_deref().and_then(CommentType::parse) else { continue };
            out.push(CommentRow { id, publisher, comment_type, target, entry: if valid != 0 { entry } else { None }, iat });
        }
    }
    out.sort_by(|a, b| b.iat.cmp(&a.iat).then(a.id.cmp(&b.id)));
    Ok(out)
}

/// `Some(true)` active, `Some(false)` withdrawn, `None` contested (same seq conflict, A30).
fn record_state(conn: &Connection, row: &CommentRow) -> HsResult<Option<bool>> {
    let Some(entry) = &row.entry else { return Ok(Some(true)) };
    match get_head(conn, entry)? {
        None => Ok(Some(true)),
        Some(h) if h.conflict_obj_id.is_some() => Ok(None),
        Some(h) if h.state == HeadState::Withdrawn => Ok(Some(false)),
        Some(_) => Ok(Some(true)),
    }
}

fn sources_of(conn: &Connection, comment_id: &str) -> HsResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT source_kind, source FROM comment_sources WHERE comment_id=?1")?;
    let rows = stmt.query_map([comment_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn own_target(conn: &Connection, target: &str) -> HsResult<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM entry_versions v JOIN published p ON p.entry=v.entry WHERE v.obj_id=?1", [target], |_| Ok(()))
        .optional()?
        .is_some())
}

fn listed_by_author(conn: &Connection, row: &CommentRow, target_publisher: &str, owner: &str) -> HsResult<bool> {
    if target_publisher == owner && own_target(conn, &row.target)? {
        return Ok(conn
            .query_row("SELECT listed FROM author_list WHERE target=?1 AND comment_id=?2", params![row.target, row.id], |r| r.get::<_, i64>(0))
            .optional()?
            .is_some_and(|l| l != 0));
    }
    Ok(sources_of(conn, &row.id)?.iter().any(|(k, s)| k == "author_list" && s == target_publisher))
}

fn listed_by_collector(conn: &Connection, row: &CommentRow, collector: Option<&str>) -> HsResult<bool> {
    Ok(sources_of(conn, &row.id)?.iter().any(|(k, s)| k == "collector" && collector.is_none_or(|c| c == s)))
}

fn in_view(conn: &Connection, row: &CommentRow, view: &str, target_publisher: &str, owner: &str) -> HsResult<bool> {
    if view == "local" {
        return Ok(true);
    }
    if view == "author" {
        return listed_by_author(conn, row, target_publisher, owner);
    }
    if let Some(collector) = view.strip_prefix("collector:") {
        return listed_by_collector(conn, row, Some(collector));
    }
    Ok(false)
}

/// Counts of one exact version in one view: one per logical publication or interaction key,
/// withdrawn and contested records excluded, private bookmarks never counted (§15.5).
pub fn stats(conn: &Connection, ctx: &ViewCtx, obj_id: &str, view: &str) -> HsResult<Value> {
    let obj_id = normalize_obj_id(obj_id);
    let target_publisher = get_feed(conn, &obj_id)?.map(|f| f.publisher).unwrap_or_default();
    let rows = comments_on(conn, std::slice::from_ref(&obj_id))?;
    let mut text = HashSet::new();
    let mut quotes = HashSet::new();
    let mut keys: HashMap<CommentType, HashSet<String>> = HashMap::new();
    let mut contested = 0;
    for row in &rows {
        if !in_view(conn, row, view, &target_publisher, &ctx.owner)? {
            continue;
        }
        match record_state(conn, row)? {
            Some(true) => {}
            Some(false) => continue,
            None => {
                contested += 1;
                continue;
            }
        }
        let logical = row.entry.clone().unwrap_or_else(|| row.id.clone());
        match row.comment_type {
            CommentType::Text => {
                text.insert(logical);
            }
            CommentType::Quote => {
                quotes.insert(logical);
            }
            t => {
                keys.entry(t).or_default().insert(row.publisher.clone());
            }
        }
    }
    let (synced_at, tracked): (Option<i64>, bool) = conn
        .query_row("SELECT last_sync_at FROM tracked WHERE target=?1", [&obj_id], |r| r.get(0))
        .optional()?
        .map(|v| (v, true))
        .unwrap_or((None, false));
    let sync = if view.starts_with("collector:") {
        "partial"
    } else if tracked && synced_at.is_none() && target_publisher != ctx.owner {
        "syncing"
    } else {
        "synced"
    };
    let mut out = json!({
        "view": view,
        "asOf": synced_at.unwrap_or_else(now_ms),
        "sync": sync,
        "textComments": text.len(),
        "likes": keys.get(&CommentType::Like).map_or(0, |k| k.len()),
        "reposts": keys.get(&CommentType::Repost).map_or(0, |k| k.len()),
        "quotes": quotes.len(),
        "bookmarks": keys.get(&CommentType::Bookmark).map_or(0, |k| k.len()),
        "contested": contested,
    });
    let claimed: Option<String> = crate::db::get_meta(conn, &format!("claimed:{obj_id}"))?;
    if let Some(claimed) = claimed.and_then(|c| serde_json::from_str::<Value>(&c).ok()) {
        if target_publisher != ctx.owner {
            out["claimed"] = json!({ "likes": claimed.get("likes"), "source": identity_view(ctx, &target_publisher)["name"] });
        }
    }
    Ok(out)
}

/// Comments removed from a maintainer's complete list between two complete snapshots (§16.3).
fn observed_removals(conn: &Connection, view: &str, target: &str) -> HsResult<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT comment_ids FROM view_snapshots WHERE view=?1 AND target=?2 AND complete=1 ORDER BY at DESC, id DESC LIMIT 2")?;
    let snaps = stmt.query_map(params![view, target], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    if snaps.len() < 2 {
        return Ok(HashSet::new());
    }
    let latest: HashSet<String> = serde_json::from_str(&snaps[0]).unwrap_or_default();
    let older: HashSet<String> = serde_json::from_str(&snaps[1]).unwrap_or_default();
    Ok(older.difference(&latest).cloned().collect())
}

impl Station {
    /// `listComments` (§15): current version and earlier ones of the entry; views by maintainer.
    pub async fn comment_list(&self, obj_id: &str, view: &str, comment_type: &str) -> HsResult<Value> {
        let ctx = self.view_ctx(crate::audience::Reader::Owner).await;
        let obj_id = normalize_obj_id(obj_id);
        let view = view.to_string();
        let filter = CommentType::parse(comment_type).ok_or_else(|| HsError::BadRequest("unknown comment type".into()))?;
        let collector_names: HashMap<String, String> = ctx.settings.collectors.iter().map(|c| (c.did.clone(), c.name.clone())).collect();
        self.db
            .call(move |c| {
                let target_publisher = get_feed(c, &obj_id)?.map(|f| f.publisher).unwrap_or_default();
                let versions = versions_of(c, &obj_id)?;
                let index = versions.iter().position(|v| v == &obj_id).unwrap_or(0);
                let related: Vec<String> = versions[..=index.min(versions.len().saturating_sub(1))].to_vec();
                let rows = comments_on(c, &related)?;
                let removed = observed_removals(c, "author", &obj_id)?;
                let mut seen_keys = HashSet::new();
                let mut comments = Vec::new();
                for row in rows {
                    if row.comment_type != filter || !in_view(c, &row, &view, &target_publisher, &ctx.owner)? {
                        continue;
                    }
                    if record_state(c, &row)? == Some(false) {
                        continue;
                    }
                    if row.comment_type.is_toggle() && !seen_keys.insert((row.publisher.clone(), row.target.clone())) {
                        continue;
                    }
                    let Some(item) = item_view(c, &ctx, &row.id, 1)? else { continue };
                    let mut paths = Vec::new();
                    for (kind, source) in sources_of(c, &row.id)? {
                        let (kind, label) = match kind.as_str() {
                            "author_list" => ("author_list", identity_view(&ctx, &source)["name"].as_str().unwrap_or_default().to_string()),
                            "collector" => ("collector", collector_names.get(&source).cloned().unwrap_or(source)),
                            "push" => ("push", identity_view(&ctx, &source)["name"].as_str().unwrap_or_default().to_string()),
                            _ => ("participant", if source.is_empty() { ctx.owner_name.clone() } else { source }),
                        };
                        let path = json!({ "kind": kind, "label": label });
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                    if row.publisher == ctx.owner {
                        paths.push(json!({ "kind": "participant", "label": ctx.owner_name }));
                    }
                    let target_version = versions.iter().position(|v| v == &row.target).map(|i| i + 1).unwrap_or(1);
                    comments.push(crate::projection::without_nulls(json!({
                        "objId": row.id,
                        "item": item,
                        "commentType": row.comment_type.as_str(),
                        "targetObjId": row.target,
                        "onOldVersion": row.target != obj_id,
                        "targetVersion": target_version,
                        "sourcePaths": paths,
                        "listedByAuthor": listed_by_author(c, &row, &target_publisher, &ctx.owner)?,
                        "listedByCollector": listed_by_collector(c, &row, None)?,
                        "authorRemoval": if removed.contains(&row.id) { Some("observed") } else { None },
                        "contested": record_state(c, &row)?.is_none(),
                    })));
                }
                let state = get_feed(c, &obj_id)?.and_then(|f| crate::projection::entry_state(c, &obj_id, &f).ok().flatten());
                Ok(json!({
                    "comments": comments,
                    "stats": stats(c, &ctx, &obj_id, &view)?,
                    "targetVersion": state.as_ref().and_then(|s| s.get("version").cloned()).unwrap_or(json!(1)),
                    "versionCount": state.as_ref().and_then(|s| s.get("versionCount").cloned()).unwrap_or(json!(1)),
                }))
            })
            .await
    }

    pub async fn track(&self, target: &str, author: &str, reason: &str) -> HsResult<()> {
        let (target, author, reason) = (normalize_obj_id(target), author.to_string(), reason.to_string());
        let now = now_ms();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO tracked(target, author, reason, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![target, author, reason, now],
                )?;
                Ok(())
            })
            .await?;
        self.wake.comments.notify_one();
        Ok(())
    }

    pub(crate) async fn comment_sync_loop(self: std::sync::Arc<Self>) {
        loop {
            if let Err(e) = self.sync_tracked().await {
                log::warn!("comment sync failed: {e}");
            }
            let _ = tokio::time::timeout(self.cfg.comment_sync_interval, self.wake.comments.notified()).await;
        }
    }

    /// Keep following discussions one took part in (§13.3): author view plus collector views.
    pub async fn sync_tracked(&self) -> HsResult<usize> {
        let owner = self.cfg.owner.clone();
        let targets: Vec<(String, String)> = self
            .db
            .call(move |c| {
                let mut stmt = c.prepare("SELECT target, author FROM tracked WHERE paused=0 AND author!=?1 ORDER BY COALESCE(last_sync_at, 0) LIMIT 50")?;
                let rows = stmt.query_map([owner], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let collectors = self.collector_dids().await;
        let before = self.comment_fingerprint().await?;
        let mut synced = 0;
        for (target, author) in targets {
            match self.pull_comment_view(&author, &target, false).await {
                Ok(Some(page)) => {
                    self.snapshot("author", &target, &page.records.iter().map(|r| r.comment_id.clone()).collect::<Vec<_>>(), page.complete).await?;
                    if let Some(claimed) = page.claimed {
                        let key = format!("claimed:{target}");
                        self.db.call(move |c| crate::db::set_meta(c, &key, &claimed.to_string())).await?;
                    }
                }
                Ok(None) => {}
                Err(e) => log::debug!("author view of {target}: {e}"),
            }
            for collector in &collectors {
                if let Ok(Some(page)) = self.pull_comment_view(collector, &target, true).await {
                    self.snapshot(&format!("collector:{collector}"), &target, &page.records.iter().map(|r| r.comment_id.clone()).collect::<Vec<_>>(), page.complete).await?;
                }
            }
            let t = target.clone();
            let now = now_ms();
            self.db.call(move |c| Ok(c.execute("UPDATE tracked SET last_sync_at=?2 WHERE target=?1", params![t, now])?)).await?;
            synced += 1;
        }
        if synced > 0 && self.comment_fingerprint().await? != before {
            self.bump(&["comments"]);
        }
        Ok(synced)
    }

    /// Changes when comment records, their Heads or the maintainers' lists change.
    async fn comment_fingerprint(&self) -> HsResult<(i64, i64, i64)> {
        self.db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT (SELECT COUNT(*) FROM comment_sources), (SELECT COUNT(*) FROM view_snapshots), (SELECT COALESCE(SUM(seq), 0) FROM heads)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .await
    }

    async fn snapshot(&self, view: &str, target: &str, ids: &[String], complete: bool) -> HsResult<()> {
        let (view, target) = (view.to_string(), target.to_string());
        let ids = serde_json::to_string(ids)?;
        let now = now_ms();
        self.db
            .call(move |c| {
                let last: Option<String> = c
                    .query_row("SELECT comment_ids FROM view_snapshots WHERE view=?1 AND target=?2 ORDER BY at DESC, id DESC LIMIT 1", params![view, target], |r| r.get(0))
                    .optional()?;
                if last.as_deref() != Some(ids.as_str()) {
                    c.execute(
                        "INSERT INTO view_snapshots(view, target, complete, comment_ids, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![view, target, complete as i64, ids, now],
                    )?;
                }
                Ok(())
            })
            .await
    }

    /// The author maintains their own list (§13.2); this never touches the comment object.
    pub async fn set_author_listing(&self, target: &str, comment_id: &str, listed: bool) -> HsResult<()> {
        let (target, comment_id) = (normalize_obj_id(target), normalize_obj_id(comment_id));
        let now = now_ms();
        self.db
            .call(move |c| {
                if !own_target(c, &target)? {
                    return Err(HsError::Forbidden("only the author's own versions have an author list".into()));
                }
                let position: i64 = c.query_row("SELECT COALESCE(MAX(position),0)+1 FROM author_list WHERE target=?1", [&target], |r| r.get(0))?;
                c.execute(
                    "INSERT INTO author_list(target, comment_id, listed, position, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(target, comment_id) DO UPDATE SET listed=excluded.listed, updated_at=excluded.updated_at",
                    params![target, comment_id, listed as i64, position, now],
                )?;
                Ok(())
            })
            .await?;
        self.bump(&["comments"]);
        Ok(())
    }
}
