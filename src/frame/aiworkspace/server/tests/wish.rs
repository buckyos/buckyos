//! 许愿格 end to end on the service (in process, real HTTP, real Deno), with a test executor playing the browser and a
//! scripted model that reads what the host gives it (许愿格交互升级): every stage writes when it ends — the analysis,
//! the macro (its trial run only into its record), the results — each with a record in the wish's `history`; nothing
//! waits for a confirmation. Running only the macro (no model loop) after the data changed; a renamed field changes
//! nothing, a deleted one asks for coding again; a repair round; inputs found missing; check levels; a hand edit
//! overwritten and rolled back; macro and analysis rollbacks; the history's bounds and objects; `llm.map` with its cache;
//! cancellation; the executor protocol; a restart that reports the run interrupted.

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
  aiws.check('total', Math.abs(direct - total) < 0.01 && aiws.request.fail !== 'total', { total, direct })
  aiws.check('soft', aiws.request.fail !== 'soft', {})
  console.log('months', rows.length)
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
                "checks": [{ "id": "total", "kind": "program", "level": "error", "text": "月度合计等于已结算合计" },
                           { "id": "soft", "kind": "program", "level": "warning", "text": "数据足够新" },
                           { "id": "cited", "kind": "review", "text": "数字可追溯" }],
                "blockers": [], "warnings": ["已按便签排除未结算订单"], "summary": "按月汇总华东订单并写解读" } }))
        }
        _ => text("分析完成。"),
    })
}

fn code_script(feedback: bool) -> Script {
    code_with(PROGRAM.to_string(), feedback)
}

fn code_with(source: String, feedback: bool) -> Script {
    Box::new(move |t: &Turn| match t.results.len() {
        0 => call("run_program", json!({ "source": source })),
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

fn stage(name: &str) -> Value {
    json!({ "wish_id": "wish-q3", "stage": name, "location": loc(), "request": { "today": "2026-10-06" } })
}

fn with_request(mut p: Value, request: Value) -> Value {
    p["request"] = request;
    p
}

async fn fresh(srv: &Srv) -> Value {
    srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-q3"] })).await["items"][0].clone()
}

async fn object(srv: &Srv, id: &Value) -> Vec<u8> {
    srv.asset_bytes(&srv.ws, id.as_str().unwrap_or_else(|| panic!("no object id: {id}"))).await
}

/// The live record values of a table, by field id.
async fn values(srv: &Srv, table: &str) -> Vec<Value> {
    srv.records(table).await.iter().map(|r| r["values"].clone()).collect()
}

async fn cell_value(srv: &Srv, table: &str, record: &str, field: &str) -> Value {
    srv.rpc("doc.read", json!({ "entity_id": table, "selector": { "kind": "table_cell", "record_id": record, "field_id": field } })).await["content"]["value"].clone()
}

/// Analyze, code, run: three runs that wrote.
async fn through(srv: &Srv) -> (Value, Value, Value) {
    let a = srv.run("wish.xllm@1", stage("analyze")).await;
    srv.done(&a);
    let e = srv.run("wish.xllm@1", stage("code")).await;
    srv.done(&e);
    let r = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&r);
    (a, e, r)
}

#[test]
fn each_stage_writes_when_it_ends_with_its_record() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;

    // coding before analysis is refused with a reason; so is running without a macro
    let r = srv.run("wish.xllm@1", stage("code")).await;
    assert_eq!((r["state"].as_str(), r["error"]["sub_code"].as_str()), (Some("refused"), Some("NEEDS_ANALYSIS")), "{r}");
    for old in ["execute", "rerun_program", "repair_program"] {
        let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": old })).await;
        assert_eq!(r["state"], "refused", "{old} is gone: {r}");
    }
    let r = srv.run("wish.mock@1", json!({ "wish_id": "wish-q3", "stage": "run", "provided": {} })).await;
    assert_eq!(r["state"], "refused", "the Mock executor is gone: {r}");
    let gone = srv.rpc_raw("proc.apply", json!({ "run_id": "run_x", "plan_digest": "d" })).await;
    assert!(gone["error"].is_string() || gone["result"]["error"].is_object(), "proc.apply is gone: {gone}");

    // ---- analysis: bound to the view and written as it ends, with its record
    let a = srv.run("wish.xllm@1", with_request(stage("analyze"), json!({ "today": "2026-10-06", "timezone": "Asia/Shanghai" }))).await;
    srv.done(&a);
    for gone in ["candidate", "preview", "simulated"] {
        assert!(a.get(gone).is_none(), "the run view has no {gone}: {a}");
    }
    assert_eq!(m.budget.lock().unwrap()["max_output_tokens"], 32_000, "every stage carries its budget");
    let w = srv.wish().await;
    assert_eq!((w["analysis"]["status"].as_str(), w["analysis"]["run_id"].as_str()), (Some("ready"), a["run_id"].as_str()));
    assert_eq!(w["inputs"][0]["selector"]["kind"], "table_view");
    assert!(!w["analysis"]["context_prompt"].as_str().unwrap().contains('@'));
    assert_eq!(w["analysis"]["checks"][0]["level"], "error");
    let rec = &a["record"];
    assert_eq!(&w["history"]["analyze"][0], rec, "the run's record is the wish's");
    assert_eq!((rec["stage"].as_str(), rec["action"].as_str(), rec["ok"].as_bool(), rec["by"].as_str()), (Some("analyze"), Some("analyze"), Some(true), Some("alice")), "{rec}");
    assert_eq!((rec["summary"]["results"].as_u64(), rec["summary"]["checks"].as_u64(), rec["summary"]["text"].as_str()), (Some(2), Some(3), Some("按月汇总华东订单并写解读")), "{rec}");
    assert_eq!(rec["warnings"], json!(["已按便签排除未结算订单"]));
    assert_eq!((rec["inputs"][0]["entity_id"].as_str(), rec["inputs"][0]["name"].as_str()), (Some("orders"), Some("sales")), "{rec}");
    assert!(rec["inputs"][0]["version"]["rev"].is_u64(), "{rec}");
    assert_eq!(rec["aicc"], json!({ "mode": "current_user", "user_id": "alice" }));
    assert_eq!((rec["usage"]["calls"].as_u64(), rec["usage"]["input_tokens"].as_u64(), rec["usage"]["model"].is_string()), (Some(3), Some(300), true), "the executor's usage: {rec}");
    assert!(rec["duration_ms"].is_u64() && rec["commit_id"] == a["applied"]["commit"]["commit_id"], "{rec}");
    assert_eq!(String::from_utf8(object(&srv, &rec["objects"]["prompt"]).await).unwrap(), "分析左边那张只显示华东的表，按月汇总销售额并写解读");
    let contract: Value = serde_json::from_slice(&object(&srv, &rec["objects"]["analysis"]).await).unwrap();
    assert_eq!(contract["analysis"], w["analysis"], "the task contract as the wish keeps it");
    let f = fresh(&srv).await;
    assert_eq!((f["needs_analysis"].as_bool(), f["needs_coding"]["reasons"][0]["code"].as_str()), (Some(false), Some("no_macro")), "{f}");
    let r = srv.run("wish.xllm@1", stage("run")).await;
    assert_eq!(r["error"]["sub_code"], "NEEDS_CODING", "{r}");

    // ---- coding: the macro is trial-run and written; its trial results only go into the record (D1)
    let e = srv.run("wish.xllm@1", stage("code")).await;
    srv.done(&e);
    assert_eq!(e["progress"]["program_runs"], 1);
    assert_eq!(m.complete_calls.load(Ordering::SeqCst), 1);
    let w = srv.wish().await;
    assert_eq!(w["program"]["api_version"], 3);
    assert_eq!(w["program"]["manifest"]["inputs"]["sales"]["entity_id"], "orders");
    let fields: Vec<&str> = w["program"]["manifest"]["inputs"]["sales"]["fields"].as_array().unwrap().iter().map(|f| f["field_id"].as_str().unwrap()).collect();
    assert_eq!(fields, ["month", "amount", "status"], "the manifest names the fields by id");
    assert!(w.get("last_run").is_none(), "no results yet: {}", w["last_run"]);
    let rec = &e["record"];
    assert_eq!((rec["stage"].as_str(), rec["action"].as_str()), (Some("code"), Some("code")));
    assert_eq!(rec["summary"]["trial"], json!({ "results": 2, "checks_passed": 2, "checks_total": 2 }), "{rec}");
    assert_eq!(rec["basis"]["analysis"], a["run_id"]);
    assert_eq!(rec["outputs"]["program"], json!({ "digest": w["program"]["digest"], "ai": true }));
    let levels: Vec<(&str, Option<&str>, &str)> = rec["checks"].as_array().unwrap().iter().map(|c| (c["id"].as_str().unwrap(), c["level"].as_str(), c["status"].as_str().unwrap())).collect();
    assert_eq!(levels, [("total", Some("error"), "passed"), ("soft", Some("warning"), "passed"), ("cited", None, "review")]);
    assert_eq!(rec["objects"]["source"], w["program"]["source"]);
    let program: Value = serde_json::from_slice(&object(&srv, &rec["objects"]["program"]).await).unwrap();
    assert_eq!(program, w["program"], "the macro as the wish keeps it");
    assert!(String::from_utf8(object(&srv, &rec["objects"]["log"]).await).unwrap().contains("months 3"), "the trial run's log");
    let f = fresh(&srv).await;
    assert!(f["needs_coding"].is_null(), "{f}");
    let body = srv.rpc("doc.read", json!({ "entity_id": "wish-q3", "as": "body" })).await;
    assert!(body.to_string().contains("reads sales: 月份, 销售额, 状态"), "the manifest is in the wish's text: {body}");

    // ---- running: the results are written, the macro's AI calls are its usage
    let calls = m.calls.load(Ordering::SeqCst);
    let p = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&p);
    assert_eq!(m.calls.load(Ordering::SeqCst), calls, "no model loop");
    assert_eq!(m.complete_calls.load(Ordering::SeqCst), 1, "the same facts as the trial run: its answer is cached");
    let w = srv.wish().await;
    assert_eq!(w["last_run"]["run_id"], p["run_id"]);
    let monthly = w["last_run"]["result_bindings"]["results"]["monthly"]["entity_id"].as_str().unwrap().to_string();
    let commentary = w["last_run"]["result_bindings"]["results"]["commentary"]["entity_id"].as_str().unwrap().to_string();
    assert_eq!(w["last_run"]["checks"]["passed"], 2);
    assert_eq!(srv.records(&monthly).await.len(), 3);
    let line = srv.rpc("doc.list", json!({ "at": w["last_run"]["result_bindings"]["group"] })).await;
    assert_eq!(line["items"].as_array().unwrap().len(), 3, "table view + line chart + text");
    let rec = &p["record"];
    assert_eq!((rec["stage"].as_str(), rec["action"].as_str(), rec["summary"]["created"].as_u64(), rec["summary"]["updated"].as_u64()), (Some("run"), Some("run"), Some(2), Some(0)), "{rec}");
    let names: Vec<(&str, &str, &str)> = rec["outputs"]["results"].as_array().unwrap().iter().map(|r| (r["name"].as_str().unwrap(), r["entity_id"].as_str().unwrap(), r["action"].as_str().unwrap())).collect();
    assert_eq!(names, [("monthly", monthly.as_str(), "create"), ("commentary", commentary.as_str(), "create")]);
    assert_eq!(rec["outputs"]["results"][0]["cells"].as_array().map(Vec::len), Some(2), "{rec}");
    assert_eq!((rec["basis"]["program"].as_str(), rec["basis"]["program_record"].as_str()), (w["program"]["digest"].as_str(), e["run_id"].as_str()));
    assert!(rec.get("usage").is_none(), "a cached answer is no AI call: {rec}");
    assert!(rec["warnings"].as_array().unwrap().is_empty(), "{rec}");
    assert_eq!(w["history"]["results_count"], 1);
    assert_eq!(fresh(&srv).await["status"], "current");

    // ---- the data changes → stale; running the macro needs no model loop, and the commentary follows the data
    let before = srv.rpc("doc.read", json!({ "entity_id": commentary, "as": "body" })).await["text"].clone();
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "1000.00", "expect": { "rev": rev } }] }])).await;
    assert_eq!(fresh(&srv).await["status"], "stale");
    let p2 = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&p2);
    assert_eq!(m.calls.load(Ordering::SeqCst), calls, "no model loop");
    assert_eq!(m.complete_calls.load(Ordering::SeqCst), 2, "the facts changed: one new inference");
    assert_eq!(p2["record"]["usage"], json!({ "calls": 1, "input_tokens": 0, "output_tokens": 10 }), "the macro used the AI once: {}", p2["record"]);
    assert_ne!(srv.rpc("doc.read", json!({ "entity_id": commentary, "as": "body" })).await["text"], before, "the text follows the data");
    assert_eq!((p2["record"]["summary"]["created"].as_u64(), p2["record"]["summary"]["updated"].as_u64()), (Some(0), Some(2)));
    assert!(srv.wish().await["program"]["digest"] == w["program"]["digest"], "running does not change the macro");
    assert_eq!(fresh(&srv).await["status"], "current", "every result was produced again");
    // the same inputs again: the AI answer comes from the cache, the results stay
    let rows = values(&srv, &monthly).await;
    let again = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&again);
    assert_eq!(m.complete_calls.load(Ordering::SeqCst), 2, "cached");
    assert_eq!(values(&srv, &monthly).await, rows, "same macro, same inputs: the same results");
    assert!(again["record"].get("usage").is_none(), "no AI call: no usage: {}", again["record"]);
    assert_eq!(srv.wish().await["history"]["results_count"], 3);

    // ---- a repair round: coding again from the parent's macro with feedback, ending in a refinement on the wish
    *m.execute.lock().unwrap() = code_script(true);
    let fb = srv.run("wish.xllm@1", json!({ "wish_id": "wish-q3", "stage": "code", "location": loc(), "parent_run_id": e["run_id"], "feedback": "解读里写上月份数" })).await;
    srv.done(&fb);
    assert_eq!(fb["record"]["action"], "repair");
    let w = srv.wish().await;
    assert_eq!(w["refinements"][0]["text"], "解读中给出月份数");
    assert_eq!(fresh(&srv).await["needs_analysis"], false, "host-sanctioned refinements keep the analysis valid");

    // ---- a renamed field changes nothing: the macro reads by id under the name it was coded with
    let fdef = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "amount" } })).await;
    srv.commit(json!([{ "op": "table.update_field", "source_id": "orders", "field_id": "amount", "changes": { "name": "金额" }, "expect": { "rev": fdef["content"]["def_rev"] } }])).await;
    assert!(fresh(&srv).await["needs_coding"].is_null());
    let renamed = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&renamed);
    assert_eq!(values(&srv, &monthly).await, rows);

    // ---- a declared field deleted: the run is refused before it starts, the wish needs coding
    let fdef = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "status" } })).await;
    srv.commit(json!([{ "op": "table.delete_field", "source_id": "orders", "field_id": "status", "expect": { "rev": fdef["content"]["def_rev"] } }])).await;
    let f = fresh(&srv).await;
    assert_eq!((f["needs_coding"]["reasons"][0]["code"].as_str(), f["needs_coding"]["reasons"][0]["field_id"].as_str()), (Some("field_deleted"), Some("status")), "{f}");
    let r = srv.run("wish.xllm@1", stage("run")).await;
    assert_eq!(r["error"]["sub_code"], "NEEDS_CODING", "{r}");
    assert!(r["error"]["detail"].as_str().unwrap().contains("状态"), "the field is named: {r}");
    let runs = srv.rpc("proc.list", json!({ "wish_id": "wish-q3" })).await;
    assert!(runs["runs"].as_array().unwrap().len() >= 5);
    let h = srv.wish().await["history"].clone();
    assert_eq!((h["analyze"].as_array().map(Vec::len), h["code"].as_array().map(Vec::len), h["run"].as_array().map(Vec::len)), (Some(1), Some(2), Some(4)), "{h}");
    });
}

#[test]
fn an_analysis_that_finds_no_input_fails_and_names_it() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    *m.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("submit_analysis", json!({ "analysis": { "schema_version": "wish.analysis.v3", "status": "needs_input", "context_prompt": "按成本表算利润率",
            "inputs": [], "output_contract": { "results": [] }, "checks": [], "warnings": [],
            "blockers": [{ "code": "NEEDS_INPUT", "message": "没有找到成本表", "input_label": "成本", "candidates": [handle(&t.user, "table view \"华东订单\"")] }] } })),
        _ => text("缺输入。"),
    });
    let a = srv.run("wish.xllm@1", stage("analyze")).await;
    assert_eq!((a["state"].as_str(), a["error"]["sub_code"].as_str()), (Some("failed"), Some("NEEDS_INPUT")), "{a}");
    let w = srv.wish().await;
    assert!(w.get("analysis").is_none(), "nothing written back: {w}");
    let rec = &a["record"];
    assert_eq!(&w["history"]["analyze"][0], rec);
    assert_eq!((rec["ok"].as_bool(), rec["error"]["sub_code"].as_str()), (Some(false), Some("NEEDS_INPUT")), "{rec}");
    assert_eq!(rec["error"]["missing_inputs"], json!([{ "label": "成本", "message": "没有找到成本表", "candidates": ["blk-orders"] }]), "{rec}");
    assert!(rec["objects"]["prompt"].is_string() && rec.get("commit_id").is_none(), "{rec}");
    // no needs_input state: the analysis is still to be made
    let f = fresh(&srv).await;
    assert_eq!(f["needs_analysis"], true, "{f}");
    let r = srv.run("wish.xllm@1", stage("code")).await;
    assert_eq!(r["error"]["sub_code"], "NEEDS_ANALYSIS", "{r}");
    });
}

#[test]
fn check_levels_decide_what_is_written() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    srv.done(&srv.run("wish.xllm@1", stage("analyze")).await);
    // coding: an error-level check that still fails in the last trial run keeps no macro
    let e = srv.run("wish.xllm@1", with_request(stage("code"), json!({ "fail": "total" }))).await;
    assert_eq!((e["state"].as_str(), e["error"]["sub_code"].as_str()), (Some("failed"), Some("CHECKS_FAILED")), "{e}");
    assert!(srv.wish().await.get("program").is_none(), "no macro kept");
    let rec = &e["record"];
    assert_eq!(rec["checks"][0], json!({ "id": "total", "kind": "program", "level": "error", "status": "failed", "text": "月度合计等于已结算合计" }), "{rec}");
    assert!(rec["objects"]["log"].is_string(), "{rec}");
    srv.done(&srv.run("wish.xllm@1", stage("code")).await);
    // running: a failed warning-level check is written and said first
    let p = srv.run("wish.xllm@1", with_request(stage("run"), json!({ "fail": "soft" }))).await;
    srv.done(&p);
    assert_eq!(p["record"]["warnings"][0], json!({ "code": "check_failed", "id": "soft", "text": "数据足够新" }), "{}", p["record"]);
    let w = srv.wish().await;
    let monthly = w["last_run"]["result_bindings"]["results"]["monthly"]["entity_id"].as_str().unwrap().to_string();
    let rows = values(&srv, &monthly).await;
    // a failed error-level check fails the run: the results stay as they were
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "1000.00", "expect": { "rev": rev } }] }])).await;
    let r = srv.run("wish.xllm@1", with_request(stage("run"), json!({ "fail": "total" }))).await;
    assert_eq!((r["state"].as_str(), r["error"]["sub_code"].as_str()), (Some("failed"), Some("CHECKS_FAILED")), "{r}");
    let w2 = srv.wish().await;
    assert_eq!(w2["last_run"], w["last_run"], "nothing written");
    assert_eq!(values(&srv, &monthly).await, rows);
    assert_eq!((w2["history"]["run"][0]["ok"].as_bool(), w2["history"]["results_count"].as_u64()), (Some(false), Some(1)), "a failure is recorded, not counted");
    });
}

#[test]
fn a_hand_edit_is_overwritten_and_a_run_is_rolled_back() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    let (_, _, r1) = through(&srv).await;
    let w = srv.wish().await;
    let b = w["last_run"]["result_bindings"].clone();
    let monthly = b["results"]["monthly"]["entity_id"].as_str().unwrap().to_string();
    let group = b["group"].as_str().unwrap().to_string();
    let first = srv.records(&monthly).await[0].clone();
    let (rid, fid) = (first["record_id"].as_str().unwrap().to_string(), b["results"]["monthly"]["fields"]["订单数"].as_str().unwrap().to_string());
    let generated = first["values"][&fid].clone();
    // a hand edit of a result; the next run overwrites it and says so first (D3)
    let rev = srv.rpc("doc.read", json!({ "entity_id": monthly, "selector": { "kind": "table_cell", "record_id": rid, "field_id": fid } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": monthly, "values": [{ "record_id": rid, "field_id": fid, "value": 99, "expect": { "rev": rev } }] }])).await;
    let r2 = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&r2);
    assert_eq!(r2["record"]["warnings"][0], json!({ "code": "overwrote_manual", "name": "monthly" }), "{}", r2["record"]);
    assert_eq!(cell_value(&srv, &monthly, &rid, &fid).await, generated);
    // only the current record rolls back
    let refused = srv.rollback(&r1["run_id"], false).await;
    assert_eq!((refused["result"]["error"]["code"].as_str(), refused["result"]["error"]["sub_code"].as_str()), (Some("INVALID_OPERATION"), Some("NOT_CURRENT_RECORD")), "{refused}");
    // rolling r2 back brings the hand edit back and the last run to r1
    let rb = srv.rollback(&r2["run_id"], false).await["result"].clone();
    assert_eq!(rb["status"], "accepted", "{rb}");
    assert!(rb["commit"]["commit_id"].is_string());
    assert_eq!(cell_value(&srv, &monthly, &rid, &fid).await, json!(99), "the hand edit is back");
    let w = srv.wish().await;
    assert_eq!(w["last_run"]["run_id"], r1["run_id"]);
    let runs = w["history"]["run"].as_array().unwrap();
    assert_eq!((runs[0]["action"].as_str(), runs[0]["rollback_of"].as_str(), runs[0]["summary"]["of"].as_str()), (Some("rollback"), r2["run_id"].as_str(), r2["run_id"].as_str()), "{}", runs[0]);
    assert_eq!(runs[0], rb["record"]);
    assert_eq!((runs[1]["id"].as_str(), runs[1]["rolled_back"]["id"].as_str()), (r2["run_id"].as_str(), runs[0]["id"].as_str()), "{}", runs[1]);
    assert_eq!(w["history"]["results_count"], 2, "a rollback takes nothing from the count");
    // r1 is current now; the hand edit came after it: the rollback names what it would discard first
    let ask = srv.rollback(&r1["run_id"], false).await["result"].clone();
    assert_eq!(ask["status"], "needs_confirm", "{ask}");
    assert_eq!((ask["lost"][0]["entity_id"].as_str(), ask["lost"][0]["what"].as_str(), ask["hidden_lost"].as_u64()), (Some(monthly.as_str()), Some("edited"), Some(0)), "{ask}");
    assert!(srv.rpc("doc.read", json!({ "entity_id": monthly })).await.get("type_id").is_some(), "nothing written yet");
    let rb = srv.rollback(&r1["run_id"], true).await["result"].clone();
    assert_eq!(rb["status"], "accepted", "{rb}");
    // what r1 created is gone, its Blocks included; the wish has no results again
    for gone in [&monthly, &group] {
        let r = srv.rpc_raw("doc.read", json!({ "entity_id": gone })).await;
        assert!(r["result"]["error"].is_object() || r["error"].is_string(), "{gone}: {r}");
    }
    let w = srv.wish().await;
    assert!(w.get("last_run").is_none(), "{}", w["last_run"]);
    assert_eq!(w["history"]["run"][0]["warnings"][0], json!({ "code": "discarded_edits", "count": 1 }));
    let again = srv.rollback(&r1["run_id"], true).await;
    assert_eq!(again["result"]["error"]["sub_code"], "NOT_CURRENT_RECORD", "{again}");
    });
}

#[test]
fn a_rolled_back_run_restores_its_rich_text_result() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    let (_, _, _) = through(&srv).await;
    let w = srv.wish().await;
    let commentary = w["last_run"]["result_bindings"]["results"]["commentary"]["entity_id"].as_str().unwrap().to_string();
    let before = srv.rpc("doc.read", json!({ "entity_id": commentary })).await["content"].clone();
    // new data: the next run writes another commentary (its blocks replaced in the run's commit)
    srv.commit(json!([{ "op": "table.insert_records", "source_id": "orders", "records": [{ "record_id": "o30", "values": {
        "month": "2026-09-01", "customer": "客户9", "region": "east", "amount": "900.50", "status": "done", "note": "服务很好" } }] }])).await;
    let r2 = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&r2);
    let after = srv.rpc("doc.read", json!({ "entity_id": commentary })).await["content"].clone();
    assert_ne!(after, before, "the second run rewrote the commentary");
    // the rollback replays the run's inverse as an internal Commit: rich text blocks take `expect: "any"` too
    let rb = srv.rollback(&r2["run_id"], false).await["result"].clone();
    assert_eq!(rb["status"], "accepted", "{rb}");
    let back = srv.rpc("doc.read", json!({ "entity_id": commentary })).await["content"].clone();
    assert!(before["content"].is_object(), "{before}");
    assert_ne!(after["content"], before["content"]);
    assert_eq!(back["content"], before["content"], "the commentary is the first run's again");
    });
}

#[test]
fn a_macro_and_an_analysis_roll_back_to_the_previous_record() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    let a1 = srv.run("wish.xllm@1", stage("analyze")).await;
    srv.done(&a1);
    let c1 = srv.run("wish.xllm@1", stage("code")).await;
    srv.done(&c1);
    let p1 = srv.wish().await["program"].clone();
    *m.execute.lock().unwrap() = code_with(format!("{PROGRAM}\n// v2\n"), false);
    let c2 = srv.run("wish.xllm@1", stage("code")).await;
    srv.done(&c2);
    assert_ne!(srv.wish().await["program"]["digest"], p1["digest"]);
    // the macro back to the previous coding's; then to none
    let rb = srv.rollback(&c2["run_id"], false).await["result"].clone();
    assert_eq!(rb["status"], "accepted", "{rb}");
    assert_eq!(srv.wish().await["program"], p1);
    let rb = srv.rollback(&c1["run_id"], false).await["result"].clone();
    assert_eq!(rb["status"], "accepted", "{rb}");
    assert!(srv.wish().await.get("program").is_none());
    assert_eq!(fresh(&srv).await["needs_coding"]["reasons"][0]["code"], "no_macro");
    // a macro changed by hand since: the rollback asks first
    let c3 = srv.run("wish.xllm@1", stage("code")).await;
    srv.done(&c3);
    let w = srv.rpc("doc.read", json!({ "entity_id": "wish-q3" })).await;
    let mut edited = w["content"]["payload"]["program"].clone();
    edited["edited_by"] = json!("alice");
    srv.commit(json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [{ "key": "program", "value": edited, "expect": { "rev": w["content"]["key_revs"]["program"] } }] }])).await;
    let ask = srv.rollback(&c3["run_id"], false).await["result"].clone();
    assert_eq!((ask["status"].as_str(), ask["lost"][0]["entity_id"].as_str(), ask["lost"][0]["what"].as_str()), (Some("needs_confirm"), Some("wish-q3"), Some("edited")), "{ask}");
    assert_eq!(srv.rollback(&c3["run_id"], true).await["result"]["status"], "accepted");
    // the analysis back to the previous one, as it was (still current); then to none
    let a2 = srv.run("wish.xllm@1", stage("analyze")).await;
    srv.done(&a2);
    assert_eq!(srv.wish().await["analysis"]["run_id"], a2["run_id"]);
    assert_eq!(srv.rollback(&a2["run_id"], false).await["result"]["status"], "accepted");
    let w = srv.wish().await;
    assert_eq!(w["analysis"]["run_id"], a1["run_id"]);
    assert_eq!(fresh(&srv).await["needs_analysis"], false, "restored with its basis");
    assert_eq!(srv.rollback(&a1["run_id"], false).await["result"]["status"], "accepted");
    assert!(srv.wish().await.get("analysis").is_none());
    let codes: Vec<&str> = w["history"]["code"].as_array().unwrap().iter().map(|r| r["action"].as_str().unwrap()).collect();
    assert_eq!(codes, ["rollback", "code", "rollback", "rollback", "code", "code"]);
    });
}

#[test]
fn the_history_keeps_twenty_records_a_stage_and_its_logs_for_editors() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), m.clone(), json!({ "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" } })).await;
    setup(&mut srv).await;
    let (_, e, r) = through(&srv).await;
    // a reader sees the records and their objects, not a macro's log (D9)
    srv.rpc("acl.set", json!({ "changes": [{ "subject": "bob", "allow": ["read"] }] })).await;
    let get = |token: &'static str, id: Value| {
        let (http, url) = (srv.http.clone(), format!("{}/asset/{}/{}", srv.base, srv.ws, id.as_str().unwrap()));
        async move { http.get(url).bearer_auth(token).send().await.unwrap().status() }
    };
    let rec = &e["record"];
    assert!(get("tok-bob", rec["objects"]["program"].clone()).await.is_success());
    assert!(get("tok-bob", rec["objects"]["prompt"].clone()).await.is_success());
    assert!(!get("tok-bob", rec["objects"]["log"].clone()).await.is_success(), "a log needs update on the wish");
    assert!(!get("tok-bob", r["record"]["objects"]["log"].clone()).await.is_success());
    assert!(get("tok-alice", r["record"]["objects"]["log"].clone()).await.is_success());
    let w = srv.call_as("tok-bob", "doc.read", json!({ "entity_id": "wish-q3" })).await;
    assert_eq!(w["result"]["content"]["payload"]["history"]["run"][0]["id"], r["run_id"], "{w}");
    assert!(srv.call_as("tok-bob", "proc.rollback", json!({ "run_id": r["run_id"] })).await["result"]["error"].is_object(), "a rollback needs update");
    // failures are recorded too; a stage keeps its newest twenty
    *m.refuse.lock().unwrap() = Some("AI_QUOTA_EXHAUSTED");
    let mut last = Value::Null;
    for _ in 0..21 {
        last = srv.run("wish.xllm@1", stage("analyze")).await;
        assert_eq!((last["state"].as_str(), last["record"]["error"]["code"].as_str()), (Some("failed"), Some("AI_QUOTA_EXHAUSTED")), "{last}");
    }
    let h = srv.wish().await["history"].clone();
    let analyze = h["analyze"].as_array().unwrap();
    assert_eq!(analyze.len(), 20);
    assert_eq!(analyze[0]["id"], last["run_id"], "newest first");
    assert!(analyze.iter().all(|x| x["ok"] == json!(false)), "the oldest (the successful one) dropped");
    assert_eq!((h["run"].as_array().map(Vec::len), h["results_count"].as_u64()), (Some(1), Some(1)));
    // a template carries no history; a clone keeps it
    let t = srv.rpc("ext.export_template", json!({ "surfaces": ["sf"], "template": { "id": "q3", "title": "Q3" } })).await;
    let mut wishes = 0;
    for f in t["files"].as_array().unwrap().iter().filter(|f| f["path"].as_str().unwrap().contains("/objects/")) {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(f["base64"].as_str().unwrap()).unwrap();
        if let Ok(o) = serde_json::from_slice::<Value>(&bytes) {
            if o["ws_type"] == json!("buckyos.wish") {
                wishes += 1;
                assert!(o["content"].get("history").is_none(), "{o}");
            }
        }
    }
    assert_eq!(wishes, 1);
    let cloned = srv.rpc("ws.clone", json!({})).await;
    let w = srv.rpc("doc.read", json!({ "workspace_id": cloned["workspace_id"], "entity_id": "wish-q3" })).await;
    assert_eq!(w["content"]["payload"]["history"]["run"][0]["id"], r["run_id"], "{w}");
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
fn llm_map_cache_and_cancel() {
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
    srv.done(&srv.run("wish.xllm@1", stage("analyze")).await);
    let e = srv.run("wish.xllm@1", stage("code")).await;
    srv.done(&e);
    let first_map_calls = m.map_calls.load(Ordering::SeqCst);
    assert!(first_map_calls >= 1);
    let p = srv.run("wish.xllm@1", stage("run")).await;
    srv.done(&p);
    assert_eq!(m.map_calls.load(Ordering::SeqCst), first_map_calls, "the trial run's answers are the run's: all cached");
    let w = srv.wish().await;
    let fid = w["last_run"]["result_bindings"]["results"]["情感"]["fields"]["情感"].as_str().unwrap().to_string();
    let v = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o2", "field_id": fid } })).await;
    assert!(v["content"]["value"].is_string(), "{v}");
    assert!(p["record"].get("usage").is_none(), "every item cached: no AI call");

    // ---- cancel: the executor's next call is refused, nothing is written, nothing recorded
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
    assert!(r["record"].is_null(), "a cancelled run has no record: {r}");
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
        // the process died before the stage's Commit happened
        let (r2, _) = ws.wish_run_create(&alice, "wish.xllm@1", &params, "k2", &json!(null)).unwrap();
        ws.wish_run_set(&r2, Some("applying"), None, None, None, Some(&json!({ "applying": { "idempotency_key": format!("run/{r2}"), "record": {} } }))).unwrap();
        (id, r1, r2)
    };
    let mut srv = Srv::start(dir, scripted()).await;
    srv.ws = ws_id;
    let r = srv.rpc("proc.get", json!({ "run_id": running })).await;
    assert_eq!(r["state"], "interrupted", "{r}");
    assert_eq!(r["error"]["code"], "INTERRUPTED");
    let r = srv.rpc("proc.get", json!({ "run_id": applying })).await;
    assert_eq!((r["state"].as_str(), r["error"]["code"].as_str()), (Some("interrupted"), Some("INTERRUPTED")), "{r}");
    });
}

#[test]
fn executors_claim_report_and_the_service_writes_what_their_stage_collected() {
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
    // the run's progress names its model for the status bar; whether the executor waits for it is the executor's word
    assert_eq!(srv.rpc("proc.get", json!({ "run_id": run_id })).await["progress"]["model"], claimed["stage"]["model"]);
    // the run token speaks for the run, reads the Workspace, and nothing else
    let hb = srv.call_as(&run_token, "proc.heartbeat", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "progress": { "phase": "thinking", "waiting_model": true } })).await;
    assert_eq!(hb["result"]["ok"], true, "{hb}");
    let got = srv.rpc("proc.get", json!({ "run_id": run_id })).await;
    assert_eq!((got["progress"]["phase"].as_str(), got["progress"]["waiting_model"].as_bool(), got["progress"]["model"].is_string()), (Some("thinking"), Some(true), true), "{got}");
    srv.call_as(&run_token, "proc.heartbeat", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "progress": { "phase": "reading" } })).await;
    assert!(srv.rpc("proc.get", json!({ "run_id": run_id })).await["progress"].get("waiting_model").is_none());
    assert_eq!(srv.call_as(&run_token, "doc.read", json!({ "workspace_id": ws, "entity_id": "orders" })).await["result"]["type_id"], "buckyos.table-source");
    assert!(srv.call_as(&run_token, "doc.commit", json!({ "workspace_id": ws, "epoch": "x", "idempotency_key": "k", "operations": [] })).await["error"].is_string(), "a run token cannot write");
    // results are never reported directly: they come from the macro
    let direct = srv.call_as(&run_token, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "results": [] })).await;
    assert!(direct["result"]["error"]["detail"].as_str().unwrap_or("").contains("come from the macro"), "{direct}");
    // an analysis made elsewhere (ids instead of handles), with the usage of the model that made it: validated and written
    let analysis = json!({ "schema_version": "wish.analysis.v3", "status": "ready", "context_prompt": "读取 blk-orders 的全部记录，按月汇总销售额并让模型写解读。",
        "inputs": [{ "name": "sales", "ref": "blk-orders" }],
        "output_contract": { "results": [{ "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"] }, { "name": "commentary", "type": "richtext", "title": "解读" }] },
        "checks": [{ "id": "total", "kind": "program", "level": "error", "text": "合计一致" }], "blockers": [], "warnings": [] });
    let usage = json!({ "model": "gpt-test", "input_tokens": 1234, "output_tokens": 56, "calls": 2 });
    let reported = srv.call_as(&run_token, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "analysis": analysis, "usage": usage })).await;
    assert!(reported["result"]["ok"] == json!(true), "{reported}");
    let done = srv.wait(&run_id).await;
    srv.done(&done);
    assert_eq!((done["usage"].clone(), done["record"]["usage"].clone()), (usage.clone(), usage), "{done}");
    assert_eq!(srv.wish().await["inputs"][0]["name"], "sales");
    // a coding run: a superseded attempt is refused
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "loc-e", "params": { "wish_id": "wish-q3", "stage": "code", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let first = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    srv.execute("tok-alice", &run_id, &claim_token).await;
    let done = srv.wait(&run_id).await;
    let stale = srv.call_as(first["run_token"].as_str().unwrap(), "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    assert_eq!(stale["result"]["error"]["code"], "RUN_REVOKED", "{stale}");
    srv.done(&done);
    assert_eq!(done["execution"]["attempt"], 2, "{done}");
    // the wish changed under a working executor: when it reports, its stage is not written (DELEGATION_STALE)
    let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "loc-e2", "params": { "wish_id": "wish-q3", "stage": "code", "location": loc(), "execution": { "mode": "local" } } })).await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    let ex = Exec { srv: &srv, rt: claimed["run_token"].as_str().unwrap().to_string(), run_id: run_id.clone(), attempt: 1, manifest: Value::Null, turns: Default::default(), said: Default::default() };
    ex.run_macro(PROGRAM).await.unwrap();
    assert_eq!(ex.tool("finish", json!({ "summary": "done" })).await.unwrap()["status"], "ok");
    let program = srv.wish().await["program"].clone();
    let rev = srv.rpc("doc.read", json!({ "entity_id": "wish-q3" })).await["content"]["key_revs"]["prompt"].clone();
    srv.commit(json!([{ "op": "entity.set_keys", "entity_id": "wish-q3", "keys": [{ "key": "prompt", "value": "改了题目", "expect": { "rev": rev } }] }])).await;
    ex.call("proc.report", json!({ "finished": true })).await;
    let stale = srv.wait(&run_id).await;
    assert_eq!((stale["state"].as_str(), stale["error"]["code"].as_str(), stale["record"]["ok"].as_bool()), (Some("failed"), Some("DELEGATION_STALE"), Some(false)), "{stale}");
    assert_eq!(srv.wish().await["program"], program, "nothing written");
    // the analysis changed with the prompt: coding is refused until it is analyzed again
    let r = srv.run("wish.xllm@1", stage("code")).await;
    assert_eq!(r["error"]["sub_code"], "NEEDS_ANALYSIS", "{r}");
    });
}

/// The run's `proc.get` as `token` once its dispatch is recorded.
async fn dispatched(srv: &Srv, token: &str, run_id: &str) -> Value {
    for _ in 0..100 {
        let got = srv.call_as(token, "proc.get", json!({ "run_id": run_id })).await["result"].clone();
        if got["dispatch"]["status"].is_string() {
            return got;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no dispatch of {run_id} recorded");
}

#[test]
fn a_hosted_run_is_handed_to_the_starters_own_agent() {
    run_test(|| async move {
    let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), scripted(), json!({
        "tok-alice": { "principal": "alice" },
        "tok-bob": { "principal": "bob" },
        "tok-carol": { "principal": "carol" },
        "tok-dave": { "principal": "dave" },
        "tok-jarvis": { "principal": "did:web:jarvis.test.buckyos.io", "class": "agent", "owner": "alice" },
        "tok-bobby": { "principal": "did:web:bobby.test.buckyos.io", "class": "agent", "owner": "bob" },
        "tok-d1": { "principal": "did:web:d1.test.buckyos.io", "class": "agent", "owner": "dave" },
        "tok-d2": { "principal": "did:web:d2.test.buckyos.io", "class": "agent", "owner": "dave" },
    })).await;
    setup(&mut srv).await;
    for who in ["bob", "carol", "dave"] {
        srv.rpc("acl.set", json!({ "changes": [{ "subject": who, "allow": ["read", "comment", "update"] }] })).await;
    }
    assert!(srv.rpc("ws.get_info", json!({})).await.get("hosted_exec").is_none(), "the owner configures nothing");
    assert!(srv.rpc_raw("ws.set_hosted_exec", json!({ "config": { "enabled": true } })).await["error"].is_string());
    let hosted = |key: &str, stage: &str, agent: Option<&str>| {
        let mut execution = json!({ "mode": "hosted" });
        if let Some(a) = agent {
            execution["agent"] = json!(a);
        }
        json!({ "program": "wish.xllm@1", "idempotency_key": key, "params": { "wish_id": "wish-q3", "stage": stage, "location": loc(), "execution": execution } })
    };
    // the starter's only Agent executes it, on the starter's AI resources; the claim token goes to the Agent alone
    let start = srv.call_as("tok-alice", "proc.start", hosted("h1", "analyze", None)).await["result"].clone();
    assert_eq!(start["context"]["executor"], json!({ "kind": "agent", "principal": "jarvis.test.buckyos.io", "did": "did:web:jarvis.test.buckyos.io", "owner": "alice" }), "{start}");
    assert_eq!((start["context"]["location"].as_str(), start["context"]["trigger"].as_str(), start["context"]["compute"]["principal"].as_str()), (Some("hosted"), Some("alice"), Some("alice")));
    assert_eq!(start["aicc"], json!({ "mode": "current_user", "user_id": "alice", "operator_id": "alice" }));
    assert!(start.get("claim_token").is_none(), "the token goes to the agent, not the starter");
    let run_id = start["run_id"].as_str().unwrap().to_string();
    // without a Zone runtime the dispatch is recorded unavailable and the run waits
    let got = dispatched(&srv, "tok-alice", &run_id).await;
    assert_eq!((got["dispatch"]["status"].as_str(), got["dispatch"]["agent_id"].as_str()), (Some("unavailable"), Some("jarvis.test.buckyos.io")), "{got}");
    // S6: the claim token never shows in the run, and only the Agent claims (a token alone is not enough)
    assert!(!got.to_string().contains("ct_"), "{got}");
    let refused = srv.call_as("tok-alice", "proc.claim", json!({ "run_id": run_id, "claim_token": "ct_aaaaaaaaaaaaaaaaaaaaaaaaaa" })).await;
    assert_eq!(refused["result"]["error"]["code"], "PERMISSION_DENIED", "{refused}");
    // a collaborator's hosted run is the collaborator's own Agent's, on the collaborator's resources
    let start = srv.call_as("tok-bob", "proc.start", hosted("h2", "analyze", None)).await["result"].clone();
    assert_eq!((start["context"]["executor"]["principal"].as_str(), start["context"]["executor"]["owner"].as_str(), start["aicc"]["user_id"].as_str()), (Some("bobby.test.buckyos.io"), Some("bob"), Some("bob")), "{start}");
    let bobs = start["run_id"].as_str().unwrap().to_string();
    // where it was handed shows to its starter and the managers; another editor sees whether it was
    let own = dispatched(&srv, "tok-bob", &bobs).await;
    assert_eq!(own["dispatch"]["agent_id"], "bobby.test.buckyos.io", "{own}");
    assert_eq!(dispatched(&srv, "tok-alice", &bobs).await["dispatch"]["agent_id"], "bobby.test.buckyos.io");
    let other = dispatched(&srv, "tok-carol", &bobs).await;
    assert_eq!(other["dispatch"], json!({ "status": own["dispatch"]["status"], "at": own["dispatch"]["at"] }), "{other}");
    // an Agent of somebody else is refused, one's own is found by its DID or its AgentId
    let r = srv.call_as("tok-alice", "proc.start", hosted("h3", "analyze", Some("did:web:bobby.test.buckyos.io"))).await;
    assert_eq!(r["result"]["error"]["code"], "PERMISSION_DENIED", "{r}");
    for (key, name) in [("h4", "jarvis.test.buckyos.io"), ("h5", "did:web:jarvis.test.buckyos.io")] {
        let r = srv.call_as("tok-alice", "proc.start", hosted(key, "analyze", Some(name))).await["result"].clone();
        assert_eq!(r["context"]["executor"]["principal"], "jarvis.test.buckyos.io", "{r}");
    }
    // no Agent, several without naming one, or no account: nobody to hand it to
    let none = srv.call_as("tok-carol", "proc.start", hosted("h6", "analyze", None)).await;
    assert_eq!(none["result"]["error"]["code"], "EXECUTION_CONTEXT_MISSING", "{none}");
    let several = srv.call_as("tok-dave", "proc.start", hosted("h7", "analyze", None)).await;
    assert_eq!((several["result"]["error"]["code"].as_str(), several["result"]["error"]["data"]["agents"].as_array().map(Vec::len)), (Some("EXECUTION_CONTEXT_MISSING"), Some(2)), "{several}");
    let named = srv.call_as("tok-dave", "proc.start", hosted("h8", "analyze", Some("d2.test.buckyos.io"))).await["result"].clone();
    assert_eq!(named["context"]["executor"]["did"], "did:web:d2.test.buckyos.io", "{named}");
    let agent = srv.call_as("tok-jarvis", "proc.start", hosted("h9", "analyze", None)).await;
    assert_eq!(agent["result"]["error"]["code"], "EXECUTION_CONTEXT_MISSING", "{agent}");
    // the run is written as the Agent: judged as its starter until the workspace configures the Agent itself
    srv.done(&srv.run("wish.xllm@1", stage("analyze")).await);
    let coding = srv.call_as("tok-alice", "proc.start", hosted("h10", "code", None)).await["result"].clone();
    assert_eq!(coding["state"], "queued", "{coding}");
    srv.rpc("acl.set", json!({ "changes": [{ "subject": "did:web:jarvis.test.buckyos.io", "deny": ["update"] }] })).await;
    let denied = srv.call_as("tok-alice", "proc.start", hosted("h11", "code", None)).await;
    assert_eq!(denied["result"]["error"]["sub_code"], "MACRO_CANNOT_WRITE", "{denied}");
    let local = srv.call_as("tok-alice", "proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "l1", "params": { "wish_id": "wish-q3", "stage": "code", "location": loc(), "execution": { "mode": "local" } } })).await;
    assert_eq!(local["result"]["state"], "queued", "{local}");
    });
}

#[test]
fn local_runs_use_the_starters_ai() {
    run_test(|| async move {
    let m = scripted();
    let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), m.clone(), json!({
        "tok-alice": { "principal": "alice" },
        "tok-bob": { "principal": "bob" },
        "tok-eve": { "principal": "did:web:eve.example.com", "class": "external" },
    })).await;
    setup(&mut srv).await;
    for who in ["bob", "did:web:eve.example.com"] {
        srv.rpc("acl.set", json!({ "changes": [{ "subject": who, "allow": ["read", "comment", "update"] }] })).await;
    }
    assert!(srv.rpc("ws.get_info", json!({})).await.get("wish_aicc_mode").is_none(), "no workspace setting: where it runs decides");
    assert_eq!(srv.rpc("ws.get_info", json!({})).await["wish_execution"], json!({ "default": "local" }));
    // a collaborator's local run is the collaborator's own
    let done = srv.run_as("tok-bob", "wish.xllm@1", stage("analyze")).await;
    srv.done(&done);
    assert_eq!(done["aicc"], json!({ "mode": "current_user", "user_id": "bob", "operator_id": "bob" }));
    assert_eq!((done["record"]["by"].as_str(), done["record"]["aicc"]["user_id"].as_str()), (Some("bob"), Some("bob")));
    // an External user has no AI resources here and is not moved to the owner's
    let refused = srv.call_as("tok-eve", "proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "e1",
        "params": { "wish_id": "wish-q3", "stage": "analyze", "location": loc() } })).await;
    assert_eq!(refused["result"]["error"]["sub_code"], "AI_ATTRIBUTION_UNAVAILABLE", "{refused}");
    let gone = srv.call_as("tok-alice", "ws.set_wish_aicc", json!({ "mode": "current_user" })).await;
    assert!(!gone["error"].is_null() || gone["result"]["error"].is_object(), "the setting is gone: {gone}");
    // the executor's own model is refused: it reports the refusal, the run fails with its sub-code
    *m.refuse.lock().unwrap() = Some("AI_QUOTA_EXHAUSTED");
    let failed = srv.run("wish.xllm@1", stage("code")).await;
    assert_eq!((failed["state"].as_str(), failed["error"]["code"].as_str()), (Some("failed"), Some("AI_QUOTA_EXHAUSTED")), "{failed}");
    assert_eq!((failed["record"]["ok"].as_bool(), failed["record"]["error"]["code"].as_str()), (Some(false), Some("AI_QUOTA_EXHAUSTED")));
    // a macro's own AI call refused while running: the run fails with that sub-code, not as a broken program
    *m.refuse.lock().unwrap() = None;
    srv.done(&srv.run("wish.xllm@1", stage("code")).await);
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": "999.00", "expect": { "rev": rev } }] }])).await;
    for code in ["AI_QUOTA_EXHAUSTED", "AI_UNSUPPORTED_OPERATION"] {
        *m.refuse.lock().unwrap() = Some(code);
        let r = srv.run("wish.xllm@1", stage("run")).await;
        assert_eq!((r["state"].as_str(), r["error"]["sub_code"].as_str()), (Some("failed"), Some(code)), "{r}");
    }
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
        "checks": [{ "id": "total", "kind": "program", "level": "error", "text": "月度合计等于已结算合计" }], "blockers": [], "warnings": [] });
    let mut unleveled = analysis.clone();
    unleveled["checks"][0].as_object_mut().unwrap().remove("level");
    let refused = srv.call_as(&rt, "proc.tool", tool("submit_analysis", json!({ "analysis": unleveled }))).await;
    assert!(refused["result"]["status"] == "error" && refused["result"]["text"].as_str().unwrap().contains("level"), "a program check needs its level: {refused}");
    let ok = srv.call_as(&rt, "proc.tool", tool("submit_analysis", json!({ "analysis": analysis }))).await;
    assert_eq!(ok["result"]["status"], "ok", "{ok}");
    let reported = srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
    assert_eq!(reported["result"]["ok"], true, "{reported}");
    let done = srv.wait(&run_id).await;
    srv.done(&done);
    assert_eq!(srv.wish().await["inputs"][0]["name"], "sales");
    assert_eq!(m.calls.load(Ordering::SeqCst), 0, "the service called no model");

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
    srv.done(&done);
    assert_eq!(done["record"]["summary"]["trial"]["results"], 2, "{done}");
    let digest = srv.wish().await["program"]["manifest"]["results"]["dash"]["code_digest"].as_str().unwrap().to_string();
    assert_eq!(digest.len(), 64);

    // running: the stored macro only; other code, or HTML code other than the coded one, ends the run
    let run = |key: &'static str| srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": key, "params": { "wish_id": "wish-q3", "stage": "run", "location": loc(), "execution": { "mode": "local" } } }));
    let start = run("host-r1").await;
    let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
    let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
    assert_eq!((claimed["stage"]["kind"].as_str(), claimed["stage"]["manifest"]["results"]["dash"]["code_digest"].as_str()), (Some("program"), Some(digest.as_str())), "{claimed}");
    assert!(srv.rpc("proc.get", json!({ "run_id": run_id })).await["progress"].get("model").is_none(), "running the macro has no model");
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
    // the same code with new data, twice: only the data moves, the definition keeps its version
    let mut version = Value::Null;
    for (key, amount) in [("host-r2", 99.5), ("host-r3", 12.5)] {
        let start = run(key).await;
        let (run_id, claim_token) = (start["run_id"].as_str().unwrap().to_string(), start["claim_token"].as_str().unwrap().to_string());
        let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
        let rt = claimed["run_token"].as_str().unwrap().to_string();
        let mut fresh_data = output.clone();
        fresh_data["results"][0]["rows"] = json!([{ "月份": "2026-07-01", "销售额": amount }]);
        let ran = srv.call_as(&rt, "proc.program", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "source": PROGRAM, "results": fresh_data, "log": "", "files": {} })).await;
        assert_eq!(ran["result"]["status"], "ok", "{ran}");
        srv.call_as(&rt, "proc.report", json!({ "workspace_id": ws, "run_id": run_id, "attempt": 1, "finished": true })).await;
        srv.done(&srv.wait(&run_id).await);
        let def = srv.wish().await["last_run"]["result_bindings"]["results"]["dash"]["entity_id"].as_str().unwrap().to_string();
        let v = srv.rpc("doc.read", json!({ "entity_id": def })).await["content"]["payload"]["version"].clone();
        if version.is_null() {
            version = v;
        } else {
            assert_eq!(v, version, "the control definition did not change");
        }
    }

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
