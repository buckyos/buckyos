//! Owner kRPC (`/kapi/homestation`): the application capabilities of §17.4 for the Desktop
//! app, and the evaluation service for other apps and Agents (§6.4). Method table in
//! `doc/homestation/HomeStation 协议与实现.md` §4.

use crate::audience::{AudienceSpec, Reader};
use crate::auth::Caller;
use crate::error::{bad, HsError, HsResult};
use crate::evaluation::{EvalRequest, EvalTarget};
use crate::protocol::normalize_obj_id;
use crate::publish::{get_entry, get_head, list_entries, PublishInput};
use crate::settings::{CollectorRef, FilterRule, MuteRule, Topic};
use crate::sources::SourceResolution;
use crate::Station;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

fn arg<T: DeserializeOwned>(params: &Value, key: &str) -> HsResult<T> {
    serde_json::from_value(params.get(key).cloned().unwrap_or(Value::Null)).map_err(|e| bad(format!("parameter {key}: {e}")))
}

fn opt<T: DeserializeOwned>(params: &Value, key: &str) -> HsResult<Option<T>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| bad(format!("parameter {key}: {e}"))),
    }
}

fn reader_of(params: &Value, owner: &str) -> HsResult<Reader> {
    let Some(reader) = params.get("reader") else { return Ok(Reader::Owner) };
    Ok(match reader.get("kind").and_then(Value::as_str) {
        Some("owner") | None => Reader::Owner,
        Some("anonymous") => Reader::Anonymous,
        Some("did") => {
            let did = reader.get("did").and_then(Value::as_str).ok_or_else(|| bad("reader.did"))?;
            if did == owner {
                Reader::Owner
            } else {
                Reader::Did(did.to_string())
            }
        }
        Some(other) => return Err(bad(format!("unknown reader kind {other}"))),
    })
}

/// Methods a zone user who is not the owner may call (reading the owner's public face).
const VISITOR_METHODS: &[&str] = &["profile.get", "published.list", "item.get"];

impl Station {
    pub async fn handle_rpc(&self, caller: &Caller, method: &str, params: Value) -> HsResult<Value> {
        let is_owner = caller.did.as_deref() == Some(self.cfg.owner.as_str());
        if !is_owner {
            if !VISITOR_METHODS.contains(&method) {
                return Err(HsError::Forbidden("only the HomeStation owner may call this method".into()));
            }
            let visitor = caller.did.clone().map(Reader::Did).unwrap_or(Reader::Anonymous);
            return self.visitor_rpc(visitor, method, params).await;
        }
        let eval_scope = match caller.app_id.as_deref() {
            None => "global".to_string(),
            Some(app) if app.contains("control-panel") || app.contains("homestation") || app.contains("desktop") => "global".to_string(),
            Some(app) => format!("app:{app}"),
        };
        let p = &params;
        Ok(match method {
            "ui.versions" => json!(self.versions()),
            "ui.bootstrap" => self.bootstrap().await?,
            "prefs.get" => json!(self.settings().await?),

            "reading.list" => {
                let query = opt(p, "query")?.unwrap_or_default();
                let page = self.list_reading(query, opt(p, "cursor")?, opt::<usize>(p, "limit")?.unwrap_or(8).clamp(1, 50)).await?;
                let cards = self.cards(page.obj_ids.clone(), Reader::Owner).await?;
                let mut v = json!(page);
                v["cards"] = json!(cards);
                v
            }
            "reading.summary" => {
                let mut query: crate::selection::ReadingQuery = opt(p, "query")?.unwrap_or_default();
                query.show_filtered = false;
                let page = self.list_reading(query, None, 10_000).await?;
                json!({ "visible": page.total, "hiddenByRules": page.hidden_by_rules, "hiddenByMute": page.hidden_by_mute })
            }
            "candidates.list" => {
                let page = self
                    .list_followed_candidates(opt(p, "cursor")?, opt(p, "includeRead")?.unwrap_or(false), opt::<usize>(p, "limit")?.unwrap_or(8).clamp(1, 50))
                    .await?;
                let ids = page.entries.iter().map(|e| e.obj_id.clone()).collect();
                let cards = self.cards(ids, Reader::Owner).await?;
                let mut v = json!(page);
                v["cards"] = json!(cards);
                v
            }
            "candidates.open" => {
                let (ok, resources) = self.open_candidate(&arg::<String>(p, "objId")?).await?;
                json!({ "ok": ok, "resources": resources.as_str() })
            }
            "candidates.admit" => json!({ "ok": self.admit_candidate(&arg::<String>(p, "objId")?).await? }),

            "item.get" => json!(self.card(&arg::<String>(p, "objId")?, reader_of(p, &self.cfg.owner)?).await?),
            "item.cards" => json!(self.cards(arg(p, "objIds")?, reader_of(p, &self.cfg.owner)?).await?),
            "item.wrapped_body" => self.wrapped_body(&arg::<String>(p, "objId")?).await?,
            "item.retry_resources" => json!(self.retry_resources(&arg::<String>(p, "objId")?).await?.as_str()),
            "item.fetch" => {
                let obj_id: String = arg(p, "objId")?;
                let holder: String = arg(p, "from")?;
                self.fetch_object_from(&holder, &obj_id, crate::ingress::Arrival::Fetch).await?;
                json!(self.card(&obj_id, Reader::Owner).await?)
            }

            "comments.list" => {
                self.comment_list(&arg::<String>(p, "objId")?, &opt::<String>(p, "view")?.unwrap_or_else(|| "local".into()), &opt::<String>(p, "type")?.unwrap_or_else(|| "text".into()))
                    .await?
            }
            "comments.set_listing" => {
                self.set_author_listing(&arg::<String>(p, "target")?, &arg::<String>(p, "commentId")?, arg(p, "listed")?).await?;
                json!({ "ok": true })
            }
            "comments.sync" => json!({ "synced": self.sync_tracked().await? }),

            "published.list" => self.published_list(p).await?,
            "published.changes" => self.published_changes(p).await?,
            "published.entries" => self.entry_debug().await?,

            "interact.like" => self.set_like(&arg::<String>(p, "objId")?, arg(p, "on")?).await?,
            "interact.bookmark" => self.set_bookmark(&arg::<String>(p, "objId")?, arg(p, "on")?, opt(p, "public")?.unwrap_or(false)).await?,
            "interact.read_later" => self.set_read_later(&arg::<String>(p, "objId")?, arg(p, "on")?).await?,
            "interact.dislike" => self.set_dislike(&arg::<String>(p, "objId")?, arg(p, "on")?).await?,
            "interact.repost" => self.repost(&arg::<String>(p, "objId")?, arg(p, "on")?).await?,
            "interact.quote" => {
                let audience: AudienceSpec = match opt(p, "audience")? {
                    Some(a) => a,
                    None => self.settings().await?.default_audience,
                };
                let published = self.quote(&arg::<String>(p, "objId")?, &arg::<String>(p, "text")?, audience).await?;
                self.task_view_for(&published.entry, published.obj_id.clone(), &format!("quote-{}", published.obj_id.clone().unwrap_or_default())).await?
            }
            "interact.comment" => {
                let (published, audience) = self.comment(&arg::<String>(p, "objId")?, &arg::<String>(p, "text")?).await?;
                let task = self.task_view_for(&published.entry, published.obj_id.clone(), &format!("comment-{}", published.obj_id.clone().unwrap_or_default())).await?;
                json!({ "task": task, "audience": audience })
            }

            "entry.withdraw" => {
                self.withdraw(&arg::<String>(p, "entry")?).await?;
                json!({ "ok": true })
            }
            "entry.edit" => json!(self.edit(&arg::<String>(p, "entry")?, &arg::<String>(p, "text")?).await?.obj_id),
            "entry.set_audience" => {
                self.set_audience(&arg::<String>(p, "entry")?, arg(p, "audience")?).await?;
                json!({ "ok": true })
            }
            "entry.retry_delivery" => {
                self.retry_delivery(&arg::<String>(p, "entry")?).await?;
                json!({ "ok": true })
            }

            "publish.create" => {
                let key: String = arg(p, "key")?;
                let input: PublishInput = arg(p, "input")?;
                let row = self.publish(&key, input).await?;
                self.task_json(row).await?
            }
            "publish.retry" => {
                let row = self.retry_publish(&arg::<String>(p, "key")?).await?;
                self.task_json(row).await?
            }
            "publish.share_capture" => {
                let row = self.share_capture(&arg::<String>(p, "objId")?).await?;
                self.task_json(row).await?
            }
            "publish.task" => {
                let key: String = arg(p, "key")?;
                let row = self.db.call(move |c| crate::publish::get_task(c, &key)).await?.map(|(r, _)| r);
                match row {
                    Some(row) => self.task_json(row).await?,
                    None => Value::Null,
                }
            }
            "publish.link_preview" => {
                let preview = self.link_preview(&arg::<String>(p, "url")?).await?;
                json!({ "title": preview.title, "summary": preview.summary })
            }

            "saved.list" => self.list_saved(&arg::<String>(p, "kind")?).await?,

            "profile.get" => self.profile_view(&opt::<String>(p, "did")?.unwrap_or_else(|| self.cfg.owner.clone()), reader_of(p, &self.cfg.owner)?).await?,
            "profile.set" => {
                let name: Option<String> = opt(p, "name")?;
                let bio: Option<String> = opt(p, "bio")?;
                json!(self
                    .update_settings(move |s| {
                        if let Some(n) = name {
                            s.profile.name = n;
                        }
                        if let Some(b) = bio {
                            s.profile.bio = b;
                        }
                    })
                    .await?
                    .profile)
            }
            "profile.set_featured" => {
                let order: Vec<String> = arg(p, "order")?;
                let order: Vec<String> = order.iter().map(|o| normalize_obj_id(o)).collect();
                self.update_settings(move |s| s.profile.featured = order).await?;
                self.bump(&["profile"]);
                json!({ "ok": true })
            }

            "sources.list" => self.list_sources().await?,
            "sources.resolve" => json!(self.resolve_source_input(&arg::<String>(p, "kind")?, &arg::<String>(p, "text")?).await?),
            "sources.follow" => json!(self.follow_resolution(arg::<SourceResolution>(p, "resolution")?).await?),
            "sources.unfollow" => json!(self.unfollow(&arg::<String>(p, "sourceId")?).await?),
            "sources.pause" => {
                self.pause_source(&arg::<String>(p, "sourceId")?, arg(p, "paused")?).await?;
                json!({ "ok": true })
            }
            "sources.sync" => {
                match opt::<String>(p, "sourceId")? {
                    Some(id) => json!(self.sync_person(&id).await?),
                    None => {
                        self.pull_all().await?;
                        json!({ "ok": true })
                    }
                }
            }

            "prefs.set_mute_rule" => {
                let rule: MuteRule = arg(p, "rule")?;
                let on: bool = arg(p, "on")?;
                self.update_settings(move |s| {
                    s.mute_rules.retain(|r| !same_mute(r, &rule));
                    if on {
                        s.mute_rules.push(rule);
                    }
                })
                .await?;
                self.bump(&["reading", "candidates"]);
                json!({ "ok": true })
            }
            "prefs.set_filter_rule" => {
                let rule: FilterRule = arg(p, "rule")?;
                if rule.conditions.is_empty() || !(0.5..=0.99).contains(&rule.min_confidence) {
                    return Err(bad("a filter rule needs conditions and a threshold in 0.5–0.99"));
                }
                self.update_settings(move |s| match s.filter_rules.iter_mut().find(|r| r.id == rule.id) {
                    Some(existing) => *existing = rule,
                    None => s.filter_rules.push(rule),
                })
                .await?;
                self.rescreen().await?;
                json!({ "ok": true })
            }
            "prefs.set_tag_override" => {
                let obj_id: String = arg(p, "objId")?;
                let tag: String = arg(p, "tag")?;
                let value: Option<String> = opt(p, "override")?;
                let (action, replacement) = match value.as_deref() {
                    None => (None, None),
                    Some("remove") => (Some("remove"), None),
                    Some("confirm") => (Some("confirm"), None),
                    Some("to_assisted") => (Some("replace"), Some("ai_assisted".to_string())),
                    Some(other) => return Err(bad(format!("unknown override {other}"))),
                };
                self.set_tag_override(&obj_id, crate::selection::dimension_of_tag(&tag), &tag, action, replacement, "global").await?;
                self.rescreen().await?;
                json!({ "ok": true })
            }
            "prefs.set_default_audience" => {
                let audience: AudienceSpec = arg(p, "audience")?;
                audience.validate().map_err(bad)?;
                self.update_settings(move |s| s.default_audience = audience).await?;
                json!({ "ok": true })
            }
            "prefs.set_topics" => {
                let topics: Vec<Topic> = arg(p, "topics")?;
                self.update_settings(move |s| s.topics = topics).await?;
                self.bump(&["reading"]);
                json!({ "ok": true })
            }
            "prefs.set_collectors" => {
                let collectors: Vec<CollectorRef> = arg(p, "collectors")?;
                self.update_settings(move |s| s.collectors = collectors).await?;
                json!({ "ok": true })
            }
            "prefs.set_comments_open" => {
                let open: bool = arg(p, "open")?;
                self.update_settings(move |s| s.comments_open = open).await?;
                json!({ "ok": true })
            }
            "prefs.mark_less_like" => {
                self.record_event(&arg::<String>(p, "objId")?, "less_like", None).await?;
                json!({ "ok": true })
            }

            "feedback.record" => {
                self.record_event(&arg::<String>(p, "objId")?, &arg::<String>(p, "event")?, opt(p, "value")?).await?;
                json!({ "ok": true })
            }
            "consumption.join" => json!({
                "agreementId": self.set_agreement(&arg::<String>(p, "target")?, &arg::<String>(p, "receiver")?, &arg::<String>(p, "action")?, opt(p, "terms")?, opt(p, "joined")?.unwrap_or(true)).await?
            }),
            "consumption.report" => json!({ "objId": self.report_consumption(&arg::<String>(p, "agreementId")?, opt(p, "result")?).await? }),
            "consumption.list" => self.list_agreements().await?,

            "eval.evaluate" => self.evaluate(arg::<EvalRequest>(p, "request")?, &eval_scope).await?,
            "eval.refresh" => {
                let mut request: EvalRequest = arg(p, "request")?;
                request.refresh = true;
                self.evaluate(request, &eval_scope).await?
            }
            "eval.get" => self.get_evaluation(&arg::<String>(p, "id")?).await?,
            "eval.find" => json!(self.find_evaluation(&arg::<EvalTarget>(p, "target")?, opt(p, "profileId")?).await?),
            "eval.set_tag_override" => {
                let scope = match opt::<String>(p, "scope")?.as_deref() {
                    Some("global") => "global".to_string(),
                    _ => eval_scope.clone(),
                };
                let action: Option<String> = opt(p, "action")?;
                let id = self
                    .set_tag_override(&arg::<String>(p, "target")?, &arg::<String>(p, "dimension")?, &arg::<String>(p, "tag")?, action.as_deref(), opt(p, "replacement")?, &scope)
                    .await?;
                self.rescreen().await?;
                json!({ "overrideId": id, "scope": scope })
            }
            "eval.overrides" => json!(self.list_overrides(&arg::<String>(p, "target")?, &eval_scope).await?),
            "eval.changes" => self.evaluation_changes(opt(p, "since")?.unwrap_or(0), &eval_scope).await?,
            "eval.share" => {
                let obj = self.share_evaluation(&arg::<String>(p, "resultId")?).await?;
                json!({ "objId": obj.obj_id, "jwt": obj.jwt })
            }

            "admin.run" => {
                match arg::<String>(p, "task")?.as_str() {
                    "delivery" => json!({ "processed": self.process_due_deliveries().await? }),
                    "pull" => {
                        self.pull_all().await?;
                        json!({ "ok": true })
                    }
                    "selection" => {
                        self.run_eval_tasks().await?;
                        json!({ "admitted": self.run_selection().await? })
                    }
                    "comments" => json!({ "synced": self.sync_tracked().await? }),
                    "friends" => {
                        self.sync_friends().await?;
                        json!({ "ok": true })
                    }
                    other => return Err(bad(format!("unknown task {other}"))),
                }
            }
            "admin.compact_stream" => {
                self.compact_stream(arg(p, "through")?).await?;
                json!({ "ok": true })
            }
            _ => return Err(HsError::NotFound(format!("unknown method {method}"))),
        })
    }

    async fn visitor_rpc(&self, reader: Reader, method: &str, params: Value) -> HsResult<Value> {
        Ok(match method {
            "profile.get" => self.profile_view(&self.cfg.owner.clone(), reader).await?,
            "published.list" => {
                let mut params = params;
                params["owner"] = json!(self.cfg.owner);
                params["reader"] = match &reader {
                    Reader::Did(d) => json!({ "kind": "did", "did": d }),
                    _ => json!({ "kind": "anonymous" }),
                };
                self.published_list(&params).await?
            }
            "item.get" => json!(self.card(&arg::<String>(&params, "objId")?, reader).await?),
            _ => return Err(HsError::Forbidden("method not available".into())),
        })
    }

    async fn bootstrap(&self) -> HsResult<Value> {
        let settings = self.settings().await?;
        let contacts = self.contacts.list().await;
        let mut groups: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for c in &contacts {
            for g in &c.groups {
                groups.entry(g.clone()).or_default().push(c.did.clone());
            }
        }
        let groups: Vec<Value> = groups.into_iter().map(|(id, members)| json!({ "id": id, "name": id, "members": members })).collect();
        let friends: Vec<String> = contacts.iter().filter(|c| c.friend && c.did != self.cfg.owner).map(|c| c.did.clone()).collect();
        let summary = self.list_reading(crate::selection::ReadingQuery::default(), None, 10_000).await?;
        let candidates = self.list_followed_candidates(None, false, 10_000).await?;
        let preparing = candidates.entries.iter().filter(|e| e.selection == "preparing" || e.resources == "preparing").count();
        let (sources, failing): (i64, i64) = self
            .db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*), COUNT(CASE WHEN last_error IS NOT NULL AND paused=0 THEN 1 END) FROM sources",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await?;
        let identities: Vec<Value> = self
            .view_ctx(Reader::Owner)
            .await
            .names
            .iter()
            .map(|(did, (name, kind))| json!({ "did": did, "name": name, "kind": kind, "hue": crate::hue_of(did) }))
            .collect();
        let followers = self.active_followers().await?;
        let following: Vec<Value> = self
            .db
            .call(|c| {
                let mut stmt = c.prepare("SELECT did, name FROM sources WHERE kind='person' AND did IS NOT NULL AND basis!='[]' ORDER BY name")?;
                let rows = stmt.query_map([], |r| Ok(json!({ "did": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)? })))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(json!({
            "owner": self.cfg.owner,
            "zone": self.cfg.zone,
            "followers": followers,
            "following": following,
            "settings": settings,
            "groups": groups,
            "friends": friends,
            "identities": identities,
            "topics": self.topic_views().await?,
            "syncStatus": {
                "candidates": candidates.entries.len(),
                "preparing": preparing,
                "lastFetchAt": candidates.last_fetch_at,
                "sources": sources,
                "failingSources": failing,
            },
            "hiddenSummary": { "hiddenByRules": summary.hidden_by_rules, "hiddenByMute": summary.hidden_by_mute },
            "versions": self.versions(),
            "collector": self.cfg.collector,
        }))
    }

    async fn task_json(&self, row: crate::publish::PublishTaskRow) -> HsResult<Value> {
        let delivery = match &row.entry {
            Some(entry) => self.entry_delivery(entry).await?,
            None => None,
        };
        Ok(crate::projection::without_nulls(json!({
            "key": row.key,
            "stage": row.stage,
            "entry": row.entry,
            "objId": row.obj_id,
            "delivery": delivery,
            "error": row.error,
            "createdAt": row.created_at,
        })))
    }

    async fn task_view_for(&self, entry: &str, obj_id: Option<String>, key: &str) -> HsResult<Value> {
        let delivery = self.entry_delivery(entry).await?;
        Ok(json!({
            "key": key,
            "stage": "published",
            "entry": entry,
            "objId": obj_id,
            "delivery": delivery.unwrap_or(crate::delivery::DeliveryProgress { state: "delivered".into(), delivered: 0, total: 0 }),
            "createdAt": crate::now_ms(),
        }))
    }

    /// `listPublished(owner, { reader, kind, cursor })`: the owner's own stream as any reader
    /// sees it, or another publisher's stream read over the network.
    async fn published_list(&self, p: &Value) -> HsResult<Value> {
        let owner: String = opt(p, "owner")?.unwrap_or_else(|| self.cfg.owner.clone());
        let reader = reader_of(p, &self.cfg.owner)?;
        let kind: Option<String> = opt(p, "kind")?;
        let cursor: Option<String> = opt(p, "cursor")?;
        let limit = opt::<usize>(p, "limit")?.unwrap_or(10).clamp(1, 50);
        if owner != self.cfg.owner {
            return self.remote_published(&owner, &reader, kind, cursor, limit).await;
        }
        // Without a kind this is the profile's feed: withdrawn entries stay out even for the owner.
        let profile_feed = kind.is_none();
        let kind = kind.or_else(|| Some("feed".into()));
        let page = self.read_display(&reader, kind, cursor, limit, false).await?;
        let ctx = self.view_ctx(reader.clone()).await;
        let is_owner = reader == Reader::Owner;
        let mut entries = Vec::new();
        let mut cards = Vec::new();
        for item in &page.items {
            let entry = item.entry.clone();
            let ctx2 = ctx.clone();
            let (view, card) = self
                .db
                .call(move |c| {
                    let Some(row) = get_entry(c, &entry)? else { return Ok((Value::Null, None)) };
                    let head = get_head(c, &entry)?;
                    let versions = crate::publish::entry_versions(c, &entry)?;
                    let obj_id = head.as_ref().and_then(|h| h.current.clone()).or_else(|| versions.last().cloned());
                    let head_view = match &obj_id {
                        Some(id) => match crate::objects::get_feed(c, id)? {
                            Some(feed) => crate::projection::entry_state(c, id, &feed)?,
                            None => None,
                        },
                        None => None,
                    }
                    .unwrap_or_else(|| {
                        json!({
                            "entry": entry, "seq": head.as_ref().map(|h| h.seq).unwrap_or(0),
                            "state": head.as_ref().map(|h| h.state.as_str()).unwrap_or("active"),
                            "isLatest": false, "version": 1, "versionCount": versions.len().max(1),
                        })
                    });
                    let spec = if ctx2.is_owner() { row.audience.clone() } else { crate::projection::tier_spec(row.audience.tier()) };
                    let card = match &obj_id {
                        Some(id) if crate::protocol::obj_type_of(id).as_deref() == Some(crate::protocol::OBJ_TYPE_FEED) => crate::projection::card_view(c, &ctx2, id, None)?,
                        _ => None,
                    };
                    Ok((
                        json!({
                            "entry": entry,
                            "objId": obj_id,
                            "head": head_view,
                            "audience": { "spec": spec, "restricted": !row.audience.is_public() },
                            "kind": row.kind.as_str(),
                            "publishedAt": row.created_at,
                        }),
                        card,
                    ))
                })
                .await?;
            if view.is_null() || (profile_feed && view["head"]["state"] == "withdrawn") {
                continue;
            }
            let mut view = view;
            if is_owner {
                let entry = item.entry.clone();
                let task: Option<crate::publish::PublishTaskRow> = self
                    .db
                    .call(move |c| {
                        let key: Option<String> =
                            rusqlite::OptionalExtension::optional(c.query_row("SELECT key FROM publish_tasks WHERE entry=?1", [&entry], |r| r.get(0)))?;
                        Ok(match key {
                            Some(k) => crate::publish::get_task(c, &k)?.map(|(r, _)| r),
                            None => None,
                        })
                    })
                    .await?;
                view["task"] = match task {
                    Some(t) => self.task_json(t).await?,
                    None => match self.entry_delivery(&item.entry).await? {
                        Some(d) => json!({ "key": format!("entry-{}", item.entry), "stage": "published", "entry": item.entry, "objId": view["objId"], "delivery": d, "createdAt": view["publishedAt"] }),
                        None => Value::Null,
                    },
                };
            }
            entries.push(crate::projection::without_nulls(view));
            if let Some(card) = card {
                cards.push(card);
            }
        }
        Ok(json!({ "entries": entries, "nextCursor": page.next, "changeCursor": page.change_cursor, "cards": cards }))
    }

    /// Visitor portal of another publisher (`/homestation/u/:did`): a display read signed as
    /// the owner, verified and stored like any other pulled content.
    /// Another publisher's stream as the owner reads it, or as an anonymous visitor. Another
    /// reader's view cannot be produced here (that reader's identity is not ours to present), so
    /// it falls back to the anonymous view and says so.
    async fn remote_published(&self, owner: &str, reader: &Reader, kind: Option<String>, cursor: Option<String>, limit: usize) -> HsResult<Value> {
        let as_owner = *reader == Reader::Owner;
        let (zone, origin) = self.origin_for_did(owner).await?;
        let mut path = format!("/home/feed?mode=display&objects=1&limit={limit}");
        if let Some(kind) = &kind {
            path.push_str(&format!("&kind={kind}"));
        }
        if let Some(cursor) = &cursor {
            path.push_str(&format!("&cursor={}", url_encode(cursor)));
        }
        let response = self.remote_get_as(&zone, &origin, &path, as_owner).await?;
        if !response.status().is_success() {
            return Err(HsError::Unavailable(format!("stream of {owner}: HTTP {}", response.status())));
        }
        let page: crate::stream::DisplayPage = response.json().await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        for (id, body) in &page.objects {
            let _ = self.ingest_expected(id, body, crate::ingress::Arrival::Fetch, false).await;
        }
        let mut entries = Vec::new();
        let mut obj_ids = Vec::new();
        for item in &page.items {
            if let Ok(v) = crate::objects::verify_jwt(self.directory.as_ref(), &item.head, Some(crate::protocol::OBJ_TYPE_HEAD)).await {
                if v.publisher != owner {
                    continue;
                }
                let _ = self.ingest_head(v, item.restricted).await;
                self.set_entry_tier(&item.entry, &item.tier).await?;
            }
            let (entry, current) = (item.entry.clone(), item.current.clone());
            let head = self
                .db
                .call(move |c| {
                    let Some(id) = current else { return Ok(None) };
                    match crate::objects::get_feed(c, &id)? {
                        Some(feed) => crate::projection::entry_state(c, &id, &feed),
                        None => Ok(None),
                    }
                })
                .await?
                .unwrap_or_else(|| {
                    let claims = crate::sign::decode_unverified(&item.head).map(|d| d.claims).unwrap_or_default();
                    json!({ "entry": entry, "seq": claims.get("seq"), "state": claims.get("state"), "currentObjId": item.current,
                            "isLatest": true, "version": 1, "versionCount": 1 })
                });
            entries.push(crate::projection::without_nulls(json!({
                "entry": item.entry,
                "objId": item.current,
                "head": head,
                "audience": { "spec": crate::projection::tier_spec(&item.tier), "restricted": item.restricted },
                "kind": item.kind,
                "publishedAt": item.iat * 1000,
            })));
            if let Some(c) = &item.current {
                obj_ids.push(c.clone());
            }
        }
        let mut cards = self.cards(obj_ids, Reader::Owner).await?;
        if !as_owner {
            for card in cards.iter_mut() {
                if let Some(map) = card.as_object_mut() {
                    map.remove("personal");
                    map.remove("reading");
                }
            }
        }
        let mut out = json!({ "entries": entries, "nextCursor": page.next, "changeCursor": page.change_cursor, "cards": cards });
        if matches!(reader, Reader::Did(_)) {
            out["readerApproximated"] = json!(true);
        }
        Ok(out)
    }

    async fn published_changes(&self, p: &Value) -> HsResult<Value> {
        let reader = reader_of(p, &self.cfg.owner)?;
        let after: i64 = opt(p, "after")?.unwrap_or(0);
        let page = self.read_changes(&reader, after, 500, false).await?;
        let changes: Vec<Value> = page
            .changes
            .iter()
            .map(|c| {
                let head = crate::sign::decode_unverified(&c.head).map(|d| d.claims).unwrap_or_default();
                json!({
                    "cursor": c.cursor,
                    "entry": c.entry,
                    "seq": c.seq,
                    "state": head.get("state").cloned().unwrap_or(json!("active")),
                    "current": head.get("current"),
                    "kind": c.kind,
                    "at": head.get("updated_at_ms"),
                })
            })
            .collect();
        Ok(json!(changes))
    }

    async fn entry_debug(&self) -> HsResult<Value> {
        self.db
            .call(|c| {
                let rows = list_entries(c, "ORDER BY updated_at DESC", &[])?;
                let mut out = Vec::new();
                for row in rows {
                    let mut stmt = c.prepare("SELECT seq, state, current, at FROM head_history WHERE entry=?1 ORDER BY seq")?;
                    let heads = stmt
                        .query_map([&row.entry], |r| Ok(json!({ "seq": r.get::<_, i64>(0)?, "state": r.get::<_, String>(1)?, "current": r.get::<_, Option<String>>(2)?, "at": r.get::<_, i64>(3)? })))?
                        .collect::<Result<Vec<_>, _>>()?;
                    out.push(json!({ "entry": row.entry, "kind": row.kind.as_str(), "audience": row.audience, "heads": heads }));
                }
                Ok(json!(out))
            })
            .await
    }

    async fn profile_view(&self, did: &str, reader: Reader) -> HsResult<Value> {
        if did == self.cfg.owner {
            let profile = self.public_profile(&reader).await?;
            return Ok(json!({
                "did": profile.did,
                "name": profile.name,
                "bio": profile.bio,
                "hue": crate::hue_of(&profile.did),
                "followers": profile.followers,
                "following": profile.following,
                "posts": profile.posts,
                "featured": profile.featured,
                "stream": profile.stream,
                "inbox": profile.inbox,
            }));
        }
        let remote = self.fetch_remote_profile_as(did, reader == Reader::Owner).await?;
        let approximated = matches!(reader, Reader::Did(_));
        Ok(crate::projection::without_nulls(json!({
            "did": did,
            "name": remote.get("name"),
            "bio": remote.get("bio"),
            "hue": crate::hue_of(did),
            "followers": remote.get("followers"),
            "following": remote.get("following"),
            "posts": remote.get("posts"),
            "featured": remote.get("featured"),
            "stream": remote.get("stream"),
            "inbox": remote.get("inbox"),
            "readerApproximated": approximated.then_some(true),
        })))
    }

    pub async fn set_entry_tier(&self, entry: &str, tier: &str) -> HsResult<()> {
        let (entry, tier) = (entry.to_string(), tier.to_string());
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE heads SET tier=?2, restricted=?3 WHERE entry=?1 AND NOT EXISTS (SELECT 1 FROM published WHERE entry=?1)",
                    rusqlite::params![entry, tier, (tier != "public") as i64],
                )?;
                Ok(())
            })
            .await
    }
}

fn same_mute(a: &MuteRule, b: &MuteRule) -> bool {
    match (a, b) {
        (MuteRule::Person { did: x, .. }, MuteRule::Person { did: y, .. }) => x == y,
        (MuteRule::Group { group_id: x, .. }, MuteRule::Group { group_id: y, .. }) => x == y,
        _ => false,
    }
}

pub fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
