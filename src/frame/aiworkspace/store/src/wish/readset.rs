//! The read set of a wish run (许愿格 §6.2): what was actually handed to the model or the
//! program, recorded as the existing version cells — never what the model says it used. The same
//! list becomes the application's preconditions, the wish's `last_run.read_set` and every result's
//! `derived.inputs`.

use aiworkspace_core::canonical::canonical_json;
use aiworkspace_core::model::*;
use aiworkspace_core::plan::resolve_cell;
use aiworkspace_core::{WsError, WsResult};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Wish / derived inputs are limited to this many entries (§6.2).
pub const MAX_READ_SET: usize = 200;

#[derive(Debug, Clone, Default)]
pub struct ReadSet {
    cells: BTreeMap<String, Value>,
    labels: BTreeMap<String, String>,
}

fn sel_key(entity_id: &str, selector: &Option<Value>) -> String {
    format!("{entity_id}|{}", selector.as_ref().map(|s| canonical_json(s).unwrap_or_default()).unwrap_or_default())
}

impl ReadSet {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn label(&mut self, entity_id: &str, label: &str) {
        self.labels.insert(entity_id.to_string(), label.to_string());
    }

    /// Record one version cell as it is in `ctx` now (`None` selector = the whole entity).
    pub fn add(&mut self, ctx: &dyn ReadCtx, entity_id: &str, selector: Option<Value>) -> WsResult<()> {
        let key = sel_key(entity_id, &selector);
        if self.cells.contains_key(&key) {
            return Ok(());
        }
        let target = json!({ "entity_id": entity_id, "selector": selector.clone().unwrap_or(json!({ "kind": "entity" })) });
        let current = resolve_cell(ctx, &target)?;
        let mut v = Map::new();
        v.insert("entity_id".into(), json!(entity_id));
        if let Some(s) = selector {
            v.insert("selector".into(), s);
        }
        let mut version = Map::new();
        version.insert("mode".into(), json!("follow"));
        version.insert(if current.is_string() { "hash".into() } else { "rev".into() }, current);
        v.insert("version".into(), Value::Object(version));
        self.cells.insert(key, Value::Object(v));
        Ok(())
    }

    pub fn entity(&mut self, ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<()> {
        self.add(ctx, &e.entity_id, None)
    }

    /// The rows of a table restricted to `fields` (all live fields when `None`): membership plus each
    /// field's type and values.
    pub fn table(&mut self, ctx: &dyn ReadCtx, table_id: &str, fields: Option<&[String]>) -> WsResult<()> {
        self.add(ctx, table_id, Some(json!({ "kind": "table_members" })))?;
        let live: Vec<String> = match fields {
            Some(f) => f.to_vec(),
            None => aiworkspace_core::filter::live_fields(ctx, table_id)?.into_iter().map(|f| f.field_id).collect(),
        };
        for f in live {
            self.add(ctx, table_id, Some(json!({ "kind": "table_field_type", "field_id": f })))?;
            self.add(ctx, table_id, Some(json!({ "kind": "table_field_values", "field_id": f })))?;
        }
        Ok(())
    }

    /// The configuration of a saved table view that decides which rows and columns it shows.
    pub fn view(&mut self, ctx: &dyn ReadCtx, cell_id: &str) -> WsResult<()> {
        for key in ["filter", "sorts", "group", "fields", "manual_order", "source_ref"] {
            self.add(ctx, cell_id, Some(json!({ "kind": "doc_key", "key": key })))?;
        }
        Ok(())
    }

    /// Folder membership (a new member makes a folder input stale).
    pub fn folder(&mut self, ctx: &dyn ReadCtx, folder_id: &str) -> WsResult<()> {
        self.add(ctx, folder_id, Some(json!({ "kind": "tree_children" })))
    }

    /// The recorded cells, coarsened per entity when there are more than `MAX_READ_SET` (a table's
    /// field cells become its whole-content version). Still too many → refused, never truncated.
    pub fn to_inputs(&self, ctx: &dyn ReadCtx) -> WsResult<Vec<Value>> {
        let mut list: Vec<Value> = self.cells.values().cloned().collect();
        if list.len() > MAX_READ_SET {
            let mut per: BTreeMap<String, usize> = BTreeMap::new();
            for v in &list {
                *per.entry(v["entity_id"].as_str().unwrap_or("").to_string()).or_default() += 1;
            }
            let mut coarse = ReadSet::default();
            for v in &list {
                let id = v["entity_id"].as_str().unwrap_or("");
                if per[id] > 2 {
                    coarse.add(ctx, id, None)?;
                } else {
                    coarse.cells.insert(sel_key(id, &v.get("selector").cloned()), v.clone());
                }
            }
            list = coarse.cells.into_values().collect();
            if list.len() > MAX_READ_SET {
                return Err(WsError::limit(format!("the run read {} separate objects; at most {MAX_READ_SET} can be recorded — narrow the inputs", list.len())));
            }
        }
        for v in &mut list {
            if let Some(l) = v["entity_id"].as_str().and_then(|id| self.labels.get(id)) {
                v["label"] = json!(l);
            }
        }
        Ok(list)
    }

    pub fn merge(&mut self, other: &ReadSet) {
        for (k, v) in &other.cells {
            self.cells.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for (k, v) in &other.labels {
            self.labels.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    pub fn from_inputs(inputs: &[Value]) -> ReadSet {
        let mut r = ReadSet::default();
        for v in inputs {
            let id = v["entity_id"].as_str().unwrap_or("");
            let mut v = v.clone();
            if let Some(l) = v.as_object_mut().and_then(|o| o.remove("label")) {
                r.labels.insert(id.to_string(), l.as_str().unwrap_or("").to_string());
            }
            r.cells.insert(sel_key(id, &v.get("selector").cloned()), v);
        }
        r
    }
}

/// Commit preconditions guarding exactly the recorded cells (follow inputs only).
pub fn preconditions(inputs: &[Value]) -> Vec<Value> {
    inputs
        .iter()
        .filter(|i| i["version"]["mode"] != json!("fixed"))
        .filter_map(|i| {
            let expect = match (i["version"].get("rev"), i["version"].get("hash")) {
                (Some(r), _) => json!({ "rev": r }),
                (None, Some(h)) => json!({ "hash": h }),
                _ => return None,
            };
            let mut target = json!({ "entity_id": i["entity_id"] });
            if let Some(s) = i.get("selector") {
                target["selector"] = s.clone();
            }
            Some(json!({ "target": target, "expect": expect }))
        })
        .collect()
}
