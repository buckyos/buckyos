//! Run-local short handles (许愿格 §5.2): `@T1` (table), `@D1` (rich text), `@R1` (record),
//! `@A1` (asset), `@F1` (folder), `@N1` (note / annotation), `@X1` (Block definition), `@W` (this
//! wish) / `@W1` (another wish), `@S1` (Surface), `@G1` (group), `@B1` (Block), fields `@T1.f2` and
//! records `@T1.r7`. Stable within one run only; everything persisted uses real ids, and an
//! unknown handle is an error — never guessed into a similar one.

use aiworkspace_core::id::is_valid_id;
use aiworkspace_core::model::*;
use aiworkspace_core::{WsError, WsResult};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Entity(String),
    Field(String, String),
    Record(String, String),
}

#[derive(Debug, Clone, Default)]
pub struct Handles {
    by_handle: BTreeMap<String, Target>,
    entity: BTreeMap<String, String>,
    field: BTreeMap<(String, String), String>,
    record: BTreeMap<(String, String), String>,
    counters: BTreeMap<String, usize>,
    /// field handles count per table (the n in `.f<n>`)
    field_counts: BTreeMap<String, usize>,
    record_counts: BTreeMap<String, usize>,
    wish_id: String,
}

pub fn prefix_for(type_id: &str, kind: Option<&str>) -> &'static str {
    match (type_id, kind) {
        (TYPE_TABLE, _) => "T",
        (TYPE_RICHTEXT, _) => "D",
        (TYPE_RECORD, _) => "R",
        (TYPE_ASSET, _) => "A",
        (TYPE_ANNOTATION, _) => "N",
        (TYPE_BLOCK_DEF, _) => "X",
        (TYPE_WISH, _) => "W",
        (TYPE_CELL, _) => "B",
        (TYPE_CONTAINER, Some("surface")) => "S",
        (TYPE_CONTAINER, Some("group")) => "G",
        _ => "F",
    }
}

impl Handles {
    pub fn new(wish_id: &str) -> Handles {
        let mut h = Handles { wish_id: wish_id.to_string(), ..Default::default() };
        h.by_handle.insert("@W".into(), Target::Entity(wish_id.to_string()));
        h.entity.insert(wish_id.to_string(), "@W".into());
        h
    }

    /// The handle of an entity, assigned on first sight.
    pub fn of(&mut self, e: &EntityRow) -> String {
        if let Some(h) = self.entity.get(&e.entity_id) {
            return h.clone();
        }
        let p = prefix_for(&e.type_id, e.payload.get("kind").and_then(Value::as_str));
        let n = self.counters.entry(p.to_string()).or_default();
        *n += 1;
        let h = format!("@{p}{n}");
        self.by_handle.insert(h.clone(), Target::Entity(e.entity_id.clone()));
        self.entity.insert(e.entity_id.clone(), h.clone());
        h
    }

    pub fn get(&self, entity_id: &str) -> Option<&str> {
        self.entity.get(entity_id).map(String::as_str)
    }

    /// Field handles follow the table's field order: call with the live fields in order once.
    pub fn fields(&mut self, table: &EntityRow, fields: &[FieldRow]) -> Vec<String> {
        let th = self.of(table);
        let mut out = Vec::new();
        for f in fields {
            let key = (table.entity_id.clone(), f.field_id.clone());
            if let Some(h) = self.field.get(&key) {
                out.push(h.clone());
                continue;
            }
            let n = self.field_counts.entry(table.entity_id.clone()).or_default();
            *n += 1;
            let h = format!("{th}.f{n}");
            self.by_handle.insert(h.clone(), Target::Field(table.entity_id.clone(), f.field_id.clone()));
            self.field.insert(key, h.clone());
            out.push(h);
        }
        out
    }

    pub fn field(&self, table_id: &str, field_id: &str) -> Option<&str> {
        self.field.get(&(table_id.to_string(), field_id.to_string())).map(String::as_str)
    }

    pub fn record(&mut self, table: &EntityRow, record_id: &str) -> String {
        let key = (table.entity_id.clone(), record_id.to_string());
        if let Some(h) = self.record.get(&key) {
            return h.clone();
        }
        let th = self.of(table);
        let n = self.record_counts.entry(table.entity_id.clone()).or_default();
        *n += 1;
        let h = format!("{th}.r{n}");
        self.by_handle.insert(h.clone(), Target::Record(table.entity_id.clone(), record_id.to_string()));
        self.record.insert(key, h.clone());
        h
    }

    /// A handle or a real id. Real ids are accepted as they are (validated by the caller's read).
    pub fn resolve(&self, s: &str) -> WsResult<Target> {
        let s = s.trim();
        if s.starts_with('@') {
            return self.by_handle.get(s).cloned().ok_or_else(|| WsError::not_found(format!("unknown handle {s} (handles are listed in WORKSPACE.md and tool results)")));
        }
        if let Some((e, rest)) = s.split_once('.') {
            if is_valid_id(e) && is_valid_id(rest) {
                return Ok(Target::Field(e.to_string(), rest.to_string()));
            }
        }
        if is_valid_id(s) {
            return Ok(Target::Entity(s.to_string()));
        }
        Err(WsError::invalid_op(format!("{s:?} is neither a handle nor an id")))
    }

    pub fn resolve_entity(&self, s: &str) -> WsResult<String> {
        match self.resolve(s)? {
            Target::Entity(e) => Ok(e),
            Target::Field(e, _) | Target::Record(e, _) if !s.starts_with('@') => Ok(e),
            _ => Err(WsError::invalid_op(format!("{s} names a field or record, not an object"))),
        }
    }

    /// Every handle → id mapping (written to `handles.json`).
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (h, t) in &self.by_handle {
            m.insert(
                h.clone(),
                match t {
                    Target::Entity(e) => json!({ "entity_id": e }),
                    Target::Field(e, f) => json!({ "entity_id": e, "field_id": f }),
                    Target::Record(e, r) => json!({ "entity_id": e, "record_id": r }),
                },
            );
        }
        json!({ "wish_id": self.wish_id, "handles": m })
    }

    /// Replace every handle in `text` by `f(handle)`; unknown handles are collected.
    pub fn rewrite(&self, text: &str, f: &dyn Fn(&str, &Target) -> Option<String>) -> (String, Vec<String>) {
        let mut out = String::with_capacity(text.len());
        let mut unknown = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '@' && i + 1 < chars.len() && chars[i + 1].is_ascii_uppercase() {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || (chars[j] == '.' && j + 1 < chars.len() && chars[j + 1].is_ascii_lowercase())) {
                    j += 1;
                }
                let h: String = chars[i..j].iter().collect();
                match self.by_handle.get(&h) {
                    Some(t) => match f(&h, t) {
                        Some(r) => out.push_str(&r),
                        None => out.push_str(&h),
                    },
                    None => {
                        unknown.push(h.clone());
                        out.push_str(&h);
                    }
                }
                i = j;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        (out, unknown)
    }
}
