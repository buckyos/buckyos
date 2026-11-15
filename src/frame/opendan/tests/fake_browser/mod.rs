//! A stand-in for Chromium: a shell script that plays the process (writes
//! `DevToolsActivePort`, stays until told to quit) and an in-process server
//! that plays the debugging endpoint (targets, flat sessions, the events the
//! browser module follows). Shared by the module's unit tests and the
//! Loader's tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

const SCRIPT: &str = r#"#!/bin/sh
# A fake chromium: the debugging endpoint is served by the test.
for a in "$@"; do
  case "$a" in
    --user-data-dir=*) profile="${a#--user-data-dir=}" ;;
    --no-sandbox) no_sandbox=1 ;;
  esac
done
root="$(dirname "$profile")"
echo "$$" > "$root/fake_pid"
echo "$*" > "$root/fake_args"
if [ -e "$root/fake_needs_no_sandbox" ] && [ -z "$no_sandbox" ]; then
  echo "[1:1:FATAL:zygote_host_impl_linux.cc] No usable sandbox! refusing to start" >&2
  exit 1
fi
port="$(cat "$root/fake_cdp_port")" || exit 3
rm -f "$root/fake_quit"
mkdir -p "$profile"
# A helper that leaves the process group, as Chromium's crash handlers do.
if [ -e "$root/fake_stray" ]; then
  setsid sleep 300 > /dev/null 2>&1 &
  echo "$!" > "$root/fake_stray_pid"
fi
printf '%s\n/devtools/browser/fake\n' "$port" > "$profile/DevToolsActivePort"
if [ -e "$root/fake_ignores_term" ]; then trap '' TERM; fi
while [ ! -e "$root/fake_quit" ] || [ -e "$root/fake_ignores_close" ]; do sleep 0.05; done
rm -f "$root/fake_quit"
"#;

/// The fake chromium executable (one per test process).
pub fn chromium() -> PathBuf {
    static SCRIPT_FILE: std::sync::OnceLock<(tempfile::TempDir, PathBuf)> = std::sync::OnceLock::new();
    SCRIPT_FILE
        .get_or_init(|| {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("chromium");
            std::fs::write(&path, SCRIPT).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            (dir, path)
        })
        .1
        .clone()
}

#[derive(Debug, Clone)]
struct Target {
    id: String,
    url: String,
    title: String,
    opener: Option<String>,
}

#[derive(Default)]
struct State {
    targets: Vec<Target>,
    seq: u64,
    /// Every command received: `(session, method, params)`.
    commands: Vec<(Option<String>, String, Value)>,
    /// Sessions by id → target.
    sessions: Vec<(String, String)>,
    /// Targets with an open JS dialog.
    dialogs: Vec<String>,
    /// Sessions whose screencast runs.
    casting: Vec<String>,
    frames: u64,
    /// Where each target went (`Page.navigate`) and which of them it shows.
    history: std::collections::HashMap<String, (Vec<String>, usize)>,
    /// The size each target was made (`Emulation.setDeviceMetricsOverride`); the others have `TAB_SIZE`.
    sizes: std::collections::HashMap<String, (u64, u64)>,
}

/// The size of a tab nobody resized.
const TAB_SIZE: (u64, u64) = (412, 915);

/// The debugging endpoint of the fake browser whose directories are `root`
/// (`<root>/profile` is its user data directory).
pub struct FakeBrowser {
    pub root: PathBuf,
    pub port: u16,
    state: Arc<Mutex<State>>,
    events: broadcast::Sender<String>,
    /// Bumped to drop every connection (the browser ended).
    hangup: broadcast::Sender<()>,
}

/// A target as the browser describes it. In an event (`announced`) the
/// title is not the document's: like headless Chromium, the fake only tells
/// a document's title when it is asked (`Target.getTargets`).
fn target_info(t: &Target, announced: bool) -> Value {
    let title = if announced { &t.url } else { &t.title };
    let mut info = json!({ "targetId": t.id, "type": "page", "title": title, "url": t.url, "attached": false });
    if let Some(opener) = &t.opener {
        info["openerId"] = json!(opener);
    }
    info
}

impl FakeBrowser {
    /// Serve the endpoint and tell the script where it is.
    pub async fn start(root: &Path) -> Arc<Self> {
        std::fs::create_dir_all(root).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(root.join("fake_cdp_port"), port.to_string()).unwrap();
        let fake = Arc::new(Self {
            root: root.to_path_buf(),
            port,
            state: Arc::new(Mutex::new(State::default())),
            events: broadcast::channel(256).0,
            hangup: broadcast::channel(4).0,
        });
        let serving = fake.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let Ok(socket) = tokio_tungstenite::accept_async(stream).await else { continue };
                tokio::spawn(serving.clone().serve(socket));
            }
        });
        fake
    }

    async fn serve(self: Arc<Self>, socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) {
        let (mut sink, mut stream) = socket.split();
        let mut events = self.events.subscribe();
        let mut hangup = self.hangup.subscribe();
        loop {
            tokio::select! {
                _ = hangup.recv() => break,
                event = events.recv() => {
                    let Ok(event) = event else { break };
                    if sink.send(Message::text(event)).await.is_err() {
                        break;
                    }
                }
                message = stream.next() => {
                    let Some(Ok(Message::Text(text))) = message else { break };
                    let request: Value = serde_json::from_str(text.as_str()).unwrap();
                    let (answer, quit) = self.answer(&request);
                    if answer.is_null() {
                        continue;
                    }
                    if sink.send(Message::text(answer.to_string())).await.is_err() || quit {
                        break;
                    }
                }
            }
        }
        let _ = sink.close().await;
    }

    fn emit(&self, method: &str, params: Value, session: Option<&str>) {
        let mut event = json!({ "method": method, "params": params });
        if let Some(session) = session {
            event["sessionId"] = json!(session);
        }
        let _ = self.events.send(event.to_string());
    }

    fn add_target(&self, url: &str, opener: Option<&str>) -> String {
        let target = {
            let mut s = self.state.lock().unwrap();
            s.seq += 1;
            let target = Target {
                id: format!("T{}", s.seq),
                url: url.to_string(),
                title: String::new(),
                opener: opener.map(str::to_string),
            };
            s.targets.push(target.clone());
            target
        };
        self.emit("Target.targetCreated", json!({ "targetInfo": target_info(&target, true) }), None);
        target.id
    }

    fn answer(&self, request: &Value) -> (Value, bool) {
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default().to_string();
        let params = request["params"].clone();
        let session = request["sessionId"].as_str().map(str::to_string);
        self.state.lock().unwrap().commands.push((session.clone(), method.clone(), params.clone()));
        let fail = |message: &str| (json!({ "id": id, "error": { "code": -32000, "message": message } }), false);
        let ok = |result: Value| {
            let mut answer = json!({ "id": id, "result": result });
            if let Some(session) = &session {
                answer["sessionId"] = json!(session);
            }
            (answer, false)
        };
        match method.as_str() {
            "Browser.getVersion" => ok(json!({ "product": "Chrome/1.2.3.4", "protocolVersion": "1.3" })),
            "Browser.close" => {
                let _ = std::fs::write(self.root.join("fake_quit"), "");
                (json!({ "id": id, "result": {} }), !self.root.join("fake_ignores_close").exists())
            }
            "Target.setDiscoverTargets" => {
                let targets = self.state.lock().unwrap().targets.clone();
                for t in &targets {
                    self.emit("Target.targetCreated", json!({ "targetInfo": target_info(t, true) }), None);
                }
                ok(json!({}))
            }
            "Target.getTargets" => {
                let targets = self.state.lock().unwrap().targets.clone();
                ok(json!({ "targetInfos": targets.iter().map(|t| target_info(t, false)).collect::<Vec<_>>() }))
            }
            "Target.createTarget" => {
                let url = params["url"].as_str().unwrap_or("about:blank");
                if url.starts_with("bad:") {
                    return fail("Cannot navigate to invalid URL");
                }
                let target_id = self.add_target(url, None);
                ok(json!({ "targetId": target_id }))
            }
            "Target.attachToTarget" => {
                let target_id = params["targetId"].as_str().unwrap_or_default();
                let mut s = self.state.lock().unwrap();
                if !s.targets.iter().any(|t| t.id == target_id) {
                    return fail("No target with given id found");
                }
                s.seq += 1;
                let session_id = format!("S{}", s.seq);
                s.sessions.push((session_id.clone(), target_id.to_string()));
                ok(json!({ "sessionId": session_id }))
            }
            "Target.closeTarget" => {
                let target_id = params["targetId"].as_str().unwrap_or_default();
                let known = {
                    let mut s = self.state.lock().unwrap();
                    let before = s.targets.len();
                    s.targets.retain(|t| t.id != target_id);
                    s.targets.len() != before
                };
                if !known {
                    return fail("No target with given id found");
                }
                self.emit("Target.targetDestroyed", json!({ "targetId": target_id }), None);
                ok(json!({ "success": true }))
            }
            "Page.navigate" => {
                let Some(session) = session.clone() else { return fail("Page.navigate needs a session") };
                let url = params["url"].as_str().unwrap_or_default().to_string();
                let target_id = self.target_of(&session).unwrap_or_default();
                self.emit("Page.frameStartedLoading", json!({ "frameId": target_id }), Some(&session));
                {
                    let mut s = self.state.lock().unwrap();
                    let (entries, at) = s.history.entry(target_id.clone()).or_insert_with(|| (vec!["about:blank".to_string()], 0));
                    entries.truncate(*at + 1);
                    entries.push(url.clone());
                    *at = entries.len() - 1;
                }
                self.set_page(&target_id, &url, &format!("Title of {url}"));
                if !url.contains("slow") {
                    self.emit("Page.frameStoppedLoading", json!({ "frameId": target_id }), Some(&session));
                    // The loaded page is painted.
                    self.frame(&target_id, b"loaded");
                }
                ok(json!({ "frameId": target_id }))
            }
            "Page.handleJavaScriptDialog" => {
                let Some(session) = session.clone() else { return fail("Page.handleJavaScriptDialog needs a session") };
                let target_id = self.target_of(&session).unwrap_or_default();
                let open = {
                    let mut s = self.state.lock().unwrap();
                    let before = s.dialogs.len();
                    s.dialogs.retain(|t| *t != target_id);
                    s.dialogs.len() != before
                };
                if !open {
                    return fail("No dialog is showing");
                }
                for session in self.sessions_of(&target_id) {
                    self.emit("Page.javascriptDialogClosed", json!({ "result": params["accept"], "userInput": "" }), Some(&session));
                }
                ok(json!({}))
            }
            "Page.getNavigationHistory" => {
                let target_id = session.as_deref().and_then(|session| self.target_of(session)).unwrap_or_default();
                let (entries, at) = self.state.lock().unwrap().history.get(&target_id).cloned().unwrap_or_else(|| (vec!["about:blank".to_string()], 0));
                let entries: Vec<Value> = entries.iter().enumerate().map(|(id, url)| json!({ "id": id, "url": url })).collect();
                ok(json!({ "currentIndex": at, "entries": entries }))
            }
            "Page.navigateToHistoryEntry" => {
                let target_id = session.as_deref().and_then(|session| self.target_of(session)).unwrap_or_default();
                let url = {
                    let mut s = self.state.lock().unwrap();
                    let Some((entries, at)) = s.history.get_mut(&target_id) else { return fail("No entry with passed id") };
                    let Some(url) = params["entryId"].as_u64().and_then(|id| entries.get(id as usize).cloned()) else { return fail("No entry with passed id") };
                    *at = params["entryId"].as_u64().unwrap_or(0) as usize;
                    url
                };
                self.set_page(&target_id, &url, &format!("Title of {url}"));
                ok(json!({}))
            }
            "Page.startScreencast" => {
                let Some(session) = session.clone() else { return fail("Page.startScreencast needs a session") };
                let target_id = self.target_of(&session).unwrap_or_default();
                {
                    let mut s = self.state.lock().unwrap();
                    if !s.casting.contains(&session) {
                        s.casting.push(session);
                    }
                }
                // What the tab shows when the screencast starts.
                self.frame(&target_id, b"first");
                ok(json!({}))
            }
            "Page.stopScreencast" => {
                self.state.lock().unwrap().casting.retain(|casting| Some(casting) != session.as_ref());
                ok(json!({}))
            }
            "Emulation.setDeviceMetricsOverride" => {
                let Some(target_id) = session.as_deref().and_then(|session| self.target_of(session)) else { return fail("Emulation.setDeviceMetricsOverride needs a session") };
                let size = (params["width"].as_u64().unwrap_or(0), params["height"].as_u64().unwrap_or(0));
                let before = self.state.lock().unwrap().sizes.insert(target_id.clone(), size).unwrap_or(TAB_SIZE);
                // A tab of another size is painted again (`fake_resizes_unpainted`: not this time, as a browser may).
                if before != size && !self.root.join("fake_resizes_unpainted").exists() {
                    self.frame(&target_id, b"resized");
                }
                ok(json!({}))
            }
            "DOM.describeNode" => ok(json!({ "node": { "nodeName": "INPUT", "attributes": ["type", "file", "accept", ".png, image/*"] } })),
            // A command the browser never answers.
            "Fake.never" => (Value::Null, false),
            _ => ok(json!({})),
        }
    }

    fn sessions_of(&self, target_id: &str) -> Vec<String> {
        self.state.lock().unwrap().sessions.iter().filter(|(_, t)| t == target_id).map(|(s, _)| s.clone()).collect()
    }

    /// A page opens a JS dialog (`alert | confirm | prompt | beforeunload`).
    pub fn dialog(&self, target_id: &str, kind: &str, message: &str) {
        self.state.lock().unwrap().dialogs.push(target_id.to_string());
        for session in self.sessions_of(target_id) {
            self.emit(
                "Page.javascriptDialogOpening",
                json!({ "url": "", "message": message, "type": kind, "hasBrowserHandler": false, "defaultPrompt": "" }),
                Some(&session),
            );
        }
    }

    /// The tab painted a frame: its screencasts get it (`data` stands for the JPEG).
    pub fn frame(&self, target_id: &str, data: &[u8]) {
        use base64::Engine;
        let (sessions, number, (width, height)) = {
            let mut s = self.state.lock().unwrap();
            s.frames += 1;
            let sessions: Vec<String> = s.sessions.iter().filter(|(session, target)| target == target_id && s.casting.contains(session)).map(|(session, _)| session.clone()).collect();
            (sessions, s.frames, s.sizes.get(target_id).copied().unwrap_or(TAB_SIZE))
        };
        for session in sessions {
            self.emit(
                "Page.screencastFrame",
                json!({
                    "data": base64::engine::general_purpose::STANDARD.encode(data),
                    "metadata": { "offsetTop": 0, "pageScaleFactor": 1, "deviceWidth": width, "deviceHeight": height, "scrollOffsetX": 0, "scrollOffsetY": 0 },
                    "sessionId": number,
                }),
                Some(&session),
            );
        }
    }

    /// An event of a tab, to the sessions attached to it.
    pub fn tell(&self, target_id: &str, method: &str, params: Value) {
        for session in self.sessions_of(target_id) {
            self.emit(method, params.clone(), Some(&session));
        }
    }

    /// An event of the browser itself.
    pub fn announce(&self, method: &str, params: Value) {
        self.emit(method, params, None);
    }

    /// Tabs with a JS dialog nobody answered.
    pub fn open_dialogs(&self) -> Vec<String> {
        self.state.lock().unwrap().dialogs.clone()
    }

    fn target_of(&self, session: &str) -> Option<String> {
        self.state.lock().unwrap().sessions.iter().find(|(s, _)| s == session).map(|(_, t)| t.clone())
    }

    /// The page of a tab changed (a navigation as the browser reports it:
    /// the new address is announced, the title is there to be read).
    pub fn set_page(&self, target_id: &str, url: &str, title: &str) {
        let info = {
            let mut s = self.state.lock().unwrap();
            let Some(t) = s.targets.iter_mut().find(|t| t.id == target_id) else { return };
            t.url = url.to_string();
            t.title = title.to_string();
            target_info(t, true)
        };
        self.emit("Target.targetInfoChanged", json!({ "targetInfo": info }), None);
    }

    /// A tab somebody else opened (the agent's Playwright, a page's `window.open`).
    pub fn open(&self, url: &str, opener: Option<&str>) -> String {
        self.add_target(url, opener)
    }

    /// A tab somebody else closed.
    pub fn close(&self, target_id: &str) {
        self.state.lock().unwrap().targets.retain(|t| t.id != target_id);
        self.emit("Target.targetDestroyed", json!({ "targetId": target_id }), None);
    }

    pub fn targets(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().targets.iter().map(|t| (t.id.clone(), t.url.clone())).collect()
    }

    /// Commands received so far, by method.
    pub fn received(&self, method: &str) -> Vec<(Option<String>, Value)> {
        let s = self.state.lock().unwrap();
        s.commands.iter().filter(|(_, m, _)| m == method).map(|(session, _, params)| (session.clone(), params.clone())).collect()
    }

    /// Every command received so far, in order: `(session, method, params)`.
    pub fn received_all(&self) -> Vec<(Option<String>, String, Value)> {
        self.state.lock().unwrap().commands.clone()
    }

    /// Pid of the script playing the browser's process.
    pub fn pid(&self) -> Option<i32> {
        std::fs::read_to_string(self.root.join("fake_pid")).ok()?.trim().parse().ok()
    }

    /// The command line the script was last started with.
    pub fn args(&self) -> String {
        std::fs::read_to_string(self.root.join("fake_args")).unwrap_or_default()
    }

    /// The browser dies: its process is killed, its tabs and connections are gone.
    pub fn crash(&self) {
        if let Some(pid) = self.pid() {
            let _ = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status();
        }
        {
            let mut s = self.state.lock().unwrap();
            s.targets.clear();
            s.sessions.clear();
        }
        let _ = self.hangup.send(());
    }

    /// A stopped browser's tabs are gone with it.
    pub fn forget_tabs(&self) {
        let mut s = self.state.lock().unwrap();
        s.targets.clear();
        s.sessions.clear();
    }

    /// Pid of the helper the script started outside its process group (`fake_stray`).
    pub fn stray_pid(&self) -> Option<i32> {
        std::fs::read_to_string(self.root.join("fake_stray_pid")).ok()?.trim().parse().ok()
    }

    /// Switch a behaviour of the script on or off (`fake_needs_no_sandbox`,
    /// `fake_ignores_close`, `fake_ignores_term`, `fake_stray`), or of the
    /// endpoint (`fake_resizes_unpainted`).
    pub fn flag(&self, name: &str, on: bool) {
        let path = self.root.join(name);
        if on {
            std::fs::write(path, "").unwrap();
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
}
