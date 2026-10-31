//! `session.*` and the session channel (第四期规划 §3–§8, TODO I9/I10): the transport side of a presentation session.
//! The store decides (`aiworkspace_store::session`); this module authenticates links and sessions, keeps one in-memory
//! room per running session — its live connections, presence, the active control terminal and a broadcast of the
//! timeline — and serves the WebSocket channel (`GET /kapi/aiworkspace/session/ws`) with a long-poll fallback
//! (`session.watch`). Every state change is confirmed in the run Workspace first, then fanned out from its events.

use crate::{blocking, AppState};
use aiworkspace_core::access::Access;
use aiworkspace_core::model::{PARTICIPANT_PREFIX, SCOPE_SHARED};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::session::registry::{InviteRow, SessionRow, LIVE};
use aiworkspace_store::session::{event_visible, Actor, FeedSel, Role};
use aiworkspace_store::Caller;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// A connection without a heartbeat for this long is gone (Q18: online = connection alive and a heartbeat within 30 s).
pub const HEARTBEAT_TIMEOUT_MS: i64 = 30_000;
/// The high-rate feeds are refreshed at most this often (Q19).
const FEED_THROTTLE_MS: u64 = 1_000;

/// Who may read a broadcast event.
#[derive(Debug, Clone)]
enum Vis {
    Public,
    Presenter,
    Participants(BTreeSet<String>),
}

#[derive(Debug)]
struct Outgoing {
    vis: Vis,
    event: Value,
}

/// One running session in memory: its live connections and the fan-out of its timeline.
pub struct Room {
    pub session_id: String,
    pub run_ws: String,
    tx: broadcast::Sender<Arc<Outgoing>>,
    /// The last event already broadcast.
    pumped: tokio::sync::Mutex<u64>,
    /// Open connections per principal (presence).
    conns: Mutex<HashMap<String, usize>>,
    /// Feeds waiting for their throttled refresh.
    dirty: Mutex<BTreeSet<&'static str>>,
    /// Audience connections following the presenter (the stage feed's `followers`).
    followers: Mutex<HashMap<u64, bool>>,
    next_conn: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
pub struct Rooms {
    map: Mutex<HashMap<String, Arc<Room>>>,
}

impl Rooms {
    fn get(&self, id: &str) -> Option<Arc<Room>> {
        self.map.lock().unwrap().get(id).cloned()
    }
}

fn s<'a>(p: &'a Value, key: &str) -> WsResult<&'a str> {
    p.get(key).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{key} required")))
}

/// The room of `s`, created on first use: its run Workspace is opened (so its head channel exists) and a task wakes
/// the fan-out whenever a Commit lands there.
async fn room(state: &Arc<AppState>, s: &SessionRow) -> WsResult<Arc<Room>> {
    if let Some(r) = state.rooms.get(&s.session_id) {
        return Ok(r);
    }
    let (st, row) = (state.clone(), s.clone());
    let last = blocking(move || st.svc.with_run(&row, |ws| Ok(ws.last_event_seq()))).await?;
    let room = {
        let mut map = state.rooms.map.lock().unwrap();
        if let Some(r) = map.get(&s.session_id) {
            return Ok(r.clone());
        }
        let (tx, _) = broadcast::channel(1024);
        let room = Arc::new(Room {
            session_id: s.session_id.clone(),
            run_ws: s.workspace_id.clone(),
            tx,
            pumped: tokio::sync::Mutex::new(last),
            conns: Mutex::new(HashMap::new()),
            dirty: Mutex::new(BTreeSet::new()),
            followers: Mutex::new(HashMap::new()),
            next_conn: std::sync::atomic::AtomicU64::new(1),
        });
        map.insert(s.session_id.clone(), room.clone());
        room
    };
    // Commits from anywhere (a live Block on stage, ink) reach the timeline through the run Workspace's head
    if let Some(mut rx) = state.heads.lock().unwrap().get(&s.workspace_id).map(|tx| tx.subscribe()) {
        let (weak, st) = (Arc::downgrade(&room), Arc::downgrade(state));
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let (Some(room), Some(state)) = (weak.upgrade(), st.upgrade()) else { break };
                pump(&state, &room).await;
            }
        });
    }
    Ok(room)
}

/// Broadcast every event the room has not sent yet, each with who may read it.
async fn pump(state: &Arc<AppState>, room: &Arc<Room>) {
    // a deleted session's room is out of the map; its timeline went with the run Workspace
    if !state.rooms.get(&room.session_id).is_some_and(|r| Arc::ptr_eq(&r, room)) {
        return;
    }
    let mut last = room.pumped.lock().await;
    let (st, ws_id, after) = (state.clone(), room.run_ws.clone(), *last);
    let read = blocking(move || {
        st.with_ws(&ws_id, |ws| {
            let (events, _) = ws.events_after(&Access::full("system"), after, 500, None)?;
            let now = ws.now();
            let mut out = Vec::new();
            for ev in events {
                let vis = match ev["scope"].as_str().unwrap_or("") {
                    "public" if ev["type"] == json!("commit") => commit_vis(ws, &ev, &now)?,
                    "public" => Vis::Public,
                    "presenter" | "diag" => Vis::Presenter,
                    s => Vis::Participants(s.strip_prefix(PARTICIPANT_PREFIX).map(|p| [p.to_string()].into_iter().collect()).unwrap_or_default()),
                };
                out.push((vis, ev));
            }
            Ok(out)
        })
    })
    .await;
    match read {
        Ok(events) => {
            for (vis, event) in events {
                *last = (*last).max(event["seq"].as_u64().unwrap_or(0));
                let _ = room.tx.send(Arc::new(Outgoing { vis, event }));
            }
        }
        Err(e) => log::warn!("session {}: reading the timeline failed: {e}", room.session_id),
    }
}

/// Who learns of a Commit: everyone when it touched shared content, else the participants whose own data it touched.
fn commit_vis(ws: &aiworkspace_store::Workspace, ev: &Value, _now: &str) -> WsResult<Vis> {
    let mut participants = BTreeSet::new();
    for id in ev["data"]["touched"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let scope: Option<String> = ws.entity_scope(id)?;
        match scope.as_deref() {
            Some(SCOPE_SHARED) | None => return Ok(Vis::Public),
            Some(s) => {
                if let Some(p) = s.strip_prefix(PARTICIPANT_PREFIX) {
                    participants.insert(p.to_string());
                }
            }
        }
    }
    Ok(if participants.is_empty() { Vis::Presenter } else { Vis::Participants(participants) })
}

/// What a connection of `actor` receives of `out`: reviewers everything, others what is theirs; a Commit's details
/// stay with reviewers (the others read its content through their own `doc.get_changes`).
fn deliver(actor: &Actor, out: &Outgoing) -> Option<Value> {
    if actor.role.reviews() {
        return Some(out.event.clone());
    }
    let ok = match &out.vis {
        Vis::Public => true,
        Vis::Presenter => false,
        Vis::Participants(p) => p.contains(&actor.principal),
    };
    if !ok {
        return None;
    }
    if out.event["type"] == json!("commit") {
        let e = &out.event;
        return Some(json!({ "seq": e["seq"], "event_id": e["event_id"], "session_id": e["session_id"], "t_rel_ms": e["t_rel_ms"], "type": "commit",
                            "scope": "public", "track": e["track"], "commit_seq": e["commit_seq"], "data": { "seq": e["data"]["seq"] } }));
    }
    Some(out.event.clone())
}

// ---------------------------------------------------------------------------
// kRPC
// ---------------------------------------------------------------------------

/// The link a request carries (`params.link`), if it names a live one.
pub fn link_of(state: &AppState, p: &Value) -> Option<InviteRow> {
    let t = p.get("link").and_then(Value::as_str).filter(|t| aiworkspace_store::session::is_link_token(t))?;
    state.svc.sessions.invite_by_token(t).ok().flatten()
}

/// A display or prompter link used as the session token (`sys[1]`): its own subject reads the run Workspace.
pub fn link_caller(state: &AppState, token: &str) -> Option<(Caller, InviteRow)> {
    if !aiworkspace_store::session::is_link_token(token) || token.starts_with("at_") {
        return None;
    }
    let invite = state.svc.sessions.invite_by_token(token).ok().flatten()?;
    Some((Caller::link(&invite.invite_id), invite))
}

/// Dispatch one `session.*` kRPC call.
pub async fn handle(state: &Arc<AppState>, method: &str, p: &Value, caller: Caller, link: Option<InviteRow>) -> WsResult<Value> {
    match method {
        "session.create" => {
            let (st, c, p) = (state.clone(), caller.clone(), p.clone());
            return blocking(move || st.svc.session_create(&c, &p)).await;
        }
        "session.list" => {
            let (st, c, w) = (state.clone(), caller.clone(), s(p, "workspace_id")?.to_string());
            return blocking(move || st.svc.session_list(&c, &w)).await;
        }
        "session.list_mine" => {
            let (st, c) = (state.clone(), caller.clone());
            let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 500) as usize;
            let mut v = blocking(move || st.svc.session_list_mine(&c, limit)).await?;
            // the room is on its courseware's own origin; the path's name is only for people
            for item in v["sessions"].as_array_mut().into_iter().flatten() {
                let (ws, sid, title) = (item["source_workspace_id"].as_str().unwrap_or(""), item["session_id"].as_str().unwrap_or(""), item["title"].as_str().unwrap_or(""));
                let slug = aiworkspace_store::source::pub_name(title);
                if let Some(page) = crate::pages::page_url(state, ws, if slug.is_empty() { "session" } else { &slug }, None) {
                    item["url"] = json!(format!("{page}?session={}", url::form_urlencoded::byte_serialize(sid.as_bytes()).collect::<String>()));
                }
            }
            return Ok(v);
        }
        "session.feed_sample" => {
            let (st, c, w, f) = (state.clone(), caller.clone(), s(p, "workspace_id")?.to_string(), s(p, "feed")?.to_string());
            let public = p.get("public").and_then(Value::as_bool).unwrap_or(false);
            return blocking(move || st.with_ws(&w, |ws| aiworkspace_store::session::feed_sample(ws, &c, &f, public))).await;
        }
        "asset.stream_url" => return crate::stream::stream_url(state, &caller, p).await,
        _ => {}
    }
    let sid = s(p, "session_id")?.to_string();
    let s = state.svc.sessions.require(&sid)?;
    if method == "session.join" {
        let (st, c, row, inv, name) = (state.clone(), caller.clone(), s.clone(), link.clone(), p.get("name").and_then(Value::as_str).map(str::to_string));
        let (actor, my, first) = blocking(move || st.svc.session_join(&row, &c, inv.as_ref(), name.as_deref())).await?;
        let room = room(state, &s).await?;
        if first {
            mark(&room, "participants");
        }
        let mut v = get(state, &s, &actor).await?;
        v["my_feeds"] = my;
        v["ok"] = json!(true);
        pump(state, &room).await;
        return Ok(v);
    }
    let actor = {
        let (st, c, row, inv) = (state.clone(), caller.clone(), s.clone(), link.clone());
        blocking(move || st.svc.require_actor(&row, &c, inv.as_ref())).await?
    };
    call(state, s, &actor, method, p, None).await
}

/// `session.get` with the media state of the room.
async fn get(state: &Arc<AppState>, s: &SessionRow, actor: &Actor) -> WsResult<Value> {
    let (st, row, a) = (state.clone(), s.clone(), actor.clone());
    let mut v = blocking(move || st.svc.session_get(&row, &a)).await?;
    v["voice_service"] = state.media.status();
    if let Some(room) = state.rooms.get(&s.session_id) {
        v["online"] = json!(room.conns.lock().unwrap().values().filter(|n| **n > 0).count());
    }
    Ok(v)
}

/// The methods of a known actor — over kRPC and over the channel (`terminal`: the connection's own control terminal).
async fn call(state: &Arc<AppState>, s: SessionRow, actor: &Actor, method: &str, p: &Value, terminal: Option<&str>) -> WsResult<Value> {
    let st = state.clone();
    let a = actor.clone();
    let p2 = p.clone();
    let mut row = s.clone();
    let result = match method {
        "session.get" => return get(state, &s, actor).await,
        "session.start" => blocking(move || st.svc.session_start(&mut row, &a)).await?,
        "session.end" => {
            let retention = state.svc.session_retention_ms;
            blocking(move || st.svc.session_end(&mut row, &a, retention)).await?
        }
        "session.save" => blocking(move || st.svc.session_save(&mut row, &a)).await?,
        "session.delete" => {
            let v = blocking(move || st.svc.session_delete(&mut row, &a)).await?;
            state.rooms.map.lock().unwrap().remove(&s.session_id);
            return Ok(v);
        }
        "session.set_access" => blocking(move || st.svc.session_set_access(&mut row, &a, &p2)).await?,
        "session.invite" => blocking(move || st.svc.session_invite(&row, &a, &p2)).await?,
        "session.list_invites" => blocking(move || st.svc.session_invites(&row, &a)).await?,
        "session.revoke_invite" => {
            let id = s_owned(p, "invite_id")?;
            blocking(move || st.svc.session_revoke_invite(&row, &a, &id)).await?
        }
        "session.command" => {
            let cmd = p.get("command").cloned().ok_or_else(|| WsError::invalid_op("command required"))?;
            check_controller(state, &s, actor, terminal.or_else(|| p.get("terminal_id").and_then(Value::as_str)))?;
            let mut cmd = cmd;
            if let Some(rect) = p.get("rect") {
                cmd["rect"] = rect.clone();
            }
            blocking(move || st.svc.session_command(&row, &a, &cmd)).await?
        }
        "session.task" => blocking(move || st.svc.session_task(&row, &a, &p2)).await?,
        "session.claim" => blocking(move || st.svc.session_claim(&row, &a, &p2)).await?,
        "session.respond" => blocking(move || st.svc.session_respond(&row, &a, &p2)).await?,
        "session.review" => blocking(move || st.svc.session_review(&row, &a, &p2)).await?,
        "session.publish" => blocking(move || st.svc.session_publish(&row, &a, &p2)).await?,
        "session.react" => {
            let v = blocking(move || st.svc.session_react(&row, &a, &p2)).await?;
            if let Some(room) = state.rooms.get(&s.session_id) {
                mark(&room, "reactions");
            }
            v
        }
        "session.sample" => {
            let kind = s_owned(p, "kind")?;
            let data = p.get("data").cloned().unwrap_or(Value::Null);
            let t = p.get("t_rel_ms").and_then(Value::as_i64);
            blocking(move || st.svc.session_sample(&row, &a, &kind, data, t)).await?
        }
        "session.takeover" => {
            if actor.role != Role::Presenter {
                return Err(WsError::denied("only the presenter takes the control over"));
            }
            let t = terminal.map(str::to_string).or_else(|| p.get("terminal_id").and_then(Value::as_str).map(str::to_string)).ok_or_else(|| WsError::invalid_op("terminal_id required"))?;
            row.controller = Some(t.clone());
            let (st2, row2) = (state.clone(), row.clone());
            blocking(move || {
                st2.svc.sessions.update(&row2)?;
                st2.svc.with_run(&row2, |ws| {
                    ws.append_event(aiworkspace_store::session::NewEvent { actor: &row2.presenter, scope: "public", track: "diag", kind: "control",
                                                                          command_id: None, data: json!({ "controller": t }), t_rel_ms: None })?;
                    Ok(())
                })
            })
            .await?;
            json!({ "ok": true, "controller": row.controller })
        }
        "session.events" => {
            let after = p.get("after").and_then(Value::as_u64).unwrap_or(0);
            let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(500).clamp(1, 2000) as usize;
            let until = p.get("until_t_rel_ms").and_then(Value::as_i64);
            let access = actor.access();
            blocking(move || st.svc.with_run(&row, |ws| {
                let (events, more) = ws.events_after(&access, after, limit, until)?;
                Ok(json!({ "ok": true, "events": events, "more": more, "head": ws.last_event_seq() }))
            }))
            .await?
        }
        "session.watch" => return watch(state, &s, actor, p).await,
        "session.replay" => {
            let access = actor.access();
            blocking(move || st.svc.with_run(&row, |ws| {
                let rows = ws.replay_baseline(&access)?;
                Ok(json!({ "ok": true, "workspace_id": ws.workspace_id, "epoch": ws.epoch, "baseline_seq": row.baseline["seq"], "head_seq": ws.head_seq,
                           "baseline": rows, "duration_ms": ws.t_rel_now(), "head_event": ws.last_event_seq(), "archive": row.archive }))
            }))
            .await?
        }
        "session.notes" => {
            if !actor.role.reviews() {
                return Err(WsError::denied("notes are the presenter's"));
            }
            blocking(move || st.svc.with_run(&row, |ws| {
                let mut notes = serde_json::Map::new();
                if let Some(id) = row.baseline["notes_entity_id"].as_str() {
                    let q = ws.query(&Caller::user(&row.owner), &json!({ "source_id": id, "limit": 1000 }), &st.svc.sources)?;
                    for r in q["rows"].as_array().into_iter().flatten() {
                        notes.insert(r["values"]["target_id"].as_str().unwrap_or("").to_string(), r["values"]["notes"].clone());
                    }
                }
                Ok(json!({ "ok": true, "notes": notes, "steps": ws.session_steps()? }))
            }))
            .await?
        }
        "session.post" => {
            let kind = p.get("kind").and_then(Value::as_str).unwrap_or("comment").to_string();
            if !matches!(kind.as_str(), "comment" | "rating" | "moment") {
                return Err(WsError::invalid_op("kind must be comment, rating or moment"));
            }
            if s.posts != "open" {
                return Err(WsError::sub(Code::PermissionDenied, "POSTS_CLOSED", "comments are closed for this session"));
            }
            if actor.role == Role::Display {
                return Err(WsError::denied("a display does not post"));
            }
            let body: Option<String> = p.get("body").and_then(Value::as_str).map(|b| b.chars().take(4000).collect());
            let score = p.get("score").and_then(Value::as_i64).filter(|n| (1..=5).contains(n));
            if kind == "rating" && score.is_none() {
                return Err(WsError::invalid_op("a rating needs a score 1…5"));
            }
            if kind != "rating" && body.as_deref().is_none_or(|b| b.trim().is_empty()) {
                return Err(WsError::invalid_op("a comment needs a body"));
            }
            let t = p.get("t_rel_ms").and_then(Value::as_i64).filter(|t| *t >= 0);
            let now = (state.svc.clock)();
            let id = aiworkspace_store::workspace::random_id("po_");
            let name = actor.name.clone();
            let principal = actor.principal.clone();
            blocking(move || st.svc.sessions.add_post(&id, &row.session_id, &kind, &principal, Some(&name), body.as_deref(), score, t, now).map(|_| json!({ "ok": true, "post_id": id }))).await?
        }
        "session.list_posts" => {
            let mine = actor.principal.clone();
            let reviewer = actor.role.reviews();
            blocking(move || {
                let posts: Vec<Value> = st.svc.sessions.posts(&row.session_id)?.into_iter()
                    // ratings are the presenter's to read, besides one's own
                    .filter(|p| reviewer || p["kind"] != json!("rating") || p["author"] == json!(mine))
                    .collect();
                Ok(json!({ "ok": true, "posts": posts, "open": row.posts == "open" }))
            })
            .await?
        }
        "session.delete_post" => {
            let id = s_owned(p, "post_id")?;
            let mine = actor.principal.clone();
            let owner = actor.is_owner(&s);
            blocking(move || {
                let author = st.svc.sessions.post_author(&row.session_id, &id)?.ok_or_else(|| WsError::not_found("post not found"))?;
                if author != mine && !owner {
                    return Err(WsError::denied("only its author or the owner deletes a post"));
                }
                Ok(json!({ "ok": true, "deleted": st.svc.sessions.delete_post(&row.session_id, &id, (st.svc.clock)())? }))
            })
            .await?
        }
        "session.rtc" => return state.media.signal(state, &s, actor, p).await,
        _ => return Err(WsError::invalid_op(crate::UNKNOWN_METHOD)),
    };
    if let Some(room) = state.rooms.get(&s.session_id) {
        pump(state, &room).await;
    } else if matches!(method, "session.start" | "session.command" | "session.task" | "session.respond" | "session.claim") {
        let fresh = state.svc.sessions.require(&s.session_id)?;
        let r = room(state, &fresh).await?;
        pump(state, &r).await;
    }
    Ok(result)
}

fn s_owned(p: &Value, key: &str) -> WsResult<String> {
    s(p, key).map(str::to_string)
}

/// One active control terminal (Q05): the presenter's other terminals observe until they take over; the prompter's
/// link is the presenter's remote and always commands.
fn check_controller(state: &Arc<AppState>, s: &SessionRow, actor: &Actor, terminal: Option<&str>) -> WsResult<()> {
    if actor.role != Role::Presenter {
        return Ok(());
    }
    let current = state.svc.sessions.require(&s.session_id)?.controller;
    match (current.as_deref(), terminal) {
        (None, _) | (_, None) => Ok(()),
        (Some(c), Some(t)) if c == t => Ok(()),
        _ => Err(WsError::sub(Code::PermissionDenied, "NOT_CONTROLLER", "another terminal controls the stage: take over to control it here")),
    }
}

fn mark(room: &Room, feed: &'static str) {
    room.dirty.lock().unwrap().insert(feed);
}

/// `session.watch { after, timeout_ms? }`: the long-poll fallback of the channel (Q04).
async fn watch(state: &Arc<AppState>, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
    let after = p.get("after").and_then(Value::as_u64).unwrap_or(0);
    let timeout = p.get("timeout_ms").and_then(Value::as_u64).unwrap_or(state.limits.max_wait_ms).min(state.limits.max_wait_ms);
    let room = room(state, s).await?;
    let mut rx = room.tx.subscribe();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
    loop {
        let (st, row, access) = (state.clone(), s.clone(), actor.access());
        let (events, head, stage) = blocking(move || st.svc.with_run(&row, |ws| {
            let (events, _) = ws.events_after(&access, after, 500, None)?;
            Ok((events, ws.last_event_seq(), ws.stage()))
        }))
        .await?;
        let events: Vec<Value> = events.into_iter().filter(|e| event_visible(&actor.access(), e["scope"].as_str().unwrap_or(""))).collect();
        if !events.is_empty() || tokio::time::Instant::now() >= deadline {
            return Ok(json!({ "ok": true, "events": events, "head": head, "stage": stage, "timed_out": events.is_empty() }));
        }
        if tokio::time::timeout_at(deadline, rx.recv()).await.is_err() {
            return Ok(json!({ "ok": true, "events": [], "head": head, "stage": stage, "timed_out": true }));
        }
    }
}

// ---------------------------------------------------------------------------
// the channel
// ---------------------------------------------------------------------------

/// `GET /kapi/aiworkspace/session/ws`: the session channel (Q04). The first message is `hello { session_id, token?,
/// link?, terminal_id, purpose, after?, name? }`; the answer `welcome`, then the timeline after `after` and live.
pub async fn ws_route(State(state): State<Arc<AppState>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| async move {
        if let Err(e) = connection(state, socket).await {
            log::debug!("session channel closed: {e}");
        }
    })
}

async fn send(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, v: Value) -> Result<(), String> {
    sink.send(Message::Text(v.to_string().into())).await.map_err(|e| e.to_string())
}

async fn authenticate(state: &Arc<AppState>, token: Option<&str>, link: Option<&InviteRow>) -> Result<Caller, String> {
    match token.filter(|t| !t.is_empty()) {
        Some(t) => {
            if let Some((c, _)) = link_caller(state, t) {
                return Ok(c);
            }
            match Caller::guest(t) {
                Some(g) => Ok(g),
                None => state.auth.authenticate(t).await.map_err(|e| match e {
                    crate::auth::AuthError::Invalid(e) => e,
                    crate::auth::AuthError::Denied(d) => format!("access denied: {}", d.reason.as_str()),
                }),
            }
        }
        None => match link {
            Some(l) if l.kind != "at" => Ok(Caller::link(&l.invite_id)),
            _ => Err("session token required".into()),
        },
    }
}

async fn connection(state: Arc<AppState>, socket: WebSocket) -> Result<(), String> {
    let (mut sink, mut stream) = socket.split();
    let hello = match tokio::time::timeout(std::time::Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())?,
        _ => return Err("no hello".into()),
    };
    let fail = |code: &str, detail: String| json!({ "type": "error", "code": code, "detail": detail });
    if hello["type"] != json!("hello") {
        let _ = send(&mut sink, fail("INVALID_OPERATION", "the first message is hello".into())).await;
        return Err("bad hello".into());
    }
    let link = link_of(&state, &hello);
    let token = hello.get("token").and_then(Value::as_str).map(str::to_string);
    let caller = match authenticate(&state, token.as_deref(), link.as_ref()).await {
        Ok(c) => c,
        Err(e) => {
            let _ = send(&mut sink, fail("UNAUTHENTICATED", e.clone())).await;
            return Err(e);
        }
    };
    let link = link.or_else(|| token.as_deref().and_then(|t| link_caller(&state, t)).map(|(_, i)| i));
    let sid = hello["session_id"].as_str().unwrap_or("").to_string();
    let terminal = hello["terminal_id"].as_str().filter(|t| !t.is_empty() && t.len() <= 64).unwrap_or("").to_string();
    let purpose = hello["purpose"].as_str().unwrap_or("audience").to_string();
    let joined = async {
        let s = state.svc.sessions.require(&sid)?;
        let mut params = json!({ "session_id": sid, "name": hello.get("name").cloned().unwrap_or(Value::Null) });
        if let Some(l) = &link {
            params["link"] = json!(l.token);
        }
        let v = handle(&state, "session.join", &params, caller.clone(), link.clone()).await?;
        let (st, row, c, l) = (state.clone(), s.clone(), caller.clone(), link.clone());
        let actor = blocking(move || st.svc.require_actor(&row, &c, l.as_ref())).await?;
        Ok::<_, WsError>((s, actor, v))
    }
    .await;
    let (s, actor, welcome) = match joined {
        Ok(x) => x,
        Err(e) => {
            let _ = send(&mut sink, json!({ "type": "error", "error": e.to_json() })).await;
            return Err(e.to_string());
        }
    };
    let room = room(&state, &s).await.map_err(|e| e.to_string())?;
    let mut rx = room.tx.subscribe();
    let conn_id = room.next_conn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let present = matches!(actor.role, Role::Participant | Role::Presenter | Role::Owner);
    // the first presenter terminal controls the stage until another one takes over (Q05)
    if actor.role == Role::Presenter && purpose == "presenter" && !terminal.is_empty() {
        let mut row = state.svc.sessions.require(&sid).map_err(|e| e.to_string())?;
        if row.controller.is_none() {
            row.controller = Some(terminal.clone());
            let _ = state.svc.sessions.update(&row);
        }
    }
    if present {
        presence(&state, &room, &s, &actor, true).await;
    }
    if actor.role == Role::Participant {
        room.followers.lock().unwrap().insert(conn_id, true);
    }
    let controller = state.svc.sessions.require(&sid).ok().and_then(|r| r.controller);
    send(&mut sink, json!({ "type": "welcome", "session": welcome, "role": actor.role.as_str(), "principal": actor.principal, "level": actor.level,
                            "name": actor.name, "terminal_id": terminal, "controller": controller, "server_ms": (state.svc.clock)(),
                            "t_rel_ms": s.started_ms.map(|t| ((state.svc.clock)() - t).max(0)).unwrap_or(0) })).await?;
    // the timeline after what the terminal already has (a reconnection resumes there)
    if let Some(after) = hello.get("after").and_then(Value::as_u64) {
        let mut cursor = after;
        loop {
            let (st, row, access) = (state.clone(), s.clone(), actor.access());
            let (events, more) = blocking(move || st.svc.with_run(&row, |ws| ws.events_after(&access, cursor, 500, None))).await.map_err(|e| e.to_string())?;
            for ev in &events {
                cursor = cursor.max(ev["seq"].as_u64().unwrap_or(0));
                let ev = if actor.role.reviews() || ev["type"] != json!("commit") { ev.clone() } else { json!({ "seq": ev["seq"], "type": "commit", "commit_seq": ev["commit_seq"], "t_rel_ms": ev["t_rel_ms"] }) };
                send(&mut sink, json!({ "type": "event", "event": ev })).await?;
            }
            if !more {
                break;
            }
        }
    }
    let mut last_ping = (state.svc.clock)();
    let mut check = tokio::time::interval(std::time::Duration::from_secs(5));
    let result: Result<(), String> = loop {
        tokio::select! {
            msg = stream.next() => {
                let Some(Ok(msg)) = msg else { break Ok(()) };
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break Ok(()),
                    _ => continue,
                };
                let Ok(m) = serde_json::from_str::<Value>(&text) else { continue };
                match m["type"].as_str().unwrap_or("") {
                    "ping" => {
                        last_ping = (state.svc.clock)();
                        if present {
                            let _ = state.svc.sessions.set_presence(&s.session_id, &actor.principal, true, last_ping);
                        }
                        let t_rel = state.svc.sessions.require(&sid).ok().and_then(|r| r.started_ms).map(|t| (last_ping - t).max(0)).unwrap_or(0);
                        send(&mut sink, json!({ "type": "pong", "c": m["c"], "server_ms": last_ping, "t_rel_ms": t_rel })).await?;
                    }
                    "follow" => {
                        if actor.role == Role::Participant {
                            room.followers.lock().unwrap().insert(conn_id, m["on"] != json!(false));
                        }
                    }
                    "call" => {
                        let method = m["method"].as_str().unwrap_or("").to_string();
                        let mut params = m.get("params").cloned().unwrap_or(json!({}));
                        params["session_id"] = json!(sid);
                        params["_conn"] = json!(conn_id);
                        let row = match state.svc.sessions.require(&sid) { Ok(r) => r, Err(e) => break Err(e.to_string()) };
                        let out = if method.starts_with("session.") && !matches!(method.as_str(), "session.create" | "session.list" | "session.list_mine" | "session.join" | "session.feed_sample") {
                            call(&state, row, &actor, &method, &params, Some(terminal.as_str()).filter(|t| !t.is_empty())).await
                        } else {
                            Err(WsError::invalid_op(format!("{method} is not a session call")))
                        };
                        let reply = match out {
                            Ok(v) => json!({ "type": "result", "id": m["id"], "result": v }),
                            Err(e) => json!({ "type": "result", "id": m["id"], "error": e.to_json() }),
                        };
                        send(&mut sink, reply).await?;
                    }
                    _ => {}
                }
            }
            out = rx.recv() => {
                match out {
                    Ok(out) => {
                        if let Some(ev) = deliver(&actor, &out) {
                            if ev["type"] == json!("control") {
                                send(&mut sink, json!({ "type": "control", "controller": ev["data"]["controller"] })).await?;
                            }
                            send(&mut sink, json!({ "type": "event", "event": ev })).await?;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // too slow to follow: the terminal reconnects from its last event
                        let _ = send(&mut sink, json!({ "type": "resync" })).await;
                        break Ok(());
                    }
                    Err(_) => break Ok(()),
                }
            }
            _ = check.tick() => {
                if (state.svc.clock)() - last_ping > HEARTBEAT_TIMEOUT_MS {
                    break Ok(());
                }
            }
        }
    };
    room.followers.lock().unwrap().remove(&conn_id);
    if present {
        presence(&state, &room, &s, &actor, false).await;
    }
    state.media.disconnect(&s.session_id, &actor.principal, conn_id);
    result
}

/// A connection of `actor` opened or closed: the first one and the last one change the presence (an event on the
/// presenter's track and the throttled `participants` feed).
async fn presence(state: &Arc<AppState>, room: &Arc<Room>, s: &SessionRow, actor: &Actor, open: bool) {
    let changed = {
        let mut conns = room.conns.lock().unwrap();
        let n = conns.entry(actor.principal.clone()).or_insert(0);
        if open {
            *n += 1;
            *n == 1
        } else {
            *n = n.saturating_sub(1);
            *n == 0
        }
    };
    if !changed {
        return;
    }
    let now = (state.svc.clock)();
    let (st, row, who, name) = (state.clone(), s.clone(), actor.principal.clone(), actor.name.clone());
    let _ = blocking(move || {
        st.svc.sessions.set_presence(&row.session_id, &who, open, now)?;
        st.svc.with_run(&row, |ws| {
            // after the end people come to watch the replay: that is not part of the session's record
            if ws.session_closed() {
                return Ok(());
            }
            ws.append_event(aiworkspace_store::session::NewEvent { actor: &who, scope: "presenter", track: "diag", kind: if open { "presence.join" } else { "presence.leave" },
                                                                  command_id: None, data: json!({ "principal": who, "name": name }), t_rel_ms: None })?;
            Ok(())
        })
    })
    .await;
    mark(room, "participants");
    pump(state, room).await;
}

/// The timers of the session service: throttled feeds every second, expiry of unsaved sessions every minute.
pub fn spawn_timers(state: Arc<AppState>) {
    let weak = Arc::downgrade(&state);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(FEED_THROTTLE_MS));
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else { break };
            n += 1;
            let rooms: Vec<Arc<Room>> = state.rooms.map.lock().unwrap().values().cloned().collect();
            for room in rooms {
                let feeds: Vec<&'static str> = std::mem::take(&mut *room.dirty.lock().unwrap()).into_iter().collect();
                let followers = room.followers.lock().unwrap().values().filter(|f| **f).count();
                if feeds.is_empty() && n % 5 != 0 {
                    continue;
                }
                let Ok(s) = state.svc.sessions.require(&room.session_id) else { continue };
                if s.state != LIVE {
                    continue;
                }
                let st = state.clone();
                let r = blocking(move || {
                    let participants = st.svc.sessions.participants(&s.session_id)?;
                    st.svc.with_run(&s, |ws| {
                        let mut stage = ws.stage();
                        if !feeds.is_empty() {
                            ws.refresh_feeds(&Caller::user(&s.owner), &s.presenter, &format!("tick/{}", s.session_id), &FeedSel::of(&feeds), &participants)?;
                        }
                        if stage.get("step_id").is_some() && stage["followers"].as_u64() != Some(followers as u64) {
                            stage["followers"] = json!(followers);
                            ws.set_followers(&stage)?;
                        }
                        Ok(())
                    })
                })
                .await;
                if let Err(e) = r {
                    log::warn!("session {}: feed refresh failed: {e}", room.session_id);
                }
                pump(&state, &room).await;
            }
            if n % 60 == 0 {
                let st = state.clone();
                let _ = blocking(move || {
                    let now = (st.svc.clock)();
                    for mut s in st.svc.sessions.in_states(&[aiworkspace_store::session::registry::ENDED])? {
                        if s.expires_ms.is_some_and(|e| e <= now) {
                            log::info!("session {} was not saved and expires", s.session_id);
                            if let Err(e) = st.svc.session_expire(&mut s) {
                                log::warn!("expiring session {}: {e}", s.session_id);
                            }
                        }
                    }
                    Ok(())
                })
                .await;
            }
        }
    });
}

/// The demonstration time of a session now (its clock's zero is the start).
pub fn t_rel(state: &AppState, s: &SessionRow) -> i64 {
    s.started_ms.map(|t| ((state.svc.clock)() - t).max(0)).unwrap_or(0)
}

pub fn started_at(s: &SessionRow) -> Option<String> {
    s.started_ms.map(format_utc_ms)
}
