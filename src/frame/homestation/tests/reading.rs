//! The reading side: Pull → candidates → evaluation and filters → preparation by ObjId →
//! one reading list; followed-but-not-admitted; cold start; stream compaction (§7–§9, §14).

mod common;

use common::*;
use serde_json::{json, Value};

async fn upload(net: &Net, name: &str, file: &str, mime: &str, data: Vec<u8>) -> Value {
    let node = net.n(name);
    net.http
        .put(format!("{}/kapi/homestation/upload?name={file}&mime={mime}&width=4&height=3", node.base))
        .header("authorization", format!("Bearer {}", node.token))
        .body(data)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn reading(net: &Net, name: &str, query: Value) -> Value {
    net.rpc(name, "reading.list", json!({ "query": query, "limit": 50 })).await
}

fn all() -> Value {
    json!({ "filter": "all", "topicId": null, "search": "", "showFiltered": false })
}

/// A01, A02, A06, A41, A51–A53, A56, E19, §8: media by ObjId, preparation with hash checks,
/// local AI labels never touch the object, user corrections win and survive re-evaluation.
#[tokio::test(flavor = "multi_thread")]
async fn pipeline_media_and_filters() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    net.rpc("bob", "prefs.set_topics", json!({ "topics": [{ "id": "topic-garden", "name": "Garden", "tags": ["garden", "园艺"], "subscribed": true }] })).await;

    let photo = upload(&net, "alice", "before.png", "image/png", vec![7u8; 4096]).await;
    let photo_id = photo["objId"].as_str().unwrap().to_string();
    assert!(photo_id.starts_with("cyfile:"));
    let image_post = json!({
        "text": "my garden before and after",
        "attachments": [{ "id": "a1", "kind": "image", "name": "before.png", "status": "uploaded", "object": photo_id }],
        "link": null,
        "audience": { "kind": "public" }
    });
    let first = publish(&net, "alice", "img-1", image_post.clone()).await;
    // The same intent key never makes a second post (A41).
    let again = net.rpc("alice", "publish.create", json!({ "key": "img-1", "input": image_post })).await;
    assert_eq!(again["objId"], first["objId"]);
    let pending = net.try_rpc("alice", "publish.create", json!({ "key": "bad", "input": {
        "text": "", "attachments": [{ "id": "a2", "kind": "image", "name": "x.png", "status": "uploading" }], "link": null, "audience": { "kind": "public" } } })).await;
    assert!(pending.unwrap_err().contains("uploadPending"));

    let ai = publish(&net, "alice", "ai-1", json!({ "text": "AI Daily Digest: as an AI language model I summarized the garden news", "attachments": [], "link": null, "audience": { "kind": "public" } })).await;
    let declared = publish(&net, "alice", "ai-2", json!({ "text": "a generated poem", "attachments": [], "link": null, "audience": { "kind": "public" }, "tags": ["AI生成"] })).await;

    net.deliver_all().await;
    net.rpc("bob", "admin.run", json!({ "task": "friends" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;

    let page = reading(&net, "bob", all()).await;
    let ids: Vec<&str> = page["objIds"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(ids.contains(&first["objId"].as_str().unwrap()), "{page}");
    assert!(!ids.contains(&ai["objId"].as_str().unwrap()), "inferred AI-generated is hidden by the default rule");
    assert!(!ids.contains(&declared["objId"].as_str().unwrap()));
    assert_eq!(page["hiddenByRules"], 2);
    // The image arrived by ObjId and its chunk was checked into Bob's store.
    let card = page["cards"].as_array().unwrap().iter().find(|c| c["item"]["objId"] == first["objId"]).unwrap().clone();
    assert_eq!(card["resources"], "local", "{card}");
    assert_eq!(card["item"]["media"][0]["file"]["meta"]["mime"], "image/png");
    assert_eq!(card["reading"]["reason"]["code"], "friend");
    assert!(card["reading"]["topics"].as_array().unwrap().contains(&json!("topic-garden")), "{card}");
    let (status, body, _) = net.get(&format!("{}/objects/{photo_id}/content?access={}", bob.home, bob.token), None).await;
    assert_eq!((status, body.len()), (200, 4096));

    // Showing filtered items reveals the inferred label with its basis; the object is untouched.
    let shown = reading(&net, "bob", json!({ "filter": "all", "topicId": null, "search": "", "showFiltered": true })).await;
    let ai_card = shown["cards"].as_array().unwrap().iter().find(|c| c["item"]["objId"] == ai["objId"]).unwrap().clone();
    let tag = ai_card["reading"]["effectiveTags"].as_array().unwrap().iter().find(|t| t["tag"] == "ai_full").unwrap().clone();
    assert_eq!((tag["source"].as_str(), tag["status"].as_str()), (Some("model"), Some("inferred")));
    assert!(tag["basis"].as_str().unwrap().contains("as an ai language model"));
    assert!(ai_card["item"]["object"]["tags"].is_null(), "author tags unchanged");
    assert_eq!(ai_card["reading"]["filteredBy"], json!(["rule-ai-full"]));

    // The user removes the label: it stays removed after re-evaluation (A53).
    net.rpc("bob", "prefs.set_tag_override", json!({ "objId": ai["objId"], "tag": "ai_full", "override": "remove" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;
    let after = reading(&net, "bob", all()).await;
    assert!(after["objIds"].as_array().unwrap().contains(&ai["objId"]), "{after}");
    net.rpc("bob", "eval.refresh", json!({ "request": { "target": { "kind": "content", "object_id": ai["objId"] }, "dimensions": ["generation_method"] } })).await;
    let after = reading(&net, "bob", all()).await;
    assert!(after["objIds"].as_array().unwrap().contains(&ai["objId"]));

    // Search runs over the whole view, not the loaded page (A44).
    let search = reading(&net, "bob", json!({ "filter": "all", "topicId": null, "search": "BEFORE AND", "showFiltered": false })).await;
    assert_eq!(search["total"], 1);
    let images = reading(&net, "bob", json!({ "filter": "images", "topicId": null, "search": "", "showFiltered": false })).await;
    assert_eq!(images["total"], 1);

    // Mute Alice: hidden from the list, follow and friendship untouched (A35).
    net.rpc("bob", "prefs.set_mute_rule", json!({ "rule": { "kind": "person", "did": alice.did, "name": "alice" }, "on": true })).await;
    let muted = reading(&net, "bob", all()).await;
    assert_eq!(muted["total"], 0);
    assert!(muted["hiddenByMute"].as_u64().unwrap() >= 2);
    let sources = net.rpc("bob", "sources.list", json!({})).await;
    assert_eq!(sources["sources"][0]["basis"], json!(["friend"]));
}

/// §9.5, A37, A38, A39: followed content beyond the admission batch stays reachable, opening
/// prepares on demand, read items leave the default view.
#[tokio::test(flavor = "multi_thread")]
async fn followed_candidates_view() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    for i in 0..16 {
        publish(&net, "alice", &format!("p{i}"), text_input(&format!("note {i}"), public())).await;
    }
    net.rpc("bob", "sources.follow", json!({ "resolution": net.rpc("bob", "sources.resolve", json!({ "kind": "follow", "text": alice.did })).await })).await;
    net.deliver_all().await;
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;
    let page = reading(&net, "bob", all()).await;
    assert_eq!(page["total"], 12, "one admission batch");
    let followed = net.rpc("bob", "candidates.list", json!({ "limit": 50 })).await;
    let entries = followed["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4, "{followed}");
    assert_eq!(entries[0]["selection"], "not_selected");
    let target = entries[0]["objId"].as_str().unwrap().to_string();
    // Media of a not-yet-prepared candidate is fetched on demand for the owner.
    let photo = upload(&net, "alice", "late.png", "image/png", vec![9u8; 2048]).await;
    let late = publish(&net, "alice", "late", json!({ "text": "late photo", "attachments": [{ "id": "a", "kind": "image", "name": "late.png", "status": "uploaded", "object": photo["objId"] }], "link": null, "audience": { "kind": "public" } })).await;
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    let late_card = net.rpc("bob", "item.get", json!({ "objId": late["objId"] })).await;
    assert_eq!(late_card["item"]["media"][0]["file"]["meta"]["mime"], "image/png");
    let url = format!("{}/objects/{}/content?access={}", bob.home, photo["objId"].as_str().unwrap(), bob.token);
    let (status, body, _) = net.get(&url, None).await;
    assert_eq!((status, body.len()), (200, 2048));
    let opened = net.rpc("bob", "candidates.open", json!({ "objId": target })).await;
    assert_eq!(opened["ok"], true);
    // Three left of the first batch plus the late photo; the opened one is read.
    let followed = net.rpc("bob", "candidates.list", json!({ "limit": 50 })).await;
    assert_eq!(followed["entries"].as_array().unwrap().len(), 4);
    assert_eq!(followed["readCount"], 1);
    let with_read = net.rpc("bob", "candidates.list", json!({ "limit": 50, "includeRead": true })).await;
    assert_eq!(with_read["entries"].as_array().unwrap().len(), 5);
    // Admitted items never reappear as "never admitted", even after leaving the window (A39).
    bob.station.db.call(|c| Ok(c.execute("DELETE FROM reading", [])?)).await.unwrap();
    let followed = net.rpc("bob", "candidates.list", json!({ "limit": 50 })).await;
    assert_eq!(followed["entries"].as_array().unwrap().len(), 4);
}

/// §14.3, A18: a new user with no friends or follows gets content from a preset collector.
#[tokio::test(flavor = "multi_thread")]
async fn cold_start_from_collector() {
    let net = Net::new(&["alice", "nora", "index"], &["index"]).await;
    let post = publish(&net, "alice", "p1", text_input("community garden opens saturday", public())).await;
    publish(&net, "alice", "p2", text_input("friends only", json!({ "kind": "friends" }))).await;
    net.deliver_all().await;
    let nora = net.n("nora");
    nora.station.pull_collectors().await.unwrap();
    net.rpc("nora", "admin.run", json!({ "task": "selection" })).await;
    let page = reading(&net, "nora", all()).await;
    assert_eq!(page["objIds"], json!([post["objId"]]), "{page}");
    assert_eq!(page["cards"][0]["reading"]["reason"]["code"], "collector");
}

/// §4.4, A67, A68: change cursors; compaction forces an explicit re-sync, never a silent skip.
#[tokio::test(flavor = "multi_thread")]
async fn change_read_and_compaction() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    publish(&net, "alice", "p1", text_input("one", public())).await;
    net.rpc("bob", "sources.follow", json!({ "resolution": net.rpc("bob", "sources.resolve", json!({ "kind": "follow", "text": alice.did })).await })).await;
    net.deliver_all().await;
    let source_id = net.rpc("bob", "sources.list", json!({})).await["sources"][0]["id"].as_str().unwrap().to_string();
    let first = net.rpc("bob", "sources.sync", json!({ "sourceId": source_id })).await;
    assert!(!first["resynced"].as_bool().unwrap());
    let second = publish(&net, "alice", "p2", text_input("two", public())).await;
    net.rpc("alice", "entry.withdraw", json!({ "entry": second["entry"] })).await;
    let report = net.rpc("bob", "sources.sync", json!({ "sourceId": source_id })).await;
    assert_eq!(report["heads"], 2, "new entry and its withdrawal: {report}");
    // Alice no longer serves the withdrawn object; Bob keeps the verified withdrawn Head (§16.6).
    let entry = second["entry"].as_str().unwrap().to_string();
    let head = bob.station.db.call(move |c| homestation::publish::get_head(c, &entry)).await.unwrap().unwrap();
    assert_eq!((head.seq, head.state), (2, homestation::protocol::HeadState::Withdrawn));

    publish(&net, "alice", "p3", text_input("three", public())).await;
    let changes: Value = serde_json::from_str(&net.get(&format!("{}/feed?mode=changes&since=0", alice.home), None).await.1).unwrap();
    let last = changes["next_cursor"].as_i64().unwrap();
    net.rpc("alice", "admin.compact_stream", json!({ "through": last })).await;
    let stale: Value = serde_json::from_str(&net.get(&format!("{}/feed?mode=changes&since=1", alice.home), None).await.1).unwrap();
    assert_eq!(stale["resync"], true);
    let report = net.rpc("bob", "sources.sync", json!({ "sourceId": source_id })).await;
    assert_eq!(report["resynced"], true);
}
