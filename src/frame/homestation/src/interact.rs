//! Comments and interactions as Feed Objects (§11.1, §12): likes, public bookmarks and reposts
//! live on interaction-key entries; text comments and quotes are independent publications;
//! private bookmarks, read-later and dislikes stay local.

use crate::audience::AudienceSpec;
use crate::error::{bad, HsError, HsResult};
use crate::objects::{get_feed, get_object, put_object, StoredObject};
use crate::protocol::*;
use crate::publish::{entry_versions, get_entry, get_head, EntryKind, NewVersion, Published};
use crate::{new_id, now_ms, now_s, Station};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub obj_id: String,
    pub publisher: String,
    pub restricted: bool,
    pub withdrawn: bool,
    pub private_capture: bool,
    pub reaction: bool,
    pub entry: Option<String>,
}

impl Station {
    pub async fn target_info(&self, obj_id: &str) -> HsResult<TargetInfo> {
        let id = normalize_obj_id(obj_id);
        let owner = self.cfg.owner.clone();
        self.db
            .call(move |c| {
                let stored = get_object(c, &id)?.ok_or_else(|| HsError::NotFound("unknown object".into()))?;
                let feed: FeedObject = serde_json::from_value(stored.body.clone()).map_err(|_| bad("not a feed object"))?;
                let (restricted, withdrawn) = match &feed.entry {
                    Some(entry) => {
                        let head = get_head(c, entry)?;
                        let own = get_entry(c, entry)?;
                        let restricted = match (&own, &head) {
                            (Some(row), _) if feed.publisher == owner => !row.audience.is_public(),
                            (_, Some(h)) => h.restricted,
                            _ => false,
                        };
                        (restricted, head.is_some_and(|h| h.state == HeadState::Withdrawn))
                    }
                    None => (false, false),
                };
                Ok(TargetInfo {
                    obj_id: id,
                    publisher: feed.publisher.clone(),
                    restricted,
                    withdrawn,
                    private_capture: stored.local_only,
                    reaction: matches!(feed.comment_type, Some(CommentType::Like | CommentType::Bookmark | CommentType::Dislike)),
                    entry: feed.entry.clone(),
                })
            })
            .await
    }

    /// Recipients of a comment or interaction on `target` (§13.1): the author always; when the
    /// target is public also participants, the comment's audience and the collectors.
    async fn interaction_recipients(&self, target: &TargetInfo, audience: &AudienceSpec, kind: EntryKind) -> HsResult<Vec<String>> {
        let mut recipients = vec![target.publisher.clone()];
        if !target.restricted {
            if matches!(kind, EntryKind::Comment | EntryKind::Quote) {
                let t = target.obj_id.clone();
                let participants: Vec<String> = self
                    .db
                    .call(move |c| {
                        let mut stmt = c.prepare("SELECT did FROM participants WHERE target=?1 LIMIT 50")?;
                        let rows = stmt.query_map([t], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                        Ok(rows)
                    })
                    .await?;
                recipients.extend(participants);
            }
            if matches!(kind, EntryKind::Comment | EntryKind::Quote | EntryKind::Repost) {
                recipients.extend(self.audience_recipients(audience).await);
            }
            if audience.is_public() {
                recipients.extend(self.collector_dids().await);
            }
        }
        recipients.retain(|r| r != &self.cfg.owner);
        recipients.sort();
        recipients.dedup();
        Ok(recipients)
    }

    /// Switch an interaction-key entry on or off (§12.4, E16): the first "on" publishes the
    /// object, later ones point a new Head at the first version again.
    async fn toggle(&self, target: &TargetInfo, comment_type: CommentType, on: bool) -> HsResult<Option<Published>> {
        let key = reaction_key(&self.cfg.owner, &target.obj_id, comment_type);
        let entry = self.own_entry(EntryNamespace::Reactions, &key);
        let entry2 = entry.clone();
        let (row, head, versions) = self.db.call(move |c| Ok((get_entry(c, &entry2)?, get_head(c, &entry2)?, entry_versions(c, &entry2)?))).await?;
        let audience = if target.restricted { AudienceSpec::only(&target.publisher) } else { AudienceSpec::Public };
        let kind = EntryKind::of_comment(comment_type);
        let published = match (row, head, on) {
            (None, _, false) => return Ok(None),
            (None, _, true) => {
                let mut obj = FeedObject {
                    kind: FeedKind::Comment,
                    comment_type: Some(comment_type),
                    publisher: self.cfg.owner.clone(),
                    iat: now_s(),
                    nonce: None,
                    entry: Some(entry.clone()),
                    content: None,
                    wraps: None,
                    references: vec![],
                    tags: vec![],
                    source: None,
                    link: None,
                    publication_category: None,
                    base_on: None,
                };
                if comment_type == CommentType::Repost {
                    obj.wraps = Some(target.obj_id.clone());
                } else {
                    obj.references = vec![FeedReference { relation: "comment_on".into(), object_id: target.obj_id.clone() }];
                }
                self.publish_version(NewVersion {
                    entry: entry.clone(),
                    kind,
                    audience: Some(audience.clone()),
                    target: Some(target.obj_id.clone()),
                    category: None,
                    obj_type: OBJ_TYPE_FEED,
                    claims: obj.to_value(),
                })
                .await?
            }
            (Some(_), Some(h), true) if h.state == HeadState::Active => return Ok(None),
            (Some(_), _, true) => self.set_entry_state(&entry, HeadState::Active, versions.first().cloned()).await?,
            (Some(_), Some(h), false) if h.state == HeadState::Withdrawn => return Ok(None),
            (Some(_), _, false) => self.set_entry_state(&entry, HeadState::Withdrawn, None).await?,
        };
        let mut recipients = self.interaction_recipients(target, &audience, kind).await?;
        recipients.extend(self.previous_recipients(&entry).await?);
        recipients.sort();
        recipients.dedup();
        let mut objects = Vec::new();
        if let (Some(obj), true) = (&published.obj_id, published.seq == 1) {
            objects.push(obj.clone());
        }
        if published.seq > 1 && on {
            if let Some(first) = versions.first() {
                objects.push(first.clone());
            }
        }
        objects.push(published.head_obj_id.clone());
        self.enqueue_delivery(Some(&entry), &objects, &recipients, None, target.restricted).await?;
        if target.publisher != self.cfg.owner {
            self.track(&target.obj_id, &target.publisher, comment_type.as_str()).await?;
        }
        Ok(Some(published))
    }

    async fn set_personal(&self, obj_id: &str, column: &'static str, value: Value) -> HsResult<()> {
        let obj_id = obj_id.to_string();
        let now = now_ms();
        self.db
            .call(move |c| {
                c.execute("INSERT OR IGNORE INTO personal(obj_id, updated_at) VALUES (?1, ?2)", params![obj_id, now])?;
                let sql = format!("UPDATE personal SET {column}=?2, updated_at=?3 WHERE obj_id=?1");
                let v: Box<dyn rusqlite::ToSql> = match value {
                    Value::Null => Box::new(Option::<String>::None),
                    Value::Bool(b) => Box::new(b as i64),
                    Value::Number(n) => Box::new(n.as_i64().unwrap_or(0)),
                    Value::String(s) => Box::new(s),
                    other => Box::new(other.to_string()),
                };
                c.execute(&sql, params![obj_id, v, now])?;
                Ok(())
            })
            .await
    }

    pub async fn personal(&self, obj_id: &str) -> HsResult<Value> {
        let id = normalize_obj_id(obj_id);
        self.db.call(move |c| crate::projection::personal_state(c, &id)).await
    }

    /// Likes are public by default; on restricted targets only the author gets them (§4.5).
    pub async fn set_like(&self, obj_id: &str, on: bool) -> HsResult<Value> {
        let target = self.target_info(obj_id).await?;
        if on && target.reaction {
            return Err(bad("interactions cannot be liked"));
        }
        let key = reaction_key(&self.cfg.owner, &target.obj_id, CommentType::Like);
        let entry = self.own_entry(EntryNamespace::Reactions, &key);
        self.toggle(&target, CommentType::Like, on).await?;
        self.set_personal(&target.obj_id, "like_entry", json!(entry)).await?;
        if on && !target.restricted {
            self.update_settings(|s| s.like_notice_shown = true).await?;
        }
        self.bump(&["published", "comments", "reading"]);
        self.personal(&target.obj_id).await
    }

    /// Bookmarks are private records (E11); `public` additionally publishes the interaction-key
    /// entry, and going back to private withdraws it while keeping the bookmark.
    pub async fn set_bookmark(&self, obj_id: &str, on: bool, public: bool) -> HsResult<Value> {
        let target = self.target_info(obj_id).await?;
        let id = target.obj_id.clone();
        let existing: Option<(Option<String>, Option<String>)> = self
            .db
            .call(move |c| Ok(c.query_row("SELECT bookmark_private, bookmark_entry FROM personal WHERE obj_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?))
            .await?;
        let (private_obj, _) = existing.unwrap_or((None, None));
        if on && private_obj.is_none() {
            let obj = FeedObject {
                kind: FeedKind::Comment,
                comment_type: Some(CommentType::Bookmark),
                publisher: self.cfg.owner.clone(),
                iat: now_s(),
                nonce: Some(new_id("n")),
                entry: None,
                content: None,
                wraps: None,
                references: vec![FeedReference { relation: "comment_on".into(), object_id: target.obj_id.clone() }],
                tags: vec![],
                source: None,
                link: None,
                publication_category: None,
                base_on: None,
            };
            let body = obj.to_value();
            let (bid, _) = obj_id_of(OBJ_TYPE_FEED, &body).map_err(bad)?;
            let stored = StoredObject {
                obj_id: bid.clone(),
                obj_type: OBJ_TYPE_FEED.into(),
                body,
                jwt: None,
                signer: None,
                publisher: Some(self.cfg.owner.clone()),
                verified: true,
                local_only: true,
            };
            let now = now_ms();
            self.db.call(move |c| put_object(c, &stored, Some("private"), now)).await?;
            self.set_personal(&target.obj_id, "bookmark_private", json!(bid)).await?;
        }
        self.set_personal(&target.obj_id, "bookmark_on", json!(on)).await?;
        self.set_personal(&target.obj_id, "saved_at", if on { json!(now_ms()) } else { Value::Null }).await?;
        let make_public = on && public;
        let key = reaction_key(&self.cfg.owner, &target.obj_id, CommentType::Bookmark);
        let entry = self.own_entry(EntryNamespace::Reactions, &key);
        let entry2 = entry.clone();
        let entry_exists = self.db.call(move |c| Ok(get_entry(c, &entry2)?.is_some())).await?;
        if make_public || entry_exists {
            self.toggle(&target, CommentType::Bookmark, make_public).await?;
            self.set_personal(&target.obj_id, "bookmark_entry", json!(entry)).await?;
        }
        self.bump(&["saved", "published", "comments"]);
        self.personal(&target.obj_id).await
    }

    pub async fn set_read_later(&self, obj_id: &str, on: bool) -> HsResult<Value> {
        let id = normalize_obj_id(obj_id);
        self.set_personal(&id, "read_later", json!(on)).await?;
        self.set_personal(&id, "read_later_at", if on { json!(now_ms()) } else { Value::Null }).await?;
        self.bump(&["saved"]);
        self.personal(&id).await
    }

    // 待确认（TODO §13）：点踩语义未定，暂只作本地反馈，不发表、不计数
    pub async fn set_dislike(&self, obj_id: &str, on: bool) -> HsResult<Value> {
        let id = normalize_obj_id(obj_id);
        self.set_personal(&id, "dislike", json!(on)).await?;
        if on {
            self.record_event(&id, "dislike", None).await?;
        }
        self.bump(&["saved"]);
        self.personal(&id).await
    }

    pub async fn check_repostable(&self, target: &TargetInfo) -> HsResult<()> {
        let reason = if target.reaction {
            Some("reaction")
        } else if target.private_capture {
            Some("private_capture")
        } else if target.withdrawn {
            Some("withdrawn")
        } else if target.restricted {
            Some("restricted")
        } else {
            None
        };
        match reason {
            Some(r) => Err(HsError::Forbidden(format!("repost_blocked:{r}"))),
            None => Ok(()),
        }
    }

    /// Plain repost: a toggle on the repost interaction key, wrapping the exact version (E12).
    pub async fn repost(&self, obj_id: &str, on: bool) -> HsResult<Value> {
        let target = self.target_info(obj_id).await?;
        if on {
            self.check_repostable(&target).await?;
        }
        let key = reaction_key(&self.cfg.owner, &target.obj_id, CommentType::Repost);
        let entry = self.own_entry(EntryNamespace::Reactions, &key);
        self.toggle(&target, CommentType::Repost, on).await?;
        self.set_personal(&target.obj_id, "repost_entry", json!(entry)).await?;
        self.bump(&["published", "comments", "profile"]);
        self.personal(&target.obj_id).await
    }

    /// Quote: an independent publication wrapping the target, with its own entry (E13).
    pub async fn quote(&self, obj_id: &str, text: &str, audience: AudienceSpec) -> HsResult<Published> {
        let target = self.target_info(obj_id).await?;
        self.check_repostable(&target).await?;
        audience.validate().map_err(bad)?;
        let text = text.trim();
        if text.is_empty() || text.chars().count() > 2000 {
            return Err(bad("homestation.validation.commentEmpty"));
        }
        let entry = self.own_entry(EntryNamespace::Feed, &new_id("q"));
        let obj = FeedObject {
            kind: FeedKind::Comment,
            comment_type: Some(CommentType::Quote),
            publisher: self.cfg.owner.clone(),
            iat: now_s(),
            nonce: Some(new_id("n")),
            entry: Some(entry.clone()),
            content: Some(FeedContent { content_type: ContentType::Text, text: Some(text.to_string()), title: None, summary: None, cover: None, media: vec![] }),
            wraps: Some(target.obj_id.clone()),
            references: vec![],
            tags: vec![],
            source: None,
            link: None,
            publication_category: None,
            base_on: None,
        };
        let published = self
            .publish_version(NewVersion {
                entry: entry.clone(),
                kind: EntryKind::Quote,
                audience: Some(audience.clone()),
                target: Some(target.obj_id.clone()),
                category: None,
                obj_type: OBJ_TYPE_FEED,
                claims: obj.to_value(),
            })
            .await?;
        let recipients = self.interaction_recipients(&target, &audience, EntryKind::Quote).await?;
        let objects = vec![published.obj_id.clone().unwrap(), published.head_obj_id.clone()];
        self.enqueue_delivery(Some(&entry), &objects, &recipients, None, !audience.is_public()).await?;
        if target.publisher != self.cfg.owner {
            self.track(&target.obj_id, &target.publisher, "quote").await?;
        }
        self.bump(&["comments", "profile"]);
        Ok(published)
    }

    /// Text comment: the commenter's own publication; on a restricted target its audience is
    /// fixed to the author (§4.5, A71).
    pub async fn comment(&self, obj_id: &str, text: &str) -> HsResult<(Published, AudienceSpec)> {
        let target = self.target_info(obj_id).await?;
        let text = text.trim();
        if text.is_empty() || text.chars().count() > 1000 {
            return Err(bad("homestation.validation.commentEmpty"));
        }
        if target.withdrawn {
            return Err(HsError::Conflict("the target was withdrawn".into()));
        }
        let audience = if target.restricted {
            AudienceSpec::only(&target.publisher)
        } else {
            self.settings().await?.default_audience
        };
        let audience = if target.publisher == self.cfg.owner && target.restricted {
            let entry = target.entry.clone().unwrap_or_default();
            self.db.call(move |c| Ok(get_entry(c, &entry)?.map(|r| r.audience))).await?.unwrap_or(audience)
        } else {
            audience
        };
        let entry = self.own_entry(EntryNamespace::Feed, &new_id("c"));
        let obj = FeedObject {
            kind: FeedKind::Comment,
            comment_type: Some(CommentType::Text),
            publisher: self.cfg.owner.clone(),
            iat: now_s(),
            nonce: Some(new_id("n")),
            entry: Some(entry.clone()),
            content: Some(FeedContent { content_type: ContentType::Text, text: Some(text.to_string()), title: None, summary: None, cover: None, media: vec![] }),
            wraps: None,
            references: vec![FeedReference { relation: "comment_on".into(), object_id: target.obj_id.clone() }],
            tags: vec![],
            source: None,
            link: None,
            publication_category: None,
            base_on: None,
        };
        let published = self
            .publish_version(NewVersion {
                entry: entry.clone(),
                kind: EntryKind::Comment,
                audience: Some(audience.clone()),
                target: Some(target.obj_id.clone()),
                category: None,
                obj_type: OBJ_TYPE_FEED,
                claims: obj.to_value(),
            })
            .await?;
        let recipients = self.interaction_recipients(&target, &audience, EntryKind::Comment).await?;
        let objects = vec![published.obj_id.clone().unwrap(), published.head_obj_id.clone()];
        self.enqueue_delivery(Some(&entry), &objects, &recipients, None, !audience.is_public()).await?;
        if target.publisher != self.cfg.owner {
            self.track(&target.obj_id, &target.publisher, "comment").await?;
        }
        self.bump(&["comments", "profile"]);
        Ok((published, audience))
    }

    /// Bookmarks and read-later, with the state of their targets (§17.4).
    pub async fn list_saved(&self, kind: &str) -> HsResult<Value> {
        let column = if kind == "bookmark" { "saved_at" } else { "read_later_at" };
        let flag = if kind == "bookmark" { "bookmark_on" } else { "read_later" };
        let sql = format!("SELECT obj_id, {column} FROM personal WHERE {flag}=1 AND {column} IS NOT NULL ORDER BY {column} DESC");
        let bookmark = kind == "bookmark";
        self.db
            .call(move |c| {
                let mut stmt = c.prepare(&sql)?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                let mut out = Vec::new();
                for (obj_id, saved_at) in rows {
                    let feed = get_feed(c, &obj_id)?;
                    let state = match feed.as_ref().and_then(|f| crate::projection::entry_state(c, &obj_id, f).ok().flatten()) {
                        Some(s) if s["state"] == "withdrawn" => "withdrawn",
                        Some(s) if s["isLatest"] == false => "updated",
                        _ => "active",
                    };
                    let personal = crate::projection::personal_state(c, &obj_id)?;
                    let visibility = if bookmark { personal["bookmark"]["visibility"].clone() } else { json!("private") };
                    out.push(json!({ "objId": obj_id, "savedAt": saved_at, "visibility": visibility, "targetState": state }));
                }
                Ok(json!(out))
            })
            .await
    }
}
