//! DID Object Provider (`/kapi/aiworkspace/obj/...`): Card / Profile / reads of
//! `obj://aiworkspace/<ws>/...` with the caller's session, and the Reference url projection.

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
