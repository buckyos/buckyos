//! TableSource operations (design doc §3.4): records, cells, fields, options,
//! type migration. Cells are not entities; they are addressed by stable
//! `(record_id, field_id)` and guarded by per-cell version cells.

use crate::access::Cap;
use crate::error::{Code, WsError, WsResult};
use crate::id::check_id;
use crate::model::*;
use crate::order_key::{check_order_key, order_key_between};
use crate::plan::{get_array, get_str, only_keys, sel, OpResult, Planner};
use crate::types::{body_ref_edge, is_url_table, value_ref_edge};
use crate::value::{self, FieldDef, FieldType, OptionDef};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn dispatch(p: &mut Planner, name: &str, op: &Value) -> OpResult {
    match name {
        "table.insert_records" => insert_records(p, op),
        "table.delete_records" => delete_records(p, op),
        "table.restore_records" => restore_records(p, op),
        "table.set_values" => set_values(p, op, false),
        "table.unset_values" => set_values(p, op, true),
        "table.set_body" => set_body(p, op),
        "table.add_field" => add_field(p, op),
        "table.update_field" => update_field(p, op),
        "table.delete_field" => delete_field(p, op),
        "table.restore_field" => restore_field(p, op),
        "table.add_option" => add_option(p, op),
        "table.update_option" => update_option(p, op),
        "table.delete_option" => delete_option(p, op),
        "table.migrate_field" => migrate_field(p, op),
        "table.restore_migration" => restore_migration(p, op),
        _ => Err(WsError::invalid_op(format!("unknown operation {name:?}"))),
    }
}

fn cell_sel(record_id: &str, field_id: &str) -> Value {
    json!({ "kind": "table_cell", "record_id": record_id, "field_id": field_id })
}
fn record_sel(record_id: &str) -> Value {
    json!({ "kind": "table_record", "record_id": record_id })
}
fn field_sel(field_id: &str) -> Value {
    json!({ "kind": "table_field", "field_id": field_id })
}

/// The table entity for a data-modifying operation.
fn source(p: &mut Planner, op: &Value, data_write: bool) -> WsResult<EntityRow> {
    let id = get_str(op, "source_id")?;
    let e = p.entity_alive(id)?;
    if e.type_id != TYPE_TABLE {
        return Err(WsError::invalid_op(format!("{id} is not a table source")));
    }
    if let Some(d) = e.degraded() {
        return Err(WsError::new(Code::UnsupportedVersion, format!("table {id} cannot be edited ({d})")));
    }
    if data_write && is_url_table(&e) {
        return Err(WsError::sub(Code::InvalidOperation, "SOURCE_READ_ONLY", "URL query sources are read-only in this phase"));
    }
    p.need_lock(&e);
    Ok(e)
}

fn live_field(p: &Planner, source_id: &str, field_id: &str) -> WsResult<FieldRow> {
    match p.ov.field(source_id, field_id)? {
        Some(f) if f.alive() => Ok(f),
        Some(_) => Err(WsError::deleted(format!("field {field_id} is deleted"))
            .with_data(json!({ "entity_id": source_id, "selector": field_sel(field_id) }))),
        None => Err(WsError::not_found(format!("field {field_id} not found"))),
    }
}

/// Writes based on a stale field *type* are schema conflicts (collected, not fatal).
fn check_type_revs(p: &mut Planner, source_id: &str, op: &Value) -> WsResult<bool> {
    let Some(revs) = op.get("field_type_revs") else { return Ok(true) };
    let revs = revs.as_object().ok_or_else(|| WsError::invalid_op("field_type_revs must be an object"))?;
    let mut ok = true;
    for (fid, want) in revs {
        let current = p.ov.field(source_id, fid)?.filter(|f| f.alive()).map(|f| f.type_rev);
        let want = want.as_u64().ok_or_else(|| WsError::invalid_op("field_type_revs values must be integers"))?;
        if current != Some(want) && current != Some(p.seq()) {
            p.conflict(Code::SchemaConflict, None, source_id, sel(field_sel(fid)), json!(want), json!(current), None);
            ok = false;
        }
    }
    Ok(ok)
}

fn target_types(def: &FieldDef) -> Option<Vec<&str>> {
    def.target_types.as_ref().map(|t| t.iter().map(String::as_str).collect())
}

/// Normalize one cell value; object references must point at something the caller may read.
fn norm_value(p: &Planner, def: &FieldDef, v: &Value) -> WsResult<Value> {
    let n = def.normalize(v)?;
    if def.ty == FieldType::ObjectRef && !n.is_null() {
        p.check_ref_target(&n, target_types(def).as_deref())?;
    }
    Ok(n)
}

/// `unique` is enforced at the authoritative commit boundary, over live records.
fn check_unique(p: &Planner, source_id: &str, def: &FieldDef) -> WsResult<()> {
    if !def.unique {
        return Ok(());
    }
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut dup: Vec<String> = Vec::new();
    p.ov.scan_records(source_id, &mut |r| {
        if r.alive() {
            if let Some(v) = r.values.get(&def.field_id).filter(|v| !v.is_null()) {
                // canonical values of unique-capable types compare by their JSON text
                let key = match def.ty {
                    FieldType::Number => v.as_f64().map(|f| f.to_string()).unwrap_or_default(),
                    _ => v.to_string(),
                };
                if let Some(first) = seen.insert(key, r.record_id.clone()) {
                    dup.push(first);
                    dup.push(r.record_id.clone());
                }
            }
        }
        Ok(dup.len() < 100)
    })?;
    if dup.is_empty() {
        return Ok(());
    }
    dup.sort();
    dup.dedup();
    Err(WsError::sub(Code::InvalidOperation, "UNIQUE_VIOLATION", format!("duplicate value in unique field {}", def.field_id))
        .with_data(json!({ "field_id": def.field_id, "record_ids": dup })))
}

/// Provenance rule (§3.4.7), decided by the backend from the trusted origin:
/// program/agent writes with a run become `derived`; a human write over a
/// derived cell keeps the provenance and flags a manual override.
fn apply_meta(p: &Planner, rec: &mut RecordRow, field_id: &str, derived: Option<&Value>) {
    let by_program = matches!(p.env.origin.as_str(), "program" | "agent") && p.env.run_id.is_some();
    if by_program {
        let mut d = derived.and_then(Value::as_object).cloned().unwrap_or_default();
        d.insert("run_id".into(), json!(p.env.run_id));
        rec.meta.insert(field_id.to_string(), json!({ "derived": d }));
    } else if let Some(m) = rec.meta.get_mut(field_id) {
        if m.get("derived").is_some() && p.env.origin == "human" {
            m["manual_override"] = json!(true);
        }
    }
}

fn bump_fields(p: &mut Planner, source_id: &str, fields: &BTreeSet<String>) -> WsResult<()> {
    for fid in fields {
        if let Some(mut f) = p.ov.field(source_id, fid)? {
            f.values_rev = p.seq();
            p.ov.put_field(f);
        }
    }
    Ok(())
}

fn finish_source(p: &mut Planner, mut e: EntityRow, members: bool) {
    e.content_rev = p.seq();
    if members {
        e.key_revs.insert(MEMBERS_KEY.to_string(), p.seq());
    }
    p.ov.put_entity(e);
}

/// A body must be a live rich text owned by this table and not used by another record.
fn check_body(p: &Planner, source_id: &str, record_id: &str, body: &Value) -> WsResult<String> {
    let r = value::normalize_reference(body)?;
    let target = p
        .check_ref_target(&r, Some(&[TYPE_RICHTEXT]))?
        .ok_or_else(|| WsError::invalid_schema("body_ref must be a local reference"))?;
    if p.ov.edge(&target.entity_id)?.map(|e| e.parent_id) != Some(source_id.to_string()) {
        return Err(WsError::invalid_schema("a record body must be a rich text whose parent is this table"));
    }
    for other in p.ov.refs_to(&target.entity_id)? {
        if other.kind == "body" && other != body_ref_edge(source_id, record_id, &target.entity_id) {
            return Err(WsError::invalid_op("rich text is already the body of another record"));
        }
    }
    Ok(target.entity_id)
}

fn insert_records(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_type_revs", "records", "derived"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    let append_only = !p.can(&sid, Cap::Update)?;
    if append_only {
        p.require(&sid, Cap::Append)?;
    }
    let records = get_array(op, "records")?;
    if records.len() > p.env.limits.max_set_values {
        return Err(WsError::limit("too many records in one operation"));
    }
    let types_ok = check_type_revs(p, &sid, op)?;
    let fields: BTreeMap<String, FieldRow> =
        p.ov.fields(&sid)?.into_iter().filter(|f| f.alive()).map(|f| (f.field_id.clone(), f)).collect();
    let seq = p.seq();
    let (mut touched_fields, mut inverse, mut norm_records) = (BTreeSet::new(), Vec::new(), Vec::new());
    for (i, r) in records.iter().enumerate() {
        let rid = get_str(r, "record_id")?;
        check_id("record_id", rid)?;
        if p.ov.record(&sid, rid)?.is_some() {
            return Err(WsError::sub(Code::InvalidOperation, "ID_CONFLICT", format!("record_id {rid} already exists")).at(format!("/records/{i}")));
        }
        let mut rec = RecordRow {
            source_id: sid.clone(),
            record_id: rid.to_string(),
            values: Map::new(),
            revs: BTreeMap::new(),
            meta: Map::new(),
            body_entity_id: None,
            created_seq: seq,
            rev: seq,
            deleted_seq: None,
        };
        let empty = Map::new();
        let values = match r.get("values") {
            None => &empty,
            Some(Value::Object(o)) => o,
            Some(_) => return Err(WsError::invalid_schema("values must be an object").at(format!("/records/{i}/values"))),
        };
        for (fid, v) in values {
            let f = fields
                .get(fid)
                .ok_or_else(|| WsError::invalid_schema(format!("unknown field {fid}")).at(format!("/records/{i}/values/{fid}")))?;
            let n = norm_value(p, &f.def, v).map_err(|e| e.at(format!("/records/{i}/values/{fid}")))?;
            if f.def.ty == FieldType::ObjectRef && !n.is_null() {
                p.ov.add_ref(value_ref_edge(&sid, rid, fid, &n));
            }
            rec.values.insert(fid.clone(), n);
            rec.revs.insert(fid.clone(), seq);
            apply_meta(p, &mut rec, fid, op.get("derived"));
            touched_fields.insert(fid.clone());
        }
        for f in fields.values() {
            if f.def.required && !rec.values.contains_key(&f.field_id) {
                return Err(WsError::invalid_schema(format!("field {} is required", f.field_id)).at(format!("/records/{i}/values")));
            }
        }
        if let Some(meta) = r.get("meta").filter(|m| !m.is_null()) {
            // provenance travels with content; only import may supply it
            if !p.env.internal {
                return Err(WsError::invalid_op("meta is set by the backend"));
            }
            rec.meta = meta.as_object().cloned().ok_or_else(|| WsError::invalid_schema("meta must be an object"))?;
        }
        if let Some(body) = r.get("body_ref").filter(|b| !b.is_null()) {
            let body_id = check_body(p, &sid, rid, body)?;
            p.ov.add_ref(body_ref_edge(&sid, rid, &body_id));
            rec.body_entity_id = Some(body_id);
        }
        let mut n = json!({ "record_id": rid, "values": rec.values });
        if !rec.meta.is_empty() && p.env.internal {
            n["meta"] = Value::Object(rec.meta.clone());
        }
        if let Some(b) = &rec.body_entity_id {
            n["body_ref"] = json!({ "entity_id": b, "version": { "mode": "live_head" } });
        }
        norm_records.push(n);
        p.ov.put_record(rec);
        p.touch(&sid, sel(record_sel(rid)), "value");
        if append_only {
            p.read_denied.insert(rid.to_string());
        }
        inverse.push(json!({ "record_id": rid, "expect": { "rev": seq } }));
    }
    if !types_ok {
        return Ok((op.clone(), None));
    }
    for fid in &touched_fields {
        check_unique(p, &sid, &fields[fid].def)?;
    }
    bump_fields(p, &sid, &touched_fields)?;
    finish_source(p, src, true);
    let mut norm = op.clone();
    norm["records"] = Value::Array(norm_records);
    let inv = json!({ "op": "table.delete_records", "source_id": sid, "records": inverse, "owned_bodies": "detach" });
    Ok((norm, Some(vec![inv])))
}

fn live_record(p: &Planner, sid: &str, rid: &str) -> WsResult<RecordRow> {
    match p.ov.record(sid, rid)? {
        Some(r) if r.alive() => Ok(r),
        Some(_) => Err(WsError::deleted(format!("record {rid} is deleted"))
            .with_data(json!({ "entity_id": sid, "selector": record_sel(rid) }))),
        None => Err(WsError::not_found(format!("record {rid} not found"))),
    }
}

/// `table.set_values` / `table.unset_values`.
fn set_values(p: &mut Planner, op: &Value, unset: bool) -> OpResult {
    only_keys(op, &["source_id", "field_type_revs", "values", "derived"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Update)?;
    let items = get_array(op, "values")?;
    if items.is_empty() || items.len() > p.env.limits.max_set_values {
        return Err(WsError::limit(format!("values must contain 1..{} items", p.env.limits.max_set_values)));
    }
    let mut ok = check_type_revs(p, &sid, op)?;
    let seq = p.seq();
    let mut field_cache: BTreeMap<String, FieldRow> = BTreeMap::new();
    let (mut restore_set, mut restore_unset, mut norm_items) = (Vec::new(), Vec::new(), Vec::new());
    let mut touched_fields = BTreeSet::new();
    for (i, item) in items.iter().enumerate() {
        let rid = get_str(item, "record_id")?;
        let fid = get_str(item, "field_id")?;
        if !field_cache.contains_key(fid) {
            field_cache.insert(fid.to_string(), live_field(p, &sid, fid)?);
        }
        let f = &field_cache[fid];
        let mut rec = match live_record(p, &sid, rid) {
            Ok(r) => r,
            Err(e) if e.code == Code::TargetDeleted => {
                // a late write never resurrects a deleted record
                p.conflict(Code::TargetDeleted, Some(i), &sid, sel(cell_sel(rid, fid)), item.get("expect").cloned().unwrap_or(Value::Null), Value::Null, None);
                ok = false;
                continue;
            }
            Err(e) => return Err(e.at(format!("/values/{i}"))),
        };
        let old = rec.values.get(fid).cloned();
        let old_meta = rec.meta.get(fid).cloned();
        if !p.check_rev(item.get("expect"), rec.value_rev(fid), Some(i), &sid, sel(cell_sel(rid, fid)), || old.clone())? {
            ok = false;
            continue;
        }
        if unset {
            if f.def.required {
                return Err(WsError::invalid_schema(format!("field {fid} is required and cannot be unset")).at(format!("/values/{i}")));
            }
            if old.is_none() {
                continue; // already unset: nothing changes, nothing to undo
            }
            if f.def.ty == FieldType::ObjectRef {
                if let Some(o) = old.as_ref().filter(|o| o.is_object()) {
                    p.ov.del_ref(value_ref_edge(&sid, rid, fid, o));
                }
            }
            rec.values.remove(fid);
            rec.meta.remove(fid);
            norm_items.push(item.clone());
        } else {
            let raw = item.get("value").ok_or_else(|| WsError::invalid_op("value required").at(format!("/values/{i}/value")))?;
            let n = norm_value(p, &f.def, raw).map_err(|e| e.at(format!("/values/{i}/value")))?;
            if f.def.ty == FieldType::ObjectRef {
                if let Some(o) = old.as_ref().filter(|o| o.is_object()) {
                    p.ov.del_ref(value_ref_edge(&sid, rid, fid, o));
                }
                if n.is_object() {
                    p.ov.add_ref(value_ref_edge(&sid, rid, fid, &n));
                }
            }
            rec.values.insert(fid.to_string(), n.clone());
            match item.get("restore_meta") {
                // undo restores the exact provenance that was there
                Some(m) if p.env.internal => {
                    if m.is_null() {
                        rec.meta.remove(fid);
                    } else {
                        rec.meta.insert(fid.to_string(), m.clone());
                    }
                }
                Some(_) => return Err(WsError::invalid_op("restore_meta is internal")),
                None => apply_meta(p, &mut rec, fid, op.get("derived")),
            }
            let mut ni = item.clone();
            ni["value"] = n;
            norm_items.push(ni);
        }
        rec.revs.insert(fid.to_string(), seq);
        rec.rev = seq;
        p.ov.put_record(rec);
        touched_fields.insert(fid.to_string());
        p.touch(&sid, sel(cell_sel(rid, fid)), "value");
        match old {
            Some(v) => restore_set.push(json!({ "record_id": rid, "field_id": fid, "value": v,
                                                "restore_meta": old_meta, "expect": { "rev": seq } })),
            None => restore_unset.push(json!({ "record_id": rid, "field_id": fid, "expect": { "rev": seq } })),
        }
    }
    if !ok {
        return Ok((op.clone(), None));
    }
    for fid in &touched_fields {
        check_unique(p, &sid, &field_cache[fid].def)?;
    }
    bump_fields(p, &sid, &touched_fields)?;
    finish_source(p, src, false);
    let mut norm = op.clone();
    norm["values"] = Value::Array(norm_items);
    let mut inverse = Vec::new();
    if !restore_set.is_empty() {
        inverse.push(json!({ "op": "table.set_values", "source_id": sid, "values": restore_set }));
    }
    if !restore_unset.is_empty() {
        inverse.push(json!({ "op": "table.unset_values", "source_id": sid, "values": restore_unset }));
    }
    Ok((norm, Some(inverse)))
}

fn record_refs(p: &Planner, rec: &RecordRow) -> WsResult<Vec<RefEdge>> {
    let mut out = Vec::new();
    for f in p.ov.fields(&rec.source_id)? {
        if f.alive() && f.def.ty == FieldType::ObjectRef {
            if let Some(v) = rec.values.get(&f.field_id).filter(|v| v.is_object()) {
                out.push(value_ref_edge(&rec.source_id, &rec.record_id, &f.field_id, v));
            }
        }
    }
    Ok(out)
}

fn delete_records(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "records", "owned_bodies"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Delete)?;
    let delete_bodies = match op.get("owned_bodies").and_then(Value::as_str).unwrap_or("delete") {
        "delete" => true,
        "detach" => false,
        _ => return Err(WsError::invalid_op("owned_bodies must be delete or detach")),
    };
    let seq = p.seq();
    let (mut ok, mut inverse_records, mut inverse_bodies, mut touched_fields) = (true, Vec::new(), Vec::new(), BTreeSet::new());
    for (i, item) in get_array(op, "records")?.iter().enumerate() {
        let rid = get_str(item, "record_id")?;
        let mut rec = match live_record(p, &sid, rid) {
            Ok(r) => r,
            Err(e) if e.code == Code::TargetDeleted => {
                p.conflict(Code::TargetDeleted, Some(i), &sid, sel(record_sel(rid)), Value::Null, Value::Null, None);
                ok = false;
                continue;
            }
            Err(e) => return Err(e),
        };
        // conservative on purpose: do not delete a record someone just changed
        if !p.check_rev(item.get("expect"), rec.rev, Some(i), &sid, sel(record_sel(rid)), || None)? {
            ok = false;
            continue;
        }
        for r in record_refs(p, &rec)? {
            p.ov.del_ref(r);
        }
        if let Some(body) = rec.body_entity_id.clone() {
            p.ov.del_ref(body_ref_edge(&sid, rid, &body));
            if delete_bodies {
                if let Some(mut b) = p.ov.entity(&body)?.filter(|b| b.alive()) {
                    b.deleted_seq = Some(seq);
                    b.life_rev = seq;
                    p.ov.put_entity(b);
                    p.touch(&body, None, "deleted");
                    inverse_bodies.push(json!({ "op": "entity.restore", "entity_id": body, "expect": { "rev": seq } }));
                }
            } else {
                rec.body_entity_id = None;
            }
        }
        touched_fields.extend(rec.values.keys().cloned());
        rec.deleted_seq = Some(seq);
        rec.rev = seq;
        p.ov.put_record(rec);
        p.touch(&sid, sel(record_sel(rid)), "value");
        inverse_records.push(json!({ "record_id": rid, "expect": { "rev": seq } }));
    }
    if !ok {
        return Ok((op.clone(), None));
    }
    bump_fields(p, &sid, &touched_fields)?;
    finish_source(p, src, true);
    let mut inverse = inverse_bodies;
    inverse.push(json!({ "op": "table.restore_records", "source_id": sid, "records": inverse_records }));
    Ok((op.clone(), Some(inverse)))
}

/// Internal: undo of `delete_records`.
fn restore_records(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "records"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Delete)?;
    let seq = p.seq();
    let fields: Vec<FieldRow> = p.ov.fields(&sid)?.into_iter().filter(|f| f.alive()).collect();
    let (mut ok, mut inverse, mut touched_fields) = (true, Vec::new(), BTreeSet::new());
    for (i, item) in get_array(op, "records")?.iter().enumerate() {
        let rid = get_str(item, "record_id")?;
        let mut rec = p.ov.record(&sid, rid)?.ok_or_else(|| WsError::not_found(format!("record {rid} not found")))?;
        if rec.alive() {
            return Err(WsError::invalid_op(format!("record {rid} is not deleted")));
        }
        if !p.check_rev(item.get("expect"), rec.rev, Some(i), &sid, sel(record_sel(rid)), || None)? {
            ok = false;
            continue;
        }
        for f in &fields {
            if f.def.required && !rec.values.contains_key(&f.field_id) {
                return Err(WsError::new(Code::SchemaConflict, format!("record {rid} lacks required field {}", f.field_id)));
            }
            if f.def.ty == FieldType::ObjectRef {
                if let Some(v) = rec.values.get(&f.field_id).filter(|v| v.is_object()) {
                    p.check_ref_target(v, target_types(&f.def).as_deref())?;
                }
            }
        }
        rec.deleted_seq = None;
        rec.rev = seq;
        if let Some(body) = rec.body_entity_id.clone() {
            // re-attach the body only if it is alive again and still unclaimed
            let free = p.ov.entity(&body)?.is_some_and(|b| b.alive()) && !p.ov.refs_to(&body)?.iter().any(|r| r.kind == "body");
            if free {
                p.ov.add_ref(body_ref_edge(&sid, rid, &body));
            } else {
                rec.body_entity_id = None;
            }
        }
        touched_fields.extend(rec.values.keys().cloned());
        let refs = record_refs(p, &rec)?;
        p.ov.put_record(rec);
        for r in refs {
            p.ov.add_ref(r);
        }
        p.touch(&sid, sel(record_sel(rid)), "value");
        inverse.push(json!({ "record_id": rid, "expect": { "rev": seq } }));
    }
    if !ok {
        return Ok((op.clone(), None));
    }
    for f in &fields {
        check_unique(p, &sid, &f.def)?;
    }
    bump_fields(p, &sid, &touched_fields)?;
    finish_source(p, src, true);
    let inv = json!({ "op": "table.delete_records", "source_id": sid, "records": inverse, "owned_bodies": "detach" });
    Ok((op.clone(), Some(vec![inv])))
}

fn set_body(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "record_id", "body_ref", "expect"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Update)?;
    let rid = get_str(op, "record_id")?;
    let mut rec = live_record(p, &sid, rid)?;
    if !p.check_rev(op.get("expect"), rec.rev, None, &sid, sel(record_sel(rid)), || None)? {
        return Ok((op.clone(), None));
    }
    let old = rec.body_entity_id.clone();
    if let Some(o) = &old {
        p.ov.del_ref(body_ref_edge(&sid, rid, o));
    }
    rec.body_entity_id = match op.get("body_ref").filter(|b| !b.is_null()) {
        Some(b) => {
            let id = check_body(p, &sid, rid, b)?;
            p.ov.add_ref(body_ref_edge(&sid, rid, &id));
            Some(id)
        }
        None => None,
    };
    let seq = p.seq();
    rec.rev = seq;
    p.ov.put_record(rec);
    p.touch(&sid, sel(record_sel(rid)), "value");
    finish_source(p, src, false);
    let old_ref = old.map(|o| json!({ "entity_id": o })).unwrap_or(Value::Null);
    let inv = json!({ "op": "table.set_body", "source_id": sid, "record_id": rid, "body_ref": old_ref, "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inv])))
}

fn name_free(p: &Planner, sid: &str, name: &str, except: &str) -> WsResult<()> {
    if p.ov.fields(sid)?.iter().any(|f| f.alive() && f.field_id != except && f.def.name == name) {
        return Err(WsError::sub(Code::InvalidOperation, "NAME_CONFLICT", format!("field name {name:?} already used")));
    }
    Ok(())
}

fn add_field(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field", "order_key", "backfill"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let def = FieldDef::from_value(op.get("field").ok_or_else(|| WsError::invalid_op("field required"))?)?;
    if p.ov.field(&sid, &def.field_id)?.is_some() {
        // tombstoned ids are never reused; undo is the only way to revive one
        return Err(WsError::sub(Code::InvalidOperation, "ID_CONFLICT", format!("field_id {} already exists", def.field_id)));
    }
    name_free(p, &sid, &def.name, "")?;
    let order_key = match op.get("order_key").and_then(Value::as_str) {
        Some(k) => {
            check_order_key(k)?;
            k.to_string()
        }
        None => {
            let last = p.ov.fields(&sid)?.last().map(|f| f.order_key.clone());
            order_key_between(last.as_deref(), None)?
        }
    };
    let seq = p.seq();
    let backfill = match op.get("backfill") {
        Some(v) => Some(norm_value(p, &def, v)?),
        None => None,
    };
    let mut filled = 0usize;
    let mut rows = Vec::new();
    p.ov.scan_records(&sid, &mut |r| {
        if r.alive() {
            rows.push(r.clone());
        }
        Ok(true)
    })?;
    if def.required && backfill.is_none() && !rows.is_empty() {
        return Err(WsError::invalid_op("a required field needs a backfill value for existing records"));
    }
    if let Some(b) = &backfill {
        for mut r in rows {
            r.values.insert(def.field_id.clone(), b.clone());
            r.revs.insert(def.field_id.clone(), seq);
            r.rev = seq;
            if def.ty == FieldType::ObjectRef && b.is_object() {
                p.ov.add_ref(value_ref_edge(&sid, &r.record_id, &def.field_id, b));
            }
            p.ov.put_record(r);
            filled += 1;
        }
    }
    p.ov.put_field(FieldRow {
        source_id: sid.clone(),
        field_id: def.field_id.clone(),
        def: def.clone(),
        order_key: order_key.clone(),
        def_rev: seq,
        type_rev: seq,
        values_rev: if filled > 0 { seq } else { 0 },
        deleted_seq: None,
    });
    check_unique(p, &sid, &def)?;
    p.touch(&sid, sel(field_sel(&def.field_id)), "schema");
    finish_source(p, src, false);
    let mut norm = op.clone();
    norm["field"] = def.to_value();
    norm["order_key"] = json!(order_key);
    if let Some(b) = backfill {
        norm["backfill"] = b;
    }
    let inv = json!({ "op": "table.delete_field", "source_id": sid, "field_id": def.field_id, "expect": { "rev": seq } });
    Ok((norm, Some(vec![inv])))
}

/// Records violating a (tightened) constraint: at most 100 ids plus the total.
fn violations(p: &Planner, sid: &str, def: &FieldDef) -> WsResult<(usize, Vec<String>)> {
    let (mut total, mut sample) = (0usize, Vec::new());
    p.ov.scan_records(sid, &mut |r| {
        if r.alive() {
            let v = r.values.get(&def.field_id);
            let bad = (def.required && v.is_none()) || (!def.nullable && v.is_some_and(Value::is_null));
            if bad {
                total += 1;
                if sample.len() < 100 {
                    sample.push(r.record_id.clone());
                }
            }
        }
        Ok(true)
    })?;
    Ok((total, sample))
}

fn update_field(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_id", "changes", "expect"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = live_field(p, &sid, fid)?;
    if !p.check_rev(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || Some(f.def.to_value()))? {
        return Ok((op.clone(), None));
    }
    let changes = op.get("changes").and_then(Value::as_object).filter(|c| !c.is_empty()).ok_or_else(|| WsError::invalid_op("changes required"))?;
    let old = f.clone();
    let mut restore = Map::new();
    let mut tightened = false;
    for (k, v) in changes {
        match k.as_str() {
            "name" => {
                let n = v.as_str().ok_or_else(|| WsError::invalid_op("name must be a string"))?;
                name_free(p, &sid, n, fid)?;
                restore.insert(k.clone(), json!(f.def.name));
                f.def.name = n.to_string();
            }
            "description" => {
                restore.insert(k.clone(), json!(f.def.description));
                f.def.description = v.as_str().map(str::to_string);
            }
            "maintained_by" => {
                restore.insert(k.clone(), json!(f.def.maintained_by.clone().unwrap_or_else(|| "human".into())));
                f.def.maintained_by = v.as_str().filter(|s| *s != "human").map(str::to_string);
            }
            "order_key" => {
                let key = v.as_str().ok_or_else(|| WsError::invalid_op("order_key must be a string"))?;
                check_order_key(key)?;
                restore.insert(k.clone(), json!(f.order_key));
                f.order_key = key.to_string();
            }
            "required" | "nullable" | "unique" => {
                let b = v.as_bool().ok_or_else(|| WsError::invalid_op(format!("{k} must be a boolean")))?;
                let slot = match k.as_str() {
                    "required" => &mut f.def.required,
                    "nullable" => &mut f.def.nullable,
                    _ => &mut f.def.unique,
                };
                restore.insert(k.clone(), json!(*slot));
                // required/unique tighten when switched on; nullable tightens when switched off
                tightened |= if k == "nullable" { *slot && !b } else { !*slot && b };
                *slot = b;
            }
            _ => return Err(WsError::invalid_op(format!("field property {k} cannot be changed here"))),
        }
    }
    f.def.validate()?;
    let seq = p.seq();
    if tightened {
        let (total, sample) = violations(p, &sid, &f.def)?;
        if total > 0 {
            return Err(WsError::invalid_op("existing values violate the tightened constraint")
                .with_data(json!({ "field_id": fid, "violating_records": sample, "total": total })));
        }
        f.type_rev = seq;
    }
    f.def_rev = seq;
    let def = f.def.clone();
    p.ov.put_field(f);
    check_unique(p, &sid, &def)?;
    let renamed_only = changes.keys().all(|k| matches!(k.as_str(), "name" | "description" | "order_key" | "maintained_by"));
    p.touch(&sid, sel(field_sel(fid)), if renamed_only { "renamed" } else { "schema" });
    finish_source(p, src, false);
    let _ = old;
    let inv = json!({ "op": "table.update_field", "source_id": sid, "field_id": fid, "changes": restore, "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inv])))
}

/// Value references held by live records in one object_ref field.
fn field_refs(p: &Planner, sid: &str, f: &FieldRow) -> WsResult<Vec<RefEdge>> {
    let mut out = Vec::new();
    if f.def.ty == FieldType::ObjectRef {
        p.ov.scan_records(sid, &mut |r| {
            if r.alive() {
                if let Some(v) = r.values.get(&f.field_id).filter(|v| v.is_object()) {
                    out.push(value_ref_edge(sid, &r.record_id, &f.field_id, v));
                }
            }
            Ok(true)
        })?;
    }
    Ok(out)
}

fn delete_field(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_id", "expect"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = live_field(p, &sid, fid)?;
    if !p.check_rev(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || Some(f.def.to_value()))? {
        return Ok((op.clone(), None));
    }
    if src.payload.get("title_field_id").and_then(Value::as_str) == Some(fid) {
        return Err(WsError::invalid_op("the title field cannot be deleted"));
    }
    for r in field_refs(p, &sid, &f)? {
        p.ov.del_ref(r);
    }
    let seq = p.seq();
    // tombstone: the values stay in storage, unreadable, so undo restores them losslessly
    f.deleted_seq = Some(seq);
    f.def_rev = seq;
    p.ov.put_field(f);
    p.touch(&sid, sel(field_sel(fid)), "schema");
    // views are never rewritten; report the ones whose configuration now dangles
    for r in p.ov.refs_to(&sid)? {
        if r.kind == "bind" {
            if let Some(cell) = p.ov.entity(&r.src_entity_id)?.filter(|c| c.alive()) {
                if Value::Object(cell.payload.clone()).to_string().contains(&format!("\"{fid}\"")) {
                    let t = crate::plan::Touched { entity_id: cell.entity_id.clone(), selector: None, change: "view", rev: cell.content_rev };
                    p.touched.push(t);
                }
            }
        }
    }
    finish_source(p, src, false);
    let inv = json!({ "op": "table.restore_field", "source_id": sid, "field_id": fid, "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inv])))
}

/// Internal: undo of `delete_field`. Re-validates in the current state and
/// reports every obstacle as a conflict of the undo; nothing is silently adjusted.
fn restore_field(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_id", "expect"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = p.ov.field(&sid, fid)?.ok_or_else(|| WsError::not_found(format!("field {fid} not found")))?;
    if f.alive() {
        return Err(WsError::invalid_op(format!("field {fid} is not deleted")));
    }
    if !p.check_rev(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || None)? {
        return Ok((op.clone(), None));
    }
    name_free(p, &sid, &f.def.name, fid)?;
    let seq = p.seq();
    f.deleted_seq = None;
    f.def_rev = seq;
    f.type_rev = seq; // writes queued against the pre-deletion type now get SCHEMA_CONFLICT
    f.values_rev = seq;
    let (missing, _) = violations(p, &sid, &f.def)?;
    if f.def.required && missing > 0 {
        return Err(WsError::new(Code::SchemaConflict, format!("{missing} records created meanwhile lack required field {fid}"))
            .with_data(json!({ "entity_id": sid, "selector": field_sel(fid), "missing": missing })));
    }
    let refs = field_refs(p, &sid, &f)?;
    for r in &refs {
        if p.ov.entity(&r.dst_entity_id)?.map_or(true, |t| !t.alive()) && r.dst_workspace_id.is_empty() {
            return Err(WsError::new(Code::ReferenceBroken, format!("residual reference to deleted entity {}", r.dst_entity_id)));
        }
    }
    let def = f.def.clone();
    p.ov.put_field(f);
    for r in refs {
        p.ov.add_ref(r);
    }
    check_unique(p, &sid, &def)?;
    p.touch(&sid, sel(field_sel(fid)), "schema");
    finish_source(p, src, false);
    let inv = json!({ "op": "table.delete_field", "source_id": sid, "field_id": fid, "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inv])))
}

fn select_field(p: &Planner, sid: &str, fid: &str) -> WsResult<FieldRow> {
    let f = live_field(p, sid, fid)?;
    if f.def.options.is_none() {
        return Err(WsError::invalid_op(format!("field {fid} has no options")));
    }
    Ok(f)
}

fn add_option(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_id", "option", "index", "expect"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = select_field(p, &sid, fid)?;
    if !p.check_rev_opt(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || None)? {
        return Ok((op.clone(), None));
    }
    let option: OptionDef = serde_json::from_value(op.get("option").cloned().unwrap_or(Value::Null))
        .map_err(|e| WsError::invalid_schema(format!("bad option: {e}")))?;
    check_id("option_id", &option.option_id)?;
    if f.def.has_option(&option.option_id) {
        return Err(WsError::sub(Code::InvalidOperation, "ID_CONFLICT", format!("option_id {} already exists", option.option_id)));
    }
    let opts = f.def.options.as_mut().unwrap();
    let index = op.get("index").and_then(Value::as_u64).map_or(opts.len(), |i| (i as usize).min(opts.len()));
    let oid = option.option_id.clone();
    opts.insert(index, option);
    f.def.validate()?;
    let seq = p.seq();
    f.def_rev = seq; // adding an option never invalidates existing values: type_rev stays
    p.ov.put_field(f);
    p.touch(&sid, sel(field_sel(fid)), "renamed");
    finish_source(p, src, false);
    let inv = json!({ "op": "table.delete_option", "source_id": sid, "field_id": fid, "option_id": oid,
                      "on_values": "reject_if_used", "expect": { "rev": seq } });
    Ok((op.clone(), Some(vec![inv])))
}

fn update_option(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_id", "option_id", "label", "index", "expect"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = select_field(p, &sid, fid)?;
    if !p.check_rev(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || None)? {
        return Ok((op.clone(), None));
    }
    let oid = get_str(op, "option_id")?;
    let pos = f.def.option_index(oid).ok_or_else(|| WsError::not_found(format!("option {oid} not found")))?;
    let seq = p.seq();
    let opts = f.def.options.as_mut().unwrap();
    let mut inv = json!({ "op": "table.update_option", "source_id": sid, "field_id": fid, "option_id": oid, "expect": { "rev": seq } });
    if let Some(l) = op.get("label") {
        let l = l.as_str().filter(|s| !s.is_empty()).ok_or_else(|| WsError::invalid_op("label must be a non-empty string"))?;
        inv["label"] = json!(opts[pos].label);
        opts[pos].label = l.to_string();
    }
    if let Some(i) = op.get("index").and_then(Value::as_u64) {
        let o = opts.remove(pos);
        opts.insert((i as usize).min(opts.len()), o);
        inv["index"] = json!(pos);
    }
    f.def_rev = seq; // labels are display only: business values are untouched
    p.ov.put_field(f);
    p.touch(&sid, sel(field_sel(fid)), "renamed");
    finish_source(p, src, false);
    Ok((op.clone(), Some(vec![inv])))
}

fn delete_option(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_id", "option_id", "on_values", "expect"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = select_field(p, &sid, fid)?;
    if !p.check_rev(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || None)? {
        return Ok((op.clone(), None));
    }
    let oid = get_str(op, "option_id")?.to_string();
    let pos = f.def.option_index(&oid).ok_or_else(|| WsError::not_found(format!("option {oid} not found")))?;
    let replace = match op.get("on_values") {
        None => None,
        Some(Value::String(s)) if s == "reject_if_used" => None,
        Some(Value::String(s)) if s == "unset" => Some(None),
        Some(Value::Object(o)) if o.len() == 1 && o.get("replace_with").is_some_and(Value::is_string) => {
            let r = o["replace_with"].as_str().unwrap().to_string();
            if r == oid || !f.def.has_option(&r) {
                return Err(WsError::invalid_op("replace_with must name another existing option"));
            }
            Some(Some(r))
        }
        _ => return Err(WsError::invalid_op("on_values must be reject_if_used, unset or { replace_with }")),
    };
    let multi = f.def.ty == FieldType::MultiSelect;
    let uses = |v: &Value| -> bool {
        if multi {
            v.as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(oid.as_str())))
        } else {
            v.as_str() == Some(oid.as_str())
        }
    };
    let mut users = Vec::new();
    p.ov.scan_records(&sid, &mut |r| {
        if r.alive() && r.values.get(fid).is_some_and(&uses) {
            users.push(r.clone());
        }
        Ok(true)
    })?;
    let Some(replace) = replace.or(if users.is_empty() { Some(None) } else { None }) else {
        return Err(WsError::invalid_op(format!("option {oid} is used by {} records", users.len()))
            .with_data(json!({ "used_by": users.len() })));
    };
    if replace.is_none() && f.def.required && !multi && !users.is_empty() {
        return Err(WsError::invalid_op("cannot unset values of a required field"));
    }
    let seq = p.seq();
    let removed = f.def.options.as_mut().unwrap().remove(pos);
    let mut restore = Vec::new();
    for mut r in users {
        let old = r.values[fid].clone();
        restore.push(json!({ "record_id": r.record_id, "field_id": fid, "value": old,
                             "restore_meta": r.meta.get(fid).cloned(), "expect": { "rev": seq } }));
        if multi {
            let mut set: BTreeSet<String> = old.as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect();
            set.remove(&oid);
            if let Some(rep) = &replace {
                set.insert(rep.clone());
            }
            r.values.insert(fid.to_string(), json!(set));
        } else {
            match &replace {
                Some(rep) => {
                    r.values.insert(fid.to_string(), json!(rep));
                }
                None => {
                    r.values.remove(fid);
                    r.meta.remove(fid);
                }
            }
        }
        r.revs.insert(fid.to_string(), seq);
        r.rev = seq;
        p.touch(&sid, sel(cell_sel(&r.record_id, fid)), "value");
        p.ov.put_record(r);
    }
    f.def_rev = seq;
    f.type_rev = seq; // removing an option changes which values are legal
    if !restore.is_empty() {
        f.values_rev = seq;
    }
    p.ov.put_field(f);
    p.touch(&sid, sel(field_sel(fid)), "schema");
    finish_source(p, src, false);
    let mut inverse = vec![json!({ "op": "table.add_option", "source_id": sid, "field_id": fid,
                                   "option": { "option_id": removed.option_id, "label": removed.label }, "index": pos,
                                   "expect": { "rev": seq } })];
    if !restore.is_empty() {
        inverse.push(json!({ "op": "table.set_values", "source_id": sid, "values": restore }));
    }
    Ok((op.clone(), Some(inverse)))
}

fn migrate_field(p: &mut Planner, op: &Value) -> OpResult {
    only_keys(op, &["source_id", "field_id", "expect", "to", "on_failure"])?;
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = live_field(p, &sid, fid)?;
    if !p.check_rev(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || Some(f.def.to_value()))? {
        return Ok((op.clone(), None));
    }
    let to = op.get("to").and_then(Value::as_object).ok_or_else(|| WsError::invalid_op("to required"))?;
    let unset_failures = match op.get("on_failure").and_then(Value::as_str).unwrap_or("reject") {
        "reject" => false,
        "unset" => true,
        _ => return Err(WsError::invalid_op("on_failure must be reject or unset")),
    };
    let from = f.def.clone();
    let mut to_json = from.to_value();
    {
        let o = to_json.as_object_mut().unwrap();
        for k in ["scale", "options", "target_types"] {
            o.remove(k);
        }
        for (k, v) in to {
            if !matches!(k.as_str(), "type" | "scale") {
                return Err(WsError::invalid_op(format!("migration target property {k} is not supported")));
            }
            o.insert(k.clone(), v.clone());
        }
    }
    let target = FieldDef::from_value(&to_json)?;
    if !value::migration_supported(from.ty, target.ty) {
        return Err(WsError::sub(Code::InvalidOperation, "MIGRATION_UNSUPPORTED",
            format!("{} → {} is not supported", from.ty.as_str(), target.ty.as_str())));
    }
    if unset_failures && target.required {
        return Err(WsError::invalid_op("on_failure: unset is not allowed on a required field"));
    }
    if src.payload.get("title_field_id").and_then(Value::as_str) == Some(fid) && target.ty != FieldType::Text {
        return Err(WsError::invalid_op("the title field must stay a text field"));
    }
    let seq = p.seq();
    let (mut total, mut unset, mut failing, mut sample) = (0usize, 0usize, 0usize, Vec::new());
    let mut rewritten: Vec<(RecordRow, Option<Value>)> = Vec::new();
    p.ov.scan_records(&sid, &mut |r| {
        if !r.alive() {
            return Ok(true);
        }
        total += 1;
        match r.values.get(fid) {
            None => unset += 1,
            Some(v) => match value::migrate_value(&from, &target, v) {
                Ok(n) => rewritten.push((r.clone(), Some(n))),
                Err(_) => {
                    failing += 1;
                    if sample.len() < 100 {
                        sample.push(json!({ "record_id": r.record_id, "value": v }));
                    }
                    rewritten.push((r.clone(), None));
                }
            },
        }
        Ok(true)
    })?;
    if total > p.env.limits.max_migrate_rows {
        return Err(WsError::limit(format!("migration of more than {} rows is not supported", p.env.limits.max_migrate_rows)));
    }
    let report = json!({ "total": total, "convertible": total - unset - failing,
                         "failing": { "count": failing, "sample": sample }, "unset": unset });
    p.reports.push(json!({ "op_index": p.op_index, "kind": "migration", "field_id": fid, "report": report }));
    if failing > 0 && !unset_failures {
        return Err(WsError::invalid_op(format!("{failing} values cannot be converted")).with_data(json!({ "report": report })));
    }
    let mut cells = Vec::new();
    for (mut r, new) in rewritten {
        let old = r.values.get(fid).cloned();
        cells.push(json!({ "record_id": r.record_id, "value": old, "meta": r.meta.get(fid).cloned() }));
        match new {
            Some(n) => {
                r.values.insert(fid.to_string(), n);
            }
            None => {
                r.values.remove(fid);
                r.meta.remove(fid);
            }
        }
        r.revs.insert(fid.to_string(), seq);
        r.rev = seq;
        p.ov.put_record(r);
    }
    f.def = target.clone();
    f.def_rev = seq;
    f.type_rev = seq;
    f.values_rev = seq;
    p.ov.put_field(f);
    check_unique(p, &sid, &target)?;
    p.touch(&sid, sel(field_sel(fid)), "schema");
    finish_source(p, src, false);
    let inv = json!({ "op": "table.restore_migration", "source_id": sid, "field_id": fid, "expect": { "rev": seq },
                      "def": from.to_value(), "cells": cells,
                      "redo": { "to": op["to"], "on_failure": op.get("on_failure").cloned().unwrap_or(json!("reject")) } });
    let inverse = if inv.to_string().len() > p.env.limits.max_inverse_bytes {
        p.warnings.push(json!({ "op_index": p.op_index, "code": "NOT_COMPENSABLE", "detail": "migration undo data exceeds the limit" }));
        None
    } else {
        Some(vec![inv])
    };
    Ok((op.clone(), inverse))
}

/// Internal: undo of `migrate_field`. Every rewritten cell must be untouched since.
fn restore_migration(p: &mut Planner, op: &Value) -> OpResult {
    let src = source(p, op, true)?;
    let sid = src.entity_id.clone();
    p.require(&sid, Cap::Structure)?;
    let fid = get_str(op, "field_id")?;
    let mut f = live_field(p, &sid, fid)?;
    if !p.check_rev(op.get("expect"), f.def_rev, None, &sid, sel(field_sel(fid)), || None)? {
        return Ok((op.clone(), None));
    }
    let migrated_at = op["expect"]["rev"].as_u64().unwrap_or(0);
    let def = FieldDef::from_value(&op["def"])?;
    let seq = p.seq();
    let mut ok = true;
    for (i, c) in get_array(op, "cells")?.iter().enumerate() {
        let rid = get_str(c, "record_id")?;
        let Some(mut r) = p.ov.record(&sid, rid)?.filter(|r| r.alive()) else {
            p.conflict(Code::TargetDeleted, Some(i), &sid, sel(record_sel(rid)), json!(migrated_at), Value::Null, None);
            ok = false;
            continue;
        };
        if r.value_rev(fid) != migrated_at {
            let cur = r.values.get(fid).cloned();
            p.conflict(Code::RevisionConflict, Some(i), &sid, sel(cell_sel(rid, fid)), json!(migrated_at), json!(r.value_rev(fid)), cur);
            ok = false;
            continue;
        }
        match c.get("value").filter(|v| !v.is_null() || def.nullable) {
            Some(v) => {
                r.values.insert(fid.to_string(), v.clone());
            }
            None => {
                r.values.remove(fid);
            }
        }
        match c.get("meta").filter(|m| !m.is_null()) {
            Some(m) => {
                r.meta.insert(fid.to_string(), m.clone());
            }
            None => {
                r.meta.remove(fid);
            }
        }
        r.revs.insert(fid.to_string(), seq);
        r.rev = seq;
        p.ov.put_record(r);
    }
    if !ok {
        return Ok((op.clone(), None));
    }
    f.def = def;
    f.def_rev = seq;
    f.type_rev = seq;
    f.values_rev = seq;
    p.ov.put_field(f);
    p.touch(&sid, sel(field_sel(fid)), "schema");
    finish_source(p, src, false);
    let inv = json!({ "op": "table.migrate_field", "source_id": sid, "field_id": fid, "expect": { "rev": seq },
                      "to": op["redo"]["to"], "on_failure": op["redo"]["on_failure"] });
    Ok((op.clone(), Some(vec![inv])))
}
