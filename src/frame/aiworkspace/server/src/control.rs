//! Block-controls over kRPC (脑图方案 §8.1, TODO M3): the catalogue each control describes itself in
//! (`aiws/controls.json`, generated from `controls/catalog.ts`) and its exchange formats — a mind map from and to a
//! Markdown outline or OPML. `buckyos aiws controls | import | export` call these; the service dispatches by control
//! kind (the outline parser is the one wish `control` results use, `store/src/wish/outline.rs`).

use aiworkspace_core::error::{WsError, WsResult};
use aiworkspace_core::model::PROTOCOL_VERSION;
use aiworkspace_core::order_key::order_key_between;
use aiworkspace_core::types::is_control_kind;
use aiworkspace_store::wish::outline::{count, format_markdown_outline, result_tree, OutlineNode};
use aiworkspace_store::workspace::{random_id, Workspace};
use aiworkspace_store::Caller;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// `control.list`: what each control is and when to use it (the self-descriptions stay with `control.describe`).
pub fn list() -> Value {
    let catalog = crate::wish::controls_catalog();
    let controls: Vec<Value> = catalog["controls"].as_array().into_iter().flatten().map(|c| {
        let mut c = c.clone();
        if let Some(o) = c.as_object_mut() {
            o.remove("agent");
        }
        c
    }).collect();
    json!({ "ok": true, "controls": controls })
}

/// `control.describe { kind }`: the whole catalogue entry, with the self-description an Agent follows.
pub fn describe(kind: &str) -> WsResult<Value> {
    let catalog = crate::wish::controls_catalog();
    let control = catalog["controls"].as_array().into_iter().flatten().find(|c| c["kind"] == kind).cloned()
        .ok_or_else(|| WsError::not_found(format!("no control {kind:?} (control.list names them)")))?;
    Ok(json!({ "ok": true, "control": control }))
}

fn format_of(p: &Value) -> WsResult<&str> {
    match p.get("format").and_then(Value::as_str).unwrap_or("markdown") {
        f @ ("markdown" | "opml") => Ok(f),
        f => Err(WsError::invalid_op(format!("format {f:?}: markdown or opml"))),
    }
}

/// `control.export { entity_id, format? }`: a control's Blocks as an outline (all expanded; a nested control is a node
/// with its own tree, another Block its title or `[view]`).
pub fn export(ws: &Workspace, caller: &Caller, p: &Value) -> WsResult<Value> {
    let id = p.get("entity_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("entity_id required"))?;
    let format = format_of(p)?;
    let outline = ws.outline(caller)?;
    let entities = outline["entities"].as_array().cloned().unwrap_or_default();
    let container = entities.iter().find(|e| e["entity_id"] == id).ok_or_else(|| WsError::not_found(format!("entity {id} not found")))?;
    let kind = container["kind"].as_str().filter(|k| is_control_kind(k)).ok_or_else(|| WsError::invalid_op(format!("{id} is not a control")))?.to_string();
    // the outline lists children in sibling order after their parent
    let mut children: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for e in &entities {
        if let Some(parent) = e["parent_id"].as_str() {
            children.entry(parent.to_string()).or_default().push(e);
        }
    }
    fn read(children: &BTreeMap<String, Vec<&Value>>, parent: &str, depth: usize) -> Vec<OutlineNode> {
        if depth > 64 {
            return Vec::new();
        }
        children.get(parent).into_iter().flatten().filter(|e| matches!(e["type_id"].as_str(), Some("buckyos.cell" | "buckyos.container"))).map(|e| {
            let title = e["title"].as_str().filter(|t| !t.is_empty());
            let nested = e["type_id"] == "buckyos.container" && e["kind"].as_str().is_some_and(is_control_kind);
            let text = title.map(str::to_string).unwrap_or_else(|| format!("[{}]", e["view_type"].as_str().or(e["kind"].as_str()).unwrap_or("block")));
            let id = e["entity_id"].as_str().unwrap_or("");
            OutlineNode { text, children: if e["type_id"] == "buckyos.cell" || nested { read(children, id, depth + 1) } else { Vec::new() } }
        }).collect()
    }
    let nodes = read(&children, id, 0);
    let title = container["title"].as_str().unwrap_or("");
    let text = if format == "opml" { format_opml(&nodes, title) } else { format_markdown_outline(&nodes) };
    Ok(json!({ "ok": true, "kind": kind, "format": format, "nodes": count(&nodes), "text": text }))
}

/// `control.import { parent_id, kind?, format?, text, at?, title?, config?, idempotency_key? }`: a new control holding
/// the outline, committed as the caller (one commit); its Blocks are laid out by the control ("auto" placement).
pub fn import(ws: &mut Workspace, caller: &Caller, p: &Value) -> WsResult<Value> {
    let parent = p.get("parent_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("parent_id required"))?;
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("mindmap");
    if kind != "mindmap" {
        return Err(WsError::invalid_op(format!("control {kind:?} has no outline import (control.list: formats)")));
    }
    let text = p.get("text").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("text required"))?;
    let (nodes, opml_title) = match format_of(p)? {
        "opml" => {
            let (title, nodes) = parse_opml(text).map_err(WsError::invalid_schema)?;
            (Value::Array(nodes.iter().map(OutlineNode::to_json).collect()), title)
        }
        _ => (Value::String(text.to_string()), None),
    };
    let tree = result_tree(Some(&nodes)).map_err(WsError::invalid_schema)?;
    let title = p.get("title").and_then(Value::as_str).map(str::to_string).or(opml_title).or_else(|| (tree.len() == 1).then(|| tree[0].text.clone()));
    let container = random_id("m");
    let last = ws.outline(caller)?["entities"].as_array().into_iter().flatten().filter(|e| e["parent_id"] == parent).filter_map(|e| e["order_key"].as_str().map(str::to_string)).max();
    let mut payload = Map::new();
    payload.insert("kind".into(), json!(kind));
    payload.insert("layout".into(), json!({ "mode": "tree" }));
    payload.insert("config".into(), p.get("config").filter(|c| c.is_object()).cloned().unwrap_or(json!({ "direction": "right", "edge": "curve" })));
    if let Some(t) = &title {
        payload.insert("title".into(), json!(aiworkspace_store::wish::outline::node_title(t)));
    }
    let mut create = json!({ "op": "entity.create", "entity_id": container, "type_id": "buckyos.container", "parent_id": parent,
                             "order_key": order_key_between(last.as_deref(), None)?, "payload": payload });
    if let Some(at) = p.get("at").filter(|a| !a.is_null()) {
        let (x, y) = (at["x"].as_f64(), at["y"].as_f64());
        let (Some(x), Some(y)) = (x, y) else { return Err(WsError::invalid_op("at must be { x, y }")) };
        create["placement"] = json!({ "x": x.round(), "y": y.round(), "w": "auto", "h": "auto" });
    }
    let mut ops = vec![create];
    fn nodes_of(list: &[OutlineNode], parent: &str, ops: &mut Vec<Value>) -> WsResult<()> {
        let mut key: Option<String> = None;
        for n in list {
            let id = random_id("n");
            let k = order_key_between(key.as_deref(), None)?;
            ops.push(json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.cell", "parent_id": parent, "order_key": k,
                             "placement": { "x": "auto", "y": "auto", "w": "auto", "h": "auto" }, "payload": { "view": { "type": "mindnode" }, "title": n.text } }));
            nodes_of(&n.children, &id, ops)?;
            key = Some(k);
        }
        Ok(())
    }
    nodes_of(&tree, &container, &mut ops)?;
    let key = p.get("idempotency_key").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| random_id("control-import/"));
    let req = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": ws.workspace_id, "epoch": ws.epoch, "idempotency_key": key, "operations": ops });
    let mut outcome = ws.commit(&req, caller, &Default::default());
    outcome["container_id"] = json!(container);
    outcome["nodes"] = json!(count(&tree));
    Ok(outcome)
}

// ---- OPML (the outliners' format: `<outline text>` nested under `<body>`)

fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 12) else { out.push('&'); rest = &rest[1..]; continue };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => { out.push(c); rest = &rest[end + 1..]; }
            None => { out.push('&'); rest = &rest[1..]; }
        }
    }
    out.push_str(rest);
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().last();
        let after = rest[i + name.len()..].trim_start();
        if before.is_some_and(char::is_whitespace) && after.starts_with('=') {
            let v = after[1..].trim_start();
            let q = v.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let end = v[1..].find(q)?;
            return Some(decode(&v[1..1 + end]));
        }
        rest = &rest[i + name.len()..];
    }
    None
}

/// The head title and the outline of an OPML document.
pub fn parse_opml(src: &str) -> Result<(Option<String>, Vec<OutlineNode>), String> {
    let mut stack: Vec<OutlineNode> = vec![OutlineNode::default()];
    let (mut opml, mut body, mut title, mut in_title) = (false, false, None, false);
    let mut rest = src;
    while let Some(lt) = rest.find('<') {
        if in_title {
            title = Some(decode(rest[..lt].trim()));
            in_title = false;
        }
        rest = &rest[lt..];
        let skip = |end: &str, r: &str| r.find(end).map(|i| i + end.len());
        let close = if rest.starts_with("<!--") { skip("-->", rest) } else if rest.starts_with("<?") { skip("?>", rest) } else { skip(">", rest) };
        let Some(close) = close else { return Err("unterminated tag".into()) };
        let tag = &rest[1..close - 1];
        rest = &rest[close..];
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| !c.is_whitespace() && *c != '/').collect();
        match (tag.starts_with('/'), name.as_str()) {
            (false, "opml") => opml = true,
            (false, "body") => body = true,
            (false, "title") if !body && !tag.ends_with('/') => in_title = true,
            (false, "outline") if body => {
                let text = attr(tag, "text").or_else(|| attr(tag, "title")).unwrap_or_default();
                let node = OutlineNode { text: text.split_whitespace().collect::<Vec<_>>().join(" "), children: Vec::new() };
                if tag.ends_with('/') {
                    stack.last_mut().unwrap().children.push(node);
                } else {
                    stack.push(node);
                }
            }
            (true, "outline") if stack.len() > 1 => {
                let done = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(done);
            }
            _ => {}
        }
    }
    if !opml || !body {
        return Err("not an OPML document (no <opml><body>)".into());
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(done);
    }
    Ok((title.filter(|t: &String| !t.is_empty()), stack.pop().unwrap().children))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "&#10;")
}

/// An OPML 2.0 document of `nodes` (what `controls/opml.ts` writes).
pub fn format_opml(nodes: &[OutlineNode], title: &str) -> String {
    fn walk(n: &OutlineNode, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent);
        if n.children.is_empty() {
            out.push_str(&format!("{pad}<outline text=\"{}\"/>\n", esc(&n.text)));
            return;
        }
        out.push_str(&format!("{pad}<outline text=\"{}\">\n", esc(&n.text)));
        for c in &n.children {
            walk(c, indent + 1, out);
        }
        out.push_str(&format!("{pad}</outline>\n"));
    }
    let mut body = String::new();
    for n in nodes {
        walk(n, 2, &mut body);
    }
    let head = if title.is_empty() { "    <title/>".to_string() } else { format!("    <title>{}</title>", esc(title)) };
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head>\n{head}\n  </head>\n  <body>\n{body}  </body>\n</opml>\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opml_round_trip() {
        let nodes = vec![OutlineNode { text: "Root & <b>".into(), children: vec![OutlineNode { text: "A \"q\"".into(), children: vec![] }, OutlineNode { text: "B".into(), children: vec![OutlineNode { text: "B1".into(), children: vec![] }] }] }];
        let xml = format_opml(&nodes, "Plan");
        assert_eq!(parse_opml(&xml).unwrap(), (Some("Plan".to_string()), nodes));
        assert!(parse_opml("<html><body/></html>").is_err());
        // what other outliners write: a DOCTYPE, comments, `title` instead of `text`, single quotes
        let other = "<?xml version='1.0'?><!-- x --><opml version='1.0'><head><title>T</title></head><body><outline title='One'><outline text='Two &amp; three'/></outline></body></opml>";
        let (title, tree) = parse_opml(other).unwrap();
        assert_eq!(title.as_deref(), Some("T"));
        assert_eq!(tree[0].text, "One");
        assert_eq!(tree[0].children[0].text, "Two & three");
    }

    #[test]
    fn the_catalogue_lists_and_describes_each_control() {
        let listed = list();
        assert!(listed["controls"].as_array().unwrap().iter().any(|c| c["kind"] == "mindmap" && c.get("agent").is_none()));
        assert!(describe("mindmap").unwrap()["control"]["agent"].as_str().unwrap().contains("doc.commit"));
        assert!(describe("carousel").is_err());
    }
}
