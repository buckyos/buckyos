//! The screen (Agent Browser Protocol §3): a WebSocket over which the owner
//! sees the current tab of one group as a stream of JPEG frames and operates
//! it. A connection says `hello` first and is admitted as the owner; from
//! then on it is told the group's state whenever it changes, gets at most
//! one frame it has not acknowledged, and what it sends is applied to the
//! page — unless the agent operates the group.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::{Sink, SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::cdp::{CdpClient, CdpError, EventStream, Scope};
use super::groups::{is_internal, Controller};
use super::input;
use super::requests::{send_settled, Notice};
use super::{BrowserError, BrowserModule, Viewer};

/// Who the `hello` of a connection is, given its token: the caller's name,
/// or why they are not admitted.
pub type Authorize = Box<dyn FnOnce(Option<String>) -> BoxFuture<'static, Result<String, String>> + Send>;

const VIEWPORT_MIN: f64 = 320.0;
const VIEWPORT_MAX: f64 = 2560.0;
const DEFAULT_FPS: u32 = 15;
const MAX_FPS: u32 = 30;
const DEFAULT_JPEG_QUALITY: u8 = 60;
/// What a phone's screen is to a page besides its size.
const MOBILE_SCALE: u32 = 2;
const TOUCH_POINTS: u32 = 5;
/// A client that does not take what it is sent for this long is gone.
const SEND_WAIT: Duration = Duration::from_secs(10);
const NAVIGATE_WAIT: Duration = Duration::from_secs(30);
const HISTORY_WAIT: Duration = Duration::from_secs(3);
const REFUSAL_EVERY: Duration = Duration::from_secs(1);
/// A screencast that was started paints a frame within this time; one that did not is started again.
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(1);
/// So does a tab that finished loading, or shows another renderer's surface.
const LOADED_FRAME_WAIT: Duration = Duration::from_millis(400);
const CAST_RESTARTS: u32 = 5;

/// The waits of the protocol (tests shorten them).
#[derive(Debug, Clone)]
pub struct Timing {
    /// From the connection to its `hello`.
    pub hello: Duration,
    /// Without anything from the client the connection is closed.
    pub silence: Duration,
    /// A JS dialog nobody answers is closed.
    pub dialog: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { hello: Duration::from_secs(5), silence: Duration::from_secs(30), dialog: Duration::from_secs(60) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
struct Viewport {
    width: u32,
    height: u32,
    mobile: bool,
}

impl Viewport {
    fn parse(value: &Value) -> Option<Self> {
        let side = |name: &str| value[name].as_f64().filter(|n| n.is_finite()).map(|n| n.round().clamp(VIEWPORT_MIN, VIEWPORT_MAX) as u32);
        Some(Self { width: side("width")?, height: side("height")?, mobile: value["mobile"].as_bool().unwrap_or(false) })
    }
}

/// The user agent of a phone with this browser's version, and what the
/// browser tells about it besides (`userAgentMetadata`).
fn mobile_agent(version: &Value) -> Option<(String, Value)> {
    let full = version["product"].as_str()?.rsplit('/').next().filter(|v| !v.is_empty())?;
    let major = full.split('.').next()?;
    let agent = format!("Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Mobile Safari/537.36");
    let metadata = json!({
        "brands": [
            { "brand": "Chromium", "version": major },
            { "brand": "Google Chrome", "version": major },
            { "brand": "Not_A Brand", "version": "99" },
        ],
        "fullVersion": full,
        "platform": "Android",
        "platformVersion": "10",
        "architecture": "",
        "model": "K",
        "mobile": true,
    });
    Some((agent, metadata))
}

/// Make a tab look to its page like the device that shows it: a phone
/// (size, touch, a phone's user agent) or a desktop window of this size.
/// `again`: the tab may have been made that device before.
async fn emulate(cdp: &CdpClient, session: &str, viewport: Viewport, mobile: Option<&(String, Value)>, again: bool) {
    let Viewport { width, height, mobile: phone } = viewport;
    let metrics = match phone {
        true => json!({ "width": width, "height": height, "deviceScaleFactor": MOBILE_SCALE, "mobile": true, "screenWidth": width, "screenHeight": height }),
        false => json!({ "width": width, "height": height, "deviceScaleFactor": 0, "mobile": false }),
    };
    if again {
        // The browser takes metrics it already got over this session for done, though a
        // screenshot taken over another one (the agent's Playwright) ends them for the page.
        // So they are sent with another screen first, which a page they still hold for does not notice.
        let mut other = metrics.clone();
        (other["screenWidth"], other["screenHeight"]) = (json!(width + 1), json!(height));
        let _ = send_settled(cdp, session, "Emulation.setDeviceMetricsOverride", other).await;
    }
    let agent = match (phone, mobile) {
        (true, Some((agent, metadata))) => json!({ "userAgent": agent, "userAgentMetadata": metadata }),
        // Empty: the browser's own again.
        _ => json!({ "userAgent": "" }),
    };
    for (method, params) in [
        ("Emulation.setDeviceMetricsOverride", metrics),
        ("Emulation.setTouchEmulationEnabled", json!({ "enabled": phone, "maxTouchPoints": TOUCH_POINTS })),
        ("Emulation.setUserAgentOverride", agent),
    ] {
        let _ = send_settled(cdp, session, method, params).await;
    }
}

/// What the user typed as an address, as an address the browser takes.
fn address(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    let complete = typed.contains("://") || ["about:", "data:", "blob:", "view-source:"].iter().any(|scheme| typed.starts_with(scheme));
    Some(if complete { typed.to_string() } else { format!("https://{typed}") })
}

// ----------------------------------------------------------------------
// The frames of a tab, shared by the connections that show it
// ----------------------------------------------------------------------

#[derive(Debug)]
pub struct Frame {
    pub tab_id: String,
    pub jpeg: Bytes,
    /// The viewport the frame shows, in CSS pixels.
    pub width: u64,
    pub height: u64,
}

/// How a connection looks at tabs: the viewport it asks for, when it set
/// it (`Casts::stamp`), and what a phone tells sites it is (`mobile_agent`).
struct Look {
    viewport: Viewport,
    set: u64,
    mobile: Option<(String, Value)>,
}

/// A connection that shows a tab.
struct Viewing {
    id: u64,
    viewport: Viewport,
    set: u64,
}

/// What a tab is to its page: the viewport of the viewer that set its own
/// last, how often the page was to be asked for again as that device, and
/// how often the tab was to be made that device once more.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Device {
    viewport: Viewport,
    reloads: u64,
    dresses: u64,
}

struct Cast {
    cdp: Arc<CdpClient>,
    session: String,
    quality: Arc<AtomicU8>,
    viewers: Vec<Viewing>,
    device: watch::Sender<Device>,
    frames: watch::Receiver<Option<Arc<Frame>>>,
    task: tokio::task::JoinHandle<()>,
    dresser: tokio::task::JoinHandle<()>,
}

impl Cast {
    /// A tab has one viewport: that of the viewer that set its own last.
    /// Returns whether the tab is to be made another device, or asked for again.
    fn settle(&self, reload: bool) -> bool {
        let Some(viewport) = self.viewers.iter().max_by_key(|viewing| viewing.set).map(|viewing| viewing.viewport) else { return false };
        self.device.send_if_modified(|device| {
            let changed = device.viewport != viewport || reload;
            device.viewport = viewport;
            device.reloads += reload as u64;
            changed
        })
    }

    fn end(self) {
        self.task.abort();
        self.dresser.abort();
    }
}

/// The screencasts that run: one per tab somebody looks at. Its latest
/// frame is all that is kept.
#[derive(Default)]
pub struct Casts {
    tabs: Mutex<HashMap<String, Cast>>,
    stamps: AtomicU64,
}

/// A connection's part in a tab's screencast, which ends with its last viewer.
pub struct Watching {
    module: Arc<BrowserModule>,
    id: u64,
    tab_id: String,
    session: String,
    frames: watch::Receiver<Option<Arc<Frame>>>,
    device: watch::Receiver<Device>,
}

impl Watching {
    /// The viewport the tab has: this connection's, or that of another one that set its own later.
    fn viewport(&self) -> Viewport {
        self.device.borrow().viewport
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.module.casts.leave(&self.module, &self.tab_id, &self.session, self.id);
    }
}

/// Keep a tab the device its viewers settled on, and tell its screencast what it was made (`dressed`).
async fn dress(cdp: Arc<CdpClient>, session: String, mobile: Option<(String, Value)>, mut device: watch::Receiver<Device>, dressed: watch::Sender<Option<Viewport>>) {
    let mut worn: Option<Device> = None;
    loop {
        let wanted = *device.borrow_and_update();
        if worn.map(|worn| (worn.viewport, worn.dresses)) != Some((wanted.viewport, wanted.dresses)) {
            // What the tab was before this screencast is not known.
            let again = worn.is_none_or(|worn| worn.viewport == wanted.viewport);
            emulate(&cdp, &session, wanted.viewport, mobile.as_ref(), again).await;
            dressed.send_replace(Some(wanted.viewport));
        }
        // A site decides what it sends by the user agent: asked again as the other device.
        if worn.is_some_and(|worn| worn.reloads != wanted.reloads) {
            let _ = cdp.post(Some(&session), "Page.reload", json!({}));
        }
        worn = Some(wanted);
        if device.changed().await.is_err() {
            break;
        }
    }
}

fn start_cast(cdp: &CdpClient, session: &str, quality: u8) {
    // The page domain is on by the time the module is done attaching; a screen may be earlier.
    let _ = cdp.post(Some(session), "Page.enable", json!({}));
    // Every frame: the browser's own thinning would drop the last frame of a change.
    let _ = cdp.post(Some(session), "Page.startScreencast", json!({ "format": "jpeg", "quality": quality, "everyNthFrame": 1 }));
}

/// What a screencast's task hears of its tab.
const CAST_EVENTS: &[&str] = &["Page.screencastFrame", "Page.frameStoppedLoading", "Page.screencastVisibilityChanged"];

async fn feed(
    cdp: Arc<CdpClient>,
    tab_id: String,
    session: String,
    mut events: EventStream,
    frames: watch::Sender<Option<Arc<Frame>>>,
    quality: Arc<AtomicU8>,
    mut dressed: watch::Receiver<Option<Viewport>>,
) {
    // When a frame is expected by. The browser paints one when a screencast
    // starts, but not always when a page was loaded again, the tab went to
    // another renderer or was made another size: a screencast that owes a
    // frame is started again.
    let mut owed = Some(Instant::now() + FIRST_FRAME_WAIT);
    let mut restarts = 0;
    // The size of the last frame, and the size the tab was made that no frame has shown yet.
    let (mut painted, mut resized) = (None, None);
    loop {
        let overdue = async {
            match owed {
                Some(by) => tokio::time::sleep_until(by).await,
                None => std::future::pending().await,
            }
        };
        let event = tokio::select! {
            event = events.next() => match event {
                Some(event) => event,
                None => break,
            },
            _ = overdue => {
                restarts += 1;
                owed = (restarts < CAST_RESTARTS).then(|| Instant::now() + FIRST_FRAME_WAIT);
                let _ = cdp.post(Some(&session), "Page.stopScreencast", json!({}));
                start_cast(&cdp, &session, quality.load(Ordering::SeqCst));
                continue;
            }
            Ok(()) = dressed.changed() => {
                resized = dressed.borrow_and_update().map(|viewport| (viewport.width as u64, viewport.height as u64)).filter(|size| Some(*size) != painted);
                if resized.is_some() {
                    restarts = 0;
                    owed = owed.or(Some(Instant::now() + LOADED_FRAME_WAIT));
                }
                continue;
            }
        };
        match event.method.as_str() {
            "Page.screencastFrame" => {
                // Acknowledged at once: the browser holds its next frames back until
                // then, and stops for good after a few. What a viewer is too slow
                // for is dropped here instead, where the latest frame replaces it.
                if let Some(number) = event.params["sessionId"].as_i64() {
                    let _ = cdp.post(Some(&session), "Page.screencastFrameAck", json!({ "sessionId": number }));
                }
                let Some(Ok(jpeg)) = event.params["data"].as_str().map(|data| base64::engine::general_purpose::STANDARD.decode(data)) else { continue };
                let side = |name: &str| event.params["metadata"][name].as_f64().unwrap_or(0.0).round() as u64;
                let frame = Frame { tab_id: tab_id.clone(), jpeg: Bytes::from(jpeg), width: side("deviceWidth"), height: side("deviceHeight") };
                painted = Some((frame.width, frame.height));
                frames.send_replace(Some(Arc::new(frame)));
                if resized.is_none() || resized == painted {
                    (owed, restarts, resized) = (None, 0, None);
                } else {
                    // Painted before the tab had its new size: the frame of that size is still owed.
                    owed = (restarts < CAST_RESTARTS).then(|| Instant::now() + LOADED_FRAME_WAIT);
                }
            }
            "Page.frameStoppedLoading" if event.params["frameId"].as_str() != Some(tab_id.as_str()) => {}
            "Page.screencastVisibilityChanged" if event.params["visible"] != true => {}
            _ => owed = owed.or(Some(Instant::now() + LOADED_FRAME_WAIT)),
        }
    }
}

impl Casts {
    /// When a connection set its viewport: later than every stamp before.
    fn stamp(&self) -> u64 {
        self.stamps.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn watch(&self, module: &Arc<BrowserModule>, cdp: &Arc<CdpClient>, tab_id: &str, session: &str, look: &Look, quality: u8) -> Watching {
        let viewing = Viewing { id: self.stamp(), viewport: look.viewport, set: look.set };
        let id = viewing.id;
        let mut tabs = self.tabs.lock().expect("casts");
        let (frames, device, changed) = match tabs.get_mut(tab_id) {
            Some(cast) if cast.session == session && Arc::ptr_eq(&cast.cdp, cdp) && !cast.task.is_finished() => {
                cast.viewers.push(viewing);
                let changed = cast.settle(false);
                (cast.frames.clone(), cast.device.subscribe(), changed)
            }
            _ => {
                if let Some(stale) = tabs.remove(tab_id) {
                    stale.end();
                }
                let (publish, frames) = watch::channel(None);
                let (device, devices) = watch::channel(Device { viewport: look.viewport, reloads: 0, dresses: 0 });
                let (dressed, dressing) = watch::channel(None);
                let events = cdp.subscribe(Scope::Session(session.to_string()), CAST_EVENTS);
                let quality = Arc::new(AtomicU8::new(quality));
                let task = tokio::spawn(feed(cdp.clone(), tab_id.to_string(), session.to_string(), events, publish, quality.clone(), dressing));
                let dresser = tokio::spawn(dress(cdp.clone(), session.to_string(), look.mobile.clone(), devices.clone(), dressed));
                start_cast(cdp, session, quality.load(Ordering::SeqCst));
                let cast = Cast { cdp: cdp.clone(), session: session.to_string(), quality, viewers: vec![viewing], device, frames: frames.clone(), task, dresser };
                tabs.insert(tab_id.to_string(), cast);
                (frames, devices, false)
            }
        };
        drop(tabs);
        if changed {
            module.bump();
        }
        Watching { module: module.clone(), id, tab_id: tab_id.to_string(), session: session.to_string(), frames, device }
    }

    /// A viewer set its viewport: the tab has it from now on, and with
    /// `reload` its page is asked for again as that device.
    fn claim(&self, watching: &Watching, look: &Look, reload: bool) {
        let changed = {
            let mut tabs = self.tabs.lock().expect("casts");
            let Some(cast) = tabs.get_mut(&watching.tab_id).filter(|cast| cast.session == watching.session) else { return };
            let Some(viewing) = cast.viewers.iter_mut().find(|viewing| viewing.id == watching.id) else { return };
            (viewing.viewport, viewing.set) = (look.viewport, look.set);
            cast.settle(reload)
        };
        if changed {
            watching.module.bump();
        }
    }

    /// The agent is done with a tab: what its Playwright did may have ended
    /// the device the tab was made (a screenshot does), so it is made that device once more.
    fn dress_again(&self, watching: &Watching) {
        let tabs = self.tabs.lock().expect("casts");
        if let Some(cast) = tabs.get(&watching.tab_id).filter(|cast| cast.session == watching.session) {
            cast.device.send_modify(|device| device.dresses += 1);
        }
    }

    /// The picture quality of a tab's frames from now on (the last viewer to say so is heard).
    fn set_quality(&self, tab_id: &str, session: &str, quality: u8) {
        let mut tabs = self.tabs.lock().expect("casts");
        if let Some(cast) = tabs.get_mut(tab_id).filter(|cast| cast.session == session && cast.quality.load(Ordering::SeqCst) != quality) {
            cast.quality.store(quality, Ordering::SeqCst);
            let _ = cast.cdp.post(Some(session), "Page.stopScreencast", json!({}));
            start_cast(&cast.cdp, session, quality);
        }
    }

    /// A viewer is gone. The tab keeps its viewport if it was the last
    /// one; else it gets the viewport of those that are left.
    fn leave(&self, module: &BrowserModule, tab_id: &str, session: &str, id: u64) {
        let changed = {
            let mut tabs = self.tabs.lock().expect("casts");
            let Some(cast) = tabs.get_mut(tab_id).filter(|cast| cast.session == session) else { return };
            cast.viewers.retain(|viewing| viewing.id != id);
            if cast.viewers.is_empty() {
                if let Some(cast) = tabs.remove(tab_id) {
                    let _ = cast.cdp.post(Some(session), "Page.stopScreencast", json!({}));
                    cast.end();
                }
                return;
            }
            cast.settle(false)
        };
        if changed {
            module.bump();
        }
    }

    /// Tabs whose frames are being taken (tests).
    pub fn running(&self) -> usize {
        self.tabs.lock().expect("casts").len()
    }
}

// ----------------------------------------------------------------------
// A connection
// ----------------------------------------------------------------------

/// The connection is over.
struct Gone;

/// What work done beside the connection's loop tells it.
enum Told {
    History { tab_id: String, back: bool, forward: bool },
    Failed(BrowserError),
}

struct Shown {
    tab_id: String,
    session: String,
    cdp: Arc<CdpClient>,
    cast: Watching,
}

struct Screen<W> {
    sink: W,
    module: Arc<BrowserModule>,
    viewer: Viewer,
    group_id: String,
    look: Look,
    max_fps: u32,
    jpeg_quality: u8,
    /// The tab whose frames the connection gets.
    shown: Option<Shown>,
    /// `(tab, address, loading)` the history was last asked for, and what it was.
    history_of: Option<(String, String, bool)>,
    history: (bool, bool),
    /// The state the client was last told (`Null`: not even `ready`).
    state: Value,
    seq: u64,
    /// The frame the client has not acknowledged.
    unacked: Option<u64>,
    /// The tab has a frame the client has not been sent.
    frame_due: bool,
    next_frame: Instant,
    touches: Vec<input::Touch>,
    refused_at: Option<Instant>,
    /// The agent operated the group when the connection last looked.
    agent: bool,
    /// Requests the client was shown that wait for an answer.
    requests: HashSet<String>,
    told: mpsc::UnboundedSender<Told>,
}

struct Hello {
    token: Option<String>,
    group_id: String,
    viewport: Viewport,
}

fn hello(text: &str) -> Result<Hello, String> {
    let message: Value = serde_json::from_str(text).map_err(|e| format!("the hello is not JSON: {e}"))?;
    if message["type"] != "hello" {
        return Err("the first message is not a hello".into());
    }
    let group_id = message["group_id"].as_str().map(str::trim).filter(|g| !g.is_empty() && g.len() <= 256 && !is_internal(g));
    Ok(Hello {
        token: message["token"].as_str().filter(|t| !t.is_empty()).map(str::to_string),
        group_id: group_id.ok_or("the hello names no group")?.to_string(),
        viewport: Viewport::parse(&message["viewport"]).ok_or("the hello has no viewport")?,
    })
}

async fn refuse<W: Sink<Message> + Unpin>(sink: &mut W, code: &str, message: &str) {
    let error = json!({ "type": "error", "code": code, "message": message });
    let _ = tokio::time::timeout(SEND_WAIT, async {
        let _ = sink.send(Message::text(error.to_string())).await;
        let _ = sink.close().await;
    })
    .await;
}

/// Serve one screen connection until it ends.
pub async fn serve<T>(socket: WebSocketStream<T>, module: Arc<BrowserModule>, authorize: Authorize)
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    let timing = module.tuning.screen.clone();
    let (mut sink, mut stream) = socket.split();
    let said = match tokio::time::timeout(timing.hello, stream.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => hello(text.as_str()).map_err(|why| ("bad_hello", why)),
        Ok(Some(Ok(Message::Binary(_)))) => Err(("bad_hello", "the first message is not a hello".to_string())),
        Ok(_) => return,
        Err(_) => Err(("hello_timeout", format!("no hello within {}s", timing.hello.as_secs()))),
    };
    let said = match said {
        Ok(said) => said,
        Err((code, why)) => return refuse(&mut sink, code, &why).await,
    };
    let who = match authorize(said.token).await {
        Ok(who) => who,
        Err(why) => return refuse(&mut sink, "unauthorized", &why).await,
    };
    let cdp = match module.ensure().await {
        Ok(cdp) => cdp,
        Err(e) => return refuse(&mut sink, "browser_unavailable", &e.message).await,
    };
    let version = cdp.send_within(None, "Browser.getVersion", json!({}), HISTORY_WAIT).await.unwrap_or(Value::Null);
    log::info!("browser: {who} looks at group {} ({}x{}{})", said.group_id, said.viewport.width, said.viewport.height, if said.viewport.mobile { ", phone" } else { "" });

    let mut changes = module.changes();
    let mut notices = module.outside.subscribe();
    let (told, mut tellings) = mpsc::unbounded_channel();
    let mut screen = Screen {
        sink,
        viewer: module.viewer(&said.group_id),
        group_id: said.group_id,
        look: Look { viewport: said.viewport, set: module.casts.stamp(), mobile: mobile_agent(&version) },
        max_fps: DEFAULT_FPS,
        jpeg_quality: DEFAULT_JPEG_QUALITY,
        shown: None,
        history_of: None,
        history: (false, false),
        state: Value::Null,
        seq: 0,
        unacked: None,
        frame_due: false,
        next_frame: Instant::now(),
        touches: Vec::new(),
        refused_at: None,
        agent: false,
        requests: HashSet::new(),
        told,
        module,
    };
    screen.module.outside.attend(&screen.module, &screen.group_id);
    let mut heard = Instant::now();
    let mut ended = screen.sync().await;
    while ended.is_ok() {
        let (wait_frame, next_frame) = (screen.frame_due && screen.unacked.is_none() && screen.shown.is_some(), screen.next_frame);
        let new_frame = async {
            match screen.shown.as_mut() {
                Some(shown) => shown.cast.frames.changed().await.is_ok(),
                None => std::future::pending().await,
            }
        };
        ended = tokio::select! {
            message = stream.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    heard = Instant::now();
                    screen.heard(text.as_str()).await
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => Err(Gone),
                Some(Ok(_)) => {
                    heard = Instant::now();
                    Ok(())
                }
            },
            changed = changes.changed() => match changed {
                Ok(()) => screen.sync().await,
                Err(_) => Err(Gone),
            },
            fresh = new_frame => {
                // A screencast that ended has no further frame: the next change of the tabs says why.
                if fresh {
                    screen.frame_due = true;
                    screen.pump().await
                } else {
                    screen.shown = None;
                    Ok(())
                }
            }
            _ = tokio::time::sleep_until(next_frame), if wait_frame => screen.pump().await,
            notice = notices.recv() => match notice {
                Ok(notice) => screen.notice(notice).await,
                Err(broadcast::error::RecvError::Lagged(_)) => screen.requests_again().await,
                Err(broadcast::error::RecvError::Closed) => Err(Gone),
            },
            telling = tellings.recv() => match telling {
                Some(Told::History { tab_id, back, forward }) => {
                    if screen.shown.as_ref().is_some_and(|shown| shown.tab_id == tab_id) {
                        screen.history = (back, forward);
                    }
                    screen.tell_state().await
                }
                Some(Told::Failed(e)) => screen.tell("error", json!({ "code": e.code, "message": e.message })).await,
                None => Err(Gone),
            },
            _ = tokio::time::sleep_until(heard + timing.silence) => {
                log::info!("browser: the screen of group {} was silent for {}s; closing it", screen.group_id, timing.silence.as_secs());
                Err(Gone)
            }
        };
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), screen.sink.close()).await;
}

impl<W: Sink<Message> + Unpin> Screen<W> {
    async fn send(&mut self, message: Message) -> Result<(), Gone> {
        match tokio::time::timeout(SEND_WAIT, self.sink.send(message)).await {
            Ok(Ok(())) => Ok(()),
            _ => Err(Gone),
        }
    }

    async fn tell(&mut self, kind: &str, mut body: Value) -> Result<(), Gone> {
        body["type"] = json!(kind);
        self.send(Message::text(body.to_string())).await
    }

    // ---- what the group is ----

    /// The group changed, or the connection is new: show the group's current
    /// tab and tell what changed.
    async fn sync(&mut self) -> Result<(), Gone> {
        // Bound as the default conversation, the group became part of the default group.
        let group_id = self.viewer.group();
        let moved = group_id != self.group_id;
        if moved {
            self.group_id = group_id;
            self.module.outside.attend(&self.module, &self.group_id);
        }
        let first = self.state.is_null();
        let switched = self.follow();
        // The group is the user's again (the agent's lease was released, ran out or was taken from it).
        let agent = self.module.controller(&self.group_id) == Controller::Agent;
        if std::mem::replace(&mut self.agent, agent) && !agent {
            if let Some(shown) = &self.shown {
                self.module.casts.dress_again(&shown.cast);
            }
        }
        self.tell_state().await?;
        if first || moved {
            self.requests_again().await?;
        }
        if switched {
            if let Some(tab_id) = self.shown.as_ref().map(|shown| shown.tab_id.clone()) {
                let focus = self.module.outside.focus(&tab_id);
                self.tell("focus", focus).await?;
            }
        }
        self.pump().await
    }

    /// Take the frames of the group's current tab. Returns whether that is another tab than before.
    fn follow(&mut self) -> bool {
        let target = match (self.module.active_tab(&self.group_id), self.module.cdp()) {
            (Some(tab), Some(cdp)) => tab.session.map(|session| (tab.tab_id, session, cdp)),
            _ => None,
        };
        let same = match (&self.shown, &target) {
            (Some(shown), Some((tab_id, session, cdp))) => shown.tab_id == *tab_id && shown.session == *session && Arc::ptr_eq(&shown.cdp, cdp),
            (None, None) => true,
            _ => false,
        };
        if same {
            return false;
        }
        self.shown = None;
        self.touches.clear();
        self.history_of = None;
        self.history = (false, false);
        let Some((tab_id, session, cdp)) = target else { return false };
        let cast = self.module.casts.watch(&self.module, &cdp, &tab_id, &session, &self.look, self.jpeg_quality);
        self.frame_due = cast.frames.borrow().is_some();
        self.shown = Some(Shown { tab_id, session, cdp, cast });
        true
    }

    async fn tell_state(&mut self) -> Result<(), Gone> {
        let group = self.module.group(&self.group_id);
        let active = group.tabs.iter().find(|tab| tab.active).cloned();
        self.read_history(active.as_ref().map(|tab| (tab.tab_id.clone(), tab.url.clone(), tab.loading)));
        let state = json!({
            "group": group,
            "active_tab_id": active.as_ref().map(|tab| tab.tab_id.as_str()),
            "url": active.as_ref().map(|tab| tab.url.as_str()).unwrap_or_default(),
            "title": active.as_ref().map(|tab| tab.title.as_str()).unwrap_or_default(),
            "loading": active.as_ref().is_some_and(|tab| tab.loading),
            "can_go_back": self.history.0,
            "can_go_forward": self.history.1,
            "viewport": self.shown.as_ref().map_or(self.look.viewport, |shown| shown.cast.viewport()),
            "running": self.module.running(),
        });
        if state == self.state {
            return Ok(());
        }
        let first = self.state.is_null();
        self.state = state.clone();
        match first {
            true => self.tell("ready", json!({ "state": state })).await,
            false => self.tell("state", state).await,
        }
    }

    /// Whether the shown tab can go back and forward is asked again when it
    /// went somewhere; the answer arrives as a `Told`.
    fn read_history(&mut self, of: Option<(String, String, bool)>) {
        let Some(shown) = self.shown.as_ref().filter(|shown| of.as_ref().is_some_and(|(tab_id, ..)| *tab_id == shown.tab_id)) else { return };
        if self.history_of == of {
            return;
        }
        self.history_of = of;
        let (cdp, session, tab_id, told) = (shown.cdp.clone(), shown.session.clone(), shown.tab_id.clone(), self.told.clone());
        tokio::spawn(async move {
            let Ok(history) = cdp.send_within(Some(&session), "Page.getNavigationHistory", json!({}), HISTORY_WAIT).await else { return };
            let (current, entries) = (history["currentIndex"].as_u64().unwrap_or(0), history["entries"].as_array().map_or(0, Vec::len) as u64);
            let _ = told.send(Told::History { tab_id, back: current > 0, forward: current + 1 < entries });
        });
    }

    // ---- frames ----

    /// Send the tab's latest frame if the client is ready for one: it
    /// acknowledged the last, and the frame rate allows it.
    async fn pump(&mut self) -> Result<(), Gone> {
        if !self.frame_due || self.unacked.is_some() || Instant::now() < self.next_frame {
            return Ok(());
        }
        let Some(shown) = self.shown.as_mut() else { return Ok(()) };
        self.frame_due = false;
        let Some(frame) = shown.cast.frames.borrow_and_update().clone() else { return Ok(()) };
        self.seq += 1;
        self.unacked = Some(self.seq);
        self.next_frame = Instant::now() + Duration::from_millis(1000 / self.max_fps as u64);
        let meta = json!({ "seq": self.seq, "tab_id": frame.tab_id, "width": frame.width, "height": frame.height });
        self.tell("frame_meta", meta).await?;
        self.send(Message::Binary(frame.jpeg.clone())).await
    }

    // ---- requests ----

    async fn notice(&mut self, notice: Notice) -> Result<(), Gone> {
        match notice {
            Notice::Request { group_id, body } if group_id.as_deref().is_none_or(|group_id| group_id == self.group_id) => self.request(body).await,
            Notice::Closed { request_id } if self.requests.remove(&request_id) => self.tell("request_closed", json!({ "request_id": request_id })).await,
            Notice::Focus { tab_id, body } if self.shown.as_ref().is_some_and(|shown| shown.tab_id == tab_id) => self.tell("focus", body).await,
            _ => Ok(()),
        }
    }

    async fn request(&mut self, body: Value) -> Result<(), Gone> {
        if matches!(body["kind"].as_str(), Some("select" | "dialog" | "file_chooser")) {
            // Shown already: it was waiting when the connection asked what waits.
            if !body["request_id"].as_str().is_some_and(|request_id| self.requests.insert(request_id.to_string())) {
                return Ok(());
            }
        }
        self.tell("request", body).await
    }

    /// Show the requests of the group that wait for an answer and were not
    /// shown on this connection; those shown that are over are closed.
    async fn requests_again(&mut self) -> Result<(), Gone> {
        let pending = self.module.outside.pending(&self.module, &self.group_id);
        let open: HashSet<String> = pending.iter().filter_map(|body| body["request_id"].as_str().map(str::to_string)).collect();
        let over: Vec<String> = self.requests.difference(&open).cloned().collect();
        for request_id in over {
            self.requests.remove(&request_id);
            self.tell("request_closed", json!({ "request_id": request_id })).await?;
        }
        for body in pending {
            self.request(body).await?;
        }
        Ok(())
    }

    // ---- what the client says ----

    async fn fail(&mut self, code: &str, message: impl Into<String>) -> Result<(), Gone> {
        self.tell("error", json!({ "code": code, "message": message.into() })).await
    }

    async fn heard(&mut self, text: &str) -> Result<(), Gone> {
        let Ok(message) = serde_json::from_str::<Value>(text) else { return self.fail("bad_message", "not JSON").await };
        let kind = message["type"].as_str().unwrap_or_default();
        if matches!(kind, "input" | "nav" | "tab") && self.module.controller(&self.group_id) == Controller::Agent {
            self.touches.clear();
            if self.refused_at.is_none_or(|at| at.elapsed() >= REFUSAL_EVERY) {
                self.refused_at = Some(Instant::now());
                return self.fail("agent_in_control", "the agent is operating the browser; take over to operate it yourself").await;
            }
            return Ok(());
        }
        match kind {
            "ack" => {
                if let (Some(sent), Some(seq)) = (self.unacked, message["seq"].as_u64()) {
                    if seq >= sent {
                        self.unacked = None;
                        return self.pump().await;
                    }
                }
                Ok(())
            }
            "ping" => self.tell("pong", json!({})).await,
            "input" => self.input(&message).await,
            "nav" => self.nav(&message).await,
            "tab" => self.tab(&message).await,
            "viewport" => match Viewport::parse(&message) {
                Some(viewport) => self.resize(viewport).await,
                None => self.fail("bad_message", "viewport needs a width and a height").await,
            },
            "quality" => {
                if let Some(fps) = message["max_fps"].as_f64().filter(|n| n.is_finite()) {
                    self.max_fps = (fps.round() as u32).clamp(1, MAX_FPS);
                }
                if let Some(quality) = message["jpeg_quality"].as_f64().filter(|n| n.is_finite()) {
                    self.jpeg_quality = quality.round().clamp(10.0, 95.0) as u8;
                    if let Some(shown) = &self.shown {
                        self.module.casts.set_quality(&shown.tab_id, &shown.session, self.jpeg_quality);
                    }
                }
                Ok(())
            }
            "respond" => {
                let Some(request_id) = message["request_id"].as_str().map(str::to_string) else {
                    return self.fail("bad_message", "respond names no request").await;
                };
                let (module, result, told) = (self.module.clone(), message["result"].clone(), self.told.clone());
                tokio::spawn(async move {
                    if let Err(e) = module.outside.respond(&module, &request_id, &result).await {
                        let _ = told.send(Told::Failed(e));
                    }
                });
                Ok(())
            }
            "hello" => Ok(()),
            other => self.fail("bad_message", format!("unknown message type `{other}`")).await,
        }
    }

    async fn input(&mut self, message: &Value) -> Result<(), Gone> {
        let commands = match input::commands(message, &mut self.touches) {
            Ok(commands) => commands,
            Err(why) => return self.fail("bad_message", why).await,
        };
        // Not waited for: an input that opens a JS dialog is answered when the dialog is closed.
        if let Some(shown) = &self.shown {
            for (method, params) in commands {
                let _ = shown.cdp.post(Some(&shown.session), method, params);
            }
        }
        Ok(())
    }

    /// The connection set its viewport: the tab it shows has it, whoever else
    /// looks at that tab, until another connection sets its own.
    async fn resize(&mut self, viewport: Viewport) -> Result<(), Gone> {
        let turned = viewport.mobile != self.look.viewport.mobile;
        (self.look.viewport, self.look.set) = (viewport, self.module.casts.stamp());
        if let Some(shown) = &self.shown {
            self.module.casts.claim(&shown.cast, &self.look, turned);
        }
        self.tell_state().await
    }

    /// Open a tab in the group as this connection's device, then go to `url`.
    fn open(&self, url: Option<String>) {
        let (module, group_id, viewport, mobile, told) = (self.module.clone(), self.group_id.clone(), self.look.viewport, self.look.mobile.clone(), self.told.clone());
        tokio::spawn(async move {
            let opened = async {
                let tab = module.open_tab(&group_id, None).await?;
                let (cdp, session) = module.tab_session(&tab.tab_id).await?;
                module.outside.prepared(&tab.tab_id).await;
                emulate(&cdp, &session, viewport, mobile.as_ref(), false).await;
                if let Some(url) = url {
                    navigate(&cdp, &session, &url).await?;
                }
                Ok::<(), BrowserError>(())
            }
            .await;
            if let Err(e) = opened {
                let _ = told.send(Told::Failed(e));
            }
        });
    }

    async fn nav(&mut self, message: &Value) -> Result<(), Gone> {
        let action = message["action"].as_str().unwrap_or_default();
        let url = match action {
            "url" => match address(message["url"].as_str().unwrap_or_default()) {
                Some(url) => Some(url),
                None => return self.fail("invalid_argument", "nav url names no address").await,
            },
            _ => None,
        };
        if url.is_some() && self.module.group(&self.group_id).tabs.is_empty() {
            self.open(url);
            return Ok(());
        }
        let Some(shown) = &self.shown else { return self.fail("no_such_tab", "the group has no tab to navigate").await };
        let (cdp, session, told) = (shown.cdp.clone(), shown.session.clone(), self.told.clone());
        match (action, url) {
            ("url", Some(url)) => {
                tokio::spawn(async move {
                    if let Err(e) = navigate(&cdp, &session, &url).await {
                        let _ = told.send(Told::Failed(e));
                    }
                });
            }
            ("reload", _) => drop(cdp.post(Some(&session), "Page.reload", json!({}))),
            ("stop", _) => drop(cdp.post(Some(&session), "Page.stopLoading", json!({}))),
            ("back" | "forward", _) => {
                let step: i64 = if action == "back" { -1 } else { 1 };
                tokio::spawn(async move {
                    let Ok(history) = cdp.send_within(Some(&session), "Page.getNavigationHistory", json!({}), HISTORY_WAIT).await else { return };
                    let at = history["currentIndex"].as_i64().unwrap_or(0) + step;
                    if let Some(entry) = usize::try_from(at).ok().and_then(|at| history["entries"].get(at)) {
                        let _ = cdp.post(Some(&session), "Page.navigateToHistoryEntry", json!({ "entryId": entry["id"] }));
                    }
                });
            }
            _ => return self.fail("bad_message", format!("unknown nav action `{action}`")).await,
        }
        Ok(())
    }

    async fn tab(&mut self, message: &Value) -> Result<(), Gone> {
        let action = message["action"].as_str().unwrap_or_default();
        if action == "open" {
            self.open(message["url"].as_str().and_then(address));
            return Ok(());
        }
        let tab_id = message["tab_id"].as_str().unwrap_or_default().to_string();
        if !self.module.tab(&tab_id).is_some_and(|tab| tab.group_id == self.group_id) {
            return self.fail("no_such_tab", format!("no tab `{tab_id}` in this group")).await;
        }
        match action {
            "activate" => match self.module.activate_tab(&tab_id) {
                Ok(_) => Ok(()),
                Err(e) => self.fail(e.code, e.message).await,
            },
            "close" => {
                let (module, told) = (self.module.clone(), self.told.clone());
                tokio::spawn(async move {
                    if let Err(e) = module.close_tab(&tab_id).await {
                        let _ = told.send(Told::Failed(e));
                    }
                });
                Ok(())
            }
            other => self.fail("bad_message", format!("unknown tab action `{other}`")).await,
        }
    }
}

/// Go to an address. What the browser refuses is an error; a page that
/// does not load shows the browser's own error page.
async fn navigate(cdp: &CdpClient, session: &str, url: &str) -> Result<(), BrowserError> {
    match cdp.send_within(Some(session), "Page.navigate", json!({ "url": url }), NAVIGATE_WAIT).await {
        Err(e @ CdpError::Remote { .. }) => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hello_names_the_group_and_the_viewport() {
        let said = hello(r#"{"type":"hello","token":"t","group_id":"default","viewport":{"width":412,"height":915,"mobile":true}}"#).unwrap();
        assert_eq!((said.token.as_deref(), said.group_id.as_str()), (Some("t"), "default"));
        assert_eq!(said.viewport, Viewport { width: 412, height: 915, mobile: true });
        // Sizes are kept within what a page can be shown at.
        let said = hello(r#"{"type":"hello","group_id":"g","viewport":{"width":100,"height":9000.4}}"#).unwrap();
        assert_eq!((said.token, said.viewport), (None, Viewport { width: 320, height: 2560, mobile: false }));
        for bad in [
            "not json",
            r#"{"type":"ack","seq":1}"#,
            r#"{"type":"hello","viewport":{"width":1,"height":1}}"#,
            r#"{"type":"hello","group_id":"","viewport":{"width":1,"height":1}}"#,
            r#"{"type":"hello","group_id":"_reflow","viewport":{"width":1,"height":1}}"#,
            r#"{"type":"hello","group_id":"g"}"#,
            r#"{"type":"hello","group_id":"g","viewport":{"width":"wide","height":1}}"#,
        ] {
            assert!(hello(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_phone_gets_a_phones_user_agent_of_the_browsers_version() {
        let (agent, metadata) = mobile_agent(&json!({ "product": "Chrome/159.0.8154.6" })).unwrap();
        assert_eq!(agent, "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/159.0.0.0 Mobile Safari/537.36");
        assert_eq!((metadata["mobile"].clone(), metadata["platform"].clone(), metadata["fullVersion"].clone()), (json!(true), json!("Android"), json!("159.0.8154.6")));
        assert!(mobile_agent(&Value::Null).is_none());
    }

    #[test]
    fn what_is_typed_becomes_an_address() {
        assert_eq!(address(" example.com/a ").as_deref(), Some("https://example.com/a"));
        assert_eq!(address("http://127.0.0.1:8080/").as_deref(), Some("http://127.0.0.1:8080/"));
        assert_eq!(address("about:blank").as_deref(), Some("about:blank"));
        assert_eq!(address("   "), None);
    }
}
