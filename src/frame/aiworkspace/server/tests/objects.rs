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

        let page: Value = http
            .post(format!("{obj}/methods/query"))
            .header("authorization", "Bearer tok-alice")
            .json(&json!({ "method": "query", "params": {} }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let objects: Vec<&str> = page["result"]["rows"].as_array().unwrap().iter().filter_map(|r| r["object"].as_str()).collect();
        assert!(objects.contains(&format!("obj://aiworkspace/{ws}/entities/orders").as_str()), "{page}");
    })
}
