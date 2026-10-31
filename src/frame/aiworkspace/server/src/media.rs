//! The voice of a session (第四期规划 §7.3.1, §7.3.3; Q11, Q22): an audio-only SFU on the Workspace host. The
//! presenter sends one Opus track, every listener (audience, a display with voice) receives it; nobody else's browser
//! relays anything. Signalling is `session.rtc` (kRPC or the session channel); media is one UDP port per host address
//! (`AIWS_MEDIA_PORT`, default 4122; `off` disables it; `AIWS_MEDIA_HOSTS` lists the addresses to offer, by default the
//! host's outbound address and loopback). The service is ICE-lite: browsers find it at its host candidates, no TURN.
//! A host whose port cannot be bound says so in `session.get.voice_service` — there is no fallback (Q22).
//!
//! Recording: the Opus packets the presenter sends are written as they arrive into Ogg Opus segments of 10 s of
//! demonstration time; each segment becomes an asset of the run Workspace and a `voice.segment` event. Silences the
//! sender left out (DTX) are written as 20 ms silence frames, so a segment's duration is its time on the line; lost
//! packets are not concealed, only counted (`lost_ms`).

use crate::{blocking, AppState};
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::session::registry::SessionRow;
use aiworkspace_store::session::{Actor, NewEvent, Role};
use aiworkspace_store::Caller;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use str0m::change::SdpOffer;
use str0m::media::{MediaData, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::{mpsc, oneshot};

/// Length of one recorded segment, in demonstration time (`AIWS_VOICE_SEGMENT_MS` shortens it for tests).
fn segment_ms() -> i64 {
    static MS: OnceLock<i64> = OnceLock::new();
    *MS.get_or_init(|| std::env::var("AIWS_VOICE_SEGMENT_MS").ok().and_then(|v| v.parse().ok()).filter(|v| *v >= 200).unwrap_or(10_000))
}
/// A 20 ms Opus silence frame (CELT, mono): what the recording writes for a DTX pause.
const SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];

enum Cmd {
    Offer { key: PeerKey, publish: bool, started_ms: Option<i64>, sdp: String, reply: oneshot::Sender<Result<String, String>> },
    Close { session: String, principal: String, conn: Option<u64> },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PeerKey {
    session: String,
    principal: String,
    conn: u64,
}

struct Peer {
    key: PeerKey,
    rtc: Rtc,
    publish: bool,
    /// The listener's receiving m-line.
    mid: Option<Mid>,
}

/// A recording in progress (one per session while its presenter sends).
struct Recorder {
    started_ms: i64,
    seg: Option<Segment>,
    index: u64,
}

struct Segment {
    ogg: OggWriter,
    t_start: i64,
    /// RTP time (48 kHz) at the segment's start and of the next expected packet.
    rtp0: u64,
    next_rtp: u64,
    lost_ms: i64,
    last_seq: Option<u64>,
}

pub struct Media {
    tx: OnceLock<mpsc::UnboundedSender<Cmd>>,
    status: Mutex<Value>,
}

impl Default for Media {
    fn default() -> Self {
        Media { tx: OnceLock::new(), status: Mutex::new(json!({ "available": false, "reason": "the voice service has not started" })) }
    }
}

impl Media {
    /// `{ available, reason?, port?, hosts? }`: what a room tells the presenter before it starts (P4-18).
    pub fn status(&self) -> Value {
        self.status.lock().unwrap().clone()
    }

    /// `session.rtc { op: offer, sdp }` → `{ sdp }` (the answer); `{ op: close }`. The presenter publishes, everyone
    /// else listens (a display only with its link's `config.voice`).
    pub async fn signal(&self, state: &Arc<AppState>, s: &SessionRow, actor: &Actor, p: &Value) -> WsResult<Value> {
        let conn = p.get("_conn").and_then(Value::as_u64).unwrap_or(0);
        self.signal_on(state, s, actor, p, conn).await
    }

    pub async fn signal_on(&self, state: &Arc<AppState>, s: &SessionRow, actor: &Actor, p: &Value, conn: u64) -> WsResult<Value> {
        let unavailable = || WsError::new(Code::DependencyUnavailable, format!("voice is not available: {}", self.status()["reason"].as_str().unwrap_or("check the host's media port")));
        let tx = self.tx.get().ok_or_else(unavailable)?;
        if self.status()["available"] != json!(true) {
            return Err(unavailable());
        }
        if s.policy["voice"] == json!(false) {
            return Err(WsError::sub(Code::InvalidOperation, "VOICE_OFF", "this room has no voice"));
        }
        let key = PeerKey { session: s.session_id.clone(), principal: actor.principal.clone(), conn };
        match p.get("op").and_then(Value::as_str).unwrap_or("offer") {
            "close" => {
                let _ = tx.send(Cmd::Close { session: key.session, principal: key.principal, conn: Some(conn) });
                Ok(json!({ "ok": true }))
            }
            "offer" => {
                let sdp = p.get("sdp").and_then(Value::as_str).filter(|s| s.len() < 64 * 1024).ok_or_else(|| WsError::invalid_op("sdp required"))?.to_string();
                let publish = actor.role == Role::Presenter || (actor.role == Role::Prompter && p["publish"] == json!(true));
                if !publish && actor.role == Role::Display {
                    let voice = state.svc.sessions.invites(&s.session_id)?.into_iter().any(|i| format!("link:{}", i.invite_id) == actor.principal && i.config["voice"] == json!(true));
                    if !voice {
                        return Err(WsError::sub(Code::InvalidOperation, "VOICE_OFF", "this display does not play the voice"));
                    }
                }
                let (reply, rx) = oneshot::channel();
                tx.send(Cmd::Offer { key, publish, started_ms: s.started_ms, sdp, reply }).map_err(|_| unavailable())?;
                let answer = rx.await.map_err(|_| unavailable())?.map_err(|e| WsError::invalid_op(format!("the offer was not accepted: {e}")))?;
                Ok(json!({ "ok": true, "sdp": answer, "role": if publish { "publish" } else { "listen" } }))
            }
            _ => Err(WsError::invalid_op("op must be offer or close")),
        }
    }

    /// The connection `conn` of `principal` closed: its peer goes.
    pub fn disconnect(&self, session_id: &str, principal: &str, conn: u64) {
        if let Some(tx) = self.tx.get() {
            let _ = tx.send(Cmd::Close { session: session_id.into(), principal: principal.into(), conn: Some(conn) });
        }
    }
}

/// The addresses to offer: `AIWS_MEDIA_HOSTS`, else the outbound address (a UDP "connect" sends nothing) and loopback.
fn host_addresses() -> Vec<IpAddr> {
    if let Ok(list) = std::env::var("AIWS_MEDIA_HOSTS") {
        return list.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    }
    let mut out = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                if !addr.ip().is_unspecified() && !addr.ip().is_loopback() {
                    out.push(addr.ip());
                }
            }
        }
    }
    out.push(IpAddr::from([127, 0, 0, 1]));
    out
}

/// Bind the media sockets and run the SFU (called once by `serve`).
pub fn start(state: &Arc<AppState>) {
    let port = match std::env::var("AIWS_MEDIA_PORT").ok().as_deref() {
        Some("off") => {
            *state.media.status.lock().unwrap() = json!({ "available": false, "reason": "voice is switched off on this host (AIWS_MEDIA_PORT=off)" });
            return;
        }
        Some(p) => p.parse::<u16>().unwrap_or(4122),
        None => 4122,
    };
    let _ = str0m::crypto::from_feature_flags().install_process_default();
    let mut sockets = Vec::new();
    let mut bound_port = port;
    for ip in host_addresses() {
        match std::net::UdpSocket::bind(SocketAddr::new(ip, bound_port)) {
            Ok(sock) => {
                if bound_port == 0 {
                    bound_port = sock.local_addr().map(|a| a.port()).unwrap_or(0);
                }
                let _ = sock.set_nonblocking(true);
                match tokio::net::UdpSocket::from_std(sock) {
                    Ok(s) => sockets.push(Arc::new(s)),
                    Err(e) => log::warn!("media socket on {ip}: {e}"),
                }
            }
            Err(e) => log::warn!("media port {ip}:{bound_port} could not be bound: {e}"),
        }
    }
    if sockets.is_empty() {
        *state.media.status.lock().unwrap() = json!({ "available": false, "reason": format!("UDP port {port} could not be bound on this host: open or free the media port (AIWS_MEDIA_PORT)") });
        return;
    }
    let hosts: Vec<String> = sockets.iter().filter_map(|s| s.local_addr().ok()).map(|a| a.to_string()).collect();
    log::info!("session voice on udp {}", hosts.join(", "));
    *state.media.status.lock().unwrap() = json!({ "available": true, "port": bound_port, "hosts": hosts, "transport": "udp" });
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = state.media.tx.set(tx);
    let (ptx, prx) = mpsc::unbounded_channel::<(Vec<u8>, SocketAddr, SocketAddr)>();
    for sock in &sockets {
        let (sock, ptx) = (sock.clone(), ptx.clone());
        tokio::spawn(async move {
            let local = match sock.local_addr() {
                Ok(a) => a,
                Err(_) => return,
            };
            let mut buf = vec![0u8; 2000];
            loop {
                match sock.recv_from(&mut buf).await {
                    Ok((n, from)) => {
                        if ptx.send((buf[..n].to_vec(), from, local)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        log::debug!("media socket {local}: {e}");
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });
    }
    tokio::spawn(run(Arc::downgrade(state), sockets, rx, prx));
}

async fn run(state: Weak<AppState>, sockets: Vec<Arc<tokio::net::UdpSocket>>, mut cmds: mpsc::UnboundedReceiver<Cmd>, mut packets: mpsc::UnboundedReceiver<(Vec<u8>, SocketAddr, SocketAddr)>) {
    let mut peers: Vec<Peer> = Vec::new();
    let mut recorders: HashMap<String, Recorder> = HashMap::new();
    loop {
        // drive every peer until it waits; forward and record what the presenters sent
        let mut next = Instant::now() + Duration::from_millis(100);
        let mut media: Vec<(String, MediaData)> = Vec::new();
        for peer in peers.iter_mut() {
            loop {
                if !peer.rtc.is_alive() {
                    break;
                }
                match peer.rtc.poll_output() {
                    Ok(Output::Transmit(t)) => {
                        if let Some(sock) = sockets.iter().find(|s| s.local_addr().ok() == Some(t.source)).or(sockets.first()) {
                            let _ = sock.try_send_to(&t.contents, t.destination);
                        }
                    }
                    Ok(Output::Timeout(t)) => {
                        next = next.min(t);
                        break;
                    }
                    Ok(Output::Event(e)) => match e {
                        Event::IceConnectionStateChange(IceConnectionState::Disconnected) => peer.rtc.disconnect(),
                        Event::MediaAdded(m) => {
                            if !peer.publish {
                                peer.mid = Some(m.mid);
                            }
                        }
                        Event::MediaData(d) if peer.publish => media.push((peer.key.session.clone(), d)),
                        _ => {}
                    },
                    Err(e) => {
                        log::debug!("rtc {:?}: {e}", peer.key);
                        peer.rtc.disconnect();
                        break;
                    }
                }
            }
        }
        for (session, data) in media {
            for peer in peers.iter_mut().filter(|p| !p.publish && p.key.session == session) {
                let Some(mid) = peer.mid else { continue };
                let Some(writer) = peer.rtc.writer(mid) else { continue };
                let Some(pt) = writer.match_params(data.params) else { continue };
                if writer.write(pt, data.network_time, data.time, data.data.clone()).is_err() {
                    peer.rtc.disconnect();
                }
            }
            if let (Some(rec), Some(st)) = (recorders.get_mut(&session), state.upgrade()) {
                let now_ms = (st.svc.clock)();
                if let Some(done) = rec.packet(&data, now_ms) {
                    finish(&st, &session, done);
                }
            }
        }
        // presenters that went away close their recording
        let gone: Vec<PeerKey> = peers.iter().filter(|p| !p.rtc.is_alive() && p.publish).map(|p| p.key.clone()).collect();
        peers.retain(|p| p.rtc.is_alive());
        for key in gone {
            if !peers.iter().any(|p| p.publish && p.key.session == key.session) {
                if let (Some(mut rec), Some(st)) = (recorders.remove(&key.session), state.upgrade()) {
                    if let Some(done) = rec.close() {
                        finish(&st, &key.session, done);
                    }
                    event(&st, &key.session, &key.principal, "voice.stop", json!({}));
                }
            }
        }
        let wait = next.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                match cmd {
                    Cmd::Offer { key, publish, started_ms, sdp, reply } => {
                        // one peer per connection: a new offer replaces the old one
                        for p in peers.iter_mut().filter(|p| p.key == key) {
                            p.rtc.disconnect();
                        }
                        let result = (|| -> Result<(Rtc, String), String> {
                            let offer = SdpOffer::from_sdp_string(&sdp).map_err(|e| e.to_string())?;
                            let mut rtc = Rtc::builder().set_ice_lite(true).build(Instant::now());
                            for sock in &sockets {
                                if let Ok(addr) = sock.local_addr() {
                                    if let Ok(c) = Candidate::host(addr, "udp") {
                                        rtc.add_local_candidate(c);
                                    }
                                }
                            }
                            let answer = rtc.sdp_api().accept_offer(offer).map_err(|e| e.to_string())?;
                            Ok((rtc, answer.to_sdp_string()))
                        })();
                        match result {
                            Ok((rtc, answer)) => {
                                if publish {
                                    let st = state.upgrade();
                                    if let (Some(st), Some(t0)) = (st, started_ms) {
                                        recorders.entry(key.session.clone()).or_insert(Recorder { started_ms: t0, seg: None, index: 0 });
                                        event(&st, &key.session, &key.principal, "voice.start", json!({}));
                                    }
                                }
                                peers.push(Peer { key, rtc, publish, mid: None });
                                let _ = reply.send(Ok(answer));
                            }
                            Err(e) => {
                                let _ = reply.send(Err(e));
                            }
                        }
                    }
                    Cmd::Close { session, principal, conn } => {
                        for p in peers.iter_mut().filter(|p| p.key.session == session && p.key.principal == principal && conn.is_none_or(|c| p.key.conn == c)) {
                            p.rtc.disconnect();
                        }
                    }
                }
            }
            packet = packets.recv() => {
                let Some((data, source, destination)) = packet else { return };
                let Ok(contents) = data.as_slice().try_into() else { continue };
                let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination, contents });
                if let Some(peer) = peers.iter_mut().find(|p| p.rtc.accepts(&input)) {
                    if peer.rtc.handle_input(input).is_err() {
                        peer.rtc.disconnect();
                    }
                }
            }
            _ = tokio::time::sleep(wait) => {}
        }
        let now = Instant::now();
        for peer in peers.iter_mut() {
            if peer.rtc.is_alive() && peer.rtc.handle_input(Input::Timeout(now)).is_err() {
                peer.rtc.disconnect();
            }
        }
    }
}

fn event(state: &Arc<AppState>, session: &str, actor: &str, kind: &'static str, data: Value) {
    let (st, sid, who) = (state.clone(), session.to_string(), actor.to_string());
    tokio::spawn(async move {
        let _ = blocking(move || {
            let s = st.svc.sessions.require(&sid)?;
            st.svc.with_run(&s, |ws| ws.append_event(NewEvent { actor: &who, scope: "public", track: "voice", kind, command_id: None, data, t_rel_ms: None }).map(|_| ()))
        })
        .await;
    });
}

/// A finished segment goes into the run Workspace (off the media loop).
fn finish(state: &Arc<AppState>, session: &str, done: Finished) {
    let (st, sid) = (state.clone(), session.to_string());
    let label = sid.clone();
    tokio::spawn(async move {
        let r = blocking(move || {
            let s = st.svc.sessions.require(&sid)?;
            let owner = Caller::user(&s.owner);
            st.svc.with_run(&s, |ws| ws.session_voice_segment(&owner, &s.presenter, &done.bytes, done.index, done.t_start, done.t_end, done.lost_ms).map(|_| ()))
        })
        .await;
        if let Err(e) = r {
            // §7.3.3: a recording that cannot be stored is said at once
            log::warn!("voice segment of {label}: {e}");
        }
    });
}

struct Finished {
    bytes: Vec<u8>,
    index: u64,
    t_start: i64,
    t_end: i64,
    lost_ms: i64,
}

impl Recorder {
    /// Write one packet; a segment that reached its length comes back to be stored.
    fn packet(&mut self, d: &MediaData, now_ms: i64) -> Option<Finished> {
        let t_rel = (now_ms - self.started_ms).max(0);
        let rtp = rtp48(d);
        let mut done = None;
        if self.seg.as_ref().is_some_and(|s| t_rel - s.t_start >= segment_ms()) {
            done = self.close();
        }
        let seg = self.seg.get_or_insert_with(|| Segment { ogg: OggWriter::new((now_ms as u32) ^ ((self.index as u32) << 16)), t_start: t_rel, rtp0: rtp, next_rtp: rtp, lost_ms: 0, last_seq: None });
        // packets lost on the way: counted, not concealed
        let seq = *d.seq_range.end().clone();
        if let Some(last) = seg.last_seq {
            let first = *d.seq_range.start().clone();
            if first > last + 1 {
                seg.lost_ms += ((first - last - 1) * 20) as i64;
            }
        }
        seg.last_seq = Some(seq);
        // a pause the sender left out (DTX): silence frames keep the segment on the time line
        if rtp > seg.next_rtp {
            let gap = (rtp - seg.next_rtp) / 960;
            for _ in 0..gap.min(500) {
                seg.next_rtp += 960;
                seg.ogg.packet(&SILENCE, seg.next_rtp - seg.rtp0);
            }
        }
        let samples = opus_samples(&d.data).max(1);
        seg.next_rtp = rtp.max(seg.next_rtp) + samples;
        seg.ogg.packet(&d.data, seg.next_rtp - seg.rtp0);
        done
    }

    fn close(&mut self) -> Option<Finished> {
        let seg = self.seg.take()?;
        let duration = ((seg.next_rtp - seg.rtp0) / 48) as i64;
        self.index += 1;
        Some(Finished { bytes: seg.ogg.finish(), index: self.index, t_start: seg.t_start, t_end: seg.t_start + duration, lost_ms: seg.lost_ms })
    }
}

/// The RTP time of a packet on Opus's 48 kHz clock.
fn rtp48(d: &MediaData) -> u64 {
    let (n, f) = (d.time.numer(), d.time.denom() as u64);
    if f == 48_000 || f == 0 { n } else { n * 48_000 / f }
}

/// Samples (48 kHz) in one Opus packet (RFC 6716 §3.1: the TOC byte's configuration and frame count).
pub fn opus_samples(packet: &[u8]) -> u64 {
    let Some(toc) = packet.first() else { return 0 };
    let config = toc >> 3;
    let frame_tenth_ms: u64 = match config {
        0..=11 => [100, 200, 400, 600][(config % 4) as usize],
        12..=15 => [100, 200][(config % 2) as usize],
        _ => [25, 50, 100, 200][(config % 4) as usize],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => packet.get(1).map(|b| (b & 0x3f) as u64).unwrap_or(1),
    };
    frames * frame_tenth_ms * 48 / 10
}

/// A minimal Ogg Opus writer (RFC 7845): the two header pages, then one page per packet.
pub struct OggWriter {
    out: Vec<u8>,
    serial: u32,
    seq: u32,
}

impl OggWriter {
    pub fn new(serial: u32) -> OggWriter {
        let mut w = OggWriter { out: Vec::new(), serial, seq: 0 };
        let mut head = b"OpusHead".to_vec();
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&0u16.to_le_bytes()); // pre-skip
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // gain
        head.push(0); // mapping family
        w.page(&head, 0, 0x02);
        let vendor = b"buckyos aiworkspace";
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        w.page(&tags, 0, 0);
        w
    }

    pub fn packet(&mut self, data: &[u8], granule: u64) {
        self.page(data, granule, 0);
    }

    pub fn finish(mut self) -> Vec<u8> {
        // the last page says the stream ended
        let granule = self.last_granule();
        self.page(&[], granule, 0x04);
        self.out
    }

    fn last_granule(&self) -> u64 {
        // the granule of the last page written: bytes 6..14 of its header (pages are appended in order)
        let mut pos = 0;
        let mut last = 0;
        while pos + 27 <= self.out.len() {
            last = u64::from_le_bytes(self.out[pos + 6..pos + 14].try_into().unwrap());
            let segs = self.out[pos + 26] as usize;
            let body: usize = self.out[pos + 27..pos + 27 + segs].iter().map(|b| *b as usize).sum();
            pos += 27 + segs + body;
        }
        last
    }

    fn page(&mut self, data: &[u8], granule: u64, flags: u8) {
        let mut lacing = Vec::new();
        let mut left = data.len();
        loop {
            if left >= 255 {
                lacing.push(255u8);
                left -= 255;
            } else {
                lacing.push(left as u8);
                break;
            }
        }
        let start = self.out.len();
        self.out.extend_from_slice(b"OggS");
        self.out.push(0);
        self.out.push(flags);
        self.out.extend_from_slice(&granule.to_le_bytes());
        self.out.extend_from_slice(&self.serial.to_le_bytes());
        self.out.extend_from_slice(&self.seq.to_le_bytes());
        self.out.extend_from_slice(&0u32.to_le_bytes());
        self.out.push(lacing.len() as u8);
        self.out.extend_from_slice(&lacing);
        self.out.extend_from_slice(data);
        let crc = ogg_crc(&self.out[start..]);
        self.out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
        self.seq += 1;
    }
}

/// CRC-32 of Ogg (polynomial 0x04c11db7, no reflection, initial and final value 0).
fn ogg_crc(data: &[u8]) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04c1_1db7 } else { r << 1 };
            }
            *e = r;
        }
        t
    });
    data.iter().fold(0u32, |crc, b| (crc << 8) ^ table[(((crc >> 24) as u8) ^ b) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ogg_pages_carry_a_valid_crc_and_the_opus_headers() {
        let mut w = OggWriter::new(7);
        w.packet(&SILENCE, 960);
        w.packet(&[0x78, 1, 2, 3], 1920);
        let bytes = w.finish();
        assert_eq!(&bytes[..4], b"OggS");
        assert_eq!(bytes[5], 0x02, "the first page begins the stream");
        assert!(bytes.windows(8).any(|x| x == b"OpusHead"));
        assert!(bytes.windows(8).any(|x| x == b"OpusTags"));
        // every page's CRC checks out
        let mut pos = 0;
        let mut pages = 0;
        while pos < bytes.len() {
            let segs = bytes[pos + 26] as usize;
            let body: usize = bytes[pos + 27..pos + 27 + segs].iter().map(|b| *b as usize).sum();
            let end = pos + 27 + segs + body;
            let mut page = bytes[pos..end].to_vec();
            let crc = u32::from_le_bytes(page[22..26].try_into().unwrap());
            page[22..26].copy_from_slice(&[0, 0, 0, 0]);
            assert_eq!(ogg_crc(&page), crc);
            pos = end;
            pages += 1;
        }
        assert_eq!(pages, 5);
    }

    #[test]
    fn opus_frame_lengths_follow_the_toc() {
        assert_eq!(opus_samples(&SILENCE), 960); // CELT 20 ms, one frame
        assert_eq!(opus_samples(&[0x78]), 960); // config 15: hybrid 20 ms
        assert_eq!(opus_samples(&[0x09]), 1920); // config 1 (SILK 20 ms), two frames
        assert_eq!(opus_samples(&[0x03, 0x03]), 1440); // SILK 10 ms × 3
    }
}
