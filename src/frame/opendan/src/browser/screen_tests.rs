//! The screen, the requests of pages and the transfers against a fake
//! browser (`tests/fake_browser`), and — ignored, run with
//! `OPENDAN_CHROMIUM=<chrome> cargo test -p opendan --lib browser::screen_tests::real -- --ignored --test-threads=1`
//! — against a real Chromium.
#![cfg(unix)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::screen::Authorize;
use super::transfer::Zone;
use super::*;
use crate::config::BrowserSection;

#[path = "../../tests/fake_browser/mod.rs"]
mod fake_browser;
use fake_browser::FakeBrowser;

const AGENT: &str = "did:bns:jarvis.alice";
const OWNER: &str = "did:bns:bob";
const TOKEN: &str = "owner-token";

/// `OPENDAN_TEST_LOG=1`: what the module logs goes to stderr.
fn logs() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            if record.target().starts_with("opendan") {
                eprintln!("[{}] {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("OPENDAN_TEST_LOG").is_some() && log::set_logger(&Stderr).is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
        }
    });
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ok() {
        assert!(started.elapsed() < Duration::from_secs(30), "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The Zone as the transfers see it: files by path, and what was asked of it.
#[derive(Debug, Default)]
struct FakeZone {
    files: Mutex<HashMap<String, Vec<u8>>>,
    read: Mutex<Vec<(Option<String>, String)>>,
    exported: Mutex<Vec<(Option<String>, PathBuf, String)>>,
}

#[async_trait]
impl Zone for FakeZone {
    async fn read(&self, token: Option<&str>, zone_path: &str, dest: &Path, limit: u64) -> Result<u64, BrowserError> {
        self.read.lock().unwrap().push((token.map(str::to_string), zone_path.to_string()));
        let bytes = self.files.lock().unwrap().get(zone_path).cloned().ok_or_else(|| BrowserError::invalid(format!("`{zone_path}`: no such file")))?;
        if bytes.len() as u64 > limit {
            return Err(transfer::too_large(limit));
        }
        std::fs::write(dest, &bytes).unwrap();
        Ok(bytes.len() as u64)
    }

    async fn export(&self, token: Option<&str>, path: &Path, name: &str) -> Result<String, BrowserError> {
        assert!(path.is_file(), "{}", path.display());
        self.exported.lock().unwrap().push((token.map(str::to_string), path.to_path_buf(), name.to_string()));
        Ok("cyfile:0123456789abcdef".to_string())
    }
}

fn deps(root: &Path) -> BrowserDeps {
    BrowserDeps {
        agent: Arc::new(libopendan::FsAgentStateClient::open(root, AGENT, None, None).unwrap()),
        supervisor: None,
        owner_did: Some(OWNER.to_string()),
        agent_root: root.to_path_buf(),
    }
}

struct Bench {
    _tmp: tempfile::TempDir,
    fake: Arc<FakeBrowser>,
    module: Arc<BrowserModule>,
    zone: Arc<FakeZone>,
}

impl Bench {
    async fn new(tune: impl FnOnce(&mut Tuning)) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = BrowserDirs::at(tmp.path().join("browser"));
        let fake = FakeBrowser::start(&dirs.root).await;
        let zone = Arc::new(FakeZone::default());
        let mut tuning = Tuning { chromium: Some(fake_browser::chromium()), dirs: Some(dirs), zone: Some(zone.clone()), ..Default::default() };
        tune(&mut tuning);
        let cfg = BrowserSection { enabled: true, idle_stop_secs: 600, max_tabs: 4 };
        let module = BrowserModule::with_tuning(cfg, deps(&tmp.path().join("agent_root")), tuning);
        Self { _tmp: tmp, fake, module, zone }
    }

    /// What the page script of a tab tells (`screen_page.js`).
    fn page_says(&self, tab_id: &str, context: i64, told: Value) {
        self.fake.tell(tab_id, "Runtime.bindingCalled", json!({ "name": "__opendanScreen", "payload": told.to_string(), "executionContextId": context }));
    }

    /// A tab shows another document.
    async fn goes_to(&self, tab_id: &str, url: &str) {
        self.module.tab_send(tab_id, "Page.navigate", json!({ "url": url })).await.unwrap();
        self.fake.tell(tab_id, "Page.frameNavigated", json!({ "frame": { "id": tab_id, "url": url } }));
    }

    fn sent(&self, method: &str) -> Vec<Value> {
        self.fake.received(method).into_iter().map(|(_, params)| params).collect()
    }
}

fn owner() -> Authorize {
    Box::new(|_| Box::pin(async { Ok(OWNER.to_string()) }))
}

fn phone() -> Value {
    json!({ "width": 412, "height": 915, "mobile": true })
}

enum Got {
    Message(Value),
    Frame(Value, Vec<u8>),
    Closed,
}

/// The App's end of a screen connection.
struct Client<S> {
    ws: WebSocketStream<S>,
    /// Frames are acknowledged as they arrive.
    acks: bool,
    frames: Vec<(Value, Vec<u8>)>,
    /// Messages that arrived while another one was waited for.
    passed: Vec<Value>,
}

async fn connect(module: &Arc<BrowserModule>, authorize: Authorize) -> Client<tokio::io::DuplexStream> {
    let (near, far) = tokio::io::duplex(1 << 20);
    let server = WebSocketStream::from_raw_socket(far, Role::Server, None).await;
    tokio::spawn(screen::serve(server, module.clone(), authorize));
    Client { ws: WebSocketStream::from_raw_socket(near, Role::Client, None).await, acks: true, frames: Vec::new(), passed: Vec::new() }
}

/// A connection that said `hello` and was told `ready`.
async fn admitted(module: &Arc<BrowserModule>, group_id: &str, viewport: Value) -> (Client<tokio::io::DuplexStream>, Value) {
    let mut client = connect(module, owner()).await;
    client.hello(group_id, viewport).await;
    let ready = client.until("ready").await;
    (client, ready["state"].clone())
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> Client<S> {
    async fn say(&mut self, message: Value) {
        self.ws.send(Message::text(message.to_string())).await.unwrap();
    }

    async fn hello(&mut self, group_id: &str, viewport: Value) {
        self.say(json!({ "type": "hello", "token": TOKEN, "group_id": group_id, "viewport": viewport })).await;
    }

    async fn next_within(&mut self, wait: Duration) -> Option<Got> {
        let message = tokio::time::timeout(wait, self.ws.next()).await.ok()?;
        Some(match message {
            Some(Ok(Message::Text(text))) => {
                let message: Value = serde_json::from_str(text.as_str()).unwrap();
                if message["type"] != "frame_meta" {
                    return Some(Got::Message(message));
                }
                // A frame always follows its description.
                match tokio::time::timeout(Duration::from_secs(10), self.ws.next()).await {
                    Ok(Some(Ok(Message::Binary(data)))) => Got::Frame(message, data.to_vec()),
                    other => panic!("frame_meta without its frame: {other:?}"),
                }
            }
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => Got::Closed,
            Some(Ok(other)) => panic!("unexpected {other:?}"),
        })
    }

    async fn took(&mut self, meta: Value, data: Vec<u8>) {
        if self.acks {
            self.say(json!({ "type": "ack", "seq": meta["seq"] })).await;
        }
        self.frames.push((meta, data));
    }

    /// The next message of `kind` (frames on the way are taken).
    async fn until(&mut self, kind: &str) -> Value {
        loop {
            match self.next_within(Duration::from_secs(20)).await {
                Some(Got::Message(message)) if message["type"] == kind => return message,
                Some(Got::Message(message)) => self.passed.push(message),
                Some(Got::Frame(meta, data)) => self.took(meta, data).await,
                Some(Got::Closed) => panic!("closed while waiting for `{kind}`"),
                None => panic!("no `{kind}` arrived"),
            }
        }
    }

    /// The next `state` that `fits`.
    async fn state(&mut self, what: &str, fits: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            assert!(started.elapsed() < Duration::from_secs(30), "no state where {what}");
            let state = self.until("state").await;
            if fits(&state) {
                return state;
            }
        }
    }

    /// The next frame, not acknowledged.
    async fn frame(&mut self) -> (Value, Vec<u8>) {
        loop {
            match self.next_within(Duration::from_secs(20)).await {
                Some(Got::Frame(meta, data)) => return (meta, data),
                Some(Got::Message(message)) => self.passed.push(message),
                _ => panic!("no frame arrived"),
            }
        }
    }

    /// What arrives within `wait`: messages by type, and how many frames.
    async fn within(&mut self, wait: Duration) -> (Vec<Value>, usize) {
        let (mut messages, mut frames, deadline) = (Vec::new(), 0, Instant::now() + wait);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.next_within(left).await {
                Some(Got::Message(message)) => messages.push(message),
                Some(Got::Frame(meta, data)) => {
                    frames += 1;
                    self.took(meta, data).await;
                }
                Some(Got::Closed) => panic!("closed"),
                None => break,
            }
        }
        (messages, frames)
    }

    async fn closes(&mut self) -> bool {
        loop {
            match self.next_within(Duration::from_secs(10)).await {
                Some(Got::Closed) => return true,
                Some(_) => {}
                None => return false,
            }
        }
    }
}

fn of_kind<'a>(messages: &'a [Value], kind: &str) -> Vec<&'a Value> {
    messages.iter().filter(|message| message["type"] == kind).collect()
}

/// SC1: a connection says `hello` in time, with what `StateService::caller`
/// admits as the owner; only then is anything started or shown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screen_is_admitted_as_the_owner_after_its_hello() {
    let b = Bench::new(|t| t.screen.hello = Duration::from_millis(300)).await;
    let m = &b.module;

    // No hello.
    let mut silent = connect(m, owner()).await;
    assert_eq!(silent.until("error").await["code"], "hello_timeout");
    assert!(silent.closes().await);
    // Something else than a hello, or a hello that names no group or no viewport.
    for bad in [
        json!({ "type": "ack", "seq": 1 }),
        json!({ "type": "hello", "token": TOKEN, "viewport": phone() }),
        json!({ "type": "hello", "token": TOKEN, "group_id": "_reflow", "viewport": phone() }),
        json!({ "type": "hello", "token": TOKEN, "group_id": "default" }),
    ] {
        let mut client = connect(m, owner()).await;
        client.say(bad.clone()).await;
        assert_eq!(client.until("error").await["code"], "bad_hello", "{bad}");
        assert!(client.closes().await);
    }
    let mut binary = connect(m, owner()).await;
    binary.ws.send(Message::Binary(Bytes::from_static(b"hello"))).await.unwrap();
    assert_eq!(binary.until("error").await["code"], "bad_hello");

    // The token is not the owner's (a wrong one, the Agent's own App identity): the service says who may.
    let presented = Arc::new(Mutex::new(None));
    for refusal in ["invalid session token", "browser.screen is the owner's: watching an Agent's browser is what its user does"] {
        let seen = presented.clone();
        let refuse: Authorize = Box::new(move |token| {
            Box::pin(async move {
                *seen.lock().unwrap() = token;
                Err(refusal.to_string())
            })
        });
        let mut client = connect(m, refuse).await;
        client.hello("default", phone()).await;
        let error = client.until("error").await;
        assert_eq!((error["code"].as_str(), error["message"].as_str()), (Some("unauthorized"), Some(refusal)));
        assert!(client.closes().await);
        assert_eq!(presented.lock().unwrap().as_deref(), Some(TOKEN), "the hello's token is what is checked");
    }
    assert!(b.fake.pid().is_none() && m.viewers() == 0, "nobody was admitted: nothing is started, nobody looks");

    // No browser on this node.
    let without = Bench::new(|t| t.chromium = Some(PathBuf::from("/nonexistent/chromium"))).await;
    let mut client = connect(&without.module, owner()).await;
    client.hello("default", phone()).await;
    let error = client.until("error").await;
    assert_eq!(error["code"], "browser_unavailable");
    assert!(error["message"].as_str().unwrap().contains("chromium"));
    assert!(client.closes().await);

    // The owner: the browser is started for them, and they are told the group's state.
    let (mut client, state) = admitted(m, "default", json!({ "width": 100, "height": 915.4, "mobile": true })).await;
    assert!(b.fake.pid().is_some() && m.running());
    assert_eq!(m.viewers(), 1);
    assert_eq!(state["group"]["group_id"], "default");
    assert_eq!((state["group"]["controller"].clone(), state["group"]["tabs"].clone()), (json!("user"), json!([])));
    assert_eq!((state["active_tab_id"].clone(), state["url"].clone(), state["title"].clone(), state["loading"].clone()), (Value::Null, json!(""), json!(""), json!(false)));
    assert_eq!((state["can_go_back"].clone(), state["can_go_forward"].clone(), state["running"].clone()), (json!(false), json!(false), json!(true)));
    assert_eq!(state["viewport"], json!({ "width": 320, "height": 915, "mobile": true }), "sizes are kept within 320–2560");

    // It answers pings, and says what it does not understand without ending.
    client.say(json!({ "type": "ping" })).await;
    assert_eq!(client.until("pong").await, json!({ "type": "pong" }));
    client.say(json!({ "type": "dance" })).await;
    assert_eq!(client.until("error").await["code"], "bad_message");
    client.ws.send(Message::text("not json")).await.unwrap();
    assert_eq!(client.until("error").await["code"], "bad_message");
    client.say(json!({ "type": "ping" })).await;
    client.until("pong").await;

    // The connection ends: nobody looks any more.
    drop(client);
    until("the viewer is gone", || m.viewers() == 0).await;
    m.shutdown().await;
}

/// SC2: a client that says nothing for too long is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_silent_screen_is_closed() {
    let b = Bench::new(|t| t.screen.silence = Duration::from_millis(600)).await;
    let (mut client, _) = admitted(&b.module, "default", phone()).await;
    // Pings keep it.
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        client.say(json!({ "type": "ping" })).await;
        client.until("pong").await;
    }
    let started = Instant::now();
    assert!(client.closes().await);
    assert!(started.elapsed() >= Duration::from_millis(500), "{:?}", started.elapsed());
    until("the viewer is gone", || b.module.viewers() == 0).await;
    b.module.shutdown().await;
}

/// SC2 / SC3: the state is told whenever it changes; tabs are opened as the
/// device that looks, navigated, switched and closed over the connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_state_follows_the_group_and_its_current_tab() {
    let b = Bench::new(|_| {}).await;
    let m = &b.module;
    let (mut c, _) = admitted(m, "thing", phone()).await;

    // An address in a group without tabs opens one: blank first, made a phone, then sent there.
    c.say(json!({ "type": "nav", "action": "url", "url": "a.example/" })).await;
    let state = c.state("the page is loaded", |s| s["url"] == "https://a.example/" && s["loading"] == false && s["title"] == "Title of https://a.example/" && s["can_go_back"] == true).await;
    let first = state["active_tab_id"].as_str().unwrap().to_string();
    assert_eq!(state["group"]["tabs"], json!([{ "tab_id": first, "url": "https://a.example/", "title": "Title of https://a.example/", "loading": false, "active": true }]));
    assert_eq!(b.sent("Target.createTarget"), vec![json!({ "url": "about:blank", "newWindow": true })]);
    let (_, session) = m.tab_session(&first).await.unwrap();
    let of_tab = |method: &str| b.fake.received(method).into_iter().filter(|(s, _)| s.as_deref() == Some(session.as_str())).map(|(_, p)| p).collect::<Vec<_>>();
    assert!(of_tab("Emulation.setDeviceMetricsOverride").contains(&json!({ "width": 412, "height": 915, "deviceScaleFactor": 2, "mobile": true, "screenWidth": 412, "screenHeight": 915 })));
    assert_eq!(of_tab("Emulation.setTouchEmulationEnabled")[0], json!({ "enabled": true, "maxTouchPoints": 5 }));
    let agent = of_tab("Emulation.setUserAgentOverride")[0].clone();
    assert_eq!(agent["userAgent"], "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/1.0.0.0 Mobile Safari/537.36");
    assert_eq!(agent["userAgentMetadata"]["mobile"], true);
    let commands: Vec<String> = b.fake.received_all().into_iter().filter(|(s, ..)| s.as_deref() == Some(session.as_str())).map(|(_, method, _)| method).collect();
    let at = |method: &str| commands.iter().position(|m| m == method).unwrap_or_else(|| panic!("{method} in {commands:?}"));
    assert!(at("Emulation.setUserAgentOverride") < at("Page.navigate"), "a phone before the first request: {commands:?}");
    assert!(of_tab("Page.startScreencast").len() == 1 && m.casts.running() == 1);

    // The same tab goes elsewhere, back and forward.
    c.say(json!({ "type": "nav", "action": "url", "url": "https://b.example/x" })).await;
    c.state("it went on", |s| s["url"] == "https://b.example/x" && s["can_go_back"] == true && s["can_go_forward"] == false).await;
    c.say(json!({ "type": "nav", "action": "back" })).await;
    c.state("it went back", |s| s["url"] == "https://a.example/" && s["can_go_forward"] == true).await;
    c.say(json!({ "type": "nav", "action": "forward" })).await;
    c.state("it went forward", |s| s["url"] == "https://b.example/x" && s["can_go_forward"] == false).await;
    c.say(json!({ "type": "nav", "action": "reload" })).await;
    c.say(json!({ "type": "nav", "action": "stop" })).await;
    until("reload and stop reached the tab", || of_tab("Page.reload").len() == 1 && of_tab("Page.stopLoading").len() == 1).await;
    assert_eq!(m.group("thing").tabs.len(), 1, "an address in a group with a tab opens none");

    // Another tab: it is the current one, and the frames are its frames.
    c.frames.clear();
    c.say(json!({ "type": "tab", "action": "open", "url": "https://c.example/" })).await;
    let state = c.state("the new tab is shown", |s| s["url"] == "https://c.example/" && s["group"]["tabs"].as_array().unwrap().len() == 2).await;
    let second = state["active_tab_id"].as_str().unwrap().to_string();
    assert_ne!(second, first);
    assert!(c.passed.contains(&json!({ "type": "focus", "tab_id": second, "editable": false })), "what has the focus is told for the tab shown: {:?}", c.passed);
    until("its frames arrive", || b.fake.received("Page.startScreencast").len() == 2).await;
    c.within(Duration::from_millis(300)).await;
    assert!(c.frames.iter().any(|(meta, _)| meta["tab_id"] == second.as_str()), "{:?}", c.frames);
    assert_eq!(of_tab("Page.stopScreencast").len(), 1, "the frames of the tab that is not shown are not taken");
    // Back to the first, the second closed.
    c.say(json!({ "type": "tab", "action": "activate", "tab_id": first })).await;
    c.state("the first tab is shown again", |s| s["active_tab_id"] == first.as_str() && s["url"] == "https://b.example/x").await;
    c.say(json!({ "type": "tab", "action": "close", "tab_id": second })).await;
    c.state("the second tab is gone", |s| s["group"]["tabs"].as_array().unwrap().len() == 1).await;
    // A tab of another group is not this connection's.
    let foreign = m.open_tab("other", Some("https://o.example/")).await.unwrap();
    for action in ["activate", "close"] {
        c.say(json!({ "type": "tab", "action": action, "tab_id": foreign.tab_id })).await;
        assert_eq!(c.until("error").await["code"], "no_such_tab");
    }
    assert!(m.tab(&foreign.tab_id).is_some());
    // What cannot be done is said: the browser is full.
    m.open_tab("other", None).await.unwrap();
    m.open_tab("other", None).await.unwrap();
    c.say(json!({ "type": "tab", "action": "open" })).await;
    assert_eq!(c.until("error").await["code"], "too_many_tabs");

    // The desktop version: the tab is a desktop window of that size and is asked for again.
    c.say(json!({ "type": "viewport", "width": 1280, "height": 800, "mobile": false })).await;
    let state = c.state("the viewport is the desktop's", |s| s["viewport"]["mobile"] == false).await;
    assert_eq!(state["viewport"], json!({ "width": 1280, "height": 800, "mobile": false }));
    until("the tab is a desktop window", || of_tab("Page.reload").len() == 2).await;
    assert_eq!(of_tab("Emulation.setDeviceMetricsOverride").last().unwrap(), &json!({ "width": 1280, "height": 800, "deviceScaleFactor": 0, "mobile": false }));
    assert_eq!(of_tab("Emulation.setTouchEmulationEnabled").last().unwrap()["enabled"], false);
    assert_eq!(of_tab("Emulation.setUserAgentOverride").last().unwrap(), &json!({ "userAgent": "" }));
    // Only the size: nothing is asked for again.
    c.say(json!({ "type": "viewport", "width": 1024, "height": 700, "mobile": false })).await;
    c.state("the viewport is smaller", |s| s["viewport"]["width"] == 1024).await;
    assert_eq!(of_tab("Page.reload").len(), 2);

    // The conversation becomes the default one: the connection shows the default group.
    m.bind_default("thing").unwrap();
    c.state("the group is the default group", |s| s["group"]["group_id"] == "default" && s["active_tab_id"] == first.as_str()).await;
    drop(c);
    until("the screencast ended with its viewer", || m.casts.running() == 0 && m.viewers() == 0).await;
    assert_eq!(of_tab("Page.stopScreencast").len(), 2);
    m.shutdown().await;
}

/// SC3: at most one frame the client has not acknowledged; the latest frame
/// replaces what a slow client did not take; the frame rate is capped; the
/// browser's own frames are acknowledged whatever the client does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn frames_are_sent_one_at_a_time_and_only_the_latest() {
    let b = Bench::new(|_| {}).await;
    let m = &b.module;
    let tab = m.open_tab("thing", Some("https://a.example/")).await.unwrap();
    let (mut c, _) = admitted(m, "thing", phone()).await;
    c.acks = false;

    // What the tab shows when its frames start to be taken.
    let (meta, data) = c.frame().await;
    assert_eq!(meta, json!({ "type": "frame_meta", "seq": 1, "tab_id": tab.tab_id, "width": 412, "height": 915 }));
    assert_eq!(data, b"first");
    assert_eq!(b.sent("Page.startScreencast"), vec![json!({ "format": "jpeg", "quality": 60, "everyNthFrame": 1 })]);

    // Not acknowledged: no further frame, whatever the page paints.
    for n in 0..5 {
        b.fake.frame(&tab.tab_id, format!("frame-{n}").as_bytes());
    }
    let (_, frames) = c.within(Duration::from_millis(500)).await;
    assert_eq!(frames, 0, "a client that did not acknowledge gets no frame");
    until("the browser's frames are acknowledged all the same", || b.sent("Page.screencastFrameAck").len() == 6).await;
    // Acknowledged: the latest frame, none of those in between.
    c.say(json!({ "type": "ack", "seq": 1 })).await;
    let (meta, data) = c.frame().await;
    assert_eq!((meta["seq"].as_u64(), data.as_slice()), (Some(2), b"frame-4".as_slice()));
    c.say(json!({ "type": "ack", "seq": 2 })).await;
    let (_, frames) = c.within(Duration::from_millis(300)).await;
    assert_eq!(frames, 0, "nothing new was painted");
    // An acknowledgement of an older frame is not one of the frame on its way.
    b.fake.frame(&tab.tab_id, b"frame-5");
    c.frame().await;
    b.fake.frame(&tab.tab_id, b"frame-6");
    c.say(json!({ "type": "ack", "seq": 2 })).await;
    assert_eq!(c.within(Duration::from_millis(300)).await.1, 0);
    c.say(json!({ "type": "ack", "seq": 3 })).await;
    assert_eq!(c.frame().await.1, b"frame-6");
    c.say(json!({ "type": "ack", "seq": 4 })).await;

    // The frame rate: a page that paints 50 times a second, a client that takes every frame at once.
    c.acks = true;
    c.say(json!({ "type": "quality", "max_fps": 5 })).await;
    let painter = {
        let (fake, tab_id) = (b.fake.clone(), tab.tab_id.clone());
        tokio::spawn(async move {
            for n in 0..50 {
                fake.frame(&tab_id, format!("burst-{n}").as_bytes());
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };
    c.frames.clear();
    let (_, frames) = c.within(Duration::from_millis(1000)).await;
    assert!((3..=6).contains(&frames), "5 frames a second at most, got {frames}");
    painter.await.unwrap();
    c.within(Duration::from_millis(500)).await;
    assert_eq!(c.frames.last().unwrap().1, b"burst-49", "what the page shows in the end is what the client has");
    // Default: 15 a second.
    c.say(json!({ "type": "quality", "max_fps": 15 })).await;
    let painter = {
        let (fake, tab_id) = (b.fake.clone(), tab.tab_id.clone());
        tokio::spawn(async move {
            for n in 0..50 {
                fake.frame(&tab_id, format!("fast-{n}").as_bytes());
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };
    let (_, frames) = c.within(Duration::from_millis(1000)).await;
    assert!((7..=16).contains(&frames), "15 frames a second at most, got {frames}");
    painter.await.unwrap();
    c.within(Duration::from_millis(300)).await;

    // The picture quality: the screencast is started again with it.
    c.say(json!({ "type": "quality", "max_fps": 15, "jpeg_quality": 80 })).await;
    until("the screencast runs with the new quality", || b.sent("Page.startScreencast").last().is_some_and(|p| p["quality"] == 80)).await;
    assert_eq!(b.sent("Page.stopScreencast").len(), 1);

    // Two screens of one tab share its frames; the screencast ends with the last of them.
    let (mut other, _) = admitted(m, "thing", phone()).await;
    other.within(Duration::from_millis(300)).await;
    assert_eq!(other.frames.len(), 1, "what the tab shows now");
    assert_eq!((m.casts.running(), b.sent("Page.startScreencast").len()), (1, 2));
    b.fake.frame(&tab.tab_id, b"for both");
    other.within(Duration::from_millis(300)).await;
    c.within(Duration::from_millis(300)).await;
    assert_eq!((other.frames.last().unwrap().1.as_slice(), c.frames.last().unwrap().1.as_slice()), (b"for both".as_slice(), b"for both".as_slice()));
    // A tab that was made another size and not painted (a browser does that now and then) owes a frame
    // of that size: its screencast is started again, which paints one.
    b.fake.flag("fake_resizes_unpainted", true);
    let starts = b.sent("Page.startScreencast").len();
    c.say(json!({ "type": "viewport", "width": 400, "height": 900, "mobile": true })).await;
    c.within(Duration::from_millis(1000)).await;
    assert_eq!(b.sent("Page.startScreencast").len(), starts + 1);
    let (meta, data) = c.frames.last().unwrap();
    assert_eq!((meta["width"].as_u64(), meta["height"].as_u64(), data.as_slice()), (Some(400), Some(900), b"first".as_slice()));
    b.fake.flag("fake_resizes_unpainted", false);
    // So does a tab that finished loading and painted nothing (a browser does that after some reloads).
    let (starts, stops) = (b.sent("Page.startScreencast").len(), b.sent("Page.stopScreencast").len());
    b.fake.tell(&tab.tab_id, "Page.frameStoppedLoading", json!({ "frameId": tab.tab_id }));
    c.within(Duration::from_millis(1000)).await;
    assert_eq!((b.sent("Page.startScreencast").len(), b.sent("Page.stopScreencast").len()), (starts + 1, stops + 1));
    assert_eq!(c.frames.last().unwrap().1, b"first");
    // One that painted owes nothing.
    b.fake.tell(&tab.tab_id, "Page.frameStoppedLoading", json!({ "frameId": tab.tab_id }));
    b.fake.frame(&tab.tab_id, b"loaded");
    c.within(Duration::from_millis(800)).await;
    assert_eq!(b.sent("Page.startScreencast").len(), starts + 1);

    drop(other);
    until("one viewer is left", || m.viewers() == 1).await;
    assert_eq!((m.casts.running(), b.sent("Page.stopScreencast").len()), (1, stops + 1));
    drop(c);
    until("the screencast ended", || m.casts.running() == 0 && b.sent("Page.stopScreencast").len() == stops + 2).await;
    m.shutdown().await;
}

/// A tab has one viewport, whoever looks at it: that of the screen that set
/// its own last. Every screen is told which one that is; when that screen
/// is gone, the tab gets the viewport of those that are left.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tab_has_the_viewport_of_the_screen_that_set_its_own_last() {
    let b = Bench::new(|_| {}).await;
    let m = &b.module;
    let tab = m.open_tab("thing", Some("https://a.example/")).await.unwrap();
    let (_, session) = m.tab_session(&tab.tab_id).await.unwrap();
    let of_tab = |method: &str| b.fake.received(method).into_iter().filter(|(s, _)| s.as_deref() == Some(session.as_str())).map(|(_, p)| p).collect::<Vec<_>>();
    let devices = || of_tab("Emulation.setDeviceMetricsOverride").iter().map(|p| (p["width"].as_u64().unwrap(), p["height"].as_u64().unwrap(), p["mobile"] == true)).collect::<Vec<_>>();
    let desktop = json!({ "width": 1078, "height": 576, "mobile": false });
    let resized = json!({ "width": 1000, "height": 600, "mobile": false });

    // The first screen: the tab is a desktop window of its size.
    let (mut wide, state) = admitted(m, "thing", desktop.clone()).await;
    assert_eq!(state["viewport"], desktop);
    until("the tab is the desktop's", || devices().last() == Some(&(1078, 576, false))).await;

    // A phone looks at the same tab: the tab is a phone now, and the desktop is told so.
    let (mut small, state) = admitted(m, "thing", phone()).await;
    assert_eq!(state["viewport"], phone());
    wide.state("the desktop sees the phone's viewport", |s| s["viewport"] == phone()).await;
    until("the tab is the phone's", || devices().last() == Some(&(412, 915, true))).await;
    until("with a touch screen", || of_tab("Emulation.setTouchEmulationEnabled").last().is_some_and(|p| p["enabled"] == true)).await;

    // The desktop's window was resized: the tab is the desktop's again, and the phone is told so.
    let mut message = resized.clone();
    message["type"] = json!("viewport");
    wide.say(message).await;
    small.state("the phone sees the desktop's viewport", |s| s["viewport"] == resized).await;
    wide.state("the desktop has its own", |s| s["viewport"] == resized).await;
    until("the tab is the desktop's again", || devices().last() == Some(&(1000, 600, false))).await;
    // Saying its viewport again makes the tab the phone's again.
    let mut message = phone();
    message["type"] = json!("viewport");
    small.say(message).await;
    wide.state("the desktop sees the phone's viewport again", |s| s["viewport"] == phone()).await;
    until("the tab is the phone's again", || devices().last() == Some(&(412, 915, true))).await;

    // The phone is gone: the tab is the desktop's, which is told so. Nothing was loaded again on the way.
    drop(small);
    wide.state("the desktop has its own again", |s| s["viewport"] == resized).await;
    until("the tab is the desktop's once more", || devices().last() == Some(&(1000, 600, false))).await;
    until("and no phone", || of_tab("Emulation.setTouchEmulationEnabled").last().unwrap()["enabled"] == false && of_tab("Emulation.setUserAgentOverride").last().unwrap() == &json!({ "userAgent": "" })).await;
    assert!(of_tab("Page.reload").is_empty(), "a screen that comes or goes loads nothing again");
    assert_eq!((m.casts.running(), of_tab("Page.startScreencast").len()), (1, 1), "one screencast all along");

    // The last screen is gone: the tab stays what it was.
    let before = devices();
    drop(wide);
    until("the screencast ended", || m.casts.running() == 0 && m.viewers() == 0).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(devices(), before);
    m.shutdown().await;
}

/// SC4: what the user does is the page's input; while the agent operates
/// the group it is dropped, and the user is told so once a second.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_the_user_does_reaches_the_page_unless_the_agent_operates() {
    let b = Bench::new(|_| {}).await;
    let m = &b.module;
    let tab = m.open_tab(DEFAULT_GROUP, Some("https://a.example/")).await.unwrap();
    let (_, session) = m.tab_session(&tab.tab_id).await.unwrap();
    let (mut c, _) = admitted(m, DEFAULT_GROUP, phone()).await;
    let of_tab = |method: &str| b.fake.received(method).into_iter().filter(|(s, _)| s.as_deref() == Some(session.as_str())).map(|(_, p)| p).collect::<Vec<_>>();

    for input in [
        json!({ "kind": "touch", "phase": "start", "points": [{ "id": 0, "x": 120, "y": 300 }] }),
        json!({ "kind": "touch", "phase": "end", "points": [] }),
        json!({ "kind": "mouse", "action": "down", "x": 10, "y": 20, "button": "left", "click_count": 1, "modifiers": 0 }),
        json!({ "kind": "wheel", "x": 10, "y": 20, "delta_x": 0, "delta_y": 120, "modifiers": 0 }),
        json!({ "kind": "key", "action": "down", "key": "Enter", "code": "Enter", "modifiers": 0, "text": "" }),
        json!({ "kind": "composition", "text": "zhong", "selection_start": 5, "selection_end": 5 }),
        json!({ "kind": "text", "text": "中文" }),
    ] {
        let mut message = input;
        message["type"] = json!("input");
        c.say(message).await;
    }
    until("the input reached the tab", || of_tab("Input.insertText") == vec![json!({ "text": "中文" })]).await;
    assert_eq!(
        of_tab("Input.dispatchTouchEvent"),
        vec![json!({ "type": "touchStart", "touchPoints": [{ "id": 0, "x": 120.0, "y": 300.0 }] }), json!({ "type": "touchEnd", "touchPoints": [] })]
    );
    let mouse = of_tab("Input.dispatchMouseEvent");
    assert_eq!(mouse.iter().map(|p| p["type"].as_str().unwrap()).collect::<Vec<_>>(), vec!["mousePressed", "mouseWheel"]);
    assert_eq!(of_tab("Input.dispatchKeyEvent")[0]["windowsVirtualKeyCode"], 13);
    assert_eq!(of_tab("Input.imeSetComposition"), vec![json!({ "text": "zhong", "selectionStart": 5, "selectionEnd": 5 })]);
    // An input that makes no sense is said, not sent.
    c.say(json!({ "type": "input", "kind": "mouse", "action": "down", "x": "left" })).await;
    assert_eq!(c.until("error").await["code"], "bad_message");

    // The agent operates: input, navigation and tabs are dropped; the user is told, once a second.
    let lease = m.acquire("s1").await.unwrap();
    assert_eq!(lease.group_id, DEFAULT_GROUP);
    c.state("the agent operates", |s| s["group"]["controller"] == "agent" && s["group"]["agent_state"] == "working").await;
    for dropped in [
        json!({ "type": "input", "kind": "text", "text": "dropped" }),
        json!({ "type": "input", "kind": "mouse", "action": "down", "x": 1, "y": 1, "button": "left" }),
        json!({ "type": "nav", "action": "url", "url": "https://dropped.example/" }),
        json!({ "type": "nav", "action": "reload" }),
        json!({ "type": "tab", "action": "open" }),
        json!({ "type": "tab", "action": "close", "tab_id": tab.tab_id }),
    ] {
        c.say(dropped).await;
    }
    let (messages, _) = c.within(Duration::from_millis(600)).await;
    assert_eq!(of_kind(&messages, "error").iter().map(|e| e["code"].as_str().unwrap()).collect::<Vec<_>>(), vec!["agent_in_control"], "{messages:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    c.say(json!({ "type": "input", "kind": "text", "text": "dropped again" })).await;
    assert_eq!(c.until("error").await["code"], "agent_in_control", "told again a second later");
    assert_eq!(of_tab("Input.insertText").len(), 1);
    assert_eq!((of_tab("Page.navigate").len(), of_tab("Page.reload").len(), m.group(DEFAULT_GROUP).tabs.len()), (0, 0, 1));
    // What is not operating the page goes on: frames, pings, the viewport.
    c.say(json!({ "type": "ping" })).await;
    c.until("pong").await;
    c.say(json!({ "type": "viewport", "width": 400, "height": 900, "mobile": true })).await;
    c.state("the viewport changed", |s| s["viewport"]["width"] == 400).await;

    // The user takes over: what they do reaches the page again.
    m.hand(OWNER, DEFAULT_GROUP, Handed::TakeOver).await.unwrap();
    c.state("the user operates", |s| s["group"]["controller"] == "user" && s["group"]["user_hold"] == true).await;
    c.say(json!({ "type": "input", "kind": "text", "text": "mine" })).await;
    until("the input reached the tab", || of_tab("Input.insertText").last() == Some(&json!({ "text": "mine" }))).await;
    // What the agent's Playwright did may have ended the device the tab was made (a screenshot does): the
    // tab is made that device once more — with another screen first, or the browser takes it for done —
    // and its page is not loaded again.
    let phone = json!({ "width": 400, "height": 900, "deviceScaleFactor": 2, "mobile": true, "screenWidth": 400, "screenHeight": 900 });
    let mut other = phone.clone();
    other["screenWidth"] = json!(401);
    until("the tab is made the phone once more", || of_tab("Emulation.setDeviceMetricsOverride").ends_with(&[other.clone(), phone.clone()])).await;
    assert_eq!(of_tab("Page.reload").len(), 0);
    m.shutdown().await;
}

/// SC5: what a page shows outside itself is a request of the screens that
/// look at its group; unanswered requests come again with a new connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_pages_show_outside_themselves_is_asked_of_the_screen() {
    let b = Bench::new(|t| t.screen.dialog = Duration::from_millis(1500)).await;
    let m = &b.module;
    let tab = m.open_tab("thing", Some("https://a.example/")).await.unwrap();
    let (_, session) = m.tab_session(&tab.tab_id).await.unwrap();
    let of_tab = |method: &str| b.fake.received(method).into_iter().filter(|(s, _)| s.as_deref() == Some(session.as_str())).map(|(_, p)| p).collect::<Vec<_>>();
    // Every document gets the page script, in a world of its own; file choosers are intercepted.
    until("the tab is prepared", || !of_tab("Page.setInterceptFileChooserDialog").is_empty()).await;
    assert_eq!(of_tab("Runtime.addBinding"), vec![json!({ "name": "__opendanScreen", "executionContextName": "opendan_screen" })]);
    let scripts = of_tab("Page.addScriptToEvaluateOnNewDocument");
    assert_eq!(scripts.iter().map(|p| p["worldName"].clone()).collect::<Vec<_>>(), vec![json!("opendan_screen"), Value::Null]);
    assert!(scripts[0]["source"].as_str().unwrap().contains("__opendanSelect"));
    let refused: Vec<String> = b.sent("Browser.setPermission").iter().map(|p| format!("{}:{}", p["permission"]["name"].as_str().unwrap(), p["setting"].as_str().unwrap())).collect();
    for permission in ["geolocation:denied", "camera:denied", "microphone:denied", "notifications:denied", "clipboard-read:denied"] {
        assert!(refused.contains(&permission.to_string()), "{permission} in {refused:?}");
    }

    // Nobody looks: a select's list is nobody's to pick from.
    b.page_says(&tab.tab_id, 7, json!({ "kind": "select", "token": 1, "multiple": false, "options": [], "rect": { "x": 0, "y": 0, "width": 1, "height": 1 } }));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(m.outside.pending(m, "thing").is_empty());

    let (mut c, _) = admitted(m, "thing", phone()).await;
    // ---- <select>
    let options = json!([
        { "index": 0, "label": "Red", "selected": true, "disabled": false },
        { "index": 1, "label": "Green", "selected": false, "disabled": false, "group": "Cold" },
    ]);
    let rect = json!({ "x": 20, "y": 300, "width": 300, "height": 40 });
    b.page_says(&tab.tab_id, 7, json!({ "kind": "select", "token": 3, "multiple": false, "options": options, "rect": rect }));
    let request = c.until("request").await;
    let select_id = request["request_id"].as_str().unwrap().to_string();
    assert_eq!(request, json!({ "type": "request", "request_id": select_id, "kind": "select", "tab_id": tab.tab_id, "multiple": false, "options": options, "rect": { "x": 20, "y": 300, "width": 300, "height": 40 } }));
    let evaluated = || of_tab("Runtime.evaluate").into_iter().filter(|p| p["expression"].as_str().is_some_and(|e| e.starts_with("globalThis.__opendanSelect"))).collect::<Vec<_>>();
    c.say(json!({ "type": "respond", "request_id": select_id, "result": { "selected": [1] } })).await;
    assert_eq!(c.until("request_closed").await["request_id"], select_id.as_str());
    until("the choice is made in the page", || evaluated() == vec![json!({ "expression": "globalThis.__opendanSelect(3, [1])", "contextId": 7 })]).await;
    // Cancelled: nothing is set. Answered twice: there is no such request.
    b.page_says(&tab.tab_id, 7, json!({ "kind": "select", "token": 4, "multiple": false, "options": options, "rect": rect }));
    let again = c.until("request").await["request_id"].as_str().unwrap().to_string();
    c.say(json!({ "type": "respond", "request_id": again, "result": { "cancel": true } })).await;
    c.until("request_closed").await;
    c.say(json!({ "type": "respond", "request_id": again, "result": { "selected": [0] } })).await;
    assert_eq!(c.until("error").await["code"], "no_such_request");
    assert_eq!(evaluated().len(), 1);

    // ---- focus
    b.page_says(&tab.tab_id, 7, json!({ "kind": "focus", "editable": true, "input_type": "email", "rect": { "x": 1, "y": 2, "width": 3, "height": 4 } }));
    assert_eq!(c.until("focus").await, json!({ "type": "focus", "tab_id": tab.tab_id, "editable": true, "input_type": "email", "rect": { "x": 1, "y": 2, "width": 3, "height": 4 } }));
    // Another document: its focus is told anew.
    b.goes_to(&tab.tab_id, "https://a.example/next").await;
    assert_eq!(c.until("focus").await, json!({ "type": "focus", "tab_id": tab.tab_id, "editable": false }));

    // ---- permission: told once, the browser has refused it.
    for _ in 0..3 {
        b.page_says(&tab.tab_id, 7, json!({ "kind": "permission", "permission": "geolocation", "origin": "https://a.example" }));
    }
    let request = c.until("request").await;
    assert_eq!((request["kind"].clone(), request["permission"].clone(), request["origin"].clone(), request["tab_id"].clone()), (json!("permission"), json!("geolocation"), json!("https://a.example"), json!(tab.tab_id)));
    let (messages, _) = c.within(Duration::from_millis(300)).await;
    assert!(of_kind(&messages, "request").is_empty(), "{messages:?}");

    // ---- JS dialogs
    let handled = || b.sent("Page.handleJavaScriptDialog");
    b.fake.dialog(&tab.tab_id, "prompt", "Your name?");
    let request = c.until("request").await;
    let dialog_id = request["request_id"].as_str().unwrap().to_string();
    assert_eq!(request, json!({ "type": "request", "request_id": dialog_id, "kind": "dialog", "tab_id": tab.tab_id, "dialog_type": "prompt", "message": "Your name?", "default_prompt": "" }));
    c.say(json!({ "type": "respond", "request_id": dialog_id, "result": { "accept": true, "prompt_text": "Bob" } })).await;
    assert_eq!(c.until("request_closed").await["request_id"], dialog_id.as_str());
    until("the dialog is answered", || handled() == vec![json!({ "accept": true, "promptText": "Bob" })]).await;
    b.fake.dialog(&tab.tab_id, "confirm", "Sure?");
    let confirm = c.until("request").await["request_id"].as_str().unwrap().to_string();
    c.say(json!({ "type": "respond", "request_id": confirm, "result": { "accept": false } })).await;
    c.until("request_closed").await;
    until("the dialog is dismissed", || handled().len() == 2 && handled()[1] == json!({ "accept": false })).await;
    // Not answered in time: closed as a dialog nobody attends to.
    let asked = Instant::now();
    b.fake.dialog(&tab.tab_id, "alert", "Saved");
    let alert = c.until("request").await["request_id"].as_str().unwrap().to_string();
    assert_eq!(c.until("request_closed").await["request_id"], alert.as_str());
    assert!(asked.elapsed() >= Duration::from_millis(1400), "{:?}", asked.elapsed());
    assert_eq!(handled().last().unwrap(), &json!({ "accept": false }));
    b.fake.dialog(&tab.tab_id, "beforeunload", "");
    c.until("request").await;
    c.until("request_closed").await;
    assert_eq!(handled().last().unwrap(), &json!({ "accept": true }), "leaving the page is let happen");

    // ---- a file chooser, and what waits for an answer when a screen connects again
    b.fake.tell(&tab.tab_id, "Page.fileChooserOpened", json!({ "frameId": tab.tab_id, "mode": "selectMultiple", "backendNodeId": 5 }));
    let request = c.until("request").await;
    let chooser = request["request_id"].as_str().unwrap().to_string();
    assert_eq!(request, json!({ "type": "request", "request_id": chooser, "kind": "file_chooser", "tab_id": tab.tab_id, "multiple": true, "accept": [".png", "image/*"] }));
    b.page_says(&tab.tab_id, 7, json!({ "kind": "select", "token": 9, "multiple": false, "options": options, "rect": rect }));
    let select = c.until("request").await["request_id"].as_str().unwrap().to_string();
    b.fake.dialog(&tab.tab_id, "confirm", "Still there?");
    let dialog = c.until("request").await["request_id"].as_str().unwrap().to_string();
    drop(c);
    until("the viewer is gone", || m.viewers() == 0).await;
    // The dialog nobody looks at any more is closed; the others wait.
    until("the dialog left behind is dismissed", || m.open_dialogs("thing").is_empty()).await;
    b.fake.dialog(&tab.tab_id, "confirm", "Opened while nobody looked");
    until("it is closed at once", || b.fake.open_dialogs().is_empty()).await;
    let (mut c, _) = admitted(m, "thing", phone()).await;
    let (messages, _) = c.within(Duration::from_millis(400)).await;
    let mut shown: Vec<String> = of_kind(&messages, "request").iter().map(|r| r["request_id"].as_str().unwrap().to_string()).collect();
    shown.sort();
    let mut waiting = vec![chooser.clone(), select.clone()];
    waiting.sort();
    assert_eq!(shown, waiting, "{messages:?}");
    assert!(!shown.contains(&dialog));
    // A dialog that was open before a screen connected (the agent's page raised it) is asked of it.
    drop(c);
    until("the viewer is gone", || m.viewers() == 0).await;
    let lease = m.acquire("s1").await.unwrap();
    let agents = m.open_tab(DEFAULT_GROUP, Some("https://d.example/")).await.unwrap();
    m.tab_session(&agents.tab_id).await.unwrap();
    b.fake.dialog(&agents.tab_id, "alert", "Open before");
    until("the dialog is open", || m.open_dialogs(DEFAULT_GROUP).len() == 1).await;
    assert!(m.outside.pending(m, DEFAULT_GROUP).is_empty(), "nobody looks: it is the agent's");
    let (mut w, _) = admitted(m, DEFAULT_GROUP, phone()).await;
    let request = w.until("request").await;
    assert_eq!((request["kind"].clone(), request["message"].clone(), request["tab_id"].clone()), (json!("dialog"), json!("Open before"), json!(agents.tab_id)));
    // Answering a request is not operating the page: the user may, while the agent operates.
    w.say(json!({ "type": "respond", "request_id": request["request_id"], "result": { "accept": true } })).await;
    w.until("request_closed").await;
    until("the dialog is answered", || handled().last() == Some(&json!({ "accept": true }))).await;
    m.release(&lease.lease_id, None, None).unwrap();
    drop(w);
    until("the viewer is gone", || m.viewers() == 0).await;

    // The page goes elsewhere: what its document asked for is over.
    let (mut c, _) = admitted(m, "thing", phone()).await;
    c.within(Duration::from_millis(300)).await;
    b.goes_to(&tab.tab_id, "https://a.example/gone").await;
    let (messages, _) = c.within(Duration::from_millis(400)).await;
    let mut closed: Vec<String> = of_kind(&messages, "request_closed").iter().map(|r| r["request_id"].as_str().unwrap().to_string()).collect();
    closed.sort();
    assert_eq!(closed, waiting, "{messages:?}");
    assert!(m.outside.pending(m, "thing").is_empty());
    m.shutdown().await;
}

fn body(bytes: &[u8]) -> http_body_util::combinators::BoxBody<Bytes, buckyos_http_server::ServerError> {
    Full::new(Bytes::copy_from_slice(bytes)).map_err(|e| match e {}).boxed()
}

/// A page opens a file chooser: the request the screen is shown.
async fn chooser(b: &Bench, c: &mut Client<tokio::io::DuplexStream>, tab_id: &str, mode: &str, node: i64) -> String {
    b.fake.tell(tab_id, "Page.fileChooserOpened", json!({ "frameId": tab_id, "mode": mode, "backendNodeId": node }));
    c.until("request").await["request_id"].as_str().unwrap().to_string()
}

/// SC6: the files the user chose reach the page's file input together, from
/// the Zone (read with the caller's token) and from the App's device; a
/// finished download becomes an object.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chosen_files_reach_the_page_and_downloads_become_objects() {
    let b = Bench::new(|t| t.upload_limit = 64).await;
    let m = &b.module;
    let tab = m.open_tab("thing", Some("https://a.example/")).await.unwrap();
    m.tab_session(&tab.tab_id).await.unwrap();
    let (mut c, _) = admitted(m, "thing", phone()).await;
    b.zone.files.lock().unwrap().insert("/home/Documents/报告.pdf".into(), b"zone bytes".to_vec());
    b.zone.files.lock().unwrap().insert("/home/big.bin".into(), vec![0u8; 65]);

    // Several files: kept until the last one says `done`, then given to the input at once.
    let request = chooser(&b, &mut c, &tab.tab_id, "selectMultiple", 5).await;
    transfer::upload(m, Some(TOKEN), &request, "/home/Documents/报告.pdf", false).await.unwrap();
    assert_eq!(b.zone.read.lock().unwrap().clone(), vec![(Some(TOKEN.to_string()), "/home/Documents/报告.pdf".to_string())], "read with the caller's token");
    assert!(b.sent("DOM.setFileInputFiles").is_empty(), "not before the last file");
    transfer::receive(m, &request, "../photo 1.png", true, body(b"device bytes")).await.unwrap();
    let set = b.sent("DOM.setFileInputFiles");
    assert_eq!((set.len(), set[0]["backendNodeId"].clone()), (1, json!(5)));
    let files: Vec<PathBuf> = set[0]["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect();
    assert_eq!(files.iter().map(|f| f.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), vec!["报告.pdf", "photo 1.png"], "the page sees the files' names");
    assert_eq!((std::fs::read(&files[0]).unwrap(), std::fs::read(&files[1]).unwrap()), (b"zone bytes".to_vec(), b"device bytes".to_vec()));
    assert!(files.iter().all(|f| f.starts_with(m.dirs().root.join("uploads").join(&request))));
    assert_eq!(c.until("request_closed").await["request_id"], request.as_str());
    assert_eq!(transfer::receive(m, &request, "late.png", true, body(b"x")).await.unwrap_err().code, "no_such_request");
    // The page reads them when it sends them: kept while its tab is there.
    assert!(files[0].exists());

    // One file for an input that takes one; a file that is too large; a file that is not there.
    let request = chooser(&b, &mut c, &tab.tab_id, "selectSingle", 6).await;
    assert_eq!(transfer::receive(m, &request, "big.bin", true, body(&[0u8; 65])).await.unwrap_err().code, "upload_too_large");
    assert_eq!(transfer::upload(m, Some(TOKEN), &request, "/home/big.bin", true).await.unwrap_err().code, "upload_too_large");
    assert_eq!(transfer::upload(m, Some(TOKEN), &request, "/home/missing.txt", true).await.unwrap_err().code, "invalid_argument");
    assert!(!m.dirs().root.join("uploads").join(&request).join("1").exists(), "what did not arrive is not kept");
    transfer::receive(m, &request, "small.txt", false, body(b"ok")).await.unwrap();
    assert_eq!(transfer::receive(m, &request, "second.txt", true, body(b"no")).await.unwrap_err().code, "invalid_argument", "it takes one file");
    // Cancelled: the input gets nothing, what was staged is deleted.
    c.say(json!({ "type": "respond", "request_id": request, "result": { "cancel": true } })).await;
    assert_eq!(c.until("request_closed").await["request_id"], request.as_str());
    until("the staged file is deleted", || !m.dirs().root.join("uploads").join(&request).exists()).await;
    assert_eq!(b.sent("DOM.setFileInputFiles").len(), 1);
    assert_eq!(transfer::upload(m, Some(TOKEN), "r-none", "/home/Documents/报告.pdf", true).await.unwrap_err().code, "no_such_request");

    // ---- downloads
    std::fs::create_dir_all(&m.dirs().downloads).unwrap();
    std::fs::write(m.dirs().downloads.join("guid-1"), b"downloaded!").unwrap();
    b.fake.announce("Browser.downloadWillBegin", json!({ "frameId": tab.tab_id, "guid": "guid-1", "url": "https://a.example/report.pdf", "suggestedFilename": "report.pdf" }));
    b.fake.announce("Browser.downloadProgress", json!({ "guid": "guid-1", "totalBytes": 11, "receivedBytes": 4, "state": "inProgress" }));
    assert_eq!(transfer::export(m, Some(TOKEN), "guid-1").await.unwrap_err().code, "no_such_download", "not before it is complete");
    b.fake.announce("Browser.downloadProgress", json!({ "guid": "guid-1", "totalBytes": 11, "receivedBytes": 11, "state": "completed" }));
    let request = c.until("request").await;
    assert_eq!(
        (request["kind"].clone(), request["tab_id"].clone(), request["download_id"].clone(), request["name"].clone(), request["size"].clone(), request["state"].clone()),
        (json!("download"), json!(tab.tab_id), json!("guid-1"), json!("report.pdf"), json!(11), json!("completed"))
    );
    assert_eq!(transfer::export(m, Some(TOKEN), "guid-1").await.unwrap(), json!({ "obj_id": "cyfile:0123456789abcdef", "name": "report.pdf", "size": 11 }));
    assert_eq!(b.zone.exported.lock().unwrap().clone(), vec![(Some(TOKEN.to_string()), m.dirs().downloads.join("guid-1"), "report.pdf".to_string())]);
    // A download that did not finish.
    b.fake.announce("Browser.downloadWillBegin", json!({ "frameId": "no-such-frame", "guid": "guid-2", "url": "https://a.example/x", "suggestedFilename": "x.zip" }));
    b.fake.announce("Browser.downloadProgress", json!({ "guid": "guid-2", "totalBytes": 0, "receivedBytes": 3, "state": "canceled" }));
    let request = c.until("request").await;
    assert_eq!((request["download_id"].clone(), request["state"].clone(), request["tab_id"].clone()), (json!("guid-2"), json!("failed"), json!("")));
    assert_eq!(transfer::export(m, Some(TOKEN), "guid-2").await.unwrap_err().code, "no_such_download");
    // A download is nothing to answer: it is not shown again.
    drop(c);
    until("the viewer is gone", || m.viewers() == 0).await;
    let (mut c, _) = admitted(m, "thing", phone()).await;
    let (messages, _) = c.within(Duration::from_millis(300)).await;
    assert!(of_kind(&messages, "request").is_empty(), "{messages:?}");

    // The tab is closed: the files it was given go with it.
    m.close_tab(&tab.tab_id).await.unwrap();
    until("the uploaded files are deleted", || !files[0].exists()).await;
    m.shutdown().await;
}

/// SC6: a file of the Zone is read as the Desktop reads it — the file
/// service's `stat` and its data plane — with the caller's token. (An
/// nfs-server of its own, which checks no token: the wire is what is tested.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_zone_file_is_read_through_the_file_service() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::fs::write(home.join("Documents/报告 2026.pdf"), b"zone file bytes").unwrap();
    std::fs::write(home.join("top.txt"), b"top").unwrap();
    let config = nfs_server::ServerConfig::new(tmp.path().join("nfs"), vec![nfs_server::ExportRoot { id: "home".into(), path: home.clone() }]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::build_router(nfs_server::AppState::new(config).unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let dest = tmp.path().join("staged");
    let read = |zone_path: &'static str, limit: u64| {
        let (origin, dest) = (origin.clone(), dest.clone());
        async move { transfer::read_zone_file(&origin, Some(TOKEN), zone_path, &dest, limit).await }
    };
    assert_eq!(read("/home/Documents/报告 2026.pdf", 1024).await.unwrap(), 15);
    assert_eq!(std::fs::read(&dest).unwrap(), b"zone file bytes");
    assert_eq!(read("/home/top.txt", 3).await.unwrap(), 3);
    assert_eq!(std::fs::read(&dest).unwrap(), b"top");
    assert_eq!(read("/home/Documents/报告 2026.pdf", 14).await.unwrap_err().code, "upload_too_large");
    for not_a_file in ["/home/Documents/missing.pdf", "/home/Documents", "/nowhere/a.txt", "home/top.txt", "/home/../etc/passwd", "/home/Documents/"] {
        let error = read(not_a_file, 1024).await.unwrap_err();
        assert_eq!(error.code, "invalid_argument", "{not_a_file}: {error}");
    }
    // No file service there.
    let error = transfer::read_zone_file("http://127.0.0.1:9", Some(TOKEN), "/home/top.txt", &dest, 1024).await.unwrap_err();
    assert_eq!(error.code, "other", "{error}");
}

/// SC2: the browser dies and is started again with other tabs; the
/// connection shows them without being told to.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screen_goes_on_when_the_browser_was_started_again() {
    let b = Bench::new(|_| {}).await;
    let m = &b.module;
    let tab = m.open_tab("thing", Some("https://a.example/kept")).await.unwrap();
    m.tab_session(&tab.tab_id).await.unwrap();
    let (mut c, state) = admitted(m, "thing", phone()).await;
    assert_eq!(state["active_tab_id"], tab.tab_id.as_str());
    b.fake.tell(&tab.tab_id, "Page.fileChooserOpened", json!({ "frameId": tab.tab_id, "mode": "selectSingle", "backendNodeId": 5 }));
    let request = c.until("request").await["request_id"].as_str().unwrap().to_string();
    c.frames.clear();

    b.fake.crash();
    c.state("the browser is gone", |s| s["running"] == false && s["active_tab_id"].is_null()).await;
    let state = c.state("the browser runs again", |s| s["running"] == true && s["url"] == "https://a.example/kept").await;
    let again = state["active_tab_id"].as_str().unwrap().to_string();
    assert_ne!(again, tab.tab_id, "it is another tab");
    c.within(Duration::from_millis(500)).await;
    assert!(c.frames.iter().any(|(meta, data)| meta["tab_id"] == again.as_str() && data == b"first"), "the new tab's frames: {:?}", c.frames);
    // What the old page asked for is over.
    assert!(c.passed.contains(&json!({ "type": "request_closed", "request_id": request })), "{:?}", c.passed);
    assert!(m.outside.pending(m, "thing").is_empty());
    // The new tab is the device that looks.
    let (_, session) = m.tab_session(&again).await.unwrap();
    assert!(b.fake.received("Emulation.setDeviceMetricsOverride").iter().any(|(s, p)| s.as_deref() == Some(session.as_str()) && p["mobile"] == true));
    c.say(json!({ "type": "input", "kind": "text", "text": "still here" })).await;
    until("input reaches the new tab", || b.fake.received("Input.insertText").iter().any(|(s, _)| s.as_deref() == Some(session.as_str()))).await;
    m.shutdown().await;
}

// ---------------------------------------------------------------------------
// A real Chromium (`OPENDAN_CHROMIUM`)
// ---------------------------------------------------------------------------

mod real {
    use super::*;

    const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Screen test</title>
<style>
  body { margin: 0; font: 16px sans-serif; height: 3000px }
  #mode { position: absolute; left: 0; top: 0; width: 100%; height: 60px; background: #0066cc; color: #fff }
  @media (max-width: 600px) { #mode { background: #cc0000 } }
  .row { position: absolute; left: 10px; width: 300px; height: 40px; box-sizing: border-box }
</style>
<div id="mode">Screen test 屏幕测试</div>
<input id="text" class="row" style="top: 80px">
<select id="pick" class="row" style="top: 140px"><option value="red">Red</option><optgroup label="Cold"><option value="green">Green</option><option value="blue" disabled>Blue</option><option value="navy">Navy</option></optgroup></select>
<button id="alert" class="row" style="top: 200px" onclick="alert('Saved'); window.after = 'alert'">alert</button>
<button id="confirm" class="row" style="top: 260px" onclick="window.confirmed = confirm('Sure?')">confirm</button>
<button id="prompt" class="row" style="top: 320px" onclick="window.prompted = prompt('Name?', 'nobody')">prompt</button>
<input id="file" type="file" class="row" style="top: 380px" accept=".txt,image/*" multiple>
<a id="download" class="row" style="top: 440px" href="/download.bin">download</a>
<button id="where" class="row" style="top: 500px" onclick="navigator.geolocation.getCurrentPosition(() => window.geo = 'granted', (e) => window.geo = 'denied ' + e.code)">where</button>
<script>
  window.events = [];
  const pick = document.getElementById('pick');
  for (const type of ['input', 'change']) pick.addEventListener(type, () => window.events.push(type + ':' + pick.value));
  for (const type of ['keydown', 'compositionstart', 'compositionend', 'click', 'touchstart', 'touchend', 'mousedown'])
    document.addEventListener(type, (e) => window.events.push(type + (e.key ? ':' + e.key : '') + (e.data ? ':' + e.data : '') + (e.target.id ? '@' + e.target.id : '')), true);
</script>"#;

    /// A site on an http origin: the test page, and a file that downloads.
    async fn site() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = [0u8; 8192];
                    let read = stream.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..read]).to_string();
                    let (head, body): (String, Vec<u8>) = if request.starts_with("GET /download.bin") {
                        ("Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"report.bin\"".into(), vec![7u8; 5000])
                    } else if request.starts_with("GET /plain") {
                        // No viewport of its own: a phone lays it out wider than its screen.
                        ("Content-Type: text/html; charset=utf-8".into(), b"<!doctype html><title>Plain</title><body style=\"margin: 0\"><p>plain</p>".to_vec())
                    } else {
                        ("Content-Type: text/html; charset=utf-8".into(), PAGE.as_bytes().to_vec())
                    };
                    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\n{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        format!("http://127.0.0.1:{port}/")
    }

    struct Real {
        _tmp: tempfile::TempDir,
        module: Arc<BrowserModule>,
        zone: Arc<FakeZone>,
        site: String,
    }

    async fn real(tune: impl FnOnce(&mut Tuning)) -> Real {
        logs();
        let chromium = std::env::var_os(process::ENV_CHROMIUM).map(PathBuf::from).expect("set OPENDAN_CHROMIUM to a Chromium executable");
        let tmp = tempfile::tempdir().unwrap();
        let zone = Arc::new(FakeZone::default());
        let mut tuning = Tuning { chromium: Some(chromium), dirs: Some(BrowserDirs::at(tmp.path().join("browser"))), zone: Some(zone.clone()), ..Default::default() };
        tune(&mut tuning);
        let cfg = BrowserSection { enabled: true, idle_stop_secs: 600, max_tabs: 12 };
        let module = BrowserModule::with_tuning(cfg, deps(&tmp.path().join("agent_root")), tuning);
        Real { _tmp: tmp, module, zone, site: site().await }
    }

    /// A screen of `group_id` that shows the test page. Returns the tab too.
    async fn showing(r: &Real, group_id: &str, viewport: Value) -> (Client<tokio::io::DuplexStream>, String) {
        let (mut c, _) = admitted(&r.module, group_id, viewport).await;
        c.say(json!({ "type": "nav", "action": "url", "url": r.site })).await;
        let state = c.state("the test page is loaded", |s| s["title"] == "Screen test" && s["loading"] == false).await;
        (c, state["active_tab_id"].as_str().unwrap().to_string())
    }

    async fn eval(m: &BrowserModule, tab_id: &str, expression: &str) -> Value {
        let params = json!({ "expression": expression, "returnByValue": true, "awaitPromise": true });
        let answer = m.tab_send(tab_id, "Runtime.evaluate", params).await.unwrap();
        answer["result"]["value"].clone()
    }

    /// Wait until `expression` is `expected` in the page.
    async fn page_has(m: &BrowserModule, tab_id: &str, expression: &str, expected: Value) {
        let started = Instant::now();
        loop {
            let value = eval(m, tab_id, expression).await;
            if value == expected {
                return;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "{expression} is {value}, not {expected}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// `(width, height)` of a JPEG, from its frame header.
    fn jpeg_size(jpeg: &[u8]) -> Option<(u64, u64)> {
        if jpeg.len() < 4 || jpeg[0] != 0xff || jpeg[1] != 0xd8 {
            return None;
        }
        let mut at = 2;
        while at + 9 < jpeg.len() && jpeg[at] == 0xff {
            let (marker, length) = (jpeg[at + 1], u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize);
            if (0xc0..=0xc2).contains(&marker) {
                return Some((u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]) as u64, u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]) as u64));
            }
            at += 2 + length;
        }
        None
    }

    /// `OPENDAN_TEST_FRAMES=<dir>`: frames are kept there to be looked at.
    fn keep(name: &str, jpeg: &[u8]) {
        if let Some(dir) = std::env::var_os("OPENDAN_TEST_FRAMES") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(Path::new(&dir).join(name), jpeg).unwrap();
        }
    }

    async fn tap<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(c: &mut Client<S>, x: f64, y: f64) {
        c.say(json!({ "type": "input", "kind": "touch", "phase": "start", "points": [{ "id": 0, "x": x, "y": y }] })).await;
        c.say(json!({ "type": "input", "kind": "touch", "phase": "end", "points": [] })).await;
    }

    async fn press<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(c: &mut Client<S>, key: &str, code: &str, modifiers: i64, text: &str) {
        for action in ["down", "up"] {
            let text = if action == "down" { text } else { "" };
            c.say(json!({ "type": "input", "kind": "key", "action": action, "key": key, "code": code, "modifiers": modifiers, "text": text })).await;
        }
    }

    async fn stop(r: &Real) {
        r.module.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn frames_show_the_page_as_the_device_that_looks() {
        let r = real(|_| {}).await;
        let m = &r.module;
        let (mut c, tab_id) = showing(&r, "thing", phone()).await;

        // A phone: its size, touch, a phone's user agent — from the first request on.
        page_has(m, &tab_id, "innerWidth + 'x' + innerHeight", json!("412x915")).await;
        assert_eq!(eval(m, &tab_id, "matchMedia('(max-width: 600px)').matches && 'ontouchstart' in window && navigator.maxTouchPoints > 0").await, true);
        let agent = eval(m, &tab_id, "navigator.userAgent").await;
        assert!(agent.as_str().unwrap().contains("Android") && agent.as_str().unwrap().contains("Mobile"), "{agent}");
        assert_eq!(eval(m, &tab_id, "devicePixelRatio").await, 2);
        // Frames are JPEGs of the viewport, in CSS pixels.
        c.within(Duration::from_millis(1500)).await;
        let (meta, jpeg) = c.frames.last().cloned().expect("frames arrive");
        assert_eq!((meta["tab_id"].as_str(), meta["width"].as_u64(), meta["height"].as_u64()), (Some(tab_id.as_str()), Some(412), Some(915)));
        assert_eq!(jpeg_size(&jpeg), Some((412, 915)), "a JPEG of the viewport's size");
        assert!(jpeg.ends_with(&[0xff, 0xd9]), "a whole JPEG");
        keep("phone.jpg", &jpeg);
        // A page that does not change paints no further frame.
        let (_, frames) = c.within(Duration::from_millis(700)).await;
        assert!(frames <= 1, "a still page sends no stream of frames: {frames}");
        // Something changes: a frame shows it.
        let before = c.frames.len();
        eval(m, &tab_id, "document.getElementById('mode').textContent = 'changed 已改变'").await;
        c.within(Duration::from_millis(700)).await;
        assert!(c.frames.len() > before, "the change is shown");
        keep("phone-changed.jpg", &c.frames.last().unwrap().1);

        // The desktop version: the page is asked for again as a desktop browser.
        c.frames.clear();
        c.say(json!({ "type": "viewport", "width": 1280, "height": 800, "mobile": false })).await;
        c.state("the viewport is the desktop's", |s| s["viewport"]["width"] == 1280).await;
        page_has(m, &tab_id, "innerWidth + 'x' + innerHeight + ' ' + matchMedia('(max-width: 600px)').matches", json!("1280x800 false")).await;
        page_has(m, &tab_id, "navigator.userAgent.includes('Mobile') || 'ontouchstart' in window", json!(false)).await;
        c.within(Duration::from_millis(1500)).await;
        let (meta, jpeg) = c.frames.last().cloned().expect("frames arrive");
        assert_eq!((meta["width"].as_u64(), meta["height"].as_u64(), jpeg_size(&jpeg)), (Some(1280), Some(800), Some((1280, 800))));
        keep("desktop.jpg", &jpeg);

        // A second tab gets the viewport when it is shown.
        c.say(json!({ "type": "tab", "action": "open", "url": r.site })).await;
        let state = c.state("the second tab is loaded", |s| s["title"] == "Screen test" && s["loading"] == false && s["active_tab_id"] != tab_id.as_str()).await;
        let second = state["active_tab_id"].as_str().unwrap().to_string();
        page_has(m, &second, "innerWidth", json!(1280)).await;
        c.frames.clear();
        c.within(Duration::from_millis(1000)).await;
        assert!(c.frames.iter().all(|(meta, _)| meta["tab_id"] == second.as_str()) && !c.frames.is_empty(), "the frames are the shown tab's");
        stop(&r).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn touches_keys_and_text_reach_the_page() {
        let r = real(|_| {}).await;
        let m = &r.module;
        let (mut c, tab_id) = showing(&r, "thing", phone()).await;
        let value = "document.getElementById('text').value";

        // A tap on the text field: it has the focus, and the App is told to show its keyboard.
        tap(&mut c, 100.0, 100.0).await;
        let focus = c.until("focus").await;
        assert_eq!((focus["tab_id"].as_str(), focus["editable"].as_bool(), focus["input_type"].as_str()), (Some(tab_id.as_str()), Some(true), Some("text")));
        assert_eq!(focus["rect"], json!({ "x": 10, "y": 80, "width": 300, "height": 40 }));
        page_has(m, &tab_id, "document.activeElement.id", json!("text")).await;
        let events = eval(m, &tab_id, "window.events.join(' ')").await;
        assert!(events.as_str().unwrap().contains("touchstart@text touchend@text") && events.as_str().unwrap().contains("click@text"), "{events}");

        // Text as a whole, composition, the committed text.
        c.say(json!({ "type": "input", "kind": "text", "text": "你好" })).await;
        page_has(m, &tab_id, value, json!("你好")).await;
        c.say(json!({ "type": "input", "kind": "composition", "text": "zhong", "selection_start": 5, "selection_end": 5 })).await;
        page_has(m, &tab_id, value, json!("你好zhong")).await;
        c.say(json!({ "type": "input", "kind": "text", "text": "中" })).await;
        page_has(m, &tab_id, value, json!("你好中")).await;
        // A composition that is cancelled leaves nothing.
        c.say(json!({ "type": "input", "kind": "composition", "text": "wen", "selection_start": 3, "selection_end": 3 })).await;
        page_has(m, &tab_id, value, json!("你好中wen")).await;
        c.say(json!({ "type": "input", "kind": "composition", "text": "", "selection_start": 0, "selection_end": 0 })).await;
        page_has(m, &tab_id, value, json!("你好中")).await;
        let events = eval(m, &tab_id, "window.events.join(' ')").await;
        assert!(events.as_str().unwrap().contains("compositionstart") && events.as_str().unwrap().contains("compositionend"), "{events}");

        // Keys: a character, Backspace, arrows, a shortcut.
        press(&mut c, "a", "KeyA", 0, "a").await;
        press(&mut c, "B", "KeyB", 8, "B").await;
        page_has(m, &tab_id, value, json!("你好中aB")).await;
        press(&mut c, "Backspace", "Backspace", 0, "").await;
        page_has(m, &tab_id, value, json!("你好中a")).await;
        press(&mut c, "Home", "Home", 0, "").await;
        press(&mut c, "Delete", "Delete", 0, "").await;
        page_has(m, &tab_id, value, json!("好中a")).await;
        press(&mut c, "End", "End", 0, "").await;
        press(&mut c, "ArrowLeft", "ArrowLeft", 0, "").await;
        press(&mut c, "x", "", 0, "x").await;
        page_has(m, &tab_id, value, json!("好中xa")).await;
        press(&mut c, "a", "KeyA", 2, "").await;
        press(&mut c, "Delete", "Delete", 0, "").await;
        page_has(m, &tab_id, value, json!("")).await;
        press(&mut c, "Enter", "Enter", 0, "").await;
        press(&mut c, "Tab", "Tab", 0, "").await;
        page_has(m, &tab_id, "document.activeElement.id", json!("pick")).await;
        let events = eval(m, &tab_id, "window.events.join(' ')").await;
        for expected in ["keydown:a@text", "keydown:Backspace@text", "keydown:Enter@text", "keydown:Tab@text"] {
            assert!(events.as_str().unwrap().contains(expected), "{expected} in {events}");
        }
        // The focus left the text field.
        let left = c.until("focus").await;
        assert_eq!(left, json!({ "type": "focus", "tab_id": tab_id, "editable": false }));

        // The mouse and the wheel (the desktop version sends these for a touch).
        c.say(json!({ "type": "input", "kind": "mouse", "action": "move", "x": 100, "y": 100, "button": "none", "click_count": 0, "modifiers": 0 })).await;
        for action in ["down", "up"] {
            c.say(json!({ "type": "input", "kind": "mouse", "action": action, "x": 100, "y": 100, "button": "left", "click_count": 1, "modifiers": 0 })).await;
        }
        page_has(m, &tab_id, "document.activeElement.id + ' ' + window.events.includes('mousedown@text')", json!("text true")).await;
        c.say(json!({ "type": "input", "kind": "wheel", "x": 200, "y": 600, "delta_x": 0, "delta_y": 400, "modifiers": 0 })).await;
        page_has(m, &tab_id, "scrollY > 0", json!(true)).await;
        // A drag of the finger scrolls too.
        eval(m, &tab_id, "scrollTo(0, 0)").await;
        c.say(json!({ "type": "input", "kind": "touch", "phase": "start", "points": [{ "id": 0, "x": 200, "y": 700 }] })).await;
        for y in [650, 600, 550, 500, 450, 400] {
            c.say(json!({ "type": "input", "kind": "touch", "phase": "move", "points": [{ "id": 0, "x": 200, "y": y }] })).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        c.say(json!({ "type": "input", "kind": "touch", "phase": "end", "points": [] })).await;
        page_has(m, &tab_id, "scrollY > 100", json!(true)).await;
        stop(&r).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn selects_dialogs_and_permissions_are_the_screens() {
        let r = real(|t| t.screen.dialog = Duration::from_secs(3)).await;
        let m = &r.module;
        let (mut c, tab_id) = showing(&r, "thing", phone()).await;

        // ---- <select>: its list is the App's to show.
        tap(&mut c, 100.0, 160.0).await;
        let request = c.until("request").await;
        assert_eq!((request["kind"].as_str(), request["tab_id"].as_str(), request["multiple"].as_bool()), (Some("select"), Some(tab_id.as_str()), Some(false)));
        assert_eq!(
            request["options"],
            json!([
                { "index": 0, "label": "Red", "selected": true, "disabled": false },
                { "index": 1, "label": "Green", "selected": false, "disabled": false, "group": "Cold" },
                { "index": 2, "label": "Blue", "selected": false, "disabled": true, "group": "Cold" },
                { "index": 3, "label": "Navy", "selected": false, "disabled": false, "group": "Cold" },
            ])
        );
        assert_eq!(request["rect"], json!({ "x": 10, "y": 140, "width": 300, "height": 40 }));
        c.say(json!({ "type": "respond", "request_id": request["request_id"], "result": { "selected": [3] } })).await;
        assert_eq!(c.until("request_closed").await["request_id"], request["request_id"]);
        page_has(m, &tab_id, "document.getElementById('pick').value + ' ' + window.events.filter((e) => e.includes(':navy')).join(' ')", json!("navy input:navy change:navy")).await;
        // Cancelled: the page keeps its value.
        tap(&mut c, 100.0, 160.0).await;
        let request = c.until("request").await;
        assert_eq!(request["options"][3]["selected"], true);
        c.say(json!({ "type": "respond", "request_id": request["request_id"], "result": { "cancel": true } })).await;
        c.until("request_closed").await;
        page_has(m, &tab_id, "document.getElementById('pick').value", json!("navy")).await;

        // ---- JS dialogs. The tap that opens one is not waited for: the connection goes on.
        tap(&mut c, 100.0, 220.0).await;
        let request = c.until("request").await;
        assert_eq!((request["kind"].as_str(), request["dialog_type"].as_str(), request["message"].as_str()), (Some("dialog"), Some("alert"), Some("Saved")));
        c.say(json!({ "type": "ping" })).await;
        c.until("pong").await;
        c.say(json!({ "type": "respond", "request_id": request["request_id"], "result": { "accept": true } })).await;
        c.until("request_closed").await;
        page_has(m, &tab_id, "window.after", json!("alert")).await;
        tap(&mut c, 100.0, 280.0).await;
        let request = c.until("request").await;
        assert_eq!((request["dialog_type"].as_str(), request["message"].as_str()), (Some("confirm"), Some("Sure?")));
        c.say(json!({ "type": "respond", "request_id": request["request_id"], "result": { "accept": true } })).await;
        page_has(m, &tab_id, "window.confirmed", json!(true)).await;
        tap(&mut c, 100.0, 340.0).await;
        let request = c.until("request").await;
        assert_eq!((request["dialog_type"].as_str(), request["message"].as_str(), request["default_prompt"].as_str()), (Some("prompt"), Some("Name?"), Some("nobody")));
        c.say(json!({ "type": "respond", "request_id": request["request_id"], "result": { "accept": true, "prompt_text": "鲍勃" } })).await;
        page_has(m, &tab_id, "window.prompted", json!("鲍勃")).await;
        // Not answered: dismissed after the wait.
        c.passed.clear();
        let asked = Instant::now();
        tap(&mut c, 100.0, 280.0).await;
        let request = c.until("request").await;
        assert_eq!(c.until("request_closed").await["request_id"], request["request_id"]);
        assert!(asked.elapsed() >= Duration::from_millis(2900), "{:?}", asked.elapsed());
        page_has(m, &tab_id, "window.confirmed", json!(false)).await;

        // ---- a permission: refused, and the user is told it was asked for.
        tap(&mut c, 100.0, 520.0).await;
        let request = c.until("request").await;
        assert_eq!((request["kind"].as_str(), request["permission"].as_str()), (Some("permission"), Some("geolocation")));
        assert_eq!(request["origin"].as_str(), Some(r.site.trim_end_matches('/')));
        page_has(m, &tab_id, "window.geo", json!("denied 1")).await;
        assert_eq!(eval(m, &tab_id, "navigator.permissions.query({ name: 'geolocation' }).then((s) => s.state)").await, "denied");
        assert_eq!(eval(m, &tab_id, "typeof window.__opendanScreen + ' ' + typeof window.__opendanSelect").await, "undefined undefined", "the page does not see the screen's script");
        stop(&r).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn files_are_uploaded_and_downloaded() {
        let r = real(|_| {}).await;
        let m = &r.module;
        let (mut c, tab_id) = showing(&r, "thing", phone()).await;

        // The page's file input: the chooser is the App's.
        tap(&mut c, 60.0, 400.0).await;
        let request = c.until("request").await;
        assert_eq!((request["kind"].as_str(), request["multiple"].as_bool(), request["accept"].clone()), (Some("file_chooser"), Some(true), json!([".txt", "image/*"])));
        let request_id = request["request_id"].as_str().unwrap().to_string();
        r.zone.files.lock().unwrap().insert("/home/Documents/笔记.txt".into(), "从 Zone 来的".as_bytes().to_vec());
        transfer::receive(m, &request_id, "local.txt", false, body(b"from the device")).await.unwrap();
        transfer::upload(m, Some(TOKEN), &request_id, "/home/Documents/笔记.txt", true).await.unwrap();
        assert_eq!(c.until("request_closed").await["request_id"], request_id.as_str());
        let files = "Promise.all([...document.getElementById('file').files].map(async (f) => f.name + ':' + await f.text())).then((all) => all.join(' | '))";
        page_has(m, &tab_id, files, json!("local.txt:from the device | 笔记.txt:从 Zone 来的")).await;
        // Cancelled: the input keeps what it has.
        tap(&mut c, 60.0, 400.0).await;
        let request = c.until("request").await;
        c.say(json!({ "type": "respond", "request_id": request["request_id"], "result": { "cancel": true } })).await;
        c.until("request_closed").await;
        assert_eq!(eval(m, &tab_id, "document.getElementById('file').files.length").await, 2);

        // A download: kept in the browser's download directory, told when it is complete.
        tap(&mut c, 30.0, 455.0).await;
        let request = c.until("request").await;
        assert_eq!(
            (request["kind"].as_str(), request["name"].as_str(), request["size"].as_u64(), request["state"].as_str(), request["tab_id"].as_str()),
            (Some("download"), Some("report.bin"), Some(5000), Some("completed"), Some(tab_id.as_str()))
        );
        let download_id = request["download_id"].as_str().unwrap();
        assert_eq!(std::fs::read(m.dirs().downloads.join(download_id)).unwrap(), vec![7u8; 5000]);
        let exported = transfer::export(m, Some(TOKEN), download_id).await.unwrap();
        assert_eq!(exported, json!({ "obj_id": "cyfile:0123456789abcdef", "name": "report.bin", "size": 5000 }));
        assert_eq!(r.zone.exported.lock().unwrap()[0].1, m.dirs().downloads.join(download_id));
        stop(&r).await;
    }

    /// Processes whose command line names this profile.
    fn processes_of(profile: &Path) -> Vec<u32> {
        let needle = profile.display().to_string();
        std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| String::from_utf8_lossy(&c).contains(&needle)))
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn the_agent_and_the_user_take_turns_and_a_killed_browser_comes_back() {
        let r = real(|_| {}).await;
        let m = &r.module;
        let (mut c, tab_id) = showing(&r, DEFAULT_GROUP, phone()).await;
        let value = "document.getElementById('text').value";
        tap(&mut c, 100.0, 100.0).await;
        c.until("focus").await;
        c.say(json!({ "type": "input", "kind": "text", "text": "mine" })).await;
        page_has(m, &tab_id, value, json!("mine")).await;

        // The agent holds a lease: the page is the agent's, the user only sees it.
        let lease = m.acquire("s1").await.unwrap();
        c.state("the agent operates", |s| s["group"]["controller"] == "agent").await;
        c.say(json!({ "type": "input", "kind": "text", "text": " dropped" })).await;
        assert_eq!(c.until("error").await["code"], "agent_in_control");
        m.tab_send(&tab_id, "Input.insertText", json!({ "text": " agent's" })).await.unwrap();
        page_has(m, &tab_id, value, json!("mine agent's")).await;
        c.frames.clear();
        c.within(Duration::from_millis(700)).await;
        assert!(!c.frames.is_empty(), "the user sees what the agent does");
        // Taken over: the user's again, and the agent gets no lease.
        m.hand(OWNER, DEFAULT_GROUP, Handed::TakeOver).await.unwrap();
        c.state("the user operates", |s| s["group"]["controller"] == "user" && s["group"]["user_hold"] == true).await;
        assert_eq!(m.renew(&lease.lease_id).unwrap_err().code, "lease_lost");
        c.say(json!({ "type": "input", "kind": "text", "text": " again" })).await;
        page_has(m, &tab_id, value, json!("mine agent's again")).await;

        // The browser is killed: the connection shows the tab it comes back with.
        let before = processes_of(&m.dirs().profile);
        let main = *before.iter().min().unwrap();
        assert_eq!(std::process::Command::new("kill").args(["-KILL", &main.to_string()]).status().unwrap().code(), Some(0));
        c.state("the browser is gone", |s| s["running"] == false).await;
        let state = c.state("the browser runs again", |s| s["running"] == true && s["title"] == "Screen test" && s["loading"] == false).await;
        let again = state["active_tab_id"].as_str().unwrap().to_string();
        assert_ne!(again, tab_id);
        page_has(m, &again, "innerWidth", json!(412)).await;
        c.frames.clear();
        c.within(Duration::from_millis(1500)).await;
        let (meta, jpeg) = c.frames.last().cloned().expect("frames of the new tab arrive");
        assert_eq!((meta["tab_id"].as_str(), jpeg_size(&jpeg)), (Some(again.as_str()), Some((412, 915))));
        tap(&mut c, 100.0, 100.0).await;
        c.until("focus").await;
        c.say(json!({ "type": "input", "kind": "text", "text": "after" })).await;
        page_has(m, &again, value, json!("after")).await;
        stop(&r).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn two_screens_of_one_tab_see_the_viewport_set_last() {
        let r = real(|_| {}).await;
        let m = &r.module;
        let desktop = json!({ "width": 1078, "height": 576, "mobile": false });
        let small = json!({ "width": 412, "height": 780, "mobile": true });
        let layout = "innerWidth + 'x' + innerHeight + ' ' + matchMedia('(max-width: 600px)').matches + ' ' + navigator.maxTouchPoints";
        let (mut wide, tab_id) = showing(&r, "thing", desktop.clone()).await;
        page_has(m, &tab_id, layout, json!("1078x576 false 0")).await;
        eval(m, &tab_id, "window.kept = 'since the first load'").await;

        // A phone looks at the same tab: the page is laid out for it, and the desktop is told whose viewport it sees.
        let (mut phone, state) = admitted(m, "thing", small.clone()).await;
        assert_eq!((state["viewport"].clone(), state["active_tab_id"].as_str()), (small.clone(), Some(tab_id.as_str())));
        wide.state("the desktop sees the phone's viewport", |s| s["viewport"] == small).await;
        page_has(m, &tab_id, layout, json!("412x780 true 5")).await;
        wide.frames.clear();
        wide.within(Duration::from_millis(1500)).await;
        phone.within(Duration::from_millis(500)).await;
        for frames in [&wide.frames, &phone.frames] {
            let (meta, jpeg) = frames.last().expect("frames arrive");
            assert_eq!((meta["width"].as_u64(), meta["height"].as_u64(), jpeg_size(jpeg)), (Some(412), Some(780), Some((412, 780))));
        }

        // The phone is gone: the tab is the desktop's again and its page is laid out for it, not loaded again.
        wide.frames.clear();
        drop(phone);
        wide.state("the desktop has its own viewport again", |s| s["viewport"] == desktop).await;
        page_has(m, &tab_id, layout, json!("1078x576 false 0")).await;
        wide.within(Duration::from_millis(1500)).await;
        let (meta, jpeg) = wide.frames.last().cloned().expect("frames arrive");
        assert_eq!((meta["width"].as_u64(), meta["height"].as_u64(), jpeg_size(&jpeg)), (Some(1078), Some(576), Some((1078, 576))));
        keep("desktop-restored.jpg", &jpeg);
        assert_eq!(eval(m, &tab_id, "window.kept").await, "since the first load");
        stop(&r).await;
    }

    /// A screenshot of the viewport taken over another session of the tab,
    /// the way the agent's Playwright takes one.
    async fn screenshot_by_the_agent(m: &BrowserModule, tab_id: &str) {
        let cdp = m.cdp().unwrap();
        let attached = cdp.send(None, "Target.attachToTarget", json!({ "targetId": tab_id, "flatten": true })).await.unwrap();
        let other = attached["sessionId"].as_str().unwrap();
        let view = cdp.send(Some(other), "Page.getLayoutMetrics", json!({})).await.unwrap()["cssVisualViewport"].clone();
        let clip = json!({ "x": view["pageX"], "y": view["pageY"], "width": view["clientWidth"], "height": view["clientHeight"], "scale": 1 });
        let shot = cdp.send(Some(other), "Page.captureScreenshot", json!({ "format": "png", "clip": clip, "captureBeyondViewport": false })).await.unwrap();
        assert!(shot["data"].as_str().is_some_and(|data| !data.is_empty()));
        cdp.send(None, "Target.detachFromTarget", json!({ "sessionId": other })).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn a_tab_is_the_screens_device_again_when_the_agent_is_done() {
        let r = real(|_| {}).await;
        let m = &r.module;
        let (mut c, tab_id) = showing(&r, DEFAULT_GROUP, json!({ "width": 412, "height": 780, "mobile": true })).await;
        let device = "devicePixelRatio + ' ' + screen.width + 'x' + screen.height + ' ' + innerWidth";
        page_has(m, &tab_id, device, json!("2 412x780 412")).await;
        eval(m, &tab_id, "window.kept = 'since the first load'").await;

        // The agent takes a screenshot: to the page, the tab is no phone any more.
        let lease = m.acquire("s1").await.unwrap();
        c.state("the agent operates", |s| s["group"]["controller"] == "agent").await;
        screenshot_by_the_agent(m, &tab_id).await;
        page_has(m, &tab_id, device, json!("1 800x600 412")).await;
        // The agent is done: the tab is the phone again, its page not loaded again.
        m.release(&lease.lease_id, Some(AgentState::Done), None).unwrap();
        c.state("the user operates", |s| s["group"]["controller"] == "user").await;
        page_has(m, &tab_id, device, json!("2 412x780 412")).await;
        assert_eq!(eval(m, &tab_id, "window.kept").await, "since the first load");

        // A page without a viewport of its own, which a phone lays out wider than its screen; this time
        // the user takes the browser from the agent.
        c.say(json!({ "type": "nav", "action": "url", "url": format!("{}plain", r.site) })).await;
        c.state("the plain page is loaded", |s| s["title"] == "Plain" && s["loading"] == false).await;
        page_has(m, &tab_id, device, json!("2 412x780 981")).await;
        m.acquire("s1").await.unwrap();
        c.state("the agent operates", |s| s["group"]["controller"] == "agent").await;
        screenshot_by_the_agent(m, &tab_id).await;
        page_has(m, &tab_id, device, json!("1 800x600 412")).await;
        c.frames.clear();
        m.hand(OWNER, DEFAULT_GROUP, Handed::TakeOver).await.unwrap();
        c.state("the user operates", |s| s["group"]["controller"] == "user" && s["group"]["user_hold"] == true).await;
        page_has(m, &tab_id, device, json!("2 412x780 981")).await;
        c.within(Duration::from_millis(1500)).await;
        let (meta, jpeg) = c.frames.last().cloned().expect("frames arrive");
        assert_eq!((meta["width"].as_u64(), meta["height"].as_u64(), jpeg_size(&jpeg)), (Some(412), Some(780), Some((412, 780))));
        stop(&r).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn the_caret_of_a_focused_field_paints_no_frames() {
        let r = real(|_| {}).await;
        let m = &r.module;
        let (mut c, tab_id) = showing(&r, "thing", json!({ "width": 412, "height": 780, "mobile": true })).await;
        tap(&mut c, 100.0, 100.0).await;
        c.until("focus").await;
        page_has(m, &tab_id, "document.activeElement.id", json!("text")).await;
        c.within(Duration::from_millis(1000)).await;
        let lit = c.frames.last().cloned().expect("the focused field is shown").1;
        keep("caret.jpg", &lit);

        // Nothing happens: nothing is painted (a blinking caret paints two frames a second).
        let (_, frames) = c.within(Duration::from_secs(5)).await;
        assert!(frames <= 1, "{frames} frames in 5 seconds of a focused field");
        // The caret is lit, not gone: without it the field looks different.
        eval(m, &tab_id, "document.getElementById('text').style.caretColor = 'transparent'").await;
        c.within(Duration::from_millis(700)).await;
        let unlit = c.frames.last().cloned().unwrap().1;
        assert!(unlit != lit, "a caret was drawn");
        eval(m, &tab_id, "document.getElementById('text').style.caretColor = ''").await;
        c.within(Duration::from_millis(700)).await;
        assert!(c.frames.last().unwrap().1 == lit, "and is drawn again");
        stop(&r).await;
    }
}
