//! §9.4/§9.6 contact-mgr lookup helper.
//!
//! Wraps `MsgCenterClient::get_contact` for two specific use cases:
//!
//!   1. **`from_name` enrichment** — the msg-center pump only sees raw
//!      `DID`s on inbound records; the LLM prompt is far more useful when
//!      it sees the human-readable name. The pump consults `ContactLookup`
//!      to fill the missing field.
//!   2. **forward_msg / forward** tool helpers (lands later) — they need
//!      to translate a name / handle into a DID before calling
//!      `msg_center.post_send`. `lookup_by_name` and `lookup_by_handle`
//!      cover that case; not yet wired but reserved here so the surface is
//!      consistent.
//!
//! Caching: `from_name` lookups are read-heavy and the contact set rarely
//! flips (humans don't rename themselves mid-conversation). We keep a
//! tiny TTL-based in-memory cache so a chatty session doesn't issue a
//! get_contact RPC per inbound message. `None` answers are also cached
//! (negative cache) but for a shorter TTL so a contact added moments after
//! a miss still gets picked up reasonably promptly.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use buckyos_api::{Contact, MsgCenterClient};
use log::debug;
use name_lib::DID;
use tokio::sync::Mutex;

/// Cache TTL for *positive* hits (contact found). Contacts changing names
/// is rare; 5 minutes balances staleness vs. RPC churn.
const POSITIVE_TTL: Duration = Duration::from_secs(300);
/// Cache TTL for *negative* hits (no such contact). Short so a recently
/// imported contact gets picked up within a minute.
const NEGATIVE_TTL: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct CachedEntry {
    name: Option<String>,
    inserted: Instant,
    ttl: Duration,
}

impl CachedEntry {
    fn is_fresh(&self) -> bool {
        self.inserted.elapsed() < self.ttl
    }
}

/// Per-agent contact lookup. Cheap to clone (just `Arc`s + a `Mutex`).
#[derive(Clone)]
pub struct ContactLookup {
    msg_center: Arc<MsgCenterClient>,
    /// Owner DID under which we scope contact-manager queries — the
    /// contact set is per-account on the contact-mgr side.
    owner: Option<DID>,
    cache: Arc<Mutex<HashMap<String, CachedEntry>>>,
}

impl ContactLookup {
    pub fn new(msg_center: Arc<MsgCenterClient>, owner: Option<DID>) -> Self {
        Self {
            msg_center,
            owner,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolve `did` to its display name. Returns `None` when:
    ///   - no such contact exists (negative cached briefly),
    ///   - or the underlying RPC failed (NOT cached — a transient error
    ///     shouldn't poison future lookups).
    pub async fn from_name(&self, did: &DID) -> Option<String> {
        let key = did.to_string();
        if let Some(cached) = self.cache_get(&key).await {
            return cached;
        }
        let result = self
            .msg_center
            .get_contact(did.clone(), self.owner.clone())
            .await;
        match result {
            Ok(Some(contact)) => {
                let name = pick_display_name(&contact);
                self.cache_put(&key, name.clone(), POSITIVE_TTL).await;
                name
            }
            Ok(None) => {
                self.cache_put(&key, None, NEGATIVE_TTL).await;
                None
            }
            Err(err) => {
                debug!("opendan.contact: get_contact({key}) failed (not cached): {err}");
                None
            }
        }
    }

    async fn cache_get(&self, key: &str) -> Option<Option<String>> {
        let guard = self.cache.lock().await;
        let entry = guard.get(key)?;
        if entry.is_fresh() {
            Some(entry.name.clone())
        } else {
            None
        }
    }

    async fn cache_put(&self, key: &str, name: Option<String>, ttl: Duration) {
        let mut guard = self.cache.lock().await;
        guard.insert(
            key.to_string(),
            CachedEntry {
                name,
                inserted: Instant::now(),
                ttl,
            },
        );
    }

    /// Drop all cached entries. Useful when the agent observes a contact
    /// import / change event and wants the next lookup to hit the wire.
    pub async fn invalidate(&self) {
        self.cache.lock().await.clear();
    }
}

fn pick_display_name(contact: &Contact) -> Option<String> {
    let trimmed = contact.name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_freshness_decays() {
        let entry = CachedEntry {
            name: Some("x".to_string()),
            inserted: Instant::now() - Duration::from_secs(120),
            ttl: Duration::from_secs(60),
        };
        assert!(!entry.is_fresh());
    }

    #[test]
    fn entry_freshness_holds() {
        let entry = CachedEntry {
            name: Some("x".to_string()),
            inserted: Instant::now(),
            ttl: Duration::from_secs(60),
        };
        assert!(entry.is_fresh());
    }

    #[test]
    fn pick_display_name_strips_empty() {
        let mut contact = Contact {
            did: DID::new("dev", "alice"),
            name: "  ".to_string(),
            avatar: None,
            note: None,
            source: buckyos_api::ContactSource::ManualCreate,
            is_verified: false,
            bindings: vec![],
            access_level: buckyos_api::AccessGroupLevel::Friend,
            temp_grants: vec![],
            groups: vec![],
            tags: vec![],
            created_at: 0,
            updated_at: 0,
        };
        assert!(pick_display_name(&contact).is_none());
        contact.name = "Alice".to_string();
        assert_eq!(pick_display_name(&contact).as_deref(), Some("Alice"));
    }
}
