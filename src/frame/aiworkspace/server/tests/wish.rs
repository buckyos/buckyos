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
        if let Some(i) = r.find("\nfacts: ") {
            let rest = &r[i + "\nfacts: ".len()..];
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
        0 => call("ws_profile", json!({ "target": handle(&t.user, "table view \"华东订单\"") })),
        1 => {
            let b = handle(&t.user, "table view \"华东订单\"");
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
        let has_program = t.user.contains("program/main.js already has a program");
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
    assert!(r["error"]["detail"].as_str().unwrap().contains("has not been analyzed yet"), "{r}");

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
            "inputs": [{ "name": "sales", "ref": handle(&t.user, "table view \"华东订单\"") }],
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
    // the analysis, the execution and the per-item judge each asked for a client charged to the owner
    let seen = m.providers.lock().unwrap().clone();
    assert!(seen.len() >= 3 && seen.iter().all(|p| p.on_behalf_of.as_ref().is_some_and(|o| o.user_id == "alice")), "{seen:?}");
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
        let (r1, _) = ws.wish_run_create(&alice, "wish.xllm@1", &params, "k1", &json!(null)).unwrap();
        ws.wish_run_set(&r1, Some("running"), None, None, None, None).unwrap();
        // the process died after submitting a commit that never happened
        let (r2, _) = ws.wish_run_create(&alice, "wish.xllm@1", &params, "k2", &json!(null)).unwrap();
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

#[test]
fn executors_outside_the_service_claim_report_and_the_service_plans_their_candidates() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    let ws = srv.ws.clone();
    // a local run: the starter's browser is the executor; the claim token comes back once
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "loc-a", "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "execution": { "mode": "local" } } })).await;
    assert_eq!((start["state"].as_str(), start["context"]["location"].as_str(), start["context"]["executor"]["kind"].as_str()), (Some("queued"), Some("local"), Some("browser")), "{start}");
    assert_eq!(start["context"]["trigger"], "alice");
    assert_eq!((start["aicc"]["mode"].as_str(), start["aicc"]["user_id"].as_str()), (Some("current_user"), Some("alice")), "a local run's model calls are the starter's own: {start}");
    assert!(start["context"]["task"]["digest"].is_string() && start["brief"]["prompt"].is_string());
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    assert_eq!(m.calls.load(Ordering::SeqCst), 0, "the service runs no model for a local run");
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let (attempt, run_token) = (claimed["attempt"].as_u64().unwrap(), claimed["run_token"].as_str().unwrap().to_string());
    assert_eq!(attempt, 1);
    // the run token speaks for the run, reads the Workspace, and nothing else
    let hb = srv.call_as(&run_token, "proc.heartbeat", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "progress": { "phase": "thinking" } })).await;
    assert_eq!(hb["result"]["ok"], true, "{hb}");
    assert_eq!(srv.call_as(&run_token, "doc.read", json!({ "workspace_id": ws, "entity_id": "orders" })).await["result"]["type_id"], "buckyos.table-source");
    assert!(srv.call_as(&run_token, "doc.commit", json!({ "workspace_id": ws, "epoch": "x", "idempotency_key": "k", "operations": [] })).await["error"].is_string(), "a run token cannot write");
    assert_eq!(srv.rpc("proc.get", json!({ "run_id": run_id })).await["progress"]["phase"], "thinking");
    // the executor reports its analysis (ids instead of handles); the service validates and plans it
    let analysis = json!({ "schema_version": "wish.analysis.v2", "status": "ready", "context_prompt": "读取 blk-orders 的全部记录，按月汇总销售额并写解读。",
        "inputs": [{ "name": "sales", "ref": "blk-orders" }],
        "output_contract": { "results": [{ "name": "commentary", "type": "richtext", "title": "解读", "approach": "direct" }] },
        "checks": [], "blockers": [], "warnings": [] });
    let reported = srv.call_as(&run_token, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "analysis": analysis })).await;
    assert!(reported["result"]["ok"] == json!(true), "{reported}");
    let done = srv.wait(&run_id).await;
    assert_eq!(done["state"], "waiting_confirmation", "{done}");
    assert_eq!(done["candidate"]["inputs"][0]["name"], "sales", "{done}");
    srv.apply(&done, json!({})).await;
    // an execute run reports results; a superseded attempt is refused; a changed wish is DELEGATION_STALE
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "loc-e", "params": { "wish_id": "wish-q3", "stage": "execute", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let first = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let second = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    assert_eq!((first["attempt"].as_u64(), second["attempt"].as_u64()), (Some(1), Some(2)));
    let stale = srv.call_as(first["run_token"].as_str().unwrap(), "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "results": [] })).await;
    assert_eq!(stale["result"]["error"]["code"], "RUN_REVOKED", "{stale}");
    let results = json!([{ "name": "commentary", "type": "richtext", "title": "解读", "content": { "markdown": "# 解读

华东三个月稳步增长。" } }]);
    let reported = srv.call_as(second["run_token"].as_str().unwrap(), "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 2, "results": results, "summary": "done", "read_set": [] })).await;
    assert!(reported["result"]["ok"] == json!(true), "{reported}");
    let done = srv.wait(&run_id).await;
    assert_eq!((done["state"].as_str(), done["candidate"]["executor"].as_str(), done["execution"]["attempt"].as_u64()), (Some("waiting_confirmation"), Some("local"), Some(2)), "{done}");
    let rev = srv.rpc("doc.read", json!({ "entity_id": "wish-q3" })).await["content"]["key_revs"]["prompt"].clone();
    srv.commit(json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [{ "key": "prompt", "value": "改了题目", "expect": { "rev": rev } }] }])).await;
    let preview = srv.rpc("proc.get", json!({ "run_id": run_id, "choices": {} })).await;
    let refused = srv.call_as("tok-alice", "proc.apply", json!({ "workspace_id": ws, "run_id": run_id, "plan_digest": preview["preview"]["plan_digest"] })).await;
    assert_eq!(refused["result"]["error"]["code"], "DELEGATION_STALE", "{refused}");
    // hosted: the owner configures the agent; without a Zone runtime the dispatch is recorded unavailable and the run waits
    let denied = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "host-0", "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "execution": { "mode": "hosted" } } }));
    assert_eq!(srv.call_as("tok-alice", "proc.start", json!({ "workspace_id": ws, "program": "wish.xllm@1", "idempotency_key": "host-0", "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "execution": { "mode": "hosted" } } })).await["result"]["error"]["code"], "EXECUTION_CONTEXT_MISSING");
    drop(denied);
    let cfg = srv.rpc("ws.set_hosted_exec", json!({ "config": { "enabled": true, "agent_id": "jarvis.test.buckyos.io", "allowed_wishes": ["wish-q3"] } })).await;
    assert!(cfg["hosted_exec"]["digest"].is_string());
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "host-1", "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "execution": { "mode": "hosted" } } })).await;
    assert_eq!((start["context"]["location"].as_str(), start["context"]["executor"]["principal"].as_str()), (Some("hosted"), Some("jarvis.test.buckyos.io")), "{start}");
    assert!(start.get("claim_token").is_none(), "the token goes to the agent, not the starter");
    let run_id = start["run_id"].as_str().unwrap().to_string();
    let mut got = srv.rpc("proc.get", json!({ "run_id": run_id })).await;
    for _ in 0..100 {
        if got["dispatch"]["status"].is_string() { break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
        got = srv.rpc("proc.get", json!({ "run_id": run_id })).await;
    }
    assert_eq!(got["dispatch"]["status"], "unavailable", "{got}");
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": got["dispatch"]["claim_token"] })).await;
    assert_eq!(claimed["attempt"], 1);
    });
}

fn llm_context_of(dir: &std::path::Path, run_id: &str) -> Value {
    fn find(dir: &std::path::Path, run_id: &str) -> Option<std::path::PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == run_id) && p.join("work/.llm_context").is_file() {
                    return Some(p.join("work/.llm_context"));
                }
                if let Some(f) = find(&p, run_id) {
                    return Some(f);
                }
            }
        }
        None
    }
    let f = find(dir, run_id).expect("the stage wrote its .llm_context");
    serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap()
}

#[test]
fn wish_runs_use_the_owners_or_the_starters_aicc() {
    run_test(|| async move {
    let m = scripted();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let mut srv = Srv::start_with(dir, m.clone(), json!({
        "tok-alice": { "principal": "alice" },
        "tok-bob": { "principal": "bob" },
        "tok-eve": { "principal": "did:web:eve.example.com", "class": "external" },
    })).await;
    setup(&mut srv).await;
    for who in ["bob", "did:web:eve.example.com"] {
        srv.rpc("ws.grant", json!({ "subject": who, "capabilities": ["read", "comment", "update"] })).await;
    }
    assert_eq!(srv.rpc("ws.get_info", json!({})).await["wish_aicc_mode"], "workspace_owner");

    // default: a collaborator's run uses the owner's AICC; the operator stays the collaborator
    let start = srv.call_as("tok-bob", "proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "b1",
        "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc() } })).await["result"].clone();
    let run_id = start["run_id"].as_str().unwrap().to_string();
    assert_eq!(start["aicc"], json!({ "mode": "workspace_owner", "user_id": "alice", "operator_id": "bob" }));
    let done = srv.wait_as("tok-bob", &run_id).await;
    assert_eq!(done["state"], "waiting_confirmation", "{done}");
    let ctx = llm_context_of(&root, &run_id);
    assert_eq!(ctx["provider"]["on_behalf_of"]["user_id"], "alice");
    assert_eq!(ctx["provider"]["on_behalf_of"]["operator_id"], "bob");
    assert_eq!(ctx["provider"]["on_behalf_of"]["run_ref"]["run_id"], json!(run_id));
    // and the AICC client the stage loop actually asked for carries it (内核架构改进 §12.2)
    let seen: Vec<_> = m.providers.lock().unwrap().drain(..).collect();
    assert!(!seen.is_empty());
    for p in &seen {
        let o = p.on_behalf_of.as_ref().expect("a delegated AICC client");
        assert_eq!((o.user_id.as_str(), o.operator_id.as_deref(), o.mode), ("alice", Some("bob"), buckyos_api::AiccAttributionMode::WorkspaceOwner));
        assert_eq!(o.run_ref.as_ref().and_then(|r| r.run_id.as_deref()), Some(run_id.as_str()));
    }

    // only a manager switches the mode
    let denied = srv.call_as("tok-bob", "ws.set_wish_aicc", json!({ "mode": "current_user" })).await;
    assert_eq!(denied["result"]["error"]["code"], "PERMISSION_DENIED", "{denied}");
    srv.rpc("ws.set_wish_aicc", json!({ "mode": "current_user" })).await;
    let start = srv.call_as("tok-bob", "proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "b2",
        "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc() } })).await["result"].clone();
    assert_eq!(start["aicc"]["user_id"], "bob");
    srv.wait_as("tok-bob", start["run_id"].as_str().unwrap()).await;
    let seen: Vec<_> = m.providers.lock().unwrap().drain(..).collect();
    assert!(!seen.is_empty() && seen.iter().all(|p| p.on_behalf_of.as_ref().is_some_and(|o| o.user_id == "bob" && o.mode == buckyos_api::AiccAttributionMode::CurrentUser)), "{seen:?}");

    // an External user has no AI resources here and is not moved to the owner's
    let refused = srv.call_as("tok-eve", "proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "e1",
        "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc() } })).await;
    assert_eq!(refused["result"]["error"]["sub_code"], "AI_ATTRIBUTION_UNAVAILABLE", "{refused}");

    // a quota refusal fails the run with its sub-code; a collaborator never sees the owner's rules
    srv.rpc("ws.set_wish_aicc", json!({ "mode": "workspace_owner" })).await;
    *m.refuse.lock().unwrap() = Some(llm_context::RefusalKind::QuotaExhausted);
    let start = srv.call_as("tok-bob", "proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "b3",
        "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc() } })).await["result"].clone();
    let failed = srv.wait_as("tok-bob", start["run_id"].as_str().unwrap()).await;
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"]["sub_code"], "AI_QUOTA_EXHAUSTED");
    assert!(failed["error"]["detail"].as_str().unwrap().contains("workspace owner"), "{failed}");
    assert!(failed["error"]["data"]["xllm"]["error"]["message"].is_null());
    let own = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "analyze", "location": loc() })).await;
    assert_eq!(own["error"]["sub_code"], "AI_QUOTA_EXHAUSTED");
    assert!(own["error"]["detail"].as_str().unwrap().contains("Your AI token quota"), "{own}");
    });
}

#[test]
fn an_outside_executor_works_on_the_services_stage_through_proc_tool_and_proc_program() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    let ws = srv.ws.clone();
    // analysis: the claim opens the stage; the executor's model loop calls the stage tools over the run token
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "host-a", "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let stage = &claimed["stage"];
    assert_eq!((stage["kind"].as_str(), stage["stage"].as_str(), stage["model"].is_string()), (Some("model"), Some("analyze"), true), "{claimed}");
    let user = stage["user"].as_str().unwrap().to_string();
    assert!(user.contains("# Wish (analysis stage)") && user.contains("华东订单"), "the user message carries the context map");
    let names: Vec<&str> = stage["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"ws_profile") && names.contains(&"submit_analysis") && !names.contains(&"run_program"), "{names:?}");
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    let b = handle(&user, "table view \"华东订单\"");
    let profile = srv.call_as(&rt, "proc.tool", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": "ws_profile", "args": { "target": b } })).await;
    assert_eq!(profile["result"]["status"], "ok", "{profile}");
    assert!(profile["result"]["text"].as_str().unwrap().contains("销售额"));
    let bad = srv.call_as(&rt, "proc.tool", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": "submit_analysis", "args": { "analysis": { "schema_version": "wish.analysis.v2", "status": "ready", "context_prompt": "x", "inputs": [{ "name": "sales", "ref": "@Z9" }], "output_contract": { "results": [] }, "checks": [], "blockers": [], "warnings": [] } } })).await;
    assert_eq!(bad["result"]["status"], "error", "a wrong handle is answered to the model, not failed: {bad}");
    let analysis = json!({ "schema_version": "wish.analysis.v2", "status": "ready",
        "context_prompt": format!("读取 {b} 的全部记录，排除 状态=未结算，按月汇总销售额与订单数；写一段解读，数字取自 facts。"),
        "inputs": [{ "name": "sales", "ref": b }],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "approach": "program", "key": ["月份"], "views": [{ "renderer": "table" }] },
            { "name": "commentary", "type": "richtext", "title": "解读", "approach": "direct" }] },
        "checks": [{ "id": "total", "kind": "program", "text": "月度合计等于已结算合计" }], "blockers": [], "warnings": [] });
    let ok = srv.call_as(&rt, "proc.tool", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": "submit_analysis", "args": { "analysis": analysis } })).await;
    assert_eq!(ok["result"]["status"], "ok", "{ok}");
    assert_eq!(srv.rpc("proc.get", json!({ "run_id": run_id })).await["progress"]["tool_calls"], 3);
    let reported = srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    assert_eq!(reported["result"]["ok"], true, "{reported}");
    let done = srv.wait(&run_id).await;
    assert_eq!((done["state"].as_str(), done["candidate"]["inputs"][0]["name"].as_str()), (Some("waiting_confirmation"), Some("sales")), "{done}");
    assert_eq!(m.calls.load(Ordering::SeqCst), 0, "the service called no model");
    srv.apply(&done, json!({})).await;
    // execution: the executor runs the program on the snapshot it fetches and hands the host's output in
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "host-e", "params": { "wish_id": "wish-q3", "stage": "execute", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let stage = &claimed["stage"];
    assert!(stage["user"].as_str().unwrap().contains("## Output contract") && stage["source"].is_null(), "{claimed}");
    let run_tool = stage["tools"].as_array().unwrap().iter().find(|t| t["name"] == "run_program").unwrap();
    assert_eq!(run_tool["parameters"]["required"][0], "source");
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    let files = srv.call_as(&rt, "proc.files", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1 })).await;
    let inputs: Value = serde_json::from_str(files["result"]["files"]["context/inputs.json"]["text"].as_str().unwrap()).unwrap();
    assert!(inputs["sales"]["entity_id"].is_string(), "{files}");
    assert!(files["result"]["files"].as_object().unwrap().keys().any(|k| k.ends_with("rows.jsonl")), "the rows are in the snapshot");
    let output = json!({ "results": [{ "name": "monthly", "type": "table", "rows": [{ "月份": "2026-07-01", "销售额": 10.5, "订单数": 1 }], "fields": [{ "name": "月份", "type": "date" }, { "name": "销售额", "type": "decimal" }, { "name": "订单数", "type": "number" }], "key": ["月份"] }],
        "facts": { "total": 10.5, "months": 1 }, "checks": [{ "id": "total", "passed": true, "detail": null }], "external": [], "llm_map": { "calls": 0, "items": 0, "cached": 0 }, "error": null });
    let ran = srv.call_as(&rt, "proc.program", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "source": PROGRAM, "results": output, "log": "", "files": { "output/note.txt": "hello" } })).await;
    assert_eq!(ran["result"]["status"], "ok", "{ran}");
    assert!(ran["result"]["text"].as_str().unwrap().contains("ran successfully"));
    let put = srv.call_as(&rt, "proc.tool", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": "put_result", "args": { "name": "commentary", "type": "text", "markdown": "## 解读\n\n合计 10.5，共 1 个月。" } })).await;
    assert_eq!(put["result"]["status"], "ok", "{put}");
    let fin = srv.call_as(&rt, "proc.tool", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": "finish", "args": { "summary": "done" } })).await;
    assert_eq!(fin["result"]["status"], "ok", "{fin}");
    let reported = srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    assert_eq!(reported["result"]["ok"], true, "{reported}");
    let done = srv.wait(&run_id).await;
    assert_eq!((done["state"].as_str(), done["candidate"]["results"].as_array().map(Vec::len), done["candidate"]["executor"].as_str()), (Some("waiting_confirmation"), Some(2), Some("xllm")), "{done}");
    assert!(done["candidate"]["program"]["object_id"].is_string(), "the program is staged with the candidate");
    assert_eq!(done["candidate"]["checks"][0]["status"], "passed");
    assert!(done["preview"]["ready"] == json!(true), "{done}");
    // cancelled under the executor: its tools answer cancelled and its report is refused; a new claim opens a fresh stage
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "host-c", "params": { "wish_id": "wish-q3", "stage": "execute", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    srv.rpc("proc.cancel", json!({ "run_id": run_id })).await;
    let refused = srv.call_as(&rt, "proc.tool", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": "check_results", "args": {} })).await;
    assert_eq!(refused["result"]["error"]["code"], "RUN_REVOKED", "{refused}");
    let again = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    assert_eq!(again["error"]["code"].as_str().or(again["code"].as_str()), Some("RUN_REVOKED"), "{again}");
    });
}
