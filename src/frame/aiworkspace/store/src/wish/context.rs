//! ContextBuilder (许愿格 §5): one stage of a wish run — the snapshot, the handle table, the canvas,
//! the context map `WORKSPACE.md`, the snapshot directory, the bound inputs and the read set — and
//! the read-only tools answered from the same snapshot (`ws_outline`, `ws_find`, `ws_profile`,
//! `ws_neighbors`, `ws_read`, `ws_query`).
//!
//! In the execution stage every input handed to the model or the program is recorded in the read set;
//! reading data that was not declared appends it as an input of this run (§5.5), materialized so the
//! program can use it by name.

use super::canvas::{reading_order, relation, Block, Canvas, Rect};
use super::handles::{Handles, Target};
use super::profile::{entity_profile, field_lines, plain_value, profile_line};
use super::readset::ReadSet;
use super::snapshot::WishSnapshot;
use aiworkspace_core::filter::{live_fields, run_query, QuerySpec};
use aiworkspace_core::id::is_valid_id;
use aiworkspace_core::model::*;
use aiworkspace_core::value::{reference_entity_id, FieldDef, FieldType};
use aiworkspace_core::wish::is_input_name;
use aiworkspace_core::{Code, WsError, WsResult};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Longest tool answer handed back to the model (characters).
pub const MAX_TOOL_CHARS: usize = 12_000;
/// Rows a read tool returns at most; more is the program's job.
pub const MAX_TOOL_ROWS: usize = 50;
/// Members of a folder input materialized at most (deeper / more is refused, not truncated).
pub const MAX_FOLDER_MEMBERS: usize = 200;
const MAX_FOLDER_DEPTH: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Analyze,
    Execute,
    /// Re-running the stored program: no model, inputs materialized only.
    Program,
}

impl StageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StageKind::Analyze => "analyze",
            StageKind::Execute => "execute",
            StageKind::Program => "program",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StageOptions {
    pub kind: StageKind,
    /// `{ cell_id?, surface_id?, selection?: [cell ids], viewport?: {x,y,w,h} }` — a hint, not a dependency.
    pub location: Value,
    /// Fixed parameters of the run (evaluation date, timezone, currency…).
    pub request: Value,
    /// Built-in Renderer catalog (`[{ renderer, title, accepts, config, description }]`).
    pub catalog: Value,
    pub map_budget_chars: usize,
    /// Materialize the bound inputs (execution / program stages).
    pub materialize: bool,
}

#[derive(Debug, Clone)]
pub struct BoundInput {
    pub name: String,
    pub entity_id: String,
    pub type_id: String,
    pub selector: Option<Value>,
    pub label: String,
    pub appended: bool,
    /// Fields this wish owns on the table (derived columns): never handed to the program.
    pub owned_fields: Vec<String>,
}

pub struct Stage {
    pub snap: WishSnapshot,
    pub wish: EntityRow,
    pub kind: StageKind,
    pub run_id: String,
    pub workdir: PathBuf,
    pub handles: Handles,
    pub canvas: Canvas,
    pub reads: ReadSet,
    pub inputs: Vec<BoundInput>,
    /// Inputs appended during execution, in wish-input form.
    pub appended: Vec<Value>,
    /// The wish's Block the run was started from.
    pub origin: Option<String>,
    pub location: Value,
    pub request: Value,
    catalog: Value,
    /// table id → fields owned by this wish (derived columns).
    owned: BTreeMap<String, Vec<String>>,
    materialized: BTreeSet<String>,
}

fn title_of(e: &EntityRow) -> String {
    e.payload
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| e.name.clone())
        .unwrap_or_else(|| e.entity_id.clone())
}

fn write_json(path: &Path, v: &Value) -> WsResult<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| WsError::io(e.to_string()))?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(v).unwrap_or_default()).map_err(|e| WsError::io(e.to_string()))
}

fn write_text(path: &Path, s: &str) -> WsResult<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| WsError::io(e.to_string()))?;
    }
    std::fs::write(path, s).map_err(|e| WsError::io(e.to_string()))
}

fn clip(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n…（结果过长已截断：请缩小范围，或用程序处理全部数据）");
    out
}

/// An identifier made from a label (`销售订单` → `input1`, `Orders 2026` → `orders_2026`).
pub fn name_from(label: &str, taken: &BTreeSet<String>) -> String {
    let mut base: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if base.is_empty() || !is_input_name(&base) || base.len() > 40 {
        base = "input".into();
    }
    let mut n = 1;
    let mut name = if base == "input" { format!("input{n}") } else { base.clone() };
    while taken.contains(&name) {
        n += 1;
        name = format!("{base}{n}");
    }
    name
}

/// Translate a filter written with field names / handles (and option labels) into the core filter
/// (`field_id`, option ids). Shorthand `{ "地区": "华东", … }` means "all equal".
pub fn translate_filter(f: &Value, fields: &[FieldRow], handles: &Handles, table_id: &str) -> WsResult<Value> {
    let find = |name: &str| -> WsResult<&FieldRow> {
        let id = match handles.resolve(name) {
            Ok(Target::Field(t, fid)) if t == table_id => Some(fid),
            _ => None,
        };
        fields
            .iter()
            .find(|x| Some(&x.field_id) == id.as_ref() || x.def.name == name || x.field_id == name)
            .ok_or_else(|| WsError::invalid_op(format!("unknown field {name:?}")))
    };
    let option = |def: &FieldDef, v: &Value| -> Value {
        match (v.as_str(), &def.options) {
            (Some(s), Some(opts)) => opts.iter().find(|o| o.label == s || o.option_id == s).map(|o| json!(o.option_id)).unwrap_or_else(|| v.clone()),
            _ => v.clone(),
        }
    };
    let o = f.as_object().ok_or_else(|| WsError::invalid_op("filter must be an object"))?;
    match o.get("op").and_then(Value::as_str) {
        None => {
            let mut args = Vec::new();
            for (k, v) in o {
                let fr = find(k)?;
                let value = if matches!(fr.def.ty, FieldType::Select) { option(&fr.def, v) } else { v.clone() };
                args.push(json!({ "op": "cmp", "field_id": fr.field_id, "operator": "eq", "value": value }));
            }
            if args.is_empty() {
                return Err(WsError::invalid_op("empty filter"));
            }
            Ok(if args.len() == 1 { args.remove(0) } else { json!({ "op": "and", "args": args }) })
        }
        Some("and") | Some("or") => {
            let args = o.get("args").and_then(Value::as_array).ok_or_else(|| WsError::invalid_op("and/or need args"))?;
            Ok(json!({ "op": o["op"], "args": args.iter().map(|a| translate_filter(a, fields, handles, table_id)).collect::<WsResult<Vec<_>>>()? }))
        }
        Some("not") => Ok(json!({ "op": "not", "arg": translate_filter(o.get("arg").unwrap_or(&Value::Null), fields, handles, table_id)? })),
        Some("cmp") => {
            let name = o.get("field").or_else(|| o.get("field_id")).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("cmp needs field"))?;
            let fr = find(name)?;
            let mut out = json!({ "op": "cmp", "field_id": fr.field_id, "operator": o.get("operator").cloned().unwrap_or(json!("eq")) });
            if let Some(v) = o.get("value") {
                out["value"] = match (fr.def.ty, v) {
                    (FieldType::Select, _) => option(&fr.def, v),
                    (FieldType::MultiSelect, Value::Array(a)) | (_, Value::Array(a)) if matches!(fr.def.ty, FieldType::MultiSelect | FieldType::Select) => {
                        Value::Array(a.iter().map(|x| option(&fr.def, x)).collect())
                    }
                    _ => v.clone(),
                };
            }
            Ok(out)
        }
        Some(op) => Err(WsError::invalid_op(format!("unknown filter op {op}"))),
    }
}

/// `[{ field, direction }]` with names / handles → core sorts.
pub fn translate_sorts(s: &Value, fields: &[FieldRow], handles: &Handles, table_id: &str) -> WsResult<Vec<Value>> {
    let mut out = Vec::new();
    for item in s.as_array().ok_or_else(|| WsError::invalid_op("sorts must be an array"))? {
        let (name, dir) = match item {
            Value::String(n) => match n.strip_prefix('-') {
                Some(rest) => (rest.to_string(), "desc"),
                None => (n.clone(), "asc"),
            },
            _ => (
                item.get("field").or_else(|| item.get("field_id")).and_then(Value::as_str).unwrap_or("").to_string(),
                item.get("direction").and_then(Value::as_str).unwrap_or("asc"),
            ),
        };
        let fid = match handles.resolve(&name) {
            Ok(Target::Field(t, fid)) if t == table_id => Some(fid),
            _ => None,
        };
        let fr = fields
            .iter()
            .find(|x| Some(&x.field_id) == fid.as_ref() || x.def.name == name || x.field_id == name)
            .ok_or_else(|| WsError::invalid_op(format!("unknown sort field {name:?}")))?;
        out.push(json!({ "field_id": fr.field_id, "direction": if dir == "desc" { "desc" } else { "asc" } }));
    }
    Ok(out)
}

/// Field names / handles → field ids.
pub fn translate_fields(s: &Value, fields: &[FieldRow], handles: &Handles, table_id: &str) -> WsResult<Vec<String>> {
    let mut out = Vec::new();
    for n in s.as_array().ok_or_else(|| WsError::invalid_op("fields must be an array"))? {
        let name = n.as_str().unwrap_or("");
        let fid = match handles.resolve(name) {
            Ok(Target::Field(t, fid)) if t == table_id => Some(fid),
            _ => None,
        };
        let fr = fields
            .iter()
            .find(|x| Some(&x.field_id) == fid.as_ref() || x.def.name == name || x.field_id == name)
            .ok_or_else(|| WsError::invalid_op(format!("unknown field {name:?}")))?;
        out.push(fr.field_id.clone());
    }
    Ok(out)
}

/// The rows a selector names, in its order: `(field ids returned, rows)`; rows are core query rows.
fn selected_rows(ctx: &dyn ReadCtx, table: &EntityRow, selector: Option<&Value>, view: Option<&EntityRow>) -> WsResult<(Option<Vec<String>>, Vec<Value>)> {
    let mut spec = QuerySpec { limit: 1000, with_meta: false, ..Default::default() };
    if let Some(cell) = view {
        spec.filter = cell.payload.get("filter").filter(|f| !f.is_null()).cloned();
        spec.sorts = cell.payload.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default();
        spec.manual_order = cell.payload.get("manual_order").and_then(Value::as_object).cloned();
    } else if let Some(s) = selector.filter(|s| s["kind"] == json!("table_query")) {
        spec.filter = s.get("filter").filter(|f| !f.is_null()).cloned();
        spec.sorts = s.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default();
        spec.fields = s.get("fields").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
    }
    let mut rows = Vec::new();
    loop {
        let page = run_query(ctx, table, &spec)?;
        rows.extend(page["rows"].as_array().cloned().unwrap_or_default());
        match page.get("next_cursor").and_then(Value::as_str) {
            Some(c) => spec.cursor = Some(c.to_string()),
            None => break,
        }
    }
    Ok((spec.fields.clone(), rows))
}

impl Stage {
    pub fn open(snap: WishSnapshot, wish_id: &str, run_id: &str, workdir: &Path, opts: &StageOptions) -> WsResult<Stage> {
        let wish = snap.readable(wish_id)?;
        if wish.type_id != TYPE_WISH {
            return Err(WsError::invalid_op(format!("{wish_id} is not a wish")));
        }
        let canvas = {
            let ctx = snap.ctx();
            let access = &snap.access;
            Canvas::build(&ctx, &|e| access.can_read(&ctx, e))?
        };
        let mut handles = Handles::new(wish_id);
        // the origin: the Block the run was started from, else the wish's first Block
        let loc_cell = opts.location.get("cell_id").and_then(Value::as_str).filter(|c| canvas.block(c).is_some_and(|b| b.source_id.as_deref() == Some(wish_id)));
        let origin = loc_cell.map(str::to_string).or_else(|| canvas.blocks.iter().find(|b| b.source_id.as_deref() == Some(wish_id)).map(|b| b.entity_id.clone()));
        // owned derived columns (§10.4)
        let mut owned: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (_, b) in aiworkspace_core::wish::current_results(&wish.payload) {
            if b["type"] == json!("table_columns") {
                let t = b["entity_id"].as_str().unwrap_or("").to_string();
                owned.entry(t).or_default().extend(b["fields"].as_object().into_iter().flatten().filter_map(|(_, f)| f.as_str().map(str::to_string)));
            }
        }
        let mut stage = Stage {
            snap,
            wish,
            kind: opts.kind,
            run_id: run_id.to_string(),
            workdir: workdir.to_path_buf(),
            handles: Handles::default(),
            canvas,
            reads: ReadSet::default(),
            inputs: Vec::new(),
            appended: Vec::new(),
            origin,
            location: opts.location.clone(),
            request: opts.request.clone(),
            catalog: opts.catalog.clone(),
            owned,
            materialized: BTreeSet::new(),
        };
        // handles: data tree in tree order, then Surfaces, groups and Blocks
        for e in stage.data_entities()? {
            handles.of(&e);
        }
        for s in stage.canvas.surfaces.clone() {
            if let Some(e) = stage.snap.entity(&s.entity_id)? {
                handles.of(&e);
            }
        }
        for g in stage.canvas.groups.clone() {
            if let Some(e) = stage.snap.entity(&g.entity_id)? {
                handles.of(&e);
            }
        }
        for b in stage.canvas.blocks.clone() {
            if let Some(e) = stage.snap.entity(&b.entity_id)? {
                handles.of(&e);
            }
        }
        stage.handles = handles;
        // bound inputs
        let mut taken = BTreeSet::new();
        let declared: Vec<Value> = stage.wish.payload.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default();
        for i in &declared {
            let id = i["entity_id"].as_str().unwrap_or("").to_string();
            let e = match stage.snap.entity(&id)? {
                Some(e) => e,
                None => continue,
            };
            let label = i.get("label").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| title_of(&e));
            let name = match i.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => name_from(&label, &taken),
            };
            taken.insert(name.clone());
            stage.inputs.push(BoundInput {
                name,
                type_id: e.type_id.clone(),
                entity_id: id.clone(),
                selector: i.get("selector").cloned(),
                label,
                appended: i.get("appended_by").is_some(),
                owned_fields: stage.owned.get(&id).cloned().unwrap_or_default(),
            });
        }
        std::fs::create_dir_all(workdir.join("output")).map_err(|e| WsError::io(e.to_string()))?;
        if opts.materialize {
            for i in stage.inputs.clone() {
                stage.materialize_input(&i)?;
            }
            // what the analysis copied into the task description is part of what the results rest on (§8.1)
            let reads: Vec<Value> = stage.wish.payload.get("analysis").and_then(|a| a.get("basis")).and_then(|b| b.get("reads")).and_then(Value::as_array).cloned().unwrap_or_default();
            for r in reads {
                if let Some(id) = r.get("entity_id").and_then(Value::as_str) {
                    let ctx = stage.snap.ctx();
                    if ctx.entity(id)?.is_some_and(|e| e.alive()) {
                        stage.reads.add(&ctx, id, r.get("selector").cloned())?;
                    }
                }
            }
        }
        stage.write_files(opts)?;
        Ok(stage)
    }

    pub fn ctx(&self) -> crate::docdb::SqlCtx<'_> {
        self.snap.ctx()
    }

    /// Readable, live data entities of the data tree in tree order (system folders excluded, this
    /// wish included only as `@W`).
    pub fn data_entities(&self) -> WsResult<Vec<EntityRow>> {
        let ctx = self.ctx();
        let mut out = Vec::new();
        let mut stack = vec![DATA_ID.to_string()];
        while let Some(id) = stack.pop() {
            for edge in ctx.children(&id)?.into_iter().rev() {
                let Some(e) = ctx.entity(&edge.child_id)?.filter(|e| e.alive()) else { continue };
                if !self.snap.access.can_read(&ctx, &e)? {
                    continue;
                }
                if e.type_id == TYPE_CONTAINER {
                    stack.push(e.entity_id.clone());
                }
                out.push(e);
            }
        }
        // the stack walk visits siblings in reverse; restore tree order
        let order: BTreeMap<String, usize> = {
            let mut order = BTreeMap::new();
            let mut n = 0;
            fn rec(ctx: &dyn ReadCtx, id: &str, order: &mut BTreeMap<String, usize>, n: &mut usize) -> WsResult<()> {
                for edge in ctx.children(id)? {
                    order.insert(edge.child_id.clone(), *n);
                    *n += 1;
                    rec(ctx, &edge.child_id, order, n)?;
                }
                Ok(())
            }
            rec(&ctx, DATA_ID, &mut order, &mut n)?;
            order
        };
        out.sort_by_key(|e| order.get(&e.entity_id).copied().unwrap_or(usize::MAX));
        Ok(out)
    }

    pub fn path_of(&self, id: &str) -> WsResult<String> {
        let ctx = self.ctx();
        let mut parts = Vec::new();
        let mut cur = id.to_string();
        for _ in 0..32 {
            let Some(e) = ctx.entity(&cur)? else { break };
            if cur == ROOT_ID {
                break;
            }
            parts.push(if cur == DATA_ID { "data".to_string() } else if cur == SURFACES_ID { "surfaces".to_string() } else { title_of(&e) });
            match ctx.edge(&cur)? {
                Some(edge) => cur = edge.parent_id,
                None => break,
            }
        }
        parts.reverse();
        Ok(format!("/{}", parts.join("/")))
    }

    fn handle_of(&mut self, id: &str) -> String {
        if let Some(h) = self.handles.get(id) {
            return h.to_string();
        }
        match self.snap.entity(id) {
            Ok(Some(e)) => self.handles.of(&e),
            _ => id.to_string(),
        }
    }

    fn profile(&mut self, e: &EntityRow) -> WsResult<Value> {
        let ctx = self.snap.ctx();
        let fh = if e.type_id == TYPE_TABLE { self.handles.fields(e, &live_fields(&ctx, &e.entity_id)?) } else { Vec::new() };
        let exclude = self.owned.get(&e.entity_id).cloned().unwrap_or_default();
        let mut p = entity_profile(&ctx, e, &fh, &exclude)?;
        if let Some(d) = &e.derived {
            let f = aiworkspace_core::freshness::entity_freshness(&ctx, &self.snap.access, &e.entity_id).unwrap_or(Value::Null);
            p["generated"] = json!({ "wish_id": d.get("wish_id"), "at": d.get("at"), "status": f["status"], "model_judgment": d.get("model_judgment"),
                                     "simulated": d.get("simulated") });
        }
        Ok(p)
    }

    // ---- materialization (§5.4)

    fn entity_dir(&self, id: &str) -> PathBuf {
        self.workdir.join("context").join("entities").join(id)
    }

    /// Write one entity (and, for a folder, its members) under `context/entities/<id>/`, recording the
    /// read set when the stage executes. Returns the meta written.
    fn materialize_entity(&mut self, e: &EntityRow, selector: Option<&Value>, owned: &[String], depth: usize, budget: &mut usize) -> WsResult<Value> {
        let record = self.kind != StageKind::Analyze;
        let dir = self.entity_dir(&e.entity_id);
        let handle = self.handles.of(e);
        let mut meta = json!({ "entity_id": e.entity_id, "handle": handle, "type_id": e.type_id, "name": e.name, "title": title_of(e),
                               "path": self.path_of(&e.entity_id)?, "selector": selector, "content_rev": e.content_rev, "complete": true });
        let ctx = self.snap.ctx();
        match e.type_id.as_str() {
            TYPE_TABLE => {
                if aiworkspace_core::types::is_url_table(e) {
                    return Err(WsError::new(Code::DependencyUnavailable, format!("{} is a URL query table: its rows have no verifiable version, so it cannot be a wish input yet", title_of(e))));
                }
                let fields: Vec<FieldRow> = live_fields(&ctx, &e.entity_id)?.into_iter().filter(|f| !owned.contains(&f.field_id)).collect();
                let fh = self.handles.fields(e, &fields);
                let view = match selector.filter(|s| s["kind"] == json!("table_view")).and_then(|s| s["cell_id"].as_str()) {
                    Some(cid) => {
                        let c = ctx.entity(cid)?.filter(|c| c.alive()).ok_or_else(|| WsError::new(Code::DependencyUnavailable, format!("table view {cid} no longer exists")))?;
                        Some(c)
                    }
                    None => None,
                };
                let (proj, rows) = selected_rows(&ctx, e, selector, view.as_ref())?;
                let visible: Option<Vec<String>> = view.as_ref().and_then(|c| c.payload.get("fields")).and_then(Value::as_array).map(|a| {
                    a.iter().filter(|f| f["hidden"] != json!(true)).filter_map(|f| f["field_id"].as_str().map(str::to_string)).collect()
                });
                let used: Vec<&FieldRow> = fields.iter().filter(|f| proj.as_ref().map_or(true, |p| p.contains(&f.field_id))).collect();
                let schema: Vec<Value> = used
                    .iter()
                    .map(|f| {
                        let i = fields.iter().position(|x| x.field_id == f.field_id).unwrap_or(0);
                        json!({ "field_id": f.field_id, "handle": fh.get(i), "name": f.def.name, "type": f.def.ty.as_str(),
                                "options": f.def.options.as_ref().map(|o| o.iter().map(|x| x.label.clone()).collect::<Vec<_>>()),
                                "visible": visible.as_ref().map_or(true, |v| v.contains(&f.field_id)), "required": f.def.required })
                    })
                    .collect();
                write_json(&dir.join("schema.json"), &json!({ "fields": schema, "title_field_id": e.payload.get("title_field_id") }))?;
                let defs: BTreeMap<String, FieldDef> = used.iter().map(|f| (f.field_id.clone(), f.def.clone())).collect();
                let mut file = std::fs::File::create(dir.join("rows.jsonl")).map_err(|er| WsError::io(er.to_string()))?;
                for r in &rows {
                    let mut vals = Map::new();
                    for (k, v) in r["values"].as_object().into_iter().flatten() {
                        if let Some(d) = defs.get(k) {
                            vals.insert(k.clone(), plain_value(d, v));
                        }
                    }
                    let line = json!({ "id": r["record_id"], "v": vals });
                    writeln!(file, "{line}").map_err(|er| WsError::io(er.to_string()))?;
                }
                meta["rows"] = json!(rows.len());
                if let Some(c) = &view {
                    meta["view"] = json!({ "cell_id": c.entity_id, "handle": self.handles.of(c), "summary": view_summary(c, &fields) });
                }
                if record {
                    let f_ids: Vec<String> = used.iter().map(|f| f.field_id.clone()).collect();
                    let mut needed = f_ids.clone();
                    // the fields deciding membership and order are read too
                    for fid in fields.iter().map(|f| f.field_id.clone()) {
                        let mentioned = |v: Option<&Value>| v.map(|v| v.to_string().contains(&format!("\"{fid}\""))).unwrap_or(false);
                        let deciding = match &view {
                            Some(c) => mentioned(c.payload.get("filter")) || mentioned(c.payload.get("sorts")),
                            None => mentioned(selector.and_then(|s| s.get("filter"))) || mentioned(selector.and_then(|s| s.get("sorts"))),
                        };
                        if deciding && !needed.contains(&fid) {
                            needed.push(fid);
                        }
                    }
                    self.reads.table(&ctx, &e.entity_id, Some(&needed))?;
                    if let Some(c) = &view {
                        self.reads.view(&ctx, &c.entity_id)?;
                    }
                }
            }
            TYPE_RICHTEXT => {
                let ast = ctx.richtext(&e.entity_id)?.map(|r| r.meta.ast).unwrap_or(Value::Null);
                write_text(&dir.join("content.md"), &aiworkspace_core::markdown::from_richtext(&ast))?;
                write_json(&dir.join("content.json"), &ast)?;
                if record {
                    self.reads.entity(&ctx, e)?;
                }
            }
            TYPE_RECORD => {
                let nested = aiworkspace_core::types::record_nested(&e.payload);
                let schema = aiworkspace_core::types::record_schema(&e.payload).unwrap_or_default();
                let props: Map<String, Value> = schema
                    .values()
                    .filter_map(|d| nested["props"].get(&d.field_id).map(|v| (d.name.clone(), plain_value(d, v))))
                    .collect();
                write_json(&dir.join("content.json"), &json!({ "schema": nested["schema"], "props": nested["props"], "by_name": props }))?;
                if record {
                    self.reads.entity(&ctx, e)?;
                }
            }
            TYPE_ASSET => {
                let obj = e.payload.get("object_id").and_then(Value::as_str).unwrap_or("");
                match self.snap.asset_bytes(obj) {
                    Ok(bytes) => {
                        write_text(&self.workdir.join("context").join("assets").join(obj.replace(':', "_")), "")?;
                        std::fs::write(self.workdir.join("context").join("assets").join(obj.replace(':', "_")), &bytes).map_err(|er| WsError::io(er.to_string()))?;
                        meta["asset_file"] = json!(format!("context/assets/{}", obj.replace(':', "_")));
                    }
                    Err(_) => {
                        meta["complete"] = json!(false);
                        meta["missing"] = json!("asset bytes are not available here");
                    }
                }
                meta["media_type"] = e.payload.get("media_type").cloned().unwrap_or(Value::Null);
                meta["file_name"] = e.payload.get("file_name").cloned().unwrap_or(Value::Null);
                meta["image"] = e.payload.get("image").cloned().unwrap_or(Value::Null);
                if record {
                    self.reads.entity(&ctx, e)?;
                }
            }
            TYPE_CONTAINER => {
                if depth >= MAX_FOLDER_DEPTH {
                    return Err(WsError::limit(format!("folder {} nests deeper than {MAX_FOLDER_DEPTH} levels: bind its sub-folders separately", title_of(e))));
                }
                let mut members = Vec::new();
                let mut kids = Vec::new();
                {
                    let ctx = self.snap.ctx();
                    for edge in ctx.children(&e.entity_id)? {
                        let Some(c) = ctx.entity(&edge.child_id)?.filter(|c| c.alive()) else { continue };
                        if !self.snap.access.can_read(&ctx, &c)? {
                            // a readable subset is never passed off as the whole folder (§6.2)
                            return Err(WsError::new(Code::DependencyUnavailable, format!("folder {} contains members you cannot read: it cannot be read completely", title_of(e))));
                        }
                        kids.push(c);
                    }
                }
                for c in kids {
                    if c.type_id == TYPE_CELL || c.type_id == TYPE_WISH || c.type_id == TYPE_BLOCK_DEF {
                        continue;
                    }
                    if *budget == 0 {
                        return Err(WsError::limit(format!("folder {} has more than {MAX_FOLDER_MEMBERS} members: narrow the input", title_of(e))));
                    }
                    *budget -= 1;
                    let m = self.materialize_entity(&c, None, &[], depth + 1, budget)?;
                    members.push(json!({ "entity_id": c.entity_id, "handle": m["handle"], "type_id": c.type_id, "title": m["title"] }));
                }
                meta["members"] = json!(members);
                if record {
                    let ctx = self.snap.ctx();
                    self.reads.folder(&ctx, &e.entity_id)?;
                }
            }
            TYPE_ANNOTATION => {
                write_json(&dir.join("content.json"), &json!({ "body": e.payload.get("body"), "kind": e.payload.get("kind"), "target": e.payload.get("target") }))?;
                if record {
                    self.reads.entity(&ctx, e)?;
                }
            }
            t => return Err(WsError::invalid_op(format!("{} ({t}) cannot be an input", title_of(e)))),
        }
        let p = self.profile(e)?;
        write_json(&dir.join("profile.json"), &p)?;
        write_json(&dir.join("meta.json"), &meta)?;
        self.materialized.insert(e.entity_id.clone());
        Ok(meta)
    }

    fn materialize_input(&mut self, i: &BoundInput) -> WsResult<()> {
        let ctx = self.snap.ctx();
        let e = match ctx.entity(&i.entity_id)? {
            Some(e) if e.alive() && self.snap.access.can_read(&ctx, &e)? => e,
            Some(e) if e.alive() => return Err(WsError::new(Code::DependencyUnavailable, format!("input {} is not readable", i.label))),
            _ => return Err(WsError::new(Code::DependencyUnavailable, format!("input {} no longer exists", i.label))),
        };
        if e.type_id == TYPE_WISH {
            return Err(WsError::invalid_op(format!("input {} is a wish: bind its results instead", i.label)));
        }
        self.reads.label(&e.entity_id, &i.label);
        let mut budget = MAX_FOLDER_MEMBERS;
        self.materialize_entity(&e, i.selector.as_ref(), &i.owned_fields, 0, &mut budget)?;
        Ok(())
    }

    /// Append an undeclared entity as an input of this run (execution stage, §5.5). Returns its name.
    pub fn append_input(&mut self, entity_id: &str, label: Option<&str>) -> WsResult<String> {
        if let Some(i) = self.inputs.iter().find(|i| i.entity_id == entity_id && i.selector.is_none()) {
            return Ok(i.name.clone());
        }
        let e = self.snap.readable(entity_id)?;
        if self.produced_by_this_wish(&e)? {
            return Err(WsError::invalid_op(format!("{} is a result of this wish: reading it would make the wish depend on itself", title_of(&e))));
        }
        let label = label.map(str::to_string).unwrap_or_else(|| title_of(&e));
        let taken: BTreeSet<String> = self.inputs.iter().map(|i| i.name.clone()).collect();
        let name = name_from(&label, &taken);
        let input = BoundInput {
            name: name.clone(),
            entity_id: entity_id.into(),
            type_id: e.type_id.clone(),
            selector: None,
            label: label.clone(),
            appended: true,
            owned_fields: self.owned.get(entity_id).cloned().unwrap_or_default(),
        };
        self.materialize_input(&input)?;
        self.inputs.push(input);
        self.appended.push(json!({ "entity_id": entity_id, "name": name, "label": label, "version": { "mode": "follow" }, "appended_by": self.run_id }));
        self.write_inputs()?;
        Ok(name)
    }

    /// Whether `e` (or, for a folder, a member) was generated by this wish.
    pub fn produced_by_this_wish(&self, e: &EntityRow) -> WsResult<bool> {
        let wid = self.wish.entity_id.as_str();
        if e.derived.as_ref().and_then(|d| d.get("wish_id")).and_then(Value::as_str) == Some(wid) {
            return Ok(true);
        }
        if e.type_id == TYPE_CONTAINER {
            let ctx = self.ctx();
            for edge in ctx.children(&e.entity_id)? {
                if let Some(c) = ctx.entity(&edge.child_id)? {
                    if c.derived.as_ref().and_then(|d| d.get("wish_id")).and_then(Value::as_str) == Some(wid) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn write_inputs(&self) -> WsResult<()> {
        let mut m = Map::new();
        for i in &self.inputs {
            m.insert(
                i.name.clone(),
                json!({ "entity_id": i.entity_id, "handle": self.handles.get(&i.entity_id), "type_id": i.type_id, "label": i.label,
                        "selector": i.selector, "dir": format!("context/entities/{}", i.entity_id), "appended": i.appended,
                        "materialized": self.materialized.contains(&i.entity_id) }),
            );
        }
        write_json(&self.workdir.join("context").join("inputs.json"), &Value::Object(m))
    }

    fn write_files(&mut self, opts: &StageOptions) -> WsResult<()> {
        let w = self.workdir.clone();
        // the data tree and the BlockTree as indexes
        let mut data = Vec::new();
        for e in self.data_entities()? {
            let parent = self.ctx().edge(&e.entity_id)?.map(|x| x.parent_id).unwrap_or_default();
            data.push(json!({ "handle": self.handle_of(&e.entity_id), "entity_id": e.entity_id, "type_id": e.type_id, "title": title_of(&e),
                              "path": self.path_of(&e.entity_id)?, "parent": self.handle_of(&parent), "generated_by": e.derived.as_ref().and_then(|d| d.get("wish_id")) }));
        }
        write_json(&w.join("context").join("data-tree.json"), &json!(data))?;
        let blocks: Vec<Value> = self
            .canvas
            .blocks
            .clone()
            .iter()
            .map(|b| {
                json!({ "handle": self.handle_of(&b.entity_id), "entity_id": b.entity_id, "surface": self.handle_of(&b.surface_id),
                        "group": b.group_id.as_ref().map(|g| self.handle_of(g)), "frame": b.frame_id.as_ref().map(|g| self.handle_of(g)),
                        "renderer": b.renderer, "title": b.title, "source": b.source_id.as_ref().map(|s| self.handle_of(s)),
                        "rect": { "x": b.rect.x, "y": b.rect.y, "w": b.rect.w, "h": b.rect.h }, "pure_ui": b.pure_ui })
            })
            .collect();
        write_json(&w.join("context").join("block-tree.json"), &json!(blocks))?;
        // profiles of every visible data entity (analysis) / of the inputs (execution)
        if self.kind == StageKind::Analyze {
            for e in self.data_entities()? {
                if matches!(e.type_id.as_str(), TYPE_TABLE | TYPE_RICHTEXT | TYPE_RECORD | TYPE_ASSET | TYPE_CONTAINER | TYPE_ANNOTATION) {
                    let p = self.profile(&e)?;
                    let dir = self.entity_dir(&e.entity_id);
                    write_json(&dir.join("profile.json"), &p)?;
                    if e.type_id == TYPE_TABLE {
                        write_json(&dir.join("schema.json"), &json!({ "fields": p["fields"] }))?;
                    }
                    write_json(&dir.join("meta.json"), &json!({ "entity_id": e.entity_id, "handle": self.handle_of(&e.entity_id), "type_id": e.type_id, "title": title_of(&e), "path": self.path_of(&e.entity_id)? }))?;
                }
            }
        }
        self.write_inputs()?;
        let catalog = self.catalog_json()?;
        write_json(&w.join("catalog").join("renderers.json"), &catalog)?;
        let map = self.render_map(opts.map_budget_chars)?;
        write_text(&w.join("WORKSPACE.md"), &map)?;
        write_json(&w.join("handles.json"), &self.handles.to_json())?;
        write_json(
            &w.join("context").join("manifest.json"),
            &json!({ "workspace_id": self.snap.workspace_id, "epoch": self.snap.epoch, "head_seq": self.snap.head_seq, "wish_id": self.wish.entity_id,
                     "run_id": self.run_id, "stage": self.kind.as_str(), "inputs": self.inputs.iter().map(|i| &i.name).collect::<Vec<_>>() }),
        )?;
        Ok(())
    }

    /// The Renderer catalog: built-ins plus the Block definitions of this Workspace.
    pub fn catalog_json(&mut self) -> WsResult<Value> {
        let mut list = self.catalog.as_array().cloned().unwrap_or_default();
        for e in self.data_entities()? {
            if e.type_id == TYPE_BLOCK_DEF {
                list.push(json!({ "renderer": e.payload.get("def_id"), "title": e.payload.get("title"), "accepts": e.payload.get("accepts").cloned().unwrap_or(json!([])),
                                  "kind": e.payload.get("kind"), "description": e.payload.get("description"), "config": e.payload.get("config_schema"),
                                  "definition": self.handle_of(&e.entity_id) }));
            }
        }
        Ok(json!(list))
    }

    // ---- the context map (§5.2)

    pub fn render_map(&mut self, budget: usize) -> WsResult<String> {
        let mut out = String::new();
        let wish_title = title_of(&self.wish);
        let origin = self.origin.clone().and_then(|o| self.canvas.block(&o).cloned());
        out.push_str("# 工作区地图\n\n");
        out.push_str("句柄（@T1、@B2、@T1.f3…）只在本次运行中有效；工具参数与输出约定中使用句柄，程序中使用输入名。\n\n");
        // 1. task location
        out.push_str("## 任务位置\n");
        match &origin {
            Some(b) => {
                let surface = self.canvas.surface(&b.surface_id).map(|s| s.title.clone()).unwrap_or_default();
                let frame = b.frame_id.as_ref().and_then(|f| self.canvas.block(f)).and_then(|f| f.title.clone());
                let group = b.group_id.as_ref().and_then(|g| self.canvas.group(g)).and_then(|g| g.title.clone());
                out.push_str(&format!(
                    "许愿格 @W「{wish_title}」的 Block {} 位于画布「{surface}」({}){}{}。\n",
                    self.handle_of(&b.entity_id),
                    self.handle_of(&b.surface_id),
                    frame.map(|f| format!("的框「{f}」内")).unwrap_or_default(),
                    group.map(|g| format!("，组「{g}」中")).unwrap_or_default()
                ));
            }
            None => out.push_str(&format!("许愿格 @W「{wish_title}」位于数据树 {}（没有画布上的 Block）。\n", self.path_of(&self.wish.entity_id)?)),
        }
        let selection: Vec<String> = self.location.get("selection").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        if !selection.is_empty() {
            let on_canvas: Vec<String> = selection.iter().filter(|s| self.canvas.block(s).is_some()).cloned().collect();
            let hs: Vec<String> = on_canvas.iter().map(|s| self.handle_of(s)).collect();
            if !hs.is_empty() {
                out.push_str(&format!("触发时选中：{}。\n", hs.join("、")));
            }
        }
        if let Some(vp) = self.location.get("viewport") {
            let r = Rect { x: vp["x"].as_f64().unwrap_or(0.0), y: vp["y"].as_f64().unwrap_or(0.0), w: vp["w"].as_f64().unwrap_or(0.0), h: vp["h"].as_f64().unwrap_or(0.0) };
            if let Some(b) = &origin {
                let visible: Vec<String> = self
                    .canvas
                    .blocks
                    .clone()
                    .iter()
                    .filter(|x| x.surface_id == b.surface_id && r.gap(&x.rect) == 0.0 && x.entity_id != b.entity_id)
                    .take(30)
                    .map(|x| self.handle_of(&x.entity_id))
                    .collect();
                if !visible.is_empty() {
                    out.push_str(&format!("视口中可见：{}。\n", visible.join("、")));
                }
            }
        }
        out.push('\n');
        // 2. nearby Blocks
        if let Some(b) = &origin {
            out.push_str("## 附近的 Block（以 @W 的 Block 为原点，由近及远）\n");
            let mut same: Vec<&Block> = self.canvas.blocks.iter().filter(|x| x.surface_id == b.surface_id && x.entity_id != b.entity_id).collect();
            let order = reading_order(&same);
            same.sort_by(|x, y| b.rect.gap(&x.rect).partial_cmp(&b.rect.gap(&y.rect)).unwrap_or(std::cmp::Ordering::Equal));
            let same: Vec<Block> = same.into_iter().cloned().collect();
            let mut lines = Vec::new();
            for x in same.iter().take(30) {
                let (dir, band, _) = relation(&b.rect, &x.rect);
                let title = match (&x.title, x.renderer.as_str(), &x.source_id) {
                    (Some(t), _, _) if !t.is_empty() => t.clone(),
                    (_, "note", Some(src)) => self.snap.entity(src)?.and_then(|a| a.payload.get("body").and_then(Value::as_str).map(|s| s.chars().take(30).collect::<String>())).unwrap_or_default(),
                    _ => String::new(),
                };
                let mut l = format!("- {} {}「{}」", self.handle_of(&x.entity_id), renderer_label(&x.renderer), title);
                if x.renderer == "frame" && x.rect.contains_point(b.rect.cx(), b.rect.cy()) {
                    l.push_str("（纯 UI 框，@W 位于其中）");
                    lines.push(l);
                    continue;
                }
                if let Some(s) = &x.source_id {
                    l.push_str(&format!(" → 数据 {}", self.handle_of(s)));
                } else if x.pure_ui {
                    l.push_str("（纯 UI，没有可读取的正文）");
                }
                l.push_str(&format!("；{dir}{band}"));
                if x.group_id.is_some() && x.group_id == b.group_id {
                    l.push_str("，同组");
                }
                if x.frame_id.is_some() && x.frame_id == b.frame_id {
                    l.push_str("，同框");
                } else if let Some(f) = x.frame_id.as_ref().and_then(|f| self.canvas.block(f)).and_then(|f| f.title.clone()) {
                    l.push_str(&format!("，在框「{f}」内"));
                }
                if let Some(n) = order.iter().position(|i| *i == x.entity_id) {
                    l.push_str(&format!("，阅读顺序 {}", n + 1));
                }
                if let Some(v) = self.block_detail(x)? {
                    l.push_str(&format!("。{v}"));
                }
                lines.push(l);
            }
            if same.len() > 30 {
                lines.push(format!("- …画布上另有 {} 个 Block（用 ws_neighbors / ws_outline 展开）", same.len() - 30));
            }
            if lines.is_empty() {
                lines.push("- （同一画布上没有其他 Block）".into());
            }
            out.push_str(&lines.join("\n"));
            out.push_str("\n\n");
        }
        // 3. other canvases
        let mut canvas_lines = Vec::new();
        for s in self.canvas.surfaces.clone() {
            if origin.as_ref().is_some_and(|b| b.surface_id == s.entity_id) {
                continue;
            }
            let blocks: Vec<Block> = self.canvas.blocks.iter().filter(|b| b.surface_id == s.entity_id).cloned().collect();
            canvas_lines.push(format!("- 画布 {}「{}」：{} 个 Block", self.handle_of(&s.entity_id), s.title, blocks.len()));
            for b in blocks.iter().take(12) {
                let mut l = format!("  - {} {}「{}」", self.handle_of(&b.entity_id), renderer_label(&b.renderer), b.title.clone().unwrap_or_default());
                if let Some(src) = &b.source_id {
                    l.push_str(&format!(" → {}", self.handle_of(src)));
                }
                if let Some(v) = self.block_detail(b)? {
                    l.push_str(&format!("。{v}"));
                }
                canvas_lines.push(l);
            }
            if blocks.len() > 12 {
                canvas_lines.push(format!("  - …另有 {} 个（ws_outline {}）", blocks.len() - 12, self.handle_of(&s.entity_id)));
            }
        }
        if !canvas_lines.is_empty() {
            out.push_str("## 其他画布\n");
            out.push_str(&canvas_lines.join("\n"));
            out.push_str("\n\n");
        }
        // 4. data
        out.push_str("## 数据\n");
        let prompt = self.wish.payload.get("prompt").and_then(Value::as_str).unwrap_or("").to_string();
        let declared: BTreeSet<String> = self.inputs.iter().map(|i| i.entity_id.clone()).collect();
        let near: BTreeSet<String> = origin
            .as_ref()
            .map(|b| self.canvas.blocks.iter().filter(|x| x.surface_id == b.surface_id).filter_map(|x| x.source_id.clone()).collect())
            .unwrap_or_default();
        let mut entries: Vec<(u8, EntityRow)> = Vec::new();
        for e in self.data_entities()? {
            if e.entity_id == self.wish.entity_id || e.type_id == TYPE_BLOCK_DEF {
                continue;
            }
            if e.payload.get("system").is_some() && e.type_id == TYPE_CONTAINER {
                continue;
            }
            let t = title_of(&e);
            let rank = if declared.contains(&e.entity_id) {
                0
            } else if !t.is_empty() && prompt.contains(&t) {
                1
            } else if near.contains(&e.entity_id) {
                2
            } else {
                3
            };
            entries.push((rank, e));
        }
        let total = entries.len();
        let mut sorted = entries.clone();
        sorted.sort_by_key(|(r, _)| *r);
        let mut shown = BTreeSet::new();
        let mut data_lines = Vec::new();
        let mut used = out.chars().count();
        for (_, e) in sorted {
            let line = self.data_line(&e)?;
            if used + line.chars().count() > budget.saturating_mul(3) / 4 && !declared.contains(&e.entity_id) {
                continue;
            }
            used += line.chars().count();
            shown.insert(e.entity_id.clone());
            data_lines.push((e.entity_id.clone(), line));
        }
        // keep tree order for what is shown
        let order: Vec<String> = entries.iter().map(|(_, e)| e.entity_id.clone()).collect();
        data_lines.sort_by_key(|(id, _)| order.iter().position(|x| x == id));
        for (_, l) in &data_lines {
            out.push_str(l);
        }
        if shown.len() < total {
            let mut folded: BTreeMap<String, usize> = BTreeMap::new();
            for (_, e) in &entries {
                if !shown.contains(&e.entity_id) {
                    let parent = self.ctx().edge(&e.entity_id)?.map(|x| x.parent_id).unwrap_or_default();
                    *folded.entry(parent).or_default() += 1;
                }
            }
            for (p, n) in folded {
                let ph = self.handle_of(&p);
                out.push_str(&format!("- …{} {} 下另有 {n} 项未列出（ws_outline {ph} 展开）\n", ph, self.path_of(&p)?));
            }
        }
        out.push('\n');
        // 5. knowledge and annotations
        let mut kn = Vec::new();
        if let Some(k) = self.wish.payload.get("knowledge").and_then(Value::as_str).filter(|k| !k.trim().is_empty()) {
            kn.push(format!("- 许愿格知识：{}", k.trim()));
        }
        for r in self.wish.payload.get("refinements").and_then(Value::as_array).into_iter().flatten() {
            if let Some(t) = r["text"].as_str() {
                kn.push(format!("- 修改意见：{t}"));
            }
        }
        let targets: Vec<String> = declared.iter().cloned().chain(near.iter().cloned()).collect();
        if !targets.is_empty() {
            let ctx = self.snap.ctx();
            let anns = aiworkspace_core::read::list_annotations(&ctx, &self.snap.access, &targets, None).unwrap_or_default();
            drop(ctx);
            for a in anns.iter().take(30) {
                let body = a["content"]["payload"]["body"].as_str().unwrap_or("");
                if body.is_empty() {
                    continue;
                }
                let target = a["content"]["payload"]["target"]["entity_id"].as_str().unwrap_or("").to_string();
                let quote = a["content"]["payload"]["context"]["quote"].as_str().map(|q| format!("（针对“{}”）", q.chars().take(40).collect::<String>())).unwrap_or_default();
                kn.push(format!("- 批注（在 {} 上{quote}）：{body}", self.handle_of(&target)));
            }
        }
        if let Some(b) = &origin {
            let notes: Vec<Block> = self
                .canvas
                .blocks
                .iter()
                .filter(|x| x.surface_id == b.surface_id && x.renderer == "note" && b.rect.gap(&x.rect) <= 600.0)
                .cloned()
                .collect();
            for n in notes {
                if let Some(a) = n.source_id.as_ref().and_then(|s| self.snap.entity(s).ok().flatten()) {
                    if let Some(body) = a.payload.get("body").and_then(Value::as_str).filter(|b| !b.is_empty()) {
                        kn.push(format!("- 附近的便签 {}：{body}", self.handle_of(&n.entity_id)));
                    }
                }
            }
        }
        if !kn.is_empty() {
            out.push_str("## 知识与批注（作者写给这个任务的说明）\n");
            out.push_str(&kn.join("\n"));
            out.push_str("\n\n");
        }
        // 6. last result
        let results = aiworkspace_core::wish::current_results(&self.wish.payload);
        if !results.is_empty() || self.wish.payload.get("program").is_some() {
            out.push_str("## 上次结果\n");
            let fresh = aiworkspace_core::freshness::entity_freshness(&self.ctx(), &self.snap.access, &self.wish.entity_id).unwrap_or(Value::Null);
            out.push_str(&format!("新鲜度：{}。重跑时沿用相同的结果名、字段和键，保持结果身份稳定。\n", fresh["status"].as_str().unwrap_or("unknown")));
            for (name, b) in &results {
                let id = b["entity_id"].as_str().unwrap_or("");
                let mut l = format!("- `{name}`（{}，{}）→ {}", b["type"].as_str().unwrap_or(""), b["approach"].as_str().unwrap_or(""), self.handle_of(id));
                if let Some(k) = b.get("key").and_then(Value::as_array) {
                    l.push_str(&format!("，键 {}", k.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("+")));
                }
                if let Some(f) = b.get("fields").and_then(Value::as_object) {
                    l.push_str(&format!("，字段 {}", f.keys().cloned().collect::<Vec<_>>().join("、")));
                }
                out.push_str(&l);
                out.push('\n');
            }
            if let Some(p) = self.wish.payload.get("program") {
                out.push_str(&format!(
                    "- 已有程序 program/main.js（产出 {}），在它上面修改，不要从头重写。\n",
                    p["produces"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("、")).unwrap_or_default()
                ));
            }
            out.push('\n');
        }
        // 7. renderer catalog
        out.push_str("## Renderer 目录（完整说明见 catalog/renderers.json）\n");
        for r in self.catalog_json()?.as_array().into_iter().flatten() {
            out.push_str(&format!(
                "- `{}` {} — 数据：{}{}\n",
                r["renderer"].as_str().unwrap_or(""),
                r["title"].as_str().unwrap_or(""),
                r["accepts"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(|t| t.trim_start_matches("buckyos.")).collect::<Vec<_>>().join("/")).filter(|s| !s.is_empty()).unwrap_or_else(|| "无".into()),
                r["config_summary"].as_str().map(|s| format!("；配置 {s}")).unwrap_or_default()
            ));
        }
        Ok(out)
    }

    fn data_line(&mut self, e: &EntityRow) -> WsResult<String> {
        let h = self.handle_of(&e.entity_id);
        let p = self.profile(e)?;
        let mut l = format!("- {h} {}「{}」 {} · {}", type_label(&e.type_id), title_of(e), self.path_of(&e.entity_id)?, profile_line(&p));
        let showing: Vec<String> = self.canvas.showing(&e.entity_id).iter().map(|b| b.entity_id.clone()).collect::<Vec<_>>();
        if !showing.is_empty() {
            let hs: Vec<String> = showing.iter().take(6).map(|b| self.handle_of(b)).collect();
            l.push_str(&format!("（显示于 {}）", hs.join("、")));
        }
        if let Some(g) = p.get("generated").filter(|g| !g.is_null()) {
            let w = g["wish_id"].as_str().map(|w| self.handle_of(w)).unwrap_or_default();
            l.push_str(&format!(" · 许愿格 {w} 的结果（{}）", g["status"].as_str().unwrap_or("")));
        }
        l.push('\n');
        if e.type_id == TYPE_TABLE {
            let lines = field_lines(&p, 16);
            for chunk in lines.chunks(3) {
                l.push_str(&format!("  - {}\n", chunk.join(" · ")));
            }
            if p["fields"].as_array().map_or(0, Vec::len) > 16 {
                l.push_str("  - …更多字段见 ws_profile\n");
            }
            for i in p["issues"].as_array().into_iter().flatten().take(3) {
                l.push_str(&format!("  - ⚠ {}\n", i.as_str().unwrap_or("")));
            }
        }
        Ok(l)
    }

    /// View conditions / chart configuration of a Block, by field names.
    fn block_detail(&mut self, b: &Block) -> WsResult<Option<String>> {
        let Some(src) = &b.source_id else { return Ok(None) };
        let Some(t) = self.snap.entity(src)?.filter(|t| t.type_id == TYPE_TABLE) else { return Ok(None) };
        let fields = live_fields(&self.ctx(), &t.entity_id)?;
        let fh = self.handles.fields(&t, &fields);
        let name = |fid: &str| -> String {
            match fields.iter().position(|f| f.field_id == fid) {
                Some(i) => format!("{}({})", fields[i].def.name, fh[i]),
                None => fid.to_string(),
            }
        };
        if b.renderer == "table" {
            let cell = self.snap.entity(&b.entity_id)?.ok_or_else(|| WsError::not_found("cell"))?;
            let s = view_summary(&cell, &fields);
            return Ok(if s.is_empty() { None } else { Some(format!("视图：{s}")) });
        }
        if let Some(cfg) = b.payload.get("config").and_then(Value::as_object) {
            let parts: Vec<String> = cfg
                .iter()
                .filter(|(k, _)| *k != "snapshot")
                .map(|(k, v)| match v.as_str() {
                    Some(s) if fields.iter().any(|f| f.field_id == s) => format!("{k}={}", name(s)),
                    _ => format!("{k}={}", v),
                })
                .collect();
            if !parts.is_empty() {
                return Ok(Some(format!("配置：{}", parts.join("，"))));
            }
        }
        Ok(None)
    }

    // ---- tools (§5.5)

    fn target_entity(&self, s: &str) -> WsResult<EntityRow> {
        let id = self.handles.resolve_entity(s)?;
        self.snap.readable(&id)
    }

    fn rows_out(&mut self, table: &EntityRow, fields: &[FieldRow], rows: &[Value]) -> Vec<Value> {
        let defs: BTreeMap<&str, &FieldRow> = fields.iter().map(|f| (f.field_id.as_str(), f)).collect();
        rows.iter()
            .map(|r| {
                let mut o = Map::new();
                o.insert("_h".into(), json!(self.handles.record(table, r["record_id"].as_str().unwrap_or(""))));
                for (k, v) in r["values"].as_object().into_iter().flatten() {
                    if let Some(f) = defs.get(k.as_str()) {
                        o.insert(f.def.name.clone(), plain_value(&f.def, v));
                    }
                }
                Value::Object(o)
            })
            .collect()
    }

    /// `ws_outline(target?, depth?)`
    pub fn ws_outline(&mut self, args: &Value) -> WsResult<String> {
        let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3) as usize;
        let roots: Vec<String> = match args.get("target").and_then(Value::as_str) {
            Some(t) => vec![self.target_entity(t)?.entity_id],
            None => vec![DATA_ID.to_string(), SURFACES_ID.to_string()],
        };
        let mut lines = Vec::new();
        fn rec(st: &mut Stage, id: &str, depth: usize, level: usize, lines: &mut Vec<String>) -> WsResult<()> {
            let children = st.ctx().children(id)?;
            for edge in children {
                let Some(e) = st.snap.entity(&edge.child_id)?.filter(|e| e.alive()) else { continue };
                if !st.snap.can_read(&e)? {
                    continue;
                }
                let h = st.handle_of(&e.entity_id);
                let mut l = format!("{}- {h} {}「{}」", "  ".repeat(level), type_label(&e.type_id), title_of(&e));
                if e.type_id == TYPE_CELL {
                    let r = e.payload.get("view").and_then(|v| v["type"].as_str()).unwrap_or("");
                    l.push_str(&format!(" renderer={r}"));
                    if let Some(s) = e.payload.get("source_ref").and_then(reference_entity_id) {
                        l.push_str(&format!(" → {}", st.handle_of(s)));
                    }
                } else if e.type_id != TYPE_CONTAINER {
                    let p = st.profile(&e)?;
                    l.push_str(&format!(" {}", profile_line(&p)));
                }
                lines.push(l);
                if e.type_id == TYPE_CONTAINER && level + 1 < depth {
                    rec(st, &e.entity_id, depth, level + 1, lines)?;
                } else if e.type_id == TYPE_CONTAINER {
                    let n = st.ctx().children(&e.entity_id)?.len();
                    if n > 0 {
                        lines.push(format!("{}  - …{n} 项（ws_outline {h}）", "  ".repeat(level)));
                    }
                }
            }
            Ok(())
        }
        for r in roots {
            let h = self.handle_of(&r);
            lines.push(format!("{h} {}", self.path_of(&r)?));
            rec(self, &r, depth, 0, &mut lines)?;
        }
        Ok(clip(lines.join("\n"), MAX_TOOL_CHARS))
    }

    /// `ws_find(text, kinds?)`
    pub fn ws_find(&mut self, args: &Value) -> WsResult<String> {
        let text = args.get("text").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("text required"))?.to_lowercase();
        if text.trim().is_empty() {
            return Err(WsError::invalid_op("text required"));
        }
        let kinds: Option<Vec<String>> = args.get("kinds").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
        let mut hits = Vec::new();
        let entities = self.data_entities()?;
        for e in entities {
            if let Some(k) = &kinds {
                let short = e.type_id.trim_start_matches("buckyos.").to_string();
                if !k.iter().any(|x| *x == short || *x == e.type_id) {
                    continue;
                }
            }
            let mut where_ = Vec::new();
            if title_of(&e).to_lowercase().contains(&text) {
                where_.push(format!("名称：{}", title_of(&e)));
            }
            match e.type_id.as_str() {
                TYPE_TABLE => {
                    let fields = live_fields(&self.ctx(), &e.entity_id)?;
                    let fh = self.handles.fields(&e, &fields);
                    for (i, f) in fields.iter().enumerate() {
                        if f.def.name.to_lowercase().contains(&text) {
                            where_.push(format!("字段 {} {}", fh[i], f.def.name));
                        }
                    }
                    let mut n = 0;
                    let mut first: Option<String> = None;
                    self.ctx().scan_records(&e.entity_id, &mut |r| {
                        if r.alive() && r.values.values().any(|v| v.to_string().to_lowercase().contains(&text)) {
                            n += 1;
                            if first.is_none() {
                                first = Some(r.record_id.clone());
                            }
                        }
                        Ok(n < 1000)
                    })?;
                    if n > 0 {
                        where_.push(format!("{n} 行的值包含该文本"));
                    }
                }
                TYPE_RICHTEXT => {
                    let ast = self.ctx().richtext(&e.entity_id)?.map(|r| r.meta.ast).unwrap_or(Value::Null);
                    let md = aiworkspace_core::markdown::from_richtext(&ast);
                    if let Some(pos) = md.to_lowercase().find(&text) {
                        let start = md[..pos.min(md.len())].char_indices().rev().nth(30).map(|(i, _)| i).unwrap_or(0);
                        let snippet: String = md[start..].chars().take(100).collect::<String>().replace('\n', " ");
                        where_.push(format!("正文：…{snippet}…"));
                    }
                }
                TYPE_RECORD | TYPE_ANNOTATION => {
                    let s = serde_json::to_string(&e.payload).unwrap_or_default().to_lowercase();
                    if s.contains(&text) {
                        where_.push("内容包含该文本".into());
                    }
                }
                _ => {}
            }
            if !where_.is_empty() {
                hits.push(format!("- {} {}「{}」 {}：{}", self.handle_of(&e.entity_id), type_label(&e.type_id), title_of(&e), self.path_of(&e.entity_id)?, where_.join("；")));
            }
            if hits.len() >= 30 {
                hits.push("- …匹配过多，请缩小查找词".into());
                break;
            }
        }
        for b in self.canvas.blocks.clone() {
            if b.title.as_deref().is_some_and(|t| t.to_lowercase().contains(&text)) && kinds.as_ref().map_or(true, |k| k.iter().any(|x| x == "cell" || x == "block")) {
                hits.push(format!("- {} Block「{}」 renderer={}{}", self.handle_of(&b.entity_id), b.title.clone().unwrap_or_default(), b.renderer, b.source_id.as_ref().map(|s| format!(" → {}", self.handle_of(s))).unwrap_or_default()));
            }
        }
        Ok(if hits.is_empty() { format!("没有找到包含“{text}”的数据或 Block。") } else { clip(hits.join("\n"), MAX_TOOL_CHARS) })
    }

    /// `ws_profile(target)`
    pub fn ws_profile(&mut self, args: &Value) -> WsResult<String> {
        let t = args.get("target").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("target required"))?;
        let mut e = self.target_entity(t)?;
        if e.type_id == TYPE_CELL {
            match e.payload.get("source_ref").and_then(reference_entity_id) {
                Some(s) => e = self.snap.readable(s)?,
                None => return Ok(format!("{t} 是纯 UI Block，没有数据。")),
            }
        }
        let mut p = self.profile(&e)?;
        p["handle"] = json!(self.handle_of(&e.entity_id));
        p["title"] = json!(title_of(&e));
        p["path"] = json!(self.path_of(&e.entity_id)?);
        Ok(clip(serde_json::to_string_pretty(&p).unwrap_or_default(), MAX_TOOL_CHARS))
    }

    /// `ws_neighbors(cell, radius?)`
    pub fn ws_neighbors(&mut self, args: &Value) -> WsResult<String> {
        let c = args.get("cell").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("cell required"))?;
        let id = self.handles.resolve_entity(c)?;
        let b = self.canvas.block(&id).cloned().ok_or_else(|| WsError::invalid_op(format!("{c} is not a Block on a canvas")))?;
        let radius = args.get("radius").and_then(Value::as_f64).unwrap_or(800.0);
        let mut near: Vec<Block> = self.canvas.blocks.iter().filter(|x| x.surface_id == b.surface_id && x.entity_id != id && b.rect.gap(&x.rect) <= radius).cloned().collect();
        near.sort_by(|x, y| b.rect.gap(&x.rect).partial_cmp(&b.rect.gap(&y.rect)).unwrap_or(std::cmp::Ordering::Equal));
        let mut lines = vec![format!("以 {c} 为原点（半径 {radius}px）：")];
        for x in near.iter().take(40) {
            let (dir, band, gap) = relation(&b.rect, &x.rect);
            let mut l = format!("- {} {}「{}」 {dir}{band}（间距 {:.0}px）", self.handle_of(&x.entity_id), renderer_label(&x.renderer), x.title.clone().unwrap_or_default(), gap);
            if let Some(s) = &x.source_id {
                l.push_str(&format!(" → {}", self.handle_of(s)));
            }
            if let Some(d) = self.block_detail(x)? {
                l.push_str(&format!("。{d}"));
            }
            lines.push(l);
        }
        Ok(clip(lines.join("\n"), MAX_TOOL_CHARS))
    }

    /// Execution stage: reading data that was not declared appends it (materialized, recorded).
    fn note_read(&mut self, e: &EntityRow) -> WsResult<Option<String>> {
        if self.kind == StageKind::Analyze || e.type_id == TYPE_CELL || e.type_id == TYPE_WISH || e.type_id == TYPE_BLOCK_DEF {
            return Ok(None);
        }
        if self.inputs.iter().any(|i| i.entity_id == e.entity_id) {
            return Ok(None);
        }
        let name = self.append_input(&e.entity_id, None)?;
        Ok(Some(format!("（{} 不在已声明的输入中：已作为执行时追加的输入 `{name}` 记入读集，程序中可用 aiws.input('{name}')）\n", self.handle_of(&e.entity_id))))
    }

    /// `ws_read(target, selector?, limit?)`
    pub fn ws_read(&mut self, args: &Value) -> WsResult<String> {
        let t = args.get("target").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("target required"))?;
        let target = self.handles.resolve(t)?;
        let mut e = self.target_entity(t)?;
        let mut prefix = String::new();
        if e.type_id == TYPE_CELL {
            let cell = e.clone();
            let mut info = json!({ "handle": self.handle_of(&cell.entity_id), "renderer": cell.payload.get("view").and_then(|v| v.get("type")), "title": cell.payload.get("title") });
            for k in ["filter", "sorts", "fields", "config", "bindings"] {
                if let Some(v) = cell.payload.get(k) {
                    info[k] = v.clone();
                }
            }
            match cell.payload.get("source_ref").and_then(reference_entity_id) {
                Some(s) => {
                    e = self.snap.readable(s)?;
                    prefix = format!("Block {}：{}\n数据 {}：\n", t, info, self.handle_of(s));
                    if e.type_id == TYPE_TABLE && cell.payload.get("view").and_then(|v| v["type"].as_str()) == Some("table") {
                        let q = json!({ "table_or_view": t, "limit": args.get("limit").cloned().unwrap_or(json!(20)) });
                        return Ok(format!("{prefix}{}", self.ws_query(&q)?));
                    }
                }
                None => return Ok(format!("Block {t} 是纯 UI（{}），没有可读取的正文。", info)),
            }
        }
        let note = self.note_read(&e)?.unwrap_or_default();
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, MAX_TOOL_ROWS as u64) as usize;
        let ctx = self.snap.ctx();
        let body = match e.type_id.as_str() {
            TYPE_TABLE => {
                drop(ctx);
                if let Target::Record(_, rid) = &target {
                    let ctx = self.snap.ctx();
                    let fields = live_fields(&ctx, &e.entity_id)?;
                    let r = ctx.record(&e.entity_id, rid)?.filter(|r| r.alive()).ok_or_else(|| WsError::not_found(format!("record {t} not found")))?;
                    drop(ctx);
                    let row = json!({ "record_id": r.record_id, "values": r.values });
                    serde_json::to_string_pretty(&self.rows_out(&e, &fields, &[row])).unwrap_or_default()
                } else {
                    let q = json!({ "table_or_view": t, "limit": limit, "cursor": args.get("cursor"), "fields": args.get("fields"), "filter": args.get("filter") });
                    return Ok(format!("{note}{prefix}{}", self.ws_query(&q)?));
                }
            }
            TYPE_RICHTEXT => {
                let ast = ctx.richtext(&e.entity_id)?.map(|r| r.meta.ast).unwrap_or(Value::Null);
                aiworkspace_core::markdown::from_richtext(&ast)
            }
            TYPE_RECORD => {
                let schema = aiworkspace_core::types::record_schema(&e.payload).unwrap_or_default();
                let nested = aiworkspace_core::types::record_nested(&e.payload);
                let props: Map<String, Value> = schema.values().map(|d| (d.name.clone(), nested["props"].get(&d.field_id).map(|v| plain_value(d, v)).unwrap_or(Value::Null))).collect();
                serde_json::to_string_pretty(&props).unwrap_or_default()
            }
            TYPE_ASSET => serde_json::to_string_pretty(&json!({ "media_type": e.payload.get("media_type"), "size": e.payload.get("size"), "file_name": e.payload.get("file_name"),
                "image": e.payload.get("image"), "note": "资产内容已放在 context/assets/ 下（执行阶段），可用 read_file 或程序读取" })).unwrap_or_default(),
            TYPE_CONTAINER => {
                drop(ctx);
                return Ok(format!("{note}{}", self.ws_outline(&json!({ "target": t, "depth": 2 }))?));
            }
            TYPE_ANNOTATION => serde_json::to_string_pretty(&json!({ "body": e.payload.get("body"), "target": e.payload.get("target"), "context": e.payload.get("context") })).unwrap_or_default(),
            TYPE_WISH => serde_json::to_string_pretty(&json!({ "prompt": e.payload.get("prompt"), "knowledge": e.payload.get("knowledge"),
                "context_prompt": e.payload.get("analysis").and_then(|a| a.get("context_prompt")) })).unwrap_or_default(),
            _ => serde_json::to_string_pretty(&e.payload).unwrap_or_default(),
        };
        Ok(clip(format!("{note}{prefix}{} {}「{}」\n{body}", self.handle_of(&e.entity_id), type_label(&e.type_id), title_of(&e)), MAX_TOOL_CHARS))
    }

    /// `ws_query(table_or_view, filter?, sorts?, fields?, limit?, cursor?)`
    pub fn ws_query(&mut self, args: &Value) -> WsResult<String> {
        let t = args.get("table_or_view").or_else(|| args.get("target")).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("table_or_view required"))?;
        let mut e = self.target_entity(t)?;
        let mut view: Option<EntityRow> = None;
        if e.type_id == TYPE_CELL {
            let src = e.payload.get("source_ref").and_then(reference_entity_id).map(str::to_string);
            if e.payload.get("view").and_then(|v| v["type"].as_str()) == Some("table") {
                view = Some(e.clone());
            }
            e = self.snap.readable(&src.ok_or_else(|| WsError::invalid_op(format!("{t} shows no table")))?)?;
        }
        if e.type_id != TYPE_TABLE {
            return Err(WsError::invalid_op(format!("{t} is not a table or a table view")));
        }
        let note = self.note_read(&e)?.unwrap_or_default();
        let ctx = self.snap.ctx();
        let fields = live_fields(&ctx, &e.entity_id)?;
        drop(ctx);
        let fh = self.handles.fields(&e, &fields);
        let owned = self.owned.get(&e.entity_id).cloned().unwrap_or_default();
        let fields: Vec<FieldRow> = fields.into_iter().filter(|f| !owned.contains(&f.field_id)).collect();
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, MAX_TOOL_ROWS as u64) as usize;
        let mut spec = QuerySpec { limit, with_meta: false, cursor: args.get("cursor").and_then(Value::as_str).map(str::to_string), ..Default::default() };
        if let Some(c) = &view {
            spec.filter = c.payload.get("filter").filter(|f| !f.is_null()).cloned();
            spec.sorts = c.payload.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default();
            spec.manual_order = c.payload.get("manual_order").and_then(Value::as_object).cloned();
        }
        if let Some(f) = args.get("filter").filter(|f| !f.is_null()) {
            let extra = translate_filter(f, &fields, &self.handles, &e.entity_id)?;
            spec.filter = Some(match spec.filter.take() {
                Some(v) => json!({ "op": "and", "args": [v, extra] }),
                None => extra,
            });
        }
        if let Some(s) = args.get("sorts").filter(|s| !s.is_null()) {
            spec.sorts = translate_sorts(s, &fields, &self.handles, &e.entity_id)?;
        }
        let proj = match args.get("fields").filter(|f| !f.is_null()) {
            Some(f) => Some(translate_fields(f, &fields, &self.handles, &e.entity_id)?),
            None => None,
        };
        spec.fields = Some(proj.clone().unwrap_or_else(|| fields.iter().map(|f| f.field_id.clone()).collect()));
        let ctx = self.snap.ctx();
        let page = run_query(&ctx, &e, &spec)?;
        drop(ctx);
        let rows = self.rows_out(&e, &fields, page["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]));
        let cols: Vec<String> = fields
            .iter()
            .filter(|f| spec.fields.as_ref().map_or(true, |p| p.contains(&f.field_id)))
            .map(|f| {
                let i = fh.iter().zip(live_fields(&self.ctx(), &e.entity_id).unwrap_or_default()).find(|(_, x)| x.field_id == f.field_id).map(|(h, _)| h.clone()).unwrap_or_default();
                format!("{} {} {}", i, f.def.name, f.def.ty.as_str())
            })
            .collect();
        let mut out = format!(
            "{note}{} 表「{}」{}：共 {} 行，本页 {} 行\n字段：{}\n",
            self.handle_of(&e.entity_id),
            title_of(&e),
            view.as_ref().map(|c| format!("（按视图 {} 读取：{}）", self.handle_of(&c.entity_id), view_summary(c, &fields))).unwrap_or_default(),
            page["total"],
            rows.len(),
            cols.join(" · ")
        );
        out.push_str(&serde_json::to_string(&rows).unwrap_or_default().replace("},{", "},\n{"));
        if let Some(c) = page.get("next_cursor") {
            out.push_str(&format!("\n下一页 cursor: {c}（大量行请改用程序处理）"));
        }
        Ok(clip(out, MAX_TOOL_CHARS))
    }

    /// Read set of this stage as wish / derived inputs.
    pub fn read_set(&self) -> WsResult<Vec<Value>> {
        self.reads.to_inputs(&self.ctx())
    }

    /// The host evidence a candidate carries (§4.3): what was read, what was appended, which wish
    /// configuration the run answered, and where in history it started.
    pub fn evidence(&self) -> WsResult<Value> {
        let mut key_revs = Map::new();
        for k in super::plan::GUARDED_KEYS {
            key_revs.insert((*k).to_string(), json!(self.wish.key_rev(k)));
        }
        Ok(json!({
            "wish_id": self.wish.entity_id, "run_id": self.run_id, "stage": self.kind.as_str(),
            "read_set": self.read_set()?, "appended_inputs": self.appended,
            "basis": { "key_revs": key_revs, "config_digest": aiworkspace_core::wish::config_digest(&self.wish.payload) },
            "epoch": self.snap.epoch, "head_seq": self.snap.head_seq, "location": self.location,
            "inputs": self.inputs.iter().map(|i| json!({ "name": i.name, "entity_id": i.entity_id, "type_id": i.type_id, "selector": i.selector, "appended": i.appended })).collect::<Vec<_>>(),
        }))
    }

    /// What the collector needs to know about the bound inputs (table record ids for derived columns).
    pub fn input_infos(&self) -> WsResult<std::collections::BTreeMap<String, super::results::InputInfo>> {
        let mut out = std::collections::BTreeMap::new();
        for i in &self.inputs {
            let records = if i.type_id == TYPE_TABLE {
                let path = self.workdir.join("context").join("entities").join(&i.entity_id).join("rows.jsonl");
                let ids: BTreeSet<String> = std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                    .filter_map(|v| v["id"].as_str().map(str::to_string))
                    .collect();
                Some(ids)
            } else {
                None
            };
            out.insert(i.name.clone(), super::results::InputInfo { entity_id: i.entity_id.clone(), type_id: i.type_id.clone(), records });
        }
        Ok(out)
    }
}

/// `筛选：地区=华东；按 销售额 降序；显示 6/11 列`
pub fn view_summary(cell: &EntityRow, fields: &[FieldRow]) -> String {
    let name = |fid: &str| fields.iter().find(|f| f.field_id == fid).map(|f| f.def.name.clone()).unwrap_or_else(|| fid.to_string());
    let mut parts = Vec::new();
    if let Some(f) = cell.payload.get("filter").filter(|f| !f.is_null()) {
        parts.push(format!("筛选：{}", filter_text(f, &name, fields)));
    }
    if let Some(s) = cell.payload.get("sorts").and_then(Value::as_array).filter(|s| !s.is_empty()) {
        let t: Vec<String> = s.iter().map(|x| format!("{} {}", name(x["field_id"].as_str().unwrap_or("")), if x["direction"] == json!("desc") { "降序" } else { "升序" })).collect();
        parts.push(format!("按 {}", t.join("、")));
    }
    if let Some(g) = cell.payload.get("group").and_then(|g| g["field_id"].as_str()) {
        parts.push(format!("按 {} 分组", name(g)));
    }
    if let Some(fl) = cell.payload.get("fields").and_then(Value::as_array) {
        let shown = fl.iter().filter(|f| f["hidden"] != json!(true)).count();
        if shown < fields.len() {
            parts.push(format!("显示 {shown}/{} 列", fields.len()));
        }
    }
    parts.join("；")
}

fn filter_text(f: &Value, name: &dyn Fn(&str) -> String, fields: &[FieldRow]) -> String {
    match f["op"].as_str().unwrap_or("") {
        "and" | "or" => {
            let sep = if f["op"] == json!("and") { " 且 " } else { " 或 " };
            f["args"].as_array().into_iter().flatten().map(|a| filter_text(a, name, fields)).collect::<Vec<_>>().join(sep)
        }
        "not" => format!("非（{}）", filter_text(&f["arg"], name, fields)),
        "cmp" => {
            let fid = f["field_id"].as_str().unwrap_or("");
            let label = |v: &Value| -> String {
                let def = fields.iter().find(|x| x.field_id == fid);
                match (v.as_str(), def.and_then(|d| d.def.options.as_ref())) {
                    (Some(s), Some(o)) => o.iter().find(|x| x.option_id == s).map(|x| x.label.clone()).unwrap_or_else(|| s.to_string()),
                    (Some(s), None) => s.to_string(),
                    _ => v.to_string(),
                }
            };
            let op = match f["operator"].as_str().unwrap_or("") {
                "eq" => "=",
                "ne" => "≠",
                "lt" => "<",
                "lte" => "≤",
                "gt" => ">",
                "gte" => "≥",
                "contains" => " 包含 ",
                "starts_with" => " 开头为 ",
                "in" => " 属于 ",
                "is_empty" => " 为空",
                "is_not_empty" => " 不为空",
                o => o,
            };
            let v = match &f["value"] {
                Value::Null => String::new(),
                Value::Array(a) => a.iter().map(|x| label(x)).collect::<Vec<_>>().join("/"),
                v => label(v),
            };
            format!("{}{op}{v}", name(fid))
        }
        _ => f.to_string(),
    }
}

pub fn type_label(t: &str) -> &'static str {
    match t {
        TYPE_TABLE => "表",
        TYPE_RICHTEXT => "富文本",
        TYPE_RECORD => "记录",
        TYPE_ASSET => "资产",
        TYPE_ANNOTATION => "批注",
        TYPE_WISH => "许愿格",
        TYPE_BLOCK_DEF => "Block 定义",
        TYPE_CONTAINER => "文件夹",
        TYPE_CELL => "Block",
        _ => "对象",
    }
}

pub fn renderer_label(r: &str) -> String {
    match r {
        "table" => "表格视图".into(),
        "richtext" => "富文本".into(),
        "record" => "记录".into(),
        "asset" => "图片/附件".into(),
        "note" => "便签".into(),
        "frame" => "框".into(),
        "shape" => "形状".into(),
        "wish" => "许愿格".into(),
        other => format!("Block[{other}]"),
    }
}

/// Is `s` a valid entity id (used by callers resolving handles into persistent ids).
pub fn looks_like_id(s: &str) -> bool {
    is_valid_id(s)
}
