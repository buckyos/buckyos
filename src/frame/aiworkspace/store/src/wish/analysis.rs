//! Host validation of an analysis (许愿格 §7.3): the model's `wish.analysis.v2` speaks in handles;
//! the host binds them to real data and ranges, rejects what cannot be bound, rewrites the context
//! prompt so it no longer depends on run-local handles, and checks the output contract against the
//! Renderer catalog and the production rules. Every problem is returned to the model in the same
//! run, so it can fix its answer.

use super::context::{translate_fields, translate_filter, translate_sorts, Stage};
use super::handles::Target;
use aiworkspace_core::filter::live_fields;
use aiworkspace_core::model::*;
use aiworkspace_core::value::reference_entity_id;
use aiworkspace_core::wish::{check_analysis, check_contract_result, is_input_name, ANALYSIS_SCHEMA};
use aiworkspace_core::WsResult;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Entities visited at most when looking for a generation cycle.
const CYCLE_BUDGET: usize = 400;

#[derive(Debug)]
pub struct Validated {
    pub analysis: Value,
    pub inputs: Vec<Value>,
}

/// Does `start` (transitively, through generation records and wish inputs) depend on a result of `wish_id`?
/// `Err(())` when the walk ran out of budget (cannot be confirmed: treated as a problem).
pub fn generation_cycle(ctx: &dyn ReadCtx, wish_id: &str, start: &str) -> WsResult<Result<bool, ()>> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([start.to_string()]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if seen.len() > CYCLE_BUDGET {
            return Ok(Err(()));
        }
        let Some(e) = ctx.entity(&id)? else { continue };
        if let Some(d) = &e.derived {
            if d.get("wish_id").and_then(Value::as_str) == Some(wish_id) {
                return Ok(Ok(true));
            }
            // a result depends on its own inputs and on the wish that produced it
            for i in d.get("inputs").and_then(Value::as_array).into_iter().flatten() {
                if let Some(x) = i["entity_id"].as_str() {
                    queue.push_back(x.to_string());
                }
            }
            if let Some(w) = d.get("wish_id").and_then(Value::as_str) {
                if w == wish_id {
                    return Ok(Ok(true));
                }
                queue.push_back(w.to_string());
            }
        }
        if e.type_id == TYPE_WISH {
            for i in e.payload.get("inputs").and_then(Value::as_array).into_iter().flatten() {
                if let Some(x) = i["entity_id"].as_str() {
                    queue.push_back(x.to_string());
                }
            }
        }
        if e.type_id == TYPE_CONTAINER {
            for edge in ctx.children(&id)? {
                queue.push_back(edge.child_id);
            }
        }
    }
    Ok(Ok(false))
}

fn renderer_accepts(catalog: &Value, renderer: &str, ty: &str) -> Option<bool> {
    let data_type = match ty {
        "table" => TYPE_TABLE,
        "richtext" => TYPE_RICHTEXT,
        "record" | "video" => TYPE_RECORD,
        "image" | "asset" => TYPE_ASSET,
        _ => return Some(true),
    };
    let r = catalog.as_array()?.iter().find(|r| r["renderer"].as_str() == Some(renderer))?;
    Some(r["accepts"].as_array().is_some_and(|a| a.iter().any(|t| t.as_str() == Some(data_type))))
}

impl Stage {
    /// Validate the model's analysis. `Err(problems)` lists everything to fix.
    pub fn validate_analysis(&mut self, submitted: &Value) -> WsResult<Result<Validated, Vec<String>>> {
        let mut problems: Vec<String> = Vec::new();
        let Some(o) = submitted.as_object() else { return Ok(Err(vec!["analysis 必须是一个 JSON 对象".into()])) };
        let known = ["schema_version", "status", "context_prompt", "inputs", "output_contract", "checks", "blockers", "warnings", "context_sources", "summary"];
        for k in o.keys() {
            if !known.contains(&k.as_str()) {
                problems.push(format!("未知字段 {k}（允许：{}）", known.join(", ")));
            }
        }
        if o.get("schema_version").and_then(Value::as_str).is_some_and(|v| v != ANALYSIS_SCHEMA) {
            problems.push(format!("schema_version 必须是 {ANALYSIS_SCHEMA}"));
        }
        let mut status = o.get("status").and_then(Value::as_str).unwrap_or("ready").to_string();
        if !matches!(status.as_str(), "ready" | "needs_input") {
            problems.push("status 必须是 ready 或 needs_input".into());
        }
        let wish_id = self.wish.entity_id.clone();
        // ---- inputs
        let mut inputs: Vec<Value> = Vec::new();
        let mut names: BTreeMap<String, String> = BTreeMap::new(); // name → entity id
        let mut handle_names: BTreeMap<String, String> = BTreeMap::new(); // handle → name
        let mut table_inputs: BTreeSet<String> = BTreeSet::new();
        for (i, input) in o.get("inputs").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let at = format!("inputs[{i}]");
            let name = input["name"].as_str().unwrap_or("").to_string();
            if !is_input_name(&name) {
                problems.push(format!("{at}.name「{name}」必须是标识符（字母、数字、下划线，不以数字开头），程序用它读取输入"));
                continue;
            }
            if names.contains_key(&name) {
                problems.push(format!("{at}.name「{name}」重复"));
                continue;
            }
            let r = input["ref"].as_str().or_else(|| input["entity_id"].as_str()).unwrap_or("");
            let target = match self.handles.resolve(r) {
                Ok(t) => t,
                Err(e) => {
                    problems.push(format!("{at}.ref {r}: {}", e.detail));
                    continue;
                }
            };
            let (entity_id, mut fields_hint) = match &target {
                Target::Entity(e) => (e.clone(), None),
                Target::Field(e, f) => (e.clone(), Some(vec![f.clone()])),
                Target::Record(e, _) => (e.clone(), None),
            };
            let e = match self.snap.readable(&entity_id) {
                Ok(e) => e,
                Err(err) => {
                    problems.push(format!("{at}.ref {r} 不可读或不存在（{}）", err.detail));
                    continue;
                }
            };
            let mut selector = input.get("selector").cloned().filter(|s| !s.is_null());
            let mut data = e.clone();
            if e.type_id == TYPE_CELL {
                match e.payload.get("source_ref").and_then(reference_entity_id) {
                    None => {
                        problems.push(format!("{at}.ref {r} 是纯 UI Block（框/形状），没有数据可绑定"));
                        continue;
                    }
                    Some(src) => {
                        data = match self.snap.readable(src) {
                            Ok(d) => d,
                            Err(_) => {
                                problems.push(format!("{at}.ref {r} 显示的数据不可读"));
                                continue;
                            }
                        };
                        let is_view = e.payload.get("view").and_then(|v| v["type"].as_str()) == Some("table") && data.type_id == TYPE_TABLE;
                        let explicit = selector.as_ref().and_then(|s| s["kind"].as_str()).map(str::to_string);
                        if is_view && explicit.as_deref().map_or(true, |k| k == "table_view") {
                            // the table the user sees is the view: its filter and order are the range (§5.2)
                            selector = Some(json!({ "kind": "table_view", "cell_id": e.entity_id }));
                        } else if explicit.as_deref() == Some("table_view") {
                            problems.push(format!("{at}: {r} 不是表格视图，不能用 table_view"));
                            continue;
                        }
                    }
                }
            }
            match data.type_id.as_str() {
                TYPE_WISH => {
                    problems.push(format!("{at}.ref {r} 是许愿格本身：请绑定它产生的结果"));
                    continue;
                }
                TYPE_BLOCK_DEF | TYPE_CELL => {
                    problems.push(format!("{at}.ref {r} 不是数据"));
                    continue;
                }
                _ => {}
            }
            if data.entity_id == wish_id {
                problems.push(format!("{at}: 许愿格不能以自己为输入"));
                continue;
            }
            match generation_cycle(&self.ctx(), &wish_id, &data.entity_id)? {
                Ok(true) => {
                    problems.push(format!("{at}.ref {r}（{}）直接或间接来自本许愿格的结果：会形成生成环", data.payload.get("title").and_then(Value::as_str).or(data.name.as_deref()).unwrap_or(&data.entity_id)));
                    continue;
                }
                Err(()) => {
                    problems.push(format!("{at}.ref {r} 的生成依赖链过长，无法确认是否成环：请改绑更直接的数据"));
                    continue;
                }
                Ok(false) => {}
            }
            if input.get("version").and_then(|v| v["mode"].as_str()) == Some("fixed") {
                problems.push(format!("{at}: 首版不支持固定历史版本（version.mode = fixed），请使用 follow"));
                continue;
            }
            // selector: by names → by field ids
            if data.type_id == TYPE_TABLE {
                table_inputs.insert(name.clone());
                let fields = live_fields(&self.ctx(), &data.entity_id)?;
                match selector.as_ref().and_then(|s| s["kind"].as_str()) {
                    None | Some("entity") => {
                        if let Some(fh) = fields_hint.take() {
                            selector = Some(json!({ "kind": "table_query", "fields": fh }));
                        } else {
                            selector = None;
                        }
                    }
                    Some("table_view") => {}
                    Some("table_query") => {
                        let s = selector.clone().unwrap_or(json!({}));
                        let mut out = json!({ "kind": "table_query" });
                        if let Some(f) = s.get("filter").filter(|f| !f.is_null()) {
                            match translate_filter(f, &fields, &self.handles, &data.entity_id) {
                                Ok(v) => out["filter"] = v,
                                Err(err) => problems.push(format!("{at}.selector.filter: {}", err.detail)),
                            }
                        }
                        if let Some(f) = s.get("sorts").filter(|f| !f.is_null()) {
                            match translate_sorts(f, &fields, &self.handles, &data.entity_id) {
                                Ok(v) => out["sorts"] = json!(v),
                                Err(err) => problems.push(format!("{at}.selector.sorts: {}", err.detail)),
                            }
                        }
                        if let Some(f) = s.get("fields").filter(|f| !f.is_null()) {
                            match translate_fields(f, &fields, &self.handles, &data.entity_id) {
                                Ok(v) => out["fields"] = json!(v),
                                Err(err) => problems.push(format!("{at}.selector.fields: {}", err.detail)),
                            }
                        }
                        selector = Some(out);
                    }
                    Some(k) => {
                        problems.push(format!("{at}.selector.kind {k} 不支持（entity / table_view / table_query）"));
                        continue;
                    }
                }
            } else if selector.as_ref().and_then(|s| s["kind"].as_str()).is_some_and(|k| k != "entity") {
                problems.push(format!("{at}: {} 只能整体绑定（selector.kind = entity）", super::context::type_label(&data.type_id)));
                continue;
            } else {
                selector = None;
            }
            if data.type_id == TYPE_TABLE && aiworkspace_core::types::is_url_table(&data) {
                problems.push(format!("{at}: URL 查询表没有可验证的版本，首版不能作为许愿格输入"));
                continue;
            }
            let label = input["label"].as_str().map(str::to_string).unwrap_or_else(|| data.payload.get("title").and_then(Value::as_str).or(data.name.as_deref()).unwrap_or(&data.entity_id).to_string());
            let mut item = json!({ "entity_id": data.entity_id, "name": name, "label": label, "version": { "mode": "follow" } });
            if let Some(s) = selector {
                item["selector"] = s;
            }
            names.insert(name.clone(), data.entity_id.clone());
            handle_names.insert(r.to_string(), name.clone());
            if let Some(h) = self.handles.get(&data.entity_id) {
                handle_names.insert(h.to_string(), name.clone());
            }
            inputs.push(item);
        }
        // ---- context prompt: persisted without run-local handles
        let raw_prompt = o.get("context_prompt").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if raw_prompt.is_empty() {
            problems.push("context_prompt 不能为空：写出可以独立执行的任务说明".into());
        }
        let field_names: BTreeMap<(String, String), String> = {
            let mut m = BTreeMap::new();
            for id in names.values() {
                if let Some(t) = self.snap.entity(id)?.filter(|t| t.type_id == TYPE_TABLE) {
                    for f in live_fields(&self.ctx(), &t.entity_id)? {
                        m.insert((t.entity_id.clone(), f.field_id.clone()), f.def.name.clone());
                    }
                }
            }
            m
        };
        let unbound_cell = std::cell::RefCell::new(Vec::<String>::new());
        let (context_prompt, unknown) = self.handles.rewrite(&raw_prompt, &|h, t| {
            let out = match t {
                Target::Entity(_) => handle_names.get(h).map(|n| format!("输入 `{n}`")),
                Target::Field(e, f) => {
                    let owner = names.iter().find(|(_, id)| *id == e).map(|(n, _)| n.clone());
                    match (owner, field_names.get(&(e.clone(), f.clone()))) {
                        (Some(n), Some(fname)) => Some(format!("`{n}` 的字段「{fname}」")),
                        _ => None,
                    }
                }
                Target::Record(_, _) => None,
            };
            if out.is_none() && !unbound_cell.borrow().contains(&h.to_string()) {
                unbound_cell.borrow_mut().push(h.to_string());
            }
            out
        });
        let unbound = unbound_cell.into_inner();
        if !unknown.is_empty() {
            problems.push(format!("context_prompt 中的句柄 {} 不存在", unknown.join("、")));
        }
        if !unbound.is_empty() {
            problems.push(format!("context_prompt 提到了 {}，但它们没有绑定为输入：句柄只在本次运行有效，请绑定为输入并用输入名指代", unbound.join("、")));
        }
        // ---- output contract
        let contract_in = o.get("output_contract").cloned().unwrap_or(json!({ "results": [] }));
        let mut results = Vec::new();
        for (i, r) in contract_in.get("results").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let at = format!("output_contract.results[{i}]");
            let mut r = r.clone();
            if let Some(m) = r.as_object_mut() {
                m.retain(|k, _| matches!(k.as_str(), "name" | "type" | "title" | "approach" | "key" | "target" | "views" | "description" | "fields"));
            }
            if let Err(e) = check_contract_result(&r) {
                problems.push(format!("{at}: {}", e.detail));
                continue;
            }
            let ty = r["type"].as_str().unwrap_or("");
            if ty == "table" && !r.get("key").and_then(Value::as_array).is_some_and(|k| !k.is_empty()) {
                problems.push(format!("{at}: 表格结果需要 key（一列或多列的逻辑键），重跑时据此保持记录身份"));
            }
            if ty == "table_columns" && !table_inputs.contains(r["target"].as_str().unwrap_or("")) {
                problems.push(format!("{at}: table_columns 的 target 必须是一个表格输入的输入名"));
            }
            if ty == "video" {
                problems.push(format!("{at}: 当前没有真实的视频生成工具，不能约定 video 结果"));
            }
            for (vi, v) in r.get("views").and_then(Value::as_array).into_iter().flatten().enumerate() {
                let renderer = v["renderer"].as_str().unwrap_or("");
                if ty == "html" {
                    continue;
                }
                match renderer_accepts(&self.catalog_json()?, renderer, ty) {
                    None => problems.push(format!("{at}.views[{vi}]: Renderer `{renderer}` 不在目录中（见 catalog/renderers.json）")),
                    Some(false) => problems.push(format!("{at}.views[{vi}]: Renderer `{renderer}` 不能显示 {ty} 结果")),
                    Some(true) => {}
                }
            }
            results.push(r);
        }
        let mut contract = Map::new();
        contract.insert("results".into(), json!(results));
        if let Some(pl) = contract_in.get("placement").and_then(Value::as_str) {
            // `frame:@B7` names a frame Block: persisted as its id
            let pl = match pl.strip_prefix("frame:") {
                Some(h) => match self.handles.resolve_entity(h) {
                    Ok(id) => format!("frame:{id}"),
                    Err(_) => {
                        problems.push(format!("output_contract.placement 的框 {h} 不存在"));
                        pl.to_string()
                    }
                },
                None => pl.to_string(),
            };
            contract.insert("placement".into(), json!(pl));
        }
        if contract_in.get("dynamic") == Some(&json!(true)) {
            contract.insert("dynamic".into(), json!(true));
        }
        // ---- blockers
        let mut blockers = Vec::new();
        for b in o.get("blockers").and_then(Value::as_array).into_iter().flatten() {
            let mut c = Vec::new();
            for h in b["candidates"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                match self.handles.resolve_entity(h).and_then(|id| self.snap.readable(&id)) {
                    Ok(e) => c.push(json!(e.entity_id)),
                    Err(_) => problems.push(format!("blocker 候选 {h} 不存在或不可读")),
                }
            }
            let mut x = json!({ "code": b["code"].as_str().unwrap_or("NEEDS_INPUT"), "message": b["message"].as_str().unwrap_or("") });
            if let Some(l) = b["input_label"].as_str() {
                x["input_label"] = json!(l);
            }
            if !c.is_empty() {
                x["candidates"] = json!(c);
            }
            blockers.push(x);
        }
        if !blockers.is_empty() {
            status = "needs_input".into();
        }
        if status == "ready" && inputs.is_empty() && raw_prompt.contains('@') {
            problems.push("context_prompt 提到了数据，但 inputs 为空".into());
        }
        // ---- semantic basis: the text whose rules or values went into the prompt (§8.1)
        let mut reads = super::readset::ReadSet::default();
        for h in o.get("context_sources").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            match self.handles.resolve_entity(h).and_then(|id| self.snap.readable(&id)) {
                Ok(e) => reads.entity(&self.ctx(), &e)?,
                Err(_) => problems.push(format!("context_sources 中的 {h} 不存在")),
            }
        }
        let analysis = json!({
            "schema_version": ANALYSIS_SCHEMA,
            "status": status,
            "prompt": self.wish.payload.get("prompt").cloned().unwrap_or(json!("")),
            "context_prompt": context_prompt,
            "output_contract": contract,
            "checks": o.get("checks").cloned().unwrap_or(json!([])),
            "blockers": blockers,
            "warnings": o.get("warnings").cloned().unwrap_or(json!([])),
            "summary": o.get("summary").cloned().unwrap_or(Value::Null),
            "basis": { "reads": reads.to_inputs(&self.ctx())? },
            "run_id": self.run_id,
            "executor": self.wish.payload.get("executor"),
        });
        let mut analysis = analysis;
        if analysis["summary"].is_null() {
            analysis.as_object_mut().unwrap().remove("summary");
        }
        if let Err(e) = check_analysis(&analysis) {
            problems.push(e.detail);
        }
        if !problems.is_empty() {
            return Ok(Err(problems));
        }
        Ok(Ok(Validated { analysis, inputs }))
    }
}
