//! A VNC client (RFB 3.8, RFC 6143) for the browser's virtual display: the
//! service reads what the display shows and operates it the way a person
//! at it would, with the pointer and the keyboard. It speaks to the display
//! over a socket only it can open, with no security type, and asks for the
//! encodings a local link needs: raw pixels, copied rectangles and size
//! changes.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

const VERSION: &[u8; 12] = b"RFB 003.008\n";
const SECURITY_NONE: u8 = 1;
const ENCODING_RAW: i32 = 0;
const ENCODING_COPY_RECT: i32 = 1;
const ENCODING_LAST_RECT: i32 = -224;
const ENCODING_DESKTOP_SIZE: i32 = -223;
const ENCODING_EXTENDED_DESKTOP_SIZE: i32 = -308;
/// Pixels of a rectangle the client takes at most (a display of 8K by 8K).
const MAX_PIXELS: usize = 8192 * 8192;
const MAX_CUT_TEXT: usize = 1 << 20;

/// What the display shows: 4 bytes a pixel, blue, green, red and one unused.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Framebuffer {
    fn sized(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; width as usize * height as usize * 4] }
    }

    fn put(&mut self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        let (fw, fh) = (self.width, self.height);
        for row in 0..h.min(fh.saturating_sub(y)) {
            let width = w.min(fw.saturating_sub(x)) as usize * 4;
            let from = row as usize * w as usize * 4;
            let to = ((y + row) as usize * fw as usize + x as usize) * 4;
            self.pixels[to..to + width].copy_from_slice(&data[from..from + width]);
        }
    }

    fn copy(&mut self, sx: u32, sy: u32, x: u32, y: u32, w: u32, h: u32) {
        let (fw, fh) = (self.width, self.height);
        if sx >= fw || sy >= fh || x >= fw || y >= fh {
            return;
        }
        let w = w.min(fw - sx).min(fw - x) as usize;
        let h = h.min(fh - sy).min(fh - y);
        let rows: Box<dyn Iterator<Item = u32>> = if sy < y { Box::new((0..h).rev()) } else { Box::new(0..h) };
        for row in rows {
            let from = ((sy + row) as usize * fw as usize + sx as usize) * 4;
            let to = ((y + row) as usize * fw as usize + x as usize) * 4;
            self.pixels.copy_within(from..from + w * 4, to);
        }
    }
}

/// An open connection to the display. Dropping it closes the connection.
pub struct Rfb {
    out: mpsc::UnboundedSender<Vec<u8>>,
    active: watch::Sender<bool>,
    framebuffer: Arc<Mutex<Framebuffer>>,
    updates: watch::Receiver<u64>,
    cut_text: watch::Receiver<Option<String>>,
    reader: tokio::task::JoinHandle<()>,
    writer: tokio::task::JoinHandle<()>,
}

impl Drop for Rfb {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
    }
}

impl Rfb {
    /// Open the display's socket and complete the handshake.
    #[cfg(unix)]
    pub async fn connect(socket: &std::path::Path) -> std::io::Result<Self> {
        let stream = tokio::net::UnixStream::connect(socket).await?;
        Self::over(stream).await
    }

    #[cfg(not(unix))]
    pub async fn connect(socket: &std::path::Path) -> std::io::Result<Self> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, format!("the display's socket {} needs a Unix socket", socket.display())))
    }

    /// The handshake over `stream`, then the connection.
    pub async fn over<S: AsyncRead + AsyncWrite + Send + 'static>(stream: S) -> std::io::Result<Self> {
        let (mut read, mut write) = tokio::io::split(stream);
        let mut version = [0u8; 12];
        read.read_exact(&mut version).await?;
        if !version.starts_with(b"RFB 003.") {
            return Err(invalid(format!("not a VNC server: {:?}", String::from_utf8_lossy(&version))));
        }
        write.write_all(VERSION).await?;
        let count = read.read_u8().await?;
        if count == 0 {
            let reason = read_string(&mut read).await?;
            return Err(invalid(format!("the display refused the connection: {reason}")));
        }
        let mut types = vec![0u8; count as usize];
        read.read_exact(&mut types).await?;
        if !types.contains(&SECURITY_NONE) {
            return Err(invalid(format!("the display asks for security types {types:?}, not none")));
        }
        write.write_u8(SECURITY_NONE).await?;
        if read.read_u32().await? != 0 {
            let reason = read_string(&mut read).await.unwrap_or_default();
            return Err(invalid(format!("the display did not accept the connection: {reason}")));
        }
        // Shared: other viewers of the display (a person with a VNC viewer while debugging) stay.
        write.write_u8(1).await?;
        let width = read.read_u16().await? as u32;
        let height = read.read_u16().await? as u32;
        let mut format = [0u8; 16];
        read.read_exact(&mut format).await?;
        let _name = read_string(&mut read).await?;

        let mut hello = Vec::new();
        hello.extend(set_pixel_format());
        hello.extend(set_encodings(&[ENCODING_RAW, ENCODING_COPY_RECT, ENCODING_LAST_RECT, ENCODING_DESKTOP_SIZE, ENCODING_EXTENDED_DESKTOP_SIZE]));
        hello.extend(update_request(false, width, height));
        write.write_all(&hello).await?;

        let framebuffer = Arc::new(Mutex::new(Framebuffer::sized(width, height)));
        let (updated, updates) = watch::channel(0u64);
        let (cut, cut_text) = watch::channel(None);
        let (active, wanted) = watch::channel(true);
        let (out, mut outgoing) = mpsc::unbounded_channel::<Vec<u8>>();
        let writer = tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if write.write_all(&message).await.is_err() {
                    break;
                }
            }
        });
        let (fb, again) = (framebuffer.clone(), out.clone());
        let reader = tokio::spawn(async move {
            if let Err(e) = read_messages(&mut read, &fb, &updated, &cut, &again, wanted).await {
                log::debug!("browser: the display connection ended: {e}");
            }
            // Watchers learn the connection is gone from the closed channels.
            drop(updated);
        });
        Ok(Self { out, active, framebuffer, updates, cut_text, reader, writer })
    }

    /// Whether the display's changes are wanted: while not, no update is
    /// asked for (the display keeps what changed and sends it when asked again).
    pub fn set_active(&self, active: bool) {
        self.active.send_if_modified(|was| std::mem::replace(was, active) != active);
    }

    /// The framebuffer as it is now (shared with the reader: hold it briefly).
    pub fn framebuffer(&self) -> Arc<Mutex<Framebuffer>> {
        self.framebuffer.clone()
    }

    /// Changes after every complete update of the framebuffer; closed when the connection ends.
    pub fn updates(&self) -> watch::Receiver<u64> {
        self.updates.clone()
    }

    /// The text the display last put on its clipboard.
    pub fn cut_text(&self) -> watch::Receiver<Option<String>> {
        self.cut_text.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.reader.is_finished() || self.out.is_closed()
    }

    fn send(&self, message: Vec<u8>) {
        let _ = self.out.send(message);
    }

    /// The pointer is at `(x, y)` (display pixels) with these buttons down
    /// (bit 0 left, 1 middle, 2 right, 3–6 the wheel's four directions).
    pub fn pointer(&self, x: u32, y: u32, buttons: u8) {
        let mut m = vec![5, buttons];
        m.extend((x.min(u16::MAX as u32) as u16).to_be_bytes());
        m.extend((y.min(u16::MAX as u32) as u16).to_be_bytes());
        self.send(m);
    }

    /// A key, by its X keysym, goes down or up.
    pub fn key(&self, keysym: u32, down: bool) {
        let mut m = vec![4, down as u8, 0, 0];
        m.extend(keysym.to_be_bytes());
        self.send(m);
    }

    /// Ask the display to be this size (the display answers with the size it took).
    pub fn set_size(&self, width: u32, height: u32) {
        let (w, h) = (width.clamp(1, u16::MAX as u32) as u16, height.clamp(1, u16::MAX as u32) as u16);
        let mut m = vec![251, 0];
        m.extend(w.to_be_bytes());
        m.extend(h.to_be_bytes());
        m.extend([1, 0]);
        m.extend(0u32.to_be_bytes());
        m.extend([0u8; 4]);
        m.extend(w.to_be_bytes());
        m.extend(h.to_be_bytes());
        m.extend(0u32.to_be_bytes());
        self.send(m);
    }

    /// Put text on the display's clipboard (Latin-1: what the base protocol carries).
    pub fn put_cut_text(&self, text: &str) {
        let bytes: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
        let mut m = vec![6, 0, 0, 0];
        m.extend((bytes.len() as u32).to_be_bytes());
        m.extend(bytes);
        self.send(m);
    }
}

fn invalid(message: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

async fn read_string<R: AsyncRead + Unpin>(read: &mut R) -> std::io::Result<String> {
    let len = read.read_u32().await? as usize;
    if len > MAX_CUT_TEXT {
        return Err(invalid(format!("a string of {len} bytes")));
    }
    let mut bytes = vec![0u8; len];
    read.read_exact(&mut bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 32 bits a pixel, little-endian, 8 bits a colour: blue, green, red, unused in memory.
fn set_pixel_format() -> Vec<u8> {
    let mut m = vec![0, 0, 0, 0];
    m.extend([32, 24, 0, 1]);
    m.extend(255u16.to_be_bytes());
    m.extend(255u16.to_be_bytes());
    m.extend(255u16.to_be_bytes());
    m.extend([16, 8, 0, 0, 0, 0]);
    m
}

fn set_encodings(encodings: &[i32]) -> Vec<u8> {
    let mut m = vec![2, 0];
    m.extend((encodings.len() as u16).to_be_bytes());
    for encoding in encodings {
        m.extend(encoding.to_be_bytes());
    }
    m
}

fn update_request(incremental: bool, width: u32, height: u32) -> Vec<u8> {
    let mut m = vec![3, incremental as u8, 0, 0, 0, 0];
    m.extend((width.min(u16::MAX as u32) as u16).to_be_bytes());
    m.extend((height.min(u16::MAX as u32) as u16).to_be_bytes());
    m
}

async fn read_messages<R: AsyncRead + Unpin>(
    read: &mut R,
    framebuffer: &Mutex<Framebuffer>,
    updated: &watch::Sender<u64>,
    cut: &watch::Sender<Option<String>>,
    out: &mpsc::UnboundedSender<Vec<u8>>,
    mut active: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let mut rect = Vec::new();
    // The update request that waits for the changes to be wanted again.
    let mut owed: Option<Vec<u8>> = None;
    loop {
        let kind = tokio::select! {
            kind = read.read_u8() => kind?,
            changed = active.changed(), if owed.is_some() => {
                if changed.is_err() {
                    return Ok(());
                }
                if *active.borrow_and_update() {
                    let _ = out.send(owed.take().expect("owed"));
                }
                continue;
            }
        };
        match kind {
            0 => {
                read.read_u8().await?;
                let count = read.read_u16().await?;
                let mut resized = false;
                for _ in 0..count {
                    let (x, y, w, h) = (read.read_u16().await? as u32, read.read_u16().await? as u32, read.read_u16().await? as u32, read.read_u16().await? as u32);
                    let encoding = read.read_i32().await?;
                    match encoding {
                        ENCODING_RAW => {
                            let len = w as usize * h as usize;
                            if len > MAX_PIXELS {
                                return Err(invalid(format!("a rectangle of {w}x{h}")));
                            }
                            rect.resize(len * 4, 0);
                            read.read_exact(&mut rect).await?;
                            framebuffer.lock().expect("framebuffer").put(x, y, w, h, &rect);
                        }
                        ENCODING_COPY_RECT => {
                            let (sx, sy) = (read.read_u16().await? as u32, read.read_u16().await? as u32);
                            framebuffer.lock().expect("framebuffer").copy(sx, sy, x, y, w, h);
                        }
                        ENCODING_LAST_RECT => break,
                        ENCODING_DESKTOP_SIZE => {
                            *framebuffer.lock().expect("framebuffer") = Framebuffer::sized(w, h);
                            resized = true;
                        }
                        ENCODING_EXTENDED_DESKTOP_SIZE => {
                            let screens = read.read_u8().await? as usize;
                            let mut skip = vec![0u8; 3 + screens * 16];
                            read.read_exact(&mut skip).await?;
                            // `y` is the status: not 0, a size the client asked for was refused.
                            if y == 0 && (w, h) != { let fb = framebuffer.lock().expect("framebuffer"); (fb.width, fb.height) } {
                                *framebuffer.lock().expect("framebuffer") = Framebuffer::sized(w, h);
                                resized = true;
                            }
                        }
                        other => return Err(invalid(format!("an update in encoding {other}, which was not asked for"))),
                    }
                }
                let (width, height) = {
                    let fb = framebuffer.lock().expect("framebuffer");
                    (fb.width, fb.height)
                };
                updated.send_modify(|n| *n += 1);
                // After a size change the framebuffer's pixels are all to be sent again.
                let request = match owed.take() {
                    Some(earlier) if earlier[1] == 0 => update_request(false, width, height),
                    _ => update_request(!resized, width, height),
                };
                if *active.borrow_and_update() {
                    let _ = out.send(request);
                } else {
                    owed = Some(request);
                }
            }
            1 => {
                let mut head = [0u8; 5];
                read.read_exact(&mut head).await?;
                let colours = u16::from_be_bytes([head[3], head[4]]) as usize;
                let mut skip = vec![0u8; colours * 6];
                read.read_exact(&mut skip).await?;
            }
            2 => {}
            3 => {
                let mut pad = [0u8; 3];
                read.read_exact(&mut pad).await?;
                let len = read.read_u32().await? as usize;
                if len > MAX_CUT_TEXT {
                    return Err(invalid(format!("clipboard text of {len} bytes")));
                }
                let mut bytes = vec![0u8; len];
                read.read_exact(&mut bytes).await?;
                cut.send_replace(Some(bytes.iter().map(|b| *b as char).collect()));
            }
            other => return Err(invalid(format!("unknown server message {other}"))),
        }
    }
}

/// The X keysym of a character: Latin-1 is itself, the rest is the Unicode
/// range (Xvnc gives such a keysym a key of its own when one is typed).
pub fn char_keysym(c: char) -> u32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        c if (0x20..0x7f).contains(&(c as u32)) || (0xa0..0x100).contains(&(c as u32)) => c as u32,
        c => 0x0100_0000 | c as u32,
    }
}

/// The X keysym of a DOM `KeyboardEvent.key` that names a key rather than a
/// character (`None`: it names a character or nothing known).
pub fn named_keysym(key: &str, code: &str) -> Option<u32> {
    let right = code.ends_with("Right");
    Some(match key {
        "Enter" => if code == "NumpadEnter" { 0xff8d } else { 0xff0d },
        "Backspace" => 0xff08,
        "Tab" => 0xff09,
        "Escape" => 0xff1b,
        "Delete" => 0xffff,
        "Insert" => 0xff63,
        "Home" => 0xff50,
        "End" => 0xff57,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "Shift" => if right { 0xffe2 } else { 0xffe1 },
        "Control" => if right { 0xffe4 } else { 0xffe3 },
        "Alt" => if right { 0xffea } else { 0xffe9 },
        "AltGraph" => 0xfe03,
        "Meta" | "OS" => if right { 0xffec } else { 0xffeb },
        "CapsLock" => 0xffe5,
        "NumLock" => 0xff7f,
        "ScrollLock" => 0xff14,
        "Pause" => 0xff13,
        "PrintScreen" => 0xff61,
        "ContextMenu" => 0xff67,
        "Clear" => 0xff0b,
        k if k.len() >= 2 && k.starts_with('F') => {
            let n: u32 = k[1..].parse().ok().filter(|n| (1..=24).contains(n))?;
            0xffbe + n - 1
        }
        _ => return None,
    })
}

/// The keysyms of the modifier bits of the protocol (Alt 1, Ctrl 2, Meta 4, Shift 8).
pub const MODIFIER_KEYSYMS: [(i64, u32); 4] = [(1, 0xffe9), (2, 0xffe3), (4, 0xffeb), (8, 0xffe1)];

#[cfg(test)]
mod tests {
    use super::*;

    /// A display of `w`x`h` speaking the server side over `stream`: the
    /// handshake, then what `serve` writes. Returns what the client sent after the handshake.
    async fn display<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, w: u16, h: u16, serve: Vec<u8>) -> Vec<u8> {
        stream.write_all(VERSION).await.unwrap();
        let mut version = [0u8; 12];
        stream.read_exact(&mut version).await.unwrap();
        assert_eq!(&version, VERSION);
        stream.write_all(&[2, 2, SECURITY_NONE]).await.unwrap();
        assert_eq!(stream.read_u8().await.unwrap(), SECURITY_NONE);
        stream.write_u32(0).await.unwrap();
        assert_eq!(stream.read_u8().await.unwrap(), 1, "shared");
        stream.write_u16(w).await.unwrap();
        stream.write_u16(h).await.unwrap();
        stream.write_all(&[0u8; 16]).await.unwrap();
        stream.write_u32(4).await.unwrap();
        stream.write_all(b"test").await.unwrap();
        stream.write_all(&serve).await.unwrap();
        // Everything the client sends until it is quiet for a while.
        let mut got = Vec::new();
        let mut chunk = vec![0u8; 4096];
        while let Ok(Ok(n)) = tokio::time::timeout(std::time::Duration::from_millis(300), stream.read(&mut chunk)).await {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&chunk[..n]);
        }
        got
    }

    fn rect(x: u16, y: u16, w: u16, h: u16, encoding: i32) -> Vec<u8> {
        let mut m = Vec::new();
        for n in [x, y, w, h] {
            m.extend(n.to_be_bytes());
        }
        m.extend(encoding.to_be_bytes());
        m
    }

    #[tokio::test]
    async fn the_client_keeps_the_framebuffer_the_display_sends() {
        let (client, server) = tokio::io::duplex(1 << 16);
        let mut serve = vec![0, 0, 0, 3];
        // A red pixel at (1, 0), then the row copied one down, then a size change to 3x3.
        serve.extend(rect(1, 0, 1, 1, ENCODING_RAW));
        serve.extend([0, 0, 255, 0]);
        serve.extend(rect(0, 1, 2, 1, ENCODING_COPY_RECT));
        serve.extend([0, 0, 0, 0]);
        serve.extend(rect(0, 0, 0, 0, ENCODING_LAST_RECT));
        serve.extend([3, 0, 0, 0, 0, 0, 0, 2]);
        serve.extend(b"hi");
        let server = tokio::spawn(display(server, 2, 2, serve));
        let rfb = Rfb::over(client).await.unwrap();
        let mut updates = rfb.updates();
        updates.changed().await.unwrap();
        {
            let fb = rfb.framebuffer();
            let fb = fb.lock().unwrap();
            assert_eq!((fb.width, fb.height), (2, 2));
            assert_eq!(&fb.pixels[4..8], &[0, 0, 255, 0]);
            assert_eq!(&fb.pixels[12..16], &[0, 0, 255, 0], "copied a row down");
        }
        let mut cut = rfb.cut_text();
        cut.wait_for(|t| t.is_some()).await.unwrap();
        assert_eq!(cut.borrow().as_deref(), Some("hi"));
        rfb.pointer(5, 6, 1);
        rfb.key(0xff0d, true);
        let sent = server.await.unwrap();
        // SetPixelFormat (20) + SetEncodings (4 + 5*4) + the first full update request (10), then an incremental request.
        assert_eq!(sent[0], 0);
        assert_eq!(&sent[4..8], &[32, 24, 0, 1]);
        assert_eq!(sent[20], 2);
        assert_eq!(&sent[44..54], &[3, 0, 0, 0, 0, 0, 0, 2, 0, 2]);
        assert_eq!(&sent[54..64], &[3, 1, 0, 0, 0, 0, 0, 2, 0, 2]);
        assert_eq!(&sent[64..70], &[5, 1, 0, 5, 0, 6]);
        assert_eq!(&sent[70..78], &[4, 1, 0, 0, 0, 0, 0xff, 0x0d]);
    }

    #[tokio::test]
    async fn a_size_change_asks_for_the_whole_display_again() {
        let (client, server) = tokio::io::duplex(1 << 16);
        let mut serve = vec![0, 0, 0, 1];
        serve.extend(rect(0, 0, 4, 3, ENCODING_EXTENDED_DESKTOP_SIZE));
        serve.extend([1, 0, 0, 0]);
        serve.extend([0u8; 16]);
        let server = tokio::spawn(display(server, 2, 2, serve));
        let rfb = Rfb::over(client).await.unwrap();
        rfb.updates().changed().await.unwrap();
        assert_eq!({ let fb = rfb.framebuffer(); let fb = fb.lock().unwrap(); (fb.width, fb.height, fb.pixels.len()) }, (4, 3, 48));
        rfb.set_size(800, 600);
        let sent = server.await.unwrap();
        assert_eq!(&sent[54..64], &[3, 0, 0, 0, 0, 0, 0, 4, 0, 3], "not incremental after the size changed");
        assert_eq!(&sent[64..70], &[251, 0, 3, 32, 2, 88]);
    }

    #[tokio::test]
    async fn a_display_that_wants_a_password_is_refused() {
        let (client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            server.write_all(VERSION).await.unwrap();
            let mut v = [0u8; 12];
            server.read_exact(&mut v).await.unwrap();
            server.write_all(&[1, 2]).await.unwrap();
            let mut rest = [0u8; 8];
            let _ = server.read(&mut rest).await;
        });
        let refused = Rfb::over(client).await.err().unwrap();
        assert!(refused.to_string().contains("security types [2]"), "{refused}");
    }

    #[test]
    fn keys_and_characters_have_their_keysyms() {
        assert_eq!(char_keysym('a'), 0x61);
        assert_eq!(char_keysym('é'), 0xe9);
        assert_eq!(char_keysym('中'), 0x0100_4e2d);
        assert_eq!(char_keysym('\n'), 0xff0d);
        assert_eq!(named_keysym("Enter", "Enter"), Some(0xff0d));
        assert_eq!(named_keysym("Enter", "NumpadEnter"), Some(0xff8d));
        assert_eq!(named_keysym("Shift", "ShiftRight"), Some(0xffe2));
        assert_eq!(named_keysym("F1", "F1"), Some(0xffbe));
        assert_eq!(named_keysym("F12", "F12"), Some(0xffc9));
        assert_eq!(named_keysym("F25", ""), None);
        assert_eq!(named_keysym("a", "KeyA"), None);
    }

    #[test]
    fn a_copy_within_the_framebuffer_overlaps_safely() {
        let mut fb = Framebuffer::sized(1, 3);
        fb.pixels.copy_from_slice(&[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
        fb.copy(0, 0, 0, 1, 1, 2);
        assert_eq!(fb.pixels, vec![1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]);
    }
}
