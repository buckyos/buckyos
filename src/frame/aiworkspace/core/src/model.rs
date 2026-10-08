//! Working-state rows, the read view (`ReadCtx`) and the candidate overlay
//! (design doc §2.1, §2.5.2). Type adapters only *return* writes into the
//! overlay; nothing in this crate touches storage.

use crate::error::WsResult;
use crate::value::FieldDef;
use loro::LoroDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub type JsonMap = Map<String, Value>;

pub const TYPE_CONTAINER: &str = "buckyos.container";
pub const TYPE_RECORD: &str = "buckyos.record";
pub const TYPE_RICHTEXT: &str = "buckyos.richtext";
pub const TYPE_TABLE: &str = "buckyos.table-source";
pub const TYPE_CELL: &str = "buckyos.cell";
pub const TYPE_ASSET: &str = "buckyos.asset-ref";
pub const TYPE_ANNOTATION: &str = "buckyos.annotation";
/// A wish cell: what one AI inference run needs (phase two §7).
pub const TYPE_WISH: &str = "buckyos.wish";
/// A Block definition saved as a document entity (declarative or HTML; phase two §10.3).
pub const TYPE_BLOCK_DEF: &str = "buckyos.block-def";
pub const ROOT_ID: &str = "root";
/// System nodes of the two trees (phase two §4): the data tree root, the Surface collection and the
/// canvas content area (a system folder under `data` holding one folder per Surface).
pub const DATA_ID: &str = "data";
pub const SURFACES_ID: &str = "surfaces";
pub const CANVAS_CONTENT_ID: &str = "canvas-content";
pub const SCOPE_SHARED: &str = "shared";
pub const POLICY_OPEN: &str = "open";
pub const POLICY_LOCK: &str = "lock_required";
/// Reserved key of `key_revs` holding `source.members_rev`.
pub const MEMBERS_KEY: &str = "#members";
pub const FORMAT_VERSION: &str = "0.3";
pub const PROTOCOL_VERSION: &str = "0.4";

pub fn is_known_type(t: &str) -> bool {
    matches!(t, TYPE_CONTAINER | TYPE_RECORD | TYPE_RICHTEXT | TYPE_TABLE | TYPE_CELL | TYPE_ASSET | TYPE_ANNOTATION | TYPE_WISH | TYPE_BLOCK_DEF)
}

/// Types that live in the data tree (everything that is neither a container nor a Block).
pub fn is_data_type(t: &str) -> bool {
    matches!(t, TYPE_RECORD | TYPE_RICHTEXT | TYPE_TABLE | TYPE_ASSET | TYPE_ANNOTATION | TYPE_WISH | TYPE_BLOCK_DEF)
}

/// Fixed entities every Workspace has; they cannot be created, deleted, moved or renamed by operations.
pub fn is_system_id(id: &str) -> bool {
    matches!(id, ROOT_ID | DATA_ID | SURFACES_ID | CANVAS_CONTENT_ID)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRow {
    pub entity_id: String,
    pub type_id: String,
    pub schema_version: u32,
    pub scope: String,
    pub name: Option<String>,
    pub write_policy: String,
    pub payload: JsonMap,
    pub key_revs: BTreeMap<String, u64>,
    /// Generation dependency record (phase two §7.3): which wish run produced this content and
    /// which input versions it read. Entity-level metadata, not part of the type's payload.
    #[serde(default)]
    pub derived: Option<Value>,
    pub created_seq: u64,
    pub meta_rev: u64,
    pub content_rev: u64,
    pub life_rev: u64,
    pub deleted_seq: Option<u64>,
}

impl EntityRow {
    pub fn alive(&self) -> bool {
        self.deleted_seq.is_none()
    }
    pub fn key_rev(&self, key: &str) -> u64 {
        self.key_revs.get(key).copied().unwrap_or(0)
    }
    /// Content operations are refused on unknown types / newer schema versions (V20).
    pub fn degraded(&self) -> Option<&'static str> {
        if !is_known_type(&self.type_id) {
            Some("MISSING_EXTENSION")
        } else if self.schema_version != 1 {
            Some("UNSUPPORTED_VERSION")
        } else {
            None
        }
    }
    pub fn owner(&self) -> Option<&str> {
        self.scope.strip_prefix("user:")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeEdge {
    pub child_id: String,
    pub parent_id: String,
    pub order_key: String,
    pub placement: Option<Value>,
    pub struct_rev: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldRow {
    pub source_id: String,
    pub field_id: String,
    pub def: FieldDef,
    pub order_key: String,
    pub def_rev: u64,
    pub type_rev: u64,
    pub values_rev: u64,
    pub deleted_seq: Option<u64>,
}

impl FieldRow {
    pub fn alive(&self) -> bool {
        self.deleted_seq.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordRow {
    pub source_id: String,
    pub record_id: String,
    pub values: JsonMap,
    pub revs: BTreeMap<String, u64>,
    pub meta: JsonMap,
    pub body_entity_id: Option<String>,
    pub created_seq: u64,
    pub rev: u64,
    pub deleted_seq: Option<u64>,
}

impl RecordRow {
    pub fn alive(&self) -> bool {
        self.deleted_seq.is_none()
    }
    pub fn value_rev(&self, field: &str) -> u64 {
        self.revs.get(field).copied().unwrap_or(0)
    }
}

/// One edge of the reference index. Empty string means "none" (never NULL, so
/// the primary key can deduplicate).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RefEdge {
    pub src_entity_id: String,
    pub src_selector: String,
    pub kind: String,
    pub dst_workspace_id: String,
    pub dst_entity_id: String,
    pub dst_object_id: String,
    pub dst_query_json: String,
}

impl RefEdge {
    pub fn local(src: &str, selector: &str, kind: &str, dst: &str) -> RefEdge {
        RefEdge {
            src_entity_id: src.into(),
            src_selector: selector.into(),
            kind: kind.into(),
            dst_workspace_id: String::new(),
            dst_entity_id: dst.into(),
            dst_object_id: String::new(),
            dst_query_json: String::new(),
        }
    }
    /// Reference kinds that block deletion of their target. `input` (wish inputs), `derived`
    /// (generation dependencies) and `connector_endpoint` (a line's bound end) do not: the target may go,
    /// the result then reads "引用不可用" and the line keeps its stored end position.
    pub fn blocks_delete(&self) -> bool {
        matches!(self.kind.as_str(), "bind" | "embed" | "value" | "body" | "def")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetInfo {
    pub object_id: String,
    pub media_type: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockInfo {
    /// Parent block id; empty for top-level blocks.
    pub parent: String,
    pub node_type: String,
    pub hash: String,
    pub struct_rev: u64,
}

pub type BlockIndex = BTreeMap<String, BlockInfo>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RichTextMeta {
    pub entity_id: String,
    pub lineage_id: String,
    pub engine: String,
    pub engine_version: String,
    pub encoding: String,
    /// Canonical AST matching `entities.content_rev`; always decoded by the backend.
    pub ast: Value,
    pub block_index: BlockIndex,
}

/// A rich text with its live CRDT document. The persisted authority is
/// snapshot + updates; this document is a cache of it.
#[derive(Debug, Clone)]
pub struct RichTextState {
    pub meta: RichTextMeta,
    pub doc: LoroDoc,
}

/// Read-only view used while planning. One instance sees one consistent state.
pub trait ReadCtx {
    fn entity(&self, id: &str) -> WsResult<Option<EntityRow>>;
    fn edge(&self, child: &str) -> WsResult<Option<TreeEdge>>;
    /// All edges under `parent`, ordered by `(order_key, child_id)`; includes deleted entities.
    fn children(&self, parent: &str) -> WsResult<Vec<TreeEdge>>;
    fn field(&self, source: &str, field: &str) -> WsResult<Option<FieldRow>>;
    /// All fields (including tombstones), ordered by `(order_key, field_id)`.
    fn fields(&self, source: &str) -> WsResult<Vec<FieldRow>>;
    fn record(&self, source: &str, record: &str) -> WsResult<Option<RecordRow>>;
    /// Visit every record row (including tombstones) in `record_id` order; return `false` to stop.
    fn scan_records(&self, source: &str, visit: &mut dyn FnMut(&RecordRow) -> WsResult<bool>) -> WsResult<()>;
    fn richtext(&self, id: &str) -> WsResult<Option<RichTextState>>;
    fn refs_to(&self, dst_entity: &str) -> WsResult<Vec<RefEdge>>;
    fn refs_from(&self, src_entity: &str) -> WsResult<Vec<RefEdge>>;
    /// A verified, available asset (already referenced or staged by an upload).
    fn asset(&self, object_id: &str) -> WsResult<Option<AssetInfo>>;
}

/// Candidate state of one Commit: writes produced so far, layered over the base.
/// Dropping it discards the candidate; nothing was touched underneath.
pub struct Overlay<'a> {
    pub base: &'a dyn ReadCtx,
    pub entities: BTreeMap<String, EntityRow>,
    pub edges: BTreeMap<String, TreeEdge>,
    pub fields: BTreeMap<(String, String), FieldRow>,
    pub records: BTreeMap<(String, String), RecordRow>,
    pub richtexts: BTreeMap<String, RichTextState>,
    /// CRDT update bytes in application order: `(entity_id, bytes)`.
    pub rt_updates: Vec<(String, Vec<u8>)>,
    /// Rich texts created in this batch (their full snapshot must be stored).
    pub rt_created: BTreeSet<String>,
    pub ref_add: BTreeSet<RefEdge>,
    pub ref_del: BTreeSet<RefEdge>,
    pub assets: BTreeMap<String, AssetInfo>,
}

impl<'a> Overlay<'a> {
    pub fn new(base: &'a dyn ReadCtx) -> Self {
        Overlay {
            base,
            entities: BTreeMap::new(),
            edges: BTreeMap::new(),
            fields: BTreeMap::new(),
            records: BTreeMap::new(),
            richtexts: BTreeMap::new(),
            rt_updates: Vec::new(),
            rt_created: BTreeSet::new(),
            ref_add: BTreeSet::new(),
            ref_del: BTreeSet::new(),
            assets: BTreeMap::new(),
        }
    }
    pub fn put_entity(&mut self, e: EntityRow) {
        self.entities.insert(e.entity_id.clone(), e);
    }
    pub fn put_edge(&mut self, e: TreeEdge) {
        self.edges.insert(e.child_id.clone(), e);
    }
    pub fn put_field(&mut self, f: FieldRow) {
        self.fields.insert((f.source_id.clone(), f.field_id.clone()), f);
    }
    pub fn put_record(&mut self, r: RecordRow) {
        self.records.insert((r.source_id.clone(), r.record_id.clone()), r);
    }
    pub fn add_ref(&mut self, r: RefEdge) {
        self.ref_del.remove(&r);
        self.ref_add.insert(r);
    }
    pub fn del_ref(&mut self, r: RefEdge) {
        self.ref_add.remove(&r);
        self.ref_del.insert(r);
    }
    /// Replace the reference set of one source position: `old` → `new`.
    pub fn set_refs(&mut self, old: &BTreeSet<RefEdge>, new: &BTreeSet<RefEdge>) {
        for r in old.difference(new) {
            self.del_ref(r.clone());
        }
        for r in new.difference(old) {
            self.add_ref(r.clone());
        }
    }
}

/// The owned content of an overlay: what a store has to persist atomically.
pub struct Changes {
    pub entities: BTreeMap<String, EntityRow>,
    pub edges: BTreeMap<String, TreeEdge>,
    pub fields: BTreeMap<(String, String), FieldRow>,
    pub records: BTreeMap<(String, String), RecordRow>,
    pub richtexts: BTreeMap<String, RichTextState>,
    pub rt_updates: Vec<(String, Vec<u8>)>,
    pub rt_created: BTreeSet<String>,
    pub ref_add: BTreeSet<RefEdge>,
    pub ref_del: BTreeSet<RefEdge>,
    pub assets: BTreeMap<String, AssetInfo>,
}

impl<'a> Overlay<'a> {
    pub fn into_changes(self) -> Changes {
        Changes {
            entities: self.entities,
            edges: self.edges,
            fields: self.fields,
            records: self.records,
            richtexts: self.richtexts,
            rt_updates: self.rt_updates,
            rt_created: self.rt_created,
            ref_add: self.ref_add,
            ref_del: self.ref_del,
            assets: self.assets,
        }
    }
}

impl ReadCtx for Overlay<'_> {
    fn entity(&self, id: &str) -> WsResult<Option<EntityRow>> {
        match self.entities.get(id) {
            Some(e) => Ok(Some(e.clone())),
            None => self.base.entity(id),
        }
    }
    fn edge(&self, child: &str) -> WsResult<Option<TreeEdge>> {
        match self.edges.get(child) {
            Some(e) => Ok(Some(e.clone())),
            None => self.base.edge(child),
        }
    }
    fn children(&self, parent: &str) -> WsResult<Vec<TreeEdge>> {
        let mut out: Vec<TreeEdge> =
            self.base.children(parent)?.into_iter().filter(|e| !self.edges.contains_key(&e.child_id)).collect();
        out.extend(self.edges.values().filter(|e| e.parent_id == parent).cloned());
        out.sort_by(|a, b| (&a.order_key, &a.child_id).cmp(&(&b.order_key, &b.child_id)));
        Ok(out)
    }
    fn field(&self, source: &str, field: &str) -> WsResult<Option<FieldRow>> {
        match self.fields.get(&(source.to_string(), field.to_string())) {
            Some(f) => Ok(Some(f.clone())),
            None => self.base.field(source, field),
        }
    }
    fn fields(&self, source: &str) -> WsResult<Vec<FieldRow>> {
        let mut out: Vec<FieldRow> = self
            .base
            .fields(source)?
            .into_iter()
            .filter(|f| !self.fields.contains_key(&(source.to_string(), f.field_id.clone())))
            .collect();
        out.extend(self.fields.values().filter(|f| f.source_id == source).cloned());
        out.sort_by(|a, b| (&a.order_key, &a.field_id).cmp(&(&b.order_key, &b.field_id)));
        Ok(out)
    }
    fn record(&self, source: &str, record: &str) -> WsResult<Option<RecordRow>> {
        match self.records.get(&(source.to_string(), record.to_string())) {
            Some(r) => Ok(Some(r.clone())),
            None => self.base.record(source, record),
        }
    }
    fn scan_records(&self, source: &str, visit: &mut dyn FnMut(&RecordRow) -> WsResult<bool>) -> WsResult<()> {
        let lo = (source.to_string(), String::new());
        let mut local = self.records.range(lo..).take_while(|((s, _), _)| s == source).map(|(_, r)| r).peekable();
        let mut stopped = false;
        self.base.scan_records(source, &mut |r| {
            while let Some(l) = local.peek() {
                if l.record_id < r.record_id {
                    let l = local.next().unwrap();
                    if !visit(l)? {
                        stopped = true;
                        return Ok(false);
                    }
                } else {
                    break;
                }
            }
            let row = match local.peek() {
                Some(l) if l.record_id == r.record_id => local.next().unwrap(),
                _ => r,
            };
            if !visit(row)? {
                stopped = true;
                return Ok(false);
            }
            Ok(true)
        })?;
        if !stopped {
            for l in local {
                if !visit(l)? {
                    break;
                }
            }
        }
        Ok(())
    }
    fn richtext(&self, id: &str) -> WsResult<Option<RichTextState>> {
        match self.richtexts.get(id) {
            Some(r) => Ok(Some(r.clone())),
            None => self.base.richtext(id),
        }
    }
    fn refs_to(&self, dst_entity: &str) -> WsResult<Vec<RefEdge>> {
        let mut out: Vec<RefEdge> =
            self.base.refs_to(dst_entity)?.into_iter().filter(|r| !self.ref_del.contains(r)).collect();
        for r in &self.ref_add {
            if r.dst_entity_id == dst_entity && r.dst_workspace_id.is_empty() && !out.contains(r) {
                out.push(r.clone());
            }
        }
        Ok(out)
    }
    fn refs_from(&self, src_entity: &str) -> WsResult<Vec<RefEdge>> {
        let mut out: Vec<RefEdge> =
            self.base.refs_from(src_entity)?.into_iter().filter(|r| !self.ref_del.contains(r)).collect();
        for r in &self.ref_add {
            if r.src_entity_id == src_entity && !out.contains(r) {
                out.push(r.clone());
            }
        }
        Ok(out)
    }
    fn asset(&self, object_id: &str) -> WsResult<Option<AssetInfo>> {
        self.base.asset(object_id)
    }
}

/// In-memory `ReadCtx`, used by tests and as the replica's simplest backing.
#[derive(Default, Clone)]
pub struct MemStore {
    pub entities: BTreeMap<String, EntityRow>,
    pub edges: BTreeMap<String, TreeEdge>,
    pub fields: BTreeMap<(String, String), FieldRow>,
    pub records: BTreeMap<(String, String), RecordRow>,
    pub richtexts: BTreeMap<String, RichTextState>,
    pub refs: BTreeSet<RefEdge>,
    pub assets: BTreeMap<String, AssetInfo>,
}

impl MemStore {
    /// A store holding the root and the system nodes, as a freshly created Workspace.
    pub fn with_root() -> MemStore {
        let mut m = MemStore::default();
        for (e, edge) in system_entities(None) {
            m.entities.insert(e.entity_id.clone(), e);
            if let Some(edge) = edge {
                m.edges.insert(edge.child_id.clone(), edge);
            }
        }
        m
    }

    pub fn apply(&mut self, c: Changes) {
        self.entities.extend(c.entities);
        self.edges.extend(c.edges);
        self.fields.extend(c.fields);
        self.records.extend(c.records);
        self.richtexts.extend(c.richtexts);
        for r in c.ref_del {
            self.refs.remove(&r);
        }
        self.refs.extend(c.ref_add);
        self.assets.extend(c.assets);
    }
}

fn system_container(id: &str, kind: &str, name: Option<&str>, title: Option<&str>) -> EntityRow {
    let mut payload = Map::new();
    payload.insert("kind".into(), Value::String(kind.into()));
    if let Some(t) = title {
        payload.insert("title".into(), Value::String(t.into()));
    }
    if id == CANVAS_CONTENT_ID {
        payload.insert("system".into(), Value::String("canvas_content".into()));
    }
    let mut key_revs = BTreeMap::new();
    for k in payload.keys() {
        key_revs.insert(k.clone(), 0);
    }
    EntityRow {
        entity_id: id.into(),
        type_id: TYPE_CONTAINER.into(),
        schema_version: 1,
        scope: SCOPE_SHARED.into(),
        name: name.map(str::to_string),
        write_policy: POLICY_OPEN.into(),
        payload,
        key_revs,
        derived: None,
        created_seq: 0,
        meta_rev: 0,
        content_rev: 0,
        life_rev: 0,
        deleted_seq: None,
    }
}

/// The root container every Workspace starts with (`seq` 0).
pub fn root_entity() -> EntityRow {
    let mut root = system_container(ROOT_ID, "root", None, None);
    root.payload.insert("layout".into(), serde_json::json!({ "mode": "flow" }));
    root.key_revs.insert("layout".into(), 0);
    root
}

/// Root plus the system nodes of the two trees (phase two §4.5), with their edges, in
/// parent-before-child order. `title` becomes the root's title.
pub fn system_entities(title: Option<&str>) -> Vec<(EntityRow, Option<TreeEdge>)> {
    let edge = |child: &str, parent: &str, key: &str| TreeEdge { child_id: child.into(), parent_id: parent.into(), order_key: key.into(), placement: None, struct_rev: 0 };
    let mut root = root_entity();
    if let Some(t) = title.filter(|t| !t.is_empty()) {
        root.payload.insert("title".into(), Value::String(t.into()));
        root.key_revs.insert("title".into(), 0);
    }
    vec![
        (root, None),
        (system_container(DATA_ID, "data", Some("data"), Some("数据")), Some(edge(DATA_ID, ROOT_ID, "a"))),
        (system_container(SURFACES_ID, "surfaces", Some("surfaces"), Some("画布")), Some(edge(SURFACES_ID, ROOT_ID, "b"))),
        (system_container(CANVAS_CONTENT_ID, "folder", Some("canvas-content"), Some("画布内容")), Some(edge(CANVAS_CONTENT_ID, DATA_ID, "zz"))),
    ]
}

impl ReadCtx for MemStore {
    fn entity(&self, id: &str) -> WsResult<Option<EntityRow>> {
        Ok(self.entities.get(id).cloned())
    }
    fn edge(&self, child: &str) -> WsResult<Option<TreeEdge>> {
        Ok(self.edges.get(child).cloned())
    }
    fn children(&self, parent: &str) -> WsResult<Vec<TreeEdge>> {
        let mut out: Vec<TreeEdge> = self.edges.values().filter(|e| e.parent_id == parent).cloned().collect();
        out.sort_by(|a, b| (&a.order_key, &a.child_id).cmp(&(&b.order_key, &b.child_id)));
        Ok(out)
    }
    fn field(&self, source: &str, field: &str) -> WsResult<Option<FieldRow>> {
        Ok(self.fields.get(&(source.to_string(), field.to_string())).cloned())
    }
    fn fields(&self, source: &str) -> WsResult<Vec<FieldRow>> {
        let mut out: Vec<FieldRow> = self.fields.values().filter(|f| f.source_id == source).cloned().collect();
        out.sort_by(|a, b| (&a.order_key, &a.field_id).cmp(&(&b.order_key, &b.field_id)));
        Ok(out)
    }
    fn record(&self, source: &str, record: &str) -> WsResult<Option<RecordRow>> {
        Ok(self.records.get(&(source.to_string(), record.to_string())).cloned())
    }
    fn scan_records(&self, source: &str, visit: &mut dyn FnMut(&RecordRow) -> WsResult<bool>) -> WsResult<()> {
        let lo = (source.to_string(), String::new());
        for (_, r) in self.records.range(lo..).take_while(|((s, _), _)| s == source) {
            if !visit(r)? {
                break;
            }
        }
        Ok(())
    }
    fn richtext(&self, id: &str) -> WsResult<Option<RichTextState>> {
        Ok(self.richtexts.get(id).cloned())
    }
    fn refs_to(&self, dst_entity: &str) -> WsResult<Vec<RefEdge>> {
        Ok(self.refs.iter().filter(|r| r.dst_entity_id == dst_entity && r.dst_workspace_id.is_empty()).cloned().collect())
    }
    fn refs_from(&self, src_entity: &str) -> WsResult<Vec<RefEdge>> {
        Ok(self.refs.iter().filter(|r| r.src_entity_id == src_entity).cloned().collect())
    }
    fn asset(&self, object_id: &str) -> WsResult<Option<AssetInfo>> {
        Ok(self.assets.get(object_id).cloned())
    }
}
