//! One service, every user of the zone (§4.4): own homes at `/home/<user>`, DID lookup,
//! in-zone follows and Push through the zone's own listener, isolation between users, other
//! zones reading a zone user; the zone feed and the default feed (§4.6).

mod common;

use common::*;
use homestation::protocol::{HomeRef, OBJ_TYPE_HEAD};
use serde_json::{json, Value};

async fn get_json(net: &Net, url: &str, auth: Option<String>) -> (u16, Value) {
    let (status, body, _) = net.get(url, auth).await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// kRPC without a session (anonymous portal reads).
async fn anonymous_rpc(net: &Net, node: &str, method: &str, params: Value) -> Value {
    let body = json!({ "method": method, "params": params, "sys": [1] });
    let response: Value = net.http.post(format!("{}/kapi/homestation", net.n(node).base)).json(&body).send().await.unwrap().json().await.unwrap();
    assert!(response.get("error").is_none(), "{method}: {response}");
    response["result"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_user_has_a_home() {
    let net = Net::new_zones(&["lin", "alice"], &[], &[], &[("lin", "kai"), ("lin", "mia")], &[]).await;
    let (lin, kai, mia, alice) = (net.n("lin"), net.n("kai"), net.n("mia"), net.n("alice"));

    // The zone index and DID lookup (§4.4 发现).
    let (status, index) = get_json(&net, &format!("{}/home/", lin.base), None).await;
    assert_eq!(status, 200);
    assert_eq!(index["defaultFeed"], "lin", "without a setting the default feed is the zone owner's");
    let (status, located) = get_json(&net, &format!("{}/home/?did={}", lin.base, kai.did), None).await;
    assert_eq!(status, 200);
    assert_eq!(located["user"], "kai");
    assert_eq!(located["stream"], "cyfs://lin.test/home/kai/feed");
    assert_eq!(located["inbox"], "cyfs://lin.test/home/kai/inbox");
    assert_eq!(net.get(&format!("{}/home/?did={}", lin.base, alice.did), None).await.0, 404);
    assert_eq!(net.get(&format!("{}/home/nobody/feed", lin.base), None).await.0, 404);
    let (_, profile) = get_json(&net, &format!("{}/profile", kai.home), None).await;
    assert_eq!((profile["did"].as_str(), profile["user"].as_str()), (Some(kai.did.as_str()), Some("kai")));

    // Users of one zone are friends: they follow each other and Push to each other through
    // the zone's own listener, like users of different zones.
    lin.add_contact(kai, true, &[]);
    kai.add_contact(lin, true, &[]);
    for n in ["lin", "kai"] {
        net.rpc(n, "admin.run", json!({ "task": "friends" })).await;
    }
    net.deliver_all().await;
    assert_eq!(kai.station.active_followers().await.unwrap(), vec![lin.did.clone()]);
    assert_eq!(lin.station.active_followers().await.unwrap(), vec![kai.did.clone()]);
    let dinner = publish(&net, "kai", "k1", text_input("dinner at home, friends only", json!({ "kind": "friends" }))).await;
    net.deliver_all().await;
    let card = net.rpc("lin", "item.get", json!({ "objId": dinner["objId"] })).await;
    assert_eq!(card["item"]["publisher"]["did"], kai.did, "{card}");
    let accepted: i64 = kai
        .station
        .db
        .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM outbox WHERE state='accepted'", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert!(accepted >= 2, "object and Head accepted by lin's inbox");
    // Pull works the same way.
    let sources = net.rpc("lin", "sources.list", json!({})).await;
    let kai_source = sources["sources"].as_array().unwrap().iter().find(|s| s["did"] == kai.did.as_str()).unwrap()["id"].clone();
    net.rpc("lin", "sources.sync", json!({ "sourceId": kai_source })).await;

    // Audience per reader: a friend reads the friends-only post, another user of the zone does not.
    let feed = format!("{}/feed?mode=display", kai.home);
    let (_, as_mia) = get_json(&net, &feed, Some(net.proof("mia", "lin"))).await;
    assert_eq!(as_mia["items"].as_array().unwrap().len(), 0);
    let (_, as_lin) = get_json(&net, &feed, Some(net.proof("lin", "lin"))).await;
    assert_eq!(as_lin["items"].as_array().unwrap().len(), 1);
    let (_, as_lin_session) = get_json(&net, &format!("{feed}&access={}", lin.token), None).await;
    assert_eq!(as_lin_session["items"].as_array().unwrap().len(), 1);

    // State is per user: each session reaches only its own HomeStation.
    publish(&net, "lin", "l1", text_input("lin's own post", public())).await;
    let own = net.rpc("kai", "published.list", json!({})).await;
    assert_eq!(own["entries"].as_array().unwrap().len(), 1);
    assert!(own["entries"][0]["entry"].as_str().unwrap().starts_with("cyfs://lin.test/home/kai/feed/@/"));
    net.rpc("kai", "prefs.set_default_audience", json!({ "audience": { "kind": "friends" } })).await;
    assert_eq!(net.rpc("lin", "prefs.get", json!({})).await["defaultAudience"]["kind"], "public");
    assert_eq!(net.rpc("mia", "ui.bootstrap", json!({})).await["home"]["user"], "mia");
    let err = net.try_rpc("mia", "entry.withdraw", json!({ "entry": dinner["entry"] })).await.unwrap_err();
    assert!(err.contains("not_found"), "{err}");

    // Another zone follows a user of this zone: discovery, declaration and Pull by the user's
    // path; the zone key's signatures verify (zone custody).
    let kai_post = publish(&net, "kai", "k2", text_input("kai's public note", public())).await;
    net.rpc("alice", "sources.follow", json!({ "resolution": net.rpc("alice", "sources.resolve", json!({ "kind": "follow", "text": kai.did })).await })).await;
    net.deliver_all().await;
    let mut followers = kai.station.active_followers().await.unwrap();
    followers.sort();
    assert_eq!(followers, vec![alice.did.clone(), lin.did.clone()]);
    let sources = net.rpc("alice", "sources.list", json!({})).await;
    let source = sources["sources"][0]["id"].clone();
    let report = net.rpc("alice", "sources.sync", json!({ "sourceId": source })).await;
    assert!(report["heads"].as_u64().unwrap() >= 1, "{report}");
    let remote = net.rpc("alice", "published.list", json!({ "owner": kai.did })).await;
    let ids: Vec<&str> = remote["entries"].as_array().unwrap().iter().map(|e| e["objId"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![kai_post["objId"].as_str().unwrap()]);
    // Heads of another user of the zone verify as theirs only: a forged home path does not.
    let forged = HomeRef::new("lin.test", "lin").entry(homestation::protocol::EntryNamespace::Feed, "x");
    assert!(!homestation::objects::entry_belongs_to(alice.station.directory.as_ref(), &kai.did, &forged).await.unwrap());
    let _ = mia;
}

#[tokio::test(flavor = "multi_thread")]
async fn zone_feed_lists_and_follows_changes() {
    let net = Net::new_zones(
        &["lin", "alice"],
        &[],
        &[],
        &[("lin", "kai"), ("lin", "mia")],
        &[("lin", json!({ "default_feed": "~zone", "zone_feed_writers": ["lin", "kai"], "zone_name": "Lin's zone" }))],
    )
    .await;
    let (lin, kai, alice) = (net.n("lin"), net.n("kai"), net.n("alice"));
    let (_, index) = get_json(&net, &format!("{}/home/", lin.base), None).await;
    assert_eq!(index["defaultFeed"], "~zone");
    assert_eq!(index["zoneFeed"]["stream"], "cyfs://lin.test/home/~zone/feed");

    // Writers list public posts; others and restricted posts are refused before publishing.
    let mut input = text_input("kai on the zone page", public());
    input["zoneFeed"] = json!(true);
    let kai_post = publish(&net, "kai", "k1", input.clone()).await;
    let err = net.try_rpc("mia", "publish.create", json!({ "key": "m1", "input": input })).await.unwrap_err();
    assert!(err.contains("notWriter"), "{err}");
    let mut restricted = text_input("friends only", json!({ "kind": "friends" }));
    restricted["zoneFeed"] = json!(true);
    let err = net.try_rpc("kai", "publish.create", json!({ "key": "k2", "input": restricted })).await.unwrap_err();
    assert!(err.contains("zoneFeedPublic"), "{err}");
    assert!(net.rpc("mia", "published.list", json!({})).await["entries"].as_array().unwrap().is_empty());
    let image: Value = net
        .http
        .put(format!("{}/kapi/homestation/upload?name=p.png&mime=image/png&width=4&height=3", lin.base))
        .header("authorization", format!("Bearer {}", lin.token))
        .body(vec![7u8; 64])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let lin_post = publish(
        &net,
        "lin",
        "l1",
        json!({ "text": "the garden", "attachments": [{ "id": "1", "kind": "image", "name": "p", "status": "uploaded", "object": image["objId"] }],
            "link": null, "audience": { "kind": "public" }, "zoneFeed": true }),
    )
    .await;
    publish(&net, "kai", "k3", text_input("only on kai's own page", public())).await;
    let own = net.rpc("kai", "published.list", json!({})).await;
    let listed: Vec<&Value> = own["entries"].as_array().unwrap().iter().filter(|e| e["zoneFeed"] == true).map(|e| &e["entry"]).collect();
    assert_eq!(listed, vec![&kai_post["entry"]]);
    assert_eq!(own["entries"].as_array().unwrap().len(), 2);

    // The zone feed is a list of the publishers' own Heads: anyone reads and verifies them.
    let (status, page) = get_json(&net, &format!("{}/home/~zone/feed?mode=display&objects=1", lin.base), None).await;
    assert_eq!(status, 200);
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let mut publishers = Vec::new();
    for item in items {
        let v = homestation::objects::verify_jwt(alice.station.directory.as_ref(), item["head"].as_str().unwrap(), Some(OBJ_TYPE_HEAD)).await.unwrap();
        publishers.push(v.publisher);
    }
    assert_eq!(publishers, vec![lin.did.clone(), kai.did.clone()]);
    assert!(page["objects"].get(lin_post["objId"].as_str().unwrap()).is_some());
    let (status, _, headers) = net.get(&format!("{}/home/~zone/objects/{}/content", lin.base, image["objId"].as_str().unwrap()), None).await;
    assert_eq!((status, headers["content-type"].to_str().unwrap()), (200, "image/png"));
    let (_, zone_profile) = get_json(&net, &format!("{}/home/~zone/profile", lin.base), None).await;
    assert_eq!((zone_profile["name"].as_str(), zone_profile["posts"].as_i64()), (Some("Lin's zone"), Some(2)));

    // Portal methods need no session.
    let home = anonymous_rpc(&net, "lin", "portal.home", json!({})).await;
    assert_eq!((home["defaultFeed"].as_str(), home["viewer"].is_null()), (Some("~zone"), true));
    let portal = anonymous_rpc(&net, "lin", "portal.list", json!({ "feed": "~zone" })).await;
    let users: Vec<&str> = portal["entries"].as_array().unwrap().iter().map(|e| e["user"].as_str().unwrap()).collect();
    assert_eq!(users, vec!["lin", "kai"]);
    assert_eq!(portal["cards"].as_array().unwrap().len(), 2);
    let kai_page = anonymous_rpc(&net, "lin", "portal.list", json!({ "feed": "kai" })).await;
    assert_eq!(kai_page["entries"].as_array().unwrap().len(), 2);
    let key = kai_post["entry"].as_str().unwrap().rsplit('/').next().unwrap();
    let item = anonymous_rpc(&net, "lin", "portal.item", json!({ "feed": "kai", "key": key })).await;
    assert_eq!(item["card"]["item"]["objId"], kai_post["objId"]);
    assert_eq!(net.rpc("kai", "portal.home", json!({})).await["viewer"]["zoneFeedWriter"], true);
    assert_eq!(net.rpc("mia", "portal.home", json!({})).await["viewer"]["zoneFeedWriter"], false);
    assert_eq!(net.rpc("kai", "ui.bootstrap", json!({})).await["home"]["zoneFeed"]["writer"], true);

    // Followers of the zone feed see edits, withdrawals and unlistings in the change read.
    let edited = net.rpc("kai", "entry.edit", json!({ "entry": kai_post["entry"], "text": "kai on the zone page, edited" })).await;
    net.rpc("kai", "entry.withdraw", json!({ "entry": kai_post["entry"] })).await;
    net.rpc("lin", "zone.set_listing", json!({ "entry": lin_post["entry"], "listed": false })).await;
    let (_, changes) = get_json(&net, &format!("{}/home/~zone/feed?mode=changes&since=0", lin.base), None).await;
    let kinds: Vec<&str> = changes["changes"].as_array().unwrap().iter().map(|c| c["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["listed", "listed", "head", "head", "unlisted"]);
    let last_kai = changes["changes"].as_array().unwrap().iter().filter(|c| c["entry"] == kai_post["entry"]).last().unwrap().clone();
    let head = homestation::sign::decode_unverified(last_kai["head"].as_str().unwrap()).unwrap().claims;
    assert_eq!(head["state"], "withdrawn");
    assert!(edited.is_string());
    let (_, page) = get_json(&net, &format!("{}/home/~zone/feed?mode=display", lin.base), None).await;
    assert!(page["items"].as_array().unwrap().is_empty());
    let err = net.try_rpc("kai", "zone.set_listing", json!({ "entry": kai_post["entry"], "listed": true })).await.unwrap_err();
    assert!(err.contains("withdrawn"), "{err}");
    let _ = kai;
}
