//! The screen (Agent Browser Protocol §3): a WebSocket over which the owner
//! sees the browser as a stream of JPEG frames and operates it. A connection
//! says `hello` first and is admitted as the owner; from then on it is told
//! the group's state whenever it changes, gets at most one frame it has not
//! acknowledged, and what it sends is applied — unless the agent operates
//! the group. A headed browser's frames are its whole display, and what the
//! user does is done on the display with its pointer and keyboard; a
//! headless one's are the current tab's, operated through DevTools.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
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
use super::display::{Display, Painting, Want};
use super::groups::{Controller, DEFAULT_GROUP};
use super::rfb;
use super::input;
use super::process;
use super::requests::{send_settled, Notice};
use super::{BrowserError, BrowserModule, Viewer};

/// Who the `hello` of a connection is, given its token: the caller's name,
/// or why they are not admitted.
pub type Authorize = Box<dyn FnOnce(Option<String>) -> BoxFuture<'static, Result<String, String>> + Send>;

const VIEWPORT_MIN: f64 = 320.0;
const VIEWPORT_MAX: f64 = 2560.0;
/// The most pixels of a frame a CSS pixel can be: that of the browser's dense screen (`process::SCREENS`).
const PIXEL_RATIO_MAX: f64 = 2.0;
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
    /// How many pixels of a frame a CSS pixel is, 1–2: the density of the screen that shows the frames.
    pixel_ratio: f64,
}

impl Viewport {
    /// `pixel_ratio` is the one of a value that names none.
    fn parse(value: &Value, pixel_ratio: f64) -> Option<Self> {
        let side = |name: &str| value[name].as_f64().filter(|n| n.is_finite()).map(|n| n.round().clamp(VIEWPORT_MIN, VIEWPORT_MAX) as u32);
        Some(Self {
            width: side("width")?,
            height: side("height")?,
            mobile: value["mobile"].as_bool().unwrap_or(false),
            pixel_ratio: pixel_ratio_of(value).unwrap_or(pixel_ratio),
        })
    }

    /// The tab is painted on the browser's dense screen.
    fn dense(&self) -> bool {
        self.pixel_ratio > 1.0
    }

    /// The largest frame of the screencast: a ratio between the two screens' is the dense screen's frame made smaller.
    fn limit(&self) -> Option<(u32, u32)> {
        let side = |n: u32| (n as f64 * self.pixel_ratio).round() as u32;
        (self.dense() && self.pixel_ratio < PIXEL_RATIO_MAX).then(|| (side(self.width), side(self.height)))
    }
}

/// The `pixel_ratio` a message names, kept within what the browser's screens have.
fn pixel_ratio_of(value: &Value) -> Option<f64> {
    value["pixel_ratio"].as_f64().filter(|n| n.is_finite()).map(|n| (n.clamp(1.0, PIXEL_RATIO_MAX) * 100.0).round() / 100.0)
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

/// Put a tab's window on the browser's screen of its pixel ratio: the first
/// one, or the dense one beside it (`process::SCREENS`). A window on the
/// dense screen paints frames of twice its pixels; its page sees no
/// difference once it is made its device again (a window that moves loses
/// the metrics it was given). One that is there already does not move.
async fn place(cdp: &CdpClient, session: &str, dense: bool) {
    let Ok(window) = send_settled(cdp, session, "Browser.getWindowForTarget", json!({})).await else { return };
    let Some(window_id) = window["windowId"].as_i64() else { return };
    let left = if dense { process::DENSE_SCREEN_LEFT } else { 0 };
    let _ = send_settled(cdp, session, "Browser.setWindowBounds", json!({ "windowId": window_id, "bounds": { "left": left, "top": 0 } })).await;
}

/// Make a tab look to its page like the device that shows it: a phone
/// (size, touch, a phone's user agent) or a desktop window of this size, on
/// the screen of its pixel ratio. `again`: the tab may have been made that device before.
async fn emulate(cdp: &CdpClient, session: &str, viewport: Viewport, mobile: Option<&(String, Value)>, again: bool) {
    let Viewport { width, height, mobile: phone, .. } = viewport;
    place(cdp, session, viewport.dense()).await;
    let metrics = match phone {
        true => json!({ "width": width, "height": height, "deviceScaleFactor": MOBILE_SCALE, "mobile": true, "screenWidth": width, "screenHeight": height }),
        // Not the screen's: on the dense one a page would be twice as dense.
        false => json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }),
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

/// How a connection looks at tabs: the viewport it asks for, the picture
/// quality of its frames, when it set its viewport (`Casts::stamp`), and
/// what a phone tells sites it is (`mobile_agent`).
struct Look {
    viewport: Viewport,
    quality: u8,
    set: u64,
    mobile: Option<(String, Value)>,
}

/// A connection that shows a tab.
struct Viewing {
    id: u64,
    viewport: Viewport,
    quality: u8,
    set: u64,
}

/// How a tab's screencast paints: for which viewport (its pixel ratio), at what quality.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Paint {
    viewport: Viewport,
    quality: u8,
}

impl Paint {
    /// What the screencast is started with: it is started again when that changes.
    fn cast(&self) -> (u8, bool, Option<(u32, u32)>) {
        (self.quality, self.viewport.dense(), self.viewport.limit())
    }
}

/// What a tab is to its page: the look of the viewer that set its viewport
/// last, how often the page was to be asked for again as that device, and
/// how often the tab was to be made that device once more.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Device {
    paint: Paint,
    reloads: u64,
    dresses: u64,
}

struct Cast {
    cdp: Arc<CdpClient>,
    session: String,
    viewers: Vec<Viewing>,
    device: watch::Sender<Device>,
    frames: watch::Receiver<Option<Arc<Frame>>>,
    task: tokio::task::JoinHandle<()>,
    dresser: tokio::task::JoinHandle<()>,
}

impl Cast {
    /// A tab has one look: that of the viewer that set its viewport last.
    /// Returns whether the tab is to be made another device, painted otherwise, or asked for again.
    fn settle(&self, reload: bool) -> bool {
        let Some(paint) = self.viewers.iter().max_by_key(|viewing| viewing.set).map(|viewing| Paint { viewport: viewing.viewport, quality: viewing.quality }) else { return false };
        self.device.send_if_modified(|device| {
            let changed = device.paint != paint || reload;
            device.paint = paint;
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
        self.device.borrow().paint.viewport
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.module.casts.leave(&self.module, &self.tab_id, &self.session, self.id);
    }
}

/// Keep a tab the device its viewers settled on, and tell its screencast what it was made and how to paint it (`dressed`).
async fn dress(cdp: Arc<CdpClient>, session: String, mobile: Option<(String, Value)>, mut device: watch::Receiver<Device>, dressed: watch::Sender<Option<Paint>>) {
    let mut worn: Option<Device> = None;
    loop {
        let wanted = *device.borrow_and_update();
        let viewport = wanted.paint.viewport;
        let made = worn.map(|worn| (worn.paint.viewport, worn.dresses)) != Some((viewport, wanted.dresses));
        if made {
            // What the tab was before this screencast is not known.
            let again = worn.is_none_or(|worn| worn.paint.viewport == viewport);
            emulate(&cdp, &session, viewport, mobile.as_ref(), again).await;
        }
        if made || worn.is_some_and(|worn| worn.paint.quality != wanted.paint.quality) {
            dressed.send_replace(Some(wanted.paint));
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

fn start_cast(cdp: &CdpClient, session: &str, paint: &Paint) {
    // The page domain is on by the time the module is done attaching; a screen may be earlier.
    let _ = cdp.post(Some(session), "Page.enable", json!({}));
    // Every frame: the browser's own thinning would drop the last frame of a change.
    let mut params = json!({ "format": "jpeg", "quality": paint.quality, "everyNthFrame": 1 });
    if let Some((width, height)) = paint.viewport.limit() {
        (params["maxWidth"], params["maxHeight"]) = (json!(width), json!(height));
    }
    let _ = cdp.post(Some(session), "Page.startScreencast", params);
}

/// What a screencast's task hears of its tab.
const CAST_EVENTS: &[&str] = &["Page.screencastFrame", "Page.frameStoppedLoading", "Page.screencastVisibilityChanged"];

async fn feed(
    cdp: Arc<CdpClient>,
    tab_id: String,
    session: String,
    mut events: EventStream,
    frames: watch::Sender<Option<Arc<Frame>>>,
    first: Paint,
    mut dressed: watch::Receiver<Option<Paint>>,
) {
    // How the screencast paints. It starts once the tab is made its device,
    // and again when it is to paint at another quality or pixel ratio.
    let mut painting: Option<Paint> = None;
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
                start_cast(&cdp, &session, painting.get_or_insert(first));
                continue;
            }
            Ok(()) = dressed.changed() => {
                let Some(paint) = *dressed.borrow_and_update() else { continue };
                resized = Some((paint.viewport.width as u64, paint.viewport.height as u64)).filter(|size| Some(*size) != painted);
                if painting.map(|painting| painting.cast()) != Some(paint.cast()) {
                    if painting.is_some() {
                        let _ = cdp.post(Some(&session), "Page.stopScreencast", json!({}));
                    }
                    start_cast(&cdp, &session, &paint);
                    (restarts, owed) = (0, Some(Instant::now() + FIRST_FRAME_WAIT));
                } else if resized.is_some() {
                    restarts = 0;
                    owed = owed.or(Some(Instant::now() + LOADED_FRAME_WAIT));
                }
                painting = Some(paint);
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

    fn watch(&self, module: &Arc<BrowserModule>, cdp: &Arc<CdpClient>, tab_id: &str, session: &str, look: &Look) -> Watching {
        let viewing = Viewing { id: self.stamp(), viewport: look.viewport, quality: look.quality, set: look.set };
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
                let paint = Paint { viewport: look.viewport, quality: look.quality };
                let (device, devices) = watch::channel(Device { paint, reloads: 0, dresses: 0 });
                let (dressed, dressing) = watch::channel(None);
                let events = cdp.subscribe(Scope::Session(session.to_string()), CAST_EVENTS);
                let task = tokio::spawn(feed(cdp.clone(), tab_id.to_string(), session.to_string(), events, publish, paint, dressing));
                let dresser = tokio::spawn(dress(cdp.clone(), session.to_string(), look.mobile.clone(), devices.clone(), dressed));
                let cast = Cast { cdp: cdp.clone(), session: session.to_string(), viewers: vec![viewing], device, frames: frames.clone(), task, dresser };
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

    /// A viewer changed its look. One that set its viewport (a later
    /// `look.set`) gives the tab its look from now on, and with `reload` its
    /// page is asked for again as that device.
    fn claim(&self, watching: &Watching, look: &Look, reload: bool) {
        let changed = {
            let mut tabs = self.tabs.lock().expect("casts");
            let Some(cast) = tabs.get_mut(&watching.tab_id).filter(|cast| cast.session == watching.session) else { return };
            let Some(viewing) = cast.viewers.iter_mut().find(|viewing| viewing.id == watching.id) else { return };
            (viewing.viewport, viewing.quality, viewing.set) = (look.viewport, look.quality, look.set);
            cast.settle(reload)
        };
        if changed {
            watching.module.bump();
        }
    }

    /// The agent is done with a tab: what its Playwright did may have ended
    /// the device the tab was made (a screenshot does), so it is made that
    /// device once more, on the screen of its pixel ratio.
    fn dress_again(&self, watching: &Watching) {
        let tabs = self.tabs.lock().expect("casts");
        if let Some(cast) = tabs.get(&watching.tab_id).filter(|cast| cast.session == watching.session) {
            cast.device.send_modify(|device| device.dresses += 1);
        }
    }

    /// A viewer is gone. The tab keeps its look if it was the last
    /// one; else it gets the look of those that are left.
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
    /// The tab's screencast (headless; a headed browser's frames are its display's).
    cast: Option<Watching>,
}

/// A connection's look at a headed browser's display, and what it holds
/// down on it.
struct DisplayView {
    display: Arc<Display>,
    id: u64,
    frames: watch::Receiver<Option<Arc<Frame>>>,
    painting: watch::Receiver<Painting>,
    buttons: u8,
    wheel_rest: (f64, f64),
    /// Modifier keys this connection holds down (keysyms).
    held: Vec<u32>,
    /// A finger on the display: where it went down, where it is, whether it scrolls.
    finger: Option<(f64, f64, f64, f64, bool)>,
}

impl Drop for DisplayView {
    fn drop(&mut self) {
        for keysym in std::mem::take(&mut self.held) {
            self.display.key(keysym, false);
        }
        if self.buttons != 0 {
            self.display.pointer(0.0, 0.0, 0);
        }
        self.display.leave(self.id);
    }
}

/// A finger that moved this far (CSS pixels) scrolls instead of tapping.
const TAP_SLOP: f64 = 8.0;

impl DisplayView {
    fn new(display: Arc<Display>, id: u64, want: Want) -> Self {
        display.watch(id, want);
        Self { frames: display.frames(), painting: display.painting(), display, id, buttons: 0, wheel_rest: (0.0, 0.0), held: Vec::new(), finger: None }
    }

    /// The viewport the display has (a desktop window: no phone is made of it).
    fn viewport(&self) -> Viewport {
        let p = *self.painting.borrow();
        Viewport { width: p.width, height: p.height, mobile: false, pixel_ratio: p.pixel_ratio }
    }

    /// Hold down the modifier keys of the protocol's bits, and only those.
    fn modifiers(&mut self, bits: i64) {
        for (bit, keysym) in rfb::MODIFIER_KEYSYMS {
            let held = self.held.contains(&keysym);
            if bits & bit != 0 && !held {
                self.display.key(keysym, true);
                self.held.push(keysym);
            } else if bits & bit == 0 && held {
                self.display.key(keysym, false);
                self.held.retain(|k| *k != keysym);
            }
        }
    }

    fn click(&mut self, x: f64, y: f64) {
        self.display.pointer(x, y, self.buttons);
        self.display.pointer(x, y, self.buttons | 1);
        self.display.pointer(x, y, self.buttons);
    }

    /// An `input` message, done on the display.
    fn input(&mut self, message: &Value) -> Result<(), String> {
        let number = |m: &Value, name: &str| m[name].as_f64().filter(|n| n.is_finite()).ok_or_else(|| format!("input `{name}` is not a number"));
        let modifiers = message["modifiers"].as_i64().unwrap_or(0) & 0xf;
        match message["kind"].as_str() {
            Some("mouse") => {
                let (x, y) = (number(message, "x")?, number(message, "y")?);
                let bit = match message["button"].as_str().unwrap_or("none") {
                    "left" => 1,
                    "middle" => 2,
                    "right" => 4,
                    "none" => 0,
                    other => return Err(format!("unknown mouse button `{other}`")),
                };
                match message["action"].as_str() {
                    Some("down") => self.buttons |= bit,
                    Some("up") => self.buttons &= !bit,
                    Some("move") => self.buttons = bit,
                    other => return Err(format!("unknown mouse action {other:?}")),
                }
                self.modifiers(modifiers);
                self.display.pointer(x, y, self.buttons);
            }
            Some("wheel") => {
                let (x, y) = (number(message, "x")?, number(message, "y")?);
                let delta = (message["delta_x"].as_f64().unwrap_or(0.0), message["delta_y"].as_f64().unwrap_or(0.0));
                self.modifiers(modifiers);
                self.display.wheel(x, y, self.buttons, delta, &mut self.wheel_rest);
            }
            Some("key") => {
                let (key, code) = (message["key"].as_str().unwrap_or_default(), message["code"].as_str().unwrap_or_default());
                if key.is_empty() && code.is_empty() {
                    return Err("a key input names no key".into());
                }
                let down = match message["action"].as_str() {
                    Some("down") => true,
                    Some("up") => false,
                    other => return Err(format!("unknown key action {other:?}")),
                };
                let single = |text: &str| {
                    let mut chars = text.chars();
                    chars.next().filter(|_| chars.next().is_none())
                };
                let keysym = rfb::named_keysym(key, code)
                    .or_else(|| single(message["text"].as_str().unwrap_or_default()).map(rfb::char_keysym))
                    .or_else(|| single(key).map(rfb::char_keysym));
                // A key the input method owns, or none known: its text arrives as `text`.
                let Some(keysym) = keysym else { return Ok(()) };
                if rfb::MODIFIER_KEYSYMS.iter().any(|(_, k)| *k == keysym) || matches!(keysym, 0xffe2 | 0xffe4 | 0xffea | 0xffec) {
                    match down {
                        true if !self.held.contains(&keysym) => self.held.push(keysym),
                        false => self.held.retain(|k| *k != keysym),
                        _ => {}
                    }
                } else if down {
                    self.modifiers(modifiers);
                }
                self.display.key(keysym, down);
            }
            Some("text") => {
                let text = message["text"].as_str().ok_or("input `text` is not a string")?;
                self.modifiers(0);
                self.display.type_text(text);
            }
            // The input method shows what it composes itself; the page gets the text it commits.
            Some("composition") => {
                message["text"].as_str().ok_or("input `text` is not a string")?;
            }
            Some("touch") => {
                let point = message["points"].as_array().and_then(|p| p.first()).map(|p| (number(p, "x"), number(p, "y")));
                match message["phase"].as_str() {
                    Some("start") => {
                        let (x, y) = match point {
                            Some((x, y)) => (x?, y?),
                            None => return Err("a touch start or move names no point".into()),
                        };
                        self.finger = Some((x, y, x, y, false));
                        self.display.pointer(x, y, self.buttons);
                    }
                    Some("move") => {
                        let (x, y) = match point {
                            Some((x, y)) => (x?, y?),
                            None => return Err("a touch start or move names no point".into()),
                        };
                        let Some((sx, sy, lx, ly, scrolling)) = self.finger else { return Ok(()) };
                        let scrolling = scrolling || (x - sx).hypot(y - sy) > TAP_SLOP;
                        if scrolling {
                            // The page goes with the finger: a wheel the other way.
                            self.display.wheel(sx, sy, self.buttons, (lx - x, ly - y), &mut self.wheel_rest);
                        }
                        self.finger = Some((sx, sy, x, y, scrolling));
                    }
                    Some("end") => {
                        if let Some((sx, sy, _, _, false)) = self.finger.take() {
                            self.click(sx, sy);
                        }
                    }
                    Some("cancel") => self.finger = None,
                    other => return Err(format!("unknown touch phase {other:?}")),
                }
            }
            other => return Err(format!("unknown input kind {other:?}")),
        }
        Ok(())
    }
}

struct Screen<W> {
    sink: W,
    module: Arc<BrowserModule>,
    /// What the connection looks at of a headed browser's display.
    display: Option<DisplayView>,
    /// Somebody looks at the group while the connection lasts.
    _viewer: Viewer,
    group_id: String,
    look: Look,
    max_fps: u32,
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
    viewport: Viewport,
}

fn hello(text: &str) -> Result<Hello, String> {
    let message: Value = serde_json::from_str(text).map_err(|e| format!("the hello is not JSON: {e}"))?;
    if message["type"] != "hello" {
        return Err("the first message is not a hello".into());
    }
    if !message["group_id"].is_null() && message["group_id"] != DEFAULT_GROUP {
        return Err(format!("the agent's browser has one group, `{DEFAULT_GROUP}`"));
    }
    Ok(Hello {
        token: message["token"].as_str().filter(|t| !t.is_empty()).map(str::to_string),
        viewport: Viewport::parse(&message["viewport"], 1.0).ok_or("the hello has no viewport")?,
    })
}

async fn refuse<W: Sink<Message> + Unpin>(sink: &mut W, code: &str, message: &str) {
    refuse_with(sink, json!({ "code": code, "message": message })).await
}

/// End a connection that is not admitted with `error` (`{code, message, …}`).
async fn refuse_with<W: Sink<Message> + Unpin>(sink: &mut W, mut error: Value) {
    error["type"] = json!("error");
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
        Err(e) => return refuse_with(&mut sink, BrowserError { code: "browser_unavailable", ..e }.to_json()).await,
    };
    let headed = module.headed();
    let version = match headed {
        true => Value::Null,
        false => cdp.send_within(None, "Browser.getVersion", json!({}), HISTORY_WAIT).await.unwrap_or(Value::Null),
    };
    let Viewport { width, height, mobile, pixel_ratio } = said.viewport;
    log::info!("browser: {who} looks at the browser ({width}x{height}{}, pixel ratio {pixel_ratio})", if mobile { ", phone" } else { "" });

    let mut changes = module.changes();
    let mut notices = module.outside.subscribe();
    let (told, mut tellings) = mpsc::unbounded_channel();
    let mut screen = Screen {
        sink,
        display: None,
        _viewer: module.viewer(DEFAULT_GROUP),
        group_id: DEFAULT_GROUP.to_string(),
        look: Look { viewport: said.viewport, quality: DEFAULT_JPEG_QUALITY, set: module.casts.stamp(), mobile: mobile_agent(&version) },
        max_fps: DEFAULT_FPS,
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
            match (screen.display.as_mut(), screen.shown.as_mut()) {
                (Some(view), Some(_)) => view.frames.changed().await.is_ok(),
                (None, Some(Shown { cast: Some(cast), .. })) => cast.frames.changed().await.is_ok(),
                _ => std::future::pending().await,
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
                // A screencast or display that ended has no further frame: the next change of the tabs says why.
                if fresh {
                    screen.frame_due = true;
                    screen.pump().await
                } else {
                    screen.shown = None;
                    screen.display = None;
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
                Some(Told::Failed(e)) => screen.tell("error", e.to_json()).await,
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
        log::trace!("browser: screen sync");
        let first = self.state.is_null();
        let switched = self.follow();
        // The group is the user's again (the agent's lease was released, ran out or was taken from it).
        let agent = self.module.controller(&self.group_id) == Controller::Agent;
        if std::mem::replace(&mut self.agent, agent) && !agent {
            if let Some(Shown { cast: Some(cast), .. }) = &self.shown {
                self.module.casts.dress_again(cast);
            }
        }
        self.tell_state().await?;
        if first {
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
        self.see_display();
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
        let cast = match &self.display {
            Some(view) => {
                self.frame_due = view.frames.borrow().is_some();
                None
            }
            None if self.module.headed() => None,
            None => {
                let cast = self.module.casts.watch(&self.module, &cdp, &tab_id, &session, &self.look);
                self.frame_due = cast.frames.borrow().is_some();
                Some(cast)
            }
        };
        self.shown = Some(Shown { tab_id, session, cdp, cast });
        true
    }

    /// Look at the display of a headed browser: the one it has now (a
    /// browser that was started again has another).
    fn see_display(&mut self) {
        let display = self.module.display();
        let same = match (&self.display, &display) {
            (Some(view), Some(display)) => Arc::ptr_eq(&view.display, display),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.display = display.map(|display| DisplayView::new(display, self.module.casts.stamp(), self.want()));
        self.frame_due = self.display.as_ref().is_some_and(|view| view.frames.borrow().is_some());
    }

    /// How the connection looks at a display.
    fn want(&self) -> Want {
        let Viewport { width, height, pixel_ratio, .. } = self.look.viewport;
        Want { width, height, pixel_ratio, quality: self.look.quality, max_fps: self.max_fps, set: self.look.set }
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
            "viewport": match (&self.display, &self.shown) {
                (Some(view), _) => view.viewport(),
                (None, Some(Shown { cast: Some(cast), .. })) => cast.viewport(),
                _ => self.look.viewport,
            },
            "running": self.module.running(),
        });
        if state == self.state {
            return Ok(());
        }
        let first = self.state.is_null();
        self.state = state.clone();
        log::trace!("browser: screen state {}", state["group"]["tabs"]);
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
        let frames = match (self.display.as_mut(), self.shown.as_mut()) {
            (Some(view), Some(_)) => &mut view.frames,
            (None, Some(Shown { cast: Some(cast), .. })) => &mut cast.frames,
            _ => return Ok(()),
        };
        self.frame_due = false;
        let Some(frame) = frames.borrow_and_update().clone() else { return Ok(()) };
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
        if kind != "ack" && kind != "ping" {
            log::debug!("browser: the screen says {kind}");
        }
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
            "viewport" => match Viewport::parse(&message, self.look.viewport.pixel_ratio) {
                Some(viewport) => self.resize(viewport).await,
                None => self.fail("bad_message", "viewport needs a width and a height").await,
            },
            "quality" => {
                if let Some(fps) = message["max_fps"].as_f64().filter(|n| n.is_finite()) {
                    self.max_fps = (fps.round() as u32).clamp(1, MAX_FPS);
                    self.claim_display();
                }
                let quality = message["jpeg_quality"].as_f64().filter(|n| n.is_finite()).map_or(self.look.quality, |n| n.round().clamp(10.0, 95.0) as u8);
                let pixel_ratio = pixel_ratio_of(&message).unwrap_or(self.look.viewport.pixel_ratio);
                if (quality, pixel_ratio) == (self.look.quality, self.look.viewport.pixel_ratio) {
                    return Ok(());
                }
                (self.look.quality, self.look.viewport.pixel_ratio) = (quality, pixel_ratio);
                // Not a viewport set: the tab is painted so while the viewport it has is this connection's.
                if let Some(Shown { cast: Some(cast), .. }) = &self.shown {
                    self.module.casts.claim(cast, &self.look, false);
                }
                self.claim_display();
                self.tell_state().await
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
        if let Some(view) = self.display.as_mut() {
            return match view.input(message) {
                Ok(()) => Ok(()),
                Err(why) => self.fail("bad_message", why).await,
            };
        }
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
        if let Some(Shown { cast: Some(cast), .. }) = &self.shown {
            self.module.casts.claim(cast, &self.look, turned);
        }
        self.claim_display();
        self.tell_state().await
    }

    /// The display is looked at as this connection looks now.
    fn claim_display(&self) {
        if let Some(view) = &self.display {
            view.display.claim(view.id, self.want());
        }
    }

    /// Open a tab in the group as this connection's device, then go to `url`.
    fn open(&self, url: Option<String>) {
        let (module, group_id, viewport, mobile, told) = (self.module.clone(), self.group_id.clone(), self.look.viewport, self.look.mobile.clone(), self.told.clone());
        tokio::spawn(async move {
            let opened = async {
                let tab = module.open_tab(&group_id, None).await?;
                let (cdp, session) = module.tab_session(&tab.tab_id).await?;
                module.outside.prepared(&tab.tab_id).await;
                if !module.headed() {
                    emulate(&cdp, &session, viewport, mobile.as_ref(), false).await;
                }
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
        // The current tab, once the module is attached to it (a tab the browser opened again after a start is not yet).
        let (cdp, session) = match (&self.shown, self.module.active_tab(&self.group_id)) {
            (Some(shown), _) => (shown.cdp.clone(), shown.session.clone()),
            (None, Some(tab)) => match self.module.tab_session(&tab.tab_id).await {
                Ok(attached) => attached,
                Err(e) => return self.tell("error", e.to_json()).await,
            },
            (None, None) => return self.fail("no_such_tab", "the group has no tab to navigate").await,
        };
        let told = self.told.clone();
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
                Err(e) => self.tell("error", e.to_json()).await,
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
    fn a_hello_names_the_viewport_and_at_most_the_one_group() {
        let said = hello(r#"{"type":"hello","token":"t","group_id":"default","viewport":{"width":412,"height":915,"mobile":true}}"#).unwrap();
        assert_eq!(said.token.as_deref(), Some("t"));
        assert_eq!(said.viewport, Viewport { width: 412, height: 915, mobile: true, pixel_ratio: 1.0 });
        // Without a group it is the one there is. Sizes are kept within what a page can be shown at,
        // pixel ratios within what the screens have.
        let said = hello(r#"{"type":"hello","viewport":{"width":100,"height":9000.4,"pixel_ratio":0.5}}"#).unwrap();
        assert_eq!((said.token, said.viewport), (None, Viewport { width: 320, height: 2560, mobile: false, pixel_ratio: 1.0 }));
        for (asked, ratio) in [(2.625, 2.0), (1.5, 1.5), (1.333, 1.33), (1.0, 1.0)] {
            let said = hello(&json!({ "type": "hello", "viewport": { "width": 412, "height": 915, "pixel_ratio": asked } }).to_string()).unwrap();
            assert_eq!(said.viewport.pixel_ratio, ratio, "{asked}");
        }
        // Between the screens' ratios the dense screen's frames are made smaller.
        let at = |pixel_ratio: f64| Viewport { width: 412, height: 780, mobile: true, pixel_ratio };
        assert_eq!([at(1.0).limit(), at(1.5).limit(), at(2.0).limit()], [None, Some((618, 1170)), None]);
        assert_eq!([at(1.0).dense(), at(1.01).dense(), at(2.0).dense()], [false, true, true]);
        for bad in [
            "not json",
            r#"{"type":"ack","seq":1}"#,
            r#"{"type":"hello","group_id":"thing","viewport":{"width":412,"height":915}}"#,
            r#"{"type":"hello","group_id":"","viewport":{"width":412,"height":915}}"#,
            r#"{"type":"hello","group_id":"_reflow","viewport":{"width":412,"height":915}}"#,
            r#"{"type":"hello","group_id":"default"}"#,
            r#"{"type":"hello","group_id":"default","viewport":{"width":"wide","height":1}}"#,
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
