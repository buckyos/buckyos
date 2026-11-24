//! What people are told of (README “Notices”): the comments others made, and the reminders of the standard to-do and
//! calendar tables. The store finds them; the service decides who reads them and sends (`server/src/notices.rs`).
//!
//! How far each was told is local state of the deployment (`notify.*` of `local_meta`); a Workspace starts from where
//! it is when the service first looks at it.

use crate::docdb::{db_err, meta_get};
use crate::workspace::{Caller, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::model::{EntityRow, ReadCtx, SCOPE_SHARED, TYPE_ANNOTATION};
use aiworkspace_core::plan::surface_of;
use aiworkspace_core::read::annotation_target_readable;
use aiworkspace_core::standard::{CALENDAR_SCHEMA, TODO_SCHEMA};
use aiworkspace_core::value::parse_datetime_ms;
use aiworkspace_core::WsResult;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

const COMMENTS_SEQ: &str = "notify.comments_seq";
const COMMENTS_AT: &str = "notify.comments_at";
const REMINDED_UNTIL: &str = "notify.reminded_until";
/// Commits whose comments are somebody's words: not imports, undo, sources, builds, runs or sessions.
const SPOKEN: [&str; 2] = ["human", "agent"];

/// Where a page shows an entity: its Surface, the Block on it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Place {
    /// `(title, id)`.
    pub surface: Option<(String, String)>,
    pub block: Option<String>,
    pub record: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub seq: u64,
    pub annotation_id: String,
    pub author: String,
    /// The name a guest gave.
    pub author_name: Option<String>,
    pub body: String,
    pub title: String,
    pub at_ms: i64,
    /// Principals the comment concerns: who made what it is on, and who commented on that before.
    pub concerns: Vec<String>,
    pub place: Place,
}

/// A record of a standard table with reminders.
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderRecord {
    pub table_id: String,
    pub schema_id: String,
    pub record_id: String,
    pub values: Value,
    /// Who wrote the record, when it was a Commit of somebody's own.
    pub created_by: Option<String>,
    /// Overridden occurrences of this series (`ext.recurrence_id.original`).
    pub overridden: Vec<String>,
    pub place: Place,
}

fn title_of(e: &EntityRow) -> String {
    e.payload.get("title").and_then(Value::as_str).or(e.name.as_deref()).unwrap_or(&e.entity_id).to_string()
}

impl Workspace {
    fn local_note(&self, key: &str) -> WsResult<Option<i64>> {
        let v: Option<String> = self.local.query_row("SELECT value FROM local_meta WHERE key = ?1", [key], |r| r.get(0)).optional().map_err(db_err)?;
        Ok(v.and_then(|v| v.parse().ok()))
    }

    fn set_local_note(&self, key: &str, value: i64) -> WsResult<()> {
        self.local.execute("INSERT OR REPLACE INTO local_meta (key, value) VALUES (?1, ?2)", params![key, value.to_string()]).map_err(db_err)?;
        Ok(())
    }

    /// The Surface (and Block on it) an entity is shown at: itself, or the first Block bound to it.
    fn place_of(&self, ctx: &dyn ReadCtx, id: &str) -> WsResult<Place> {
        let mut candidates = vec![id.to_string()];
        candidates.extend(ctx.refs_to(id)?.into_iter().filter(|r| r.kind == "bind").map(|r| r.src_entity_id));
        for block in candidates {
            if !ctx.entity(&block)?.is_some_and(|e| e.alive()) {
                continue;
            }
            if let Some(surface) = surface_of(ctx, &block)?.filter(|s| s.alive()) {
                let block = (block != surface.entity_id).then_some(block);
                return Ok(Place { surface: Some((title_of(&surface), surface.entity_id)), block, record: None });
            }
        }
        Ok(Place::default())
    }

    fn writer_of(&self, seq: u64) -> WsResult<Option<String>> {
        let found: Option<(String, String)> = self
            .doc
            .query_row("SELECT principal, origin FROM commits WHERE seq = ?1", [seq], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(db_err)?;
        Ok(found.filter(|(_, origin)| SPOKEN.contains(&origin.as_str())).map(|(principal, _)| principal))
    }

    /// The comments nobody was told of yet, oldest first, and the seq they reach to (`comments_told` once they are).
    pub fn untold_comments(&self) -> WsResult<(Vec<Comment>, u64)> {
        let head = self.head_seq;
        let told = match self.local_note(COMMENTS_SEQ)? {
            // a Workspace seen for the first time, or restored to an earlier head: nothing before now is news
            Some(told) if told as u64 <= head => told as u64,
            _ => {
                self.set_local_note(COMMENTS_SEQ, head as i64)?;
                return Ok((Vec::new(), head));
            }
        };
        if told == head {
            return Ok((Vec::new(), head));
        }
        let mut st = self
            .doc
            .prepare_cached(
                "SELECT o.seq, o.op_json, c.principal, c.origin, c.accepted_at FROM commit_ops o JOIN commits c ON c.seq = o.seq \
                 WHERE o.seq > ?1 AND o.op_json LIKE '%buckyos.annotation%' ORDER BY o.seq, o.op_index",
            )
            .map_err(db_err)?;
        let rows: Vec<(u64, String, String, String, String)> = st
            .query_map([told], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .and_then(|rows| rows.collect())
            .map_err(db_err)?;
        drop(st);
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for (seq, op, author, origin, at) in rows {
            let Ok(op) = serde_json::from_str::<Value>(&op) else { continue };
            if op["op"] != json!("entity.create") || op["type_id"] != json!(TYPE_ANNOTATION) || !SPOKEN.contains(&origin.as_str()) {
                continue;
            }
            let Some(e) = op["entity_id"].as_str().map(|id| ctx.entity(id)).transpose()?.flatten() else { continue };
            let body = e.payload.get("body").and_then(Value::as_str).unwrap_or("").trim().to_string();
            // a note without a target is canvas content, not a comment
            let Some(target_id) = e.payload.get("target").and_then(|t| t["entity_id"].as_str()) else { continue };
            if !e.alive() || e.scope != SCOPE_SHARED || body.is_empty() {
                continue;
            }
            let Some(target) = ctx.entity(target_id)?.filter(|t| t.alive()) else { continue };
            let record_id = e.payload.get("target").and_then(|t| t["selector"]["record_id"].as_str());
            let maker = match record_id {
                Some(r) => ctx.record(target_id, r)?.map(|r| self.writer_of(r.created_seq)).transpose()?.flatten().or_else(|| target.created_by.clone()),
                None => target.created_by.clone(),
            };
            let mut concerns: Vec<String> = Vec::new();
            match maker {
                Some(m) => concerns.push(m),
                None => concerns.extend(self.owner()?),
            }
            for r in ctx.refs_to(target_id)?.into_iter().filter(|r| r.kind == "anchor" && r.src_entity_id != e.entity_id) {
                let earlier = ctx.entity(&r.src_entity_id)?.filter(|a| a.alive() && a.created_seq < e.created_seq);
                let Some(earlier) = earlier else { continue };
                // on a record: those who spoke of that record
                if earlier.payload.get("target").and_then(|t| t["selector"]["record_id"].as_str()) != record_id {
                    continue;
                }
                if let Some(a) = aiworkspace_core::types::annotation_author(&earlier) {
                    concerns.push(a.to_string());
                }
            }
            concerns.retain(|p| *p != author);
            concerns.sort();
            concerns.dedup();
            let place = Place { record: record_id.map(str::to_string), ..self.place_of(&ctx, target_id)? };
            out.push(Comment {
                seq,
                annotation_id: e.entity_id.clone(),
                author_name: e.payload.get("author_name").and_then(Value::as_str).map(str::to_string),
                author,
                body,
                title: title_of(&target),
                at_ms: parse_datetime_ms(&at).unwrap_or(0),
                concerns,
                place,
            });
        }
        if out.is_empty() {
            self.set_local_note(COMMENTS_SEQ, head as i64)?;
        }
        Ok((out, head))
    }

    /// When the last notices about comments went out (ms).
    pub fn comments_told_at(&self) -> WsResult<i64> {
        Ok(self.local_note(COMMENTS_AT)?.unwrap_or(0))
    }

    /// Comments up to `upto` need no telling any more; `sent_at_ms`: when notices of them went out.
    pub fn comments_told(&self, upto: u64, sent_at_ms: Option<i64>) -> WsResult<()> {
        self.set_local_note(COMMENTS_SEQ, upto as i64)?;
        match sent_at_ms {
            Some(at) => self.set_local_note(COMMENTS_AT, at),
            None => Ok(()),
        }
    }

    /// Which of these comments `caller` may read.
    pub fn readable_comments(&self, caller: &Caller, ids: &[String]) -> WsResult<Vec<String>> {
        let access = self.access(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for id in ids {
            let Some(e) = ctx.entity(id)?.filter(|e| e.alive()) else { continue };
            if access.can_read(&ctx, &e)? && annotation_target_readable(&ctx, &access, &e.payload)? {
                out.push(id.clone());
            }
        }
        Ok(out)
    }

    pub fn may_read_entity(&self, caller: &Caller, entity_id: &str) -> WsResult<bool> {
        let access = self.access(caller)?;
        let now = self.now();
        access.can(&self.ctx(&now), entity_id, Cap::Read)
    }

    /// Reminders went out up to here (ms); the first call starts at `now_ms`.
    pub fn reminded_until(&self, now_ms: i64) -> WsResult<i64> {
        match self.local_note(REMINDED_UNTIL)? {
            Some(at) => Ok(at),
            None => {
                self.set_local_note(REMINDED_UNTIL, now_ms)?;
                Ok(now_ms)
            }
        }
    }

    pub fn set_reminded_until(&self, at_ms: i64) -> WsResult<()> {
        self.set_local_note(REMINDED_UNTIL, at_ms)
    }

    /// The records of the standard tables that carry `ext.reminders`. Left out: those an Agent keeps the reminders of
    /// (`ext.maintenance` / `ext.covered_until`, D4 §5.4), done to-dos, cancelled events, and what a clone or an import
    /// brought and nobody has touched since (a copy does not remind by itself, D4 §5.3).
    pub fn reminder_records(&self) -> WsResult<Vec<ReminderRecord>> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let copied = meta_get(&self.doc, "cloned_from")?.is_some_and(|v| v != "null");
        let mut st = self.doc.prepare_cached("SELECT entity_id, schema_id FROM entities WHERE schema_id IN (?1, ?2) AND deleted_seq IS NULL").map_err(db_err)?;
        let tables: Vec<(String, String)> =
            st.query_map([TODO_SCHEMA, CALENDAR_SCHEMA], |r| Ok((r.get(0)?, r.get(1)?))).and_then(|rows| rows.collect()).map_err(db_err)?;
        drop(st);
        let mut out = Vec::new();
        for (table_id, schema_id) in tables {
            if !ctx.entity(&table_id)?.is_some_and(|e| e.alive()) {
                continue;
            }
            let mut st = self
                .doc
                .prepare_cached(
                    "SELECT record_id, values_json, created_seq, rev FROM table_records WHERE source_id = ?1 AND deleted_seq IS NULL \
                     AND (values_json LIKE '%\"reminders\"%' OR values_json LIKE '%\"recurrence_id\"%')",
                )
                .map_err(db_err)?;
            let rows: Vec<(String, String, u64, u64)> =
                st.query_map([&table_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).and_then(|rows| rows.collect()).map_err(db_err)?;
            drop(st);
            let rows: Vec<(String, Value, u64, u64)> =
                rows.into_iter().filter_map(|(id, v, created, rev)| serde_json::from_str(&v).ok().map(|v| (id, v, created, rev))).collect();
            if rows.is_empty() {
                continue;
            }
            let place = self.place_of(&ctx, &table_id)?;
            for (record_id, values, created_seq, rev) in &rows {
                let ext = &values["ext"];
                if !ext["reminders"].as_array().is_some_and(|r| !r.is_empty()) || ext.get("maintenance").is_some() || ext.get("covered_until").is_some() {
                    continue;
                }
                if matches!(values["status"].as_str(), Some("done" | "cancelled")) || (copied && self.origin_of(*rev)?.as_deref() == Some("import")) {
                    continue;
                }
                let overridden = rows
                    .iter()
                    .filter(|(_, v, ..)| v["ext"]["recurrence_id"]["master"] == json!(record_id))
                    .filter_map(|(_, v, ..)| v["ext"]["recurrence_id"]["original"].as_str().map(str::to_string))
                    .collect();
                out.push(ReminderRecord {
                    table_id: table_id.clone(),
                    schema_id: schema_id.clone(),
                    record_id: record_id.clone(),
                    values: values.clone(),
                    created_by: self.writer_of(*created_seq)?,
                    overridden,
                    place: Place { record: Some(record_id.clone()), ..place.clone() },
                });
            }
        }
        Ok(out)
    }

    fn origin_of(&self, seq: u64) -> WsResult<Option<String>> {
        self.doc.query_row("SELECT origin FROM commits WHERE seq = ?1", [seq], |r| r.get(0)).optional().map_err(db_err)
    }
}
