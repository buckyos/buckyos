//! 许愿格 AI Native 第一期 end to end on the service (in process, real HTTP, real Deno), with a test executor playing the
//! browser and a scripted model that reads what the host gives it: analysis through `submit_analysis`; coding — a macro
//! that declares what it reads, computes with SQL, has the model write its commentary from the facts, and is trial-run;
//! applying writes the macro and its results in one Commit; running only the macro (no model loop) after the data
//! changed; a renamed field changes nothing, a deleted one asks for coding again; a feedback round; `llm.map` with its
//! cache; the Mock executor; cancellation; the executor protocol; a restart that reports the run interrupted.

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
  aiws.declare({ inputs: { sales: ['月份', '销售额', '状态'] }, ai: { complete: { max_calls: 3 } } })
  const { rows } = await aiws.sql(`SELECT "月份", round(sum("销售额"), 2) AS "销售额", count(*) AS "订单数"
    FROM sales WHERE "状态" <> '未结算' GROUP BY 1 ORDER BY 1`)
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal', 订单数: 'number' } })
  const total = Math.round(rows.reduce((s, r) => s + r['销售额'], 0) * 100) / 100
  aiws.facts({ total, months: rows.length })
  const settled = aiws.input('sales').rows().filter((r) => r['状态'] !== '未结算')
  const direct = Math.round(settled.reduce((s, r) => s + (r['销售额'] ?? 0), 0) * 100) / 100
  aiws.check('total', Math.abs(direct - total) < 0.01, { total, direct })
  const { text } = await aiws.ai.complete({ system: '只用 facts 里的数字写解读', prompt: JSON.stringify({ total, months: rows.length }) })
  aiws.result.text('commentary', text)
}
"#;

fn analysis_script() -> Script {
    Box::new(|t: &Turn| match t.results.len() {
        0 => call("ws_profile", json!({ "target": handle(&t.user, "table view \"华东订单\"") })),
        1 => {
            let b = handle(&t.user, "table view \"华东订单\"");
            call("submit_analysis", json!({ "analysis": {
                "schema_version": "wish.analysis.v3", "status": "ready",
                "context_prompt": format!("读取 {b} 的全部记录，排除 状态=未结算，按月汇总销售额与订单数；宏让模型按 facts 写一段解读。"),
                "inputs": [{ "name": "sales", "ref": b }],
                "output_contract": { "results": [
                    { "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"],
                      "views": [{ "renderer": "table" }, { "renderer": "sample.line-chart", "config": { "x": "月份", "y": "销售额" } }] },
                    { "name": "commentary", "type": "richtext", "title": "解读" }] },
                "checks": [{ "id": "total", "kind": "program", "text": "月度合计等于已结算合计" }, { "id": "cited", "kind": "review", "text": "数字可追溯" }],
                "blockers": [], "warnings": ["已按便签排除未结算订单"] } }))
        }
        _ => text("分析完成。"),
    })
}

fn code_script(feedback: bool) -> Script {
    Box::new(move |t: &Turn| match t.results.len() {
        0 => call("run_program", json!({ "source": PROGRAM })),
        1 => call("check_results", json!({})),
        2 => {
            let mut args = json!({ "summary": "完成", "assumptions": ["空值按 0"], "review_notes": [{ "id": "cited", "note": "解读中的数字取自 facts" }] });
            if feedback {
                args["refinements"] = json!(["解读中给出月份数"]);
            }
            call("finish", args)
        }
        _ => text("已完成。"),
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
    *m.execute.lock().unwrap() = code_script(false);
    m
}

fn loc() -> Value {
    json!({ "cell_id": "blk-wish", "selection": ["blk-orders"] })
}

async fn fresh(srv: &Srv) -> Value {
    srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-q3"] })).await["items"][0].clone()
}

#[test]
fn analyze_code_apply_run_feedback() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;

    // coding before analysis is refused with a reason; so is running without a macro
    let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc() })).await;
    assert_eq!((r["state"].as_str(), r["error"]["sub_code"].as_str()), (Some("refused"), Some("NEEDS_ANALYSIS")), "{r}");
    for old in ["execute", "rerun_program", "repair_program"] {
        let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": old })).await;
        assert_eq!(r["state"], "refused", "{old} is gone: {r}");
    }

    // ---- analysis: bound to the view, written back by the UI's apply
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "request": { "today": "2026-10-06", "timezone": "Asia/Shanghai" } })).await;
    assert_eq!(a["state"], "waiting_confirmation", "{a}");
    assert_eq!(m.budget.lock().unwrap()["max_output_tokens"], 32_000, "every stage carries its budget");
    assert_eq!(a["candidate"]["inputs"][0]["selector"]["kind"], "table_view");
    assert!(!a["candidate"]["analysis"]["context_prompt"].as_str().unwrap().contains('@'));
    assert_eq!(a["candidate"]["analysis"]["schema_version"], "wish.analysis.v3");
    srv.apply(&a, json!({})).await;
    let w = srv.wish().await;
    assert_eq!(w["analysis"]["status"], "ready");
    let f = fresh(&srv).await;
    assert_eq!((f["needs_analysis"].as_bool(), f["needs_coding"]["reasons"][0]["code"].as_str()), (Some(false), Some("no_macro")), "{f}");
    let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "run", "location": loc() })).await;
    assert_eq!(r["error"]["sub_code"], "NEEDS_CODING", "{r}");

    // ---- coding: the macro is trial-run; its candidate is the first run's
    let e = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc(), "request": { "today": "2026-10-06" } })).await;
    assert_eq!(e["state"], "waiting_confirmation", "{e}");
    let cand = &e["candidate"];
    assert_eq!(cand["mode"], "code");
    assert_eq!(cand["results"][0]["table"]["rows"].as_array().unwrap().len(), 3);
    assert_eq!(cand["checks"].as_array().unwrap().iter().find(|c| c["id"] == "total").unwrap()["status"], "passed");
    assert_eq!(cand["checks"].as_array().unwrap().iter().find(|c| c["id"] == "cited").unwrap()["status"], "review");
    assert!(cand["program"]["object_id"].is_string(), "the macro source is staged");
    let fields: Vec<&str> = cand["program"]["manifest"]["inputs"]["sales"]["fields"].as_array().unwrap().iter().map(|f| f["field_id"].as_str().unwrap()).collect();
    assert_eq!(fields, ["month", "amount", "status"], "the manifest names the fields by id");
    assert_eq!(cand["uncited_numbers"], json!([]), "the commentary cites only facts: {cand}");
    assert_eq!(cand["model_judgment"], json!(["monthly", "commentary"]));
    assert_eq!(e["progress"]["program_runs"], 1);
    assert_eq!(m.complete_calls.load(Ordering::SeqCst), 1);
    let applied = srv.apply(&e, json!({})).await;
    assert!(applied["applied"]["commit"]["commit_id"].is_string());
    let w = srv.wish().await;
    assert_eq!(w["program"]["api_version"], 3);
    assert_eq!(w["program"]["manifest"]["inputs"]["sales"]["entity_id"], "orders");
    let monthly = w["last_run"]["result_bindings"]["results"]["monthly"]["entity_id"].as_str().unwrap().to_string();
    assert_eq!(w["last_run"]["checks"]["passed"], 1);
    assert_eq!(srv.records(&monthly).await.len(), 3);
    let line = srv.rpc("doc.list", json!({ "at": w["last_run"]["result_bindings"]["group"] })).await;
    assert_eq!(line["items"].as_array().unwrap().len(), 3, "table view + line chart + text");
    let f = fresh(&srv).await;
    assert_eq!((f["status"].as_str(), f["needs_coding"].is_null()), (Some("current"), true), "{f}");
    let body = srv.rpc("doc.read", json!({ "entity_id": "wish-q3", "as": "body" })).await;
    assert!(body.to_string().contains("reads sales: 月份, 销售额, 状态"), "the manifest is in the wish's text: {body}");

    // ---- the data changes → stale; running the macro needs no model loop, and the commentary follows the data
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "1000.00", "expect": { "rev": rev } }] }])).await;
    assert_eq!(fresh(&srv).await["status"], "stale");
    let calls = m.calls.load(Ordering::SeqCst);
    let p = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "run", "location": loc() })).await;
    assert_eq!((p["state"].as_str(), p["candidate"]["mode"].as_str()), (Some("waiting_confirmation"), Some("run")), "{p}");
    assert_eq!(m.calls.load(Ordering::SeqCst), calls, "no model loop");
    assert_eq!(m.complete_calls.load(Ordering::SeqCst), 2, "the facts changed: one new inference");
    let commentary = p["candidate"]["results"].as_array().unwrap().iter().find(|r| r["name"] == "commentary").unwrap()["markdown"].as_str().unwrap().to_string();
    let total = p["candidate"]["facts"]["total"].clone();
    assert!(commentary.contains(&total.to_string()), "the text and the table rest on the same snapshot: {commentary} / {total}");
    assert!(p["candidate"].get("program").is_none_or(|x| x.is_null()), "running does not change the macro");
    srv.apply(&p, json!({})).await;
    let f = fresh(&srv).await;
    assert_eq!(f["status"], "current", "every result was produced again: {f}");
    // the same inputs again: the AI answer comes from the cache
    let again = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "run", "location": loc() })).await;
    assert_eq!(again["state"], "waiting_confirmation");
    assert_eq!(m.complete_calls.load(Ordering::SeqCst), 2, "cached");
    assert_eq!(again["candidate"]["results"], p["candidate"]["results"], "same macro, same inputs: the same results");

    // ---- a feedback round: coding again from the stored macro, ending in a refinement on the wish
    *m.execute.lock().unwrap() = code_script(true);
    let fb = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc(), "parent_run_id": e["run_id"], "feedback": "解读里写上月份数" })).await;
    assert_eq!(fb["state"], "waiting_confirmation", "{fb}");
    srv.apply(&fb, json!({})).await;
    let w = srv.wish().await;
    assert_eq!(w["refinements"][0]["text"], "解读中给出月份数");
    assert_eq!(fresh(&srv).await["needs_analysis"], false, "host-sanctioned refinements keep the analysis valid");

    // ---- a renamed field changes nothing: the macro reads by id under the name it was coded with
    let fdef = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "amount" } })).await;
    srv.commit(json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "amount", "changes": { "name": "金额" }, "expect": { "rev": fdef["content"]["def_rev"] } }])).await;
    assert!(fresh(&srv).await["needs_coding"].is_null());
    let renamed = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "run", "location": loc() })).await;
    assert_eq!(renamed["state"], "waiting_confirmation", "{renamed}");
    assert_eq!(renamed["candidate"]["facts"]["total"], total);

    // ---- a declared field deleted: the run is refused before it starts, the wish needs coding
    let fdef = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "status" } })).await;
    srv.commit(json!([{ "op": "table.delete_field", "source_id": "orders", "field_id": "status", "expect": { "rev": fdef["content"]["def_rev"] } }])).await;
    let f = fresh(&srv).await;
    assert_eq!((f["needs_coding"]["reasons"][0]["code"].as_str(), f["needs_coding"]["reasons"][0]["field_id"].as_str()), (Some("field_deleted"), Some("status")), "{f}");
    let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "run", "location": loc() })).await;
    assert_eq!(r["error"]["sub_code"], "NEEDS_CODING", "{r}");
    assert!(r["error"]["detail"].as_str().unwrap().contains("状态"), "the field is named: {r}");
    let runs = srv.rpc("proc.list", json!({ "wish_id": "wish-q3" })).await;
    assert!(runs["runs"].as_array().unwrap().len() >= 5);
    });
}

const MAP_PROGRAM: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['反馈'] }, ai: { map: { max_items: 100 } } })
  const rows = aiws.input('sales').rows()
  const tags = await aiws.llm.map(rows.map((r) => r['反馈']), '判断情感', { type: 'string', enum: ['正面', '负面'] })
  aiws.result.columns('情感', 'sales', Object.fromEntries(rows.map((r, i) => [r._id, { 情感: tags[i] }])), { fields: { 情感: { type: 'select', options: ['正面', '负面'] } } })
}"#;

#[test]
fn llm_map_cache_mock_and_cancel() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    // an analysis with a per-item judgement result
    *m.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("submit_analysis", json!({ "analysis": { "status": "ready", "schema_version": "wish.analysis.v3", "context_prompt": "给 sales 的反馈打情感标签",
            "inputs": [{ "name": "sales", "ref": handle(&t.user, "table view \"华东订单\"") }],
            "output_contract": { "results": [{ "name": "情感", "type": "table_columns", "target": "sales" }] },
            "checks": [], "blockers": [], "warnings": [] } })),
        _ => text("好"),
    });
    *m.execute.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("run_program", json!({ "source": MAP_PROGRAM })),
        1 => call("finish", json!({ "summary": "已打标签" })),
        _ => text("完成"),
    });
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "analyze", "location": loc() })).await;
    srv.apply(&a, json!({})).await;
    let e = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc() })).await;
    assert_eq!(e["state"], "waiting_confirmation", "{e}");
    let first_map_calls = m.map_calls.load(Ordering::SeqCst);
    assert!(first_map_calls >= 1);
    assert_eq!(e["candidate"]["model_judgment"], json!(["情感"]));
    assert!(e["candidate"]["ai"]["map"]["items"].as_u64().unwrap() > 0, "{}", e["candidate"]["ai"]);
    srv.apply(&e, json!({})).await;
    let w = srv.wish().await;
    let fid = w["last_run"]["result_bindings"]["results"]["情感"]["fields"]["情感"].as_str().unwrap().to_string();
    let v = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o2", "field_id": fid } })).await;
    assert!(v["content"]["value"].is_string(), "{v}");
    // a run asks the model only for changed items: none changed → cached
    let p = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "run", "location": loc() })).await;
    assert_eq!(p["state"], "waiting_confirmation", "{p}");
    assert_eq!(m.map_calls.load(Ordering::SeqCst), first_map_calls, "all cached");
    assert_eq!(p["candidate"]["ai"]["map"]["cached"], p["candidate"]["ai"]["map"]["items"]);

    // ---- the Mock: results provided by the browser executor, planned by the service
    srv.commit(json!([{ "op": "entity.create", "entity_id": "wish-mock", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "x",
        "payload": { "prompt": "模拟", "executor": "mock", "output": { "container_id": "sf-content", "surface_id": "sf", "name": "模拟结果" } } }])).await;
    let mock = srv.run("wish.mock@1", json!({ "wish_id": "wish-mock", "stage": "run", "provided": {
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

    // ---- cancel: the executor's next call is refused, nothing is written
    m.delay_ms.store(1500, Ordering::SeqCst);
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "params": { "wish_id": "wish-q3", "stage": "analyze" }, "idempotency_key": "slow" })).await;
    let id = start["run_id"].as_str().unwrap().to_string();
    let claim = start["claim_token"].as_str().unwrap().to_string();
    let head = srv.rpc("ws.get_info", json!({})).await["head_seq"].clone();
    let (exec, cancel) = tokio::join!(srv.execute("tok-alice", &id, &claim), async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let r = srv.rpc("proc.get", json!({ "run_id": id })).await;
        assert_eq!(r["state"], "running", "{r}");
        srv.rpc("proc.cancel", json!({ "run_id": id })).await
    });
    let _ = exec;
    assert_eq!(cancel["state"], "cancelled");
    let r = srv.wait(&id).await;
    assert_eq!(r["state"], "cancelled", "a late report does not revive it: {r}");
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
        let objects = aiworkspace_store::objects::ObjectStore::embedded(&dir.path().join("pre-store"), buckyos_api::aiworkspace_bucket_decls(), None).unwrap();
        let svc = Service::open(&dir.path().join("data"), objects).unwrap();
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
fn executors_claim_report_and_the_service_plans_what_their_stage_collected() {
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
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let (attempt, run_token) = (claimed["attempt"].as_u64().unwrap(), claimed["run_token"].as_str().unwrap().to_string());
    assert_eq!(attempt, 1);
    assert_eq!(claimed["stage"]["budget"]["sql_max_calls"], 200, "{claimed}");
    // the run token speaks for the run, reads the Workspace, and nothing else
    let hb = srv.call_as(&run_token, "proc.heartbeat", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "progress": { "phase": "thinking" } })).await;
    assert_eq!(hb["result"]["ok"], true, "{hb}");
    assert_eq!(srv.call_as(&run_token, "doc.read", json!({ "workspace_id": ws, "entity_id": "orders" })).await["result"]["type_id"], "buckyos.table-source");
    assert!(srv.call_as(&run_token, "doc.commit", json!({ "workspace_id": ws, "epoch": "x", "idempotency_key": "k", "operations": [] })).await["error"].is_string(), "a run token cannot write");
    assert_eq!(srv.rpc("proc.get", json!({ "run_id": run_id })).await["progress"]["phase"], "thinking");
    // results are never reported directly: they come from the macro
    let direct = srv.call_as(&run_token, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "results": [] })).await;
    assert!(direct["result"]["error"]["detail"].as_str().unwrap_or("").contains("come from the macro"), "{direct}");
    // an analysis made elsewhere (ids instead of handles); the service validates and plans it
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "context_prompt": "读取 blk-orders 的全部记录，按月汇总销售额并让模型写解读。",
        "inputs": [{ "name": "sales", "ref": "blk-orders" }],
        "output_contract": { "results": [{ "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"] }, { "name": "commentary", "type": "richtext", "title": "解读" }] },
        "checks": [{ "id": "total", "kind": "program", "text": "合计一致" }], "blockers": [], "warnings": [] });
    let reported = srv.call_as(&run_token, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "analysis": analysis })).await;
    assert!(reported["result"]["ok"] == json!(true), "{reported}");
    let done = srv.wait(&run_id).await;
    assert_eq!(done["state"], "waiting_confirmation", "{done}");
    assert_eq!(done["candidate"]["inputs"][0]["name"], "sales", "{done}");
    srv.apply(&done, json!({})).await;
    // a coding run: a superseded attempt is refused; a changed wish is DELEGATION_STALE
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "loc-e", "params": { "wish_id": "wish-q3", "stage": "code", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let first = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    srv.execute("tok-alice", &run_id, &claim_token).await;
    let done = srv.wait(&run_id).await;
    let stale = srv.call_as(first["run_token"].as_str().unwrap(), "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    assert_eq!(stale["result"]["error"]["code"], "RUN_REVOKED", "{stale}");
    assert_eq!((done["state"].as_str(), done["candidate"]["executor"].as_str(), done["execution"]["attempt"].as_u64()), (Some("waiting_confirmation"), Some("xllm"), Some(2)), "{done}");
    let rev = srv.rpc("doc.read", json!({ "entity_id": "wish-q3" })).await["content"]["key_revs"]["prompt"].clone();
    srv.commit(json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [{ "key": "prompt", "value": "改了题目", "expect": { "rev": rev } }] }])).await;
    let preview = srv.rpc("proc.get", json!({ "run_id": run_id, "choices": {} })).await;
    let refused = srv.call_as("tok-alice", "proc.apply", json!({ "workspace_id": ws, "run_id": run_id, "plan_digest": preview["preview"]["plan_digest"] })).await;
    assert_eq!(refused["result"]["error"]["code"], "DELEGATION_STALE", "{refused}");
    // the analysis changed with the prompt: coding is refused until it is analyzed again
    let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc() })).await;
    assert_eq!(r["error"]["sub_code"], "NEEDS_ANALYSIS", "{r}");
    // hosted: the owner configures the agent; without a Zone runtime the dispatch is recorded unavailable and the run waits
    assert_eq!(srv.call_as("tok-alice", "proc.start", json!({ "workspace_id": ws, "program": "wish.xllm@1", "idempotency_key": "host-0", "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "execution": { "mode": "hosted" } } })).await["result"]["error"]["code"], "EXECUTION_CONTEXT_MISSING");
    let cfg = srv.rpc("ws.set_hosted_exec", json!({ "config": { "enabled": true, "agent_id": "jarvis.test.buckyos.io", "allowed_wishes": ["wish-q3"] } })).await;
    assert!(cfg["hosted_exec"]["digest"].is_string());
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "host-1", "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc(), "execution": { "mode": "hosted" } } })).await;
    assert_eq!((start["context"]["location"].as_str(), start["context"]["executor"]["principal"].as_str()), (Some("hosted"), Some("jarvis.test.buckyos.io")), "{start}");
    assert_eq!((start["aicc"]["mode"].as_str(), start["aicc"]["user_id"].as_str()), (Some("workspace_owner"), Some("alice")), "a hosted run uses the owner's: {start}");
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

#[test]
fn local_runs_use_the_starters_ai_hosted_runs_the_owners() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), m.clone(), json!({
        "tok-alice": { "principal": "alice" },
        "tok-bob": { "principal": "bob" },
        "tok-eve": { "principal": "did:web:eve.example.com", "class": "external" },
    })).await;
    setup(&mut srv).await;
    for who in ["bob", "did:web:eve.example.com"] {
        srv.rpc("ws.grant", json!({ "subject": who, "capabilities": ["read", "comment", "update"] })).await;
    }
    assert!(srv.rpc("ws.get_info", json!({})).await.get("wish_aicc_mode").is_none(), "no workspace setting: where it runs decides");
    assert_eq!(srv.rpc("ws.get_info", json!({})).await["wish_execution"], json!({ "default": "local" }));
    // a collaborator's local run is the collaborator's own
    let done = srv.run_as("tok-bob", "wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "analyze", "location": loc() })).await;
    assert_eq!(done["state"], "waiting_confirmation", "{done}");
    assert_eq!(done["aicc"], json!({ "mode": "current_user", "user_id": "bob", "operator_id": "bob" }));
    // an External user has no AI resources here and is not moved to the owner's
    let refused = srv.call_as("tok-eve", "proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "e1",
        "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc() } })).await;
    assert_eq!(refused["result"]["error"]["sub_code"], "AI_ATTRIBUTION_UNAVAILABLE", "{refused}");
    let gone = srv.call_as("tok-alice", "ws.set_wish_aicc", json!({ "mode": "current_user" })).await;
    assert!(!gone["error"].is_null() || gone["result"]["error"].is_object(), "the setting is gone: {gone}");
    // the executor's own model is refused: it reports the refusal, the run fails with its sub-code
    srv.apply(&done, json!({})).await;
    *m.refuse.lock().unwrap() = Some("AI_QUOTA_EXHAUSTED");
    let failed = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc() })).await;
    assert_eq!((failed["state"].as_str(), failed["error"]["code"].as_str()), (Some("failed"), Some("AI_QUOTA_EXHAUSTED")), "{failed}");
    // a macro's own AI call refused while running: the run fails with that sub-code, not as a broken program
    *m.refuse.lock().unwrap() = None;
    let e = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc() })).await;
    srv.apply(&e, json!({})).await;
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "999.00", "expect": { "rev": rev } }] }])).await;
    *m.refuse.lock().unwrap() = Some("AI_QUOTA_EXHAUSTED");
    let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "run", "location": loc() })).await;
    assert_eq!((r["state"].as_str(), r["error"]["sub_code"].as_str()), (Some("failed"), Some("AI_QUOTA_EXHAUSTED")), "{r}");
    });
}

#[test]
fn an_executor_works_on_the_services_stage_through_proc_tool_and_proc_program() {
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
    let tool = |name: &str, args: Value| json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": name, "args": args });
    let profile = srv.call_as(&rt, "proc.tool", tool("ws_profile", json!({ "target": b }))).await;
    assert_eq!(profile["result"]["status"], "ok", "{profile}");
    assert!(profile["result"]["text"].as_str().unwrap().contains("销售额"));
    let sql = srv.call_as(&rt, "proc.tool", tool("sql", json!({ "query": "SELECT 1" }))).await;
    assert_eq!(sql["result"]["status"], "error", "aiws.sql is for macros: {sql}");
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready",
        "context_prompt": format!("读取 {b} 的全部记录，排除 状态=未结算，按月汇总销售额与订单数；宏让模型按 facts 写解读。"),
        "inputs": [{ "name": "sales", "ref": b }],
        "output_contract": { "results": [
            { "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"], "views": [{ "renderer": "table" }] },
            { "name": "dash", "type": "html", "title": "看板" }] },
        "checks": [{ "id": "total", "kind": "program", "text": "月度合计等于已结算合计" }], "blockers": [], "warnings": [] });
    let ok = srv.call_as(&rt, "proc.tool", tool("submit_analysis", json!({ "analysis": analysis }))).await;
    assert_eq!(ok["result"]["status"], "ok", "{ok}");
    let reported = srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    assert_eq!(reported["result"]["ok"], true, "{reported}");
    let done = srv.wait(&run_id).await;
    assert_eq!((done["state"].as_str(), done["candidate"]["inputs"][0]["name"].as_str()), (Some("waiting_confirmation"), Some("sales")), "{done}");
    assert_eq!(m.calls.load(Ordering::SeqCst), 0, "the service called no model");
    srv.apply(&done, json!({})).await;

    // coding: the executor runs the macro on the snapshot it fetches; aiws.sql sees the declared inputs only
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "host-e", "params": { "wish_id": "wish-q3", "stage": "code", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let stage = &claimed["stage"];
    assert!(stage["user"].as_str().unwrap().contains("## Output contract") && stage["source"].is_null(), "{claimed}");
    assert!(stage["system"].as_str().unwrap().contains("# Writing a wish macro (aiws v3)"), "the coding prompt embeds the guide");
    let names: Vec<&str> = stage["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"run_program") && !names.contains(&"put_result"), "{names:?}");
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    let tool = |name: &str, args: Value| json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": name, "args": args });
    let files = srv.call_as(&rt, "proc.files", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1 })).await;
    let paths: Vec<&String> = files["result"]["files"].as_object().unwrap().keys().collect();
    assert!(paths.iter().any(|k| k.ends_with("rows.jsonl")) && paths.iter().any(|k| *k == "lib/host.js"), "{paths:?}");
    assert!(!paths.iter().any(|k| k.ends_with("data-tree.json") || k.ends_with("block-tree.json")), "no workspace tree in a snapshot: {paths:?}");
    let declare = json!({ "inputs": { "sales": ["月份", "销售额"] } });
    let q = srv.call_as(&rt, "proc.tool", tool("sql", json!({ "query": "SELECT \"月份\", count(*) AS n FROM sales GROUP BY 1 ORDER BY 1", "declare": declare }))).await;
    assert_eq!(q["result"]["status"], "ok", "{q}");
    assert_eq!(q["result"]["sql"]["rows"].as_array().unwrap().len(), 3, "a view input is the rows its view shows: {q}");
    let cols = srv.call_as(&rt, "proc.tool", tool("sql", json!({ "query": "SELECT * FROM sales LIMIT 1", "declare": declare }))).await;
    assert_eq!(cols["result"]["sql"]["columns"], json!(["_id", "月份", "月份#raw", "销售额", "销售额#raw"]), "only the declared fields: {cols}");
    for bad in ["SELECT * FROM \"/data/订单\"", "SELECT * FROM table_records", "SELECT * FROM main.entities", "DELETE FROM sales"] {
        let r = srv.call_as(&rt, "proc.tool", tool("sql", json!({ "query": bad, "declare": declare }))).await;
        assert_eq!(r["result"]["status"], "error", "{bad}: {r}");
    }
    let undeclared = srv.call_as(&rt, "proc.tool", tool("sql", json!({ "query": "SELECT 1", "declare": { "inputs": { "sales": ["不存在"] } } }))).await;
    assert_eq!(undeclared["result"]["code"], "NOT_DECLARED", "{undeclared}");
    let key = "a".repeat(64);
    let put = srv.call_as(&rt, "proc.tool", tool("llm_cache", json!({ "put": [{ "key": key, "value": { "text": "x" } }] }))).await;
    assert_eq!(put["result"]["status"], "ok", "{put}");
    let got = srv.call_as(&rt, "proc.tool", tool("llm_cache", json!({ "get": [key, "b".repeat(64)] }))).await;
    assert_eq!(got["result"]["hits"], json!({ key.clone(): { "text": "x" } }));
    assert_eq!(srv.call_as(&rt, "proc.tool", tool("llm_cache", json!({ "get": ["../x"] }))).await["result"]["status"], "error");
    // an HTML result: its code is part of the macro; data goes in through bindings
    let html = "<div id=n></div>";
    let output = json!({ "results": [
            { "name": "monthly", "type": "table", "rows": [{ "月份": "2026-07-01", "销售额": 10.5 }], "fields": [{ "name": "月份", "type": "date" }, { "name": "销售额", "type": "decimal" }], "key": ["月份"] },
            { "name": "dash", "type": "html", "html": html, "css": "", "js": "aiws.input('t')", "bindings": { "t": "result:monthly" } }],
        "facts": {}, "checks": [{ "id": "total", "passed": true, "detail": null }], "external": [], "ai": null, "declare": declare, "error": null });
    let ran = srv.call_as(&rt, "proc.program", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "source": PROGRAM, "results": output, "log": "", "files": { "output/note.txt": "hello" } })).await;
    assert_eq!(ran["result"]["status"], "ok", "{ran}");
    assert!(ran["result"]["text"].as_str().unwrap().contains("reads sales: 月份, 销售额"), "{ran}");
    let fin = srv.call_as(&rt, "proc.tool", tool("finish", json!({ "summary": "done" }))).await;
    assert_eq!(fin["result"]["status"], "ok", "{fin}");
    srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    let done = srv.wait(&run_id).await;
    assert_eq!((done["state"].as_str(), done["candidate"]["results"].as_array().map(Vec::len)), (Some("waiting_confirmation"), Some(2)), "{done}");
    let digest = done["candidate"]["program"]["manifest"]["results"]["dash"]["code_digest"].as_str().unwrap().to_string();
    assert_eq!(digest.len(), 64);
    srv.apply(&done, json!({})).await;
    let w = srv.wish().await;
    let def = w["last_run"]["result_bindings"]["results"]["dash"]["entity_id"].as_str().unwrap().to_string();
    let version = srv.rpc("doc.read", json!({ "entity_id": def })).await["content"]["payload"]["version"].clone();

    // running: the stored macro only; other code, or HTML code other than the coded one, ends the run
    let run = |key: &'static str| srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": key, "params": { "wish_id": "wish-q3", "stage": "run", "location": loc(), "execution": { "mode": "local" } } }));
    let start = run("host-r1").await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    assert_eq!((claimed["stage"]["kind"].as_str(), claimed["stage"]["manifest"]["results"]["dash"]["code_digest"].as_str()), (Some("program"), Some(digest.as_str())), "{claimed}");
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    let other = srv.call_as(&rt, "proc.program", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "source": "export default async function main() {}", "results": output, "log": "", "files": {} })).await;
    assert_eq!(other["result"]["code"], "MACRO_CODE_CHANGED", "{other}");
    let mut changed = output.clone();
    changed["results"][1]["html"] = json!("<div id=n>10.5</div>");
    let ran = srv.call_as(&rt, "proc.program", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "source": PROGRAM, "results": changed, "log": "", "files": {} })).await;
    assert_eq!(ran["result"]["code"], "MACRO_CODE_CHANGED", "{ran}");
    srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    let failed = srv.wait(&run_id).await;
    assert_eq!((failed["state"].as_str(), failed["error"]["sub_code"].as_str()), (Some("failed"), Some("MACRO_CODE_CHANGED")), "{failed}");
    // the same code with new data: only the data moves, the definition keeps its version
    let start = run("host-r2").await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    let mut fresh_data = output.clone();
    fresh_data["results"][0]["rows"] = json!([{ "月份": "2026-07-01", "销售额": 99.5 }]);
    let ran = srv.call_as(&rt, "proc.program", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "source": PROGRAM, "results": fresh_data, "log": "", "files": {} })).await;
    assert_eq!(ran["result"]["status"], "ok", "{ran}");
    srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    let done = srv.wait(&run_id).await;
    assert_eq!(done["state"], "waiting_confirmation", "{done}");
    srv.apply(&done, json!({})).await;
    assert_eq!(srv.rpc("doc.read", json!({ "entity_id": def })).await["content"]["payload"]["version"], version, "the control definition did not change");

    // cancelled under the executor: its tools answer cancelled and its report is refused
    let start = run("host-c").await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let rt = claimed["run_token"].as_str().unwrap().to_string();
    srv.rpc("proc.cancel", json!({ "run_id": run_id })).await;
    let refused = srv.call_as(&rt, "proc.tool", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "name": "check_results", "args": {} })).await;
    assert_eq!(refused["result"]["error"]["code"], "RUN_REVOKED", "{refused}");
    let again = srv.rpc_raw("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    assert_eq!(again["result"]["error"]["code"], "RUN_REVOKED", "{again}");
    });
}
