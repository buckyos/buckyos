//! Pictures and names shown for people and for the zone feed: Message Center contacts supply
//! people's pictures (the owner's own contact included); the Zone's display settings name the
//! zone feed unless HomeStation has a name of its own.

mod common;

use common::*;
use homestation::contacts::ContactInfo;
use homestation::host::ZoneBrand;
use serde_json::{json, Value};

const ALICE_PIC: &str = "data:image/png;base64,QUxJQ0U=";
const BOB_PIC: &str = "data:image/png;base64,Qk9C";

fn identity<'a>(boot: &'a Value, did: &str) -> &'a Value {
    boot["identities"].as_array().unwrap().iter().find(|identity| identity["did"] == did).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn identities_and_profiles_carry_contact_pictures() {
    let net = Net::new(&["alice", "bob", "carol"], &[]).await;
    let (alice, bob, carol) = (net.n("alice"), net.n("bob"), net.n("carol"));
    alice.add_contact(bob, true, &[]);
    alice.add_contact(carol, true, &[]);
    bob.add_contact(alice, true, &[]);
    {
        let mut list = alice.contacts.0.lock().unwrap();
        list.iter_mut().find(|contact| contact.did == bob.did).unwrap().avatar = Some(BOB_PIC.into());
        list.push(ContactInfo { did: alice.did.clone(), name: alice.name.clone(), friend: false, blocked: false, tags: vec![], following: false, avatar: Some(ALICE_PIC.into()) });
    }

    let boot = net.rpc("alice", "ui.bootstrap", json!({})).await;
    assert_eq!(identity(&boot, &bob.did)["avatar"], BOB_PIC);
    assert_eq!(identity(&boot, &alice.did)["avatar"], ALICE_PIC);
    assert!(identity(&boot, &carol.did).get("avatar").is_none(), "no picture, no field");

    let own = net.rpc("alice", "profile.get", json!({})).await;
    assert_eq!(own["avatar"], ALICE_PIC);
    // Another station reads the public profile: the picture travels with it.
    let remote = net.rpc("bob", "profile.get", json!({ "did": alice.did })).await;
    assert_eq!(remote["avatar"], ALICE_PIC);
}

#[tokio::test(flavor = "multi_thread")]
async fn zone_feed_is_named_by_homestation_then_the_zone_display_settings() {
    let net = Net::new_zones(&["lin"], &[], &[], &[("lin", "kai")], &[]).await;
    let lin = net.n("lin");
    let host = &lin.state.host;
    let (_, index) = get(&net, &format!("{}/home/", lin.base)).await;
    assert_eq!(index["zoneName"], "lin.test", "the zone host when nothing is set");

    host.set_zone_brand(ZoneBrand { name: Some("我的家庭云".into()), icon: Some(ALICE_PIC.into()) });
    let (_, index) = get(&net, &format!("{}/home/", lin.base)).await;
    assert_eq!(index["zoneName"], "我的家庭云");
    assert_eq!(host.zone_profile().await.unwrap()["avatar"], ALICE_PIC);

    let net = Net::new_zones(&["lin"], &[], &[], &[], &[("lin", json!({ "zone_name": "Lin's zone" }))]).await;
    let host = &net.n("lin").state.host;
    host.set_zone_brand(ZoneBrand { name: Some("我的家庭云".into()), icon: None });
    assert_eq!(host.zone_name(), "Lin's zone", "HomeStation's own name wins");
    assert!(host.zone_profile().await.unwrap().get("avatar").is_none());
}

async fn get(net: &Net, url: &str) -> (u16, Value) {
    let response = net.http.get(url).send().await.unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}
