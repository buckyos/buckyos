//! `body`: what an entity is in a plain-text environment (核心 API 调整 R6, §2.3; design draft §4). Agents read it,
//! text queries match it, a screen reader will speak it: one projection per type, rebuilt from the native content
//! (which stays the truth). A body is a sequence of *parts* — a table record, a rich text block, a directory member,
//! a Block of a canvas — joined by newlines; reads page by parts and never cut one unless it alone exceeds the page.
//!
//! Typed values are written as they are shown (`display`): a select's label, a reference's title and path, a currency
//! with its code. Renaming an option or a referenced entity therefore changes the body without changing the record:
//! the `etag` covers the field definitions and the referenced entities' versions.
//!
//! A Block's body is its own text and what it shows by path; it does not inline the content of the data it shows (a
//! canvas reads as its structure, and a text query finds that content once, in the data entity). A note Block shows
//! its note's text.

use crate::access::Access;
use crate::canonical::{canonical_json, sha256_hex};
use crate::error::{Code, WsError, WsResult};
use crate::filter::live_fields;
use crate::model::*;
use crate::value::{FieldDef, FieldType};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Version of the text projections: raised whenever a projection changes.
pub const BODY_FORMAT_VERSION: u32 = 1;
pub const DEFAULT_MAX_BYTES: usize = 32 * 1024;
pub const MAX_MAX_BYTES: usize = 256 * 1024;
/// Text assets up to this size have their content as body.
pub const ASSET_TEXT_MAX: u64 = 256 * 1024;

/// Reads the bytes of an uploaded object (the store's), for text assets.
pub type AssetLoader<'a> = &'a dyn Fn(&str) -> WsResult<Option<Vec<u8>>>;

/// What reading the body of an entity gives, known without reading it.
#[derive(Debug, Clone, PartialEq)]
pub struct Desc {
    /// `full` · `definition` (a URL table: definition only) · `fallback` (native serialization) · `unavailable`
    pub state: &'static str,
    pub format: &'static str,
    pub reason: Option<&'static str>,
}

fn desc(state: &'static str, format: &'static str, reason: Option<&'static str>) -> Desc {
    Desc { state, format, reason }
}

fn is_text_media(media_type: &str) -> bool {
    media_type.starts_with("text/") || matches!(media_type, "application/json" | "application/xml" | "application/yaml" | "application/x-yaml" | "image/svg+xml")
}

pub fn describe(_ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<Desc> {
    if e.degraded().is_some() {
        return Ok(desc("fallback", "application/json", None));
    }
    Ok(match e.type_id.as_str() {
        TYPE_TABLE if crate::types::is_url_table(e) => desc("definition", "text/plain", None),
        TYPE_TABLE => desc("full", "text/csv", None),
        TYPE_RICHTEXT | TYPE_CELL | TYPE_WISH | TYPE_BLOCK_DEF => desc("full", "text/markdown", None),
        TYPE_CONTAINER => match e.payload.get("kind").and_then(Value::as_str).unwrap_or("") {
            "surface" | "group" => desc("full", "text/markdown", None),
            k if crate::types::is_control_kind(k) => desc("full", "text/markdown", None),
            _ => desc("full", "text/plain", None),
        },
        TYPE_RECORD | TYPE_ANNOTATION | TYPE_DIR => desc("full", "text/plain", None),
        TYPE_ASSET => {
            let mt = e.payload.get("media_type").and_then(Value::as_str).unwrap_or("");
            let size = e.payload.get("size").and_then(Value::as_u64).unwrap_or(u64::MAX);
            if !is_text_media(mt) {
                desc("unavailable", "text/plain", Some("binary"))
            } else if size > ASSET_TEXT_MAX {
                desc("unavailable", "text/plain", Some("too_large"))
            } else {
                desc("full", "text/plain", None)
            }
        }
        _ => desc("fallback", "application/json", None),
    })
}

/// One part of a body.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// The D04 selector of what this part shows inside the entity (a record, a block, a member, a property).
    pub selector: Option<Value>,
    /// The entity this part shows, when it is another one (a child, a Block of a canvas).
    pub entity_id: Option<String>,
    pub text: String,
}

impl Part {
    fn text(t: impl Into<String>) -> Part {
        Part { selector: None, entity_id: None, text: t.into() }
    }
    fn sel(selector: Value, t: impl Into<String>) -> Part {
        Part { selector: Some(selector), entity_id: None, text: t.into() }
    }
    fn of(entity_id: &str, t: impl Into<String>) -> Part {
        Part { selector: None, entity_id: Some(entity_id.to_string()), text: t.into() }
    }
}

// ---- display values (body and the SQL display columns share them, design draft §4.3)

/// RFC 4180: quote a CSV field when it holds a comma, a quote or a line break.
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn number_text(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The text of a stored value as people see it. `refs` gives a referenced entity's text (`title (path)`).
pub fn display_value(def: &FieldDef, v: Option<&Value>, refs: &mut dyn FnMut(&Value) -> String) -> String {
    let Some(v) = v.filter(|v| !v.is_null() && !crate::value::is_formula(v)) else { return String::new() };
    match def.ty {
        FieldType::Text | FieldType::Date | FieldType::Datetime | FieldType::Decimal => v.as_str().map(str::to_string).unwrap_or_else(|| number_text(v)),
        FieldType::Number => number_text(v),
        FieldType::Boolean => match v.as_bool() {
            Some(true) => "true".into(),
            Some(false) => "false".into(),
            None => v.to_string(),
        },
        FieldType::Select => v.as_str().map(|id| def.option_label(id).unwrap_or(id).to_string()).unwrap_or_else(|| v.to_string()),
        FieldType::MultiSelect => v
            .as_array()
            .map(|a| a.iter().map(|x| x.as_str().map(|id| def.option_label(id).unwrap_or(id).to_string()).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join("; "))
            .unwrap_or_else(|| v.to_string()),
        FieldType::ObjectRef => refs(v),
        FieldType::Currency => {
            let (code, amount) = crate::value::currency_parts(v);
            format!("{code} {amount}")
        }
        FieldType::Json => v.to_string(),
    }
}

// ---- table view descriptions (shared with the stage tools)

/// A table view's settings in words: `filter: 状态=已完成; sort by 金额 descending; group by 地区; 3/5 columns shown`.
pub fn view_summary(cell: &EntityRow, fields: &[FieldRow]) -> String {
    let name = |fid: &str| fields.iter().find(|f| f.field_id == fid).map(|f| f.def.name.clone()).unwrap_or_else(|| fid.to_string());
    let mut parts = Vec::new();
    if let Some(f) = cell.payload.get("filter").filter(|f| !f.is_null()) {
        parts.push(format!("filter: {}", filter_text(f, &name, fields)));
    }
    if let Some(s) = cell.payload.get("sorts").and_then(Value::as_array).filter(|s| !s.is_empty()) {
        let t: Vec<String> = s.iter().map(|x| format!("{} {}", name(x["field_id"].as_str().unwrap_or("")), if x["direction"] == json!("desc") { "descending" } else { "ascending" })).collect();
        parts.push(format!("sort by {}", t.join(", ")));
    }
    if let Some(g) = cell.payload.get("group").and_then(|g| g["field_id"].as_str()) {
        parts.push(format!("group by {}", name(g)));
    }
    if let Some(fl) = cell.payload.get("fields").and_then(Value::as_array) {
        let shown = fl.iter().filter(|f| f["hidden"] != json!(true)).count();
        if shown < fields.len() {
            parts.push(format!("{shown}/{} columns shown", fields.len()));
        }
    }
    parts.join("; ")
}

pub fn filter_text(f: &Value, name: &dyn Fn(&str) -> String, fields: &[FieldRow]) -> String {
    match f["op"].as_str().unwrap_or("") {
        "and" | "or" => {
            let sep = if f["op"] == json!("and") { " and " } else { " or " };
            f["args"].as_array().into_iter().flatten().map(|a| filter_text(a, name, fields)).collect::<Vec<_>>().join(sep)
        }
        "not" => format!("not ({})", filter_text(&f["arg"], name, fields)),
        "cmp" => {
            let fid = f["field_id"].as_str().unwrap_or("");
            let label = |v: &Value| -> String {
                let def = fields.iter().find(|x| x.field_id == fid);
                match (v.as_str(), def.and_then(|d| d.def.options.as_ref())) {
                    (Some(s), Some(o)) => o.iter().find(|x| x.option_id == s).map(|x| x.label.clone()).unwrap_or_else(|| s.to_string()),
                    (Some(s), None) => s.to_string(),
                    _ => v.to_string(),
                }
            };
            let op = match f["operator"].as_str().unwrap_or("") {
                "eq" => "=",
                "ne" => "≠",
                "lt" => "<",
                "lte" => "≤",
                "gt" => ">",
                "gte" => "≥",
                "contains" => " contains ",
                "starts_with" => " starts with ",
                "in" => " in ",
                "has_any" => " has any of ",
                "has_all" => " has all of ",
                "is_empty" => " is empty",
                "is_not_empty" => " is not empty",
                o => o,
            };
            let v = match &f["value"] {
                Value::Null => String::new(),
                Value::Array(a) => a.iter().map(label).collect::<Vec<_>>().join("/"),
                v => label(v),
            };
            format!("{}{op}{v}", name(fid))
        }
        _ => f.to_string(),
    }
}

// ---- building

/// A body: its parts and what it depends on (for the `etag`).
#[derive(Debug, Clone)]
pub struct Body {
    pub desc: Desc,
    pub parts: Vec<Part>,
    pub deps: Vec<String>,
}

/// Builds bodies for one caller; caches paths and remembers dependencies.
pub struct Builder<'a> {
    pub ctx: &'a dyn ReadCtx,
    pub access: &'a Access,
    pub assets: Option<AssetLoader<'a>>,
    deps: Vec<String>,
    paths: BTreeMap<String, String>,
}

fn title_of(e: &EntityRow) -> Option<String> {
    crate::naming::title_source(&e.type_id, &e.payload)
}

fn label_of(e: &EntityRow) -> String {
    title_of(e).or_else(|| e.name.clone()).unwrap_or_else(|| e.entity_id.clone())
}

impl<'a> Builder<'a> {
    pub fn new(ctx: &'a dyn ReadCtx, access: &'a Access, assets: Option<AssetLoader<'a>>) -> Builder<'a> {
        Builder { ctx, access, assets, deps: Vec::new(), paths: BTreeMap::new() }
    }

    fn dep(&mut self, e: &EntityRow) {
        self.deps.push(format!("{}:{}:{}", e.entity_id, e.content_rev, e.meta_rev));
    }

    /// The path of a readable entity, or `None`.
    fn readable(&mut self, id: &str) -> WsResult<Option<(EntityRow, String)>> {
        let Some(e) = self.ctx.entity(id)? else { return Ok(None) };
        if !e.alive() || !self.access.can_read(self.ctx, &e)? {
            return Ok(None);
        }
        self.dep(&e);
        let path = match self.paths.get(id) {
            Some(p) => p.clone(),
            None => {
                let p = crate::path::path_of(self.ctx, self.access, id)?;
                self.paths.insert(id.to_string(), p.clone());
                p
            }
        };
        Ok(Some((e, path)))
    }

    /// `title (path)` of a referenced entity, or `~<id>` when the caller cannot read it.
    fn reference_text(&mut self, v: &Value) -> String {
        let Some(id) = crate::value::reference_entity_id(v) else { return v.to_string() };
        if v.get("workspace_id").is_some() {
            return format!("~{id} (another workspace)");
        }
        match self.readable(id) {
            Ok(Some((e, path))) => format!("{} ({path})", label_of(&e)),
            _ => format!("~{id}"),
        }
    }

    /// The columns of a local table's CSV and its header line.
    pub fn table_header(&mut self, table: &EntityRow) -> WsResult<(Vec<FieldRow>, bool, String)> {
        let fields = live_fields(self.ctx, &table.entity_id)?;
        for f in &fields {
            self.deps.push(format!("f:{}:{}:{}", f.field_id, f.def_rev, f.type_rev));
        }
        let bodies = self.ctx.refs_from(&table.entity_id)?.iter().any(|r| r.kind == "body");
        let mut cols = vec!["_id".to_string()];
        cols.extend(fields.iter().map(|f| csv_field(&f.def.name)));
        if bodies {
            cols.push("_body".into());
        }
        Ok((fields, bodies, cols.join(",")))
    }

    /// The CSV line of one record.
    pub fn table_row(&mut self, fields: &[FieldRow], bodies: bool, r: &RecordRow) -> String {
        let mut cells = vec![csv_field(&r.record_id)];
        for f in fields {
            let text = display_value(&f.def, r.values.get(&f.field_id), &mut |v| self.reference_text(v));
            cells.push(csv_field(&text));
        }
        if bodies {
            let b = match &r.body_entity_id {
                Some(id) => match self.readable(id) {
                    Ok(Some((_, path))) => path,
                    _ => String::new(),
                },
                None => String::new(),
            };
            cells.push(csv_field(&b));
        }
        cells.join(",")
    }

    /// The dependencies of a local table's body without reading its records: its versions, its fields and the
    /// entities its records reference.
    pub fn table_deps(&mut self, table: &EntityRow) -> WsResult<()> {
        self.dep(table);
        self.table_header(table)?;
        for r in self.ctx.refs_from(&table.entity_id)? {
            if matches!(r.kind.as_str(), "value" | "body") && r.dst_workspace_id.is_empty() {
                if let Some(t) = self.ctx.entity(&r.dst_entity_id)? {
                    self.dep(&t);
                }
            }
        }
        Ok(())
    }

    /// The whole body of `e` (a local table: the header and every record).
    pub fn build(&mut self, e: &EntityRow) -> WsResult<Body> {
        self.deps.clear();
        self.dep(e);
        let d = describe(self.ctx, e)?;
        let parts = self.parts(e, &d, 0)?;
        let mut deps = std::mem::take(&mut self.deps);
        deps.sort();
        deps.dedup();
        Ok(Body { desc: d, parts, deps })
    }

    fn fallback(e: &EntityRow) -> Vec<Part> {
        vec![Part::text(canonical_json(&Value::Object(e.payload.clone())).unwrap_or_else(|_| Value::Object(e.payload.clone()).to_string()))]
    }

    fn parts(&mut self, e: &EntityRow, d: &Desc, depth: usize) -> WsResult<Vec<Part>> {
        if d.state == "fallback" {
            return Ok(Self::fallback(e));
        }
        Ok(match e.type_id.as_str() {
            TYPE_RICHTEXT => self.richtext_parts(e)?,
            TYPE_TABLE if crate::types::is_url_table(e) => self.url_table_parts(e)?,
            TYPE_TABLE => {
                let (fields, bodies, header) = self.table_header(e)?;
                let mut rows = vec![Part::text(header)];
                let mut out = Vec::new();
                self.ctx.scan_records(&e.entity_id, &mut |r| {
                    if r.alive() {
                        out.push(r.clone());
                    }
                    Ok(true)
                })?;
                for r in out {
                    let line = self.table_row(&fields, bodies, &r);
                    rows.push(Part::sel(json!({ "kind": "table_record", "record_id": r.record_id }), line));
                }
                rows
            }
            TYPE_RECORD => self.record_parts(e),
            TYPE_ANNOTATION => self.annotation_parts(e)?,
            TYPE_ASSET => self.asset_parts(e)?,
            TYPE_DIR => self.dir_parts(e),
            TYPE_WISH => self.wish_parts(e)?,
            TYPE_BLOCK_DEF => self.def_parts(e),
            TYPE_CELL => vec![Part::text(self.block_text(e)?)],
            TYPE_CONTAINER => {
                let kind = e.payload.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
                match kind.as_str() {
                    "surface" | "group" => self.canvas_parts(e, depth)?,
                    k if crate::types::is_control_kind(k) => vec![Part::text(self.control_text(e, depth)?)],
                    _ => self.folder_parts(e)?,
                }
            }
            _ => Self::fallback(e),
        })
    }

    fn richtext_parts(&mut self, e: &EntityRow) -> WsResult<Vec<Part>> {
        let Some(rt) = self.ctx.richtext(&e.entity_id)? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for block in rt.meta.ast.get("content").and_then(Value::as_array).into_iter().flatten() {
            let md = crate::markdown::from_richtext(&json!({ "type": "doc", "content": [block] }));
            let md = md.trim_end().to_string();
            match block.get("attrs").and_then(|a| a.get("block_id")).and_then(Value::as_str) {
                Some(id) => out.push(Part::sel(json!({ "kind": "richtext_block", "block_id": id }), md)),
                None => out.push(Part::text(md)),
            }
        }
        Ok(out)
    }

    fn url_table_parts(&mut self, e: &EntityRow) -> WsResult<Vec<Part>> {
        let fields = live_fields(self.ctx, &e.entity_id)?;
        let mut lines = vec![format!("Table {} (rows live at a remote source and are not stored here)", label_of(e))];
        if let Some(r) = e.payload.get("source_ref") {
            let url = r.get("url").or_else(|| r.get("source")).and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| r.to_string());
            lines.push(format!("source: {url}"));
        }
        for f in &fields {
            lines.push(format!("field {}: {}", f.def.name, f.def.ty.as_str()));
        }
        Ok(vec![Part::text(lines.join("\n"))])
    }

    fn record_parts(&mut self, e: &EntityRow) -> Vec<Part> {
        let mut out = Vec::new();
        for (k, v) in &e.payload {
            if let Some(name) = k.strip_prefix("p:") {
                let text = match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                out.push(Part::sel(json!({ "kind": "doc_key", "key": k }), format!("{name}: {text}")));
            }
        }
        out
    }

    fn annotation_parts(&mut self, e: &EntityRow) -> WsResult<Vec<Part>> {
        let target = e.payload.get("target").and_then(|t| t.get("entity_id")).and_then(Value::as_str).map(str::to_string);
        let head = match target {
            Some(t) => match self.readable(&t)? {
                Some((_, path)) => format!("Note on {path}"),
                None => format!("Note on ~{t}"),
            },
            None => "Note".to_string(),
        };
        let mut out = vec![Part::text(head)];
        if let Some(q) = e.payload.get("context").and_then(|c| c.get("quote")).and_then(Value::as_str) {
            out.push(Part::text(format!("> {q}")));
        }
        out.push(Part::sel(json!({ "kind": "doc_key", "key": "body" }), e.payload.get("body").and_then(Value::as_str).unwrap_or("").to_string()));
        Ok(out)
    }

    fn asset_parts(&mut self, e: &EntityRow) -> WsResult<Vec<Part>> {
        let mt = e.payload.get("media_type").and_then(Value::as_str).unwrap_or("").to_string();
        let size = e.payload.get("size").and_then(Value::as_u64).unwrap_or(0);
        let name = e.payload.get("file_name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| label_of(e));
        let mut out = vec![Part::text(format!("{name} ({mt}, {size} bytes)"))];
        if describe(self.ctx, e)?.state == "full" {
            if let (Some(load), Some(obj)) = (self.assets, e.payload.get("object_id").and_then(Value::as_str)) {
                if let Some(bytes) = load(obj)? {
                    out.push(Part::text(String::from_utf8_lossy(&bytes).trim_end().to_string()));
                }
            }
        }
        Ok(out)
    }

    fn dir_parts(&mut self, e: &EntityRow) -> Vec<Part> {
        let mut out = Vec::new();
        for (name, m) in e.payload.get("members").and_then(Value::as_object).into_iter().flatten() {
            let line = match m.get("kind").and_then(Value::as_str) {
                Some("dir") => format!("{name}/"),
                _ => format!("{name} ({}, {} bytes)", m.get("media_type").and_then(Value::as_str).unwrap_or(""), m.get("size").and_then(Value::as_u64).unwrap_or(0)),
            };
            out.push(Part::sel(json!({ "kind": "folder_member", "name": name }), line));
        }
        out
    }

    fn wish_parts(&mut self, e: &EntityRow) -> WsResult<Vec<Part>> {
        let mut out = vec![Part::text(format!("# {}", label_of(e)))];
        if let Some(p) = e.payload.get("prompt").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            out.push(Part::sel(json!({ "kind": "doc_key", "key": "prompt" }), p.to_string()));
        }
        let mut inputs = Vec::new();
        for i in e.payload.get("inputs").and_then(Value::as_array).into_iter().flatten() {
            let id = i.get("ref").or_else(|| i.get("source")).and_then(|r| r.get("entity_id")).and_then(Value::as_str);
            let label = i.get("name").or_else(|| i.get("label")).and_then(Value::as_str).unwrap_or("");
            let target = match id {
                Some(id) => match self.readable(id)? {
                    Some((_, path)) => path,
                    None => format!("~{id}"),
                },
                None => String::new(),
            };
            inputs.push(format!("- input {label}: {target}").replace(":  ", ": ").trim_end().to_string());
        }
        if !inputs.is_empty() {
            out.push(Part::sel(json!({ "kind": "doc_key", "key": "inputs" }), inputs.join("\n")));
        }
        if let Some(o) = e.payload.get("output").filter(|o| !o.is_null()) {
            out.push(Part::sel(json!({ "kind": "doc_key", "key": "output" }), format!("output: {o}")));
        }
        if let Some(m) = e.payload.get("program").and_then(|p| p.get("manifest")).filter(|m| m.is_object()) {
            out.push(Part::sel(json!({ "kind": "doc_key", "key": "program" }), manifest_text(m)));
        }
        if let Some(state) = e.payload.get("last_run").and_then(|r| r.get("state")).and_then(Value::as_str) {
            out.push(Part::text(format!("last run: {state}")));
        }
        Ok(out)
    }

    fn def_parts(&mut self, e: &EntityRow) -> Vec<Part> {
        let mut out = vec![Part::text(format!("# {}", label_of(e)))];
        for key in ["description", "agent"] {
            if let Some(t) = e.payload.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                out.push(Part::sel(json!({ "kind": "doc_key", "key": key }), t.to_string()));
            }
        }
        if let Some(props) = e.payload.get("config_schema").and_then(|s| s.get("properties")).and_then(Value::as_object) {
            let lines: Vec<String> = props
                .iter()
                .map(|(k, p)| format!("- config.{k}: {}", p.get("description").or_else(|| p.get("title")).and_then(Value::as_str).unwrap_or_else(|| p.get("type").and_then(Value::as_str).unwrap_or(""))))
                .collect();
            out.push(Part::sel(json!({ "kind": "doc_key", "key": "config_schema" }), lines.join("\n")));
        }
        if let Some(b) = e.payload.get("bindings").and_then(Value::as_object) {
            let lines: Vec<String> = b.keys().map(|k| format!("- binding {k}")).collect();
            out.push(Part::sel(json!({ "kind": "doc_key", "key": "bindings" }), lines.join("\n")));
        }
        out
    }

    fn folder_parts(&mut self, e: &EntityRow) -> WsResult<Vec<Part>> {
        let mut out = Vec::new();
        for c in crate::path::visible_children(self.ctx, self.access, &e.entity_id)? {
            if !c.readable {
                continue;
            }
            self.dep(&c.entity);
            let x = &c.entity;
            let name = if c.shadowed { format!("~{}", x.entity_id) } else { x.name.clone().unwrap_or_default() };
            let dir = matches!(x.type_id.as_str(), TYPE_CONTAINER) || !self.ctx.children(&x.entity_id)?.is_empty();
            let title = title_of(x).filter(|t| Some(t) != x.name.as_ref()).map(|t| format!(" \"{t}\"")).unwrap_or_default();
            let kind = match x.type_id.as_str() {
                TYPE_CONTAINER => x.payload.get("kind").and_then(Value::as_str).unwrap_or("container").to_string(),
                t => crate::path::short_type(t).to_string(),
            };
            out.push(Part::of(&x.entity_id, format!("{name}{} {kind}{title}", if dir { "/" } else { "" })));
        }
        Ok(out)
    }

    /// The Blocks of a canvas or group in reading order: top to bottom, then left to right (sibling order without a
    /// placement).
    fn reading_order(&mut self, id: &str) -> WsResult<Vec<crate::path::Child>> {
        let kids: Vec<crate::path::Child> = crate::path::visible_children(self.ctx, self.access, id)?.into_iter().filter(|c| c.readable).collect();
        let mut keyed: Vec<(f64, f64, usize, crate::path::Child)> = Vec::new();
        for (i, c) in kids.into_iter().enumerate() {
            let p = self.ctx.edge(&c.entity.entity_id)?.and_then(|e| e.placement);
            let n = |k: &str| p.as_ref().and_then(|p| p.get(k)).and_then(Value::as_f64);
            keyed.push((n("y").unwrap_or(f64::MAX), n("x").unwrap_or(f64::MAX), i, c));
        }
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).then(a.2.cmp(&b.2)));
        Ok(keyed.into_iter().map(|k| k.3).collect())
    }

    fn canvas_parts(&mut self, e: &EntityRow, depth: usize) -> WsResult<Vec<Part>> {
        let mut out = vec![Part::text(format!("# {}", label_of(e)))];
        for c in self.reading_order(&e.entity_id)? {
            let x = c.entity;
            self.dep(&x);
            if crate::types::is_connector(&x.type_id, &x.payload) {
                continue;
            }
            let name = if c.shadowed { format!("~{}", x.entity_id) } else { x.name.clone().unwrap_or_default() };
            let text = match x.type_id.as_str() {
                TYPE_CELL => self.block_text(&x)?,
                TYPE_CONTAINER if depth < 8 => {
                    let d = describe(self.ctx, &x)?;
                    self.parts(&x, &d, depth + 1)?.into_iter().map(|p| p.text).collect::<Vec<_>>().join("\n")
                }
                _ => label_of(&x),
            };
            out.push(Part::of(&x.entity_id, format!("[{name}] {text}").trim_end().to_string()));
        }
        Ok(out)
    }

    /// The Markdown outline of a control's Blocks (a mind map); a data control names the tables it shows.
    fn control_text(&mut self, e: &EntityRow, depth: usize) -> WsResult<String> {
        let kind = e.payload.get("kind").and_then(Value::as_str).unwrap_or("");
        let mut out = format!("{kind} control");
        if let Some(t) = title_of(e) {
            out.push_str(&format!(" \"{t}\""));
        }
        if crate::types::is_data_control(kind) {
            let mut shown = Vec::new();
            for (k, v) in e.payload.get("bindings").and_then(Value::as_object).into_iter().flatten() {
                let ids: Vec<&str> = match v {
                    Value::Array(a) => a.iter().filter_map(|i| i.get("entity_id").and_then(Value::as_str)).collect(),
                    _ => v.get("entity_id").and_then(Value::as_str).into_iter().collect(),
                };
                for id in ids {
                    let p = self.readable(id)?.map(|(_, p)| p).unwrap_or_else(|| format!("~{id}"));
                    shown.push(format!("{k}: {p}"));
                }
            }
            if !shown.is_empty() {
                out.push_str(&format!(", shows {}", shown.join(", ")));
            }
            return Ok(out);
        }
        fn walk(b: &mut Builder, id: &str, level: usize, depth: usize, out: &mut String) -> WsResult<()> {
            if level > 64 {
                return Ok(());
            }
            for c in b.reading_order_tree(id)? {
                let x = c.entity;
                b.dep(&x);
                if crate::types::is_connector(&x.type_id, &x.payload) {
                    continue;
                }
                let text = match x.type_id.as_str() {
                    TYPE_CELL => b.block_text(&x)?.replace('\n', " "),
                    TYPE_CONTAINER if depth < 8 => b.control_text(&x, depth + 1)?.replace('\n', " "),
                    _ => label_of(&x),
                };
                let slot = x.payload.get("slot").and_then(Value::as_str).map(|s| format!("[{s}] ")).unwrap_or_default();
                out.push_str(&format!("\n{}- {slot}{text}", "  ".repeat(level)));
                if x.type_id == TYPE_CELL {
                    walk(b, &x.entity_id, level + 1, depth, out)?;
                }
            }
            Ok(())
        }
        walk(self, &e.entity_id, 0, depth, &mut out)?;
        Ok(out)
    }

    /// Children in sibling order (a tree control's order is its order keys).
    fn reading_order_tree(&mut self, id: &str) -> WsResult<Vec<crate::path::Child>> {
        Ok(crate::path::visible_children(self.ctx, self.access, id)?.into_iter().filter(|c| c.readable).collect())
    }

    /// A Block in words: its title, its view and what it shows (by path), a note's text.
    pub fn block_text(&mut self, e: &EntityRow) -> WsResult<String> {
        let view = e.payload.get("view").and_then(|v| v.get("type")).and_then(Value::as_str).unwrap_or("block").to_string();
        let title = title_of(e);
        let mut lines = Vec::new();
        if crate::types::is_text_view(&view) && e.payload.get("source_ref").is_none() {
            lines.push(title.clone().unwrap_or_default());
        } else {
            lines.push(match &title {
                Some(t) => format!("{view} \"{t}\""),
                None => view.clone(),
            });
        }
        if let Some(id) = e.payload.get("source_ref").and_then(crate::value::reference_entity_id).map(str::to_string) {
            match self.readable(&id)? {
                Some((x, _)) if x.type_id == TYPE_ANNOTATION => {
                    lines.push(x.payload.get("body").and_then(Value::as_str).unwrap_or("").to_string());
                }
                Some((x, path)) if x.type_id == TYPE_TABLE && view == "table" => {
                    let fields = live_fields(self.ctx, &x.entity_id)?;
                    lines.push(format!("table view of {path}"));
                    let summary = view_summary(e, &fields);
                    if !summary.is_empty() {
                        lines.push(summary);
                    }
                    let shown: Vec<String> = match e.payload.get("fields").and_then(Value::as_array) {
                        Some(list) => list
                            .iter()
                            .filter(|f| f["hidden"] != json!(true))
                            .filter_map(|f| f["field_id"].as_str())
                            .filter_map(|id| fields.iter().find(|x| x.field_id == id).map(|x| x.def.name.clone()))
                            .collect(),
                        None => fields.iter().map(|f| f.def.name.clone()).collect(),
                    };
                    lines.push(format!("columns: {}", shown.join(", ")));
                    match crate::sqlview::cell_view(self.ctx, self.access, e, crate::sqlview::Projection::Display) {
                        Ok(Some(v)) => lines.push(format!("sql: {}", v.sql)),
                        Ok(None) => {}
                        Err(err) => lines.push(format!("view broken: {}", err.detail)),
                    }
                }
                Some((x, path)) => lines.push(format!("shows {} {path}", crate::path::short_type(&x.type_id))),
                None => lines.push(format!("shows ~{id}")),
            }
        }
        if let Some(c) = e.payload.get("config").and_then(Value::as_object) {
            for k in ["text", "label", "caption", "alt", "url"] {
                if let Some(t) = c.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    lines.push(format!("{k}: {t}"));
                }
            }
        }
        Ok(lines.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n"))
    }
}

/// The `etag` of a body: its format version and the versions it depends on.
pub fn etag_of(deps: &[String]) -> String {
    let mut h = format!("v{BODY_FORMAT_VERSION}");
    for d in deps {
        h.push('\n');
        h.push_str(d);
    }
    sha256_hex(h.as_bytes())[..24].to_string()
}

/// The `etag` of `e`'s body without reading a table's records.
pub fn etag(ctx: &dyn ReadCtx, access: &Access, e: &EntityRow, assets: Option<AssetLoader>) -> WsResult<String> {
    let mut b = Builder::new(ctx, access, assets);
    if e.type_id == TYPE_TABLE && !crate::types::is_url_table(e) && e.degraded().is_none() {
        b.table_deps(e)?;
        let mut deps = std::mem::take(&mut b.deps);
        deps.sort();
        deps.dedup();
        return Ok(etag_of(&deps));
    }
    Ok(etag_of(&b.build(e)?.deps))
}

// ---- reading (`doc.read { as: "body" }`)

fn cursor_of(c: &str) -> WsResult<Value> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(c)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .ok_or_else(|| WsError::invalid_op("bad cursor"))
}

fn encode(v: &Value) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string())
}

/// The largest prefix of `s` of at most `max` bytes that ends on a character boundary.
pub fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

struct Page {
    text: String,
    parts: Vec<Value>,
    max: usize,
    full: bool,
}

impl Page {
    /// Add a part from byte `offset`: `(taken, cut_at)`; not taken when the page is full (the part did not start here).
    /// A part other than the body's first starts with the newline that separates it from the previous one, also at the
    /// top of a page: the pages of a body concatenate to the body exactly.
    fn push(&mut self, p: &Part, offset: usize, first_of_body: bool) -> (bool, Option<usize>) {
        let sep = if offset == 0 && !first_of_body { "\n" } else { "" };
        let rest = &p.text[offset..];
        if self.text.len() + sep.len() + rest.len() <= self.max {
            self.text.push_str(sep);
            let start = self.text.len();
            self.text.push_str(rest);
            self.parts.push(part_json(p, start, self.text.len(), offset > 0));
            return (true, None);
        }
        if !self.text.is_empty() {
            self.full = true;
            return (false, None);
        }
        // a part longer than a page: cut on a character boundary, continue there
        self.text.push_str(sep);
        let room = self.max.saturating_sub(sep.len());
        let piece = cut(rest, room);
        let piece = if piece.is_empty() { &rest[..rest.chars().next().map_or(0, char::len_utf8)] } else { piece };
        let start = self.text.len();
        self.text.push_str(piece);
        self.parts.push(part_json(p, start, self.text.len(), true));
        self.full = true;
        (true, Some(offset + piece.len()))
    }
}

fn part_json(p: &Part, start: usize, end: usize, partial: bool) -> Value {
    let mut v = json!({ "start": start, "end": end });
    if let Some(s) = &p.selector {
        v["selector"] = s.clone();
    }
    if let Some(id) = &p.entity_id {
        v["entity_id"] = json!(id);
    }
    if partial {
        v["partial"] = json!(true);
    }
    v
}

/// `doc.read { as: "body", selector?, cursor?, max_bytes?, parts? }` of `e` (readable, live). The pages concatenate to
/// the body exactly.
pub fn read_body(ctx: &dyn ReadCtx, access: &Access, e: &EntityRow, params: &Value, assets: Option<AssetLoader>) -> WsResult<Value> {
    let max = match params.get("max_bytes") {
        None | Some(Value::Null) => DEFAULT_MAX_BYTES,
        Some(v) => v.as_u64().filter(|n| *n >= 64 && *n as usize <= MAX_MAX_BYTES).map(|n| n as usize).ok_or_else(|| WsError::limit(format!("max_bytes must be 64..{MAX_MAX_BYTES}")))?,
    };
    let with_parts = params.get("parts").and_then(Value::as_bool).unwrap_or(false);
    let d = describe(ctx, e)?;
    let local_table = e.type_id == TYPE_TABLE && d.state == "full";
    let mut b = Builder::new(ctx, access, assets);
    let mut out = json!({
        "entity_id": e.entity_id, "path": crate::path::path_of(ctx, access, &e.entity_id)?,
        "format": d.format, "format_version": BODY_FORMAT_VERSION, "state": d.state,
    });
    if let Some(r) = d.reason {
        out["reason"] = json!(r);
    }
    // one part, named by its selector
    if let Some(sel) = params.get("selector").filter(|s| !s.is_null()) {
        let text = if local_table && sel.get("kind").and_then(Value::as_str) == Some("table_record") {
            let id = sel.get("record_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("selector.record_id required"))?;
            let r = ctx.record(&e.entity_id, id)?.filter(|r| r.alive()).ok_or_else(|| WsError::not_found(format!("record {id} not found")))?;
            let (fields, bodies, header) = b.table_header(e)?;
            format!("{header}\n{}", b.table_row(&fields, bodies, &r))
        } else {
            let body = b.build(e)?;
            let want = canonical_json(sel)?;
            body.parts
                .iter()
                .find(|p| p.selector.as_ref().and_then(|s| canonical_json(s).ok()).as_deref() == Some(want.as_str()))
                .map(|p| p.text.clone())
                .ok_or_else(|| WsError::not_found("no part of the body has that selector"))?
        };
        b.deps.clear();
        out["etag"] = json!(etag(ctx, access, e, assets)?);
        out["text"] = json!(text);
        out["complete"] = json!(true);
        out["selector"] = sel.clone();
        return Ok(out);
    }
    let tag = etag(ctx, access, e, assets)?;
    let cursor = match params.get("cursor").and_then(Value::as_str) {
        Some(c) => {
            let c = cursor_of(c)?;
            if c["e"] != json!(tag) {
                return Err(WsError::new(Code::SnapshotExpired, "the body changed since this cursor; read it again").with_data(json!({ "etag": tag })));
            }
            Some(c)
        }
        None => None,
    };
    let mut page = Page { text: String::new(), parts: Vec::new(), max, full: false };
    let mut next: Option<Value> = None;
    if local_table {
        let (fields, bodies, header) = b.table_header(e)?;
        let from = cursor.as_ref().and_then(|c| c["r"].as_str()).map(str::to_string);
        let mut offset = cursor.as_ref().and_then(|c| c["o"].as_u64()).unwrap_or(0) as usize;
        // the header is the first part (never cut); pages after the first continue the records
        if from.is_none() {
            page.text.push_str(&header);
            page.parts.push(part_json(&Part::text(header.clone()), 0, header.len(), false));
        }
        let mut stop: Option<Value> = None;
        ctx.scan_records_from(&e.entity_id, from.as_deref().unwrap_or(""), &mut |r| {
            if !r.alive() {
                return Ok(true);
            }
            let part = Part::sel(json!({ "kind": "table_record", "record_id": r.record_id }), b.table_row(&fields, bodies, r));
            let (taken, cut_at) = page.push(&part, offset.min(part.text.len()), false);
            offset = 0;
            if !taken {
                stop = Some(json!({ "e": tag, "r": r.record_id }));
                return Ok(false);
            }
            if let Some(o) = cut_at {
                stop = Some(json!({ "e": tag, "r": r.record_id, "o": o }));
                return Ok(false);
            }
            Ok(true)
        })?;
        next = stop;
    } else {
        let body = b.build(e)?;
        let start = cursor.as_ref().and_then(|c| c["i"].as_u64()).unwrap_or(0) as usize;
        let mut offset = cursor.as_ref().and_then(|c| c["o"].as_u64()).unwrap_or(0) as usize;
        for (i, p) in body.parts.iter().enumerate().skip(start) {
            let (taken, cut_at) = page.push(p, offset, i == 0);
            offset = 0;
            if !taken {
                next = Some(json!({ "e": tag, "i": i }));
                break;
            }
            if let Some(o) = cut_at {
                next = Some(json!({ "e": tag, "i": i, "o": o }));
                break;
            }
        }
    }
    out["etag"] = json!(tag);
    out["text"] = json!(page.text);
    out["complete"] = json!(next.is_none());
    if let Some(n) = next {
        out["next_cursor"] = json!(encode(&n));
    }
    if with_parts {
        out["parts"] = Value::Array(page.parts);
    }
    Ok(out)
}

/// A macro's manifest as plain text (AI Native 第一期 §2.2): what running it again reads, which AI it may call,
/// what it writes — the same lines for an Agent and a screen reader.
pub fn manifest_text(m: &Value) -> String {
    let mut lines = vec!["macro:".to_string()];
    for (name, i) in m.get("inputs").and_then(Value::as_object).into_iter().flatten() {
        let fields: Vec<&str> = i.get("fields").and_then(Value::as_array).into_iter().flatten().filter_map(|f| f.get("name").and_then(Value::as_str)).collect();
        lines.push(if fields.is_empty() { format!("- reads {name}") } else { format!("- reads {name}: {}", fields.join(", ")) });
    }
    let results: Vec<String> = m.get("results").and_then(Value::as_object).into_iter().flatten().map(|(n, r)| format!("{n} ({})", r.get("type").and_then(Value::as_str).unwrap_or(""))).collect();
    if !results.is_empty() {
        lines.push(format!("- writes {}", results.join(", ")));
    }
    let ai = m.get("ai").cloned().unwrap_or(Value::Null);
    if let Some(c) = ai.get("complete").filter(|c| c.is_object()) {
        let n = |k: &str| c.get(k).and_then(Value::as_u64).map(|n| n.to_string()).unwrap_or_else(|| "default".into());
        lines.push(format!("- AI: single inferences, at most {} calls and {} tokens", n("max_calls"), n("max_tokens")));
    }
    if let Some(mp) = ai.get("map").filter(|c| c.is_object()) {
        lines.push(format!("- AI: per-item judgements, at most {} items", mp.get("max_items").and_then(Value::as_u64).map(|n| n.to_string()).unwrap_or_else(|| "default".into())));
    }
    for (kind, what) in [("image", "images"), ("video", "videos")] {
        if let Some(x) = ai.get(kind).filter(|c| c.is_object()) {
            lines.push(format!("- AI: generated {what}, at most {}", x.get("max_calls").and_then(Value::as_u64).map(|n| n.to_string()).unwrap_or_else(|| "default".into())));
        }
    }
    if let Some(z) = m.get("zone").and_then(Value::as_array).filter(|z| !z.is_empty()) {
        lines.push(format!("- Zone resources: {}", z.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
    }
    if let Some(p) = m.get("params").and_then(Value::as_array).filter(|p| !p.is_empty()) {
        lines.push(format!("- parameters: {}", p.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
    }
    lines.join("\n")
}
