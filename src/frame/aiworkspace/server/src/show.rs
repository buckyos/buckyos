//! `show.*`: the relay of a non-public show (third phase §8, §11.3, §12), in memory.
//!
//! The stage (the window that started the show) is the authority: it runs the show controller, publishes the
//! latest state and executes commands. Prompters (any browser holding the prompter link) send commands and watch
//! the state. A prompter authenticates with the show's token instead of a session — the token can only command,
//! watch and read the notes of this one show, and dies with it. A service restart ends every show.

use crate::AppState;
use aiworkspace_core::access::Cap;
use aiworkspace_core::id::is_prefixed_id;
use aiworkspace_core::model::{TYPE_CELL, TYPE_SHOW_PATH, TYPE_VIEWPORT};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::show::ShowLock;
use aiworkspace_store::workspace::random_id;
use aiworkspace_store::Caller;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// The show lock and the clone live this long without a heartbeat (the stage renews every 20 s).
pub const LEASE_MS: i64 = 60_000;
const MAX_STATE_BYTES: usize = 64 * 1024;
const MAX_COMMANDS: usize = 200;

/// The lease, shortened by `AIWS_SHOW_LEASE_MS` for tests of a crashed stage.
fn lease_ms() -> i64 {
    static LEASE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *LEASE.get_or_init(|| std::env::var("AIWS_SHOW_LEASE_MS").ok().and_then(|v| v.parse().ok()).filter(|v| *v >= 500).unwrap_or(LEASE_MS))
}

/// Who calls: a session of the zone, or the holder of a prompter link.
pub enum Auth {
    Session(Caller),
    Token(String),
}

struct Show {
    workspace_id: String,
    principal: String,
    path_id: String,
    clone_id: Option<String>,
    locked: bool,
    token: String,
    started_at: String,
    expires_ms: i64,
    /// The latest state the stage published, and its `seq` (the stage numbers them).
    state: Value,
    seq: u64,
    /// Commands not yet seen by the stage, by cursor; ids already accepted (de-duplication).
    commands: VecDeque<(u64, Value)>,
    cursor: u64,
    command_ids: VecDeque<String>,
}

#[derive(Default)]
pub struct Shows {
    map: Mutex<HashMap<String, Show>>,
    /// Bumped on every change of any show: watchers re-check their own.
    tick: Mutex<Option<watch::Sender<u64>>>,
}

impl Shows {
    fn notify(&self) {
        let mut tick = self.tick.lock().unwrap();
        let tx = tick.get_or_insert_with(|| watch::channel(0).0);
        tx.send_modify(|n| *n += 1);
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        let mut tick = self.tick.lock().unwrap();
        tick.get_or_insert_with(|| watch::channel(0).0).subscribe()
    }
}

fn s<'a>(p: &'a Value, key: &str) -> WsResult<&'a str> {
    p.get(key).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{key} required")))
}

fn ended() -> WsError {
    WsError::sub(Code::NotFound, "SHOW_ENDED", "the show has ended")
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> WsResult<T> + Send + 'static) -> WsResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| WsError::io(e.to_string()))?
}

/// The show `show_id` of `workspace_id` as `auth` may use it: the presenter's sessions for everything, the token only
/// for commands, watching and notes.
fn check(show: &Show, workspace_id: &str, auth: &Auth, method: &str) -> WsResult<()> {
    if show.workspace_id != workspace_id {
        return Err(ended());
    }
    match auth {
        Auth::Session(c) if c.principal == show.principal => Ok(()),
        Auth::Token(t) if *t == show.token && matches!(method, "show.command" | "show.watch" | "show.notes") => Ok(()),
        Auth::Token(_) => Err(ended()),
        Auth::Session(_) => Err(WsError::denied("only the presenter controls the show")),
    }
}

/// Dispatch one `show.*` method.
pub async fn handle(state: &Arc<AppState>, method: &str, p: &Value, auth: Auth) -> WsResult<Value> {
    match method {
        "show.start" => match auth {
            Auth::Session(caller) => start(state, p, caller).await,
            Auth::Token(_) => Err(WsError::denied("a prompter cannot start a show")),
        },
        "show.watch" => watch_show(state, p, auth).await,
        "show.notes" => notes(state, p, auth).await,
        "show.end" => end(state, p, auth).await,
        "show.heartbeat" | "show.publish" | "show.command" => {
            let ws_id = s(p, "workspace_id")?;
            let show_id = s(p, "show_id")?;
            let now = (state.svc.clock)();
            let out = {
                let mut map = state.shows.map.lock().unwrap();
                let show = map.get_mut(show_id).ok_or_else(ended)?;
                check(show, ws_id, &auth, method)?;
                match method {
                    "show.heartbeat" => {
                        show.expires_ms = now + lease_ms();
                        if show.locked {
                            state.svc.show_renew(ws_id, show_id, show.expires_ms);
                        }
                        json!({ "ok": true, "expires_at": format_utc_ms(show.expires_ms) })
                    }
                    "show.publish" => {
                        let seq = p.get("seq").and_then(Value::as_u64).ok_or_else(|| WsError::invalid_op("seq required"))?;
                        let st = p.get("state").filter(|v| v.is_object()).ok_or_else(|| WsError::invalid_op("state must be an object"))?;
                        if st.to_string().len() > MAX_STATE_BYTES {
                            return Err(WsError::limit("a show state is limited to 64 KiB"));
                        }
                        // a late request of an older state never overwrites a newer one
                        if seq > show.seq {
                            show.seq = seq;
                            show.state = st.clone();
                        }
                        json!({ "ok": true, "seq": show.seq })
                    }
                    _ => {
                        let cmd = p.get("command").and_then(Value::as_object).ok_or_else(|| WsError::invalid_op("command must be an object"))?;
                        let id = cmd.get("command_id").and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 128).ok_or_else(|| WsError::invalid_op("command.command_id required"))?;
                        if !cmd.get("kind").and_then(Value::as_str).is_some_and(|k| !k.is_empty() && k.len() <= 64) {
                            return Err(WsError::invalid_op("command.kind required"));
                        }
                        if Value::Object(cmd.clone()).to_string().len() > 8 * 1024 {
                            return Err(WsError::limit("a command is limited to 8 KiB"));
                        }
                        if show.command_ids.iter().any(|x| x == id) {
                            json!({ "ok": true, "duplicate": true, "cursor": show.cursor })
                        } else {
                            show.cursor += 1;
                            show.commands.push_back((show.cursor, Value::Object(cmd.clone())));
                            show.command_ids.push_back(id.to_string());
                            while show.commands.len() > MAX_COMMANDS {
                                show.commands.pop_front();
                            }
                            while show.command_ids.len() > MAX_COMMANDS * 4 {
                                show.command_ids.pop_front();
                            }
                            json!({ "ok": true, "duplicate": false, "cursor": show.cursor })
                        }
                    }
                }
            };
            if method != "show.heartbeat" {
                state.shows.notify();
            }
            Ok(out)
        }
        _ => Err(WsError::invalid_op(crate::UNKNOWN_METHOD)),
    }
}

/// `show.start { workspace_id, path_id, live }`: with workspace-level write access the Workspace is locked, and a show
/// with operable Blocks (`live`) gets a temporary clone; without it the show reads the Workspace as it is.
async fn start(state: &Arc<AppState>, p: &Value, caller: Caller) -> WsResult<Value> {
    let ws_id = s(p, "workspace_id")?.to_string();
    let path_id = s(p, "path_id")?.to_string();
    let live = p.get("live").and_then(Value::as_bool).unwrap_or(false);
    let (st, c, w, path) = (state.clone(), caller.clone(), ws_id.clone(), path_id.clone());
    let writer = blocking(move || {
        st.with_ws(&w, |ws| {
            let access = ws.require_ws(&c, Cap::Read)?;
            if ws.purpose().is_some() {
                return Err(WsError::invalid_op("a show clone cannot be presented"));
            }
            let read = ws.read(&c, &path, None)?;
            if read["type_id"] != json!(TYPE_SHOW_PATH) {
                return Err(WsError::invalid_op(format!("{path} is not a presentation path")));
            }
            Ok(access.ws_caps.any_write())
        })
    })
    .await?;
    let show_id = random_id("sh_");
    let token = random_id("pt_");
    let now = (state.svc.clock)();
    let expires_ms = now + lease_ms();
    let started_at = format_utc_ms(now);
    if writer {
        state.svc.show_lock(&ws_id, ShowLock { show_id: show_id.clone(), principal: caller.principal.clone(), started_at: started_at.clone(), expires_ms })?;
    }
    let clone_id = if writer && live {
        let (st, w, sid, exp) = (state.clone(), ws_id.clone(), show_id.clone(), format_utc_ms(expires_ms));
        match blocking(move || st.svc.clone_for_show(&w, &sid, &exp)).await {
            Ok(id) => Some(id),
            Err(e) => {
                state.svc.show_unlock(&ws_id, &show_id);
                return Err(e);
            }
        }
    } else {
        None
    };
    log::info!("show {show_id} started on {ws_id} by {} (locked: {writer}, clone: {clone_id:?})", caller.principal);
    let out = json!({ "ok": true, "show_id": show_id, "prompter_token": token, "clone_workspace_id": clone_id, "locked": writer,
                      "live": clone_id.is_some(), "started_at": started_at, "expires_at": format_utc_ms(expires_ms), "lease_ms": lease_ms() });
    state.shows.map.lock().unwrap().insert(
        show_id,
        Show {
            workspace_id: ws_id,
            principal: caller.principal,
            path_id,
            clone_id,
            locked: writer,
            token,
            started_at,
            expires_ms,
            state: json!({}),
            seq: 0,
            commands: VecDeque::new(),
            cursor: 0,
            command_ids: VecDeque::new(),
        },
    );
    state.shows.notify();
    Ok(out)
}

/// `show.watch { workspace_id, show_id, after_seq?, after_command?, timeout_ms? }`: long poll until the state is newer
/// than `after_seq`, a command arrived after `after_command`, or the time is up; without either it answers at once.
async fn watch_show(state: &Arc<AppState>, p: &Value, auth: Auth) -> WsResult<Value> {
    let ws_id = s(p, "workspace_id")?;
    let show_id = s(p, "show_id")?;
    let after_seq = p.get("after_seq").and_then(Value::as_u64);
    let after_command = p.get("after_command").and_then(Value::as_u64);
    let timeout = p.get("timeout_ms").and_then(Value::as_u64).unwrap_or(state.limits.max_wait_ms).min(state.limits.max_wait_ms);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
    let mut rx = state.shows.subscribe();
    let mut timed_out = false;
    loop {
        rx.borrow_and_update();
        {
            let map = state.shows.map.lock().unwrap();
            let show = map.get(show_id).ok_or_else(ended)?;
            check(show, ws_id, &auth, "show.watch")?;
            let newer = after_seq.is_some_and(|a| show.seq > a);
            let commands: Vec<Value> = match after_command {
                Some(c) => show.commands.iter().filter(|(n, _)| *n > c).map(|(n, cmd)| json!({ "cursor": n, "command": cmd })).collect(),
                None => Vec::new(),
            };
            if timed_out || newer || !commands.is_empty() || (after_seq.is_none() && after_command.is_none()) {
                return Ok(json!({ "ok": true, "state": show.state, "seq": show.seq, "commands": commands, "cursor": show.cursor,
                                  "path_id": show.path_id, "started_at": show.started_at, "expires_at": format_utc_ms(show.expires_ms),
                                  "timed_out": timed_out }));
            }
        }
        if tokio::time::timeout_at(deadline, rx.changed()).await.is_err() {
            timed_out = true;
        }
    }
}

/// `show.notes`: title, notes and caption of every step of the path, read as the presenter (§11.2).
async fn notes(state: &Arc<AppState>, p: &Value, auth: Auth) -> WsResult<Value> {
    let ws_id = s(p, "workspace_id")?.to_string();
    let show_id = s(p, "show_id")?;
    let (presenter, path_id) = {
        let map = state.shows.map.lock().unwrap();
        let show = map.get(show_id).ok_or_else(ended)?;
        check(show, &ws_id, &auth, "show.notes")?;
        (Caller::user(&show.principal), show.path_id.clone())
    };
    let st = state.clone();
    blocking(move || {
        st.with_ws(&ws_id, |ws| {
            let path = ws.read(&presenter, &path_id, None)?;
            let payload = &path["content"]["payload"];
            let mut steps = Vec::new();
            for step in payload["steps"].as_array().into_iter().flatten() {
                let target_id = step["target"]["entity_id"].as_str().unwrap_or("");
                let target = ws.read(&presenter, target_id, None).ok();
                let tp = target.as_ref().map(|t| &t["content"]["payload"]);
                let (notes, caption) = match target.as_ref().map(|t| t["type_id"].clone()) {
                    Some(t) if t == json!(TYPE_CELL) => (tp.map(|p| p["presentation"]["notes"].clone()), tp.map(|p| p["presentation"]["caption"].clone())),
                    Some(t) if t == json!(TYPE_VIEWPORT) => (tp.map(|p| p["notes"].clone()), tp.map(|p| p["caption"].clone())),
                    _ => (None, None),
                };
                let title = step["title"].as_str().map(str::to_string).or_else(|| tp.and_then(|p| p["title"].as_str().map(str::to_string)));
                steps.push(json!({ "id": step["id"], "kind": step["target"]["kind"], "target_id": target_id, "title": title,
                                   "enabled": step.get("enabled").cloned().unwrap_or(json!(true)), "missing": target.is_none(),
                                   "notes": notes.unwrap_or(Value::Null), "caption": caption.unwrap_or(Value::Null) }));
            }
            Ok(json!({ "ok": true, "title": payload["title"], "purpose": payload["purpose"], "stage": payload["stage"], "steps": steps }))
        })
    })
    .await
}

/// `show.end`: the presenter ends the show, or a manager of the Workspace forces it to end (like `lock.break`).
async fn end(state: &Arc<AppState>, p: &Value, auth: Auth) -> WsResult<Value> {
    let ws_id = s(p, "workspace_id")?.to_string();
    let show_id = s(p, "show_id")?.to_string();
    let Auth::Session(caller) = auth else { return Err(WsError::denied("a prompter cannot end the show")) };
    let presenter = {
        let map = state.shows.map.lock().unwrap();
        let show = map.get(&show_id).ok_or_else(ended)?;
        if show.workspace_id != ws_id {
            return Err(ended());
        }
        show.principal.clone()
    };
    if presenter != caller.principal {
        let (st, c, w) = (state.clone(), caller.clone(), ws_id.clone());
        blocking(move || st.with_ws(&w, |ws| ws.require_ws(&c, Cap::Manage).map(|_| ()))).await?;
        log::info!("show {show_id} on {ws_id} ended by manager {}", caller.principal);
    }
    end_show(state, &show_id).await;
    Ok(json!({ "ok": true }))
}

/// Release the lock, delete the clone, forget the show (its token with it) and wake its watchers.
pub async fn end_show(state: &Arc<AppState>, show_id: &str) {
    let Some(show) = state.shows.map.lock().unwrap().remove(show_id) else { return };
    if show.locked {
        state.svc.show_unlock(&show.workspace_id, show_id);
    }
    if let Some(clone) = show.clone_id {
        let st = state.clone();
        if let Err(e) = blocking(move || st.svc.drop_clone(&clone)).await {
            log::warn!("could not delete the clone of show {show_id}: {e}");
        }
    }
    state.shows.notify();
    log::info!("show {show_id} on {} ended", show.workspace_id);
}

/// End every show whose stage stopped renewing it (crash, network loss): lock released, clone deleted.
pub async fn sweep(state: &Arc<AppState>) {
    let now = (state.svc.clock)();
    let expired: Vec<String> = state.shows.map.lock().unwrap().iter().filter(|(_, s)| s.expires_ms <= now).map(|(id, _)| id.clone()).collect();
    for id in expired {
        end_show(state, &id).await;
    }
}

pub fn spawn_sweeper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis((lease_ms() / 12).clamp(100, 5_000) as u64));
        loop {
            tick.tick().await;
            sweep(&state).await;
        }
    });
}

/// A prompter link's token as the kRPC envelope may carry it (params `prompter_token`).
pub fn prompter_token(p: &Value) -> Option<String> {
    p.get("prompter_token").and_then(Value::as_str).filter(|t| is_prefixed_id("pt_", t)).map(str::to_string)
}
