//! Selection Service and Reading List (§8, §9): local effective tags from evaluations and user
//! corrections, filter and "不看" rules, simple replaceable scoring, preparation before
//! admission, one bounded reading list, and the followed-candidates view (§9.5).

use crate::contacts::ContactInfo;
use crate::error::HsResult;
use crate::evaluation::{latest_record, overrides_for, EvalRequest, EvalTarget, RULES_PROFILE};
use crate::objects::get_feed;
use crate::protocol::*;
use crate::publish::get_head;
use crate::resources::ResourceState;
use crate::settings::{FilterRule, MuteRule, UserSettings};
use crate::{now_ms, Station};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveTag {
    pub tag: String,
    pub label: String,
    pub source: String,
    pub status: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier_revision: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Classified {
    pub generation: bool,
    pub quality: bool,
}

pub fn dimension_of_tag(tag: &str) -> &'static str {
    match tag {
        "ai_full" | "ai_assisted" => "generation_method",
        "low_quality" => "quality",
        "ad" => "ad",
        _ => "topic",
    }
}

/// Author tags (declared) + local evaluation + user corrections; the object is never changed (§8.5).
pub fn effective_tags(conn: &Connection, obj_id: &str, feed: &FeedObject, scope: &str) -> HsResult<(Vec<EffectiveTag>, Classified)> {
    let mut tags: Vec<EffectiveTag> = feed
        .tags
        .iter()
        .map(|t| EffectiveTag {
            tag: t.clone(),
            label: t.clone(),
            source: "author".into(),
            status: "declared".into(),
            scope: "whole_content".into(),
            confidence: None,
            basis: None,
            classifier_revision: None,
        })
        .collect();
    let mut classified = Classified::default();
    if let Some(record) = latest_record(conn, obj_id, RULES_PROFILE)? {
        classified.generation = record.dimensions.iter().any(|d| d == "generation_method") && !record.unknown_dimensions.iter().any(|d| d == "generation_method");
        classified.quality = record.dimensions.iter().any(|d| d == "quality") && !record.unknown_dimensions.iter().any(|d| d == "quality");
        for a in record.assertions.iter().filter(|a| a.value) {
            if tags.iter().any(|t| t.tag.eq_ignore_ascii_case(&a.tag)) {
                continue;
            }
            if a.source == "author" && a.dimension == "topic" {
                continue;
            }
            tags.push(EffectiveTag {
                tag: a.tag.clone(),
                label: a.tag.clone(),
                source: if a.source == "author" { "author".into() } else { "model".into() },
                status: a.status.clone(),
                scope: if a.scope == "whole_content" || a.scope == "link_card" { "whole_content".into() } else { "part".into() },
                confidence: a.confidence,
                basis: a.basis.clone(),
                classifier_revision: Some(record.profile_revision.clone()),
            });
        }
    }
    for o in overrides_for(conn, obj_id, scope)? {
        match o.action.as_str() {
            "remove" => tags.retain(|t| !t.tag.eq_ignore_ascii_case(&o.tag)),
            "confirm" => {
                for t in tags.iter_mut().filter(|t| t.tag.eq_ignore_ascii_case(&o.tag)) {
                    t.source = "user".into();
                    t.status = "confirmed".into();
                }
            }
            "replace" | "add" => {
                let new_tag = if o.action == "replace" { o.replacement.clone().unwrap_or_default() } else { o.tag.clone() };
                let previous = tags.iter().find(|t| t.tag.eq_ignore_ascii_case(&o.tag)).cloned();
                if o.action == "replace" {
                    tags.retain(|t| !t.tag.eq_ignore_ascii_case(&o.tag));
                }
                if !new_tag.is_empty() && !tags.iter().any(|t| t.tag == new_tag) {
                    tags.push(EffectiveTag {
                        tag: new_tag.clone(),
                        label: new_tag,
                        source: "user".into(),
                        status: "confirmed".into(),
                        scope: previous.as_ref().map(|p| p.scope.clone()).unwrap_or_else(|| "whole_content".into()),
                        confidence: None,
                        basis: previous.and_then(|p| p.basis),
                        classifier_revision: None,
                    });
                }
            }
            _ => {}
        }
        match dimension_of_tag(&o.tag) {
            "generation_method" => classified.generation = true,
            "quality" => classified.quality = true,
            _ => {}
        }
    }
    Ok((tags, classified))
}

/// A rule hits when all its conditions hold; inferred tags only count above the threshold,
/// unknown follows the rule's policy (§8.5, A52, A54).
pub fn filter_hits(tags: &[EffectiveTag], classified: &Classified, rules: &[FilterRule]) -> Vec<String> {
    rules
        .iter()
        .filter(|r| r.enabled && !r.conditions.is_empty())
        .filter(|rule| {
            rule.conditions.iter().all(|condition| match tags.iter().find(|t| &t.tag == condition) {
                Some(tag) if tag.source == "model" && tag.status == "inferred" => {
                    rule.accept_inferred && tag.confidence.unwrap_or(0.0) >= rule.min_confidence
                }
                Some(_) => true,
                None => {
                    let known = match dimension_of_tag(condition) {
                        "generation_method" => classified.generation,
                        "quality" => classified.quality,
                        _ => true,
                    };
                    !known && rule.unknown == "hide"
                }
            })
        })
        .map(|r| r.id.clone())
        .collect()
}

pub fn is_muted(publisher: &str, rules: &[MuteRule], contacts: &[ContactInfo]) -> bool {
    rules.iter().any(|rule| match rule {
        MuteRule::Person { did, .. } => did == publisher,
        MuteRule::Group { group_id, .. } => contacts.iter().any(|c| c.did == publisher && c.groups.iter().any(|g| g == group_id)),
    })
}

pub fn topics_for(tags: &[EffectiveTag], settings: &UserSettings) -> Vec<String> {
    settings
        .topics
        .iter()
        .filter(|topic| {
            topic
                .tags
                .iter()
                .chain(std::iter::once(&topic.name))
                .any(|t| tags.iter().any(|e| e.tag.eq_ignore_ascii_case(t)))
        })
        .map(|t| t.id.clone())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonRef {
    pub kind: String,
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingReason {
    pub code: String,
    pub text: String,
    pub refs: Vec<ReasonRef>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ReadingQuery {
    pub filter: String,
    pub topic_id: Option<String>,
    pub search: String,
    pub show_filtered: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingPage {
    pub obj_ids: Vec<String>,
    pub next_cursor: Option<String>,
    pub hidden_by_rules: usize,
    pub hidden_by_mute: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateView {
    pub obj_id: String,
    pub selection: String,
    pub source_paths: Vec<Value>,
    pub arrived_at: i64,
    pub first_admitted_at: Option<i64>,
    pub opened_at: Option<i64>,
    pub resources: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePage {
    pub entries: Vec<CandidateView>,
    pub next_cursor: Option<String>,
    pub read_count: usize,
    pub retention_days: i64,
    pub last_fetch_at: i64,
}

fn head_withdrawn(conn: &Connection, feed: &FeedObject) -> HsResult<bool> {
    Ok(match &feed.entry {
        Some(entry) => get_head(conn, entry)?.is_some_and(|h| h.state == HeadState::Withdrawn),
        None => false,
    })
}

fn source_label(conn: &Connection, path: &Value) -> HsResult<String> {
    if let Some(label) = path.get("label").and_then(Value::as_str) {
        return Ok(label.to_string());
    }
    if let Some(sender) = path.get("sender").and_then(Value::as_str) {
        let name: Option<String> = conn.query_row("SELECT name FROM sources WHERE did=?1", [sender], |r| r.get(0)).optional()?;
        return Ok(name.unwrap_or_else(|| sender.to_string()));
    }
    Ok(String::new())
}

impl Station {
    pub(crate) async fn selection_loop(self: std::sync::Arc<Self>) {
        loop {
            if let Err(e) = self.run_eval_tasks().await {
                log::warn!("evaluation tasks failed: {e}");
            }
            if let Err(e) = self.run_selection().await {
                log::warn!("selection pass failed: {e}");
            }
            let _ = tokio::time::timeout(self.cfg.select_interval, self.wake.select.notified()).await;
        }
    }

    async fn classify(&self, obj_id: &str, refresh: bool) -> HsResult<()> {
        let request = EvalRequest {
            target: EvalTarget { kind: "content".into(), object_id: Some(obj_id.to_string()), ..Default::default() },
            dimensions: vec!["topic".into(), "generation_method".into(), "quality".into(), "ad".into()],
            profile_id: Some(RULES_PROFILE.into()),
            refresh,
            ..Default::default()
        };
        self.evaluate_now(request, "global").await.map(|_| ())
    }

    /// One pass: screen new candidates, pick the best within the reading window, prepare and
    /// admit them (§8.2).
    pub async fn run_selection(&self) -> HsResult<usize> {
        let settings = self.settings().await?;
        let contacts = self.contacts.list().await;
        let retention_ms = self.cfg.candidate_retention_days * 86_400_000;
        let since = now_ms() - retention_ms;
        let pending: Vec<(String, String)> = self
            .db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT obj_id, selection FROM candidates WHERE selection IN ('unscreened','not_selected','preparing') AND arrived_at>=?1 ORDER BY arrived_at DESC LIMIT 400",
                )?;
                let rows = stmt.query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        for (obj_id, selection) in &pending {
            if selection == "unscreened" {
                if let Err(e) = self.classify(obj_id, false).await {
                    log::debug!("classify {obj_id}: {e}");
                }
            }
        }
        let friends: Vec<String> = contacts.iter().filter(|c| c.friend).map(|c| c.did.clone()).collect();
        let settings2 = settings.clone();
        let contacts2 = contacts.clone();
        let window = self.cfg.reading_window;
        let batch = self.cfg.admission_batch;
        let affinity = self.publisher_affinity().await.unwrap_or_default();
        // Score everything still waiting; filtered and withdrawn ones are set aside.
        let chosen: (Vec<(String, Value)>, usize) = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                let mut scored = Vec::new();
                let now = now_ms();
                for (obj_id, _) in &pending {
                    let Some(feed) = get_feed(&tx, obj_id)? else { continue };
                    if head_withdrawn(&tx, &feed)? {
                        tx.execute("UPDATE candidates SET selection='dropped', reason='withdrawn', updated_at=?2 WHERE obj_id=?1", params![obj_id, now])?;
                        continue;
                    }
                    let (tags, classified) = effective_tags(&tx, obj_id, &feed, "global")?;
                    let hits = filter_hits(&tags, &classified, &settings2.filter_rules);
                    if is_muted(&feed.publisher, &settings2.mute_rules, &contacts2) {
                        tx.execute("UPDATE candidates SET selection='not_selected', reason='muted', updated_at=?2 WHERE obj_id=?1", params![obj_id, now])?;
                        continue;
                    }
                    let paths: String = tx.query_row("SELECT source_paths FROM candidates WHERE obj_id=?1", [obj_id], |r| r.get(0))?;
                    let paths: Vec<Value> = serde_json::from_str(&paths).unwrap_or_default();
                    let followed = crate::ingress::is_followed_source(&tx, &feed.publisher)?;
                    let friend = friends.contains(&feed.publisher);
                    let topics = topics_for(&tags, &settings2);
                    let subscribed: Vec<&crate::settings::Topic> = settings2.topics.iter().filter(|t| t.subscribed && topics.contains(&t.id)).collect();
                    let collector = paths.iter().find_map(|p| p.get("collector").and_then(Value::as_str).map(str::to_string));
                    let subscription = paths.iter().any(|p| p.get("subscription_id").is_some()) && followed.is_none();
                    let mut score = 0.0;
                    let mut refs = Vec::new();
                    let code;
                    let mut text = Vec::new();
                    let name = || -> HsResult<String> {
                        Ok(tx
                            .query_row("SELECT name FROM sources WHERE did=?1", [&feed.publisher], |r| r.get::<_, String>(0))
                            .optional()?
                            .unwrap_or_else(|| feed.publisher.clone()))
                    };
                    if friend {
                        score += 3.0;
                        code = "friend";
                        let n = name()?;
                        text.push(format!("From your friend {n}"));
                        refs.push(json!({ "kind": "source", "id": followed.clone().unwrap_or_else(|| feed.publisher.clone()), "label": n }));
                    } else if let Some(source) = &followed {
                        score += 2.5;
                        code = "followed";
                        let n = name()?;
                        text.push(format!("From {n}, whom you follow"));
                        refs.push(json!({ "kind": "source", "id": source, "label": n }));
                    } else if subscription {
                        score += 1.5;
                        code = "subscription";
                        let path = paths.iter().find(|p| p.get("subscription_id").is_some()).unwrap();
                        let label = path.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
                        text.push(format!("From your subscription {label}"));
                        refs.push(json!({ "kind": "source", "id": path.get("subscription_id"), "label": label }));
                    } else if let Some(collector) = &collector {
                        score += 1.0;
                        code = "collector";
                        text.push("From a collector you use".to_string());
                        refs.push(json!({ "kind": "collector", "id": collector, "label": collector }));
                    } else {
                        code = "rule";
                    }
                    for topic in &subscribed {
                        score += 1.5;
                        refs.push(json!({ "kind": "topic", "id": topic.id, "label": topic.name }));
                    }
                    if !subscribed.is_empty() {
                        text.push(format!("matches {}", subscribed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")));
                    }
                    let code = if code == "rule" && !subscribed.is_empty() { "topic" } else { code };
                    let age_h = ((now - (feed.iat as i64 * 1000)) / 3_600_000).max(0) as f64;
                    score -= (age_h * 0.05).min(2.0);
                    score += affinity.get(&feed.publisher).copied().unwrap_or(0.0);
                    let reason = json!({ "code": code, "text": text.join("; "), "refs": refs });
                    if !hits.is_empty() {
                        // Filtered before preparation (§8.2); kept with its reason so the user can still look (A56).
                        tx.execute(
                            "UPDATE candidates SET selection='filtered', reason=?2, rule_ids=?3, score=?4, updated_at=?5 WHERE obj_id=?1",
                            params![obj_id, reason.to_string(), serde_json::to_string(&hits)?, score, now],
                        )?;
                        continue;
                    }
                    tx.execute("UPDATE candidates SET score=?2, updated_at=?3 WHERE obj_id=?1", params![obj_id, score, now])?;
                    scored.push((obj_id.clone(), score, reason));
                }
                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let reading: i64 = tx.query_row("SELECT COUNT(*) FROM reading", [], |r| r.get(0))?;
                let mut chosen = Vec::new();
                for (obj_id, score, reason) in scored {
                    if chosen.len() < batch && score >= 1.0 {
                        tx.execute("UPDATE candidates SET selection='preparing', reason=?2, updated_at=?3 WHERE obj_id=?1", params![obj_id, reason.to_string(), now])?;
                        chosen.push((obj_id, reason));
                    } else {
                        let why = if score < 1.0 { "below_threshold" } else if reading as usize >= window { "reading_window_full" } else { "batch_full" };
                        tx.execute(
                            "UPDATE candidates SET selection='not_selected', reason=?2, updated_at=?3 WHERE obj_id=?1 AND selection!='preparing'",
                            params![obj_id, why, now],
                        )?;
                    }
                }
                let mut changed = 0;
                for (obj_id, previous) in &pending {
                    let current: Option<String> = tx.query_row("SELECT selection FROM candidates WHERE obj_id=?1", [obj_id], |r| r.get(0)).optional()?;
                    if current.as_deref() != Some(previous.as_str()) {
                        changed += 1;
                    }
                }
                tx.commit()?;
                Ok((chosen, changed))
            })
            .await?;
        let (chosen, changed) = chosen;
        let mut admitted = 0;
        for (obj_id, reason) in chosen {
            if self.prepare_and_admit(&obj_id, reason).await? {
                admitted += 1;
            }
        }
        self.trim_reading_window().await?;
        if changed > 0 || admitted > 0 {
            self.bump(&["reading", "candidates"]);
        }
        Ok(admitted)
    }

    async fn prepare_and_admit(&self, obj_id: &str, reason: Value) -> HsResult<bool> {
        let obj_id = self.maybe_snapshot(obj_id).await.unwrap_or_else(|_| obj_id.to_string());
        let obj_id = obj_id.as_str();
        let state = self.prepare_resources(obj_id).await.unwrap_or(ResourceState::Unavailable);
        let id = obj_id.to_string();
        if state == ResourceState::Unavailable {
            let now = now_ms();
            self.db
                .call(move |c| {
                    c.execute(
                        "UPDATE candidates SET selection='not_selected', reason='resources_unavailable', resources='unavailable', updated_at=?2 WHERE obj_id=?1",
                        params![id, now],
                    )?;
                    Ok(())
                })
                .await?;
            return Ok(false);
        }
        // Judgements that need the body are completed now, and the filters checked again (§8.2).
        self.classify(obj_id, true).await.ok();
        let settings = self.settings().await?;
        let rules = settings.filter_rules.clone();
        let now = now_ms();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                let Some(feed) = get_feed(&tx, &id)? else { return Ok(false) };
                let (tags, classified) = effective_tags(&tx, &id, &feed, "global")?;
                let hits = filter_hits(&tags, &classified, &rules);
                if !hits.is_empty() {
                    tx.execute(
                        "UPDATE candidates SET selection='filtered', reason=?5, rule_ids=?2, resources=?3, updated_at=?4 WHERE obj_id=?1",
                        params![id, serde_json::to_string(&hits)?, state.as_str(), now, reason.to_string()],
                    )?;
                    tx.commit()?;
                    return Ok(false);
                }
                let topics = topics_for(&tags, &settings);
                tx.execute(
                    "INSERT INTO reading(obj_id, admitted_at, order_key, reason, resources, topics) VALUES (?1, ?2, ?2, ?3, ?4, ?5)
                     ON CONFLICT(obj_id) DO UPDATE SET resources=excluded.resources",
                    params![id, now, reason.to_string(), state.as_str(), serde_json::to_string(&topics)?],
                )?;
                tx.execute(
                    "INSERT INTO admission_history(obj_id, first_admitted_at) VALUES (?1, ?2)
                     ON CONFLICT(obj_id) DO UPDATE SET first_admitted_at=COALESCE(admission_history.first_admitted_at, excluded.first_admitted_at)",
                    params![id, now],
                )?;
                tx.execute(
                    "UPDATE candidates SET selection='admitted', resources=?2, updated_at=?3 WHERE obj_id=?1",
                    params![id, state.as_str(), now],
                )?;
                tx.commit()?;
                Ok(true)
            })
            .await
    }

    /// Leaving the window is not deletion, not "read", and the admission record stays (A39).
    async fn trim_reading_window(&self) -> HsResult<()> {
        let window = self.cfg.reading_window as i64;
        self.db
            .call(move |c| {
                c.execute(
                    "DELETE FROM reading WHERE obj_id IN (SELECT obj_id FROM reading ORDER BY order_key DESC LIMIT -1 OFFSET ?1)",
                    [window],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn expire_candidates(&self) -> HsResult<()> {
        let before = now_ms() - self.cfg.candidate_retention_days * 86_400_000;
        self.db
            .call(move |c| {
                c.execute("DELETE FROM candidates WHERE arrived_at<?1 AND selection!='admitted'", [before])?;
                Ok(())
            })
            .await
    }

    /// Reading query (§17.4): one list, views by filter / Topic / search; mute and filter rules
    /// are applied at read time so rule changes take effect at once.
    pub async fn list_reading(&self, query: ReadingQuery, cursor: Option<String>, page: usize) -> HsResult<ReadingPage> {
        let settings = self.settings().await?;
        let contacts = self.contacts.list().await;
        let names = self.identity_names().await;
        let since = now_ms() - self.cfg.candidate_retention_days * 86_400_000;
        self.db
            .call(move |c| {
                // Admitted entries plus candidates the rules filtered before admission: both are
                // "hidden by rules" and both can be shown on request (§8.5, A56).
                let mut stmt = c.prepare(
                    "SELECT obj_id, reason, order_key AS k FROM reading
                     UNION ALL
                     SELECT obj_id, COALESCE(reason, '{}'), arrived_at AS k FROM candidates
                       WHERE selection='filtered' AND arrived_at>=?1 AND obj_id NOT IN (SELECT obj_id FROM reading)
                     ORDER BY k DESC, obj_id",
                )?;
                let rows = stmt.query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                let mut visible = Vec::new();
                let (mut hidden_rules, mut hidden_mute) = (0, 0);
                let search = query.search.trim().to_lowercase();
                for (obj_id, reason) in rows {
                    let Some(feed) = get_feed(c, &obj_id)? else { continue };
                    let reason: Value = serde_json::from_str(&reason).unwrap_or_default();
                    let code = reason.get("code").and_then(Value::as_str).unwrap_or_default();
                    let content_type = feed.content.as_ref().map(|c| c.content_type);
                    let matches = match query.filter.as_str() {
                        "following" => code == "followed" || code == "friend",
                        "images" => content_type == Some(ContentType::Image),
                        "videos" => content_type == Some(ContentType::Video),
                        "longform" => content_type == Some(ContentType::Article),
                        "news" => feed.source.is_some(),
                        _ => true,
                    };
                    if !matches {
                        continue;
                    }
                    let (tags, classified) = effective_tags(c, &obj_id, &feed, "global")?;
                    if let Some(topic) = &query.topic_id {
                        if !topics_for(&tags, &settings).contains(topic) {
                            continue;
                        }
                    }
                    if !search.is_empty() {
                        let mut haystack = vec![feed.search_text().to_lowercase(), names.get(&feed.publisher).cloned().unwrap_or_default().to_lowercase()];
                        if let Some(source) = &feed.source {
                            haystack.push(source.original_author.clone().unwrap_or_default().to_lowercase());
                        }
                        if let Some(inner) = feed.wraps.as_deref().and_then(|w| get_feed(c, w).ok().flatten()) {
                            haystack.push(inner.search_text().to_lowercase());
                        }
                        if !haystack.iter().any(|h| h.contains(&search)) {
                            continue;
                        }
                    }
                    if is_muted(&feed.publisher, &settings.mute_rules, &contacts) {
                        hidden_mute += 1;
                        continue;
                    }
                    if !filter_hits(&tags, &classified, &settings.filter_rules).is_empty() && !query.show_filtered {
                        hidden_rules += 1;
                        continue;
                    }
                    visible.push(obj_id);
                }
                let start = match &cursor {
                    Some(cur) => visible.iter().position(|v| v == cur).map(|i| i + 1).unwrap_or(visible.len()),
                    None => 0,
                };
                let slice: Vec<String> = visible.iter().skip(start).take(page).cloned().collect();
                let next_cursor = if start + page < visible.len() && !slice.is_empty() { slice.last().cloned() } else { None };
                Ok(ReadingPage { obj_ids: slice, next_cursor, hidden_by_rules: hidden_rules, hidden_by_mute: hidden_mute, total: visible.len() })
            })
            .await
    }

    /// Followed but never admitted (§9.5): a query on candidates, not a second list.
    pub async fn list_followed_candidates(&self, cursor: Option<String>, include_read: bool, page: usize) -> HsResult<CandidatePage> {
        let settings = self.settings().await?;
        let contacts = self.contacts.list().await;
        let friends: Vec<String> = contacts.iter().filter(|c| c.friend).map(|c| c.did.clone()).collect();
        let retention = self.cfg.candidate_retention_days;
        let since = now_ms() - retention * 86_400_000;
        self.db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT c.obj_id, c.selection, c.source_paths, c.arrived_at, c.resources, h.first_admitted_at, h.opened_at, c.publisher
                     FROM candidates c LEFT JOIN admission_history h ON h.obj_id=c.obj_id
                     WHERE c.arrived_at>=?1 AND c.private_capture=0 ORDER BY c.arrived_at DESC",
                )?;
                type Row = (String, String, String, i64, String, Option<i64>, Option<i64>, String);
                let rows: Vec<Row> = stmt
                    .query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut entries = Vec::new();
                let mut read_count = 0;
                for (obj_id, selection, paths, arrived_at, resources, first_admitted, opened, publisher) in rows {
                    if first_admitted.is_some() || selection == "dropped" {
                        continue;
                    }
                    if crate::ingress::is_followed_source(c, &publisher)?.is_none() && !friends.contains(&publisher) {
                        continue;
                    }
                    let Some(feed) = get_feed(c, &obj_id)? else { continue };
                    if head_withdrawn(c, &feed)? || is_muted(&publisher, &settings.mute_rules, &contacts) {
                        continue;
                    }
                    let (tags, classified) = effective_tags(c, &obj_id, &feed, "global")?;
                    if !filter_hits(&tags, &classified, &settings.filter_rules).is_empty() {
                        continue;
                    }
                    if opened.is_some() {
                        read_count += 1;
                        if !include_read {
                            continue;
                        }
                    }
                    let mut source_paths = Vec::new();
                    for path in serde_json::from_str::<Vec<Value>>(&paths).unwrap_or_default() {
                        let transport = path.get("transport").and_then(Value::as_str).unwrap_or("pull").to_string();
                        source_paths.push(json!({ "transport": transport, "label": source_label(c, &path)? }));
                    }
                    let selection = match selection.as_str() {
                        "unscreened" => "unscreened",
                        "preparing" => "preparing",
                        _ => "not_selected",
                    };
                    entries.push(CandidateView {
                        obj_id,
                        selection: selection.into(),
                        source_paths,
                        arrived_at,
                        first_admitted_at: first_admitted,
                        opened_at: opened,
                        resources: if resources == "unknown" { "reachable".into() } else { resources },
                    });
                }
                let start = match &cursor {
                    Some(cur) => entries.iter().position(|e| &e.obj_id == cur).map(|i| i + 1).unwrap_or(entries.len()),
                    None => 0,
                };
                let total = entries.len();
                let slice: Vec<CandidateView> = entries.into_iter().skip(start).take(page).collect();
                let next_cursor = if start + page < total && !slice.is_empty() { slice.last().map(|e| e.obj_id.clone()) } else { None };
                let last_fetch_at: i64 = c.query_row("SELECT COALESCE(MAX(last_fetch_at), 0) FROM sources", [], |r| r.get(0))?;
                Ok(CandidatePage { entries: slice, next_cursor, read_count, retention_days: retention, last_fetch_at })
            })
            .await
    }

    /// Open a candidate directly (A38): prepare on demand, remember that it was read here.
    pub async fn open_candidate(&self, obj_id: &str) -> HsResult<(bool, ResourceState)> {
        let id = normalize_obj_id(obj_id);
        self.set_resource_state(&id, ResourceState::Preparing).await?;
        let state = self.prepare_resources(&id).await.unwrap_or(ResourceState::Unavailable);
        self.set_resource_state(&id, state).await?;
        if state == ResourceState::Unavailable {
            return Ok((false, state));
        }
        let now = now_ms();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO admission_history(obj_id, opened_at) VALUES (?1, ?2)
                     ON CONFLICT(obj_id) DO UPDATE SET opened_at=COALESCE(admission_history.opened_at, excluded.opened_at)",
                    params![id, now],
                )?;
                Ok(())
            })
            .await?;
        self.bump(&["candidates"]);
        Ok((true, state))
    }

    /// Add an opened candidate to the reading list by the user's choice (§9.5).
    pub async fn admit_candidate(&self, obj_id: &str) -> HsResult<bool> {
        let reason = json!({ "code": "rule", "text": "Added by you", "refs": [] });
        let admitted = self.prepare_and_admit(&normalize_obj_id(obj_id), reason).await?;
        self.bump(&["reading", "candidates"]);
        Ok(admitted)
    }

    pub async fn reading_entry(&self, obj_id: &str) -> HsResult<Option<Value>> {
        let settings = self.settings().await?;
        let id = obj_id.to_string();
        self.db
            .call(move |c| {
                let row: Option<(i64, String, String)> = c
                    .query_row(
                        "SELECT admitted_at, reason, resources FROM reading WHERE obj_id=?1
                         UNION ALL SELECT arrived_at, COALESCE(reason, '{}'), CASE resources WHEN 'unknown' THEN 'reachable' ELSE resources END
                           FROM candidates WHERE obj_id=?1 AND selection='filtered' LIMIT 1",
                        [&id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((admitted_at, reason, resources)) = row else { return Ok(None) };
                let Some(feed) = get_feed(c, &id)? else { return Ok(None) };
                let (tags, classified) = effective_tags(c, &id, &feed, "global")?;
                let hits = filter_hits(&tags, &classified, &settings.filter_rules);
                let topics = topics_for(&tags, &settings);
                Ok(Some(json!({
                    "objId": id,
                    "admittedAt": admitted_at,
                    "reason": serde_json::from_str::<Value>(&reason).unwrap_or_default(),
                    "effectiveTags": tags,
                    "resources": resources,
                    "filteredBy": hits,
                    "topics": topics,
                })))
            })
            .await
    }

    pub async fn topic_views(&self) -> HsResult<Value> {
        let settings = self.settings().await?;
        let contacts = self.contacts.list().await;
        let week_ago = now_ms() - 7 * 86_400_000;
        let counts = self
            .db
            .call({
                let settings = settings.clone();
                move |c| {
                    let mut stmt = c.prepare("SELECT obj_id FROM reading WHERE admitted_at>=?1")?;
                    let ids = stmt.query_map([week_ago], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                    let mut counts: std::collections::HashMap<String, usize> = Default::default();
                    for id in ids {
                        let Some(feed) = get_feed(c, &id)? else { continue };
                        let (tags, classified) = effective_tags(c, &id, &feed, "global")?;
                        if !filter_hits(&tags, &classified, &settings.filter_rules).is_empty() || is_muted(&feed.publisher, &settings.mute_rules, &contacts) {
                            continue;
                        }
                        for topic in topics_for(&tags, &settings) {
                            *counts.entry(topic).or_default() += 1;
                        }
                    }
                    Ok(counts)
                }
            })
            .await?;
        Ok(json!(settings
            .topics
            .iter()
            .map(|t| json!({ "id": t.id, "name": t.name, "tags": t.tags, "subscribed": t.subscribed, "recentCount": counts.get(&t.id).copied().unwrap_or(0) }))
            .collect::<Vec<_>>()))
    }

    /// Re-evaluate filters after rule or override changes: filtered candidates get another
    /// chance, admitted ones are recomputed at read time (A56).
    pub async fn rescreen(&self) -> HsResult<()> {
        self.db
            .call(|c| {
                c.execute("UPDATE candidates SET selection='not_selected' WHERE selection='filtered'", [])?;
                Ok(())
            })
            .await?;
        self.wake.select.notify_one();
        self.bump(&["reading", "candidates"]);
        Ok(())
    }
}
