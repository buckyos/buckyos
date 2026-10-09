//! Markdown (CommonMark + GFM tables, strikethrough, task lists) → canonical rich text AST
//! (`richtext.basic.v1`). Used for written wish results on the service and by the `aiws` Block
//! host in the browser, so both sides produce the same document from the same text.
//!
//! Links to other Workspace objects are written `[label](result:<name>)` / `[label](input:<name>)`
//! (or any `scheme:target` the caller resolves) and become `object_link` nodes; unresolved
//! links and disallowed URL schemes keep their text only. Block ids are `<prefix>-<n>`.

use crate::error::{WsError, WsResult};
use crate::richtext;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::{json, Map, Value};

/// Resolves `scheme:target` links (e.g. `result:monthly`) to a reference `{ entity_id, … }`.
pub type LinkResolver<'a> = &'a dyn Fn(&str) -> Option<Value>;

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Strong,
    Em,
    Strike,
    Code,
}

impl Mark {
    fn name(self) -> &'static str {
        match self {
            Mark::Strong => "strong",
            Mark::Em => "em",
            Mark::Strike => "strike",
            Mark::Code => "code",
        }
    }
}

enum Link {
    Href(String),
    Object(Value),
    Plain,
}

/// An open block while parsing.
struct Frame {
    ty: &'static str,
    attrs: Map<String, Value>,
    content: Vec<Value>,
}

struct Builder<'a> {
    prefix: String,
    n: usize,
    stack: Vec<Frame>,
    marks: Vec<Mark>,
    links: Vec<Link>,
    /// Pending object-link label (text between `[` and `]` of an object link).
    object_label: Option<String>,
    resolve: LinkResolver<'a>,
    in_table_head: bool,
}

fn allowed_href(h: &str) -> bool {
    ["http:", "https:", "mailto:", "#"].iter().any(|p| h.starts_with(p)) && !h.chars().any(|c| c.is_control())
}

impl<'a> Builder<'a> {
    fn id(&mut self) -> String {
        self.n += 1;
        format!("{}-{}", self.prefix, self.n)
    }

    fn open(&mut self, ty: &'static str, block_id: bool) {
        let mut attrs = Map::new();
        if block_id {
            attrs.insert("block_id".into(), json!(self.id()));
        }
        self.stack.push(Frame { ty, attrs, content: Vec::new() });
    }

    fn top(&mut self) -> &mut Frame {
        self.stack.last_mut().expect("doc frame")
    }

    /// Inline content may only go into a textblock: inside a list item or a quote it gets a paragraph.
    fn ensure_inline_container(&mut self) {
        if matches!(self.top().ty, "list_item" | "blockquote" | "doc") {
            self.open("paragraph", true);
        }
    }

    fn close(&mut self) {
        let f = self.stack.pop().expect("frame");
        let mut node = Map::new();
        node.insert("type".into(), json!(f.ty));
        if !f.attrs.is_empty() {
            node.insert("attrs".into(), Value::Object(f.attrs));
        }
        let mut content = f.content;
        if f.ty == "list_item" {
            content = fix_list_item(content, self);
        }
        if f.ty == "blockquote" && content.is_empty() {
            let id = self.id();
            content.push(json!({ "type": "paragraph", "attrs": { "block_id": id } }));
        }
        if !content.is_empty() {
            node.insert("content".into(), Value::Array(content));
        }
        self.top().content.push(Value::Object(node));
    }

    /// Close an implicit paragraph opened for loose inline content.
    fn close_implicit(&mut self) {
        if self.top().ty == "paragraph" && self.stack.len() >= 2 && matches!(self.stack[self.stack.len() - 2].ty, "list_item" | "blockquote" | "doc") {
            self.close();
        }
    }

    fn text(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        if let Some(label) = self.object_label.as_mut() {
            label.push_str(t);
            return;
        }
        self.ensure_inline_container();
        let in_code_block = self.top().ty == "code_block";
        let mut node = json!({ "type": "text", "text": t });
        if !in_code_block {
            let mut marks: Vec<Value> = Vec::new();
            if self.marks.contains(&Mark::Code) {
                // `code` excludes every other mark
                marks.push(json!({ "type": "code" }));
            } else {
                for m in &self.marks {
                    let v = json!({ "type": m.name() });
                    if !marks.contains(&v) {
                        marks.push(v);
                    }
                }
                if let Some(Link::Href(h)) = self.links.iter().rev().find(|l| !matches!(l, Link::Plain)) {
                    marks.push(json!({ "type": "link", "attrs": { "href": h } }));
                }
            }
            if !marks.is_empty() {
                node["marks"] = Value::Array(marks);
            }
        }
        self.top().content.push(node);
    }

    fn atom(&mut self, node: Value) {
        self.ensure_inline_container();
        self.top().content.push(node);
    }
}

/// A list item holds one paragraph, then paragraphs and lists only.
fn fix_list_item(content: Vec<Value>, b: &mut Builder) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for node in content {
        match node["type"].as_str().unwrap_or("") {
            "paragraph" | "bullet_list" | "ordered_list" => out.push(node),
            "heading" | "code_block" => {
                let mut p = node.clone();
                p["type"] = json!("paragraph");
                if let Some(a) = p.get_mut("attrs").and_then(Value::as_object_mut) {
                    a.retain(|k, _| k == "block_id");
                }
                out.push(p);
            }
            "blockquote" => out.extend(node["content"].as_array().cloned().unwrap_or_default().into_iter().filter(|c| c["type"] == json!("paragraph"))),
            _ => {}
        }
    }
    if out.first().map_or(true, |f| f["type"] != json!("paragraph")) {
        let id = b.id();
        out.insert(0, json!({ "type": "paragraph", "attrs": { "block_id": id } }));
    }
    out
}

/// Convert Markdown to a canonical, validated document. `prefix` must make valid block ids
/// (lowercase letters, digits, `_`, `-`).
pub fn to_richtext(markdown: &str, prefix: &str, resolve: LinkResolver) -> WsResult<Value> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let mut b = Builder {
        prefix: prefix.to_string(),
        n: 0,
        stack: vec![Frame { ty: "doc", attrs: Map::new(), content: Vec::new() }],
        marks: Vec::new(),
        links: Vec::new(),
        object_label: None,
        resolve,
        in_table_head: false,
    };
    for event in Parser::new_ext(markdown, opts) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if b.top().ty == "table_cell" || b.top().ty == "code_block" {
                        continue;
                    }
                    b.close_implicit();
                    b.open("paragraph", true);
                }
                Tag::Heading { level, .. } => {
                    b.close_implicit();
                    b.open("heading", true);
                    let l = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        _ => 3,
                    };
                    if l != 1 {
                        b.top().attrs.insert("level".into(), json!(l));
                    }
                }
                Tag::BlockQuote(_) => {
                    b.close_implicit();
                    b.open("blockquote", true);
                }
                Tag::CodeBlock(kind) => {
                    b.close_implicit();
                    b.open("code_block", true);
                    if let CodeBlockKind::Fenced(lang) = kind {
                        let lang = lang.split_whitespace().next().unwrap_or("").chars().take(32).collect::<String>();
                        if !lang.is_empty() {
                            b.top().attrs.insert("language".into(), json!(lang));
                        }
                    }
                }
                Tag::List(start) => {
                    b.close_implicit();
                    match start {
                        Some(n) => {
                            b.open("ordered_list", true);
                            if n != 1 {
                                b.top().attrs.insert("start".into(), json!(n.max(1)));
                            }
                        }
                        None => b.open("bullet_list", true),
                    }
                }
                Tag::Item => {
                    b.close_implicit();
                    b.open("list_item", true);
                }
                Tag::Table(_) => {
                    b.close_implicit();
                    b.open("table", true);
                }
                Tag::TableHead => {
                    b.in_table_head = true;
                    b.open("table_row", false);
                }
                Tag::TableRow => b.open("table_row", false),
                Tag::TableCell => {
                    b.open("table_cell", false);
                    if b.in_table_head {
                        b.top().attrs.insert("header".into(), json!(1));
                    }
                }
                Tag::Emphasis => b.marks.push(Mark::Em),
                Tag::Strong => b.marks.push(Mark::Strong),
                Tag::Strikethrough => b.marks.push(Mark::Strike),
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    let url = dest_url.to_string();
                    let link = match url.split_once(':') {
                        Some((scheme, target)) if !scheme.is_empty() && !matches!(scheme, "http" | "https" | "mailto") && !url.starts_with('#') => {
                            match (b.resolve)(&format!("{scheme}:{target}")) {
                                Some(r) => Link::Object(r),
                                None => Link::Plain,
                            }
                        }
                        _ if allowed_href(&url) => Link::Href(url),
                        _ => Link::Plain,
                    };
                    if matches!(link, Link::Object(_)) {
                        b.object_label = Some(String::new());
                    }
                    b.links.push(link);
                }
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => {
                    if b.top().ty == "paragraph" {
                        b.close();
                    }
                }
                TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::TableCell => b.close(),
                TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item | TagEnd::Table => {
                    b.close_implicit();
                    b.close();
                }
                TagEnd::TableHead => {
                    b.in_table_head = false;
                    b.close();
                }
                TagEnd::TableRow => b.close(),
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    b.marks.pop();
                }
                TagEnd::Link | TagEnd::Image => {
                    if let Some(Link::Object(r)) = b.links.pop() {
                        let label = b.object_label.take().unwrap_or_default();
                        b.atom(json!({ "type": "object_link", "attrs": { "ref": r, "label": label } }));
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if b.top().ty == "code_block" {
                    let t = t.strip_suffix('\n').map(str::to_string).unwrap_or_else(|| t.to_string());
                    // a code block keeps its lines as one text run
                    match b.top().content.last_mut() {
                        Some(last) => {
                            let joined = format!("{}\n{}", last["text"].as_str().unwrap_or(""), t);
                            last["text"] = json!(joined);
                        }
                        None => b.text(&t),
                    }
                } else {
                    b.text(&t);
                }
            }
            Event::Code(t) => {
                b.marks.push(Mark::Code);
                b.text(&t);
                b.marks.pop();
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => b.text(&t),
            Event::Html(t) | Event::InlineHtml(t) => {
                let t = t.trim_end_matches('\n');
                if !t.trim().is_empty() {
                    b.text(t);
                }
            }
            Event::SoftBreak => b.text(" "),
            Event::HardBreak => {
                if b.object_label.is_none() {
                    b.atom(json!({ "type": "hard_break" }));
                }
            }
            Event::Rule => {
                b.close_implicit();
                let id = b.id();
                b.top().content.push(json!({ "type": "horizontal_rule", "attrs": { "block_id": id } }));
            }
            Event::TaskListMarker(done) => b.text(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(t) => b.text(&format!("[{t}]")),
        }
    }
    while b.stack.len() > 1 {
        b.close();
    }
    let mut doc = b.stack.pop().expect("doc");
    if doc.content.is_empty() {
        doc.content.push(json!({ "type": "paragraph", "attrs": { "block_id": format!("{prefix}-1") } }));
    }
    let ast = json!({ "type": "doc", "content": doc.content });
    let canon = richtext::canonicalize(&ast).map_err(|e| WsError::invalid_schema(format!("markdown conversion: {}", e.detail)))?;
    richtext::validate_ast(&canon, &richtext::Limits::default())?;
    Ok(canon)
}

/// Plain Markdown of a document (headings, lists, quotes, code, tables) — what programs and
/// models read back (`content.md`, `aiws.input(name).markdown()`).
pub fn from_richtext(ast: &Value) -> String {
    let mut out = String::new();
    blocks(ast.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]), "", &mut out);
    out.trim_end().to_string() + "\n"
}

fn inline(nodes: &[Value]) -> String {
    let mut s = String::new();
    for n in nodes {
        match n["type"].as_str().unwrap_or("") {
            "text" => {
                let mut t = n["text"].as_str().unwrap_or("").to_string();
                for m in n.get("marks").and_then(Value::as_array).into_iter().flatten() {
                    t = match m["type"].as_str().unwrap_or("") {
                        "strong" => format!("**{t}**"),
                        "em" => format!("*{t}*"),
                        "strike" => format!("~~{t}~~"),
                        "code" => format!("`{t}`"),
                        "link" => format!("[{t}]({})", m["attrs"]["href"].as_str().unwrap_or("")),
                        _ => t,
                    };
                }
                s.push_str(&t);
            }
            "hard_break" => s.push_str("  \n"),
            "object_link" => {
                let label = n["attrs"]["label"].as_str().filter(|l| !l.is_empty()).unwrap_or("link");
                s.push_str(&format!("[{label}](entity:{})", n["attrs"]["ref"]["entity_id"].as_str().unwrap_or("")));
            }
            _ => {}
        }
    }
    s
}

fn kids(n: &Value) -> &[Value] {
    n.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn blocks(nodes: &[Value], indent: &str, out: &mut String) {
    for n in nodes {
        match n["type"].as_str().unwrap_or("") {
            "paragraph" => out.push_str(&format!("{indent}{}\n\n", inline(kids(n)))),
            "heading" => {
                let level = n["attrs"]["level"].as_u64().unwrap_or(1) as usize;
                out.push_str(&format!("{indent}{} {}\n\n", "#".repeat(level), inline(kids(n))));
            }
            "bullet_list" | "ordered_list" => {
                let ordered = n["type"] == json!("ordered_list");
                let start = n["attrs"]["start"].as_u64().unwrap_or(1);
                for (i, item) in kids(n).iter().enumerate() {
                    let marker = if ordered { format!("{}. ", start + i as u64) } else { "- ".to_string() };
                    let mut inner = String::new();
                    blocks(kids(item), "", &mut inner);
                    let pad = " ".repeat(marker.len());
                    for (j, line) in inner.trim_end().lines().enumerate() {
                        if j == 0 {
                            out.push_str(&format!("{indent}{marker}{line}\n"));
                        } else if line.is_empty() {
                            continue;
                        } else {
                            out.push_str(&format!("{indent}{pad}{line}\n"));
                        }
                    }
                }
                out.push('\n');
            }
            "blockquote" => {
                let mut inner = String::new();
                blocks(kids(n), "", &mut inner);
                for line in inner.trim_end().lines() {
                    out.push_str(&format!("{indent}> {line}\n"));
                }
                out.push('\n');
            }
            "code_block" => {
                let lang = n["attrs"]["language"].as_str().unwrap_or("");
                out.push_str(&format!("{indent}```{lang}\n{}\n{indent}```\n\n", inline(kids(n))));
            }
            "horizontal_rule" => out.push_str(&format!("{indent}---\n\n")),
            "table" => {
                for (i, row) in kids(n).iter().enumerate() {
                    let cells: Vec<String> = kids(row).iter().map(|c| inline(kids(c)).replace('|', "\\|")).collect();
                    out.push_str(&format!("{indent}| {} |\n", cells.join(" | ")));
                    if i == 0 {
                        out.push_str(&format!("{indent}|{}\n", " --- |".repeat(cells.len())));
                    }
                }
                out.push('\n');
            }
            "object_embed" => out.push_str(&format!("{indent}[embedded block {}]\n\n", n["attrs"]["ref"]["entity_id"].as_str().unwrap_or(""))),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<Value> {
        None
    }

    #[test]
    fn commonmark_and_gfm() {
        let md = "# 季度解读\n\n本季度销售额 **1,234.5**，环比 *+12%*，见 [月度汇总](result:monthly)。\n\n- 华东 `52%`\n- 华南\n  1. 深圳\n  2. 广州\n\n> 注意：已排除未结算订单\n\n```js\nconst a = 1\nconst b = 2\n```\n\n| 月份 | 销售额 |\n| --- | ---: |\n| 7 | 100 |\n\n---\n\n- [x] 已核对\n\n~~旧口径~~ [官网](https://example.com) [坏链接](javascript:alert(1))\n";
        let resolve = |s: &str| (s == "result:monthly").then(|| json!({ "entity_id": "res-monthly" }));
        let ast = to_richtext(md, "c1", &resolve).unwrap();
        let types: Vec<&str> = ast["content"].as_array().unwrap().iter().map(|n| n["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["heading", "paragraph", "bullet_list", "blockquote", "code_block", "table", "horizontal_rule", "bullet_list", "paragraph"]);
        let p = &ast["content"][1];
        assert!(p["content"].as_array().unwrap().iter().any(|n| n["type"] == json!("object_link") && n["attrs"]["ref"]["entity_id"] == json!("res-monthly") && n["attrs"]["label"] == json!("月度汇总")));
        assert!(p["content"].as_array().unwrap().iter().any(|n| n["marks"] == json!([{ "type": "strong" }]) && n["text"] == json!("1,234.5")));
        assert_eq!(ast["content"][4]["attrs"]["language"], json!("js"));
        assert_eq!(ast["content"][4]["content"][0]["text"], json!("const a = 1\nconst b = 2"));
        assert_eq!(ast["content"][5]["content"][0]["content"][0]["attrs"]["header"], json!(1));
        let last = serde_json::to_string(&ast["content"][8]).unwrap();
        assert!(last.contains("https://example.com") && !last.contains("javascript:") && last.contains("坏链接"));
        // the nested ordered list lives inside the second item, after its paragraph
        let item = &ast["content"][2]["content"][1];
        assert_eq!(item["content"][0]["type"], json!("paragraph"));
        assert_eq!(item["content"][1]["type"], json!("ordered_list"));
        // back to Markdown keeps the structure
        let back = from_richtext(&ast);
        assert!(back.contains("# 季度解读") && back.contains("| 月份 | 销售额 |") && back.contains("> 注意") && back.contains("```js"));
    }

    #[test]
    fn degenerate_inputs_stay_valid() {
        for md in ["", "   \n", "<div>x</div>", "- ", "> ", "1. a\n\n   ```\n   code\n   ```\n", "####### h7", "| a |\n|---|\n"] {
            let ast = to_richtext(md, "x", &none).unwrap_or_else(|e| panic!("{md:?}: {e:?}"));
            assert_eq!(ast["type"], json!("doc"));
        }
    }
}
