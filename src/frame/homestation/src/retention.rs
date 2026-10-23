//! Holds the files an owner's entries reference in the zone named store (share design D17).
//!
//! Each Active entry retains the FileObjects its versions fix as content (wraps, cover,
//! media) under the owner `homestation:<entry>`; a withdrawn entry releases them. Pins are
//! taken before a version is published and recorded in `retentions` (`pending` → `held`,
//! `releasing` → deleted), so a crash or a store failure in between is finished by
//! [`Station::reconcile_retentions`], which also backfills entries published before this
//! table existed and records unreadable content in `retention_issues`.

use crate::error::HsResult;
use crate::protocol::{obj_type_of, FeedObject, HeadState, OBJ_TYPE_FEED, OBJ_TYPE_FILE, OBJ_TYPE_DIR};
use crate::objects::get_object;
use crate::publish::{entry_versions, get_head};
use crate::Station;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn retention_owner(entry: &str) -> String {
    format!("homestation:{entry}")
}

/// FileObjects among a Feed Object's content parts.
pub fn file_parts(feed: &FeedObject) -> Vec<String> {
    feed.content_parts().into_iter().filter(|p| matches!(obj_type_of(p).as_deref(), Some(OBJ_TYPE_FILE | OBJ_TYPE_DIR))).collect()
}

/// What the entry should hold right now: the files of all its versions while its Head is Active.
fn wanted(conn: &Connection, entry: &str) -> HsResult<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    if get_head(conn, entry)?.is_none_or(|h| h.state != HeadState::Active) {
        return Ok(out);
    }
    for version in entry_versions(conn, entry)? {
        let Some(obj) = get_object(conn, &version)? else { continue };
        if obj.obj_type != OBJ_TYPE_FEED {
            continue;
        }
        if let Ok(feed) = serde_json::from_value::<FeedObject>(obj.body) {
            out.extend(file_parts(&feed));
        }
    }
    Ok(out)
}

fn rows(conn: &Connection, entry: &str) -> HsResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT obj_id, state FROM retentions WHERE entry=?1")?;
    let rows = stmt.query_map([entry], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn set_state(conn: &Connection, entry: &str, obj_id: &str, state: &str, now: i64) -> HsResult<()> {
    conn.execute(
        "INSERT INTO retentions(entry, obj_id, state, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(entry, obj_id) DO UPDATE SET state=excluded.state, updated_at=excluded.updated_at",
        params![entry, obj_id, state, now],
    )?;
    Ok(())
}

/// Marks the pins a version took as held, inside the transaction that publishes it.
pub fn confirm_pending(conn: &Connection, entry: &str, now: i64) -> HsResult<()> {
    conn.execute("UPDATE retentions SET state='held', updated_at=?2 WHERE entry=?1 AND state='pending'", params![entry, now])?;
    Ok(())
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct RetentionReport {
    pub entries: usize,
    pub pinned: usize,
    pub released: usize,
    pub failed: usize,
    /// `(entry, obj_id, issue)` for content that is held but cannot be read here.
    pub issues: Vec<(String, String, String)>,
}

impl Station {
    /// Pins `parts` for `entry` before the version referencing them is published.
    pub(crate) async fn retain_parts(&self, entry: &str, parts: &[String]) -> HsResult<()> {
        if parts.is_empty() {
            return Ok(());
        }
        let (entry_s, todo) = (entry.to_string(), parts.to_vec());
        let now = crate::now_ms();
        let todo = self
            .db
            .call(move |c| {
                let held: BTreeSet<String> = rows(c, &entry_s)?.into_iter().filter(|(_, s)| s == "held").map(|(o, _)| o).collect();
                let todo: Vec<String> = todo.into_iter().filter(|p| !held.contains(p)).collect();
                for part in &todo {
                    set_state(c, &entry_s, part, "pending", now)?;
                }
                Ok(todo)
            })
            .await?;
        let owner = retention_owner(entry);
        for part in &todo {
            self.chunks.retain(part, &owner).await?;
        }
        Ok(())
    }

    /// Brings one entry's pins to what it should hold; failures stay recorded for a retry.
    pub async fn sync_retention(&self, entry: &str) -> HsResult<(usize, usize)> {
        let entry_s = entry.to_string();
        let (wanted, current) = self.db.call(move |c| Ok((wanted(c, &entry_s)?, rows(c, &entry_s)?))).await?;
        let owner = retention_owner(entry);
        let (mut pinned, mut released) = (0, 0);
        let mut first_err = None;
        for obj_id in &wanted {
            if current.iter().any(|(o, s)| o == obj_id && s == "held") {
                continue;
            }
            let (e, o) = (entry.to_string(), obj_id.clone());
            self.db.call(move |c| set_state(c, &e, &o, "pending", crate::now_ms())).await?;
            match self.chunks.retain(obj_id, &owner).await {
                Ok(()) => {
                    let (e, o) = (entry.to_string(), obj_id.clone());
                    self.db.call(move |c| set_state(c, &e, &o, "held", crate::now_ms())).await?;
                    pinned += 1;
                }
                Err(err) => first_err = first_err.or(Some(err)),
            }
        }
        for (obj_id, _) in current.iter().filter(|(o, _)| !wanted.contains(o)) {
            let (e, o) = (entry.to_string(), obj_id.clone());
            self.db.call(move |c| set_state(c, &e, &o, "releasing", crate::now_ms())).await?;
            match self.chunks.release(obj_id, &owner).await {
                Ok(()) => {
                    let (e, o) = (entry.to_string(), obj_id.clone());
                    self.db
                        .call(move |c| {
                            c.execute("DELETE FROM retentions WHERE entry=?1 AND obj_id=?2 AND state='releasing'", params![e, o])?;
                            c.execute("DELETE FROM retention_issues WHERE entry=?1 AND obj_id=?2", params![e, o])?;
                            Ok(())
                        })
                        .await?;
                    released += 1;
                }
                Err(err) => first_err = first_err.or(Some(err)),
            }
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok((pinned, released)),
        }
    }

    /// Every own entry and every recorded pin: finish interrupted pins and releases, backfill
    /// older entries, and check that held content is still readable from the store.
    pub async fn reconcile_retentions(&self) -> HsResult<RetentionReport> {
        let entries = self
            .db
            .call(|c| {
                let mut stmt = c.prepare("SELECT entry FROM published UNION SELECT entry FROM retentions")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let mut report = RetentionReport { entries: entries.len(), ..Default::default() };
        for entry in &entries {
            match self.sync_retention(entry).await {
                Ok((p, r)) => {
                    report.pinned += p;
                    report.released += r;
                }
                Err(e) => {
                    report.failed += 1;
                    log::warn!("retention of {entry} not settled: {e}");
                }
            }
        }
        let held = self
            .db
            .call(|c| {
                let mut stmt = c.prepare("SELECT entry, obj_id FROM retentions WHERE state='held'")?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let now = crate::now_ms();
        for (entry, obj_id) in held {
            let issue = self.content_issue(&obj_id).await;
            let (e, o, i) = (entry.clone(), obj_id.clone(), issue.clone());
            self.db
                .call(move |c| {
                    match &i {
                        Some(issue) => c.execute(
                            "INSERT INTO retention_issues(entry, obj_id, issue, checked_at) VALUES (?1, ?2, ?3, ?4)
                             ON CONFLICT(entry, obj_id) DO UPDATE SET issue=excluded.issue, checked_at=excluded.checked_at",
                            params![e, o, issue, now],
                        )?,
                        None => c.execute("DELETE FROM retention_issues WHERE entry=?1 AND obj_id=?2", params![e, o])?,
                    };
                    Ok(())
                })
                .await?;
            if let Some(issue) = issue {
                log::warn!("retained {obj_id} of {entry} is not readable: {issue}");
                report.issues.push((entry, obj_id, issue));
            }
        }
        Ok(report)
    }

    /// Why a held FileObject cannot be served from the store, if it cannot.
    async fn content_issue(&self, obj_id: &str) -> Option<String> {
        match self.chunks.is_complete(obj_id).await {
            Ok(true) => None,
            Ok(false) => Some("content_missing".into()),
            Err(error) => Some(format!("object_unavailable: {error}")),
        }
    }

    pub async fn retention_status(&self) -> HsResult<Value> {
        self.db
            .call(|c| {
                let count = |state: &str| -> HsResult<i64> {
                    Ok(c.query_row("SELECT COUNT(*) FROM retentions WHERE state=?1", [state], |r| r.get(0))?)
                };
                let mut stmt = c.prepare("SELECT entry, obj_id, issue, checked_at FROM retention_issues ORDER BY entry, obj_id")?;
                let issues = stmt
                    .query_map([], |r| {
                        Ok(json!({"entry": r.get::<_, String>(0)?, "objId": r.get::<_, String>(1)?, "issue": r.get::<_, String>(2)?, "checkedAt": r.get::<_, i64>(3)?}))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"held": count("held")?, "pending": count("pending")?, "releasing": count("releasing")?, "issues": issues}))
            })
            .await
    }
}
