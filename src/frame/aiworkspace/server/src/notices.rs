//! System Notify notices of the AI Workspace (README “Notices”): comments on what somebody made or talked about, and
//! the reminders of the standard to-do and calendar tables. The store finds them (`aiworkspace_store::notices`); here
//! it is decided who is told — Zone users who may read it now, never what they wrote themselves — and the notice goes
//! to msg-center as this service.

use crate::recurrence::{date_of, day_of, expand, local_day, zoned, Rule, Start};
use crate::{blocking, pages, prepare_caller, AppState};
use aiworkspace_core::standard::{CALENDAR_SCHEMA, TODO_SCHEMA};
use aiworkspace_core::value::parse_datetime_ms;
use aiworkspace_core::{WsError, WsResult};
use aiworkspace_store::notices::{Comment, Place, ReminderRecord};
use aiworkspace_store::Caller;
use buckyos_api::{NotifyCategory, NotifyLevel, NotifySendReq, NotifySource};
use chrono::TimeZone;
use chrono_tz::Tz;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, Weak};

const TICK_SECS: u64 = 60;
/// After notices about the comments of a Workspace, the next ones wait this long and go out together.
const COMMENTS_INTERVAL_MS: i64 = 5 * 60_000;
/// Comments nobody could be told of for this long are dropped.
const COMMENT_MAX_AGE_MS: i64 = 24 * 3_600_000;
/// A reminder is sent at most this late (the service was down); older ones are dropped.
const REMINDER_MAX_LATE_MS: i64 = 3_600_000;
/// How often the point reminders reached is written down when nothing was sent (a restart sends what followed it again,
/// under the same keys).
const REMINDED_SAVE_MS: i64 = 15 * 60_000;
/// How long a reminder stays open after what it reminds of.
const REMINDER_OPEN_MS: i64 = 24 * 3_600_000;
/// The reminders of a date (an all-day event, a to-do's due date) count from this hour of it.
const DATE_MINUTE: i64 = 9 * 60;
const MAX_LINES: usize = 5;
const MAX_TEXT_CHARS: usize = 200;

#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    /// Whether there is a message center to send to (none in standalone mode).
    fn available(&self) -> bool;
    async fn send(&self, req: NotifySendReq) -> Result<(), String>;
}

pub struct RuntimeNotifier;

#[async_trait::async_trait]
impl Notifier for RuntimeNotifier {
    fn available(&self) -> bool {
        buckyos_api::get_buckyos_api_runtime().is_ok()
    }

    async fn send(&self, req: NotifySendReq) -> Result<(), String> {
        let client = buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?.get_msg_center_client().await.map_err(|e| e.to_string())?;
        client.notify_send(req).await.map(|_| ()).map_err(|e| e.to_string())
    }
}

/// What the reminders of a Workspace need between two ticks.
#[derive(Clone)]
struct Watched {
    /// The records as of a head: read again only when it moved.
    epoch: String,
    head_seq: u64,
    records: Arc<Vec<ReminderRecord>>,
    /// Reminders were looked at up to here (ms), and up to here in the Workspace's own state.
    until: i64,
    saved: i64,
}

#[derive(Default)]
pub struct Reminders(Mutex<HashMap<String, Watched>>);

pub async fn run(state: Weak<AppState>) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(TICK_SECS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        timer.tick().await;
        let Some(state) = state.upgrade() else { break };
        tick(&state).await;
    }
}

/// What is new in every Workspace.
pub async fn tick(state: &Arc<AppState>) {
    if !state.notifier.available() {
        return;
    }
    let st = state.clone();
    let Ok(ids) = blocking(move || st.svc.all_workspace_ids()).await else { return };
    // without the account list nobody can be addressed: the next minute tries again
    let users: BTreeSet<String> = state
        .auth
        .subjects()
        .await
        .into_iter()
        .filter(|s| s["kind"] == "user")
        .filter_map(|s| s["subject"].as_str().map(str::to_string))
        .collect();
    if users.is_empty() {
        return;
    }
    for id in ids {
        if let Err(e) = workspace_tick(state, &id, &users).await {
            log::warn!("notices of {id}: {e}");
        }
    }
}

pub async fn workspace_tick(state: &Arc<AppState>, ws_id: &str, users: &BTreeSet<String>) -> WsResult<()> {
    let (st, w) = (state.clone(), ws_id.to_string());
    // not from the copies a show or a session runs on
    let gives = blocking(move || Ok(st.svc.session_of_run(&w)?.is_none() && st.with_ws(&w, |ws| Ok(ws.purpose().is_none()))?)).await?;
    if !gives {
        return Ok(());
    }
    comments(state, ws_id, users).await?;
    reminders(state, ws_id, users).await
}

fn shorten(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let mut out: String = line.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// The page a notice opens: the Surface (and the Block, record and occurrence on it), else the Workspace.
fn link(state: &Arc<AppState>, ws_id: &str, slug: &str, place: &Place, occurrence: Option<&str>) -> Option<String> {
    let surface = place.surface.as_ref().map(|(title, id)| (title.as_str(), id.as_str()));
    let mut url = pages::page_url(state, ws_id, slug, surface)?;
    if surface.is_some() {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in [("block", place.block.as_deref()), ("record", place.record.as_deref()), ("occ", occurrence)] {
            if let Some(value) = value {
                query.append_pair(key, value);
            }
        }
        let query = query.finish();
        if !query.is_empty() {
            url = format!("{url}&{query}");
        }
    }
    Some(url)
}

/// `@name` in a comment, when `name` is an account (and no mail address).
fn mentioned<'a>(body: &'a str, users: &BTreeSet<String>) -> BTreeSet<&'a str> {
    let part = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.');
    body.match_indices('@')
        .filter(|(i, _)| !body[..*i].ends_with(part))
        .filter_map(|(i, _)| {
            let rest = &body[i + 1..];
            let end = rest.find(|c: char| !part(c)).unwrap_or(rest.len());
            Some(rest[..end].trim_end_matches('.')).filter(|name| users.contains(*name))
        })
        .collect()
}

/// The account as the service knows it now, its memberships settled; `None` when it may not act.
async fn account(state: &Arc<AppState>, ws_id: &str, user: &str) -> Option<Caller> {
    let mut caller = state.auth.share_author(user).await.ok()?;
    prepare_caller(state, &mut caller, "notices", Some(ws_id)).await;
    Some(caller)
}

async fn comments(state: &Arc<AppState>, ws_id: &str, users: &BTreeSet<String>) -> WsResult<()> {
    let (st, w) = (state.clone(), ws_id.to_string());
    let (found, upto, told_at, now, title, slug) = blocking(move || {
        st.with_ws(&w, |ws| {
            let (found, upto) = ws.untold_comments()?;
            Ok((found, upto, ws.comments_told_at()?, (ws.clock)(), ws.title(), ws.workspace_slug()))
        })
    })
    .await?;
    if found.is_empty() || now - told_at < COMMENTS_INTERVAL_MS {
        return Ok(());
    }
    let mut concerned: BTreeMap<String, Vec<(Comment, bool)>> = BTreeMap::new();
    for c in found.into_iter().filter(|c| now - c.at_ms < COMMENT_MAX_AGE_MS) {
        let named = mentioned(&c.body, users);
        let everyone: BTreeSet<&str> = c.concerns.iter().map(String::as_str).filter(|p| users.contains(*p)).chain(named.iter().copied()).collect();
        for user in everyone.into_iter().filter(|u| *u != c.author) {
            concerned.entry(user.to_string()).or_default().push((c.clone(), named.contains(user)));
        }
    }
    let mut sent = false;
    for (user, list) in concerned {
        let Some(caller) = account(state, ws_id, &user).await else { continue };
        let (st, w) = (state.clone(), ws_id.to_string());
        let ids: Vec<String> = list.iter().map(|(c, _)| c.annotation_id.clone()).collect();
        let readable = blocking(move || st.with_ws(&w, |ws| ws.readable_comments(&caller, &ids))).await?;
        let list: Vec<(Comment, bool)> = list.into_iter().filter(|(c, _)| readable.contains(&c.annotation_id)).collect();
        if list.is_empty() {
            continue;
        }
        let req = comment_notice(state, ws_id, &title, &slug, &user, &list);
        state.notifier.send(req).await.map_err(|e| WsError::io(format!("notify {user}: {e}")))?;
        sent = true;
    }
    let (st, w) = (state.clone(), ws_id.to_string());
    blocking(move || st.with_ws(&w, |ws| ws.comments_told(upto, sent.then_some(now)))).await
}

fn comment_notice(state: &Arc<AppState>, ws_id: &str, ws_title: &str, slug: &str, user: &str, list: &[(Comment, bool)]) -> NotifySendReq {
    let (first, named) = &list[0];
    // a guest goes by the name it chose, which may be anybody's
    let who = |c: &Comment| c.author_name.as_ref().map(|n| format!("{n} (guest)")).unwrap_or_else(|| c.author.clone());
    let any_named = list.iter().any(|(_, named)| *named);
    let (title, message, place) = if list.len() == 1 {
        let what = if *named { "mentioned you in a comment on" } else { "commented on" };
        (format!("{} {what} “{}”", who(first), shorten(&first.title, 60)), shorten(&first.body, MAX_TEXT_CHARS), first.place.clone())
    } else {
        let mut lines: Vec<String> =
            list.iter().take(MAX_LINES).map(|(c, _)| format!("{} on “{}”: {}", who(c), shorten(&c.title, 60), shorten(&c.body, 80))).collect();
        if list.len() > MAX_LINES {
            lines.push(format!("and {} more", list.len() - MAX_LINES));
        }
        // one place when all are on the same thing
        let place = if list.iter().all(|(c, _)| c.place == first.place) { first.place.clone() } else { Place { record: None, block: None, ..first.place.clone() } };
        let place = if list.iter().all(|(c, _)| c.place.surface == first.place.surface) { place } else { Place::default() };
        (format!("{} new comments in “{}”", list.len(), shorten(ws_title, 60)), lines.join("\n"), place)
    };
    let mut req = NotifySendReq::notice(user, title, message, format!("aiws:comments:{ws_id}:{}:{user}", first.annotation_id));
    req.level = Some(if any_named { NotifyLevel::Timely } else { NotifyLevel::Normal });
    req.source = Some(NotifySource {
        operation: Some(format!("Comments in the workspace “{}”", shorten(ws_title, 60))),
        link: link(state, ws_id, slug, &place, None),
        ..Default::default()
    });
    req
}

async fn reminders(state: &Arc<AppState>, ws_id: &str, users: &BTreeSet<String>) -> WsResult<()> {
    let (st, w) = (state.clone(), ws_id.to_string());
    let (watched, since, now, owner, title, slug) = blocking(move || {
        st.with_ws(&w, |ws| {
            let now = (ws.clock)();
            let before = st.reminders.0.lock().unwrap().get(&w).filter(|x| x.epoch == ws.epoch).cloned();
            let since = match &before {
                Some(x) => x.until,
                None => ws.reminded_until(now)?,
            };
            let records = match &before {
                Some(x) if x.head_seq == ws.head_seq => x.records.clone(),
                _ => Arc::new(ws.reminder_records()?),
            };
            let saved = before.map_or(since, |x| x.saved);
            let watched = Watched { epoch: ws.epoch.clone(), head_seq: ws.head_seq, records, until: now, saved };
            Ok((watched, since, now, ws.owner()?.unwrap_or_default(), ws.title(), ws.workspace_slug()))
        })
    })
    .await?;
    let since = since.max(now - REMINDER_MAX_LATE_MS);
    let mut sent = false;
    for record in watched.records.iter() {
        let Some((user, tz, due)) = reminded(state, ws_id, record, &owner, users, since, now).await? else { continue };
        for due in due {
            let req = reminder_notice(state, ws_id, &title, &slug, &user, record, &due, tz);
            state.notifier.send(req).await.map_err(|e| WsError::io(format!("notify {user}: {e}")))?;
            sent = true;
        }
    }
    let mut watched = watched;
    if !watched.records.is_empty() && (sent || now - watched.saved >= REMINDED_SAVE_MS) {
        let (st, w) = (state.clone(), ws_id.to_string());
        blocking(move || st.with_ws(&w, |ws| ws.set_reminded_until(now))).await?;
        watched.saved = now;
    }
    state.reminders.0.lock().unwrap().insert(ws_id.to_string(), watched);
    Ok(())
}

/// What `record` reminds of in `(since, now]`, and whom: a to-do's assignee, who wrote the record, the owner — the first
/// that is an account and may read the table — in the time zone they chose. `None` when nothing is due.
async fn reminded(state: &Arc<AppState>, ws_id: &str, record: &ReminderRecord, owner: &str, users: &BTreeSet<String>, since: i64, now: i64)
                  -> WsResult<Option<(String, Tz, Vec<Due>)>> {
    let assignee = (record.schema_id == TODO_SCHEMA).then(|| record.values["ext"]["assignee"].as_str()).flatten();
    let mut candidates: Vec<(String, Tz, Vec<Due>)> = Vec::new();
    for user in [assignee, record.created_by.as_deref(), Some(owner)].into_iter().flatten().filter(|u| users.contains(*u)) {
        if candidates.iter().any(|(u, ..)| u == user) {
            continue;
        }
        let prefs = state.svc.users.get_prefs(&Caller::user(user)).ok();
        let tz = prefs.and_then(|p| p["prefs"]["timezone"].as_str().and_then(|tz| tz.parse().ok())).unwrap_or(Tz::UTC);
        candidates.push((user.to_string(), tz, due_reminders(record, tz, since, now)));
    }
    if candidates.iter().all(|(.., due)| due.is_empty()) {
        return Ok(None);
    }
    for (user, tz, due) in candidates {
        let Some(caller) = account(state, ws_id, &user).await else { continue };
        let (st, w, table) = (state.clone(), ws_id.to_string(), record.table_id.clone());
        if blocking(move || st.with_ws(&w, |ws| ws.may_read_entity(&caller, &table))).await? {
            return Ok(Some((user, tz, due)));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Due {
    /// What the reminder counts back from.
    at_ms: i64,
    /// The date, when there is no time.
    date: Option<String>,
    before_min: i64,
    /// The occurrence of a series.
    occurrence: Option<String>,
}

/// The reminders of a record that fall in `(since, now]`.
fn due_reminders(record: &ReminderRecord, tz: Tz, since: i64, now: i64) -> Vec<Due> {
    let v = &record.values;
    let ext = &v["ext"];
    let before: BTreeSet<i64> = ext["reminders"].as_array().into_iter().flatten().filter_map(|r| r["before_min"].as_i64()).collect();
    let (Some(first), Some(last)) = (before.first().copied(), before.last().copied()) else { return Vec::new() };
    let dated = record.schema_id == TODO_SCHEMA || v["all_day"] == Value::Bool(true);
    let date = if record.schema_id == TODO_SCHEMA { v["due"].as_str() } else { v["start_date"].as_str() };
    let start = match dated {
        true => date.and_then(day_of).map(|day| Start::Date { day, minute: DATE_MINUTE, tz }),
        // a series repeats in its own zone
        false => v["start"].as_str().and_then(parse_datetime_ms).map(|ms| Start::Time { ms, tz: ext["tz"].as_str().and_then(|z| z.parse().ok()).unwrap_or(tz) }),
    };
    let Some(start) = start else { return Vec::new() };
    let own = match start {
        Start::Date { day, minute, tz } => (day, zoned(day, minute * 60, tz)),
        Start::Time { ms, tz } => (local_day(ms, tz), ms),
    };
    let (lo, hi) = (since + first * 60_000, now + last * 60_000);
    let repeats = record.schema_id == CALENDAR_SCHEMA && ext.get("recurrence_id").is_none();
    let rule = ext["rrule"].as_str().filter(|_| repeats).and_then(Rule::parse);
    let listed = ext["occurrences"].as_array().filter(|o| repeats && !o.is_empty());
    let mut series = true;
    let occurrences: Vec<(i64, i64)> = if let Some(rule) = rule {
        expand(&rule, start, lo, hi).into_iter().map(|o| (o.day, o.at_ms)).collect()
    } else if let Some(listed) = listed {
        listed
            .iter()
            .filter_map(|o| o.as_str())
            .filter_map(|o| match start {
                Start::Date { minute, tz, .. } => day_of(o).map(|day| (day, zoned(day, minute * 60, tz))),
                Start::Time { tz, .. } => parse_datetime_ms(o).map(|ms| (local_day(ms, tz), ms)),
            })
            .collect()
    } else {
        series = false;
        vec![own]
    };
    // an original is compared by what it names, however it is written
    let named = |list: Vec<&str>| -> BTreeSet<i64> { list.into_iter().filter_map(|o| if dated { day_of(o) } else { parse_datetime_ms(o) }).collect() };
    let mut left_out = named(record.overridden.iter().map(String::as_str).collect());
    left_out.extend(named(ext["exdate"].as_array().into_iter().flatten().filter_map(Value::as_str).collect()));
    let mut out = Vec::new();
    for (day, at_ms) in occurrences {
        if series && left_out.contains(if dated { &day } else { &at_ms }) {
            continue;
        }
        for before_min in before.iter().copied() {
            let when = at_ms - before_min * 60_000;
            if when > since && when <= now {
                let original = if dated { date_of(day) } else { aiworkspace_core::value::format_utc_ms(at_ms) };
                out.push(Due { at_ms, date: dated.then(|| date_of(day)), before_min, occurrence: series.then_some(original) });
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn reminder_notice(state: &Arc<AppState>, ws_id: &str, ws_title: &str, slug: &str, user: &str, record: &ReminderRecord, due: &Due, tz: Tz) -> NotifySendReq {
    let todo = record.schema_id == TODO_SCHEMA;
    let name = record.values["title"].as_str().map(|t| shorten(t, 120)).filter(|t| !t.is_empty());
    let title = name.unwrap_or_else(|| if todo { "To-do".to_string() } else { "Event".to_string() });
    let mut message = match (&due.date, todo) {
        (Some(date), true) => format!("Due {date}"),
        (Some(date), false) => format!("{date}, all day"),
        (None, _) => {
            let local = tz.timestamp_millis_opt(due.at_ms).unwrap();
            format!("Starts {} ({})", local.format("%Y-%m-%d %H:%M"), tz.name())
        }
    };
    if let Some(location) = record.values["ext"]["location"].as_str().filter(|l| !l.trim().is_empty()) {
        message.push_str(&format!(" · {}", shorten(location, 120)));
    }
    let key = format!("aiws:reminder:{ws_id}:{}:{}:{}:{}:{user}", record.table_id, record.record_id, due.at_ms, due.before_min);
    let mut req = NotifySendReq::notice(user, title, message, key);
    req.level = Some(NotifyLevel::Timely);
    req.category = todo.then_some(NotifyCategory::Todo);
    req.expires_at_ms = u64::try_from(due.at_ms + REMINDER_OPEN_MS).ok();
    req.source = Some(NotifySource {
        operation: Some(format!("{} in the workspace “{}”", if todo { "A to-do" } else { "An event" }, shorten(ws_title, 60))),
        link: link(state, ws_id, slug, &record.place, due.occurrence.as_deref()),
        ..Default::default()
    });
    req
}
