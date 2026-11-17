//! The agent's browser: one Chromium the Agent Service starts on demand in
//! the agent's private runtime, with the agent's logins and one group of
//! tabs for all its sessions. On Linux it runs headed on a virtual display
//! of its own ([`display`]), as a browser on a desktop does; elsewhere it is
//! headless. The agent operates it with
//! Playwright over its debugging endpoint (`agent-browser`); the owner sees
//! and operates it through the screen. One party operates it at a time and
//! hands it over explicitly ([Agent Browser Protocol]).
//!
//! - [`process`]: the Chromium process (start, stop, what it leaves behind).
//! - [`display`]: the virtual display a headed browser draws on; [`rfb`]: how it is read and operated.
//! - [`cdp`]: the DevTools Protocol client.
//! - [`identity`]: what a headless browser's pages are told it is; passkeys refused.
//! - [`groups`]: groups, tabs, leases, who operates.
//! - [`sessions`]: the browser conversation and its records.
//! - [`rpc`]: `browser.*`; [`http`]: the screen and upload entries.
//! - [`screen`]: a screen connection; [`input`]: what the user does as
//!   input commands; [`requests`]: what a page shows outside itself;
//!   [`transfer`]: uploads and downloads.
//! - [`reflow`]: the owner's reflow library and refreshing a page of it.
//!
//! [Agent Browser Protocol]: ../../../../../doc/opendan/protocol/Agent%20Browser%20Protocol.md

pub mod cdp;
pub mod display;
pub mod groups;
pub mod http;
pub mod identity;
pub mod input;
pub mod process;
pub mod proxy;
pub mod reflow;
pub mod requests;
pub mod rfb;
pub mod rpc;
pub mod screen;
#[cfg(test)]
mod screen_tests;
pub mod sessions;
#[cfg(test)]
mod tests;
pub mod transfer;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use libopendan::host::Supervisor;
use libopendan::state::AgentStateClient;
use libopendan::OpenDanError;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, watch};

pub use cdp::{CdpClient, CdpError, CdpEvent, EventStream, Scope};
pub use groups::{AgentState, Controller, Group, Handover, LeaseInfo, Tab, TabRef, DEFAULT_GROUP};
pub use process::BrowserDirs;
pub use rpc::BrowserError;
pub use sessions::Handed;

use crate::config::BrowserSection;
use display::{Display, DisplayMode};
use groups::Groups;
use process::Chromium;

/// Where a command of a session reaches `browser.*`: the service's kRPC
/// address on the loopback interface.
pub const ENV_BROWSER_RPC: &str = "OPENDAN_BROWSER_RPC";

const CLOSE_WAIT: Duration = Duration::from_secs(5);
const TERMINATE_WAIT: Duration = Duration::from_secs(3);
const ATTACH_WAIT: Duration = Duration::from_secs(5);
/// A browser that ran this long was not started into a crash.
const STABLE_RUN: Duration = Duration::from_secs(60);
const MAX_RESTARTS: u32 = 5;
/// Ticks between two looks at which conversations wait for an answer.
const WAITING_EVERY: u64 = 3;

fn now_ms() -> u64 {
    libopendan::now_ms()
}

fn backoff(crashes: u32) -> Duration {
    Duration::from_secs(1u64 << crashes.saturating_sub(1).min(6)).min(Duration::from_secs(60))
}

pub struct BrowserDeps {
    pub agent: Arc<dyn AgentStateClient>,
    /// Hosts the session a record was posted to (`None`: nothing is hosted here).
    pub supervisor: Option<Arc<Supervisor>>,
    /// The owner's DID: whose conversations the groups are.
    pub owner_did: Option<String>,
    pub agent_root: PathBuf,
}

/// What tests and debugging change about the module.
#[derive(Debug, Clone)]
pub struct Tuning {
    /// The browser's executable, started as it is (default: what
    /// `tool_dirs` or [`process::find_chromium`] finds).
    pub chromium: Option<PathBuf>,
    /// The node's tool directories (default: those of [`process::find_chromium`]).
    pub tool_dirs: Option<Vec<PathBuf>>,
    /// The browser's directories (default: [`BrowserDirs::of`] the AgentRoot).
    pub dirs: Option<BrowserDirs>,
    /// Interval of the housekeeping loop.
    pub tick: Duration,
    pub lease_ttl_ms: u64,
    /// The `agent-browser` command (default: [`reflow::find_agent_browser`]).
    pub agent_browser: Option<PathBuf>,
    /// The owner's reflow library (default: the one of the Zone the service runs in).
    pub reflow_library: Option<reflow::Library>,
    /// The waits of a screen connection.
    pub screen: screen::Timing,
    /// What one uploaded file may be at most, in bytes.
    pub upload_limit: u64,
    /// The Zone uploads are read from and downloads are exported to
    /// (default: the one the service runs in).
    pub zone: Option<Arc<dyn transfer::Zone>>,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            chromium: None,
            tool_dirs: None,
            dirs: None,
            tick: Duration::from_secs(1),
            lease_ttl_ms: groups::LEASE_TTL_MS,
            agent_browser: None,
            reflow_library: None,
            screen: screen::Timing::default(),
            upload_limit: transfer::UPLOAD_LIMIT,
            zone: None,
        }
    }
}

/// `browser.acquire`.
#[derive(Debug, Clone, Serialize)]
pub struct Acquired {
    pub lease_id: String,
    pub group_id: String,
    pub ttl_ms: u64,
    pub expires_at_ms: u64,
    /// `http://127.0.0.1:<port>`, for `chromium.connectOverCDP`.
    pub cdp_url: String,
    pub tabs: Vec<Tab>,
    pub active_tab_id: Option<String>,
}

struct Proc {
    generation: u64,
    tree: process::ProcessTree,
    /// The display a headed browser draws on, and the task that keeps its windows on it.
    display: Option<(Arc<Display>, tokio::task::JoinHandle<()>)>,
    cdp: Arc<CdpClient>,
    exited: watch::Receiver<bool>,
    tracker: tokio::task::JoinHandle<()>,
    /// A headless browser's connection that dresses its targets as they start ([`identity`]).
    dresser: Option<tokio::task::JoinHandle<()>>,
    started: Instant,
}

#[derive(Default)]
struct Life {
    proc: Option<Proc>,
    generation: u64,
    crashes: u32,
}

struct Shared {
    groups: Groups,
    cdp: Option<Arc<CdpClient>>,
    display: Option<Arc<Display>>,
    /// Tabs a page opened (popups): they keep the size they were opened at.
    popups: HashSet<String>,
    /// Port and path of the debugging endpoint.
    port: Option<u16>,
    endpoint_path: String,
    browser_version: Option<String>,
    last_error: Option<String>,
    /// Who looks at which group, in the order they came.
    viewers: Vec<(u64, String)>,
    /// Groups tabs are being opened for (the latest counts).
    expected: Vec<(u64, String)>,
    guard_seq: u64,
    /// Tabs the module started to attach to.
    attaching: HashSet<String>,
    /// JS dialogs that are open, by tab: the `Page.javascriptDialogOpening` they came with.
    dialogs: HashMap<String, Value>,
    /// A headless browser whose pages are told what it is as they start: the pages that were.
    dressed: Option<Arc<identity::Dressed>>,
    last_active: Instant,
    stopping: bool,
    restoring: bool,
    /// What `groups.json` holds.
    written: String,
}

pub struct BrowserModule {
    me: Weak<BrowserModule>,
    cfg: BrowserSection,
    mode: DisplayMode,
    deps: BrowserDeps,
    tuning: Tuning,
    dirs: BrowserDirs,
    groups_file: PathBuf,
    /// Serializes starting and stopping the browser.
    life: tokio::sync::Mutex<Life>,
    shared: Mutex<Shared>,
    revision: watch::Sender<u64>,
    closed: AtomicBool,
    reflow: reflow::Reflow,
    /// What pages show outside themselves, for the screens.
    outside: requests::Outside,
    /// The tabs whose frames are being taken.
    casts: screen::Casts,
    /// The agent's connections to the browser, by lease.
    pub proxies: Arc<proxy::Proxies>,
}

/// Someone looking at the browser (a screen connection). While one exists
/// the browser is not stopped for being idle, and its dialogs are left to it.
pub struct Viewer {
    module: Arc<BrowserModule>,
    id: u64,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        {
            let mut s = self.module.shared();
            s.viewers.retain(|(id, _)| *id != self.id);
            s.last_active = Instant::now();
        }
        self.module.close_unattended_dialogs();
    }
}

/// While it lives, tabs that appear without an opener belong to its group
/// (a command opens tabs of its own, like a reflow refresh in `_reflow`).
pub struct ExpectedTabs {
    module: Arc<BrowserModule>,
    id: u64,
}

impl Drop for ExpectedTabs {
    fn drop(&mut self) {
        self.module.shared().expected.retain(|(id, _)| *id != self.id);
    }
}

async fn ended(exited: &mut watch::Receiver<bool>, within: Duration) -> bool {
    tokio::time::timeout(within, exited.wait_for(|e| *e)).await.is_ok()
}

impl BrowserModule {
    pub fn new(cfg: BrowserSection, deps: BrowserDeps) -> Arc<Self> {
        Self::with_tuning(cfg, deps, Tuning::default())
    }

    pub fn with_tuning(cfg: BrowserSection, deps: BrowserDeps, tuning: Tuning) -> Arc<Self> {
        let groups_file = deps.agent_root.join("state").join("browser").join("groups.json");
        let mut groups = Groups::new(cfg.max_tabs).with_lease_ttl(tuning.lease_ttl_ms);
        groups.load(&groups_file);
        let written = groups.to_json();
        let outside = requests::Outside::new(&tuning.dirs.clone().unwrap_or_else(|| BrowserDirs::of(&deps.agent_root)), tuning.screen.dialog);
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            dirs: tuning.dirs.clone().unwrap_or_else(|| BrowserDirs::of(&deps.agent_root)),
            mode: cfg.display.resolved(),
            cfg,
            deps,
            tuning,
            groups_file,
            life: tokio::sync::Mutex::new(Life::default()),
            shared: Mutex::new(Shared {
                groups,
                cdp: None,
                display: None,
                popups: HashSet::new(),
                port: None,
                endpoint_path: String::new(),
                browser_version: None,
                last_error: None,
                viewers: Vec::new(),
                expected: Vec::new(),
                guard_seq: 0,
                attaching: HashSet::new(),
                dialogs: HashMap::new(),
                dressed: None,
                last_active: Instant::now(),
                stopping: false,
                restoring: false,
                written,
            }),
            revision: watch::channel(0).0,
            closed: AtomicBool::new(false),
            reflow: reflow::Reflow::default(),
            outside,
            casts: screen::Casts::default(),
            proxies: Arc::new(proxy::Proxies::default()),
        })
    }

    fn me(&self) -> Arc<Self> {
        self.me.upgrade().expect("the module is alive while it is called")
    }

    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().expect("browser state")
    }

    fn bump(&self) {
        self.revision.send_modify(|r| *r += 1);
    }

    fn failed(&self, message: &str) {
        log::warn!("browser: {message}");
        self.shared().last_error = Some(message.to_string());
    }

    // ------------------------------------------------------------------
    // What the screen and the other parts of the service use
    // ------------------------------------------------------------------

    /// Changes whenever something a `state` message shows changed (tabs,
    /// who operates, the agent's state, whether the browser runs): take
    /// [`Self::group`] again.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// The connection to the running browser.
    pub fn cdp(&self) -> Option<Arc<CdpClient>> {
        self.shared().cdp.clone()
    }

    pub fn running(&self) -> bool {
        self.shared().cdp.is_some()
    }

    /// Whether the browser runs headed on a virtual display.
    pub fn headed(&self) -> bool {
        self.mode == DisplayMode::Virtual
    }

    /// The display of the running headed browser.
    pub fn display(&self) -> Option<Arc<Display>> {
        self.shared().display.clone()
    }

    /// Device pixels per CSS pixel a headed browser draws at.
    fn scale(&self) -> f64 {
        let scale = self.cfg.device_scale_factor;
        if scale.is_finite() { (scale * 100.0).round().clamp(100.0, 300.0) / 100.0 } else { 1.0 }
    }

    pub fn dirs(&self) -> &BrowserDirs {
        &self.dirs
    }

    /// What the node has of the browser, as it is now: nothing of it is
    /// remembered, so a browser that was installed or downloaded since the
    /// last call is found.
    fn chromium(&self) -> Chromium {
        match (&self.tuning.chromium, &self.tuning.tool_dirs) {
            (Some(path), _) => process::named_chromium(path),
            (None, Some(dirs)) => process::chromium_in(dirs),
            (None, None) => process::find_chromium(),
        }
    }

    fn usable_with(&self, chromium: Chromium) -> Result<PathBuf, BrowserError> {
        if !self.cfg.enabled {
            return Err(BrowserError::no_browser(
                "disabled",
                "the browser is switched off for this agent ([loader.browser] enabled = false)",
            ));
        }
        let exe = match chromium {
            Chromium::Ready(exe) => exe,
            Chromium::NotDownloaded { why, .. } => return Err(BrowserError::no_browser("not_downloaded", why)),
            Chromium::NoPackage(why) => return Err(BrowserError::no_browser("no_package", why)),
        };
        if self.headed() && display::find_xvnc().is_none() {
            return Err(BrowserError::no_browser(
                "no_display",
                "this runtime has no virtual display for the browser (`Xvnc` of TigerVNC is not on PATH; it comes with the Agent's container image)",
            ));
        }
        Ok(exe)
    }

    /// The browser's executable, or why this agent has no browser.
    pub fn usable(&self) -> Result<PathBuf, BrowserError> {
        self.usable_with(self.chromium())
    }

    /// `(enabled, usable, note)` for `loader.status`, which shows what the
    /// service found when it started.
    pub fn module_status(&self) -> (bool, bool, Option<String>) {
        match self.usable() {
            Ok(exe) => (true, true, Some(format!("{}; started on demand", exe.display()))),
            Err(e) if self.cfg.enabled => (true, false, Some(format!("{}; looked for again at every use", e.message))),
            Err(e) => (false, false, Some(e.message)),
        }
    }

    /// The group as `browser.status` and the screen's `state` show it.
    pub fn group(&self, group_id: &str) -> Group {
        self.shared().groups.group(group_id, now_ms())
    }

    pub fn controller(&self, group_id: &str) -> Controller {
        self.shared().groups.controller(group_id, now_ms())
    }

    pub fn tab(&self, tab_id: &str) -> Option<TabRef> {
        self.shared().groups.tab(tab_id)
    }

    /// The group's current tab.
    pub fn active_tab(&self, group_id: &str) -> Option<TabRef> {
        let s = self.shared();
        s.groups.active_tab(group_id).and_then(|id| s.groups.tab(&id))
    }

    /// Register someone looking at `group_id` until the value is dropped.
    pub fn viewer(&self, group_id: &str) -> Viewer {
        let mut s = self.shared();
        s.guard_seq += 1;
        let id = s.guard_seq;
        s.viewers.push((id, group_id.to_string()));
        s.last_active = Instant::now();
        Viewer { module: self.me(), id }
    }

    pub fn viewers(&self) -> usize {
        self.shared().viewers.len()
    }

    /// Whether someone looks at `group_id`.
    pub fn watched(&self, group_id: &str) -> bool {
        self.shared().viewers.iter().any(|(_, group)| group == group_id)
    }

    /// The tab the module reaches through the flat session `session`.
    pub fn tab_of_session(&self, session: &str) -> Option<TabRef> {
        let s = self.shared();
        s.groups.tab_of_session(session).and_then(|tab_id| s.groups.tab(&tab_id))
    }

    /// Tabs that appear without an opener while the value lives belong to
    /// `group_id` (a group whose id starts with `_` is the module's own: not
    /// listed, not saved, not counted against `max_tabs`).
    pub fn expect_tabs(&self, group_id: &str) -> ExpectedTabs {
        let mut s = self.shared();
        s.guard_seq += 1;
        let id = s.guard_seq;
        s.expected.push((id, group_id.to_string()));
        ExpectedTabs { module: self.me(), id }
    }

    /// Open a tab in `group_id` (blank without `url`); it becomes the
    /// group's current tab. Starts the browser when it is not running.
    pub async fn open_tab(&self, group_id: &str, url: Option<&str>) -> Result<Tab, BrowserError> {
        let cdp = self.ensure().await?;
        {
            let s = self.shared();
            if !groups::is_internal(group_id) && !s.groups.has_room() {
                return Err(BrowserError::too_many_tabs(s.groups.max_tabs()));
            }
        }
        let tab_id = self.open_in(&cdp, group_id, url.unwrap_or("about:blank")).await?;
        self.group(group_id)
            .tabs
            .into_iter()
            .find(|t| t.tab_id == tab_id)
            .ok_or_else(|| BrowserError::no_such_tab(&tab_id))
    }

    async fn open_in(&self, cdp: &CdpClient, group_id: &str, url: &str) -> Result<String, BrowserError> {
        // A page that is told what the browser is as it starts opens blank and goes to `url` once it was
        // told: a tab opened at an address sends its first request before anything is set for it.
        let dressed = self.shared().dressed.clone().filter(|_| url != "about:blank");
        let first = if dressed.is_some() { "about:blank" } else { url };
        // A window of its own: tabs of one window hide each other.
        let expected = self.expect_tabs(group_id);
        log::debug!("browser: opening a tab of group {group_id} at {url}");
        let answer = cdp.send(None, "Target.createTarget", json!({ "url": first, "newWindow": true })).await;
        log::debug!("browser: the tab is opened: {answer:?}");
        drop(expected);
        let tab_id = answer?["targetId"]
            .as_str()
            .ok_or_else(|| BrowserError::unavailable("the browser opened no tab"))?
            .to_string();
        self.shared().groups.tab_seen(&tab_id, group_id, url, "");
        self.bump();
        if let Some(dressed) = dressed {
            if !dressed.wait(&tab_id, ATTACH_WAIT).await {
                log::warn!("browser: tab {tab_id} was not told what the browser is in time; it goes to {url} anyway");
            }
            let went = match self.tab_session(&tab_id).await {
                Ok((cdp, session)) => cdp.send(Some(&session), "Page.navigate", json!({ "url": url })).await.map_err(BrowserError::from),
                Err(e) => Err(e),
            };
            // An address the browser does not take opens no tab, as it does not when a tab is opened at it.
            if let Err(e) = went {
                let _ = cdp.send(None, "Target.closeTarget", json!({ "targetId": tab_id })).await;
                return Err(e);
            }
        }
        Ok(tab_id)
    }

    pub async fn close_tab(&self, tab_id: &str) -> Result<(), BrowserError> {
        if self.tab(tab_id).is_none() {
            return Err(BrowserError::no_such_tab(tab_id));
        }
        let cdp = self.cdp().ok_or_else(|| BrowserError::unavailable("the browser is not running"))?;
        cdp.send(None, "Target.closeTarget", json!({ "targetId": tab_id })).await?;
        if self.forget_tab(tab_id) {
            self.bump();
        }
        Ok(())
    }

    /// Make `tab_id` its group's current tab. Returns the group.
    pub fn activate_tab(&self, tab_id: &str) -> Result<String, BrowserError> {
        let group_id = self.shared().groups.activate(tab_id)?;
        self.bump();
        Ok(group_id)
    }

    /// The module's flat CDP session of a tab, once it is attached.
    pub async fn tab_session(&self, tab_id: &str) -> Result<(Arc<CdpClient>, String), BrowserError> {
        let mut changes = self.changes();
        let late = tokio::time::sleep(ATTACH_WAIT);
        tokio::pin!(late);
        loop {
            let (tab, cdp) = {
                let s = self.shared();
                (s.groups.tab(tab_id), s.cdp.clone())
            };
            let tab = tab.ok_or_else(|| BrowserError::no_such_tab(tab_id))?;
            let cdp = cdp.ok_or_else(|| BrowserError::unavailable("the browser is not running"))?;
            if let Some(session) = tab.session {
                return Ok((cdp, session));
            }
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                _ = &mut late => break,
            }
        }
        Err(BrowserError::unavailable(format!("tab {tab_id} is not attached yet")))
    }

    /// Send a command to a tab and wait for its answer.
    pub async fn tab_send(&self, tab_id: &str, method: &str, params: Value) -> Result<Value, BrowserError> {
        let (cdp, session) = self.tab_session(tab_id).await?;
        Ok(cdp.send(Some(&session), method, params).await?)
    }

    /// Events of a tab from now on (`methods` empty: all of them).
    pub async fn tab_events(&self, tab_id: &str, methods: &[&str]) -> Result<EventStream, BrowserError> {
        let (cdp, session) = self.tab_session(tab_id).await?;
        Ok(cdp.subscribe(Scope::Session(session), methods))
    }

    // ------------------------------------------------------------------
    // The process
    // ------------------------------------------------------------------

    /// Start the browser unless it runs. Returns the connection to it.
    pub async fn ensure(&self) -> Result<Arc<CdpClient>, BrowserError> {
        self.usable()?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(BrowserError::unavailable("the Agent Service is shutting down"));
        }
        let mut life = self.life.lock().await;
        if let Some(proc) = life.proc.as_ref().filter(|p| !p.cdp.is_closed()) {
            self.shared().last_active = Instant::now();
            return Ok(proc.cdp.clone());
        }
        self.stop(&mut life, "its connection is closed").await;
        life.crashes = 0;
        self.start(&mut life).await
    }

    async fn start(&self, life: &mut Life) -> Result<Arc<CdpClient>, BrowserError> {
        let exe = self.usable()?;
        let (display, shown) = match self.headed() {
            false => (None, process::Shown::Headless),
            true => {
                let xvnc = display::find_xvnc().ok_or_else(|| BrowserError::no_browser("no_display", "`Xvnc` is gone"))?;
                let scale = self.scale();
                match Display::start(&xvnc, &self.dirs, scale).await {
                    Ok(display) => {
                        let painting = *display.painting().borrow();
                        let shown = process::Shown::Display { display: display.name(), scale, size: (painting.width, painting.height) };
                        (Some(display), shown)
                    }
                    Err(e) => {
                        let message = format!("the browser's display did not start: {e}");
                        self.failed(&message);
                        return Err(BrowserError::unavailable(message));
                    }
                }
            }
        };
        let launched = match process::launch(&exe, &self.dirs, &shown).await {
            Ok(launched) => launched,
            Err(e) => {
                if let Some(display) = &display {
                    display.stop().await;
                }
                self.failed(&e);
                return Err(BrowserError::unavailable(e));
            }
        };
        let endpoint = format!("ws://127.0.0.1:{}{}", launched.port, launched.path);
        let mut connected = Err(CdpError::Closed);
        for _ in 0..20 {
            connected = CdpClient::connect(&endpoint).await;
            if connected.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let process::Launched { mut child, tree, port, path } = launched;
        let cdp = match connected {
            Ok(cdp) => cdp,
            Err(e) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                tree.sweep().await;
                if let Some(display) = &display {
                    display.stop().await;
                }
                let message = format!("the browser's debugging endpoint {endpoint} is not reachable: {e}");
                self.failed(&message);
                return Err(BrowserError::unavailable(message));
            }
        };
        life.generation += 1;
        let generation = life.generation;
        let pid = tree.pid;
        // Subscribed before the browser is asked to report its tabs.
        let events = cdp.events();
        let (exited_tx, exited) = watch::channel(false);
        {
            let mut s = self.shared();
            s.cdp = Some(cdp.clone());
            s.display = display.clone();
            s.port = Some(port);
            s.endpoint_path = path;
            s.last_error = None;
            s.last_active = Instant::now();
            s.stopping = false;
            s.restoring = true;
        }
        let tracker = tokio::spawn(self.me().track(cdp.clone(), events, pid));
        tokio::spawn(self.me().monitor(child, generation, exited_tx));
        let display = display.map(|display| {
            let keeper = tokio::spawn(self.me().keep_windows(cdp.clone(), display.clone()));
            (display, keeper)
        });
        life.proc = Some(Proc { generation, tree, display, cdp: cdp.clone(), exited, tracker, dresser: None, started: Instant::now() });

        // Before its tabs are discovered: the page that reads its identity is not one of them.
        if !self.headed() {
            let identity = match identity::Identity::read(&cdp).await {
                Ok(identity) => Some(identity),
                Err(e) => {
                    log::warn!("browser: what it is was not read ({e}); its pages say `HeadlessChrome`");
                    None
                }
            };
            let told = identity.is_some();
            match identity::dress_all(&endpoint, identity).await {
                Ok((dresser, dressed)) => {
                    life.proc.as_mut().expect("just set").dresser = Some(dresser);
                    self.shared().dressed = told.then_some(dressed);
                }
                Err(e) => log::warn!("browser: its pages are not dressed as they start: {e}"),
            }
        }
        let prepared = async {
            cdp.send(None, "Target.setDiscoverTargets", json!({ "discover": true })).await?;
            let version = cdp.send(None, "Browser.getVersion", json!({})).await?;
            let downloads = json!({
                "behavior": "allowAndName",
                "downloadPath": self.dirs.downloads.display().to_string(),
                "eventsEnabled": true,
            });
            if let Err(e) = cdp.send(None, "Browser.setDownloadBehavior", downloads).await {
                log::warn!("browser: downloads are not directed to {}: {e}", self.dirs.downloads.display());
            }
            Ok::<Value, CdpError>(version)
        }
        .await;
        let version = match prepared {
            Ok(version) => version,
            Err(e) => {
                self.stop(life, "it does not answer").await;
                let message = format!("the browser started but does not answer: {e}");
                self.failed(&message);
                return Err(BrowserError::unavailable(message));
            }
        };
        if let Some(proc) = &life.proc {
            proc.tree.note_strays();
        }
        let product = version["product"].as_str().unwrap_or_default();
        let browser_version = product.rsplit('/').next().filter(|v| !v.is_empty()).map(str::to_string);
        log::info!("browser: {product} started (pid {pid}, debugging port {port}, profile {})", self.dirs.profile.display());

        // What was open when it stopped.
        let saved = {
            let mut s = self.shared();
            s.browser_version = browser_version;
            s.groups.take_saved()
        };
        for (group_id, tabs, active) in saved {
            let mut opened = Vec::new();
            for tab in tabs {
                match self.open_in(&cdp, &group_id, &tab.url).await {
                    Ok(tab_id) => opened.push(Some(tab_id)),
                    Err(e) => {
                        log::warn!("browser: tab {} of group {group_id} was not opened again: {e}", tab.url);
                        opened.push(None);
                    }
                }
            }
            if let Some(tab_id) = active.and_then(|i| opened.get(i).cloned().flatten()) {
                let _ = self.shared().groups.activate(&tab_id);
            }
        }
        self.shared().restoring = false;
        self.bump();
        Ok(cdp)
    }

    /// End the browser: it is asked to close; if it does not, its process
    /// group is told to end; what is left then is killed and collected.
    async fn stop(&self, life: &mut Life, why: &str) {
        let Some(proc) = life.proc.take() else { return };
        self.shared().stopping = true;
        self.persist();
        proc.tree.note_strays();
        log::info!("browser: stopping (pid {}): {why}", proc.tree.pid);
        let mut exited = proc.exited.clone();
        let _ = proc.cdp.post(None, "Browser.close", json!({}));
        if !ended(&mut exited, CLOSE_WAIT).await {
            log::warn!("browser: pid {} did not end when asked to close; ending its process group", proc.tree.pid);
            proc.tree.terminate();
            ended(&mut exited, TERMINATE_WAIT).await;
        }
        proc.tree.sweep().await;
        if !ended(&mut exited, TERMINATE_WAIT).await {
            log::error!("browser: pid {} is still there", proc.tree.pid);
        }
        self.gone(&proc);
        Self::end_display(&proc).await;
    }

    async fn end_display(proc: &Proc) {
        if let Some((display, keeper)) = &proc.display {
            keeper.abort();
            display.stop().await;
        }
    }

    /// The browser of `proc` is gone: its tabs are kept to be opened again.
    fn gone(&self, proc: &Proc) {
        proc.tracker.abort();
        if let Some(dresser) = &proc.dresser {
            dresser.abort();
        }
        {
            let mut s = self.shared();
            s.groups.suspend();
            s.cdp = None;
            s.display = None;
            s.popups.clear();
            s.port = None;
            s.attaching.clear();
            s.dialogs.clear();
            s.dressed = None;
            s.stopping = false;
            s.restoring = false;
        }
        self.outside.browser_gone();
        self.bump();
        self.persist();
    }

    fn monitor(
        self: Arc<Self>,
        mut child: tokio::process::Child,
        generation: u64,
        exited: watch::Sender<bool>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let status = match child.wait().await {
                Ok(status) => status.to_string(),
                Err(e) => format!("wait: {e}"),
            };
            let _ = exited.send(true);
            self.exited(generation, status).await;
        })
    }

    /// The browser's process ended. When nothing stopped it, it crashed (or
    /// was killed): it is started again, later each time it keeps ending.
    async fn exited(&self, generation: u64, status: String) {
        let mut life = self.life.lock().await;
        if life.proc.as_ref().map(|p| p.generation) != Some(generation) {
            return;
        }
        let proc = life.proc.take().expect("checked");
        let ran = proc.started.elapsed();
        proc.tree.sweep().await;
        self.gone(&proc);
        Self::end_display(&proc).await;
        life.crashes = if ran >= STABLE_RUN { 1 } else { life.crashes + 1 };
        self.failed(&format!("the browser ended unexpectedly ({status}) after {}s", ran.as_secs()));
        loop {
            if self.closed.load(Ordering::SeqCst) {
                return;
            }
            if life.crashes > MAX_RESTARTS {
                log::error!("browser: it keeps ending; it is not started again until it is asked for");
                return;
            }
            let delay = backoff(life.crashes);
            drop(life);
            tokio::time::sleep(delay).await;
            life = self.life.lock().await;
            if life.proc.is_some() || life.generation != generation || self.closed.load(Ordering::SeqCst) {
                return;
            }
            match self.start(&mut life).await {
                Ok(_) => {
                    log::info!("browser: started again after it ended unexpectedly");
                    return;
                }
                Err(e) => {
                    log::warn!("browser: not started again: {e}");
                    life.crashes += 1;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Tabs as the browser reports them
    // ------------------------------------------------------------------

    async fn track(self: Arc<Self>, cdp: Arc<CdpClient>, mut events: broadcast::Receiver<Arc<CdpEvent>>, pid: u32) {
        loop {
            let event = tokio::select! {
                biased;
                event = events.recv() => event,
                _ = cdp.closed() => break,
            };
            match event {
                Ok(event) => self.on_event(&cdp, &event),
                Err(broadcast::error::RecvError::Lagged(lost)) => {
                    log::warn!("browser: {lost} events were lost; reading the tabs again");
                    self.resync(&cdp).await;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        if !self.shared().stopping {
            // The process's end is what starts it again.
            log::warn!("browser: the connection to pid {pid} closed; ending it");
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
    }

    fn on_event(&self, cdp: &Arc<CdpClient>, event: &CdpEvent) {
        // A browser that is closing takes its tabs away one by one: they are kept as they were.
        if self.shared().stopping {
            return;
        }
        self.outside.on_event(self, cdp, event);
        let changed = match (event.session_id.as_deref(), event.method.as_str()) {
            (None, "Target.targetCreated") => self.target_created(cdp, &event.params["targetInfo"]),
            (None, "Target.targetInfoChanged") => {
                let info = &event.params["targetInfo"];
                let (id, url, title) = (info["targetId"].as_str(), info["url"].as_str(), info["title"].as_str());
                match id {
                    Some(id) => self.shared().groups.tab_changed(id, url.unwrap_or_default(), title.unwrap_or_default()),
                    None => false,
                }
            }
            (None, "Target.targetDestroyed") => event.params["targetId"].as_str().is_some_and(|id| self.forget_tab(id)),
            (None, "Target.detachedFromTarget") => {
                if let Some(session) = event.params["sessionId"].as_str() {
                    let mut s = self.shared();
                    if let Some(tab_id) = s.groups.tab_of_session(session) {
                        s.groups.tab_attached(&tab_id, None);
                        s.attaching.remove(&tab_id);
                    }
                }
                false
            }
            (Some(session), "Page.frameStartedLoading" | "Page.frameStoppedLoading" | "Page.lifecycleEvent") => {
                let name = event.params["name"].as_str();
                let loading = match (event.method.as_str(), name) {
                    ("Page.frameStartedLoading", _) | ("Page.lifecycleEvent", Some("init")) => Some(true),
                    ("Page.frameStoppedLoading", _) | ("Page.lifecycleEvent", Some("load")) => Some(false),
                    _ => None,
                };
                let mut s = self.shared();
                match s.groups.tab_of_session(session) {
                    // The tab's own document, not a frame in it.
                    Some(tab_id) if event.params["frameId"].as_str() == Some(tab_id.as_str()) => {
                        // The document has its title by now, and the browser does not announce titles.
                        if loading == Some(false) || name == Some("DOMContentLoaded") {
                            let (me, cdp) = (self.me(), cdp.clone());
                            tokio::spawn(async move { me.refresh_tabs(&cdp).await });
                        }
                        loading.is_some_and(|loading| s.groups.set_loading(&tab_id, loading))
                    }
                    _ => false,
                }
            }
            (Some(session), "Page.javascriptDialogOpening") => {
                let known = {
                    let mut s = self.shared();
                    let tab_id = s.groups.tab_of_session(session);
                    if let Some(tab_id) = &tab_id {
                        s.dialogs.insert(tab_id.clone(), event.params.clone());
                    }
                    tab_id.is_some()
                };
                if known {
                    self.close_unattended_dialogs();
                }
                known
            }
            (Some(session), "Page.javascriptDialogClosed") => {
                let mut s = self.shared();
                s.groups.tab_of_session(session).is_some_and(|tab_id| s.dialogs.remove(&tab_id).is_some())
            }
            _ => false,
        };
        if changed {
            self.bump();
        }
    }

    fn forget_tab(&self, tab_id: &str) -> bool {
        self.outside.tab_gone(tab_id);
        let mut s = self.shared();
        s.attaching.remove(tab_id);
        s.dialogs.remove(tab_id);
        s.popups.remove(tab_id);
        if let Some(dressed) = &s.dressed {
            dressed.forget(tab_id);
        }
        s.groups.tab_removed(tab_id)
    }

    /// The JS dialogs open in a group: `(tab_id, params of Page.javascriptDialogOpening)`.
    pub fn open_dialogs(&self, group_id: &str) -> Vec<(String, Value)> {
        let s = self.shared();
        s.dialogs
            .iter()
            .filter(|(tab_id, _)| s.groups.tab(tab_id).is_some_and(|t| t.group_id == group_id))
            .map(|(tab_id, dialog)| (tab_id.clone(), dialog.clone()))
            .collect()
    }

    /// Close the JS dialogs nobody attends to: those of groups nobody looks
    /// at and no agent operates (an agent's Playwright answers the dialogs
    /// of the pages it works on). A dialog left open blocks its page, and
    /// every Playwright that connects to the browser, whatever its group.
    /// `beforeunload` is accepted (the page may leave), the others are
    /// dismissed.
    pub fn close_unattended_dialogs(&self) {
        let now = now_ms();
        let (cdp, closing) = {
            let mut s = self.shared();
            let Some(cdp) = s.cdp.clone() else { return };
            let mut closing = Vec::new();
            for (tab_id, dialog) in &s.dialogs {
                let Some(tab) = s.groups.tab(tab_id) else { continue };
                let attended = s.viewers.iter().any(|(_, group)| *group == tab.group_id)
                    || s.groups.controller(&tab.group_id, now) == Controller::Agent;
                if let (false, Some(session)) = (attended, tab.session) {
                    closing.push((tab_id.clone(), session, dialog["type"] == "beforeunload"));
                }
            }
            for (tab_id, _, _) in &closing {
                s.dialogs.remove(tab_id);
            }
            (cdp, closing)
        };
        if closing.is_empty() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        runtime.spawn(async move {
            for (tab_id, session, accept) in closing {
                match cdp.send(Some(&session), "Page.handleJavaScriptDialog", json!({ "accept": accept })).await {
                    Ok(_) => log::info!("browser: a dialog of tab {tab_id} nobody attends to was closed"),
                    Err(e) => log::debug!("browser: dialog of tab {tab_id} not closed: {e}"),
                }
            }
        });
    }

    /// Read every tab's address and title from the browser. Titles are not
    /// announced (a headless window has no tab strip to update): they are
    /// read when a document is loaded, while someone looks, and before a
    /// group is shown or handed over.
    async fn refresh_tabs(&self, cdp: &CdpClient) {
        let Ok(targets) = cdp.send(None, "Target.getTargets", json!({})).await else { return };
        let mut changed = false;
        {
            let mut s = self.shared();
            if s.stopping {
                return;
            }
            for info in targets["targetInfos"].as_array().into_iter().flatten().filter(|i| i["type"] == "page") {
                if let (Some(tab_id), Some(url), Some(title)) = (info["targetId"].as_str(), info["url"].as_str(), info["title"].as_str()) {
                    changed |= s.groups.tab_changed(tab_id, url, title);
                }
            }
        }
        if changed {
            self.bump();
        }
    }

    async fn refresh_running(&self) {
        if let Some(cdp) = self.cdp() {
            self.refresh_tabs(&cdp).await;
        }
    }

    /// A target appeared. A page is a tab: it gets its group, and the
    /// module attaches to it.
    fn target_created(&self, cdp: &Arc<CdpClient>, info: &Value) -> bool {
        let (Some("page"), Some(tab_id)) = (info["type"].as_str(), info["targetId"].as_str()) else { return false };
        let (url, title) = (info["url"].as_str().unwrap_or_default(), info["title"].as_str().unwrap_or_default());
        let mut s = self.shared();
        let mut changed = false;
        if s.groups.tab(tab_id).is_some() {
            changed = s.groups.tab_changed(tab_id, url, title);
        } else {
            let expected = s.expected.last().map(|(_, group)| group.clone());
            let group_id = s.groups.place(info["openerId"].as_str(), expected.as_deref());
            if expected.is_none() && !groups::is_internal(&group_id) && !s.groups.has_room() {
                log::warn!("browser: tab {tab_id} ({url}) is one more than max_tabs = {}; closing it", s.groups.max_tabs());
                let (cdp, tab_id) = (cdp.clone(), tab_id.to_string());
                tokio::spawn(async move {
                    let _ = cdp.send(None, "Target.closeTarget", json!({ "targetId": tab_id })).await;
                });
                return false;
            }
            changed |= s.groups.tab_seen(tab_id, &group_id, url, title);
            if info["openerId"].as_str().is_some_and(|o| !o.is_empty()) {
                s.popups.insert(tab_id.to_string());
            }
        }
        if s.attaching.insert(tab_id.to_string()) {
            tokio::spawn(self.me().attach(cdp.clone(), tab_id.to_string()));
        }
        changed
    }

    /// Attach to a tab with a flat session: its loading state is followed,
    /// and it renders whether or not its window is in front (the screen of
    /// one group and the agent in another must not hide each other).
    async fn attach(self: Arc<Self>, cdp: Arc<CdpClient>, tab_id: String) {
        let attached = cdp.send(None, "Target.attachToTarget", json!({ "targetId": tab_id, "flatten": true })).await;
        let session = match attached.as_ref().map(|v| v["sessionId"].as_str()) {
            Ok(Some(session)) => session.to_string(),
            other => {
                log::debug!("browser: tab {tab_id} was not attached: {other:?}");
                self.shared().attaching.remove(&tab_id);
                return;
            }
        };
        self.shared().groups.tab_attached(&tab_id, Some(&session));
        self.bump();
        let mut setup = vec![("Page.enable", json!({})), ("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))];
        // A headless window is never in front: its page is told it has the focus. A headed one has what the display gives it.
        if !self.headed() {
            setup.push(("Emulation.setFocusEmulationEnabled", json!({ "enabled": true })));
        }
        for (method, params) in setup {
            if let Err(e) = cdp.send(Some(&session), method, params).await {
                log::debug!("browser: {method} of tab {tab_id}: {e}");
            }
        }
        if let Some(display) = self.display() {
            let size = { let p = *display.painting().borrow(); (p.width, p.height) };
            let popup = self.shared().popups.contains(&tab_id);
            fit_window(&cdp, &tab_id, size, popup).await;
        }
        self.outside.instrument(&cdp, &tab_id, &session, self.headed()).await;
    }

    /// Keep a headed browser's windows on its display: a tab's window has
    /// the display's size (a popup keeps its own, within the display); the
    /// group's current tab is the window in front, also after a window of
    /// the module's own (a reflow's) opened over it.
    async fn keep_windows(self: Arc<Self>, cdp: Arc<CdpClient>, display: Arc<Display>) {
        let mut changes = self.changes();
        let mut painting = display.painting();
        let mut size = { let p = *painting.borrow(); (p.width, p.height) };
        let (mut raised, mut known): (Option<String>, Vec<String>) = (None, Vec::new());
        loop {
            let (active, all) = {
                let s = self.shared();
                (s.groups.active_tab(DEFAULT_GROUP), s.groups.tab_ids())
            };
            if active != raised || (all != known && active.is_some()) {
                if let Some(tab_id) = &active {
                    display.tab.send_replace(tab_id.clone());
                    if let Err(e) = cdp.send(None, "Target.activateTarget", json!({ "targetId": tab_id })).await {
                        log::debug!("browser: tab {tab_id} not brought to the front: {e}");
                    }
                }
                raised = active;
            }
            known = all;
            tokio::select! {
                changed = changes.changed() => if changed.is_err() { return },
                changed = painting.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let p = *painting.borrow_and_update();
                    if (p.width, p.height) != size {
                        size = (p.width, p.height);
                        let (tabs, popups) = {
                            let s = self.shared();
                            (s.groups.tab_ids(), s.popups.clone())
                        };
                        for tab_id in tabs {
                            fit_window(&cdp, &tab_id, size, popups.contains(&tab_id)).await;
                        }
                    }
                }
                _ = cdp.closed() => return,
            }
        }
    }

    /// Events were lost: the tabs are what the browser says they are now.
    async fn resync(&self, cdp: &Arc<CdpClient>) {
        let Ok(targets) = cdp.send(None, "Target.getTargets", json!({})).await else { return };
        let infos = targets["targetInfos"].as_array().cloned().unwrap_or_default();
        let live: Vec<&str> = infos.iter().filter(|i| i["type"] == "page").filter_map(|i| i["targetId"].as_str()).collect();
        let stale: Vec<String> = self.shared().groups.tab_ids().into_iter().filter(|id| !live.contains(&id.as_str())).collect();
        for tab_id in &stale {
            self.forget_tab(tab_id);
        }
        for info in &infos {
            self.target_created(cdp, info);
        }
        self.bump();
    }

    // ------------------------------------------------------------------
    // Housekeeping
    // ------------------------------------------------------------------

    /// Leases that ran out, the browser nobody uses, `groups.json`.
    pub async fn run(self: Arc<Self>) {
        let mut ticks = 0u64;
        loop {
            tokio::time::sleep(self.tuning.tick).await;
            ticks += 1;
            self.housekeep(ticks).await;
        }
    }

    fn idle(&self) -> bool {
        let mut s = self.shared();
        if !s.viewers.is_empty() || s.groups.has_lease(now_ms()) || self.reflow.working() {
            s.last_active = Instant::now();
        }
        s.cdp.is_some() && s.last_active.elapsed() >= Duration::from_secs(self.cfg.idle_stop_secs)
    }

    async fn housekeep(&self, ticks: u64) {
        let (expired, watched) = {
            let mut s = self.shared();
            (s.groups.expire(now_ms()), !s.viewers.is_empty())
        };
        if expired {
            self.bump();
        }
        self.cut_agent();
        self.close_unattended_dialogs();
        if watched {
            self.refresh_running().await;
            if ticks % WAITING_EVERY == 0 {
                self.refresh_waiting().await;
            }
        }
        if self.idle() {
            let mut life = self.life.lock().await;
            if self.idle() {
                self.stop(&mut life, &format!("nobody used it for {}s", self.cfg.idle_stop_secs)).await;
            }
        }
        self.persist();
    }

    /// The agent's browser conversation (`browser.bind_default`).
    pub fn default_session(&self) -> Option<String> {
        self.shared().groups.default_session().map(str::to_string)
    }

    /// Whether the agent waits in the browser conversation for an answer of the user.
    async fn refresh_waiting(&self) {
        let waiting = sessions::waiting_answer(self.deps.agent.as_ref(), self.default_session().as_deref()).await;
        if self.shared().groups.set_waiting_answer(waiting) {
            self.bump();
        }
    }

    /// Write `groups.json` when what it holds changed.
    fn persist(&self) {
        let json = {
            let mut s = self.shared();
            if s.restoring {
                return;
            }
            let json = s.groups.to_json();
            if json == s.written {
                return;
            }
            s.written = json.clone();
            json
        };
        let write = || {
            let dir = self.groups_file.parent().expect("groups.json has a directory");
            std::fs::create_dir_all(dir)?;
            let tmp = self.groups_file.with_extension("json.tmp");
            std::fs::write(&tmp, json)?;
            std::fs::rename(&tmp, &self.groups_file)
        };
        if let Err(e) = write() {
            log::warn!("browser: {} not written: {e}", self.groups_file.display());
        }
    }

    /// Stop the browser for good (the service ends).
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut life = self.life.lock().await;
        self.stop(&mut life, "the Agent Service ends").await;
    }

    // ------------------------------------------------------------------
    // browser.*
    // ------------------------------------------------------------------

    /// `browser.status`: the browser and its one group.
    pub async fn status(&self) -> Value {
        if self.cfg.enabled {
            self.refresh_waiting().await;
            self.refresh_running().await;
        }
        let chromium = self.chromium();
        let available = matches!(chromium, Chromium::Ready(_));
        // What the package says it brings, whether or not the browser is downloaded.
        let package = chromium
            .launcher()
            .filter(|_| self.cfg.enabled)
            .map(|launcher| process::package_versions(launcher).unwrap_or(Value::Null));
        let usable = self.usable_with(chromium);
        let now = now_ms();
        let s = self.shared();
        let running = s.cdp.is_some();
        let (unavailable, unavailable_code) = match usable {
            Err(e) => (Some(e.message), e.unavailable_code),
            Ok(_) if !running => (s.last_error.clone(), None),
            Ok(_) => (None, None),
        };
        let version = package.map(|package| {
            json!({
                "browser": s.browser_version.clone().map(Value::from).unwrap_or_else(|| package["chromium"].clone()),
                "playwright": package["playwright"],
            })
        });
        json!({
            "enabled": self.cfg.enabled,
            "available": available,
            "display": if self.headed() { "virtual" } else { "headless" },
            "device_scale_factor": if self.headed() { Some(self.scale()) } else { None },
            "running": running,
            "unavailable": unavailable,
            "unavailable_code": unavailable_code,
            "version": version,
            "default_session_id": s.groups.default_session(),
            "groups": [s.groups.group(DEFAULT_GROUP, now)],
        })
    }

    /// `browser.acquire`: a lease of `session_id` on the browser.
    pub async fn acquire(&self, session_id: &str) -> Result<Acquired, BrowserError> {
        self.usable()?;
        // Refused before the browser is started for nothing.
        if self.group(DEFAULT_GROUP).user_hold {
            return Err(BrowserError::user_in_control());
        }
        let cdp = self.ensure().await?;
        self.refresh_tabs(&cdp).await;
        let now = now_ms();
        let (lease, group, port, ttl_ms) = {
            let mut s = self.shared();
            let lease = s.groups.acquire(DEFAULT_GROUP, session_id, now)?;
            s.last_active = Instant::now();
            (lease, s.groups.group(DEFAULT_GROUP, now), s.port, s.groups.lease_ttl_ms())
        };
        self.bump();
        let port = port.ok_or_else(|| BrowserError::unavailable("the browser stopped"))?;
        // Through this service, where the lease's end closes the connection; straight to the browser where there is no service to go through (tests).
        let cdp_url = match std::env::var(ENV_BROWSER_RPC).ok().filter(|rpc| !rpc.is_empty()) {
            Some(rpc) => format!("{}/browser/cdp/{}", rpc.trim_end_matches('/'), lease.lease_id),
            None => format!("http://127.0.0.1:{port}"),
        };
        Ok(Acquired {
            lease_id: lease.lease_id,
            group_id: DEFAULT_GROUP.to_string(),
            ttl_ms,
            expires_at_ms: lease.expires_at_ms,
            cdp_url,
            active_tab_id: group.tabs.iter().find(|t| t.active).map(|t| t.tab_id.clone()),
            tabs: group.tabs,
        })
    }

    /// Close the agent's connections whose lease ended: from now on no
    /// command of what it was doing reaches the browser.
    pub fn cut_agent(&self) {
        let now = now_ms();
        let cut = self.proxies.cut(|lease_id| self.shared().groups.lease_valid(lease_id, now));
        if cut > 0 {
            log::info!("browser: {cut} connection(s) of the agent to the browser closed: their lease ended");
        }
    }

    /// Whether `lease_id` is a lease of the agent that has not ended.
    pub fn lease_valid(&self, lease_id: &str) -> bool {
        self.shared().groups.lease_valid(lease_id, now_ms())
    }

    /// `ws://…` of the running browser's DevTools endpoint, and its version.
    pub fn endpoint(&self) -> Option<(String, Option<String>)> {
        let s = self.shared();
        s.port.map(|port| (format!("ws://127.0.0.1:{port}{}", s.endpoint_path), s.browser_version.clone()))
    }

    /// `browser.renew`: the lease's new end.
    pub fn renew(&self, lease_id: &str) -> Result<u64, BrowserError> {
        self.usable()?;
        let mut s = self.shared();
        s.last_active = Instant::now();
        s.groups.renew(lease_id, now_ms())
    }

    /// `browser.release`.
    pub fn release(&self, lease_id: &str, state: Option<AgentState>, note: Option<String>) -> Result<(), BrowserError> {
        self.usable()?;
        let note = note.filter(|n| !n.trim().is_empty());
        self.shared().groups.release(lease_id, state.map(|state| (state, note)), now_ms())?;
        self.bump();
        self.cut_agent();
        self.close_unattended_dialogs();
        Ok(())
    }

    /// `browser.request_handover`: the user is asked to take the browser over.
    pub async fn request_handover(&self, reason: &str, note: Option<String>) -> Result<(), BrowserError> {
        self.usable()?;
        let note = note.filter(|n| !n.trim().is_empty());
        self.shared().groups.request_handover(DEFAULT_GROUP, reason, note, now_ms());
        self.bump();
        self.cut_agent();
        Ok(())
    }

    /// `browser.take_over`, `browser.hand_back`, `browser.hand_to_agent`:
    /// the browser changes hands and the browser conversation is told.
    pub async fn hand(&self, who: &str, what: Handed) -> Result<Group, BrowserError> {
        self.usable()?;
        self.refresh_running().await;
        let (group, page) = {
            let mut s = self.shared();
            match what {
                Handed::TakeOver => s.groups.take_over(DEFAULT_GROUP),
                Handed::HandBack | Handed::HandToAgent => s.groups.hand_back(DEFAULT_GROUP),
            }
            s.last_active = Instant::now();
            let page = s.groups.active_tab(DEFAULT_GROUP).and_then(|id| s.groups.tab(&id));
            (s.groups.group(DEFAULT_GROUP, now_ms()), page)
        };
        self.bump();
        self.cut_agent();
        let event = sessions::record(what, page.as_ref().map(|t| (t.url.as_str(), t.title.as_str())));
        let (agent, default_session) = (self.deps.agent.as_ref(), self.default_session());
        if let Some(sid) = sessions::post_record(agent, self.deps.owner_did.as_deref(), default_session.as_deref(), who, event).await {
            if let Some(supervisor) = &self.deps.supervisor {
                match supervisor.ensure_task(&sid, "browser").await {
                    Ok(()) | Err(OpenDanError::NotDriver { .. }) => {}
                    Err(e) => log::warn!("browser: session {sid} not hosted: {e}"),
                }
            }
        }
        Ok(group)
    }

    /// `browser.bind_default`: the conversation `session_id` is the agent's
    /// browser conversation from now on. Returns whether that changed.
    pub fn bind_default(&self, session_id: &str) -> Result<bool, BrowserError> {
        self.usable()?;
        let changed = self.shared().groups.bind_default(session_id);
        if changed {
            log::info!("browser: the browser conversation is {session_id}");
            self.bump();
            self.persist();
        }
        Ok(changed)
    }

    /// `browser.clear_profile`: stop the browser and delete its logins,
    /// site data and cache. The tabs are opened again at the next start.
    pub async fn clear_profile(&self) -> Result<(), BrowserError> {
        self.usable()?;
        let mut life = self.life.lock().await;
        self.stop(&mut life, "its profile is cleared").await;
        for dir in [&self.dirs.profile, &self.dirs.cache] {
            match std::fs::remove_dir_all(dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(BrowserError::new("other", format!("remove {}: {e}", dir.display()))),
            }
        }
        log::info!("browser: profile {} cleared", self.dirs.profile.display());
        Ok(())
    }
}

/// Give a tab's window the display's size `(width, height)` (CSS pixels),
/// or keep a popup's own size within it.
async fn fit_window(cdp: &CdpClient, tab_id: &str, (width, height): (u32, u32), popup: bool) {
    let Ok(window) = cdp.send(None, "Browser.getWindowForTarget", json!({ "targetId": tab_id })).await else { return };
    let Some(window_id) = window["windowId"].as_i64() else { return };
    let bounds = &window["bounds"];
    let side = |name: &str| bounds[name].as_i64().unwrap_or(0);
    let (w, h) = (width as i64, height as i64);
    let wanted = if popup {
        let (pw, ph) = (side("width").clamp(1, w), side("height").clamp(1, h));
        (side("left").clamp(0, w - pw), side("top").clamp(0, h - ph), pw, ph)
    } else {
        (0, 0, w, h)
    };
    if bounds["windowState"] == "normal" && (side("left"), side("top"), side("width"), side("height")) == wanted {
        return;
    }
    if bounds["windowState"] != "normal" {
        let _ = cdp.send(None, "Browser.setWindowBounds", json!({ "windowId": window_id, "bounds": { "windowState": "normal" } })).await;
    }
    let (left, top, w, h) = wanted;
    let bounds = json!({ "left": left, "top": top, "width": w, "height": h });
    if let Err(e) = cdp.send(None, "Browser.setWindowBounds", json!({ "windowId": window_id, "bounds": bounds })).await {
        log::debug!("browser: the window of tab {tab_id} was not fitted to the display: {e}");
    }
}
