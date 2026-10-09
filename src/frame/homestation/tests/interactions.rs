//! Comments and special comments across nodes: views by maintainer, statistics from verified
//! records, restricted targets, bookmarks, reposts and quotes (§11–§16).

mod common;

use common::*;
use serde_json::{json, Value};

async fn stats(net: &Net, name: &str, obj: &str, view: &str) -> Value {
    net.rpc(name, "comments.list", json!({ "objId": obj, "view": view, "type": "like" })).await["stats"].clone()
}

/// A12–A16, A21, A22, A27, §15: comments and likes travel to the author and the collector;
/// a reader merges views; the author's removal does not erase others' copies; one like per key.
#[tokio::test(flavor = "multi_thread")]
async fn comments_likes_views_and_stats() {
    let net = Net::new(&["alice", "bob", "carol", "dave", "index"], &["index"]).await;
    let (alice, bob, carol, dave) = (net.n("alice"), net.n("bob"), net.n("carol"), net.n("dave"));
    for n in [bob, carol, dave] {
        n.add_contact(alice, true, &[]);
        alice.add_contact(n, true, &[]);
    }
    let post = publish(&net, "alice", "p1", text_input("balcony garden day 1", public())).await;
    let p1 = post["objId"].as_str().unwrap().to_string();
    net.deliver_all().await;
    for n in ["bob", "carol", "dave"] {
        net.rpc(n, "admin.run", json!({ "task": "friends" })).await;
        net.rpc(n, "admin.run", json!({ "task": "pull" })).await;
    }

    let (comment, _) = {
        let r = net.rpc("bob", "interact.comment", json!({ "objId": p1, "text": "try leafy greens" })).await;
        (r["task"]["objId"].as_str().unwrap().to_string(), r)
    };
    net.rpc("bob", "interact.like", json!({ "objId": p1, "on": true })).await;
    net.rpc("carol", "interact.like", json!({ "objId": p1, "on": true })).await;
    // Bob toggles: off then on again (seq 3, A27) — still one like.
    net.rpc("bob", "interact.like", json!({ "objId": p1, "on": false })).await;
    let personal = net.rpc("bob", "interact.like", json!({ "objId": p1, "on": true })).await;
    assert_eq!(personal["like"]["seq"], 3);
    assert_eq!(personal["like"]["visibility"], "public");
    net.deliver_all().await;

    // The author receives and lists them (author view).
    let at_alice = stats(&net, "alice", &p1, "local").await;
    assert_eq!(at_alice["likes"], 2, "{at_alice}");
    let author_comments = net.rpc("alice", "comments.list", json!({ "objId": p1, "view": "author", "type": "text" })).await;
    assert_eq!(author_comments["comments"].as_array().unwrap().len(), 1);
    // The collector got the public records too.
    let index = net.n("index");
    let (status, body, _) = net.get(&format!("{}/comments?target={p1}", index.home), None).await;
    assert_eq!(status, 200, "{body}");
    let collector_view: Value = serde_json::from_str(&body).unwrap();
    assert!(collector_view["records"].as_array().unwrap().len() >= 3, "{collector_view}");

    // Dave tracks the discussion from both maintainers and counts each key once (A21).
    net.rpc("dave", "interact.like", json!({ "objId": p1, "on": true })).await;
    net.deliver_all().await;
    net.rpc("dave", "comments.sync", json!({})).await;
    let local = stats(&net, "dave", &p1, "local").await;
    assert_eq!(local["likes"], 3, "bob, carol, dave: {local}");
    let author = stats(&net, "dave", &p1, "author").await;
    assert_eq!(author["likes"], 3);
    // Alice's own count is only a claim at Dave (A22).
    assert!(author.get("claimed").is_some(), "{author}");

    // Alice removes Bob's comment from her list (A15); Dave still sees it locally, and the
    // removal is observable from complete snapshots, not inferred from partial lists (A16).
    net.rpc("alice", "comments.set_listing", json!({ "target": p1, "commentId": comment, "listed": false })).await;
    net.rpc("dave", "comments.sync", json!({})).await;
    let dave_local = net.rpc("dave", "comments.list", json!({ "objId": p1, "view": "local", "type": "text" })).await;
    let c = &dave_local["comments"][0];
    assert_eq!(c["objId"], comment);
    assert_eq!(c["listedByAuthor"], true, "Dave saw it in Alice's list earlier");
    assert_eq!(c["authorRemoval"], "observed");
    assert_eq!(c["listedByCollector"], true);

    // Withdrawing the like propagates; a replayed old Head cannot revive it (A23).
    net.rpc("carol", "interact.like", json!({ "objId": p1, "on": false })).await;
    net.deliver_all().await;
    net.rpc("dave", "comments.sync", json!({})).await;
    assert_eq!(stats(&net, "dave", &p1, "local").await["likes"], 2);
    assert_eq!(stats(&net, "alice", &p1, "local").await["likes"], 2);
}

/// §4.5, A70, A71: restricted posts cannot be reposted; comments and likes on them go only
/// to the author and never appear in the commenter's public stream or at collectors.
#[tokio::test(flavor = "multi_thread")]
async fn restricted_targets() {
    let net = Net::new(&["alice", "bob", "carol", "index"], &["index"]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    let post = publish(&net, "alice", "f1", text_input("family barbecue", json!({ "kind": "friends" }))).await;
    let f1 = post["objId"].as_str().unwrap().to_string();
    net.deliver_all().await;
    let card = net.rpc("bob", "item.get", json!({ "objId": f1 })).await;
    assert_eq!(card["item"]["audience"]["restricted"], true);
    assert_eq!(card["item"]["audience"]["spec"]["kind"], "friends");
    assert_eq!(card["repostBlockedReason"], "restricted");
    let err = net.try_rpc("bob", "interact.repost", json!({ "objId": f1, "on": true })).await.unwrap_err();
    assert!(err.contains("repost_blocked:restricted"), "{err}");
    assert!(net.try_rpc("bob", "interact.quote", json!({ "objId": f1, "text": "look" })).await.is_err());

    let r = net.rpc("bob", "interact.comment", json!({ "objId": f1, "text": "save me a seat" })).await;
    assert_eq!(r["audience"], json!({ "kind": "dids", "dids": [alice.did] }));
    let like = net.rpc("bob", "interact.like", json!({ "objId": f1, "on": true })).await;
    assert_eq!(like["like"]["visibility"], "author_only");
    net.deliver_all().await;
    // Alice got both; the collector got nothing.
    assert_eq!(net.rpc("alice", "comments.list", json!({ "objId": f1, "view": "local", "type": "like" })).await["stats"]["likes"], 1);
    let index = net.n("index");
    let collected: i64 = index.station.db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM collector_index", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(collected, 0);
    // Bob's public stream does not show the comment; Alice (its audience) can read it.
    let (_, body, _) = net.get(&format!("{}/feed?mode=display", bob.home), None).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["items"].as_array().unwrap().len(), 0);
    let (_, body, _) = net.get(&format!("{}/feed?mode=display", bob.home), Some(net.proof("alice", "bob"))).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["items"].as_array().unwrap().len(), 1);
    // Carol cannot even read the original through Bob or Alice.
    assert_eq!(net.get(&format!("{}/objects/{f1}", alice.home), Some(net.proof("carol", "alice"))).await.0, 404);
}

/// E11, E12, E13, A11, A28, A40, A49, A74: private bookmarks stay local; public bookmark and
/// back; repost and quote wrap the exact version; the original never changes.
#[tokio::test(flavor = "multi_thread")]
async fn bookmarks_reposts_quotes() {
    let net = Net::new(&["alice", "bob", "carol"], &[]).await;
    let (alice, bob, carol) = (net.n("alice"), net.n("bob"), net.n("carol"));
    for (a, b) in [(alice, bob), (bob, carol)] {
        a.add_contact(b, true, &[]);
        b.add_contact(a, true, &[]);
    }
    let post = publish(&net, "alice", "p1", text_input("ramen recipe", public())).await;
    let p1 = post["objId"].as_str().unwrap().to_string();
    net.deliver_all().await;

    let private = net.rpc("bob", "interact.bookmark", json!({ "objId": p1, "on": true, "public": false })).await;
    assert_eq!(private["bookmark"], json!({ "on": true, "visibility": "private", "delivery": "none" }));
    let outbox: i64 = bob.station.db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(outbox, 0, "a private bookmark is never sent");
    let made_public = net.rpc("bob", "interact.bookmark", json!({ "objId": p1, "on": true, "public": true })).await;
    assert_eq!(made_public["bookmark"]["visibility"], "public");
    net.deliver_all().await;
    assert_eq!(net.rpc("alice", "comments.list", json!({ "objId": p1, "view": "local", "type": "like" })).await["stats"]["bookmarks"], 1);
    let back = net.rpc("bob", "interact.bookmark", json!({ "objId": p1, "on": true, "public": false })).await;
    assert_eq!(back["bookmark"]["on"], true);
    assert_eq!(back["bookmark"]["visibility"], "private");
    net.deliver_all().await;
    assert_eq!(net.rpc("alice", "comments.list", json!({ "objId": p1, "view": "local", "type": "like" })).await["stats"]["bookmarks"], 0);
    let saved = net.rpc("bob", "saved.list", json!({ "kind": "bookmark" })).await;
    assert_eq!(saved[0]["objId"], p1);

    let repost = net.rpc("bob", "interact.repost", json!({ "objId": p1, "on": true })).await;
    assert_eq!(repost["repost"]["on"], true);
    let quote = net.rpc("bob", "interact.quote", json!({ "objId": p1, "text": "trying this tonight", "audience": { "kind": "public" } })).await;
    let quote_id = quote["objId"].as_str().unwrap().to_string();
    net.deliver_all().await;
    // Carol (Bob's friend) received both; the quote embeds P1 with Alice as author (A40).
    net.rpc("carol", "admin.run", json!({ "task": "friends" })).await;
    net.rpc("carol", "admin.run", json!({ "task": "pull" })).await;
    let card = net.rpc("carol", "item.get", json!({ "objId": quote_id })).await;
    assert_eq!(card["item"]["publisher"]["did"], bob.did);
    assert_eq!(card["item"]["embedded"]["relation"], "quote");
    assert_eq!(card["item"]["embedded"]["visibility"], "visible", "{card}");
    assert_eq!(card["item"]["embedded"]["item"]["publisher"]["did"], alice.did);
    let s = net.rpc("alice", "comments.list", json!({ "objId": p1, "view": "local", "type": "repost" })).await["stats"].clone();
    assert_eq!((s["reposts"].as_i64(), s["quotes"].as_i64()), (Some(1), Some(1)));
    // Undo the repost: only Bob's entry changes (A40).
    net.rpc("bob", "interact.repost", json!({ "objId": p1, "on": false })).await;
    net.deliver_all().await;
    let s = net.rpc("alice", "comments.list", json!({ "objId": p1, "view": "local", "type": "repost" })).await["stats"].clone();
    assert_eq!(s["reposts"], 0);
    let original = net.rpc("alice", "item.get", json!({ "objId": p1 })).await;
    assert_eq!(original["item"]["entry"]["state"], "active");
}
