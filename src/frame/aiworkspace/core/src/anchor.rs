//! Annotation anchors (design doc §3.7). An annotation records where it belongs at three
//! depths — `target` (entity + selector, stable ids only), an optional `range` inside the
//! target, and `context.quote`, the annotated text — and is shown at the deepest one that
//! still resolves.
//!
//! Each content type contributes an [`AnchorAdapter`] for its selectors and built-in range
//! kinds. Range kinds named `<app>/<name>` belong to applications: the backend stores them
//! verbatim and leaves locating them to the application (`range_status: unchecked`).
//!
//! Writes are strict, replay is lenient: a commit accepted by a newer backend (an anchor kind
//! this one does not know) is kept as it is and reads back as `unsupported`.

use crate::canonical::canonical_json;
use crate::error::{Code, WsError, WsResult};
use crate::id::is_valid_id;
use crate::model::*;
use crate::richtext::{TextIndex, TextPos};
use crate::value::{normalize_reference, reference_entity_id};
use base64::Engine;
use serde_json::{json, Value};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
pub const MAX_ANCHOR_BYTES: usize = 4096;
pub const MAX_QUOTE_CHARS: usize = 2000;
pub const MAX_QUOTE_SIDE_CHARS: usize = 64;
pub const MAX_LABEL_CHARS: usize = 200;
const MAX_CURSOR_CHARS: usize = 512;
/// Text returned in `anchor.position.text`.
const POSITION_TEXT_CHARS: usize = 200;
/// A quote searched outside its recorded target must be at least this long.
pub const MIN_RELOCATE_CHARS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Shown where it was recorded (a range relocated inside its target counts).
    Resolved,
    /// Shown, but coarser than recorded or found again elsewhere by its quote.
    Degraded,
    /// The target entity is gone: nothing in the content to show it on.
    TargetDeleted,
    /// This backend does not understand the selector; shown on the entity.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Range,
    Target,
    Entity,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeStatus {
    /// Found by its stable position.
    Exact,
    /// Found again by `context.quote`.
    Relocated,
    Lost,
    /// An application's range: only the application can locate it.
    Unchecked,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Resolved => "resolved",
            State::Degraded => "degraded",
            State::TargetDeleted => "target_deleted",
            State::Unsupported => "unsupported",
        }
    }
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Range => "range",
            Level::Target => "target",
            Level::Entity => "entity",
            Level::None => "none",
        }
    }
}

impl RangeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RangeStatus::Exact => "exact",
            RangeStatus::Relocated => "relocated",
            RangeStatus::Lost => "lost",
            RangeStatus::Unchecked => "unchecked",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub state: State,
    /// The deepest level the annotation can be shown at now.
    pub level: Level,
    /// Present when the annotation has a range.
    pub range: Option<RangeStatus>,
    /// Where it is now, in the adapter's terms (e.g. the block its quote was found in).
    pub position: Option<Value>,
    /// The selector still names something.
    pub target_found: bool,
}

impl Anchor {
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "state": self.state.as_str(), "level": self.level.as_str() });
        if let Some(r) = self.range {
            v["range_status"] = json!(r.as_str());
        }
        if let Some(p) = &self.position {
            v["position"] = p.clone();
        }
        v
    }
}

/// `context.quote`: the annotated text and a little of what surrounds it.
pub struct Quote {
    pub exact: String,
    pub prefix: Vec<char>,
    pub suffix: Vec<char>,
}

impl Quote {
    pub fn from_context(context: Option<&Value>) -> Option<Quote> {
        let q = context?.get("quote")?;
        let exact = q.get("exact")?.as_str().filter(|s| !s.is_empty())?.to_string();
        let side = |k: &str| q.get(k).and_then(Value::as_str).unwrap_or("").chars().collect::<Vec<_>>();
        Some(Quote { exact, prefix: side("prefix"), suffix: side("suffix") })
    }

    fn len(&self) -> usize {
        self.exact.chars().count()
    }

    /// Char ranges of every occurrence in `hay`, each with how much of prefix/suffix agrees.
    fn occurrences(&self, hay: &str) -> Vec<(usize, usize, usize)> {
        let chars: Vec<char> = hay.chars().collect();
        let byte_to_char: std::collections::HashMap<usize, usize> = hay.char_indices().enumerate().map(|(c, (b, _))| (b, c)).collect();
        let n = self.len();
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(found) = hay[from..].find(&self.exact) {
            let b = from + found;
            let start = byte_to_char[&b];
            let before = self.prefix.iter().rev().zip(chars[..start].iter().rev()).take_while(|(a, b)| a == b).count();
            let after = self.suffix.iter().zip(chars[start + n..].iter()).take_while(|(a, b)| a == b).count();
            out.push((start, start + n, before + after));
            from = b + hay[b..].chars().next().map_or(1, char::len_utf8);
        }
        out
    }

    /// The best occurrence: most context agreement, then the earliest.
    pub fn find_best(&self, hay: &str) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, usize)> = None;
        for o in self.occurrences(hay) {
            if best.is_none_or(|b| o.2 > b.2) {
                best = Some(o);
            }
        }
        best.map(|(s, e, _)| (s, e))
    }

    /// An occurrence that cannot be confused with another: the only one, or the only one whose
    /// recorded context agrees completely. Short quotes never qualify.
    pub fn find_unique(&self, hay: &str) -> Option<(usize, usize)> {
        if self.len() < MIN_RELOCATE_CHARS {
            return None;
        }
        let all = self.occurrences(hay);
        if all.len() == 1 {
            return Some((all[0].0, all[0].1));
        }
        let full = self.prefix.len() + self.suffix.len();
        let agreeing: Vec<_> = all.iter().filter(|o| full > 0 && o.2 == full).collect();
        (agreeing.len() == 1).then(|| (agreeing[0].0, agreeing[0].1))
    }
}

/// What an adapter found for one anchor.
pub struct Found {
    pub target: bool,
    /// Status of a range the adapter understands; `None` without one.
    pub range: Option<RangeStatus>,
    pub position: Option<Value>,
}

impl Found {
    fn target(found: bool) -> Found {
        Found { target: found, range: None, position: None }
    }
}

/// Anchoring knowledge of one content type.
pub trait AnchorAdapter: Sync {
    /// Selector kinds besides `entity`.
    fn selector_kinds(&self) -> &'static [&'static str];
    /// Built-in range kinds (not the applications' `<app>/<name>` kinds).
    fn range_kinds(&self) -> &'static [&'static str] {
        &[]
    }
    /// Structure of one of `selector_kinds` (write path).
    fn check_selector(&self, selector: &Value) -> WsResult<()>;
    /// Structure of one of `range_kinds` under `selector` (write path).
    fn check_range(&self, _selector: &Value, _range: &Value) -> WsResult<()> {
        Ok(())
    }
    /// `range` is passed only when it is one of `range_kinds`.
    fn locate(&self, ctx: &dyn ReadCtx, e: &EntityRow, selector: &Value, range: Option<&Value>, quote: Option<&Quote>) -> WsResult<Found>;
}

pub fn adapter(type_id: &str) -> Option<&'static dyn AnchorAdapter> {
    match type_id {
        TYPE_TABLE => Some(&TableAnchors),
        TYPE_RICHTEXT => Some(&RichTextAnchors),
        _ => None,
    }
}

/// `<app>/<name>`: a range kind owned by an application.
pub fn is_app_kind(kind: &str) -> bool {
    let part = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'_'));
    kind.len() <= 128 && kind.split_once('/').is_some_and(|(app, name)| part(app) && part(name))
}

fn bad(detail: impl Into<String>) -> WsError {
    WsError::invalid_schema(detail)
}

fn kind_of(v: &Value) -> &str {
    v.get("kind").and_then(Value::as_str).unwrap_or("")
}

fn only(v: &Value, allowed: &[&str], what: &str) -> WsResult<()> {
    let o = v.as_object().ok_or_else(|| bad(format!("{what} must be an object")))?;
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(bad(format!("{what}: unknown key {k}"))),
        None => Ok(()),
    }
}

fn id_at(v: &Value, key: &str, what: &str) -> WsResult<()> {
    if v.get(key).and_then(Value::as_str).is_some_and(is_valid_id) {
        Ok(())
    } else {
        Err(bad(format!("{what}.{key} must be an id")))
    }
}

fn text_at(v: &Value, key: &str, max: usize, what: &str) -> WsResult<()> {
    match v.get(key) {
        None => Ok(()),
        Some(Value::String(s)) if s.chars().count() <= max => Ok(()),
        Some(_) => Err(bad(format!("{what}.{key} must be a string of at most {max} chars"))),
    }
}

/// Where the annotation in `payload` can be shown now.
pub fn resolve(ctx: &dyn ReadCtx, payload: &JsonMap) -> WsResult<Anchor> {
    let mut a = Anchor { state: State::TargetDeleted, level: Level::None, range: None, position: None, target_found: false };
    // a free note (no target) is shown wherever it is placed: nothing to resolve, nothing lost
    let Some(target) = payload.get("target") else {
        a.state = State::Resolved;
        return Ok(a);
    };
    let Some(e) = reference_entity_id(target).map(|id| ctx.entity(id)).transpose()?.flatten().filter(EntityRow::alive) else {
        return Ok(a);
    };
    let entity = json!({ "kind": "entity" });
    let selector = target.get("selector").unwrap_or(&entity);
    let kind = kind_of(selector);
    let adapter = adapter(&e.type_id);
    if kind != "entity" && !adapter.is_some_and(|ad| ad.selector_kinds().contains(&kind)) {
        a.state = State::Unsupported;
        a.level = Level::Entity;
        return Ok(a);
    }
    let range = payload.get("range");
    let found = match adapter {
        Some(ad) => {
            let known = range.filter(|r| ad.range_kinds().contains(&kind_of(r)));
            ad.locate(ctx, &e, selector, known, Quote::from_context(payload.get("context")).as_ref())?
        }
        None => Found::target(true),
    };
    a.target_found = found.target;
    a.range = range.map(|_| found.range.unwrap_or(RangeStatus::Unchecked));
    a.position = found.position;
    (a.state, a.level) = match (found.target, a.range) {
        (true, None | Some(RangeStatus::Unchecked)) => (State::Resolved, Level::Target),
        (true, Some(RangeStatus::Exact | RangeStatus::Relocated)) => (State::Resolved, Level::Range),
        (true, Some(RangeStatus::Lost)) => (State::Degraded, Level::Target),
        (false, Some(RangeStatus::Relocated)) => (State::Degraded, Level::Range),
        (false, _) if a.position.is_some() => (State::Degraded, Level::Target),
        (false, _) => (State::Degraded, Level::Entity),
    };
    Ok(a)
}

/// Normalize and check the anchor keys of an annotation (`target`, `range`, `context`).
/// `strict` — every write except replay and import — also requires an anchor this backend
/// understands whose target exists now. The caller has checked the target entity itself.
pub fn check(ctx: &dyn ReadCtx, payload: &mut JsonMap, strict: bool) -> WsResult<()> {
    let target = payload.get("target").ok_or_else(|| bad("annotation needs target"))?;
    if target.get("workspace_id").is_some() {
        return Err(bad("an annotation target must be in the same Workspace"));
    }
    let mut norm = normalize_reference(&json!({ "entity_id": target.get("entity_id").cloned().unwrap_or(Value::Null),
                                                 "selector": target.get("selector").cloned().unwrap_or(json!({ "kind": "entity" })) }))?;
    norm.as_object_mut().unwrap().remove("version");
    let entity = json!({ "kind": "entity" });
    let selector = norm.get("selector").unwrap_or(&entity).clone();
    let range = payload.get("range").cloned();
    if canonical_json(&selector)?.len() > MAX_ANCHOR_BYTES || range.as_ref().map_or(Ok(0), |r| canonical_json(r).map(|s| s.len()))? > MAX_ANCHOR_BYTES {
        return Err(WsError::limit(format!("annotation selector and range are limited to {MAX_ANCHOR_BYTES} bytes each")));
    }
    let e = reference_entity_id(&norm).map(|id| ctx.entity(id)).transpose()?.flatten();
    let adapter = e.as_ref().and_then(|e| adapter(&e.type_id));
    let kind = kind_of(&selector);
    if kind != "entity" {
        match adapter.filter(|a| a.selector_kinds().contains(&kind)) {
            Some(a) => a.check_selector(&selector)?,
            None if strict => return Err(bad(format!("annotation anchor kind {kind} is not supported on this target"))),
            None => {}
        }
    }
    if let Some(r) = &range {
        let rk = r.get("kind").and_then(Value::as_str).ok_or_else(|| bad("range.kind required"))?;
        if !is_app_kind(rk) {
            match adapter.filter(|a| a.range_kinds().contains(&rk)) {
                Some(a) => a.check_range(&selector, r)?,
                None if strict => return Err(bad(format!("range kind {rk} is not supported on this target"))),
                None => {}
            }
        }
    }
    if strict {
        if let Some(c) = payload.get("context") {
            check_context(c)?;
        }
    }
    payload.insert("target".into(), norm);
    if strict && !resolve(ctx, payload)?.target_found {
        return Err(WsError::new(Code::ReferenceBroken, "annotation target does not exist"));
    }
    Ok(())
}

fn check_context(c: &Value) -> WsResult<()> {
    only(c, &["quote", "label"], "context")?;
    text_at(c, "label", MAX_LABEL_CHARS, "context")?;
    if let Some(q) = c.get("quote") {
        only(q, &["exact", "prefix", "suffix"], "context.quote")?;
        if !q.get("exact").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
            return Err(bad("context.quote.exact required"));
        }
        text_at(q, "exact", MAX_QUOTE_CHARS, "context.quote")?;
        text_at(q, "prefix", MAX_QUOTE_SIDE_CHARS, "context.quote")?;
        text_at(q, "suffix", MAX_QUOTE_SIDE_CHARS, "context.quote")?;
    }
    Ok(())
}

// ---- table sources ----

struct TableAnchors;

impl AnchorAdapter for TableAnchors {
    fn selector_kinds(&self) -> &'static [&'static str] {
        &["table_record", "table_field", "table_cell"]
    }

    fn check_selector(&self, s: &Value) -> WsResult<()> {
        let keys: &[&str] = match kind_of(s) {
            "table_record" => &["kind", "record_id"],
            "table_field" => &["kind", "field_id"],
            _ => &["kind", "record_id", "field_id"],
        };
        only(s, keys, "selector")?;
        keys[1..].iter().try_for_each(|k| id_at(s, k, "selector"))
    }

    fn locate(&self, ctx: &dyn ReadCtx, e: &EntityRow, s: &Value, _range: Option<&Value>, _quote: Option<&Quote>) -> WsResult<Found> {
        let record = |k: &str| -> WsResult<bool> { Ok(ctx.record(&e.entity_id, s[k].as_str().unwrap_or(""))?.is_some_and(|r| r.alive())) };
        let field = |k: &str| -> WsResult<bool> { Ok(ctx.field(&e.entity_id, s[k].as_str().unwrap_or(""))?.is_some_and(|f| f.alive())) };
        Ok(Found::target(match kind_of(s) {
            "table_record" => record("record_id")?,
            "table_field" => field("field_id")?,
            "table_cell" => record("record_id")? && field("field_id")?,
            _ => true,
        }))
    }
}

// ---- rich text ----

/// `richtext_block` selects a block by id. `richtext_text` is a text range that starts in that
/// block: `{ start: { block_id, cursor? }, end: { block_id, cursor? } }`, each cursor an encoded
/// Loro cursor (base64). Cursors follow concurrent text edits; a block rebuilt by a block
/// operation (replace/move) loses them and the range is found again by its quote.
struct RichTextAnchors;

fn decode_cursor(end: &Value) -> Option<Vec<u8>> {
    B64.decode(end.get("cursor")?.as_str()?).ok()
}

fn span_position(idx: &TextIndex, (from, to): (TextPos, TextPos)) -> Value {
    let mut p = json!({ "block_id": idx.blocks[from.0].block_id,
                        "text": idx.text_between(from, to).chars().take(POSITION_TEXT_CHARS).collect::<String>() });
    if to.0 != from.0 {
        p["end_block_id"] = json!(idx.blocks[to.0].block_id);
    }
    p
}

/// Search `quote` in textblocks `lo..=hi`: the best match, or only an unmistakable one.
fn search(idx: &TextIndex, lo: usize, hi: usize, quote: &Quote, unique: bool) -> Option<(TextPos, TextPos)> {
    let (flat, starts) = idx.flat(lo, hi);
    let (s, e) = if unique { quote.find_unique(&flat)? } else { quote.find_best(&flat)? };
    Some((idx.unflat(lo, &starts, s), idx.unflat(lo, &starts, e)))
}

impl AnchorAdapter for RichTextAnchors {
    fn selector_kinds(&self) -> &'static [&'static str] {
        &["richtext_block"]
    }

    fn range_kinds(&self) -> &'static [&'static str] {
        &["richtext_text"]
    }

    fn check_selector(&self, s: &Value) -> WsResult<()> {
        only(s, &["kind", "block_id"], "selector")?;
        id_at(s, "block_id", "selector")
    }

    fn check_range(&self, selector: &Value, r: &Value) -> WsResult<()> {
        if kind_of(selector) != "richtext_block" {
            return Err(bad("a richtext_text range needs a richtext_block selector"));
        }
        only(r, &["kind", "start", "end"], "range")?;
        for k in ["start", "end"] {
            let end = r.get(k).ok_or_else(|| bad(format!("range.{k} required")))?;
            only(end, &["block_id", "cursor"], "range end")?;
            id_at(end, "block_id", &format!("range.{k}"))?;
            match end.get("cursor") {
                None | Some(Value::Null) => {}
                Some(Value::String(c)) if c.len() <= MAX_CURSOR_CHARS => {
                    let ok = B64.decode(c).ok().is_some_and(|b| loro::cursor::Cursor::decode(&b).is_ok());
                    if !ok {
                        return Err(bad(format!("range.{k}.cursor is not an encoded Loro cursor")));
                    }
                }
                Some(_) => return Err(bad(format!("range.{k}.cursor must be a string"))),
            }
        }
        if r["start"]["block_id"] != selector["block_id"] {
            return Err(bad("a richtext_text range starts in the selected block"));
        }
        Ok(())
    }

    fn locate(&self, ctx: &dyn ReadCtx, e: &EntityRow, s: &Value, range: Option<&Value>, quote: Option<&Quote>) -> WsResult<Found> {
        if kind_of(s) != "richtext_block" {
            return Ok(Found::target(true));
        }
        let Some(rt) = ctx.richtext(&e.entity_id)? else { return Ok(Found::target(false)) };
        let block_id = s["block_id"].as_str().unwrap_or("");
        let target = rt.meta.block_index.contains_key(block_id);
        if range.is_none() && (target || quote.is_none()) {
            return Ok(Found::target(target));
        }
        let idx = TextIndex::build(&rt.doc);
        let last = idx.blocks.len().checked_sub(1);
        let anywhere = |q: &Quote| last.and_then(|l| search(&idx, 0, l, q, true));
        let Some(r) = range else {
            // a block-level annotation whose block is gone: the block its quote is in now
            let position = quote.and_then(&anywhere).map(|(from, _)| json!({ "block_id": idx.blocks[from.0].block_id }));
            return Ok(Found { target, range: None, position });
        };
        let found = |status, span| Found { target, range: Some(status), position: Some(span_position(&idx, span)) };
        if target {
            let start = decode_cursor(&r["start"]).and_then(|c| idx.cursor(&rt.doc, &c));
            let end = decode_cursor(&r["end"]).and_then(|c| idx.cursor(&rt.doc, &c));
            if let (Some(from), Some(to)) = (start, end) {
                if from < to && idx.blocks[from.0].block_id == block_id {
                    return Ok(found(RangeStatus::Exact, (from, to)));
                }
            }
            if let (Some(q), Some(lo)) = (quote, idx.block(block_id)) {
                let hi = r["end"]["block_id"].as_str().and_then(|b| idx.block(b)).filter(|hi| *hi >= lo).unwrap_or(lo);
                if let Some(span) = search(&idx, lo, hi, q, false) {
                    return Ok(found(RangeStatus::Relocated, span));
                }
            }
            return Ok(Found { target, range: Some(RangeStatus::Lost), position: None });
        }
        Ok(match quote.and_then(anywhere) {
            Some(span) => found(RangeStatus::Relocated, span),
            None => Found { target, range: Some(RangeStatus::Lost), position: None },
        })
    }
}
