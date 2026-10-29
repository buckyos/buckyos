//! DID Object Provider (`/kapi/homestation/obj/...`): Card, Profile and reads of
//! `obj://homestation/home/<user>/...`, with the reader rules of the protocol paths.

mod common;

use common::*;
use serde_json::{json, Value};

async fn get_json(net: &Net, url: &str, auth: Option<String>) -> (u16, Value) {
    let (status, body, _) = net.get(url, auth).await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn home_objects_have_https_default_urls_and_follow_reader_rules() {
    let net = Net::new(&["alice"], &[]).await;
    let alice = net.n("alice");
    let post = publish(&net, "alice", "p1", text_input("hello objects", json!({ "kind": "public" }))).await;
    let entry = post["entry"].as_str().unwrap().to_string();
    let obj = format!("{}/kapi/homestation/obj/home/alice", alice.base);
    let bearer = Some(format!("Bearer {}", alice.token));

    let (status, profile) = get_json(&net, &format!("{obj}/feed?type=profile"), None).await;
    assert_eq!(status, 200, "{profile}");
    name_lib::ObjectProfile::from_value(profile).unwrap().validate().unwrap();

    let (status, card) = get_json(&net, &format!("{obj}/feed"), bearer.clone()).await;
    assert_eq!(status, 200, "{card}");
    let aka: Vec<&str> = card["alsoKnownAs"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(aka[0].starts_with("https://") && aka[0].ends_with("/home/alice/feed"), "{aka:?}");
    assert_eq!(aka.last().copied(), Some("obj://homestation/home/alice/feed"));

    let body = json!({ "method": "query", "params": {} });
    let page: Value = net.http.post(format!("{obj}/feed/methods/query")).header("authorization", bearer.clone().unwrap()).json(&body).send().await.unwrap().json().await.unwrap();
    let rows = page["result"]["rows"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["object"] == json!(entry)), "{page}");
    assert!(entry.starts_with("https://"), "entries are https object urls: {entry}");

    let key = entry.rsplit('/').next().unwrap();
    let (status, head) = get_json(&net, &format!("{obj}/feed/@/{key}/props/head"), bearer.clone()).await;
    assert_eq!(status, 200, "{head}");
    let (status, inbox) = get_json(&net, &format!("{obj}/inbox/props/target"), bearer).await;
    assert_eq!(status, 200);
    assert!(inbox["target"].as_str().unwrap().ends_with("/home/alice/inbox"));
    assert_eq!(get_json(&net, &format!("{obj}/nothing"), None).await.0, 404);
}
