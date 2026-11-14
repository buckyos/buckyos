//! 许愿格 under the permission model (权限需求 §4.5, Q31–Q33): a run is written as its macro identity, so one that could
//! never be applied does not start; an editor's macro may write its page, trusted only when a manager writes or vouches
//! for it; a macro bound with `intersection` reads no more than its group; a preview is judged as the apply will be; only
//! changes of the macro itself are audited; the owner's hosted configuration shows to managers only; an input the
//! viewer cannot read tells no story of its fields.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::{json, Value};
use std::time::Duration;
use wishkit::*;

const TABLE: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额'] } })
  const { rows } = await aiws.sql('SELECT "月份", round(sum("销售额"), 2) AS "销售额" FROM sales GROUP BY 1 ORDER BY 1')
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal' } })
}"#;

const DASH: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额'] } })
  const { rows } = await aiws.sql('SELECT "月份", round(sum("销售额"), 2) AS "销售额" FROM sales GROUP BY 1 ORDER BY 1')
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal' } })
  aiws.result.html('dash', { html: '<div id=n></div>', bindings: { monthly: 'result:monthly' } })
}"#;

const DASH_OWNER: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额'] } })
  const { rows } = await aiws.sql('SELECT "月份", round(sum("销售额"), 2) AS "销售额" FROM sales GROUP BY 1 ORDER BY 1')
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal' } })
  aiws.result.html('dash', { html: '<div id=total></div>', bindings: { monthly: 'result:monthly' } })
}"#;

fn analysis(html: bool) -> Script {
    Box::new(move |t: &Turn| match t.results.len() {
        0 => call("ws_profile", json!({ "target": handle(&t.user, "table \"订单\"") })),
        1 => {
            let b = handle(&t.user, "table \"订单\"");
            let mut results = vec![json!({ "name": "monthly", "type": "table", "title": "月度汇总", "key": ["月份"] })];
            if html {
                results.push(json!({ "name": "dash", "type": "html", "title": "看板" }));
            }
            call("submit_analysis", json!({ "analysis": {
                "schema_version": "wish.analysis.v3", "status": "ready", "context_prompt": format!("读取 {b}，按月汇总销售额"),
                "inputs": [{ "name": "sales", "ref": b }], "output_contract": { "results": results }, "checks": [], "blockers": [], "warnings": [] } }))
        }
        _ => text("分析完成"),
    })
}

fn coding(source: &'static str) -> Script {
    Box::new(move |t: &Turn| match t.results.len() {
        0 => call("run_program", json!({ "source": source })),
        1 => call("finish", json!({ "summary": "完成" })),
        _ => text("完成"),
    })
}

async fn start(tokens: Value) -> Srv {
    let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), model(), tokens).await;
    let ws = srv.rpc("ws.create", json!({ "title": "权限" })).await;
    srv.ws = ws["workspace_id"].as_str().unwrap().to_string();
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "看板" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "看板",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "看板", "content_folder_id": "sf-content" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "订单",
          "payload": { "fields": [{ "field_id": "month", "name": "月份", "type": "date" }, { "field_id": "amount", "name": "销售额", "type": "decimal", "scale": 2 }] } },
        { "op": "table.insert_records", "source_id": "orders", "records": [{ "record_id": "o1", "values": { "month": "2026-07-01", "amount": "10.00" } }] },
        { "op": "entity.create", "entity_id": "w", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w", "name": "汇总",
          "payload": { "title": "汇总", "prompt": "按月汇总销售额", "executor": "xllm", "output_mode": "overwrite",
                       "output": { "container_id": "sf-content", "surface_id": "sf", "name": "结果" } } },
        { "op": "entity.create", "entity_id": "blk-w", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "d", "placement": { "x": 0, "y": 0, "w": 400, "h": 300 },
          "payload": { "view": { "type": "wish" }, "source_ref": { "entity_id": "w" } } },
    ])).await;
    srv
}

fn loc() -> Value {
    json!({ "cell_id": "blk-w" })
}

fn stage(name: &str) -> Value {
    json!({ "wish_id": "w", "stage": name, "location": loc() })
}

/// Preview and apply as `token`: `{ ready, problems, apply }` (`apply`: the raw answer).
async fn apply_as(srv: &Srv, token: &str, run: &Value) -> Value {
    let id = run["run_id"].as_str().unwrap();
    let preview = srv.call_as(token, "proc.get", json!({ "run_id": id, "choices": {} })).await["result"]["preview"].clone();
    let apply = srv.call_as(token, "proc.apply", json!({ "run_id": id, "plan_digest": preview["plan_digest"], "session_id": "s1" })).await;
    json!({ "ready": preview["ready"], "problems": preview["summary"]["problems"], "apply": apply })
}

fn accepted(r: &Value) -> bool {
    r["apply"]["result"]["applied"]["status"] == json!("accepted")
}

async fn set_amount(srv: &Srv, value: &str) {
    let rev = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_cell", "record_id": "o1", "field_id": "amount" } })).await["content"]["rev"].clone();
    srv.commit(json!([{ "op": "table.set_values", "source_id": "orders", "values": [{ "record_id": "o1", "field_id": "amount", "value": value, "expect": { "rev": rev } }] }])).await;
}

async fn analyzed_and_coded(srv: &Srv, token: &str, html: bool, source: &'static str) {
    *srv.model.analyze.lock().unwrap() = analysis(html);
    *srv.model.execute.lock().unwrap() = coding(source);
    let a = srv.run_as(token, "wish.xllm@1", stage("analyze")).await;
    assert!(accepted(&apply_as(srv, token, &a).await), "{a}");
    let c = srv.run_as(token, "wish.xllm@1", stage("code")).await;
    let applied = apply_as(srv, token, &c).await;
    assert!(accepted(&applied), "{applied}");
}

async fn dash_def(srv: &Srv) -> Value {
    let w = srv.wish_of("w").await;
    let id = w["last_run"]["result_bindings"]["results"]["dash"]["entity_id"].as_str().unwrap().to_string();
    srv.rpc("doc.read", json!({ "entity_id": id })).await["content"]["payload"].clone()
}

async fn macro_audits(srv: &Srv) -> usize {
    srv.rpc("acl.audit", json!({})).await["items"].as_array().unwrap().iter().filter(|i| i["action"] == "macro.changed").count()
}

#[test]
fn a_run_starts_only_when_its_macro_identity_could_write_and_an_editors_page_waits_for_a_manager() {
    run_test(|| async move {
        let srv = start(json!({ "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" }, "tok-carol": { "principal": "carol" } })).await;
        srv.rpc("acl.set", json!({ "changes": [
            { "subject": "bob", "allow": ["read", "run"] },
            { "subject": "carol", "allow": ["read", "comment", "append", "update", "delete", "structure"] }] })).await;

        // an editor without manage codes a page: it is written, but not trusted — a viewer's host shows its snapshot
        analyzed_and_coded(&srv, "tok-carol", true, DASH).await;
        let def = dash_def(&srv).await;
        assert_eq!(def["html"]["html"], "<div id=n></div>");
        assert!(def.get("trusted").is_none(), "an editor's page is not vouched for: {def}");
        // the editor's page is vouched for by a manager
        let w = srv.wish_of("w").await;
        let def_id = w["last_run"]["result_bindings"]["results"]["dash"]["entity_id"].as_str().unwrap().to_string();
        let rev = srv.rpc("doc.read", json!({ "entity_id": def_id })).await["content"]["key_revs"]["trusted"].as_u64().unwrap_or(0);
        let refused = srv.call_as("tok-carol", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": srv.ws, "epoch": srv.rpc("ws.get_info", json!({})).await["epoch"], "idempotency_key": "vouch-c", "session_id": "s1",
            "operations": [{ "op": "entity.set_keys", "entity_id": def_id, "keys": [{ "key": "trusted", "value": true, "expect": { "rev": rev } }] }] })).await;
        assert_eq!(refused["result"]["code"], "PERMISSION_DENIED", "only a manager vouches: {refused}");
        srv.commit(json!([{ "op": "entity.set_keys", "entity_id": def_id, "keys": [{ "key": "trusted", "value": true, "expect": { "rev": rev } }] }])).await;
        assert_eq!(dash_def(&srv).await["trusted"]["by"], "alice");

        // the owner codes another page: written and trusted as it is written
        *srv.model.execute.lock().unwrap() = coding(DASH_OWNER);
        let c = srv.run("wish.xllm@1", stage("code")).await;
        let applied = apply_as(&srv, "tok-alice", &c).await;
        assert!(accepted(&applied), "{c}\n{applied}");
        let def = dash_def(&srv).await;
        assert_eq!((def["html"]["html"].as_str(), def["trusted"]["by"].as_str()), (Some("<div id=total></div>"), Some("alice")), "{def}");
        // the editor's next page drops the trust again
        *srv.model.execute.lock().unwrap() = coding(DASH);
        let c = srv.run_as("tok-carol", "wish.xllm@1", stage("code")).await;
        assert!(accepted(&apply_as(&srv, "tok-carol", &c).await));
        assert!(dash_def(&srv).await.get("trusted").is_none());
        // running it moves only data: the page keeps its state, an editor's run included
        set_amount(&srv, "20.00").await;
        let r = srv.run_as("tok-carol", "wish.xllm@1", stage("run")).await;
        assert!(accepted(&apply_as(&srv, "tok-carol", &r).await), "{r}");

        // holding only `run`: nothing it runs could ever be applied, so nothing starts — no run, no AI spent
        let calls = srv.model.calls.load(std::sync::atomic::Ordering::SeqCst);
        let refused = srv.run_as("tok-bob", "wish.xllm@1", stage("run")).await;
        assert_eq!((refused["state"].as_str(), refused["error"]["code"].as_str(), refused["error"]["sub_code"].as_str()),
                   (Some("refused"), Some("PERMISSION_DENIED"), Some("MACRO_CANNOT_WRITE")), "{refused}");
        assert_eq!(srv.model.calls.load(std::sync::atomic::Ordering::SeqCst), calls);
        let mine = srv.call_as("tok-bob", "proc.list", json!({ "wish_id": "w" })).await;
        assert_eq!(mine["result"]["runs"], json!([]), "{mine}");
        let refused = srv.run_as("tok-bob", "wish.xllm@1", stage("analyze")).await;
        assert_eq!(refused["error"]["code"], "PERMISSION_DENIED", "analysis needs update (R29): {refused}");
    });
}

#[test]
fn an_intersection_group_bounds_what_the_macro_reads_and_a_preview_is_judged_as_the_apply() {
    run_test(|| async move {
        let srv = start(json!({ "tok-alice": { "principal": "alice" } })).await;
        analyzed_and_coded(&srv, "tok-alice", false, TABLE).await;
        srv.rpc("acl.set_group", json!({ "group_id": "writers", "title": "写结果" })).await;
        srv.rpc("acl.set", json!({ "changes": [
            { "subject": "@writers", "resource": "/", "allow": ["read", "comment", "append", "update", "delete", "structure"] },
            { "subject": "@writers", "resource": "~orders", "deny": ["read"] }] })).await;

        // bound with intersection to a group that cannot read the input: the owner's run reads no more than the group
        srv.rpc("acl.bind_macro", json!({ "wish": "~w", "group_id": "writers", "mode": "intersection" })).await;
        set_amount(&srv, "20.00").await;
        let r = srv.run("wish.xllm@1", stage("run")).await;
        assert_eq!((r["state"].as_str(), r["error"]["code"].as_str()), (Some("failed"), Some("DEPENDENCY_UNAVAILABLE")), "the macro read an input its group cannot: {r}");
        assert!(r["error"]["detail"].as_str().unwrap_or("").contains("not readable"), "{r}");

        // unbound, the owner's run is ready; bound afterwards to a group that writes nothing, the preview says so
        srv.rpc("acl.bind_macro", json!({ "wish": "~w", "group_id": null })).await;
        let r = srv.run("wish.xllm@1", stage("run")).await;
        assert_eq!(r["state"], "waiting_confirmation", "{r}");
        srv.rpc("acl.set_group", json!({ "group_id": "readers", "title": "只读" })).await;
        srv.rpc("acl.set", json!({ "changes": [{ "subject": "@readers", "resource": "/", "allow": ["read"] }] })).await;
        srv.rpc("acl.bind_macro", json!({ "wish": "~w", "group_id": "readers", "mode": "intersection" })).await;
        let applied = apply_as(&srv, "tok-alice", &r).await;
        assert_eq!(applied["ready"], false, "{applied}");
        assert!(applied["problems"].to_string().contains("PERMISSION_DENIED"), "{applied}");
        assert!(!accepted(&applied));
        // and a new run does not start
        let refused = srv.run("wish.xllm@1", stage("run")).await;
        assert_eq!(refused["error"]["sub_code"], "MACRO_CANNOT_WRITE", "{refused}");
        assert_eq!(refused["error"]["data"]["group_id"], "readers");
    });
}

#[test]
fn only_changes_of_the_macro_itself_are_audited() {
    run_test(|| async move {
        let srv = start(json!({ "tok-alice": { "principal": "alice" } })).await;
        srv.rpc("acl.set_group", json!({ "group_id": "g", "title": "G" })).await;
        srv.rpc("acl.bind_macro", json!({ "wish": "~w", "group_id": "g", "mode": "union" })).await;
        *srv.model.analyze.lock().unwrap() = analysis(false);
        *srv.model.execute.lock().unwrap() = coding(TABLE);
        let a = srv.run("wish.xllm@1", stage("analyze")).await;
        srv.apply(&a, json!({})).await;
        assert_eq!(macro_audits(&srv).await, 0, "an analysis is no macro");
        let c = srv.run("wish.xllm@1", stage("code")).await;
        srv.apply(&c, json!({})).await;
        assert_eq!(macro_audits(&srv).await, 1, "a new macro");
        set_amount(&srv, "30.00").await;
        let r = srv.run("wish.xllm@1", stage("run")).await;
        srv.apply(&r, json!({})).await;
        let rev = srv.rpc("doc.read", json!({ "entity_id": "w" })).await["content"]["key_revs"]["prompt"].clone();
        srv.commit(json!([{ "op": "entity.set_keys", "entity_id": "w", "keys": [{ "key": "prompt", "value": "按月汇总", "expect": { "rev": rev } }] }])).await;
        assert_eq!(macro_audits(&srv).await, 1, "results and other keys are no change of the macro");
    });
}

#[test]
fn the_owners_hosted_configuration_shows_to_managers_only() {
    run_test(|| async move {
        let srv = start(json!({ "tok-alice": { "principal": "alice" }, "tok-carol": { "principal": "carol" } })).await;
        srv.rpc("acl.set", json!({ "changes": [{ "subject": "carol", "allow": ["read", "comment", "update", "structure"] }] })).await;
        srv.rpc("ws.set_hosted_exec", json!({ "config": { "enabled": true, "agent_id": "jarvis.test.buckyos.io", "allowed_wishes": ["w"], "limits": { "max_tool_iterations": 5 } } })).await;
        let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": "h1", "params": { "wish_id": "w", "stage": "analyze", "location": loc(), "execution": { "mode": "hosted" } } })).await;
        let run_id = start["run_id"].as_str().unwrap().to_string();
        let mut owner = srv.rpc("proc.get", json!({ "run_id": run_id })).await;
        for _ in 0..100 {
            if owner["dispatch"]["status"].is_string() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            owner = srv.rpc("proc.get", json!({ "run_id": run_id })).await;
        }
        assert_eq!(owner["context"]["executor"]["limits"]["max_tool_iterations"], 5, "{owner}");
        assert!(owner["context"]["executor"]["config_digest"].is_string() && owner["dispatch"]["agent_id"].is_string(), "{owner}");
        let editor = srv.call_as("tok-carol", "proc.get", json!({ "run_id": run_id })).await["result"].clone();
        assert_eq!(editor["context"]["executor"]["kind"], "agent", "{editor}");
        assert!(editor["context"]["executor"].get("limits").is_none() && editor["context"]["executor"].get("config_digest").is_none(), "{editor}");
        assert_eq!(editor["dispatch"], json!({ "status": owner["dispatch"]["status"], "at": owner["dispatch"]["at"] }));
    });
}

#[test]
fn an_input_the_viewer_cannot_read_tells_no_story_of_its_fields() {
    run_test(|| async move {
        let srv = start(json!({ "tok-alice": { "principal": "alice" }, "tok-dave": { "principal": "dave" } })).await;
        analyzed_and_coded(&srv, "tok-alice", false, TABLE).await;
        srv.rpc("acl.set", json!({ "changes": [{ "subject": "dave", "resource": "/", "allow": ["read"] }, { "subject": "dave", "resource": "~orders", "deny": ["read"] }] })).await;
        let fdef = srv.rpc("doc.read", json!({ "entity_id": "orders", "selector": { "kind": "table_field", "field_id": "amount" } })).await;
        srv.commit(json!([{ "op": "table.delete_field", "source_id": "orders", "field_id": "amount", "expect": { "rev": fdef["content"]["def_rev"] } }])).await;
        let owner = srv.rpc("doc.freshness", json!({ "entity_ids": ["w"] })).await["items"][0].clone();
        assert_eq!(owner["needs_coding"]["reasons"][0]["code"], "field_deleted", "{owner}");
        let viewer = srv.call_as("tok-dave", "doc.freshness", json!({ "entity_ids": ["w"] })).await["result"]["items"][0].clone();
        assert_eq!(viewer["input_problems"], json!([{ "entity_id": "orders", "reason": "unreadable" }]), "{viewer}");
        assert!(viewer["needs_coding"].is_null(), "{viewer}");
    });
}
