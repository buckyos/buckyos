//! The https paths of a home are its objects too (agent-object-url TODO, 第 3 段遗留 4): a public read of
//! `/home/<user>`, its feed or an entry carries the object's Card in `BuckyOS-DID-Document` (base protocol §6.1),
//! whose default URL is that https path on the Zone's host.

mod common;

use base64::Engine;
use common::*;
use serde_json::Value;

fn card_of(headers: &reqwest::header::HeaderMap) -> Option<Value> {
    let value = headers.get("BuckyOS-DID-Document")?.to_str().ok()?;
    let encoded = value.strip_prefix("json ")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn home_paths_carry_the_card_of_their_object() {
    let net = Net::new(&["lin"], &[]).await;
    let lin = net.n("lin");

    let (status, body, headers) = net.get(&lin.home, None).await;
    assert_eq!(status, 200, "{body}");
    let card = card_of(&headers).expect("the home's Card");
    assert_eq!(card["alsoKnownAs"][0], format!("https://{}/home/lin", lin.zone), "{card}");
    assert!(card["alsoKnownAs"].as_array().unwrap().iter().any(|u| u == "obj://homestation/home/lin"), "{card}");

    let (status, _, headers) = net.get(&format!("{}/feed", lin.home), None).await;
    assert_eq!(status, 200);
    assert_eq!(card_of(&headers).expect("the feed's Card")["alsoKnownAs"][0], format!("https://{}/home/lin/feed", lin.zone));

    // Not an object: no Card.
    let (_, _, headers) = net.get(&format!("{}/profile", lin.home), None).await;
    assert!(card_of(&headers).is_none());
}
