//! Zone Agents as publishers (agent-homestation-auto-share TODO §4): an Agent has a home
//! under its short name, follows its owner and the zone's other Agents by default (basis
//! `zone`), its public posts reach them through the zone's own listener, and an Agent that
//! leaves the zone is unfollowed again.

mod common;

use common::*;
use serde_json::{json, Value};

fn source_of<'a>(sources: &'a Value, did: &str) -> Option<&'a Value> {
    sources["sources"].as_array().unwrap().iter().find(|s| s["did"] == did)
}

fn bases(source: &Value) -> Vec<&str> {
    source["basis"].as_array().unwrap().iter().map(|b| b.as_str().unwrap()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_have_homes_and_follow_their_zone() {
    let net = Net::new_zones_with_agents(&["lin"], &[], &[], &[("lin", "kai")], &[], &[("lin", "jarvis"), ("lin", "helper")]).await;
    let (lin, kai, jarvis, helper) = (net.n("lin"), net.n("kai"), net.n("jarvis"), net.n("helper"));

    // A home of its own, marked as an Agent's.
    let (status, body, _) = net.get(&format!("{}/profile", jarvis.home), None).await;
    assert_eq!(status, 200);
    let profile: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((profile["user"].as_str(), profile["kind"].as_str(), profile["owner"].as_str()), (Some("jarvis"), Some("agent"), Some(lin.did.as_str())));
    let (_, body, _) = net.get(&format!("{}/home/?did={}", lin.base, jarvis.did), None).await;
    let located: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((located["user"].as_str(), located["kind"].as_str()), (Some("jarvis"), Some("agent")));
    let (_, body, _) = net.get(&format!("{}/profile", lin.home), None).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["kind"], "person");
    // The Agent's runtime App speaks for the Agent, not for its owner's account.
    assert_eq!(net.rpc("jarvis", "ui.bootstrap", json!({})).await["home"]["user"], "jarvis");

    // Zone relations: owner ↔ its Agents, Agent ↔ Agent; another user of the zone is not involved.
    for n in ["lin", "kai", "jarvis", "helper"] {
        net.rpc(n, "admin.run", json!({ "task": "friends" })).await;
    }
    net.deliver_all().await;
    let lin_sources = net.rpc("lin", "sources.list", json!({})).await;
    assert_eq!(bases(source_of(&lin_sources, &jarvis.did).unwrap()), vec!["zone"]);
    assert_eq!(bases(source_of(&lin_sources, &helper.did).unwrap()), vec!["zone"]);
    assert!(source_of(&lin_sources, &kai.did).is_none());
    let jarvis_sources = net.rpc("jarvis", "sources.list", json!({})).await;
    assert_eq!(bases(source_of(&jarvis_sources, &lin.did).unwrap()), vec!["zone"]);
    assert_eq!(bases(source_of(&jarvis_sources, &helper.did).unwrap()), vec!["zone"]);
    assert!(source_of(&jarvis_sources, &kai.did).is_none());
    assert!(net.rpc("kai", "sources.list", json!({})).await["sources"].as_array().unwrap().is_empty());
    let mut followers = jarvis.station.active_followers().await.unwrap();
    followers.sort();
    assert_eq!(followers, vec![helper.did.clone(), lin.did.clone()]);
    let mut followers = lin.station.active_followers().await.unwrap();
    followers.sort();
    assert_eq!(followers, vec![helper.did.clone(), jarvis.did.clone()]);
    // Derived follows cannot be cancelled by hand.
    let source_id = source_of(&lin_sources, &jarvis.did).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(net.rpc("lin", "sources.unfollow", json!({ "sourceId": source_id })).await, "zone_basis");
    assert_eq!(bases(source_of(&net.rpc("lin", "sources.list", json!({})).await, &jarvis.did).unwrap()), vec!["zone"]);

    // The Agent's public post reaches its owner and the other Agent.
    let note = publish(&net, "jarvis", "j1", text_input("how I finished the backup task", public())).await;
    net.deliver_all().await;
    for reader in ["lin", "helper"] {
        let card = net.rpc(reader, "item.get", json!({ "objId": note["objId"] })).await;
        assert_eq!(card["item"]["publisher"]["did"], jarvis.did, "{reader}: {card}");
    }
    let candidates = net.rpc("lin", "candidates.list", json!({})).await;
    assert!(candidates["entries"].as_array().unwrap().iter().any(|e| e["objId"] == note["objId"]), "{candidates}");

    // The Agent is removed from the zone: its follows are withdrawn and its sources dropped.
    lin.users.0.lock().unwrap().retain(|u| u.did != helper.did);
    assert!(lin.state.host.refresh_users().await.unwrap());
    net.deliver_all().await;
    assert!(source_of(&net.rpc("lin", "sources.list", json!({})).await, &helper.did).is_none());
    assert!(source_of(&net.rpc("jarvis", "sources.list", json!({})).await, &helper.did).is_none());
    for follower in ["lin", "jarvis"] {
        let key = homestation::protocol::follow_key(&net.n(follower).did, &helper.did);
        let entries = net.rpc(follower, "published.entries", json!({})).await;
        let follow = entries.as_array().unwrap().iter().find(|e| e["entry"].as_str().unwrap().ends_with(&format!("/follows/@/{key}"))).unwrap();
        assert_eq!(follow["heads"].as_array().unwrap().last().unwrap()["state"], "withdrawn", "{follower}: {follow}");
    }
    // Its home is no longer served; the database stays on disk.
    assert_eq!(net.get(&format!("{}/profile", helper.home), None).await.0, 404);
    assert!(!lin.state.host.refresh_users().await.unwrap(), "a second look at the same users changes nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_added_later_is_served_without_a_restart() {
    let net = Net::new_zones_with_agents(&["lin"], &[], &[], &[], &[], &[("lin", "jarvis"), ("lin", "helper")]).await;
    let (lin, helper) = (net.n("lin"), net.n("helper"));
    net.rpc("lin", "admin.run", json!({ "task": "friends" })).await;
    let gone = {
        let mut users = lin.users.0.lock().unwrap();
        let at = users.iter().position(|u| u.did == helper.did).unwrap();
        users.remove(at)
    };
    assert!(lin.state.host.refresh_users().await.unwrap());
    assert!(source_of(&net.rpc("lin", "sources.list", json!({})).await, &helper.did).is_none());
    assert_eq!(net.get(&format!("{}/profile", helper.home), None).await.0, 404);

    lin.users.0.lock().unwrap().push(gone);
    assert!(lin.state.host.refresh_users().await.unwrap());
    net.deliver_all().await;
    assert_eq!(bases(source_of(&net.rpc("lin", "sources.list", json!({})).await, &helper.did).unwrap()), vec!["zone"]);
    assert_eq!(helper.station.active_followers().await.unwrap().len(), 2, "owner and the other Agent follow again");
    let (status, body, _) = net.get(&format!("{}/profile", helper.home), None).await;
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["kind"], "agent");
}
