//! Subscription / Source Manager (§7.6, §7.7): follows with knowing notification, friend-
//! derived follows, URL and natural-language subscriptions with persistent intents.

use crate::audience::AudienceSpec;
use crate::error::{bad, HsError, HsResult};
use crate::protocol::*;
use crate::publish::{entry_versions, get_entry, get_head, EntryKind, NewVersion};
use crate::settings::Topic;
use crate::{new_id, now_ms, now_s, Station};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub basis: Vec<String>,
    #[serde(default)]
    pub notify: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<Value>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResolution {
    pub input_kind: String,
    pub input: String,
    pub candidates: Vec<SourceView>,
    pub notify_hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_text: Option<String>,
}

fn source_row(c: &Connection, id: &str) -> HsResult<Option<SourceView>> {
    let row = c
        .query_row(
            "SELECT id, name, kind, did, url, description, basis, notify, last_success_at, last_error, paused, intent_id, follow_entry FROM sources WHERE id=?1",
            [id],
            |r| {
                Ok((
                    SourceView {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        kind: r.get(2)?,
                        did: r.get(3)?,
                        url: r.get(4)?,
                        description: r.get(5)?,
                        basis: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                        notify: r.get(7)?,
                        last_success_at: r.get(8)?,
                        last_error: r.get::<_, Option<String>>(9)?.and_then(|e| serde_json::from_str(&e).ok()),
                        paused: r.get::<_, i64>(10)? != 0,
                        intent_id: r.get(11)?,
                        credibility: None,
                        update_mode: None,
                    },
                    r.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .optional()?;
    let Some((mut view, follow_entry)) = row else { return Ok(None) };
    if view.kind == "person" {
        view.credibility = Some("verified_did".into());
        view.update_mode = Some("Home feed changes + push".into());
        if let (Some(entry), Some(did)) = (&follow_entry, &view.did) {
            let state: Option<(String, i64)> = c
                .query_row(
                    "SELECT state, attempts FROM outbox WHERE entry=?1 AND recipient=?2 AND purpose='follow' ORDER BY id DESC LIMIT 1",
                    params![entry, did],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            view.notify = match state {
                Some((s, _)) if s == "accepted" => "acknowledged".into(),
                Some((s, attempts)) if s == "pending" && attempts == 0 => "pending".into(),
                Some(_) => "retrying".into(),
                None if view.basis.is_empty() => "pending".into(),
                None => view.notify,
            };
        }
    } else {
        view.notify = "unsupported".into();
        view.credibility = Some(if view.kind == "rss" { "known_site".into() } else { "unknown".into() });
        view.update_mode = Some(match view.kind.as_str() {
            "rss" => "RSS poll".into(),
            "channel" => "Collector query".into(),
            _ => "Spider crawl".into(),
        });
    }
    Ok(Some(view))
}

impl Station {
    pub async fn list_sources(&self) -> HsResult<Value> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare("SELECT id FROM sources ORDER BY created_at")?;
                let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                let mut sources = Vec::new();
                for id in ids {
                    if let Some(s) = source_row(c, &id)? {
                        sources.push(s);
                    }
                }
                let mut stmt = c.prepare("SELECT id, text, status, source_ids, updated_at FROM intents ORDER BY created_at")?;
                let intents = stmt
                    .query_map([], |r| {
                        Ok(json!({
                            "id": r.get::<_, String>(0)?,
                            "text": r.get::<_, String>(1)?,
                            "status": r.get::<_, String>(2)?,
                            "sourceIds": serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap_or(json!([])),
                            "updatedAt": r.get::<_, i64>(4)?,
                        }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({ "sources": sources, "intents": intents }))
            })
            .await
    }

    async fn person_candidate(&self, did: &str, fallback_name: Option<String>) -> SourceView {
        let did_s = did.to_string();
        let existing = self
            .db
            .call(move |c| {
                let id: Option<String> = c.query_row("SELECT id FROM sources WHERE did=?1", [&did_s], |r| r.get(0)).optional()?;
                match id {
                    Some(id) => source_row(c, &id),
                    None => Ok(None),
                }
            })
            .await
            .ok()
            .flatten();
        if let Some(existing) = existing {
            return existing;
        }
        let mut name = fallback_name;
        let mut description = None;
        if let Ok(profile) = self.fetch_remote_profile(did).await {
            name = name.or_else(|| profile.get("name").and_then(Value::as_str).map(str::to_string));
            description = profile.get("bio").and_then(Value::as_str).map(str::to_string).filter(|b| !b.is_empty());
        }
        SourceView {
            id: format!("src-{}", &crate::new_id("p")[2..]),
            name: name.unwrap_or_else(|| did.trim_start_matches("did:").split_once(':').map(|x| x.1).unwrap_or(did).to_string()),
            kind: "person".into(),
            did: Some(did.to_string()),
            url: None,
            description,
            basis: vec![],
            notify: "pending".into(),
            last_success_at: None,
            last_error: None,
            paused: false,
            intent_id: None,
            credibility: Some("verified_did".into()),
            update_mode: Some("Home feed changes + push".into()),
        }
    }

    pub async fn fetch_remote_profile(&self, did: &str) -> HsResult<Value> {
        self.fetch_remote_profile_as(did, true).await
    }

    pub async fn fetch_remote_profile_as(&self, did: &str, as_owner: bool) -> HsResult<Value> {
        let (zone, origin) = self.origin_for_did(did).await?;
        let response = self.remote_get_as(&zone, &origin, "/home/profile", as_owner).await?;
        if !response.status().is_success() {
            return Err(HsError::Unavailable(format!("profile HTTP {}", response.status())));
        }
        response.json::<Value>().await.map_err(|e| HsError::Unavailable(e.to_string()))
    }

    /// `resolveSourceInput`: Follow / URL / natural language → candidate sources (§7.7).
    pub async fn resolve_source_input(&self, kind: &str, text: &str) -> HsResult<SourceResolution> {
        let input = text.trim().to_string();
        match kind {
            "follow" => {
                let mut candidates = Vec::new();
                let lower = input.to_lowercase();
                for contact in self.contacts.list().await {
                    if contact.did == self.cfg.owner || contact.blocked {
                        continue;
                    }
                    if contact.did.to_lowercase() == lower || contact.name.to_lowercase().contains(&lower) {
                        candidates.push(self.person_candidate(&contact.did, Some(contact.name.clone())).await);
                    }
                    if candidates.len() >= 4 {
                        break;
                    }
                }
                if candidates.is_empty() {
                    if let Ok(did) = name_lib::DID::from_friendly_name(&input) {
                        let did = did.to_string();
                        if did != self.cfg.owner {
                            candidates.push(self.person_candidate(&did, None).await);
                        }
                    }
                }
                Ok(SourceResolution { input_kind: kind.into(), input, candidates, notify_hint: "pending".into(), intent_text: None })
            }
            "url" => {
                if !is_http_url(&input) {
                    return Err(bad("homestation.validation.url"));
                }
                let host = url::Url::parse(&input).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
                if let Ok(Some(did)) = self.homestation_at(&input).await {
                    let candidate = self.person_candidate(&did, None).await;
                    return Ok(SourceResolution { input_kind: kind.into(), input, candidates: vec![candidate], notify_hint: "pending".into(), intent_text: None });
                }
                let mut candidate = SourceView {
                    id: format!("src-{}", &new_id("u")[2..]),
                    name: host.trim_start_matches("www.").to_string(),
                    kind: "website".into(),
                    did: None,
                    url: Some(input.clone()),
                    description: None,
                    basis: vec![],
                    notify: "unsupported".into(),
                    last_success_at: None,
                    last_error: None,
                    paused: false,
                    intent_id: None,
                    credibility: Some("unknown".into()),
                    update_mode: Some("Spider crawl".into()),
                };
                match self.fetch_text(&input).await {
                    Ok((body, _)) if crate::spider::is_feed_document(&body) => {
                        candidate.kind = "rss".into();
                        candidate.credibility = Some("known_site".into());
                        candidate.update_mode = Some("RSS poll".into());
                    }
                    Ok((body, _)) => {
                        let preview = crate::spider::preview_of_html(&body, &input);
                        if !preview.title.is_empty() {
                            candidate.name = preview.site_name.clone().unwrap_or(preview.title.clone());
                            candidate.description = Some(preview.summary.clone()).filter(|s| !s.is_empty());
                        }
                        if let Some(feed) = preview.feed_url {
                            candidate.kind = "rss".into();
                            candidate.url = Some(feed);
                            candidate.credibility = Some("known_site".into());
                            candidate.update_mode = Some("RSS poll".into());
                        }
                    }
                    Err(e) => candidate.last_error = Some(json!({ "at": now_ms(), "reason": e.to_string() })),
                }
                let url = candidate.url.clone();
                let existing = self
                    .db
                    .call(move |c| {
                        let id: Option<String> = c.query_row("SELECT id FROM sources WHERE url=?1", [url], |r| r.get(0)).optional()?;
                        match id {
                            Some(id) => source_row(c, &id),
                            None => Ok(None),
                        }
                    })
                    .await?;
                Ok(SourceResolution {
                    input_kind: kind.into(),
                    input,
                    candidates: vec![existing.unwrap_or(candidate)],
                    notify_hint: "unsupported".into(),
                    intent_text: None,
                })
            }
            "natural" => {
                if input.chars().count() < 4 {
                    return Err(bad("homestation.validation.intentTooShort"));
                }
                let candidates = self.map_intent(&input).await?;
                Ok(SourceResolution { input_kind: kind.into(), input: input.clone(), candidates, notify_hint: "unsupported".into(), intent_text: Some(input) })
            }
            _ => Err(bad("kind is follow, url or natural")),
        }
    }

    /// Whether a URL is served by a HomeStation (its `/home/profile` names a DID).
    async fn homestation_at(&self, input: &str) -> HsResult<Option<String>> {
        let url = url::Url::parse(input).map_err(|_| bad("invalid url"))?;
        let origin = url.origin().ascii_serialization();
        let response = self
            .http
            .get(format!("{origin}/home/profile"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| HsError::Unavailable(e.to_string()))?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let profile: Value = response.json().await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        Ok(profile.get("did").and_then(Value::as_str).filter(|d| d.starts_with("did:")).map(str::to_string))
    }

    /// Map a natural-language intent to sources: matching known people, preset collectors and
    /// a Topic carrying the intent's keywords. The intent itself is kept and re-mapped (§7.7).
    async fn map_intent(&self, text: &str) -> HsResult<Vec<SourceView>> {
        let words = intent_keywords(text);
        let mut candidates = Vec::new();
        for contact in self.contacts.list().await {
            let name = contact.name.to_lowercase();
            if !contact.blocked && contact.did != self.cfg.owner && words.iter().any(|w| name.contains(w)) {
                candidates.push(self.person_candidate(&contact.did, Some(contact.name)).await);
            }
        }
        for collector in self.settings().await?.collectors {
            candidates.push(SourceView {
                id: format!("src-{}", &new_id("c")[2..]),
                name: format!("{} · {}", collector.name, truncate_words(text)),
                kind: "channel".into(),
                did: None,
                url: None,
                description: Some(format!("Collector {} filtered by: {}", collector.did, words.join(", "))),
                basis: vec![],
                notify: "unsupported".into(),
                last_success_at: None,
                last_error: None,
                paused: false,
                intent_id: None,
                credibility: Some("known_site".into()),
                update_mode: Some("Collector query".into()),
            });
        }
        Ok(candidates)
    }

    /// `follow(resolution)`: add the active basis to every candidate; persons get a follow
    /// declaration, intents are saved with their source mapping and a Topic.
    pub async fn follow_resolution(&self, resolution: SourceResolution) -> HsResult<Vec<SourceView>> {
        let mut intent_id = None;
        if resolution.input_kind == "natural" {
            let id = new_id("intent");
            let text = resolution.intent_text.clone().unwrap_or(resolution.input.clone());
            let words = intent_keywords(&text);
            let topic_id = format!("topic-{}", &id[7..]);
            let name = truncate_words(&text);
            self.update_settings(move |s| {
                s.topics.push(Topic { id: topic_id, name, tags: words, subscribed: true });
            })
            .await?;
            let now = now_ms();
            let id2 = id.clone();
            self.db
                .call(move |c| {
                    c.execute(
                        "INSERT INTO intents(id, text, status, source_ids, created_at, updated_at) VALUES (?1, ?2, 'mapping', '[]', ?3, ?3)",
                        params![id2, text, now],
                    )?;
                    Ok(())
                })
                .await?;
            intent_id = Some(id);
        }
        let mut added = Vec::new();
        for candidate in resolution.candidates {
            let id = match candidate.kind.as_str() {
                "person" => {
                    let did = candidate.did.clone().ok_or_else(|| bad("person source needs a DID"))?;
                    self.follow_did(&did, &candidate.name, "active", intent_id.as_deref()).await?
                }
                _ => self.add_feed_source(&candidate, intent_id.as_deref()).await?,
            };
            let id2 = id.clone();
            if let Some(view) = self.db.call(move |c| source_row(c, &id2)).await? {
                added.push(view);
            }
        }
        if let Some(intent) = intent_id {
            let ids: Vec<String> = added.iter().map(|s| s.id.clone()).collect();
            let status = if ids.is_empty() { "mapping" } else { "collecting" };
            let now = now_ms();
            self.db
                .call(move |c| {
                    c.execute(
                        "UPDATE intents SET source_ids=?2, status=?3, updated_at=?4 WHERE id=?1",
                        params![intent, serde_json::to_string(&ids)?, status, now],
                    )?;
                    Ok(())
                })
                .await?;
        }
        self.bump(&["sources"]);
        self.wake.pull.notify_one();
        Ok(added)
    }

    async fn add_feed_source(&self, candidate: &SourceView, intent_id: Option<&str>) -> HsResult<String> {
        let candidate = candidate.clone();
        let intent_id = intent_id.map(str::to_string);
        let now = now_ms();
        self.db
            .call(move |c| {
                if let Some(url) = &candidate.url {
                    let existing: Option<(String, String)> =
                        c.query_row("SELECT id, basis FROM sources WHERE url=?1", [url], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
                    if let Some((id, basis)) = existing {
                        let mut basis: Vec<String> = serde_json::from_str(&basis).unwrap_or_default();
                        if !basis.contains(&"active".to_string()) {
                            basis.push("active".into());
                        }
                        c.execute("UPDATE sources SET basis=?2, updated_at=?3 WHERE id=?1", params![id, serde_json::to_string(&basis)?, now])?;
                        return Ok(id);
                    }
                }
                let id = if candidate.id.starts_with("src-") { candidate.id.clone() } else { format!("src-{}", &new_id("s")[2..]) };
                c.execute(
                    "INSERT INTO sources(id, kind, did, url, name, description, basis, paused, intent_id, notify, created_at, updated_at)
                     VALUES (?1, ?2, NULL, ?3, ?4, ?5, '[\"active\"]', 0, ?6, 'unsupported', ?7, ?7)",
                    params![id, candidate.kind, candidate.url, candidate.name, candidate.description, intent_id, now],
                )?;
                Ok(id)
            })
            .await
    }

    /// Ensure a person source with `basis`; publish or re-activate the follow declaration
    /// (E17) and deliver it. Returns the source id.
    pub async fn follow_did(&self, did: &str, name: &str, basis: &str, intent_id: Option<&str>) -> HsResult<String> {
        if did == self.cfg.owner {
            return Err(bad("cannot follow yourself"));
        }
        name_lib::DID::from_str(did).map_err(|_| bad("invalid DID"))?;
        if self.is_local_principal(did).await {
            return Err(bad("users of this zone share this HomeStation and have no stream of their own yet"));
        }
        let did_s = did.to_string();
        let name = name.to_string();
        let basis_s = basis.to_string();
        let intent_id = intent_id.map(str::to_string);
        let now = now_ms();
        let id = self
            .db
            .call(move |c| {
                let existing: Option<(String, String)> =
                    c.query_row("SELECT id, basis FROM sources WHERE did=?1", [&did_s], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
                match existing {
                    Some((id, basis)) => {
                        let mut list: Vec<String> = serde_json::from_str(&basis).unwrap_or_default();
                        if !list.contains(&basis_s) {
                            list.push(basis_s);
                        }
                        c.execute("UPDATE sources SET basis=?2, updated_at=?3 WHERE id=?1", params![id, serde_json::to_string(&list)?, now])?;
                        Ok(id)
                    }
                    None => {
                        let id = format!("src-{}", &new_id("p")[2..]);
                        c.execute(
                            "INSERT INTO sources(id, kind, did, name, basis, paused, intent_id, notify, created_at, updated_at)
                             VALUES (?1, 'person', ?2, ?3, ?4, 0, ?5, 'pending', ?6, ?6)",
                            params![id, did_s, name, serde_json::to_string(&vec![basis_s])?, intent_id, now],
                        )?;
                        Ok(id)
                    }
                }
            })
            .await?;
        self.publish_follow(did, true).await?;
        let id2 = id.clone();
        let entry = self.own_entry(EntryNamespace::Follows, &follow_key(&self.cfg.owner, did));
        self.db.call(move |c| Ok(c.execute("UPDATE sources SET follow_entry=?2 WHERE id=?1", params![id2, entry])?)).await?;
        self.bump(&["sources", "profile"]);
        self.wake.pull.notify_one();
        Ok(id)
    }

    /// The follow declaration entry: first publication, re-activation or withdrawal (E17).
    async fn publish_follow(&self, did: &str, active: bool) -> HsResult<()> {
        let entry = self.own_entry(EntryNamespace::Follows, &follow_key(&self.cfg.owner, did));
        let entry2 = entry.clone();
        let (row, head, versions) = self
            .db
            .call(move |c| Ok((get_entry(c, &entry2)?, get_head(c, &entry2)?, entry_versions(c, &entry2)?)))
            .await?;
        let published = match (row, head, active) {
            (None, _, false) => return Ok(()),
            (None, _, true) => {
                let zone = self.directory.zone_of(did).await.map_err(|e| HsError::Unavailable(e.to_string()))?;
                let follow = FollowDeclaration {
                    kind: FOLLOW_KIND.into(),
                    publisher: self.cfg.owner.clone(),
                    iat: now_s(),
                    entry: entry.clone(),
                    target: FollowTarget { publisher: did.to_string(), stream: stream_url(&zone) },
                };
                self.publish_version(NewVersion {
                    entry: entry.clone(),
                    kind: EntryKind::Follow,
                    audience: Some(AudienceSpec::only(did)),
                    target: Some(did.to_string()),
                    category: None,
                    obj_type: OBJ_TYPE_FOLLOW,
                    claims: serde_json::to_value(&follow)?,
                })
                .await?
            }
            (Some(_), Some(head), true) if head.state == HeadState::Active => return Ok(()),
            (Some(_), _, true) => {
                let current = versions.first().cloned();
                self.set_entry_state(&entry, HeadState::Active, current).await?
            }
            (Some(_), Some(head), false) if head.state == HeadState::Withdrawn => return Ok(()),
            (Some(_), _, false) => self.set_entry_state(&entry, HeadState::Withdrawn, None).await?,
        };
        let mut objects = Vec::new();
        if let Some(obj) = &published.obj_id {
            objects.push(obj.clone());
        }
        objects.push(published.head_obj_id.clone());
        self.enqueue_delivery(Some(&entry), &objects, &[did.to_string()], None, true).await
    }

    /// Remove the active basis. Friend-derived follows stay: use "不看" instead (§7.6).
    pub async fn unfollow(&self, source_id: &str) -> HsResult<&'static str> {
        let id = source_id.to_string();
        let row: Option<(Option<String>, String)> = self
            .db
            .call(move |c| Ok(c.query_row("SELECT did, basis FROM sources WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?))
            .await?;
        let Some((did, basis)) = row else { return Ok("removed") };
        let mut basis: Vec<String> = serde_json::from_str(&basis).unwrap_or_default();
        basis.retain(|b| b != "active");
        let id = source_id.to_string();
        let now = now_ms();
        if basis.contains(&"friend".to_string()) {
            self.db
                .call(move |c| Ok(c.execute("UPDATE sources SET basis=?2, updated_at=?3 WHERE id=?1", params![id, serde_json::to_string(&basis)?, now])?))
                .await?;
            self.bump(&["sources"]);
            return Ok("friend_basis");
        }
        if let Some(did) = &did {
            self.publish_follow(did, false).await?;
        }
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute("DELETE FROM sources WHERE id=?1", [&id])?;
                let mut stmt = tx.prepare("SELECT id, source_ids FROM intents")?;
                let intents = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                for (intent, ids) in intents {
                    let mut ids: Vec<String> = serde_json::from_str(&ids).unwrap_or_default();
                    if ids.contains(&id) {
                        ids.retain(|i| i != &id);
                        if ids.is_empty() {
                            tx.execute("DELETE FROM intents WHERE id=?1", [&intent])?;
                        } else {
                            tx.execute("UPDATE intents SET source_ids=?2, updated_at=?3 WHERE id=?1", params![intent, serde_json::to_string(&ids)?, now])?;
                        }
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.bump(&["sources", "candidates", "profile"]);
        Ok("removed")
    }

    pub async fn pause_source(&self, source_id: &str, paused: bool) -> HsResult<()> {
        let id = source_id.to_string();
        self.db.call(move |c| Ok(c.execute("UPDATE sources SET paused=?2 WHERE id=?1", params![id, paused as i64])?)).await?;
        self.bump(&["sources"]);
        Ok(())
    }

    /// Message Center friends follow each other by default (§7.6, A34, A36): add or remove
    /// the friend basis; keep follows that still have an active basis.
    pub async fn sync_friends(&self) -> HsResult<()> {
        self.contacts.invalidate().await;
        let contacts = self.contacts.list().await;
        let mut friends: Vec<(String, String)> = Vec::new();
        for c in contacts.iter().filter(|c| c.friend && !c.blocked && c.did != self.cfg.owner) {
            if !self.is_local_principal(&c.did).await {
                friends.push((c.did.clone(), c.name.clone()));
            }
        }
        for (did, name) in &friends {
            if let Err(e) = self.follow_did(did, name, "friend", None).await {
                log::warn!("friend follow {did}: {e}");
            }
        }
        let friend_dids: Vec<String> = friends.iter().map(|f| f.0.clone()).collect();
        let stale: Vec<(String, String, Vec<String>)> = self
            .db
            .call(move |c| {
                let mut stmt = c.prepare("SELECT id, did, basis FROM sources WHERE kind='person' AND basis LIKE '%friend%'")?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows
                    .into_iter()
                    .filter(|(_, did, _)| !friend_dids.contains(did))
                    .map(|(id, did, basis)| (id, did, serde_json::from_str(&basis).unwrap_or_default()))
                    .collect())
            })
            .await?;
        for (id, did, mut basis) in stale {
            basis.retain(|b| b != "friend");
            if basis.is_empty() {
                self.publish_follow(&did, false).await?;
                let id = id.clone();
                self.db.call(move |c| Ok(c.execute("DELETE FROM sources WHERE id=?1", [id])?)).await?;
            } else {
                let now = now_ms();
                self.db
                    .call(move |c| Ok(c.execute("UPDATE sources SET basis=?2, updated_at=?3 WHERE id=?1", params![id, serde_json::to_string(&basis)?, now])?))
                    .await?;
            }
        }
        self.bump(&["sources"]);
        Ok(())
    }
}

/// Keywords of an intent: Latin words of 3+ letters and CJK runs of 2+ characters.
pub fn intent_keywords(text: &str) -> Vec<String> {
    const STOP: &[&str] = &["the", "and", "for", "with", "about", "from", "that", "this", "news", "want", "into", "near", "what"];
    let mut words = Vec::new();
    let mut latin = String::new();
    let mut cjk = String::new();
    let flush = |buf: &mut String, words: &mut Vec<String>, min: usize| {
        let chars: Vec<char> = buf.chars().collect();
        let cjk = chars.first().is_some_and(|c| !c.is_ascii());
        if cjk && chars.len() > 4 {
            for pair in chars.windows(2) {
                let w: String = pair.iter().collect();
                if !words.contains(&w) {
                    words.push(w);
                }
            }
        } else if chars.len() >= min && !STOP.contains(&buf.as_str()) && !words.contains(buf) {
            words.push(buf.clone());
        }
        buf.clear();
    };
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            flush(&mut cjk, &mut words, 2);
            latin.push(ch);
        } else if ch as u32 >= 0x3400 && (ch as u32) <= 0x9FFF {
            flush(&mut latin, &mut words, 3);
            cjk.push(ch);
        } else {
            flush(&mut latin, &mut words, 3);
            flush(&mut cjk, &mut words, 2);
        }
    }
    flush(&mut latin, &mut words, 3);
    flush(&mut cjk, &mut words, 2);
    words.truncate(8);
    words
}

fn truncate_words(text: &str) -> String {
    let t: String = text.chars().take(32).collect();
    if text.chars().count() > 32 {
        format!("{t}…")
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords() {
        assert_eq!(intent_keywords("Second-hand bikes near Palo Alto"), vec!["second", "hand", "bikes", "palo", "alto"]);
        assert!(intent_keywords("我想看阳台种菜的内容").contains(&"种菜".to_string()));
        assert_eq!(intent_keywords("阳台种菜"), vec!["阳台种菜"]);
    }
}
