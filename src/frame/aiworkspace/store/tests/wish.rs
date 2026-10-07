//! 许愿格 v0.2 host pipeline against a real SQLite Workspace (no model, no Deno: program output is
//! given as the JSON `lib/run.js` writes): the context map and tools on a snapshot, analysis
//! validation and write-back, execution materialization and read sets, result collection, the
//! planner (identity, table merge by key, derived columns, manual edits, structure confirmation) and
//! the application with freshness afterwards.

mod common;
use aiworkspace_store::wish::context::{Stage, StageKind, StageOptions};
use aiworkspace_store::wish::plan::PlanRequest;
use aiworkspace_store::wish::results::{program_output, Collector};
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
        map_budget_chars: 16_000,
        materialize: kind != StageKind::Analyze,
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

fn stage(ws: &Workspace, kind: StageKind, run: &str, dir: &std::path::Path) -> Stage {
    let snap = ws.wish_snapshot(&alice()).unwrap();
    Stage::open(snap, "wish-q3", run, &dir.join(run), &opts(kind)).unwrap()
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
    assert!(map.contains("画布「Q3 经营」") && map.contains("框「华东复盘」"), "{map}");
    assert!(map.contains(&format!("触发时选中：{b_orders}")), "{map}");
    let line = map.lines().find(|l| l.contains(&b_orders) && l.contains("表格视图")).unwrap();
    assert!(line.contains("左侧相邻") && line.contains("同框") && line.contains("地区=华东") && line.contains("月份 降序"), "{line}");
    assert!(map.contains("value=销售额(@T1.f4)") && map.contains("by=月份(@T1.f1)"), "{map}");
    assert!(map.contains("框「华东复盘」（纯 UI 框，@W 位于其中）") && map.contains("便签「9 月数据含未结算订单"), "{map}");
    assert!(map.contains("9 月数据含未结算订单") && map.contains("季度按自然季度"), "{map}");
    assert!(map.contains(&format!("{t_orders} 表「订单」 /data/销售/订单 · 30 行 · 5 列")), "{map}");
    assert!(map.contains("地区 select（华东 73%"), "{map}");
    // the snapshot holds while the Workspace moves on
    let rev = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o1", "field_id": "amount" }))).unwrap()["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "999.00", "expect": { "rev": rev } }] }]));
    let q = st.ws_query(&json!({ "table_or_view": b_orders, "limit": 50 })).unwrap();
    assert!(q.contains("按视图") && q.contains("共 22 行") && !q.contains("999") && q.contains("110.5"), "{q}");
    let q2 = st.ws_query(&json!({ "table_or_view": t_orders, "filter": { "状态": "未结算" } })).unwrap();
    assert!(q2.contains("共 1 行") && q2.contains("客户1"), "{q2}");
    assert!(st.ws_find(&json!({ "text": "含税" })).unwrap().contains("口径说明"));
    assert!(st.ws_neighbors(&json!({ "cell": handle_of(&st, "blk-wish") })).unwrap().contains("上方相邻"));
    assert!(st.ws_profile(&json!({ "target": b_orders })).unwrap().contains("\"rows\": 30"));
    assert!(st.ws_read(&json!({ "target": "@T99" })).unwrap_err().detail.contains("unknown handle"));

    // analysis: handles bind real data; a view handle binds the view; the prompt loses its handles
    let analysis = json!({
        "status": "ready",
        "context_prompt": format!("读取 {b_orders} 的全部记录，排除状态=未结算，按 {} 的口径汇总", handle_of(&st, "blk-chart")),
        "inputs": [{ "name": "sales", "ref": b_orders }, { "name": "rules", "ref": handle_of(&st, "blk-rules") }],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "approach": "program", "key": ["月份"],
              "views": [{ "renderer": "table" }, { "renderer": "sample.bar-chart", "config": { "value": "销售额", "by": "月份" } }] },
            { "name": "commentary", "type": "richtext", "title": "解读", "approach": "direct" }] },
        "checks": [{ "id": "total", "kind": "program", "text": "合计等于输入合计" }, { "id": "cited", "kind": "review", "text": "数字可追溯" }],
        "blockers": [], "warnings": []
    });
    let bad = st.validate_analysis(&analysis).unwrap().unwrap_err();
    assert!(bad.iter().any(|p| p.contains(&handle_of(&st, "blk-chart")) && p.contains("没有绑定")), "an unbound handle in the prompt: {bad:?}");
    let mut fixed = analysis.clone();
    fixed["context_prompt"] = json!(format!("读取 {b_orders} 的全部记录，排除状态=未结算，按月汇总 {t_orders}.f4；口径见 {}", handle_of(&st, "rules")));
    let mut wrong = fixed.clone();
    wrong["output_contract"]["results"][1]["type"] = json!("table");
    wrong["output_contract"]["results"][1]["approach"] = json!("direct");
    assert!(st.validate_analysis(&wrong).unwrap().unwrap_err().iter().any(|p| p.contains("program")));
    let mut wrong = fixed.clone();
    wrong["output_contract"]["results"][0]["views"][1]["renderer"] = json!("acme.pie");
    assert!(st.validate_analysis(&wrong).unwrap().unwrap_err().iter().any(|p| p.contains("acme.pie")));
    let v = st.validate_analysis(&fixed).unwrap().unwrap();
    assert_eq!(v.inputs[0]["entity_id"], "orders");
    assert_eq!(v.inputs[0]["selector"], json!({ "kind": "table_view", "cell_id": "blk-orders" }));
    assert_eq!(v.inputs[1]["entity_id"], "rules");
    let cp = v.analysis["context_prompt"].as_str().unwrap();
    assert!(!cp.contains('@') && cp.contains("输入 `sales`") && cp.contains("输入 `rules`") && cp.contains("`sales` 的字段「销售额」"), "{cp}");
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

#[test]
fn execute_collect_plan_apply_and_refresh() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "季度", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    // analysis written as the host would
    let analysis = json!({ "schema_version": "wish.analysis.v2", "status": "ready", "prompt": "分析左边那张只显示华东的表，按月汇总销售额",
        "context_prompt": "读取输入 `sales`，按月汇总", "checks": [{ "id": "total", "kind": "program", "text": "合计一致" }], "blockers": [], "warnings": [],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "approach": "program", "key": ["月份"],
              "views": [{ "renderer": "table" }, { "renderer": "sample.bar-chart", "config": { "value": "销售额", "by": "月份" } }] },
            { "name": "commentary", "type": "richtext", "title": "解读", "approach": "direct" }], "placement": "frame:frame-1" } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } },
        { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales", "selector": { "kind": "table_view", "cell_id": "blk-orders" } }], "expect": { "rev": 0 } }] }]));
    let work = tempfile::tempdir().unwrap();
    let e1 = new_run(&mut ws, "execute");
    let mut st = stage(&ws, StageKind::Execute, &e1, work.path());
    // the view is the range: 22 east rows, newest month first, option labels and numbers
    let rows = std::fs::read_to_string(work.path().join(&e1).join("context/entities/orders/rows.jsonl")).unwrap();
    assert_eq!(rows.lines().count(), 22);
    assert!(rows.lines().next().unwrap().contains("2026-09-01") && rows.contains("\"华东\"") && rows.contains("110.5"));
    let inputs: Value = serde_json::from_str(&std::fs::read_to_string(work.path().join(&e1).join("context/inputs.json")).unwrap()).unwrap();
    assert_eq!(inputs["sales"]["selector"]["kind"], "table_view");
    // reading undeclared data during execution appends it, recorded and materialized
    let note = st.ws_read(&json!({ "target": handle_of(&st, "rules") })).unwrap();
    assert!(note.contains("执行时追加的输入 `input1`") || note.contains("追加的输入"), "{note}");
    assert!(work.path().join(&e1).join("context/entities/rules/content.md").exists());
    let ev = st.evidence().unwrap();
    let rs = ev["read_set"].as_array().unwrap();
    let has = |id: &str, kind: &str| rs.iter().any(|r| r["entity_id"] == id && r["selector"]["kind"].as_str().unwrap_or("entity") == kind);
    assert!(has("orders", "table_members") && has("orders", "table_field_values") && has("blk-orders", "doc_key") && has("rules", "entity"), "{ev}");
    assert!(rs.iter().any(|r| r["entity_id"] == "blk-orders" && r["selector"]["key"] == "filter"));
    assert_eq!(ev["appended_inputs"][0]["entity_id"], "rules");

    // collection: contract rules and the on-the-spot errors
    let mut c = Collector { contract: analysis_contract(&ws), checks_def: analysis_checks(&ws), workdir: work.path().join(&e1), inputs: st.input_infos().unwrap(), ..Default::default() };
    let direct_table = json!({ "name": "commentary", "type": "table", "rows": [] });
    assert!(c.normalize(&direct_table, "direct").unwrap_err()[0].contains("程序"));
    let program_text = json!({ "name": "monthly", "type": "text", "markdown": "x" });
    assert!(c.normalize(&program_text, "direct").unwrap_err()[0].contains("程序"));
    let extra = json!({ "name": "bonus", "type": "text", "markdown": "x" });
    assert!(c.normalize(&extra, "direct").unwrap_err()[0].contains("不在输出约定中"));
    let dup = monthly_output(&[("2026-07-01", 1.0, 1), ("2026-07-01", 2.0, 1)], 3.0);
    assert!(program_output(&c, &dup, "src", "").unwrap_err().iter().any(|e| e.contains("重复")));
    let out = monthly_output(&[("2026-07-01", 1000.5, 8), ("2026-08-01", 1100.5, 7), ("2026-09-01", 1200.5, 7)], 3301.5);
    c.program = Some(program_output(&c, &out, "export default async function main(aiws) {}", "").unwrap());
    assert!(c.blocking().iter().any(|b| b.contains("commentary")));
    let text = c.normalize(&json!({ "name": "commentary", "type": "text", "markdown": "## 解读\n\n三季度合计 3,301.5，9 月 1,200.5，环比增长 9.1%，约 1,300 单。" }), "direct").unwrap();
    c.direct.insert("commentary".into(), text);
    let uncited = c.uncited_numbers(&[]);
    assert_eq!(uncited[0]["numbers"], json!(["9.1", "1,300"]), "{uncited:?}");
    assert!(c.blocking().is_empty(), "{:?}", c.blocking());
    c.finished = Some(json!({ "summary": "完成", "assumptions": ["空值按 0"], "review_notes": [{ "id": "cited", "note": "已核对" }] }));
    let checks = c.checks();
    assert_eq!(checks.iter().find(|x| x["id"] == "total").unwrap()["status"], "passed");

    let candidate = build_candidate(&c, &st, &e1);
    drop(st);
    // preview → apply
    let plan = ws.wish_plan(&alice(), &PlanRequest { run_id: &e1, candidate: &candidate, choices: &json!({}), location: &json!({ "cell_id": "blk-wish" }) }).unwrap();
    assert!(plan.ready, "{}", plan.summary);
    let results = plan.summary["results"].as_array().unwrap();
    assert_eq!(results.iter().map(|r| r["action"].as_str().unwrap()).collect::<Vec<_>>(), ["create", "create"]);
    assert_eq!(results[0]["blocks"], 2, "two views of the same table");
    apply(&mut ws, &e1, &plan);
    let wish = ws.read(&alice(), "wish-q3", None).unwrap()["content"]["payload"].clone();
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
    let t = ws.query(&alice(), &json!({ "source_id": monthly, "limit": 10 }), &Default::default()).unwrap();
    assert_eq!(t["total"], 3);
    let chart = ws.list_children(&alice(), b["group"].as_str().unwrap(), false).unwrap();
    let chart = chart["children"].as_array().unwrap().iter().find(|c| c["entity_id"].as_str().unwrap().ends_with("v1")).unwrap().clone();
    let chart = ws.read(&alice(), chart["entity_id"].as_str().unwrap(), None).unwrap();
    assert_eq!(chart["content"]["payload"]["config"], json!({ "value": "f2", "by": "f1" }), "chart config by field ids");
    let d = ws.read(&alice(), &commentary, None).unwrap()["derived"].clone();
    assert_eq!((d["approach"].as_str(), d["result_key"].as_str()), (Some("direct"), Some("commentary")));
    let f = ws.freshness(&alice(), &["wish-q3".into(), monthly.clone()]).unwrap();
    assert_eq!(f["items"][0]["status"], "current", "{f}");
    assert_eq!(f["items"][1]["status"], "current");
    // moving a Block does not matter; changing what the view shows does
    ok(&mut ws, &alice(), json!([{ "op": "tree.place", "entity_id": "blk-chart", "placement": { "x": 600, "y": 40, "w": 420, "h": 220 } }]));
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "current");
    let rev = ws.read(&alice(), "orders", Some(&json!({ "kind": "table_cell", "record_id": "o1", "field_id": "amount" }))).unwrap()["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "150.00", "expect": { "rev": rev } }] }]));
    assert_eq!(ws.freshness(&alice(), &["wish-q3".into()]).unwrap()["items"][0]["status"], "stale");

    // a program re-run: same identities, rows merged by key, written text untouched and now stale
    let work2 = tempfile::tempdir().unwrap();
    let p2 = new_run(&mut ws, "rerun_program");
    let st2 = stage(&ws, StageKind::Program, &p2, work2.path());
    let mut c2 = Collector { contract: analysis_contract(&ws), checks_def: analysis_checks(&ws), workdir: work2.path().join(&p2), inputs: st2.input_infos().unwrap(),
                             program_only: true, prev_bindings: bindings(&ws), ..Default::default() };
    let out = monthly_output(&[("2026-10-01", 10.0, 1), ("2026-09-01", 1200.5, 7), ("2026-08-01", 1100.5, 7), ("2026-07-01", 1040.0, 8)], 3351.0);
    c2.program = Some(program_output(&c2, &out, "export default async function main(aiws) {}", "").unwrap());
    assert!(c2.blocking().is_empty(), "{:?}", c2.blocking());
    let mut cand2 = build_candidate(&c2, &st2, &p2);
    cand2["mode"] = json!("program");
    drop(st2);
    let plan2 = ws.wish_plan(&alice(), &PlanRequest { run_id: &p2, candidate: &cand2, choices: &json!({}), location: &json!({}) }).unwrap();
    assert!(plan2.ready, "{}", plan2.summary);
    let rec_ids_before: Vec<String> = ids(&ws, &monthly);
    apply(&mut ws, &p2, &plan2);
    let rec_ids_after = ids(&ws, &monthly);
    assert!(rec_ids_before.iter().all(|i| rec_ids_after.contains(i)), "record ids kept");
    assert_eq!(rec_ids_after.len(), 4);
    // the program sorts newest first now: the Block follows
    assert_eq!(view_months(&ws, &table_cell), ["2026-10-01", "2026-09-01", "2026-08-01", "2026-07-01"]);
    assert!(plan2.summary["results"].as_array().unwrap().iter().any(|r| r["name"] == "monthly" && r["reordered"] == true), "{}", plan2.summary);
    // the user sorts the Block: later runs leave its order alone
    let rev = ws.read(&alice(), &table_cell, None).unwrap()["content"]["key_revs"]["sorts"].as_u64().unwrap_or(0);
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": table_cell, "keys": [{ "key": "sorts", "value": [{ "field_id": "f2", "direction": "desc" }], "expect": { "rev": rev } }] }]));
    let f = ws.freshness(&alice(), &["wish-q3".into(), commentary.clone(), monthly.clone()]).unwrap();
    assert_eq!(f["items"][2]["status"], "current");
    assert_eq!(f["items"][1]["status"], "stale", "the text was not regenerated");
    assert_eq!(f["items"][0]["direct_stale"], true);

    // a manual edit of the table: the next application asks keep / replace / new
    let r0 = ids(&ws, &monthly)[0].clone();
    let rev = ws.read(&alice(), &monthly, Some(&json!({ "kind": "table_cell", "record_id": r0, "field_id": "f3" }))).unwrap()["content"]["rev"].as_u64().unwrap();
    ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": monthly, "values": [{ "record_id": r0, "field_id": "f3", "value": 99, "expect": { "rev": rev } }] }]));
    let work3 = tempfile::tempdir().unwrap();
    let p3 = new_run(&mut ws, "rerun_program");
    let st3 = stage(&ws, StageKind::Program, &p3, work3.path());
    let mut c3 = Collector { contract: analysis_contract(&ws), checks_def: analysis_checks(&ws), workdir: work3.path().join(&p3), inputs: st3.input_infos().unwrap(),
                             program_only: true, ..Default::default() };
    // a row disappears this time: destructive, must be confirmed
    let out = monthly_output(&[("2026-07-01", 1040.0, 8), ("2026-08-01", 1100.5, 7), ("2026-09-01", 1200.5, 7)], 3341.0);
    c3.program = Some(program_output(&c3, &out, "export default async function main(aiws) {}", "").unwrap());
    let mut cand3 = build_candidate(&c3, &st3, &p3);
    cand3["mode"] = json!("program");
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
    assert!(!again.ready && again.summary["problems"].to_string().contains("另一组结果"));
}

#[test]
fn derived_columns_are_owned_and_do_not_stale_themselves() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "工资", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    setup(&mut ws);
    let analysis = json!({ "schema_version": "wish.analysis.v2", "status": "ready", "prompt": "p", "context_prompt": "差值",
        "checks": [], "blockers": [], "warnings": [],
        "output_contract": { "results": [{ "name": "差值", "type": "table_columns", "approach": "program", "target": "sales" }] } });
    ok(&mut ws, &alice(), json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [
        { "key": "analysis", "value": analysis, "expect": { "rev": 0 } },
        { "key": "inputs", "value": [{ "entity_id": "orders", "name": "sales" }], "expect": { "rev": 0 } }] }]));
    let run = |ws: &mut Workspace, run: &str, choices: Value| -> aiworkspace_store::wish::plan::Plan {
        let run = &ws.wish_run_create(&alice(), PROGRAM_XLLM, &json!({ "wish_id": "wish-q3", "stage": "execute", "k": run }), run).unwrap().0.clone();
        let work = tempfile::tempdir().unwrap();
        let st = stage(ws, StageKind::Execute, run, work.path());
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
        let out = json!({ "results": [{ "name": "差值", "type": "columns", "input": "sales", "fields": { "差值": "decimal" }, "values": values }], "facts": {}, "checks": [] });
        c.program = Some(program_output(&c, &out, "src", "").unwrap());
        let cand = build_candidate(&c, &st, run);
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
    let q = ws.query(&alice(), &json!({ "view_id": cell, "limit": 100 }), &Default::default()).unwrap();
    q["rows"].as_array().unwrap().iter().map(|r| r["values"]["f1"].as_str().unwrap().to_string()).collect()
}
fn ids(ws: &Workspace, table: &str) -> Vec<String> {
    let q = ws.query(&alice(), &json!({ "source_id": table, "limit": 100 }), &Default::default()).unwrap();
    q["rows"].as_array().unwrap().iter().map(|r| r["record_id"].as_str().unwrap().to_string()).collect()
}

fn build_candidate(c: &Collector, st: &Stage, run: &str) -> Value {
    let mut cand = c.candidate();
    let ev = st.evidence().unwrap();
    for k in ["wish_id", "read_set", "appended_inputs", "basis"] {
        cand[k] = ev[k].clone();
    }
    cand["executor"] = json!("xllm");
    cand["run_id"] = json!(run);
    cand["executed_at"] = json!("2026-10-06T10:00:00.000Z");
    if let Some(p) = &c.program {
        // program source staged as the server does
        cand["program"] = json!({ "digest": p.digest, "object_id": Value::Null, "produces": p.results.iter().map(|r| r["name"].clone()).collect::<Vec<_>>() });
        cand["program"] = Value::Null;
    }
    cand
}

fn new_run(ws: &mut Workspace, stage: &str) -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let key = format!("t-run-{}", N.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    ws.wish_run_create(&alice(), PROGRAM_XLLM, &json!({ "wish_id": "wish-q3", "stage": stage, "k": key }), &key).unwrap().0
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
        assert!(problems.iter().any(|p| p.contains("生成环")), "{target}: {problems:?}");
    }
    let h = st.handles.get("orders").unwrap().to_string();
    assert!(st.validate_analysis(&a(&h)).unwrap().is_ok());
}

/// §8.1: a document whose rules went into the task description (`context_sources`) is part of the
/// analysis basis: changing it asks for a new analysis, and execution reads it as an input version.
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
    // execution records the source document as read
    let e = new_run(&mut ws, "execute");
    let w2 = tempfile::tempdir().unwrap();
    let st = stage(&ws, StageKind::Execute, &e, w2.path());
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
