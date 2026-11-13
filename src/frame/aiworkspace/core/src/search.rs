//! Text queries (核心 API 调整 R3, R6, §3.2; design draft §6): a literal string looked for in the names, titles and
//! bodies of the entities under a node, as the caller may read them. What is matched is the same text `body` reads
//! (a select by its label, a reference by its target's title), so a hit's selector points into what the caller can
//! read. Containers are matched by name and title only (their bodies are their children, which are searched
//! themselves). No index: a bounded scan whose coverage the answer states; an empty answer means "not there" only
//! when `coverage.complete` is true.

use crate::access::Access;
use crate::body::{describe, display_value, AssetLoader, Builder};
use crate::error::{WsError, WsResult};
use crate::model::*;
use crate::path::{child_path, decode_cursor, new_cursor, path_of, request_digest, resolve, short_type, visible_children, Child, TypeFilter};
use serde_json::{json, Value};

pub const HITS_DEFAULT: usize = 20;
pub const HITS_MAX: usize = 100;
/// Hits of one entity in one answer; the rest are counted in the last one's `more`.
pub const HITS_PER_ENTITY: usize = 5;
/// Text one call scans at most.
pub const SCAN_BUDGET_BYTES: usize = 8 * 1024 * 1024;
pub const SNIPPET_CHARS: usize = 40;
pub const MAX_DEPTH: usize = 16;

/// Literal matching, case-insensitive unless `case`.
pub struct Matcher {
    needle: String,
    case: bool,
}

impl Matcher {
    pub fn new(text: &str, case: bool) -> Matcher {
        Matcher { needle: if case { text.to_string() } else { text.to_lowercase() }, case }
    }

    /// Byte ranges of the matches in `hay` (at most `max`, and the total count).
    pub fn find(&self, hay: &str, max: usize) -> (Vec<(usize, usize)>, usize) {
        let mut out = Vec::new();
        let mut n = 0;
        if self.case {
            let mut from = 0;
            while let Some(i) = hay[from..].find(&self.needle) {
                let s = from + i;
                n += 1;
                if out.len() < max {
                    out.push((s, s + self.needle.len()));
                }
                from = s + self.needle.len().max(1);
                if from > hay.len() {
                    break;
                }
            }
            return (out, n);
        }
        // lower-case per character, remembering where each lowered character came from
        let mut low = String::with_capacity(hay.len());
        let mut origin: Vec<(usize, usize)> = Vec::with_capacity(hay.len());
        for (i, c) in hay.char_indices() {
            for l in c.to_lowercase() {
                let at = low.len();
                low.push(l);
                for _ in at..low.len() {
                    origin.push((i, i + c.len_utf8()));
                }
            }
        }
        let mut from = 0;
        while let Some(i) = low[from..].find(&self.needle) {
            let s = from + i;
            let e = s + self.needle.len();
            n += 1;
            if out.len() < max {
                out.push((origin[s].0, origin[e - 1].1));
            }
            from = e.max(s + 1);
            while from < low.len() && !low.is_char_boundary(from) {
                from += 1;
            }
            if from >= low.len() {
                break;
            }
        }
        (out, n)
    }
}

/// `SNIPPET_CHARS` characters around a match, line breaks as spaces.
pub fn snippet(text: &str, start: usize, end: usize) -> String {
    let before: String = text[..start].chars().rev().take(SNIPPET_CHARS).collect::<Vec<_>>().into_iter().rev().collect();
    let after: String = text[end..].chars().take(SNIPPET_CHARS).collect();
    format!("{before}{}{after}", &text[start..end]).replace(['\n', '\r'], " ")
}

struct Run<'a> {
    m: Matcher,
    hits: Vec<Value>,
    limit: usize,
    scanned_bytes: usize,
    scanned: usize,
    skipped: Vec<Value>,
    stop: &'a dyn Fn() -> bool,
}

impl Run<'_> {
    fn over_budget(&self) -> bool {
        self.scanned_bytes >= SCAN_BUDGET_BYTES || (self.stop)()
    }
}

struct EntityHits {
    hits: Vec<Value>,
    more: usize,
}

impl EntityHits {
    /// The first match of one text gets a hit (while the entity has room); further matches are counted.
    fn add(&mut self, hit: Value, count: usize) {
        let mut count = count;
        if count > 0 && self.hits.len() < HITS_PER_ENTITY {
            self.hits.push(hit);
            count -= 1;
        }
        self.more += count;
    }
}

fn base_hit(e: &EntityRow, path: &str, on: &str) -> Value {
    json!({ "entity_id": e.entity_id, "path": path, "type": short_type(&e.type_id), "on": on })
}

/// Look in one entity. Returns `Some(record_id)` when a table scan stopped on the budget there (resume from it).
fn scan_entity(run: &mut Run, ctx: &dyn ReadCtx, access: &Access, e: &EntityRow, path: &str, assets: Option<AssetLoader>, resume: Option<&str>) -> WsResult<Option<String>> {
    run.scanned += 1;
    let mut eh = EntityHits { hits: Vec::new(), more: 0 };
    let text_hit = |eh: &mut EntityHits, on: &str, selector: Option<Value>, text: &str, run: &mut Run| {
        run.scanned_bytes += text.len();
        let (found, n) = run.m.find(text, 1);
        if let Some((s, t)) = found.first() {
            let mut h = base_hit(e, path, on);
            if let Some(sel) = selector {
                h["selector"] = sel;
            }
            h["snippet"] = json!(snippet(text, *s, *t));
            eh.add(h, n);
        }
    };
    if resume.is_none() {
        if let Some(n) = &e.name {
            text_hit(&mut eh, "name", None, n, run);
        }
        if let Some(t) = crate::naming::title_source(&e.type_id, &e.payload).filter(|t| Some(t) != e.name.as_ref()) {
            text_hit(&mut eh, "title", None, &t, run);
        }
    }
    let d = describe(ctx, e)?;
    let mut stopped = None;
    let container = e.type_id == TYPE_CONTAINER && e.degraded().is_none();
    if container {
        // matched by name and title only
    } else if d.state == "unavailable" {
        run.skipped.push(json!({ "entity_id": e.entity_id, "reason": d.reason.unwrap_or("unavailable") }));
    } else if e.type_id == TYPE_TABLE && d.state != "definition" && e.degraded().is_none() {
        let mut b = Builder::new(ctx, access, assets);
        let (fields, _, _) = b.table_header(e)?;
        let mut refs = |v: &Value| -> String {
            let Some(id) = crate::value::reference_entity_id(v) else { return v.to_string() };
            match ctx.entity(id).ok().flatten() {
                Some(x) if x.alive() && access.can_read(ctx, &x).unwrap_or(false) => {
                    let label = crate::naming::title_source(&x.type_id, &x.payload).or_else(|| x.name.clone()).unwrap_or_default();
                    format!("{label} ({})", path_of(ctx, access, id).unwrap_or_default())
                }
                _ => format!("~{id}"),
            }
        };
        let mut seen = 0usize;
        ctx.scan_records_from(&e.entity_id, resume.unwrap_or(""), &mut |r| {
            if !r.alive() {
                return Ok(true);
            }
            seen += 1;
            if seen % 256 == 0 && run.over_budget() {
                stopped = Some(r.record_id.clone());
                return Ok(false);
            }
            for f in &fields {
                let text = display_value(&f.def, r.values.get(&f.field_id), &mut refs);
                if !text.is_empty() {
                    text_hit(&mut eh, "body", Some(json!({ "kind": "table_cell", "record_id": r.record_id, "field_id": f.field_id })), &text, run);
                }
            }
            Ok(true)
        })?;
    } else {
        if d.state == "definition" {
            // a URL table (R7): its definition is searched below, its rows are remote and not
            run.skipped.push(json!({ "entity_id": e.entity_id, "reason": "definition" }));
        }
        let mut b = Builder::new(ctx, access, assets);
        let body = b.build(e)?;
        for p in &body.parts {
            text_hit(&mut eh, "body", p.selector.clone(), &p.text, run);
        }
    }
    if let Some(last) = eh.hits.last_mut() {
        if eh.more > 0 {
            last["more"] = json!(eh.more);
        }
    }
    let room = run.limit.saturating_sub(run.hits.len());
    if eh.hits.len() > room {
        let dropped = eh.hits.len() - room;
        eh.hits.truncate(room);
        if let Some(last) = eh.hits.last_mut() {
            last["more"] = json!(last["more"].as_u64().unwrap_or(0) as usize + dropped);
        }
    }
    run.hits.extend(eh.hits);
    Ok(stopped)
}

/// `doc.query { text, case?, at?, depth?, type?, not_type?, limit?, cursor? }`. `stop` says when the time budget is
/// spent (the core has no clock).
pub fn text_query(ctx: &dyn ReadCtx, access: &Access, params: &Value, head_seq: u64, assets: Option<AssetLoader>, stop: &dyn Fn() -> bool) -> WsResult<Value> {
    for k in ["regex", "pattern", "mode", "semantic"] {
        if params.get(k).is_some() {
            return Err(WsError::invalid_op(format!("{k} is not supported: a text query is a literal string")));
        }
    }
    let text = params.get("text").and_then(Value::as_str).filter(|t| !t.is_empty() && t.chars().count() <= 256).ok_or_else(|| WsError::invalid_op("text must be a string of 1..256 characters"))?;
    let case = params.get("case").and_then(Value::as_bool).unwrap_or(false);
    let at = params.get("at").and_then(Value::as_str).unwrap_or("/");
    let filter = TypeFilter::parse(params, true)?;
    let limit = match params.get("limit") {
        None | Some(Value::Null) => HITS_DEFAULT,
        Some(v) => v.as_u64().filter(|n| *n >= 1 && *n as usize <= HITS_MAX).ok_or_else(|| WsError::limit(format!("limit must be 1..{HITS_MAX}")))? as usize,
    };
    let depth = match params.get("depth") {
        None | Some(Value::Null) => MAX_DEPTH,
        Some(v) => v.as_u64().filter(|n| (*n as usize) <= MAX_DEPTH).ok_or_else(|| WsError::limit(format!("depth must be 0..{MAX_DEPTH}")))? as usize,
    };
    let digest = request_digest(params, &["text", "case", "at", "depth", "type", "not_type"]);
    let cursor = decode_cursor(params.get("cursor").and_then(Value::as_str), head_seq, &digest)?;
    let skip = cursor.as_ref().map_or(0, |c| c["n"].as_u64().unwrap_or(0) as usize);
    let resume_record = cursor.as_ref().and_then(|c| c["r"].as_str()).map(str::to_string);
    let id = resolve(ctx, access, at)?;
    let at_path = path_of(ctx, access, &id)?;
    let mut run = Run { m: Matcher::new(text, case), hits: Vec::new(), limit, scanned_bytes: 0, scanned: 0, skipped: Vec::new(), stop };
    // the walk: `at` itself (unless the root), then its subtree in pre-order
    let mut stack: Vec<(Child, String, usize)> = Vec::new();
    let push_children = |stack: &mut Vec<(Child, String, usize)>, parent: &str, parent_path: &str, level: usize| -> WsResult<()> {
        let kids: Vec<Child> = visible_children(ctx, access, parent)?.into_iter().filter(|c| c.readable).collect();
        for c in kids.into_iter().rev() {
            let path = child_path(parent_path, &c);
            stack.push((c, path, level));
        }
        Ok(())
    };
    if id == ROOT_ID {
        push_children(&mut stack, &id, &at_path, 1)?;
    } else {
        let e = ctx.entity(&id)?.ok_or_else(|| WsError::not_found("entity not found"))?;
        stack.push((Child { entity: e, readable: true, shadowed: false }, at_path.clone(), 0));
    }
    let mut visited = 0usize;
    let mut next: Option<Value> = None;
    while let Some((c, path, level)) = stack.pop() {
        if visited >= skip && (run.hits.len() >= limit || run.over_budget()) {
            next = Some(json!({ "n": visited }));
            break;
        }
        visited += 1;
        if visited > skip && filter.matches(&c.entity) {
            let resume = if visited == skip + 1 { resume_record.as_deref() } else { None };
            if let Some(r) = scan_entity(&mut run, ctx, access, &c.entity, &path, assets, resume)? {
                // stopped inside a table: the next call resumes it at that record
                next = Some(json!({ "n": visited - 1, "r": r }));
                break;
            }
        }
        if level < depth {
            push_children(&mut stack, &c.entity.entity_id, &path, level + 1)?;
        }
    }
    let complete = next.is_none();
    let mut out = json!({
        "hits": run.hits,
        "coverage": { "complete": complete, "scanned": run.scanned, "bytes": run.scanned_bytes, "skipped": run.skipped },
        "at": { "entity_id": id, "path": at_path },
        "head_seq": head_seq,
    });
    if let Some(n) = next {
        out["next_cursor"] = json!(new_cursor(head_seq, &digest, n));
    }
    Ok(out)
}

/// The same matching over the text of one table cell, for callers that hold rows already (the stage's `ws_find`).
pub fn cell_text(def: &crate::value::FieldDef, v: Option<&Value>, refs: &mut dyn FnMut(&Value) -> String) -> String {
    display_value(def, v, refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        let m = Matcher::new("abc", false);
        let (hits, n) = m.find("xxABCyyabc", 5);
        assert_eq!(n, 2);
        assert_eq!(hits, vec![(2, 5), (7, 10)]);
        let m = Matcher::new("销售", false);
        let (hits, _) = m.find("本月销售额", 5);
        assert_eq!(&"本月销售额"[hits[0].0..hits[0].1], "销售");
        let m = Matcher::new("İ", false);
        let (hits, n) = m.find("aİb", 5);
        assert_eq!(n, 1, "{hits:?}");
        let m = Matcher::new("ABC", true);
        assert_eq!(m.find("abcABC", 5).1, 1);
        assert_eq!(snippet("line1\nhello world", 6, 11), "line1 hello world");
    }
}
