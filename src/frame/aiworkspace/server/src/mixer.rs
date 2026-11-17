//! The mixer of a session's voice (多人发言 TODO §4.5): every sender's packets go into a jitter buffer; every 20 ms the
//! loudest `K` voices by the browsers' `ssrc-audio-level` header are decoded — the presenter always among them while
//! they speak — and summed. Everyone gets the one total mix, encoded once; each voice in the mix gets the mix without
//! itself, so nobody hears their own voice come back. Packets of voices not in the mix are dropped undecoded, so a
//! client without a gate costs the host bandwidth but no decoding. It runs on a thread of its own: the media loop
//! hands it packets and takes back encoded frames, one a tick, `all: None` while nobody speaks (the downlink goes quiet,
//! the recording writes silence).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Samples of one tick (20 ms at 48 kHz).
pub const FRAME: usize = 960;
const TICK: Duration = Duration::from_millis(20);
/// Jitter buffer: where a talkspurt starts, and the range the delay adapts in (§4.5: 60 ms, 40–120 ms).
const DELAY_START: u64 = 3 * FRAME as u64;
const DELAY_MIN: u64 = 2 * FRAME as u64;
const DELAY_MAX: u64 = 6 * FRAME as u64;
/// A voice is heard from this level up (`ssrc-audio-level` is −dBov; −50 dBov is well above a quiet room after the
/// browser's noise suppression).
const VOICED_DBOV: i8 = -50;
/// A voice not yet in a full mix replaces the weakest one only when it is this much louder (dB).
const SWAP_MARGIN: f32 = 6.0;
/// A voice keeps its place in the mix through pauses this long (ticks).
const HANGOVER: u32 = 25;
/// `voice.speaking` (§4.6): on after 300 ms in the mix and audible, off after 1 s without.
const SPEAKING_ON: u32 = 15;
const SPEAKING_OFF: u32 = 50;
/// A sender with nothing to play for this long ends its talkspurt (ticks).
const IDLE: u32 = 10;
/// Losses up to this long are concealed (PLC / FEC); longer gaps are the sender's DTX.
const CONCEAL_MAX: u64 = 5 * FRAME as u64;

pub enum MixIn {
    Join { id: u64, principal: Arc<str>, name: Arc<str>, presenter: bool },
    Packet { id: u64, rtp: u64, level: Option<i8>, data: Vec<u8>, at: Instant },
    Leave { id: u64 },
}

#[derive(Debug)]
pub enum MixOut {
    /// One tick: the total mix (`None`: nobody in it) and the mixes without each voice in it, by sender id.
    Frame { session: Arc<str>, rtp: u64, all: Option<Vec<u8>>, own: Vec<(u64, Vec<u8>)> },
    /// `voice.speaking` (§4.6, Q8).
    Speaking { session: Arc<str>, principal: Arc<str>, name: Arc<str>, on: bool },
    /// What became of a sender's packets, when it leaves (S10, T1).
    /// `levels`: packets that carried the browser's audio level (V4).
    Counts { session: Arc<str>, principal: Arc<str>, decoded: u64, skipped: u64, late: u64, concealed: u64, levels: u64 },
}

/// The handle the media loop keeps; dropping it ends the thread once it has drained what was sent.
pub struct MixerHandle {
    pub tx: mpsc::Sender<MixIn>,
}

/// Start the mixer of `session` (at most `k` voices at once, Q2).
pub fn spawn(session: Arc<str>, k: usize, out: tokio::sync::mpsc::UnboundedSender<MixOut>) -> MixerHandle {
    let (tx, rx) = mpsc::channel();
    let name = format!("aiws-mix-{}", session.chars().take(12).collect::<String>());
    let spawned = std::thread::Builder::new().name(name).spawn(move || match Mixer::new(session.clone(), k) {
        Ok(mut m) => m.run(rx, out),
        Err(e) => log::warn!("session {session}: the mixer could not start: {e}"),
    });
    if let Err(e) = spawned {
        log::warn!("the mixer thread could not start: {e}");
    }
    MixerHandle { tx }
}

fn encoder() -> Result<opus::Encoder, opus::Error> {
    let mut e = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip)?;
    e.set_bitrate(opus::Bitrate::Bits(32_000))?;
    e.set_vbr(true)?;
    e.set_inband_fec(true)?;
    e.set_packet_loss_perc(10)?;
    e.set_dtx(true)?;
    Ok(e)
}

struct Pkt {
    level: Option<i8>,
    data: Vec<u8>,
}

impl Pkt {
    /// A DTX or comfort-noise frame: nothing to hear.
    fn dtx(&self) -> bool {
        self.data.len() <= 2
    }
    fn voiced(&self) -> bool {
        !self.dtx() && self.level.is_none_or(|l| l >= VOICED_DBOV)
    }
    /// Loudness on 0…127 (127: 0 dBov); a packet without the header counts as a normal voice.
    fn loudness(&self) -> f32 {
        if self.dtx() {
            return 0.0;
        }
        127.0 + self.level.unwrap_or(-40) as f32
    }
}

struct Sender {
    principal: Arc<str>,
    name: Arc<str>,
    presenter: bool,
    decoder: opus::Decoder,
    buf: BTreeMap<u64, Pkt>,
    /// The RTP time playing now; `None` between talkspurts.
    cursor: Option<u64>,
    delay: u64,
    /// Interarrival jitter (RFC 3550), in samples.
    jitter: f64,
    last_transit: Option<f64>,
    /// Decoded samples from `cursor` on.
    pcm: VecDeque<f32>,
    /// The end of the last packet taken (loss detection).
    next_rtp: Option<u64>,
    loudness: f32,
    voiced_ticks: u32,
    quiet_ticks: u32,
    starved: u32,
    late_in_spurt: bool,
    spurts: u32,
    speaking: bool,
    audible_run: u32,
    silent_run: u32,
    decoded: u64,
    skipped: u64,
    late: u64,
    concealed: u64,
    levels: u64,
}

impl Sender {
    fn new(principal: Arc<str>, name: Arc<str>, presenter: bool) -> Result<Sender, opus::Error> {
        Ok(Sender { principal, name, presenter, decoder: opus::Decoder::new(48_000, opus::Channels::Mono)?, buf: BTreeMap::new(), cursor: None, delay: DELAY_START,
                    jitter: 0.0, last_transit: None, pcm: VecDeque::new(), next_rtp: None, loudness: 0.0, voiced_ticks: 0, quiet_ticks: u32::MAX / 2, starved: 0,
                    late_in_spurt: false, spurts: 0, speaking: false, audible_run: 0, silent_run: 0, decoded: 0, skipped: 0, late: 0, concealed: 0, levels: 0 })
    }

    fn arrive(&mut self, rtp: u64, pkt: Pkt, at: Instant, epoch: Instant) {
        let arrival = at.saturating_duration_since(epoch).as_secs_f64() * 48_000.0;
        let transit = arrival - rtp as f64;
        if let Some(prev) = self.last_transit {
            let d = (transit - prev).abs();
            if d < 48_000.0 {
                self.jitter += (d - self.jitter) / 16.0;
            }
        }
        self.last_transit = Some(transit);
        if pkt.level.is_some() {
            self.levels += 1;
        }
        if self.cursor.is_some_and(|c| rtp + crate::media::opus_samples(&pkt.data).max(1) <= c) {
            // too late for its tick: the next talkspurt waits longer
            self.late += 1;
            self.late_in_spurt = true;
            return;
        }
        if self.buf.len() < 500 {
            self.buf.insert(rtp, pkt);
        }
    }

    /// A talkspurt starts: the first packet plays after the jitter delay.
    fn anchor(&mut self) {
        let Some((&first, _)) = self.buf.iter().next() else { return };
        let want = ((3.0 * self.jitter) as u64 + FRAME as u64).div_ceil(FRAME as u64) * FRAME as u64;
        // the delay follows the jitter: up at once (and after a late packet), down a frame per talkspurt
        let delay = if self.spurts == 0 {
            want.max(DELAY_START)
        } else if self.late_in_spurt {
            want.max(self.delay + FRAME as u64)
        } else {
            want.max(self.delay.saturating_sub(FRAME as u64))
        };
        self.delay = delay.clamp(DELAY_MIN, DELAY_MAX);
        self.late_in_spurt = false;
        self.spurts += 1;
        self.cursor = Some(first.saturating_sub(self.delay));
        self.pcm.clear();
        self.next_rtp = None;
    }

    /// The packets that play in this tick, oldest first.
    fn due(&self) -> impl Iterator<Item = (&u64, &Pkt)> {
        let end = self.cursor.map(|c| c + FRAME as u64).unwrap_or(0);
        self.buf.range(..end)
    }

    /// Take this tick's packets: decoded into `pcm` when the voice is in the mix, dropped otherwise.
    fn take(&mut self, mixed: bool) {
        let Some(cursor) = self.cursor else { return };
        let end = cursor + FRAME as u64;
        let due: Vec<u64> = self.buf.range(..end).map(|(r, _)| *r).collect();
        if due.is_empty() && self.pcm.is_empty() {
            self.starved += 1;
        } else {
            self.starved = 0;
        }
        for rtp in due {
            let pkt = self.buf.remove(&rtp).unwrap();
            let samples = crate::media::opus_samples(&pkt.data).max(1);
            if !mixed {
                self.skipped += 1;
                self.next_rtp = Some(rtp + samples);
                continue;
            }
            let filled = cursor + self.pcm.len() as u64;
            if rtp + samples <= filled {
                continue;
            }
            if rtp > filled {
                // the start of a talkspurt, or the end of the sender's silence: nothing was said there
                self.pcm.extend(std::iter::repeat_n(0.0, (rtp - filled) as usize));
            }
            let mut out = vec![0f32; 5760];
            match self.decoder.decode_float(&pkt.data, &mut out, false) {
                Ok(n) => {
                    self.decoded += 1;
                    let skip = (cursor + self.pcm.len() as u64).saturating_sub(rtp) as usize;
                    self.pcm.extend(out[skip.min(n)..n].iter().copied());
                }
                Err(_) => self.pcm.extend(std::iter::repeat_n(0.0, samples as usize)),
            }
            self.next_rtp = Some(rtp + samples);
        }
        // nothing for this tick although the talkspurt goes on: a packet was lost on the way. The one after it carries
        // its FEC when it is already here; otherwise the decoder conceals it.
        if mixed && self.pcm.is_empty() && self.next_rtp.is_some_and(|n| n < end) {
            if let Some((&next, pkt)) = self.buf.iter().next().filter(|(r, _)| **r - self.next_rtp.unwrap_or(0) <= CONCEAL_MAX) {
                let fec = next <= end;
                let mut out = [0f32; FRAME];
                if let Ok(n) = self.decoder.decode_float(if fec { &pkt.data } else { &[] }, &mut out, fec) {
                    self.concealed += 1;
                    self.pcm.extend(out[..n.min(FRAME)].iter().copied());
                }
                self.next_rtp = Some(end);
            }
        }
    }

    /// This tick's samples (zeros where nothing was decoded); the cursor moves on.
    fn advance(&mut self, out: &mut [f32; FRAME]) {
        for o in out.iter_mut() {
            *o = self.pcm.pop_front().unwrap_or(0.0);
        }
        if let Some(c) = self.cursor.as_mut() {
            *c += FRAME as u64;
        }
        if self.starved >= IDLE && self.buf.is_empty() {
            self.cursor = None;
            self.pcm.clear();
            self.starved = 0;
        }
    }
}

struct Mixer {
    session: Arc<str>,
    k: usize,
    senders: HashMap<u64, Sender>,
    /// The voices in the mix, in the order they came in.
    mixed: Vec<u64>,
    all: opus::Encoder,
    own: HashMap<u64, opus::Encoder>,
    clip: opus::SoftClip,
    rtp0: u64,
    tick: u64,
    epoch: Instant,
}

impl Mixer {
    fn new(session: Arc<str>, k: usize) -> Result<Mixer, opus::Error> {
        let rtp0 = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64).unwrap_or(0) & 0x7fff_ffff;
        Ok(Mixer { session, k: k.max(1), senders: HashMap::new(), mixed: Vec::new(), all: encoder()?, own: HashMap::new(), clip: opus::SoftClip::new(opus::Channels::Mono),
                   rtp0, tick: 0, epoch: Instant::now() })
    }

    fn run(&mut self, rx: mpsc::Receiver<MixIn>, out: tokio::sync::mpsc::UnboundedSender<MixOut>) {
        let mut start = Instant::now();
        let mut ticks: u32 = 0;
        loop {
            let deadline = start + TICK * (ticks + 1);
            loop {
                let wait = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(m) => self.input(m, &out),
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            for m in self.step() {
                if out.send(m).is_err() {
                    return;
                }
            }
            ticks += 1;
            // a host that stalled (suspended, overloaded) does not catch up in a burst
            if Instant::now() > start + TICK * (ticks + 10) || ticks > 1_000_000 {
                start = Instant::now();
                ticks = 0;
            }
        }
    }

    fn input(&mut self, m: MixIn, out: &tokio::sync::mpsc::UnboundedSender<MixOut>) {
        match m {
            MixIn::Join { id, principal, name, presenter } => match Sender::new(principal, name, presenter) {
                Ok(s) => {
                    self.senders.insert(id, s);
                }
                Err(e) => log::warn!("session {}: no decoder: {e}", self.session),
            },
            MixIn::Packet { id, rtp, level, data, at } => {
                if let Some(s) = self.senders.get_mut(&id) {
                    s.arrive(rtp, Pkt { level, data }, at, self.epoch);
                }
            }
            MixIn::Leave { id } => {
                self.mixed.retain(|m| *m != id);
                self.own.remove(&id);
                if let Some(s) = self.senders.remove(&id) {
                    if s.speaking {
                        let _ = out.send(MixOut::Speaking { session: self.session.clone(), principal: s.principal.clone(), name: s.name.clone(), on: false });
                    }
                    let _ = out.send(MixOut::Counts { session: self.session.clone(), principal: s.principal, decoded: s.decoded, skipped: s.skipped, late: s.late, concealed: s.concealed, levels: s.levels });
                }
            }
        }
    }

    /// Who is in the mix this tick (§4.5): the presenter whenever they speak, then the loudest, with hysteresis.
    fn select(&mut self) {
        for s in self.senders.values_mut() {
            if s.cursor.is_none() && !s.buf.is_empty() {
                s.anchor();
            }
            let (voiced, loud) = s.due().fold((false, 0f32), |(v, l), (_, p)| (v || p.voiced(), l.max(p.loudness())));
            s.loudness = s.loudness * 0.8 + loud * 0.2;
            if voiced {
                s.voiced_ticks = s.voiced_ticks.saturating_add(1);
                s.quiet_ticks = 0;
            } else {
                s.voiced_ticks = 0;
                s.quiet_ticks = s.quiet_ticks.saturating_add(1);
            }
        }
        let senders = &self.senders;
        self.mixed.retain(|id| senders.get(id).is_some_and(|s| s.quiet_ticks <= HANGOVER));
        let mut waiting: Vec<u64> = self.senders.iter().filter(|(id, s)| s.voiced_ticks > 0 && !self.mixed.contains(id)).map(|(id, _)| *id).collect();
        waiting.sort_by(|a, b| {
            let (sa, sb) = (&self.senders[a], &self.senders[b]);
            sb.presenter.cmp(&sa.presenter).then(sb.loudness.total_cmp(&sa.loudness)).then(a.cmp(b))
        });
        for id in waiting {
            if self.mixed.len() < self.k {
                self.mixed.push(id);
                continue;
            }
            let cand = &self.senders[&id];
            // the weakest voice that is not the presenter's
            let weakest = self.mixed.iter().copied().filter(|m| !self.senders[m].presenter).min_by(|a, b| self.senders[a].loudness.total_cmp(&self.senders[b].loudness));
            let Some(weakest) = weakest else { continue };
            let w = &self.senders[&weakest];
            if cand.presenter || (cand.voiced_ticks >= 3 && cand.loudness > w.loudness + SWAP_MARGIN) {
                self.mixed.retain(|m| *m != weakest);
                self.own.remove(&weakest);
                self.mixed.push(id);
            }
        }
    }

    fn step(&mut self) -> Vec<MixOut> {
        self.select();
        let mut total = [0f32; FRAME];
        let mut parts: Vec<(u64, [f32; FRAME])> = Vec::new();
        let mut events = Vec::new();
        let ids: Vec<u64> = self.senders.keys().copied().collect();
        for id in ids {
            let mixed = self.mixed.contains(&id);
            let s = self.senders.get_mut(&id).unwrap();
            let voiced = s.voiced_ticks > 0;
            s.take(mixed);
            let mut pcm = [0f32; FRAME];
            s.advance(&mut pcm);
            if mixed {
                for (t, x) in total.iter_mut().zip(pcm.iter()) {
                    *t += x;
                }
                parts.push((id, pcm));
            }
            // speaking: in the mix and audible for a while, quiet for a second before it ends
            if mixed && voiced {
                s.audible_run += 1;
                s.silent_run = 0;
            } else {
                s.silent_run += 1;
                s.audible_run = 0;
            }
            let on = if s.speaking { s.silent_run < SPEAKING_OFF } else { s.audible_run >= SPEAKING_ON };
            if on != s.speaking {
                s.speaking = on;
                events.push(MixOut::Speaking { session: self.session.clone(), principal: s.principal.clone(), name: s.name.clone(), on });
            }
        }
        let rtp = self.rtp0 + self.tick * FRAME as u64;
        self.tick += 1;
        let mut frame = MixOut::Frame { session: self.session.clone(), rtp, all: None, own: Vec::new() };
        if !parts.is_empty() {
            let MixOut::Frame { all, own, .. } = &mut frame else { unreachable!() };
            let mut buf = vec![0u8; 1500];
            let mut mix = total;
            self.clip.apply(&mut mix);
            if let Ok(n) = self.all.encode_float(&mix, &mut buf) {
                *all = Some(buf[..n].to_vec());
            }
            for (id, pcm) in &parts {
                let mut minus = total;
                for (m, x) in minus.iter_mut().zip(pcm.iter()) {
                    *m -= x;
                }
                self.clip.apply(&mut minus);
                let enc = match self.own.entry(*id) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(v) => match encoder() {
                        Ok(e) => v.insert(e),
                        Err(_) => continue,
                    },
                };
                // a DTX frame (≤ 2 bytes) need not be sent (RFC 6716 §3.1 / libopus)
                if let Ok(n) = enc.encode_float(&minus, &mut buf) {
                    if n > 2 {
                        own.push((*id, buf[..n].to_vec()));
                    }
                }
            }
        }
        self.own.retain(|id, _| self.mixed.contains(id));
        let mut out = vec![frame];
        out.extend(events);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, n: u64) -> [f32; FRAME] {
        let mut out = [0f32; FRAME];
        for (i, o) in out.iter_mut().enumerate() {
            let t = (n * FRAME as u64 + i as u64) as f32 / 48_000.0;
            *o = 0.3 * (2.0 * std::f32::consts::PI * freq * t).sin();
        }
        out
    }

    /// The energy of `freq` in `pcm` (a single-bin DFT, normalised).
    pub fn power(pcm: &[f32], freq: f32) -> f32 {
        let (mut re, mut im) = (0f32, 0f32);
        for (i, x) in pcm.iter().enumerate() {
            let w = 2.0 * std::f32::consts::PI * freq * i as f32 / 48_000.0;
            re += x * w.cos();
            im += x * w.sin();
        }
        (re * re + im * im).sqrt() / pcm.len() as f32
    }

    fn decode_all(frames: &[Vec<u8>]) -> Vec<f32> {
        let mut d = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
        let mut out = Vec::new();
        for f in frames {
            let mut buf = vec![0f32; 5760];
            let n = d.decode_float(f, &mut buf, false).unwrap();
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    #[test]
    fn everyone_hears_the_mix_and_each_voice_the_mix_without_itself() {
        let mut m = Mixer::new("s".into(), 4).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        m.input(MixIn::Join { id: 1, principal: "a".into(), name: "A".into(), presenter: true }, &tx);
        m.input(MixIn::Join { id: 2, principal: "b".into(), name: "B".into(), presenter: false }, &tx);
        m.input(MixIn::Join { id: 3, principal: "c".into(), name: "C".into(), presenter: false }, &tx);
        let (mut ea, mut eb, mut ec) = (encoder().unwrap(), encoder().unwrap(), encoder().unwrap());
        let (mut all, mut own_a, mut own_b) = (Vec::new(), Vec::new(), Vec::new());
        let mut speaking = Vec::new();
        let mut buf = vec![0u8; 1500];
        let t0 = Instant::now();
        for n in 0..150u64 {
            let at = t0 + TICK * n as u32;
            for (id, enc, freq, level) in [(1, &mut ea, 440.0, -20i8), (2, &mut eb, 880.0, -22), (3, &mut ec, 1320.0, -90)] {
                let k = enc.encode_float(&tone(freq, n), &mut buf).unwrap();
                m.input(MixIn::Packet { id, rtp: 1000 + n * FRAME as u64, level: Some(level), data: buf[..k].to_vec(), at }, &tx);
            }
            for o in m.step() {
                match o {
                    MixOut::Frame { all: Some(a), own, .. } => {
                        all.push(a);
                        for (id, p) in own {
                            if id == 1 { own_a.push(p) } else if id == 2 { own_b.push(p) }
                        }
                    }
                    MixOut::Speaking { principal, on, .. } => speaking.push((principal.to_string(), on)),
                    _ => {}
                }
            }
        }
        let tail = |v: Vec<f32>| v[v.len() - 48_000..].to_vec();
        let (all, own_a, own_b) = (tail(decode_all(&all)), tail(decode_all(&own_a)), tail(decode_all(&own_b)));
        // the quiet one (−90 dBov: a closed gate, room noise) is not mixed
        assert!(power(&all, 440.0) > 0.05 && power(&all, 880.0) > 0.05, "{} {}", power(&all, 440.0), power(&all, 880.0));
        assert!(power(&all, 1320.0) < 0.01, "{}", power(&all, 1320.0));
        assert!(power(&own_a, 880.0) > 0.05 && power(&own_a, 440.0) < 0.01, "a hears b only");
        assert!(power(&own_b, 440.0) > 0.05 && power(&own_b, 880.0) < 0.01, "b hears a only");
        assert_eq!(m.senders[&3].decoded, 0, "a voice not in the mix is not decoded");
        assert!(m.senders[&3].skipped > 100);
        assert!(speaking.contains(&("a".to_string(), true)) && speaking.contains(&("b".to_string(), true)));
        assert!(!speaking.iter().any(|(p, _)| p == "c"));
    }

    #[test]
    fn at_most_k_voices_and_the_presenter_always_among_them() {
        let mut m = Mixer::new("s".into(), 2).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        for id in 1..=3u64 {
            m.input(MixIn::Join { id, principal: format!("p{id}").into(), name: "x".into(), presenter: false }, &tx);
        }
        m.input(MixIn::Join { id: 9, principal: "teacher".into(), name: "T".into(), presenter: true }, &tx);
        let mut enc = encoder().unwrap();
        let mut buf = vec![0u8; 1500];
        let t0 = Instant::now();
        for n in 0..60u64 {
            let k = enc.encode_float(&tone(500.0, n), &mut buf).unwrap();
            let at = t0 + TICK * n as u32;
            for id in 1..=3u64 {
                m.input(MixIn::Packet { id, rtp: n * FRAME as u64, level: Some(-20), data: buf[..k].to_vec(), at }, &tx);
            }
            // the teacher starts talking later, more quietly than everyone
            if n >= 20 {
                m.input(MixIn::Packet { id: 9, rtp: n * FRAME as u64, level: Some(-40), data: buf[..k].to_vec(), at }, &tx);
            }
            m.step();
            assert!(m.mixed.len() <= 2);
        }
        assert!(m.mixed.contains(&9), "{:?}", m.mixed);
        let unheard = (1..=3u64).filter(|id| !m.mixed.contains(id)).count();
        assert_eq!(unheard, 2);
    }

    #[test]
    fn a_lost_packet_is_concealed_and_late_ones_are_counted() {
        let mut m = Mixer::new("s".into(), 4).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        m.input(MixIn::Join { id: 1, principal: "a".into(), name: "A".into(), presenter: true }, &tx);
        let mut enc = encoder().unwrap();
        let mut buf = vec![0u8; 1500];
        let at = Instant::now();
        for n in 0..40u64 {
            let k = enc.encode_float(&tone(440.0, n), &mut buf).unwrap();
            if n != 20 {
                m.input(MixIn::Packet { id: 1, rtp: 10_000 + n * FRAME as u64, level: Some(-20), data: buf[..k].to_vec(), at: at + TICK * n as u32 }, &tx);
            }
            m.step();
        }
        // a packet long past its tick
        m.input(MixIn::Packet { id: 1, rtp: 10_000, level: Some(-20), data: buf[..10].to_vec(), at: Instant::now() }, &tx);
        let s = &m.senders[&1];
        assert!(s.concealed >= 1, "{}", s.concealed);
        assert_eq!(s.late, 1);
    }
}
