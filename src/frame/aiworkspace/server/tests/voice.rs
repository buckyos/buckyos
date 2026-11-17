//! The voice of a session against the real process (第四期规划 §7.3.1, §7.3.3, P4-15/P4-18; 多人发言 TODO T1): WebRTC
//! peers — str0m standing in for browsers, sending Opus tones encoded with libopus — negotiate through `session.rtc`;
//! the host mixes whoever may speak, every listener hears the mix, every voice in it hears the mix without itself, and
//! the host records the mix as Ogg Opus segments on the session's time line.

use serde_json::{json, Value};
use std::net::UdpSocket;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use str0m::change::SdpAnswer;
use str0m::format::Codec;
use str0m::media::{Direction, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

const TOKENS: [(&str, &str); 4] = [("tok-alice", "alice"), ("tok-bob", "bob"), ("tok-carol", "carol"), ("tok-dave", "dave")];

const FIXTURE: &str = include_str!("../../fixtures/presentation/commits.json");

struct Server {
    child: Child,
    base: String,
    http: reqwest::Client,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    async fn start(dir: &std::path::Path, media: &str) -> Server {
        Server::start_with(dir, media, &[]).await
    }

    async fn start_with(dir: &std::path::Path, media: &str, env: &[(&str, &str)]) -> Server {
        let auth = dir.join("tokens.json");
        let tokens: serde_json::Map<String, Value> = TOKENS.iter().map(|(t, p)| (t.to_string(), json!({ "principal": p }))).collect();
        std::fs::write(&auth, json!({ "tokens": tokens }).to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--log-level").arg("warn").env("AIWS_MEDIA_PORT", media).env("AIWS_MEDIA_HOSTS", "127.0.0.1").env("AIWS_VOICE_SEGMENT_MS", "1000")
            .envs(env.iter().copied())
            .stdout(Stdio::null()).stderr(Stdio::null())
            .spawn()
            .expect("spawn aiworkspace");
        let server = Server { child, base: format!("http://127.0.0.1:{port}/kapi/aiworkspace"), http: reqwest::Client::new() };
        for _ in 0..200 {
            if server.http.get(format!("{}/healthz", server.base)).send().await.is_ok_and(|r| r.status().is_success()) {
                return server;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("server did not start");
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [1, token] })).send().await.unwrap().json().await.unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    async fn asset(&self, token: &str, ws: &str, object_id: &str) -> Vec<u8> {
        let r = self.http.get(format!("{}/asset/{ws}/{object_id}", self.base)).bearer_auth(token).send().await.unwrap();
        assert!(r.status().is_success(), "{}", r.status());
        r.bytes().await.unwrap().to_vec()
    }

    async fn session(&self) -> (String, String) {
        let ws = self.rpc("tok-alice", "ws.create", json!({ "title": "语音课" })).await;
        let (id, epoch) = (ws["workspace_id"].as_str().unwrap().to_string(), ws["epoch"].as_str().unwrap().to_string());
        let fx: Value = serde_json::from_str(FIXTURE).unwrap();
        for (i, c) in fx["commits"].as_array().unwrap().iter().enumerate() {
            let r = self.rpc("tok-alice", "doc.commit", json!({ "protocol_version": "1.0", "workspace_id": id, "epoch": epoch, "idempotency_key": format!("fx/{i}"), "operations": c["operations"] })).await;
            assert_eq!(r["status"], "accepted");
        }
        let s = self.rpc("tok-alice", "session.create", json!({ "workspace_id": id, "path_id": "path-intro" })).await;
        let sid = s["session"]["session_id"].as_str().unwrap().to_string();
        let at = self.rpc("tok-alice", "session.invite", json!({ "session_id": sid, "kind": "at" })).await["invite"]["token"].as_str().unwrap().to_string();
        for (token, _) in &TOKENS[1..] {
            assert_eq!(self.rpc(token, "session.join", json!({ "session_id": sid, "link": at })).await["ok"], true);
        }
        (sid, id)
    }
}

/// A browser-like peer: full ICE on a loopback socket, one audio m-line; a speaker sends a tone (libopus, 20 ms frames,
/// with its audio level), everyone keeps what the host sends back.
struct Peer {
    rtc: Rtc,
    sock: UdpSocket,
    mid: Mid,
    connected: bool,
    tone: Option<(f32, i8)>,
    enc: opus::Encoder,
    frames: u64,
    heard: Vec<(u64, Vec<u8>)>,
}

impl Peer {
    fn new(dir: Direction, tone: Option<(f32, i8)>) -> (Peer, String, str0m::change::SdpPendingOffer) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
        let mut rtc = Rtc::builder().build(Instant::now());
        rtc.add_local_candidate(Candidate::host(sock.local_addr().unwrap(), "udp").unwrap());
        let mut api = rtc.sdp_api();
        let mid = api.add_media(MediaKind::Audio, dir, None, None, None);
        let (offer, pending) = api.apply().unwrap();
        let enc = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip).unwrap();
        (Peer { rtc, sock, mid, connected: false, tone, enc, frames: 0, heard: Vec::new() }, offer.to_sdp_string(), pending)
    }

    /// Run the peer until `until`; a speaker sends a 20 ms frame every 20 ms once connected.
    fn drive(&mut self, until: Instant) {
        let mut buf = vec![0u8; 2000];
        let mut next_frame = Instant::now();
        while Instant::now() < until {
            loop {
                match self.rtc.poll_output().unwrap() {
                    Output::Transmit(t) => {
                        self.sock.send_to(&t.contents, t.destination).unwrap();
                    }
                    Output::Timeout(_) => break,
                    Output::Event(Event::Connected) => self.connected = true,
                    Output::Event(Event::MediaData(d)) => self.heard.push((d.time.numer(), d.data.to_vec())),
                    Output::Event(_) => {}
                }
            }
            if let (Some((freq, level)), true) = (self.tone, self.connected) {
                if Instant::now() >= next_frame {
                    next_frame += Duration::from_millis(20);
                    let pcm: Vec<f32> = (0..960).map(|i| 0.3 * (2.0 * std::f32::consts::PI * freq * (self.frames * 960 + i) as f32 / 48_000.0).sin()).collect();
                    let mut out = vec![0u8; 1500];
                    let n = self.enc.encode_float(&pcm, &mut out).unwrap();
                    self.frames += 1;
                    if let Some(writer) = self.rtc.writer(self.mid) {
                        let pt = writer.payload_params().find(|p| p.spec().codec == Codec::Opus).map(|p| p.pt());
                        if let Some(pt) = pt {
                            let _ = writer.audio_level(level, true).write(pt, Instant::now(), MediaTime::new(self.frames * 960, str0m::media::Frequency::FORTY_EIGHT_KHZ), out[..n].to_vec());
                        }
                    }
                }
            }
            if let Ok((n, from)) = self.sock.recv_from(&mut buf) {
                let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source: from, destination: self.sock.local_addr().unwrap(), contents: buf[..n].try_into().unwrap() });
                self.rtc.handle_input(input).unwrap();
            }
            self.rtc.handle_input(Input::Timeout(Instant::now())).unwrap();
        }
    }

    /// What this peer heard since the last call, decoded.
    fn take_heard(&mut self) -> Vec<f32> {
        let mut frames = std::mem::take(&mut self.heard);
        frames.sort_by_key(|(t, _)| *t);
        decode(frames.iter().map(|(_, d)| d.as_slice()))
    }
}

fn decode<'a>(packets: impl Iterator<Item = &'a [u8]>) -> Vec<f32> {
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let mut out = Vec::new();
    for p in packets {
        let mut buf = vec![0f32; 5760];
        if let Ok(n) = dec.decode_float(p, &mut buf, false) {
            out.extend_from_slice(&buf[..n]);
        }
    }
    out
}

/// How much of `freq` there is in `pcm` (a one-bin DFT over the last second, normalised).
fn power(pcm: &[f32], freq: f32) -> f32 {
    let pcm = &pcm[pcm.len().saturating_sub(48_000)..];
    let (mut re, mut im) = (0f32, 0f32);
    for (i, x) in pcm.iter().enumerate() {
        let w = 2.0 * std::f32::consts::PI * freq * i as f32 / 48_000.0;
        re += x * w.cos();
        im += x * w.sin();
    }
    (re * re + im * im).sqrt() / pcm.len().max(1) as f32
}

/// The Opus packets of an Ogg Opus file (one per page after the two header pages, as the host writes them).
fn ogg_packets(bytes: &[u8]) -> Vec<Vec<u8>> {
    let (mut pos, mut out) = (0, Vec::new());
    while pos + 27 <= bytes.len() {
        let segs = bytes[pos + 26] as usize;
        let body: usize = bytes[pos + 27..pos + 27 + segs].iter().map(|b| *b as usize).sum();
        let data = &bytes[pos + 27 + segs..pos + 27 + segs + body];
        if !data.is_empty() && !data.starts_with(b"OpusHead") && !data.starts_with(b"OpusTags") {
            out.push(data.to_vec());
        }
        pos += 27 + segs + body;
    }
    out
}

async fn offer(srv: &Server, token: &str, sid: &str, dir: Direction, send: bool, tone: Option<(f32, i8)>) -> Peer {
    offer_on(srv, token, sid, 0, dir, send, tone).await
}

/// An offer as the session channel's connection `conn` makes it (kRPC calls are connection 0).
async fn offer_on(srv: &Server, token: &str, sid: &str, conn: u64, dir: Direction, send: bool, tone: Option<(f32, i8)>) -> Peer {
    let (mut peer, sdp, pending) = Peer::new(dir, tone);
    let answer = srv.rpc(token, "session.rtc", json!({ "session_id": sid, "op": "offer", "sdp": sdp, "send": send, "_conn": conn })).await;
    assert_eq!(answer["send"], send, "{answer}");
    peer.rtc.sdp_api().accept_answer(pending, SdpAnswer::from_sdp_string(answer["sdp"].as_str().unwrap()).unwrap()).unwrap();
    peer
}

/// Drive every peer for `ms` at once.
async fn drive(peers: Vec<Peer>, ms: u64) -> Vec<Peer> {
    let until = Instant::now() + Duration::from_millis(ms);
    let tasks: Vec<_> = peers.into_iter().map(|mut p| tokio::task::spawn_blocking(move || { p.drive(until); p })).collect();
    let mut out = Vec::new();
    for t in tasks {
        out.push(t.await.unwrap());
    }
    out
}

async fn events_of(srv: &Server, sid: &str, kind: &str) -> Vec<Value> {
    let events = srv.rpc("tok-alice", "session.events", json!({ "session_id": sid, "after": 0, "limit": 2000 })).await;
    events["events"].as_array().unwrap().iter().filter(|e| e["type"] == json!(kind)).cloned().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_mixes_who_may_speak_and_each_voice_hears_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start_with(dir.path(), "0", &[("AIWS_VOICE_MAX_SPEAKERS", "2")]).await;
    let (sid, _) = srv.session().await;
    let got = srv.rpc("tok-alice", "session.get", json!({ "session_id": sid })).await;
    assert_eq!(got["voice_service"]["available"], true, "{got}");
    assert_eq!(got["voice_service"]["mixing"]["max_speakers"], 2, "{got}");
    srv.rpc("tok-alice", "session.start", json!({ "session_id": sid })).await;
    for who in ["bob", "dave"] {
        srv.rpc("tok-alice", "session.voice", json!({ "session_id": sid, "op": "allow", "principal": who, "allow": true })).await;
    }
    // carol may not speak: she asks to send and only listens
    let (_carol_try, sdp, _) = Peer::new(Direction::SendRecv, None);
    let r = srv.rpc("tok-carol", "session.rtc", json!({ "session_id": sid, "op": "offer", "sdp": sdp, "send": true })).await;
    assert_eq!(r["send"], false, "{r}");
    srv.rpc("tok-carol", "session.rtc", json!({ "session_id": sid, "op": "close" })).await;

    let alice = offer(&srv, "tok-alice", &sid, Direction::SendRecv, true, Some((440.0, -20))).await;
    let bob = offer(&srv, "tok-bob", &sid, Direction::SendRecv, true, Some((880.0, -20))).await;
    let carol = offer(&srv, "tok-carol", &sid, Direction::RecvOnly, false, None).await;
    let mut peers = drive(vec![alice, bob, carol], 1500).await;
    // a third voice, quieter than both: with K = 2 it never gets into the mix, so it is never decoded
    peers.push(offer(&srv, "tok-dave", &sid, Direction::SendRecv, true, Some((1320.0, -35))).await);
    for p in peers.iter_mut() {
        p.take_heard();
    }
    let mut peers = drive(peers, 3500).await;
    assert!(peers.iter().all(|p| p.connected));
    let heard: Vec<Vec<f32>> = peers.iter_mut().map(|p| p.take_heard()).collect();
    let (a, b, c, d) = (&heard[0], &heard[1], &heard[2], &heard[3]);
    eprintln!("listener 440 {} 880 {} 1320 {}", power(c, 440.0), power(c, 880.0), power(c, 1320.0));
    assert!(power(c, 440.0) > 0.03 && power(c, 880.0) > 0.03, "the listener hears both voices");
    assert!(power(c, 1320.0) < 0.01, "the third voice is not in the mix");
    assert!(power(a, 880.0) > 0.03 && power(a, 440.0) < 0.01, "alice hears bob, not herself: {} {}", power(a, 880.0), power(a, 440.0));
    assert!(power(b, 440.0) > 0.03 && power(b, 880.0) < 0.01, "bob hears alice, not himself");
    assert!(power(d, 440.0) > 0.03 && power(d, 880.0) > 0.03, "a voice out of the mix hears the whole mix");

    // dave hangs up: what became of his packets is on the diag track
    let mut dave = peers.remove(3);
    srv.rpc("tok-dave", "session.rtc", json!({ "session_id": sid, "op": "close" })).await;
    dave.drive(Instant::now() + Duration::from_millis(200));
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mix = events_of(&srv, &sid, "voice.mix").await;
    let dave_mix = mix.iter().find(|e| e["data"]["principal"] == "dave").unwrap_or_else(|| panic!("{mix:?}"));
    // a voice in the mix that pauses over 500 ms gives up its place; on a loaded machine a stalled peer can let dave in
    // for a moment, never for long
    assert!(dave_mix["data"]["decoded"].as_u64().unwrap() <= 5, "{dave_mix}");
    assert!(dave_mix["data"]["skipped"].as_u64().unwrap() > 50, "{dave_mix}");
    assert!(events_of(&srv, &sid, "voice.uplink").await.iter().any(|e| e["data"]["principal"] == "dave" && e["data"]["bytes"].as_u64().unwrap() > 0));

    // bob loses the right to speak: he is out of the mix at once, whatever his client keeps sending
    srv.rpc("tok-alice", "session.voice", json!({ "session_id": sid, "op": "allow", "principal": "bob", "allow": false })).await;
    let mut peers = drive(peers, 3000).await;
    let c = peers[2].take_heard();
    for p in peers.iter_mut() {
        p.take_heard();
    }
    assert!(power(&c, 440.0) > 0.03 && power(&c, 880.0) < 0.01, "only alice now: {} {}", power(&c, 440.0), power(&c, 880.0));

    // carol is allowed and changes her listening connection to a sending one (the same connection, renegotiated)
    srv.rpc("tok-alice", "session.voice", json!({ "session_id": sid, "op": "allow", "principal": "carol", "allow": true })).await;
    let mut carol = peers.remove(2);
    let mut api = carol.rtc.sdp_api();
    api.set_direction(carol.mid, Direction::SendRecv);
    let (sdp, pending) = api.apply().unwrap();
    let r = srv.rpc("tok-carol", "session.rtc", json!({ "session_id": sid, "op": "offer", "sdp": sdp.to_sdp_string(), "send": true })).await;
    assert_eq!(r["send"], true, "{r}");
    carol.rtc.sdp_api().accept_answer(pending, SdpAnswer::from_sdp_string(r["sdp"].as_str().unwrap()).unwrap()).unwrap();
    carol.tone = Some((660.0, -20));
    peers.push(carol);
    let mut peers = drive(peers, 3000).await;
    let a = peers[0].take_heard();
    for p in peers.iter_mut() {
        p.take_heard();
    }
    assert!(power(&a, 660.0) > 0.03, "alice hears carol on her renegotiated connection: {}", power(&a, 660.0));

    // mute all: everyone but the presenter is held until they pick a mode again
    srv.rpc("tok-alice", "session.voice", json!({ "session_id": sid, "op": "mute_all" })).await;
    let mut peers = drive(peers, 2500).await;
    let a = peers[0].take_heard();
    assert!(power(&a, 660.0) < 0.01, "carol is held: {}", power(&a, 660.0));
    srv.rpc("tok-carol", "session.voice", json!({ "session_id": sid, "op": "mode", "mode": "auto" })).await;
    let mut peers = drive(peers, 2500).await;
    let a = peers[0].take_heard();
    assert!(power(&a, 660.0) > 0.03, "carol speaks again: {}", power(&a, 660.0));

    // the end of the voice: the recording is the mix, `voice.speaking` says who spoke
    for (token, _) in &TOKENS {
        srv.rpc(token, "session.rtc", json!({ "session_id": sid, "op": "close" })).await;
    }
    drop(peers);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let segments = events_of(&srv, &sid, "voice.segment").await;
    assert!(segments.len() >= 8, "{segments:?}");
    let run = srv.rpc("tok-alice", "session.get", json!({ "session_id": sid })).await["workspace_id"].as_str().unwrap().to_string();
    let first = srv.asset("tok-alice", &run, segments[1]["data"]["segment_id"].as_str().unwrap()).await;
    let pcm = decode(ogg_packets(&first).iter().map(Vec::as_slice));
    assert!(power(&pcm, 440.0) > 0.03 && power(&pcm, 880.0) > 0.03, "the recording is the mix: {} {}", power(&pcm, 440.0), power(&pcm, 880.0));
    let speaking = events_of(&srv, &sid, "voice.speaking").await;
    for who in ["alice", "bob", "carol"] {
        assert!(speaking.iter().any(|e| e["data"]["principal"] == who && e["data"]["on"] == true), "{who}: {speaking:?}");
    }
    assert!(!speaking.iter().any(|e| e["data"]["principal"] == "dave"), "{speaking:?}");
    let allow = events_of(&srv, &sid, "voice.allow").await;
    assert_eq!(allow.len(), 4, "{allow:?}");
    let kinds: Vec<String> = srv.rpc("tok-bob", "session.events", json!({ "session_id": sid, "after": 0, "limit": 2000 })).await["events"].as_array().unwrap().iter()
        .filter_map(|e| e["type"].as_str()).filter(|t| t.starts_with("voice.")).map(str::to_string).collect();
    assert_eq!(kinds.first().map(String::as_str), Some("voice.allow"), "{kinds:?}");
    assert!(kinds.contains(&"voice.start".to_string()) && kinds.contains(&"voice.stop".to_string()), "{kinds:?}");
    // bob reads his own voice.allow events, not anyone else's, and nothing of the diag track
    assert!(!kinds.contains(&"voice.mix".to_string()));
    let bobs: Vec<Value> = srv.rpc("tok-bob", "session.events", json!({ "session_id": sid, "after": 0, "limit": 2000 })).await["events"].as_array().unwrap().iter()
        .filter(|e| e["type"] == "voice.allow").cloned().collect();
    assert!(bobs.iter().all(|e| e["data"]["principal"] == "bob") && bobs.len() == 2, "{bobs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offer_whose_packets_never_reach_the_port_puts_no_voice_on_the_time_line() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path(), "0").await;
    let (sid, _) = srv.session().await;
    srv.rpc("tok-alice", "session.start", json!({ "session_id": sid })).await;
    // a firewall in front of the port: the answer comes back over the channel, no packet goes out
    let (_alice, offer, _pending) = Peer::new(Direction::SendRecv, Some((440.0, -20)));
    let answer = srv.rpc("tok-alice", "session.rtc", json!({ "session_id": sid, "op": "offer", "sdp": offer, "send": true })).await;
    assert_eq!(answer["send"], true, "{answer}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    srv.rpc("tok-alice", "session.rtc", json!({ "session_id": sid, "op": "close" })).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let events = srv.rpc("tok-bob", "session.events", json!({ "session_id": sid, "after": 0 })).await;
    assert!(!events["events"].as_array().unwrap().iter().any(|e| e["type"].as_str().is_some_and(|t| t.starts_with("voice."))), "{events}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_without_its_media_port_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path(), "off").await;
    let (sid, _) = srv.session().await;
    let got = srv.rpc("tok-alice", "session.get", json!({ "session_id": sid })).await;
    assert_eq!(got["voice_service"]["available"], false);
    assert!(got["voice_service"]["reason"].as_str().unwrap().contains("off"));
    let r = srv.rpc("tok-alice", "session.rtc", json!({ "session_id": sid, "op": "offer", "sdp": "v=0" })).await;
    assert_eq!(r["error"]["code"], "DEPENDENCY_UNAVAILABLE", "{r}");
}

/// The host's CPU for one class: `AIWS_LOAD_SPEAKERS` voices (4) and `AIWS_LOAD_CONNECTIONS` connections in all (60) for
/// `AIWS_LOAD_SECONDS` (15), measured from the host process's CPU time (Linux). Run it on the OOD it is for:
/// `cargo test --release -p aiworkspace --test voice -- --ignored --nocapture load`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn load_one_class_of_many_connections() {
    let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (speakers, connections, seconds) = (env("AIWS_LOAD_SPEAKERS", 4), env("AIWS_LOAD_CONNECTIONS", 60), env("AIWS_LOAD_SECONDS", 15));
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path(), "0").await;
    let (sid, _) = srv.session().await;
    srv.rpc("tok-alice", "session.set_access", json!({ "session_id": sid, "speak": "all" })).await;
    srv.rpc("tok-alice", "session.start", json!({ "session_id": sid })).await;
    let tokens: Vec<&str> = TOKENS.iter().map(|(t, _)| *t).collect();
    let mut peers = Vec::new();
    for i in 0..connections {
        let token = tokens[i as usize % tokens.len()];
        let speak = i < speakers;
        // one person sends on one connection: the speakers are the first of each account, the rest listen
        let tone = speak.then(|| (300.0 + 110.0 * i as f32, -20 - i as i8));
        peers.push(offer_on(&srv, token, &sid, i + 1, if speak { Direction::SendRecv } else { Direction::RecvOnly }, speak, tone).await);
    }
    let cpu = |pid: u32| -> f64 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let f: Vec<&str> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
        (f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap()) as f64 / 100.0
    };
    let pid = srv.child.id();
    let mut peers = drive(peers, 3000).await;
    assert!(peers.iter().all(|p| p.connected), "every connection is up");
    for p in peers.iter_mut() {
        p.take_heard();
    }
    let (c0, t0) = (cpu(pid), Instant::now());
    let mut peers = drive(peers, seconds * 1000).await;
    let (used, wall) = (cpu(pid) - c0, t0.elapsed().as_secs_f64());
    let heard = peers.iter_mut().skip(speakers as usize).map(|p| p.take_heard().len()).min().unwrap_or(0);
    eprintln!("load: {speakers} voices, {connections} connections, {wall:.1} s: host CPU {:.1}% of one core; every listener got ≥ {:.1} s of audio",
              100.0 * used / wall, heard as f64 / 48_000.0);
    assert!(connections <= speakers || heard as f64 / 48_000.0 > seconds as f64 * 0.8, "listeners heard the mix");
}
