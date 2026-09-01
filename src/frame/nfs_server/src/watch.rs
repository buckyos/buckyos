//! Watch plane (NFSP §5.6) and Dir revision tokens (nfs_server.md §3.4).
//!
//! Dir revisions are lazily-created in-memory counters, encoded with a process
//! epoch so tokens never survive a restart looking valid. They are opaque
//! equality tokens only — documented deviation from I3. View/Collection/Group
//! revisions come from filedb generations and are encoded as "g-<n>".

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::broadcast;

/// Canonical container key used for revision tracking and watch-token filtering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContainerKey {
    Root,
    Dir { root: String, rel: String },
    Node(i64),
}

impl ContainerKey {
    pub fn canonical(&self) -> String {
        match self {
            ContainerKey::Root => "root".to_string(),
            ContainerKey::Dir { root, rel } => format!("dir:{}\u{0}{}", root, rel),
            ContainerKey::Node(id) => format!("node:{}", id),
        }
    }
}

pub struct RevisionMgr {
    epoch: u64,
    counters: Mutex<HashMap<String, u64>>,
}

impl RevisionMgr {
    pub fn new() -> Self {
        RevisionMgr {
            epoch: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            counters: Mutex::new(HashMap::new()),
        }
    }

    pub fn current(&self, key: &ContainerKey) -> String {
        if matches!(key, ContainerKey::Root) {
            return format!("root-{}", self.epoch);
        }
        let mut map = self.counters.lock().unwrap();
        let c = map.entry(key.canonical()).or_insert(1);
        format!("d-{}-{}", self.epoch, c)
    }

    pub fn bump(&self, key: &ContainerKey) -> String {
        let mut map = self.counters.lock().unwrap();
        let c = map.entry(key.canonical()).or_insert(1);
        *c += 1;
        format!("d-{}-{}", self.epoch, c)
    }
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub server_rev: u64,
    /// SSE event name: container_changed | meta_changed | lease_recall | resync
    pub event: String,
    pub data: Value,
    /// canonical container key, for watch-token filtering (None = broadcast)
    pub container: Option<String>,
}

pub struct EventBus {
    tx: broadcast::Sender<WatchEvent>,
    server_rev: AtomicU64,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        EventBus { tx, server_rev: AtomicU64::new(1) }
    }

    pub fn server_rev(&self) -> u64 {
        self.server_rev.load(Ordering::SeqCst)
    }

    pub fn next_server_rev(&self) -> u64 {
        self.server_rev.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: &str, container: Option<&ContainerKey>, data: Value) {
        let ev = WatchEvent {
            server_rev: self.next_server_rev(),
            event: event.to_string(),
            data,
            container: container.map(|c| c.canonical()),
        };
        // No receivers is fine.
        let _ = self.tx.send(ev);
    }

    pub fn emit_container_changed(
        &self,
        container: &ContainerKey,
        node_ref: Value,
        kind: &str,
        revision: &str,
        reason: &str,
        hint: Option<Value>,
    ) {
        let mut data = serde_json::json!({
            "ref": node_ref,
            "kind": kind,
            "revision": revision,
            "reason": reason,
        });
        if let Some(h) = hint {
            data["hint"] = h;
        }
        self.emit("container_changed", Some(container), data);
    }

    pub fn emit_resync(&self, reason: &str) {
        self.emit("resync", None, serde_json::json!({ "reason": reason }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_are_equality_tokens() {
        let m = RevisionMgr::new();
        let k = ContainerKey::Dir { root: "home".into(), rel: "a".into() };
        let r1 = m.current(&k);
        assert_eq!(m.current(&k), r1);
        let r2 = m.bump(&k);
        assert_ne!(r1, r2);
        assert_eq!(m.current(&k), r2);
        // Distinct dirs don't interfere; the key uses a NUL separator so
        // ("ho", "me/x") and ("home", "x") can't collide.
        let ka = ContainerKey::Dir { root: "ho".into(), rel: "me/x".into() };
        let kb = ContainerKey::Dir { root: "home".into(), rel: "x".into() };
        assert_ne!(ka.canonical(), kb.canonical());
    }

    #[tokio::test]
    async fn event_bus_delivery() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let key = ContainerKey::Node(7);
        bus.emit_container_changed(
            &key,
            serde_json::json!({"type":"live","node_id":"n_7","gen":1}),
            "collection",
            "g-2",
            "entries_changed",
            None,
        );
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.event, "container_changed");
        assert_eq!(ev.container.as_deref(), Some("node:7"));
        assert_eq!(ev.data["revision"], "g-2");
        assert!(ev.server_rev >= 2);
    }
}
