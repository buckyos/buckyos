//! §9.6 of NewOpenDANRuntime — per-session kevent subscription pump.
//!
//! Bridges the kevent bus into per-session `Inbound::Event` deliveries:
//!
//! ```text
//!   AgentSession::subscribe_event(pattern)
//!       └── flushes meta + nudges AIAgent → push_session_subscriptions(...)
//!                                              │
//!                                              ▼
//!   SessionEventPump  (this module)
//!       ├── aggregates union of all session patterns
//!       ├── (re)creates a single EventReader on change / reader loss
//!       └── on pull_event hit: matches event.eventid against each session's
//!           own patterns, sends Inbound::Event { target_session_id, ... }
//!           into AIAgent::inbox()
//! ```
//!
//! Why one reader, not one-per-session: kevent readers maintain an internal
//! queue. Many idle UI sessions sharing the union would over-allocate.
//! A single reader plus an in-memory per-session match table is cheaper and
//! avoids cross-reader race conditions when the same event matches multiple
//! patterns.
//!
//! The pump is the *route* layer only — subscriptions are owned by sessions
//! (persisted in `SessionMeta.event_subscriptions`) and replayed across
//! restarts via `AIAgent::restore_active_sessions`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use buckyos_api::{match_event_patterns, EventReader, KEventClient, KEventError};
use log::{debug, info, warn};
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::sleep;

use crate::agent::Inbound;
use crate::session_model::UI_CLOCK_TIMER_EVENT_ID;

/// Same as `msg_center_pump::EVENT_PULL_TIMEOUT_MS` — short enough that
/// shutdown / subscription refresh latency stays low; long enough to avoid
/// pointless RPC churn.
const EVENT_PULL_TIMEOUT_MS: u64 = 1_000;

pub struct SessionEventPump {
    agent_name: String,
    kevent_client: Arc<KEventClient>,
    inbox_tx: mpsc::Sender<Inbound>,
    /// Snapshot of each live session's current pattern list. Keyed by
    /// session id. Updated through `set_session_subscriptions` /
    /// `remove_session`; reader rebuilds happen on `refresh`.
    subscriptions: Mutex<HashMap<String, Vec<String>>>,
    /// Notified by AIAgent whenever the subscription map changes — the
    /// pump uses this to rebuild its EventReader.
    refresh: Arc<Notify>,
    shutdown: Arc<Notify>,
}

impl SessionEventPump {
    pub fn new(
        agent_name: String,
        kevent_client: Arc<KEventClient>,
        inbox_tx: mpsc::Sender<Inbound>,
        shutdown: Arc<Notify>,
    ) -> Arc<Self> {
        Arc::new(Self {
            agent_name,
            kevent_client,
            inbox_tx,
            subscriptions: Mutex::new(HashMap::new()),
            refresh: Arc::new(Notify::new()),
            shutdown,
        })
    }

    /// Replace the pattern list for one session. Empty list ⇒ effectively
    /// the same as `remove_session`. Wakes the reader thread so the union
    /// is rebuilt promptly.
    pub async fn set_session_subscriptions(&self, session_id: &str, patterns: Vec<String>) {
        let mut guard = self.subscriptions.lock().await;
        if patterns.is_empty() {
            guard.remove(session_id);
        } else {
            guard.insert(session_id.to_string(), patterns);
        }
        drop(guard);
        self.refresh.notify_waiters();
    }

    /// Forget a session entirely. Called when a session ends or is dropped.
    pub async fn remove_session(&self, session_id: &str) {
        let mut guard = self.subscriptions.lock().await;
        if guard.remove(session_id).is_some() {
            drop(guard);
            self.refresh.notify_waiters();
        }
    }

    /// Run until `shutdown` fires or the inbox receiver is dropped. Errors
    /// are logged and the loop continues — losing event delivery silently
    /// would be worse than retry-on-warn.
    pub async fn run(self: Arc<Self>) {
        info!("opendan.event_pump[{}]: starting", self.agent_name);
        let mut reader: Option<Arc<EventReader>> = None;
        let mut last_union: Vec<String> = Vec::new();

        loop {
            if self.inbox_tx.is_closed() {
                info!(
                    "opendan.event_pump[{}]: inbox closed, exiting",
                    self.agent_name
                );
                return;
            }

            // (Re)build the reader when the pattern union shifts. We diff
            // the sorted vec so reorderings inside a session don't force a
            // pointless rebuild.
            let current_union = self.union_patterns().await;
            let needs_rebuild = current_union != last_union || reader.is_none();
            if needs_rebuild {
                // Drop the previous reader first to release its registration
                // before creating a new one with the new pattern set.
                if let Some(prev) = reader.take() {
                    let _ = prev.close().await;
                }
                if !current_union.is_empty() {
                    match self
                        .kevent_client
                        .create_event_reader(current_union.clone())
                        .await
                    {
                        Ok(r) => {
                            info!(
                                "opendan.event_pump[{}]: reader reader_id={} patterns={:?}",
                                self.agent_name,
                                r.reader_id(),
                                current_union
                            );
                            reader = Some(Arc::new(r));
                        }
                        Err(err) => {
                            warn!(
                                "opendan.event_pump[{}]: create_event_reader failed: {err:?}",
                                self.agent_name
                            );
                            // fall through — we'll retry next cycle
                        }
                    }
                }
                last_union = current_union;
            }

            if let Some(r) = reader.as_ref().cloned() {
                tokio::select! {
                    _ = self.shutdown.notified() => {
                        info!("opendan.event_pump[{}]: shutdown received", self.agent_name);
                        let _ = r.close().await;
                        return;
                    }
                    _ = self.refresh.notified() => {
                        // Subscription changed — loop back to rebuild.
                        continue;
                    }
                    res = r.pull_event(Some(EVENT_PULL_TIMEOUT_MS)) => match res {
                        Ok(Some(event)) => {
                            self.route_event(event).await;
                        }
                        Ok(None) => {
                            // timeout — nothing to do, just loop.
                        }
                        Err(KEventError::ReaderClosed(_)) => {
                            warn!(
                                "opendan.event_pump[{}]: reader closed unexpectedly — recreating",
                                self.agent_name
                            );
                            reader = None;
                            last_union.clear(); // force rebuild on next iter
                        }
                        Err(err) => {
                            warn!(
                                "opendan.event_pump[{}]: pull_event error: {err:?}",
                                self.agent_name
                            );
                        }
                    }
                }
            } else {
                // No active subscriptions — just wait for one to be added
                // or for shutdown.
                tokio::select! {
                    _ = self.shutdown.notified() => {
                        info!("opendan.event_pump[{}]: shutdown received (idle)", self.agent_name);
                        return;
                    }
                    _ = self.refresh.notified() => {}
                    _ = sleep(Duration::from_millis(EVENT_PULL_TIMEOUT_MS * 5)) => {
                        // Periodic wake even without notify, so a missed
                        // refresh doesn't strand a freshly-subscribed
                        // session (defense in depth — Notify is one-shot,
                        // a tight subscription churn could swallow the
                        // signal between waiter snapshots).
                    }
                }
            }
        }
    }

    /// Sorted union of all currently-subscribed patterns across sessions.
    /// Sorted so cheap equality vs. `last_union` actually detects "same
    /// set, different insertion order".
    async fn union_patterns(&self) -> Vec<String> {
        let guard = self.subscriptions.lock().await;
        let mut acc: Vec<String> = guard
            .values()
            .flat_map(|patterns| patterns.iter().cloned())
            .collect();
        acc.sort();
        acc.dedup();
        acc
    }

    /// For each session whose pattern list matches `event.eventid`, push
    /// an `Inbound::Event` targeted at that session. One event can fan out
    /// to multiple sessions if their patterns overlap.
    async fn route_event(&self, event: buckyos_api::Event) {
        let snapshot: Vec<(String, Vec<String>)> = {
            let guard = self.subscriptions.lock().await;
            guard
                .iter()
                .map(|(sid, patterns)| (sid.clone(), patterns.clone()))
                .collect()
        };
        let mut delivered = 0usize;
        for (sid, patterns) in snapshot {
            if !match_event_patterns(&patterns, &event.eventid) {
                continue;
            }
            let inbound = Inbound::Event {
                event_id: event.eventid.clone(),
                target_session_id: Some(sid.clone()),
                data: event.data.clone(),
            };
            if let Err(err) = self.inbox_tx.send(inbound).await {
                warn!(
                    "opendan.event_pump[{}]: inbox send failed (sid={sid}): {err}",
                    self.agent_name
                );
                // The receiver is gone — no point trying further sessions.
                return;
            }
            delivered += 1;
            if event.eventid == UI_CLOCK_TIMER_EVENT_ID {
                info!(
                    "opendan.event_pump[{}]: routed ui clock timer event_id={} session_id={} patterns={:?}",
                    self.agent_name, event.eventid, sid, patterns
                );
            }
        }
        debug!(
            "opendan.event_pump[{}]: routed event {} to {delivered} session(s)",
            self.agent_name, event.eventid
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tokio::sync::mpsc;

    fn dummy_client() -> Arc<KEventClient> {
        Arc::new(KEventClient::new_local("test-event-pump"))
    }

    #[tokio::test]
    async fn union_patterns_dedups_and_sorts() {
        let (tx, _rx) = mpsc::channel(1);
        let pump = SessionEventPump::new(
            "test".to_string(),
            dummy_client(),
            tx,
            Arc::new(Notify::new()),
        );
        pump.set_session_subscriptions("a", vec!["/y".into(), "/x".into()])
            .await;
        pump.set_session_subscriptions("b", vec!["/x".into(), "/z".into()])
            .await;
        let union = pump.union_patterns().await;
        // Local kevent ids/patterns don't start with '/' but
        // match_event_patterns + the pump only cares about lexical
        // dedup + sort here, so the test is still valid.
        assert_eq!(union, vec!["/x".to_string(), "/y".into(), "/z".into()]);
    }

    #[tokio::test]
    async fn remove_session_drops_patterns() {
        let (tx, _rx) = mpsc::channel(1);
        let pump = SessionEventPump::new(
            "test".to_string(),
            dummy_client(),
            tx,
            Arc::new(Notify::new()),
        );
        pump.set_session_subscriptions("a", vec!["/x".into()]).await;
        pump.set_session_subscriptions("b", vec!["/y".into()]).await;
        pump.remove_session("a").await;
        let union = pump.union_patterns().await;
        assert_eq!(union, vec!["/y".to_string()]);
    }

    #[tokio::test]
    async fn route_event_targets_matching_sessions() {
        let (tx, mut rx) = mpsc::channel(8);
        let pump = SessionEventPump::new(
            "test".to_string(),
            dummy_client(),
            tx,
            Arc::new(Notify::new()),
        );
        pump.set_session_subscriptions("ui-1", vec!["/timer/**".into()])
            .await;
        pump.set_session_subscriptions("ui-2", vec!["/contact/**".into()])
            .await;

        let event = buckyos_api::Event {
            eventid: "/timer/wake".to_string(),
            source_node: "test".to_string(),
            source_pid: 0,
            ingress_node: None,
            timestamp: 0,
            data: Value::Null,
        };
        pump.route_event(event).await;

        let received = rx.recv().await.expect("Inbound::Event");
        match received {
            Inbound::Event {
                target_session_id,
                event_id,
                ..
            } => {
                assert_eq!(target_session_id.as_deref(), Some("ui-1"));
                assert_eq!(event_id, "/timer/wake");
            }
            other => panic!("expected Inbound::Event, got {:?}", other),
        }
        // ui-2 should not have received anything.
        assert!(rx.try_recv().is_err());
    }
}
