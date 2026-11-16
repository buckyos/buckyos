//! What a page shows outside itself (Agent Browser Protocol §3.4): the list
//! of a `<select>`, JS dialogs, file choosers, downloads, permission
//! requests, and which element has the focus. The module hears of them from
//! the browser and from a script it puts into every document; the screens
//! show them and answer. Requests that wait for an answer are kept here, not
//! in a connection, so a screen that reconnects gets them again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{broadcast, watch};

use super::cdp::{CdpClient, CdpError, CdpEvent};
use super::groups::Controller;
use super::rpc::BrowserError;
use super::{BrowserDirs, BrowserModule};

/// The function the page script tells the module through.
const BINDING: &str = "__opendanScreen";
/// The world the page script runs in: page scripts do not see it.
const WORLD: &str = "opendan_screen";
const PAGE_SCRIPT: &str = include_str!("screen_page.js");
const PERMISSION_SCRIPT: &str = include_str!("screen_permissions.js");
/// A command that sets a tab up or answers a request is not waited for
/// longer: a page with an open JS dialog answers nothing until it is closed.
const COMMAND_WAIT: Duration = Duration::from_secs(5);
/// What a tab answers while a navigation gives it another renderer.
const NO_ACTIVE_PAGE: &str = "Not attached to an active page";
const SETTLE_TRIES: usize = 10;
const SETTLE_WAIT: Duration = Duration::from_millis(200);
const MAX_PENDING: usize = 64;
const MAX_DOWNLOADS: usize = 64;
const DOWNLOAD_KEPT: Duration = Duration::from_secs(24 * 3600);
const MAX_UPLOAD_FILES: usize = 32;
/// How long the files handed to a page are kept: the page reads them when
/// it sends them, which may be long after they were chosen.
const UPLOAD_KEPT: Duration = Duration::from_secs(3600);
const PERMISSION_QUIET: Duration = Duration::from_secs(60);
/// What a page would have to ask the user for. The browser refuses them all:
/// nobody sits in front of it to answer.
const REFUSED_PERMISSIONS: &[&str] = &[
    "geolocation",
    "notifications",
    "camera",
    "microphone",
    "midi",
    "clipboard-read",
    "idle-detection",
    "window-management",
    "local-fonts",
    "nfc",
    "storage-access",
    "top-level-storage-access",
    "display-capture",
    "payment-handler",
    "captured-surface-control",
    "speaker-selection",
];

/// What the screens are told.
#[derive(Debug, Clone)]
pub enum Notice {
    /// A `request` message (without `type`) for the screens of `group_id`
    /// (`None`: the page it came from is not known, every screen shows it).
    Request { group_id: Option<String>, body: Value },
    /// A request is over: answered, timed out, or its page is gone.
    Closed { request_id: String },
    /// A `focus` message (without `type`) of a tab.
    Focus { tab_id: String, body: Value },
}

enum Kind {
    /// The list of a `<select>`: where the page script keeps the element.
    Select { context: i64, token: u64 },
    Dialog { dialog_type: String },
    /// A file input: `taken` files are staged or on their way, `serial`
    /// numbers their directories.
    FileChooser { node: i64, multiple: bool, taken: usize, serial: usize, files: Vec<PathBuf> },
}

impl Kind {
    fn name(&self) -> &'static str {
        match self {
            Kind::Select { .. } => "select",
            Kind::Dialog { .. } => "dialog",
            Kind::FileChooser { .. } => "file_chooser",
        }
    }
}

struct Pending {
    request_id: String,
    tab_id: String,
    body: Value,
    kind: Kind,
}

/// A file the browser downloaded.
#[derive(Debug, Clone)]
pub struct Downloaded {
    pub download_id: String,
    pub name: String,
    pub size: u64,
    pub path: PathBuf,
}

struct Download {
    file: Downloaded,
    tab_id: Option<String>,
    completed: bool,
    at: Instant,
}

#[derive(Default)]
struct State {
    pending: Vec<Pending>,
    downloads: VecDeque<Download>,
    /// What the focused element of each tab is, as last told.
    focus: HashMap<String, Value>,
    /// Upload directories handed to a page: `(tab_id, directory, when)`.
    handed: Vec<(String, PathBuf, Instant)>,
    /// Permission notices already shown: `(tab_id, permission, origin)`.
    noticed: HashMap<(String, String, String), Instant>,
    /// The browser connection whose permissions were refused.
    refused_for: usize,
    /// Tabs that are prepared (`Outside::instrument` is through with them).
    prepared: HashSet<String>,
}

pub struct Outside {
    state: Mutex<State>,
    /// Bumped when a tab is prepared.
    prepared: watch::Sender<u64>,
    notices: broadcast::Sender<Notice>,
    seq: AtomicU64,
    dialog_wait: Duration,
    downloads_dir: PathBuf,
    uploads_dir: PathBuf,
}

/// Send a command that sets a tab up. A tab between two renderers refuses
/// it: it is sent again once the tab has settled.
pub(super) async fn send_settled(cdp: &CdpClient, session: &str, method: &str, params: Value) -> Result<Value, CdpError> {
    let mut tries = 0;
    loop {
        match cdp.send_within(Some(session), method, params.clone(), COMMAND_WAIT).await {
            Err(CdpError::Remote { message, .. }) if message.contains(NO_ACTIVE_PAGE) && tries < SETTLE_TRIES => {
                tries += 1;
                tokio::time::sleep(SETTLE_WAIT).await;
            }
            answer => return answer,
        }
    }
}

fn no_such_request(request_id: &str) -> BrowserError {
    BrowserError::new("no_such_request", format!("no open request `{request_id}`"))
}

/// The name a staged file gets: what the page sees as the file's name.
fn file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    while cleaned.len() > 200 {
        cleaned.pop();
    }
    match cleaned.trim() {
        "" | "." | ".." => "file".to_string(),
        name => name.to_string(),
    }
}

/// `accept` of a file input as the request carries it.
fn accept_of(described: &Value) -> Vec<String> {
    let attributes = described["node"]["attributes"].as_array().cloned().unwrap_or_default();
    attributes
        .chunks(2)
        .find(|pair| pair[0] == "accept")
        .and_then(|pair| pair.get(1)?.as_str().map(str::to_string))
        .map(|accept| accept.split(',').map(|part| part.trim().to_string()).filter(|part| !part.is_empty()).collect())
        .unwrap_or_default()
}

fn rect_of(value: &Value) -> Option<Value> {
    let side = |name: &str| value[name].as_f64().filter(|n| n.is_finite()).map(|n| n.round() as i64);
    Some(json!({ "x": side("x")?, "y": side("y")?, "width": side("width")?, "height": side("height")? }))
}

impl Outside {
    pub(super) fn new(dirs: &BrowserDirs, dialog_wait: Duration) -> Self {
        let uploads_dir = dirs.root.join("uploads");
        // What an earlier service staged belongs to pages that are gone.
        let _ = std::fs::remove_dir_all(&uploads_dir);
        Self {
            state: Mutex::new(State::default()),
            prepared: watch::channel(0).0,
            notices: broadcast::channel(256).0,
            seq: AtomicU64::new(0),
            dialog_wait,
            downloads_dir: dirs.downloads.clone(),
            uploads_dir,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("requests")
    }

    fn tell(&self, notice: Notice) {
        let _ = self.notices.send(notice);
    }

    fn next_id(&self) -> String {
        format!("r{:x}-{}", super::now_ms(), self.seq.fetch_add(1, Ordering::SeqCst) + 1)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Notice> {
        self.notices.subscribe()
    }

    /// The requests of a group's tabs that wait for an answer.
    pub fn pending(&self, module: &BrowserModule, group_id: &str) -> Vec<Value> {
        let pending: Vec<(String, Value)> = self.state().pending.iter().map(|p| (p.tab_id.clone(), p.body.clone())).collect();
        pending.into_iter().filter(|(tab_id, _)| module.tab(tab_id).is_some_and(|tab| tab.group_id == group_id)).map(|(_, body)| body).collect()
    }

    /// What the focused element of a tab is (a `focus` message without `type`).
    pub fn focus(&self, tab_id: &str) -> Value {
        self.state().focus.get(tab_id).cloned().unwrap_or_else(|| json!({ "tab_id": tab_id, "editable": false }))
    }

    /// Someone looks at `group_id` now: the JS dialogs that are open in it
    /// are theirs to answer.
    pub fn attend(&self, module: &BrowserModule, group_id: &str) {
        if module.headed() {
            return;
        }
        for (tab_id, dialog) in module.open_dialogs(group_id) {
            let known = self.state().pending.iter().any(|p| p.tab_id == tab_id && matches!(p.kind, Kind::Dialog { .. }));
            if !known {
                self.raise_dialog(module, &tab_id, &dialog);
            }
        }
    }

    // ------------------------------------------------------------------
    // Requests
    // ------------------------------------------------------------------

    /// Keep a request and show it. A tab has one request of a kind at a
    /// time: an older one is over.
    fn raise(&self, module: &BrowserModule, tab_id: &str, mut body: Value, kind: Kind) -> String {
        let request_id = self.next_id();
        body["request_id"] = json!(request_id);
        body["kind"] = json!(kind.name());
        body["tab_id"] = json!(tab_id);
        let over: Vec<Pending> = {
            let mut s = self.state();
            let mut over = Vec::new();
            let mut i = 0;
            while i < s.pending.len() {
                if s.pending[i].tab_id == tab_id && s.pending[i].kind.name() == kind.name() {
                    over.push(s.pending.remove(i));
                } else {
                    i += 1;
                }
            }
            while s.pending.len() >= MAX_PENDING {
                over.push(s.pending.remove(0));
            }
            s.pending.push(Pending { request_id: request_id.clone(), tab_id: tab_id.to_string(), body: body.clone(), kind });
            over
        };
        for request in over {
            self.ended(request);
        }
        self.tell(Notice::Request { group_id: module.tab(tab_id).map(|tab| tab.group_id), body });
        request_id
    }

    /// A request that was taken off the list is over.
    fn ended(&self, request: Pending) {
        if let Kind::FileChooser { .. } = request.kind {
            let _ = std::fs::remove_dir_all(self.uploads_dir.join(&request.request_id));
        }
        self.tell(Notice::Closed { request_id: request.request_id });
    }

    fn take(&self, request_id: &str) -> Option<Pending> {
        let mut s = self.state();
        let at = s.pending.iter().position(|p| p.request_id == request_id)?;
        Some(s.pending.remove(at))
    }

    fn close_where(&self, over: impl Fn(&Pending) -> bool) {
        let closed: Vec<Pending> = {
            let mut s = self.state();
            let (closed, kept) = std::mem::take(&mut s.pending).into_iter().partition(|p| over(p));
            s.pending = kept;
            closed
        };
        for request in closed {
            self.ended(request);
        }
    }

    fn raise_dialog(&self, module: &BrowserModule, tab_id: &str, dialog: &Value) {
        let dialog_type = dialog["type"].as_str().unwrap_or("alert").to_string();
        let mut body = json!({ "dialog_type": dialog_type, "message": dialog["message"].as_str().unwrap_or_default() });
        if dialog_type == "prompt" {
            body["default_prompt"] = json!(dialog["defaultPrompt"].as_str().unwrap_or_default());
        }
        let request_id = self.raise(module, tab_id, body, Kind::Dialog { dialog_type });
        let (module, wait) = (module.me(), self.dialog_wait);
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            module.outside.unanswered(&module, &request_id).await;
        });
    }

    /// Nobody answered a dialog in time: it is closed as one nobody attends
    /// to (`beforeunload` lets the page leave, the others are dismissed).
    async fn unanswered(&self, module: &BrowserModule, request_id: &str) {
        let Some(request) = self.take(request_id) else { return };
        if let Kind::Dialog { dialog_type } = &request.kind {
            log::info!("browser: nobody answered the {dialog_type} dialog of tab {}; closing it", request.tab_id);
            let _ = self.answer_dialog(module, &request.tab_id, dialog_type == "beforeunload", None).await;
        }
        self.ended(request);
    }

    async fn answer_dialog(&self, module: &BrowserModule, tab_id: &str, accept: bool, prompt_text: Option<&str>) -> Result<(), BrowserError> {
        let (cdp, session) = module.tab_session(tab_id).await?;
        let mut params = json!({ "accept": accept });
        if let Some(text) = prompt_text {
            params["promptText"] = json!(text);
        }
        cdp.send_within(Some(&session), "Page.handleJavaScriptDialog", params, COMMAND_WAIT).await?;
        Ok(())
    }

    /// `respond`: the answer to a request (§3.4).
    pub async fn respond(&self, module: &BrowserModule, request_id: &str, result: &Value) -> Result<(), BrowserError> {
        let request = self.take(request_id).ok_or_else(|| no_such_request(request_id))?;
        let cancelled = result["cancel"].as_bool().unwrap_or(false);
        let answered = match &request.kind {
            Kind::Dialog { .. } => {
                let accept = !cancelled && result["accept"].as_bool().unwrap_or(false);
                self.answer_dialog(module, &request.tab_id, accept, result["prompt_text"].as_str().filter(|_| accept)).await
            }
            Kind::Select { .. } if cancelled => Ok(()),
            Kind::Select { context, token } => {
                let selected: Vec<u64> = result["selected"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
                let expression = format!("globalThis.__opendanSelect({token}, {})", json!(selected));
                match module.tab_session(&request.tab_id).await {
                    Ok((cdp, session)) => cdp
                        .send_within(Some(&session), "Runtime.evaluate", json!({ "expression": expression, "contextId": context }), COMMAND_WAIT)
                        .await
                        .map(|_| ())
                        .map_err(BrowserError::from),
                    Err(e) => Err(e),
                }
            }
            // Chosen files arrive as uploads; an answer only says that none will.
            Kind::FileChooser { .. } => Ok(()),
        };
        self.ended(request);
        answered
    }

    // ------------------------------------------------------------------
    // What the browser tells
    // ------------------------------------------------------------------

    /// Prepare a tab the module attached to: the page script in every
    /// document (the open one too), file choosers intercepted.
    pub(super) async fn instrument(&self, cdp: &Arc<CdpClient>, tab_id: &str, session: &str, headed: bool) {
        // A headed browser asks the user for permissions itself, on the display.
        if !headed {
            self.refuse_permissions(cdp);
        }
        let page_script = if headed { format!("globalThis.__opendanHeaded = true;\n{PAGE_SCRIPT}") } else { PAGE_SCRIPT.to_string() };
        let run = |method: &'static str, params: Value| async move {
            let answer = send_settled(cdp, session, method, params).await;
            if let Err(e) = &answer {
                log::debug!("browser: {method} of tab {tab_id}: {e}");
            }
            answer.ok()
        };
        // A binding only exists while the runtime domain is on.
        run("Runtime.enable", json!({})).await;
        run("Runtime.addBinding", json!({ "name": BINDING, "executionContextName": WORLD })).await;
        run("Page.addScriptToEvaluateOnNewDocument", json!({ "source": page_script, "worldName": WORLD })).await;
        if !headed {
            run("Page.addScriptToEvaluateOnNewDocument", json!({ "source": PERMISSION_SCRIPT })).await;
        }
        run("Page.setInterceptFileChooserDialog", json!({ "enabled": true })).await;
        // The document that is already there.
        if !headed {
            run("Runtime.evaluate", json!({ "expression": PERMISSION_SCRIPT })).await;
        }
        let tree = run("Page.getFrameTree", json!({})).await.unwrap_or(Value::Null);
        let mut frames = vec![tree["frameTree"].clone()];
        while let Some(frame) = frames.pop() {
            frames.extend(frame["childFrames"].as_array().cloned().unwrap_or_default());
            let Some(frame_id) = frame["frame"]["id"].as_str() else { continue };
            let Some(world) = run("Page.createIsolatedWorld", json!({ "frameId": frame_id, "worldName": WORLD })).await else { continue };
            if let Some(context) = world["executionContextId"].as_i64() {
                run("Runtime.evaluate", json!({ "expression": page_script, "contextId": context })).await;
            }
        }
        self.state().prepared.insert(tab_id.to_string());
        self.prepared.send_modify(|n| *n += 1);
    }

    /// Returns once a tab is prepared: the documents it loads from then on
    /// have the page script from their start. Not waited for longer than
    /// the commands that prepare it are.
    pub async fn prepared(&self, tab_id: &str) {
        let mut changes = self.prepared.subscribe();
        let _ = tokio::time::timeout(COMMAND_WAIT, async {
            while !self.state().prepared.contains(tab_id) && changes.changed().await.is_ok() {}
        })
        .await;
    }

    /// Once per browser: every permission a page could ask the user for is refused.
    fn refuse_permissions(&self, cdp: &Arc<CdpClient>) {
        let browser = Arc::as_ptr(cdp) as usize;
        if std::mem::replace(&mut self.state().refused_for, browser) == browser {
            return;
        }
        let cdp = cdp.clone();
        tokio::spawn(async move {
            let mut permissions: Vec<Value> = REFUSED_PERMISSIONS.iter().map(|name| json!({ "name": name })).collect();
            permissions.push(json!({ "name": "midi", "sysex": true }));
            permissions.push(json!({ "name": "push", "userVisibleOnly": true }));
            for permission in permissions {
                if let Err(e) = cdp.send(None, "Browser.setPermission", json!({ "permission": permission, "setting": "denied" })).await {
                    log::debug!("browser: permission {permission} not refused: {e}");
                }
            }
        });
    }

    pub(super) fn on_event(&self, module: &BrowserModule, cdp: &Arc<CdpClient>, event: &CdpEvent) {
        match (event.session_id.as_deref(), event.method.as_str()) {
            (Some(session), "Runtime.bindingCalled") if event.params["name"] == BINDING => {
                let (Some(tab), Some(payload)) = (module.tab_of_session(session), event.params["payload"].as_str()) else { return };
                let Ok(told) = serde_json::from_str::<Value>(payload) else { return };
                self.told_by_page(module, &tab.tab_id, &tab.group_id, event.params["executionContextId"].as_i64().unwrap_or(0), &told);
            }
            (Some(session), "Page.javascriptDialogOpening") => {
                let Some(tab) = module.tab_of_session(session) else { return };
                // A headed browser shows its dialog on the display.
                if module.watched(&tab.group_id) && !module.headed() {
                    self.raise_dialog(module, &tab.tab_id, &event.params);
                }
            }
            (Some(session), "Page.javascriptDialogClosed") => {
                let Some(tab) = module.tab_of_session(session) else { return };
                self.close_where(|p| p.tab_id == tab.tab_id && matches!(p.kind, Kind::Dialog { .. }));
            }
            (Some(session), "Page.fileChooserOpened") => {
                let (Some(tab), Some(node)) = (module.tab_of_session(session), event.params["backendNodeId"].as_i64()) else { return };
                if !module.watched(&tab.group_id) {
                    return;
                }
                let multiple = event.params["mode"] == "selectMultiple";
                let (module, cdp, session) = (module.me(), cdp.clone(), session.to_string());
                tokio::spawn(async move {
                    let described = cdp.send_within(Some(&session), "DOM.describeNode", json!({ "backendNodeId": node }), COMMAND_WAIT).await;
                    let accept = accept_of(&described.unwrap_or(Value::Null));
                    let kind = Kind::FileChooser { node, multiple, taken: 0, serial: 0, files: Vec::new() };
                    module.outside.raise(&module, &tab.tab_id, json!({ "multiple": multiple, "accept": accept }), kind);
                });
            }
            (Some(session), "Page.frameNavigated") if event.params["frame"]["parentId"].is_null() => {
                // Another document: what the old one asked for is gone with it.
                let Some(tab) = module.tab_of_session(session) else { return };
                self.close_where(|p| p.tab_id == tab.tab_id && !matches!(p.kind, Kind::Dialog { .. }));
                let body = json!({ "tab_id": tab.tab_id, "editable": false });
                if self.state().focus.insert(tab.tab_id.clone(), body.clone()).is_some_and(|before| before != body) {
                    self.tell(Notice::Focus { tab_id: tab.tab_id, body });
                }
            }
            (None, "Browser.downloadWillBegin") => {
                let Some(guid) = event.params["guid"].as_str() else { return };
                let file = Downloaded {
                    download_id: guid.to_string(),
                    name: file_name(event.params["suggestedFilename"].as_str().unwrap_or_default()),
                    size: 0,
                    path: self.downloads_dir.join(guid),
                };
                let tab_id = event.params["frameId"].as_str().filter(|frame| module.tab(frame).is_some()).map(str::to_string);
                let mut s = self.state();
                while s.downloads.len() >= MAX_DOWNLOADS || s.downloads.front().is_some_and(|d| d.at.elapsed() > DOWNLOAD_KEPT) {
                    if let Some(old) = s.downloads.pop_front() {
                        let _ = std::fs::remove_file(&old.file.path);
                    }
                }
                s.downloads.push_back(Download { file, tab_id, completed: false, at: Instant::now() });
            }
            (None, "Browser.downloadProgress") => {
                let state = match event.params["state"].as_str() {
                    Some("completed") => "completed",
                    Some("canceled") => "failed",
                    _ => return,
                };
                let found = {
                    let mut s = self.state();
                    let Some(at) = s.downloads.iter().position(|d| Some(d.file.download_id.as_str()) == event.params["guid"].as_str() && !d.completed) else {
                        return;
                    };
                    if state == "completed" {
                        let download = &mut s.downloads[at];
                        download.completed = true;
                        download.at = Instant::now();
                        if let Some(path) = event.params["filePath"].as_str() {
                            download.file.path = PathBuf::from(path);
                        }
                        download.file.size = std::fs::metadata(&download.file.path)
                            .map(|meta| meta.len())
                            .unwrap_or_else(|_| event.params["receivedBytes"].as_f64().unwrap_or(0.0) as u64);
                        (download.file.clone(), download.tab_id.clone())
                    } else {
                        let download = s.downloads.remove(at).expect("found");
                        (download.file, download.tab_id)
                    }
                };
                let (file, tab_id) = found;
                let body = json!({
                    "request_id": self.next_id(),
                    "kind": "download",
                    "tab_id": tab_id.clone().unwrap_or_default(),
                    "download_id": file.download_id,
                    "name": file.name,
                    "size": file.size,
                    "state": state,
                });
                let group_id = tab_id.and_then(|tab_id| module.tab(&tab_id)).map(|tab| tab.group_id);
                self.tell(Notice::Request { group_id, body });
            }
            _ => {}
        }
    }

    /// What the page script told (`screen_page.js`).
    fn told_by_page(&self, module: &BrowserModule, tab_id: &str, group_id: &str, context: i64, told: &Value) {
        match told["kind"].as_str() {
            Some("select") => {
                // The list is the user's to pick from: shown when they look and operate.
                if !module.watched(group_id) || module.controller(group_id) != Controller::User {
                    return;
                }
                let (Some(token), Some(options), Some(rect)) = (told["token"].as_u64(), told["options"].as_array(), rect_of(&told["rect"])) else { return };
                let body = json!({ "multiple": told["multiple"].as_bool().unwrap_or(false), "options": options, "rect": rect });
                self.raise(module, tab_id, body, Kind::Select { context, token });
            }
            Some("focus") => {
                let mut body = json!({ "tab_id": tab_id, "editable": told["editable"].as_bool().unwrap_or(false) });
                if body["editable"] == true {
                    body["input_type"] = json!(told["input_type"].as_str().unwrap_or("text"));
                    if let Some(rect) = rect_of(&told["rect"]) {
                        body["rect"] = rect;
                    }
                }
                self.state().focus.insert(tab_id.to_string(), body.clone());
                self.tell(Notice::Focus { tab_id: tab_id.to_string(), body });
            }
            Some("permission") => {
                let (Some(permission), Some(origin)) = (told["permission"].as_str(), told["origin"].as_str()) else { return };
                {
                    let mut s = self.state();
                    s.noticed.retain(|_, at| at.elapsed() < PERMISSION_QUIET);
                    let key = (tab_id.to_string(), permission.to_string(), origin.to_string());
                    if s.noticed.len() >= MAX_PENDING || s.noticed.insert(key, Instant::now()).is_some() {
                        return;
                    }
                }
                let body = json!({ "request_id": self.next_id(), "kind": "permission", "tab_id": tab_id, "permission": permission, "origin": origin });
                self.tell(Notice::Request { group_id: Some(group_id.to_string()), body });
            }
            _ => {}
        }
    }

    pub(super) fn tab_gone(&self, tab_id: &str) {
        self.close_where(|p| p.tab_id == tab_id);
        let handed: Vec<PathBuf> = {
            let mut s = self.state();
            s.focus.remove(tab_id);
            s.prepared.remove(tab_id);
            let (gone, kept) = std::mem::take(&mut s.handed).into_iter().partition(|(tab, ..)| tab == tab_id);
            s.handed = kept;
            gone.into_iter().map(|(_, dir, _)| dir).collect()
        };
        for dir in handed {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    pub(super) fn browser_gone(&self) {
        self.close_where(|_| true);
        {
            let mut s = self.state();
            s.focus.clear();
            s.handed.clear();
            s.noticed.clear();
            s.prepared.clear();
        }
        let _ = std::fs::remove_dir_all(&self.uploads_dir);
    }

    // ------------------------------------------------------------------
    // Uploads and downloads
    // ------------------------------------------------------------------

    /// Where the next file of a file chooser request is written.
    pub fn stage(&self, request_id: &str, name: &str) -> Result<PathBuf, BrowserError> {
        let (index, expired) = {
            let mut s = self.state();
            let (expired, kept) = std::mem::take(&mut s.handed).into_iter().partition(|(.., at)| at.elapsed() > UPLOAD_KEPT);
            s.handed = kept;
            let request = s.pending.iter_mut().find(|p| p.request_id == request_id).ok_or_else(|| no_such_request(request_id))?;
            let Kind::FileChooser { multiple, taken, serial, .. } = &mut request.kind else { return Err(no_such_request(request_id)) };
            if *taken >= if *multiple { MAX_UPLOAD_FILES } else { 1 } {
                return Err(BrowserError::invalid(format!("request `{request_id}` takes no more files")));
            }
            *taken += 1;
            *serial += 1;
            (*serial, expired)
        };
        for (_, dir, _) in expired {
            let _ = std::fs::remove_dir_all(dir);
        }
        let dir = self.uploads_dir.join(request_id).join(index.to_string());
        std::fs::create_dir_all(&dir).map_err(|e| BrowserError::new("other", format!("create {}: {e}", dir.display())))?;
        Ok(dir.join(file_name(name)))
    }

    /// A staged file did not arrive: its place is free again.
    pub fn unstage(&self, request_id: &str, path: &Path) {
        if let Some(Kind::FileChooser { taken, .. }) = self.state().pending.iter_mut().find(|p| p.request_id == request_id).map(|p| &mut p.kind) {
            *taken = taken.saturating_sub(1);
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A staged file is complete. With `done` the request's files are given
    /// to the page's file input and the request is over.
    pub async fn staged(&self, module: &BrowserModule, request_id: &str, path: PathBuf, done: bool) -> Result<(), BrowserError> {
        let request = {
            let mut s = self.state();
            let Some(at) = s.pending.iter().position(|p| p.request_id == request_id && matches!(p.kind, Kind::FileChooser { .. })) else {
                drop(s);
                self.unstage(request_id, &path);
                return Err(no_such_request(request_id));
            };
            if let Kind::FileChooser { files, .. } = &mut s.pending[at].kind {
                files.push(path);
            }
            if !done {
                return Ok(());
            }
            s.pending.remove(at)
        };
        let Kind::FileChooser { node, files, .. } = &request.kind else { return Err(no_such_request(request_id)) };
        let files: Vec<String> = files.iter().map(|file| file.display().to_string()).collect();
        let set = match module.tab_session(&request.tab_id).await {
            Ok((cdp, session)) => cdp
                .send_within(Some(&session), "DOM.setFileInputFiles", json!({ "files": files, "backendNodeId": node }), COMMAND_WAIT)
                .await
                .map_err(BrowserError::from),
            Err(e) => Err(e),
        };
        match set {
            Ok(_) => {
                let dir = self.uploads_dir.join(&request.request_id);
                self.state().handed.push((request.tab_id.clone(), dir, Instant::now()));
                self.tell(Notice::Closed { request_id: request.request_id });
                Ok(())
            }
            Err(e) => {
                self.ended(request);
                Err(e)
            }
        }
    }

    /// A download that is complete.
    pub fn download(&self, download_id: &str) -> Result<Downloaded, BrowserError> {
        let s = self.state();
        s.downloads
            .iter()
            .find(|d| d.file.download_id == download_id && d.completed && d.at.elapsed() <= DOWNLOAD_KEPT)
            .map(|d| d.file.clone())
            .ok_or_else(|| BrowserError::new("no_such_download", format!("no finished download `{download_id}`")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_files_keep_their_names_and_nothing_else_of_their_paths() {
        for (given, kept) in [
            ("report.pdf", "report.pdf"),
            ("/home/alice/Documents/报告 2026.pdf", "报告 2026.pdf"),
            ("C:\\Users\\a\\photo.png", "photo.png"),
            ("../../etc/passwd", "passwd"),
            ("..", "file"),
            ("", "file"),
            ("a\u{0}b\n.txt", "ab.txt"),
            (".bashrc", ".bashrc"),
        ] {
            assert_eq!(file_name(given), kept, "{given:?}");
        }
        assert!(file_name(&"字".repeat(200)).len() <= 200);
    }

    #[test]
    fn the_accept_list_of_a_file_input_is_read_from_its_attributes() {
        let described = json!({ "node": { "attributes": ["id", "f", "type", "file", "accept", ".png, image/*,", "multiple", ""] } });
        assert_eq!(accept_of(&described), vec![".png", "image/*"]);
        assert!(accept_of(&json!({ "node": { "attributes": ["type", "file"] } })).is_empty());
        assert!(accept_of(&Value::Null).is_empty());
    }
}
