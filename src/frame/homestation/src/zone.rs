//! Zone feed list (§4.6): an aggregate list maintained by the zone. A user who may write to it
//! chooses, when posting, to also list the post here. Items stay the publishers' own signed
//! Heads in their own streams; the zone only keeps which entries are listed and a change
//! sequence, so readers verify every item exactly like items of a personal stream.

use crate::db::Db;
use crate::error::HsResult;
use crate::now_ms;
use rusqlite::{params, OptionalExtension};
use std::path::Path;

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS zone_feed (
  entry TEXT PRIMARY KEY,
  user TEXT NOT NULL,
  publisher TEXT NOT NULL,
  iat INTEGER NOT NULL,
  listed INTEGER NOT NULL DEFAULT 1,
  listed_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS zone_feed_order ON zone_feed(listed, iat, entry);
CREATE TABLE IF NOT EXISTS zone_changes (
  cursor INTEGER PRIMARY KEY AUTOINCREMENT,
  entry TEXT NOT NULL,
  user TEXT NOT NULL,
  kind TEXT NOT NULL,
  at INTEGER NOT NULL
);
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct ZoneRow {
    pub entry: String,
    pub user: String,
    pub iat: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZoneChangeRow {
    pub cursor: i64,
    pub entry: String,
    pub user: String,
    /// `listed` / `unlisted` / `head` / `audience`
    pub kind: String,
}

#[derive(Clone)]
pub struct ZoneFeed {
    pub db: Db,
}

impl ZoneFeed {
    pub fn open(path: &Path) -> HsResult<Self> {
        Ok(Self { db: Db::open_schema(path, SCHEMA, SCHEMA_VERSION)? })
    }

    /// List or unlist an entry; returns whether anything changed.
    pub async fn set_listed(&self, user: &str, publisher: &str, entry: &str, iat: i64, listed: bool) -> HsResult<bool> {
        let (user, publisher, entry) = (user.to_string(), publisher.to_string(), entry.to_string());
        let now = now_ms();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                let current: Option<i64> = tx.query_row("SELECT listed FROM zone_feed WHERE entry=?1", [&entry], |r| r.get(0)).optional()?;
                if current.map(|v| v != 0) == Some(listed) || (current.is_none() && !listed) {
                    return Ok(false);
                }
                tx.execute(
                    "INSERT INTO zone_feed(entry, user, publisher, iat, listed, listed_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                     ON CONFLICT(entry) DO UPDATE SET listed=excluded.listed, iat=excluded.iat, updated_at=excluded.updated_at,
                       listed_at=CASE WHEN excluded.listed=1 THEN excluded.listed_at ELSE zone_feed.listed_at END",
                    params![entry, user, publisher, iat, listed as i64, now],
                )?;
                tx.execute(
                    "INSERT INTO zone_changes(entry, user, kind, at) VALUES (?1, ?2, ?3, ?4)",
                    params![entry, user, if listed { "listed" } else { "unlisted" }, now],
                )?;
                tx.commit()?;
                Ok(true)
            })
            .await
    }

    /// The publisher changed a listed entry (new Head or audience): readers following the zone
    /// feed see it in the change read.
    pub async fn touch(&self, entry: &str, kind: &str) -> HsResult<()> {
        let (entry, kind) = (entry.to_string(), kind.to_string());
        let now = now_ms();
        self.db
            .call(move |c| {
                let user: Option<String> = c.query_row("SELECT user FROM zone_feed WHERE entry=?1 AND listed=1", [&entry], |r| r.get(0)).optional()?;
                if let Some(user) = user {
                    c.execute("INSERT INTO zone_changes(entry, user, kind, at) VALUES (?1, ?2, ?3, ?4)", params![entry, user, kind, now])?;
                    c.execute("UPDATE zone_feed SET updated_at=?2 WHERE entry=?1", params![entry, now])?;
                }
                Ok(())
            })
            .await
    }

    pub async fn is_listed(&self, entry: &str) -> HsResult<bool> {
        let entry = entry.to_string();
        self.db
            .call(move |c| Ok(c.query_row("SELECT 1 FROM zone_feed WHERE entry=?1 AND listed=1", [entry], |_| Ok(())).optional()?.is_some()))
            .await
    }

    /// Listed entries in display order (newest declared `iat` first) after the cursor
    /// `<iat>|<entry>`.
    pub async fn page(&self, after: Option<(i64, String)>, limit: usize) -> HsResult<Vec<ZoneRow>> {
        let limit = limit as i64;
        self.db
            .call(move |c| {
                let row = |r: &rusqlite::Row| Ok(ZoneRow { entry: r.get(0)?, user: r.get(1)?, iat: r.get(2)? });
                let rows = match after {
                    Some((at, ae)) => c
                        .prepare(
                            "SELECT entry, user, iat FROM zone_feed WHERE listed=1 AND (iat<?1 OR (iat=?1 AND entry<?2))
                             ORDER BY iat DESC, entry DESC LIMIT ?3",
                        )?
                        .query_map(params![at, ae, limit], row)?
                        .collect::<Result<Vec<_>, _>>()?,
                    None => c
                        .prepare("SELECT entry, user, iat FROM zone_feed WHERE listed=1 ORDER BY iat DESC, entry DESC LIMIT ?1")?
                        .query_map(params![limit], row)?
                        .collect::<Result<Vec<_>, _>>()?,
                };
                Ok(rows)
            })
            .await
    }

    pub async fn changes(&self, since: i64, limit: usize) -> HsResult<Vec<ZoneChangeRow>> {
        let limit = limit as i64;
        self.db
            .call(move |c| {
                let mut stmt = c.prepare("SELECT cursor, entry, user, kind FROM zone_changes WHERE cursor>?1 ORDER BY cursor LIMIT ?2")?;
                let rows = stmt
                    .query_map(params![since, limit], |r| Ok(ZoneChangeRow { cursor: r.get(0)?, entry: r.get(1)?, user: r.get(2)?, kind: r.get(3)? }))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn max_cursor(&self) -> HsResult<i64> {
        self.db.call(|c| Ok(c.query_row("SELECT COALESCE(MAX(cursor), 0) FROM zone_changes", [], |r| r.get(0))?)).await
    }

    pub async fn count(&self) -> HsResult<i64> {
        self.db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM zone_feed WHERE listed=1", [], |r| r.get(0))?)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn listing_orders_and_records_changes() {
        let dir = tempfile::tempdir().unwrap();
        let feed = ZoneFeed::open(&dir.path().join("zone.db")).unwrap();
        assert!(feed.set_listed("alice", "did:a", "cyfs://z/home/alice/feed/@/p1", 10, true).await.unwrap());
        assert!(!feed.set_listed("alice", "did:a", "cyfs://z/home/alice/feed/@/p1", 10, true).await.unwrap());
        assert!(feed.set_listed("bob", "did:b", "cyfs://z/home/bob/feed/@/p2", 20, true).await.unwrap());
        assert!(!feed.set_listed("bob", "did:b", "cyfs://z/home/bob/feed/@/never", 30, false).await.unwrap());
        let page = feed.page(None, 10).await.unwrap();
        assert_eq!(page.iter().map(|r| r.user.as_str()).collect::<Vec<_>>(), ["bob", "alice"]);
        let next = feed.page(Some((page[0].iat, page[0].entry.clone())), 10).await.unwrap();
        assert_eq!(next.len(), 1);
        feed.touch("cyfs://z/home/alice/feed/@/p1", "head").await.unwrap();
        feed.touch("cyfs://z/home/alice/feed/@/unlisted", "head").await.unwrap();
        assert!(feed.set_listed("alice", "did:a", "cyfs://z/home/alice/feed/@/p1", 10, false).await.unwrap());
        assert!(!feed.is_listed("cyfs://z/home/alice/feed/@/p1").await.unwrap());
        let kinds: Vec<String> = feed.changes(0, 10).await.unwrap().into_iter().map(|c| c.kind).collect();
        assert_eq!(kinds, ["listed", "listed", "head", "unlisted"]);
        assert_eq!(feed.count().await.unwrap(), 1);
    }
}
