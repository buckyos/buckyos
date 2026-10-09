//! Multi-node scenarios over real HTTP (architecture §20 acceptance list, A-numbers below).

mod common;

use common::*;
use homestation::protocol::*;
use serde_json::{json, Value};

fn ids(page: &Value) -> Vec<String> {
    page["items"].as_array().unwrap().iter().map(|i| i["current"].as_str().unwrap_or_default().to_string()).collect()
}

/// A66, A69, §4.4: locate by DID, display read per reader identity, reader proofs.
#[tokio::test(flavor = "multi_thread")]
async fn stream_audience_per_reader() {
    let net = Net::new(&["alice", "bob", "carol", "dave"], &[]).await;
    let (alice, bob, dave) = (net.n("alice"), net.n("bob"), net.n("dave"));
    alice.add_contact(dave, true, &["hiking"]);
    let public_post = publish(&net, "alice", "k1", text_input("public garden", public())).await;
    let friends_post = publish(&net, "alice", "k2", text_input("friends only", json!({ "kind": "friends" }))).await;
    let group_post = publish(&net, "alice", "k3", text_input("hiking group", json!({ "kind": "group", "groupId": "hiking" }))).await;
    let followers_post = publish(&net, "alice", "k4", text_input("for followers", json!({ "kind": "followers" }))).await;

    // Bob follows Alice by DID only: the declaration reaches Alice's inbox (E17).
    let resolution = net.rpc("bob", "sources.resolve", json!({ "kind": "follow", "text": alice.did })).await;
    assert_eq!(resolution["candidates"][0]["did"], alice.did);
    net.rpc("bob", "sources.follow", json!({ "resolution": resolution })).await;
    net.deliver_all().await;
    let followers = alice.station.active_followers().await.unwrap();
    assert_eq!(followers, vec![bob.did.clone()]);
    let sources = net.rpc("bob", "sources.list", json!({})).await;
    assert_eq!(sources["sources"][0]["notify"], "acknowledged");

    let feed = format!("{}/home/feed?mode=display", alice.base);
    let (status, body, _) = net.get(&feed, None).await;
    assert_eq!(status, 200);
    let anonymous: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(ids(&anonymous), vec![public_post["objId"].as_str().unwrap()]);

    let (_, body, _) = net.get(&feed, Some(net.proof("bob", "alice"))).await;
    let as_bob: Value = serde_json::from_str(&body).unwrap();
    let bob_ids = ids(&as_bob);
    assert!(bob_ids.contains(&followers_post["objId"].as_str().unwrap().to_string()));
    assert!(!bob_ids.contains(&friends_post["objId"].as_str().unwrap().to_string()));

    let (_, body, _) = net.get(&feed, Some(net.proof("dave", "alice"))).await;
    let as_dave: Value = serde_json::from_str(&body).unwrap();
    let dave_ids = ids(&as_dave);
    for post in [&public_post, &friends_post, &group_post, &followers_post] {
        assert!(dave_ids.contains(&post["objId"].as_str().unwrap().to_string()), "friend in group sees {post}");
    }
    // Items are signed Heads; restricted entries say so without naming their audience.
    let item = as_dave["items"].as_array().unwrap().iter().find(|i| i["current"] == friends_post["objId"]).unwrap();
    assert_eq!(item["restricted"], true);
    assert_eq!(item["tier"], "friends");

    // Knowing an ObjId grants nothing (A32): Carol cannot read the friends-only object or its Head.
    let object = format!("{}/home/objects/{}", alice.base, friends_post["objId"].as_str().unwrap());
    assert_eq!(net.get(&object, Some(net.proof("carol", "alice"))).await.0, 404);
    assert_eq!(net.get(&object, Some(net.proof("dave", "alice"))).await.0, 200);
    let key = friends_post["entry"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
    assert_eq!(net.get(&format!("{}/home/feed/@/{key}", alice.base), None).await.0, 404);
    assert_eq!(net.get(&format!("{}/home/feed/@/{key}", alice.base), Some(net.proof("dave", "alice"))).await.0, 200);
    // A proof for another zone or a forged one is refused.
    assert_eq!(net.get(&feed, Some(net.proof("bob", "carol"))).await.0, 401);
}

/// §7.3, §7.4, A01, A04, A05: admission at the inbox, disclosure only on `accepted`,
/// idempotent re-delivery, Push and Pull of the same object merge.
#[tokio::test(flavor = "multi_thread")]
async fn push_admission_and_idempotency() {
    let net = Net::new(&["alice", "bob", "eve"], &[]).await;
    let (alice, bob, eve) = (net.n("alice"), net.n("bob"), net.n("eve"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    let post = publish(&net, "alice", "p1", text_input("hello friends", public())).await;
    let obj_id = post["objId"].as_str().unwrap().to_string();
    net.deliver_all().await;
    let delivered: Value = net.rpc("alice", "publish.task", json!({ "key": "p1" })).await;
    assert_eq!(delivered["delivery"]["state"], "delivered", "{delivered}");
    let admission: (String, Option<String>) = alice
        .station
        .db
        .call({
            let id = obj_id.clone();
            move |c| Ok(c.query_row("SELECT state, admission FROM outbox WHERE obj_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?)
        })
        .await
        .unwrap();
    assert_eq!(admission, ("accepted".to_string(), Some("preferred".to_string())));

    // A stranger's post to Bob's inbox is not admitted.
    publish(&net, "eve", "e1", text_input("buy now", public())).await;
    let eve_post = eve.station.db.call(|c| Ok(c.query_row("SELECT obj_id FROM published p JOIN heads h ON h.entry=p.entry LIMIT 1", [], |r| r.get::<_, String>(0)).ok())).await.unwrap();
    let _ = eve_post;
    let eve_obj: String = eve.station.db.call(|c| Ok(c.query_row("SELECT current FROM heads LIMIT 1", [], |r| r.get(0))?)).await.unwrap();
    let outcome = eve.station.dispatch_to(&eve_obj, &bob.did, None).await.unwrap().1;
    assert_eq!(outcome, homestation::delivery::DispatchOutcome::Rejected { reason: "not-admitted".into(), retryable: false });

    // Re-delivering the same object is accepted again without a second candidate.
    let again = alice.station.dispatch_to(&obj_id, &bob.did, None).await.unwrap().1;
    assert!(matches!(again, homestation::delivery::DispatchOutcome::Accepted { .. }));
    // Bob follows Alice (friend-derived) and pulls the same object: one candidate, two paths.
    net.rpc("bob", "admin.run", json!({ "task": "friends" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    let paths: (i64, String) = bob
        .station
        .db
        .call({
            let id = obj_id.clone();
            move |c| Ok(c.query_row("SELECT COUNT(*), MAX(source_paths) FROM candidates WHERE obj_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?)
        })
        .await
        .unwrap();
    assert_eq!(paths.0, 1);
    let paths: Vec<Value> = serde_json::from_str(&paths.1).unwrap();
    assert!(paths.iter().any(|p| p["transport"] == "push") && paths.iter().any(|p| p["transport"] == "pull"), "{paths:?}");

    // A gateway that forwards with a loopback Host still reaches this zone's inbox.
    let jwt: String = alice.station.db.call({ let id = obj_id.clone(); move |c| Ok(c.query_row("SELECT jwt FROM objects WHERE obj_id=?1", [id], |r| r.get(0))?) }).await.unwrap();
    let forwarded = net
        .http
        .put(format!("{}/home/inbox", bob.base))
        .header("content-type", "application/cyfs-named-object+jwt")
        .header("cyfs-obj-id", ndn_lib::ObjId::new(&obj_id).unwrap().to_base32())
        .body(jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(forwarded.status().as_u16(), 200);
    let other_zone = net
        .http
        .put(format!("{}/home/inbox", bob.base))
        .header("host", "carol.test")
        .header("content-type", "application/cyfs-named-object+jwt")
        .body("x.y.z")
        .send()
        .await
        .unwrap();
    assert_eq!(other_zone.status().as_u16(), 404);

    // The status query reports what was accepted.
    let id = ndn_lib::ObjId::new(&obj_id).unwrap();
    let (status, body, headers) = net.get(&format!("{}/home/inbox?dispatch-status={}", bob.base, id.to_base32()), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(headers.get("cyfs-dispatch-status").unwrap(), "accepted");
}

/// E14–E16, A17, A23, A24, A26, A27, A30, A50: versions by Head only, monotonic seq, stale
/// replays ignored, Head before object, same-seq conflicts surfaced.
#[tokio::test(flavor = "multi_thread")]
async fn heads_versions_withdraw_and_conflicts() {
    let net = Net::new(&["alice", "bob", "carol"], &[]).await;
    let (alice, bob, carol) = (net.n("alice"), net.n("bob"), net.n("carol"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    let post = publish(&net, "alice", "p1", text_input("plan: one big box", public())).await;
    let entry = post["entry"].as_str().unwrap().to_string();
    let p1 = post["objId"].as_str().unwrap().to_string();
    net.deliver_all().await;

    // Bob comments on P1, Alice edits to P2: the comment stays on P1.
    net.rpc("bob", "interact.comment", json!({ "objId": p1, "text": "try leafy greens" })).await;
    net.deliver_all().await;
    let p2 = net.rpc("alice", "entry.edit", json!({ "entry": entry, "text": "plan: two small boxes" })).await;
    let p2 = p2.as_str().unwrap().to_string();
    net.deliver_all().await;
    let card = net.rpc("bob", "item.get", json!({ "objId": p1 })).await;
    assert_eq!(card["item"]["entry"]["isLatest"], false);
    assert_eq!(card["item"]["entry"]["currentObjId"], p2);
    assert_eq!(card["item"]["entry"]["versionCount"], 2);
    let p2_comments = net.rpc("alice", "comments.list", json!({ "objId": p2, "view": "local", "type": "text" })).await;
    assert_eq!(p2_comments["comments"][0]["onOldVersion"], true, "the P1 comment is shown as on the old version");
    assert_eq!(p2_comments["stats"]["textComments"], 0, "old-version comments do not count for P2");
    let p1_stats = net.rpc("alice", "comments.list", json!({ "objId": p1, "view": "local", "type": "text" })).await;
    assert_eq!(p1_stats["stats"]["textComments"], 1);

    // Withdraw (seq 3), then replay the seq-1 Head to Bob: the withdrawal stands (A23).
    let seq1_head: String = alice
        .station
        .db
        .call({
            let e = entry.clone();
            move |c| Ok(c.query_row("SELECT o.jwt FROM head_history h JOIN objects o ON o.obj_id=h.head_obj_id WHERE h.entry=?1 AND h.seq=1", [e], |r| r.get(0))?)
        })
        .await
        .unwrap();
    net.rpc("alice", "entry.withdraw", json!({ "entry": entry })).await;
    net.deliver_all().await;
    let head = bob.station.db.call({ let e = entry.clone(); move |c| homestation::publish::get_head(c, &e) }).await.unwrap().unwrap();
    assert_eq!((head.seq, head.state), (3, HeadState::Withdrawn));
    let id = homestation::protocol::obj_id_of(OBJ_TYPE_HEAD, &homestation::sign::decode_unverified(&seq1_head).unwrap().claims).unwrap().0;
    let replay = net
        .http
        .put(format!("{}/home/inbox", bob.base))
        .header("host", &bob.zone)
        .header("content-type", "application/cyfs-named-object+jwt")
        .header("cyfs-obj-id", ndn_lib::ObjId::new(&id).unwrap().to_base32())
        .body(seq1_head.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status().as_u16(), 200);
    let head = bob.station.db.call({ let e = entry.clone(); move |c| homestation::publish::get_head(c, &e) }).await.unwrap().unwrap();
    assert_eq!(head.seq, 3, "a replayed lower seq never overrides");
    let card = net.rpc("bob", "item.get", json!({ "objId": p1 })).await;
    assert_eq!(card["item"]["entry"]["state"], "withdrawn");
    assert_eq!(card["canRepost"], false);

    // Head before object (A24): Carol learns of the withdrawal first, then gets the object.
    let withdrawn_head: String = alice
        .station
        .db
        .call({
            let e = entry.clone();
            move |c| Ok(c.query_row("SELECT o.jwt FROM heads h JOIN objects o ON o.obj_id=h.head_obj_id WHERE h.entry=?1", [e], |r| r.get(0))?)
        })
        .await
        .unwrap();
    let v = homestation::objects::verify_jwt(carol.station.directory.as_ref(), &withdrawn_head, None).await.unwrap();
    carol.station.ingest_head(v, false).await.unwrap();
    let p1_jwt: String = alice.station.db.call({ let p = p1.clone(); move |c| Ok(c.query_row("SELECT jwt FROM objects WHERE obj_id=?1", [p], |r| r.get(0))?) }).await.unwrap();
    carol.station.ingest_wire(&p1_jwt, homestation::ingress::Arrival::Fetch, false).await.unwrap();
    let card = net.rpc("carol", "item.get", json!({ "objId": p1 })).await;
    assert_eq!(card["item"]["entry"]["state"], "withdrawn");

    // Two different Heads with the same seq, both validly signed by Alice: conflict (A30).
    let signer = homestation::sign::Signer::from_pem(alice.pem.as_bytes(), format!("{}#main_key", alice.did)).unwrap();
    let other_entry = entry_url(&alice.zone, EntryNamespace::Feed, "forked");
    for current in [p1.clone(), p2.clone()] {
        let obj = homestation::protocol::FeedObject {
            entry: Some(other_entry.clone()),
            ..serde_json::from_value(alice.station.db.call({ let p = current.clone(); move |c| Ok(homestation::objects::get_feed(c, &p)?.unwrap()) }).await.unwrap().to_value()).unwrap()
        };
        let _ = obj;
        let head = json!({ "kind": "feed_head", "publisher": alice.did, "entry": other_entry, "seq": 7, "state": "active", "current": current, "updated_at_ms": 1 });
        let jwt = signer.sign(&head).unwrap();
        let v = homestation::objects::verify_jwt(bob.station.directory.as_ref(), &jwt, None).await.unwrap();
        bob.station.ingest_head(v, false).await.unwrap();
    }
    let head = bob.station.db.call({ let e = other_entry.clone(); move |c| homestation::publish::get_head(c, &e) }).await.unwrap().unwrap();
    assert!(head.conflict_obj_id.is_some(), "same-seq contradiction is marked");
}

/// §5.4 rules 2–3, A46, A47, A48: entries outside the publisher's namespace are invalid,
/// URLs are not content references.
#[tokio::test(flavor = "multi_thread")]
async fn entry_namespace_and_content_rules() {
    let net = Net::new(&["alice", "bob", "mallory"], &[]).await;
    let (alice, bob, mallory) = (net.n("alice"), net.n("bob"), net.n("mallory"));
    bob.add_contact(mallory, true, &[]);
    let signer = homestation::sign::Signer::from_pem(mallory.pem.as_bytes(), format!("{}#main_key", mallory.did)).unwrap();
    // Mallory claims an entry in Alice's zone.
    let stolen = entry_url(&alice.zone, EntryNamespace::Feed, "garden-0001");
    let obj = json!({ "kind": "post", "publisher": mallory.did, "iat": 1, "entry": stolen, "content": { "type": "text", "text": "mine now" } });
    let jwt = signer.sign(&obj).unwrap();
    let id = bob.station.ingest_wire(&jwt, homestation::ingress::Arrival::Fetch, false).await.unwrap();
    assert!(!bob.station.db.call({ let i = id.clone(); move |c| homestation::objects::entry_valid(c, &i) }).await.unwrap(), "treated as terminal");
    let head = json!({ "kind": "feed_head", "publisher": mallory.did, "entry": stolen, "seq": 1, "state": "withdrawn", "updated_at_ms": 1 });
    let v = homestation::objects::verify_jwt(bob.station.directory.as_ref(), &signer.sign(&head).unwrap(), None).await.unwrap();
    assert!(bob.station.ingest_head(v, false).await.is_err(), "Head outside the namespace is refused");
    // Mallory cannot sign as Alice.
    let forged = json!({ "kind": "post", "publisher": alice.did, "iat": 1, "content": { "type": "text", "text": "I am Alice" } });
    assert!(homestation::objects::verify_jwt(bob.station.directory.as_ref(), &signer.sign(&forged).unwrap(), None).await.is_err());
    // A remote URL as media is not a valid object (A48).
    let url_media = json!({ "kind": "post", "publisher": mallory.did, "iat": 1, "content": { "type": "image", "media": [{ "object": "https://cdn.example/x.png" }] } });
    assert!(bob.station.ingest_wire(&signer.sign(&url_media).unwrap(), homestation::ingress::Arrival::Fetch, false).await.is_err());
    // Terminal objects (no entry) have no state to query (A47).
    let terminal = json!({ "kind": "post", "publisher": mallory.did, "iat": 2, "content": { "type": "text", "text": "final" } });
    let id = bob.station.ingest_wire(&signer.sign(&terminal).unwrap(), homestation::ingress::Arrival::Fetch, false).await.unwrap();
    let card = net.rpc("bob", "item.get", json!({ "objId": id })).await;
    assert!(card["item"]["entry"].is_null());
}

/// §9.6, A43: previewing another publisher's portal as anonymous reads anonymously; owner-only
/// fields never appear; an idle node does not report changes.
#[tokio::test(flavor = "multi_thread")]
async fn portal_preview_and_idle_versions() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    publish(&net, "alice", "p1", text_input("public garden", public())).await;
    publish(&net, "alice", "f1", text_input("family barbecue", json!({ "kind": "friends" }))).await;
    net.deliver_all().await;

    let as_bob = net.rpc("bob", "published.list", json!({ "owner": alice.did })).await;
    assert_eq!(as_bob["entries"].as_array().unwrap().len(), 2, "Bob is Alice's friend");
    assert!(as_bob["entries"][0]["head"]["seq"].is_u64());
    let anonymous = net.rpc("bob", "published.list", json!({ "owner": alice.did, "reader": { "kind": "anonymous" } })).await;
    let entries = anonymous["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{anonymous}");
    assert!(anonymous["cards"][0].get("personal").is_none());
    assert!(anonymous["cards"][0].get("repostBlockedReason").is_none(), "optional fields are omitted, not null");
    let other = net.rpc("bob", "published.list", json!({ "owner": alice.did, "reader": { "kind": "did", "did": "did:test:carol" } })).await;
    assert_eq!(other["readerApproximated"], true);
    let profile = net.rpc("bob", "profile.get", json!({ "did": alice.did, "reader": { "kind": "anonymous" } })).await;
    assert_eq!(profile["posts"], 1);
    assert!(profile.get("readerApproximated").is_none());
    let profile = net.rpc("bob", "profile.get", json!({ "did": alice.did, "reader": { "kind": "did", "did": "did:test:carol" } })).await;
    assert_eq!((profile["posts"].as_i64(), profile["readerApproximated"].as_bool()), (Some(1), Some(true)));

    // Nothing new: pulls, selection and comment sync leave the change counters alone.
    for _ in 0..2 {
        net.rpc("bob", "admin.run", json!({ "task": "friends" })).await;
        net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
        net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;
        net.rpc("bob", "admin.run", json!({ "task": "comments" })).await;
        net.deliver_all().await;
    }
    let before = net.rpc("bob", "ui.versions", json!({})).await;
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "comments" })).await;
    net.deliver_all().await;
    let after = net.rpc("bob", "ui.versions", json!({})).await;
    assert_eq!(before, after);
    let bootstrap = net.rpc("alice", "ui.bootstrap", json!({})).await;
    assert_eq!(bootstrap["followers"], json!([bob.did]));
}
