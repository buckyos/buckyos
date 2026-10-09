//! RichText operations (design doc §3.3.4, §3.3.5). Every change is made on a
//! fork of the authoritative document; the fork only replaces it after the
//! storage transaction commits. A rejected candidate never touches the
//! authoritative CRDT, the persisted state or anything already broadcast.

use crate::access::Cap;
use crate::error::{Code, WsError, WsResult};
use crate::id::check_id;
use crate::model::*;
use crate::plan::{get_array, get_str, only_keys, sel, OpResult, Planner};
use crate::richtext::{self, BlockOp, Position, Validated};
use crate::types::richtext_ref_edge;
use base64::Engine;
use loro::LoroDoc;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub fn dispatch(p: &mut Planner, name: &str, op: &Value) -> OpResult {
    match name {
        "richtext.apply_update" => apply_update(p, op),
        "richtext.insert_blocks" | "richtext.replace_block" | "richtext.delete_blocks" | "richtext.move_block" => {
            block_op(p, name, op)
        }
        _ => Err(WsError::invalid_op(format!("unknown operation {name:?}"))),
    }
}

fn block_sel(block_id: &str) -> Value {
    json!({ "kind": "richtext_block", "block_id": block_id })
}

fn ref_set(entity_id: &str, v: &Validated) -> BTreeSet<RefEdge> {
    v.refs.iter().map(|r| richtext_ref_edge(entity_id, &r.block_id, &r.reference)).collect()
}

/// References that appear with this change must point at readable, live
/// targets of the right type. Existing references are not re-litigated.
fn check_new_refs(p: &Planner, old: &Validated, new: &Validated) -> WsResult<()> {
    for r in &new.refs {
        if old.refs.contains(r) {
            continue;
        }
        let types: Option<&[&str]> = if r.kind == "embed" { Some(&[TYPE_CELL]) } else { None };
        p.check_ref_target(&r.reference, types)?;
    }
    Ok(())
}

/// Create the rich text state for a new entity. The recorded payload always
/// carries the CRDT snapshot so replicas obtain the very same lineage.
pub fn create(p: &mut Planner, row: &mut EntityRow, payload: JsonMap) -> WsResult<Value> {
    for k in payload.keys() {
        if !matches!(k.as_str(), "content" | "lineage_id" | "snapshot") {
            return Err(WsError::invalid_schema(format!("richtext: unknown key {k}")));
        }
    }
    let id = row.entity_id.clone();
    let (doc, lineage_id) = match (payload.get("content"), payload.get("snapshot")) {
        (Some(_), Some(_)) => return Err(WsError::invalid_schema("give either content or snapshot")),
        (_, Some(s)) => {
            // created offline by a client with the same core; decoded and fully validated below
            let lineage = payload.get("lineage_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_schema("lineage_id required with snapshot"))?;
            check_id("lineage_id", lineage)?;
            let bytes = s.as_str().and_then(|s| B64.decode(s).ok()).ok_or_else(|| WsError::invalid_schema("snapshot must be base64"))?;
            if bytes.len() > 8 * p.env.limits.richtext.max_update_bytes {
                return Err(WsError::limit("rich text snapshot too large"));
            }
            (richtext::load_doc(&bytes, std::iter::empty())?, lineage.to_string())
        }
        (content, None) => {
            let ast = match content {
                Some(c) => richtext::canonicalize(c)?,
                None => richtext::empty_ast("b0"),
            };
            let mut lineage = format!("ln{}-{}", p.seq(), id);
            lineage.truncate(64);
            (richtext::build_doc(&ast, p.env.peer_id)?, lineage)
        }
    };
    let ast = richtext::decode_ast(&doc)?;
    let v = richtext::validate_ast(&ast, &p.env.limits.richtext)?;
    check_new_refs(p, &Validated::default(), &v)?;
    p.ov.set_refs(&BTreeSet::new(), &ref_set(&id, &v));
    let snapshot = richtext::export_snapshot(&doc)?;
    let meta = RichTextMeta {
        entity_id: id.clone(),
        lineage_id: lineage_id.clone(),
        engine: richtext::ENGINE.into(),
        engine_version: richtext::engine_version().into(),
        encoding: richtext::ENCODING.into(),
        ast,
        block_index: richtext::initial_block_index(&v, p.seq()),
    };
    p.ov.richtexts.insert(id.clone(), RichTextState { meta, doc });
    p.ov.rt_created.insert(id);
    Ok(json!({ "lineage_id": lineage_id, "snapshot": B64.encode(snapshot) }))
}

fn state(p: &mut Planner, entity_id: &str, cap: Cap) -> WsResult<(EntityRow, RichTextState)> {
    let e = p.entity_alive(entity_id)?;
    if e.type_id != TYPE_RICHTEXT {
        return Err(WsError::invalid_op(format!("{entity_id} is not a rich text")));
    }
    if let Some(d) = e.degraded() {
        return Err(WsError::new(Code::UnsupportedVersion, format!("rich text {entity_id} cannot be edited ({d})")));
    }
    p.require(entity_id, cap)?;
    p.need_lock(&e);
    let st = p.ov.richtext(entity_id)?.ok_or_else(|| WsError::io(format!("rich text state of {entity_id} is missing")))?;
    Ok((e, st))
}

/// The candidate document: a fork of the authoritative one, or the candidate
/// this very batch already produced.
fn candidate(p: &Planner, entity_id: &str, st: &RichTextState) -> LoroDoc {
    if p.ov.richtexts.contains_key(entity_id) {
        st.doc.clone()
    } else {
        richtext::fork(&st.doc)
    }
}

/// Validate the candidate and stage it. Returns the ids of blocks whose
/// content or position changed.
fn stage(p: &mut Planner, mut e: EntityRow, old: RichTextState, cand: LoroDoc, update: Vec<u8>) -> WsResult<Vec<String>> {
    let id = e.entity_id.clone();
    if update.len() > p.env.limits.richtext.max_update_bytes {
        return Err(WsError::limit("rich text update exceeds the size limit"));
    }
    let ast = richtext::decode_ast(&cand)?;
    let new_v = richtext::validate_ast(&ast, &p.env.limits.richtext)?;
    let old_v = richtext::validate_ast(&old.meta.ast, &p.env.limits.richtext)?;
    check_new_refs(p, &old_v, &new_v)?;
    let block_index = richtext::next_block_index(&old.meta.block_index, &old_v, &new_v, p.seq());
    let mut changed: Vec<String> = block_index
        .iter()
        .filter(|(b, info)| old.meta.block_index.get(*b).map_or(true, |o| o.hash != info.hash || o.struct_rev != info.struct_rev))
        .map(|(b, _)| b.clone())
        .collect();
    changed.extend(old.meta.block_index.keys().filter(|b| !block_index.contains_key(*b)).cloned());
    p.ov.set_refs(&ref_set(&id, &old_v), &ref_set(&id, &new_v));
    let meta = RichTextMeta { ast, block_index, ..old.meta };
    p.ov.richtexts.insert(id.clone(), RichTextState { meta, doc: cand });
    p.ov.rt_updates.push((id.clone(), update));
    e.content_rev = p.seq();
    p.ov.put_entity(e);
    p.touch(&id, None, "text");
    for b in changed.iter().take(256) {
        p.touch(&id, sel(block_sel(b)), "text");
    }
    Ok(changed)
}

fn apply_update(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["entity_id", "lineage_id", "update"])?;
    let id = get_str(op, "entity_id")?;
    let (e, st) = state(p, id, Cap::Update)?;
    if get_str(op, "lineage_id")? != st.meta.lineage_id {
        return Err(WsError::sub(Code::InvalidOperation, "LINEAGE_MISMATCH", "update belongs to another collaboration lineage"));
    }
    let bytes = B64.decode(get_str(op, "update")?).map_err(|_| WsError::invalid_op("update must be base64"))?;
    if bytes.len() > p.env.limits.richtext.max_update_bytes {
        return Err(WsError::limit("rich text update exceeds the size limit"));
    }
    let cand = candidate(p, id, &st);
    if !richtext::import_update(&cand, &bytes)? {
        return Err(WsError::new(Code::BaseUnknown, "update depends on operations the backend does not have; catch up and resend"));
    }
    stage(p, e, st, cand, bytes)?;
    // SessionLocal: only the originating editor session can undo it
    Ok((op.clone(), None))
}

fn hash_of(st: &RichTextState, block_id: &str) -> Option<String> {
    st.meta.block_index.get(block_id).map(|b| b.hash.clone())
}

/// Check the content/position tokens a block operation must carry.
fn check_block(
    p: &mut Planner,
    entity_id: &str,
    st: &RichTextState,
    block_id: &str,
    expect: Option<&Value>,
    need_hash: bool,
    need_pos: bool,
    item: Option<usize>,
) -> WsResult<bool> {
    let Some(info) = st.meta.block_index.get(block_id) else {
        p.conflict(Code::TargetDeleted, item, entity_id, sel(block_sel(block_id)), expect.cloned().unwrap_or(Value::Null), Value::Null, None);
        return Ok(false);
    };
    let expect = expect.and_then(Value::as_object).ok_or_else(|| WsError::invalid_op("expect required").at("/expect"))?;
    let mut ok = true;
    if need_hash {
        let want = expect.get("hash").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("expect.hash required"))?;
        ok &= want == info.hash;
    }
    if need_pos {
        let want = expect.get("struct_rev").and_then(Value::as_u64).ok_or_else(|| WsError::invalid_op("expect.struct_rev required"))?;
        ok &= want == info.struct_rev || info.struct_rev == p.seq();
    }
    if !ok {
        let blocks = richtext::index_blocks(&st.meta.ast);
        let current = blocks.get(block_id).map(|l| json!({ "node": l.node, "position": richtext::position_of(l).to_json() }));
        p.conflict(
            Code::RevisionConflict,
            item,
            entity_id,
            sel(block_sel(block_id)),
            Value::Object(expect.clone()),
            json!({ "hash": info.hash, "struct_rev": info.struct_rev }),
            current,
        );
    }
    Ok(ok)
}

fn canon_block(node: &Value) -> WsResult<Value> {
    let n = richtext::canonicalize(node)?;
    if richtext::block_id_of(&n).is_none() {
        return Err(WsError::invalid_schema("a block node with block_id is required"));
    }
    Ok(n)
}

fn block_op(p: &mut Planner, name: &str, op: &Value) -> OpResult {
    let id = get_str(op, "entity_id")?;
    let (e, st) = state(p, id, Cap::Update)?;
    let before = richtext::index_blocks(&st.meta.ast);
    let seq = p.seq();
    let mut norm = op.clone();
    // inverse operations are completed with post-commit tokens once the new index is known
    enum Undo {
        Inserted(Vec<String>),
        Replaced(String, Value),
        Deleted(Vec<(Position, Value)>),
        Moved(String, Position),
    }
    let (bop, undo) = match name {
        "richtext.insert_blocks" => {
            only_keys(op, &["entity_id", "position", "blocks", "server_update", "lineage_id"])?;
            let position = Position::parse(op.get("position").ok_or_else(|| WsError::invalid_op("position required"))?)?;
            let blocks = get_array(op, "blocks")?.iter().map(canon_block).collect::<WsResult<Vec<_>>>()?;
            if blocks.is_empty() {
                return Err(WsError::invalid_op("blocks must not be empty"));
            }
            let ids = blocks.iter().filter_map(|b| richtext::block_id_of(b).map(str::to_string)).collect();
            norm["blocks"] = Value::Array(blocks.clone());
            (BlockOp::Insert { position, blocks }, Undo::Inserted(ids))
        }
        "richtext.replace_block" => {
            only_keys(op, &["entity_id", "block_id", "expect", "node", "server_update", "lineage_id"])?;
            let block_id = get_str(op, "block_id")?.to_string();
            if !check_block(p, id, &st, &block_id, op.get("expect"), true, false, None)? {
                return Ok((op.clone(), None));
            }
            let node = canon_block(op.get("node").ok_or_else(|| WsError::invalid_op("node required"))?)?;
            if richtext::block_id_of(&node) != Some(block_id.as_str()) {
                return Err(WsError::invalid_schema("node.attrs.block_id must equal block_id"));
            }
            norm["node"] = node.clone();
            let old = before[&block_id].node.clone();
            (BlockOp::Replace { block_id: block_id.clone(), node }, Undo::Replaced(block_id, old))
        }
        "richtext.delete_blocks" => {
            only_keys(op, &["entity_id", "blocks", "server_update", "lineage_id"])?;
            let (mut ok, mut ids, mut old) = (true, Vec::new(), Vec::new());
            for (i, b) in get_array(op, "blocks")?.iter().enumerate() {
                let block_id = get_str(b, "block_id")?.to_string();
                if !check_block(p, id, &st, &block_id, b.get("expect"), true, true, Some(i))? {
                    ok = false;
                    continue;
                }
                old.push((richtext::position_of(&before[&block_id]), before[&block_id].node.clone()));
                ids.push(block_id);
            }
            if !ok {
                return Ok((op.clone(), None));
            }
            (BlockOp::Delete { block_ids: ids }, Undo::Deleted(old))
        }
        _ => {
            only_keys(op, &["entity_id", "block_id", "expect", "position", "server_update", "lineage_id"])?;
            let block_id = get_str(op, "block_id")?.to_string();
            if !check_block(p, id, &st, &block_id, op.get("expect"), false, true, None)? {
                return Ok((op.clone(), None));
            }
            let position = Position::parse(op.get("position").ok_or_else(|| WsError::invalid_op("position required"))?)?;
            let back = richtext::position_of(&before[&block_id]);
            (BlockOp::Move { block_id: block_id.clone(), position }, Undo::Moved(block_id, back))
        }
    };
    let cand = candidate(p, id, &st);
    let update = match op.get("server_update").and_then(Value::as_str).filter(|_| p.env.replay) {
        // a replica applies the very bytes the backend produced, so both hold the same CRDT history
        Some(bytes) => {
            let bytes = B64.decode(bytes).map_err(|_| WsError::invalid_op("server_update must be base64"))?;
            if !richtext::import_update(&cand, &bytes)? {
                return Err(WsError::new(Code::BaseUnknown, "replica is missing earlier rich text updates"));
            }
            bytes
        }
        None => {
            cand.set_peer_id(p.env.peer_id).map_err(|e| WsError::io(format!("crdt peer: {e}")))?;
            let vv = cand.oplog_vv();
            richtext::apply_block_ops(&cand, &[bop])?;
            richtext::export_updates(&cand, &vv)?
        }
    };
    // the bytes the backend produced are what clients and replicas apply
    norm["server_update"] = json!(B64.encode(&update));
    norm["lineage_id"] = json!(st.meta.lineage_id);
    stage(p, e, st, cand, update)?;
    let now = p.ov.richtext(id)?.expect("staged");
    let inverse = match undo {
        Undo::Inserted(ids) => {
            let blocks: Vec<Value> = ids
                .iter()
                .map(|b| json!({ "block_id": b, "expect": { "hash": hash_of(&now, b), "struct_rev": seq } }))
                .collect();
            vec![json!({ "op": "richtext.delete_blocks", "entity_id": id, "blocks": blocks })]
        }
        Undo::Replaced(b, old) => {
            vec![json!({ "op": "richtext.replace_block", "entity_id": id, "block_id": b,
                         "expect": { "hash": hash_of(&now, &b) }, "node": old })]
        }
        Undo::Deleted(old) => old
            .into_iter()
            .map(|(pos, node)| json!({ "op": "richtext.insert_blocks", "entity_id": id, "position": pos.to_json(), "blocks": [node] }))
            .collect(),
        Undo::Moved(b, back) => {
            vec![json!({ "op": "richtext.move_block", "entity_id": id, "block_id": b,
                         "expect": { "struct_rev": seq }, "position": back.to_json() })]
        }
    };
    Ok((norm, Some(inverse)))
}
