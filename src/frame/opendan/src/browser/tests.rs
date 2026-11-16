//! The module against a fake browser (`tests/fake_browser`), and — ignored,
//! run with `OPENDAN_CHROMIUM=<chrome> cargo test -p opendan --lib browser::tests::real -- --ignored --test-threads=1`
//! — against a real Chromium.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;
use crate::config::BrowserSection;

#[path = "../../tests/fake_browser/mod.rs"]
mod fake_browser;
use fake_browser::FakeBrowser;

const AGENT: &str = "did:bns:jarvis.alice";
const OWNER: &str = "did:bns:bob";

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

fn alive(pid: i32) -> bool {
    // A zombie is not alive: nothing of it runs any more.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit_once(')').is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z')))
}

fn section(max_tabs: usize, idle_stop_secs: u64) -> BrowserSection {
    BrowserSection { enabled: true, idle_stop_secs, max_tabs, display: crate::browser::display::DisplayMode::Headless, ..Default::default() }
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
    root: PathBuf,
    fake: Arc<FakeBrowser>,
    module: Arc<BrowserModule>,
}

impl Bench {
    /// A module whose browser is the fake one.
    async fn new(cfg: BrowserSection, tune: impl FnOnce(&mut Tuning)) -> Self {
        logs();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("agent_root");
        let dirs = BrowserDirs::at(tmp.path().join("browser"));
        let fake = FakeBrowser::start(&dirs.root).await;
        let mut tuning = Tuning { chromium: Some(fake_browser::chromium()), dirs: Some(dirs), ..Default::default() };
        tune(&mut tuning);
        let module = BrowserModule::with_tuning(cfg, deps(&root), tuning);
        Self { _tmp: tmp, root, fake, module }
    }

    fn tabs(&self, group_id: &str) -> Vec<Tab> {
        self.module.group(group_id).tabs
    }
}

/// The CDP client by itself against the fake endpoint: answers find their
/// requests, the browser's refusals are errors, events reach the
/// subscribers they are for, and a closed connection ends everything.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cdp_client_pairs_answers_and_broadcasts_events() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = FakeBrowser::start(tmp.path()).await;
    let cdp = CdpClient::connect(&format!("ws://127.0.0.1:{}/devtools/browser/fake", fake.port)).await.unwrap();
    assert!(!cdp.is_closed());

    // Requests sent at once get their own answers.
    let mut browser_events = cdp.subscribe(Scope::Browser, &["Target.targetCreated"]);
    let (version, first, second) = tokio::join!(
        cdp.send(None, "Browser.getVersion", json!({})),
        cdp.send(None, "Target.createTarget", json!({ "url": "https://a.example/" })),
        cdp.send(None, "Target.createTarget", json!({ "url": "https://b.example/" })),
    );
    assert_eq!(version.unwrap()["product"], "Chrome/1.2.3.4");
    let (first, second) = (first.unwrap()["targetId"].as_str().unwrap().to_string(), second.unwrap()["targetId"].as_str().unwrap().to_string());
    assert_ne!(first, second);
    let created: Vec<String> = [browser_events.next().await.unwrap(), browser_events.next().await.unwrap()]
        .iter()
        .map(|e| e.params["targetInfo"]["targetId"].as_str().unwrap().to_string())
        .collect();
    assert!(created.contains(&first) && created.contains(&second));
    // What the browser refuses is an error with its words.
    let refused = cdp.send(None, "Target.attachToTarget", json!({ "targetId": "nope", "flatten": true })).await.unwrap_err();
    assert_eq!(refused, CdpError::Remote { code: -32000, message: "No target with given id found".into() });

    // Flat sessions: a tab's events go to the subscribers of that session, by method.
    let attach = |target: String| {
        let cdp = cdp.clone();
        async move { cdp.send(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await.unwrap()["sessionId"].as_str().unwrap().to_string() }
    };
    let (one, other) = (attach(first.clone()).await, attach(second.clone()).await);
    let mut of_one = cdp.subscribe(Scope::Session(one.clone()), &[]);
    let mut stops_of_other = cdp.subscribe(Scope::Session(other.clone()), &["Page.frameStoppedLoading"]);
    let mut everything = cdp.subscribe(Scope::Any, &["Page.frameStartedLoading"]);
    cdp.send(Some(&other), "Page.navigate", json!({ "url": "https://b.example/next" })).await.unwrap();
    // Not waited for: the answer of a command that opens a dialog only comes when the dialog is closed.
    cdp.post(Some(&one), "Page.navigate", json!({ "url": "https://a.example/next" })).unwrap();
    let stop = stops_of_other.next().await.unwrap();
    assert_eq!((stop.method.as_str(), stop.session_id.as_deref()), ("Page.frameStoppedLoading", Some(other.as_str())));
    let seen: Vec<String> = [of_one.next().await.unwrap(), of_one.next().await.unwrap()].iter().map(|e| e.method.clone()).collect();
    assert_eq!(seen, vec!["Page.frameStartedLoading", "Page.frameStoppedLoading"]);
    let starts: Vec<Option<String>> = [everything.next().await.unwrap(), everything.next().await.unwrap()].iter().map(|e| e.session_id.clone()).collect();
    assert_eq!(starts, vec![Some(other.clone()), Some(one.clone())]);
    // A command that is not answered in time is given up.
    let late = cdp.send_within(None, "Fake.never", json!({}), Duration::from_millis(200)).await.unwrap_err();
    assert_eq!(late, CdpError::Timeout("Fake.never".into()));

    // The browser goes away: waiting requests and subscribers end, later calls fail at once.
    let waiting = {
        let cdp = cdp.clone();
        tokio::spawn(async move { cdp.send(None, "Fake.never", json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    fake.crash();
    assert_eq!(waiting.await.unwrap().unwrap_err(), CdpError::Closed);
    cdp.closed().await;
    assert!(cdp.is_closed());
    assert!(of_one.next().await.is_none());
    assert_eq!(cdp.send(None, "Browser.getVersion", json!({})).await.unwrap_err(), CdpError::Closed);
    assert_eq!(cdp.post(None, "Browser.close", json!({})).unwrap_err(), CdpError::Closed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_browser_starts_on_demand_and_ends_without_leaving_processes() {
    let b = Bench::new(section(12, 600), |_| {}).await;
    let status = b.module.status().await;
    assert_eq!((status["enabled"].clone(), status["available"].clone(), status["running"].clone()), (json!(true), json!(true), json!(false)));
    assert_eq!(status["groups"].as_array().unwrap().len(), 1, "the default group is always there");
    assert!(b.fake.pid().is_none(), "status starts nothing");

    let cdp = b.module.ensure().await.unwrap();
    assert!(b.module.running() && !cdp.is_closed());
    let args = b.fake.args();
    for expected in [
        "--headless=new",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-dev-shm-usage",
        "--no-startup-window",
    ] {
        assert!(args.contains(expected), "{expected} in {args}");
    }
    // A port of its own, given to it: never 0.
    assert!(args.contains("--remote-debugging-port=") && !args.contains("--remote-debugging-port=0 "), "{args}");
    assert!(args.contains(&format!("--user-data-dir={}", b.module.dirs().profile.display())), "{args}");
    assert!(args.contains(&format!("--disk-cache-dir={}", b.module.dirs().cache.display())), "{args}");
    assert_eq!(args.contains("--no-sandbox"), unsafe { libc::geteuid() } == 0, "{args}");
    let downloads = b.fake.received("Browser.setDownloadBehavior");
    assert_eq!(downloads[0].1["downloadPath"], json!(b.module.dirs().downloads.display().to_string()));
    let status = b.module.status().await;
    assert_eq!(status["running"], true);
    assert_eq!(status["version"]["browser"], "1.2.3.4");
    assert_eq!(status["unavailable"], Value::Null);
    // Asking again starts nothing new.
    let pid = b.fake.pid().unwrap();
    b.module.ensure().await.unwrap();
    assert_eq!(b.fake.pid(), Some(pid));

    b.module.shutdown().await;
    assert_eq!(b.fake.received("Browser.close").len(), 1, "it is asked to close first");
    assert!(!alive(pid) && process::group_pids(pid as u32).is_empty(), "nothing of the browser is left");
    assert!(!b.module.running() && cdp.is_closed());
    assert_eq!(b.module.ensure().await.unwrap_err().code, "browser_unavailable", "a service that ends starts no browser");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_that_does_not_close_is_ended_with_its_process_group() {
    let b = Bench::new(section(12, 600), |_| {}).await;
    b.fake.flag("fake_ignores_close", true);
    b.fake.flag("fake_ignores_term", true);
    b.module.ensure().await.unwrap();
    let pid = b.fake.pid().unwrap();
    until("the script's sleep is in its group", || process::group_pids(pid as u32).len() >= 2).await;
    let started = Instant::now();
    b.module.shutdown().await;
    assert!(started.elapsed() >= Duration::from_secs(5), "it was given the time to close and to end");
    assert!(!alive(pid));
    assert!(process::group_pids(pid as u32).is_empty(), "the group is killed and collected");
}

/// Chromium's crash handlers leave its process group and its parent: they
/// are found by what the browser's processes carry in their environment,
/// and end (and are collected) with it — stopped, or dead.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn helpers_that_left_the_browsers_process_group_end_with_it() {
    let b = Bench::new(section(12, 600), |_| {}).await;
    b.fake.flag("fake_stray", true);
    let there = |pid: i32| std::path::Path::new(&format!("/proc/{pid}")).exists();

    // Stopped.
    b.module.ensure().await.unwrap();
    let (pid, stray) = (b.fake.pid().unwrap(), b.fake.stray_pid().unwrap());
    assert!(alive(stray) && !process::group_pids(pid as u32).contains(&(stray as u32)), "the helper is outside the group");
    b.module.shutdown().await;
    until("the helper is gone, not left as a zombie", || !there(stray)).await;
    assert!(!there(pid));

    // Dead: a new service on the same directories, whose browser is killed.
    let tuning = Tuning { chromium: Some(fake_browser::chromium()), dirs: Some(b.module.dirs().clone()), ..Default::default() };
    let again = BrowserModule::with_tuning(section(12, 600), deps(&b.root), tuning);
    again.ensure().await.unwrap();
    let (pid, stray) = (b.fake.pid().unwrap(), b.fake.stray_pid().unwrap());
    assert!(alive(stray));
    b.fake.crash();
    until("the helper of the dead browser is gone", || !there(stray)).await;
    until("the browser is started again", || again.running() && b.fake.pid() != Some(pid)).await;
    again.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_usable_sandbox_the_browser_is_started_once_more_without_it() {
    let b = Bench::new(section(12, 600), |_| {}).await;
    b.fake.flag("fake_needs_no_sandbox", true);
    b.module.ensure().await.unwrap();
    assert!(b.fake.args().contains("--no-sandbox"));
    b.module.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tabs_belong_to_groups() {
    let b = Bench::new(section(4, 600), |_| {}).await;
    let m = &b.module;
    let mut changes = m.changes();
    changes.borrow_and_update();

    // The user opens a tab.
    let first = m.open_tab(DEFAULT_GROUP, Some("https://a.example/")).await.unwrap();
    assert!(first.active && first.url == "https://a.example/");
    assert!(changes.has_changed().unwrap(), "the screen is told");
    assert_eq!(b.fake.received("Target.createTarget")[0].1, json!({ "url": "https://a.example/", "newWindow": true }));
    assert_eq!(m.active_tab(DEFAULT_GROUP).unwrap().tab_id, first.tab_id);
    // The module attaches to it: its events are followed and it renders in the background.
    let (cdp, session) = m.tab_session(&first.tab_id).await.unwrap();
    until("the tab is set up", || b.fake.received("Emulation.setFocusEmulationEnabled").iter().any(|(s, _)| s.as_deref() == Some(session.as_str()))).await;
    assert!(b.fake.received("Page.enable").iter().any(|(s, _)| s.as_deref() == Some(session.as_str())));

    // Address and loading follow the browser's events; the title is read once the document is there.
    let mut events = m.tab_events(&first.tab_id, &["Page.frameStartedLoading"]).await.unwrap();
    m.tab_send(&first.tab_id, "Page.navigate", json!({ "url": "https://a.example/slow" })).await.unwrap();
    assert_eq!(events.next().await.unwrap().method, "Page.frameStartedLoading");
    until("the page is loading", || b.tabs(DEFAULT_GROUP)[0].loading && b.tabs(DEFAULT_GROUP)[0].url == "https://a.example/slow").await;
    cdp.send(Some(&session), "Page.navigate", json!({ "url": "https://a.example/done" })).await.unwrap();
    until("the page is loaded", || !b.tabs(DEFAULT_GROUP)[0].loading && b.tabs(DEFAULT_GROUP)[0].title == "Title of https://a.example/done").await;
    // A title a page changes later is seen when the group is shown.
    b.fake.set_page(&first.tab_id, "https://a.example/done", "Changed by the page");
    assert_eq!(m.status().await["groups"][0]["tabs"][0]["title"], "Changed by the page");

    // A page opened by a page joins its opener's group.
    let popup = b.fake.open("https://a.example/popup", Some(&first.tab_id));
    until("the popup is a tab", || m.tab(&popup).is_some()).await;
    assert_eq!(m.tab(&popup).unwrap().group_id, DEFAULT_GROUP);
    assert_eq!(m.active_tab(DEFAULT_GROUP).unwrap().tab_id, popup, "the newest tab is the current one");
    assert_eq!(m.activate_tab(&first.tab_id).unwrap(), DEFAULT_GROUP);
    assert_eq!(b.tabs(DEFAULT_GROUP).iter().map(|t| t.active).collect::<Vec<_>>(), vec![true, false]);

    // A tab without an opener is the default group's, whoever opened it and whoever holds a lease.
    let seen = b.fake.open("https://b.example/", None);
    until("the tab is seen", || m.tab(&seen).is_some()).await;
    assert_eq!(m.tab(&seen).unwrap().group_id, DEFAULT_GROUP);
    let lease = m.acquire("no-such-session").await.unwrap();
    assert_eq!(lease.group_id, DEFAULT_GROUP, "every session works in the one group");
    let by_agent = b.fake.open("https://c.example/", None);
    until("the agent's tab is seen", || m.tab(&by_agent).is_some()).await;
    assert_eq!(m.tab(&by_agent).unwrap().group_id, DEFAULT_GROUP);
    m.release(&lease.lease_id, None, None).unwrap();

    // max_tabs: the module opens no more, and a tab from elsewhere is closed.
    assert_eq!(m.open_tab(DEFAULT_GROUP, None).await.unwrap_err().code, "too_many_tabs");
    let extra = b.fake.open("https://d.example/", None);
    until("the extra tab is closed", || b.fake.received("Target.closeTarget").iter().any(|(_, p)| p["targetId"] == extra.as_str())).await;
    assert!(m.tab(&extra).is_none());
    // The module's own groups are not counted and not listed.
    {
        let _expected = m.expect_tabs("_reflow");
        let own = b.fake.open("https://e.example/", None);
        until("the module's tab is seen", || m.tab(&own).is_some()).await;
        assert_eq!(m.tab(&own).unwrap().group_id, "_reflow");
    }
    let listed: Vec<String> = m.status().await["groups"].as_array().unwrap().iter().map(|g| g["group_id"].as_str().unwrap().to_string()).collect();
    assert_eq!(listed, vec!["default"]);
    assert_eq!(b.tabs(DEFAULT_GROUP).len(), 4);

    // Closing: by the module, and by anyone else.
    m.close_tab(&popup).await.unwrap();
    assert!(m.tab(&popup).is_none() && !b.fake.targets().iter().any(|(id, _)| *id == popup));
    assert_eq!(m.close_tab(&popup).await.unwrap_err().code, "no_such_tab");
    b.fake.close(&first.tab_id);
    until("the closed tab is gone", || b.tabs(DEFAULT_GROUP).len() == 2).await;
    assert_eq!(m.open_tab(DEFAULT_GROUP, Some("bad:url")).await.unwrap_err().code, "invalid_argument");
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialog_nobody_attends_to_is_closed() {
    let b = Bench::new(section(12, 600), |t| t.tick = Duration::from_millis(50)).await;
    let m = &b.module;
    let housekeeping = tokio::spawn(m.clone().run());
    let handled = || b.fake.received("Page.handleJavaScriptDialog").iter().map(|(_, p)| p["accept"].clone()).collect::<Vec<_>>();
    let tab = m.open_tab(DEFAULT_GROUP, Some("https://a.example/")).await.unwrap();
    m.tab_session(&tab.tab_id).await.unwrap();

    // Nobody looks, no agent operates: it would block the page and every Playwright that connects.
    b.fake.dialog(&tab.tab_id, "confirm", "Leave?");
    until("the dialog is dismissed", || handled() == vec![json!(false)]).await;
    until("it is known to be closed", || m.open_dialogs(DEFAULT_GROUP).is_empty() && b.fake.open_dialogs().is_empty()).await;
    b.fake.dialog(&tab.tab_id, "beforeunload", "");
    until("leaving the page is accepted", || handled() == vec![json!(false), json!(true)]).await;

    // Someone looks at the browser: the dialog is theirs to answer ...
    let viewer = m.viewer(DEFAULT_GROUP);
    b.fake.dialog(&tab.tab_id, "alert", "Saved");
    until("the dialog is open", || m.open_dialogs(DEFAULT_GROUP).len() == 1).await;
    assert_eq!(m.open_dialogs(DEFAULT_GROUP)[0], (tab.tab_id.clone(), json!({ "url": "", "message": "Saved", "type": "alert", "hasBrowserHandler": false, "defaultPrompt": "" })));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(handled().len(), 2, "a watched dialog is left open");
    // ... until they leave.
    drop(viewer);
    until("the dialog left behind is dismissed", || handled().len() == 3).await;

    // An agent operating the browser answers its own dialogs (Playwright does); what it leaves is closed.
    let lease = m.acquire("s1").await.unwrap();
    assert_eq!(lease.group_id, DEFAULT_GROUP);
    let agents = m.open_tab(DEFAULT_GROUP, Some("https://b.example/")).await.unwrap();
    m.tab_session(&agents.tab_id).await.unwrap();
    b.fake.dialog(&agents.tab_id, "confirm", "Delete?");
    until("the dialog is open", || m.open_dialogs(DEFAULT_GROUP).len() == 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(handled().len(), 3, "the agent's dialog is the agent's");
    m.release(&lease.lease_id, None, None).unwrap();
    until("the dialog the agent left is dismissed", || handled().len() == 4).await;
    housekeeping.abort();
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_that_dies_is_started_again_with_its_tabs() {
    let b = Bench::new(section(12, 600), |_| {}).await;
    let m = &b.module;
    m.open_tab(DEFAULT_GROUP, Some("https://a.example/")).await.unwrap();
    let second = m.open_tab(DEFAULT_GROUP, Some("https://b.example/one")).await.unwrap();
    m.open_tab(DEFAULT_GROUP, Some("https://b.example/two")).await.unwrap();
    m.activate_tab(&second.tab_id).unwrap();
    let lease = m.acquire("s1").await.unwrap();
    let pid = b.fake.pid().unwrap();
    let mut changes = m.changes();
    changes.borrow_and_update();

    b.fake.crash();
    until("the browser is seen to be gone", || !m.running() || b.fake.pid() != Some(pid)).await;
    assert!(changes.has_changed().unwrap());
    until("the browser runs again", || m.running() && b.fake.pid().is_some_and(|p| p != pid) && b.tabs(DEFAULT_GROUP).len() == 3).await;
    assert!(!alive(pid) && process::group_pids(pid as u32).is_empty());
    let tabs = b.tabs(DEFAULT_GROUP);
    assert_eq!(
        tabs.iter().map(|t| (t.url.as_str(), t.active)).collect::<Vec<_>>(),
        vec![("https://a.example/", false), ("https://b.example/one", true), ("https://b.example/two", false)]
    );
    assert_ne!(tabs[1].tab_id, second.tab_id, "they are new tabs");
    assert_eq!(m.renew(&lease.lease_id).unwrap_err().code, "lease_lost", "a lease ends with the browser it was for");
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_idle_browser_is_stopped_and_its_tabs_come_back() {
    let b = Bench::new(section(12, 1), |t| t.tick = Duration::from_millis(50)).await;
    let m = &b.module;
    let housekeeping = tokio::spawn(m.clone().run());
    m.open_tab(DEFAULT_GROUP, Some("https://a.example/kept")).await.unwrap();
    let pid = b.fake.pid().unwrap();

    // Someone looking keeps it running.
    let viewer = m.viewer(DEFAULT_GROUP);
    tokio::time::sleep(Duration::from_millis(1600)).await;
    assert!(m.running(), "a watched browser is not idle");
    drop(viewer);
    // So does a lease, until it runs out.
    let lease = m.acquire("s1").await.unwrap();
    m.release(&lease.lease_id, None, None).unwrap();
    until("the idle browser is stopped", || !m.running()).await;
    assert!(!alive(pid) && process::group_pids(pid as u32).is_empty());
    assert!(b.tabs(DEFAULT_GROUP).is_empty(), "a stopped browser has no tabs");
    let file: Value = serde_json::from_slice(&std::fs::read(b.root.join("state/browser/groups.json")).unwrap()).unwrap();
    assert_eq!(file["schema"], "opendan.browser.groups/1");
    assert_eq!(file["groups"][0]["group_id"], "default");
    assert_eq!(file["groups"][0]["tabs"][0]["url"], "https://a.example/kept");

    // The next use starts it with what was open.
    b.fake.forget_tabs();
    m.ensure().await.unwrap();
    assert_eq!(b.tabs(DEFAULT_GROUP).iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), vec!["https://a.example/kept"]);
    housekeeping.abort();
    m.shutdown().await;

    // A new service (a restart) finds them in the file.
    b.fake.forget_tabs();
    let tuning = Tuning { chromium: Some(fake_browser::chromium()), dirs: Some(m.dirs().clone()), ..Default::default() };
    let again = BrowserModule::with_tuning(section(12, 600), deps(&b.root), tuning);
    assert!(again.group(DEFAULT_GROUP).tabs.is_empty());
    again.ensure().await.unwrap();
    assert_eq!(again.group(DEFAULT_GROUP).tabs[0].url, "https://a.example/kept");
    again.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_party_operates_a_group_at_a_time() {
    let b = Bench::new(section(12, 600), |t| {
        t.tick = Duration::from_millis(50);
        t.lease_ttl_ms = 600;
    })
    .await;
    let m = &b.module;
    let housekeeping = tokio::spawn(m.clone().run());
    assert_eq!(m.controller(DEFAULT_GROUP), Controller::User);

    // The agent takes a lease: the browser is started for it, the agent operates.
    let lease = m.acquire("ui-1").await.unwrap();
    assert!(m.running());
    assert_eq!(lease.cdp_url, format!("http://127.0.0.1:{}", b.fake.port));
    assert_eq!((lease.ttl_ms, lease.active_tab_id.clone(), lease.tabs.len()), (600, None, 0), "no tab is opened for the agent");
    let group = m.group(DEFAULT_GROUP);
    assert_eq!((group.controller, group.agent_state), (Controller::Agent, AgentState::Working));
    assert!(m.renew(&lease.lease_id).unwrap() >= lease.expires_at_ms);

    // The user takes over: the agent's lease is void and it gets no new one.
    let mut changes = m.changes();
    changes.borrow_and_update();
    let taken = m.hand("did:bns:bob", Handed::TakeOver).await.unwrap();
    assert!(taken.user_hold && taken.controller == Controller::User && taken.lease.is_none());
    assert!(changes.has_changed().unwrap(), "the screen is told");
    assert_eq!(m.renew(&lease.lease_id).unwrap_err().code, "lease_lost");
    assert_eq!(m.acquire("ui-1").await.unwrap_err().code, "user_in_control");
    // Handed back: the agent operates again.
    let back = m.hand("did:bns:bob", Handed::HandBack).await.unwrap();
    assert!(!back.user_hold);
    let lease = m.acquire("ui-1").await.unwrap();
    assert_eq!(m.controller(DEFAULT_GROUP), Controller::Agent);

    // A lease that is not renewed runs out: the user operates, and the screen is told.
    changes.borrow_and_update();
    until("the lease ran out", || m.controller(DEFAULT_GROUP) == Controller::User).await;
    until("the change is announced", || changes.has_changed().unwrap()).await;
    assert_eq!(m.group(DEFAULT_GROUP).agent_state, AgentState::Idle);
    assert_eq!(m.release(&lease.lease_id, Some(AgentState::Done), None).unwrap_err().code, "lease_lost");

    // The agent asks the user to take over.
    let lease = m.acquire("ui-1").await.unwrap();
    m.request_handover("password", Some("the bank's login".into())).await.unwrap();
    let group = m.group(DEFAULT_GROUP);
    assert_eq!((group.controller, group.agent_state, group.user_hold), (Controller::User, AgentState::WaitingTakeover, true));
    assert_eq!(group.handover.as_ref().map(|h| h.reason.as_str()), Some("password"));
    assert_eq!(group.note.as_deref(), Some("the bank's login"));
    assert_eq!(m.renew(&lease.lease_id).unwrap_err().code, "lease_lost");
    assert_eq!(m.acquire("ui-1").await.unwrap_err().code, "user_in_control");
    m.hand("did:bns:bob", Handed::HandToAgent).await.unwrap();
    let lease = m.acquire("ui-1").await.unwrap();
    m.release(&lease.lease_id, Some(AgentState::Done), Some("booked".into())).unwrap();
    let group = m.group(DEFAULT_GROUP);
    assert_eq!((group.controller, group.agent_state, group.note.as_deref()), (Controller::User, AgentState::Done, Some("booked")));
    // Whether the user holds the browser is kept across a restart of the service.
    m.hand("did:bns:bob", Handed::TakeOver).await.unwrap();
    housekeeping.abort();
    m.shutdown().await;
    let tuning = Tuning { chromium: Some(fake_browser::chromium()), dirs: Some(m.dirs().clone()), ..Default::default() };
    let again = BrowserModule::with_tuning(section(12, 600), deps(&b.root), tuning);
    assert!(again.group(DEFAULT_GROUP).user_hold);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_browser_conversation_is_bound() {
    let b = Bench::new(section(12, 600), |_| {}).await;
    let m = &b.module;
    assert_eq!(m.status().await["default_session_id"], Value::Null);
    let own = m.open_tab(DEFAULT_GROUP, Some("https://a.example/")).await.unwrap();
    let mut changes = m.changes();
    changes.borrow_and_update();

    // Binding names the conversation; the tabs stay.
    assert!(m.bind_default("conv-1").unwrap());
    assert!(changes.has_changed().unwrap(), "the screen is told");
    assert_eq!(m.default_session().as_deref(), Some("conv-1"));
    assert_eq!(b.tabs(DEFAULT_GROUP).iter().map(|t| t.tab_id.clone()).collect::<Vec<_>>(), vec![own.tab_id.clone()]);
    let status = m.status().await;
    assert_eq!(status["default_session_id"], "conv-1");
    assert_eq!(status["groups"].as_array().unwrap().len(), 1);
    // Written at once, and read by the next service.
    let file: Value = serde_json::from_slice(&std::fs::read(b.root.join("state/browser/groups.json")).unwrap()).unwrap();
    assert_eq!(file["default_session_id"], "conv-1");
    changes.borrow_and_update();
    assert!(!m.bind_default("conv-1").unwrap());
    assert!(!changes.has_changed().unwrap(), "bound again: nothing happens");
    m.shutdown().await;
    b.fake.forget_tabs();
    let tuning = Tuning { chromium: Some(fake_browser::chromium()), dirs: Some(m.dirs().clone()), ..Default::default() };
    let again = BrowserModule::with_tuning(section(12, 600), deps(&b.root), tuning);
    assert_eq!(again.default_session().as_deref(), Some("conv-1"));
    again.ensure().await.unwrap();
    assert_eq!(again.group(DEFAULT_GROUP).tabs.len(), 1);
    again.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_without_a_browser_says_so() {
    // Switched off.
    let b = Bench::new(BrowserSection { enabled: false, ..section(12, 600) }, |_| {}).await;
    let status = b.module.status().await;
    assert_eq!((status["enabled"].clone(), status["running"].clone(), status["version"].clone()), (json!(false), json!(false), Value::Null));
    assert!(status["unavailable"].as_str().unwrap().contains("enabled = false"));
    assert_eq!((status["unavailable_code"].clone(), status["available"].clone()), (json!("disabled"), json!(true)), "the browser is there, the agent may not use it");
    for error in [
        b.module.ensure().await.err(),
        b.module.acquire("s").await.err(),
        b.module.renew("l").err(),
        b.module.hand("did:bns:bob", Handed::TakeOver).await.err(),
        b.module.bind_default("conv-1").err(),
        b.module.clear_profile().await.err(),
    ] {
        let error = error.unwrap();
        assert_eq!((error.code, error.unavailable_code), ("browser_unavailable", Some("disabled")));
    }
    assert_eq!(b.module.module_status().0, false);

    // No chromium on this node.
    let b = Bench::new(section(12, 600), |t| t.chromium = Some(PathBuf::from("/nonexistent/chromium"))).await;
    let status = b.module.status().await;
    assert_eq!((status["enabled"].clone(), status["available"].clone(), status["version"].clone()), (json!(true), json!(false), Value::Null));
    assert!(status["unavailable"].as_str().unwrap().contains("chromium"));
    assert_eq!(status["unavailable_code"], "no_package");
    let error = b.module.ensure().await.unwrap_err();
    assert_eq!((error.code, error.unavailable_code), ("browser_unavailable", Some("no_package")));
    let (enabled, usable, note) = b.module.module_status();
    assert!(enabled && !usable && note.unwrap().starts_with(status["unavailable"].as_str().unwrap()));

    // A browser that does not start: the reason is kept for `status`.
    let b = Bench::new(section(12, 600), |_| {}).await;
    std::fs::remove_file(b.fake.root.join("fake_cdp_port")).unwrap();
    let error = b.module.ensure().await.unwrap_err();
    assert_eq!((error.code, error.unavailable_code), ("browser_unavailable", None));
    assert!(error.message.contains("ended while starting"), "{}", error.message);
    let status = b.module.status().await;
    assert_eq!((status["available"].clone(), status["running"].clone()), (json!(true), json!(false)));
    assert!(status["unavailable"].as_str().unwrap().contains("ended while starting"));
    assert_eq!(status["unavailable_code"], Value::Null, "the browser is there: it did not start");
}

/// The extension package brings the `chromium` launcher; the browser it
/// starts is the package's online item, downloaded by the node afterwards.
/// What the node has is looked at on every call: once the download is
/// ready the same service has a browser.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_browser_is_usable_as_soon_as_the_node_downloaded_it() {
    let tools = tempfile::tempdir().unwrap();
    let tools = tools.path();
    let dirs = vec![tools.join("bin"), tools.join("agent-tools")];
    let b = Bench::new(section(12, 600), |t| (t.chromium, t.tool_dirs) = (None, Some(dirs))).await;
    let m = &b.module;
    let record = |record: Value| {
        std::fs::create_dir_all(tools.join("online")).unwrap();
        std::fs::write(tools.join("online/chromium.json"), record.to_string()).unwrap();
    };
    async fn unavailable(m: &BrowserModule, unavailable_code: &str) -> String {
        let status = m.status().await;
        assert_eq!((status["enabled"].clone(), status["available"].clone(), status["running"].clone()), (json!(true), json!(false), json!(false)));
        assert_eq!(status["unavailable_code"], unavailable_code);
        let why = status["unavailable"].as_str().unwrap().to_string();
        for error in [m.ensure().await.err(), m.acquire("s").await.err(), m.renew("l").err(), m.clear_profile().await.err()] {
            let error = error.unwrap();
            assert_eq!((error.code, error.unavailable_code, error.message.as_str()), ("browser_unavailable", Some(unavailable_code), why.as_str()));
            let wire: Value = serde_json::from_str(&error.to_wire()).unwrap();
            assert_eq!((wire["code"].as_str(), wire["unavailable_code"].as_str()), (Some("browser_unavailable"), Some(unavailable_code)));
        }
        why
    }

    // No extension package: no launcher.
    assert!(unavailable(m, "no_package").await.contains("no `chromium` on this node"));
    assert_eq!(m.status().await["version"], Value::Null);

    // The package is installed; its browser is not downloaded yet.
    std::fs::create_dir_all(tools.join("bin")).unwrap();
    std::fs::copy(fake_browser::chromium(), tools.join("bin/chromium")).unwrap();
    std::fs::create_dir_all(tools.join("store/ext-core/browser")).unwrap();
    std::fs::write(tools.join("store/ext-core/browser/versions.json"), json!({ "playwright": "1.65.1", "chromium": "159.0.8154.6", "revision": "1255" }).to_string()).unwrap();
    assert!(unavailable(m, "not_downloaded").await.contains("not downloaded on this node yet"));
    assert_eq!(m.status().await["version"], json!({ "browser": "159.0.8154.6", "playwright": "1.65.1" }), "what the package will download");
    assert_eq!(m.module_status().1, false);

    // Being downloaded, and a download that failed.
    record(json!({ "schema": "buckyos.ext_tools.online/1", "id": "chromium", "state": "downloading", "size_bytes": 200u64 << 20, "downloaded_bytes": 50u64 << 20 }));
    assert!(unavailable(m, "not_downloaded").await.contains("being downloaded to this node (25% of 200 MiB)"));
    record(json!({ "state": "failed", "error": "GET https://storage.googleapis.com/…: connection timed out", "size_bytes": 200u64 << 20, "downloaded_bytes": 0 }));
    assert!(unavailable(m, "not_downloaded").await.contains("failed: GET https://storage.googleapis.com/…: connection timed out. Start it again"));
    assert!(b.fake.pid().is_none(), "nothing was started");

    // Downloaded: the same service starts it, nothing was remembered.
    record(json!({ "state": "downloading", "size_bytes": 200u64 << 20, "downloaded_bytes": 199u64 << 20 }));
    std::fs::create_dir_all(tools.join("online/chromium")).unwrap();
    unavailable(m, "not_downloaded").await;
    record(json!({ "state": "ready", "version": "159.0.8154.6", "size_bytes": 200u64 << 20, "downloaded_bytes": 200u64 << 20 }));
    let status = m.status().await;
    assert_eq!((status["available"].clone(), status["unavailable"].clone(), status["unavailable_code"].clone()), (json!(true), Value::Null, Value::Null));
    assert_eq!(m.module_status().1, true);
    let lease = m.acquire("s").await.unwrap();
    assert!(b.fake.pid().is_some() && m.running(), "acquiring started it");
    let status = m.status().await;
    assert_eq!((status["running"].clone(), status["version"].clone()), (json!(true), json!({ "browser": "1.2.3.4", "playwright": "1.65.1" })));
    assert!(b.fake.args().contains("--user-agent=") && b.fake.args().contains("Chrome/159.0.0.0"), "the package's browser gets a windowed browser's user agent");
    m.release(&lease.lease_id, None, None).unwrap();
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clearing_the_profile_stops_the_browser_and_deletes_its_data() {
    let b = Bench::new(section(12, 600), |_| {}).await;
    let m = &b.module;
    m.open_tab("default", Some("https://a.example/")).await.unwrap();
    let pid = b.fake.pid().unwrap();
    std::fs::write(m.dirs().profile.join("Cookies"), "logins").unwrap();
    std::fs::create_dir_all(&m.dirs().downloads).unwrap();
    std::fs::write(m.dirs().downloads.join("kept.pdf"), "a download").unwrap();
    m.clear_profile().await.unwrap();
    assert!(!m.running() && !alive(pid));
    assert!(!m.dirs().profile.exists() && !m.dirs().cache.exists());
    assert!(m.dirs().downloads.join("kept.pdf").exists(), "downloads are not logins");
    b.fake.forget_tabs();
    m.ensure().await.unwrap();
    assert_eq!(b.tabs("default")[0].url, "https://a.example/", "the tabs come back, logged out");
    m.shutdown().await;
}

// ---------------------------------------------------------------------------
// A real Chromium (`OPENDAN_CHROMIUM`)
// ---------------------------------------------------------------------------

mod real {
    use super::*;

    struct Real {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        module: Arc<BrowserModule>,
    }

    fn real(cfg: BrowserSection, tune: impl FnOnce(&mut Tuning)) -> Real {
        logs();
        let chromium = std::env::var_os(process::ENV_CHROMIUM).map(PathBuf::from).expect("set OPENDAN_CHROMIUM to a Chromium executable");
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("agent_root");
        let mut tuning = Tuning { chromium: Some(chromium), dirs: Some(BrowserDirs::at(tmp.path().join("browser"))), ..Default::default() };
        tune(&mut tuning);
        let module = BrowserModule::with_tuning(cfg, deps(&root), tuning);
        Real { _tmp: tmp, root, module }
    }

    /// Processes whose command line names this profile.
    fn processes_of(profile: &Path) -> Vec<u32> {
        let needle = profile.display().to_string();
        std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| String::from_utf8_lossy(&c).contains(&needle))
                    && alive(*pid as i32)
            })
            .collect()
    }

    fn page(title: &str) -> String {
        format!("data:text/html,<title>{title}</title><body><h1>{title}</h1></body>")
    }

    async fn eval(m: &BrowserModule, tab_id: &str, expression: &str) -> Value {
        let answer = m.tab_send(tab_id, "Runtime.evaluate", json!({ "expression": expression, "returnByValue": true, "userGesture": true })).await.unwrap();
        answer["result"]["value"].clone()
    }

    /// A page on an http origin (cookies need one): everything gets the same document.
    async fn site() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = [0u8; 4096];
                    let _ = stream.read(&mut request).await;
                    let body = "<title>Site</title><body>site</body>";
                    let _ = stream
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                        .await;
                });
            }
        });
        format!("http://127.0.0.1:{port}/")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn starts_shows_tabs_and_ends_without_leaving_processes() {
        let r = real(section(12, 600), |_| {});
        let m = &r.module;
        m.ensure().await.unwrap();
        let processes = processes_of(&m.dirs().profile);
        assert!(processes.len() > 1, "the browser and its helpers run");
        let group = process::group_pids(*processes.iter().min().unwrap());
        assert!(group.len() > 1, "the browser's helpers are in its process group");
        let status = m.status().await;
        assert!(status["version"]["browser"].as_str().unwrap().split('.').count() == 4, "{status}");
        assert_eq!(status["groups"][0]["tabs"], json!([]), "the browser starts without a tab of its own");

        // Two tabs, one of them the module's own: both render at once.
        let a = m.open_tab(DEFAULT_GROUP, Some(&page("Alpha"))).await.unwrap();
        let b = m.open_tab("_own", Some(&page("Beta"))).await.unwrap();
        until("titles arrive", || m.group(DEFAULT_GROUP).tabs[0].title == "Alpha" && m.group("_own").tabs[0].title == "Beta").await;
        until("the pages are loaded", || !m.group(DEFAULT_GROUP).tabs[0].loading && !m.group("_own").tabs[0].loading).await;
        for tab in [&a, &b] {
            // Set by the module's attachment, which may still be on its way.
            let started = Instant::now();
            while eval(m, &tab.tab_id, "document.visibilityState").await != "visible" {
                assert!(started.elapsed() < Duration::from_secs(5), "tab {} stays hidden", tab.tab_id);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        assert_eq!(eval(m, &a.tab_id, "innerWidth").await, 1280);

        // A page opened by a page joins its group; a tab without opener is the default group's.
        eval(m, &b.tab_id, "window.open('about:blank') && 1").await;
        until("the popup is a tab of its opener's group", || m.group("_own").tabs.len() == 2).await;
        let cdp = m.cdp().unwrap();
        let foreign = cdp.send(None, "Target.createTarget", json!({ "url": page("Gamma") })).await.unwrap();
        let foreign = foreign["targetId"].as_str().unwrap().to_string();
        until("the foreign tab is seen", || m.tab(&foreign).is_some()).await;
        assert_eq!(m.tab(&foreign).unwrap().group_id, DEFAULT_GROUP);
        // It took the window of another tab: both still render.
        until("the foreign tab is attached", || m.tab(&foreign).unwrap().session.is_some()).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        for tab_id in [&a.tab_id, &b.tab_id, &foreign] {
            assert_eq!(eval(m, tab_id, "document.visibilityState").await, "visible", "tab {tab_id}");
        }

        // Navigation: loading is followed.
        let mut events = m.tab_events(&a.tab_id, &["Page.frameStoppedLoading"]).await.unwrap();
        m.tab_send(&a.tab_id, "Page.navigate", json!({ "url": page("Delta") })).await.unwrap();
        assert!(events.next().await.is_some());
        until("the new title arrives", || m.group(DEFAULT_GROUP).tabs[0].title == "Delta" && !m.group(DEFAULT_GROUP).tabs[0].loading).await;

        // A dialog nobody attends to is closed: the page goes on.
        eval(m, &a.tab_id, "setTimeout(() => alert('unattended'), 0) && 1").await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(eval(m, &a.tab_id, "2 + 2").await, 4);
        assert!(m.open_dialogs(DEFAULT_GROUP).is_empty());

        // The crash handlers left the browser's process group; they carry its mark and end with it.
        let mark = format!("{}={}-", process::ENV_MARK, std::process::id());
        let handlers: Vec<u32> = std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|env| String::from_utf8_lossy(&env).contains(&mark)))
            .filter(|pid| !group.contains(pid))
            .collect();
        assert!(!handlers.is_empty(), "this Chromium starts crash handlers outside its group");

        m.close_tab(&b.tab_id).await.unwrap();
        assert_eq!(m.group("_own").tabs.len(), 1);
        m.shutdown().await;
        until("the processes are gone", || processes_of(&m.dirs().profile).is_empty()).await;
        until("the crash handlers are gone", || handlers.iter().all(|pid| !Path::new(&format!("/proc/{pid}")).exists())).await;
        assert!(m.dirs().profile.join("Default").exists() || m.dirs().profile.join("Local State").exists(), "the profile is kept");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn a_killed_browser_is_started_again_with_its_tabs() {
        let r = real(section(12, 600), |_| {});
        let m = &r.module;
        m.open_tab(DEFAULT_GROUP, Some(&page("Kept"))).await.unwrap();
        until("the title arrives", || m.group(DEFAULT_GROUP).tabs[0].title == "Kept").await;
        let before = processes_of(&m.dirs().profile);
        let main = *before.iter().min().unwrap();
        assert_eq!(std::process::Command::new("kill").args(["-KILL", &main.to_string()]).status().unwrap().code(), Some(0));
        until("the browser is seen to be gone", || !m.running()).await;
        until("it runs again with the tab", || m.running() && m.group(DEFAULT_GROUP).tabs.first().is_some_and(|t| t.title == "Kept")).await;
        let after = processes_of(&m.dirs().profile);
        assert!(!after.is_empty() && after.iter().all(|pid| !before.contains(pid)), "before {before:?}, after {after:?}");
        m.shutdown().await;
        until("the processes are gone", || processes_of(&m.dirs().profile).is_empty()).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn an_idle_browser_is_stopped_and_comes_back_with_tabs_and_logins() {
        let r = real(section(12, 2), |t| t.tick = Duration::from_millis(100));
        let m = &r.module;
        let housekeeping = tokio::spawn(m.clone().run());
        let site = site().await;
        let tab = m.open_tab(DEFAULT_GROUP, Some(&site)).await.unwrap();
        until("the site is loaded", || m.group(DEFAULT_GROUP).tabs[0].title == "Site" && !m.group(DEFAULT_GROUP).tabs[0].loading).await;
        eval(m, &tab.tab_id, "document.cookie = 'login=alice; max-age=3600'").await;
        assert_eq!(eval(m, &tab.tab_id, "document.cookie").await, "login=alice");

        until("the idle browser is stopped", || !m.running()).await;
        assert!(processes_of(&m.dirs().profile).is_empty());
        let file: Value = serde_json::from_slice(&std::fs::read(r.root.join("state/browser/groups.json")).unwrap()).unwrap();
        assert_eq!(file["groups"][0]["tabs"][0]["url"], site.as_str());

        m.ensure().await.unwrap();
        until("the tab is back", || m.group(DEFAULT_GROUP).tabs.first().is_some_and(|t| t.title == "Site" && !t.loading)).await;
        let tab = m.group(DEFAULT_GROUP).tabs[0].clone();
        assert_eq!(eval(m, &tab.tab_id, "document.cookie").await, "login=alice", "logins are kept");

        // Clearing the profile logs out.
        m.clear_profile().await.unwrap();
        assert!(processes_of(&m.dirs().profile).is_empty());
        m.ensure().await.unwrap();
        until("the tab is back again", || m.group(DEFAULT_GROUP).tabs.first().is_some_and(|t| t.title == "Site" && !t.loading)).await;
        let tab = m.group(DEFAULT_GROUP).tabs[0].clone();
        assert_eq!(eval(m, &tab.tab_id, "document.cookie").await, "");
        housekeeping.abort();
        m.shutdown().await;
    }

    /// `OPENDAN_TEST_PLAYWRIGHT=<script.mjs>`: a script run as `node <script> <cdp_url>` that
    /// connects with Playwright, opens a page titled "From Playwright" and disconnects.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs OPENDAN_CHROMIUM"]
    async fn the_lease_gives_playwright_an_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let r = real(section(12, 600), |_| {});
        let m = &r.module;
        let user_tab = m.open_tab(DEFAULT_GROUP, Some(&page("Users"))).await.unwrap();
        let _viewer = m.viewer(DEFAULT_GROUP);
        let lease = m.acquire("ui-1").await.unwrap();
        assert_eq!(lease.group_id, DEFAULT_GROUP);

        // The endpoint answers the discovery request `connectOverCDP` starts with.
        let address = lease.cdp_url.strip_prefix("http://").unwrap();
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(format!("GET /json/version HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        // The browser keeps the connection open: read until the answer is there.
        let mut answer = String::new();
        let mut chunk = [0u8; 4096];
        while !answer.contains("webSocketDebuggerUrl") {
            let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await.expect("the endpoint answers").unwrap();
            assert!(read > 0, "{answer}");
            answer.push_str(&String::from_utf8_lossy(&chunk[..read]));
        }

        let Some(script) = std::env::var_os("OPENDAN_TEST_PLAYWRIGHT") else {
            eprintln!("OPENDAN_TEST_PLAYWRIGHT is not set: Playwright itself was not run");
            m.shutdown().await;
            return;
        };
        let output = tokio::process::Command::new("node").arg(script).arg(&lease.cdp_url).output().await.unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        // What the agent opened is a tab of the browser beside the user's.
        until("the agent's tab is seen", || m.group(DEFAULT_GROUP).tabs.iter().any(|t| t.title == "From Playwright")).await;
        assert_eq!(m.group(DEFAULT_GROUP).tabs.len(), 2);
        // Playwright connected and left: the user's tab is as it was.
        assert_eq!(eval(m, &user_tab.tab_id, "document.title").await, "Users");
        m.release(&lease.lease_id, Some(AgentState::Done), None).unwrap();
        m.shutdown().await;
        until("the processes are gone", || processes_of(&m.dirs().profile).is_empty()).await;
    }
}
