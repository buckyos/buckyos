//! Sources and delivery robustness: offline recipients and retries, the Spider on a local
//! fixture site, friend-derived follows, natural-language intents (§5.6, §7.6, §7.7, §17.3).

mod common;

use axum::response::IntoResponse;
use common::*;
use serde_json::{json, Value};

async fn fixture_site() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let feed_base = base.clone();
    let app = axum::Router::new()
        .route(
            "/feed.xml",
            axum::routing::get(move || {
                let b = feed_base.clone();
                async move {
                    let xml = format!(
                        r#"<?xml version="1.0"?><rss><channel><title>Garden Weekly</title>
                        <item><title>Raised beds in small spaces</title><link>{b}/a.html</link><description>Four boxes on a balcony</description><dc:creator>Ann Gardener</dc:creator></item>
                        <item><title>Watering by the moon</title><link>{b}/b.html</link><description>Folklore checked</description></item>
                        </channel></rss>"#
                    );
                    ([("content-type", "application/rss+xml")], xml).into_response()
                }
            }),
        )
        .route(
            "/a.html",
            axum::routing::get(|| async {
                ([("content-type", "text/html")], "<html><head><title>Raised beds</title><meta property=\"og:title\" content=\"Raised beds in small spaces\"></head><body><p>Build four boxes. garden notes.</p><script>track()</script></body></html>").into_response()
            }),
        )
        .route("/b.html", axum::routing::get(|| async { ([("content-type", "text/html")], "<html><body>moon</body></html>").into_response() }))
        .route(
            "/site",
            axum::routing::get(|| async {
                ([("content-type", "text/html")], "<html><head><title>Site</title><link rel=\"alternate\" type=\"application/rss+xml\" href=\"/feed.xml\"></head></html>").into_response()
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    base
}

/// A13, A33, §17.3: an offline author or followee only delays convergence; the sender keeps
/// retrying, never reports success early, and the receiver gets it once it is back.
#[tokio::test(flavor = "multi_thread")]
async fn offline_recipient_retry() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    bob.stop();
    let post = publish(&net, "alice", "p1", text_input("while you were away", public())).await;
    net.deliver_all().await;
    let task = net.rpc("alice", "publish.task", json!({ "key": "p1" })).await;
    assert_eq!(task["delivery"]["state"], "delivering", "{task}");
    let row: (String, i64, String) = alice
        .station
        .db
        .call(|c| Ok(c.query_row("SELECT state, attempts, last_result FROM outbox WHERE purpose='object'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
        .await
        .unwrap();
    assert_eq!((row.0.as_str(), row.1), ("pending", 1));
    assert!(row.2.contains("no_response"), "{}", row.2);
    bob.start().await;
    alice.station.db.call(|c| Ok(c.execute("UPDATE outbox SET next_at=0", [])?)).await.unwrap();
    net.deliver_all().await;
    let task = net.rpc("alice", "publish.task", json!({ "key": "p1" })).await;
    assert_eq!(task["delivery"]["state"], "delivered");
    let card = net.rpc("bob", "item.get", json!({ "objId": post["objId"] })).await;
    assert_eq!(card["item"]["publisher"]["did"], alice.did);
}

/// §5.6, §18.1, A03, A45, A72: RSS becomes private captures (never in the stream, no
/// notification), a selected card gets a snapshot, sharing publishes the owner's own object.
#[tokio::test(flavor = "multi_thread")]
async fn spider_private_captures_and_share() {
    let site = fixture_site().await;
    let net = Net::new_with(&["bob", "carol"], &[], &["bob"]).await;
    let (bob, carol) = (net.n("bob"), net.n("carol"));
    bob.add_contact(carol, true, &[]);
    carol.add_contact(bob, true, &[]);
    net.rpc("bob", "prefs.set_topics", json!({ "topics": [{ "id": "t-garden", "name": "garden", "tags": ["garden", "boxes"], "subscribed": true }] })).await;
    // A web page that advertises its feed resolves to the RSS source.
    let resolution = net.rpc("bob", "sources.resolve", json!({ "kind": "url", "text": format!("{site}/site") })).await;
    assert_eq!(resolution["candidates"][0]["kind"], "rss", "{resolution}");
    assert_eq!(resolution["candidates"][0]["notify"], "unsupported");
    let added = net.rpc("bob", "sources.follow", json!({ "resolution": resolution })).await;
    assert_eq!(added[0]["url"], format!("{site}/feed.xml"));
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    let captures: i64 = bob.station.db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM candidates WHERE private_capture=1 AND selection!='dropped'", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(captures, 2, "each item once");
    net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;
    let page = net.rpc("bob", "reading.list", json!({ "query": { "filter": "news", "topicId": null, "search": "", "showFiltered": false } })).await;
    let card = page["cards"].as_array().unwrap().iter().find(|c| c["item"]["object"]["content"]["title"] == "Raised beds in small spaces").unwrap().clone();
    assert_eq!(card["item"]["isPrivateCapture"], true);
    assert_eq!(card["item"]["originalAuthor"], "Ann Gardener");
    assert_eq!(card["item"]["capturedFrom"], "127.0.0.1");
    assert_eq!(card["canRepost"], false);
    assert_eq!(card["repostBlockedReason"], "private_capture");
    // Selection replaced the link card by a snapshot-wrapping object (§18.1 step 4).
    assert_eq!(card["item"]["contentType"], "article");
    assert_eq!(card["item"]["wrappedFile"]["meta"]["mime"], "text/html");
    let body = net.rpc("bob", "item.wrapped_body", json!({ "objId": card["item"]["objId"] })).await;
    assert_eq!(body["state"], "ready");
    assert!(body["markdown"].as_str().unwrap().contains("Build four boxes"));
    // Private captures are not in Bob's stream (A72).
    let (_, stream, _) = net.get(&format!("{}/feed?mode=display", bob.home), Some(net.proof("carol", "bob"))).await;
    assert_eq!(serde_json::from_str::<Value>(&stream).unwrap()["items"].as_array().unwrap().len(), 0);
    let capture_id = card["item"]["objId"].as_str().unwrap().to_string();
    assert_eq!(net.get(&format!("{}/objects/{capture_id}", bob.home), Some(net.proof("carol", "bob"))).await.0, 404);

    // Sharing publishes Bob's own object wrapping the snapshot, with the source kept as data.
    let task = net.rpc("bob", "publish.share_capture", json!({ "objId": capture_id })).await;
    assert_eq!(task["stage"], "published");
    net.deliver_all().await;
    let shared = net.rpc("carol", "item.get", json!({ "objId": task["objId"] })).await;
    assert_eq!(shared["item"]["verification"], "wrapper_only");
    assert_eq!(shared["item"]["isPrivateCapture"], false);
    assert_eq!(shared["item"]["object"]["source"]["original_url"], format!("{site}/a.html"));
    let again = net.rpc("bob", "item.get", json!({ "objId": capture_id })).await;
    assert_eq!(again["sharedAs"], task["objId"]);
    // Carol fetches the snapshot from Bob once it is part of a visible publication.
    let snapshot = shared["item"]["object"]["wraps"].as_str().unwrap().to_string();
    assert_eq!(net.get(&format!("{}/objects/{snapshot}", bob.home), Some(net.proof("carol", "bob"))).await.0, 200);
}

/// §7.6, A34, A36: Message Center friends follow each other by default; losing the friend
/// basis withdraws the follow unless an active basis remains.
#[tokio::test(flavor = "multi_thread")]
async fn friend_derived_follows() {
    let net = Net::new(&["alice", "bob", "carol"], &[]).await;
    let (alice, bob, carol) = (net.n("alice"), net.n("bob"), net.n("carol"));
    for (a, b) in [(alice, bob), (alice, carol)] {
        a.add_contact(b, true, &[]);
        b.add_contact(a, true, &[]);
    }
    for n in ["alice", "bob", "carol"] {
        net.rpc(n, "admin.run", json!({ "task": "friends" })).await;
    }
    net.deliver_all().await;
    let mut followers = alice.station.active_followers().await.unwrap();
    followers.sort();
    assert_eq!(followers, vec![bob.did.clone(), carol.did.clone()]);
    let profile = net.rpc("alice", "profile.get", json!({})).await;
    assert_eq!((profile["followers"].as_i64(), profile["following"].as_i64()), (Some(2), Some(2)));
    // Carol also follows actively; then both friendships end.
    net.rpc("carol", "sources.follow", json!({ "resolution": net.rpc("carol", "sources.resolve", json!({ "kind": "follow", "text": alice.did })).await })).await;
    let sources = net.rpc("bob", "sources.list", json!({})).await;
    let friend_only = sources["sources"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(net.rpc("bob", "sources.unfollow", json!({ "sourceId": friend_only })).await, "friend_basis");
    bob.remove_contact(alice);
    carol.remove_contact(alice);
    net.rpc("bob", "admin.run", json!({ "task": "friends" })).await;
    net.rpc("carol", "admin.run", json!({ "task": "friends" })).await;
    net.deliver_all().await;
    assert_eq!(alice.station.active_followers().await.unwrap(), vec![carol.did.clone()], "Carol's active basis keeps the follow");
    assert_eq!(net.rpc("bob", "sources.list", json!({})).await["sources"].as_array().unwrap().len(), 0);
}

/// §7.7, A42: a natural-language subscription is a kept intent with a Topic and a mapping.
#[tokio::test(flavor = "multi_thread")]
async fn natural_language_intent() {
    let net = Net::new(&["bob", "index"], &["index"]).await;
    let resolution = net.rpc("bob", "sources.resolve", json!({ "kind": "natural", "text": "second-hand bikes near Palo Alto" })).await;
    assert_eq!(resolution["intentText"], "second-hand bikes near Palo Alto");
    assert_eq!(resolution["candidates"][0]["kind"], "channel");
    net.rpc("bob", "sources.follow", json!({ "resolution": resolution })).await;
    let list = net.rpc("bob", "sources.list", json!({})).await;
    assert_eq!(list["intents"][0]["status"], "collecting");
    assert_eq!(list["intents"][0]["sourceIds"].as_array().unwrap().len(), 1);
    let settings = net.rpc("bob", "prefs.get", json!({})).await;
    let topic = settings["topics"].as_array().unwrap().iter().find(|t| t["tags"].as_array().unwrap().contains(&json!("bikes"))).unwrap().clone();
    assert_eq!(topic["subscribed"], true);
}

/// Friends without a HomeStation (an agent of the zone, say) have no stream: no follow, no
/// declaration, no Push; following them explicitly says so.
#[tokio::test(flavor = "multi_thread")]
async fn friends_without_homestation() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    let kid = "did:test:alice-kid";
    alice.contacts.0.lock().unwrap().push(homestation::contacts::ContactInfo { did: kid.into(), name: "kid".into(), friend: true, blocked: false, groups: vec![] });
    alice.add_contact(bob, true, &[]);
    net.rpc("alice", "admin.run", json!({ "task": "friends" })).await;
    let sources = net.rpc("alice", "sources.list", json!({})).await;
    let dids: Vec<&str> = sources["sources"].as_array().unwrap().iter().map(|s| s["did"].as_str().unwrap()).collect();
    assert_eq!(dids, vec![bob.did.as_str()]);
    let err = net.try_rpc("alice", "sources.follow", json!({ "resolution": { "inputKind": "follow", "input": kid, "notifyHint": "pending",
        "candidates": [{ "id": "src-kid", "name": "kid", "kind": "person", "did": kid, "basis": [], "notify": "pending", "paused": false }] } })).await.unwrap_err();
    assert!(err.contains("noHome"), "{err}");
    publish(&net, "alice", "p1", text_input("friends only", json!({ "kind": "friends" }))).await;
    let queued: Vec<String> = alice.station.db.call(|c| {
        let mut stmt = c.prepare("SELECT DISTINCT recipient FROM outbox")?;
        let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(rows)
    }).await.unwrap();
    assert_eq!(queued, vec![bob.did.clone()]);
    let boot = net.rpc("alice", "ui.bootstrap", json!({})).await;
    assert!(boot["friends"].as_array().unwrap().contains(&json!(kid)));
}

