//! A small network of real HomeStations: each has its own key, database, chunk folder and
//! HTTP listener; they reach each other only through the protocol paths.

#![allow(dead_code)]

use homestation::contacts::ContactInfo;
use homestation::directory::{StaticDirectory, StaticIdentity};
use homestation::http::serve_listener;
use homestation::settings::{CollectorRef, UserSettings};
use homestation::standalone::{build, StandaloneConfig};
use homestation::Station;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

pub struct Node {
    pub port: u16,
    pub state: homestation::http::AppState,
    handle: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub name: String,
    pub did: String,
    pub zone: String,
    pub base: String,
    /// `<base>/home/<user>`: this node's user's protocol paths.
    pub home: String,
    pub token: String,
    /// The zone key (the zone owner's key in these tests) and its DID: it signs for every user.
    pub pem: String,
    pub signer: String,
    pub station: Arc<Station>,
    pub contacts: Arc<homestation::contacts::StaticContacts>,
    _dir: Option<TempDir>,
}

impl Node {
    /// Take the node offline (its listener closes) and bring it back on the same port.
    pub fn stop(&self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            h.abort();
        }
    }

    pub async fn start(&self) {
        let listener = loop {
            match tokio::net::TcpListener::bind(("127.0.0.1", self.port)).await {
                Ok(l) => break l,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        };
        let state = self.state.clone();
        *self.handle.lock().unwrap() = Some(tokio::spawn(async move {
            let _ = serve_listener(state, listener).await;
        }));
    }

    pub fn remove_contact(&self, other: &Node) {
        self.contacts.0.lock().unwrap().retain(|c| c.did != other.did);
    }

    pub fn add_contact(&self, other: &Node, friend: bool, groups: &[&str]) {
        let mut list = self.contacts.0.lock().unwrap();
        list.retain(|c| c.did != other.did);
        list.push(ContactInfo { did: other.did.clone(), name: other.name.clone(), friend, blocked: false, groups: groups.iter().map(|g| g.to_string()).collect() });
    }

    pub fn block(&self, other: &Node) {
        let mut list = self.contacts.0.lock().unwrap();
        list.retain(|c| c.did != other.did);
        list.push(ContactInfo { did: other.did.clone(), name: other.name.clone(), friend: false, blocked: true, groups: vec![] });
    }
}

pub struct Net {
    pub nodes: Vec<Node>,
    pub http: reqwest::Client,
}

pub fn did_of(name: &str) -> String {
    format!("did:test:{name}")
}

impl Net {
    /// `collectors`: names that run as collectors; every other node uses them as presets.
    pub async fn new(names: &[&str], collectors: &[&str]) -> Net {
        Self::new_with(names, collectors, &[]).await
    }

    pub async fn new_with(names: &[&str], collectors: &[&str], spider: &[&str]) -> Net {
        Self::new_zones(names, collectors, spider, &[], &[]).await
    }

    /// `extra`: (zone owner, user) — more users served by that owner's zone, signed by its key.
    /// `zone_options`: (zone owner, JSON merged into that zone's standalone config).
    pub async fn new_zones(names: &[&str], collectors: &[&str], spider: &[&str], extra: &[(&str, &str)], zone_options: &[(&str, Value)]) -> Net {
        let mut listeners = Vec::new();
        let mut keys = Vec::new();
        let mut directory = StaticDirectory::default();
        for name in names {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (pem, x) = homestation::sign::generate_key();
            let zone = format!("{name}.test");
            directory.identities.insert(did_of(name), StaticIdentity { zone: Some(zone.clone()), user: Some(name.to_string()), public_key_x: Some(x), devices: vec![], owner: None });
            // An identity of the same zone without a HomeStation (an agent, say).
            directory.identities.insert(format!("did:test:{name}-kid"), StaticIdentity { zone: Some(zone.clone()), user: None, public_key_x: None, devices: vec![], owner: None });
            for (owner, user) in extra.iter().filter(|(o, _)| o == name) {
                let _ = owner;
                directory.identities.insert(did_of(user), StaticIdentity { zone: Some(zone.clone()), user: Some(user.to_string()), public_key_x: None, devices: vec![did_of(name)], owner: None });
            }
            directory.zones.insert(zone, format!("http://127.0.0.1:{port}"));
            listeners.push((listener, port));
            keys.push(pem);
        }
        let collector_refs: Vec<CollectorRef> = collectors.iter().map(|c| CollectorRef { did: did_of(c), name: format!("{c} index") }).collect();
        let mut nodes = Vec::new();
        for ((name, (listener, port)), pem) in names.iter().zip(listeners).zip(keys) {
            let dir = tempfile::tempdir().unwrap();
            let token = format!("tok-{name}");
            let mut settings = UserSettings::default();
            settings.profile.name = name.to_string();
            if !collectors.contains(name) {
                settings.collectors = collector_refs.clone();
            }
            let users: Vec<&str> = extra.iter().filter(|(o, _)| o == name).map(|(_, u)| *u).collect();
            let mut raw = json!({
                "owner": did_of(name),
                "owner_name": name,
                "user": name,
                "zone": format!("{name}.test"),
                "private_key_pem": pem,
                "collector": collectors.contains(name),
                "spider": spider.contains(name),
                "workers": false,
                "tokens": {
                    token.clone(): { "principal": name, "did": did_of(name), "app_id": "control-panel" },
                    format!("{token}-app"): { "principal": name, "did": did_of(name), "app_id": "other-app" },
                    format!("{token}-guest"): { "principal": "guest", "did": format!("did:test:guest-{name}"), "app_id": "control-panel" }
                },
                "settings": settings,
                "users": users.iter().map(|u| {
                    let mut s = UserSettings::default();
                    s.profile.name = u.to_string();
                    s.collectors = collector_refs.clone();
                    json!({ "user": u, "did": did_of(u), "name": u, "settings": s })
                }).collect::<Vec<_>>(),
            });
            for u in &users {
                raw["tokens"][format!("tok-{u}")] = json!({ "principal": u, "did": did_of(u), "app_id": "control-panel" });
            }
            for (_, options) in zone_options.iter().filter(|(o, _)| o == name) {
                for (k, v) in options.as_object().unwrap() {
                    raw[k] = v.clone();
                }
            }
            let mut cfg: StandaloneConfig = serde_json::from_value(raw).unwrap();
            cfg.directory = directory.clone();
            let standalone = build(cfg, dir.path(), None, None).unwrap();
            let state = standalone.state.clone();
            let serve_state = state.clone();
            let handle = tokio::spawn(async move {
                let _ = serve_listener(serve_state, listener).await;
            });
            for u in &users {
                nodes.push(Node {
                    port,
                    state: state.clone(),
                    handle: std::sync::Mutex::new(None),
                    name: u.to_string(),
                    did: did_of(u),
                    zone: format!("{name}.test"),
                    base: format!("http://127.0.0.1:{port}"),
                    home: format!("http://127.0.0.1:{port}/home/{u}"),
                    token: format!("tok-{u}"),
                    pem: pem.clone(),
                    signer: did_of(name),
                    station: standalone.host.station(u).await.unwrap().unwrap(),
                    contacts: standalone.user_contacts[*u].clone(),
                    _dir: None,
                });
            }
            nodes.push(Node {
                port,
                state,
                handle: std::sync::Mutex::new(Some(handle)),
                name: name.to_string(),
                did: did_of(name),
                zone: format!("{name}.test"),
                base: format!("http://127.0.0.1:{port}"),
                home: format!("http://127.0.0.1:{port}/home/{name}"),
                token,
                pem,
                signer: did_of(name),
                station: standalone.station,
                contacts: standalone.contacts,
                _dir: Some(dir),
            });
        }
        Net { nodes, http: reqwest::Client::new() }
    }

    pub fn n(&self, name: &str) -> &Node {
        self.nodes.iter().find(|n| n.name == name).unwrap()
    }

    /// Owner kRPC over HTTP; returns `result` or panics with the error.
    pub async fn rpc(&self, name: &str, method: &str, params: Value) -> Value {
        match self.try_rpc(name, method, params).await {
            Ok(v) => v,
            Err(e) => panic!("{name} {method}: {e}"),
        }
    }

    pub async fn try_rpc(&self, name: &str, method: &str, params: Value) -> Result<Value, String> {
        self.try_rpc_token(name, &self.n(name).token.clone(), method, params).await
    }

    pub async fn try_rpc_token(&self, name: &str, token: &str, method: &str, params: Value) -> Result<Value, String> {
        let node = self.n(name);
        let body = json!({ "method": method, "params": params, "sys": [1, token] });
        let response: Value = self.http.post(format!("{}/kapi/homestation", node.base)).json(&body).send().await.unwrap().json().await.unwrap();
        match response.get("error") {
            Some(e) => Err(e.as_str().unwrap_or_default().to_string()),
            None => Ok(response["result"].clone()),
        }
    }

    /// Run every node's outbox until nothing is due (Push is only for speed; tests settle it).
    pub async fn deliver_all(&self) {
        for _ in 0..10 {
            let mut total = 0;
            for node in &self.nodes {
                total += node.station.process_due_deliveries().await.unwrap();
            }
            if total == 0 {
                return;
            }
        }
    }

    pub async fn get(&self, url: &str, auth: Option<String>) -> (u16, String, reqwest::header::HeaderMap) {
        let mut req = self.http.get(url);
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        let response = req.send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        (status, response.text().await.unwrap(), headers)
    }

    /// A reader proof as `reader` for `target`'s zone.
    pub fn proof(&self, reader: &str, target: &str) -> String {
        let node = self.n(reader);
        let signer = homestation::sign::Signer::from_pem(node.pem.as_bytes(), format!("{}#main_key", node.signer)).unwrap();
        let proof = homestation::auth::make_reader_proof(&signer, &did_of(reader), &self.n(target).zone).unwrap();
        format!("DID {proof}")
    }
}

pub fn text_input(text: &str, audience: Value) -> Value {
    json!({ "text": text, "attachments": [], "link": null, "audience": audience })
}

pub fn public() -> Value {
    json!({ "kind": "public" })
}

pub async fn publish(net: &Net, name: &str, key: &str, input: Value) -> Value {
    let task = net.rpc(name, "publish.create", json!({ "key": key, "input": input })).await;
    assert_eq!(task["stage"], "published", "{task}");
    task
}

pub fn map_of(value: &Value) -> HashMap<String, Value> {
    serde_json::from_value(value.clone()).unwrap_or_default()
}
