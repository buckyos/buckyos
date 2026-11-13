//! Entity names (核心 API 调整 R14, R19; design draft §2.2): every entity but the root has a `name`, unique
//! among its visible siblings, so a path of names is a meaningful, unambiguous address.
//!
//! A name nobody chose follows the entity: it is the entity's title turned into a name (`销售`), or, without a
//! title, a short type prefix and a counter (`text-3`, `table-1`). Whether a name *follows* is not stored: a name
//! follows while it is still what the rule would give (the title's name, possibly with a `-<n>` suffix, or the
//! counter name of the type). A name someone set to anything else stays as it is.

use crate::error::WsResult;
use crate::model::*;
use serde_json::Value;
use std::collections::BTreeMap;

/// The longest name derived from a title.
pub const MAX_DERIVED_CHARS: usize = 64;

/// The text a name follows: the title, a file's name, a record's `title` / `name` property.
pub fn title_source(type_id: &str, payload: &JsonMap) -> Option<String> {
    let s = |k: &str| payload.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    match type_id {
        TYPE_ASSET => s("title").or_else(|| s("file_name")),
        TYPE_RECORD => s("p:title").or_else(|| s("p:name")),
        _ => s("title"),
    }
}

/// A title turned into a name: no control characters, `/` → `-`, no leading `~`, at most 64 characters; `None` when
/// nothing is left.
pub fn slug(title: &str) -> Option<String> {
    let cleaned: String = title.chars().filter(|c| !c.is_control()).map(|c| if c == '/' { '-' } else { c }).collect();
    let cleaned = cleaned.trim().trim_start_matches('~').trim();
    let cut: String = cleaned.chars().take(MAX_DERIVED_CHARS).collect();
    let cut = cut.trim().to_string();
    if cut.is_empty() || cut == "." || cut == ".." {
        None
    } else {
        Some(cut)
    }
}

fn sanitize_prefix(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let out = out.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let out: String = out.chars().take(32).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "item".into()
    } else {
        out
    }
}

/// The counter prefix of an entity without a title: a Block's view type (`text`, `table`), a container's kind
/// (`folder`, `group`), otherwise a short name of its type.
pub fn counter_prefix(type_id: &str, payload: &JsonMap) -> String {
    match type_id {
        TYPE_CELL => sanitize_prefix(payload.get("view").and_then(|v| v.get("type")).and_then(Value::as_str).unwrap_or("block")),
        TYPE_CONTAINER => sanitize_prefix(payload.get("kind").and_then(Value::as_str).unwrap_or("folder")),
        TYPE_TABLE => "table".into(),
        TYPE_RICHTEXT => "doc".into(),
        TYPE_RECORD => "record".into(),
        TYPE_ASSET => "file".into(),
        TYPE_ANNOTATION => "note".into(),
        TYPE_WISH => "wish".into(),
        TYPE_BLOCK_DEF => "def".into(),
        TYPE_DIR => "dir".into(),
        TYPE_VIEWPORT => "viewport".into(),
        TYPE_SHOW_PATH => "path".into(),
        TYPE_TASK => "task".into(),
        TYPE_RESPONSE => "response".into(),
        TYPE_RESULT => "result".into(),
        t => sanitize_prefix(t.rsplit('.').next().unwrap_or(t)),
    }
}

/// What a following name is made from: the title's name, or the counter prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    Title(String),
    Counter(String),
}

pub fn base_of(type_id: &str, payload: &JsonMap) -> Base {
    match title_source(type_id, payload).and_then(|t| slug(&t)) {
        Some(s) => Base::Title(s),
        None => Base::Counter(counter_prefix(type_id, payload)),
    }
}

/// `name` is `<base>-<n>` (n ≥ 1, no leading zero): `Some(n)`.
pub fn suffix_of(name: &str, base: &str) -> Option<u64> {
    let rest = name.strip_prefix(base)?.strip_prefix('-')?;
    if rest.is_empty() || rest.starts_with('0') || !rest.bytes().all(|b| b.is_ascii_digit()) || rest.len() > 12 {
        return None;
    }
    rest.parse().ok()
}

/// Does `name` follow an entity whose base is `base` (it is what the rule gives, up to the suffix)?
pub fn follows(name: Option<&str>, base: &Base) -> bool {
    let Some(name) = name else { return true };
    match base {
        Base::Title(t) => name == t || suffix_of(name, t).is_some(),
        Base::Counter(p) => suffix_of(name, p).is_some(),
    }
}

/// Does the name of `e` follow it?
pub fn entity_follows(e: &EntityRow) -> bool {
    follows(e.name.as_deref(), &base_of(&e.type_id, &e.payload))
}

/// Do two scopes share a namespace among siblings? Shared names are seen by everyone; a personal (or participant)
/// entity also meets its own scope.
pub fn same_namespace(scope: &str, other: &str) -> bool {
    other == SCOPE_SHARED || other == scope
}

/// One sibling, as naming sees it.
#[derive(Debug, Clone)]
pub struct Sibling {
    pub entity_id: String,
    pub name: Option<String>,
    pub scope: String,
    pub alive: bool,
}

/// The siblings under `parent`.
pub fn siblings(ctx: &dyn ReadCtx, parent: &str) -> WsResult<Vec<Sibling>> {
    let mut out = Vec::new();
    for edge in ctx.children(parent)? {
        if let Some(e) = ctx.entity(&edge.child_id)? {
            out.push(Sibling { entity_id: e.entity_id, name: e.name, scope: e.scope, alive: e.deleted_seq.is_none() });
        }
    }
    Ok(out)
}

/// Is `name` used by a live sibling `scope` meets (other than `except`)?
pub fn taken(sibs: &[Sibling], name: &str, scope: &str, except: &str) -> bool {
    sibs.iter().any(|s| s.alive && s.entity_id != except && same_namespace(scope, &s.scope) && s.name.as_deref() == Some(name))
}

/// A free name for an entity of `scope` whose base is `base`: the title's name, else with the first free `-2`, `-3`, …;
/// a counter name one above the largest counter of that prefix among all siblings (deleted ones too, so a name is not
/// reused while its entity can come back). `keep` is the current name: kept when it already follows and is free.
pub fn free_name(sibs: &[Sibling], base: &Base, scope: &str, except: &str, keep: Option<&str>) -> String {
    if let Some(k) = keep.filter(|k| follows(Some(k), base) && !taken(sibs, k, scope, except)) {
        return k.to_string();
    }
    match base {
        Base::Title(t) => {
            if !taken(sibs, t, scope, except) {
                return t.clone();
            }
            let mut n = 2u64;
            loop {
                let candidate = format!("{t}-{n}");
                if !taken(sibs, &candidate, scope, except) {
                    return candidate;
                }
                n += 1;
            }
        }
        Base::Counter(prefix) => {
            let top = sibs
                .iter()
                .filter(|s| s.entity_id != except)
                .filter_map(|s| s.name.as_deref().and_then(|n| suffix_of(n, prefix)))
                .max()
                .unwrap_or(0);
            let mut n = top + 1;
            loop {
                let candidate = format!("{prefix}-{n}");
                if !taken(sibs, &candidate, scope, except) {
                    return candidate;
                }
                n += 1;
            }
        }
    }
}

/// Names for every live entity that has none (存量回填): in tree order, each given the name the rule gives
/// at that point. Returns `entity_id → name`. The root keeps no name.
pub fn backfill(ctx: &dyn ReadCtx) -> WsResult<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![ROOT_ID.to_string()];
    while let Some(parent) = stack.pop() {
        let mut sibs = siblings(ctx, &parent)?;
        for i in 0..sibs.len() {
            let id = sibs[i].entity_id.clone();
            if sibs[i].name.is_none() {
                if let Some(e) = ctx.entity(&id)? {
                    let name = free_name(&sibs, &base_of(&e.type_id, &e.payload), &e.scope, &id, None);
                    sibs[i].name = Some(name.clone());
                    out.insert(id.clone(), name);
                }
            }
            stack.push(id);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sib(id: &str, name: &str, alive: bool) -> Sibling {
        Sibling { entity_id: id.into(), name: Some(name.into()), scope: SCOPE_SHARED.into(), alive }
    }

    #[test]
    fn slugs() {
        assert_eq!(slug(" 销售/2026 ").as_deref(), Some("销售-2026"));
        assert_eq!(slug("~x").as_deref(), Some("x"));
        assert_eq!(slug("  "), None);
        assert_eq!(slug(".."), None);
        assert_eq!(slug(&"a".repeat(100)).map(|s| s.len()), Some(64));
    }

    #[test]
    fn following() {
        let b = Base::Title("销售".into());
        assert!(follows(Some("销售"), &b));
        assert!(follows(Some("销售-3"), &b));
        assert!(!follows(Some("销售-03"), &b));
        assert!(!follows(Some("订单"), &b));
        let c = Base::Counter("text".into());
        assert!(follows(Some("text-12"), &c));
        assert!(!follows(Some("text"), &c));
        assert!(follows(None, &c));
    }

    #[test]
    fn free_names() {
        let sibs = vec![sib("a", "销售", true), sib("b", "text-4", false), sib("c", "text-2", true)];
        assert_eq!(free_name(&sibs, &Base::Title("销售".into()), SCOPE_SHARED, "x", None), "销售-2");
        assert_eq!(free_name(&sibs, &Base::Title("订单".into()), SCOPE_SHARED, "x", None), "订单");
        assert_eq!(free_name(&sibs, &Base::Counter("text".into()), SCOPE_SHARED, "x", None), "text-5", "deleted names count");
        assert_eq!(free_name(&sibs, &Base::Title("销售".into()), SCOPE_SHARED, "a", Some("销售")), "销售", "kept");
        // a personal entity meets shared names and its own scope, not another person's
        let mine = vec![Sibling { entity_id: "p".into(), name: Some("草稿".into()), scope: "user:bob".into(), alive: true }];
        assert_eq!(free_name(&mine, &Base::Title("草稿".into()), "user:alice", "x", None), "草稿");
        assert_eq!(free_name(&mine, &Base::Title("草稿".into()), "user:bob", "x", None), "草稿-2");
    }

    #[test]
    fn bases() {
        let p = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(base_of(TYPE_CELL, &p(json!({ "view": { "type": "text" } }))), Base::Counter("text".into()));
        assert_eq!(base_of(TYPE_CELL, &p(json!({ "view": { "type": "ext:acme.vote" }, "title": "投票" }))), Base::Title("投票".into()));
        assert_eq!(base_of(TYPE_CELL, &p(json!({ "view": { "type": "ext:acme.vote" } }))), Base::Counter("ext-acme-vote".into()));
        assert_eq!(base_of(TYPE_ASSET, &p(json!({ "file_name": "a.png" }))), Base::Title("a.png".into()));
        assert_eq!(base_of(TYPE_RECORD, &p(json!({ "p:title": "周报" }))), Base::Title("周报".into()));
        assert_eq!(base_of(TYPE_CONTAINER, &p(json!({ "kind": "group" }))), Base::Counter("group".into()));
    }
}
