//! Markdown outline ⇄ the tree of a Block-control (脑图方案 §8, TODO M3), for wish `control` results and control
//! inputs: a port of `parseMarkdownOutline` / `formatMarkdownOutline` / `controlOutline` of `controls/outline.ts`.
//! Both parsers run the cases of `fixtures/mindmap/outlines.json`, so a program, the model and the Desktop land
//! the same tree.

use aiworkspace_core::model::*;
use aiworkspace_core::types::{enclosing_container, is_control_kind};
use aiworkspace_core::{WsError, WsResult};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The core's limit on a title.
pub const MAX_TITLE_CHARS: usize = 256;
/// Nodes of one control result / one control input at most.
pub const MAX_NODES: usize = 2000;
/// Levels of a control result at most.
pub const MAX_DEPTH: usize = 32;
/// Levels of a control input read at most (a deeper branch is refused, never cut).
const MAX_READ_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutlineNode {
    pub text: String,
    pub children: Vec<OutlineNode>,
}

impl OutlineNode {
    pub fn to_json(&self) -> Value {
        json!({ "text": self.text, "children": self.children.iter().map(OutlineNode::to_json).collect::<Vec<_>>() })
    }
}

/// `\s` of JavaScript: what the TypeScript parser trims and splits on.
fn js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

fn clean(text: &str) -> String {
    text.split(js_space).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

/// `^([-*_])(\s*\1){2,}$`: a thematic break.
fn is_rule(t: &str) -> bool {
    let mut cs = t.chars();
    let Some(c) = cs.next().filter(|c| matches!(c, '-' | '*' | '_')) else { return false };
    let mut n = 1;
    for x in cs {
        if x == c {
            n += 1;
        } else if !js_space(x) {
            return false;
        }
    }
    n >= 3
}

/// `^(#{1,6})\s+(.*?)\s*#*\s*$` on a trimmed line: (level, text).
fn heading(t: &str) -> Option<(u32, &str)> {
    let hashes = t.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &t[hashes..];
    let body = rest.trim_start_matches(js_space);
    if body.len() == rest.len() {
        return None;
    }
    Some((hashes as u32, body.trim_end_matches(js_space).trim_end_matches('#').trim_end_matches(js_space)))
}

/// `^(\s*)(?:[-*+]|\d{1,9}[.)])\s+(.*)$`: (indentation, text).
fn list_item(line: &str) -> Option<(usize, &str)> {
    let body = line.trim_start_matches(js_space);
    let indent = line[..line.len() - body.len()].chars().count();
    let marker = if body.starts_with(['-', '*', '+']) {
        1
    } else {
        let digits = body.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) || !matches!(body.as_bytes().get(digits), Some(b'.' | b')')) {
            return None;
        }
        digits + 1
    };
    let rest = &body[marker..];
    let text = rest.trim_start_matches(js_space);
    (text.len() < rest.len()).then_some((indent, text))
}

/// `^\[[ xX]\]\s+`: a task box.
fn strip_task(t: &str) -> &str {
    for b in ["[ ]", "[x]", "[X]"] {
        if let Some(r) = t.strip_prefix(b) {
            let s = r.trim_start_matches(js_space);
            if s.len() < r.len() {
                return s;
            }
        }
    }
    t
}

/// A parsed outline before it becomes a tree: bounds are checked here, without recursion.
#[derive(Default)]
struct Arena {
    /// text, children, depth (roots at 1)
    nodes: Vec<(String, Vec<usize>, usize)>,
    roots: Vec<usize>,
}

impl Arena {
    fn push(&mut self, text: String, parent: Option<usize>) -> usize {
        let i = self.nodes.len();
        let depth = parent.map_or(1, |p| self.nodes[p].2 + 1);
        self.nodes.push((text, Vec::new(), depth));
        match parent {
            Some(p) => self.nodes[p].1.push(i),
            None => self.roots.push(i),
        }
        i
    }

    fn depth(&self) -> usize {
        self.nodes.iter().map(|n| n.2).max().unwrap_or(0)
    }

    fn tree(&self, list: &[usize]) -> Vec<OutlineNode> {
        list.iter().map(|&i| OutlineNode { text: self.nodes[i].0.clone(), children: self.tree(&self.nodes[i].1) }).collect()
    }
}

/// A Markdown outline: headings nest by level, list items by indentation under the heading (or loose line) they
/// follow; a loose line of text heads the list after it; indented continuation lines join their item. Code fences,
/// rules, tables and quotes are skipped. Node text keeps its inline Markdown.
fn parse(markdown: &str) -> Arena {
    let mut a = Arena::default();
    // the heading / loose-line context: doubled level (a loose line sits half a level below its heading) → node
    let mut heads: Vec<(u32, usize)> = Vec::new();
    // list items of the current list: indentation → node
    let mut items: Vec<(usize, usize)> = Vec::new();
    let mut fence: Option<&str> = None;
    let head_parent = |heads: &mut Vec<(u32, usize)>, level: u32| {
        while heads.last().is_some_and(|h| h.0 >= level) {
            heads.pop();
        }
        heads.last().map(|h| h.1)
    };
    for raw in markdown.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        let line = raw.replace('\t', "    ");
        let trimmed = line.trim_matches(js_space);
        let marker = ["```", "~~~"].into_iter().find(|m| trimmed.starts_with(m));
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if marker.is_some() {
            fence = marker;
            continue;
        }
        if trimmed.is_empty() || is_rule(trimmed) || trimmed.starts_with('|') || trimmed.starts_with('>') {
            continue;
        }
        if let Some((level, text)) = heading(trimmed).filter(|_| line.starts_with('#')) {
            let parent = head_parent(&mut heads, level * 2);
            let node = a.push(clean(text), parent);
            heads.push((level * 2, node));
            items.clear();
            continue;
        }
        if let Some((indent, text)) = list_item(&line) {
            while items.last().is_some_and(|i| i.0 >= indent) {
                items.pop();
            }
            let parent = items.last().map(|i| i.1).or(heads.last().map(|h| h.1));
            let node = a.push(clean(strip_task(text)), parent);
            items.push((indent, node));
            continue;
        }
        let indent = line.chars().count() - line.trim_start_matches(js_space).chars().count();
        if let Some(&(last_indent, last)) = items.last() {
            if indent > last_indent {
                a.nodes[last].0 = clean(&format!("{} {trimmed}", a.nodes[last].0));
                continue;
            }
        }
        // a loose line heads what follows, one level below the current heading
        let level = heads.last().map_or(0, |h| h.0) + 1;
        let parent = head_parent(&mut heads, level);
        let node = a.push(clean(trimmed), parent);
        heads.push((level, node));
        items.clear();
    }
    a
}

pub fn parse_markdown_outline(markdown: &str) -> Vec<OutlineNode> {
    let a = parse(markdown);
    a.tree(&a.roots)
}

/// A nested list, two spaces per level.
pub fn format_markdown_outline(nodes: &[OutlineNode]) -> String {
    fn walk(n: &OutlineNode, depth: usize, out: &mut String) {
        out.push_str(&format!("{}- {}\n", "  ".repeat(depth), n.text.replace('\n', " ")));
        for c in &n.children {
            walk(c, depth + 1, out);
        }
    }
    let mut out = String::new();
    for n in nodes {
        walk(n, 0, &mut out);
    }
    out
}

/// A title the core accepts: at most 256 characters, cut with an ellipsis.
pub fn node_title(text: &str) -> String {
    if text.chars().count() <= MAX_TITLE_CHARS {
        return text.to_string();
    }
    let mut s: String = text.chars().take(MAX_TITLE_CHARS - 1).collect();
    s.push('…');
    s
}

pub fn count(nodes: &[OutlineNode]) -> usize {
    nodes.iter().map(|n| 1 + count(&n.children)).sum()
}

/// The children of a control result — a Markdown outline or `[{ text, children? }]` — as a bounded tree of
/// titles (trimmed, cut to 256 characters). An empty tree is an error.
pub fn result_tree(children: Option<&Value>) -> Result<Vec<OutlineNode>, String> {
    fn titled(n: &mut OutlineNode) {
        n.text = node_title(n.text.trim());
        n.children.iter_mut().for_each(titled);
    }
    fn from_array(list: &[Value], depth: usize, n: &mut usize) -> Result<Vec<OutlineNode>, String> {
        if depth > MAX_DEPTH {
            return Err(format!("nests deeper than {MAX_DEPTH} levels"));
        }
        let mut out = Vec::new();
        for v in list {
            *n += 1;
            if *n > MAX_NODES {
                return Err(format!("has more than {MAX_NODES} nodes"));
            }
            let Some(text) = v.get("text").and_then(Value::as_str) else { return Err("needs a text in every node ({ text, children? })".into()) };
            let children = match v.get("children") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => from_array(a, depth + 1, n)?,
                Some(_) => return Err("children of a node must be an array".into()),
            };
            out.push(OutlineNode { text: text.to_string(), children });
        }
        Ok(out)
    }
    let mut tree = match children {
        Some(Value::String(md)) => {
            let a = parse(md);
            if a.nodes.len() > MAX_NODES {
                return Err(format!("has more than {MAX_NODES} nodes"));
            }
            if a.depth() > MAX_DEPTH {
                return Err(format!("nests deeper than {MAX_DEPTH} levels"));
            }
            a.tree(&a.roots)
        }
        Some(Value::Array(list)) => from_array(list, 1, &mut 0)?,
        _ => return Err("needs children: a Markdown outline or [{ text, children }]".into()),
    };
    tree.iter_mut().for_each(titled);
    if tree.is_empty() {
        return Err("has no nodes (the outline is empty)".into());
    }
    Ok(tree)
}

/// A tree back from its JSON form (`to_json`).
pub fn tree_of(v: &Value) -> Vec<OutlineNode> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|n| OutlineNode { text: n["text"].as_str().unwrap_or("").to_string(), children: tree_of(&n["children"]) })
        .collect()
}

fn control_kind(e: &EntityRow) -> Option<String> {
    e.payload.get("kind").and_then(Value::as_str).filter(|k| is_control_kind(k)).map(str::to_string)
}

/// The control a Block input belongs to: a control container itself, or a Block laid out by one (a mind map
/// node). `None` for anything else.
pub fn control_of(ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<Option<String>> {
    Ok(match e.type_id.as_str() {
        TYPE_CONTAINER => control_kind(e),
        TYPE_CELL => enclosing_container(ctx, &e.entity_id)?.as_ref().and_then(control_kind),
        _ => None,
    })
}

/// A control input as an outline (all expanded): a control's nodes, or a node with its branch as the one root.
/// Text nodes read as their title, other Blocks as their title or `[view]`, a nested control as its title with
/// its own tree below. Branches under `skip` (what this wish made there last time) are left out; `visited` gets
/// every node read, in order.
pub fn read_tree(ctx: &dyn ReadCtx, can_read: &dyn Fn(&EntityRow) -> WsResult<bool>, e: &EntityRow, skip: &BTreeSet<String>, visited: &mut Vec<String>) -> WsResult<Vec<OutlineNode>> {
    fn node(ctx: &dyn ReadCtx, can_read: &dyn Fn(&EntityRow) -> WsResult<bool>, e: &EntityRow, skip: &BTreeSet<String>, depth: usize, visited: &mut Vec<String>) -> WsResult<OutlineNode> {
        if depth > MAX_READ_DEPTH {
            return Err(WsError::limit(format!("the branch nests deeper than {MAX_READ_DEPTH} levels: bind a node further down")));
        }
        visited.push(e.entity_id.clone());
        if visited.len() > MAX_NODES {
            return Err(WsError::limit(format!("the branch has more than {MAX_NODES} nodes: bind a node further down")));
        }
        let title = e.payload.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
        let (text, open) = match e.type_id.as_str() {
            TYPE_CONTAINER => match control_kind(e) {
                Some(k) => (title.unwrap_or(k), true),
                None => (title.unwrap_or_else(|| e.payload.get("kind").and_then(Value::as_str).unwrap_or("group").to_string()), false),
            },
            _ => (title.unwrap_or_else(|| format!("[{}]", e.payload.get("view").and_then(|v| v["type"].as_str()).unwrap_or("block"))), true),
        };
        let children = if open { kids(ctx, can_read, &e.entity_id, skip, depth + 1, visited)? } else { Vec::new() };
        Ok(OutlineNode { text, children })
    }
    fn kids(ctx: &dyn ReadCtx, can_read: &dyn Fn(&EntityRow) -> WsResult<bool>, id: &str, skip: &BTreeSet<String>, depth: usize, visited: &mut Vec<String>) -> WsResult<Vec<OutlineNode>> {
        let mut out = Vec::new();
        for edge in ctx.children(id)? {
            let Some(c) = ctx.entity(&edge.child_id)?.filter(|c| c.alive()) else { continue };
            if skip.contains(&c.entity_id) || !matches!(c.type_id.as_str(), TYPE_CELL | TYPE_CONTAINER) || !can_read(&c)? {
                continue;
            }
            out.push(node(ctx, can_read, &c, skip, depth, visited)?);
        }
        Ok(out)
    }
    if e.type_id == TYPE_CONTAINER {
        visited.push(e.entity_id.clone());
        kids(ctx, can_read, &e.entity_id, skip, 1, visited)
    } else {
        Ok(vec![node(ctx, can_read, e, skip, 1, visited)?])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../../../fixtures/mindmap/outlines.json");

    fn tree_json(nodes: &[OutlineNode]) -> Value {
        json!(nodes.iter().map(OutlineNode::to_json).collect::<Vec<_>>())
    }

    #[test]
    fn outline_cases_shared_with_the_typescript_parser() {
        let file: Value = serde_json::from_str(CASES).unwrap();
        let cases = file["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for c in cases {
            let got = parse_markdown_outline(c["markdown"].as_str().unwrap());
            assert_eq!(tree_json(&got), c["tree"], "{}", c["name"]);
        }
    }

    #[test]
    fn edge_cases_follow_the_typescript_regexes() {
        let t = |md: &str| tree_json(&parse_markdown_outline(md));
        // seven hashes are not a heading, closing hashes are dropped, an indented `#` is a loose line
        assert_eq!(t("####### x"), json!([{ "text": "####### x", "children": [] }]));
        assert_eq!(t("## C# ##\n- a"), json!([{ "text": "C#", "children": [{ "text": "a", "children": [] }] }]));
        assert_eq!(t("# C#"), json!([{ "text": "C", "children": [] }]));
        // ten digits are not a list marker; `1)` is
        assert_eq!(t("1234567890. x\n1) y"), json!([{ "text": "1234567890. x", "children": [{ "text": "y", "children": [] }] }]));
        // `* * *` is a rule, `**bold**` a loose line; an unclosed fence swallows the rest
        assert_eq!(t("* * *\n**bold**\n```\n- hidden"), json!([{ "text": "**bold**", "children": [] }]));
        // CRLF and full-width spaces
        assert_eq!(t("- a\r\n  - b\u{3000}c"), json!([{ "text": "a", "children": [{ "text": "b c", "children": [] }] }]));
    }

    #[test]
    fn result_trees_are_bounded_and_titled() {
        let long = "字".repeat(300);
        let tree = result_tree(Some(&json!(format!("- {long}\n  - b")))).unwrap();
        assert_eq!(tree[0].text.chars().count(), MAX_TITLE_CHARS);
        assert!(tree[0].text.ends_with('…'));
        let arr = result_tree(Some(&json!([{ "text": "  root ", "children": [{ "text": "x" }] }]))).unwrap();
        assert_eq!(format_markdown_outline(&arr), "- root\n  - x\n");
        assert!(result_tree(Some(&json!(""))).unwrap_err().contains("no nodes"));
        assert!(result_tree(Some(&json!([]))).is_err());
        assert!(result_tree(None).is_err());
        assert!(result_tree(Some(&json!([{ "children": [] }]))).is_err());
        let deep: String = (0..40).map(|i| format!("{}- n{i}\n", "  ".repeat(i))).collect();
        assert!(result_tree(Some(&json!(deep))).unwrap_err().contains("deeper"));
        let wide: String = (0..2001).map(|i| format!("- n{i}\n")).collect();
        assert!(result_tree(Some(&json!(wide))).unwrap_err().contains("2000"));
        let md = "- Plan\n  - **Market**\n    - [Interviews](https://example.com/a)\n  - Product\n";
        assert_eq!(format_markdown_outline(&parse_markdown_outline(md)), md);
    }
}
