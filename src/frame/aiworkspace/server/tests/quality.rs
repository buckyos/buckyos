//! 许愿格质量任务集（详细设计 §15）在可控模型返回下的机械判定，以及若干语义场景（§16.2）。
//! The scripted model plays a competent model; what is judged is what the host guarantees: the map
//! carries what is needed to find the right data, bindings are real ids and ranges, numbers come
//! from programs, results are validated on the spot, identities stay stable, re-runs need no model,
//! and nothing is written before the user applies.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use wishkit::*;

fn facts(t: &Turn) -> Value {
    for r in t.results.iter().rev() {
        if let Some(i) = r.find("facts：") {
            let rest = &r[i + "facts：".len()..];
            if let Ok(v) = serde_json::from_str::<Value>(&rest[..rest.find('\n').unwrap_or(rest.len())]) {
                return v;
            }
        }
    }
    json!({})
}

/// A competent execution: (re)write the program when asked, run it, write the direct results
/// (`{fact}` placeholders filled from the program's facts), check, finish.
struct Flow {
    program: Option<&'static str>,
    direct: Vec<Value>,
    finish: Value,
}

fn exec(flow: Flow) -> Script {
    let flow = Arc::new(flow);
    Box::new(move |t: &Turn| {
        let mut steps: Vec<String> = Vec::new();
        if flow.program.is_some() {
            steps.push("write".into());
        }
        steps.push("run".into());
        for i in 0..flow.direct.len() {
            steps.push(format!("put{i}"));
        }
        steps.extend(["check".into(), "finish".into(), "done".into()]);
        let step = steps.get(t.results.len()).cloned().unwrap_or_else(|| "done".into());
        match step.as_str() {
            "write" => call("write_file", json!({ "path": "program/main.js", "content": flow.program.unwrap() })),
            "run" => call("run_program", json!({})),
            "check" => call("check_results", json!({})),
            "finish" => call("finish", flow.finish.clone()),
            s if s.starts_with("put") => {
                let mut d = flow.direct[s[3..].parse::<usize>().unwrap()].clone();
                if let Some(md) = d["markdown"].as_str() {
                    let mut md = md.to_string();
                    if let Some(f) = facts(t).as_object() {
                        for (k, v) in f {
                            md = md.replace(&format!("{{{k}}}"), &v.to_string());
                        }
                    }
                    d["markdown"] = json!(md);
                }
                call("put_result", d)
            }
            _ => text("完成。"),
        }
    })
}

fn submit(analysis: Value) -> Script {
    let a = Arc::new(analysis);
    Box::new(move |t: &Turn| {
        if t.results.is_empty() {
            // the handles of this run come from the map: `<<needle>>` → the handle of the line holding it
            let mut text_a = a.to_string();
            while let Some(i) = text_a.find("<<") {
                let j = text_a[i..].find(">>").unwrap() + i;
                let needle = text_a[i + 2..j].to_string();
                text_a.replace_range(i..j + 2, &handle(&t.user, &needle));
            }
            return call("submit_analysis", json!({ "analysis": serde_json::from_str::<Value>(&text_a).unwrap() }));
        }
        if t.results.last().is_some_and(|r| r.contains("未被接受")) {
            return text("放弃。");
        }
        text("完成。")
    })
}

async fn base(srv: &mut Srv) {
    let ws = srv.rpc("ws.create", json!({ "title": "质量" })).await;
    srv.ws = ws["workspace_id"].as_str().unwrap().to_string();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "复盘" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "复盘",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "复盘", "content_folder_id": "sf-content" } },
        { "op": "entity.create", "entity_id": "f-sales", "type_id": "buckyos.container", "parent_id": "data", "order_key": "a", "name": "销售", "payload": { "kind": "folder", "title": "销售" } },
        { "op": "entity.create", "entity_id": "f-old", "type_id": "buckyos.container", "parent_id": "data", "order_key": "b", "name": "归档", "payload": { "kind": "folder", "title": "归档" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "f-sales", "order_key": "a", "name": "订单",
          "payload": { "fields": [
            { "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "customer", "name": "客户", "type": "text" },
            { "field_id": "region", "name": "地区", "type": "select", "options": [{ "option_id": "east", "label": "华东" }, { "option_id": "south", "label": "华南" }] },
            { "field_id": "amount", "name": "销售额", "type": "decimal", "scale": 2 },
            { "field_id": "status", "name": "状态", "type": "select", "options": [{ "option_id": "done", "label": "已结算" }, { "option_id": "open", "label": "未结算" }] } ] } },
        { "op": "entity.create", "entity_id": "orders-old", "type_id": "buckyos.table-source", "parent_id": "f-old", "order_key": "a", "name": "订单",
          "payload": { "fields": [{ "field_id": "x", "name": "月份", "type": "text" }] } },
        { "op": "entity.create", "entity_id": "salary", "type_id": "buckyos.table-source", "parent_id": "f-sales", "order_key": "c", "name": "工资",
          "payload": { "fields": [{ "field_id": "who", "name": "姓名", "type": "text" }, { "field_id": "pay", "name": "工资", "type": "number" }] } },
    ])).await;
    let rows: Vec<Value> = (0..40).map(|i| json!({ "record_id": format!("o{i}"), "values": {
        "month": (["2026-07-01", "2026-08-01", "2026-09-01"])[i % 3], "customer": format!("客户{}", i % 9),
        "region": if i % 4 == 0 { "south" } else { "east" }, "amount": format!("{}.25", 100 + i * 7), "status": if i % 10 == 9 { "open" } else { "done" } } })).collect();
    srv.commit(json!([{ "op": "table.insert_records", "source_id": "orders", "records": rows },
        { "op": "table.insert_records", "source_id": "salary", "records": [
            { "record_id": "s1", "values": { "who": "甲", "pay": 10000 } }, { "record_id": "s2", "values": { "who": "乙", "pay": 12000 } }, { "record_id": "s3", "values": { "who": "丙", "pay": 8000 } }] }])).await;
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "w", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w", "name": "分析",
          "payload": { "title": "分析", "prompt": "分析左边那张只显示华东的表", "executor": "xllm", "output": { "container_id": "sf-content", "surface_id": "sf", "name": "分析结果" } } },
        { "op": "entity.create", "entity_id": "note-1", "type_id": "buckyos.annotation", "parent_id": "sf-content", "order_key": "n",
          "payload": { "kind": "note", "body": "统计时排除 状态=未结算", "target": { "entity_id": "orders" } } },
        { "op": "entity.create", "entity_id": "frame", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 1500, "h": 900 },
          "payload": { "view": { "type": "frame" }, "title": "华东复盘" } },
        { "op": "entity.create", "entity_id": "v-east", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "b", "placement": { "x": 40, "y": 300, "w": 500, "h": 300 },
          "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "orders" }, "title": "华东订单", "filter": { "op": "cmp", "field_id": "region", "operator": "eq", "value": "east" } } },
        { "op": "entity.create", "entity_id": "v-all", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "c", "placement": { "x": 1100, "y": 300, "w": 300, "h": 300 },
          "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "orders" }, "title": "全部订单" } },
        { "op": "entity.create", "entity_id": "b-wish", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "d", "placement": { "x": 580, "y": 300, "w": 420, "h": 300 },
          "payload": { "view": { "type": "wish" }, "source_ref": { "entity_id": "w" } } },
    ])).await;
}

fn loc() -> Value {
    json!({ "cell_id": "b-wish", "selection": ["v-east"] })
}

async fn analyzed(srv: &Srv, analysis: Value) -> Value {
    *srv.model.analyze.lock().unwrap() = submit(analysis);
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze", "location": loc() })).await;
    assert_eq!(a["state"], "waiting_confirmation", "{a}");
    srv.apply(&a, json!({})).await;
    a
}

fn monthly_analysis() -> Value {
    json!({ "status": "ready", "context_prompt": "读取 <<表格视图「华东订单」>> 全部记录，排除 状态=未结算，按月汇总销售额；写解读，数字取自 facts",
        "inputs": [{ "name": "sales", "ref": "<<表格视图「华东订单」>>" }],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "approach": "program", "key": ["月份"],
              "views": [{ "renderer": "table" }, { "renderer": "sample.line-chart", "config": { "x": "月份", "y": "销售额" } }] },
            { "name": "commentary", "type": "richtext", "title": "解读", "approach": "direct" }] },
        "checks": [{ "id": "total", "kind": "program", "text": "合计一致" }, { "id": "months", "kind": "program", "text": "覆盖三个月" }],
        "blockers": [], "warnings": [] })
}

const MONTHLY: &str = r#"
export default async function main(aiws) {
  const rows = aiws.input('sales').rows({ fields: ['月份', '销售额', '状态'] })
  const settled = rows.filter((r) => r['状态'] !== '未结算')
  const sum = (xs) => Math.round(xs.reduce((s, x) => s + (x ?? 0), 0) * 100) / 100
  const by = new Map()
  for (const r of settled) by.set(r['月份'], [...(by.get(r['月份']) ?? []), r])
  const monthly = [...by].sort(([a], [b]) => a.localeCompare(b)).map(([月份, xs]) => ({ 月份, 销售额: sum(xs.map((r) => r['销售额'])), 订单数: xs.length }))
  aiws.result.table('monthly', monthly, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal', 订单数: 'number' } })
  const total = sum(settled.map((r) => r['销售额']))
  aiws.facts({ total, months: monthly.length })
  aiws.check('total', Math.abs(sum(monthly.map((m) => m['销售额'])) - total) < 0.01)
  aiws.check('months', monthly.length === 3)
}
"#;

/// Q01 指代 + Q04 口径 + Q06 数字可追溯 + Q07 图表视图.
#[test]
fn q01_q04_q06_q07_reference_rules_numbers_views() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        // the map tells the model where things are and what the note says
        *srv.model.analyze.lock().unwrap() = Box::new(|t: &Turn| {
            let line = t.user.lines().find(|l| l.contains("表格视图「华东订单」")).unwrap_or("");
            assert!(line.contains("左侧相邻") && line.contains("地区=华东"), "{line}");
            assert!(t.user.contains("触发时选中：") && t.user.contains("统计时排除 状态=未结算"), "{}", t.user);
            text("看过了。")
        });
        let probe = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze", "location": loc() })).await;
        assert_eq!(probe["state"], "failed", "no analysis submitted: the run fails, nothing is written");
        // Q07: a renderer outside the catalog is refused on the spot
        let mut bad = monthly_analysis();
        bad["output_contract"]["results"][0]["views"][1]["renderer"] = json!("acme.pie");
        *srv.model.analyze.lock().unwrap() = submit(bad);
        let r = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze", "location": loc() })).await;
        assert_eq!(r["state"], "failed");
        analyzed(&srv, monthly_analysis()).await;
        let w = srv.wish_of("w").await;
        assert_eq!(w["inputs"][0]["selector"], json!({ "kind": "table_view", "cell_id": "v-east" }), "Q01: the view the user sees");
        // Q06: a number nobody computed is reported; the facts are cited
        *srv.model.execute.lock().unwrap() = exec(Flow {
            program: Some(MONTHLY),
            direct: vec![json!({ "name": "commentary", "type": "text", "markdown": "已结算销售额合计 {total}，共 {months} 个月，约 98,765 元来自老客户。" })],
            finish: json!({ "summary": "完成", "review_notes": [] }),
        });
        let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        assert_eq!(e["candidate"]["uncited_numbers"], json!([{ "result": "commentary", "numbers": ["98,765"] }]));
        // Q04: the rule was applied by the program (no unsettled order counted)
        let rows = e["candidate"]["results"][0]["table"]["rows"].as_array().unwrap().clone();
        let count: u64 = rows.iter().map(|r| r["订单数"].as_u64().unwrap()).sum();
        assert_eq!(count, 30 - 4, "30 east rows, 4 of them unsettled");
        srv.apply(&e, json!({})).await;
        // Q07: the chart Block stores field ids
        let w = srv.wish_of("w").await;
        let b = &w["last_run"]["result_bindings"]["results"]["monthly"];
        let chart = b["cells"].as_array().unwrap().iter().find(|c| c.as_str().unwrap().ends_with("v1")).unwrap().clone();
        let chart = srv.rpc("doc.read", json!({ "entity_id": chart })).await;
        assert_eq!(chart["content"]["payload"]["view"]["type"], "sample.line-chart");
        assert_eq!(chart["content"]["payload"]["config"], json!({ "x": b["fields"]["月份"], "y": b["fields"]["销售额"] }));
        // W18: a wish bound to a table view travels: the package stages the view's Surface after the wish
        let fork = srv.rpc("ws.fork", json!({ "title": "副本" })).await;
        assert_eq!(fork["ok"], true, "{fork}");
        let copy = srv.rpc("doc.read", json!({ "workspace_id": fork["workspace_id"], "entity_id": "w" })).await;
        assert_eq!(copy["content"]["payload"]["inputs"][0]["selector"], json!({ "kind": "table_view", "cell_id": "v-east" }));
    });
}

/// Q02 重名、缺失、无关数据: candidates instead of a guess; execution is refused.
#[test]
fn q02_ambiguity_is_a_blocker() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        *srv.model.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
            0 => call("ws_find", json!({ "text": "订单" })),
            1 => {
                // two tables are called 订单: ask, do not pick
                let hits: Vec<String> = t.results[0].lines().filter(|l| l.contains("表「订单」")).map(|l| l.split_whitespace().nth(1).unwrap().to_string()).collect();
                assert_eq!(hits.len(), 2, "{}", t.results[0]);
                call("submit_analysis", json!({ "analysis": { "status": "needs_input", "context_prompt": "待确认", "inputs": [],
                    "output_contract": { "results": [] }, "checks": [], "warnings": [],
                    "blockers": [{ "code": "AMBIGUOUS", "message": "有两张名为「订单」的表", "input_label": "订单", "candidates": hits }] } }))
            }
            _ => text("需要用户选择。"),
        });
        let a = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze", "location": loc() })).await;
        assert_eq!(a["state"], "waiting_confirmation", "{a}");
        let cands = a["candidate"]["analysis"]["blockers"][0]["candidates"].clone();
        assert_eq!(cands, json!(["orders", "orders-old"]), "real ids");
        srv.apply(&a, json!({})).await;
        let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e["state"], "failed");
        assert_eq!(e["error"]["sub_code"], "NEEDS_INPUT", "{e}");
        // a handle that does not exist, or a pure UI Block, is never bound
        *srv.model.analyze.lock().unwrap() = submit(json!({ "status": "ready", "context_prompt": "x", "inputs": [{ "name": "a", "ref": "@T99" }, { "name": "b", "ref": "<<框「华东复盘」>>" }],
            "output_contract": { "results": [] }, "checks": [], "blockers": [], "warnings": [] }));
        let r = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze", "location": loc() })).await;
        assert_eq!(r["state"], "failed", "{r}");
    });
}

/// Q03 大表聚合: ≥ 10,000 rows through the program, exact, nothing truncated.
#[test]
fn q03_large_table_through_the_program() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        srv.commit(json!([{ "op": "entity.create", "entity_id": "big", "type_id": "buckyos.table-source", "parent_id": "f-sales", "order_key": "z", "name": "流水",
            "payload": { "fields": [{ "field_id": "g", "name": "分组", "type": "text" }, { "field_id": "v", "name": "金额", "type": "number" }] } }])).await;
        let mut expect = std::collections::BTreeMap::new();
        for chunk in 0..3 {
            let rows: Vec<Value> = (0..4000).map(|i| {
                let n = chunk * 4000 + i;
                *expect.entry(format!("g{}", n % 25)).or_insert(0i64) += n as i64;
                json!({ "record_id": format!("r{n}"), "values": { "g": format!("g{}", n % 25), "v": n } })
            }).collect();
            srv.commit(json!([{ "op": "table.insert_records", "source_id": "big", "records": rows }])).await;
        }
        analyzed(&srv, json!({ "status": "ready", "context_prompt": "按分组汇总 <<表「流水」>> 的金额", "inputs": [{ "name": "flow", "ref": "<<表「流水」>>" }],
            "output_contract": { "results": [{ "name": "groups", "type": "table", "title": "分组合计", "approach": "program", "key": ["分组"] }] },
            "checks": [{ "id": "rows", "kind": "program", "text": "读到全部 12000 行" }], "blockers": [], "warnings": [] })).await;
        *srv.model.execute.lock().unwrap() = exec(Flow { program: Some(r#"
export default async function main(aiws) {
  const rows = aiws.input('flow').rows()
  const by = new Map()
  for (const r of rows) by.set(r['分组'], (by.get(r['分组']) ?? 0) + r['金额'])
  aiws.result.table('groups', [...by].map(([分组, 合计]) => ({ 分组, 合计 })), { key: ['分组'] })
  aiws.check('rows', rows.length === 12000, rows.length)
}"#), direct: vec![], finish: json!({ "summary": "完成" }) });
        let started = std::time::Instant::now();
        let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        assert!(started.elapsed().as_secs() < 30, "took {:?}", started.elapsed());
        assert_eq!(e["candidate"]["checks"][0]["status"], "passed");
        let got: std::collections::BTreeMap<String, i64> = e["candidate"]["results"][0]["table"]["rows"].as_array().unwrap().iter()
            .map(|r| (r["分组"].as_str().unwrap().to_string(), r["合计"].as_i64().unwrap())).collect();
        assert_eq!(got, expect, "independent computation agrees");
        assert!(e["evidence"]["read_set"].as_array().unwrap().len() <= 200);
        srv.apply(&e, json!({})).await;
    });
}

/// Q05 派生列 + W22: only the wish's own column; new rows only need the program.
#[test]
fn q05_derived_column_and_rerun() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        analyzed(&srv, json!({ "status": "ready", "context_prompt": "计算 <<表「工资」>> 每人工资与平均工资的差值",
            "inputs": [{ "name": "salary", "ref": "<<表「工资」>>" }],
            "output_contract": { "results": [{ "name": "差值", "type": "table_columns", "approach": "program", "target": "salary" }] },
            "checks": [{ "id": "zero", "kind": "program", "text": "差值之和约等于 0" }], "blockers": [], "warnings": [] })).await;
        *srv.model.execute.lock().unwrap() = exec(Flow { program: Some(r#"
export default async function main(aiws) {
  const rows = aiws.input('salary').rows()
  const avg = rows.reduce((s, r) => s + r['工资'], 0) / rows.length
  const values = Object.fromEntries(rows.map((r) => [r._id, { 差值: Math.round((r['工资'] - avg) * 100) / 100 }]))
  aiws.result.columns('差值', 'salary', values, { fields: { 差值: 'decimal' } })
  aiws.check('zero', Math.abs(Object.values(values).reduce((s, v) => s + v['差值'], 0)) < 0.05)
}"#), direct: vec![], finish: json!({ "summary": "完成" }) });
        let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        srv.apply(&e, json!({})).await;
        let schema = srv.rpc("doc.read", json!({ "entity_id": "salary" })).await;
        assert_eq!(schema["content"]["fields"].as_array().unwrap().len(), 3);
        assert_eq!(srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0]["status"], "current");
        // whole-number differences: the column needs no fraction digits yet
        let fid = srv.wish_of("w").await["last_run"]["result_bindings"]["results"]["差值"]["fields"]["差值"].clone();
        let field = |schema: &Value| schema["content"]["fields"].as_array().unwrap().iter().find(|f| f["field_id"] == fid).unwrap()["scale"].clone();
        assert_eq!(field(&schema), 0);
        srv.commit(json!([{ "op": "table.insert_records", "source_id": "salary", "records": [{ "record_id": "s4", "values": { "who": "丁", "pay": 14001 } }] }])).await;
        assert_eq!(srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0]["status"], "stale");
        let calls = srv.model.calls.load(Ordering::SeqCst);
        let p = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "rerun_program" })).await;
        assert_eq!(p["state"], "waiting_confirmation", "{p}");
        assert_eq!(srv.model.calls.load(Ordering::SeqCst), calls);
        // the average is 11000.25 now: the column is widened, never rounded
        assert!(p["preview"]["summary"]["structure"].to_string().contains("widen_scale"), "{}", p["preview"]);
        srv.apply(&p, json!({})).await;
        let cell = srv.rpc("doc.read", json!({ "entity_id": "salary", "selector": { "kind": "table_cell", "record_id": "s4", "field_id": fid } })).await;
        assert_eq!(cell["content"]["value"], "3000.75");
        assert_eq!(field(&srv.rpc("doc.read", json!({ "entity_id": "salary" })).await), 2);
        assert_eq!(srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0]["status"], "current");
    });
}

/// Q09 修程序: a renamed field breaks the re-run; "let the AI fix it" restores it with a small change.
#[test]
fn q09_repair_after_rename() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        analyzed(&srv, monthly_analysis()).await;
        *srv.model.execute.lock().unwrap() = exec(Flow { program: Some(MONTHLY),
            direct: vec![json!({ "name": "commentary", "type": "text", "markdown": "合计 {total}。" })], finish: json!({ "summary": "完成" }) });
        let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        srv.apply(&e, json!({})).await;
        let f = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "amount" } })).await;
        srv.commit(json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "amount", "changes": { "name": "金额" }, "expect": { "rev": f["content"]["def_rev"] } }])).await;
        let p = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "rerun_program" })).await;
        assert_eq!(p["state"], "failed");
        assert_eq!(p["error"]["sub_code"], "PROGRAM_FAILED", "{p}");
        // the analysis basis did not change (a field name is data): the repair runs on the same analysis
        let fixed: &'static str = Box::leak(MONTHLY.replace("fields: ['月份', '销售额', '状态']", "fields: ['月份', '金额', '状态']").replace("xs.map((r) => r['销售额'])", "xs.map((r) => r['金额'])").replace("settled.map((r) => r['销售额'])", "settled.map((r) => r['金额'])").into_boxed_str());
        *srv.model.execute.lock().unwrap() = Box::new(move |t: &Turn| {
            assert!(t.user.contains("程序运行失败") && t.user.contains("修复程序"), "{}", &t.user[..400.min(t.user.len())]);
            match t.results.len() {
                0 => call("write_file", json!({ "path": "program/main.js", "content": fixed })),
                1 => call("run_program", json!({})),
                2 => call("finish", json!({ "summary": "字段改名为 金额，程序改为读取 金额" })),
                _ => text("完成"),
            }
        });
        let r = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "repair_program", "parent_run_id": p["run_id"], "location": loc() })).await;
        assert_eq!(r["state"], "waiting_confirmation", "{r}");
        assert_eq!(r["candidate"]["checks"][0]["status"], "passed");
        // the written text is carried as it was (the repair did not touch it)
        srv.apply(&r, json!({})).await;
    });
}

/// Q10 HTML 结果 + W24: a definition with `aiws` v2 and a Block bound by name.
#[test]
fn q10_html_result_with_bindings() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        let mut a = monthly_analysis();
        a["output_contract"]["results"] = json!([
            { "name": "monthly", "type": "table", "title": "月度汇总", "approach": "program", "key": ["月份"] },
            { "name": "dash", "type": "html", "title": "看板", "approach": "direct" }]);
        a["checks"] = json!([]);
        analyzed(&srv, a).await;
        *srv.model.execute.lock().unwrap() = exec(Flow { program: Some(r#"
export default async function main(aiws) {
  const rows = aiws.input('sales').rows()
  const by = new Map()
  for (const r of rows) by.set(r['月份'], (by.get(r['月份']) ?? 0) + r['销售额'])
  aiws.result.table('monthly', [...by].map(([月份, 销售额]) => ({ 月份, 销售额 })), { key: ['月份'] })
}"#), direct: vec![json!({ "name": "dash", "type": "html", "html": "<div id=n></div>", "js": "aiws.watch('monthly', async () => { document.getElementById('n').textContent = (await aiws.input('monthly').rows()).length })",
            "bindings": { "monthly": "result:monthly", "orders": "input:sales" } })], finish: json!({ "summary": "完成" }) });
        let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        srv.apply(&e, json!({})).await;
        let w = srv.wish_of("w").await;
        let dash = &w["last_run"]["result_bindings"]["results"]["dash"];
        let def = srv.rpc("doc.read", json!({ "entity_id": dash["entity_id"] })).await;
        assert_eq!(def["content"]["payload"]["html"]["api_version"], 2);
        let cell = srv.rpc("doc.read", json!({ "entity_id": dash["cells"][0] })).await;
        let monthly = w["last_run"]["result_bindings"]["results"]["monthly"]["entity_id"].clone();
        assert_eq!(cell["content"]["payload"]["bindings"]["monthly"]["entity_id"], monthly);
        assert_eq!(cell["content"]["payload"]["bindings"]["orders"]["entity_id"], "orders");
        let rel = srv.rpc("doc.relations", json!({ "entity_id": monthly })).await;
        assert!(rel["blocks"].as_array().unwrap().iter().any(|b| b["entity_id"] == dash["cells"][0]), "the binding is a reference: {rel}");
    });
}

/// W05 / W08 / W12: configuration changes during a run, invalid deliveries, two applications.
#[test]
fn w05_w08_w12_guards() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        // W05: the prompt changes while the analysis waits: its write-back is refused
        *srv.model.analyze.lock().unwrap() = submit(monthly_analysis());
        let a = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze", "location": loc() })).await;
        let rev = srv.rpc("doc.read", json!({ "entity_id": "w" })).await["content"]["key_revs"]["prompt"].clone();
        srv.commit(json!([{ "op": "entity.set_keys", "entity_id": "w", "keys": [{ "key": "prompt", "value": "换个需求", "expect": { "rev": rev } }] }])).await;
        let p = srv.rpc("proc.get", json!({ "run_id": a["run_id"], "choices": {} })).await;
        assert_eq!(p["preview"]["ready"], false);
        assert!(p["preview"]["summary"]["problems"].to_string().contains("prompt"));
        analyzed(&srv, monthly_analysis()).await;
        // W08: invalid deliveries are refused on the spot; the run corrects itself
        *srv.model.execute.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
            0 => call("put_result", json!({ "name": "monthly", "type": "text", "markdown": "| a |\n|---|\n" })),
            1 => {
                assert!(t.results[0].contains("由程序产生"), "{}", t.results[0]);
                call("put_result", json!({ "name": "commentary", "type": "file", "path": "../../../etc/passwd" }))
            }
            2 => {
                assert!(t.results[1].contains("不在 output/") || t.results[1].contains("不存在"), "{}", t.results[1]);
                call("finish", json!({ "summary": "x" }))
            }
            3 => {
                assert!(t.results[2].contains("还不能结束"), "{}", t.results[2]);
                call("write_file", json!({ "path": "program/main.js", "content": MONTHLY }))
            }
            4 => call("run_program", json!({})),
            5 => call("put_result", json!({ "name": "monthly", "type": "text", "markdown": "x" })),
            6 => {
                assert!(t.results[5].contains("名称冲突") || t.results[5].contains("由程序产生"), "{}", t.results[5]);
                call("put_result", json!({ "name": "commentary", "type": "text", "markdown": "合计见表。" }))
            }
            7 => call("finish", json!({ "summary": "完成" })),
            _ => text("完成"),
        });
        let e1 = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e1["state"], "waiting_confirmation", "{e1}");
        // W12: a second candidate from the same base; the first application wins, the second conflicts
        *srv.model.execute.lock().unwrap() = exec(Flow { program: Some(MONTHLY), direct: vec![json!({ "name": "commentary", "type": "text", "markdown": "又一版。" })], finish: json!({ "summary": "完成" }) });
        let e2 = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e2["state"], "waiting_confirmation", "{e2}");
        srv.apply(&e1, json!({})).await;
        let p2 = srv.rpc("proc.get", json!({ "run_id": e2["run_id"], "choices": {} })).await;
        assert_eq!(p2["preview"]["ready"], false, "{}", p2["preview"]);
        assert!(p2["preview"]["summary"]["problems"].to_string().contains("另一组结果"));
        // applying the stale preview it had before is a conflict, and the document keeps the first
        let r = srv.rpc("proc.apply", json!({ "run_id": e2["run_id"], "plan_digest": e2["preview"]["plan_digest"], "session_id": "s1" })).await;
        assert_eq!(r["applied"]["status"], "conflict", "{r}");
        assert_eq!(srv.rpc("proc.get", json!({ "run_id": e2["run_id"] })).await["state"], "conflict");
    });
}

/// W02 纯提示词任务 + W25 程序联网读取 + W18 导出/导入/Fork.
#[test]
fn w02_w18_w25_zero_inputs_external_data_packages() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        // W02: a pure creation task binds nothing and still runs
        analyzed(&srv, json!({ "status": "ready", "context_prompt": "写一句欢迎词，并读取一份外部状态",
            "inputs": [], "output_contract": { "results": [{ "name": "hello", "type": "record", "title": "欢迎", "approach": "program" }] },
            "checks": [], "blockers": [], "warnings": [] })).await;
        assert_eq!(srv.wish_of("w").await["inputs"], json!([]));
        // W25: the program reads from outside the Workspace
        let external = format!("{}/healthz", srv.base);
        let program: &'static str = Box::leak(format!(r#"
export default async function main(aiws) {{
  const status = await (await fetch('{external}')).text()
  aiws.result.record('hello', {{ 欢迎: '你好', 外部状态: status }})
}}"#).into_boxed_str());
        *srv.model.execute.lock().unwrap() = exec(Flow { program: Some(program), direct: vec![], finish: json!({ "summary": "完成" }) });
        let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "execute", "location": loc() })).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        assert_eq!(e["candidate"]["external_data"], json!([external]));
        srv.apply(&e, json!({})).await;
        let w = srv.wish_of("w").await;
        let hello = w["last_run"]["result_bindings"]["results"]["hello"]["entity_id"].as_str().unwrap().to_string();
        let f = srv.rpc("doc.freshness", json!({ "entity_ids": [hello] })).await;
        assert_eq!((f["items"][0]["status"].as_str(), f["items"][0]["external_data"].as_bool()), (Some("current"), Some(true)));
        // W18: a package carries the program (its source asset), the dependency records and the bindings
        let ex = srv.rpc("doc.export", json!({ "mode": "share", "self_contained": true })).await;
        let pkg = srv.http.get(format!("{}/export/{}/{}", srv.base, srv.ws, ex["export_id"].as_str().unwrap())).bearer_auth("tok-alice").send().await.unwrap().bytes().await.unwrap();
        let begin = srv.rpc("ws.begin_import", json!({})).await;
        let put = srv.http.put(format!("{}/upload/{}", srv.base, begin["upload_id"].as_str().unwrap())).bearer_auth("tok-alice").body(pkg.to_vec()).send().await.unwrap();
        assert!(put.status().is_success());
        let imported = srv.rpc("ws.import", json!({ "upload_id": begin["upload_id"], "semantics": "new" })).await;
        let ws2 = imported["workspace_id"].as_str().unwrap().to_string();
        let w2 = srv.rpc("doc.read", json!({ "workspace_id": ws2, "entity_id": "w" })).await["content"]["payload"].clone();
        assert_eq!(w2["program"]["digest"], w["program"]["digest"]);
        let src = srv.http.get(format!("{}/asset/{}/{}", srv.base, ws2, w2["program"]["source"].as_str().unwrap())).bearer_auth("tok-alice").send().await.unwrap();
        assert!(src.status().is_success(), "the program source travels with the package");
        assert!(String::from_utf8_lossy(&src.bytes().await.unwrap()).contains("aiws.result.record"));
        let f2 = srv.rpc("doc.freshness", json!({ "workspace_id": ws2, "entity_ids": ["w", hello] })).await;
        assert_eq!(f2["items"][0]["status"], "current", "{f2}");
        assert_eq!(f2["items"][1]["external_data"], true);
        // the runs are local: none travels with the package
        let runs = srv.rpc("proc.list", json!({ "workspace_id": ws2, "wish_id": "w" })).await;
        assert_eq!(runs["runs"].as_array().unwrap().len(), 0);
    });
}
