//! Restricted sessions (hybrid SSO contract §3.2–3.3): a Limited account and an External DID
//! read portals and protocol paths as their own DID; publishing and managing a home stay closed.

use homestation::standalone::{build, StandaloneConfig};
use serde_json::{json, Value};

const VISITOR: &str = "did:test:visitor";

struct Zone {
    base: String,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl Zone {
    async fn start() -> Zone {
        let dir = tempfile::tempdir().unwrap();
        let (pem, _) = homestation::sign::generate_key();
        let cfg: StandaloneConfig = serde_json::from_value(json!({
            "owner": "did:test:alice", "owner_name": "alice", "user": "alice", "zone": "alice.test",
            "private_key_pem": pem, "workers": false, "spider": false,
            "users": [{ "user": "carol", "did": "did:test:carol", "name": "carol", "zone_feed_writer": false }],
            "tokens": {
                "tok-alice": { "principal": "alice", "did": "did:test:alice", "app_id": "control-panel" },
                "tok-carol": { "principal": "carol", "did": "did:test:carol", "app_id": "control-panel", "restricted": true },
                "tok-visitor": { "principal": VISITOR, "did": VISITOR, "app_id": "control-panel", "restricted": true }
            }
        }))
        .unwrap();
        let hs = build(cfg, &dir.path().join("hs"), None, None).unwrap();
        for (key, audience) in [("p-public", json!({ "kind": "public" })), ("p-visitor", json!({ "kind": "dids", "dids": [VISITOR] }))] {
            let input = serde_json::from_value(json!({ "text": key, "attachments": [], "link": null, "audience": audience })).unwrap();
            assert_eq!(hs.station.publish(key, input).await.unwrap().stage, "published");
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(homestation::http::serve_listener(hs.state.clone(), listener));
        Zone { base, http: reqwest::Client::new(), _dir: dir }
    }

    /// The whole kRPC response (`result`, or `error` with `code` / `data` for a denial).
    async fn rpc(&self, token: Option<&str>, method: &str, params: Value) -> Value {
        let sys = match token {
            Some(token) => json!([1, token]),
            None => json!([1]),
        };
        let body = json!({ "method": method, "params": params, "sys": sys });
        self.http.post(format!("{}/kapi/homestation", self.base)).json(&body).send().await.unwrap().json().await.unwrap()
    }

    async fn feed(&self, token: Option<&str>) -> (u16, Value) {
        let mut req = self.http.get(format!("{}/home/alice/feed", self.base));
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let response = req.send().await.unwrap();
        (response.status().as_u16(), response.json().await.unwrap_or(Value::Null))
    }
}

fn assert_kind_not_allowed(response: &Value, method: &str) {
    assert_eq!(response["code"], "access_denied", "{method}: {response}");
    assert_eq!(response["data"]["reason"], "session_kind_not_allowed", "{method}: {response}");
    assert_eq!(response["data"]["adjust"]["party"], "product", "{method}: {response}");
    assert_eq!(response["data"]["method"], method, "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_external_session_reads_as_its_own_did() {
    let zone = Zone::start().await;
    let home = zone.rpc(Some("tok-visitor"), "portal.home", json!({})).await;
    assert_eq!(home["result"]["viewer"], json!({ "did": VISITOR }), "{home}");

    let listed = |response: Value| response["result"]["entries"].as_array().map(Vec::len).unwrap_or_default();
    assert_eq!(listed(zone.rpc(Some("tok-visitor"), "portal.list", json!({ "feed": "alice" })).await), 2);
    assert_eq!(listed(zone.rpc(None, "portal.list", json!({ "feed": "alice" })).await), 1);

    let (status, page) = zone.feed(Some("tok-visitor")).await;
    assert_eq!((status, page["items"].as_array().map(Vec::len)), (200, Some(2)), "{page}");
    let (status, page) = zone.feed(None).await;
    assert_eq!((status, page["items"].as_array().map(Vec::len)), (200, Some(1)), "{page}");

    // No account here: no home of its own, and owner methods stay closed.
    assert_kind_not_allowed(&zone.rpc(Some("tok-visitor"), "ui.bootstrap", json!({})).await, "ui.bootstrap");
    let comments = zone.rpc(Some("tok-visitor"), "comments.list", json!({ "objId": "x" })).await;
    assert!(comments.get("error").is_some() && comments.get("code").is_none(), "{comments}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_session_cannot_publish_or_manage_a_home() {
    let zone = Zone::start().await;
    for (method, params) in [
        ("ui.bootstrap", json!({})),
        ("prefs.get", json!({})),
        ("publish.create", json!({ "key": "c1", "input": { "text": "hi", "attachments": [], "link": null, "audience": { "kind": "public" } } })),
        ("profile.set", json!({ "profile": { "name": "x" } })),
        ("zone.set_listing", json!({ "entry": "x", "listed": true })),
        ("sources.list", json!({})),
    ] {
        assert_kind_not_allowed(&zone.rpc(Some("tok-carol"), method, params).await, method);
    }
    // Reading and commenting stay open: a missing item is an ordinary error, not a denial.
    let comments = zone.rpc(Some("tok-carol"), "comments.list", json!({ "objId": "x" })).await;
    assert!(comments.get("code").is_none(), "{comments}");
    assert!(zone.rpc(Some("tok-carol"), "portal.home", json!({})).await.get("error").is_none());

    assert_kind_not_allowed(&zone.rpc(Some("tok-carol"), "object.register", json!({ "objId": "cyfile:missing" })).await, "object.register");
    assert_eq!(zone.http.put(format!("{}/kapi/homestation/upload", zone.base)).body("hello").send().await.unwrap().status().as_u16(), 404);
    assert!(zone.rpc(Some("tok-alice"), "ui.bootstrap", json!({})).await.get("error").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_session_falls_back_to_anonymous_only_on_portals() {
    let zone = Zone::start().await;
    let home = zone.rpc(Some("tok-stale"), "portal.home", json!({})).await;
    assert_eq!(home["result"]["viewer"], Value::Null, "{home}");
    let owner = zone.rpc(Some("tok-stale"), "ui.bootstrap", json!({})).await;
    assert!(owner.get("error").is_some() && owner.get("result").is_none() && owner.get("code").is_none(), "{owner}");
    assert_eq!(zone.feed(Some("tok-stale")).await.0, 401);
}
