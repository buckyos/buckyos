//! The evaluation service as a public capability (§6, A57–A65), proof of consumption
//! (§11, A19–A20) and permission boundaries (A25, A32, A43).

mod common;

use common::*;
use serde_json::{json, Value};

async fn upload_text(net: &Net, name: &str, file: &str, text: &str) -> String {
    let node = net.n(name);
    let v: Value = net
        .http
        .put(format!("{}/kapi/homestation/upload?name={file}&mime=text/markdown", node.base))
        .header("authorization", format!("Bearer {}", node.token))
        .body(text.as_bytes().to_vec())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["objId"].as_str().unwrap().to_string()
}

/// A57–A62, E20–E22: identity and content targets without any Feed entry, paths bound to a
/// version, failures as states, unknown never turned into "human-made".
#[tokio::test(flavor = "multi_thread")]
async fn evaluation_targets_and_binding() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    net.rpc("bob", "prefs.set_topics", json!({ "topics": [{ "id": "t-garden", "name": "garden", "tags": ["garden"], "subscribed": true }] })).await;

    // A file that never became a Feed Object (E21): no evidence of AI use → unknown, not "human".
    let doc = upload_text(&net, "bob", "notes.md", "Watering schedule for the garden, written by hand.").await;
    let r = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "object_id": doc }, "dimensions": ["generation_method", "topic"] } })).await;
    assert_eq!(r["state"], "partial", "{r}");
    assert_eq!(r["unknown_dimensions"], json!(["generation_method"]));
    assert!(r["assertions"].as_array().unwrap().iter().any(|a| a["dimension"] == "topic" && a["tag"] == "garden"));
    assert_eq!(r["evaluator"], bob.did);
    // Reuse within the same configuration; a refresh makes a new record linked to the old one.
    let again = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "object_id": doc }, "dimensions": ["generation_method"] } })).await;
    assert_eq!(again["result_id"], r["result_id"]);
    let fresh = net.rpc("bob", "eval.refresh", json!({ "request": { "target": { "kind": "content", "object_id": doc }, "dimensions": ["generation_method", "topic"] } })).await;
    assert_ne!(fresh["result_id"], r["result_id"]);
    assert_eq!(fresh["supersedes"], r["result_id"]);

    // An identity with observed content (E20).
    for (i, text) in ["my garden today", "garden tools review", "garden harvest"].iter().enumerate() {
        publish(&net, "alice", &format!("g{i}"), text_input(text, public())).await;
    }
    net.deliver_all().await;
    let id = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "identity", "did": alice.did }, "dimensions": ["topic_affinity", "delivery_behavior"] } })).await;
    assert_eq!(id["state"], "complete", "{id}");
    assert!(id["assertions"].as_array().unwrap().iter().any(|a| a["tag"] == "garden" && a["scope"] == "identity"));
    assert!(id["evidence_scope"]["observed_from"].is_u64());
    let stranger = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "identity", "did": "did:test:nobody" } } })).await;
    assert_eq!(stranger["state"], "unknown");

    // An entry path resolves to its current version; after an edit the old result is not the
    // path's result any more (A59); a withdrawn path is a state, not a judgement (A62).
    let post = publish(&net, "alice", "path-1", text_input("garden plan v1", public())).await;
    let path = format!("cyfs://{}/home/feed/@/{}", alice.zone, post["entry"].as_str().unwrap().rsplit('/').next().unwrap());
    let r1 = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "object_path": path }, "dimensions": ["topic"] } })).await;
    assert_eq!(r1["target"]["object_id"], post["objId"]);
    assert_eq!(r1["resolution"]["head_seq"], 1);
    let v2 = net.rpc("alice", "entry.edit", json!({ "entry": post["entry"], "text": "garden plan v2" })).await;
    let r2 = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "object_path": path }, "dimensions": ["topic"] } })).await;
    assert_eq!(r2["target"]["object_id"], v2);
    assert_ne!(r2["result_id"], r1["result_id"]);
    net.rpc("alice", "entry.withdraw", json!({ "entry": post["entry"] })).await;
    let r3 = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "object_path": path }, "dimensions": ["topic"] } })).await;
    assert_eq!((r3["state"].as_str(), r3["failure"].as_str()), (Some("unknown"), Some("withdrawn")));

    // Root ObjId + InnerPath (A60): a member that is an ObjId is resolved, a plain value is
    // bound as root + path.
    let card = net.rpc("bob", "item.get", json!({ "objId": post["objId"] })).await;
    let _ = card;
    let r4 = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "root_object_id": post["objId"], "inner_path": "/content/text" }, "dimensions": ["topic"] } })).await;
    assert_eq!(r4["target"]["root_object_id"], post["objId"]);
    assert_eq!(r4["target"]["inner_path"], "/content/text");
    assert_eq!(r4["coverage"]["scope"], "part");

    // Asynchronous requests are queryable tasks.
    let task = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "object_id": doc }, "async": true } })).await;
    assert_eq!(task["state"], "queued");
    net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;
    let done = net.rpc("bob", "eval.get", json!({ "id": task["task_id"] })).await;
    assert_eq!(done["state"], "done", "{done}");
    // A shared evaluation is a signed named object (§6.5).
    let shared = net.rpc("bob", "eval.share", json!({ "resultId": fresh["result_id"] })).await;
    assert!(shared["objId"].as_str().unwrap().starts_with("cyfeval:"));
}

/// A64, A65: overrides are scoped — an app's correction stays in that app, the user's global
/// correction applies everywhere — and evaluating never changes permissions or objects.
#[tokio::test(flavor = "multi_thread")]
async fn overrides_are_scoped() {
    let net = Net::new(&["bob"], &[]).await;
    let bob = net.n("bob");
    let doc = upload_text(&net, "bob", "x.md", "As an AI language model, here is a garden summary.").await;
    let r = net.rpc("bob", "eval.evaluate", json!({ "request": { "target": { "kind": "content", "object_id": doc }, "dimensions": ["generation_method"] } })).await;
    assert_eq!(r["assertions"][0]["tag"], "ai_full");
    let app_token = format!("{}-app", bob.token);
    let set = net
        .try_rpc_token("bob", &app_token, "eval.set_tag_override", json!({ "target": doc, "dimension": "generation_method", "tag": "ai_full", "action": "remove" }))
        .await
        .unwrap();
    assert_eq!(set["scope"], "app:other-app");
    let in_app = net.try_rpc_token("bob", &app_token, "eval.overrides", json!({ "target": doc })).await.unwrap();
    assert_eq!(in_app.as_array().unwrap().len(), 1);
    let global = net.rpc("bob", "eval.overrides", json!({ "target": doc })).await;
    assert_eq!(global.as_array().unwrap().len(), 0, "the app's correction does not spread");
    let changes = net.rpc("bob", "eval.changes", json!({ "since": 0 })).await;
    assert!(changes["changes"].as_array().unwrap().iter().any(|c| c["kind"] == "result"));
}

/// A19, A20, §11: no proof without joining; a proof carries only the agreed facts and reaches
/// the receiver's inbox.
#[tokio::test(flavor = "multi_thread")]
async fn consumption_proofs() {
    let net = Net::new(&["alice", "bob", "brand"], &[]).await;
    let (alice, bob, brand) = (net.n("alice"), net.n("bob"), net.n("brand"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    let post = publish(&net, "alice", "ad", text_input("watch our video", public())).await;
    net.deliver_all().await;
    let agreement = net.rpc("bob", "consumption.join", json!({ "target": post["objId"], "receiver": brand.did, "action": "video_complete", "joined": false })).await;
    let err = net.try_rpc("bob", "consumption.report", json!({ "agreementId": agreement["agreementId"] })).await.unwrap_err();
    assert!(err.contains("not joined"), "{err}");
    net.rpc("bob", "consumption.join", json!({ "target": post["objId"], "receiver": brand.did, "action": "video_complete", "joined": true })).await;
    let proof = net.rpc("bob", "consumption.report", json!({ "agreementId": agreement["agreementId"], "result": { "completed": true } })).await;
    net.deliver_all().await;
    let proof_id = proof["objId"].as_str().unwrap().to_string();
    let stored: Value = brand
        .station
        .db
        .call(move |c| Ok(serde_json::from_str(&c.query_row("SELECT body FROM objects WHERE obj_id=?1", [proof_id], |r| r.get::<_, String>(0))?)?))
        .await
        .unwrap();
    assert_eq!(stored["action"], "video_complete");
    assert_eq!(stored["publisher"], bob.did);
    assert_eq!(stored.as_object().unwrap().keys().filter(|k| ["events", "dwell", "history"].contains(&k.as_str())).count(), 0);
}

/// A25, A43, §9.6: only controllers change their entries; visitors and other users only get
/// the public face; private state never leaks through the portal.
#[tokio::test(flavor = "multi_thread")]
async fn permission_boundaries() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    let post = publish(&net, "alice", "p1", text_input("hello", public())).await;
    net.deliver_all().await;
    let comment = net.rpc("bob", "interact.comment", json!({ "objId": post["objId"], "text": "hi" })).await;
    net.deliver_all().await;
    // Alice cannot withdraw Bob's comment entry (it is not hers).
    let err = net.try_rpc("alice", "entry.withdraw", json!({ "entry": comment["task"]["entry"] })).await.unwrap_err();
    assert!(err.contains("not_found") || err.contains("forbidden"), "{err}");
    // Another user of Alice's zone gets the public face only; tokens of other zones mean nothing here.
    let guest = format!("{}-guest", alice.token);
    let foreign = net.try_rpc_token("alice", &guest, "reading.list", json!({})).await.unwrap_err();
    assert!(foreign.contains("forbidden"), "{foreign}");
    assert!(net.try_rpc_token("alice", &bob.token, "profile.get", json!({})).await.unwrap_err().contains("unauthorized"));
    let profile = net.try_rpc_token("alice", &guest, "profile.get", json!({})).await.unwrap();
    assert_eq!(profile["did"], alice.did);
    assert_eq!(profile["posts"], 1);
    // Uploads are the owner's (A41 materials are user assets).
    let status = net
        .http
        .put(format!("{}/kapi/homestation/upload?name=x&mime=text/plain", alice.base))
        .header("authorization", format!("Bearer {}-guest", alice.token))
        .body("x")
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(status, 403);
    // A bad token is refused, an anonymous portal read works.
    assert!(net.try_rpc_token("alice", "nope", "profile.get", json!({})).await.unwrap_err().contains("unauthorized"));
    let (status, body, _) = net.get(&format!("{}/home/profile", alice.base), None).await;
    assert_eq!(status, 200);
    let profile: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(profile["stream"], format!("cyfs://{}/home/feed", alice.zone));
    assert_eq!(profile["inbox"], format!("cyfs://{}/home/inbox", alice.zone));
}
