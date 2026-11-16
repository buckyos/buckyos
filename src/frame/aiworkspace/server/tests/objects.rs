//! DID Object Provider (`/kapi/aiworkspace/obj/...`): Card / Profile / reads of
//! `obj://aiworkspace/<ws>/...` with the caller's session, and the Reference url projection; the actions
//! (AI Native 余下任务 C1, C2-3: 统一发现) — declared where they apply, judged exactly as their kRPC method.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::{json, Value};
use wishkit::*;

#[test]
fn workspace_objects_read_through_krpc_with_the_callers_grants() {
    run_test(|| async move {
        let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), model(), json!({ "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" } })).await;
        let ws = srv.rpc("ws.create", json!({ "title": "Objects" })).await["workspace_id"].as_str().unwrap().to_string();
        srv.ws = ws.clone();
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "orders", "payload": { "fields": [{ "field_id": "m", "name": "m", "type": "text" }] } },
            { "op": "table.insert_records", "source_id": "orders", "records": [{ "record_id": "r1", "values": { "m": "hello world" } }] },
            { "op": "entity.create", "entity_id": "t2", "type_id": "buckyos.container", "parent_id": "data", "order_key": "b", "payload": { "kind": "folder", "title": "第二张" } },
            { "op": "entity.create", "entity_id": "t3", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "c", "payload": { "fields": [] } },
        ])).await;
        let obj = format!("{}/obj/{ws}", srv.base);
        let http = reqwest::Client::new();
        let get = |url: String, token: &'static str| {
            let http = http.clone();
            async move {
                let r = http.get(url).header("authorization", format!("Bearer {token}")).header("host", "zone.invalid").send().await.unwrap();
                (r.status().as_u16(), r.json::<Value>().await.unwrap_or(Value::Null))
            }
        };

        let (status, profile) = get(format!("{obj}/entities/orders?type=profile"), "tok-alice").await;
        assert_eq!(status, 200, "{profile}");
        name_lib::ObjectProfile::from_value(profile).unwrap().validate().unwrap();
        let (status, card) = get(format!("{obj}/entities/orders"), "tok-alice").await;
        assert_eq!(status, 200, "{card}");
        assert_eq!(card["alsoKnownAs"].as_array().unwrap().last().unwrap(), &json!(format!("obj://aiworkspace/{ws}/entities/orders")));
        assert_eq!(get(format!("{obj}/entities/orders"), "tok-bob").await.0, 404, "without a grant the workspace does not exist for bob");

        let (status, content) = get(format!("{obj}/entities/orders/props/content"), "tok-alice").await;
        assert_eq!(status, 200, "{content}");
        let (status, resolved) = get(format!("{obj}/path/data/orders/props/resolved"), "tok-alice").await;
        assert_eq!(status, 200, "{resolved}");
        assert_eq!(resolved["object"], json!(format!("obj://aiworkspace/{ws}/entities/orders")));

        // meta and body (核心 API 调整 §3.1): what it is, and its text
        let (status, meta) = get(format!("{obj}/entities/orders/props/meta"), "tok-alice").await;
        assert_eq!(status, 200, "{meta}");
        assert_eq!((meta["type"].as_str(), meta["path"].as_str(), meta["body"]["format"].as_str()), (Some("table"), Some("/data/orders"), Some("text/csv")), "{meta}");
        assert!(meta["ops"].as_array().unwrap().iter().any(|o| o == "table.insert_records"));
        let (status, body) = get(format!("{obj}/entities/orders/props/body?max_bytes=4096"), "tok-alice").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["text"], "_id,m\nr1,hello world", "{body}");
        // names follow titles, or count: every entity can be addressed by path
        let (_, meta2) = get(format!("{obj}/entities/t2/props/meta"), "tok-alice").await;
        let (_, meta3) = get(format!("{obj}/entities/t3/props/meta"), "tok-alice").await;
        assert_eq!((meta2["path"].as_str(), meta3["path"].as_str()), (Some("/data/第二张"), Some("/data/table-1")));
        // a fixed version is read or refused, never read as the head (R2)
        let (status, fixed) = get(format!("{obj}/entities/orders/props/content?mode=fixed&version=%7B%22epoch%22%3A%22ep_x%22%2C%22seq%22%3A1%7D"), "tok-alice").await;
        assert_eq!(status, 404, "{fixed}");
        let (status, body) = get(format!("{obj}/entities/orders/props/body?mode=fixed&version=%7B%22epoch%22%3A%22ep_x%22%2C%22seq%22%3A1%7D"), "tok-alice").await;
        assert_eq!(status, 400, "{body}");

        // the Workspace index: ls (default), find, grep — one index, the action in the query (R1)
        let query = |suffix: &'static str, params: Value| {
            let http = http.clone();
            let url = format!("{obj}/methods/query{suffix}");
            async move {
                let v: Value = http.post(url).header("authorization", "Bearer tok-alice").json(&json!({ "method": "query", "params": params })).send().await.unwrap().json().await.unwrap();
                v["result"].clone()
            }
        };
        let page = query("", json!({ "query": { "at": "/data" }, "page_size": 2 })).await;
        let objects: Vec<&str> = page["rows"].as_array().unwrap().iter().filter_map(|r| r["object"].as_str()).collect();
        assert_eq!(objects.len(), 2, "{page}");
        assert!(objects.contains(&format!("obj://aiworkspace/{ws}/entities/orders").as_str()), "{page}");
        let next = page["next_cursor"].as_str().unwrap_or_else(|| panic!("{page}")).to_string();
        let more: Value = http
            .post(format!("{obj}/methods/page"))
            .header("authorization", "Bearer tok-alice")
            .json(&json!({ "method": "page", "params": { "cursor": next, "page_size": 2, "query": { "at": "/data" } } }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(more["result"]["rows"].as_array().unwrap().len() >= 1, "{more}");
        let found = query("", json!({ "query": { "op": "find", "name": "第二" } })).await;
        assert_eq!(found["rows"][0]["cells"]["path"], "/data/第二张", "{found}");
        let hits = query("?op=grep&text=HELLO", json!({})).await;
        assert_eq!(hits["rows"][0]["cells"]["on"], "body", "{hits}");
        assert_eq!(hits["rows"][0]["cells"]["selector"]["record_id"], "r1", "{hits}");
        assert!(hits["rows"][0]["object"].as_str().unwrap().contains("selector="), "{hits}");
        assert_eq!(hits["coverage"]["complete"], true, "{hits}");
    })
}

/// The analysis of a wish over the table “订单”: one result, no checks.
fn analysis() -> Script {
    Box::new(|t: &Turn| match t.results.len() {
        0 => call("ws_profile", json!({ "target": handle(&t.user, "table \"订单\"") })),
        1 => {
            let b = handle(&t.user, "table \"订单\"");
            call("submit_analysis", json!({ "analysis": {
                "schema_version": "wish.analysis.v3", "status": "ready", "context_prompt": format!("读取 {b}，按月汇总"),
                "inputs": [{ "name": "orders", "ref": b }], "output_contract": { "results": [{ "name": "monthly", "type": "table", "title": "月度", "key": ["月份"] }] },
                "checks": [], "blockers": [], "warnings": [] } }))
        }
        _ => text("分析完成"),
    })
}

/// 统一发现 (PRD §9): what an entity can do is in its Profile — a wish its three stages, an entity a Source writes its
/// refresh, each with its `risk` — and a reader finds it whatever it may do; calling one through the object is the kRPC
/// method itself: refused with the same error, started with the same run.
#[test]
fn actions_are_declared_where_they_apply_and_judged_as_their_krpc_method() {
    run_test(|| async move {
        let tokens = json!({ "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" }, "tok-carol": { "principal": "carol" } });
        let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), model(), tokens).await;
        let a = srv.rpc("ws.create", json!({ "title": "来源" })).await["workspace_id"].as_str().unwrap().to_string();
        srv.ws = a.clone();
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "orders",
              "payload": { "fields": [{ "field_id": "month", "name": "月份", "type": "text" }] } },
            { "op": "table.insert_records", "source_id": "orders", "records": [{ "record_id": "r1", "values": { "month": "2026-07" } }] },
        ])).await;
        srv.rpc("ws.publish", json!({ "entity_id": "orders" })).await;
        srv.ws = String::new();
        let ws = srv.rpc("ws.create", json!({ "title": "动作" })).await["workspace_id"].as_str().unwrap().to_string();
        srv.ws = ws.clone();
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
              "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "看板" } },
            { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "看板",
              "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "看板", "content_folder_id": "sf-content" } },
            { "op": "entity.create", "entity_id": "orders", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "订单",
              "payload": { "fields": [{ "field_id": "month", "name": "月份", "type": "date" }] } },
            { "op": "entity.create", "entity_id": "copy", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "引用",
              "payload": { "source": { "kind": "workspace", "workspace_id": a, "entity_id": "orders", "mode": "live" } } },
            { "op": "entity.create", "entity_id": "w", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w", "name": "汇总",
              "payload": { "title": "汇总", "prompt": "按月汇总", "executor": "xllm", "output_mode": "overwrite", "output": { "container_id": "sf-content", "surface_id": "sf", "name": "结果" } } },
        ])).await;
        srv.rpc("acl.set", json!({ "changes": [{ "subject": "bob", "allow": ["read", "run"] }, { "subject": "carol", "allow": ["read"] }] })).await;
        let obj = format!("{}/obj/{ws}", srv.base);
        let http = reqwest::Client::new();
        let get = |url: String, token: &'static str| {
            let http = http.clone();
            async move {
                let r = http.get(url).bearer_auth(token).send().await.unwrap();
                (r.status().as_u16(), r.json::<Value>().await.unwrap_or(Value::Null))
            }
        };
        let xcall = |token: &'static str, entity: &'static str, action: &'static str, params: Value| {
            let http = http.clone();
            let url = format!("{obj}/entities/{entity}/methods/{action}");
            async move {
                let r = http.post(url).bearer_auth(token).json(&json!({ "method": action, "params": params })).send().await.unwrap();
                (r.status().as_u16(), r.json::<Value>().await.unwrap())
            }
        };
        let names = |profile: &Value| profile["actions"].as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();

        // discovery: the wish declares its stages, the sourced copy its refresh, a plain table nothing; every action with
        // its risk and the RBAC entry it is (an object's resource, `write`)
        let (status, wish) = get(format!("{obj}/entities/w?type=profile"), "tok-alice").await;
        assert_eq!(status, 200, "{wish}");
        name_lib::ObjectProfile::from_value(wish.clone()).unwrap().validate().unwrap();
        assert_eq!(names(&wish), vec!["analyze", "code", "run"], "{wish}");
        for stage in ["analyze", "code", "run"] {
            // running overwrites results a person edited: destructive (owner 2026-11-16)
            assert_eq!(wish["actions"][stage]["x-buckyos:risk"], if stage == "run" { "destructive" } else { "write" }, "{wish}");
            assert_eq!(wish["actions"][stage]["x-buckyos:rbac"], json!({ "resource": format!("obj://aiworkspace/{ws}/entities/w"), "action": "write" }));
            assert!(wish["actions"][stage]["description"].as_str().unwrap().contains("Authorized as that kRPC method"), "{wish}");
        }
        let (_, copy) = get(format!("{obj}/entities/copy?type=profile"), "tok-alice").await;
        let copy_meta = get(format!("{obj}/entities/copy/props/meta"), "tok-alice").await;
        assert_eq!(names(&copy), vec!["refresh"], "{copy_meta:?}");
        assert_eq!(copy["actions"]["refresh"]["x-buckyos:risk"], "idempotent_write");
        let (_, plain) = get(format!("{obj}/entities/orders?type=profile"), "tok-alice").await;
        assert!(names(&plain).is_empty(), "{plain}");
        // discoverable is not permitted: a reader sees the stages, and its capabilities say it may not start them
        let (_, seen) = get(format!("{obj}/entities/w?type=profile"), "tok-carol").await;
        assert_eq!(names(&seen), vec!["analyze", "code", "run"], "{seen}");
        let (_, meta) = get(format!("{obj}/entities/w/props/meta"), "tok-carol").await;
        assert!(!meta["capabilities"].as_array().unwrap().iter().any(|c| c == "update"), "{meta}");

        // refused through the object exactly as through kRPC: the same error, and no run
        for (token, entity, action, program, params) in [
            ("tok-bob", "w", "analyze", "wish.xllm@1", json!({ "wish_id": "w", "stage": "analyze" })),
            ("tok-bob", "w", "run", "wish.xllm@1", json!({ "wish_id": "w", "stage": "run" })),
            ("tok-carol", "copy", "refresh", "source.refresh@1", json!({ "entity_id": "copy" })),
        ] {
            let krpc = srv.call_as(token, "proc.start", json!({ "program": program, "params": params, "idempotency_key": "k-refused" })).await["result"]["error"].clone();
            assert!(krpc.is_object(), "{action}: {krpc}");
            let (status, answer) = xcall(token, entity, action, json!({})).await;
            let expected = if krpc["code"] == "PERMISSION_DENIED" { 403 } else { 400 };
            assert_eq!(status, expected, "{action}: {answer}");
            let message: Value = serde_json::from_str(answer["error"]["message"].as_str().unwrap()).unwrap();
            assert_eq!(message, krpc, "{action}");
        }
        assert_eq!(srv.call_as("tok-bob", "proc.list", json!({ "wish_id": "w" })).await["result"]["runs"], json!([]));

        // started through the object: the same run as proc.start, executed and written as any run
        *srv.model.analyze.lock().unwrap() = analysis();
        let input = json!({ "idempotency_key": "x-analyze", "location": { "surface_id": "sf" } });
        let (status, started) = xcall("tok-alice", "w", "analyze", input.clone()).await;
        assert_eq!(status, 200, "{started}");
        let run = started["result"].clone();
        assert_eq!((run["stage"].as_str(), run["wish_id"].as_str()), (Some("analyze"), Some("w")), "{run}");
        let run_id = run["run_id"].as_str().unwrap().to_string();
        let (_, again) = xcall("tok-alice", "w", "analyze", input).await;
        assert_eq!(again["result"]["run_id"], json!(run_id), "the same key answers the same run: {again}");
        srv.execute("tok-alice", &run_id, run["claim_token"].as_str().unwrap()).await;
        srv.done(&srv.wait(&run_id).await);
        assert_eq!(srv.wish_of("w").await["analysis"]["status"], "ready");

        let (status, started) = xcall("tok-alice", "copy", "refresh", json!({})).await;
        assert_eq!(status, 200, "{started}");
        let done = srv.wait(started["result"]["run_id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "succeeded", "{done}");
        assert_eq!(srv.records("copy").await.len(), 1);
        // an action the object does not declare is still the method's to judge
        assert_ne!(xcall("tok-alice", "orders", "refresh", json!({})).await.0, 200);

        // the Block-controls catalogue is discoverable from the root
        let root = format!("{}/obj", srv.base);
        let (status, controls) = get(format!("{root}/props/controls"), "tok-carol").await;
        assert_eq!(status, 200, "{controls}");
        assert!(controls["controls"].as_array().unwrap().iter().any(|c| c["kind"] == "mindmap"), "{controls}");
    })
}
