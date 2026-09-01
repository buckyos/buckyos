//! Native TCP transport to the node-daemon KEvent bridge.
//!
//! Deployment shape: node-daemon is the loader of every container, so it is
//! also the natural home of the bridge server. Processes that cannot reach
//! the host's shared ring buffer (anything running in a container) use this
//! transport as their *only* global-event channel.
//!
//! Connection model (see doc/arch/kevent/kevent.md §5.4):
//!
//! * The wire protocol is strictly ordered request/response with no request
//!   id, so a connection may carry only one in-flight request at a time.
//!   A long-poll `pull_event` would therefore head-of-line block everything
//!   else sharing its connection.
//! * Each `EventReader` gets its own connection: register the full pattern
//!   set, then loop on long-poll pulls. Pattern edits mark the link dirty and
//!   are pushed as a fresh full registration once the in-flight pull returns,
//!   so the pull timeout bounds how long a pattern change takes to land.
//! * `pub_event` is not tied to a reader and uses one lazily-connected
//!   publisher connection, serialized by a mutex so concurrent publishers
//!   cannot mis-pair responses.
//! * Closing a reader just drops its connection: the daemon reclaims every
//!   reader registered on that connection, which doubles as crash cleanup.
//!
//! Recovery is "reconnect, then re-register the full set". Nothing is
//! replayed — KEvent is an allowed-to-drop notification channel and
//! consumers keep their own polling backstop.

use crate::kevent_client::{
    Event, KEventDaemonRequest, KEventDaemonResponse, KEventError, KEventResult,
};
use async_trait::async_trait;
use log::{info, warn};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Upper bound on a single native frame, mirrored from the daemon side.
const MAX_NATIVE_FRAME_SIZE: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Read budget for short request/response round trips (register, publish).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Long-poll budget of a single `pull_event`. This is also the upper bound on
/// how long a pattern edit waits before it reaches the daemon, so keep it
/// small enough to stay unsurprising.
pub const BRIDGE_PULL_TIMEOUT_MS: u64 = 5_000;
/// Extra read budget on top of the long poll before we call the connection dead.
const PULL_READ_SLACK: Duration = Duration::from_secs(5);
/// How many times in a row a live connection may report READER_CLOSED before
/// we treat it as a broken link and fall back to reconnect-with-backoff.
const MAX_READER_CLOSED_RETRIES: u32 = 3;
/// How often an idle link (a reader that currently has no global patterns)
/// re-checks whether it has something to subscribe to again.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const BACKOFF_MIN_MS: u64 = 200;
const BACKOFF_MAX_MS: u64 = 5_000;
/// While a link stays down we log one summary per this interval instead of
/// one line per failed attempt.
const FAILURE_LOG_INTERVAL_MS: u64 = 60_000;

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bounded exponential backoff with jitter: 200ms → 5s.
fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.min(5);
    let capped = BACKOFF_MIN_MS
        .saturating_mul(1_u64 << shift)
        .min(BACKOFF_MAX_MS);
    let half = capped / 2;
    let jitter = if half > 0 {
        rand::random::<u64>() % half
    } else {
        0
    };
    Duration::from_millis(half + jitter)
}

fn transport_err(context: &str, err: impl std::fmt::Display) -> KEventError {
    KEventError::DaemonUnavailable(format!("{}: {}", context, err))
}

/// Observable transport health. Exposed through
/// `KEventClient::transport_status()` so "is the bridge down?" can be
/// answered by querying state instead of grepping logs.
#[derive(Debug, Clone, Default)]
pub struct KEventTransportStatus {
    pub endpoint: String,
    pub publisher_connected: bool,
    pub live_readers: usize,
    pub connected_readers: usize,
    /// Aggregate across all links (publisher + readers): any successful round
    /// trip clears it, so this answers "is the bridge currently broken?"
    /// rather than "which link is broken?".
    pub consecutive_failures: u64,
    pub total_failures: u64,
    /// Connections established, including the first one of each link.
    pub reconnects: u64,
    pub events_received: u64,
    pub publishes_dropped: u64,
    pub last_error: Option<String>,
    pub last_error_at_ms: u64,
    pub last_ok_at_ms: u64,
}

#[derive(Default)]
struct BridgeStats {
    consecutive_failures: AtomicU64,
    total_failures: AtomicU64,
    reconnects: AtomicU64,
    events_received: AtomicU64,
    publishes_dropped: AtomicU64,
    live_readers: AtomicU64,
    connected_readers: AtomicU64,
    publisher_connected: AtomicBool,
    last_error_at_ms: AtomicU64,
    last_ok_at_ms: AtomicU64,
    last_failure_log_ms: AtomicU64,
    last_error: StdMutex<Option<String>>,
}

impl BridgeStats {
    fn record_failure(&self, link: &str, endpoint: &str, err: &KEventError) {
        let streak = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        self.total_failures.fetch_add(1, Ordering::Relaxed);
        let now = now_millis();
        self.last_error_at_ms.store(now, Ordering::Relaxed);
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(err.to_string());
        }

        // First failure of a streak is logged in full; after that at most one
        // summary per interval, so a daemon that stays down for an hour costs
        // ~60 lines instead of one per retry.
        let should_log = if streak == 1 {
            self.last_failure_log_ms.store(now, Ordering::Relaxed);
            true
        } else {
            let last = self.last_failure_log_ms.load(Ordering::Relaxed);
            if now.saturating_sub(last) >= FAILURE_LOG_INTERVAL_MS {
                self.last_failure_log_ms.store(now, Ordering::Relaxed);
                true
            } else {
                false
            }
        };

        if should_log {
            warn!(
                "kevent bridge {} to {} unavailable (failure #{}): {}",
                link, endpoint, streak, err
            );
        }
    }

    fn record_ok(&self, link: &str, endpoint: &str) {
        let previous = self.consecutive_failures.swap(0, Ordering::Relaxed);
        self.last_ok_at_ms.store(now_millis(), Ordering::Relaxed);
        if previous > 0 {
            info!(
                "kevent bridge {} to {} recovered after {} failed attempt(s)",
                link, endpoint, previous
            );
        }
    }

    fn snapshot(&self, endpoint: &str) -> KEventTransportStatus {
        KEventTransportStatus {
            endpoint: endpoint.to_string(),
            publisher_connected: self.publisher_connected.load(Ordering::Relaxed),
            live_readers: self.live_readers.load(Ordering::Relaxed) as usize,
            connected_readers: self.connected_readers.load(Ordering::Relaxed) as usize,
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            total_failures: self.total_failures.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            events_received: self.events_received.load(Ordering::Relaxed),
            publishes_dropped: self.publishes_dropped.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|slot| slot.clone()),
            last_error_at_ms: self.last_error_at_ms.load(Ordering::Relaxed),
            last_ok_at_ms: self.last_ok_at_ms.load(Ordering::Relaxed),
        }
    }
}

/// One TCP connection carrying strictly ordered request/response frames.
struct FramedConnection {
    stream: TcpStream,
}

impl FramedConnection {
    async fn connect(endpoint: &str) -> KEventResult<Self> {
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(endpoint))
            .await
            .map_err(|_| {
                transport_err(
                    "connect kevent daemon",
                    format!("{} timed out after {:?}", endpoint, CONNECT_TIMEOUT),
                )
            })?
            .map_err(|err| transport_err(&format!("connect kevent daemon {}", endpoint), err))?;
        let _ = stream.set_nodelay(true);
        Ok(Self { stream })
    }

    async fn request(
        &mut self,
        req: &KEventDaemonRequest,
        read_timeout: Duration,
    ) -> KEventResult<KEventDaemonResponse> {
        let payload = serde_json::to_vec(req).map_err(|err| {
            KEventError::Internal(format!("encode kevent daemon request failed: {}", err))
        })?;
        self.write_frame(&payload)
            .await
            .map_err(|err| transport_err("write kevent frame", err))?;

        let frame = tokio::time::timeout(read_timeout, self.read_frame())
            .await
            .map_err(|_| {
                transport_err(
                    "read kevent frame",
                    format!("no response within {:?}", read_timeout),
                )
            })?
            .map_err(|err| transport_err("read kevent frame", err))?;

        serde_json::from_slice(&frame).map_err(|err| {
            KEventError::Internal(format!("decode kevent daemon response failed: {}", err))
        })
    }

    async fn write_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        if payload.is_empty() || payload.len() > MAX_NATIVE_FRAME_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid kevent native frame length: {}", payload.len()),
            ));
        }
        self.stream.write_u32(payload.len() as u32).await?;
        self.stream.write_all(payload).await?;
        self.stream.flush().await
    }

    async fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        let frame_len = self.stream.read_u32().await? as usize;
        if frame_len == 0 || frame_len > MAX_NATIVE_FRAME_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid kevent native frame length: {}", frame_len),
            ));
        }
        let mut frame = vec![0_u8; frame_len];
        self.stream.read_exact(&mut frame).await?;
        Ok(frame)
    }
}

fn response_to_result(resp: KEventDaemonResponse) -> KEventResult<Option<Event>> {
    match resp {
        KEventDaemonResponse::Ok { event } => Ok(event),
        KEventDaemonResponse::Err { code, message } => Err(match code.as_str() {
            "INVALID_EVENTID" => KEventError::InvalidEventId(message),
            "INVALID_PATTERN" => KEventError::InvalidPattern(message),
            "DAEMON_UNAVAILABLE" => KEventError::DaemonUnavailable(message),
            "TIMER_INVALID_TARGET" => KEventError::TimerInvalidTarget(message),
            "TIMER_NOT_FOUND" => KEventError::TimerNotFound(message),
            "NOT_SUPPORTED" => KEventError::NotSupported(message),
            "READER_CLOSED" => KEventError::ReaderClosed(message),
            _ => KEventError::Internal(message),
        }),
    }
}

/// Publisher half of the bridge: one lazily-created connection, serialized so
/// only one request is ever in flight (the wire protocol has no request id, so
/// concurrent senders would read each other's responses).
struct PublisherLink {
    endpoint: String,
    conn: Mutex<Option<FramedConnection>>,
    /// Single-flight backoff gate: while a connect is failing we fail fast
    /// instead of opening a socket per publish.
    next_connect_at_ms: AtomicU64,
    attempt: AtomicU32,
    stats: Arc<BridgeStats>,
}

impl PublisherLink {
    fn new(endpoint: String, stats: Arc<BridgeStats>) -> Self {
        Self {
            endpoint,
            conn: Mutex::new(None),
            next_connect_at_ms: AtomicU64::new(0),
            attempt: AtomicU32::new(0),
            stats,
        }
    }

    async fn publish(&self, event: &Event) -> KEventResult<()> {
        let mut guard = self.conn.lock().await;

        // Reuse the live connection first. Once a request has been written,
        // a missing response leaves delivery ambiguous: the daemon may already
        // have distributed the event. Never replay that event on a fresh
        // connection because Event has no id that subscribers can deduplicate.
        if guard.is_some() {
            let conn = guard.as_mut().expect("checked above");
            match conn
                .request(
                    &KEventDaemonRequest::PublishGlobal {
                        event: event.clone(),
                    },
                    REQUEST_TIMEOUT,
                )
                .await
            {
                Ok(resp) => {
                    self.attempt.store(0, Ordering::Relaxed);
                    self.next_connect_at_ms.store(0, Ordering::Relaxed);
                    self.stats.record_ok("publisher", &self.endpoint);
                    return response_to_result(resp).map(|_| ());
                }
                Err(err) if is_transport_error(&err) => {
                    *guard = None;
                    self.stats
                        .publisher_connected
                        .store(false, Ordering::Relaxed);
                    let attempt = self.attempt.fetch_add(1, Ordering::Relaxed);
                    self.next_connect_at_ms.store(
                        now_millis() + backoff_delay(attempt).as_millis() as u64,
                        Ordering::Relaxed,
                    );
                    self.stats.record_failure("publisher", &self.endpoint, &err);
                    return Err(err);
                }
                Err(err) => return Err(err),
            }
        }

        let now = now_millis();
        if now < self.next_connect_at_ms.load(Ordering::Relaxed) {
            return Err(KEventError::DaemonUnavailable(format!(
                "kevent daemon {} unreachable, retrying later",
                self.endpoint
            )));
        }

        let mut conn = match FramedConnection::connect(&self.endpoint).await {
            Ok(conn) => conn,
            Err(err) => {
                let attempt = self.attempt.fetch_add(1, Ordering::Relaxed);
                self.next_connect_at_ms.store(
                    now_millis() + backoff_delay(attempt).as_millis() as u64,
                    Ordering::Relaxed,
                );
                self.stats.record_failure("publisher", &self.endpoint, &err);
                return Err(err);
            }
        };
        self.stats.reconnects.fetch_add(1, Ordering::Relaxed);

        let result = conn
            .request(
                &KEventDaemonRequest::PublishGlobal {
                    event: event.clone(),
                },
                REQUEST_TIMEOUT,
            )
            .await;

        match result {
            Ok(resp) => {
                *guard = Some(conn);
                self.stats
                    .publisher_connected
                    .store(true, Ordering::Relaxed);
                self.attempt.store(0, Ordering::Relaxed);
                self.next_connect_at_ms.store(0, Ordering::Relaxed);
                self.stats.record_ok("publisher", &self.endpoint);
                response_to_result(resp).map(|_| ())
            }
            Err(err) => {
                if is_transport_error(&err) {
                    self.stats
                        .publisher_connected
                        .store(false, Ordering::Relaxed);
                    let attempt = self.attempt.fetch_add(1, Ordering::Relaxed);
                    self.stats.record_failure("publisher", &self.endpoint, &err);
                    self.next_connect_at_ms.store(
                        now_millis() + backoff_delay(attempt).as_millis() as u64,
                        Ordering::Relaxed,
                    );
                }
                Err(err)
            }
        }
    }
}

/// Errors that mean "the channel is down", as opposed to "the daemon rejected
/// this request". Only the former are eligible for best-effort drop + retry.
pub fn is_transport_error(err: &KEventError) -> bool {
    matches!(
        err,
        KEventError::DaemonUnavailable(_) | KEventError::Internal(_)
    )
}

/// What the per-reader pull task needs from the client: the reader's current
/// authoritative pattern set, and somewhere to put pulled events.
#[async_trait]
pub trait BridgeReaderSink: Send + Sync {
    /// Current full global pattern set, or `None` once the reader is gone.
    /// An empty set means "nothing to subscribe right now" — the reader is
    /// still alive and may get global patterns back later.
    async fn global_patterns(&self) -> Option<Vec<String>>;
    async fn deliver(&self, event: Event);
}

/// The bridge as seen by `KEventClient`.
pub struct KEventDaemonBridgeTransport {
    endpoint: String,
    publisher: PublisherLink,
    stats: Arc<BridgeStats>,
}

impl KEventDaemonBridgeTransport {
    pub fn new(endpoint: impl Into<String>) -> KEventResult<Self> {
        let endpoint = endpoint.into().trim().to_string();
        if endpoint.is_empty() {
            return Err(KEventError::DaemonUnavailable(
                "kevent daemon bridge endpoint is empty".to_string(),
            ));
        }
        let stats = Arc::new(BridgeStats::default());
        Ok(Self {
            publisher: PublisherLink::new(endpoint.clone(), stats.clone()),
            endpoint,
            stats,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn publish_global(&self, event: &Event) -> KEventResult<()> {
        self.publisher.publish(event).await
    }

    pub fn note_publish_dropped(&self) {
        self.stats.publishes_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn status(&self) -> KEventTransportStatus {
        self.stats.snapshot(&self.endpoint)
    }

    /// Start the dedicated connection for one reader: register the full
    /// pattern set, then long-poll until the reader is closed.
    pub fn spawn_reader(
        self: &Arc<Self>,
        reader_id: String,
        sink: Arc<dyn BridgeReaderSink>,
    ) -> Arc<KEventBridgeReaderLink> {
        let shared = Arc::new(ReaderLinkShared {
            dirty: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        });

        let transport = self.clone();
        // The task must not hold the public link handle: the handle's Drop is
        // what stops the task, so an ownership cycle would keep both alive
        // forever. It only shares the dirty/stopped flags.
        let task_shared = shared.clone();
        let handle = tokio::spawn(async move {
            run_reader_link(transport, reader_id, sink, task_shared).await;
        });

        Arc::new(KEventBridgeReaderLink {
            shared,
            task: StdMutex::new(Some(handle)),
        })
    }
}

struct ReaderLinkShared {
    dirty: AtomicBool,
    stopped: AtomicBool,
}

/// Handle owned by an `EventReader`. Dropping it closes the reader's
/// connection, which is how the daemon learns to reclaim the reader.
pub struct KEventBridgeReaderLink {
    shared: Arc<ReaderLinkShared>,
    task: StdMutex<Option<JoinHandle<()>>>,
}

impl KEventBridgeReaderLink {
    /// Patterns changed locally; push the new full set after the in-flight
    /// pull returns.
    pub fn mark_dirty(&self) {
        self.shared.dirty.store(true, Ordering::Release);
    }

    pub fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = self.task.lock() {
            if let Some(handle) = slot.take() {
                // Aborting drops the TcpStream; the daemon reclaims every
                // reader of that connection. No need to wait for the
                // in-flight long poll.
                handle.abort();
            }
        }
    }
}

impl Drop for KEventBridgeReaderLink {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Keeps the reader gauges honest even when the task is aborted mid-poll.
struct ReaderGauge {
    stats: Arc<BridgeStats>,
    connected: AtomicBool,
}

impl ReaderGauge {
    fn new(stats: Arc<BridgeStats>) -> Self {
        stats.live_readers.fetch_add(1, Ordering::Relaxed);
        Self {
            stats,
            connected: AtomicBool::new(false),
        }
    }

    fn set_connected(&self, connected: bool) {
        if self.connected.swap(connected, Ordering::Relaxed) == connected {
            return;
        }
        if connected {
            self.stats.connected_readers.fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats.connected_readers.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Drop for ReaderGauge {
    fn drop(&mut self) {
        self.set_connected(false);
        self.stats.live_readers.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn snapshot_global_patterns(
    sink: &Arc<dyn BridgeReaderSink>,
    link: &Arc<ReaderLinkShared>,
) -> Option<Vec<String>> {
    link.dirty.swap(false, Ordering::AcqRel);
    sink.global_patterns().await
}

async fn run_reader_link(
    transport: Arc<KEventDaemonBridgeTransport>,
    reader_id: String,
    sink: Arc<dyn BridgeReaderSink>,
    link: Arc<ReaderLinkShared>,
) {
    let endpoint = transport.endpoint.clone();
    let stats = transport.stats.clone();
    let link_name = format!("reader {}", reader_id);
    let gauge = ReaderGauge::new(stats.clone());
    let mut attempt: u32 = 0;

    loop {
        if link.stopped.load(Ordering::Relaxed) {
            break;
        }

        // Registration is always the complete pattern set — the same call on
        // first connect and on every reconnect, so there is no add/remove
        // history to replay.
        let patterns = match snapshot_global_patterns(&sink, &link).await {
            // Reader gone: nothing left to serve.
            None => break,
            // The reader dropped all its global patterns but is still alive
            // and may get some back. Hold the link open (idle, disconnected)
            // instead of ending the task, which would leave a later
            // `add_patterns` with nothing to wake up.
            Some(patterns) if patterns.is_empty() => {
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            }
            Some(patterns) => patterns,
        };

        let mut conn = match FramedConnection::connect(&endpoint).await {
            Ok(conn) => conn,
            Err(err) => {
                stats.record_failure(&link_name, &endpoint, &err);
                let delay = backoff_delay(attempt);
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
                continue;
            }
        };
        stats.reconnects.fetch_add(1, Ordering::Relaxed);
        match conn
            .request(
                &KEventDaemonRequest::RegisterReader {
                    reader_id: reader_id.clone(),
                    patterns,
                },
                REQUEST_TIMEOUT,
            )
            .await
            .and_then(response_to_result)
        {
            Ok(_) => {
                attempt = 0;
                stats.record_ok(&link_name, &endpoint);
                gauge.set_connected(true);
            }
            Err(err) => {
                stats.record_failure(&link_name, &endpoint, &err);
                let delay = backoff_delay(attempt);
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
                continue;
            }
        }

        let disconnect_reason = pump_reader(&mut conn, &reader_id, &sink, &link, &stats).await;
        gauge.set_connected(false);
        drop(conn);

        match disconnect_reason {
            PumpExit::Stopped => break,
            // Dropped every global pattern: back to the idle branch above,
            // with the connection closed so the daemon reclaims the reader.
            PumpExit::Idle => continue,
            PumpExit::Failed(err) => {
                stats.record_failure(&link_name, &endpoint, &err);
                let delay = backoff_delay(attempt);
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
            }
        }
    }

    drop(gauge);
}

enum PumpExit {
    /// Reader closed or client dropped: end the task.
    Stopped,
    /// No global patterns left; disconnect and wait for new ones.
    Idle,
    /// Transport problem: reconnect after backoff.
    Failed(KEventError),
}

async fn pump_reader(
    conn: &mut FramedConnection,
    reader_id: &str,
    sink: &Arc<dyn BridgeReaderSink>,
    link: &Arc<ReaderLinkShared>,
    stats: &Arc<BridgeStats>,
) -> PumpExit {
    let mut reader_closed_streak = 0_u32;
    loop {
        if link.stopped.load(Ordering::Relaxed) {
            return PumpExit::Stopped;
        }

        // A pattern edit landed while we were polling: resend the full set.
        if link.dirty.swap(false, Ordering::AcqRel) {
            let patterns = match sink.global_patterns().await {
                None => return PumpExit::Stopped,
                Some(patterns) if patterns.is_empty() => return PumpExit::Idle,
                Some(patterns) => patterns,
            };
            if let Err(err) = conn
                .request(
                    &KEventDaemonRequest::RegisterReader {
                        reader_id: reader_id.to_string(),
                        patterns,
                    },
                    REQUEST_TIMEOUT,
                )
                .await
                .and_then(response_to_result)
            {
                link.dirty.store(true, Ordering::Release);
                return PumpExit::Failed(err);
            }
        }

        let pulled = conn
            .request(
                &KEventDaemonRequest::PullEvent {
                    reader_id: reader_id.to_string(),
                    timeout_ms: Some(BRIDGE_PULL_TIMEOUT_MS),
                },
                Duration::from_millis(BRIDGE_PULL_TIMEOUT_MS) + PULL_READ_SLACK,
            )
            .await
            .and_then(response_to_result);

        match pulled {
            Ok(Some(event)) => {
                reader_closed_streak = 0;
                if sink.global_patterns().await.is_none() {
                    return PumpExit::Stopped;
                }
                stats.events_received.fetch_add(1, Ordering::Relaxed);
                sink.deliver(event).await;
            }
            Ok(None) => {
                reader_closed_streak = 0;
                if sink.global_patterns().await.is_none() {
                    return PumpExit::Stopped;
                }
            }
            Err(err @ KEventError::ReaderClosed(_)) => {
                // The connection survived but the daemon lost our reader.
                // Re-register in place instead of tearing the socket down.
                link.dirty.store(true, Ordering::Release);
                reader_closed_streak += 1;
                // Register-then-immediately-closed would otherwise spin at
                // full speed; hand it to the outer loop so it gets backoff.
                if reader_closed_streak > MAX_READER_CLOSED_RETRIES {
                    return PumpExit::Failed(err);
                }
            }
            Err(err) => return PumpExit::Failed(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::Notify;

    #[test]
    fn backoff_is_bounded_and_jittered() {
        for attempt in 0..10_u32 {
            let delay = backoff_delay(attempt).as_millis() as u64;
            assert!(
                delay >= BACKOFF_MIN_MS / 2,
                "attempt {} → {}ms",
                attempt,
                delay
            );
            assert!(delay <= BACKOFF_MAX_MS, "attempt {} → {}ms", attempt, delay);
        }
        // Deep retries must sit at the ceiling band, never grow without bound.
        let deep = backoff_delay(30).as_millis() as u64;
        assert!(deep >= BACKOFF_MAX_MS / 2 && deep <= BACKOFF_MAX_MS);
    }

    #[test]
    fn empty_endpoint_is_rejected_at_construction() {
        assert!(KEventDaemonBridgeTransport::new("   ").is_err());
        assert!(KEventDaemonBridgeTransport::new("127.0.0.1:3183").is_ok());
    }

    #[test]
    fn transport_errors_are_the_retryable_ones() {
        assert!(is_transport_error(&KEventError::DaemonUnavailable(
            "x".into()
        )));
        assert!(!is_transport_error(&KEventError::InvalidEventId(
            "x".into()
        )));
        assert!(!is_transport_error(&KEventError::NotSupported("x".into())));
    }

    #[tokio::test]
    async fn request_failure_advances_publisher_backoff() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let frame_len = stream.read_u32().await.unwrap() as usize;
            let mut frame = vec![0_u8; frame_len];
            stream.read_exact(&mut frame).await.unwrap();
        });

        let publisher = PublisherLink::new(endpoint, Arc::new(BridgeStats::default()));
        let event = Event {
            eventid: "/publisher/backoff".to_string(),
            source_node: "test".to_string(),
            source_pid: std::process::id(),
            ingress_node: Some("test".to_string()),
            timestamp: now_millis(),
            data: json!({}),
        };

        let err = publisher.publish(&event).await.unwrap_err();
        assert!(is_transport_error(&err));
        server.await.unwrap();
        assert_eq!(publisher.attempt.load(Ordering::Relaxed), 1);
        assert!(publisher.next_connect_at_ms.load(Ordering::Relaxed) > now_millis());
    }

    struct BlockingPatternSink {
        entered: Notify,
        release: Notify,
    }

    #[async_trait]
    impl BridgeReaderSink for BlockingPatternSink {
        async fn global_patterns(&self) -> Option<Vec<String>> {
            self.entered.notify_one();
            self.release.notified().await;
            Some(vec!["/patterns/current".to_string()])
        }

        async fn deliver(&self, _event: Event) {}
    }

    #[tokio::test]
    async fn pattern_edit_racing_with_snapshot_remains_dirty() {
        let link = Arc::new(ReaderLinkShared {
            dirty: AtomicBool::new(true),
            stopped: AtomicBool::new(false),
        });
        let sink = Arc::new(BlockingPatternSink {
            entered: Notify::new(),
            release: Notify::new(),
        });
        let task_link = link.clone();
        let task_sink: Arc<dyn BridgeReaderSink> = sink.clone();
        let snapshot =
            tokio::spawn(async move { snapshot_global_patterns(&task_sink, &task_link).await });

        sink.entered.notified().await;
        assert!(!link.dirty.load(Ordering::Acquire));
        link.dirty.store(true, Ordering::Release);
        sink.release.notify_one();

        assert_eq!(
            snapshot.await.unwrap(),
            Some(vec!["/patterns/current".to_string()])
        );
        assert!(link.dirty.load(Ordering::Acquire));
    }
}
