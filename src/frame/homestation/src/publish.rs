//! Publish Service (§4.3, §16): signs Feed Objects, serially issues the Heads of the owner's
//! entries, keeps the stream's change sequence and read grants, and records publish tasks.

use crate::audience::AudienceSpec;
use crate::error::{bad, HsError, HsResult};
use crate::objects::{get_object, index_feed, put_object, StoredObject};
use crate::protocol::*;
use crate::sign::Signer;
use crate::{new_id, now_ms, now_s, Station};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Post,
    Comment,
    Repost,
    Quote,
    Like,
    Dislike,
    Bookmark,
    Follow,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Post => "post",
            Self::Comment => "comment",
            Self::Repost => "repost",
            Self::Quote => "quote",
            Self::Like => "like",
            Self::Dislike => "dislike",
            Self::Bookmark => "bookmark",
            Self::Follow => "follow",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "post" => Self::Post,
            "comment" => Self::Comment,
            "repost" => Self::Repost,
            "quote" => Self::Quote,
            "like" => Self::Like,
            "dislike" => Self::Dislike,
            "bookmark" => Self::Bookmark,
            "follow" => Self::Follow,
            _ => return None,
        })
    }
    pub fn of_comment(comment_type: CommentType) -> Self {
        match comment_type {
            CommentType::Text => Self::Comment,
            CommentType::Like => Self::Like,
            CommentType::Dislike => Self::Dislike,
            CommentType::Bookmark => Self::Bookmark,
            CommentType::Repost => Self::Repost,
            CommentType::Quote => Self::Quote,
        }
    }
    pub fn is_reaction(self) -> bool {
        matches!(self, Self::Like | Self::Dislike | Self::Bookmark)
    }
}

#[derive(Debug, Clone)]
pub struct EntryRow {
    pub entry: String,
    pub key: String,
    pub namespace: String,
    pub kind: EntryKind,
    pub audience: AudienceSpec,
    pub category: Option<String>,
    pub target: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn entry_row(r: &rusqlite::Row) -> rusqlite::Result<EntryRow> {
    let audience: String = r.get(4)?;
    let kind: String = r.get(3)?;
    Ok(EntryRow {
        entry: r.get(0)?,
        key: r.get(1)?,
        namespace: r.get(2)?,
        kind: EntryKind::parse(&kind).unwrap_or(EntryKind::Post),
        audience: serde_json::from_str(&audience).unwrap_or(AudienceSpec::Public),
        category: r.get(5)?,
        target: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

pub const ENTRY_COLUMNS: &str = "entry, key, namespace, kind, audience, category, target, created_at, updated_at";

pub fn get_entry(conn: &Connection, entry: &str) -> HsResult<Option<EntryRow>> {
    Ok(conn
        .query_row(&format!("SELECT {ENTRY_COLUMNS} FROM published WHERE entry=?1"), [entry], entry_row)
        .optional()?)
}

pub fn list_entries(conn: &Connection, sql_where: &str, args: &[&dyn rusqlite::ToSql]) -> HsResult<Vec<EntryRow>> {
    let mut stmt = conn.prepare(&format!("SELECT {ENTRY_COLUMNS} FROM published {sql_where}"))?;
    let rows = stmt.query_map(args, entry_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeadRow {
    pub entry: String,
    pub publisher: String,
    pub seq: u64,
    pub state: HeadState,
    pub current: Option<String>,
    pub head_obj_id: String,
    pub conflict_obj_id: Option<String>,
    pub restricted: bool,
    pub updated_at_ms: i64,
}

pub fn get_head(conn: &Connection, entry: &str) -> HsResult<Option<HeadRow>> {
    Ok(conn
        .query_row(
            "SELECT entry, publisher, seq, state, current, head_obj_id, conflict_obj_id, restricted, updated_at_ms FROM heads WHERE entry=?1",
            [entry],
            |r| {
                let state: String = r.get(3)?;
                Ok(HeadRow {
                    entry: r.get(0)?,
                    publisher: r.get(1)?,
                    seq: r.get::<_, i64>(2)? as u64,
                    state: HeadState::parse(&state).unwrap_or(HeadState::Active),
                    current: r.get(4)?,
                    head_obj_id: r.get(5)?,
                    conflict_obj_id: r.get(6)?,
                    restricted: r.get::<_, i64>(7)? != 0,
                    updated_at_ms: r.get(8)?,
                })
            },
        )
        .optional()?)
}

/// Known versions of an entry, oldest first.
pub fn entry_versions(conn: &Connection, entry: &str) -> HsResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT v.obj_id FROM entry_versions v LEFT JOIN feed_index f ON f.obj_id=v.obj_id
         WHERE v.entry=?1 ORDER BY COALESCE(f.iat, 0), v.first_seen, v.rowid",
    )?;
    let rows = stmt.query_map([entry], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

pub fn add_version(conn: &Connection, entry: &str, obj_id: &str, now: i64) -> HsResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO entry_versions(entry, obj_id, first_seen) VALUES (?1, ?2, ?3)",
        params![entry, obj_id, now],
    )?;
    Ok(())
}

/// Read grant: `obj_id` may be served to whoever may read `entry` (§4.5). FileObjects pass the
/// grant on to their content chunk, wrapped Feed Objects to their own parts.
pub fn grant(conn: &Connection, obj_id: &str, entry: &str, depth: u8) -> HsResult<()> {
    let obj_id = normalize_obj_id(obj_id);
    conn.execute("INSERT OR IGNORE INTO object_grants(obj_id, entry) VALUES (?1, ?2)", params![obj_id, entry])?;
    if let Some(obj) = get_object(conn, &obj_id)? {
        if obj.obj_type == OBJ_TYPE_FILE {
            if let Some(content) = obj.body.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    conn.execute("INSERT OR IGNORE INTO object_grants(obj_id, entry) VALUES (?1, ?2)", params![normalize_obj_id(content), entry])?;
                }
            }
        } else if obj.obj_type == OBJ_TYPE_FEED && depth < 2 {
            if let Ok(feed) = serde_json::from_value::<FeedObject>(obj.body.clone()) {
                for part in feed.content_parts() {
                    grant(conn, &part, entry, depth + 1)?;
                }
                if let Some(entry_of_inner) = &feed.entry {
                    if let Some(head) = get_head(conn, entry_of_inner)? {
                        conn.execute("INSERT OR IGNORE INTO object_grants(obj_id, entry) VALUES (?1, ?2)", params![head.head_obj_id, entry])?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn append_change(conn: &Connection, entry: &str, kind: &str, seq: u64, now: i64) -> HsResult<i64> {
    conn.execute(
        "INSERT INTO stream_changes(entry, kind, seq, at) VALUES (?1, ?2, ?3, ?4)",
        params![entry, kind, seq as i64, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn store_head_row(conn: &Connection, head: &FeedHead, head_obj_id: &str, restricted: bool, now: i64) -> HsResult<()> {
    conn.execute(
        "INSERT INTO heads(entry, publisher, seq, state, current, head_obj_id, conflict_obj_id, restricted, updated_at_ms, verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9)
         ON CONFLICT(entry) DO UPDATE SET publisher=excluded.publisher, seq=excluded.seq, state=excluded.state,
           current=excluded.current, head_obj_id=excluded.head_obj_id, conflict_obj_id=NULL,
           restricted=excluded.restricted, updated_at_ms=excluded.updated_at_ms, verified_at=excluded.verified_at",
        params![
            head.entry,
            head.publisher,
            head.seq as i64,
            head.state.as_str(),
            head.current,
            head_obj_id,
            restricted as i64,
            head.updated_at_ms as i64,
            now
        ],
    )?;
    Ok(())
}

/// Next Head of an owner entry: the seq is assigned under the database lock, so Heads of one
/// entry are strictly serial (§16.5).
fn write_own_head(
    conn: &Connection,
    signer: &Signer,
    owner: &str,
    entry: &str,
    state: HeadState,
    current: Option<&str>,
    restricted: bool,
    now: i64,
) -> HsResult<(String, u64)> {
    let last: Option<i64> = conn.query_row("SELECT MAX(seq) FROM head_history WHERE entry=?1", [entry], |r| r.get(0))?;
    let seq = last.unwrap_or(0) as u64 + 1;
    let head = FeedHead {
        kind: HEAD_KIND.into(),
        publisher: owner.to_string(),
        entry: entry.to_string(),
        seq,
        state,
        current: current.map(str::to_string),
        updated_at_ms: now as u64,
    };
    let claims = serde_json::to_value(&head)?;
    let jwt = signer.sign(&claims).map_err(HsError::Internal)?;
    let (head_obj_id, _) = obj_id_of(OBJ_TYPE_HEAD, &claims).map_err(HsError::Internal)?;
    put_object(
        conn,
        &StoredObject {
            obj_id: head_obj_id.clone(),
            obj_type: OBJ_TYPE_HEAD.into(),
            body: claims,
            jwt: Some(jwt),
            signer: Some(signer.signer_did.clone()),
            publisher: Some(owner.to_string()),
            verified: true,
            local_only: false,
        },
        Some("self"),
        now,
    )?;
    conn.execute(
        "INSERT INTO head_history(entry, seq, head_obj_id, state, current, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![entry, seq as i64, head_obj_id, state.as_str(), current, now],
    )?;
    store_head_row(conn, &head, &head_obj_id, restricted, now)?;
    conn.execute("INSERT OR IGNORE INTO object_grants(obj_id, entry) VALUES (?1, ?2)", params![head_obj_id, entry])?;
    append_change(conn, entry, "head", seq, now)?;
    conn.execute("UPDATE published SET updated_at=?2 WHERE entry=?1", params![entry, now])?;
    Ok((head_obj_id, seq))
}

#[derive(Debug, Clone, Serialize)]
pub struct Published {
    pub entry: String,
    pub obj_id: Option<String>,
    pub head_obj_id: String,
    pub seq: u64,
    pub state: HeadState,
}

pub struct NewVersion {
    pub entry: String,
    pub kind: EntryKind,
    /// Required when the entry does not exist yet; kept otherwise.
    pub audience: Option<AudienceSpec>,
    pub target: Option<String>,
    pub category: Option<String>,
    pub obj_type: &'static str,
    pub claims: Value,
}

impl Station {
    pub fn own_entry(&self, namespace: EntryNamespace, key: &str) -> String {
        entry_url(&self.cfg.zone, namespace, key)
    }

    pub fn sign_object(&self, obj_type: &'static str, claims: &Value) -> HsResult<StoredObject> {
        if obj_type == OBJ_TYPE_FEED {
            let feed: FeedObject = serde_json::from_value(claims.clone()).map_err(|e| bad(format!("invalid feed object: {e}")))?;
            validate_feed_object(&feed).map_err(bad)?;
        }
        let (obj_id, _) = obj_id_of(obj_type, claims).map_err(bad)?;
        let jwt = self.signer.sign(claims).map_err(HsError::Internal)?;
        Ok(StoredObject {
            obj_id,
            obj_type: obj_type.into(),
            body: claims.clone(),
            jwt: Some(jwt),
            signer: Some(self.signer.signer_did.clone()),
            publisher: Some(self.cfg.owner.clone()),
            verified: true,
            local_only: false,
        })
    }

    /// Create the entry with its first Head, or publish a new version on it (§16.4 编辑).
    pub async fn publish_version(&self, version: NewVersion) -> HsResult<Published> {
        let obj = self.sign_object(version.obj_type, &version.claims)?;
        let signer = self.signer.clone();
        let owner = self.cfg.owner.clone();
        let key = match EntryRef::parse(&version.entry).map_err(bad)? {
            EntryRef::Path { zone, namespace, key } => {
                if zone != self.cfg.zone {
                    return Err(bad("entry is not in this zone"));
                }
                (namespace, key)
            }
            EntryRef::Did(_) => return Err(bad("DID entries are published through their name document")),
        };
        let now = now_ms();
        let result = self
            .db
            .call(move |conn| {
                let tx = conn.transaction()?;
                let existing = get_entry(&tx, &version.entry)?;
                let audience = match (&existing, version.audience) {
                    (Some(row), _) => row.audience.clone(),
                    (None, Some(audience)) => audience,
                    (None, None) => return Err(bad("a new entry needs an audience")),
                };
                if existing.is_none() {
                    tx.execute(
                        "INSERT INTO published(entry, key, namespace, kind, audience, category, target, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                        params![
                            version.entry,
                            key.1,
                            key.0.as_str(),
                            version.kind.as_str(),
                            serde_json::to_string(&audience)?,
                            version.category,
                            version.target,
                            now
                        ],
                    )?;
                }
                put_object(&tx, &obj, Some("self"), now)?;
                if obj.obj_type == OBJ_TYPE_FEED {
                    let feed: FeedObject = serde_json::from_value(obj.body.clone())?;
                    index_feed(&tx, &obj.obj_id, &feed, true)?;
                }
                add_version(&tx, &version.entry, &obj.obj_id, now)?;
                grant(&tx, &obj.obj_id, &version.entry, 0)?;
                let restricted = !audience.is_public();
                let (head_obj_id, seq) =
                    write_own_head(&tx, &signer, &owner, &version.entry, HeadState::Active, Some(&obj.obj_id), restricted, now)?;
                tx.commit()?;
                Ok(Published { entry: version.entry, obj_id: Some(obj.obj_id), head_obj_id, seq, state: HeadState::Active })
            })
            .await?;
        self.bump(&["published", "profile"]);
        Ok(result)
    }

    /// A new Head on an existing owner entry: withdraw it, or point it at a known version
    /// again (E16 再次点赞).
    pub async fn set_entry_state(&self, entry: &str, state: HeadState, current: Option<String>) -> HsResult<Published> {
        let signer = self.signer.clone();
        let owner = self.cfg.owner.clone();
        let entry = entry.to_string();
        let now = now_ms();
        let result = self
            .db
            .call(move |conn| {
                let tx = conn.transaction()?;
                let row = get_entry(&tx, &entry)?.ok_or_else(|| HsError::NotFound("no such entry".into()))?;
                if let Some(current) = &current {
                    if !entry_versions(&tx, &entry)?.contains(current) {
                        return Err(bad("the Head may only point at a version of this entry"));
                    }
                }
                let (head_obj_id, seq) = write_own_head(&tx, &signer, &owner, &entry, state, current.as_deref(), !row.audience.is_public(), now)?;
                tx.commit()?;
                Ok(Published { entry, obj_id: current, head_obj_id, seq, state })
            })
            .await?;
        self.bump(&["published", "profile", "comments", "saved"]);
        Ok(result)
    }

    /// Audience is an entry policy: changing it issues no Head, only a stream change (§4.5).
    pub async fn set_audience(&self, entry: &str, audience: AudienceSpec) -> HsResult<()> {
        audience.validate().map_err(bad)?;
        let entry = entry.to_string();
        let entry2 = entry.clone();
        let now = now_ms();
        let audience2 = audience.clone();
        let (seq, versions) = self
            .db
            .call(move |conn| {
                let tx = conn.transaction()?;
                let row = get_entry(&tx, &entry2)?.ok_or_else(|| HsError::NotFound("no such entry".into()))?;
                if row.kind.is_reaction() || row.kind == EntryKind::Follow || row.kind == EntryKind::Repost {
                    return Err(bad("the audience of interactions follows their target"));
                }
                tx.execute(
                    "UPDATE published SET audience=?2, updated_at=?3 WHERE entry=?1",
                    params![entry2, serde_json::to_string(&audience2)?, now],
                )?;
                tx.execute("UPDATE heads SET restricted=?2 WHERE entry=?1", params![entry2, (!audience2.is_public()) as i64])?;
                let head = get_head(&tx, &entry2)?.ok_or_else(|| HsError::Internal("entry without head".into()))?;
                append_change(&tx, &entry2, "audience", head.seq, now)?;
                let versions = entry_versions(&tx, &entry2)?;
                tx.commit()?;
                Ok((head, versions))
            })
            .await?;
        self.bump(&["published", "profile"]);
        if seq.state == HeadState::Active {
            let recipients = self.audience_recipients(&audience).await;
            let mut objects = versions.last().cloned().into_iter().collect::<Vec<_>>();
            objects.push(seq.head_obj_id.clone());
            self.enqueue_delivery(Some(&entry), &objects, &recipients, None, !audience.is_public()).await?;
        }
        Ok(())
    }

    /// Who may currently read an entry with this audience and is known to us (Push targets).
    pub async fn audience_recipients(&self, audience: &AudienceSpec) -> Vec<String> {
        let contacts = self.contacts.list().await;
        let followers = self.active_followers().await.unwrap_or_default();
        let friends: Vec<String> = contacts.iter().filter(|c| c.friend && !c.blocked).map(|c| c.did.clone()).collect();
        let mut out: Vec<String> = match audience {
            AudienceSpec::Public | AudienceSpec::Followers => followers.into_iter().chain(friends).collect(),
            AudienceSpec::Friends => friends,
            AudienceSpec::Group { group_id } => {
                contacts.iter().filter(|c| !c.blocked && c.groups.iter().any(|g| g == group_id)).map(|c| c.did.clone()).collect()
            }
            AudienceSpec::Dids { dids } => dids.clone(),
        };
        out.retain(|d| d != &self.cfg.owner);
        out.sort();
        out.dedup();
        out
    }

    pub async fn active_followers(&self) -> HsResult<Vec<String>> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare("SELECT did FROM followers WHERE state='active'")?;
                let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn collector_dids(&self) -> Vec<String> {
        self.settings().await.map(|s| s.collectors.into_iter().map(|c| c.did).collect()).unwrap_or_default()
    }

    /// Everyone who received an object of this entry before: edits and withdrawals follow the
    /// known propagation paths (§13.5).
    pub async fn previous_recipients(&self, entry: &str) -> HsResult<Vec<String>> {
        let entry = entry.to_string();
        self.db
            .call(move |c| {
                let mut stmt = c.prepare("SELECT DISTINCT recipient FROM outbox WHERE entry=?1")?;
                let rows = stmt.query_map([entry], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn withdraw(&self, entry: &str) -> HsResult<Option<Published>> {
        let entry_s = entry.to_string();
        let head = self.db.call(move |c| get_head(c, &entry_s)).await?;
        let Some(head) = head else { return Err(HsError::NotFound("no such entry".into())) };
        if head.publisher != self.cfg.owner {
            return Err(HsError::Forbidden("only the entry controller can withdraw it (§16.4)".into()));
        }
        if head.state == HeadState::Withdrawn {
            return Ok(None);
        }
        let published = self.set_entry_state(entry, HeadState::Withdrawn, None).await?;
        let mut recipients = self.previous_recipients(entry).await?;
        let entry_s = entry.to_string();
        let row = self.db.call(move |c| get_entry(c, &entry_s)).await?;
        if let Some(row) = &row {
            recipients.extend(self.audience_recipients(&row.audience).await);
            if row.audience.is_public() {
                recipients.extend(self.collector_dids().await);
            }
        }
        recipients.sort();
        recipients.dedup();
        let restricted = row.is_some_and(|r| !r.audience.is_public());
        self.enqueue_delivery(Some(entry), std::slice::from_ref(&published.head_obj_id), &recipients, None, restricted).await?;
        self.after_own_state_change(entry).await?;
        Ok(Some(published))
    }

    /// Edit the text of a post, comment or quote: a new version declaring the same entry,
    /// `base_on` the previous one (E14).
    pub async fn edit(&self, entry: &str, text: &str) -> HsResult<Published> {
        let text = text.trim().to_string();
        if text.is_empty() || text.chars().count() > 2000 {
            return Err(bad("text must be 1–2000 characters"));
        }
        let entry_s = entry.to_string();
        let (row, head) = self.db.call(move |c| Ok((get_entry(c, &entry_s)?, get_head(c, &entry_s)?))).await?;
        let row = row.ok_or_else(|| HsError::NotFound("no such entry".into()))?;
        let head = head.ok_or_else(|| HsError::NotFound("no such entry".into()))?;
        if head.state == HeadState::Withdrawn {
            return Err(HsError::Conflict("a withdrawn entry cannot be edited".into()));
        }
        if !matches!(row.kind, EntryKind::Post | EntryKind::Comment | EntryKind::Quote) {
            return Err(bad("only posts, comments and quotes have text to edit"));
        }
        let current = head.current.clone().ok_or_else(|| HsError::Internal("active head without version".into()))?;
        let current2 = current.clone();
        let previous = self
            .db
            .call(move |c| crate::objects::get_feed(c, &current2))
            .await?
            .ok_or_else(|| HsError::Internal("current version missing".into()))?;
        let mut next = previous.clone();
        next.iat = now_s();
        next.nonce = Some(new_id("n"));
        next.base_on = Some(current);
        match next.content.as_mut() {
            Some(content) => content.text = Some(text),
            None => {
                next.content = Some(FeedContent {
                    content_type: ContentType::Text,
                    text: Some(text),
                    title: None,
                    summary: None,
                    cover: None,
                    media: vec![],
                })
            }
        }
        let published = self
            .publish_version(NewVersion {
                entry: entry.to_string(),
                kind: row.kind,
                audience: None,
                target: row.target.clone(),
                category: row.category.clone(),
                obj_type: OBJ_TYPE_FEED,
                claims: next.to_value(),
            })
            .await?;
        let mut recipients = self.previous_recipients(entry).await?;
        recipients.extend(self.audience_recipients(&row.audience).await);
        recipients.sort();
        recipients.dedup();
        let objects = vec![published.obj_id.clone().unwrap(), published.head_obj_id.clone()];
        self.enqueue_delivery(Some(entry), &objects, &recipients, None, !row.audience.is_public()).await?;
        self.bump(&["comments", "reading"]);
        Ok(published)
    }

    /// Bookkeeping after one of the owner's own entries changed state (personal state of
    /// interactions, author list of comments).
    pub async fn after_own_state_change(&self, entry: &str) -> HsResult<()> {
        let entry = entry.to_string();
        self.db
            .call(move |c| {
                let Some(row) = get_entry(c, &entry)? else { return Ok(()) };
                let Some(head) = get_head(c, &entry)? else { return Ok(()) };
                if head.state == HeadState::Withdrawn && row.kind == EntryKind::Bookmark {
                    if let Some(target) = &row.target {
                        c.execute("UPDATE personal SET bookmark_entry=NULL WHERE obj_id=?1 AND bookmark_entry=?2", params![target, entry])?;
                    }
                }
                Ok(())
            })
            .await?;
        self.bump(&["saved", "comments"]);
        Ok(())
    }
}

/* ── publish tasks (§9.7): idempotent per intent key ── */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub object: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkCard {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleInput {
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishInput {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub link: Option<LinkCard>,
    #[serde(default)]
    pub article: Option<ArticleInput>,
    pub audience: AudienceSpec,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl PublishInput {
    pub fn validate(&self) -> HsResult<()> {
        self.audience.validate().map_err(bad)?;
        if self.text.trim().chars().count() > 2000 {
            return Err(bad("homestation.validation.textTooLong"));
        }
        if self.attachments.len() > MAX_MEDIA_PARTS {
            return Err(bad("homestation.validation.tooManyAttachments"));
        }
        if self.text.trim().is_empty() && self.attachments.is_empty() && self.link.is_none() && self.article.is_none() {
            return Err(bad("homestation.validation.empty"));
        }
        if self.attachments.iter().any(|a| a.status != "uploaded" || a.object.is_none()) {
            return Err(bad("homestation.validation.uploadPending"));
        }
        let count = |kind: &str| self.attachments.iter().filter(|a| a.kind == kind).count();
        if count("video") > 1 || count("audio") > 1 {
            return Err(bad("homestation.validation.oneVideoOrAudio"));
        }
        if let Some(link) = &self.link {
            if !is_http_url(&link.url) || link.title.trim().is_empty() || link.title.chars().count() > 120 || link.summary.chars().count() > 280 {
                return Err(bad("homestation.validation.url"));
            }
        }
        if let Some(category) = &self.category {
            if category != "work" && category != "product" {
                return Err(bad("category is work or product"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishTaskRow {
    pub key: String,
    pub stage: String,
    pub entry: Option<String>,
    pub obj_id: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
}

pub fn get_task(conn: &Connection, key: &str) -> HsResult<Option<(PublishTaskRow, String)>> {
    Ok(conn
        .query_row(
            "SELECT key, stage, entry, obj_id, error, created_at, input FROM publish_tasks WHERE key=?1",
            [key],
            |r| {
                Ok((
                    PublishTaskRow {
                        key: r.get(0)?,
                        stage: r.get(1)?,
                        entry: r.get(2)?,
                        obj_id: r.get(3)?,
                        error: r.get(4)?,
                        created_at: r.get(5)?,
                    },
                    r.get(6)?,
                ))
            },
        )
        .optional()?)
}

fn set_task(conn: &Connection, key: &str, stage: &str, entry: Option<&str>, obj_id: Option<&str>, error: Option<&str>, now: i64) -> HsResult<()> {
    conn.execute(
        "UPDATE publish_tasks SET stage=?2, entry=?3, obj_id=?4, error=?5, updated_at=?6 WHERE key=?1",
        params![key, stage, entry, obj_id, error, now],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum TaskSource {
    Input(PublishInput),
    Share { capture: String },
}

impl Station {
    /// Publish a new post for an intent key; the same key never produces a second post (A41).
    pub async fn publish(&self, key: &str, input: PublishInput) -> HsResult<PublishTaskRow> {
        input.validate()?;
        self.run_task(key, TaskSource::Input(input)).await
    }

    pub async fn retry_publish(&self, key: &str) -> HsResult<PublishTaskRow> {
        let key_s = key.to_string();
        let task = self.db.call(move |c| get_task(c, &key_s)).await?;
        let (row, input) = task.ok_or_else(|| HsError::NotFound("unknown_task".into()))?;
        if row.stage == "published" {
            return Ok(row);
        }
        let source: Value = serde_json::from_str(&input)?;
        let source = match source.get("share").and_then(Value::as_str) {
            Some(capture) => TaskSource::Share { capture: capture.to_string() },
            None => TaskSource::Input(serde_json::from_value(source)?),
        };
        self.run_task(key, source).await
    }

    async fn run_task(&self, key: &str, source: TaskSource) -> HsResult<PublishTaskRow> {
        if key.is_empty() || key.len() > 128 {
            return Err(bad("invalid publish key"));
        }
        let stored_input = match &source {
            TaskSource::Input(input) => serde_json::to_string(input)?,
            TaskSource::Share { capture } => json!({ "share": capture }).to_string(),
        };
        let key_s = key.to_string();
        let now = now_ms();
        let existing = self
            .db
            .call(move |c| {
                if let Some((row, _)) = get_task(c, &key_s)? {
                    if row.stage != "failed" {
                        return Ok(Some(row));
                    }
                    set_task(c, &key_s, "uploading", None, None, None, now)?;
                } else {
                    c.execute(
                        "INSERT INTO publish_tasks(key, stage, input, created_at, updated_at) VALUES (?1, 'uploading', ?2, ?3, ?3)",
                        params![key_s, stored_input, now],
                    )?;
                }
                Ok(None)
            })
            .await?;
        if let Some(row) = existing {
            return Ok(row);
        }
        self.bump(&["published"]);
        let outcome = match source {
            TaskSource::Input(input) => self.publish_input(&input).await,
            TaskSource::Share { capture } => self.publish_share(&capture).await,
        };
        let key_s = key.to_string();
        let now = now_ms();
        let row = self
            .db
            .call(move |c| {
                match &outcome {
                    Ok(p) => set_task(c, &key_s, "published", Some(&p.entry), p.obj_id.as_deref(), None, now)?,
                    Err(e) => set_task(c, &key_s, "failed", None, None, Some(e.message()), now)?,
                }
                Ok(get_task(c, &key_s)?.map(|(row, _)| row).unwrap())
            })
            .await?;
        self.bump(&["published", "profile"]);
        Ok(row)
    }

    async fn ensure_file(&self, obj_id: &str) -> HsResult<StoredObject> {
        match crate::objects::load_local(&self.db, self.chunks.as_ref(), obj_id).await? {
            Some(obj) if obj.obj_type == OBJ_TYPE_FILE => Ok(obj),
            _ => Err(HsError::Unavailable("upload_missing".into())),
        }
    }

    async fn publish_input(&self, input: &PublishInput) -> HsResult<Published> {
        for attachment in &input.attachments {
            self.ensure_file(attachment.object.as_deref().unwrap_or_default()).await?;
        }
        let text = input.text.trim();
        let text_opt = (!text.is_empty()).then(|| text.to_string());
        let mut obj = FeedObject {
            kind: FeedKind::Post,
            comment_type: None,
            publisher: self.cfg.owner.clone(),
            iat: now_s(),
            nonce: Some(new_id("n")),
            entry: None,
            content: None,
            wraps: None,
            references: vec![],
            tags: input.tags.clone(),
            source: None,
            link: None,
            publication_category: input.category.clone(),
            base_on: None,
        };
        let content = |content_type, text: Option<String>| FeedContent {
            content_type,
            text,
            title: None,
            summary: None,
            cover: None,
            media: vec![],
        };
        let video = input.attachments.iter().find(|a| a.kind == "video");
        let audio: Vec<&Attachment> = input.attachments.iter().filter(|a| a.kind == "audio").collect();
        let images: Vec<&Attachment> = input.attachments.iter().filter(|a| a.kind == "image").collect();
        if let Some(article) = &input.article {
            let file = self.store_text_file(&format!("{}.md", slug(&article.title)), "text/markdown", article.markdown.as_bytes()).await?;
            let mut c = content(ContentType::Article, text_opt.clone());
            c.title = Some(article.title.clone());
            c.summary = (!article.summary.is_empty()).then(|| article.summary.clone());
            obj.content = Some(c);
            obj.wraps = Some(file);
        } else if let Some(link) = &input.link {
            let content_type = if input.category.as_deref() == Some("product") { ContentType::Product } else { ContentType::Link };
            let mut c = content(content_type, text_opt.clone());
            c.title = Some(link.title.trim().to_string());
            c.summary = (!link.summary.trim().is_empty()).then(|| link.summary.trim().to_string());
            c.cover = link.cover.clone();
            obj.content = Some(c);
            obj.link = Some(link.url.clone());
        } else if let Some(video) = video {
            let mut c = content(ContentType::Video, text_opt.clone());
            c.title = text_opt.as_ref().map(|t| t.chars().take(80).collect());
            c.cover = images.first().and_then(|i| i.object.clone());
            obj.content = Some(c);
            obj.wraps = video.object.clone();
        } else if !audio.is_empty() {
            let mut c = content(ContentType::Audio, text_opt.clone());
            c.media = audio.iter().map(|a| MediaPart { object: a.object.clone().unwrap(), alt: None }).collect();
            obj.content = Some(c);
        } else if !images.is_empty() {
            let mut c = content(ContentType::Image, text_opt.clone());
            c.media = images.iter().map(|a| MediaPart { object: a.object.clone().unwrap(), alt: Some(a.name.clone()) }).collect();
            obj.content = Some(c);
        } else {
            obj.content = Some(content(ContentType::Text, text_opt.clone()));
        }
        let entry = self.own_entry(EntryNamespace::Feed, &new_id("p"));
        obj.entry = Some(entry.clone());
        self.publish_post_object(obj, input.audience.clone(), input.category.clone()).await
    }

    pub async fn publish_post_object(&self, obj: FeedObject, audience: AudienceSpec, category: Option<String>) -> HsResult<Published> {
        let entry = obj.entry.clone().ok_or_else(|| bad("post needs an entry"))?;
        let published = self
            .publish_version(NewVersion {
                entry: entry.clone(),
                kind: EntryKind::Post,
                audience: Some(audience.clone()),
                target: None,
                category,
                obj_type: OBJ_TYPE_FEED,
                claims: obj.to_value(),
            })
            .await?;
        let mut recipients = self.audience_recipients(&audience).await;
        if audience.is_public() {
            recipients.extend(self.collector_dids().await);
        }
        recipients.sort();
        recipients.dedup();
        let objects = vec![published.obj_id.clone().unwrap(), published.head_obj_id.clone()];
        self.enqueue_delivery(Some(&entry), &objects, &recipients, None, !audience.is_public()).await?;
        Ok(published)
    }

    /// Share a private capture (§5.6): publish the owner's own object wrapping the snapshot
    /// (or a link card), with its own entry.
    async fn publish_share(&self, capture_id: &str) -> HsResult<Published> {
        let id = normalize_obj_id(capture_id);
        let id2 = id.clone();
        let capture = self
            .db
            .call(move |c| Ok((get_object(c, &id2)?, crate::objects::get_feed(c, &id2)?)))
            .await?;
        let (Some(stored), Some(feed)) = capture else { return Err(HsError::NotFound("no such capture".into())) };
        if !stored.local_only || feed.source.is_none() {
            return Err(bad("only private captures can be shared"));
        }
        let audience = self.settings().await?.default_audience;
        let entry = self.own_entry(EntryNamespace::Feed, &new_id("clip"));
        let obj = FeedObject {
            kind: FeedKind::Post,
            comment_type: None,
            publisher: self.cfg.owner.clone(),
            iat: now_s(),
            nonce: Some(new_id("n")),
            entry: Some(entry.clone()),
            content: feed.content.clone(),
            wraps: feed.wraps.clone(),
            references: vec![],
            tags: feed.tags.clone(),
            source: feed.source.clone(),
            link: feed.link.clone(),
            publication_category: None,
            base_on: None,
        };
        let published = self.publish_post_object(obj, audience, None).await?;
        let shared = published.obj_id.clone().unwrap();
        let entry2 = entry.clone();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO capture_shares(capture_id, shared_id, entry) VALUES (?1, ?2, ?3)",
                    params![id, shared, entry2],
                )?;
                Ok(())
            })
            .await?;
        self.bump(&["reading"]);
        Ok(published)
    }

    pub async fn share_capture(&self, capture_id: &str) -> HsResult<PublishTaskRow> {
        let key = format!("share-{}", normalize_obj_id(capture_id));
        self.run_task(&key, TaskSource::Share { capture: capture_id.to_string() }).await
    }

    /// Store bytes as a single-chunk FileObject (`cyfile`), the content-reference form (§5.5).
    pub async fn store_text_file(&self, name: &str, mime: &str, data: &[u8]) -> HsResult<String> {
        let (obj_id, _) = self.store_file(name, mime, data.to_vec(), Value::Null).await?;
        Ok(obj_id)
    }

    pub async fn store_file(&self, name: &str, mime: &str, data: Vec<u8>, extra_meta: Value) -> HsResult<(String, Value)> {
        let chunk_id = crate::objects::chunk_id_of(&data)?;
        let size = data.len() as u64;
        self.chunks.put_chunk(&chunk_id, data).await?;
        let mut file = ndn_lib::FileObject::new(name.to_string(), size, chunk_id);
        file.meta.insert("mime".into(), json!(mime));
        if let Value::Object(map) = extra_meta {
            for (k, v) in map {
                if !v.is_null() {
                    file.meta.insert(k, v);
                }
            }
        }
        let body = serde_json::to_value(&file)?;
        let (obj_id, jcs) = obj_id_of(OBJ_TYPE_FILE, &body).map_err(bad)?;
        self.chunks.put_named_object(&obj_id, &jcs).await?;
        let stored = StoredObject {
            obj_id: obj_id.clone(),
            obj_type: OBJ_TYPE_FILE.into(),
            body: body.clone(),
            jwt: None,
            signer: None,
            publisher: None,
            verified: true,
            local_only: false,
        };
        let now = now_ms();
        self.db.call(move |c| put_object(c, &stored, Some("upload"), now)).await?;
        Ok((obj_id, body))
    }
}

fn slug(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    if s.is_empty() {
        "article".into()
    } else {
        s
    }
}
