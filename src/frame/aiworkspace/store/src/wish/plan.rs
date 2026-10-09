//! The result planner (许愿格 §10, §11): one candidate (`wish.results.v2`, from any executor) →
//! the ordinary operations and preconditions of exactly one Commit, plus the preview the user
//! confirms. Deterministic for a given document state, candidate and choices; the preview's
//! `plan_digest` names the plan the user saw and is the only plan that can be applied.
//!
//! Identity rules: a logical result keeps its data entity through `last_run.result_bindings`
//! (never inferred from titles); table rows keep their record ids through the logical key
//! (`record_id_for_key`), fields through their names; a type change of a logical result is a
//! conflict, not a silent conversion; results missing this time are listed, never deleted.

use crate::docdb::SqlCtx;
use crate::workspace::{Caller, Workspace};
use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_core::filter::live_fields;
use aiworkspace_core::model::*;
use aiworkspace_core::order_key::order_key_between;
use aiworkspace_core::value::{reference_entity_id, FieldType};
use aiworkspace_core::wish::{config_digest, current_results, record_id_for_key, with_basis};
use aiworkspace_core::{richtext, WsError, WsResult};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Wish keys whose revision at the start of the run guards the application (§11.2).
pub const GUARDED_KEYS: &[&str] = &["prompt", "knowledge", "refinements", "analysis", "inputs", "executor", "executor_config", "program", "output", "output_mode", "last_run"];
/// Rows per insert / set operation.
const CHUNK: usize = 5000;
/// A commit request above this size is refused before submission (the service limit is 8 MiB).
pub const MAX_PLAN_BYTES: usize = 7 * 1024 * 1024 + 512 * 1024;

pub fn default_renderer(ty: &str) -> &'static str {
    match ty {
        "table" => "table",
        "richtext" => "richtext",
        "record" => "record",
        "image" | "asset" => "asset",
        "video" => "sample.video",
        _ => "richtext",
    }
}

fn default_size(renderer: &str, ty: &str) -> (f64, f64) {
    match (renderer, ty) {
        ("table", _) => (560.0, 320.0),
        ("richtext", _) => (460.0, 300.0),
        ("record", _) => (360.0, 240.0),
        ("asset", _) => (360.0, 280.0),
        ("sample.metric", _) => (240.0, 140.0),
        (_, "html") => (480.0, 360.0),
        _ => (420.0, 300.0),
    }
}

fn short_hash(s: &str, n: usize) -> String {
    sha256_hex(s.as_bytes())[..n].to_string()
}

/// A lowercase id-safe slug of a result name.
pub fn slug(name: &str) -> String {
    let ascii: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let ascii = ascii.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let tail = short_hash(name, 6);
    if ascii.is_empty() {
        format!("r{tail}")
    } else {
        format!("{}-{tail}", ascii.chars().take(20).collect::<String>().trim_matches('-'))
    }
}

/// A valid, bounded entity id from parts.
fn make_id(parts: &[&str]) -> String {
    let joined = parts.join("-");
    if joined.len() <= 60 && aiworkspace_core::id::is_valid_id(&joined) {
        joined
    } else {
        format!("w{}", short_hash(&joined, 30))
    }
}

/// A plain value (as programs write it) in the stored form of `ty`; `None` = no value.
pub fn store_value(ty: FieldType, scale: Option<u8>, options: &BTreeMap<String, String>, v: &Value) -> Option<Value> {
    if v.is_null() || v.as_str() == Some("") && ty != FieldType::Text {
        return None;
    }
    Some(match ty {
        FieldType::Text => match v {
            Value::String(s) => json!(s),
            other => json!(other.to_string()),
        },
        FieldType::Boolean => match v {
            Value::Bool(b) => json!(b),
            Value::String(s) => json!(matches!(s.as_str(), "true" | "是" | "yes" | "1")),
            Value::Number(n) => json!(n.as_f64().unwrap_or(0.0) != 0.0),
            _ => return None,
        },
        FieldType::Number => match v {
            Value::Number(n) => json!(n),
            Value::String(s) => json!(super::profile::numeric_text(s)?),
            _ => return None,
        },
        FieldType::Decimal => {
            let n = match v {
                Value::Number(n) => n.as_f64()?,
                Value::String(s) => super::profile::numeric_text(s)?,
                _ => return None,
            };
            json!(format!("{:.*}", scale.unwrap_or(2) as usize, n))
        }
        FieldType::Date => json!(v.as_str()?.chars().take(10).collect::<String>()),
        FieldType::Datetime => json!(v.as_str()?),
        FieldType::Select => json!(options.get(v.as_str()?)?),
        FieldType::MultiSelect => {
            let list: Vec<Value> = match v {
                Value::Array(a) => a.iter().filter_map(|x| x.as_str().and_then(|s| options.get(s)).map(|o| json!(o))).collect(),
                Value::String(s) => s.split([',', '，', '、']).map(str::trim).filter(|s| !s.is_empty()).filter_map(|s| options.get(s)).map(|o| json!(o)).collect(),
                _ => return None,
            };
            json!(list)
        }
        FieldType::ObjectRef => match v {
            Value::String(s) => json!({ "entity_id": s }),
            Value::Object(_) => v.clone(),
            _ => return None,
        },
    })
}

fn field_type(s: &str) -> FieldType {
    match s {
        "boolean" => FieldType::Boolean,
        "number" => FieldType::Number,
        "decimal" => FieldType::Decimal,
        "date" => FieldType::Date,
        "datetime" => FieldType::Datetime,
        "select" => FieldType::Select,
        "multi_select" => FieldType::MultiSelect,
        _ => FieldType::Text,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub operations: Vec<Value>,
    pub preconditions: Vec<Value>,
    /// Assets the plan references: `(object_id, absolute path of the bytes)` — re-staged at apply.
    pub assets: Vec<(String, String)>,
    pub summary: Value,
    pub digest: String,
    /// No blocking problem and every required choice was made.
    pub ready: bool,
}

impl Plan {
    pub fn to_json(&self) -> Value {
        json!({ "operations": self.operations, "preconditions": self.preconditions,
                "assets": self.assets.iter().map(|(o, p)| json!({ "object_id": o, "path": p })).collect::<Vec<_>>(),
                "summary": self.summary, "plan_digest": self.digest, "ready": self.ready })
    }
}

struct Planner<'a> {
    ctx: SqlCtx<'a>,
    ops: Vec<Value>,
    problems: Vec<String>,
    manual: Vec<Value>,
    structure: Vec<Value>,
    destructive: bool,
    confirm_structure: bool,
    results_summary: Vec<Value>,
    assets: Vec<(String, String)>,
    /// result name → entity id (links between results and html bindings)
    ids: BTreeMap<String, String>,
    inputs: BTreeMap<String, String>,
    last_key: BTreeMap<String, String>,
}

impl<'a> Planner<'a> {
    fn next_key(&mut self, parent: &str) -> WsResult<String> {
        let last = match self.last_key.get(parent) {
            Some(k) => Some(k.clone()),
            None => self.ctx.children(parent)?.last().map(|e| e.order_key.clone()),
        };
        let k = order_key_between(last.as_deref(), None)?;
        self.last_key.insert(parent.to_string(), k.clone());
        Ok(k)
    }

    fn alive(&self, id: &str) -> WsResult<Option<EntityRow>> {
        Ok(self.ctx.entity(id)?.filter(|e| e.alive()))
    }

    fn cells_of(&self, entity_id: &str) -> WsResult<Vec<EntityRow>> {
        let mut out = Vec::new();
        for r in self.ctx.refs_to(entity_id)? {
            if r.kind == "bind" {
                if let Some(c) = self.alive(&r.src_entity_id)?.filter(|c| c.type_id == TYPE_CELL) {
                    out.push(c);
                }
            }
        }
        Ok(out)
    }
}

/// What the planner needs besides the candidate: the run, the user's choices and where it started.
pub struct PlanRequest<'a> {
    pub run_id: &'a str,
    pub candidate: &'a Value,
    /// `{ results: { <name>: keep | replace | new }, confirm_structure?: bool }`
    pub choices: &'a Value,
    /// `{ cell_id?, surface_id? }`
    pub location: &'a Value,
}

impl Workspace {
    /// Plan the application of a candidate against the document as it is now (§11.1).
    pub fn wish_plan(&self, caller: &Caller, req: &PlanRequest) -> WsResult<Plan> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let cand = req.candidate;
        let wish_id = cand["wish_id"].as_str().ok_or_else(|| WsError::invalid_op("candidate.wish_id missing"))?.to_string();
        let wish = aiworkspace_core::read::readable_entity(&ctx, &access, &wish_id)?;
        if !wish.alive() {
            return Err(WsError::deleted("the wish was deleted"));
        }
        let mut p = Planner {
            ctx,
            ops: Vec::new(),
            problems: Vec::new(),
            manual: Vec::new(),
            structure: Vec::new(),
            destructive: false,
            confirm_structure: req.choices.get("confirm_structure") == Some(&json!(true)),
            results_summary: Vec::new(),
            assets: Vec::new(),
            ids: BTreeMap::new(),
            inputs: BTreeMap::new(),
            last_key: BTreeMap::new(),
        };
        let payload = &wish.payload;
        // the wish as it was when the run started: any change since means the candidate answers an old question
        let basis = cand.get("basis").and_then(|b| b.get("key_revs")).cloned().unwrap_or(json!({}));
        let mut preconditions: Vec<Value> = super::readset::preconditions(cand["read_set"].as_array().map(Vec::as_slice).unwrap_or(&[]));
        for k in GUARDED_KEYS {
            let at_start = basis.get(*k).and_then(Value::as_u64).unwrap_or(0);
            preconditions.push(json!({ "target": { "entity_id": wish_id, "selector": { "kind": "doc_key", "key": k } }, "expect": { "rev": at_start } }));
            if wish.key_rev(k) != at_start {
                p.problems.push(match *k {
                    "last_run" => "这个许愿格在本次运行开始后已经应用过另一组结果：请重新执行（或重新预览后选择新建）".to_string(),
                    _ => format!("运行开始后许愿格的 {k} 已被修改：这个候选对应旧的配置，不能应用"),
                });
            }
        }
        for i in payload.get("inputs").and_then(Value::as_array).into_iter().flatten().chain(cand["appended_inputs"].as_array().into_iter().flatten()) {
            if let (Some(n), Some(id)) = (i["name"].as_str(), i["entity_id"].as_str()) {
                p.inputs.insert(n.to_string(), id.to_string());
            }
        }

        // ---- result group: data folder + canvas group (§10.1)
        let mode = payload.get("output_mode").and_then(Value::as_str).unwrap_or("overwrite").to_string();
        let output = payload.get("output").cloned().unwrap_or(json!({}));
        let wish_parent = p.ctx.edge(&wish_id)?.map(|e| e.parent_id).unwrap_or_else(|| DATA_ID.to_string());
        let container = output.get("container_id").and_then(Value::as_str).unwrap_or(&wish_parent).to_string();
        if p.alive(&container)?.is_none() {
            p.problems.push(format!("输出位置 {container} 不存在：请修改许愿格的输出设置"));
        }
        let out_name = output.get("name").and_then(Value::as_str).unwrap_or("结果").to_string();
        let last_run = payload.get("last_run").cloned().unwrap_or(json!({}));
        let prev_bindings: BTreeMap<String, Value> = current_results(payload).into_iter().collect();
        let seq = last_run.get("seq").and_then(Value::as_u64).unwrap_or(0) + 1;
        // the wish's Block the run started from anchors the layout
        let wish_cells: Vec<EntityRow> = p.cells_of(&wish_id)?;
        let wanted_surface = output.get("surface_id").and_then(Value::as_str).map(str::to_string);
        let origin_cell = req
            .location
            .get("cell_id")
            .and_then(Value::as_str)
            .and_then(|c| wish_cells.iter().find(|x| x.entity_id == c))
            .or_else(|| wish_cells.iter().find(|c| wanted_surface.as_deref().map_or(true, |s| surface_of(&p.ctx, &c.entity_id).ok().flatten().as_deref() == Some(s))))
            .cloned();
        let surface = match (&origin_cell, &wanted_surface) {
            (Some(c), _) => surface_of(&p.ctx, &c.entity_id)?,
            (None, Some(s)) => Some(s.clone()),
            (None, None) => None,
        };
        if let Some(s) = &surface {
            if !p.alive(s)?.is_some_and(|e| e.payload.get("kind").and_then(Value::as_str) == Some("surface")) {
                p.problems.push(format!("输出画布 {s} 不存在：请修改许愿格的输出画布，结果不会落到别的画布上"));
            }
        }
        let anchor = match &origin_cell {
            Some(c) => abs_rect(&p.ctx, &c.entity_id)?,
            None => (40.0, 40.0, 0.0, 0.0),
        };
        let prev_group = last_run.get("result_bindings").and_then(|b| b.get("group")).and_then(Value::as_str).map(str::to_string);
        let prev_folder = last_run.get("result_bindings").and_then(|b| b.get("folder")).and_then(Value::as_str).map(str::to_string);
        let fresh = |p: &Planner, live: Option<String>, base: String| -> WsResult<String> {
            if let Some(l) = live.filter(|g| p.alive(g).ok().flatten().is_some()) {
                return Ok(l);
            }
            // a deleted entity keeps its id: never revive it by accident
            Ok(if p.ctx.entity(&base)?.is_some() { make_id(&[&base, req.run_id]) } else { base })
        };
        let (group_id, folder_id, title) = if mode == "overwrite" {
            (fresh(&p, prev_group, make_id(&[&wish_id, "out"]))?, fresh(&p, prev_folder, make_id(&[&wish_id, "out", "data"]))?, out_name.clone())
        } else {
            let mut n = seq;
            while p.ctx.entity(&make_id(&[&wish_id, &format!("run{n}")]))?.is_some() || p.ctx.entity(&make_id(&[&wish_id, &format!("run{n}"), "data"]))?.is_some() {
                n += 1;
            }
            (make_id(&[&wish_id, &format!("run{n}")]), make_id(&[&wish_id, &format!("run{n}"), "data"]), format!("{out_name} #{n}"))
        };
        let simulated = cand.get("simulated") == Some(&json!(true));
        let folder_exists = p.alive(&folder_id)?.is_some();
        if !folder_exists {
            let key = p.next_key(&container)?;
            p.ops.push(json!({ "op": "entity.create", "entity_id": folder_id, "type_id": TYPE_CONTAINER, "parent_id": container, "order_key": key,
                               "name": title, "payload": { "kind": "folder", "title": title } }));
        }

        // ---- results
        let results: Vec<Value> = cand["results"].as_array().cloned().unwrap_or_default();
        let empty_choices = json!({});
        let choices = req.choices.get("results").unwrap_or(&empty_choices);
        // pass 1: identities (links between results need them before content is built)
        let mut items = Vec::new();
        for r in &results {
            let name = r["name"].as_str().unwrap_or("").to_string();
            let ty = r["type"].as_str().unwrap_or("").to_string();
            let binding = if mode == "overwrite" { prev_bindings.get(&name).cloned() } else { None };
            let choice = choices.get(&name).and_then(Value::as_str).map(str::to_string);
            let mut entity_id = match (&binding, ty.as_str()) {
                (_, "table_columns") => r["columns"]["target_id"].as_str().unwrap_or("").to_string(),
                (Some(b), _) => b["entity_id"].as_str().unwrap_or("").to_string(),
                (None, _) => make_id(&[&group_id, &slug(&name)]),
            };
            let mut existing = if ty == "table_columns" { None } else { p.alive(&entity_id)? };
            if let (Some(b), Some(e)) = (&binding, &existing) {
                let was = b["type"].as_str().unwrap_or("");
                if was != ty && ty != "table_columns" {
                    p.problems.push(format!("结果「{name}」的类型从 {was} 变为 {ty}：不能原地转换，请换一个结果名，或改为“每次新建一组”"));
                }
                if e.derived.as_ref().and_then(|d| d.get("wish_id")).and_then(Value::as_str) != Some(wish_id.as_str()) {
                    p.problems.push(format!("结果「{name}」的目标 {} 不属于这个许愿格：不会接管", e.entity_id));
                }
            }
            if existing.is_none() && ty != "table_columns" {
                if let Some(other) = p.ctx.entity(&entity_id)? {
                    if other.alive() || other.type_id != type_for(&ty) {
                        // a deleted result of the same type would be revived by an id clash: take a fresh id instead
                        entity_id = make_id(&[&group_id, &slug(&name), req.run_id]);
                    } else {
                        entity_id = make_id(&[&group_id, &slug(&name), req.run_id]);
                    }
                }
            }
            let mut new_copy = false;
            if let Some(e) = &existing {
                let manual = e.derived.as_ref().is_some_and(|d| e.content_rev > d.get("generated_rev").and_then(Value::as_u64).unwrap_or(0));
                if manual {
                    match choice.as_deref() {
                        Some("keep") | Some("replace") => {}
                        Some("new") => {
                            new_copy = true;
                            entity_id = make_id(&[&group_id, &slug(&name), &format!("m{seq}")]);
                            existing = None;
                        }
                        _ => p.manual.push(json!({ "name": name, "entity_id": e.entity_id })),
                    }
                }
            }
            p.ids.insert(name.clone(), entity_id.clone());
            items.push(Item { r: r.clone(), name, ty, entity_id, existing, binding, choice, new_copy });
        }
        // the configuration the results are generated under: the wish as it is after this application
        let mut after = payload.clone();
        let mut wish_keys: Vec<Value> = Vec::new();
        let mut inputs_changed = false;
        if let Some(app) = cand["appended_inputs"].as_array().filter(|a| !a.is_empty()) {
            let mut list = payload.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default();
            for a in app {
                if !list.iter().any(|i| i["entity_id"] == a["entity_id"] && i.get("selector").is_none()) {
                    list.push(a.clone());
                    inputs_changed = true;
                }
            }
            if inputs_changed {
                after.insert("inputs".into(), json!(list));
                wish_keys.push(json!({ "key": "inputs", "value": list, "expect": { "rev": wish.key_rev("inputs") } }));
            }
        }
        let mut refinements_changed = false;
        if let Some(add) = cand["refinements"].as_array().filter(|a| !a.is_empty()) {
            let mut list = payload.get("refinements").and_then(Value::as_array).cloned().unwrap_or_default();
            list.extend(add.iter().cloned());
            let start = list.len().saturating_sub(aiworkspace_core::wish::MAX_REFINEMENTS);
            let list: Vec<Value> = list[start..].to_vec();
            after.insert("refinements".into(), json!(list));
            wish_keys.push(json!({ "key": "refinements", "value": list, "expect": { "rev": wish.key_rev("refinements") } }));
            refinements_changed = true;
        }
        let program_after = cand.get("program").filter(|p| p.is_object() && p.get("digest").is_some()).cloned();
        let mut program_change = Value::Null;
        if let Some(prog) = &program_after {
            let before = payload.get("program").and_then(|p| p.get("digest")).cloned();
            if before.as_ref() != prog.get("digest") {
                let stored = json!({ "language": "js", "api_version": aiworkspace_core::wish::PROGRAM_API_VERSION, "source": prog["object_id"], "digest": prog["digest"],
                                     "produces": prog.get("produces").cloned().unwrap_or(json!([])), "run_id": req.run_id });
                if let Some(path) = prog.get("path").and_then(Value::as_str) {
                    p.assets.push((prog["object_id"].as_str().unwrap_or("").to_string(), path.to_string()));
                }
                after.insert("program".into(), stored.clone());
                wish_keys.push(json!({ "key": "program", "value": stored, "expect": { "rev": wish.key_rev("program") } }));
                program_change = json!({ "from": before, "to": prog["digest"] });
            }
        }
        if (inputs_changed || refinements_changed) && after.get("analysis").is_some() {
            // the host re-states the analysis so its basis includes the inputs / refinements it sanctioned
            let a = after.get("analysis").cloned().unwrap_or(Value::Null);
            with_basis(&mut after);
            wish_keys.push(json!({ "key": "analysis", "value": a, "expect": { "rev": wish.key_rev("analysis") } }));
        }
        let config = config_digest(&after);
        let program_digest = after.get("program").and_then(|p| p.get("digest")).cloned();
        let derived_base = |name: &str, approach: &Value| {
            let mut d = json!({ "wish_id": wish_id, "run_id": req.run_id, "executor": cand["executor"], "inputs": cand["read_set"],
                                "simulated": simulated, "at": cand["executed_at"], "output_mode": mode, "group": group_id,
                                "config_digest": config, "result_key": name, "mode": cand.get("mode").cloned().unwrap_or(json!("generate")) });
            if approach.is_string() {
                d["approach"] = approach.clone();
            }
            if let Some(pd) = &program_digest {
                if approach == "program" {
                    d["program_digest"] = pd.clone();
                }
            }
            if cand["model_judgment"].as_array().is_some_and(|a| a.iter().any(|x| x == name)) {
                d["model_judgment"] = json!(true);
            }
            if cand["external_data"].as_array().is_some_and(|a| !a.is_empty()) && approach == "program" {
                d["external_data"] = json!(true);
            }
            d
        };

        // canvas group
        let group_exists = p.alive(&group_id)?.is_some();
        let mut cells_to_place: Vec<(String, Value, (f64, f64))> = Vec::new();
        let mut bindings_out: Map<String, Value> = Map::new();
        let mut produced: Vec<String> = Vec::new();
        for it in &items {
            let approach = it.r.get("approach").cloned().unwrap_or(Value::Null);
            let mut action = if it.existing.is_some() { "update" } else { "create" };
            let mut cells: Vec<String> = it.binding.as_ref().and_then(|b| b["cells"].as_array()).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
            let mut binding = json!({ "type": it.ty, "entity_id": it.entity_id, "approach": approach, "title": it.r.get("title") });
            if it.existing.is_some() && it.choice.as_deref() == Some("keep") && p.manual.iter().all(|m| m["name"] != json!(it.name)) {
                let is_manual = it.existing.as_ref().is_some_and(|e| e.derived.as_ref().is_some_and(|d| e.content_rev > d.get("generated_rev").and_then(Value::as_u64).unwrap_or(0)));
                if is_manual {
                    // the user confirms keeping their version: re-recorded with this run's read set (§11.3)
                    let mut d = derived_base(&it.name, &approach);
                    d["kept_manual"] = json!(true);
                    p.ops.push(json!({ "op": "entity.set_derived", "entity_id": it.entity_id, "derived": d }));
                    if let Some(prev) = &it.binding {
                        binding = prev.clone();
                    }
                    p.results_summary.push(json!({ "name": it.name, "type": it.ty, "entity_id": it.entity_id, "action": "keep_manual" }));
                    produced.push(it.entity_id.clone());
                    bindings_out.insert(it.name.clone(), binding);
                    continue;
                }
            }
            let parent = folder_id.clone();
            let mut row_order: Option<Vec<String>> = None;
            let changed = match it.ty.as_str() {
                "richtext" => plan_richtext(&mut p, it, &parent)?,
                "record" | "video" => plan_record(&mut p, it, &parent)?,
                "table" => plan_table(&mut p, it, &parent, &mut binding, &mut row_order)?,
                "table_columns" => plan_columns(&mut p, it, &wish_id, req.run_id, it.choice.as_deref(), &mut binding)?,
                "image" | "asset" => plan_asset(&mut p, it, &parent)?,
                "html" => plan_html(&mut p, it, &parent, &wish_id, &mut binding)?,
                t => {
                    p.problems.push(format!("结果「{}」的类型 {t} 不支持", it.name));
                    false
                }
            };
            if it.existing.is_some() && !changed {
                action = "unchanged";
            }
            if it.new_copy {
                action = "new_copy";
            }
            if it.ty != "table_columns" {
                p.ops.push(json!({ "op": "entity.set_derived", "entity_id": it.entity_id, "derived": derived_base(&it.name, &approach) }));
                produced.push(it.entity_id.clone());
            }
            // the program's row order is shown by its table Blocks (records have no order of their own)
            let order = row_order.filter(|ids| ids.len() <= MAX_ORDERED_ROWS).map(|ids| manual_order(&ids));
            let order_digest = order.as_ref().map(|o| sha256_hex(canonical_json(o).unwrap_or_default().as_bytes())[..16].to_string());
            let mut reordered = false;
            if let Some(d) = &order_digest {
                binding["order_digest"] = json!(d);
            }
            // Blocks: existing ones keep their place, size and view settings (§10.2)
            cells.retain(|c| p.alive(c).ok().flatten().is_some());
            if it.existing.is_some() && cells.is_empty() {
                cells = p.cells_of(&it.entity_id)?.into_iter().map(|c| c.entity_id).collect();
            }
            if surface.is_some() && cells.is_empty() && it.ty != "table_columns" {
                let views: Vec<Value> = match it.r.get("views").and_then(Value::as_array).filter(|v| !v.is_empty()) {
                    Some(v) => v.clone(),
                    None => vec![json!({ "renderer": it.r.get("renderer").and_then(Value::as_str).unwrap_or(default_renderer(&it.ty)) })],
                };
                for (vi, v) in views.iter().enumerate() {
                    // an HTML result is shown by the generic `html` Block running its definition (`def_ref`)
                    let renderer = if it.ty == "html" { "html".to_string() } else { v["renderer"].as_str().unwrap_or(default_renderer(&it.ty)).to_string() };
                    let cell_id = make_id(&[&it.entity_id, &format!("v{vi}")]);
                    let (w, h) = match v.get("size") {
                        Some(s) => (s["w"].as_f64().unwrap_or(400.0), s["h"].as_f64().unwrap_or(280.0)),
                        None => it.r.get("size").map(|s| (s["w"].as_f64().unwrap_or(400.0), s["h"].as_f64().unwrap_or(280.0))).unwrap_or_else(|| default_size(&renderer, &it.ty)),
                    };
                    let mut cp = json!({ "view": { "type": renderer }, "title": v.get("title").or_else(|| it.r.get("title")).cloned().unwrap_or(json!(it.name)) });
                    if it.ty == "html" {
                        cp["def_ref"] = json!({ "entity_id": it.entity_id });
                        if let Some(b) = binding.get("bindings").filter(|b| b.as_object().is_some_and(|o| !o.is_empty())) {
                            cp["bindings"] = b.clone();
                        }
                    } else {
                        cp["source_ref"] = json!({ "entity_id": it.entity_id });
                    }
                    if let Some(cfg) = v.get("config").filter(|c| c.is_object()) {
                        cp["config"] = map_view_config(cfg, &binding);
                    } else if let Some(cfg) = it.r.get("config").filter(|c| c.is_object()) {
                        cp["config"] = cfg.clone();
                    }
                    if renderer == "asset" {
                        cp["options"] = json!({ "fit": "contain" });
                    }
                    if let (true, Some(o)) = (renderer == "table", &order) {
                        cp["manual_order"] = o.clone();
                    }
                    cells_to_place.push((cell_id.clone(), cp, (w, h)));
                    cells.push(cell_id);
                }
            } else if let Some(o) = &order {
                // a table Block follows the new row order unless the user sorted or reordered it
                let prev = it.binding.as_ref().and_then(|b| b.get("order_digest")).and_then(Value::as_str);
                for c in &cells {
                    let Some(cell) = p.alive(c)? else { continue };
                    let is_table = cell.payload.get("view").and_then(|v| v["type"].as_str()) == Some("table");
                    let sorted = cell.payload.get("sorts").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
                    let current = cell.payload.get("manual_order").filter(|m| !m.is_null());
                    let current_digest = current.map(|m| sha256_hex(canonical_json(m).unwrap_or_default().as_bytes())[..16].to_string());
                    if is_table && !sorted && current_digest.as_deref() == prev && current != Some(o) {
                        p.ops.push(json!({ "op": "entity.set_keys", "entity_id": c, "keys": [{ "key": "manual_order", "value": o, "expect": { "rev": cell.key_rev("manual_order") } }] }));
                        reordered = true;
                    }
                }
            } else if it.ty == "html" {
                // a regenerated HTML Block keeps its place; its bindings follow the result
                for c in &cells {
                    if let Some(cell) = p.alive(c)? {
                        let want = binding.get("bindings").cloned().unwrap_or(json!({}));
                        if cell.payload.get("bindings").cloned().unwrap_or(json!({})) != want {
                            p.ops.push(json!({ "op": "entity.set_keys", "entity_id": c, "keys": [{ "key": "bindings", "value": want, "expect": { "rev": cell.key_rev("bindings") } }] }));
                        }
                    }
                }
            }
            binding["cells"] = json!(cells);
            p.results_summary.push(json!({ "name": it.name, "type": it.ty, "title": it.r.get("title"), "entity_id": it.entity_id, "action": action,
                                           "approach": approach, "blocks": binding["cells"].as_array().map_or(0, Vec::len), "reordered": reordered }));
            bindings_out.insert(it.name.clone(), binding);
        }
        // results produced earlier and not this time: listed, never deleted (§10.2); a program re-run
        // leaves the written results as they are
        let mut missing = Vec::new();
        if mode == "overwrite" {
            for (name, b) in &prev_bindings {
                if bindings_out.contains_key(name) {
                    continue;
                }
                let keep_direct = cand.get("mode") == Some(&json!("program")) && b["approach"] == json!("direct");
                if keep_direct {
                    bindings_out.insert(name.clone(), b.clone());
                    continue;
                }
                if p.alive(b["entity_id"].as_str().unwrap_or(""))?.is_some() {
                    missing.push(json!({ "name": name, "entity_id": b["entity_id"] }));
                }
            }
        }
        // layout of the new Blocks (§8.5 placement): right of / below the wish, in a two-column grid
        let mut group_rect = Value::Null;
        if let Some(s) = &surface {
            if !cells_to_place.is_empty() {
                let (ax, ay, aw, ah) = anchor;
                let placement_hint = cand.get("placement").and_then(Value::as_str).unwrap_or("right_of_wish");
                let col_w = cells_to_place.iter().map(|(_, _, (w, _))| *w).fold(0.0, f64::max);
                let mut x: f64 = 20.0;
                let mut y: f64 = 48.0;
                let mut row_h: f64 = 0.0;
                let mut col = 0;
                let start_y = if group_exists {
                    let mut max_y: f64 = 0.0;
                    for edge in p.ctx.children(&group_id)? {
                        if let Some(pl) = edge.placement {
                            max_y = max_y.max(pl["y"].as_f64().unwrap_or(0.0) + pl["h"].as_f64().unwrap_or(0.0));
                        }
                    }
                    max_y + 20.0
                } else {
                    48.0
                };
                y = y.max(start_y);
                let mut max_x: f64 = 0.0;
                let mut max_y: f64 = 0.0;
                for (cell_id, cp, (w, h)) in &cells_to_place {
                    if col == 2 {
                        col = 0;
                        x = 20.0;
                        y += row_h + 20.0;
                        row_h = 0.0;
                    }
                    let key = p.next_key(&group_id)?;
                    p.ops.push(json!({ "op": "entity.create", "entity_id": cell_id, "type_id": TYPE_CELL, "parent_id": group_id, "order_key": key,
                                       "placement": { "x": x, "y": y, "w": w, "h": h }, "payload": cp }));
                    max_x = max_x.max(x + w);
                    max_y = max_y.max(y + h);
                    row_h = row_h.max(*h);
                    x += col_w + 20.0;
                    col += 1;
                }
                if !group_exists {
                    // `frame:<id>`: inside that frame (below its title) when it is on this Surface
                    let frame = match placement_hint.strip_prefix("frame:") {
                        Some(f) if p.alive(f)?.is_some() && surface_of(&p.ctx, f)?.as_deref() == Some(s.as_str()) => Some(abs_rect(&p.ctx, f)?),
                        _ => None,
                    };
                    let (gx, gy) = match (frame, placement_hint) {
                        (Some((fx, fy, _, _)), _) => (fx + 20.0, fy + 40.0),
                        (None, "below_wish") => (ax, ay + ah + 60.0),
                        _ => (ax + aw + 60.0, ay),
                    };
                    let offset = if mode == "new" { (seq.saturating_sub(1)) as f64 * 40.0 } else { 0.0 };
                    let rect = json!({ "x": gx + offset, "y": gy + offset, "w": max_x + 20.0, "h": max_y + 20.0 });
                    let key = p.next_key(s)?;
                    p.ops.insert(
                        if folder_exists { 0 } else { 1 },
                        json!({ "op": "entity.create", "entity_id": group_id, "type_id": TYPE_CONTAINER, "parent_id": s, "order_key": key, "placement": rect,
                                "payload": { "kind": "group", "layout": { "mode": "free" }, "title": format!("{title}{}", if simulated { "（模拟）" } else { "" }) } }),
                    );
                    group_rect = rect;
                }
            }
        }
        if !group_exists && cells_to_place.is_empty() && surface.is_some() && p.ops.iter().any(|o| o["parent_id"] == json!(group_id)) {
            p.problems.push("内部错误：结果组缺失".into());
        }
        // ---- the wish: last run, program, refinements, appended inputs — the same commit (§11.4)
        let checks = cand.get("checks").cloned().unwrap_or(json!([]));
        let check_summary = summarize_checks(&checks);
        let last = json!({
            "run_id": req.run_id, "state": "succeeded", "at": cand["executed_at"], "seq": seq, "mode": cand.get("mode").cloned().unwrap_or(json!("generate")),
            "executor": cand["executor"], "simulated": simulated, "group": group_id,
            "config_digest": config, "program_digest": program_digest, "read_set": cand["read_set"], "produced": produced,
            "missing": missing.iter().map(|m| m["entity_id"].clone()).collect::<Vec<_>>(), "checks": check_summary,
            "result_bindings": { "group": group_id, "folder": folder_id, "results": bindings_out },
        });
        wish_keys.push(json!({ "key": "last_run", "value": last, "expect": { "rev": wish.key_rev("last_run") } }));
        p.ops.push(json!({ "op": "entity.set_keys", "entity_id": wish_id, "keys": wish_keys }));
        if p.destructive && !p.confirm_structure {
            p.problems.push("结构变化会删除行或字段：请在预览中确认结构差异后再应用".into());
        }
        let mut plan = Plan { operations: std::mem::take(&mut p.ops), preconditions, assets: std::mem::take(&mut p.assets), ..Default::default() };
        let size = canonical_json(&json!({ "o": plan.operations })).map(|s| s.len()).unwrap_or(0);
        if size > MAX_PLAN_BYTES {
            p.problems.push(format!("应用内容约 {} MiB，超过单次提交上限：请缩小结果规模", size / (1024 * 1024)));
        }
        if plan.operations.len() > 9000 {
            p.problems.push("应用包含的操作过多：请缩小结果规模".into());
        }
        let blocking_checks = false; // failed checks need confirmation, not a block (§8.7)
        let failed_checks: Vec<Value> = checks.as_array().into_iter().flatten().filter(|c| c["status"] == json!("failed")).cloned().collect();
        plan.ready = p.problems.is_empty() && p.manual.is_empty() && !blocking_checks;
        plan.digest = sha256_hex(canonical_json(&json!({ "o": plan.operations, "p": plan.preconditions }))?.as_bytes())[..32].to_string();
        plan.summary = json!({
            "mode": mode, "simulated": simulated,
            "group": { "group_id": group_id, "folder_id": folder_id, "surface_id": surface, "exists": group_exists, "rect": group_rect, "title": title },
            "results": p.results_summary, "missing": missing, "manual": p.manual, "structure": p.structure, "destructive": p.destructive,
            "appended_inputs": cand["appended_inputs"], "program": program_change, "refinements": cand["refinements"],
            "checks": checks, "failed_checks": failed_checks.len(), "problems": p.problems, "operations": plan.operations.len(), "config_digest": config,
        });
        Ok(plan)
    }
}

fn type_for(ty: &str) -> &'static str {
    match ty {
        "richtext" => TYPE_RICHTEXT,
        "record" | "video" => TYPE_RECORD,
        "table" => TYPE_TABLE,
        "image" | "asset" => TYPE_ASSET,
        "html" => TYPE_BLOCK_DEF,
        _ => "",
    }
}

pub fn summarize_checks(checks: &Value) -> Value {
    let mut passed = 0;
    let mut failed = 0;
    let mut not_run = 0;
    let mut review = 0;
    for c in checks.as_array().into_iter().flatten() {
        match c["status"].as_str().unwrap_or("") {
            "passed" => passed += 1,
            "failed" => failed += 1,
            "review" => review += 1,
            _ => not_run += 1,
        }
    }
    json!({ "passed": passed, "failed": failed, "not_run": not_run, "review": review,
            "items": checks.as_array().map(|a| a.iter().map(|c| json!({ "id": c["id"], "kind": c["kind"], "status": c["status"], "text": c["text"] })).collect::<Vec<_>>()).unwrap_or_default() })
}

fn surface_of(ctx: &dyn ReadCtx, id: &str) -> WsResult<Option<String>> {
    let mut cur = id.to_string();
    for _ in 0..32 {
        let Some(edge) = ctx.edge(&cur)? else { return Ok(None) };
        if edge.parent_id == SURFACES_ID {
            return Ok(Some(cur));
        }
        cur = edge.parent_id;
    }
    Ok(None)
}

/// A Block's rectangle in Surface coordinates.
fn abs_rect(ctx: &dyn ReadCtx, id: &str) -> WsResult<(f64, f64, f64, f64)> {
    let edge = ctx.edge(id)?;
    let pl = edge.as_ref().and_then(|e| e.placement.clone()).unwrap_or(json!({}));
    let (mut x, mut y) = (pl["x"].as_f64().unwrap_or(0.0), pl["y"].as_f64().unwrap_or(0.0));
    let (w, h) = (pl["w"].as_f64().unwrap_or(0.0), pl["h"].as_f64().unwrap_or(0.0));
    let mut parent = edge.map(|e| e.parent_id);
    for _ in 0..32 {
        let Some(pid) = parent.clone() else { break };
        let Some(pe) = ctx.edge(&pid)? else { break };
        if pe.parent_id == SURFACES_ID {
            break;
        }
        if let Some(pp) = &pe.placement {
            x += pp["x"].as_f64().unwrap_or(0.0);
            y += pp["y"].as_f64().unwrap_or(0.0);
        }
        parent = Some(pe.parent_id);
    }
    Ok((x, y, w, h))
}

/// View configuration writes field names; Blocks store field ids (§8.5).
fn map_view_config(cfg: &Value, binding: &Value) -> Value {
    let fields = binding.get("fields").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut out = cfg.clone();
    if let Some(o) = out.as_object_mut() {
        for (_, v) in o.iter_mut() {
            if let Some(s) = v.as_str() {
                if let Some(fid) = fields.get(s).and_then(Value::as_str) {
                    *v = json!(fid);
                }
            }
        }
    }
    out
}

// ---- per type

struct Item {
    r: Value,
    name: String,
    ty: String,
    entity_id: String,
    existing: Option<EntityRow>,
    binding: Option<Value>,
    choice: Option<String>,
    new_copy: bool,
}

impl Item {
    fn r(&self) -> &Value {
        &self.r
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn entity_id(&self) -> &str {
        &self.entity_id
    }
    fn existing(&self) -> Option<&EntityRow> {
        self.existing.as_ref()
    }
    fn binding(&self) -> Option<&Value> {
        self.binding.as_ref()
    }
}

fn link_resolver<'b>(p: &'b Planner) -> impl Fn(&str) -> Option<Value> + 'b {
    move |s: &str| {
        let (scheme, target) = s.split_once(':')?;
        match scheme {
            "result" => p.ids.get(target).map(|id| json!({ "entity_id": id })),
            "input" => p.inputs.get(target).map(|id| json!({ "entity_id": id })),
            "entity" if aiworkspace_core::id::is_valid_id(target) => Some(json!({ "entity_id": target })),
            _ => None,
        }
    }
}

fn block_prefix(entity_id: &str) -> String {
    format!("b{}", short_hash(entity_id, 8))
}

fn plan_richtext(p: &mut Planner, it: &Item, parent: &str) -> WsResult<bool> {
    let md = it.r()["markdown"].as_str().unwrap_or("");
    let ast = {
        let resolve = link_resolver(p);
        aiworkspace_core::markdown::to_richtext(md, &block_prefix(it.entity_id()), &resolve)?
    };
    match it.existing() {
        Some(e) => {
            let rt = p.ctx.richtext(&e.entity_id)?.ok_or_else(|| WsError::io("rich text state missing"))?;
            let ops = richtext::diff_blocks(&e.entity_id, &rt.meta.ast, &ast, &rt.meta.block_index)?;
            let changed = !ops.is_empty();
            p.ops.extend(ops);
            Ok(changed)
        }
        None => {
            let key = p.next_key(parent)?;
            p.ops.push(json!({ "op": "entity.create", "entity_id": it.entity_id(), "type_id": TYPE_RICHTEXT, "parent_id": parent, "order_key": key,
                               "name": it.name(), "payload": { "content": ast } }));
            Ok(true)
        }
    }
}

/// `record.properties: [{ name, type, key? }]`, `record.props: { name: value }`.
fn plan_record(p: &mut Planner, it: &Item, parent: &str) -> WsResult<bool> {
    let rec = &it.r()["record"];
    let existing_schema = it.existing().map(|e| aiworkspace_core::types::record_schema(&e.payload).unwrap_or_default()).unwrap_or_default();
    let by_name: BTreeMap<String, String> = existing_schema.values().map(|d| (d.name.clone(), d.field_id.clone())).collect();
    let mut props_schema = Vec::new();
    let mut props = Map::new();
    let mut used = BTreeSet::new();
    for (i, prop) in rec["properties"].as_array().into_iter().flatten().enumerate() {
        let name = prop["name"].as_str().unwrap_or("").to_string();
        let ty = prop["type"].as_str().unwrap_or("text");
        let key = by_name
            .get(&name)
            .cloned()
            .or_else(|| prop["key"].as_str().filter(|k| aiworkspace_core::id::is_valid_id(k)).map(str::to_string))
            .or_else(|| {
                let s: String = name.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                aiworkspace_core::id::is_valid_id(&s).then_some(s)
            })
            .unwrap_or_else(|| format!("k{}", i + 1));
        let key = if used.contains(&key) { format!("{key}{}", i + 1) } else { key };
        used.insert(key.clone());
        let mut def = json!({ "key": key, "name": name, "type": ty });
        let ft = field_type(ty);
        let scale = if ft == FieldType::Decimal { Some(prop["scale"].as_u64().unwrap_or(2) as u8) } else { None };
        if let Some(s) = scale {
            def["scale"] = json!(s);
        }
        let mut options = BTreeMap::new();
        if matches!(ft, FieldType::Select | FieldType::MultiSelect) {
            let labels: Vec<String> = prop["options"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            def["options"] = json!(labels.iter().enumerate().map(|(j, l)| json!({ "option_id": format!("o{}", j + 1), "label": l })).collect::<Vec<_>>());
            options = labels.iter().enumerate().map(|(j, l)| (l.clone(), format!("o{}", j + 1))).collect();
        }
        if let Some(v) = rec["props"].get(&name).and_then(|v| store_value(ft, scale, &options, v)) {
            props.insert(key, v);
        }
        props_schema.push(def);
    }
    let schema = json!({ "properties": props_schema });
    match it.existing() {
        Some(e) => {
            let cur = aiworkspace_core::types::record_nested(&e.payload);
            let mut keys = Vec::new();
            if cur["schema"] != schema {
                keys.push(json!({ "key": "schema", "value": schema, "expect": { "rev": e.key_rev("schema") } }));
                for k in cur["props"].as_object().into_iter().flatten().map(|(k, _)| k.clone()) {
                    if !props.contains_key(&k) {
                        keys.push(json!({ "key": format!("p:{k}"), "value": Value::Null, "expect": { "rev": e.key_rev(&format!("p:{k}")) } }));
                    }
                }
            }
            for (k, v) in &props {
                if cur["props"].get(k) != Some(v) {
                    keys.push(json!({ "key": format!("p:{k}"), "value": v, "expect": { "rev": e.key_rev(&format!("p:{k}")) } }));
                }
            }
            let changed = !keys.is_empty();
            if changed {
                // nulls unset a property
                let (set, unset): (Vec<Value>, Vec<Value>) = keys.into_iter().partition(|k| !k["value"].is_null());
                if !set.is_empty() {
                    p.ops.push(json!({ "op": "entity.set_keys", "entity_id": e.entity_id, "keys": set }));
                }
                if !unset.is_empty() {
                    p.ops.push(json!({ "op": "entity.unset_keys", "entity_id": e.entity_id, "keys": unset.iter().map(|k| json!({ "key": k["key"], "expect": k["expect"] })).collect::<Vec<_>>() }));
                }
            }
            Ok(changed)
        }
        None => {
            let key = p.next_key(parent)?;
            p.ops.push(json!({ "op": "entity.create", "entity_id": it.entity_id(), "type_id": TYPE_RECORD, "parent_id": parent, "order_key": key,
                               "name": it.name(), "payload": { "schema": schema, "props": props } }));
            Ok(true)
        }
    }
}

/// Field definitions with option ids for `table` / `table_columns`: existing ids are kept by name,
/// new ones numbered after them.
struct FieldPlan {
    field_id: String,
    name: String,
    ty: FieldType,
    scale: Option<u8>,
    /// label → option id
    options: BTreeMap<String, String>,
    new_options: Vec<(String, String)>,
    exists: bool,
    /// An existing decimal field too narrow for the values: its `def_rev`, migrated to `scale`.
    widen: Option<u64>,
    def: Value,
}

/// Fraction digits a decimal column needs so that no program value is rounded (at most 6).
fn needed_scale(rows: &[&Map<String, Value>], name: &str) -> u8 {
    let digits = |v: &Value| {
        let s = match v {
            Value::Number(n) => n.as_f64().map(|f| format!("{f}")).unwrap_or_default(),
            Value::String(s) => s.trim().to_string(),
            _ => String::new(),
        };
        s.split_once('.').map_or(0, |(_, f)| f.trim_end_matches('0').len())
    };
    rows.iter().filter_map(|r| r.get(name)).map(digits).max().unwrap_or(0).min(6) as u8
}

fn field_plans(spec: &[Value], rows: &[&Map<String, Value>], existing: &[FieldRow], prefix: &str, taken_ids: &BTreeSet<String>) -> Vec<FieldPlan> {
    let mut ids: BTreeSet<String> = taken_ids.clone();
    let mut n = existing.len() + 1;
    let mut out = Vec::new();
    for f in spec {
        let name = f["name"].as_str().unwrap_or("").to_string();
        let ty = field_type(f["type"].as_str().unwrap_or("text"));
        let cur = existing.iter().find(|x| x.def.name == name && x.def.ty == ty);
        let field_id = match cur {
            Some(c) => c.field_id.clone(),
            None => loop {
                let id = format!("{prefix}{n}");
                n += 1;
                if !ids.contains(&id) {
                    ids.insert(id.clone());
                    break id;
                }
            },
        };
        // decimals are never rounded silently: the declared scale, else what the values need; an
        // existing field keeps its scale unless that is too small
        let (scale, widen) = if ty == FieldType::Decimal {
            let need = f["scale"].as_u64().map_or_else(|| needed_scale(rows, &name), |s| s.min(18) as u8);
            match cur.and_then(|c| c.def.scale.map(|s| (s, c.def_rev))) {
                Some((s, rev)) if s < need => (Some(need), Some(rev)),
                Some((s, _)) => (Some(s), None),
                None => (Some(need), None),
            }
        } else {
            (None, None)
        };
        let mut options: BTreeMap<String, String> = cur.and_then(|c| c.def.options.as_ref()).map(|o| o.iter().map(|x| (x.label.clone(), x.option_id.clone())).collect()).unwrap_or_default();
        let mut new_options = Vec::new();
        if matches!(ty, FieldType::Select | FieldType::MultiSelect) {
            let mut labels: Vec<String> = f["options"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            for r in rows {
                match r.get(&name) {
                    Some(Value::String(s)) if !s.is_empty() => labels.push(s.clone()),
                    Some(Value::Array(a)) => labels.extend(a.iter().filter_map(Value::as_str).map(str::to_string)),
                    _ => {}
                }
            }
            let mut on = options.len() + 1;
            for l in labels {
                if !options.contains_key(&l) {
                    let mut oid = format!("o{on}");
                    while options.values().any(|x| *x == oid) {
                        on += 1;
                        oid = format!("o{on}");
                    }
                    on += 1;
                    options.insert(l.clone(), oid.clone());
                    new_options.push((l, oid));
                }
            }
        }
        let mut def = json!({ "field_id": field_id, "name": name, "type": ty.as_str() });
        if let Some(s) = scale {
            def["scale"] = json!(s);
        }
        if matches!(ty, FieldType::Select | FieldType::MultiSelect) {
            let mut opts: Vec<(&String, &String)> = options.iter().collect();
            opts.sort_by_key(|(_, id)| id[1..].parse::<u64>().unwrap_or(u64::MAX));
            def["options"] = json!(opts.iter().map(|(l, id)| json!({ "option_id": id, "label": l })).collect::<Vec<_>>());
        }
        out.push(FieldPlan { field_id, name, ty, scale, options, new_options, exists: cur.is_some(), widen, def });
    }
    out
}

/// A decimal field that must hold more fraction digits is migrated before its values are written.
fn widen_op(f: &FieldPlan, source_id: &str, changes: &mut Vec<Value>, ops: &mut Vec<Value>) {
    if let (Some(rev), Some(scale)) = (f.widen, f.scale) {
        changes.push(json!({ "kind": "widen_scale", "field": f.name, "to": scale }));
        ops.push(json!({ "op": "table.migrate_field", "source_id": source_id, "field_id": f.field_id, "expect": { "rev": rev }, "to": { "type": "decimal", "scale": scale } }));
    }
}

/// Table results keep the program's row order on their Blocks up to this many rows.
const MAX_ORDERED_ROWS: usize = 1000;

/// A table Block's `manual_order` for rows in this order (fixed-width keys sort as listed).
fn manual_order(ids: &[String]) -> Value {
    let width = format!("{:x}", ids.len()).len();
    Value::Object(ids.iter().enumerate().map(|(i, id)| (id.clone(), json!(format!("{:0>w$x}x", i, w = width)))).collect())
}

fn plan_table(p: &mut Planner, it: &Item, parent: &str, binding: &mut Value, row_order: &mut Option<Vec<String>>) -> WsResult<bool> {
    let t = &it.r()["table"];
    let rows: Vec<&Map<String, Value>> = t["rows"].as_array().into_iter().flatten().filter_map(Value::as_object).collect();
    let key: Vec<String> = t["key"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let spec: Vec<Value> = t["fields"].as_array().cloned().unwrap_or_default();
    let existing_fields: Vec<FieldRow> = match it.existing() {
        Some(e) => live_fields(&p.ctx, &e.entity_id)?,
        None => Vec::new(),
    };
    let all_ids: BTreeSet<String> = match it.existing() {
        Some(e) => p.ctx.fields(&e.entity_id)?.into_iter().map(|f| f.field_id).collect(),
        None => BTreeSet::new(),
    };
    let fps = field_plans(&spec, &rows, &existing_fields, "f", &all_ids);
    // record ids from the logical key (or the row position when the result has no key)
    let mut desired: Vec<(String, Map<String, Value>)> = Vec::new();
    let mut seen = BTreeSet::new();
    for (i, r) in rows.iter().enumerate() {
        let id = if key.is_empty() {
            format!("p{}", i + 1)
        } else {
            record_id_for_key(&key.iter().map(|k| r.get(k).cloned().unwrap_or(Value::Null)).collect::<Vec<_>>())
        };
        if !seen.insert(id.clone()) {
            p.problems.push(format!("结果「{}」的键 {} 有重复值", it.name(), key.join("+")));
            break;
        }
        let mut vals = Map::new();
        for f in &fps {
            if let Some(v) = r.get(&f.name).and_then(|v| store_value(f.ty, f.scale, &f.options, v)) {
                vals.insert(f.field_id.clone(), v);
            }
        }
        desired.push((id, vals));
    }
    binding["key"] = json!(key);
    binding["fields"] = json!(fps.iter().map(|f| (f.name.clone(), json!(f.field_id))).collect::<Map<String, Value>>());
    *row_order = Some(desired.iter().map(|(id, _)| id.clone()).collect());
    let sid = it.entity_id().to_string();
    match it.existing() {
        None => {
            let key_field = fps.iter().find(|f| key.first() == Some(&f.name) && f.ty == FieldType::Text).or_else(|| fps.iter().find(|f| f.ty == FieldType::Text));
            let mut payload = json!({ "fields": fps.iter().map(|f| f.def.clone()).collect::<Vec<_>>() });
            if let Some(kf) = key_field {
                payload["title_field_id"] = json!(kf.field_id);
            }
            let k = p.next_key(parent)?;
            p.ops.push(json!({ "op": "entity.create", "entity_id": sid, "type_id": TYPE_TABLE, "parent_id": parent, "order_key": k, "name": it.name(), "payload": payload }));
            for chunk in desired.chunks(CHUNK) {
                p.ops.push(json!({ "op": "table.insert_records", "source_id": sid, "records": chunk.iter().map(|(id, v)| json!({ "record_id": id, "values": v })).collect::<Vec<_>>() }));
            }
            Ok(true)
        }
        Some(e) => {
            let mut changes = Vec::new();
            let mut ops = Vec::new();
            let prev_fields: BTreeMap<String, String> = it
                .binding()
                .and_then(|b| b["fields"].as_object())
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                .unwrap_or_default();
            // fields: the wish's own fields that are gone or changed type are removed (destructive);
            // fields a person added to the result table are left alone
            for f in &existing_fields {
                let produced_by_wish = prev_fields.values().any(|id| *id == f.field_id);
                let still = fps.iter().any(|x| x.field_id == f.field_id);
                if produced_by_wish && !still {
                    changes.push(json!({ "kind": "delete_field", "field": f.def.name, "destructive": true }));
                    ops.push(json!({ "op": "table.delete_field", "source_id": sid, "field_id": f.field_id, "expect": { "rev": f.def_rev } }));
                    p.destructive = true;
                }
            }
            for f in &fps {
                if !f.exists {
                    if existing_fields.iter().any(|x| x.def.name == f.name) {
                        let old = existing_fields.iter().find(|x| x.def.name == f.name).unwrap();
                        if prev_fields.values().any(|id| *id == old.field_id) {
                            changes.push(json!({ "kind": "change_type", "field": f.name, "from": old.def.ty.as_str(), "to": f.ty.as_str(), "destructive": true }));
                            ops.push(json!({ "op": "table.delete_field", "source_id": sid, "field_id": old.field_id, "expect": { "rev": old.def_rev } }));
                            p.destructive = true;
                        } else {
                            p.problems.push(format!("结果「{}」的字段「{}」与人工添加的同名字段冲突", it.name(), f.name));
                            continue;
                        }
                    } else {
                        changes.push(json!({ "kind": "add_field", "field": f.name, "type": f.ty.as_str() }));
                    }
                    ops.push(json!({ "op": "table.add_field", "source_id": sid, "field": f.def }));
                } else {
                    widen_op(f, &sid, &mut changes, &mut ops);
                    for (label, oid) in &f.new_options {
                        ops.push(json!({ "op": "table.add_option", "source_id": sid, "field_id": f.field_id, "option": { "option_id": oid, "label": label } }));
                    }
                }
            }
            if key != it.binding().and_then(|b| b["key"].as_array()).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default()
                && it.binding().is_some()
            {
                changes.push(json!({ "kind": "key_changed", "key": key }));
            }
            // rows: by record id
            let mut current: BTreeMap<String, RecordRow> = BTreeMap::new();
            p.ctx.scan_records(&sid, &mut |r| {
                if r.alive() {
                    current.insert(r.record_id.clone(), r.clone());
                }
                Ok(true)
            })?;
            let desired_ids: BTreeSet<&String> = desired.iter().map(|(id, _)| id).collect();
            let mut deletes = Vec::new();
            for (id, r) in &current {
                if desired_ids.contains(id) {
                    continue;
                }
                // rows the wish wrote (program provenance) go; rows a person added stay
                let by_program = r.meta.values().any(|m| m.get("derived").is_some()) || id.starts_with('k') || id.starts_with('p');
                if by_program {
                    deletes.push(json!({ "record_id": id, "expect": { "rev": r.rev } }));
                }
            }
            if !deletes.is_empty() {
                changes.push(json!({ "kind": "delete_rows", "count": deletes.len(), "destructive": true }));
                p.destructive = true;
                for chunk in deletes.chunks(CHUNK) {
                    ops.push(json!({ "op": "table.delete_records", "source_id": sid, "records": chunk }));
                }
            }
            let mut inserts = Vec::new();
            let mut sets = Vec::new();
            let mut unsets = Vec::new();
            for (id, vals) in &desired {
                match current.get(id) {
                    None => inserts.push(json!({ "record_id": id, "values": vals })),
                    Some(r) => {
                        for f in &fps {
                            let want = vals.get(&f.field_id);
                            let have = r.values.get(&f.field_id);
                            if want == have {
                                continue;
                            }
                            match want {
                                Some(v) => sets.push(json!({ "record_id": id, "field_id": f.field_id, "value": v, "expect": { "rev": r.value_rev(&f.field_id) } })),
                                None => unsets.push(json!({ "record_id": id, "field_id": f.field_id, "expect": { "rev": r.value_rev(&f.field_id) } })),
                            }
                        }
                    }
                }
            }
            if !inserts.is_empty() {
                changes.push(json!({ "kind": "insert_rows", "count": inserts.len() }));
                for chunk in inserts.chunks(CHUNK) {
                    ops.push(json!({ "op": "table.insert_records", "source_id": sid, "records": chunk }));
                }
            }
            if !sets.is_empty() {
                changes.push(json!({ "kind": "update_values", "count": sets.len() }));
                for chunk in sets.chunks(CHUNK * 4) {
                    ops.push(json!({ "op": "table.set_values", "source_id": sid, "values": chunk }));
                }
            }
            if !unsets.is_empty() {
                changes.push(json!({ "kind": "clear_values", "count": unsets.len() }));
                for chunk in unsets.chunks(CHUNK * 4) {
                    ops.push(json!({ "op": "table.unset_values", "source_id": sid, "values": chunk }));
                }
            }
            let changed = !ops.is_empty();
            if !changes.is_empty() {
                p.structure.push(json!({ "name": it.name(), "entity_id": sid, "changes": changes, "rows_before": current.len(), "rows_after": desired.len() }));
            }
            let _ = e;
            p.ops.extend(ops);
            Ok(changed)
        }
    }
}

/// Derived columns on an input table (§10.4): only the wish's own fields are written.
fn plan_columns(p: &mut Planner, it: &Item, wish_id: &str, run_id: &str, choice: Option<&str>, binding: &mut Value) -> WsResult<bool> {
    let c = &it.r()["columns"];
    let tid = c["target_id"].as_str().unwrap_or("").to_string();
    let Some(table) = p.alive(&tid)?.filter(|t| t.type_id == TYPE_TABLE) else {
        p.problems.push(format!("派生列「{}」的目标表不存在", it.name()));
        return Ok(false);
    };
    let fields = live_fields(&p.ctx, &tid)?;
    let all_ids: BTreeSet<String> = p.ctx.fields(&tid)?.into_iter().map(|f| f.field_id).collect();
    let owned: BTreeMap<String, String> = it
        .binding()
        .and_then(|b| b["fields"].as_object())
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    let spec: Vec<Value> = c["fields"].as_array().cloned().unwrap_or_default();
    let values: Map<String, Value> = c["values"].as_object().cloned().unwrap_or_default();
    let rows: Vec<&Map<String, Value>> = values.values().filter_map(Value::as_object).collect();
    // fields: the wish's own by id; a new one must not collide with a person's field name
    let own_rows: Vec<FieldRow> = fields.iter().filter(|f| owned.values().any(|id| *id == f.field_id)).cloned().collect();
    let prefix = format!("w{}", short_hash(&format!("{wish_id}/{}", it.name()), 6));
    let mut fps = field_plans(&spec, &rows, &own_rows, &format!("{prefix}-"), &all_ids);
    let mut ops = Vec::new();
    let mut changes = Vec::new();
    for f in &mut fps {
        if owned.get(&f.name).is_some_and(|id| !fields.iter().any(|x| x.field_id == *id)) {
            // the person deleted the wish's column: it is recreated only when they asked for it
            if choice.is_none() {
                p.manual.push(json!({ "name": it.name(), "entity_id": tid, "field_deleted": f.name }));
                continue;
            }
            changes.push(json!({ "kind": "recreate_field", "field": f.name }));
        }
        if !f.exists {
            if fields.iter().any(|x| x.def.name == f.name) {
                p.problems.push(format!("派生列「{}」与表中已有字段同名：请换一个列名", f.name));
                continue;
            }
            f.def["maintained_by"] = json!("program");
            changes.push(json!({ "kind": "add_field", "field": f.name, "type": f.ty.as_str() }));
            ops.push(json!({ "op": "table.add_field", "source_id": tid, "field": f.def }));
        } else {
            widen_op(&f, &tid, &mut changes, &mut ops);
            for (label, oid) in &f.new_options {
                ops.push(json!({ "op": "table.add_option", "source_id": tid, "field_id": f.field_id, "option": { "option_id": oid, "label": label } }));
            }
        }
    }
    // the wish's own fields that are not produced any more are removed (destructive)
    for (name, id) in &owned {
        if !fps.iter().any(|f| f.field_id == *id) {
            if let Some(f) = fields.iter().find(|x| x.field_id == *id) {
                changes.push(json!({ "kind": "delete_field", "field": name, "destructive": true }));
                ops.push(json!({ "op": "table.delete_field", "source_id": tid, "field_id": id, "expect": { "rev": f.def_rev } }));
                p.destructive = true;
            }
        }
    }
    // values; cells changed by hand need the user's decision (§11.3)
    let mut sets = Vec::new();
    let mut manual_cells = 0;
    for (rid, vals) in &values {
        let Some(r) = p.ctx.record(&tid, rid)?.filter(|r| r.alive()) else { continue };
        let Some(vals) = vals.as_object() else { continue };
        for f in &fps {
            let want = vals.get(&f.name).and_then(|v| store_value(f.ty, f.scale, &f.options, v));
            let have = r.values.get(&f.field_id);
            if want.as_ref() == have {
                continue;
            }
            if r.meta.get(&f.field_id).is_some_and(|m| m["manual_override"] == json!(true)) {
                manual_cells += 1;
                if choice != Some("replace") {
                    continue;
                }
            }
            if let Some(v) = want {
                sets.push(json!({ "record_id": rid, "field_id": f.field_id, "value": v, "expect": { "rev": r.value_rev(&f.field_id) } }));
            }
        }
    }
    if manual_cells > 0 && choice.is_none() {
        p.manual.push(json!({ "name": it.name(), "entity_id": tid, "cells": manual_cells }));
    }
    if !sets.is_empty() {
        changes.push(json!({ "kind": "update_values", "count": sets.len() }));
        for chunk in sets.chunks(CHUNK * 4) {
            ops.push(json!({ "op": "table.set_values", "source_id": tid, "values": chunk, "derived": { "wish_id": wish_id, "run_id": run_id, "result_key": it.name() } }));
        }
    }
    if !changes.is_empty() {
        p.structure.push(json!({ "name": it.name(), "entity_id": tid, "changes": changes, "kept_manual_cells": if choice == Some("replace") { 0 } else { manual_cells } }));
    }
    binding["entity_id"] = json!(tid);
    binding["target"] = c["target"].clone();
    binding["fields"] = json!(fps.iter().map(|f| (f.name.clone(), json!(f.field_id))).collect::<Map<String, Value>>());
    let changed = !ops.is_empty();
    p.ops.extend(ops);
    let _ = table;
    Ok(changed)
}

fn plan_asset(p: &mut Planner, it: &Item, parent: &str) -> WsResult<bool> {
    let f = &it.r()["file"];
    let obj = f["object_id"].as_str().ok_or_else(|| WsError::invalid_op(format!("result {} has no staged file", it.name())))?.to_string();
    if let Some(path) = f["path"].as_str() {
        p.assets.push((obj.clone(), path.to_string()));
    }
    let image = f.get("image").filter(|i| i["width"].as_u64().is_some() && i["height"].as_u64().is_some()).cloned();
    match it.existing() {
        Some(e) => {
            if e.payload.get("object_id").and_then(Value::as_str) == Some(obj.as_str()) {
                return Ok(false);
            }
            let mut keys = vec![json!({ "key": "object_id", "value": obj, "expect": { "rev": e.key_rev("object_id") } })];
            if let Some(i) = image {
                keys.push(json!({ "key": "image", "value": i, "expect": { "rev": e.key_rev("image") } }));
            }
            p.ops.push(json!({ "op": "entity.set_keys", "entity_id": e.entity_id, "keys": keys }));
            Ok(true)
        }
        None => {
            let key = p.next_key(parent)?;
            let mut payload = json!({ "object_id": obj, "file_name": f.get("file_name").cloned().unwrap_or(json!(it.name())) });
            if let Some(i) = image {
                payload["image"] = i;
            }
            p.ops.push(json!({ "op": "entity.create", "entity_id": it.entity_id(), "type_id": TYPE_ASSET, "parent_id": parent, "order_key": key, "name": it.name(), "payload": payload }));
            Ok(true)
        }
    }
}

/// An HTML result: a Block definition (kind html, `aiws` v2) plus Blocks bound by name (§8.5, §9.4).
fn plan_html(p: &mut Planner, it: &Item, parent: &str, wish_id: &str, binding: &mut Value) -> WsResult<bool> {
    let h = &it.r()["html"];
    let def_id = format!("wish.{}", short_hash(&format!("{wish_id}/{}", it.name()), 12));
    let mut bindings = Map::new();
    for (name, target) in h["bindings"].as_object().into_iter().flatten() {
        let t = target.as_str().unwrap_or("");
        let id = match t.split_once(':') {
            Some(("result", n)) => p.ids.get(n).cloned(),
            Some(("input", n)) => p.inputs.get(n).cloned(),
            _ => None,
        };
        match id {
            Some(id) => {
                bindings.insert(name.clone(), json!({ "entity_id": id }));
            }
            None => p.problems.push(format!("HTML 结果「{}」的绑定 {name} → {t} 无法解析", it.name())),
        }
    }
    binding["def_id"] = json!(def_id);
    binding["bindings"] = Value::Object(bindings);
    let body = json!({ "html": h["html"], "css": h.get("css").cloned().unwrap_or(json!("")), "js": h.get("js").cloned().unwrap_or(json!("")), "api_version": aiworkspace_core::types::HTML_API_VERSION });
    match it.existing() {
        Some(e) => {
            if e.payload.get("html") == Some(&body) {
                return Ok(false);
            }
            let version = e.payload.get("version").and_then(Value::as_u64).unwrap_or(1) + 1;
            p.ops.push(json!({ "op": "entity.set_keys", "entity_id": e.entity_id, "keys": [
                { "key": "html", "value": body, "expect": { "rev": e.key_rev("html") } },
                { "key": "version", "value": version, "expect": { "rev": e.key_rev("version") } }] }));
            Ok(true)
        }
        None => {
            let key = p.next_key(parent)?;
            p.ops.push(json!({ "op": "entity.create", "entity_id": it.entity_id(), "type_id": TYPE_BLOCK_DEF, "parent_id": parent, "order_key": key, "name": it.name(),
                               "payload": { "def_id": def_id, "version": 1, "kind": "html", "title": it.r().get("title").cloned().unwrap_or(json!(it.name())),
                                            "allow_no_source": true, "html": body, "description": h.get("description").cloned().unwrap_or(json!("由许愿格生成的 HTML Block")) } }));
            Ok(true)
        }
    }
}

/// Is `entity_id` referenced as the source of any Block (helper for callers).
pub fn bound_source(cell: &EntityRow) -> Option<&str> {
    cell.payload.get("source_ref").and_then(reference_entity_id)
}

/// Wish keys an analysis write-back is guarded by (§7.3): a newer prompt never gets an old analysis.
pub const ANALYSIS_GUARDED_KEYS: &[&str] = &["prompt", "knowledge", "refinements", "executor", "executor_config", "inputs", "analysis"];

impl Workspace {
    /// The write-back of a validated analysis: `analysis` + normalized `inputs` in one Commit.
    pub fn wish_analysis_plan(&self, caller: &Caller, candidate: &Value) -> WsResult<Plan> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let wish_id = candidate["wish_id"].as_str().ok_or_else(|| WsError::invalid_op("candidate.wish_id missing"))?;
        let wish = aiworkspace_core::read::readable_entity(&ctx, &access, wish_id)?;
        let basis = candidate.get("basis").and_then(|b| b.get("key_revs")).cloned().unwrap_or(json!({}));
        let mut problems = Vec::new();
        let mut preconditions = Vec::new();
        for k in ANALYSIS_GUARDED_KEYS {
            let at_start = basis.get(*k).and_then(Value::as_u64).unwrap_or(0);
            preconditions.push(json!({ "target": { "entity_id": wish_id, "selector": { "kind": "doc_key", "key": k } }, "expect": { "rev": at_start } }));
            if wish.key_rev(k) != at_start {
                problems.push(format!("分析期间许愿格的 {k} 已被修改：这份分析只保留在运行记录中，请重新分析"));
            }
        }
        let mut analysis = candidate["analysis"].clone();
        analysis["at"] = candidate.get("executed_at").cloned().unwrap_or(json!(now));
        let operations = vec![json!({ "op": "entity.set_keys", "entity_id": wish_id, "keys": [
            { "key": "analysis", "value": analysis, "expect": { "rev": wish.key_rev("analysis") } },
            { "key": "inputs", "value": candidate["inputs"], "expect": { "rev": wish.key_rev("inputs") } }] })];
        let digest = sha256_hex(canonical_json(&json!({ "o": operations, "p": preconditions }))?.as_bytes())[..32].to_string();
        Ok(Plan {
            summary: json!({ "kind": "analysis", "status": candidate["analysis"]["status"], "inputs": candidate["inputs"], "problems": problems }),
            ready: problems.is_empty(),
            operations,
            preconditions,
            assets: Vec::new(),
            digest,
        })
    }
}
