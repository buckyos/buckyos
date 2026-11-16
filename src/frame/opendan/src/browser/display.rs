//! The browser's display on Linux: a virtual X server with a VNC server in
//! it (TigerVNC's `Xvnc`), started with the browser and ended after it.
//! The browser runs headed on it, as it would on a desktop: its windows,
//! its own dialogs, menus and prompts are all on the display. The screens
//! see the whole display as frames and operate it with the pointer and the
//! keyboard (`rfb`), not through the DevTools protocol.
//!
//! The display has a fixed device scale factor (`[loader.browser]
//! device_scale_factor`): the browser draws at it, pages see it as their
//! `devicePixelRatio`. Its size in CSS pixels follows the screen that set
//! its viewport last; frames are made smaller for a client that asks for
//! fewer pixels, the page never changes for that.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use serde::Serialize;
use tokio::sync::watch;
use tokio::time::Instant;

use super::process::BrowserDirs;
use super::rfb::{self, Rfb};
use super::screen::Frame;

pub const ENV_XVNC: &str = "OPENDAN_XVNC";
const START_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_WAIT: Duration = Duration::from_secs(3);
/// The first display number tried (lower ones are a desktop's).
const FIRST_DISPLAY: u32 = 20;
const DISPLAYS: u32 = 80;
/// CSS pixels the browser scrolls for one notch of a wheel on X11.
const WHEEL_STEP: f64 = 53.0;
const MAX_WHEEL_NOTCHES: i64 = 20;
/// The display's size until a screen sets a viewport, in CSS pixels.
pub const DEFAULT_SIZE: (u32, u32) = (1280, 800);
/// The narrowest window the browser makes (CSS pixels): a narrower
/// viewport (a phone's) gets a display this wide, shown made smaller.
pub const MIN_WIDTH: u32 = 500;

/// How the browser is shown (`[loader.browser] display`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    /// The platform's: `virtual` on Linux, `headless` elsewhere.
    #[default]
    Auto,
    /// Headed on a virtual display (Linux).
    Virtual,
    /// Headless: frames and input through the DevTools protocol, the
    /// page made the device that looks at it.
    Headless,
}

impl DisplayMode {
    pub fn resolved(self) -> Self {
        match self {
            Self::Auto if cfg!(target_os = "linux") => Self::Virtual,
            Self::Auto => Self::Headless,
            other => other,
        }
    }
}

/// `Xvnc`: `OPENDAN_XVNC`, else the one on `PATH`.
pub fn find_xvnc() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(ENV_XVNC).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path)).filter(|p| p.is_file());
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|dir| dir.join("Xvnc")).find(|p| p.is_file())
}

fn log_tail(file: &Path) -> String {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('>') && !l.contains("Could not resolve keysym")).collect();
    lines[lines.len().saturating_sub(6)..].join(" | ")
}

/// The X server process.
pub struct Xvnc {
    pub number: u32,
    pub socket: PathBuf,
    child: tokio::process::Child,
    log: PathBuf,
}

impl Xvnc {
    /// `DISPLAY` for the browser.
    pub fn display(&self) -> String {
        format!(":{}", self.number)
    }

    /// Start a display of `width`x`height` pixels. A display number some
    /// other X server holds is passed over.
    pub async fn start(xvnc: &Path, dirs: &BrowserDirs, width: u32, height: u32, dpi: u32) -> Result<Self, String> {
        std::fs::create_dir_all(&dirs.root).map_err(|e| format!("create {}: {e}", dirs.root.display()))?;
        let log = dirs.root.join("display.log");
        let mut last = String::new();
        for number in FIRST_DISPLAY..FIRST_DISPLAY + DISPLAYS {
            if Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() || Path::new(&format!("/tmp/.X{number}-lock")).exists() {
                continue;
            }
            // Short: a socket's path has about a hundred bytes.
            let socket = PathBuf::from(format!("/tmp/opendan-display-{number}.rfb"));
            let _ = std::fs::remove_file(&socket);
            let file = std::fs::File::create(&log).map_err(|e| format!("create {}: {e}", log.display()))?;
            let mut cmd = tokio::process::Command::new(xvnc);
            cmd.arg(format!(":{number}"))
                .args(["-geometry", &format!("{width}x{height}"), "-depth", "24", "-dpi", &dpi.to_string()])
                .args(["-rfbport", "-1", "-rfbunixpath"])
                .arg(&socket)
                .args(["-rfbunixmode", "0600", "-SecurityTypes", "None", "-AlwaysShared", "-AcceptSetDesktopSize", "-nolisten", "tcp"])
                .args(["-desktop", "Agent browser"])
                .stdin(std::process::Stdio::null())
                .stdout(file.try_clone().map_err(|e| e.to_string())?)
                .stderr(file)
                .kill_on_drop(true);
            #[cfg(unix)]
            cmd.process_group(0);
            let mut child = cmd.spawn().map_err(|e| format!("start {}: {e}", xvnc.display()))?;
            let started = Instant::now();
            loop {
                if socket.exists() && Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() {
                    log::info!("browser: display :{number} started ({width}x{height}, {dpi} dpi)");
                    return Ok(Self { number, socket, child, log });
                }
                if let Ok(Some(status)) = child.try_wait() {
                    last = format!("Xvnc :{number} ended while starting ({status}): {}", log_tail(&log));
                    break;
                }
                if started.elapsed() > START_TIMEOUT {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(format!("Xvnc :{number} did not start in {}s: {}", START_TIMEOUT.as_secs(), log_tail(&log)));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // Another server took the number between the check and the start: the next one.
            if !last.contains("already active") {
                return Err(last);
            }
        }
        Err(if last.is_empty() { format!("no free X display number from :{FIRST_DISPLAY} on") } else { last })
    }

    /// Whether the server has ended (`Some`: how).
    pub fn ended(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("{status}: {}", log_tail(&self.log))),
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    pub async fn stop(mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
        if tokio::time::timeout(STOP_WAIT, self.child.wait()).await.is_err() {
            let _ = self.child.start_kill();
            let _ = self.child.wait().await;
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

// ----------------------------------------------------------------------
// The display as the screens see it
// ----------------------------------------------------------------------

/// How a screen connection looks at the display (the part of its `Look` the display uses).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Want {
    /// The viewport in CSS pixels.
    pub width: u32,
    pub height: u32,
    /// Frame pixels per CSS pixel the client asks for (at most the display's).
    pub pixel_ratio: f64,
    pub quality: u8,
    pub max_fps: u32,
    /// When it set its viewport: the latest one gives the display its size.
    pub set: u64,
}

struct Viewing {
    id: u64,
    want: Want,
}

/// What the display is to the screens: its size in CSS pixels and how its frames are made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Painting {
    pub width: u32,
    pub height: u32,
    pub pixel_ratio: f64,
    pub quality: u8,
    pub max_fps: u32,
}

/// The running display: the X server, the connection to it, its frames.
pub struct Display {
    xvnc: Mutex<Option<Xvnc>>,
    rfb: Rfb,
    /// Device pixels per CSS pixel the browser draws at.
    pub scale: f64,
    viewers: Mutex<Vec<Viewing>>,
    stamps: AtomicU64,
    painting: watch::Sender<Painting>,
    frames: watch::Sender<Option<Arc<Frame>>>,
    /// The tab the frames are said to be of.
    pub tab: watch::Sender<String>,
    encoder: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for Display {
    fn drop(&mut self) {
        if let Some(task) = self.encoder.lock().expect("encoder").take() {
            task.abort();
        }
    }
}

impl Display {
    /// Start the X server and connect to it.
    pub async fn start(xvnc: &Path, dirs: &BrowserDirs, scale: f64) -> Result<Arc<Self>, String> {
        let (w, h) = DEFAULT_SIZE;
        let server = Xvnc::start(xvnc, dirs, (w as f64 * scale).round() as u32, (h as f64 * scale).round() as u32, (96.0 * scale).round() as u32).await?;
        let mut connected = Err(std::io::Error::other("not tried"));
        for _ in 0..20 {
            connected = Rfb::connect(&server.socket).await;
            if connected.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let rfb = match connected {
            Ok(rfb) => rfb,
            Err(e) => {
                let message = format!("the display's VNC socket {} is not reachable: {e}", server.socket.display());
                server.stop().await;
                return Err(message);
            }
        };
        rfb.set_active(false);
        let painting = Painting { width: w, height: h, pixel_ratio: 1.0, quality: 60, max_fps: 15 };
        let display = Arc::new(Self {
            xvnc: Mutex::new(Some(server)),
            rfb,
            scale,
            viewers: Mutex::new(Vec::new()),
            stamps: AtomicU64::new(0),
            painting: watch::channel(painting).0,
            frames: watch::channel(None).0,
            tab: watch::channel(String::new()).0,
            encoder: Mutex::new(None),
        });
        let task = tokio::spawn(encode(Arc::downgrade(&display), display.rfb.updates(), display.painting.subscribe()));
        *display.encoder.lock().expect("encoder") = Some(task);
        Ok(display)
    }

    /// `DISPLAY` for the browser.
    pub fn name(&self) -> String {
        self.xvnc.lock().expect("xvnc").as_ref().map(Xvnc::display).unwrap_or_default()
    }

    /// Why the display is gone (`None`: it runs).
    pub fn ended(&self) -> Option<String> {
        if self.rfb.is_closed() {
            return Some("the connection to the display closed".into());
        }
        self.xvnc.lock().expect("xvnc").as_mut().and_then(Xvnc::ended)
    }

    pub async fn stop(&self) {
        let server = self.xvnc.lock().expect("xvnc").take();
        if let Some(server) = server {
            server.stop().await;
        }
    }

    /// The display's size and frames as they are now; changes when they do.
    pub fn painting(&self) -> watch::Receiver<Painting> {
        self.painting.subscribe()
    }

    pub fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
        self.frames.subscribe()
    }

    /// When a connection set its viewport: later than every one before.
    pub fn stamp(&self) -> u64 {
        self.stamps.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// A connection looks at the display from now on (`id` is its own).
    pub fn watch(&self, id: u64, want: Want) {
        self.viewers.lock().expect("viewers").push(Viewing { id, want });
        self.settle();
    }

    /// A connection changed how it looks at the display.
    pub fn claim(&self, id: u64, want: Want) {
        if let Some(viewing) = self.viewers.lock().expect("viewers").iter_mut().find(|v| v.id == id) {
            viewing.want = want;
        }
        self.settle();
    }

    pub fn leave(&self, id: u64) {
        self.viewers.lock().expect("viewers").retain(|v| v.id != id);
        self.settle();
    }

    /// The display gets the size of the viewport set last and makes its
    /// frames for it; its frame rate is the highest a connection takes.
    /// Without connections it keeps its size and stops making frames.
    fn settle(&self) {
        let viewers = self.viewers.lock().expect("viewers");
        self.rfb.set_active(!viewers.is_empty());
        let Some(last) = viewers.iter().max_by_key(|v| v.want.set) else { return };
        let max_fps = viewers.iter().map(|v| v.want.max_fps).max().unwrap_or(15);
        let painting = Painting {
            width: last.want.width.max(MIN_WIDTH),
            height: last.want.height,
            pixel_ratio: last.want.pixel_ratio.clamp(1.0, self.scale.max(1.0)),
            quality: last.want.quality,
            max_fps,
        };
        drop(viewers);
        let resized = self.painting.send_if_modified(|was| {
            let resized = (was.width, was.height) != (painting.width, painting.height);
            let changed = *was != painting;
            *was = painting;
            changed && resized
        });
        if resized {
            let px = |css: u32| (css as f64 * self.scale).round() as u32;
            log::debug!("browser: the display is made {}x{} ({}x{} pixels)", painting.width, painting.height, px(painting.width), px(painting.height));
            self.rfb.set_size(px(painting.width), px(painting.height));
        }
    }

    // ---- input, in CSS pixels of the display ----

    fn px(&self, css: f64) -> u32 {
        (css.max(0.0) * self.scale).round() as u32
    }

    pub fn pointer(&self, x: f64, y: f64, buttons: u8) {
        self.rfb.pointer(self.px(x), self.px(y), buttons);
    }

    /// A wheel turned at `(x, y)` by this many CSS pixels; `rest` keeps the
    /// part smaller than a notch for the next turn. Returns the notches sent.
    pub fn wheel(&self, x: f64, y: f64, buttons: u8, delta: (f64, f64), rest: &mut (f64, f64)) -> i64 {
        let (px, py) = (self.px(x), self.px(y));
        let mut sent = 0;
        // Bits of the buttons: 3 up, 4 down, 5 left, 6 right.
        for ((delta, rest), (less, more)) in [((delta.0, &mut rest.0), (5u8, 6u8)), ((delta.1, &mut rest.1), (3, 4))] {
            *rest += delta;
            let notches = (*rest / WHEEL_STEP).trunc() as i64;
            *rest -= notches as f64 * WHEEL_STEP;
            let bit = if notches < 0 { less } else { more };
            for _ in 0..notches.abs().min(MAX_WHEEL_NOTCHES) {
                self.rfb.pointer(px, py, buttons | (1 << bit));
                self.rfb.pointer(px, py, buttons);
                sent += 1;
            }
        }
        sent
    }

    pub fn key(&self, keysym: u32, down: bool) {
        self.rfb.key(keysym, down);
    }

    /// Type text the way a keyboard would, one character after another.
    pub fn type_text(&self, text: &str) {
        for c in text.chars() {
            let keysym = rfb::char_keysym(c);
            self.rfb.key(keysym, true);
            self.rfb.key(keysym, false);
        }
    }

    pub fn clipboard(&self) -> watch::Receiver<Option<String>> {
        self.rfb.cut_text()
    }
}

/// Make the display's frames: when it changed or is to be painted
/// otherwise, at most as often as the fastest connection takes them.
async fn encode(display: std::sync::Weak<Display>, mut updates: watch::Receiver<u64>, mut painting: watch::Receiver<Painting>) {
    let mut next = Instant::now();
    loop {
        tokio::select! {
            changed = updates.changed() => if changed.is_err() { return },
            changed = painting.changed() => if changed.is_err() { return },
        }
        tokio::time::sleep_until(next).await;
        let paint = *painting.borrow_and_update();
        updates.borrow_and_update();
        let Some(display) = display.upgrade() else { return };
        let (fb, scale) = (display.rfb.framebuffer(), display.scale);
        let made = tokio::task::spawn_blocking(move || {
            let (rgb, width, height, css) = {
                let fb = fb.lock().expect("framebuffer");
                let (rgb, w, h) = rgb(&fb, paint.pixel_ratio / scale);
                let css = ((fb.width as f64 / scale).round() as u64, (fb.height as f64 / scale).round() as u64);
                (rgb, w, h, css)
            };
            jpeg(&rgb, width, height, paint.quality).map(|jpeg| (jpeg, css))
        })
        .await;
        match made {
            Ok(Ok((jpeg, (width, height)))) if width > 0 && height > 0 => {
                log::trace!("browser: a frame of the display, {width}x{height}, {} bytes", jpeg.len());
                let tab_id = display.tab.borrow().clone();
                display.frames.send_replace(Some(Arc::new(Frame { tab_id, jpeg: Bytes::from(jpeg), width, height })));
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => log::warn!("browser: a frame of the display was not encoded: {e}"),
            Err(e) => log::warn!("browser: encoding a frame of the display failed: {e}"),
        }
        drop(display);
        next = Instant::now() + Duration::from_millis(1000 / paint.max_fps.max(1) as u64);
    }
}

/// The framebuffer as RGB, made `factor` (at most 1) times its size: each
/// pixel is the average of the ones it covers.
pub fn rgb(fb: &rfb::Framebuffer, factor: f64) -> (Vec<u8>, u32, u32) {
    let (sw, sh) = (fb.width as usize, fb.height as usize);
    if sw == 0 || sh == 0 {
        return (Vec::new(), 0, 0);
    }
    let factor = factor.clamp(0.05, 1.0);
    let (dw, dh) = (((sw as f64 * factor).round() as usize).max(1), ((sh as f64 * factor).round() as usize).max(1));
    let mut out = Vec::with_capacity(dw * dh * 3);
    if (dw, dh) == (sw, sh) {
        for px in fb.pixels.chunks_exact(4) {
            out.extend([px[2], px[1], px[0]]);
        }
        return (out, sw as u32, sh as u32);
    }
    let span = |d: usize, dn: usize, sn: usize| {
        let a = d * sn / dn;
        (a, ((d + 1) * sn / dn).max(a + 1).min(sn))
    };
    let xs: Vec<(usize, usize)> = (0..dw).map(|x| span(x, dw, sw)).collect();
    for y in 0..dh {
        let (y0, y1) = span(y, dh, sh);
        for &(x0, x1) in &xs {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for row in y0..y1 {
                let line = &fb.pixels[(row * sw + x0) * 4..(row * sw + x1) * 4];
                for px in line.chunks_exact(4) {
                    b += px[0] as u32;
                    g += px[1] as u32;
                    r += px[2] as u32;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            out.extend([(r / n) as u8, (g / n) as u8, (b / n) as u8]);
        }
    }
    (out, dw as u32, dh as u32)
}

pub fn jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if width == 0 || height == 0 {
        return Ok(out);
    }
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(10, 95))
        .encode(rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_made_smaller_averages_what_it_covers() {
        let mut fb = rfb::Framebuffer { width: 4, height: 2, pixels: vec![0; 32] };
        // Left half white, right half blue (BGRX).
        for (i, px) in fb.pixels.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(if i % 4 < 2 { &[255, 255, 255, 0] } else { &[255, 0, 0, 0] });
        }
        let (full, w, h) = rgb(&fb, 1.0);
        assert_eq!((w, h, &full[..3], &full[6..9]), (4, 2, &[255u8, 255, 255][..], &[0u8, 0, 255][..]));
        let (half, w, h) = rgb(&fb, 0.5);
        assert_eq!((w, h), (2, 1));
        assert_eq!(half, vec![255, 255, 255, 0, 0, 255]);
        let jpeg = jpeg(&half, w, h, 60).unwrap();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    }

    #[test]
    fn the_display_mode_is_the_platforms_unless_named() {
        assert_eq!(DisplayMode::Headless.resolved(), DisplayMode::Headless);
        assert_eq!(DisplayMode::Virtual.resolved(), DisplayMode::Virtual);
        let auto = DisplayMode::Auto.resolved();
        assert_eq!(auto, if cfg!(target_os = "linux") { DisplayMode::Virtual } else { DisplayMode::Headless });
    }
}
