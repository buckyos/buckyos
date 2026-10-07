//! 许愿格 v0.2 end to end on the service (in process, real HTTP, real xllm loop, real Deno), with a
//! scripted model that reads what the host gives it: analysis through `submit_analysis`, execution
//! through a program and the delivery tools, preview and application, re-running only the program
//! (no model), a feedback round that turns into refinements, `llm.map` with its cache, the Mock
//! executor's provided results, cancellation, and a restart that reports the run interrupted.

#[path = "common/wishkit.rs"]
mod wishkit;
use aiworkspace_store::Service;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use wishkit::*;

const PROGRAM: &str = r#"
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
  aiws.check('total', Math.abs(sum(monthly.map((m) => m['销售额'])) - total) < 0.01, { total })
}
"#;

fn facts_of(results: &[String]) -> Value {
    for r in results.iter().rev() {
        if let Some(i) = r.find("facts：") {
            let rest = &r[i + "facts：".len()..];
            let end = rest.find('\n').unwrap_or(rest.len());
            if let Ok(v) = serde_json::from_str::<Value>(&rest[..end]) {
                return v;
            }
        }
    }
    json!({})
}

fn analysis_script() -> Script {
    Box::new(|t: &Turn| match t.results.len() {
        0 => call("ws_profile", json!({ "target": handle(&t.user, "表格视图「华东订单」") })),
        1 => {
            let b = handle(&t.user, "表格视图「华东订单」");
            call("submit_analysis", json!({ "analysis": {
                "schema_version": "wish.analysis.v2", "status": "ready",
                "context_prompt": format!("读取 {b} 的全部记录，排除 状态=未结算，按月汇总销售额与订单数；写一段解读，数字取自 facts。"),
                "inputs": [{ "name": "sales", "ref": b }],
                "output_contract": { "results": [
                    { "name": "monthly", "type": "table", "title": "月度汇总", "approach": "program", "key": ["月份"],
                      "views": [{ "renderer": "table" }, { "renderer": "sample.line-chart", "config": { "x": "月份", "y": "销售额" } }] },
                    { "name": "commentary", "type": "richtext", "title": "解读", "approach": "direct" }] },
                "checks": [{ "id": "total", "kind": "program", "text": "月度合计等于已结算合计" }, { "id": "cited", "kind": "review", "text": "数字可追溯" }],
                "blockers": [], "warnings": ["已按便签排除未结算订单"] } }))
        }
        _ => text("分析完成。"),
    })
}

fn execute_script(feedback: bool) -> Script {
    Box::new(move |t: &Turn| {
        let has_program = t.user.contains("program/main.js 已有程序");
        let mut steps: Vec<&str> = Vec::new();
        if !has_program {
            steps.push("write");
        }
        steps.extend(["run", "put", "check", "finish", "done"]);
        let step = steps.get(t.results.len()).copied().unwrap_or("done");
        match step {
            "write" => call("write_file", json!({ "path": "program/main.js", "content": PROGRAM })),
            "run" => call("run_program", json!({})),
            "put" => {
                let f = facts_of(&t.results);
                call("put_result", json!({ "name": "commentary", "type": "text", "markdown": format!("## 解读\n\n已结算销售额合计 {}，共 {} 个月。", f["total"], f["months"]) }))
            }
            "check" => call("check_results", json!({})),
            "finish" => {
                let mut args = json!({ "summary": "完成", "assumptions": ["空值按 0"], "review_notes": [{ "id": "cited", "note": "解读中的数字取自 facts" }] });
                if feedback {
                    args["refinements"] = json!(["解读中给出月份数"]);
                }
                call("finish", args)
            }
            _ => text("已完成。"),
        }
    })
}

async fn setup(srv: &mut Srv) {
    let ws = srv.rpc("ws.create", json!({ "title": "季度" })).await;
    srv.ws = ws["workspace_id"].as_str().unwrap().to_string();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "Q3 经营" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "Q3 经营",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "Q3 经营", "content_folder_id": "sf-content" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "订单",
          "payload": { "fields": [
            { "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "customer", "name": "客户", "type": "text" },
            { "field_id": "region", "name": "地区", "type": "select", "options": [{ "option_id": "east", "label": "华东" }, { "option_id": "south", "label": "华南" }] },
            { "field_id": "amount", "name": "销售额", "type": "decimal", "scale": 2 },
            { "field_id": "status", "name": "状态", "type": "select", "options": [{ "option_id": "done", "label": "已结算" }, { "option_id": "open", "label": "未结算" }] },
            { "field_id": "note", "name": "反馈", "type": "text" } ] } },
    ])).await;
    let rows: Vec<Value> = (0..30).map(|i| json!({ "record_id": format!("o{i}"), "values": {
        "month": (["2026-07-01", "2026-08-01", "2026-09-01"])[i % 3], "customer": format!("客户{}", i % 7), "region": if i % 4 == 0 { "south" } else { "east" },
        "amount": format!("{}.50", 100 + i * 10), "status": if i == 29 { "open" } else { "done" }, "note": if i % 2 == 0 { "服务很好" } else { "太慢了" } } })).collect();
    srv.commit(json!([{ "op": "table.insert_records", "source_id": "orders", "records": rows }])).await;
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "wish-q3", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w", "name": "季度分析",
          "payload": { "title": "季度分析", "prompt": "分析左边那张只显示华东的表，按月汇总销售额并写解读", "executor": "xllm", "output_mode": "overwrite",
                       "output": { "container_id": "sf-content", "surface_id": "sf", "name": "季度分析结果" } } },
        { "op": "entity.create", "entity_id": "blk-orders", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "b", "placement": { "x": 40, "y": 300, "w": 500, "h": 300 },
          "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "orders" }, "title": "华东订单",
                       "filter": { "op": "cmp", "field_id": "region", "operator": "eq", "value": "east" } } },
        { "op": "entity.create", "entity_id": "blk-wish", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "d", "placement": { "x": 580, "y": 300, "w": 420, "h": 300 },
          "payload": { "view": { "type": "wish" }, "source_ref": { "entity_id": "wish-q3" } } },
    ])).await;
}

fn scripted() -> Arc<ScriptedModel> {
    let m = model();
    *m.analyze.lock().unwrap() = analysis_script();
    *m.execute.lock().unwrap() = execute_script(false);
    m
}

fn loc() -> Value {
    json!({ "cell_id": "blk-wish", "selection": ["blk-orders"] })
}

#[test]
fn analyze_execute_apply_rerun_feedback() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;

    // execution before analysis is refused with a reason
    let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "execute", "location": loc() })).await;
    assert_eq!(r["state"], "failed");
    assert!(r["error"]["detail"].as_str().unwrap().contains("还没有分析"), "{r}");

    // ---- analysis: bound to the view, written back by the UI's apply
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "request": { "today": "2026-10-06", "timezone": "Asia/Shanghai" } })).await;
    assert_eq!(a["state"], "waiting_confirmation", "{a}");
    // every stage request carries an output limit (Claude requires one)
    assert_eq!(*m.max_tokens.lock().unwrap(), Some(32_000));
    assert_eq!(a["candidate"]["inputs"][0]["selector"]["kind"], "table_view");
    assert!(!a["candidate"]["analysis"]["context_prompt"].as_str().unwrap().contains('@'));
    srv.apply(&a, json!({})).await;
    let w = srv.wish().await;
    assert_eq!(w["analysis"]["status"], "ready");
    let f = srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-q3"] })).await;
    assert_eq!(f["items"][0]["needs_analysis"], false);

    // ---- execution: program → run_program → put_result → check → finish → candidate with a preview
    let e = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "execute", "location": loc(), "request": { "today": "2026-10-06" } })).await;
    assert_eq!(e["state"], "waiting_confirmation", "{e}");
    let cand = &e["candidate"];
    assert_eq!(cand["results"][0]["table"]["rows"].as_array().unwrap().len(), 3);
    assert_eq!(cand["checks"].as_array().unwrap().iter().find(|c| c["id"] == "total").unwrap()["status"], "passed");
    assert_eq!(cand["checks"].as_array().unwrap().iter().find(|c| c["id"] == "cited").unwrap()["status"], "review");
    assert!(cand["program"]["object_id"].is_string(), "the program source is staged");
    assert_eq!(e["progress"]["program_runs"], 1);
    let applied = srv.apply(&e, json!({})).await;
    assert!(applied["applied"]["commit"]["commit_id"].is_string());
    let w = srv.wish().await;
    assert_eq!(w["program"]["api_version"], 2);
    let monthly = w["last_run"]["result_bindings"]["results"]["monthly"]["entity_id"].as_str().unwrap().to_string();
    assert_eq!(w["last_run"]["checks"]["passed"], 1);
    let t = srv.rpc("doc.query", json!({ "source_id": monthly, "limit": 10 })).await;
    assert_eq!(t["total"], 3);
    let line = srv.rpc("doc.list_children", json!({ "entity_id": w["last_run"]["result_bindings"]["group"] })).await;
    assert_eq!(line["children"].as_array().unwrap().len(), 3, "table view + line chart + text");
    let f = srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-q3"] })).await;
    assert_eq!(f["items"][0]["status"], "current", "{f}");

    // ---- data changes → stale; re-running only the program needs no model
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "1000.00", "expect": { "rev": rev } }] }])).await;
    assert_eq!(srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-q3"] })).await["items"][0]["status"], "stale");
    let calls = m.calls.load(Ordering::SeqCst);
    let p = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "rerun_program", "location": loc() })).await;
    assert_eq!(p["state"], "waiting_confirmation", "{p}");
    assert_eq!(m.calls.load(Ordering::SeqCst), calls, "no model call");
    srv.apply(&p, json!({})).await;
    let f = srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-q3"] })).await;
    assert_eq!(f["items"][0]["direct_stale"], true, "{f}");

    // ---- a feedback round: starts from the program, ends in a refinement on the wish
    *m.execute.lock().unwrap() = execute_script(true);
    let fb = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "execute", "location": loc(), "parent_run_id": e["run_id"], "feedback": "解读里写上月份数" })).await;
    assert_eq!(fb["state"], "waiting_confirmation", "{fb}");
    srv.apply(&fb, json!({})).await;
    let w = srv.wish().await;
    assert_eq!(w["refinements"][0]["text"], "解读里写上月份数".replace("解读里写上月份数", "解读中给出月份数"));
    assert_eq!(srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-q3"] })).await["items"][0]["needs_analysis"], false, "host-sanctioned refinements keep the analysis valid");
    let runs = srv.rpc("proc.list", json!({ "wish_id": "wish-q3" })).await;
    assert!(runs["runs"].as_array().unwrap().len() >= 5);
    });
}

#[test]
fn llm_map_cache_mock_and_cancel() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    // an analysis with a per-item judgement result
    *m.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("submit_analysis", json!({ "analysis": { "status": "ready", "context_prompt": "给 sales 的反馈打情感标签",
            "inputs": [{ "name": "sales", "ref": handle(&t.user, "表格视图「华东订单」") }],
            "output_contract": { "results": [{ "name": "情感", "type": "table_columns", "approach": "program", "target": "sales" }] },
            "checks": [], "blockers": [], "warnings": [] } })),
        _ => text("好"),
    });
    *m.execute.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("write_file", json!({ "path": "program/main.js", "content": r#"
export default async function main(aiws) {
  const rows = aiws.input('sales').rows({ fields: ['反馈'] })
  const tags = await aiws.llm.map(rows.map((r) => r['反馈']), '判断情感', { type: 'string', enum: ['正面', '负面'] })
  aiws.result.columns('情感', 'sales', Object.fromEntries(rows.map((r, i) => [r._id, { 情感: tags[i] }])), { fields: { 情感: { type: 'select', options: ['正面', '负面'] } } })
}"# })),
        1 => call("run_program", json!({})),
        2 => call("finish", json!({ "summary": "已打标签" })),
        _ => text("完成"),
    });
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "analyze", "location": loc() })).await;
    srv.apply(&a, json!({})).await;
    let e = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "execute", "location": loc() })).await;
    assert_eq!(e["state"], "waiting_confirmation", "{e}");
    let first_map_calls = m.map_calls.load(Ordering::SeqCst);
    assert!(first_map_calls >= 1);
    assert_eq!(e["candidate"]["model_judgment"], json!(["情感"]));
    srv.apply(&e, json!({})).await;
    let w = srv.wish().await;
    let fid = w["last_run"]["result_bindings"]["results"]["情感"]["fields"]["情感"].as_str().unwrap().to_string();
    let v = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o2", "field_id": fid } })).await;
    assert!(v["content"]["value"].is_string(), "{v}");
    // a re-run asks the model only for changed items: none changed → cached
    let p = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "rerun_program", "location": loc() })).await;
    assert_eq!(p["state"], "waiting_confirmation", "{p}");
    assert_eq!(m.map_calls.load(Ordering::SeqCst), first_map_calls, "all cached");

    // ---- the Mock: results provided by the browser executor, planned by the service
    srv.commit(json!([{ "op": "entity.create", "entity_id": "wish-mock", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "x",
        "payload": { "prompt": "模拟", "executor": "mock", "output": { "container_id": "sf-content", "surface_id": "sf", "name": "模拟结果" } } }])).await;
    let mock = srv.run("wish.mock@1", json!({ "wish_id": "wish-mock", "stage": "execute", "provided": {
        "results": [
            { "name": "概览", "type": "richtext", "content": { "markdown": "# 概览\n\n- 一\n- 二" } },
            { "name": "明细", "type": "table", "content": { "fields": [{ "field_id": "a", "name": "名称", "type": "text" }, { "field_id": "b", "name": "数", "type": "number" }], "rows": [{ "a": "x", "b": 1 }, { "a": "y", "b": 2 }] } },
            { "name": "图", "type": "image", "content": { "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"32\"></svg>", "width": 64, "height": 32 } }],
        "read_set": [], "summary": "模拟" } })).await;
    assert_eq!(mock["state"], "waiting_confirmation", "{mock}");
    assert_eq!(mock["simulated"], true);
    srv.apply(&mock, json!({})).await;
    let wm = srv.rpc("doc.read", json!({ "entity_id": "wish-mock" })).await["content"]["payload"].clone();
    assert_eq!(wm["last_run"]["simulated"], true);
    assert_eq!(wm["last_run"]["result_bindings"]["results"].as_object().unwrap().len(), 3);

    // ---- cancel: the model is interrupted, nothing is written
    m.delay_ms.store(3000, Ordering::SeqCst);
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "params": { "wish_id": "wish-q3", "stage": "analyze" }, "idempotency_key": "slow" })).await;
    let id = start["run_id"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(300)).await;
    // the snapshot is fixed and the model is working
    let r = srv.rpc("proc.get", json!({ "run_id": id })).await;
    assert_eq!((r["state"].as_str(), r["progress"]["waiting_model"].as_bool()), (Some("running"), Some(true)), "{r}");
    let head = srv.rpc("ws.get_info", json!({})).await["head_seq"].clone();
    let c = srv.rpc("proc.cancel", json!({ "run_id": id })).await;
    assert_eq!(c["state"], "cancelled");
    let r = srv.wait(&id).await;
    assert_eq!(r["state"], "cancelled", "{r}");
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(srv.rpc("proc.get", json!({ "run_id": id })).await["state"], "cancelled", "a late answer does not revive it");
    assert_eq!(srv.rpc("ws.get_info", json!({})).await["head_seq"], head);
    // the same start request is the same run
    let again = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "params": { "wish_id": "wish-q3", "stage": "analyze" }, "idempotency_key": "slow" })).await;
    assert_eq!(again["run_id"], json!(id));
    });
}

#[test]
fn restart_reports_interrupted_and_reconciles_applying() {
    run_test(|| async move {
    let dir = tempfile::tempdir().unwrap();
    let (ws_id, running, applying) = {
        let svc = Service::open(&dir.path().join("data")).unwrap();
        let alice = aiworkspace_store::Caller::user("alice");
        let id = svc.create_workspace(&alice, "w", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
        let h = svc.workspace(&id).unwrap();
        let mut ws = h.lock().unwrap();
        let params = json!({ "wish_id": "w1", "stage": "analyze" });
        let (r1, _) = ws.wish_run_create(&alice, "wish.xllm@1", &params, "k1").unwrap();
        ws.wish_run_set(&r1, Some("running"), None, None, None, None).unwrap();
        // the process died after submitting a commit that never happened
        let (r2, _) = ws.wish_run_create(&alice, "wish.xllm@1", &params, "k2").unwrap();
        ws.wish_run_set(&r2, Some("applying"), None, None, None, Some(&json!({ "applying": { "plan_digest": "d", "idempotency_key": "run/none/d" } }))).unwrap();
        (id, r1, r2)
    };
    let mut srv = Srv::start(dir, scripted()).await;
    srv.ws = ws_id;
    let r = srv.rpc("proc.get", json!({ "run_id": running })).await;
    assert_eq!(r["state"], "interrupted", "{r}");
    assert_eq!(r["error"]["code"], "INTERRUPTED");
    let r = srv.rpc("proc.get", json!({ "run_id": applying })).await;
    assert_eq!(r["state"], "waiting_confirmation", "{r}");
    });
}
