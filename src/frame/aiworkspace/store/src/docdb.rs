//! SQLite implementation of the kernel's read view and of applying a
//! candidate (`Changes`) inside one transaction.

use aiworkspace_core::model::*;
use aiworkspace_core::value::FieldDef;
use aiworkspace_core::{richtext, Code, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

pub fn db_err(e: rusqlite::Error) -> WsError {
    let code = match &e {
        rusqlite::Error::SqliteFailure(f, _) => match f.code {
            rusqlite::ErrorCode::DiskFull => Code::StorageFull,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => Code::WriterBusy,
            _ => Code::StorageIoError,
        },
        _ => Code::StorageIoError,
    };
    WsError::new(code, format!("storage: {e}"))
}

fn bad_row(what: &str, e: impl std::fmt::Display) -> WsError {
    WsError::io(format!("corrupt {what} row: {e}"))
}

fn json_of<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> WsResult<T> {
    serde_json::from_str(text).map_err(|e| bad_row(what, e))
}

const ENTITY_COLS: &str = "entity_id, type_id, schema_version, scope, name, write_policy, payload_json, key_revs_json, \
                           created_seq, meta_rev, content_rev, life_rev, deleted_seq";

fn entity_row(r: &Row) -> rusqlite::Result<(EntityRow, String, String)> {
    Ok((
        EntityRow {
            entity_id: r.get(0)?,
            type_id: r.get(1)?,
            schema_version: r.get(2)?,
            scope: r.get(3)?,
            name: r.get(4)?,
            write_policy: r.get(5)?,
            payload: JsonMap::new(),
            key_revs: BTreeMap::new(),
            created_seq: r.get(8)?,
            meta_rev: r.get(9)?,
            content_rev: r.get(10)?,
            life_rev: r.get(11)?,
            deleted_seq: r.get(12)?,
        },
        r.get(6)?,
        r.get(7)?,
    ))
}

fn finish_entity((mut e, payload, revs): (EntityRow, String, String)) -> WsResult<EntityRow> {
    e.payload = json_of("entity", &payload)?;
    e.key_revs = json_of("entity", &revs)?;
    Ok(e)
}

fn edge_row(r: &Row) -> rusqlite::Result<(TreeEdge, Option<String>)> {
    Ok((
        TreeEdge { child_id: r.get(0)?, parent_id: r.get(1)?, order_key: r.get(2)?, placement: None, struct_rev: r.get(4)? },
        r.get(3)?,
    ))
}

fn finish_edge((mut e, placement): (TreeEdge, Option<String>)) -> WsResult<TreeEdge> {
    if let Some(p) = placement {
        e.placement = Some(json_of("tree edge", &p)?);
    }
    Ok(e)
}

type RawField = (String, String, String, String, u64, u64, u64, Option<u64>);

fn field_row(r: &Row) -> rusqlite::Result<RawField> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
}

fn finish_field(raw: RawField) -> WsResult<FieldRow> {
    let def: FieldDef = json_of("field", &raw.2)?;
    Ok(FieldRow { source_id: raw.0, field_id: raw.1, def, order_key: raw.3, def_rev: raw.4, type_rev: raw.5, values_rev: raw.6, deleted_seq: raw.7 })
}

type RawRecord = (String, String, String, String, String, Option<String>, u64, u64, Option<u64>);

fn record_row(r: &Row) -> rusqlite::Result<RawRecord> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
}

fn finish_record(raw: RawRecord) -> WsResult<RecordRow> {
    Ok(RecordRow {
        source_id: raw.0,
        record_id: raw.1,
        values: json_of("record", &raw.2)?,
        revs: json_of("record", &raw.3)?,
        meta: json_of("record", &raw.4)?,
        body_entity_id: raw.5,
        created_seq: raw.6,
        rev: raw.7,
        deleted_seq: raw.8,
    })
}

fn ref_row(r: &Row) -> rusqlite::Result<RefEdge> {
    Ok(RefEdge {
        src_entity_id: r.get(0)?,
        src_selector: r.get(1)?,
        kind: r.get(2)?,
        dst_workspace_id: r.get(3)?,
        dst_entity_id: r.get(4)?,
        dst_object_id: r.get(5)?,
        dst_query_json: r.get(6)?,
    })
}

const REF_COLS: &str = "src_entity_id, src_selector, kind, dst_workspace_id, dst_entity_id, dst_object_id, dst_query_json";
const RECORD_COLS: &str = "source_id, record_id, values_json, revs_json, meta_json, body_entity_id, created_seq, rev, deleted_seq";
const FIELD_COLS: &str = "source_id, field_id, def_json, order_key, def_rev, type_rev, values_rev, deleted_seq";

/// Cache of authoritative CRDT documents. It only ever holds what the database
/// holds: documents enter after their transaction committed.
#[derive(Default)]
pub struct DocCache {
    pub docs: HashMap<String, RichTextState>,
}

pub const DOC_CACHE_LIMIT: usize = 64;

pub struct SqlCtx<'a> {
    pub conn: &'a Connection,
    /// Optional second database holding staged (uploaded, not yet referenced) assets.
    pub local: Option<&'a Connection>,
    pub cache: &'a RefCell<DocCache>,
    pub now: &'a str,
}

impl SqlCtx<'_> {
    pub fn all_entity_ids(&self) -> WsResult<Vec<String>> {
        let mut st = self.conn.prepare_cached("SELECT entity_id FROM entities ORDER BY entity_id").map_err(db_err)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)
    }

    pub fn all_refs(&self) -> WsResult<Vec<RefEdge>> {
        let mut st = self.conn.prepare_cached(&format!("SELECT {REF_COLS} FROM refs")).map_err(db_err)?;
        let rows = st.query_map([], ref_row).map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)
    }

    /// Load a rich text from its persisted authority: snapshot, then updates in `seq` order.
    fn load_richtext(&self, id: &str) -> WsResult<Option<RichTextState>> {
        let row = self
            .conn
            .query_row(
                "SELECT lineage_id, engine, engine_version, encoding, snapshot, snapshot_seq, ast_json, block_index_json \
                 FROM richtext_states WHERE entity_id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Vec<u8>>(4)?,
                        r.get::<_, u64>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(db_err)?;
        let Some((lineage_id, engine, engine_version, encoding, snapshot, snapshot_seq, ast, index)) = row else { return Ok(None) };
        if engine != richtext::ENGINE || encoding != richtext::ENCODING {
            return Err(WsError::new(Code::UnsupportedVersion, format!("rich text {id} uses {engine}/{encoding}")));
        }
        let mut st = self
            .conn
            .prepare_cached("SELECT update_bytes FROM richtext_updates WHERE entity_id = ?1 AND seq > ?2 ORDER BY seq, idx")
            .map_err(db_err)?;
        let updates: Vec<Vec<u8>> =
            st.query_map(params![id, snapshot_seq], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let doc = richtext::load_doc(&snapshot, updates.iter().map(Vec::as_slice))?;
        let meta = RichTextMeta {
            entity_id: id.to_string(),
            lineage_id,
            engine,
            engine_version,
            encoding,
            ast: json_of("rich text", &ast)?,
            block_index: json_of("rich text", &index)?,
        };
        Ok(Some(RichTextState { meta, doc }))
    }
}

impl ReadCtx for SqlCtx<'_> {
    fn entity(&self, id: &str) -> WsResult<Option<EntityRow>> {
        let mut st = self.conn.prepare_cached(&format!("SELECT {ENTITY_COLS} FROM entities WHERE entity_id = ?1")).map_err(db_err)?;
        st.query_row([id], entity_row).optional().map_err(db_err)?.map(finish_entity).transpose()
    }
    fn edge(&self, child: &str) -> WsResult<Option<TreeEdge>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT child_id, parent_id, order_key, placement_json, struct_rev FROM tree_edges WHERE child_id = ?1")
            .map_err(db_err)?;
        st.query_row([child], edge_row).optional().map_err(db_err)?.map(finish_edge).transpose()
    }
    fn children(&self, parent: &str) -> WsResult<Vec<TreeEdge>> {
        let mut st = self
            .conn
            .prepare_cached(
                "SELECT child_id, parent_id, order_key, placement_json, struct_rev FROM tree_edges \
                 WHERE parent_id = ?1 ORDER BY order_key, child_id",
            )
            .map_err(db_err)?;
        let rows = st.query_map([parent], edge_row).map_err(db_err)?.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
        rows.into_iter().map(finish_edge).collect()
    }
    fn field(&self, source: &str, field: &str) -> WsResult<Option<FieldRow>> {
        let mut st = self
            .conn
            .prepare_cached(&format!("SELECT {FIELD_COLS} FROM table_fields WHERE source_id = ?1 AND field_id = ?2"))
            .map_err(db_err)?;
        st.query_row([source, field], field_row).optional().map_err(db_err)?.map(finish_field).transpose()
    }
    fn fields(&self, source: &str) -> WsResult<Vec<FieldRow>> {
        let mut st = self
            .conn
            .prepare_cached(&format!("SELECT {FIELD_COLS} FROM table_fields WHERE source_id = ?1 ORDER BY order_key, field_id"))
            .map_err(db_err)?;
        let rows = st.query_map([source], field_row).map_err(db_err)?.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
        rows.into_iter().map(finish_field).collect()
    }
    fn record(&self, source: &str, record: &str) -> WsResult<Option<RecordRow>> {
        let mut st = self
            .conn
            .prepare_cached(&format!("SELECT {RECORD_COLS} FROM table_records WHERE source_id = ?1 AND record_id = ?2"))
            .map_err(db_err)?;
        st.query_row([source, record], record_row).optional().map_err(db_err)?.map(finish_record).transpose()
    }
    fn scan_records(&self, source: &str, visit: &mut dyn FnMut(&RecordRow) -> WsResult<bool>) -> WsResult<()> {
        let mut st = self
            .conn
            .prepare_cached(&format!("SELECT {RECORD_COLS} FROM table_records WHERE source_id = ?1 ORDER BY record_id"))
            .map_err(db_err)?;
        let mut rows = st.query([source]).map_err(db_err)?;
        while let Some(r) = rows.next().map_err(db_err)? {
            let rec = finish_record(record_row(r).map_err(db_err)?)?;
            if !visit(&rec)? {
                break;
            }
        }
        Ok(())
    }
    fn richtext(&self, id: &str) -> WsResult<Option<RichTextState>> {
        if let Some(s) = self.cache.borrow().docs.get(id) {
            return Ok(Some(s.clone()));
        }
        let loaded = self.load_richtext(id)?;
        if let Some(s) = &loaded {
            let mut c = self.cache.borrow_mut();
            if c.docs.len() >= DOC_CACHE_LIMIT {
                // eviction needs no write-back: the authority is in the database
                if let Some(k) = c.docs.keys().next().cloned() {
                    c.docs.remove(&k);
                }
            }
            c.docs.insert(id.to_string(), s.clone());
        }
        Ok(loaded)
    }
    fn refs_to(&self, dst_entity: &str) -> WsResult<Vec<RefEdge>> {
        let mut st = self
            .conn
            .prepare_cached(&format!("SELECT {REF_COLS} FROM refs WHERE dst_workspace_id = '' AND dst_entity_id = ?1"))
            .map_err(db_err)?;
        let rows = st.query_map([dst_entity], ref_row).map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)
    }
    fn refs_from(&self, src_entity: &str) -> WsResult<Vec<RefEdge>> {
        let mut st = self.conn.prepare_cached(&format!("SELECT {REF_COLS} FROM refs WHERE src_entity_id = ?1")).map_err(db_err)?;
        let rows = st.query_map([src_entity], ref_row).map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)
    }
    fn asset(&self, object_id: &str) -> WsResult<Option<AssetInfo>> {
        let known = self
            .conn
            .query_row("SELECT media_type, size FROM assets WHERE object_id = ?1", [object_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
            })
            .optional()
            .map_err(db_err)?;
        let staged = match (&known, self.local) {
            (None, Some(local)) => local
                .query_row(
                    "SELECT media_type, size FROM staged_assets WHERE object_id = ?1 AND expires_at > ?2",
                    params![object_id, self.now],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)),
                )
                .optional()
                .map_err(db_err)?,
            _ => None,
        };
        Ok(known.or(staged).map(|(media_type, size)| AssetInfo { object_id: object_id.to_string(), media_type, size }))
    }
}

fn text(v: &impl serde::Serialize) -> String {
    serde_json::to_string(v).expect("serializable row")
}

/// Statistics of one applied candidate (tests assert on written row counts).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct WriteStats {
    pub entities: usize,
    pub edges: usize,
    pub fields: usize,
    pub records: usize,
    pub richtext_updates: usize,
    pub refs: usize,
}

/// Write a candidate into the open transaction. Rich text created in this
/// batch stores a full snapshot; later changes store their update bytes.
pub fn apply_changes(tx: &Connection, c: &Changes, seq: u64) -> WsResult<WriteStats> {
    let mut stats = WriteStats::default();
    {
        let mut st = tx
            .prepare_cached(&format!(
                "INSERT OR REPLACE INTO entities ({ENTITY_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"
            ))
            .map_err(db_err)?;
        for e in c.entities.values() {
            st.execute(params![
                e.entity_id, e.type_id, e.schema_version, e.scope, e.name, e.write_policy, text(&e.payload), text(&e.key_revs),
                e.created_seq, e.meta_rev, e.content_rev, e.life_rev, e.deleted_seq
            ])
            .map_err(db_err)?;
            stats.entities += 1;
        }
    }
    {
        let mut st = tx
            .prepare_cached("INSERT OR REPLACE INTO tree_edges (child_id, parent_id, order_key, placement_json, struct_rev) VALUES (?1,?2,?3,?4,?5)")
            .map_err(db_err)?;
        for e in c.edges.values() {
            st.execute(params![e.child_id, e.parent_id, e.order_key, e.placement.as_ref().map(text), e.struct_rev]).map_err(db_err)?;
            stats.edges += 1;
        }
    }
    {
        let mut st = tx
            .prepare_cached(&format!("INSERT OR REPLACE INTO table_fields ({FIELD_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"))
            .map_err(db_err)?;
        for f in c.fields.values() {
            st.execute(params![f.source_id, f.field_id, text(&f.def), f.order_key, f.def_rev, f.type_rev, f.values_rev, f.deleted_seq])
                .map_err(db_err)?;
            stats.fields += 1;
        }
    }
    {
        let mut st = tx
            .prepare_cached(&format!("INSERT OR REPLACE INTO table_records ({RECORD_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"))
            .map_err(db_err)?;
        for r in c.records.values() {
            st.execute(params![
                r.source_id, r.record_id, text(&r.values), text(&r.revs), text(&r.meta), r.body_entity_id, r.created_seq, r.rev, r.deleted_seq
            ])
            .map_err(db_err)?;
            stats.records += 1;
        }
    }
    for (id, st) in &c.richtexts {
        if c.rt_created.contains(id) {
            let snapshot = richtext::export_snapshot(&st.doc)?;
            tx.execute(
                "INSERT INTO richtext_states (entity_id, lineage_id, engine, engine_version, encoding, snapshot, snapshot_seq, ast_json, block_index_json) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id, st.meta.lineage_id, st.meta.engine, st.meta.engine_version, st.meta.encoding, snapshot, seq, text(&st.meta.ast), text(&st.meta.block_index)],
            )
            .map_err(db_err)?;
        } else {
            tx.execute(
                "UPDATE richtext_states SET ast_json = ?2, block_index_json = ?3 WHERE entity_id = ?1",
                params![id, text(&st.meta.ast), text(&st.meta.block_index)],
            )
            .map_err(db_err)?;
        }
    }
    let mut idx: HashMap<&str, u32> = HashMap::new();
    for (id, bytes) in &c.rt_updates {
        if c.rt_created.contains(id) {
            continue; // already inside the creation snapshot
        }
        let n = idx.entry(id.as_str()).or_default();
        tx.execute("INSERT INTO richtext_updates (entity_id, seq, idx, update_bytes) VALUES (?1,?2,?3,?4)", params![id, seq, *n, bytes])
            .map_err(db_err)?;
        *n += 1;
        stats.richtext_updates += 1;
    }
    for r in &c.ref_del {
        tx.execute(
            "DELETE FROM refs WHERE src_entity_id=?1 AND src_selector=?2 AND kind=?3 AND dst_workspace_id=?4 AND dst_entity_id=?5 AND dst_object_id=?6 AND dst_query_json=?7",
            params![r.src_entity_id, r.src_selector, r.kind, r.dst_workspace_id, r.dst_entity_id, r.dst_object_id, r.dst_query_json],
        )
        .map_err(db_err)?;
        stats.refs += 1;
    }
    for r in &c.ref_add {
        tx.execute(
            &format!("INSERT OR IGNORE INTO refs ({REF_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7)"),
            params![r.src_entity_id, r.src_selector, r.kind, r.dst_workspace_id, r.dst_entity_id, r.dst_object_id, r.dst_query_json],
        )
        .map_err(db_err)?;
        stats.refs += 1;
    }
    for a in c.assets.values() {
        tx.execute(
            "INSERT OR IGNORE INTO assets (object_id, media_type, size, first_seq) VALUES (?1,?2,?3,?4)",
            params![a.object_id, a.media_type, a.size, seq],
        )
        .map_err(db_err)?;
    }
    Ok(stats)
}

pub fn meta_get(conn: &Connection, key: &str) -> WsResult<Option<String>> {
    conn.query_row("SELECT value FROM workspace_meta WHERE key = ?1", [key], |r| r.get(0)).optional().map_err(db_err)
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> WsResult<()> {
    conn.execute("INSERT OR REPLACE INTO workspace_meta (key, value) VALUES (?1, ?2)", params![key, value]).map_err(db_err)?;
    Ok(())
}

pub fn json_text(v: &Value) -> String {
    v.to_string()
}
