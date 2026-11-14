//! `presence.*`: who has a Workspace open right now (UI 框架 PRD §4.2, §4.5, §12, K2), in memory like the shows.
//!
//! Each window registers with its own connection id and renews it with a heartbeat every `interval_ms` (10 s); a
//! connection not renewed within `ttl_ms` (30 s) is gone — the rule of the session rooms. A person with several windows
//! or devices is one member, shown on the Surface its most recently changed window reported. Nothing of it reaches the
//! document or its history, and a restart forgets it (the windows register again with their next heartbeat). What a
//! reader is told of the others is judged by the store (`aiworkspace_store::presence`): Surfaces it may read, and
//! groups only when it manages the Workspace (Q6).

use crate::AppState;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{WsError, WsResult};
use aiworkspace_store::presence::PresenceInput;
use aiworkspace_store::Caller;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A connection without a heartbeat for this long is gone; windows renew at a third of it (every 10 s).
pub const TTL_MS: i64 = 30_000;

/// The time to live, shortened by `AIWS_PRESENCE_TTL_MS` for tests of a window that went away.
fn ttl_ms() -> i64 {
    static TTL: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *TTL.get_or_init(|| std::env::var("AIWS_PRESENCE_TTL_MS").ok().and_then(|v| v.parse().ok()).filter(|v| *v >= 300).unwrap_or(TTL_MS))
}
/// Connections one Workspace keeps (a misbehaving client cannot grow it without bound).
const MAX_CONNECTIONS: usize = 2_000;
const MAX_NAME_CHARS: usize = 32;

struct Conn {
    /// As verified at its latest heartbeat (memberships settled then).
    caller: Caller,
    /// A guest's chosen name (a guest has no other).
    name: Option<String>,
    surface_id: Option<String>,
    /// When the window last reported a different Surface (or registered).
    moved_ms: i64,
    since_ms: i64,
    seen_ms: i64,
}

/// `workspace_id → (principal, connection id) → connection`.
#[derive(Default)]
pub struct Presence {
    map: Mutex<HashMap<String, HashMap<(String, String), Conn>>>,
    /// When every Workspace was last swept of expired connections.
    swept_ms: Mutex<i64>,
}

/// One person, all its live connections together.
struct Person {
    caller: Caller,
    name: Option<String>,
    surface_id: Option<String>,
    moved_ms: i64,
    since_ms: i64,
    seen_ms: i64,
    devices: usize,
}

fn s<'a>(p: &'a Value, key: &str) -> WsResult<&'a str> {
    p.get(key).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{key} required")))
}

fn conn_id(p: &Value) -> WsResult<String> {
    let id = s(p, "conn_id")?;
    if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')) {
        return Err(WsError::invalid_op("conn_id must be 1–128 characters of [A-Za-z0-9_.:-]"));
    }
    Ok(id.to_string())
}

impl Presence {
    /// The live people of a Workspace, oldest arrival first; expired connections go.
    fn people(&self, workspace_id: &str, now: i64) -> Vec<Person> {
        let mut map = self.map.lock().unwrap();
        let Some(conns) = map.get_mut(workspace_id) else { return Vec::new() };
        conns.retain(|_, c| now - c.seen_ms <= ttl_ms());
        let mut by: HashMap<&str, Person> = HashMap::new();
        for c in conns.values() {
            let p = by.entry(c.caller.principal.as_str()).or_insert_with(|| Person {
                caller: c.caller.clone(),
                name: None,
                surface_id: None,
                moved_ms: i64::MIN,
                since_ms: c.since_ms,
                seen_ms: c.seen_ms,
                devices: 0,
            });
            p.devices += 1;
            p.since_ms = p.since_ms.min(c.since_ms);
            if c.seen_ms >= p.seen_ms {
                p.seen_ms = c.seen_ms;
                p.caller = c.caller.clone();
                if c.name.is_some() {
                    p.name = c.name.clone();
                }
            }
            if c.surface_id.is_some() && c.moved_ms >= p.moved_ms {
                p.moved_ms = c.moved_ms;
                p.surface_id = c.surface_id.clone();
            }
        }
        let mut out: Vec<Person> = by.into_values().collect();
        out.sort_by(|a, b| (a.since_ms, &a.caller.principal).cmp(&(b.since_ms, &b.caller.principal)));
        if conns.is_empty() {
            map.remove(workspace_id);
        }
        out
    }

    /// Forget every connection that expired, in every Workspace (a heartbeat sweeps at most once a minute).
    fn sweep(&self, now: i64) {
        {
            let mut swept = self.swept_ms.lock().unwrap();
            if now - *swept < 60_000 {
                return;
            }
            *swept = now;
        }
        let mut map = self.map.lock().unwrap();
        map.retain(|_, conns| {
            conns.retain(|_, c| now - c.seen_ms <= ttl_ms());
            !conns.is_empty()
        });
    }
}

/// The member list as `viewer` sees it: itself first, then the others by arrival.
fn members(state: &Arc<AppState>, ws_id: &str, viewer: &Caller) -> WsResult<Value> {
    let now = (state.svc.clock)();
    let mut people = state.presence.people(ws_id, now);
    // the viewer first (it may be missing: a list asked for before registering)
    if let Some(i) = people.iter().position(|p| p.caller.principal == viewer.principal) {
        let me = people.remove(i);
        people.insert(0, me);
    }
    let inputs: Vec<PresenceInput> = people.iter().map(|p| PresenceInput { caller: &p.caller, surface_id: p.surface_id.as_deref() }).collect();
    let views = state.with_ws(ws_id, |ws| ws.presence_view(viewer, &inputs))?;
    let list: Vec<Value> = people
        .iter()
        .zip(views)
        .map(|(p, mut v)| {
            v["name"] = json!(p.name);
            v["devices"] = json!(p.devices);
            v["since"] = json!(format_utc_ms(p.since_ms));
            v["last_seen"] = json!(format_utc_ms(p.seen_ms));
            v
        })
        .collect();
    Ok(json!({ "ok": true, "members": list, "interval_ms": ttl_ms() / 3, "ttl_ms": ttl_ms(), "server_ms": now }))
}

/// Dispatch one `presence.*` method (the generic checks of the method table ran before).
pub fn handle(state: &Arc<AppState>, method: &str, p: &Value, caller: &Caller) -> WsResult<Value> {
    let ws_id = s(p, "workspace_id")?;
    match method {
        // `{ conn_id, surface_id?, name? }`: register or renew this window; answers the member list
        "presence.heartbeat" => {
            let id = conn_id(p)?;
            let surface = match p.get("surface_id").and_then(Value::as_str) {
                Some(sid) => state.with_ws(ws_id, |ws| ws.readable_surface(caller, sid))?,
                None => None,
            };
            let name = if caller.is_guest() {
                p.get("name").and_then(Value::as_str).map(|n| n.trim().chars().take(MAX_NAME_CHARS).collect::<String>()).filter(|n| !n.is_empty())
            } else {
                None
            };
            let now = (state.svc.clock)();
            {
                let mut map = state.presence.map.lock().unwrap();
                let conns = map.entry(ws_id.to_string()).or_default();
                let key = (caller.principal.clone(), id);
                if !conns.contains_key(&key) && conns.len() >= MAX_CONNECTIONS {
                    conns.retain(|_, c| now - c.seen_ms <= ttl_ms());
                    if conns.len() >= MAX_CONNECTIONS {
                        return Err(WsError::limit("too many open windows on this workspace"));
                    }
                }
                let c = conns.entry(key).or_insert_with(|| Conn { caller: caller.clone(), name: None, surface_id: None, moved_ms: now, since_ms: now, seen_ms: now });
                if c.surface_id != surface {
                    c.moved_ms = now;
                }
                c.caller = caller.clone();
                c.name = name;
                c.surface_id = surface;
                c.seen_ms = now;
            }
            state.presence.sweep(now);
            members(state, ws_id, caller)
        }
        // `{ conn_id }`: this window closed
        "presence.leave" => {
            let id = conn_id(p)?;
            let mut map = state.presence.map.lock().unwrap();
            let removed = map.get_mut(ws_id).and_then(|conns| conns.remove(&(caller.principal.clone(), id))).is_some();
            if map.get(ws_id).is_some_and(|conns| conns.is_empty()) {
                map.remove(ws_id);
            }
            Ok(json!({ "ok": true, "removed": removed }))
        }
        "presence.list" => members(state, ws_id, caller),
        _ => Err(WsError::invalid_op(crate::UNKNOWN_METHOD)),
    }
}
