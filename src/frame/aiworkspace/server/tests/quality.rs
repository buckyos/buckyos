//! 许愿格质量任务集（详细设计 §15）在可控模型返回下的机械判定，以及若干语义场景（§16.2）。
//! The scripted model plays a competent model; what is judged is what the host guarantees: the map
//! carries what is needed to find the right data, bindings are real ids and ranges, every result and
//! every number comes from the macro (its text from the model, given the macro's facts), the macro's
//! output is validated on the spot, identities stay stable, running needs no model, a macro that no
//! longer fits its inputs is refused before it runs and repaired by coding, and nothing is written
//! before the user applies.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use wishkit::*;

/// A competent coding round: write the macro, trial-run it, look at its results, finish.
fn coding(source: String) -> Script {
    Box::new(move |t: &Turn| match t.results.len() {
        0 => call("write_file", json!({ "path": "program/main.js", "content": source })),
        1 => call("run_program", json!({})),
        2 => call("check_results", json!({})),
        3 => call("finish", json!({ "summary": "完成" })),
        _ => text("完成。"),
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
                let needle = text_a[i + 2..j].replace("\\\"", "\"");
                text_a.replace_range(i..j + 2, &handle(&t.user, &needle));
            }
            return call("submit_analysis", json!({ "analysis": serde_json::from_str::<Value>(&text_a).unwrap() }));
        }
        if t.results.last().is_some_and(|r| r.contains("not accepted")) {
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

async fn code(srv: &Srv, extra: Value) -> Value {
    let mut params = json!({ "wish_id": "w", "stage": "code", "location": loc() });
    for (k, v) in extra.as_object().into_iter().flatten() {
        params[k] = v.clone();
    }
    srv.run("wish.xllm@1", params).await
}

async fn run(srv: &Srv) -> Value {
    srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "run", "location": loc() })).await
}

async fn fresh(srv: &Srv) -> Value {
    srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0].clone()
}

fn monthly_analysis() -> Value {
    json!({ "status": "ready", "context_prompt": "读取 <<table view \"华东订单\">> 全部记录，排除 状态=未结算，按月汇总销售额；宏让模型按 facts 写解读",
        "inputs": [{ "name": "sales", "ref": "<<table view \"华东订单\">>" }],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"],
              "views": [{ "renderer": "table" }, { "renderer": "sample.line-chart", "config": { "x": "月份", "y": "销售额" } }] },
            { "name": "commentary", "type": "richtext", "title": "解读" }] },
        "checks": [{ "id": "total", "kind": "program", "text": "合计一致" }, { "id": "months", "kind": "program", "text": "覆盖三个月" }],
        "blockers": [], "warnings": [] })
}

/// The monthly macro; `{AMOUNT}` is the name it reads the amount field under.
const MONTHLY: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '{AMOUNT}', '状态'] }, ai: { complete: { max_calls: 2 } } })
  const rows = aiws.input('sales').rows()
  const settled = rows.filter((r) => r['状态'] !== '未结算')
  const sum = (xs) => Math.round(xs.reduce((s, x) => s + (x ?? 0), 0) * 100) / 100
  const by = new Map()
  for (const r of settled) by.set(r['月份'], [...(by.get(r['月份']) ?? []), r])
  const monthly = [...by].sort(([a], [b]) => a.localeCompare(b)).map(([月份, xs]) => ({ 月份, 销售额: sum(xs.map((r) => r['{AMOUNT}'])), 订单数: xs.length }))
  aiws.result.table('monthly', monthly, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal', 订单数: 'number' } })
  const total = sum(settled.map((r) => r['{AMOUNT}']))
  aiws.facts({ total, months: monthly.length })
  aiws.check('total', Math.abs(sum(monthly.map((m) => m['销售额'])) - total) < 0.01)
  aiws.check('months', monthly.length === 3)
  const { text } = await aiws.ai.complete({ system: '只用 facts 里的数字写解读', prompt: JSON.stringify({ total, months: monthly.length }) })
  aiws.result.text('commentary', text)
}
"#;

fn monthly(amount: &str) -> String {
    MONTHLY.replace("{AMOUNT}", amount)
}

/// Q01 指代 + Q04 口径 + Q06 数字可追溯 + Q07 图表视图.
#[test]
fn q01_q04_q06_q07_reference_rules_numbers_views() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        // the map tells the model where things are and what the note says
        *srv.model.analyze.lock().unwrap() = Box::new(|t: &Turn| {
            let line = t.user.lines().find(|l| l.contains("table view \"华东订单\"")).unwrap_or("");
            assert!(line.contains("to the left, adjacent") && line.contains("地区=华东"), "{line}");
            assert!(t.user.contains("Selected when triggered: ") && t.user.contains("统计时排除 状态=未结算"), "{}", t.user);
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
        // Q06: the macro gives the model its facts; a number the model made up is reported, the facts are cited
        *srv.model.complete.lock().unwrap() = Box::new(|_, prompt| format!("解读：{prompt}；约 98,765 元来自老客户。"));
        *srv.model.execute.lock().unwrap() = coding(monthly("销售额"));
        let e = code(&srv, json!({})).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        assert_eq!(e["candidate"]["uncited_numbers"], json!([{ "result": "commentary", "numbers": ["98,765"] }]));
        assert_eq!(e["candidate"]["model_judgment"], json!(["monthly", "commentary"]), "the macro called the model");
        assert_eq!(srv.model.complete_calls.load(Ordering::SeqCst), 1);
        // Q04: the rule was applied by the macro (no unsettled order counted)
        let rows = e["candidate"]["results"][0]["table"]["rows"].as_array().unwrap().clone();
        let count: u64 = rows.iter().map(|r| r["订单数"].as_u64().unwrap()).sum();
        assert_eq!(count, 30 - 4, "30 east rows, 4 of them unsettled");
        let checks: Vec<(&str, &str)> = e["candidate"]["checks"].as_array().unwrap().iter().map(|c| (c["id"].as_str().unwrap(), c["status"].as_str().unwrap())).collect();
        assert_eq!(checks, [("total", "passed"), ("months", "passed")]);
        srv.apply(&e, json!({})).await;
        // Q07: the chart Block stores field ids
        let w = srv.wish_of("w").await;
        let b = &w["last_run"]["result_bindings"]["results"]["monthly"];
        let chart = b["cells"].as_array().unwrap().iter().find(|c| c.as_str().unwrap().ends_with("v1")).unwrap().clone();
        let chart = srv.rpc("doc.read", json!({ "entity_id": chart })).await;
        assert_eq!(chart["content"]["payload"]["view"]["type"], "sample.line-chart");
        assert_eq!(chart["content"]["payload"]["config"], json!({ "x": b["fields"]["月份"], "y": b["fields"]["销售额"] }));
        // W18: a wish bound to a table view travels: the package stages the view's Surface after the wish
        let cloned = srv.rpc("ws.clone", json!({})).await;
        assert_eq!(cloned["ok"], true, "{cloned}");
        let copy = srv.rpc("doc.read", json!({ "workspace_id": cloned["workspace_id"], "entity_id": "w" })).await;
        assert_eq!(copy["content"]["payload"]["inputs"][0]["selector"], json!({ "kind": "table_view", "cell_id": "v-east" }));
    });
}

/// Q02 重名、缺失、无关数据: candidates instead of a guess; coding and running are refused.
#[test]
fn q02_ambiguity_is_a_blocker() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        *srv.model.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
            0 => call("ws_find", json!({ "text": "订单" })),
            1 => {
                // two tables are called 订单: ask, do not pick
                let hits: Vec<String> = t.results[0].lines().filter(|l| l.contains("table \"订单\"")).map(|l| l.split_whitespace().nth(1).unwrap().to_string()).collect();
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
        let calls = srv.model.calls.load(Ordering::SeqCst);
        for stage in ["code", "run"] {
            let e = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": stage, "location": loc() })).await;
            assert_eq!((e["state"].as_str(), e["error"]["sub_code"].as_str()), (Some("refused"), Some("NEEDS_INPUT")), "{stage}: {e}");
            assert!(e["error"]["detail"].as_str().unwrap().contains("有两张名为「订单」的表"), "the blocker is named: {e}");
        }
        assert_eq!(srv.model.calls.load(Ordering::SeqCst), calls, "refused before any executor worked on it");
        // a handle that does not exist, or a pure UI Block, is never bound
        *srv.model.analyze.lock().unwrap() = submit(json!({ "status": "ready", "context_prompt": "x", "inputs": [{ "name": "a", "ref": "@T99" }, { "name": "b", "ref": "<<frame \"华东复盘\" (a pure UI frame>>" }],
            "output_contract": { "results": [] }, "checks": [], "blockers": [], "warnings": [] }));
        let r = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze", "location": loc() })).await;
        assert_eq!(r["state"], "failed", "{r}");
    });
}

/// Q03 大表聚合: ≥ 10,000 rows through the macro (SQL over the declared input), exact, nothing truncated.
#[test]
fn q03_large_table_through_the_macro() {
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
        analyzed(&srv, json!({ "status": "ready", "context_prompt": "按分组汇总 <<table \"流水\">> 的金额", "inputs": [{ "name": "flow", "ref": "<<table \"流水\">>" }],
            "output_contract": { "results": [{ "name": "groups", "type": "table", "title": "分组合计", "key": ["分组"] }] },
            "checks": [{ "id": "rows", "kind": "program", "text": "读到全部 12000 行" }], "blockers": [], "warnings": [] })).await;
        *srv.model.execute.lock().unwrap() = coding(r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { flow: ['分组', '金额'] } })
  const { rows, truncated } = await aiws.sql('SELECT "分组", sum("金额") AS "合计" FROM flow GROUP BY 1 ORDER BY 1')
  aiws.result.table('groups', rows, { key: ['分组'] })
  const n = aiws.input('flow').rows().length
  aiws.check('rows', n === 12000 && !truncated, { n, truncated })
}"#.to_string());
        let started = std::time::Instant::now();
        let e = code(&srv, json!({})).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        assert!(started.elapsed().as_secs() < 30, "took {:?}", started.elapsed());
        assert_eq!(e["candidate"]["checks"][0]["status"], "passed", "{}", e["candidate"]["checks"]);
        let got: std::collections::BTreeMap<String, i64> = e["candidate"]["results"][0]["table"]["rows"].as_array().unwrap().iter()
            .map(|r| (r["分组"].as_str().unwrap().to_string(), r["合计"].as_f64().unwrap() as i64)).collect();
        assert_eq!(got, expect, "independent computation agrees");
        assert!(e["evidence"]["read_set"].as_array().unwrap().len() <= 200);
        srv.apply(&e, json!({})).await;
    });
}

/// Q05 派生列 + W22: only the wish's own column; new rows only need the macro to run again.
#[test]
fn q05_derived_column_and_run() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        analyzed(&srv, json!({ "status": "ready", "context_prompt": "计算 <<table \"工资\">> 每人工资与平均工资的差值",
            "inputs": [{ "name": "salary", "ref": "<<table \"工资\">>" }],
            "output_contract": { "results": [{ "name": "差值", "type": "table_columns", "target": "salary" }] },
            "checks": [{ "id": "zero", "kind": "program", "text": "差值之和约等于 0" }], "blockers": [], "warnings": [] })).await;
        *srv.model.execute.lock().unwrap() = coding(r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { salary: ['工资'] } })
  const rows = aiws.input('salary').rows()
  const avg = rows.reduce((s, r) => s + r['工资'], 0) / rows.length
  const values = Object.fromEntries(rows.map((r) => [r._id, { 差值: Math.round((r['工资'] - avg) * 100) / 100 }]))
  aiws.result.columns('差值', 'salary', values, { fields: { 差值: 'decimal' } })
  aiws.check('zero', Math.abs(Object.values(values).reduce((s, v) => s + v['差值'], 0)) < 0.05)
}"#.to_string());
        let e = code(&srv, json!({})).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        srv.apply(&e, json!({})).await;
        let schema = srv.rpc("doc.read", json!({ "entity_id": "salary" })).await;
        assert_eq!(schema["content"]["fields"].as_array().unwrap().len(), 3);
        assert_eq!(fresh(&srv).await["status"], "current");
        // whole-number differences: the column needs no fraction digits yet
        let fid = srv.wish_of("w").await["last_run"]["result_bindings"]["results"]["差值"]["fields"]["差值"].clone();
        let field = |schema: &Value| schema["content"]["fields"].as_array().unwrap().iter().find(|f| f["field_id"] == fid).unwrap()["scale"].clone();
        assert_eq!(field(&schema), 0);
        srv.commit(json!([{ "op": "table.insert_records", "source_id": "salary", "records": [{ "record_id": "s4", "values": { "who": "丁", "pay": 14001 } }] }])).await;
        assert_eq!(fresh(&srv).await["status"], "stale");
        let calls = srv.model.calls.load(Ordering::SeqCst);
        let p = srv.run("wish.xllm@1", json!({ "wish_id": "w", "stage": "run" })).await;
        assert_eq!((p["state"].as_str(), p["candidate"]["mode"].as_str()), (Some("waiting_confirmation"), Some("run")), "{p}");
        assert_eq!(srv.model.calls.load(Ordering::SeqCst), calls, "no model loop");
        // the average is 11000.25 now: the column is widened, never rounded
        assert!(p["preview"]["summary"]["structure"].to_string().contains("widen_scale"), "{}", p["preview"]);
        srv.apply(&p, json!({})).await;
        let cell = srv.rpc("doc.read", json!({ "entity_id": "salary", "selector": { "kind": "table_cell", "record_id": "s4", "field_id": fid } })).await;
        assert_eq!(cell["content"]["value"], "3000.75");
        assert_eq!(field(&srv.rpc("doc.read", json!({ "entity_id": "salary" })).await), 2);
        assert_eq!(fresh(&srv).await["status"], "current");
    });
}

/// Q09 修宏: a renamed field changes nothing (the macro reads by id); a declared field that changed its type stops the
/// wish before it runs (`NEEDS_CODING`), and coding from the last run repairs the macro with a small change.
#[test]
fn q09_rename_runs_type_change_needs_coding() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        analyzed(&srv, monthly_analysis()).await;
        *srv.model.execute.lock().unwrap() = coding(monthly("销售额"));
        let e = code(&srv, json!({})).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        let total = e["candidate"]["facts"]["total"].clone();
        srv.apply(&e, json!({})).await;

        // a renamed field: no reason to code again, the run reads it under the name it was coded with
        let f = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "amount" } })).await;
        srv.commit(json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "amount", "changes": { "name": "金额" }, "expect": { "rev": f["content"]["def_rev"] } }])).await;
        assert!(fresh(&srv).await["needs_coding"].is_null());
        let calls = srv.model.calls.load(Ordering::SeqCst);
        let renamed = run(&srv).await;
        assert_eq!((renamed["state"].as_str(), renamed["candidate"]["mode"].as_str()), (Some("waiting_confirmation"), Some("run")), "{renamed}");
        assert_eq!(renamed["candidate"]["facts"]["total"], total);
        assert_eq!(renamed["candidate"]["checks"][0]["status"], "passed");
        assert_eq!(srv.model.calls.load(Ordering::SeqCst), calls, "no model loop");
        srv.apply(&renamed, json!({})).await;

        // a declared field changes its type: the run is refused before it starts, naming the field
        let f = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "status" } })).await;
        srv.commit(json!([{ "op": "table.migrate_field", "source_id": "orders", "field_id": "status", "expect": { "rev": f["content"]["def_rev"] }, "to": { "type": "text" } }])).await;
        let fr = fresh(&srv).await;
        let reason = &fr["needs_coding"]["reasons"][0];
        assert_eq!((reason["code"].as_str(), reason["field_id"].as_str(), reason["to"].as_str()), (Some("field_type_changed"), Some("status"), Some("text")), "{fr}");
        let refused = run(&srv).await;
        assert_eq!((refused["state"].as_str(), refused["error"]["sub_code"].as_str()), (Some("refused"), Some("NEEDS_CODING")), "{refused}");
        assert!(refused["error"]["detail"].as_str().unwrap().contains("状态"), "the field is named: {refused}");
        assert_eq!(srv.model.calls.load(Ordering::SeqCst), calls);

        // the repair: coding from the last run starts from its macro, whose declaration no longer resolves; the model
        // reads the error and changes the field name it reads the amount under
        let fixed = monthly("金额");
        *srv.model.execute.lock().unwrap() = Box::new(move |t: &Turn| {
            assert!(t.user.contains("## Current macro") && t.user.contains("'销售额', '状态'"), "the last run's macro: {}", &t.user[..600.min(t.user.len())]);
            match t.results.len() {
                0 => call("run_program", json!({})),
                1 => {
                    assert!(t.results[0].contains("has no field \"销售额\""), "{}", t.results[0]);
                    call("write_file", json!({ "path": "program/main.js", "content": fixed }))
                }
                2 => call("run_program", json!({})),
                3 => call("finish", json!({ "summary": "字段 销售额 改名为 金额，状态 改为文本：宏改为读取 金额，并按新类型重新声明" })),
                _ => text("完成"),
            }
        });
        let r = code(&srv, json!({ "parent_run_id": renamed["run_id"] })).await;
        assert_eq!(r["state"], "waiting_confirmation", "{r}");
        assert_eq!(r["candidate"]["checks"][0]["status"], "passed");
        assert_eq!(r["candidate"]["facts"]["total"], total, "the same data, the same result");
        let declared: Vec<(&str, &str, &str)> = r["candidate"]["program"]["manifest"]["inputs"]["sales"]["fields"].as_array().unwrap().iter()
            .map(|f| (f["field_id"].as_str().unwrap(), f["name"].as_str().unwrap(), f["type"].as_str().unwrap())).collect();
        assert_eq!(declared, [("month", "月份", "date"), ("amount", "金额", "decimal"), ("status", "状态", "text")]);
        srv.apply(&r, json!({})).await;
        let fr = fresh(&srv).await;
        assert_eq!((fr["status"].as_str(), fr["needs_coding"].is_null()), (Some("current"), true), "{fr}");
        let again = run(&srv).await;
        assert_eq!((again["state"].as_str(), &again["candidate"]["facts"]["total"]), (Some("waiting_confirmation"), &total), "{again}");
    });
}

const DASH_BAD: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额'] } })
  const { rows } = await aiws.sql('SELECT "月份", round(sum("销售额"), 2) AS "销售额" FROM sales GROUP BY 1 ORDER BY 1')
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal' } })
  const total = Math.round(rows.reduce((s, r) => s + r['销售额'], 0) * 100) / 100
  aiws.result.html('dash', { html: `<div id=n>合计 ${total}</div>`, bindings: { monthly: 'result:monthly', orders: 'input:sales' } })
}"#;

const DASH: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额'] } })
  const { rows } = await aiws.sql('SELECT "月份", round(sum("销售额"), 2) AS "销售额" FROM sales GROUP BY 1 ORDER BY 1')
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal' } })
  aiws.result.html('dash', { html: '<div id=n></div>',
    js: "aiws.watch('monthly', async () => { document.getElementById('n').textContent = (await aiws.input('monthly').rows()).length })",
    bindings: { monthly: 'result:monthly', orders: 'input:sales' } })
}"#;

async fn set_amount(srv: &Srv, record: &str, value: &str) {
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": record, "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": record, "field_id": "amount", "value": value, "expect": { "rev": rev } }] }])).await;
}

/// Q10 HTML 结果 + W24: the page comes from the macro (a definition with `aiws` v2 and a Block bound by name); its code is
/// fixed when it is coded — a macro that writes data into the code fails the next run and is repaired by coding from
/// that failed run; a run moves only the data, the definition keeps its version.
#[test]
fn q10_html_result_with_bindings() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        let mut a = monthly_analysis();
        a["output_contract"]["results"] = json!([
            { "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"] },
            { "name": "dash", "type": "html", "title": "看板" }]);
        a["checks"] = json!([]);
        analyzed(&srv, a).await;
        // a page with the data written into its code: coded, applied, then the data changes
        *srv.model.execute.lock().unwrap() = coding(DASH_BAD.to_string());
        let e = code(&srv, json!({})).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        assert_eq!(e["candidate"]["program"]["manifest"]["results"]["dash"]["code_digest"].as_str().map(str::len), Some(64), "the page's code is part of the macro version");
        srv.apply(&e, json!({})).await;
        set_amount(&srv, "o1", "1000.00").await;
        let head = srv.rpc("ws.get_info", json!({})).await["head_seq"].clone();
        let failed = run(&srv).await;
        assert_eq!((failed["state"].as_str(), failed["error"]["sub_code"].as_str()), (Some("failed"), Some("MACRO_CODE_CHANGED")), "{failed}");
        assert_eq!(srv.rpc("ws.get_info", json!({})).await["head_seq"], head, "a failed run writes nothing");

        // repairing the failed run: the model is told why the macro failed and moves the data into bindings
        let good = DASH.to_string();
        *srv.model.execute.lock().unwrap() = Box::new(move |t: &Turn| {
            assert!(t.user.contains("The macro failed when it ran") && t.user.contains("HTML code other than"), "{}", t.user);
            match t.results.len() {
                0 => call("run_program", json!({ "source": good })),
                1 => call("finish", json!({ "summary": "看板改为通过绑定读取月度汇总" })),
                _ => text("完成"),
            }
        });
        let r = code(&srv, json!({ "parent_run_id": failed["run_id"] })).await;
        assert_eq!(r["state"], "waiting_confirmation", "{r}");
        srv.apply(&r, json!({})).await;
        let w = srv.wish_of("w").await;
        let dash = w["last_run"]["result_bindings"]["results"]["dash"].clone();
        let def = srv.rpc("doc.read", json!({ "entity_id": dash["entity_id"] })).await["content"]["payload"].clone();
        assert_eq!(def["html"]["api_version"], 2);
        assert_eq!(def["version"], 2, "coding changed the page's code");
        let cell = srv.rpc("doc.read", json!({ "entity_id": dash["cells"][0] })).await;
        let monthly = w["last_run"]["result_bindings"]["results"]["monthly"]["entity_id"].as_str().unwrap().to_string();
        assert_eq!(cell["content"]["payload"]["bindings"]["monthly"]["entity_id"], json!(monthly));
        assert_eq!(cell["content"]["payload"]["bindings"]["orders"]["entity_id"], "orders");
        let rel = srv.rpc("doc.relations", json!({ "entity_id": monthly })).await;
        assert!(rel["blocks"].as_array().unwrap().iter().any(|b| b["entity_id"] == dash["cells"][0]), "the binding is a reference: {rel}");

        // new data: running moves only the data the page is bound to
        let before = srv.records(&monthly).await;
        set_amount(&srv, "o2", "2000.00").await;
        let calls = srv.model.calls.load(Ordering::SeqCst);
        let p = run(&srv).await;
        assert_eq!((p["state"].as_str(), p["candidate"]["mode"].as_str()), (Some("waiting_confirmation"), Some("run")), "{p}");
        assert_eq!(srv.model.calls.load(Ordering::SeqCst), calls, "no model loop");
        srv.apply(&p, json!({})).await;
        let after = srv.rpc("doc.read", json!({ "entity_id": dash["entity_id"] })).await["content"]["payload"].clone();
        assert_eq!((&after["version"], &after["html"]), (&def["version"], &def["html"]), "the definition did not change");
        let w = srv.wish_of("w").await;
        assert_eq!(w["last_run"]["result_bindings"]["results"]["dash"]["cells"], dash["cells"], "the same Block");
        assert_eq!(w["last_run"]["result_bindings"]["results"]["monthly"]["entity_id"], json!(monthly));
        let moved = srv.records(&monthly).await;
        assert_eq!(moved.len(), 3);
        assert_ne!(moved, before, "the data moved");
    });
}

const W08_WRONG: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份'] } })
  aiws.result.text('monthly', '| a |\n|---|\n')
  aiws.result.file('commentary', '/etc/passwd', 'text/plain')
}"#;

const W08_UNDECLARED: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份'] } })
  const total = aiws.input('sales').rows().reduce((s, r) => s + r['销售额'], 0)
  aiws.result.text('commentary', `合计 ${total}`)
}"#;

const W08_NO_DECLARE: &str = r#"
export default async function main(aiws) {
  aiws.result.table('monthly', [{ 月份: '2026-07-01', 销售额: 1, 订单数: 1 }], { key: ['月份'] })
  aiws.result.text('commentary', '合计见表。')
}"#;

/// W05 / W08 / W12: configuration changes during a run, invalid macro output, two applications.
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
        // W08: what a trial run produced is validated on the spot; the model corrects the macro in the same run
        let extra = monthly("销售额").replace("aiws.result.text('commentary', text)", "aiws.result.text('commentary', text)\n  aiws.result.text('extra', '多写一个')");
        let good = monthly("销售额");
        *srv.model.execute.lock().unwrap() = Box::new(move |t: &Turn| match t.results.len() {
            0 => call("run_program", json!({ "source": W08_WRONG })),
            1 => {
                let r = &t.results[0];
                assert!(r.contains("contracted as table, but it is richtext"), "{r}");
                assert!(r.contains("File /etc/passwd is not under output/"), "only files in output/ become results: {r}");
                call("finish", json!({ "summary": "x" }))
            }
            2 => {
                assert!(t.results[1].contains("Cannot finish yet") && t.results[1].contains("has not run successfully"), "{}", t.results[1]);
                call("run_program", json!({ "source": W08_UNDECLARED }))
            }
            3 => {
                assert!(t.results[2].contains("threw an error") && t.results[2].contains("field 销售额 is not declared"), "{}", t.results[2]);
                call("run_program", json!({ "source": extra }))
            }
            4 => {
                assert!(t.results[3].contains("Result \"extra\" is not in the output contract"), "{}", t.results[3]);
                call("run_program", json!({ "source": W08_NO_DECLARE }))
            }
            5 => {
                assert!(t.results[4].contains("No manifest"), "{}", t.results[4]);
                call("finish", json!({ "summary": "x" }))
            }
            6 => {
                assert!(t.results[5].contains("Cannot finish yet") && t.results[5].contains("declares nothing"), "{}", t.results[5]);
                call("write_file", json!({ "path": "program/main.js", "content": good }))
            }
            7 => call("run_program", json!({})),
            8 => call("finish", json!({ "summary": "完成" })),
            _ => text("完成"),
        });
        let e1 = code(&srv, json!({})).await;
        assert_eq!(e1["state"], "waiting_confirmation", "{e1}");
        assert!(e1["candidate"]["program"]["manifest"]["inputs"]["sales"].is_object(), "the candidate is the last successful trial run: {}", e1["candidate"]["program"]);
        // W12: a second candidate from the same base; the first application wins, the second conflicts
        *srv.model.execute.lock().unwrap() = coding(monthly("销售额"));
        let e2 = code(&srv, json!({})).await;
        assert_eq!(e2["state"], "waiting_confirmation", "{e2}");
        srv.apply(&e1, json!({})).await;
        let p2 = srv.rpc("proc.get", json!({ "run_id": e2["run_id"], "choices": {} })).await;
        assert_eq!(p2["preview"]["ready"], false, "{}", p2["preview"]);
        assert!(p2["preview"]["summary"]["problems"].to_string().contains("another set of results"));
        // applying the stale preview it had before is a conflict, and the document keeps the first
        let r = srv.rpc("proc.apply", json!({ "run_id": e2["run_id"], "plan_digest": e2["preview"]["plan_digest"], "session_id": "s1" })).await;
        assert_eq!(r["applied"]["status"], "conflict", "{r}");
        assert_eq!(srv.rpc("proc.get", json!({ "run_id": e2["run_id"] })).await["state"], "conflict");
    });
}

/// W02 纯提示词任务 + W25 宏读取 Zone 资源 + W18 导出/导入/Clone.
#[test]
fn w02_w18_w25_zero_inputs_zone_data_packages() {
    run_test(|| async move {
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), model()).await;
        base(&mut srv).await;
        // W02: a pure creation task binds nothing and still runs
        analyzed(&srv, json!({ "status": "ready", "context_prompt": "写一句欢迎词，并读取一份 Zone 里的状态",
            "inputs": [], "output_contract": { "results": [{ "name": "hello", "type": "record", "title": "欢迎" }] },
            "checks": [], "blockers": [], "warnings": [] })).await;
        assert_eq!(srv.wish_of("w").await["inputs"], json!([]));
        // W25: the macro reads a resource of the Zone (outside the Workspace): the results are marked as resting on it
        *srv.model.execute.lock().unwrap() = coding(r#"
export default async function main(aiws) {
  aiws.declare({ inputs: {}, zone: ['/kapi/status'] })
  const status = await (await aiws.zone.fetch('/kapi/status')).text()
  aiws.result.record('hello', { 欢迎: '你好', 外部状态: status })
}"#.to_string());
        let e = code(&srv, json!({})).await;
        assert_eq!(e["state"], "waiting_confirmation", "{e}");
        assert_eq!(e["candidate"]["external_data"], json!(["https://zone.test/kapi/status"]));
        assert_eq!(e["candidate"]["program"]["manifest"]["zone"], json!(["/kapi/status"]));
        srv.apply(&e, json!({})).await;
        let w = srv.wish_of("w").await;
        let hello = w["last_run"]["result_bindings"]["results"]["hello"]["entity_id"].as_str().unwrap().to_string();
        let f = srv.rpc("doc.freshness", json!({ "entity_ids": [hello] })).await;
        assert_eq!((f["items"][0]["status"].as_str(), f["items"][0]["external_data"].as_bool()), (Some("current"), Some(true)));
        // W18: a package carries the macro (its source asset, its manifest), the dependency records and the bindings
        let ex = srv.rpc("doc.export", json!({ "mode": "share", "self_contained": true })).await;
        let pkg = srv.nfs.fetch(&ex["download"]).await;
        let package = srv.nfs.put("alice", "exports", "import/w18.bcanvas", pkg).await;
        let imported = srv.rpc("ws.import", json!({ "bucket_id": package["bucket_id"], "key": package["key"], "semantics": "new" })).await;
        let ws2 = imported["workspace_id"].as_str().unwrap().to_string();
        let w2 = srv.rpc("doc.read", json!({ "workspace_id": ws2, "entity_id": "w" })).await["content"]["payload"].clone();
        assert_eq!(w2["program"]["digest"], w["program"]["digest"]);
        assert_eq!((&w2["program"]["api_version"], &w2["program"]["manifest"]), (&json!(3), &w["program"]["manifest"]));
        let src = srv.http.get(format!("{}/asset/{}/{}", srv.base, ws2, w2["program"]["source"].as_str().unwrap())).bearer_auth("tok-alice").send().await.unwrap();
        assert!(src.status().is_success(), "the macro source travels with the package");
        assert!(String::from_utf8_lossy(&src.bytes().await.unwrap()).contains("aiws.result.record"));
        let f2 = srv.rpc("doc.freshness", json!({ "workspace_id": ws2, "entity_ids": ["w", hello] })).await;
        assert_eq!(f2["items"][0]["status"], "current", "{f2}");
        assert_eq!(f2["items"][1]["external_data"], true);
        // the runs are local: none travels with the package
        let runs = srv.rpc("proc.list", json!({ "workspace_id": ws2, "wish_id": "w" })).await;
        assert_eq!(runs["runs"].as_array().unwrap().len(), 0);
    });
}
