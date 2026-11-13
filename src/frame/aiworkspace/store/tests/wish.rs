//! 许愿格 host pipeline against a real SQLite Workspace (no model, no Deno: macro output is given as
//! the JSON `lib/run.js` writes): the context map and tools on a snapshot, analysis validation and
//! write-back, the three stages (analyze → code → run, AI Native 第一期 §2.1), coding-stage
//! materialization and read sets, result collection with the macro's `declare` resolved into its
//! manifest, the planner (identity, table merge by key, derived columns, manual edits, structure
//! confirmation), the application with freshness afterwards, and running the stored macro on only
//! what its manifest declares.

mod common;
use aiworkspace_core::freshness::main_status;
use aiworkspace_store::wish::context::{Stage, StageKind, StageOptions};
use aiworkspace_store::wish::plan::PlanRequest;
use aiworkspace_store::wish::results::{code_changed, html_code_digest, program_output, Collector};
use aiworkspace_store::wish::runs::PROGRAM_XLLM;
use aiworkspace_store::workspace::Workspace;
use common::*;
use serde_json::{json, Value};

fn catalog() -> Value {
    json!([
        { "renderer": "table", "title": "表格视图", "accepts": ["buckyos.table-source"] },
        { "renderer": "richtext", "title": "富文本", "accepts": ["buckyos.richtext"] },
        { "renderer": "record", "title": "记录", "accepts": ["buckyos.record"] },
        { "renderer": "asset", "title": "图片", "accepts": ["buckyos.asset-ref"] },
        { "renderer": "sample.bar-chart", "title": "柱状图", "accepts": ["buckyos.table-source"], "config_summary": "value=数值字段, by=分组字段" }
    ])
}

fn opts(kind: StageKind) -> StageOptions {
    StageOptions {
        kind,
        location: json!({ "cell_id": "blk-wish", "selection": ["blk-orders"] }),
        request: json!({ "today": "2026-10-06", "timezone": "Asia/Shanghai", "currency": "CNY" }),
        catalog: catalog(),
        controls: json!({ "controls": [] }),
        map_budget_chars: 16_000,
        materialize: kind != StageKind::Analyze,
        manifest: None,
    }
}

/// A canvas with a frame, an orders table shown through a filtered view on the left of the wish, a
/// chart above it, a note nearby and an annotation on the amount field.
fn setup(ws: &mut Workspace) {
    let who = alice();
    ok(ws, &who, json!([
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "Q3 经营" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "Q3 经营",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "Q3 经营", "content_folder_id": "sf-content" } },
        { "op": "entity.create", "entity_id": "sales-folder", "type_id": "buckyos.container", "parent_id": "data", "order_key": "s", "name": "销售",
          "payload": { "kind": "folder", "title": "销售" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "sales-folder", "order_key": "a", "name": "订单",
          "payload": { "title_field_id": "customer", "fields": [
            { "field_id": "month", "name": "月份", "type": "date" },
            { "field_id": "customer", "name": "客户", "type": "text" },
            { "field_id": "region", "name": "地区", "type": "select", "options": [{ "option_id": "east", "label": "华东" }, { "option_id": "south", "label": "华南" }] },
            { "field_id": "amount", "name": "销售额", "type": "decimal", "scale": 2 },
            { "field_id": "status", "name": "状态", "type": "select", "options": [{ "option_id": "done", "label": "已结算" }, { "option_id": "open", "label": "未结算" }] } ] } },
    ]));
    let rows: Vec<Value> = (0..30)
        .map(|i| {
            let month = ["2026-07-01", "2026-08-01", "2026-09-01"][i % 3];
            json!({ "record_id": format!("o{i}"), "values": { "month": month, "customer": format!("客户{}", i % 7), "region": if i % 4 == 0 { "south" } else { "east" },
                                                              "amount": format!("{}.50", 100 + i * 10), "status": if i == 29 { "open" } else { "done" } } })
        })
        .collect();
    ok(ws, &who, json!([{ "op": "table.insert_records", "source_id": "orders", "records": rows }]));
    ok(ws, &who, json!([
        { "op": "entity.create", "entity_id": "rules", "type_id": "buckyos.richtext", "parent_id": "sales-folder", "order_key": "b", "name": "口径说明",
          "payload": { "content": { "type": "doc", "content": [
            { "type": "heading", "attrs": { "block_id": "h1", "level": 2 }, "content": [{ "type": "text", "text": "口径" }] },
            { "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "销售额按含税金额统计。" }] }] } } },
        { "op": "entity.create", "entity_id": "wish-q3", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w", "name": "季度分析",
          "payload": { "title": "季度分析", "prompt": "分析左边那张只显示华东的表，按月汇总销售额", "executor": "xllm", "output_mode": "overwrite",
                       "knowledge": "季度按自然季度", "output": { "container_id": "sf-content", "surface_id": "sf", "name": "季度分析结果" } } },
        { "op": "entity.create", "entity_id": "note-1", "type_id": "buckyos.annotation", "parent_id": "sf-content", "order_key": "n",
          "payload": { "kind": "note", "body": "9 月数据含未结算订单，统计时排除 状态=未结算" } },
        { "op": "entity.create", "entity_id": "frame-1", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 1600, "h": 1000 },
          "payload": { "view": { "type": "frame" }, "title": "华东复盘" } },
        { "op": "entity.create", "entity_id": "blk-orders", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "b", "placement": { "x": 40, "y": 300, "w": 500, "h": 300 },
          "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "orders" }, "title": "华东订单",
                       "filter": { "op": "cmp", "field_id": "region", "operator": "eq", "value": "east" }, "sorts": [{ "field_id": "month", "direction": "desc" }] } },
        { "op": "entity.create", "entity_id": "blk-chart", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "c", "placement": { "x": 580, "y": 40, "w": 420, "h": 220 },
          "payload": { "view": { "type": "sample.bar-chart" }, "source_ref": { "entity_id": "orders" }, "title": "月度销售额", "config": { "value": "amount", "by": "month" } } },
        { "op": "entity.create", "entity_id": "blk-wish", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "d", "placement": { "x": 580, "y": 300, "w": 420, "h": 300 },
          "payload": { "view": { "type": "wish" }, "source_ref": { "entity_id": "wish-q3" }, "title": "许愿格" } },
        { "op": "entity.create", "entity_id": "blk-note", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "e", "placement": { "x": 1040, "y": 320, "w": 200, "h": 120 },
          "payload": { "view": { "type": "note" }, "source_ref": { "entity_id": "note-1" } } },
        { "op": "entity.create", "entity_id": "blk-rules", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "f", "placement": { "x": 1300, "y": 300, "w": 260, "h": 200 },
          "payload": { "view": { "type": "richtext" }, "source_ref": { "entity_id": "rules" }, "title": "口径说明" } },
    ]));
}

/// A stage of `wish-q3` as the host opens it: running reads the stored macro's manifest.
fn stage(ws: &Workspace, kind: StageKind, run: &str, dir: &std::path::Path) -> Stage {
    let snap = ws.wish_snapshot(&alice()).unwrap();
    let mut o = opts(kind);
    if kind == StageKind::Run {
        o.manifest = stored_program(ws).get("manifest").cloned().filter(Value::is_object);
    }
    Stage::open(snap, "wish-q3", run, &dir.join(run), &o).unwrap()
}

/// The macro `wish-q3` stores (`program`), or null.
fn stored_program(ws: &Workspace) -> Value {
    ws.read(&alice(), "wish-q3", None).unwrap()["content"]["payload"]["program"].clone()
}

fn handle_of(stage: &Stage, id: &str) -> String {
    stage.handles.get(id).unwrap().to_string()
}

#[test]
fn map_tools_analysis_and_writeback() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "季度", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let work = tempfile::tempdir().unwrap();
    let mut st = stage(&ws, StageKind::Analyze, "run-a", work.path());
    let map = std::fs::read_to_string(work.path().join("run-a/WORKSPACE.md")).unwrap();
    let b_orders = handle_of(&st, "blk-orders");
    let t_orders = handle_of(&st, "orders");
    // task location, direction, view conditions by name, the frame, the note and knowledge
    assert!(map.contains("canvas \"Q3 经营\"") && map.contains("frame \"华东复盘\""), "{map}");
    assert!(map.contains(&format!("Selected when triggered: {b_orders}")), "{map}");
    let line = map.lines().find(|l| l.contains(&b_orders) && l.contains("table view")).unwrap();
    assert!(line.contains("to the left, adjacent") && line.contains("same frame") && line.contains("地区=华东") && line.contains("月份 descending"), "{line}");
    assert!(map.contains("value=销售额(@T1.f4)") && map.contains("by=月份(@T1.f1)"), "{map}");
    assert!(map.contains("frame \"华东复盘\" (a pure UI frame; @W is inside it)") && map.contains("sticky note \"9 月数据含未结算订单"), "{map}");
    assert!(map.contains("9 月数据含未结算订单") && map.contains("季度按自然季度"), "{map}");
    assert!(map.contains(&format!("{t_orders} table \"订单\" /data/销售/订单 · 30 rows · 5 columns")), "{map}");
    assert!(map.contains("地区 select (华东 73%"), "{map}");
    // the snapshot holds while the Workspace moves on
    let rev = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o1", "field_id": "amount" }))).unwrap()["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "999.00", "expect": { "rev": rev } }] }]));
    let q = st.ws_read(&json!({ "target": b_orders, "limit": 50 })).unwrap();
    assert!(q.contains("read through view") && q.contains("22 rows in total") && !q.contains("999") && q.contains("110.5"), "{q}");
    assert!(q.contains("The view as SQL (ws_query): SELECT"), "{q}");
    let q2 = st.ws_query(&json!({ "sql": format!("SELECT * FROM \"{t_orders}\" WHERE \"状态\" = '未结算'") })).unwrap();
    assert!(q2.starts_with("1 row(s)") && q2.contains("客户1"), "{q2}");
    let q3 = st.ws_query(&json!({ "sql": "SELECT count(*) FROM \"/data/销售/订单\"" })).unwrap();
    assert!(q3.contains("[30]"), "the snapshot holds for SQL too: {q3}");
    assert!(st.ws_find(&json!({ "text": "含税" })).unwrap().contains("口径说明"));
    assert!(st.ws_neighbors(&json!({ "cell": handle_of(&st, "blk-wish") })).unwrap().contains("above, adjacent"));
    assert!(st.ws_profile(&json!({ "target": b_orders })).unwrap().contains("\"rows\": 30"));
    assert!(st.ws_read(&json!({ "target": "@T99" })).unwrap_err().detail.contains("unknown handle"));

    // analysis: handles bind real data; a view handle binds the view; the prompt loses its handles
    let analysis = json!({
        "status": "ready",
        "context_prompt": format!("读取 {b_orders} 的全部记录，排除状态=未结算，按 {} 的口径汇总", handle_of(&st, "blk-chart")),
        "inputs": [{ "name": "sales", "ref": b_orders }, { "name": "rules", "ref": handle_of(&st, "blk-rules") }],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"],
              "views": [{ "renderer": "table" }, { "renderer": "sample.bar-chart", "config": { "value": "销售额", "by": "月份" } }] },
            { "name": "commentary", "type": "richtext", "title": "解读" }] },
        "checks": [{ "id": "total", "kind": "program", "text": "合计等于输入合计" }, { "id": "cited", "kind": "review", "text": "数字可追溯" }],
        "blockers": [], "warnings": []
    });
    let bad = st.validate_analysis(&analysis).unwrap().unwrap_err();
    assert!(bad.iter().any(|p| p.contains(&handle_of(&st, "blk-chart")) && p.contains("not bound as inputs")), "an unbound handle in the prompt: {bad:?}");
    let mut fixed = analysis.clone();
    fixed["context_prompt"] = json!(format!("读取 {b_orders} 的全部记录，排除状态=未结算，按月汇总 {t_orders}.f4；口径见 {}", handle_of(&st, "rules")));
    // a table result needs its logical key; an old schema version is refused
    let mut wrong = fixed.clone();
    wrong["output_contract"]["results"][0].as_object_mut().unwrap().remove("key");
    assert!(st.validate_analysis(&wrong).unwrap().unwrap_err().iter().any(|p| p.contains("needs a key")));
    let mut wrong = fixed.clone();
    wrong["schema_version"] = json!("wish.analysis.v2");
    assert!(st.validate_analysis(&wrong).unwrap().unwrap_err().iter().any(|p| p.contains("wish.analysis.v3")));
    // v3 has no approach: every result comes from the macro, so an approach the model still writes is dropped
    let mut old = fixed.clone();
    old["output_contract"]["results"][0]["approach"] = json!("program");
    old["output_contract"]["results"][1]["approach"] = json!("direct");
    let v = st.validate_analysis(&old).unwrap().unwrap();
    assert_eq!(v.analysis["schema_version"], "wish.analysis.v3");
    assert!(v.analysis["output_contract"]["results"].as_array().unwrap().iter().all(|r| r.get("approach").is_none()), "{}", v.analysis);
    let mut wrong = fixed.clone();
    wrong["output_contract"]["results"][0]["views"][1]["renderer"] = json!("acme.pie");
    assert!(st.validate_analysis(&wrong).unwrap().unwrap_err().iter().any(|p| p.contains("acme.pie")));
    let v = st.validate_analysis(&fixed).unwrap().unwrap();
    assert_eq!(v.inputs[0]["entity_id"], "orders");
    assert_eq!(v.inputs[0]["selector"], json!({ "kind": "table_view", "cell_id": "blk-orders" }));
    assert_eq!(v.inputs[1]["entity_id"], "rules");
    let cp = v.analysis["context_prompt"].as_str().unwrap();
    assert!(!cp.contains('@') && cp.contains("input `sales`") && cp.contains("input `rules`") && cp.contains("field \"销售额\" of `sales`"), "{cp}");
    // write-back: one commit; then a prompt change makes it need analysis again
    let mut cand = json!({ "wish_id": "wish-q3", "analysis": v.analysis, "inputs": v.inputs });
    cand["basis"] = st.evidence().unwrap()["basis"].clone();
    let plan = ws.wish_analysis_plan(&alice(), &cand).unwrap();
    assert!(plan.ready);
    let run = new_run(&mut ws, "analyze");
    apply(&mut ws, &run, &plan);
    let f = ws.freshness(&alice(), &["wish-q3".into()]).unwrap();
    assert_eq!(f["items"][0]["needs_analysis"], false, "{f}");
    // the same analysis plan again: the wish moved on, so it is refused
    assert!(!ws.wish_analysis_plan(&alice(), &cand).unwrap().ready);
}

/// The JSON `lib/run.js` writes for the monthly summary.
fn monthly_output(rows: &[(&str, f64, u64)], total: f64) -> Value {
    json!({
        "results": [{ "name": "monthly", "type": "table", "key": ["月份"],
                      "fields": [{ "name": "月份", "type": "date" }, { "name": "销售额", "type": "decimal" }, { "name": "订单数", "type": "number" }],
                      "rows": rows.iter().map(|(m, s, n)| json!({ "月份": m, "销售额": s, "订单数": n })).collect::<Vec<_>>() }],
        "facts": { "total": total, "months": rows.len() },
        "checks": [{ "id": "total", "passed": true, "detail": "一致" }]
    })
}

/// The commentary the macro had the model write (`aiws.ai.complete`), as a rich text result.
fn with_commentary(mut out: Value, text: &str) -> Value {
    out["results"].as_array_mut().unwrap().push(json!({ "name": "commentary", "type": "text", "markdown": text }));
    out["ai"] = json!({ "complete": { "calls": 1, "cached": 0, "tokens": 800 } });
    out
}

/// What the macro's `aiws.declare` wrote: the fields of `sales` it reads, by their names while coding.
fn with_declare(mut out: Value) -> Value {
    out["declare"] = json!({ "inputs": { "sales": ["月份", "销售额", "状态"] }, "ai": { "complete": { "max_calls": 2 } }, "params": ["today"] });
    out
}

const SOURCE: &str = "export default async function main(aiws) { aiws.declare({ inputs: { sales: ['月份', '销售额', '状态'] } }) }";

#[test]
fn code_collect_plan_apply_then_run_the_macro() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "季度", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    // analysis written as the host would
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "prompt": "分析左边那张只显示华东的表，按月汇总销售额",
        "context_prompt": "读取输入 `sales`，按月汇总", "checks": [{ "id": "total", "kind": "program", "text": "合计一致" }], "blockers": [], "warnings": [],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"],
              "views": [{ "renderer": "table" }, { "renderer": "sample.bar-chart", "config": { "value": "销售额", "by": "月份" } }] },
            { "name": "commentary", "type": "richtext", "title": "解读" }], "placement": "frame:frame-1" } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } },
        { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales", "selector": { "kind": "table_view", "cell_id": "blk-orders" } }], "expect": { "rev": 0 } }] }]));
    // analysed, nothing coded yet: coding is next
    let f = ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0].clone();
    assert_eq!(f["needs_coding"], json!({ "reasons": [{ "code": "no_macro" }] }), "{f}");
    assert_eq!(main_status(&f, &json!({}))["main"], "needs_coding");
    let work = tempfile::tempdir().unwrap();
    let e1 = new_run(&mut ws, "code");
    let mut st = stage(&ws, StageKind::Code, &e1, work.path());
    // the view is the range: 22 east rows, newest month first, option labels and numbers; coding sees every field
    let rows = std::fs::read_to_string(work.path().join(&e1).join("context/entities/orders/rows.jsonl")).unwrap();
    assert_eq!(rows.lines().count(), 22);
    assert!(rows.lines().next().unwrap().contains("2026-09-01") && rows.contains("\"华东\"") && rows.contains("110.5"));
    let inputs: Value = serde_json::from_str(&std::fs::read_to_string(work.path().join(&e1).join("context/inputs.json")).unwrap()).unwrap();
    assert_eq!(inputs["sales"]["selector"]["kind"], "table_view");
    assert_eq!(st.input_infos().unwrap()["sales"].fields.len(), 5);
    assert!(work.path().join(&e1).join("WORKSPACE.md").exists(), "the model codes with the map");
    for gone in ["context/data-tree.json", "context/block-tree.json"] {
        assert!(!work.path().join(&e1).join(gone).exists(), "{gone}: the macro gets no tree of the Workspace");
    }
    // reading undeclared data while coding appends it, recorded and materialized
    let note = st.ws_read(&json!({ "target": handle_of(&st, "rules") })).unwrap();
    assert!(note.contains("input `input1` appended during execution") || note.contains("appended during execution"), "{note}");
    assert!(work.path().join(&e1).join("context/entities/rules/content.md").exists());
    let ev = st.evidence().unwrap();
    assert_eq!(ev["stage"], "code");
    let rs = ev["read_set"].as_array().unwrap();
    let has = |id: &str, kind: &str| rs.iter().any(|r| r["entity_id"] == id && r["selector"]["kind"].as_str().unwrap_or("entity") == kind);
    assert!(has("orders", "table_members") && has("orders", "table_field_values") && has("blk-orders", "doc_key") && has("rules", "entity"), "{ev}");
    assert!(rs.iter().any(|r| r["entity_id"] == "blk-orders" && r["selector"]["key"] == "filter"));
    assert_eq!(ev["appended_inputs"][0]["entity_id"], "rules");

    // collection: contract rules and the on-the-spot errors
    let mut c = Collector { contract: analysis_contract(&ws), checks_def: analysis_checks(&ws), workdir: work.path().join(&e1), inputs: st.input_infos().unwrap(), ..Default::default() };
    let wrong_type = json!({ "name": "commentary", "type": "table", "key": ["x"], "rows": [{ "x": 1 }] });
    assert!(c.normalize(&wrong_type, "program").unwrap_err()[0].contains("contracted as richtext"));
    let program_text = json!({ "name": "monthly", "type": "text", "markdown": "x" });
    assert!(c.normalize(&program_text, "program").unwrap_err()[0].contains("contracted as table"));
    let extra = json!({ "name": "bonus", "type": "text", "markdown": "x" });
    assert!(c.normalize(&extra, "program").unwrap_err()[0].contains("not in the output contract"));
    let dup = monthly_output(&[("2026-07-01", 1.0, 1), ("2026-07-01", 2.0, 1)], 3.0);
    assert!(program_output(&c, &dup, SOURCE, "").unwrap_err().iter().any(|e| e.contains("duplicate")));
    // `declare` is resolved against the inputs of this stage: unknown inputs and fields are errors
    let mut bad = with_declare(monthly_output(&[("2026-07-01", 1.0, 1)], 1.0));
    bad["declare"]["inputs"]["sales"] = json!(["月份", "金额"]);
    bad["declare"]["inputs"]["nope"] = json!([]);
    let errs = program_output(&c, &bad, SOURCE, "").unwrap_err();
    assert!(errs.iter().any(|e| e.contains("no field 金额")) && errs.iter().any(|e| e.contains("no input named nope")), "{errs:?}");
    let mut bad = with_declare(monthly_output(&[("2026-07-01", 1.0, 1)], 1.0));
    bad["declare"]["ai"] = json!({ "complete": { "max_calls": 0 } });
    assert!(program_output(&c, &bad, SOURCE, "").unwrap_err().iter().any(|e| e.contains("aiws.declare: manifest.ai.complete")));
    // a run without the commentary and without a declaration does not finish
    let rows3 = [("2026-07-01", 1000.5, 8), ("2026-08-01", 1100.5, 7), ("2026-09-01", 1200.5, 7)];
    c.program = Some(program_output(&c, &monthly_output(&rows3, 3301.5), SOURCE, "").unwrap());
    let blocking = c.blocking();
    assert!(blocking.iter().any(|b| b.contains("commentary")) && blocking.iter().any(|b| b.contains("aiws.declare")), "{blocking:?}");
    // the macro writes the commentary itself; its numbers are checked against what it computed
    let out = with_declare(with_commentary(monthly_output(&rows3, 3301.5), "## 解读\n\n三季度合计 3,301.5，9 月 1,200.5，环比增长 9.1%，约 1,300 单。"));
    let run = program_output(&c, &out, SOURCE, "").unwrap();
    let manifest = run.manifest.clone().unwrap();
    assert_eq!(manifest["inputs"]["sales"], json!({ "entity_id": "orders", "type_id": "buckyos.table-source", "fields": [
        { "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "amount", "name": "销售额", "type": "decimal" }, { "field_id": "status", "name": "状态", "type": "select" }] }));
    assert_eq!(manifest["results"], json!({ "monthly": { "type": "table" }, "commentary": { "type": "richtext" } }));
    assert_eq!((&manifest["ai"], &manifest["params"]), (&json!({ "complete": { "max_calls": 2 } }), &json!(["today"])));
    assert_eq!(run.digest, aiworkspace_core::wish::macro_digest(SOURCE, &manifest), "the macro version is its source and its manifest");
    assert!(run.used_ai());
    c.program = Some(run);
    let uncited = c.uncited_numbers(&[]);
    assert_eq!(uncited, vec![json!({ "result": "commentary", "numbers": ["9.1", "1,300"] })], "{uncited:?}");
    assert!(c.blocking().is_empty(), "{:?}", c.blocking());
    c.finished = Some(json!({ "summary": "完成", "assumptions": ["空值按 0"], "review_notes": [{ "id": "cited", "note": "已核对" }] }));
    let checks = c.checks();
    assert_eq!(checks.iter().find(|x| x["id"] == "total").unwrap()["status"], "passed");

    let candidate = build_candidate(&mut ws, &c, &st, &e1, "code");
    assert_eq!(candidate["model_judgment"], json!(["monthly", "commentary"]));
    drop(st);
    // preview → apply: the trial run is the first run, the macro is written with its results
    let plan = ws.wish_plan(&alice(), &PlanRequest { run_id: &e1, candidate: &candidate, choices: &json!({}), location: &json!({ "cell_id": "blk-wish" }) }).unwrap();
    assert!(plan.ready, "{}", plan.summary);
    let results = plan.summary["results"].as_array().unwrap();
    assert_eq!(results.iter().map(|r| r["action"].as_str().unwrap()).collect::<Vec<_>>(), ["create", "create"]);
    assert_eq!(results[0]["blocks"], 2, "two views of the same table");
    assert_eq!(plan.summary["program"]["to"], candidate["program"]["digest"]);
    apply(&mut ws, &e1, &plan);
    let wish = ws.read(&alice(), "wish-q3", None).unwrap()["content"]["payload"].clone();
    assert_eq!((wish["program"]["api_version"].as_u64(), &wish["program"]["manifest"], &wish["program"]["digest"]),
               (Some(aiworkspace_core::wish::PROGRAM_API_VERSION), &manifest, &candidate["program"]["digest"]), "{}", wish["program"]);
    let b = &wish["last_run"]["result_bindings"];
    let monthly = b["results"]["monthly"]["entity_id"].as_str().unwrap().to_string();
    let commentary = b["results"]["commentary"]["entity_id"].as_str().unwrap().to_string();
    assert_eq!(b["results"]["monthly"]["fields"]["销售额"], "f2");
    let table_cell = b["results"]["monthly"]["cells"][0].as_str().unwrap().to_string();
    assert_eq!(view_months(&ws, &table_cell), ["2026-07-01", "2026-08-01", "2026-09-01"], "the table Block shows the program's row order");
    // the contract names a frame: the result group sits inside it, below its title
    let group = ws.read(&alice(), b["group"].as_str().unwrap(), None).unwrap();
    assert_eq!((group["placement"]["x"].as_f64(), group["placement"]["y"].as_f64()), (Some(20.0), Some(40.0)), "{group}");
    assert_eq!(wish["inputs"].as_array().unwrap().len(), 2, "the appended input joined the wish");
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["needs_analysis"], false, "the host re-stated the analysis");
    let t = ws.query(&alice(), &json!({ "sql": "SELECT count(*) FROM t", "tables": { "t": monthly } })).unwrap();
    assert_eq!(t["rows"][0][0], 3);
    let chart = ws.list(&alice(), &json!({ "at": b["group"] })).unwrap();
    let chart = chart["items"].as_array().unwrap().iter().find(|c| c["entity_id"].as_str().unwrap().ends_with("v1")).unwrap().clone();
    let chart = ws.read(&alice(), chart["entity_id"].as_str().unwrap(), None).unwrap();
    assert_eq!(chart["content"]["payload"]["config"], json!({ "value": "f2", "by": "f1" }), "chart config by field ids");
    let d = ws.read(&alice(), &commentary, None).unwrap()["derived"].clone();
    assert_eq!((d["approach"].as_str(), d["result_key"].as_str(), d["model_judgment"].as_bool()), (Some("program"), Some("commentary"), Some(true)), "{d}");
    assert_eq!(d["program_digest"], candidate["program"]["digest"]);
    // the wish's body states its macro in plain text
    let body = ws.read_as(&alice(), &json!({ "entity_id": "wish-q3", "as": "body" })).unwrap();
    assert!(body["text"].as_str().unwrap().contains("macro:\n- reads sales: 月份, 销售额, 状态\n- writes commentary (richtext), monthly (table)"), "{body}");
    let f = ws.freshness(&alice(), &["wish-q3".into(), monthly.clone()]).unwrap();
    assert_eq!(f["items"][0]["status"], "current", "{f}");
    assert_eq!(f["items"][0]["needs_coding"], Value::Null, "{f}");
    assert!(f["items"][0].get("direct_stale").is_none());
    assert_eq!(main_status(&f["items"][0], &json!({})), json!({ "main": "current", "next_action": "none", "reasons": [], "stale": false }));
    assert_eq!(f["items"][1]["status"], "current");
    // moving a Block does not matter; changing what the view shows does
    ok(&mut ws, &alice(), json!([{ "op": "tree.place", "entity_id": "blk-chart", "placement": { "x": 600, "y": 40, "w": 420, "h": 220 } }]));
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "current");
    let rev = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o1", "field_id": "amount" }))).unwrap()["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "150.00", "expect": { "rev": rev } }] }]));
    let f = ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0].clone();
    assert_eq!(f["status"], "stale");
    assert_eq!(main_status(&f, &json!({}))["next_action"], "run", "the macro still fits: run it");

    // running the stored macro: no model, the same identities, rows merged by key, and every result regenerated
    let work2 = tempfile::tempdir().unwrap();
    let p2 = new_run(&mut ws, "run");
    let st2 = stage(&ws, StageKind::Run, &p2, work2.path());
    assert_eq!(st2.evidence().unwrap()["stage"], "run");
    let mut c2 = Collector { contract: analysis_contract(&ws), checks_def: analysis_checks(&ws), workdir: work2.path().join(&p2), inputs: st2.input_infos().unwrap(),
                             prev_bindings: bindings(&ws), stored_manifest: Some(manifest.clone()), ..Default::default() };
    // a stored macro is not asked to declare again: its manifest is the stored one
    let out = with_commentary(monthly_output(&[("2026-10-01", 10.0, 1), ("2026-09-01", 1200.5, 7), ("2026-08-01", 1100.5, 7), ("2026-07-01", 1040.0, 8)], 3351.0),
                              "## 解读\n\n合计 3,351，10 月新增 1 单。");
    let run2 = program_output(&c2, &out, SOURCE, "").unwrap();
    assert_eq!(run2.manifest.as_ref(), Some(&manifest));
    assert_eq!(run2.digest, wish["program"]["digest"], "the same macro version");
    assert!(code_changed(&manifest, &run2.results).is_empty());
    c2.program = Some(run2);
    assert!(c2.blocking().is_empty(), "{:?}", c2.blocking());
    let cand2 = build_candidate(&mut ws, &c2, &st2, &p2, "run");
    assert!(cand2["program"].is_null(), "running never writes the macro");
    drop(st2);
    let plan2 = ws.wish_plan(&alice(), &PlanRequest { run_id: &p2, candidate: &cand2, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(plan2.ready, "{}", plan2.summary);
    assert_eq!(plan2.summary["program"], Value::Null);
    let rec_ids_before: Vec<String> = ids(&ws, &monthly);
    apply(&mut ws, &p2, &plan2);
    let rec_ids_after = ids(&ws, &monthly);
    assert!(rec_ids_before.iter().all(|i| rec_ids_after.contains(i)), "record ids kept");
    assert_eq!(rec_ids_after.len(), 4);
    // the program sorts newest first now: the Block follows
    assert_eq!(view_months(&ws, &table_cell), ["2026-10-01", "2026-09-01", "2026-08-01", "2026-07-01"]);
    assert!(plan2.summary["results"].as_array().unwrap().iter().any(|r| r["name"] == "monthly" && r["reordered"] == true), "{}", plan2.summary);
    assert_eq!(stored_program(&ws)["digest"], wish["program"]["digest"], "the macro is unchanged");
    // the user sorts the Block: later runs leave its order alone
    let rev = ws.read(&alice(), &table_cell, None).unwrap()["content"]["key_revs"]["sorts"].as_u64().unwrap_or(0);
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": table_cell, "keys": [{ "key": "sorts", "value": [{ "field_id": "f2", "direction": "desc" }], "expect": { "rev": rev } }] }]));
    let f = ws.freshness(&alice(), &["wish-q3".into(), commentary.clone(), monthly.clone()]).unwrap();
    assert_eq!(f["items"][2]["status"], "current");
    assert_eq!(f["items"][1]["status"], "current", "the run wrote the commentary again: nothing is left stale");
    assert_eq!(ws.read(&alice(), &commentary, None).unwrap()["derived"]["run_id"], json!(p2));
    let text = ws.read_as(&alice(), &json!({ "entity_id": commentary, "as": "body" })).unwrap();
    assert!(text["text"].as_str().unwrap().contains("合计 3,351"), "{text}");
    assert_eq!(f["items"][0]["status"], "current", "{f}");
    assert!(f["items"][0].get("direct_stale").is_none());

    // a manual edit of the table: the next application asks keep / replace / new
    let r0 = ids(&ws, &monthly)[0].clone();
    let rev = ws.read(&alice(), &monthly, Some(&json!({ "kind": "table_cell", "record_id": r0, "field_id": "f3" }))).unwrap()["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": monthly, "values": [{ "record_id": r0, "field_id": "f3", "value": 99, "expect": { "rev": rev } }] }]));
    let work3 = tempfile::tempdir().unwrap();
    let p3 = new_run(&mut ws, "run");
    let st3 = stage(&ws, StageKind::Run, &p3, work3.path());
    let mut c3 = Collector { contract: analysis_contract(&ws), checks_def: analysis_checks(&ws), workdir: work3.path().join(&p3), inputs: st3.input_infos().unwrap(),
                             stored_manifest: Some(manifest.clone()), ..Default::default() };
    // a row disappears this time: destructive, must be confirmed
    let out = with_commentary(monthly_output(&[("2026-07-01", 1040.0, 8), ("2026-08-01", 1100.5, 7), ("2026-09-01", 1200.5, 7)], 3341.0), "## 解读\n\n合计 3,341。");
    c3.program = Some(program_output(&c3, &out, SOURCE, "").unwrap());
    let cand3 = build_candidate(&mut ws, &c3, &st3, &p3, "run");
    drop(st3);
    let plan3 = ws.wish_plan(&alice(), &PlanRequest { run_id: &p3, candidate: &cand3, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(!plan3.ready);
    assert_eq!(plan3.summary["manual"][0]["name"], "monthly");
    let plan3 = ws.wish_plan(&alice(), &PlanRequest { run_id: &p3, candidate: &cand3, choices: &json!({ "results": { "monthly": "replace" } }), location: &json!({}) }).unwrap();
    assert!(!plan3.ready && plan3.summary["destructive"] == json!(true), "{}", plan3.summary);
    let plan3 = ws.wish_plan(&alice(), &PlanRequest { run_id: &p3, candidate: &cand3, choices: &json!({ "results": { "monthly": "replace" }, "confirm_structure": true }), location: &json!({}) }).unwrap();
    assert!(plan3.ready, "{}", plan3.summary);
    apply(&mut ws, &p3, &plan3);
    assert!(plan3.summary["results"].as_array().unwrap().iter().any(|r| r["name"] == "monthly" && r["reordered"] == false));
    assert_eq!(view_months(&ws, &table_cell), ["2026-09-01", "2026-08-01", "2026-07-01"], "the user's sort stays");
    assert_eq!(ids(&ws, &monthly).len(), 3);
    // the stale preview of an older run cannot be applied: the wish's last run moved on
    let again = ws.wish_plan(&alice(), &PlanRequest { run_id: &p2, candidate: &cand2, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(!again.ready && again.summary["problems"].to_string().contains("another set of results"));
}

#[test]
fn derived_columns_are_owned_and_do_not_stale_themselves() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "工资", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "prompt": "p", "context_prompt": "差值",
        "checks": [], "blockers": [], "warnings": [],
        "output_contract": { "results": [{ "name": "差值", "type": "table_columns", "target": "sales" }] } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } },
        { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales" }], "expect": { "rev": 0 } }] }]));
    let run = |ws: &mut Workspace, run: &str, choices: Value| -> aiworkspace_store::wish::plan::Plan {
        let run = &ws.wish_run_create(&alice(), PROGRAM_XLLM, &json!({ "wish_id": "wish-q3", "stage": "code", "k": run }), run, &json!(null)).unwrap().0.clone();
        let work = tempfile::tempdir().unwrap();
        let st = stage(ws, StageKind::Code, run, work.path());
        let rows = std::fs::read_to_string(work.path().join(run).join("context/entities/orders/rows.jsonl")).unwrap();
        let mut values = serde_json::Map::new();
        let parsed: Vec<Value> = rows.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        // owned columns are never handed to the program
        assert!(parsed.iter().all(|r| r["v"].as_object().unwrap().len() == 5), "{}", parsed[0]);
        let avg: f64 = parsed.iter().map(|r| r["v"]["amount"].as_f64().unwrap()).sum::<f64>() / parsed.len() as f64;
        for r in &parsed {
            values.insert(r["id"].as_str().unwrap().into(), json!({ "差值": r["v"]["amount"].as_f64().unwrap() - avg }));
        }
        let mut c = Collector { contract: analysis_contract(ws), checks_def: vec![], workdir: work.path().join(run), inputs: st.input_infos().unwrap(), prev_bindings: bindings(ws), ..Default::default() };
        let out = json!({ "results": [{ "name": "差值", "type": "columns", "input": "sales", "fields": { "差值": "decimal" }, "values": values }], "facts": {}, "checks": [],
                          "declare": { "inputs": { "sales": ["销售额"] } } });
        c.program = Some(program_output(&c, &out, "src", "").unwrap());
        // a column this wish writes is not an input field the macro can declare
        let mut own = out.clone();
        own["declare"]["inputs"]["sales"] = json!(["差值"]);
        assert!(program_output(&c, &own, "src", "").is_err());
        let cand = build_candidate(ws, &c, &st, run, "code");
        drop(st);
        ws.wish_plan(&alice(), &PlanRequest { run_id: run, candidate: &cand, choices: &choices, location: &json!({}) }).unwrap()
    };
    let plan = run(&mut ws, "run-c1", json!({}));
    assert!(plan.ready, "{}", plan.summary);
    let c1 = run_of(&plan);
    apply(&mut ws, &c1, &plan);
    let wish = ws.read(&alice(), "wish-q3", None).unwrap()["content"]["payload"].clone();
    let fid = wish["last_run"]["result_bindings"]["results"]["差值"]["fields"]["差值"].as_str().unwrap().to_string();
    let field = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_field", "field_id": fid }))).unwrap();
    assert_eq!(field["content"]["maintained_by"], "program");
    let cell = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o0", "field_id": fid }))).unwrap();
    assert!(cell["content"]["meta"]["derived"]["run_id"].is_string(), "{cell}");
    // writing its own column did not make the wish stale (the column is not in its read set)
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "current");
    // a new record makes it stale; a re-run fills the new row and keeps the hand-edited cell when asked
    let rev = cell["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o0", "field_id": fid, "value": "1.00", "expect": { "rev": rev } }] }]));
    ok(&mut ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "orders", "records": [{ "record_id": "o99", "values": { "month": "2026-09-01", "customer": "新客户", "region": "east", "amount": "500.00", "status": "done" } }] }]));
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "stale");
    let plan = run(&mut ws, "run-c2", json!({}));
    assert_eq!(plan.summary["manual"][0]["cells"], 1, "{}", plan.summary);
    let plan = run(&mut ws, "run-c2", json!({ "results": { "差值": "keep" } }));
    assert!(plan.ready, "{}", plan.summary);
    let c2 = run_of(&plan);
    apply(&mut ws, &c2, &plan);
    let kept = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o0", "field_id": fid }))).unwrap();
    assert_eq!(kept["content"]["value"].as_str().and_then(|v| v.parse::<f64>().ok()), Some(1.0), "{kept}");
    let filled = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o99", "field_id": fid }))).unwrap();
    assert!(filled["content"]["value"].is_string());
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "current");
}

/// The run a plan was made for (its last_run write names it).
fn run_of(plan: &aiworkspace_store::wish::plan::Plan) -> String {
    plan.operations.iter().rev().find_map(|o| o["keys"].as_array().and_then(|k| k.iter().find(|k| k["key"] == "last_run")).map(|k| k["value"]["run_id"].as_str().unwrap().to_string())).unwrap()
}

fn analysis_contract(ws: &Workspace) -> Value {
    ws.read(&alice(), "wish-q3", None).unwrap()["content"]["payload"]["analysis"]["output_contract"].clone()
}
fn analysis_checks(ws: &Workspace) -> Vec<Value> {
    ws.read(&alice(), "wish-q3", None).unwrap()["content"]["payload"]["analysis"]["checks"].as_array().cloned().unwrap_or_default()
}
fn bindings(ws: &Workspace) -> std::collections::BTreeMap<String, Value> {
    aiworkspace_core::wish::current_results(ws.read(&alice(), "wish-q3", None).unwrap()["content"]["payload"].as_object().unwrap()).into_iter().collect()
}
/// 月份 (field f1) of the rows as a table Block shows them.
fn view_months(ws: &Workspace, cell: &str) -> Vec<String> {
    let m = ws.read_as(&alice(), &json!({ "entity_id": cell, "as": "meta" })).unwrap();
    let sql = m["shows"]["view_sql"].as_str().unwrap().replacen("SELECT \"_id\"", "SELECT \"_record\"", 1);
    let q = ws.query(&alice(), &json!({ "sql": sql })).unwrap();
    q["rows"].as_array().unwrap().iter().map(|r| serde_json::from_str::<Value>(r[0].as_str().unwrap()).unwrap()["values"]["f1"].as_str().unwrap().to_string()).collect()
}
fn ids(ws: &Workspace, table: &str) -> Vec<String> {
    let q = ws.query(&alice(), &json!({ "sql": "SELECT _id FROM t ORDER BY _id", "tables": { "t": table } })).unwrap();
    q["rows"].as_array().unwrap().iter().map(|r| r[0].as_str().unwrap().to_string()).collect()
}

/// The candidate as the host makes it (`code_tail` / `run_tail`): coding carries the macro version, its source
/// staged as an asset; running never does.
fn build_candidate(ws: &mut Workspace, c: &Collector, st: &Stage, run: &str, mode: &str) -> Value {
    let mut cand = c.candidate();
    let ev = st.evidence().unwrap();
    for k in ["wish_id", "read_set", "appended_inputs", "basis"] {
        cand[k] = ev[k].clone();
    }
    cand["executor"] = json!("xllm");
    cand["run_id"] = json!(run);
    cand["executed_at"] = json!("2026-10-06T10:00:00.000Z");
    cand["mode"] = json!(mode);
    if let Some(p) = c.program.as_ref().filter(|_| mode == "code") {
        let staged = ws.stage_asset(&alice(), p.source.as_bytes()).unwrap();
        cand["program"] = json!({ "digest": p.digest, "object_id": staged["object_id"], "manifest": p.manifest,
                                  "produces": p.results.iter().map(|r| r["name"].clone()).collect::<Vec<_>>() });
    }
    cand
}

fn new_run(ws: &mut Workspace, stage: &str) -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let key = format!("t-run-{}", N.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    ws.wish_run_create(&alice(), PROGRAM_XLLM, &json!({ "wish_id": "wish-q3", "stage": stage, "k": key }), &key, &json!(null)).unwrap().0
}

/// Preview kept, then applied through the run (one ordinary, idempotent commit).
fn apply(ws: &mut Workspace, run: &str, plan: &aiworkspace_store::wish::plan::Plan) {
    ws.wish_run_set(run, Some("waiting_confirmation"), None, Some(&json!({ "epoch": ws.epoch })), None, None).unwrap();
    ws.wish_run_keep_plan(run, plan).unwrap();
    let r = ws.wish_apply(&alice(), run, &plan.digest, Some("s1")).unwrap();
    assert_eq!(r["status"], "accepted", "{r}");
    // applying the same preview again is the same application
    assert_eq!(ws.wish_apply(&alice(), run, &plan.digest, Some("s1")).unwrap()["commit"]["commit_id"], r["commit"]["commit_id"]);
}

/// W07: an input that (indirectly) comes from this wish's own results is refused; a folder holding
/// one too. The walk is bounded.
#[test]
fn generation_cycles_are_refused_through_other_wishes_and_folders() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "环", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let doc = |id: &str, parent: &str| json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.richtext", "parent_id": parent, "order_key": "z",
        "payload": { "content": { "type": "doc", "content": [{ "type": "paragraph", "attrs": { "block_id": "p1" } }] } } });
    let derived = |wish: &str, input: &str| json!({ "wish_id": wish, "run_id": "r", "executor": "xllm", "inputs": [{ "entity_id": input, "version": { "mode": "follow", "rev": 1 } }] });
    ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "f-mid", "type_id": "buckyos.container", "parent_id": "data", "order_key": "y", "name": "中间", "payload": { "kind": "folder", "title": "中间" } },
        doc("r-own", "sf-content"), { "op": "entity.set_derived", "entity_id": "r-own", "derived": derived("wish-q3", "orders") },
        doc("r-other", "f-mid"), { "op": "entity.set_derived", "entity_id": "r-other", "derived": derived("w-other", "r-own") },
    ]));
    let work = tempfile::tempdir().unwrap();
    let run = new_run(&mut ws, "analyze");
    let mut st = stage(&ws, StageKind::Analyze, &run, work.path());
    let a = |r: &str| json!({ "status": "ready", "context_prompt": "x", "inputs": [{ "name": "x", "ref": r }], "output_contract": { "results": [] }, "checks": [], "blockers": [], "warnings": [] });
    for target in ["r-own", "r-other", "f-mid"] {
        let h = st.handles.get(target).unwrap().to_string();
        let problems = st.validate_analysis(&a(&h)).unwrap().unwrap_err();
        assert!(problems.iter().any(|p| p.contains("generation cycle")), "{target}: {problems:?}");
    }
    let h = st.handles.get("orders").unwrap().to_string();
    assert!(st.validate_analysis(&a(&h)).unwrap().is_ok());
}

/// §8.1: a document whose rules went into the task description (`context_sources`) is part of the
/// analysis basis: changing it asks for a new analysis, and coding reads it as an input version.
#[test]
fn context_sources_require_reanalysis_when_they_change() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "口径", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let work = tempfile::tempdir().unwrap();
    let run = new_run(&mut ws, "analyze");
    let mut st = stage(&ws, StageKind::Analyze, &run, work.path());
    let rules = st.handles.get("rules").unwrap().to_string();
    let orders = st.handles.get("blk-orders").unwrap().to_string();
    let v = st.validate_analysis(&json!({ "status": "ready", "context_prompt": format!("读取 {orders}，销售额按含税金额（见口径说明）"), "inputs": [{ "name": "sales", "ref": orders }],
        "output_contract": { "results": [] }, "checks": [], "blockers": [], "warnings": [], "context_sources": [rules] })).unwrap().unwrap();
    assert_eq!(v.analysis["basis"]["reads"][0]["entity_id"], "rules");
    let mut cand = json!({ "wish_id": "wish-q3", "analysis": v.analysis, "inputs": v.inputs });
    cand["basis"] = st.evidence().unwrap()["basis"].clone();
    drop(st);
    let plan = ws.wish_analysis_plan(&alice(), &cand).unwrap();
    apply(&mut ws, &run, &plan);
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["needs_analysis"], false);
    // coding records the source document as read
    let e = new_run(&mut ws, "code");
    let w2 = tempfile::tempdir().unwrap();
    let st = stage(&ws, StageKind::Code, &e, w2.path());
    assert!(st.evidence().unwrap()["read_set"].as_array().unwrap().iter().any(|r| r["entity_id"] == "rules"));
    drop(st);
    // the rule text changes: the analysis no longer stands
    let hash = ws.read(&alice(), "rules", Some(&json!({ "kind": "richtext_block", "block_id": "p1" }))).unwrap()["content"]["hash"].clone();
    ok(&mut ws, &alice(), json!([{ "op": "richtext.replace_block", "entity_id": "rules", "block_id": "p1", "expect": { "hash": hash },
        "node": { "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "销售额按不含税金额统计。" }] } }]));
    let f = ws.freshness(&alice(), &["wish-q3".into()]).unwrap();
    assert_eq!(f["items"][0]["needs_analysis"], true, "{f}");
    assert_eq!(f["items"][0]["analysis_sources_changed"][0]["entity_id"], "rules");
}

/// W13: the application was accepted but the process died before the run said so. Reconciling
/// through the Commit idempotency record finds it: the run is succeeded and nothing is written twice.
#[test]
fn an_accepted_application_is_reconciled_after_a_crash() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "对账", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let work = tempfile::tempdir().unwrap();
    let run = new_run(&mut ws, "analyze");
    let mut st = stage(&ws, StageKind::Analyze, &run, work.path());
    let orders = st.handles.get("blk-orders").unwrap().to_string();
    let v = st.validate_analysis(&json!({ "status": "ready", "context_prompt": format!("读取 {orders}"), "inputs": [{ "name": "sales", "ref": orders }],
        "output_contract": { "results": [] }, "checks": [], "blockers": [], "warnings": [] })).unwrap().unwrap();
    let mut cand = json!({ "wish_id": "wish-q3", "analysis": v.analysis, "inputs": v.inputs });
    cand["basis"] = st.evidence().unwrap()["basis"].clone();
    drop(st);
    let plan = ws.wish_analysis_plan(&alice(), &cand).unwrap();
    apply(&mut ws, &run, &plan);
    let head = ws.head_seq;
    let key = format!("run/{run}/{}", &plan.digest[..plan.digest.len().min(32)]);
    // the crash: the run record still says applying
    ws.wish_run_set(&run, Some("applying"), None, None, None, Some(&json!({ "applied": Value::Null, "applying": { "plan_digest": plan.digest, "idempotency_key": key } }))).unwrap();
    assert_eq!(ws.wish_reconcile_apply(&run).unwrap().as_deref(), Some("succeeded"));
    let r = ws.wish_run(&alice(), &run).unwrap();
    assert_eq!(r["state"], "succeeded");
    assert_eq!(r["detail"]["applied"]["reconciled"], true);
    // the client resends apply: the same application, no new commit
    assert_eq!(ws.wish_apply(&alice(), &run, &plan.digest, Some("s1")).unwrap()["status"], "accepted");
    assert_eq!(ws.head_seq, head);
}

/// Stores a macro on `wish-q3` as coding would, reading `fields` (`(field_id, coding name, type)`) of `sales`.
fn store_macro(ws: &mut Workspace, fields: &[(&str, &str, &str)], extra_inputs: Value) -> Value {
    let mut inputs = json!({ "sales": { "entity_id": "orders", "type_id": "buckyos.table-source",
                                        "fields": fields.iter().map(|(id, n, t)| json!({ "field_id": id, "name": n, "type": t })).collect::<Vec<_>>() } });
    for (k, v) in extra_inputs.as_object().into_iter().flatten() {
        inputs[k] = v.clone();
    }
    let manifest = json!({ "inputs": inputs, "results": { "monthly": { "type": "table" } } });
    let staged = ws.stage_asset(&alice(), SOURCE.as_bytes()).unwrap();
    let program = json!({ "language": "js", "api_version": aiworkspace_core::wish::PROGRAM_API_VERSION, "source": staged["object_id"],
                          "digest": aiworkspace_core::wish::macro_digest(SOURCE, &manifest), "manifest": manifest, "produces": ["monthly"] });
    let rev = ws.read(&alice(), "wish-q3", None).unwrap()["content"]["key_revs"]["program"].as_u64().unwrap_or(0);
    ok(ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [{ "key": "program", "value": program, "expect": { "rev": rev } }] }]));
    manifest
}

fn field_rev(ws: &Workspace, field: &str) -> u64 {
    ws.read(&alice(), "orders", Some(&json!({ "kind": "table_field", "field_id": field }))).unwrap()["content"]["def_rev"].as_u64().unwrap()
}

fn read_json(path: std::path::PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

/// AI Native 第一期 §2.2, §2.6: running a stored macro materializes only the inputs and fields its manifest
/// declares, under the names they had when it was coded (a field renamed since keeps its coding name), and hands it
/// no map of the Workspace: no `WORKSPACE.md`, no handles, no data or Block tree.
#[test]
fn a_run_materializes_only_what_the_macro_declared() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "宏", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "prompt": "p", "context_prompt": "按月汇总 `sales`",
        "checks": [], "blockers": [], "warnings": [], "output_contract": { "results": [{ "name": "monthly", "type": "table", "key": ["月份"] }] } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } },
        { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales", "selector": { "kind": "table_view", "cell_id": "blk-orders" } },
                                     { "entity_id": "rules", "name": "rules" }, { "entity_id": "note-1", "name": "note" }], "expect": { "rev": 0 } }] }]));
    // coded reading 月份 and 销售额 of sales, and the rules document; the note is bound but not declared
    let manifest = store_macro(&mut ws, &[("month", "月份", "date"), ("amount", "销售额", "decimal")], json!({ "rules": { "entity_id": "rules", "type_id": "buckyos.richtext" } }));
    // since then 销售额 was renamed: no reason to code again
    let rev = field_rev(&ws, "amount");
    ok(&mut ws, &alice(), json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "amount", "changes": { "name": "金额" }, "expect": { "rev": rev } }]));
    let f = ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0].clone();
    assert_eq!((f["needs_analysis"].as_bool(), &f["needs_coding"]), (Some(false), &Value::Null), "{f}");

    let work = tempfile::tempdir().unwrap();
    let run = new_run(&mut ws, "run");
    let st = stage(&ws, StageKind::Run, &run, work.path());
    let dir = work.path().join(&run);
    // only the declared inputs
    assert_eq!(st.inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["sales", "rules"]);
    let inputs = read_json(dir.join("context/inputs.json"));
    assert_eq!(inputs.as_object().unwrap().keys().collect::<Vec<_>>(), ["rules", "sales"], "{inputs}");
    assert!(!dir.join("context/entities/note-1").exists(), "an undeclared input is not materialized");
    assert!(dir.join("context/entities/rules/content.md").exists());
    // only the declared fields, under their coding names, the view still deciding the rows
    let schema = read_json(dir.join("context/entities/orders/schema.json"));
    let names: Vec<(&str, &str)> = schema["fields"].as_array().unwrap().iter().map(|f| (f["field_id"].as_str().unwrap(), f["name"].as_str().unwrap())).collect();
    assert_eq!(names, [("month", "月份"), ("amount", "销售额")], "{schema}");
    let rows: Vec<Value> = std::fs::read_to_string(dir.join("context/entities/orders/rows.jsonl")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(rows.len(), 22, "the 华东 rows of the view");
    assert!(rows.iter().all(|r| r["v"].as_object().unwrap().keys().all(|k| k == "month" || k == "amount")), "{}", rows[0]);
    assert_eq!(rows[0]["v"]["month"], "2026-09-01", "in the view's order");
    let info = &st.input_infos().unwrap()["sales"];
    assert_eq!(info.fields, vec![("month".to_string(), "月份".to_string(), "date".to_string()), ("amount".to_string(), "销售额".to_string(), "decimal".to_string())]);
    // no map of the Workspace for a macro
    for gone in ["WORKSPACE.md", "handles.json", "context/data-tree.json", "context/block-tree.json"] {
        assert!(!dir.join(gone).exists(), "{gone}");
    }
    assert_eq!(read_json(dir.join("context/manifest.json"))["stage"], "run");
    // what it read: the declared fields' versions, not the others'
    let rs = st.evidence().unwrap()["read_set"].as_array().unwrap().clone();
    let read = |sel: &str, field: &str| rs.iter().any(|r| r["entity_id"] == "orders" && r["selector"]["kind"] == sel && r["selector"]["field_id"] == field);
    assert!(read("table_field_values", "month") && read("table_field_values", "amount") && read("table_field_type", "amount"), "{rs:?}");
    assert!(!read("table_field_values", "customer") && !read("table_field_values", "status"), "{rs:?}");
    assert!(rs.iter().any(|r| r["entity_id"] == "rules") && !rs.iter().any(|r| r["entity_id"] == "note-1"), "{rs:?}");
    // the macro's own SQL sees the same: the declared columns under their coding names, the view's rows
    let mut st = st;
    let cols: std::collections::BTreeMap<String, Vec<(String, String)>> = manifest["inputs"].as_object().unwrap().iter()
        .map(|(n, d)| (n.clone(), d["fields"].as_array().into_iter().flatten().map(|f| (f["field_id"].as_str().unwrap().to_string(), f["name"].as_str().unwrap().to_string())).collect()))
        .collect();
    let q = st.macro_sql("SELECT \"月份\", count(*), sum(\"销售额\") FROM sales GROUP BY 1 ORDER BY 1", None, &cols).unwrap();
    assert_eq!(q["columns"], json!(["月份", "count(*)", "sum(\"销售额\")"]));
    assert_eq!(q["rows"].as_array().unwrap().iter().map(|r| r[1].as_u64().unwrap()).sum::<u64>(), 22);
    drop(st);

    // coding sees everything under the names of now, with the map
    let work2 = tempfile::tempdir().unwrap();
    let code = new_run(&mut ws, "code");
    let st = stage(&ws, StageKind::Code, &code, work2.path());
    let dir = work2.path().join(&code);
    assert_eq!(st.inputs.len(), 3);
    let schema = read_json(dir.join("context/entities/orders/schema.json"));
    assert_eq!(schema["fields"].as_array().unwrap().len(), 5);
    assert!(schema["fields"].as_array().unwrap().iter().any(|f| f["field_id"] == "amount" && f["name"] == "金额"), "{schema}");
    assert!(dir.join("WORKSPACE.md").exists() && dir.join("context/entities/note-1").exists());
    assert!(!dir.join("context/data-tree.json").exists() && !dir.join("context/block-tree.json").exists());
    let (input, fields) = st.input_fields("sales").unwrap().unwrap();
    assert_eq!(input.entity_id, "orders");
    assert!(fields.contains(&("amount".to_string(), "金额".to_string(), "decimal".to_string())), "{fields:?}");
    assert_eq!(st.input_fields("rules").unwrap().unwrap().1, vec![], "not a table: no fields");
    assert!(st.input_fields("nope").unwrap().is_none());
    drop(st);

    // a declared field deleted, or bound to another table: the macro must be coded again before it runs
    let rev = field_rev(&ws, "amount");
    ok(&mut ws, &alice(), json!([{ "op": "table.delete_field", "source_id": "orders", "field_id": "amount", "expect": { "rev": rev } }]));
    let f = ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0].clone();
    assert_eq!(f["needs_coding"], json!({ "reasons": [{ "code": "field_deleted", "input": "sales", "field_id": "amount", "name": "销售额" }] }), "{f}");
    assert_eq!(main_status(&f, &json!({}))["next_action"], "code");
}

/// 第一期 §2.5: the code of an HTML result belongs to the macro version. Coding writes the Block definition; running
/// the stored macro never rewrites it (only the data it is bound to moves), and code other than the stored digest is
/// caught before planning.
#[test]
fn html_code_is_fixed_when_coding() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "页面", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "prompt": "p", "context_prompt": "按月汇总 `sales` 并画成页面",
        "checks": [], "blockers": [], "warnings": [], "output_contract": { "results": [
            { "name": "monthly", "type": "table", "key": ["月份"] }, { "name": "page", "type": "html" }] } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } },
        { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales" }], "expect": { "rev": 0 } }] }]));
    let page = |js: &str| json!({ "name": "page", "type": "html", "html": "<div id=a></div>", "css": "div{}", "js": js, "bindings": { "data": "result:monthly" } });
    let output = |rows: &[(&str, f64, u64)], js: &str| {
        let mut out = monthly_output(rows, 0.0);
        out["checks"] = json!([]);
        out["results"].as_array_mut().unwrap().push(page(js));
        out
    };
    let def_html = |ws: &Workspace| -> (Value, u64) {
        let b = bindings(ws);
        let def = ws.read(&alice(), b["page"]["entity_id"].as_str().unwrap(), None).unwrap()["content"]["payload"].clone();
        (def["html"].clone(), def["version"].as_u64().unwrap())
    };
    // coding: the code digest goes into the manifest, the definition is written
    let work = tempfile::tempdir().unwrap();
    let e1 = new_run(&mut ws, "code");
    let st = stage(&ws, StageKind::Code, &e1, work.path());
    let mut c = Collector { contract: analysis_contract(&ws), checks_def: vec![], workdir: work.path().join(&e1), inputs: st.input_infos().unwrap(), ..Default::default() };
    let mut out = output(&[("2026-07-01", 1.0, 1)], "render(1)");
    out["declare"] = json!({ "inputs": { "sales": ["月份"] } });
    let run = program_output(&c, &out, SOURCE, "").unwrap();
    let manifest = run.manifest.clone().unwrap();
    let digest = html_code_digest(&json!({ "html": "<div id=a></div>", "css": "div{}", "js": "render(1)" }));
    assert_eq!(manifest["results"]["page"], json!({ "type": "html", "code_digest": digest }));
    c.program = Some(run);
    let cand = build_candidate(&mut ws, &c, &st, &e1, "code");
    drop(st);
    let plan = ws.wish_plan(&alice(), &PlanRequest { run_id: &e1, candidate: &cand, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(plan.ready, "{}", plan.summary);
    apply(&mut ws, &e1, &plan);
    let (html, version) = def_html(&ws);
    assert_eq!((html["js"].as_str(), version), (Some("render(1)"), 1));

    // running with the same code: the data moves, the definition is not touched
    let run_with = |ws: &mut Workspace, js: &str, rows: &[(&str, f64, u64)]| {
        let work = tempfile::tempdir().unwrap();
        let r = new_run(ws, "run");
        let st = stage(ws, StageKind::Run, &r, work.path());
        let mut c = Collector { contract: analysis_contract(ws), checks_def: vec![], workdir: work.path().join(&r), inputs: st.input_infos().unwrap(),
                                prev_bindings: bindings(ws), stored_manifest: Some(manifest.clone()), ..Default::default() };
        let run = program_output(&c, &output(rows, js), SOURCE, "").unwrap();
        let changed = code_changed(&manifest, &run.results);
        c.program = Some(run);
        let cand = build_candidate(ws, &c, &st, &r, "run");
        drop(st);
        (r, cand, changed)
    };
    let (r2, cand2, changed) = run_with(&mut ws, "render(1)", &[("2026-07-01", 2.0, 1), ("2026-08-01", 3.0, 1)]);
    assert!(changed.is_empty());
    let plan2 = ws.wish_plan(&alice(), &PlanRequest { run_id: &r2, candidate: &cand2, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(plan2.ready, "{}", plan2.summary);
    let def_id = bindings(&ws)["page"]["entity_id"].as_str().unwrap().to_string();
    // its dependency record is re-recorded (`entity.set_derived`); its keys, the code among them, are not written
    let writes_def = |ops: &[Value]| ops.iter().any(|o| o["entity_id"] == json!(def_id) && o["op"] != "entity.set_derived");
    assert!(plan2.operations.iter().any(|o| o["entity_id"] == json!(def_id) && o["op"] == "entity.set_derived"));
    assert!(!writes_def(&plan2.operations), "the definition is not written: {:?}", plan2.operations);
    apply(&mut ws, &r2, &plan2);
    assert_eq!(def_html(&ws), (html.clone(), 1));
    assert_eq!(ids(&ws, bindings(&ws)["monthly"]["entity_id"].as_str().unwrap()).len(), 2, "the bound data moved");
    // other code: caught (MACRO_CODE_CHANGED), and even a plan made from it leaves the definition alone
    let (r3, cand3, changed) = run_with(&mut ws, "render(2)", &[("2026-07-01", 2.0, 1)]);
    assert_eq!(changed, ["page"]);
    let plan3 = ws.wish_plan(&alice(), &PlanRequest { run_id: &r3, candidate: &cand3, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(!writes_def(&plan3.operations), "running never rewrites the code: {:?}", plan3.operations);
}

/// Every run writes the macro's text results again (no result is kept as "written directly" any more), so a run
/// whose text shares no block with the last one must apply too. It does not yet: `richtext::diff_blocks` emits the
/// deletions before the insertions, and deleting every block of a document is refused on its own ("doc: content
/// does not match schema", the document needs one block), so the application is rejected.
#[test]
fn a_run_may_replace_every_block_of_a_text_result() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "文字", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "prompt": "p", "context_prompt": "解读 `sales`",
        "checks": [], "blockers": [], "warnings": [], "output_contract": { "results": [{ "name": "commentary", "type": "richtext" }] } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } }, { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales" }], "expect": { "rev": 0 } }] }]));
    let pass = |ws: &mut Workspace, kind: StageKind, text: &str| {
        let work = tempfile::tempdir().unwrap();
        let r = new_run(ws, kind.as_str());
        let st = stage(ws, kind, &r, work.path());
        let mut out = with_commentary(json!({ "results": [], "facts": {}, "checks": [] }), text);
        if kind == StageKind::Code {
            out["declare"] = json!({ "inputs": { "sales": ["月份"] } });
        }
        let mut c = Collector { contract: analysis_contract(ws), checks_def: vec![], workdir: work.path().join(&r), inputs: st.input_infos().unwrap(),
                                prev_bindings: bindings(ws), stored_manifest: stored_program(ws).get("manifest").cloned().filter(Value::is_object), ..Default::default() };
        c.program = Some(program_output(&c, &out, SOURCE, "").unwrap());
        let cand = build_candidate(ws, &c, &st, &r, kind.as_str());
        drop(st);
        let plan = ws.wish_plan(&alice(), &PlanRequest { run_id: &r, candidate: &cand, choices: &json!({}), location: &json!({}) }).unwrap();
        assert!(plan.ready, "{}", plan.summary);
        apply(ws, &r, &plan);
    };
    pass(&mut ws, StageKind::Code, "## 解读\n\n合计 3,301.5。");
    // the heading is gone and the paragraph is new: no block survives
    pass(&mut ws, StageKind::Run, "合计 3,351。");
    let commentary = bindings(&ws)["commentary"]["entity_id"].as_str().unwrap().to_string();
    assert!(ws.read_as(&alice(), &json!({ "entity_id": commentary, "as": "body" })).unwrap()["text"].as_str().unwrap().contains("3,351"));
}

/// A table-view input's rows are decided by the view's filter. Running a stored macro materializes only its declared
/// fields, but the fields the filter (and sort) read decide its rows all the same, so they belong to the run's read
/// set, as they do while coding. They are missing: `materialize_entity` looks for the deciding fields among the
/// declared ones only, so a record moving into the view after a run does not make the wish stale.
#[test]
fn a_run_reads_the_fields_its_view_filters_on() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "视图", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "prompt": "p", "context_prompt": "按月汇总 `sales`",
        "checks": [], "blockers": [], "warnings": [], "output_contract": { "results": [{ "name": "monthly", "type": "table", "key": ["月份"] }] } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } },
        { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales", "selector": { "kind": "table_view", "cell_id": "blk-orders" } }], "expect": { "rev": 0 } }] }]));
    // the view keeps 地区 = 华东; the macro reads 月份 and 销售额 only
    let manifest = store_macro(&mut ws, &[("month", "月份", "date"), ("amount", "销售额", "decimal")], json!({}));
    let work = tempfile::tempdir().unwrap();
    let run = new_run(&mut ws, "run");
    let st = stage(&ws, StageKind::Run, &run, work.path());
    let rs = st.evidence().unwrap()["read_set"].clone();
    assert!(rs.as_array().unwrap().iter().any(|r| r["entity_id"] == "orders" && r["selector"]["kind"] == "table_field_values" && r["selector"]["field_id"] == "region"), "{rs}");
    let mut c = Collector { contract: analysis_contract(&ws), checks_def: vec![], workdir: work.path().join(&run), inputs: st.input_infos().unwrap(),
                            stored_manifest: Some(manifest), ..Default::default() };
    let mut out = monthly_output(&[("2026-07-01", 1.0, 1)], 1.0);
    out["checks"] = json!([]);
    c.program = Some(program_output(&c, &out, SOURCE, "").unwrap());
    let cand = build_candidate(&mut ws, &c, &st, &run, "run");
    drop(st);
    let plan = ws.wish_plan(&alice(), &PlanRequest { run_id: &run, candidate: &cand, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(plan.ready, "{}", plan.summary);
    apply(&mut ws, &run, &plan);
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "current");
    // o0 (华南) moves to 华东: the view now shows one more row
    let rev = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o0", "field_id": "region" }))).unwrap()["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o0", "field_id": "region", "value": "east", "expect": { "rev": rev } }] }]));
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "stale");
}
