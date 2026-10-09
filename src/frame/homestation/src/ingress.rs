//! Social Ingress (§7.3, §7.4) and the receiver side of entries and Heads (§16.6), shared by
//! Push (inbox dispatch) and Pull (stream reads, comment views, object fetches).

use crate::error::{bad, HsError, HsResult};
use crate::objects::{entry_belongs_to, get_object, index_feed, put_object, verify_jwt, StoredObject, Verified, VerifyError};
use crate::protocol::*;
use crate::publish::{add_version, get_entry, get_head, store_head_row, HeadRow};
use crate::{now_ms, Station};
use ndn_lib::{normalize_cyfs_dispatch_target, validate_cyfs_dispatch_body, CyfsDispatchResult, CyfsDispatchStatus, CyfsNamedObjectEncoding};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How an object reached this node; kept as candidate source path and comment source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum Arrival {
    Push { sender: String },
    Pull { source_id: String, label: String },
    AuthorList { author: String },
    Collector { collector: String },
    Fetch,
    Local,
}

impl Arrival {
    fn comment_source(&self) -> Option<(&'static str, String)> {
        match self {
            Self::Push { sender } => Some(("push", sender.clone())),
            Self::Pull { label, .. } => Some(("pull", label.clone())),
            Self::AuthorList { author } => Some(("author_list", author.clone())),
            Self::Collector { collector } => Some(("collector", collector.clone())),
            Self::Fetch => Some(("fetch", String::new())),
            Self::Local => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HeadApply {
    New,
    Unchanged,
    Stale,
    Conflict,
    Ignored,
}

#[derive(Debug, Clone, Default)]
pub struct Relation {
    pub friend: bool,
    pub blocked: bool,
    pub followed: bool,
    pub follower: bool,
    pub collector: bool,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DispatchReply {
    #[serde(skip)]
    pub http_status: u16,
    #[serde(flatten)]
    pub result: CyfsDispatchResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admission: Option<String>,
}

fn reply(status: u16, result: CyfsDispatchResult, admission: Option<String>) -> DispatchReply {
    DispatchReply { http_status: status, result, admission }
}

fn rejected(status: u16, id: Option<ndn_lib::ObjId>, target: &str, reason: &str, retryable: bool) -> DispatchReply {
    reply(status, CyfsDispatchResult::rejected(id, target.to_string(), reason, retryable), None)
}

pub fn is_followed_source(conn: &Connection, did: &str) -> HsResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM sources WHERE did=?1 AND basis!='[]' AND paused=0",
            [did],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
}

fn own_version(conn: &Connection, obj_id: &str) -> HsResult<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM entry_versions v JOIN published p ON p.entry=v.entry WHERE v.obj_id=?1",
            [obj_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn tracked_target(conn: &Connection, obj_id: &str) -> HsResult<bool> {
    Ok(conn.query_row("SELECT 1 FROM tracked WHERE target=?1", [obj_id], |_| Ok(())).optional()?.is_some())
}

/// Merge a verified Head into the per-entry highest-seq state (§16.6).
pub fn merge_head(conn: &Connection, head: &FeedHead, head_obj_id: &str, restricted: bool, now: i64) -> HsResult<HeadApply> {
    let existing = get_head(conn, &head.entry)?;
    match existing {
        Some(HeadRow { seq, head_obj_id: ref known, .. }) if seq == head.seq => {
            if known == head_obj_id {
                Ok(HeadApply::Unchanged)
            } else {
                conn.execute("UPDATE heads SET conflict_obj_id=?2 WHERE entry=?1", params![head.entry, head_obj_id])?;
                Ok(HeadApply::Conflict)
            }
        }
        Some(HeadRow { seq, .. }) if seq > head.seq => Ok(HeadApply::Stale),
        Some(row) => {
            store_head_row(conn, head, head_obj_id, restricted || row.restricted, now)?;
            Ok(HeadApply::New)
        }
        None => {
            store_head_row(conn, head, head_obj_id, restricted, now)?;
            Ok(HeadApply::New)
        }
    }
}

/// §5.4 rule 3: a Head's current version must declare the same entry and publisher.
fn bind_version(conn: &Connection, entry: &str, publisher: &str, obj_id: &str, now: i64) -> HsResult<bool> {
    let Some(obj) = get_object(conn, obj_id)? else { return Ok(false) };
    let declared = obj.body.get("entry").and_then(Value::as_str);
    let by = obj.body.get("publisher").and_then(Value::as_str);
    if declared == Some(entry) && by == Some(publisher) {
        add_version(conn, entry, obj_id, now)?;
        return Ok(true);
    }
    Ok(false)
}

pub fn add_candidate(conn: &Connection, obj_id: &str, publisher: &str, path: &Value, private_capture: bool, now: i64) -> HsResult<bool> {
    let existing: Option<String> =
        conn.query_row("SELECT source_paths FROM candidates WHERE obj_id=?1", [obj_id], |r| r.get(0)).optional()?;
    match existing {
        Some(paths) => {
            let mut paths: Vec<Value> = serde_json::from_str(&paths).unwrap_or_default();
            if !paths.contains(path) {
                paths.push(path.clone());
                conn.execute(
                    "UPDATE candidates SET source_paths=?2, updated_at=?3 WHERE obj_id=?1",
                    params![obj_id, serde_json::to_string(&paths)?, now],
                )?;
            }
            Ok(false)
        }
        None => {
            conn.execute(
                "INSERT INTO candidates(obj_id, publisher, selection, source_paths, private_capture, arrived_at, updated_at)
                 VALUES (?1, ?2, 'unscreened', ?3, ?4, ?5, ?5)",
                params![obj_id, publisher, serde_json::to_string(&vec![path.clone()])?, private_capture as i64, now],
            )?;
            Ok(true)
        }
    }
}

impl Station {
    pub async fn relation(&self, did: &str) -> HsResult<Relation> {
        let contact = self.contacts.get(did).await;
        let collectors = self.collector_dids().await;
        let did_s = did.to_string();
        let (source_id, follower) = self
            .db
            .call(move |c| {
                let source = is_followed_source(c, &did_s)?;
                let follower = c
                    .query_row("SELECT 1 FROM followers WHERE did=?1 AND state='active'", [&did_s], |_| Ok(()))
                    .optional()?
                    .is_some();
                Ok((source, follower))
            })
            .await?;
        Ok(Relation {
            friend: contact.as_ref().is_some_and(|c| c.friend),
            blocked: contact.as_ref().is_some_and(|c| c.blocked),
            followed: source_id.is_some(),
            follower,
            collector: collectors.iter().any(|c| c == did),
            source_id,
        })
    }

    /// CYFS dispatch receiver for `cyfs://<zone>/home/inbox` (§4.4, §7.4).
    pub async fn receive_dispatch(&self, host: &str, path: &str, content_type: &str, claimed: Option<&str>, restricted_hint: bool, body: &[u8]) -> DispatchReply {
        let host = host.split(':').next().unwrap_or(host);
        // A gateway may forward with a loopback Host; what reaches this service is for this zone.
        let host = if host.is_empty() || host == "localhost" || host.parse::<std::net::IpAddr>().is_ok() { self.cfg.zone.as_str() } else { host };
        let own_target = inbox_target(&self.cfg.zone);
        let target = match normalize_cyfs_dispatch_target(host, path) {
            Ok(t) if t == own_target => t,
            _ => return rejected(404, None, &own_target, "no-handler", false),
        };
        let Some(encoding) = CyfsNamedObjectEncoding::from_content_type(content_type) else {
            return rejected(415, None, &target, "unsupported-content-type", false);
        };
        if body.len() > MAX_OBJECT_BYTES {
            return rejected(413, None, &target, "object-too-large", false);
        }
        let obj_id = match validate_cyfs_dispatch_body(encoding, body, claimed) {
            Ok(id) => id,
            Err(_) => return rejected(400, None, &target, "invalid-object", false),
        };
        if encoding != CyfsNamedObjectEncoding::Jwt {
            return rejected(400, Some(obj_id), &target, "signature-required", false);
        }
        let jwt = match std::str::from_utf8(body) {
            Ok(s) => s.trim().to_string(),
            Err(_) => return rejected(400, Some(obj_id), &target, "invalid-object", false),
        };
        let verified = match verify_jwt(self.directory.as_ref(), &jwt, None).await {
            Ok(v) => v,
            Err(VerifyError::Invalid(_)) => return rejected(400, Some(obj_id), &target, "invalid-object", false),
            Err(VerifyError::Unauthorized(_)) => return rejected(403, Some(obj_id), &target, "invalid-signature", false),
            Err(VerifyError::KeyUnavailable(_)) => return rejected(503, Some(obj_id), &target, "signing-key-unavailable", true),
        };
        if verified.obj_id != obj_id.to_string() {
            return rejected(400, Some(obj_id), &target, "object-id-mismatch", false);
        }
        let id_s = verified.obj_id.clone();
        let receipt = self
            .db
            .call(move |c| {
                Ok(c.query_row("SELECT admission FROM inbox_receipts WHERE obj_id=?1", [id_s], |r| r.get::<_, Option<String>>(0))
                    .optional()?)
            })
            .await;
        match receipt {
            Ok(Some(admission)) => return self.accepted(obj_id, &target, admission),
            Ok(None) => {}
            Err(_) => return rejected(503, Some(obj_id), &target, "storage-unavailable", true),
        }
        let sender = verified.publisher.clone();
        let admission = match self.admit(&verified).await {
            Ok(Ok(admission)) => admission,
            Ok(Err(reason)) => return rejected(403, Some(obj_id), &target, &reason, false),
            Err(_) => return rejected(503, Some(obj_id), &target, "storage-unavailable", true),
        };
        let arrival = Arrival::Push { sender: sender.clone() };
        let collect = self.cfg.collector && verified.obj_type == OBJ_TYPE_FEED && !restricted_hint;
        if let Err(e) = self.ingest(verified, arrival, restricted_hint).await {
            return match e {
                HsError::BadRequest(_) => rejected(400, Some(obj_id), &target, "invalid-object", false),
                HsError::Forbidden(_) => rejected(403, Some(obj_id), &target, "invalid-entry", false),
                _ => rejected(503, Some(obj_id), &target, "storage-unavailable", true),
            };
        }
        let id_s = obj_id.to_string();
        let admission_s = admission.clone();
        let now = now_ms();
        let stored = self
            .db
            .call(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO inbox_receipts(obj_id, sender, admission, received_at) VALUES (?1, ?2, ?3, ?4)",
                    params![id_s, sender, admission_s, now],
                )?;
                if collect {
                    c.execute(
                        "INSERT OR IGNORE INTO collector_index(obj_id, submitted_by, received_at) VALUES (?1, ?2, ?3)",
                        params![id_s, sender, now],
                    )?;
                }
                Ok(())
            })
            .await;
        if stored.is_err() {
            return rejected(503, Some(obj_id), &target, "storage-unavailable", true);
        }
        self.wake.select.notify_one();
        self.accepted(obj_id, &target, Some(admission))
    }

    fn accepted(&self, obj_id: ndn_lib::ObjId, target: &str, admission: Option<String>) -> DispatchReply {
        let admission = if self.cfg.disclose_admission { admission } else { None };
        reply(200, CyfsDispatchResult::new(Some(obj_id), target.to_string(), CyfsDispatchStatus::Accepted), admission)
    }

    /// Admission at the inbox (§7.3): wider than chat, never unconditional. Returns the
    /// disclosure (`candidate` / `preferred`) or a rejection reason.
    async fn admit(&self, v: &Verified) -> HsResult<Result<String, String>> {
        let sender = &v.publisher;
        if sender == &self.cfg.owner {
            return Ok(Ok("candidate".into()));
        }
        let relation = self.relation(sender).await?;
        if relation.blocked {
            return Ok(Err("not-admitted".into()));
        }
        let sender_s = sender.clone();
        let since = now_ms() - 3_600_000;
        let recent: i64 = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM inbox_receipts WHERE sender=?1 AND received_at>?2",
                    params![sender_s, since],
                    |r| r.get(0),
                )?)
            })
            .await?;
        if recent > 500 && !relation.friend {
            return Ok(Err("rate-limited".into()));
        }
        let preferred = relation.friend || relation.followed;
        let level = |p: bool| if p { "preferred".to_string() } else { "candidate".to_string() };
        if self.cfg.collector {
            return Ok(Ok(level(preferred)));
        }
        match v.obj_type.as_str() {
            OBJ_TYPE_FOLLOW => {
                let target = v.claims.pointer("/target/publisher").and_then(Value::as_str);
                Ok(if target == Some(self.cfg.owner.as_str()) { Ok(level(preferred)) } else { Err("not-addressed".into()) })
            }
            OBJ_TYPE_CONSUMPTION => {
                let receiver = v.claims.get("receiver").and_then(Value::as_str);
                Ok(if receiver == Some(self.cfg.owner.as_str()) { Ok("candidate".into()) } else { Err("not-addressed".into()) })
            }
            OBJ_TYPE_HEAD => {
                let entry = v.claims.get("entry").and_then(Value::as_str).unwrap_or_default().to_string();
                let known = self
                    .db
                    .call(move |c| {
                        Ok(c.query_row("SELECT 1 FROM heads WHERE entry=?1 UNION SELECT 1 FROM feed_index WHERE entry=?1", [entry], |_| Ok(()))
                            .optional()?
                            .is_some())
                    })
                    .await?;
                Ok(if known || preferred || relation.follower || relation.collector { Ok(level(preferred)) } else { Err("not-admitted".into()) })
            }
            OBJ_TYPE_FEED => {
                let feed: FeedObject = match serde_json::from_value(v.claims.clone()) {
                    Ok(f) => f,
                    Err(_) => return Ok(Err("invalid-object".into())),
                };
                if let Some(target) = feed.comment_target() {
                    let t = target.clone();
                    let (mine, tracked) = self.db.call(move |c| Ok((own_version(c, &t)?, tracked_target(c, &t)?))).await?;
                    if mine || tracked {
                        return Ok(Ok(level(preferred)));
                    }
                }
                Ok(if preferred || relation.collector { Ok(level(preferred)) } else { Err("not-admitted".into()) })
            }
            _ => Ok(Err("unsupported-object".into())),
        }
    }

    /// Store and apply a verified object, whatever the transport.
    pub async fn ingest(&self, v: Verified, arrival: Arrival, restricted_hint: bool) -> HsResult<()> {
        match v.obj_type.as_str() {
            OBJ_TYPE_HEAD => self.ingest_head(v, restricted_hint).await.map(|_| ()),
            OBJ_TYPE_FEED => self.ingest_feed(v, arrival, restricted_hint).await,
            OBJ_TYPE_FOLLOW => self.ingest_follow(v).await,
            _ => {
                let now = now_ms();
                self.db.call(move |c| put_object(c, &stored(&v, false), Some("push"), now)).await
            }
        }
    }

    pub async fn ingest_head(&self, v: Verified, restricted_hint: bool) -> HsResult<HeadApply> {
        let head: FeedHead = serde_json::from_value(v.claims.clone()).map_err(|e| bad(format!("invalid head: {e}")))?;
        validate_head(&head).map_err(bad)?;
        if head.publisher == self.cfg.owner {
            return Ok(HeadApply::Ignored);
        }
        let belongs = entry_belongs_to(self.directory.as_ref(), &head.publisher, &head.entry)
            .await
            .map_err(|e| HsError::Unavailable(e.to_string()))?;
        if !belongs {
            return Err(HsError::Forbidden("entry is not in the publisher's namespace (A46)".into()));
        }
        let now = now_ms();
        let owner = self.cfg.owner.clone();
        let result = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                put_object(&tx, &stored(&v, false), Some("head"), now)?;
                let applied = merge_head(&tx, &head, &v.obj_id, restricted_hint, now)?;
                if applied == HeadApply::New {
                    if let Some(current) = &head.current {
                        bind_version(&tx, &head.entry, &head.publisher, current, now)?;
                    }
                    if let Ok(EntryRef::Path { namespace: EntryNamespace::Follows, .. }) = EntryRef::parse(&head.entry) {
                        let follow: Option<String> = tx
                            .query_row(
                                "SELECT body FROM objects WHERE obj_type=?1 AND publisher=?2 AND json_extract(body,'$.entry')=?3",
                                params![OBJ_TYPE_FOLLOW, head.publisher, head.entry],
                                |r| r.get(0),
                            )
                            .optional()?;
                        let to_me = follow
                            .and_then(|b| serde_json::from_str::<Value>(&b).ok())
                            .and_then(|b| b.pointer("/target/publisher").and_then(Value::as_str).map(str::to_string))
                            .is_some_and(|t| t == owner);
                        if to_me {
                            set_follower(&tx, &head.publisher, &head.entry, head.state, head.seq, now)?;
                        }
                    }
                }
                tx.commit()?;
                Ok(applied)
            })
            .await?;
        if result == HeadApply::New || result == HeadApply::Conflict {
            self.bump(&["reading", "candidates", "comments", "saved", "sources", "profile"]);
        }
        Ok(result)
    }

    async fn ingest_feed(&self, v: Verified, arrival: Arrival, restricted_hint: bool) -> HsResult<()> {
        let feed: FeedObject = serde_json::from_value(v.claims.clone()).map_err(|e| bad(format!("invalid feed object: {e}")))?;
        validate_feed_object(&feed).map_err(bad)?;
        let entry_valid = match &feed.entry {
            Some(entry) => entry_belongs_to(self.directory.as_ref(), &feed.publisher, entry)
                .await
                .map_err(|e| HsError::Unavailable(e.to_string()))?,
            None => false,
        };
        let relation = self.relation(&feed.publisher).await?;
        let owner = self.cfg.owner.clone();
        let settings = self.settings().await?;
        let now = now_ms();
        let path = candidate_path(&arrival, &relation);
        let arrival2 = arrival.clone();
        let (candidate, existed) = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                let existed = crate::objects::has_object(&tx, &v.obj_id)?;
                put_object(&tx, &stored(&v, false), Some(origin_label(&arrival2)), now)?;
                index_feed(&tx, &v.obj_id, &feed, entry_valid)?;
                if entry_valid {
                    let entry = feed.entry.clone().unwrap();
                    add_version(&tx, &entry, &v.obj_id, now)?;
                    if restricted_hint {
                        tx.execute("UPDATE heads SET restricted=1 WHERE entry=?1", [&entry])?;
                    }
                }
                let mut is_candidate = false;
                if let Some(target) = feed.comment_target() {
                    if let Some((kind, source)) = arrival2.comment_source() {
                        tx.execute(
                            "INSERT OR IGNORE INTO comment_sources(comment_id, source_kind, source, seen_at) VALUES (?1, ?2, ?3, ?4)",
                            params![v.obj_id, kind, source, now],
                        )?;
                    }
                    tx.execute(
                        "INSERT OR IGNORE INTO participants(target, did, seen_at) VALUES (?1, ?2, ?3)",
                        params![target, feed.publisher, now],
                    )?;
                    if own_version(&tx, &target)? && settings.comments_open && feed.publisher != owner {
                        let position: i64 = tx.query_row("SELECT COALESCE(MAX(position),0)+1 FROM author_list WHERE target=?1", [&target], |r| r.get(0))?;
                        tx.execute(
                            "INSERT OR IGNORE INTO author_list(target, comment_id, listed, position, updated_at) VALUES (?1, ?2, 1, ?3, ?4)",
                            params![target, v.obj_id, position, now],
                        )?;
                    }
                    let shows_in_feed = matches!(feed.comment_type, Some(CommentType::Repost) | Some(CommentType::Quote) | Some(CommentType::Text));
                    if shows_in_feed && path.is_some() && feed.publisher != owner {
                        is_candidate = add_candidate(&tx, &v.obj_id, &feed.publisher, path.as_ref().unwrap(), false, now)?;
                    }
                } else if let Some(path) = &path {
                    if feed.publisher != owner {
                        is_candidate = add_candidate(&tx, &v.obj_id, &feed.publisher, path, false, now)?;
                    }
                }
                tx.commit()?;
                Ok((is_candidate, existed))
            })
            .await?;
        if !existed || candidate {
            self.bump(&["comments", "candidates", "reading"]);
        }
        if candidate {
            self.wake.select.notify_one();
        }
        Ok(())
    }

    async fn ingest_follow(&self, v: Verified) -> HsResult<()> {
        let follow: FollowDeclaration = serde_json::from_value(v.claims.clone()).map_err(|e| bad(format!("invalid follow: {e}")))?;
        validate_follow(&follow).map_err(bad)?;
        if !entry_belongs_to(self.directory.as_ref(), &follow.publisher, &follow.entry)
            .await
            .map_err(|e| HsError::Unavailable(e.to_string()))?
        {
            return Err(HsError::Forbidden("follow entry outside the follower's namespace".into()));
        }
        let owner = self.cfg.owner.clone();
        let now = now_ms();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                put_object(&tx, &stored(&v, false), Some("follow"), now)?;
                if follow.target.publisher == owner {
                    add_version(&tx, &follow.entry, &v.obj_id, now)?;
                    match get_head(&tx, &follow.entry)? {
                        Some(head) => set_follower(&tx, &follow.publisher, &follow.entry, head.state, head.seq, now)?,
                        None => set_follower(&tx, &follow.publisher, &follow.entry, HeadState::Active, 0, now)?,
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.bump(&["profile", "sources"]);
        Ok(())
    }

    /// Verify and ingest an object handed over in a read response or fetched by ObjId.
    pub async fn ingest_wire(&self, body: &str, arrival: Arrival, restricted: bool) -> HsResult<String> {
        let body = body.trim();
        if body.starts_with('{') {
            return self.ingest_plain(body).await;
        }
        let v = verify_jwt(self.directory.as_ref(), body, None).await.map_err(HsError::from)?;
        let id = v.obj_id.clone();
        self.ingest(v, arrival, restricted).await?;
        Ok(id)
    }

    /// Unsigned objects (FileObjects and other parts) are accepted by content hash only.
    async fn ingest_plain(&self, body: &str) -> HsResult<String> {
        let value: Value = serde_json::from_str(body).map_err(|_| bad("invalid object JSON"))?;
        let obj_type = match crate::objects::obj_type_for_claims(&value) {
            Some(t) => return Err(bad(format!("{t} objects must be signed"))),
            None => OBJ_TYPE_FILE,
        };
        let (obj_id, _) = obj_id_of(obj_type, &value).map_err(bad)?;
        let obj = StoredObject {
            obj_id: obj_id.clone(),
            obj_type: obj_type.into(),
            body: value,
            jwt: None,
            signer: None,
            publisher: None,
            verified: true,
            local_only: false,
        };
        let now = now_ms();
        self.db.call(move |c| put_object(c, &obj, Some("fetch"), now)).await?;
        Ok(obj_id)
    }

    /// Ingest a plain object whose id is known in advance (checked against its content).
    pub async fn ingest_expected(&self, expected: &str, body: &str, arrival: Arrival, restricted: bool) -> HsResult<()> {
        let expected = normalize_obj_id(expected);
        let body = body.trim();
        let obj_type = obj_type_of(&expected).ok_or_else(|| bad("invalid object id"))?;
        if body.starts_with('{') {
            let value: Value = serde_json::from_str(body).map_err(|_| bad("invalid object JSON"))?;
            let (computed, _) = obj_id_of(&obj_type, &value).map_err(bad)?;
            if computed != expected {
                return Err(bad("object does not match its id"));
            }
            if crate::objects::obj_type_for_claims(&value).is_some() {
                return Err(bad("signed objects must arrive as JWT"));
            }
            let obj = StoredObject {
                obj_id: expected,
                obj_type,
                body: value,
                jwt: None,
                signer: None,
                publisher: None,
                verified: true,
                local_only: false,
            };
            let now = now_ms();
            return self.db.call(move |c| put_object(c, &obj, Some("fetch"), now)).await;
        }
        let v = verify_jwt(self.directory.as_ref(), body, Some(&obj_type)).await.map_err(HsError::from)?;
        if v.obj_id != expected {
            return Err(bad("object does not match its id"));
        }
        self.ingest(v, arrival, restricted).await
    }

    pub async fn own_entry_row(&self, entry: &str) -> HsResult<Option<crate::publish::EntryRow>> {
        let entry = entry.to_string();
        self.db.call(move |c| get_entry(c, &entry)).await
    }
}

fn stored(v: &Verified, local_only: bool) -> StoredObject {
    StoredObject {
        obj_id: v.obj_id.clone(),
        obj_type: v.obj_type.clone(),
        body: v.claims.clone(),
        jwt: Some(v.jwt.clone()),
        signer: Some(v.signer.clone()),
        publisher: Some(v.publisher.clone()),
        verified: true,
        local_only,
    }
}

fn origin_label(arrival: &Arrival) -> &'static str {
    match arrival {
        Arrival::Push { .. } => "push",
        Arrival::Pull { .. } => "pull",
        Arrival::AuthorList { .. } => "author_list",
        Arrival::Collector { .. } => "collector",
        Arrival::Fetch => "fetch",
        Arrival::Local => "local",
    }
}

/// Candidate source path when the arrival makes the object a reading candidate.
fn candidate_path(arrival: &Arrival, relation: &Relation) -> Option<Value> {
    match arrival {
        Arrival::Push { sender } if relation.friend || relation.followed => {
            Some(serde_json::json!({ "transport": "push", "sender": sender }))
        }
        Arrival::Pull { source_id, label } => Some(serde_json::json!({ "transport": "pull", "subscription_id": source_id, "label": label })),
        Arrival::Collector { collector } => Some(serde_json::json!({ "transport": "pull", "collector": collector, "label": collector })),
        _ => None,
    }
}

pub fn set_follower(conn: &Connection, did: &str, entry: &str, state: HeadState, seq: u64, now: i64) -> HsResult<()> {
    conn.execute(
        "INSERT INTO followers(did, entry, state, seq, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(did) DO UPDATE SET entry=excluded.entry, state=excluded.state, seq=excluded.seq, updated_at=excluded.updated_at
         WHERE excluded.seq >= followers.seq",
        params![did, entry, state.as_str(), seq as i64, now],
    )?;
    Ok(())
}
