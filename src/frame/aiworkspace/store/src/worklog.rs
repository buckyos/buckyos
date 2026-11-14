//! The work log (UI 框架 PRD §4.5, §12, K3): one timeline of a Workspace, newest first, read from what is recorded
//! anyway — the Commits (with their messages), the wish runs (start, end), the permission operation record (managers
//! only) and the presentation sessions of the registry, plus the non-public shows the server keeps in memory — never a
//! copy of its own. Each reader sees what it may read now: a Commit when it touched something readable, a run as
//! `proc.get` would show it, a session as `session.list` would.
//!
//! Entries are ordered by `(at, id)`; `at` is the fixed-width UTC text of `format_utc_ms`, so the key `"<at>|<id>"`
//! orders as text. A page is the newest `limit` entries strictly older than the cursor (`before`, the key of the last
//! entry of the previous page). Each source is filtered on its own and looks at no more than `SCAN_CAP` rows per call;
//! when one stops short, the page ends where it stopped (`next_before` is that position) and may hold fewer entries.

use crate::docdb::db_err;
use crate::service::Service;
use crate::wish::runs::{PROGRAM_MOCK, PROGRAM_XLLM};
use crate::workspace::{Caller, Workspace};
use aiworkspace_core::access::{Access, Cap};
use aiworkspace_core::model::{ReadCtx, CANVAS_CONTENT_ID, DATA_ID, ROOT_ID, SHOWS_ID, SURFACES_ID, TYPE_CONTAINER};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{WsError, WsResult};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;
/// Rows one source looks at in one call.
const SCAN_CAP: usize = 1000;
const BATCH: usize = 200;
/// Run states in which the run is still being computed: no end entry yet.
const RUNNING: &str = "'queued','snapshotting','running','validating'";
/// The time a run's state last changed (`detail.state_at`, written by `wish_run_set`); a run the service start marked
/// `interrupted` keeps the time of that marking.
const RUN_ENDED: &str = "(CASE WHEN state = 'interrupted' THEN updated_at ELSE COALESCE(json_extract(result_json, '$.state_at'), updated_at) END)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    /// What the caller did itself.
    Mine,
    /// Agents and macros: Commits of origin `agent` / `program` and every wish run.
    Automation,
}

impl Filter {
    pub fn parse(s: Option<&str>) -> WsResult<Filter> {
        match s.unwrap_or("all") {
            "all" => Ok(Filter::All),
            "mine" => Ok(Filter::Mine),
            "automation" => Ok(Filter::Automation),
            other => Err(WsError::invalid_op(format!("filter must be all, mine or automation, not {other}"))),
        }
    }
}

/// Entries strictly older than `(at, id)`.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub at: String,
    pub id: String,
}

impl Cursor {
    pub fn parse(s: &str) -> WsResult<Cursor> {
        match s.split_once('|') {
            Some((at, id)) if at.len() == 24 && at.ends_with('Z') && !id.is_empty() && id.len() <= 200 => Ok(Cursor { at: at.to_string(), id: id.to_string() }),
            _ => Err(WsError::invalid_op("before is not a work log position")),
        }
    }
    fn key(&self) -> String {
        format!("{}|{}", self.at, self.id)
    }
}

pub struct Query {
    pub before: Option<Cursor>,
    pub limit: usize,
    pub filter: Filter,
}

impl Query {
    pub fn parse(p: &Value) -> WsResult<Query> {
        let before = p.get("before").and_then(Value::as_str).filter(|s| !s.is_empty()).map(Cursor::parse).transpose()?;
        let limit = p.get("limit").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        Ok(Query { before, limit, filter: Filter::parse(p.get("filter").and_then(Value::as_str))? })
    }
}

/// A non-public show of the server's memory (third phase §11.3): it starts and ends, and is gone after a restart.
#[derive(Debug, Clone)]
pub struct ShowEvent {
    pub show_id: String,
    pub presenter: String,
    pub path_id: String,
    pub at_ms: i64,
    pub end: bool,
}

/// Who may see an entry that does not come from this Workspace's own tables.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Seen {
    /// Anyone who may read its presentation path.
    PathReaders,
    /// Its owner, presenter and participants; the Workspace's managers.
    Involved(bool),
}

struct Happening {
    id: String,
    at: String,
    kind: &'static str,
    end: bool,
    actor: String,
    path_id: String,
    title: Option<String>,
    session_id: Option<String>,
    seen: Seen,
    mine: bool,
}

struct Entry {
    key: String,
    value: Value,
}

struct Got {
    entries: Vec<Entry>,
    /// The position a source stopped at after `SCAN_CAP` rows without filling the page.
    frontier: Option<String>,
    done: bool,
}

/// Page through one source newest first: `fetch(position, n)` returns up to `n` rows `(at, id, row)` strictly older than
/// the position, `convert` keeps what the reader sees.
fn scan<R>(
    limit: usize,
    before: Option<&Cursor>,
    mut fetch: impl FnMut(Option<(&str, &str)>, usize) -> WsResult<Vec<(String, String, R)>>,
    mut convert: impl FnMut(&str, &str, R) -> WsResult<Option<Value>>,
) -> WsResult<Got> {
    let mut entries = Vec::new();
    let mut pos: Option<(String, String)> = before.map(|c| (c.at.clone(), c.id.clone()));
    let mut scanned = 0usize;
    loop {
        let rows = fetch(pos.as_ref().map(|(a, i)| (a.as_str(), i.as_str())), BATCH)?;
        let n = rows.len();
        for (at, id, row) in rows {
            scanned += 1;
            if let Some(mut v) = convert(&at, &id, row)? {
                v["id"] = json!(id);
                v["at"] = json!(at);
                entries.push(Entry { key: format!("{at}|{id}"), value: v });
            }
            pos = Some((at, id));
            if entries.len() >= limit {
                return Ok(Got { entries, frontier: None, done: false });
            }
        }
        if n < BATCH {
            return Ok(Got { entries, frontier: None, done: true });
        }
        if scanned >= SCAN_CAP {
            return Ok(Got { entries, frontier: pos.map(|(a, i)| format!("{a}|{i}")), done: false });
        }
    }
}

fn is_system(id: &str) -> bool {
    matches!(id, ROOT_ID | DATA_ID | SURFACES_ID | CANVAS_CONTENT_ID | SHOWS_ID)
}

/// Who wrote a Commit, for the log: an Agent, a macro (a run's result), the service's own machinery (a Source refresh,
/// the session service), else the person whose request it was (an undo, a purge or an import are internal Commits of
/// that person's request).
fn actor_kind(origin: &str) -> &'static str {
    match origin {
        "agent" => "agent",
        "program" => "macro",
        "source" | "session" => "system",
        _ => "person",
    }
}

/// Readability of entities, asked once per entity per call.
struct Readable<'a> {
    access: &'a Access,
    ctx: &'a dyn ReadCtx,
    seen: BTreeMap<String, bool>,
}

impl Readable<'_> {
    fn can(&mut self, id: &str) -> WsResult<bool> {
        if let Some(b) = self.seen.get(id) {
            return Ok(*b);
        }
        let ok = match self.ctx.entity(id)? {
            Some(e) => self.access.can_read(self.ctx, &e)?,
            None => false,
        };
        self.seen.insert(id.to_string(), ok);
        Ok(ok)
    }

    /// The Surface an entity is on, for “locate”: itself, the Surface above a Block, or the Surface whose content folder
    /// holds a data object — when the reader may read it.
    fn surface_of(&mut self, id: &str) -> WsResult<Option<String>> {
        let mut cur = id.to_string();
        for _ in 0..64 {
            let Some(e) = self.ctx.entity(&cur)? else { return Ok(None) };
            if e.type_id == TYPE_CONTAINER {
                let found = match (e.payload.get("kind").and_then(Value::as_str), e.payload.get("system").and_then(Value::as_str)) {
                    (Some("surface"), _) => Some(cur.clone()),
                    (_, Some("surface_content")) => e.payload.get("surface_id").and_then(Value::as_str).map(str::to_string),
                    _ => None,
                };
                if let Some(s) = found {
                    return Ok(if self.can(&s)? { Some(s) } else { None });
                }
            }
            match self.ctx.edge(&cur)? {
                Some(edge) if !is_system(&edge.parent_id) => cur = edge.parent_id,
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    /// What “locate” needs of a readable entity.
    fn target(&mut self, id: &str) -> WsResult<Value> {
        let Some(e) = self.ctx.entity(id)? else { return Ok(json!({ "entity_id": id })) };
        Ok(json!({ "entity_id": id, "type_id": e.type_id, "name": e.name, "title": e.payload.get("title").and_then(Value::as_str),
                   "kind": e.payload.get("kind").and_then(Value::as_str), "deleted": !e.alive(), "surface_id": self.surface_of(id)? }))
    }
}

impl Workspace {
    /// `ws.work_log { before?, limit?, filter? }` with the sessions and shows the service knows about.
    fn work_log_with(&self, caller: &Caller, q: &Query, happenings: Vec<Happening>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let manages = access.ws_caps.has(Cap::Manage);
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut readable = Readable { access: &access, ctx: &ctx, seen: BTreeMap::new() };
        let me = caller.principal.as_str();
        let before = q.before.as_ref();
        let mut sources: Vec<Got> = Vec::new();

        // Commits: newest first by seq (their acceptance times follow it), each when it touched something readable
        sources.push(scan(
            q.limit,
            before,
            |pos, n| {
                let (at, id) = pos.unzip();
                let mut st = self
                    .doc
                    .prepare_cached(
                        "SELECT seq, commit_id, principal, origin, run_id, undoes, message, accepted_at FROM commits \
                         WHERE (?1 IS NULL OR accepted_at < ?1 OR (accepted_at = ?1 AND printf('c:%012d', seq) < ?2)) ORDER BY seq DESC LIMIT ?3",
                    )
                    .map_err(db_err)?;
                let rows = st
                    .query_map(params![at, id, n as i64], |r| {
                        let seq: i64 = r.get(0)?;
                        let at: String = r.get(7)?;
                        Ok((at, format!("c:{seq:012}"), (seq, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?, r.get::<_, Option<String>>(6)?)))
                    })
                    .map_err(db_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(db_err)?;
                Ok(rows)
            },
            |_, _, (seq, commit_id, principal, origin, run_id, undoes, message)| {
                match q.filter {
                    Filter::Mine if principal != me => return Ok(None),
                    Filter::Automation if !matches!(origin.as_str(), "agent" | "program") => return Ok(None),
                    _ => {}
                }
                let mut st = self.doc.prepare_cached("SELECT touched_json FROM commit_ops WHERE seq = ?1 ORDER BY op_index").map_err(db_err)?;
                let lists: Vec<String> = st.query_map([seq], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
                drop(st);
                let (mut count, mut first, mut primary): (u64, Option<String>, Option<String>) = (0, None, None);
                for list in lists {
                    let touched: Vec<Value> = serde_json::from_str(&list).map_err(|e| WsError::io(e.to_string()))?;
                    for t in touched {
                        let id = t["entity_id"].as_str().unwrap_or("");
                        if id.is_empty() || !readable.can(id)? {
                            continue;
                        }
                        count += 1;
                        if first.is_none() {
                            first = Some(id.to_string());
                        }
                        if primary.is_none() && !is_system(id) {
                            primary = Some(id.to_string());
                        }
                    }
                }
                if count == 0 {
                    return Ok(None);
                }
                let target = match primary.or(first) {
                    Some(id) => readable.target(&id)?,
                    None => Value::Null,
                };
                let mut v = json!({ "kind": if origin == "import" { "import" } else { "commit" }, "actor": principal, "actor_kind": actor_kind(&origin),
                                    "origin": origin, "seq": seq, "commit_id": commit_id, "run_id": run_id, "undoes": undoes, "message": message,
                                    "target": target, "targets": count, "mine": principal == me });
                // a result of the Mock executor is no macro's (Q27)
                if origin == "program" {
                    if let Some(run) = run_id.as_deref() {
                        let program: Option<String> = self.local.query_row("SELECT program FROM runs WHERE run_id = ?1", [run], |r| r.get(0)).optional().map_err(db_err)?;
                        if program.is_some_and(|p| p == PROGRAM_MOCK || p.starts_with("mock.")) {
                            v["simulated"] = json!(true);
                        }
                    }
                }
                Ok(Some(v))
            },
        )?);

        // wish runs: as `proc.get` shows them (one's own, or any with `update` / `manage` on the wish), on a readable wish
        let mut run_entry = |phase_end: bool, principal: String, state: String, params_json: String, error: Option<String>, run_id: &str, program: &str| -> WsResult<Option<Value>> {
            if q.filter == Filter::Mine && principal != me {
                return Ok(None);
            }
            let params: Value = serde_json::from_str(&params_json).unwrap_or(Value::Null);
            let Some(wish_id) = params["wish_id"].as_str() else { return Ok(None) };
            if !readable.can(wish_id)? {
                return Ok(None);
            }
            let caps = access.caps(&ctx, wish_id)?;
            if principal != me && !caps.has(Cap::Update) && !caps.has(Cap::Manage) {
                return Ok(None);
            }
            let mut target = readable.target(wish_id)?;
            if target["surface_id"].is_null() {
                if let Some(s) = params["location"]["surface_id"].as_str() {
                    if readable.can(s)? {
                        target["surface_id"] = json!(s);
                    }
                }
            }
            let mut v = json!({ "kind": "run", "phase": if phase_end { "end" } else { "start" }, "actor": principal, "actor_kind": "macro", "run_id": run_id,
                                "stage": params["stage"].as_str().unwrap_or("run"), "target": target, "mine": principal == me });
            // the Mock executor's run runs no macro (UI 框架 PRD Q27): the log says so
            if program == PROGRAM_MOCK {
                v["simulated"] = json!(true);
            }
            if phase_end {
                v["state"] = json!(state);
                v["error_code"] = json!(error);
            }
            Ok(Some(v))
        };
        for end in [false, true] {
            let sql = if end {
                format!(
                    "SELECT run_id, principal, state, params_json, json_extract(result_json, '$.error.code'), {RUN_ENDED} AS t, program FROM runs \
                     WHERE program IN (?4, ?5) AND state NOT IN ({RUNNING}) AND (?1 IS NULL OR t < ?1 OR (t = ?1 AND ('r:' || run_id || ':1') < ?2)) \
                     ORDER BY t DESC, run_id DESC LIMIT ?3"
                )
            } else {
                "SELECT run_id, principal, state, params_json, NULL, created_at AS t, program FROM runs \
                 WHERE program IN (?4, ?5) AND (?1 IS NULL OR t < ?1 OR (t = ?1 AND ('r:' || run_id || ':0') < ?2)) ORDER BY t DESC, run_id DESC LIMIT ?3"
                    .to_string()
            };
            sources.push(scan(
                q.limit,
                before,
                |pos, n| {
                    let (at, id) = pos.unzip();
                    let mut st = self.local.prepare_cached(&sql).map_err(db_err)?;
                    let rows = st
                        .query_map(params![at, id, n as i64, PROGRAM_XLLM, PROGRAM_MOCK], |r| {
                            let run_id: String = r.get(0)?;
                            let t: String = r.get(5)?;
                            let id = format!("r:{run_id}:{}", end as u8);
                            Ok((t, id, (run_id, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, String>(6)?)))
                        })
                        .map_err(db_err)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(db_err)?;
                    Ok(rows)
                },
                |_, _, (run_id, principal, state, params_json, error, program)| run_entry(end, principal, state, params_json, error, &run_id, &program),
            )?);
        }

        // the permission operation record: managers only, never an Agent's or a macro's
        if manages && q.filter != Filter::Automation {
            sources.push(scan(
                q.limit,
                before,
                |pos, n| {
                    let (at, id) = pos.unzip();
                    let mut st = self
                        .local
                        .prepare_cached(
                            "SELECT seq, at, actor, action, target_json FROM acl_audit \
                             WHERE (?1 IS NULL OR at < ?1 OR (at = ?1 AND printf('a:%012d', seq) < ?2)) ORDER BY seq DESC LIMIT ?3",
                        )
                        .map_err(db_err)?;
                    let rows = st
                        .query_map(params![at, id, n as i64], |r| {
                            let seq: i64 = r.get(0)?;
                            Ok((r.get::<_, String>(1)?, format!("a:{seq:012}"), (seq, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))
                        })
                        .map_err(db_err)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(db_err)?;
                    Ok(rows)
                },
                |_, _, (seq, actor, action, target_json)| {
                    if q.filter == Filter::Mine && actor != me {
                        return Ok(None);
                    }
                    let detail: Value = serde_json::from_str(&target_json).unwrap_or(Value::Null);
                    let entity = ["entity_id", "wish_id"].iter().find_map(|k| detail.get(*k).and_then(Value::as_str)).map(str::to_string);
                    let target = match entity {
                        Some(id) if readable.can(&id)? => readable.target(&id)?,
                        _ => Value::Null,
                    };
                    Ok(Some(json!({ "kind": "acl", "actor": actor, "actor_kind": "person", "seq": seq, "action": action, "detail": detail, "target": target, "mine": actor == me })))
                },
            )?);
        }

        // sessions and shows (already in memory, newest first)
        if q.filter != Filter::Automation {
            let mut list: Vec<Happening> = happenings.into_iter().filter(|h| q.filter != Filter::Mine || h.mine).collect();
            list.sort_by(|a, b| (&b.at, &b.id).cmp(&(&a.at, &a.id)));
            let mut items = list.into_iter().filter(|h| before.is_none_or(|c| format!("{}|{}", h.at, h.id) < c.key()));
            sources.push(scan(
                q.limit,
                before,
                |_, n| Ok(items.by_ref().take(n).map(|h| (h.at.clone(), h.id.clone(), h)).collect()),
                |_, _, h| {
                    let path = readable.can(&h.path_id)?;
                    let visible = match h.seen {
                        Seen::PathReaders => path,
                        Seen::Involved(involved) => involved || manages,
                    };
                    if !visible {
                        return Ok(None);
                    }
                    let target = if path { readable.target(&h.path_id)? } else { Value::Null };
                    Ok(Some(json!({ "kind": h.kind, "phase": if h.end { "end" } else { "start" }, "actor": h.actor, "actor_kind": "person",
                                    "title": h.title, "session_id": h.session_id, "target": target, "mine": h.mine })))
                },
            )?);
        }

        // merge: nothing past the newest position a source stopped at without filling the page
        let bound = sources.iter().filter_map(|s| s.frontier.clone()).max();
        let more_rows = sources.iter().any(|s| !s.done);
        let mut all: Vec<Entry> = sources.into_iter().flat_map(|s| s.entries).collect();
        all.sort_by(|a, b| b.key.cmp(&a.key));
        let total = all.len();
        let page: Vec<Entry> = all.into_iter().filter(|e| bound.as_ref().is_none_or(|b| e.key > *b)).take(q.limit).collect();
        let next = if page.len() == q.limit && (total > page.len() || more_rows) {
            page.last().map(|e| e.key.clone())
        } else if page.len() < q.limit {
            bound
        } else {
            None
        };
        Ok(json!({ "ok": true, "entries": page.into_iter().map(|e| e.value).collect::<Vec<_>>(), "next_before": next, "more": next.is_some() }))
    }
}

impl Service {
    /// `ws.work_log` (K3): the Workspace's timeline as `caller` may read it; `shows` are the non-public shows of the
    /// server's memory.
    pub fn work_log(&self, caller: &Caller, workspace_id: &str, q: &Query, shows: Vec<ShowEvent>) -> WsResult<Value> {
        let me = caller.principal.as_str();
        let mut happenings = Vec::new();
        for s in self.sessions.list_for_source(workspace_id)? {
            let involved = s.owner == me || s.presenter == me || self.sessions.participant(&s.session_id, me)?.is_some();
            for (ms, end) in [(s.started_ms, false), (s.ended_ms, true)] {
                let Some(ms) = ms else { continue };
                happenings.push(Happening {
                    id: format!("s:{}:{}", s.session_id, end as u8),
                    at: format_utc_ms(ms),
                    kind: "session",
                    end,
                    actor: s.presenter.clone(),
                    path_id: s.path_id.clone(),
                    title: Some(s.title.clone()),
                    session_id: Some(s.session_id.clone()),
                    seen: Seen::Involved(involved),
                    mine: s.owner == me || s.presenter == me,
                });
            }
        }
        for e in shows {
            happenings.push(Happening {
                id: format!("h:{}:{}", e.show_id, e.end as u8),
                at: format_utc_ms(e.at_ms),
                kind: "show",
                end: e.end,
                mine: e.presenter == me,
                actor: e.presenter,
                path_id: e.path_id,
                title: None,
                session_id: None,
                seen: Seen::PathReaders,
            });
        }
        let handle = self.workspace(workspace_id)?;
        let ws = handle.lock().map_err(|_| WsError::new(aiworkspace_core::Code::WriterBusy, "workspace writer was reset; retry"))?;
        ws.work_log_with(caller, q, happenings)
    }
}
