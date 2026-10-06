//! Browser facade of `aiworkspace-core`: the same validation, planning,
//! encoding and rich text code the backend runs, exposed to the Replica Worker.
//! JSON travels as strings; binary data as `Uint8Array`.
//!
//! Failures throw a JS `Error` whose message is the JSON of the structured
//! error (`{ code, retryable, detail, ... }`).

use aiworkspace_core::access::Access;
use aiworkspace_core::canonical;
use aiworkspace_core::model::BlockIndex;
use aiworkspace_core::replica::{dump_rows, load_rows, Replica as CoreReplica};
use aiworkspace_core::richtext;
use aiworkspace_core::value::FieldDef;
use aiworkspace_core::{read, WsError};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

fn fail(e: WsError) -> JsError {
    JsError::new(&e.to_json().to_string())
}

fn parse(text: &str) -> Result<Value, JsError> {
    canonical::parse_strict(text).map_err(fail)
}

#[wasm_bindgen]
pub fn core_version() -> String {
    format!("{} (loro {})", env!("CARGO_PKG_VERSION"), richtext::engine_version())
}

/// RFC 8785 text of a JSON value, after the strict pre-checks (V01).
#[wasm_bindgen]
pub fn canonical_json(json_text: &str) -> Result<String, JsError> {
    canonical::canonical_json(&parse(json_text)?).map_err(fail)
}

/// `type:hex` ObjectId of a JSON NamedObject.
#[wasm_bindgen]
pub fn object_id(obj_type: &str, json_text: &str) -> Result<String, JsError> {
    canonical::named_object(obj_type, &parse(json_text)?).map(|(id, _)| id).map_err(fail)
}

#[wasm_bindgen]
pub fn verify_object(id: &str, canonical_text: &str) -> Result<(), JsError> {
    canonical::verify_named_object(id, canonical_text).map(|_| ()).map_err(fail)
}

#[wasm_bindgen]
pub fn chunk_id(data: &[u8]) -> String {
    canonical::chunk_id(data)
}

/// FileObject id of a single-chunk file with these bytes.
#[wasm_bindgen]
pub fn file_object_id(data: &[u8]) -> Result<String, JsError> {
    canonical::file_object(data.len() as u64, &canonical::chunk_id(data)).map(|(id, _)| id).map_err(fail)
}

#[wasm_bindgen]
pub fn order_key_between(a: Option<String>, b: Option<String>) -> Result<String, JsError> {
    aiworkspace_core::order_key::order_key_between(a.as_deref(), b.as_deref()).map_err(fail)
}

/// Normalize a value for a field definition (same rules as a write).
#[wasm_bindgen]
pub fn normalize_value(field_def_json: &str, value_json: &str) -> Result<String, JsError> {
    let def = FieldDef::from_value(&parse(field_def_json)?).map_err(fail)?;
    def.normalize(&parse(value_json)?).map(|v| v.to_string()).map_err(fail)
}

/// The rich text schema definition both sides are generated from.
#[wasm_bindgen]
pub fn richtext_schema() -> String {
    richtext::SCHEMA_JSON.to_string()
}

#[wasm_bindgen]
pub fn richtext_canonicalize(ast_json: &str) -> Result<String, JsError> {
    richtext::canonicalize(&parse(ast_json)?).map(|v| v.to_string()).map_err(fail)
}

/// Decode a Loro snapshot (optionally followed by updates) into the canonical AST.
#[wasm_bindgen]
pub fn richtext_decode(snapshot: &[u8]) -> Result<String, JsError> {
    let doc = richtext::load_doc(snapshot, std::iter::empty()).map_err(fail)?;
    richtext::decode_ast(&doc).map(|v| v.to_string()).map_err(fail)
}

/// Build a new-lineage document from an AST (offline creation); returns the snapshot bytes.
#[wasm_bindgen]
pub fn richtext_build(ast_json: &str, peer: f64) -> Result<Vec<u8>, JsError> {
    let ast = richtext::canonicalize(&parse(ast_json)?).map_err(fail)?;
    let doc = richtext::build_doc(&ast, peer as u64).map_err(fail)?;
    richtext::export_snapshot(&doc).map_err(fail)
}

/// Block index (`{ block_id: { parent, node_type, hash, struct_rev } }`) of a canonical AST.
#[wasm_bindgen]
pub fn richtext_block_index(ast_json: &str) -> Result<String, JsError> {
    let ast = richtext::canonicalize(&parse(ast_json)?).map_err(fail)?;
    let v = richtext::validate_ast(&ast, &richtext::Limits::default()).map_err(fail)?;
    Ok(serde_json::to_string(&richtext::initial_block_index(&v, 0)).unwrap())
}

/// Block-level operations turning `base` into `target` (explicit draft commit).
/// `block_index_json` is the base's `{ block_id: { hash, struct_rev } }` as read from the backend.
#[wasm_bindgen]
pub fn richtext_diff(entity_id: &str, base_ast_json: &str, target_ast_json: &str, block_index_json: &str) -> Result<String, JsError> {
    let raw = parse(block_index_json)?;
    let mut index = BlockIndex::new();
    for (k, v) in raw.as_object().into_iter().flatten() {
        index.insert(k.clone(), aiworkspace_core::model::BlockInfo {
            parent: String::new(),
            node_type: String::new(),
            hash: v["hash"].as_str().unwrap_or("").to_string(),
            struct_rev: v["struct_rev"].as_u64().unwrap_or(0),
        });
    }
    let base = richtext::canonicalize(&parse(base_ast_json)?).map_err(fail)?;
    let target = richtext::canonicalize(&parse(target_ast_json)?).map_err(fail)?;
    richtext::diff_blocks(entity_id, &base, &target, &index).map(|ops| json!(ops).to_string()).map_err(fail)
}

/// The offline replica: confirmed layer + pending submissions (design §6.3).
#[wasm_bindgen]
pub struct Replica {
    inner: CoreReplica,
}

#[wasm_bindgen]
impl Replica {
    /// `tables_json`: `{ entities, tree_edges, table_fields, table_records, richtext_states, refs, assets }`
    /// as rows of the replica database (BLOBs as base64).
    #[wasm_bindgen(constructor)]
    pub fn new(principal: &str, workspace_id: &str, epoch: &str, tables_json: &str, confirmed_seq: f64, peer: f64) -> Result<Replica, JsError> {
        let confirmed = load_rows(&parse(tables_json)?).map_err(fail)?;
        Ok(Replica { inner: CoreReplica::new(principal, workspace_id, epoch, confirmed, confirmed_seq as u64, peer as u64) })
    }

    #[wasm_bindgen(getter)]
    pub fn confirmed_seq(&self) -> f64 {
        self.inner.confirmed_seq as f64
    }

    /// Restore persisted pending submissions: `[{ idempotency_key, request, state, result? }]`.
    pub fn restore_pending(&mut self, rows_json: &str) -> Result<(), JsError> {
        let rows = parse(rows_json)?;
        self.inner.restore_pending(rows.as_array().map(Vec::as_slice).unwrap_or(&[]));
        Ok(())
    }

    /// Plan a local Commit on the working view. Returns `{ status: "saved_locally", ... }`
    /// or the same conflict/rejected result the backend would give.
    pub fn submit_local(&mut self, request_json: &str) -> Result<String, JsError> {
        Ok(self.inner.submit_local(&parse(request_json)?).to_string())
    }

    /// Apply change-stream events to the confirmed layer and rebase.
    pub fn apply_remote(&mut self, events_json: &str) -> Result<String, JsError> {
        let events = parse(events_json)?;
        self.inner.apply_remote(events.as_array().map(Vec::as_slice).unwrap_or(&[])).map(|v| v.to_string()).map_err(fail)
    }

    /// The next queued request ready to send, or `null`.
    pub fn next_to_send(&self) -> Result<String, JsError> {
        self.inner.next_to_send().map(|v| v.unwrap_or(Value::Null).to_string()).map_err(fail)
    }

    pub fn mark(&mut self, key: &str, state: &str, result_json: Option<String>) -> Result<(), JsError> {
        let result = match result_json {
            Some(t) => Some(parse(&t)?),
            None => None,
        };
        self.inner.mark(key, state, result).map_err(fail)
    }

    pub fn discard(&mut self, key: &str) -> bool {
        self.inner.discard(key).is_some()
    }

    pub fn pending(&self) -> String {
        self.inner.pending_json().to_string()
    }

    /// Rows of the confirmed layer, to persist after it advanced.
    pub fn confirmed_rows(&self) -> Result<String, JsError> {
        dump_rows(&self.inner.confirmed).map(|v| v.to_string()).map_err(fail)
    }

    /// Rows of the confirmed layer changed by `apply_remote` since the last call (upserts by
    /// primary key, plus `refs_deleted`): what the replica database writes in the same
    /// transaction as the new `confirmed_seq` and the pending rows.
    pub fn take_confirmed_delta(&mut self) -> Result<String, JsError> {
        self.inner.take_confirmed_delta().map(|v| v.to_string()).map_err(fail)
    }

    /// Rich text updates of still-undecided pending submissions for one entity, in order
    /// (base64): confirmed snapshot + these = the working document.
    pub fn pending_richtext_updates(&self, entity_id: &str) -> String {
        let mut out = Vec::new();
        for p in &self.inner.pending {
            if p.provisional_seq.is_none() {
                continue;
            }
            for op in p.request.get("operations").and_then(Value::as_array).into_iter().flatten() {
                if op["op"] == "richtext.apply_update" && op["entity_id"] == entity_id {
                    if let Some(u) = op["update"].as_str() {
                        out.push(json!({ "idempotency_key": p.key, "update": u }));
                    }
                }
            }
        }
        json!(out).to_string()
    }

    // ---- reads on the working view (same shapes as the backend's doc.* results) ----

    pub fn outline(&self) -> Result<String, JsError> {
        let access = Access::full(&self.inner.principal);
        let m = &self.inner.working;
        let mut out = Vec::new();
        let mut stack = vec!["root".to_string()];
        use aiworkspace_core::model::ReadCtx;
        while let Some(id) = stack.pop() {
            let Some(e) = m.entity(&id).map_err(fail)? else { continue };
            if !e.alive() {
                continue;
            }
            let mut env = read::envelope(m, &access, &e).map_err(fail)?;
            env["title"] = e.payload.get("title").cloned().unwrap_or(Value::Null);
            if e.type_id == aiworkspace_core::model::TYPE_CONTAINER {
                env["kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
                env["layout"] = e.payload.get("layout").cloned().unwrap_or(Value::Null);
            }
            if e.type_id == aiworkspace_core::model::TYPE_CELL {
                env["view_type"] = e.payload.get("view").and_then(|v| v.get("type")).cloned().unwrap_or(Value::Null);
                env["source_id"] = e.payload.get("source_ref").and_then(|r| r.get("entity_id")).cloned().unwrap_or(Value::Null);
            }
            out.push(env);
            for edge in m.children(&id).map_err(fail)?.into_iter().rev() {
                stack.push(edge.child_id);
            }
        }
        Ok(json!({ "ok": true, "epoch": self.inner.epoch, "head_seq": self.inner.confirmed_seq, "entities": out }).to_string())
    }

    pub fn read(&self, entity_id: &str, selector_json: Option<String>) -> Result<String, JsError> {
        let selector = match selector_json {
            Some(t) => Some(parse(&t)?),
            None => None,
        };
        let access = Access::full(&self.inner.principal);
        read::read(&self.inner.working, &access, entity_id, selector.as_ref()).map(|v| v.to_string()).map_err(fail)
    }

    /// `doc.list_annotations` on the working view: `{ target_ids?, parent_id? }`.
    pub fn list_annotations(&self, params_json: &str) -> Result<String, JsError> {
        let p = parse(params_json)?;
        let targets: Vec<String> = p.get("target_ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let access = Access::full(&self.inner.principal);
        let annotations = read::list_annotations(&self.inner.working, &access, &targets, p.get("parent_id").and_then(Value::as_str)).map_err(fail)?;
        Ok(json!({ "ok": true, "annotations": annotations, "head_seq": self.inner.confirmed_seq }).to_string())
    }

    /// `doc.query` on the working view (embedded tables only; URL tables need the network).
    pub fn query(&self, params_json: &str) -> Result<String, JsError> {
        use aiworkspace_core::filter::{run_query, QuerySpec};
        use aiworkspace_core::model::ReadCtx;
        let p = parse(params_json)?;
        let m = &self.inner.working;
        let mut spec = QuerySpec {
            limit: p.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize,
            cursor: p.get("cursor").and_then(Value::as_str).map(str::to_string),
            best_effort: true,
            with_meta: true,
            ..Default::default()
        };
        let mut source_id = p.get("source_id").and_then(Value::as_str).map(str::to_string);
        if let Some(view) = p.get("view_id").and_then(Value::as_str) {
            let cell = m.entity(view).map_err(fail)?.ok_or_else(|| fail(WsError::not_found("view not found")))?;
            source_id = cell.payload.get("source_ref").and_then(|r| r["entity_id"].as_str()).map(str::to_string);
            spec.filter = cell.payload.get("filter").filter(|f| !f.is_null()).cloned();
            spec.sorts = cell.payload.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default();
            spec.group = cell.payload.get("group").and_then(|g| g["field_id"].as_str()).map(str::to_string);
            spec.manual_order = cell.payload.get("manual_order").and_then(Value::as_object).cloned();
        }
        if let Some(extra) = p.get("filter").filter(|f| !f.is_null()) {
            spec.filter = Some(match spec.filter.take() {
                Some(f) => json!({ "op": "and", "args": [f, extra] }),
                None => extra.clone(),
            });
        }
        if let Some(s) = p.get("sorts").and_then(Value::as_array) {
            spec.sorts = s.clone();
        }
        let source_id = source_id.ok_or_else(|| fail(WsError::invalid_op("source_id or view_id required")))?;
        let source = m.entity(&source_id).map_err(fail)?.filter(|e| e.alive()).ok_or_else(|| fail(WsError::not_found("table not found")))?;
        if aiworkspace_core::types::is_url_table(&source) {
            return Err(fail(WsError::new(aiworkspace_core::Code::DependencyUnavailable, "URL query tables are not available offline")));
        }
        let mut out = run_query(m, &source, &spec).map_err(fail)?;
        out["ok"] = json!(true);
        out["source_id"] = json!(source_id);
        Ok(out.to_string())
    }

    /// Full-history snapshot of a rich text in the *confirmed* layer (the editor's confirmed document).
    pub fn collab_snapshot(&self, entity_id: &str) -> Result<Vec<u8>, JsError> {
        let st = self.inner.confirmed.richtexts.get(entity_id).ok_or_else(|| fail(WsError::not_found("not a rich text")))?;
        richtext::export_snapshot(&st.doc).map_err(fail)
    }

    pub fn lineage_id(&self, entity_id: &str) -> Option<String> {
        self.inner.confirmed.richtexts.get(entity_id).map(|s| s.meta.lineage_id.clone())
    }
}
