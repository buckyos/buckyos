//! The voice of a session (第四期规划 §7.3.1, §7.3.3; 多人发言 TODO §4): an audio mixer on the Workspace host. Whoever
//! may speak sends one Opus track to the host; the host mixes the voices (`mixer.rs`) and every connection — those who
//! speak, those who listen, a display with voice — receives one mixed track back; nobody's browser relays anything.
//! Signalling is `session.rtc` (kRPC or the session channel); media is one UDP port per host address (`AIWS_MEDIA_PORT`,
//! default 4122; `off` disables it; `AIWS_MEDIA_HOSTS` lists the addresses to offer, by default the host's outbound
//! address and loopback). The service is ICE-lite: browsers find it at its host candidates, no TURN. A host whose port
//! cannot be bound says so in `session.get.voice_service` — there is no fallback (Q22).
//!
//! Who speaks is the host's decision (§4.2): a connection sends into the mix only while its person may speak, one
//! sending connection per person (the newest takes over), and a manager's "mute all" holds everyone but the presenter
//! until they pick a speaking mode again. A browser changes direction on the same connection (a new offer with the
//! same ICE credentials renegotiates it).
//!
//! Recording: the total mix — what everyone heard — is written into Ogg Opus segments of 10 s of demonstration time;
//! each segment becomes an asset of the run Workspace and a `voice.segment` event. `voice.start` is put on the time
//! line when the first sending connection is up in a started session, `voice.stop` when the last one goes; ticks
//! nobody spoke in are written as 20 ms silence frames, so a segment's duration is its time on the line.

use crate::mixer::{self, MixIn, MixOut, MixerHandle};
use crate::{blocking, AppState};
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::session::registry::SessionRow;
use aiworkspace_store::session::{Actor, NewEvent, Role};
use aiworkspace_store::Caller;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use str0m::change::SdpOffer;
use str0m::format::{Codec, FormatParams};
use str0m::media::{Frequency, MediaData, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::{mpsc, oneshot};

/// Length of one recorded segment, in demonstration time (`AIWS_VOICE_SEGMENT_MS` shortens it for tests).
fn segment_ms() -> i64 {
    static MS: OnceLock<i64> = OnceLock::new();
    *MS.get_or_init(|| std::env::var("AIWS_VOICE_SEGMENT_MS").ok().and_then(|v| v.parse().ok()).filter(|v| *v >= 200).unwrap_or(10_000))
}

/// At most this many voices are mixed at once (Q2: 4, the presenter's among them; `AIWS_VOICE_MAX_SPEAKERS` for tests).
pub fn max_speakers() -> usize {
    static K: OnceLock<usize> = OnceLock::new();
    *K.get_or_init(|| std::env::var("AIWS_VOICE_MAX_SPEAKERS").ok().and_then(|v| v.parse().ok()).filter(|k| (1..=16).contains(k)).unwrap_or(4))
}

/// A 20 ms Opus silence frame (CELT, mono): what the recording writes for a tick nobody spoke in.
const SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];
/// Each sending connection's uplink goes on the diag track this often while it sends (S10), and when it ends.
const UPLINK_REPORT: Duration = Duration::from_secs(60);

enum Cmd {
    Offer { key: PeerKey, send: bool, presenter: bool, name: String, started_ms: Option<i64>, show_names: bool, sdp: String, reply: oneshot::Sender<Result<(String, bool), String>> },
    /// Connections go: one, a person's, or (no principal) the whole session's.
    Close { session: String, principal: Option<String>, conn: Option<u64> },
    /// Who may speak now: other senders stop being mixed.
    Speakers { session: String, speakers: BTreeSet<String> },
    MuteAll { session: String },
    Mode { session: String, principal: String, mode: String },
    Started { session: String, started_ms: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PeerKey {
    session: String,
    principal: String,
    conn: u64,
}

struct Peer {
    id: u64,
    key: PeerKey,
    rtc: Rtc,
    /// The browser's ICE ufrag and DTLS fingerprint: an offer with the same ones renegotiates this connection.
    remote: (String, String),
    /// Its packets go into the mix: the person may speak and this is their sending connection.
    send: bool,
    presenter: bool,
    name: Arc<str>,
    /// The audio m-line the mix goes out on.
    mid: Option<Mid>,
    connected: bool,
    /// Joined to the session's mixer.
    in_mix: bool,
    up: Uplink,
}

/// What a connection sent (S10): enough for a manager to tell a terminal that never stops sending.
#[derive(Default)]
struct Uplink {
    bytes: u64,
    packets: u64,
    run_start: Option<Instant>,
    last: Option<Instant>,
    active: Duration,
    longest: Duration,
    since_report: u64,
    reported: Option<Instant>,
}

impl Uplink {
    fn packet(&mut self, len: usize, now: Instant) {
        self.bytes += len as u64;
        self.since_report += len as u64;
        self.packets += 1;
        match self.last {
            Some(l) if now.saturating_duration_since(l) < Duration::from_secs(1) => self.active += now.saturating_duration_since(l),
            _ => self.run_start = Some(now),
        }
        self.last = Some(now);
        if let Some(s) = self.run_start {
            self.longest = self.longest.max(now.saturating_duration_since(s));
        }
    }

    fn json(&self, principal: &str, conn: u64) -> Value {
        json!({ "principal": principal, "conn": conn, "bytes": self.bytes, "packets": self.packets, "active_ms": self.active.as_millis() as u64,
                "longest_ms": self.longest.as_millis() as u64 })
    }
}

/// The media side of one session.
struct Room {
    started_ms: Option<i64>,
    show_names: bool,
    mixer: Option<MixerHandle>,
    recorder: Option<Recorder>,
    /// `voice.start` is on the time line.
    on: bool,
    /// Muted by a manager (Q7) until they pick a speaking mode again.
    held: BTreeSet<String>,
    /// The last one who sent (the actor of `voice.stop`).
    last_sender: String,
}

/// A recording in progress (one per session while someone sends).
struct Recorder {
    started_ms: i64,
    seg: Option<Segment>,
    index: u64,
}

struct Segment {
    ogg: OggWriter,
    t_start: i64,
    /// RTP time (48 kHz) at the segment's start and of the next frame.
    rtp0: u64,
    next_rtp: u64,
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
    /// `{ available, reason?, port?, hosts?, mixing? }`: what a room tells the presenter before it starts (P4-18).
    pub fn status(&self) -> Value {
        self.status.lock().unwrap().clone()
    }

    /// `session.rtc { op: offer, sdp, send? }` → `{ sdp, send }` (the answer, and whether this connection sends into the
    /// mix); `{ op: close }`. `send` is a wish: the host grants it to whoever may speak (§4.2). Everyone receives the
    /// mix (a display only with its link's `config.voice`).
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
                let _ = tx.send(Cmd::Close { session: key.session, principal: Some(key.principal), conn: Some(conn) });
                Ok(json!({ "ok": true }))
            }
            "offer" => {
                let sdp = p.get("sdp").and_then(Value::as_str).filter(|s| s.len() < 64 * 1024).ok_or_else(|| WsError::invalid_op("sdp required"))?.to_string();
                if actor.role == Role::Display {
                    let voice = state.svc.sessions.invites(&s.session_id)?.into_iter().any(|i| format!("link:{}", i.invite_id) == actor.principal && i.config["voice"] == json!(true));
                    if !voice {
                        return Err(WsError::sub(Code::InvalidOperation, "VOICE_OFF", "this display does not play the voice"));
                    }
                }
                let send = p["send"] == json!(true) && state.svc.actor_speaks(s, actor)?;
                let (reply, rx) = oneshot::channel();
                tx.send(Cmd::Offer { key, send, presenter: actor.principal == s.presenter, name: actor.name.clone(), started_ms: s.started_ms,
                                     show_names: s.policy["show_names"] != json!(false), sdp, reply })
                    .map_err(|_| unavailable())?;
                let (answer, send) = rx.await.map_err(|_| unavailable())?.map_err(|e| WsError::invalid_op(format!("the offer was not accepted: {e}")))?;
                Ok(json!({ "ok": true, "sdp": answer, "send": send }))
            }
            _ => Err(WsError::invalid_op("op must be offer or close")),
        }
    }

    fn cmd(&self, cmd: Cmd) {
        if let Some(tx) = self.tx.get() {
            let _ = tx.send(cmd);
        }
    }

    /// The connection `conn` of `principal` closed: its peer goes.
    pub fn disconnect(&self, session_id: &str, principal: &str, conn: u64) {
        self.cmd(Cmd::Close { session: session_id.into(), principal: Some(principal.into()), conn: Some(conn) });
    }

    /// Who may speak changed (a manager's decision, the room's setting, a revoked link): whoever may no longer speak
    /// leaves the mix at once (§4.2); their client hears of it from `voice.allow`.
    pub fn recheck(&self, state: &AppState, s: &SessionRow) {
        if self.tx.get().is_none() {
            return;
        }
        match state.svc.speakers(s) {
            Ok(speakers) => self.cmd(Cmd::Speakers { session: s.session_id.clone(), speakers }),
            Err(e) => log::warn!("session {}: who speaks: {e}", s.session_id),
        }
    }

    /// A manager muted the room (Q7): everyone but the presenter is held until they pick a speaking mode again.
    pub fn mute_all(&self, session_id: &str) {
        self.cmd(Cmd::MuteAll { session: session_id.into() });
    }

    /// Someone picked a speaking mode (`voice.mode`).
    pub fn mode(&self, session_id: &str, principal: &str, mode: &str) {
        self.cmd(Cmd::Mode { session: session_id.into(), principal: principal.into(), mode: mode.into() });
    }

    /// The session started: voice from now on is recorded.
    pub fn started(&self, session_id: &str, started_ms: i64) {
        self.cmd(Cmd::Started { session: session_id.into(), started_ms });
    }

    /// The session ended: its voice ends with it.
    pub fn ended(&self, session_id: &str) {
        self.cmd(Cmd::Close { session: session_id.into(), principal: None, conn: None });
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

/// Bind the media sockets and run the voice service (called once by `serve`).
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
    *state.media.status.lock().unwrap() = json!({ "available": true, "port": bound_port, "hosts": hosts, "transport": "udp", "mixing": { "max_speakers": max_speakers() } });
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

/// The browser's side of an offer: its ICE ufrag and DTLS fingerprint.
fn remote_of(sdp: &str) -> (String, String) {
    let line = |prefix: &str| sdp.lines().find_map(|l| l.trim().strip_prefix(prefix)).unwrap_or("").trim().to_string();
    (line("a=ice-ufrag:"), line("a=fingerprint:"))
}

/// A connection of the host: ICE-lite at the host candidates; Opus answered with `usedtx=1`, so browsers send almost
/// nothing while their gate is closed (S9).
fn new_rtc(sockets: &[Arc<tokio::net::UdpSocket>]) -> Rtc {
    let mut config = Rtc::builder().set_ice_lite(true);
    config.codec_config().enable_opus(false, false);
    config.codec_config().add_config(111.into(), None, Codec::Opus, Frequency::FORTY_EIGHT_KHZ, Some(2),
                                     FormatParams { min_p_time: Some(10), use_inband_fec: Some(true), use_dtx: Some(true), ..Default::default() });
    let mut rtc = config.build(Instant::now());
    for sock in sockets {
        if let Ok(addr) = sock.local_addr() {
            if let Ok(c) = Candidate::host(addr, "udp") {
                rtc.add_local_candidate(c);
            }
        }
    }
    rtc
}

struct Loop {
    state: Weak<AppState>,
    sockets: Vec<Arc<tokio::net::UdpSocket>>,
    peers: Vec<Peer>,
    rooms: HashMap<String, Room>,
    mix_tx: mpsc::UnboundedSender<MixOut>,
    next_id: u64,
}

async fn run(state: Weak<AppState>, sockets: Vec<Arc<tokio::net::UdpSocket>>, mut cmds: mpsc::UnboundedReceiver<Cmd>, mut packets: mpsc::UnboundedReceiver<(Vec<u8>, SocketAddr, SocketAddr)>) {
    let (mix_tx, mut mix_rx) = mpsc::unbounded_channel();
    let mut lp = Loop { state, sockets, peers: Vec::new(), rooms: HashMap::new(), mix_tx, next_id: 1 };
    loop {
        let next = lp.drive();
        lp.reconcile();
        let wait = next.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                lp.command(cmd);
            }
            packet = packets.recv() => {
                let Some((data, source, destination)) = packet else { return };
                let Ok(contents) = data.as_slice().try_into() else { continue };
                let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination, contents });
                if let Some(peer) = lp.peers.iter_mut().find(|p| p.rtc.accepts(&input)) {
                    if peer.rtc.handle_input(input).is_err() {
                        peer.rtc.disconnect();
                    }
                }
            }
            out = mix_rx.recv() => {
                if let Some(out) = out {
                    lp.mixed(out);
                }
            }
            _ = tokio::time::sleep(wait) => {}
        }
        let now = Instant::now();
        for peer in lp.peers.iter_mut() {
            if peer.rtc.is_alive() && peer.rtc.handle_input(Input::Timeout(now)).is_err() {
                peer.rtc.disconnect();
            }
        }
    }
}

impl Loop {
    /// Drive every peer until it waits: send what it has to send, hand what the speakers sent to their mixer.
    fn drive(&mut self) -> Instant {
        let mut next = Instant::now() + Duration::from_millis(100);
        let now = Instant::now();
        for peer in self.peers.iter_mut() {
            loop {
                if !peer.rtc.is_alive() {
                    break;
                }
                match peer.rtc.poll_output() {
                    Ok(Output::Transmit(t)) => {
                        if let Some(sock) = self.sockets.iter().find(|s| s.local_addr().ok() == Some(t.source)).or(self.sockets.first()) {
                            let _ = sock.try_send_to(&t.contents, t.destination);
                        }
                    }
                    Ok(Output::Timeout(t)) => {
                        next = next.min(t);
                        break;
                    }
                    Ok(Output::Event(e)) => match e {
                        Event::IceConnectionStateChange(IceConnectionState::Disconnected) => peer.rtc.disconnect(),
                        Event::Connected => peer.connected = true,
                        Event::MediaAdded(m) => peer.mid = Some(m.mid),
                        Event::MediaData(d) => {
                            peer.up.packet(d.data.len(), now);
                            if peer.in_mix {
                                if let Some(m) = self.rooms.get(&peer.key.session).and_then(|r| r.mixer.as_ref()) {
                                    let _ = m.tx.send(MixIn::Packet { id: peer.id, rtp: rtp48(&d), level: d.ext_vals.audio_level, data: d.data.to_vec(), at: now });
                                }
                            }
                        }
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
        next
    }

    /// Keep the rooms in step with their peers: a mixer while someone sends, each sender in it while it may send,
    /// `voice.start` / `voice.stop` and the recording, uplink reports, rooms nobody is in any more.
    fn reconcile(&mut self) {
        let now = Instant::now();
        let st = self.state.upgrade();
        // peers that went away
        for p in self.peers.iter().filter(|p| !p.rtc.is_alive()) {
            if p.in_mix {
                if let Some(m) = self.rooms.get(&p.key.session).and_then(|r| r.mixer.as_ref()) {
                    let _ = m.tx.send(MixIn::Leave { id: p.id });
                }
            }
            if p.up.bytes > 0 {
                if let Some(st) = &st {
                    event(st, &p.key.session, &p.key.principal, "diag", "voice.uplink", p.up.json(&p.key.principal, p.key.conn));
                }
            }
        }
        self.peers.retain(|p| p.rtc.is_alive());
        // uplink reports of long sends
        for p in self.peers.iter_mut().filter(|p| p.up.since_report > 0 && p.up.reported.is_none_or(|r| now.saturating_duration_since(r) >= UPLINK_REPORT)) {
            if p.up.reported.is_none() {
                p.up.reported = Some(now);
                continue;
            }
            p.up.reported = Some(now);
            p.up.since_report = 0;
            if let Some(st) = &st {
                event(st, &p.key.session, &p.key.principal, "diag", "voice.uplink", p.up.json(&p.key.principal, p.key.conn));
            }
        }
        let sessions: Vec<String> = self.rooms.keys().cloned().collect();
        for sid in sessions {
            let room = self.rooms.get_mut(&sid).unwrap();
            let sending = self.peers.iter().any(|p| p.key.session == sid && p.send && p.connected);
            if sending && room.mixer.is_none() {
                room.mixer = Some(mixer::spawn(sid.as_str().into(), max_speakers(), self.mix_tx.clone()));
            }
            if let Some(m) = room.mixer.as_ref() {
                for p in self.peers.iter_mut().filter(|p| p.key.session == sid) {
                    let want = p.send && p.connected && !room.held.contains(&p.key.principal);
                    if want && !p.in_mix {
                        let _ = m.tx.send(MixIn::Join { id: p.id, principal: p.key.principal.as_str().into(), name: p.name.clone(), presenter: p.presenter });
                        room.last_sender = p.key.principal.clone();
                    } else if !want && p.in_mix {
                        let _ = m.tx.send(MixIn::Leave { id: p.id });
                    }
                    p.in_mix = want;
                }
            }
            if sending && !room.on {
                if let (Some(t0), Some(st)) = (room.started_ms, &st) {
                    room.on = true;
                    room.recorder = Some(Recorder { started_ms: t0, seg: None, index: 0 });
                    event(st, &sid, &room.last_sender, "public", "voice.start", json!({}));
                }
            }
            if !sending && room.mixer.is_some() {
                // the last sender went: its Leave is already queued, the thread drains it and ends
                room.mixer = None;
                if let (Some(mut rec), Some(st)) = (room.recorder.take(), &st) {
                    if let Some(done) = rec.close() {
                        finish(st, &sid, done);
                    }
                }
                if room.on {
                    room.on = false;
                    if let Some(st) = &st {
                        event(st, &sid, &room.last_sender, "public", "voice.stop", json!({}));
                    }
                }
            }
            if room.mixer.is_none() && !self.peers.iter().any(|p| p.key.session == sid) {
                self.rooms.remove(&sid);
            }
        }
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Offer { key, send, presenter, name, started_ms, show_names, sdp, reply } => {
                let room = self.rooms.entry(key.session.clone()).or_insert_with(|| Room { started_ms, show_names, mixer: None, recorder: None, on: false, held: BTreeSet::new(),
                                                                                         last_sender: key.principal.clone() });
                room.started_ms = room.started_ms.or(started_ms);
                // one sending connection per person: the newest takes over (§4.2)
                if send {
                    for p in self.peers.iter_mut().filter(|p| p.key.session == key.session && p.key.principal == key.principal && p.key.conn != key.conn) {
                        p.send = false;
                    }
                }
                let remote = remote_of(&sdp);
                let same = self.peers.iter().position(|p| p.key == key && p.rtc.is_alive() && !remote.0.is_empty() && p.remote == remote);
                let result = match same {
                    // the same browser connection changes direction: it is renegotiated where it is
                    Some(i) => {
                        let p = &mut self.peers[i];
                        let answer = SdpOffer::from_sdp_string(&sdp).map_err(|e| e.to_string()).and_then(|o| p.rtc.sdp_api().accept_offer(o).map_err(|e| e.to_string()));
                        if answer.is_ok() {
                            p.send = send;
                        }
                        answer.map(|a| a.to_sdp_string())
                    }
                    None => {
                        for p in self.peers.iter_mut().filter(|p| p.key == key) {
                            p.rtc.disconnect();
                        }
                        let mut rtc = new_rtc(&self.sockets);
                        let answer = SdpOffer::from_sdp_string(&sdp).map_err(|e| e.to_string()).and_then(|o| rtc.sdp_api().accept_offer(o).map_err(|e| e.to_string()));
                        if answer.is_ok() {
                            let id = self.next_id;
                            self.next_id += 1;
                            self.peers.push(Peer { id, key, rtc, remote, send, presenter, name: name.as_str().into(), mid: None, connected: false, in_mix: false, up: Uplink::default() });
                        }
                        answer.map(|a| a.to_sdp_string())
                    }
                };
                let _ = reply.send(result.map(|a| (a, send)));
            }
            Cmd::Close { session, principal, conn } => {
                for p in self.peers.iter_mut().filter(|p| p.key.session == session && principal.as_ref().is_none_or(|w| p.key.principal == *w) && conn.is_none_or(|c| p.key.conn == c)) {
                    p.rtc.disconnect();
                }
            }
            Cmd::Speakers { session, speakers } => {
                for p in self.peers.iter_mut().filter(|p| p.key.session == session && p.send && !speakers.contains(&p.key.principal)) {
                    p.send = false;
                }
            }
            Cmd::MuteAll { session } => {
                if let Some(room) = self.rooms.get_mut(&session) {
                    for p in self.peers.iter().filter(|p| p.key.session == session && !p.presenter) {
                        room.held.insert(p.key.principal.clone());
                    }
                }
            }
            Cmd::Mode { session, principal, mode } => {
                if mode != "mute" {
                    if let Some(room) = self.rooms.get_mut(&session) {
                        room.held.remove(&principal);
                    }
                }
            }
            Cmd::Started { session, started_ms } => {
                if let Some(room) = self.rooms.get_mut(&session) {
                    room.started_ms = Some(started_ms);
                }
            }
        }
    }

    /// What the mixer made: the tick's frames out to every connection of the session and into the recording; who
    /// started or stopped speaking; what became of a sender's packets.
    fn mixed(&mut self, out: MixOut) {
        let st = self.state.upgrade();
        match out {
            MixOut::Frame { session, rtp, all, own } => {
                let time = MediaTime::new(rtp, Frequency::FORTY_EIGHT_KHZ);
                let now = Instant::now();
                for p in self.peers.iter_mut().filter(|p| p.key.session == *session && p.connected) {
                    let Some(data) = own.iter().find(|(id, _)| *id == p.id).map(|(_, d)| d).or(all.as_ref()) else { continue };
                    let Some(mid) = p.mid else { continue };
                    if !p.rtc.media(mid).is_some_and(|m| m.direction().is_sending()) {
                        continue;
                    }
                    let Some(writer) = p.rtc.writer(mid) else { continue };
                    let Some(pt) = writer.payload_params().find(|x| x.spec().codec == Codec::Opus).map(|x| x.pt()) else { continue };
                    if writer.write(pt, now, time, data.clone()).is_err() {
                        p.rtc.disconnect();
                    }
                }
                if let (Some(rec), Some(st)) = (self.rooms.get_mut(&*session).and_then(|r| r.recorder.as_mut()), &st) {
                    if let Some(done) = rec.frame(rtp, all.as_deref(), (st.svc.clock)()) {
                        finish(st, &session, done);
                    }
                }
            }
            MixOut::Speaking { session, principal, name, on } => {
                if let Some(st) = &st {
                    let mut data = json!({ "principal": &*principal, "on": on });
                    if self.rooms.get(&*session).is_none_or(|r| r.show_names) {
                        data["name"] = json!(&*name);
                    }
                    event(st, &session, &principal, "public", "voice.speaking", data);
                }
            }
            MixOut::Counts { session, principal, decoded, skipped, late, concealed, levels } => {
                if let Some(st) = &st {
                    event(st, &session, &principal, "diag", "voice.mix", json!({ "principal": &*principal, "decoded": decoded, "skipped": skipped, "late": late, "concealed": concealed, "with_level": levels }));
                }
            }
        }
    }
}

/// An event of the voice track, then the room's fan-out (media events come from no call that would pump it).
fn event(state: &Arc<AppState>, session: &str, actor: &str, scope: &'static str, kind: &'static str, data: Value) {
    let (st, sid, who) = (state.clone(), session.to_string(), actor.to_string());
    tokio::spawn(async move {
        let (st2, sid2) = (st.clone(), sid.clone());
        let r = blocking(move || {
            let s = st2.svc.sessions.require(&sid2)?;
            st2.svc.with_run(&s, |ws| ws.append_event(NewEvent { actor: &who, scope, track: "voice", kind, command_id: None, data, t_rel_ms: None }).map(|_| ()))
        })
        .await;
        if r.is_ok() {
            crate::session::nudge(&st, &sid).await;
        }
    });
}

/// A finished segment goes into the run Workspace (off the media loop).
fn finish(state: &Arc<AppState>, session: &str, done: Finished) {
    let (st, sid) = (state.clone(), session.to_string());
    let label = sid.clone();
    tokio::spawn(async move {
        let st2 = st.clone();
        let sid2 = sid.clone();
        let r = blocking(move || {
            let s = st2.svc.sessions.require(&sid2)?;
            let owner = Caller::internal(&s.owner);
            st2.svc.with_run(&s, |ws| ws.session_voice_segment(&owner, &s.presenter, &done.bytes, done.index, done.t_start, done.t_end, 0).map(|_| ()))
        })
        .await;
        match r {
            Ok(()) => crate::session::nudge(&st, &sid).await,
            // §7.3.3: a recording that cannot be stored is said at once
            Err(e) => log::warn!("voice segment of {label}: {e}"),
        }
    });
}

struct Finished {
    bytes: Vec<u8>,
    index: u64,
    t_start: i64,
    t_end: i64,
}

impl Recorder {
    /// Write one tick of the mix (`None`: nobody spoke, a silence frame); a segment that reached its length comes back
    /// to be stored.
    fn frame(&mut self, rtp: u64, data: Option<&[u8]>, now_ms: i64) -> Option<Finished> {
        let t_rel = (now_ms - self.started_ms).max(0);
        let mut done = None;
        if self.seg.as_ref().is_some_and(|s| t_rel - s.t_start >= segment_ms()) {
            done = self.close();
        }
        let seg = self.seg.get_or_insert_with(|| Segment { ogg: OggWriter::new((now_ms as u32) ^ ((self.index as u32) << 16)), t_start: t_rel, rtp0: rtp, next_rtp: rtp });
        // ticks the mixer skipped (a stalled host): silence keeps the segment on the time line
        if rtp > seg.next_rtp {
            let gap = (rtp - seg.next_rtp) / 960;
            for _ in 0..gap.min(500) {
                seg.next_rtp += 960;
                seg.ogg.packet(&SILENCE, seg.next_rtp - seg.rtp0);
            }
        }
        let packet = data.unwrap_or(&SILENCE);
        seg.next_rtp = rtp.max(seg.next_rtp) + opus_samples(packet).max(1);
        seg.ogg.packet(packet, seg.next_rtp - seg.rtp0);
        done
    }

    fn close(&mut self) -> Option<Finished> {
        let seg = self.seg.take()?;
        let duration = ((seg.next_rtp - seg.rtp0) / 48) as i64;
        self.index += 1;
        Some(Finished { bytes: seg.ogg.finish(), index: self.index, t_start: seg.t_start, t_end: seg.t_start + duration })
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
