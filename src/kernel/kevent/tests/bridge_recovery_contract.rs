//! Acceptance contract for the daemon-bridge transport and its self-recovery
//! (github issue #524).
//!
//! Each test drives a real `KEventClient` in `DaemonBridge` mode against a
//! real native TCP server, so "the daemon went away and came back" is
//! exercised end to end rather than mocked.

use buckyos_api::{KEventClient, KEventError, KEventTransportKind, BRIDGE_PULL_TIMEOUT_MS};
use kevent::{handle_native_tcp_connection, KEventService};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

/// A stand-in node-daemon that can be stopped and restarted on a fixed port,
/// the way `systemctl restart node-daemon` looks to a client.
struct TestDaemon {
    service: Arc<KEventService>,
    addr: String,
    server: Option<JoinHandle<()>>,
}

impl TestDaemon {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let service = Arc::new(KEventService::new("test_node"));
        let server = spawn_server(service.clone(), listener);
        Self {
            service,
            addr,
            server: Some(server),
        }
    }

    /// Reserve a port without serving on it, so the first client connect is
    /// refused exactly like a daemon that hasn't started yet.
    async fn reserve_addr() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        drop(listener);
        addr
    }

    async fn start_on(addr: &str) -> Self {
        let listener = TcpListener::bind(addr).await.unwrap();
        let service = Arc::new(KEventService::new("test_node"));
        let server = spawn_server(service.clone(), listener);
        Self {
            service,
            addr: addr.to_string(),
            server: Some(server),
        }
    }

    /// Kill the listener *and* every live connection, which is what a daemon
    /// restart looks like from the client side.
    fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }

    /// Restart on the same address with a fresh service — reader state from
    /// the previous incarnation is gone, as it would be after a real restart.
    async fn restart(&mut self) {
        self.stop();
        // Give the aborted task a moment to release the port.
        for _ in 0..50 {
            match TcpListener::bind(&self.addr).await {
                Ok(listener) => {
                    let service = Arc::new(KEventService::new("test_node"));
                    self.service = service.clone();
                    self.server = Some(spawn_server(service, listener));
                    return;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        panic!("failed to rebind {}", self.addr);
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Same accept loop as node-daemon's, but with the connection tasks held in a
/// `JoinSet` so aborting the server also drops every live connection — that
/// is what a client sees when the daemon process dies.
fn spawn_server(service: Arc<KEventService>, listener: TcpListener) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            let service = service.clone();
            connections.spawn(async move {
                let _ = handle_native_tcp_connection(service, stream).await;
            });
            while connections.try_join_next().is_some() {}
        }
    })
}

/// Poll until `f` holds, so tests don't depend on a fixed reconnect delay
/// (backoff is jittered by design).
async fn wait_until<F, Fut>(label: &str, timeout: Duration, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for: {}", label);
}

/// Publish repeatedly until the reader sees the event: during recovery events
/// are allowed to drop, so a single publish proves nothing about liveness.
async fn expect_event_eventually(
    daemon: &TestDaemon,
    reader: &buckyos_api::EventReader,
    eventid: &str,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        daemon
            .service
            .publish_local_global(eventid, json!({ "ping": true }))
            .await
            .unwrap();
        if let Ok(Some(event)) = reader.pull_event(Some(200)).await {
            assert_eq!(event.eventid, eventid);
            return event.data;
        }
    }
    panic!("event {} never reached the reader", eventid);
}

/// A bridge client with no endpoint must not be constructible. This is the
/// guard against the failure mode that hid the whole problem: a container
/// silently creating a private ring buffer nobody else is attached to.
#[tokio::test]
async fn bridge_without_endpoint_fails_at_construction() {
    assert!(matches!(
        KEventClient::new_daemon_bridge("opendan", ""),
        Err(KEventError::DaemonUnavailable(_))
    ));
    assert!(matches!(
        KEventClient::new_daemon_bridge("opendan", "   "),
        Err(KEventError::DaemonUnavailable(_))
    ));

    let client = KEventClient::new_daemon_bridge("opendan", "127.0.0.1:1").unwrap();
    assert_eq!(client.transport_kind(), KEventTransportKind::DaemonBridge);
}

/// Events published on the daemon reach a bridge client — the path that was
/// never actually working inside containers.
#[tokio::test]
async fn bridge_reader_receives_daemon_published_events() {
    let daemon = TestDaemon::start().await;
    let client = KEventClient::new_daemon_bridge("opendan", &daemon.addr).unwrap();
    let reader = client
        .create_event_reader(vec!["/msg_center/**".to_string()])
        .await
        .unwrap();

    let data = expect_event_eventually(
        &daemon,
        &reader,
        "/msg_center/alice/box_in_alice/changed",
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(data["ping"], json!(true));

    let status = client.transport_status().unwrap();
    assert_eq!(status.live_readers, 1);
    assert_eq!(status.connected_readers, 1);
    assert_eq!(status.consecutive_failures, 0);
}

/// The daemon isn't listening when the reader is created. Creation still
/// succeeds and the reader starts working on its own once the daemon comes up.
#[tokio::test]
async fn reader_created_before_daemon_recovers_by_itself() {
    let addr = TestDaemon::reserve_addr().await;
    let client = KEventClient::new_daemon_bridge("opendan", &addr).unwrap();

    // Creation is optimistic: an unreachable daemon is a runtime condition,
    // not a configuration error.
    let reader = client
        .create_event_reader(vec!["/sys/**".to_string()])
        .await
        .unwrap();

    wait_until("first connect attempt to fail", Duration::from_secs(5), || {
        let client = client.clone();
        async move { client.transport_status().unwrap().consecutive_failures > 0 }
    })
    .await;

    let daemon = TestDaemon::start_on(&addr).await;
    let data = expect_event_eventually(&daemon, &reader, "/sys/node/online", Duration::from_secs(15))
        .await;
    assert_eq!(data["ping"], json!(true));
}

/// Restarting the daemon wipes its reader table. The client must re-register
/// its full pattern set on reconnect without any help from the caller.
#[tokio::test]
async fn daemon_restart_restores_patterns_automatically() {
    let mut daemon = TestDaemon::start().await;
    let client = KEventClient::new_daemon_bridge("opendan", &daemon.addr).unwrap();
    let reader = client
        .create_event_reader(vec!["/sys/**".to_string()])
        .await
        .unwrap();

    expect_event_eventually(&daemon, &reader, "/sys/before", Duration::from_secs(10)).await;

    daemon.restart().await;
    assert_eq!(daemon.service.reader_count().await, 0);

    // No re-subscribe call anywhere: the link reconnects and replays the full
    // registration by itself.
    expect_event_eventually(&daemon, &reader, "/sys/after", Duration::from_secs(20)).await;
    assert_eq!(daemon.service.reader_count().await, 1);
}

/// Patterns edited while the daemon is unreachable are locally authoritative:
/// the call succeeds, and the daemon ends up with the final full set once the
/// link is back.
#[tokio::test]
async fn patterns_changed_while_disconnected_land_after_recovery() {
    let mut daemon = TestDaemon::start().await;
    let client = KEventClient::new_daemon_bridge("opendan", &daemon.addr).unwrap();
    let reader = client
        .create_event_reader(vec!["/sys/**".to_string()])
        .await
        .unwrap();
    expect_event_eventually(&daemon, &reader, "/sys/first", Duration::from_secs(10)).await;

    daemon.stop();
    // A local state change must not fail just because the transport is down.
    reader
        .add_patterns(vec!["/apps/**".to_string()])
        .await
        .unwrap();
    reader
        .remove_patterns(vec!["/sys/**".to_string()])
        .await
        .unwrap();
    assert_eq!(reader.patterns().await, vec!["/apps/**".to_string()]);

    daemon.restart().await;

    // The new pattern is live...
    expect_event_eventually(&daemon, &reader, "/apps/installed", Duration::from_secs(20)).await;
    // ...and the removed one really is gone on the daemon side, i.e. what got
    // replayed was the final full set, not an add/remove history.
    daemon
        .service
        .publish_local_global("/sys/dropped", json!({}))
        .await
        .unwrap();
    assert!(reader.pull_event(Some(300)).await.unwrap().is_none());
}

/// Two clients that both number their readers from 1 must not steal each
/// other's events — the daemon namespaces readers per connection.
#[tokio::test]
async fn concurrent_clients_do_not_collide_on_reader_ids() {
    let daemon = TestDaemon::start().await;
    let client_a = KEventClient::new_daemon_bridge("agent_a", &daemon.addr).unwrap();
    let client_b = KEventClient::new_daemon_bridge("agent_b", &daemon.addr).unwrap();

    let reader_a = client_a
        .create_event_reader(vec!["/agent/a/**".to_string()])
        .await
        .unwrap();
    let reader_b = client_b
        .create_event_reader(vec!["/agent/b/**".to_string()])
        .await
        .unwrap();

    wait_until("both readers registered", Duration::from_secs(10), || {
        let service = daemon.service.clone();
        async move { service.reader_count().await == 2 }
    })
    .await;

    expect_event_eventually(&daemon, &reader_a, "/agent/a/hello", Duration::from_secs(10)).await;
    // B subscribed elsewhere and must stay empty despite A's traffic.
    assert!(reader_b.pull_event(Some(300)).await.unwrap().is_none());

    expect_event_eventually(&daemon, &reader_b, "/agent/b/hello", Duration::from_secs(10)).await;
}

/// While the daemon is down, `pull_event` must still consume the caller's
/// timeout instead of returning an error instantly — otherwise every
/// "kevent is just acceleration" consumer spins at full speed.
#[tokio::test]
async fn pull_event_paces_normally_while_daemon_is_down() {
    let addr = TestDaemon::reserve_addr().await;
    let client = KEventClient::new_daemon_bridge("opendan", &addr).unwrap();
    let reader = client
        .create_event_reader(vec!["/sys/**".to_string()])
        .await
        .unwrap();

    const ROUNDS: u32 = 5;
    const TIMEOUT_MS: u64 = 200;
    let start = Instant::now();
    for _ in 0..ROUNDS {
        // No error leaks to the consumer; the transport keeps its failures
        // to itself and the call simply times out.
        assert!(reader.pull_event(Some(TIMEOUT_MS)).await.unwrap().is_none());
    }
    let elapsed = start.elapsed();

    assert!(
        elapsed >= Duration::from_millis(TIMEOUT_MS * ROUNDS as u64 * 9 / 10),
        "pull loop spun instead of pacing: {:?} for {} rounds of {}ms",
        elapsed,
        ROUNDS,
        TIMEOUT_MS
    );

    // The transport did keep retrying underneath, with bounded backoff.
    let status = client.transport_status().unwrap();
    assert!(status.consecutive_failures > 0);
    assert!(status.last_error.is_some());
}

/// A process that both publishes and subscribes to the same global event must
/// see exactly one copy: over the bridge the daemon is the only loop-back
/// path, so the client must not also dispatch locally.
#[tokio::test]
async fn self_published_global_event_is_delivered_once() {
    let daemon = TestDaemon::start().await;
    let client = KEventClient::new_daemon_bridge("opendan", &daemon.addr).unwrap();
    let reader = client
        .create_event_reader(vec!["/loop/**".to_string()])
        .await
        .unwrap();

    wait_until("reader registered", Duration::from_secs(10), || {
        let service = daemon.service.clone();
        async move { service.reader_count().await == 1 }
    })
    .await;

    client
        .pub_event("/loop/echo", json!({ "seq": 1 }))
        .await
        .unwrap();

    let event = reader.pull_event(Some(3_000)).await.unwrap().unwrap();
    assert_eq!(event.eventid, "/loop/echo");
    assert_eq!(event.data["seq"], json!(1));
    // Exactly one copy — a second delivery would mean both the local dispatch
    // and the daemon loop-back fired.
    assert!(reader.pull_event(Some(300)).await.unwrap().is_none());
}

/// The wire protocol has no request id, so the publisher connection must
/// serialize its round trips; otherwise concurrent publishers read each
/// other's responses.
#[tokio::test]
async fn concurrent_publishes_do_not_cross_responses() {
    let daemon = TestDaemon::start().await;
    let client = KEventClient::new_daemon_bridge("opendan", &daemon.addr).unwrap();
    daemon
        .service
        .register_reader("collector", vec!["/burst/**".to_string()])
        .await
        .unwrap();

    let mut handles = Vec::new();
    for seq in 0..32 {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            client
                .pub_event("/burst/event", json!({ "seq": seq }))
                .await
        }));
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    let mut seen = Vec::new();
    while let Some(event) = daemon
        .service
        .pull_event("collector", Some(500))
        .await
        .unwrap()
    {
        seen.push(event.data["seq"].as_u64().unwrap());
        if seen.len() == 32 {
            break;
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, (0..32).collect::<Vec<u64>>());
}

/// Publishing while the daemon is down drops the event instead of failing the
/// caller's business operation, and closing a reader releases it daemon-side.
#[tokio::test]
async fn publish_is_best_effort_and_close_releases_the_reader() {
    let mut daemon = TestDaemon::start().await;
    let client = KEventClient::new_daemon_bridge("opendan", &daemon.addr).unwrap();
    let reader = client
        .create_event_reader(vec!["/sys/**".to_string()])
        .await
        .unwrap();
    wait_until("reader registered", Duration::from_secs(10), || {
        let service = daemon.service.clone();
        async move { service.reader_count().await == 1 }
    })
    .await;

    // Closing drops the connection; the daemon reclaims the reader without an
    // explicit unregister round trip. The daemon notices the dead peer when
    // its in-flight long poll returns, so reclaim is bounded by the pull
    // timeout rather than immediate — the same bound that applies when a
    // client process is killed outright.
    reader.close().await.unwrap();
    wait_until(
        "reader reclaimed after close",
        Duration::from_millis(BRIDGE_PULL_TIMEOUT_MS * 3),
        || {
            let service = daemon.service.clone();
            async move { service.reader_count().await == 0 }
        },
    )
    .await;

    daemon.stop();
    for _ in 0..5 {
        // Transport outage must not surface as a business failure.
        client
            .pub_event("/sys/while_down", json!({ "ok": true }))
            .await
            .unwrap();
    }
    let status = client.transport_status().unwrap();
    assert!(
        status.publishes_dropped > 0,
        "dropped publishes should be counted, status={:?}",
        status
    );

    // Input errors are still errors, outage or not.
    assert!(matches!(
        client.pub_event("bad/eventid", json!({})).await,
        Err(KEventError::InvalidEventId(_))
    ));
}
