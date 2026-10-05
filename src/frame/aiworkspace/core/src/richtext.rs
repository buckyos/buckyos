//! RichText: ProseMirror document model on a Loro CRDT (design doc §3.3).
//!
//! The ProseMirror↔Loro container layout is part of the protocol
//! (`encoding = "pm-loro-1"`, the layout of loro-prosemirror 0.4.x):
//!
//! - root `LoroMap` named `"doc"`;
//! - every non-text node is a `LoroMap` { `nodeName`: string, `attributes`:
//!   `LoroMap` (null attrs omitted), `children`: `LoroList` };
//! - a run of consecutive text nodes is one `LoroText` in `children`; marks are
//!   text attributes `{ markName: attrsObject }`;
//! - inline atoms (`hard_break`, `object_link`) are separate `LoroMap`s in
//!   `children`, splitting the text run around them.

use crate::canonical::{canonical_json, sha256_hex};
use crate::error::{Code, WsError, WsResult};
use crate::id::is_valid_id;
use crate::model::{BlockIndex, BlockInfo};
use crate::value::normalize_reference;
use loro::{
    Container, ExpandType, ExportMode, LoroDoc, LoroList, LoroMap, LoroText, LoroValue, StyleConfig, StyleConfigMap,
    ValueOrContainer, VersionVector,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub const ENGINE: &str = "loro";
pub const ENCODING: &str = "pm-loro-1";
pub const EDITOR_SCHEMA: &str = "buckyos.richtext.basic.v1";
pub const SCHEMA_JSON: &str = include_str!("../../schemas/richtext.basic.v1.json");

pub fn engine_version() -> &'static str {
    loro::LORO_VERSION
}

pub fn schema() -> &'static Value {
    static S: OnceLock<Value> = OnceLock::new();
    S.get_or_init(|| serde_json::from_str(SCHEMA_JSON).expect("richtext schema"))
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_blocks: usize,
    pub max_text_utf16: usize,
    pub max_depth: usize,
    pub max_update_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        let l = &schema()["limits"];
        let n = |k: &str| l[k].as_u64().unwrap_or(0) as usize;
        Limits {
            max_blocks: n("max_blocks"),
            max_text_utf16: n("max_text_utf16"),
            max_depth: n("max_depth"),
            max_update_bytes: n("max_update_bytes"),
        }
    }
}

fn crdt_err(e: impl std::fmt::Display) -> WsError {
    WsError::invalid_op(format!("crdt: {e}"))
}

/// Mark expansion must be configured identically on every replica.
pub fn configure(doc: &LoroDoc) {
    let mut styles = StyleConfigMap::new();
    for (name, spec) in schema()["marks"].as_object().unwrap() {
        let expand = if spec["inclusive"] == json!(false) { ExpandType::None } else { ExpandType::After };
        styles.insert(name.as_str().into(), StyleConfig { expand });
    }
    doc.config_text_style(styles);
}

pub fn load_doc<'a>(snapshot: &[u8], updates: impl Iterator<Item = &'a [u8]>) -> WsResult<LoroDoc> {
    let doc = LoroDoc::new();
    configure(&doc);
    doc.import(snapshot).map_err(crdt_err)?;
    for u in updates {
        doc.import(u).map_err(crdt_err)?;
    }
    Ok(doc)
}

pub fn fork(doc: &LoroDoc) -> LoroDoc {
    let f = doc.fork();
    configure(&f);
    f
}

pub fn export_snapshot(doc: &LoroDoc) -> WsResult<Vec<u8>> {
    doc.commit();
    doc.export(ExportMode::Snapshot).map_err(crdt_err)
}

pub fn export_updates(doc: &LoroDoc, since: &VersionVector) -> WsResult<Vec<u8>> {
    doc.commit();
    doc.export(ExportMode::updates(since)).map_err(crdt_err)
}

/// Import an update into a candidate. `Ok(false)` = it depends on operations
/// this document does not have (`BASE_UNKNOWN`).
pub fn import_update(doc: &LoroDoc, update: &[u8]) -> WsResult<bool> {
    let status = doc.import(update).map_err(|e| WsError::invalid_op(format!("undecodable update: {e}")))?;
    Ok(status.pending.is_none())
}

// ---- LoroValue <-> JSON ----

fn lv_to_json(v: &LoroValue) -> Value {
    match v {
        LoroValue::Null => Value::Null,
        LoroValue::Bool(b) => json!(b),
        LoroValue::I64(i) => json!(i),
        LoroValue::Double(f) => {
            if f.fract() == 0.0 && f.abs() < 9.0e15 {
                json!(*f as i64)
            } else {
                serde_json::Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null)
            }
        }
        LoroValue::String(s) => json!(s.to_string()),
        LoroValue::List(l) => Value::Array(l.iter().map(lv_to_json).collect()),
        LoroValue::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.to_string(), lv_to_json(v))).collect()),
        LoroValue::Binary(_) | LoroValue::Container(_) => Value::Null,
    }
}

fn json_to_lv(v: &Value) -> LoroValue {
    match v {
        Value::Null => LoroValue::Null,
        Value::Bool(b) => LoroValue::from(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => LoroValue::from(i),
            None => LoroValue::from(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => LoroValue::from(s.as_str()),
        Value::Array(a) => LoroValue::from(a.iter().map(json_to_lv).collect::<Vec<_>>()),
        Value::Object(o) => {
            let m: std::collections::HashMap<String, LoroValue> = o.iter().map(|(k, v)| (k.clone(), json_to_lv(v))).collect();
            LoroValue::from(m)
        }
    }
}

// ---- schema helpers ----

fn node_spec(name: &str) -> Option<&'static Value> {
    schema()["nodes"].get(name)
}
fn has_block_id(name: &str) -> bool {
    node_spec(name).is_some_and(|s| s["block_id"] == json!(true))
}
fn in_group(name: &str, group: &str) -> bool {
    node_spec(name).is_some_and(|s| s["group"].as_str() == Some(group))
}
/// Container blocks hold child blocks (lists, list items); leaf blocks hold inline content.
pub fn is_container_block(name: &str) -> bool {
    matches!(name, "bullet_list" | "ordered_list" | "list_item")
}

fn matches_of(of: &Value, child: &str) -> bool {
    of.as_array().is_some_and(|a| {
        a.iter().filter_map(Value::as_str).any(|x| match x.strip_prefix('@') {
            Some(g) => in_group(child, g),
            None => x == child,
        })
    })
}

fn bad(detail: impl Into<String>) -> WsError {
    WsError::invalid_schema(detail)
}

/// Canonical attrs: schema defaults and nulls are omitted; references normalized.
fn canon_attrs(spec: &Value, attrs: Option<&Value>, node: &str) -> WsResult<Map<String, Value>> {
    let given = match attrs {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return Err(bad(format!("{node}: attrs must be an object"))),
    };
    let declared = spec.get("attrs").and_then(Value::as_object);
    let mut out = Map::new();
    for (k, v) in &given {
        if v.is_null() {
            continue;
        }
        if k == "block_id" {
            if !has_block_id(node) && spec.get("block_id").is_none() {
                return Err(bad(format!("{node}: unexpected block_id")));
            }
            let id = v.as_str().filter(|s| is_valid_id(s)).ok_or_else(|| bad(format!("{node}: invalid block_id")))?;
            out.insert(k.clone(), json!(id));
            continue;
        }
        let a = declared.and_then(|d| d.get(k)).ok_or_else(|| bad(format!("{node}: unknown attr {k}")))?;
        let val = match a["type"].as_str().unwrap_or("") {
            "integer" => {
                let n = v.as_i64().ok_or_else(|| bad(format!("{node}.{k}: expected integer")))?;
                if a["min"].as_i64().is_some_and(|m| n < m) || a["max"].as_i64().is_some_and(|m| n > m) {
                    return Err(bad(format!("{node}.{k}: out of range")));
                }
                json!(n)
            }
            "string" => json!(v.as_str().ok_or_else(|| bad(format!("{node}.{k}: expected string")))?),
            "reference" => normalize_reference(v)?,
            "href" => {
                let s = v.as_str().ok_or_else(|| bad("link.href: expected string"))?;
                let ok = a["schemes"].as_array().is_some_and(|x| x.iter().filter_map(Value::as_str).any(|p| s.starts_with(p)));
                if !ok || s.chars().any(|c| c.is_control()) {
                    return Err(bad("link.href: scheme not allowed"));
                }
                json!(s)
            }
            t => return Err(bad(format!("schema: unknown attr type {t}"))),
        };
        if a.get("default") != Some(&val) {
            out.insert(k.clone(), val);
        }
    }
    if let Some(d) = declared {
        for (k, a) in d {
            if a["required"] == json!(true) && !out.contains_key(k) {
                return Err(bad(format!("{node}: attr {k} required")));
            }
        }
    }
    Ok(out)
}

fn canon_marks(marks: Option<&Value>) -> WsResult<Vec<Value>> {
    let list = match marks {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(bad("marks must be an array")),
    };
    let mut by_name: BTreeMap<String, Value> = BTreeMap::new();
    for m in list {
        let name = m["type"].as_str().ok_or_else(|| bad("mark needs type"))?;
        let spec = schema()["marks"].get(name).ok_or_else(|| bad(format!("unknown mark {name}")))?;
        let attrs = canon_attrs(spec, m.get("attrs"), name)?;
        let mut o = Map::new();
        o.insert("type".into(), json!(name));
        if !attrs.is_empty() {
            o.insert("attrs".into(), Value::Object(attrs));
        }
        if by_name.insert(name.to_string(), Value::Object(o)).is_some() {
            return Err(bad(format!("duplicate mark {name}")));
        }
    }
    if by_name.len() > 1 {
        for name in by_name.keys() {
            if schema()["marks"][name]["exclusive"] == json!(true) {
                return Err(bad(format!("mark {name} excludes other marks")));
            }
        }
    }
    Ok(by_name.into_values().collect())
}

/// Bring any schema-valid AST into the canonical form used for hashing and
/// materialization: default attrs omitted, marks sorted by name, adjacent text
/// with equal marks merged, empty text and empty `content` dropped.
/// Fails on anything the schema does not allow.
pub fn canonicalize(node: &Value) -> WsResult<Value> {
    canon_node(node, 0, &Limits::default())
}

fn canon_node(node: &Value, depth: usize, limits: &Limits) -> WsResult<Value> {
    if depth > limits.max_depth + 1 {
        return Err(WsError::limit("rich text nesting too deep"));
    }
    let o = node.as_object().ok_or_else(|| bad("node must be an object"))?;
    let ty = o.get("type").and_then(Value::as_str).ok_or_else(|| bad("node needs type"))?;
    let spec = node_spec(ty).ok_or_else(|| bad(format!("unknown node type {ty}")))?;
    for k in o.keys() {
        if !matches!(k.as_str(), "type" | "attrs" | "content" | "marks" | "text") {
            return Err(bad(format!("{ty}: unknown key {k}")));
        }
    }
    let mut out = Map::new();
    out.insert("type".into(), json!(ty));
    if spec["text"] == json!(true) {
        let text = o.get("text").and_then(Value::as_str).ok_or_else(|| bad("text node needs text"))?;
        if o.contains_key("attrs") || o.contains_key("content") {
            return Err(bad("text node takes no attrs/content"));
        }
        let marks = canon_marks(o.get("marks"))?;
        if !marks.is_empty() {
            out.insert("marks".into(), Value::Array(marks));
        }
        out.insert("text".into(), json!(text));
        return Ok(Value::Object(out));
    }
    if o.contains_key("text") || o.get("marks").is_some_and(|m| m.as_array().map_or(true, |a| !a.is_empty())) {
        return Err(bad(format!("{ty}: text/marks not allowed")));
    }
    let attrs = canon_attrs(spec, o.get("attrs"), ty)?;
    if has_block_id(ty) && !attrs.contains_key("block_id") {
        return Err(bad(format!("{ty}: block_id required")));
    }
    if !attrs.is_empty() {
        out.insert("attrs".into(), Value::Object(attrs));
    }
    let children = match o.get("content") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(bad(format!("{ty}: content must be an array"))),
    };
    let mut canon: Vec<Value> = Vec::with_capacity(children.len());
    for c in &children {
        let c = canon_node(c, depth + 1, limits)?;
        if c["type"] == json!("text") {
            if c["text"].as_str() == Some("") {
                continue;
            }
            if let Some(prev) = canon.last_mut() {
                if prev["type"] == json!("text") && prev.get("marks") == c.get("marks") {
                    let merged = format!("{}{}", prev["text"].as_str().unwrap(), c["text"].as_str().unwrap());
                    prev["text"] = json!(merged);
                    continue;
                }
            }
        }
        canon.push(c);
    }
    // content model: a sequence of { of, min, max } items, matched greedily
    let empty = vec![];
    let items = spec.get("content").and_then(Value::as_array).unwrap_or(&empty);
    let mut i = 0;
    for item in items {
        let (min, max) = (item["min"].as_u64().unwrap_or(0), item["max"].as_u64().unwrap_or(u64::MAX));
        let mut n = 0;
        while i < canon.len() && n < max && matches_of(&item["of"], canon[i]["type"].as_str().unwrap_or("")) {
            i += 1;
            n += 1;
        }
        if n < min {
            return Err(bad(format!("{ty}: content does not match schema")));
        }
    }
    if i != canon.len() {
        return Err(bad(format!("{ty}: unexpected child {}", canon[i]["type"].as_str().unwrap_or("?"))));
    }
    if !canon.is_empty() {
        out.insert("content".into(), Value::Array(canon));
    }
    Ok(Value::Object(out))
}

pub fn block_id_of(node: &Value) -> Option<&str> {
    node.get("attrs")?.get("block_id")?.as_str()
}

pub fn block_hash(node: &Value) -> WsResult<String> {
    Ok(sha256_hex(canonical_json(node)?.as_bytes())[..32].to_string())
}

/// One reference found in a document.
#[derive(Debug, Clone, PartialEq)]
pub struct DocRef {
    pub block_id: String,
    /// `embed` (object_embed) or `link` (object_link)
    pub kind: &'static str,
    pub reference: Value,
}

#[derive(Debug, Clone, Default)]
pub struct Validated {
    /// `block_id → (parent block id or "", node type, content hash)`
    pub blocks: BTreeMap<String, (String, String, String)>,
    /// Child block ids per parent (`""` = top level), in document order.
    pub order: BTreeMap<String, Vec<String>>,
    pub refs: Vec<DocRef>,
    pub text_utf16: usize,
}

/// Validate a *canonical* AST against limits and block-id uniqueness.
pub fn validate_ast(ast: &Value, limits: &Limits) -> WsResult<Validated> {
    if ast["type"] != json!(schema()["top"]) {
        return Err(bad("top node must be doc"));
    }
    let mut v = Validated::default();
    walk(ast, "", 0, limits, &mut v)?;
    if v.blocks.len() > limits.max_blocks {
        return Err(WsError::limit(format!("more than {} blocks", limits.max_blocks)));
    }
    if v.text_utf16 > limits.max_text_utf16 {
        return Err(WsError::limit("rich text too long"));
    }
    Ok(v)
}

fn walk(node: &Value, parent_block: &str, depth: usize, limits: &Limits, v: &mut Validated) -> WsResult<()> {
    if depth > limits.max_depth {
        return Err(WsError::limit("rich text nesting too deep"));
    }
    let ty = node["type"].as_str().unwrap_or("");
    let mut scope = parent_block.to_string();
    if let Some(id) = block_id_of(node) {
        if v.blocks.insert(id.to_string(), (parent_block.to_string(), ty.to_string(), block_hash(node)?)).is_some() {
            return Err(bad(format!("duplicate block_id {id}")));
        }
        v.order.entry(parent_block.to_string()).or_default().push(id.to_string());
        scope = id.to_string();
    } else if has_block_id(ty) {
        return Err(bad(format!("{ty}: block_id required")));
    }
    match ty {
        "text" => v.text_utf16 += node["text"].as_str().map_or(0, |s| s.encode_utf16().count()),
        "object_embed" => v.refs.push(DocRef { block_id: scope.clone(), kind: "embed", reference: node["attrs"]["ref"].clone() }),
        "object_link" => v.refs.push(DocRef { block_id: scope.clone(), kind: "link", reference: node["attrs"]["ref"].clone() }),
        _ => {}
    }
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for c in children {
            walk(c, &scope, depth + 1, limits, v)?;
        }
    }
    Ok(())
}

/// Longest common subsequence membership of `b` relative to `a` (same element set).
fn lcs_keep(a: &[&String], b: &[&String]) -> BTreeSet<String> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut keep) = (0, 0, BTreeSet::new());
    while i < n && j < m {
        if a[i] == b[j] {
            keep.insert(a[i].clone());
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    keep
}

/// Blocks of `new` that kept their position relative to `old`: same parent and
/// part of the LCS of that parent's surviving children.
fn unmoved(old: &Validated, new: &Validated) -> BTreeSet<String> {
    let mut keep = BTreeSet::new();
    for (parent, new_children) in &new.order {
        let same_parent = |id: &&String, side: &Validated| side.blocks.get(*id).is_some_and(|b| &b.0 == parent);
        let b: Vec<&String> = new_children.iter().filter(|id| same_parent(id, old)).collect();
        let a: Vec<&String> =
            old.order.get(parent).map(|c| c.iter().filter(|id| same_parent(id, new)).collect()).unwrap_or_default();
        if a == b {
            keep.extend(b.into_iter().cloned());
        } else {
            keep.extend(lcs_keep(&a, &b));
        }
    }
    keep
}

/// New block index after a commit at `seq`: content hashes from the new AST,
/// `struct_rev` bumped only for new or moved blocks (design doc §3.3.3).
pub fn next_block_index(old_index: &BlockIndex, old: &Validated, new: &Validated, seq: u64) -> BlockIndex {
    let keep = unmoved(old, new);
    new.blocks
        .iter()
        .map(|(id, (parent, node_type, hash))| {
            let struct_rev = match old_index.get(id) {
                Some(o) if keep.contains(id) => o.struct_rev,
                _ => seq,
            };
            (id.clone(), BlockInfo { parent: parent.clone(), node_type: node_type.clone(), hash: hash.clone(), struct_rev })
        })
        .collect()
}

pub fn initial_block_index(v: &Validated, seq: u64) -> BlockIndex {
    next_block_index(&BlockIndex::new(), &Validated::default(), v, seq)
}

// ---- Loro containers <-> AST ----

fn as_map(v: Option<ValueOrContainer>) -> Option<LoroMap> {
    match v {
        Some(ValueOrContainer::Container(Container::Map(m))) => Some(m),
        _ => None,
    }
}
fn as_list(v: Option<ValueOrContainer>) -> Option<LoroList> {
    match v {
        Some(ValueOrContainer::Container(Container::List(l))) => Some(l),
        _ => None,
    }
}

fn node_name(map: &LoroMap) -> Option<String> {
    match map.get("nodeName") {
        Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

fn raw_node(map: &LoroMap) -> WsResult<Value> {
    let name = node_name(map).ok_or_else(|| bad("crdt node without nodeName"))?;
    let mut out = Map::new();
    out.insert("type".into(), json!(name));
    if let Some(attrs) = as_map(map.get("attributes")) {
        let a = lv_to_json(&attrs.get_deep_value());
        if a.as_object().is_some_and(|o| !o.is_empty()) {
            out.insert("attrs".into(), a);
        }
    }
    let mut content = Vec::new();
    if let Some(children) = as_list(map.get("children")) {
        for i in 0..children.len() {
            match children.get(i) {
                Some(ValueOrContainer::Container(Container::Map(m))) => content.push(raw_node(&m)?),
                Some(ValueOrContainer::Container(Container::Text(t))) => {
                    for run in lv_to_json(&t.get_richtext_value()).as_array().cloned().unwrap_or_default() {
                        let Some(text) = run["insert"].as_str() else { continue };
                        let mut n = Map::new();
                        n.insert("type".into(), json!("text"));
                        n.insert("text".into(), json!(text));
                        if let Some(attrs) = run.get("attributes").and_then(Value::as_object) {
                            let marks: Vec<Value> = attrs
                                .iter()
                                .filter(|(_, v)| !v.is_null())
                                .map(|(k, v)| match v.as_object() {
                                    Some(o) if !o.is_empty() => json!({ "type": k, "attrs": v }),
                                    _ => json!({ "type": k }),
                                })
                                .collect();
                            if !marks.is_empty() {
                                n.insert("marks".into(), Value::Array(marks));
                            }
                        }
                        content.push(Value::Object(n));
                    }
                }
                _ => return Err(bad("crdt children must be node maps or text")),
            }
        }
    }
    if !content.is_empty() {
        out.insert("content".into(), Value::Array(content));
    }
    Ok(Value::Object(out))
}

/// Containers → canonical AST. Fails if the document violates the schema.
pub fn decode_ast(doc: &LoroDoc) -> WsResult<Value> {
    canonicalize(&raw_node(&doc.get_map("doc"))?)
}

fn fill_node(map: &LoroMap, node: &Value) -> WsResult<()> {
    map.insert("nodeName", node["type"].as_str().unwrap_or("")).map_err(crdt_err)?;
    let attrs = map.insert_container("attributes", LoroMap::new()).map_err(crdt_err)?;
    let spec = node_spec(node["type"].as_str().unwrap_or(""));
    // defaults are materialized, like the editor binding writes node.attrs
    if let Some(decl) = spec.and_then(|s| s.get("attrs")).and_then(Value::as_object) {
        for (k, a) in decl {
            if let Some(d) = a.get("default") {
                if node["attrs"].get(k).is_none() {
                    attrs.insert(k, json_to_lv(d)).map_err(crdt_err)?;
                }
            }
        }
    }
    if let Some(a) = node.get("attrs").and_then(Value::as_object) {
        for (k, v) in a {
            attrs.insert(k, json_to_lv(v)).map_err(crdt_err)?;
        }
    }
    let children = map.insert_container("children", LoroList::new()).map_err(crdt_err)?;
    fill_children(&children, node.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]))
}

fn fill_children(list: &LoroList, content: &[Value]) -> WsResult<()> {
    let mut i = 0;
    while i < content.len() {
        if content[i]["type"] == json!("text") {
            let text = list.push_container(LoroText::new()).map_err(crdt_err)?;
            // Insert the whole run first, mark afterwards: text inserted right after an
            // already marked range would inherit its expanding marks (`ExpandType::After`).
            let mut pos = 0;
            let mut ranges: Vec<(usize, usize, &Value)> = Vec::new();
            while i < content.len() && content[i]["type"] == json!("text") {
                let s = content[i]["text"].as_str().unwrap_or("");
                let len = s.chars().count();
                text.insert(pos, s).map_err(crdt_err)?;
                if let Some(marks) = content[i].get("marks").and_then(Value::as_array) {
                    for m in marks {
                        ranges.push((pos, pos + len, m));
                    }
                }
                pos += len;
                i += 1;
            }
            for (from, to, m) in ranges {
                if from == to {
                    continue;
                }
                let attrs = m.get("attrs").cloned().unwrap_or_else(|| json!({}));
                text.mark(from..to, m["type"].as_str().unwrap_or(""), json_to_lv(&attrs)).map_err(crdt_err)?;
            }
        } else {
            let m = list.push_container(LoroMap::new()).map_err(crdt_err)?;
            fill_node(&m, &content[i])?;
            i += 1;
        }
    }
    Ok(())
}

/// Canonical AST → a document with a new lineage, authored by `peer`.
pub fn build_doc(ast: &Value, peer: u64) -> WsResult<LoroDoc> {
    let doc = LoroDoc::new();
    configure(&doc);
    doc.set_peer_id(peer).map_err(crdt_err)?;
    fill_node(&doc.get_map("doc"), ast)?;
    doc.commit();
    Ok(doc)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Position {
    After(String),
    Before(String),
    FirstChildOf(String),
    Start,
    End,
}

impl Position {
    pub fn parse(v: &Value) -> WsResult<Position> {
        let o = v.as_object().filter(|o| o.len() == 1).ok_or_else(|| WsError::invalid_op("position needs exactly one key"))?;
        let (k, val) = o.iter().next().unwrap();
        let id = || val.as_str().filter(|s| is_valid_id(s)).map(str::to_string).ok_or_else(|| WsError::invalid_op("position: bad block id"));
        Ok(match k.as_str() {
            "after" => Position::After(id()?),
            "before" => Position::Before(id()?),
            "first_child_of" => Position::FirstChildOf(id()?),
            "start" if val == &json!(true) => Position::Start,
            "end" if val == &json!(true) => Position::End,
            _ => return Err(WsError::invalid_op("unknown position")),
        })
    }
    pub fn to_json(&self) -> Value {
        match self {
            Position::After(b) => json!({ "after": b }),
            Position::Before(b) => json!({ "before": b }),
            Position::FirstChildOf(b) => json!({ "first_child_of": b }),
            Position::Start => json!({ "start": true }),
            Position::End => json!({ "end": true }),
        }
    }
    pub fn anchor(&self) -> Option<&str> {
        match self {
            Position::After(b) | Position::Before(b) | Position::FirstChildOf(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BlockOp {
    Insert { position: Position, blocks: Vec<Value> },
    Replace { block_id: String, node: Value },
    Delete { block_ids: Vec<String> },
    Move { block_id: String, position: Position },
}

fn children_of(map: &LoroMap) -> WsResult<LoroList> {
    as_list(map.get("children")).ok_or_else(|| bad("crdt node without children"))
}

fn map_block_id(map: &LoroMap) -> Option<String> {
    match as_map(map.get("attributes"))?.get("block_id") {
        Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

/// Locate a block: `(parent children list, index, node map)`.
fn find_block(parent: &LoroMap, id: &str) -> WsResult<Option<(LoroList, usize, LoroMap)>> {
    let list = children_of(parent)?;
    for i in 0..list.len() {
        if let Some(m) = as_map(list.get(i)) {
            if map_block_id(&m).as_deref() == Some(id) {
                return Ok(Some((list, i, m)));
            }
            if let Some(found) = find_block(&m, id)? {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

fn locate(doc: &LoroDoc, id: &str) -> WsResult<(LoroList, usize, LoroMap)> {
    find_block(&doc.get_map("doc"), id)?
        .ok_or_else(|| WsError::new(Code::ReferenceBroken, format!("block {id} not found")).with_data(json!({ "block_id": id })))
}

fn resolve(doc: &LoroDoc, pos: &Position) -> WsResult<(LoroList, usize)> {
    Ok(match pos {
        Position::Start => (children_of(&doc.get_map("doc"))?, 0),
        Position::End => {
            let l = children_of(&doc.get_map("doc"))?;
            let n = l.len();
            (l, n)
        }
        Position::After(b) => {
            let (l, i, _) = locate(doc, b)?;
            (l, i + 1)
        }
        Position::Before(b) => {
            let (l, i, _) = locate(doc, b)?;
            (l, i)
        }
        Position::FirstChildOf(b) => {
            let (_, _, m) = locate(doc, b)?;
            (children_of(&m)?, 0)
        }
    })
}

fn insert_at(list: &LoroList, index: usize, node: &Value) -> WsResult<()> {
    let m = list.insert_container(index, LoroMap::new()).map_err(crdt_err)?;
    fill_node(&m, node)
}

/// Execute block-level operations on a (candidate) document as local ops of
/// the document's current peer. Schema validity is checked by the caller on
/// the decoded result.
pub fn apply_block_ops(doc: &LoroDoc, ops: &[BlockOp]) -> WsResult<()> {
    for op in ops {
        match op {
            BlockOp::Insert { position, blocks } => {
                let (list, index) = resolve(doc, position)?;
                for (k, b) in blocks.iter().enumerate() {
                    insert_at(&list, index + k, b)?;
                }
            }
            BlockOp::Replace { block_id, node } => {
                let (list, index, _) = locate(doc, block_id)?;
                list.delete(index, 1).map_err(crdt_err)?;
                insert_at(&list, index, node)?;
            }
            BlockOp::Delete { block_ids } => {
                for id in block_ids {
                    // a block may already be gone with a deleted ancestor of the same op
                    if let Some((list, index, _)) = find_block(&doc.get_map("doc"), id)? {
                        list.delete(index, 1).map_err(crdt_err)?;
                    }
                }
            }
            BlockOp::Move { block_id, position } => {
                let (list, index, map) = locate(doc, block_id)?;
                let node = canonicalize(&raw_node(&map)?)?;
                if let Some(anchor) = position.anchor() {
                    if anchor == block_id || find_block(&map, anchor)?.is_some() {
                        return Err(WsError::invalid_op("cannot move a block relative to itself or its descendant"));
                    }
                }
                list.delete(index, 1).map_err(crdt_err)?;
                let (dst, at) = resolve(doc, position)?;
                insert_at(&dst, at, &node)?;
            }
        }
    }
    doc.commit();
    Ok(())
}

// ---- AST navigation used by the planner and by diff ----

#[derive(Debug, Clone)]
pub struct BlockLoc {
    pub parent: String,
    pub prev: Option<String>,
    pub node: Value,
}

/// Every block with its parent and previous sibling block.
pub fn index_blocks(ast: &Value) -> BTreeMap<String, BlockLoc> {
    fn rec(node: &Value, parent: &str, out: &mut BTreeMap<String, BlockLoc>) {
        let mut prev: Option<String> = None;
        for c in node.get("content").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = block_id_of(c) {
                out.insert(id.to_string(), BlockLoc { parent: parent.to_string(), prev: prev.clone(), node: c.clone() });
                prev = Some(id.to_string());
                rec(c, id, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    rec(ast, "", &mut out);
    out
}

/// Where a block sits, expressed without referring to the block itself.
pub fn position_of(loc: &BlockLoc) -> Position {
    match (&loc.prev, loc.parent.as_str()) {
        (Some(p), _) => Position::After(p.clone()),
        (None, "") => Position::Start,
        (None, parent) => Position::FirstChildOf(parent.to_string()),
    }
}

fn shallow(node: &Value) -> Value {
    json!({ "type": node["type"], "attrs": node.get("attrs") })
}

/// Block-level difference `base → target` as protocol operations for
/// `entity_id`, each carrying the expectations taken from `base_index`.
/// Deterministic; applying the result to `base` yields `target`.
pub fn diff_blocks(entity_id: &str, base: &Value, target: &Value, base_index: &BlockIndex) -> WsResult<Vec<Value>> {
    let b = index_blocks(base);
    let t = index_blocks(target);
    // survivors: present on both sides, not a re-typed container, with all ancestors surviving on both sides
    let mut alive: BTreeSet<String> = b
        .iter()
        .filter(|(id, bl)| {
            t.get(*id).is_some_and(|tl| {
                let ty = bl.node["type"].as_str().unwrap_or("");
                bl.node["type"] == tl.node["type"] && (!is_container_block(ty) || shallow(&bl.node) == shallow(&tl.node))
            })
        })
        .map(|(id, _)| id.clone())
        .collect();
    loop {
        let drop: Vec<String> = alive
            .iter()
            .filter(|id| {
                let (bp, tp) = (&b[*id].parent, &t[*id].parent);
                (!bp.is_empty() && !alive.contains(bp)) || (!tp.is_empty() && !alive.contains(tp))
            })
            .cloned()
            .collect();
        if drop.is_empty() {
            break;
        }
        for d in drop {
            alive.remove(&d);
        }
    }
    let expect = |id: &str, hash: bool, pos: bool| {
        let info = base_index.get(id);
        let mut e = Map::new();
        if hash {
            e.insert("hash".into(), json!(info.map(|i| i.hash.clone()).unwrap_or_default()));
        }
        if pos {
            e.insert("struct_rev".into(), json!(info.map_or(0, |i| i.struct_rev)));
        }
        Value::Object(e)
    };
    let mut ops = Vec::new();
    // 1. deletions (top-most only), in base document order
    let mut dels = Vec::new();
    fn order(node: &Value, out: &mut Vec<String>) {
        for c in node.get("content").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = block_id_of(c) {
                out.push(id.to_string());
                order(c, out);
            }
        }
    }
    let (mut base_order, mut target_order) = (Vec::new(), Vec::new());
    order(base, &mut base_order);
    order(target, &mut target_order);
    for id in &base_order {
        let p = &b[id].parent;
        if !alive.contains(id) && (p.is_empty() || alive.contains(p)) {
            dels.push(json!({ "block_id": id, "expect": expect(id, true, true) }));
        }
    }
    if !dels.is_empty() {
        ops.push(json!({ "op": "richtext.delete_blocks", "entity_id": entity_id, "blocks": dels }));
    }
    // 2. which survivors keep their place
    let mut keep = BTreeSet::new();
    let mut parents: BTreeSet<&str> = t.values().map(|l| l.parent.as_str()).collect();
    parents.insert("");
    for parent in parents {
        let side = |ord: &Vec<String>, idx: &BTreeMap<String, BlockLoc>, other: &BTreeMap<String, BlockLoc>| -> Vec<String> {
            ord.iter()
                .filter(|id| alive.contains(*id) && idx[*id].parent == parent && other[*id].parent == parent)
                .cloned()
                .collect()
        };
        let a = side(&base_order, &b, &t);
        let bb = side(&target_order, &t, &b);
        if a == bb {
            keep.extend(bb);
        } else {
            keep.extend(lcs_keep(&a.iter().collect::<Vec<_>>(), &bb.iter().collect::<Vec<_>>()));
        }
    }
    // 3. walk the target in document order: insert new subtrees, move displaced survivors, replace changed leaves
    for id in &target_order {
        let tl = &t[id];
        if !tl.parent.is_empty() && !alive.contains(&tl.parent) {
            continue; // arrives inside an inserted subtree
        }
        let position = position_of(tl).to_json();
        if !alive.contains(id) {
            ops.push(json!({ "op": "richtext.insert_blocks", "entity_id": entity_id, "position": position, "blocks": [tl.node] }));
            continue;
        }
        if !keep.contains(id) {
            ops.push(json!({ "op": "richtext.move_block", "entity_id": entity_id, "block_id": id,
                             "expect": expect(id, false, true), "position": position }));
        }
        let ty = tl.node["type"].as_str().unwrap_or("");
        if !is_container_block(ty) && b[id].node != tl.node {
            ops.push(json!({ "op": "richtext.replace_block", "entity_id": entity_id, "block_id": id,
                             "expect": expect(id, true, false), "node": tl.node }));
        }
    }
    Ok(ops)
}

/// The smallest valid document: one empty paragraph.
pub fn empty_ast(block_id: &str) -> Value {
    json!({ "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": block_id } }] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({ "type": "doc", "content": [
            { "type": "heading", "attrs": { "block_id": "h1", "level": 2 }, "content": [{ "type": "text", "text": "本周项目进展 😀" }] },
            { "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [
                { "type": "text", "text": "plain " },
                { "type": "text", "text": "bold", "marks": [{ "type": "strong" }] },
                { "type": "text", "text": "both", "marks": [{ "type": "strong" }, { "type": "em" }] },
                { "type": "hard_break" },
                { "type": "text", "text": "site", "marks": [{ "type": "link", "attrs": { "href": "https://example.com" } }] },
                { "type": "object_link", "attrs": { "ref": { "entity_id": "task-42-details" }, "label": "详情" } }
            ] },
            { "type": "bullet_list", "attrs": { "block_id": "l1" }, "content": [
                { "type": "list_item", "attrs": { "block_id": "li1" }, "content": [
                    { "type": "paragraph", "attrs": { "block_id": "li1p" }, "content": [{ "type": "text", "text": "一" }] },
                    { "type": "ordered_list", "attrs": { "block_id": "l2", "start": 3 }, "content": [
                        { "type": "list_item", "attrs": { "block_id": "li2" }, "content": [
                            { "type": "paragraph", "attrs": { "block_id": "li2p" } }] }] }] }] },
            { "type": "object_embed", "attrs": { "block_id": "e1", "ref": { "entity_id": "cell-open-tasks" } } },
            { "type": "paragraph", "attrs": { "block_id": "p2" } }
        ] })
    }

    #[test]
    fn canonical_roundtrip_through_crdt() {
        let ast = canonicalize(&sample()).unwrap();
        // canonical form: default attrs dropped, marks sorted, references normalized
        assert_eq!(ast["content"][1]["content"][2]["marks"], json!([{ "type": "em" }, { "type": "strong" }]));
        assert_eq!(ast["content"][3]["attrs"]["ref"], json!({ "entity_id": "cell-open-tasks", "version": { "mode": "live_head" } }));
        let h1 = canonicalize(&json!({ "type": "doc", "content": [
            { "type": "heading", "attrs": { "block_id": "h", "level": 1 } }] })).unwrap();
        assert!(h1["content"][0]["attrs"].get("level").is_none());

        let doc = build_doc(&ast, 7).unwrap();
        assert_eq!(decode_ast(&doc).unwrap(), ast);
        let restored = load_doc(&export_snapshot(&doc).unwrap(), std::iter::empty()).unwrap();
        assert_eq!(decode_ast(&restored).unwrap(), ast);
        let v = validate_ast(&ast, &Limits::default()).unwrap();
        assert_eq!(v.blocks.len(), 10);
        assert_eq!(v.order[""], vec!["h1", "p1", "l1", "e1", "p2"]);
        assert_eq!(v.refs.len(), 2);
        assert_eq!(v.blocks["li2p"].0, "li2");
    }

    /// Unmarked text that follows marked text keeps no marks after a build (the
    /// expanding marks of the previous run must not leak into it).
    #[test]
    fn build_keeps_mark_boundaries() {
        let ast = canonicalize(&json!({ "type": "doc", "content": [
            { "type": "paragraph", "attrs": { "block_id": "p" }, "content": [
                { "type": "text", "text": "第一期围绕" },
                { "type": "text", "text": "文档格式", "marks": [{ "type": "strong" }] },
                { "type": "text", "text": "、" },
                { "type": "text", "text": "Command Engine", "marks": [{ "type": "em" }, { "type": "strong" }] },
                { "type": "text", "text": " 与内置对象" },
                { "type": "text", "text": "code", "marks": [{ "type": "code" }] },
                { "type": "text", "text": "。" }
            ] }] })).unwrap();
        assert_eq!(ast["content"][0]["content"].as_array().unwrap().len(), 7);
        assert_eq!(decode_ast(&build_doc(&ast, 7).unwrap()).unwrap(), ast);
    }

    #[test]
    fn rejects_invalid() {
        let bad_docs = [
            json!({ "type": "doc" }),
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            json!({ "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "a" }, "content": [{ "type": "paragraph", "attrs": { "block_id": "b" } }] }] }),
            json!({ "type": "doc", "content": [{ "type": "heading", "attrs": { "block_id": "a", "level": 4 } }] }),
            json!({ "type": "doc", "content": [{ "type": "bullet_list", "attrs": { "block_id": "a" } }] }),
            json!({ "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "a" }, "content": [
                { "type": "text", "text": "x", "marks": [{ "type": "code" }, { "type": "strong" }] }] }] }),
            json!({ "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "a" }, "content": [
                { "type": "text", "text": "x", "marks": [{ "type": "link", "attrs": { "href": "javascript:alert(1)" } }] }] }] }),
            json!({ "type": "doc", "content": [{ "type": "video", "attrs": { "block_id": "a" } }] }),
        ];
        for d in bad_docs {
            assert!(canonicalize(&d).is_err(), "{d}");
        }
        let dup = canonicalize(&json!({ "type": "doc", "content": [
            { "type": "paragraph", "attrs": { "block_id": "a" } }, { "type": "paragraph", "attrs": { "block_id": "a" } }] })).unwrap();
        assert!(validate_ast(&dup, &Limits::default()).is_err());
    }

    #[test]
    fn concurrent_updates_converge_in_any_order() {
        let base = build_doc(&canonicalize(&sample()).unwrap(), 1).unwrap();
        let snap = export_snapshot(&base).unwrap();
        let edit = |peer: u64, block: &str, text: &str| -> Vec<u8> {
            let d = load_doc(&snap, std::iter::empty()).unwrap();
            d.set_peer_id(peer).unwrap();
            let vv = d.oplog_vv();
            apply_block_ops(&d, &[BlockOp::Insert {
                position: Position::After(block.into()),
                blocks: vec![json!({ "type": "paragraph", "attrs": { "block_id": format!("n{peer}") }, "content": [{ "type": "text", "text": text }] })],
            }]).unwrap();
            export_updates(&d, &vv).unwrap()
        };
        let (ua, ub) = (edit(11, "h1", "from a"), edit(12, "h1", "from b"));
        let apply = |order: &[&[u8]]| {
            let d = load_doc(&snap, std::iter::empty()).unwrap();
            for u in order {
                assert!(import_update(&d, u).unwrap());
            }
            decode_ast(&d).unwrap()
        };
        let x = apply(&[&ua, &ub]);
        assert_eq!(x, apply(&[&ub, &ua]));
        assert_eq!(x, apply(&[&ua, &ub, &ua]), "duplicate delivery is idempotent");
        assert_eq!(validate_ast(&x, &Limits::default()).unwrap().blocks.len(), 12);
        // an update that depends on unseen operations is reported, not half-applied
        let d2 = load_doc(&snap, std::iter::empty()).unwrap();
        d2.set_peer_id(13).unwrap();
        assert!(import_update(&d2, &ua).unwrap());
        let vv = d2.oplog_vv();
        apply_block_ops(&d2, &[BlockOp::Delete { block_ids: vec!["n11".into()] }]).unwrap();
        let dependent = export_updates(&d2, &vv).unwrap();
        let fresh = load_doc(&snap, std::iter::empty()).unwrap();
        assert!(!import_update(&fresh, &dependent).unwrap());
        assert_eq!(decode_ast(&fresh).unwrap(), canonicalize(&sample()).unwrap());
    }

    #[test]
    fn fork_isolates_candidate() {
        let doc = build_doc(&canonicalize(&sample()).unwrap(), 1).unwrap();
        let before = export_snapshot(&doc).unwrap();
        let cand = fork(&doc);
        apply_block_ops(&cand, &[BlockOp::Delete { block_ids: vec!["p1".into(), "l1".into()] }]).unwrap();
        assert_ne!(decode_ast(&cand).unwrap(), decode_ast(&doc).unwrap());
        assert_eq!(export_snapshot(&doc).unwrap(), before, "authoritative document untouched");
    }

    fn apply_protocol_ops(base: &Value, ops: &[Value]) -> Value {
        let doc = build_doc(base, 1).unwrap();
        for op in ops {
            let bop = match op["op"].as_str().unwrap() {
                "richtext.insert_blocks" => BlockOp::Insert {
                    position: Position::parse(&op["position"]).unwrap(),
                    blocks: op["blocks"].as_array().unwrap().clone(),
                },
                "richtext.replace_block" => BlockOp::Replace { block_id: op["block_id"].as_str().unwrap().into(), node: op["node"].clone() },
                "richtext.delete_blocks" => BlockOp::Delete {
                    block_ids: op["blocks"].as_array().unwrap().iter().map(|b| b["block_id"].as_str().unwrap().to_string()).collect(),
                },
                "richtext.move_block" => BlockOp::Move {
                    block_id: op["block_id"].as_str().unwrap().into(),
                    position: Position::parse(&op["position"]).unwrap(),
                },
                o => panic!("{o}"),
            };
            apply_block_ops(&doc, &[bop]).unwrap();
        }
        decode_ast(&doc).unwrap()
    }

    #[test]
    fn diff_roundtrip() {
        let base = canonicalize(&sample()).unwrap();
        let p = |id: &str, text: &str| json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] });
        let targets = [
            // edit a leaf, delete one, add two, reorder
            json!({ "type": "doc", "content": [
                p("new1", "inserted first"),
                base["content"][4].clone(),
                { "type": "heading", "attrs": { "block_id": "h1", "level": 3 }, "content": [{ "type": "text", "text": "改过的标题" }] },
                base["content"][2].clone(),
                p("new2", "tail") ] }),
            // nested: change list item text, add an item, retype the inner list
            json!({ "type": "doc", "content": [
                base["content"][0].clone(),
                { "type": "bullet_list", "attrs": { "block_id": "l1" }, "content": [
                    { "type": "list_item", "attrs": { "block_id": "li9" }, "content": [p("li9p", "new item")] },
                    { "type": "list_item", "attrs": { "block_id": "li1" }, "content": [
                        p("li1p", "一改"),
                        { "type": "bullet_list", "attrs": { "block_id": "l2" }, "content": [
                            { "type": "list_item", "attrs": { "block_id": "li2" }, "content": [p("li2p", "x")] }] }] }] },
                base["content"][1].clone() ] }),
            base.clone(),
        ];
        let index = initial_block_index(&validate_ast(&base, &Limits::default()).unwrap(), 5);
        for t in targets {
            let t = canonicalize(&t).unwrap();
            let ops = diff_blocks("rt", &base, &t, &index).unwrap();
            assert_eq!(apply_protocol_ops(&base, &ops), t, "{}", serde_json::to_string_pretty(&ops).unwrap());
            if t == base {
                assert!(ops.is_empty());
            }
        }
    }

    #[test]
    fn struct_rev_tracks_moves_only() {
        let base = canonicalize(&sample()).unwrap();
        let v0 = validate_ast(&base, &Limits::default()).unwrap();
        let i0 = initial_block_index(&v0, 5);
        // insert next to p1, edit h1, move p2 to the front
        let mut c = base["content"].as_array().unwrap().clone();
        let p2 = c.pop().unwrap();
        c.insert(0, p2);
        c.insert(3, json!({ "type": "paragraph", "attrs": { "block_id": "nn" } }));
        c[1]["content"] = json!([{ "type": "text", "text": "edited" }]);
        let next = canonicalize(&json!({ "type": "doc", "content": c })).unwrap();
        let v1 = validate_ast(&next, &Limits::default()).unwrap();
        let i1 = next_block_index(&i0, &v0, &v1, 9);
        assert_eq!(i1["p2"].struct_rev, 9, "moved");
        assert_eq!(i1["nn"].struct_rev, 9, "new");
        for id in ["h1", "p1", "l1", "e1", "li1", "li2p"] {
            assert_eq!(i1[id].struct_rev, 5, "{id} did not move");
        }
        assert_ne!(i1["h1"].hash, i0["h1"].hash);
        assert_eq!(i1["p1"].hash, i0["p1"].hash);
    }
}
