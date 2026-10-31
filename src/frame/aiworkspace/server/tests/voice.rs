//! The voice of a session against the real process (第四期规划 §7.3.1, §7.3.3, P4-15/P4-18): two WebRTC peers —
//! str0m standing in for browsers — negotiate through `session.rtc`; what the presenter sends reaches the listener
//! through the host (no peer relays), and the host records it as Ogg Opus segments on the session's time line.

use serde_json::{json, Value};
use std::net::UdpSocket;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use str0m::change::SdpAnswer;
use str0m::format::Codec;
use str0m::media::{Direction, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

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
        let auth = dir.join("tokens.json");
        std::fs::write(&auth, json!({ "tokens": { "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" } } }).to_string()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new(env!("CARGO_BIN_EXE_aiworkspace"))
            .arg("--data-dir").arg(dir.join("data")).arg("--listen").arg(format!("127.0.0.1:{port}")).arg("--auth-file").arg(&auth)
            .arg("--log-level").arg("warn").env("AIWS_MEDIA_PORT", media).env("AIWS_MEDIA_HOSTS", "127.0.0.1").env("AIWS_VOICE_SEGMENT_MS", "1000")
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
        assert_eq!(self.rpc("tok-bob", "session.join", json!({ "session_id": sid, "link": at })).await["ok"], true);
        (sid, id)
    }
}

/// A browser-like peer: full ICE on a loopback socket, one audio m-line.
struct Peer {
    rtc: Rtc,
    sock: UdpSocket,
    mid: Mid,
    connected: bool,
    received: usize,
}

impl Peer {
    fn new(dir: Direction) -> (Peer, String, str0m::change::SdpPendingOffer) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
        let mut rtc = Rtc::builder().build(Instant::now());
        rtc.add_local_candidate(Candidate::host(sock.local_addr().unwrap(), "udp").unwrap());
        let mut api = rtc.sdp_api();
        let mid = api.add_media(MediaKind::Audio, dir, None, None, None);
        let (offer, pending) = api.apply().unwrap();
        (Peer { rtc, sock, mid, connected: false, received: 0 }, offer.to_sdp_string(), pending)
    }

    /// Run the peer for a while; the publisher sends a 20 ms Opus frame every 20 ms once connected.
    fn drive(&mut self, until: Instant, publish: bool) {
        let mut buf = vec![0u8; 2000];
        let mut sent_at = Instant::now();
        let mut rtp: u64 = 0;
        while Instant::now() < until {
            loop {
                match self.rtc.poll_output().unwrap() {
                    Output::Transmit(t) => {
                        self.sock.send_to(&t.contents, t.destination).unwrap();
                    }
                    Output::Timeout(_) => break,
                    Output::Event(Event::Connected) => self.connected = true,
                    Output::Event(Event::MediaData(_)) => self.received += 1,
                    Output::Event(_) => {}
                }
            }
            if publish && self.connected && sent_at.elapsed() >= Duration::from_millis(20) {
                sent_at = Instant::now();
                if let Some(writer) = self.rtc.writer(self.mid) {
                    let pt = writer.payload_params().find(|p| p.spec().codec == Codec::Opus).map(|p| p.pt());
                    if let Some(pt) = pt {
                        rtp += 960;
                        let _ = writer.write(pt, Instant::now(), MediaTime::new(rtp, str0m::media::Frequency::FORTY_EIGHT_KHZ), vec![0xf8, 0xff, 0xfe]);
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
}

#[tokio::test(flavor = "multi_thread")]
async fn the_presenters_voice_reaches_the_audience_through_the_host_and_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start(dir.path(), "0").await;
    let (sid, _) = srv.session().await;
    let got = srv.rpc("tok-alice", "session.get", json!({ "session_id": sid })).await;
    assert_eq!(got["voice_service"]["available"], true, "{got}");
    srv.rpc("tok-alice", "session.start", json!({ "session_id": sid })).await;
    let (mut alice, offer, pending) = Peer::new(Direction::SendOnly);
    let answer = srv.rpc("tok-alice", "session.rtc", json!({ "session_id": sid, "op": "offer", "sdp": offer })).await;
    assert_eq!(answer["role"], "publish", "{answer}");
    alice.rtc.sdp_api().accept_answer(pending, SdpAnswer::from_sdp_string(answer["sdp"].as_str().unwrap()).unwrap()).unwrap();
    let (mut bob, offer, pending) = Peer::new(Direction::RecvOnly);
    let answer = srv.rpc("tok-bob", "session.rtc", json!({ "session_id": sid, "op": "offer", "sdp": offer })).await;
    assert_eq!(answer["role"], "listen");
    bob.rtc.sdp_api().accept_answer(pending, SdpAnswer::from_sdp_string(answer["sdp"].as_str().unwrap()).unwrap()).unwrap();
    let until = Instant::now() + Duration::from_millis(3500);
    let listener = tokio::task::spawn_blocking(move || {
        bob.drive(until, false);
        bob
    });
    let speaker = tokio::task::spawn_blocking(move || {
        alice.drive(until, true);
        alice
    });
    let (alice, bob) = (speaker.await.unwrap(), listener.await.unwrap());
    assert!(alice.connected && bob.connected);
    assert!(bob.received > 50, "the listener got {} frames", bob.received);
    // the presenter hangs up: the last segment is written; the time line names every segment
    srv.rpc("tok-alice", "session.rtc", json!({ "session_id": sid, "op": "close" })).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let events = srv.rpc("tok-bob", "session.events", json!({ "session_id": sid, "after": 0 })).await;
    let segments: Vec<&Value> = events["events"].as_array().unwrap().iter().filter(|e| e["type"] == json!("voice.segment")).collect();
    assert!(segments.len() >= 2, "{events}");
    for s in &segments {
        assert_eq!(s["data"]["media_type"], "audio/ogg", "{s}");
        assert!(s["data"]["t_rel_end"].as_i64().unwrap() > s["data"]["t_rel_start"].as_i64().unwrap());
    }
    assert!(events["events"].as_array().unwrap().iter().any(|e| e["type"] == json!("voice.start")));
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
