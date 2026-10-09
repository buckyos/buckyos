//! A seeded local network for UI development and real-backend e2e:
//!
//!   cargo run -p homestation --example devnet -- [--port 4131] [--data-dir /tmp/hs-devnet] [--fresh]
//!
//! Starts the zone of `me` (the Desktop's user, token `tok-me`; a second user `kai` of the same
//! zone, token `tok-kai`) on --port, then the zones of alice, bob, sarah, a collector `index`
//! and a small RSS/web fixture site on the following ports. All of them talk only through the
//! HomeStation protocol paths; me and kai through their own zone's. Point the Desktop dev
//! server at `HS_BACKEND=http://127.0.0.1:<port>` (see
//! src/frame/desktop/src/app/homestation/UI_DATAMODEL.md §11).

use axum::response::IntoResponse;
use homestation::contacts::ContactInfo;
use homestation::directory::{StaticDirectory, StaticIdentity};
use homestation::settings::{CollectorRef, Topic, UserSettings};
use homestation::standalone::{build, Standalone, StandaloneConfig};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

const NAMES: &[(&str, &str)] = &[("me", "Lin"), ("alice", "Alice Chen"), ("bob", "Bob Zhang"), ("sarah", "Sarah Kim"), ("index", "Open Index")];

fn did(name: &str) -> String {
    format!("did:test:{name}")
}

fn svg(hue: u32, label: &str) -> Vec<u8> {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="800" viewBox="0 0 1200 800"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl({hue} 55% 62%)"/><stop offset="1" stop-color="hsl({} 45% 36%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="600" y="720" font-family="system-ui" font-size="54" fill="white" text-anchor="middle">{label}</text></svg>"#,
        (hue + 50) % 360
    )
    .into_bytes()
}

struct Net {
    nodes: Vec<(String, Arc<homestation::Station>)>,
}

/// The second user of me's zone.
const KAI: (&str, &str) = ("kai", "Kai Wen");

impl Net {
    fn st(&self, name: &str) -> &Arc<homestation::Station> {
        &self.nodes.iter().find(|(n, _)| n == name).unwrap().1
    }

    async fn rpc(&self, name: &str, method: &str, params: Value) -> Value {
        let station = self.st(name);
        let caller = homestation::auth::Caller { principal: name.into(), did: Some(did(name)), app_id: Some("control-panel".into()) };
        match station.handle_rpc(&caller, method, params).await {
            Ok(v) => v,
            Err(e) => panic!("{name} {method}: {e}"),
        }
    }

    async fn settle(&self) {
        for _ in 0..6 {
            let mut n = 0;
            for (_, s) in &self.nodes {
                n += s.process_due_deliveries().await.unwrap_or(0);
            }
            if n == 0 {
                break;
            }
        }
    }

    async fn upload(&self, name: &str, file: &str, data: Vec<u8>) -> String {
        self.st(name).store_file(file, "image/svg+xml", data, json!({ "width": 1200, "height": 800 })).await.unwrap().0
    }

    async fn post(&self, name: &str, key: &str, input: Value) -> Value {
        self.rpc(name, "publish.create", json!({ "key": key, "input": input })).await
    }
}

fn text(t: &str, audience: Value) -> Value {
    json!({ "text": t, "attachments": [], "link": null, "audience": audience })
}

async fn fixture(port: u16) {
    let base = format!("http://127.0.0.1:{port}");
    let feed_base = base.clone();
    let app = axum::Router::new()
        .route(
            "/feed.xml",
            axum::routing::get(move || {
                let b = feed_base.clone();
                async move {
                    let items = [
                        ("Personal servers are back", "a.html", "Self-hosting is getting easier for families.", "Tech Weekly"),
                        ("Balcony gardens in winter", "b.html", "What still grows when it gets cold.", "Green Notes"),
                        ("AI Daily Digest", "c.html", "As an AI language model, I summarized today's news.", "Digest Bot"),
                    ];
                    let items: String = items
                        .iter()
                        .map(|(t, p, d, a)| format!("<item><title>{t}</title><link>{b}/{p}</link><description>{d}</description><dc:creator>{a}</dc:creator></item>"))
                        .collect();
                    ([("content-type", "application/rss+xml")], format!("<?xml version=\"1.0\"?><rss><channel><title>Devnet News</title>{items}</channel></rss>")).into_response()
                }
            }),
        )
        .route(
            "/{page}",
            axum::routing::get(|axum::extract::Path(page): axum::extract::Path<String>| async move {
                (
                    [("content-type", "text/html")],
                    format!("<html><head><title>{page}</title></head><body><h1>{page}</h1><p>Fixture article body for {page}. Personal servers, gardens and tech news.</p></body></html>"),
                )
                    .into_response()
            }),
        );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("fixture port");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let port: u16 = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(4131);
    let data = PathBuf::from(arg("--data-dir").unwrap_or_else(|| "/tmp/homestation-devnet".into()));
    if args.iter().any(|a| a == "--fresh") {
        let _ = std::fs::remove_dir_all(&data);
    }
    let fresh = !homestation::standalone::user_db_path(&data.join("me").join("users"), "me").exists();
    let fixture_port = port + NAMES.len() as u16;
    fixture(fixture_port).await;

    // Keys are kept so a restarted devnet keeps its identities.
    let mut directory = StaticDirectory::default();
    let mut keys = Vec::new();
    for (i, (name, _)) in NAMES.iter().enumerate() {
        let key_file = data.join(format!("{name}.key.json"));
        let (pem, x) = match std::fs::read(&key_file).ok().and_then(|b| serde_json::from_slice::<(String, String)>(&b).ok()) {
            Some(k) => k,
            None => {
                let k = homestation::sign::generate_key();
                std::fs::create_dir_all(&data).unwrap();
                std::fs::write(&key_file, serde_json::to_vec(&k).unwrap()).unwrap();
                k
            }
        };
        let zone = format!("{name}.devnet");
        directory.identities.insert(did(name), StaticIdentity { zone: Some(zone.clone()), user: Some(name.to_string()), public_key_x: Some(x), devices: vec![], owner: None });
        directory.zones.insert(zone, format!("http://127.0.0.1:{}", port + i as u16));
        keys.push(pem);
    }
    let mut nets = Vec::new();
    let mut summary = Vec::new();
    for (i, ((name, display), pem)) in NAMES.iter().zip(keys).enumerate() {
        let mut settings = UserSettings::default();
        settings.profile.name = display.to_string();
        settings.profile.bio = format!("{display} on the HomeStation devnet");
        if *name != "index" {
            settings.collectors = vec![CollectorRef { did: did("index"), name: "Open Index".into() }];
        }
        if *name == "me" {
            settings.topics = vec![
                Topic { id: "topic-tech".into(), name: "Tech".into(), tags: vec!["tech".into(), "servers".into(), "buckyos".into()], subscribed: true },
                Topic { id: "topic-garden".into(), name: "Garden".into(), tags: vec!["garden".into(), "gardens".into(), "园艺".into()], subscribed: true },
                Topic { id: "topic-outdoors".into(), name: "Outdoors".into(), tags: vec!["hiking".into(), "trail".into()], subscribed: false },
            ];
        }
        let contacts: Vec<ContactInfo> = match *name {
            "me" => vec![
                ContactInfo { did: did("alice"), name: "Alice Chen".into(), friend: true, blocked: false, groups: vec![] },
                ContactInfo { did: did("bob"), name: "Bob Zhang".into(), friend: true, blocked: false, groups: vec!["Hiking buddies".into()] },
                ContactInfo { did: did(KAI.0), name: KAI.1.into(), friend: true, blocked: false, groups: vec![] },
            ],
            "alice" | "bob" => vec![
                ContactInfo { did: did("me"), name: "Lin".into(), friend: true, blocked: false, groups: vec![] },
                ContactInfo { did: did(if *name == "alice" { "bob" } else { "alice" }), name: "friend".into(), friend: true, blocked: false, groups: vec![] },
            ],
            _ => vec![],
        };
        let mut tokens = json!({ format!("tok-{name}"): { "principal": name, "did": did(name), "app_id": "control-panel" } });
        let mut users = json!([]);
        if *name == "me" {
            tokens[format!("tok-{}", KAI.0)] = json!({ "principal": KAI.0, "did": did(KAI.0), "app_id": "control-panel" });
            let mut kai = UserSettings::default();
            kai.profile.name = KAI.1.into();
            kai.profile.bio = "Lin's housemate, also on this zone".into();
            kai.collectors = settings.collectors.clone();
            users = json!([{ "user": KAI.0, "did": did(KAI.0), "name": KAI.1, "settings": kai,
                "contacts": [{ "did": did("me"), "name": "Lin", "friend": true }] }]);
        }
        let cfg: StandaloneConfig = serde_json::from_value(json!({
            "owner": did(name),
            "owner_name": display,
            "user": name,
            "zone": format!("{name}.devnet"),
            "zone_name": if *name == "me" { "Lin & Kai's zone".to_string() } else { format!("{name}.devnet") },
            "private_key_pem": pem,
            "listen": format!("127.0.0.1:{}", port + i as u16),
            "collector": *name == "index",
            "spider": true,
            "workers": false,
            "intervals_s": { "pull": 20, "delivery": 5, "select": 10, "comments": 30 },
            "tokens": tokens,
            "contacts": contacts,
            "settings": settings,
            "users": users,
        }))
        .unwrap();
        let mut cfg = cfg;
        cfg.directory = directory.clone();
        let Standalone { station, state, host, .. } = build(cfg, &data.join(name), None, None).expect("build node");
        let listen = format!("127.0.0.1:{}", port + i as u16);
        let listener = tokio::net::TcpListener::bind(&listen).await.expect("node port");
        tokio::spawn(async move {
            let _ = homestation::http::serve_listener(state, listener).await;
        });
        summary.push(json!({ "name": name, "did": did(name), "base": format!("http://{listen}"), "home": format!("http://{listen}/home/{name}"), "token": format!("tok-{name}") }));
        for other in host.opened() {
            if other.cfg.user != *name {
                summary.push(json!({ "name": other.cfg.user, "did": other.cfg.owner, "base": format!("http://{listen}"), "home": format!("http://{listen}/home/{}", other.cfg.user), "token": format!("tok-{}", other.cfg.user) }));
                nets.push((other.cfg.user.clone(), other));
            }
        }
        nets.push((name.to_string(), station));
    }
    let net = Net { nodes: nets };
    if fresh {
        seed(&net, fixture_port).await;
    }
    for (_, s) in &net.nodes {
        s.start_workers();
    }
    println!("{}", json!({ "nodes": summary, "fixture": format!("http://127.0.0.1:{fixture_port}"), "seeded": fresh }));
    tokio::signal::ctrl_c().await.ok();
}

async fn seed(net: &Net, fixture_port: u16) {
    for name in ["me", "alice", "bob", KAI.0] {
        net.rpc(name, "admin.run", json!({ "task": "friends" })).await;
    }
    let follow = net.rpc("me", "sources.resolve", json!({ "kind": "follow", "text": did("sarah") })).await;
    net.rpc("me", "sources.follow", json!({ "resolution": follow })).await;
    let follow = net.rpc("sarah", "sources.resolve", json!({ "kind": "follow", "text": did("me") })).await;
    net.rpc("sarah", "sources.follow", json!({ "resolution": follow })).await;

    let a1 = net.post("alice", "a1", json!({ "text": "Started our balcony garden today: two boxes, basil and lettuce.", "attachments": [], "link": null, "audience": { "kind": "public" }, "tags": ["garden"] })).await;
    let img1 = net.upload("alice", "before.svg", svg(120, "Before")).await;
    let img2 = net.upload("alice", "after.svg", svg(90, "After")).await;
    net.post(
        "alice",
        "a2",
        json!({ "text": "Before and after of the garden weekend", "attachments": [
            { "id": "1", "kind": "image", "name": "Before", "status": "uploaded", "object": img1 },
            { "id": "2", "kind": "image", "name": "After", "status": "uploaded", "object": img2 }
        ], "link": null, "audience": { "kind": "public" } }),
    )
    .await;
    let article = net
        .post(
            "alice",
            "a3",
            json!({ "text": "", "attachments": [], "link": null, "audience": { "kind": "public" }, "tags": ["garden", "tech"],
                "article": { "title": "A week of balcony gardening", "summary": "Light, containers and watering, with notes on what failed.",
                    "markdown": "# A week of balcony gardening\n\n## Light\nSix hours of sun is the minimum for tomatoes.\n\n## Containers\nTwo boxes beat one big one.\n\n## Watering\nMornings, not evenings." } }),
        )
        .await;
    net.post("alice", "a4", text("Family barbecue on Saturday — friends welcome!", json!({ "kind": "friends" }))).await;
    net.post("alice", "a5", json!({ "text": "AIGC experiment: a poem about soil", "attachments": [], "link": null, "audience": { "kind": "public" }, "tags": ["AI生成"] })).await;
    let trail = net.upload("bob", "trail.svg", svg(30, "Mission Peak")).await;
    net.post(
        "bob",
        "b1",
        json!({ "text": "Saturday trail plan: Mission Peak at 7am", "attachments": [{ "id": "1", "kind": "image", "name": "Trail", "status": "uploaded", "object": trail }], "link": null, "audience": { "kind": "public" }, "tags": ["hiking"] }),
    )
    .await;
    net.post("bob", "b2", text("Hiking buddies only: carpool from Fremont", json!({ "kind": "group", "groupId": "Hiking buddies" }))).await;
    let cover = net.upload("sarah", "cover.svg", svg(280, "Portfolio")).await;
    net.post(
        "sarah",
        "s1",
        json!({ "text": "New card design series", "attachments": [], "audience": { "kind": "public" }, "category": "work",
            "link": { "url": "https://sarah.example/works/cards", "title": "Card design series", "summary": "Twelve cards, one palette.", "cover": cover } }),
    )
    .await;
    net.post("sarah", "s2", text("Color palettes for winter interfaces", public_audience())).await;
    let mut m1 = text("Testing my HomeStation — hello from the devnet!", public_audience());
    m1["zoneFeed"] = json!(true);
    net.post("me", "m1", m1).await;
    let mut k1 = text("Kai here: our zone now has a shared front page.", public_audience());
    k1["zoneFeed"] = json!(true);
    net.post(KAI.0, "k1", k1).await;
    net.post(KAI.0, "k2", text("Only for my own home feed.", public_audience())).await;
    net.post("me", "m2", text("Friends only: dinner photos coming soon", json!({ "kind": "friends" }))).await;
    net.settle().await;

    let a1_id = a1["objId"].as_str().unwrap().to_string();
    net.rpc("bob", "interact.comment", json!({ "objId": a1_id, "text": "Try leafy greens first, they forgive mistakes." })).await;
    net.rpc("bob", "interact.like", json!({ "objId": a1_id, "on": true })).await;
    net.rpc("sarah", "admin.run", json!({ "task": "pull" })).await;
    net.rpc("bob", "interact.quote", json!({ "objId": article["objId"], "text": "Good notes — copying the watering schedule." })).await;
    net.settle().await;

    let rss = net.rpc("me", "sources.resolve", json!({ "kind": "url", "text": format!("http://127.0.0.1:{fixture_port}/feed.xml") })).await;
    net.rpc("me", "sources.follow", json!({ "resolution": rss })).await;
    for name in ["me", "alice", "bob", "sarah"] {
        net.rpc(name, "admin.run", json!({ "task": "pull" })).await;
    }
    net.settle().await;
    for name in ["me", "alice", "bob", "sarah"] {
        net.rpc(name, "admin.run", json!({ "task": "selection" })).await;
        net.rpc(name, "admin.run", json!({ "task": "comments" })).await;
    }
    net.rpc("me", "interact.like", json!({ "objId": a1_id, "on": true })).await;
    net.settle().await;
}

fn public_audience() -> Value {
    json!({ "kind": "public" })
}
