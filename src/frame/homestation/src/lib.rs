//! HomeStation service: personal publication stream, delivery inbox, reading pipeline,
//! evaluation service and comment network (architecture v0.7). One service serves every user
//! of the zone (`Host`), each with an own `Station`; plus the zone feed list. See README.md.

pub mod api;
pub mod audience;
pub mod auth;
pub mod comments;
pub mod contacts;
pub mod db;
pub mod delivery;
pub mod directory;
pub mod error;
pub mod evaluation;
pub mod feedback;
pub mod host;
pub mod http;
pub mod ingress;
pub mod interact;
pub mod objects;
pub mod projection;
pub mod protocol;
pub mod publish;
pub mod pull;
pub mod resources;
pub mod selection;
pub mod settings;
pub mod sign;
pub mod sources;
pub mod standalone;
pub mod spider;
pub mod stream;
pub mod users;
pub mod zone;

use crate::contacts::CachedContacts;
use crate::db::Db;
use crate::directory::Directory;
use crate::evaluation::ModelClient;
use crate::objects::ChunkStore;
use crate::protocol::HomeRef;
use crate::sign::Signer;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::sync::Notify;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn now_s() -> u64 {
    (now_ms() / 1000) as u64
}

pub fn new_id(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &id[..16])
}

#[derive(Debug, Clone)]
pub struct StationConfig {
    /// The publisher identity this HomeStation serves: one user of the zone.
    pub owner: String,
    pub owner_name: String,
    /// Zone hostname: entries are `cyfs://<zone>/home/<user>/...`.
    pub zone: String,
    /// The user's path segment (zone username).
    pub user: String,
    /// Act as a collector (§14): accept and index public submissions from anyone.
    pub collector: bool,
    /// Attach the admission disclosure (`candidate` / `preferred`) to `accepted` (§7.4).
    pub disclose_admission: bool,
    pub reading_window: usize,
    pub admission_batch: usize,
    pub candidate_retention_days: i64,
    pub behavior_retention_days: i64,
    pub prefetch: bool,
    pub max_prefetch_bytes: u64,
    pub pull_interval: Duration,
    pub delivery_interval: Duration,
    pub select_interval: Duration,
    pub comment_sync_interval: Duration,
    /// Directly reachable web for the Spider (off in tests that must stay offline).
    pub spider_enabled: bool,
}

impl StationConfig {
    pub fn new(owner: &str, owner_name: &str, zone: &str, user: &str) -> Self {
        Self {
            owner: owner.to_string(),
            owner_name: owner_name.to_string(),
            zone: zone.to_ascii_lowercase(),
            user: user.to_string(),
            collector: false,
            disclose_admission: true,
            reading_window: 300,
            admission_batch: 12,
            candidate_retention_days: 14,
            behavior_retention_days: 7,
            prefetch: true,
            max_prefetch_bytes: 32 * 1024 * 1024,
            pull_interval: Duration::from_secs(300),
            delivery_interval: Duration::from_secs(30),
            select_interval: Duration::from_secs(60),
            comment_sync_interval: Duration::from_secs(600),
            spider_enabled: true,
        }
    }
}

#[derive(Default)]
pub struct Wakers {
    pub delivery: Notify,
    pub pull: Notify,
    pub select: Notify,
    pub comments: Notify,
}

pub struct Station {
    pub cfg: StationConfig,
    pub db: Db,
    pub signer: Signer,
    pub directory: Arc<dyn Directory>,
    pub contacts: CachedContacts,
    pub chunks: Arc<dyn ChunkStore>,
    pub http: reqwest::Client,
    pub model: Option<Arc<dyn ModelClient>>,
    pub wake: Wakers,
    versions: Mutex<BTreeMap<&'static str, u64>>,
    /// The zone-level service holding this station (zone feed, the other users).
    host: OnceLock<Weak<crate::host::Host>>,
}

pub const DOMAINS: &[&str] = &["reading", "candidates", "published", "comments", "saved", "sources", "prefs", "profile", "evaluation"];

impl Station {
    pub fn new(
        cfg: StationConfig,
        db: Db,
        signer: Signer,
        directory: Arc<dyn Directory>,
        contacts: CachedContacts,
        chunks: Arc<dyn ChunkStore>,
        model: Option<Arc<dyn ModelClient>>,
    ) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("BuckyOS-HomeStation/0.7")
            .build()
            .expect("http client");
        Arc::new(Self {
            cfg,
            db,
            signer,
            directory,
            contacts,
            chunks,
            http,
            model,
            wake: Wakers::default(),
            versions: Mutex::new(DOMAINS.iter().map(|d| (*d, 0)).collect()),
            host: OnceLock::new(),
        })
    }

    pub fn owner(&self) -> &str {
        &self.cfg.owner
    }

    /// This user's home: `cyfs://<zone>/home/<user>`.
    pub fn home(&self) -> HomeRef {
        HomeRef::new(&self.cfg.zone, &self.cfg.user)
    }

    pub fn attach_host(&self, host: Weak<crate::host::Host>) {
        let _ = self.host.set(host);
    }

    pub fn host(&self) -> Option<Arc<crate::host::Host>> {
        self.host.get().and_then(Weak::upgrade)
    }

    /// Change counters the UI polls to revalidate its lists.
    pub fn bump(&self, domains: &[&'static str]) {
        let mut versions = self.versions.lock().unwrap();
        for domain in domains {
            *versions.entry(domain).or_insert(0) += 1;
        }
    }

    pub fn versions(&self) -> BTreeMap<&'static str, u64> {
        self.versions.lock().unwrap().clone()
    }

    /// Background loops: delivery retries, follow sync, reading pipeline, comment tracking.
    pub fn start_workers(self: &Arc<Self>) {
        let station = self.clone();
        tokio::spawn(async move { station.delivery_loop().await });
        let station = self.clone();
        tokio::spawn(async move { station.pull_loop().await });
        let station = self.clone();
        tokio::spawn(async move { station.selection_loop().await });
        let station = self.clone();
        tokio::spawn(async move { station.comment_sync_loop().await });
        let station = self.clone();
        tokio::spawn(async move { station.housekeeping_loop().await });
    }

    async fn housekeeping_loop(self: Arc<Self>) {
        let mut first = true;
        loop {
            if !first {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            first = false;
            if let Err(e) = self.purge_behavior().await {
                log::warn!("behavior purge failed: {e}");
            }
            if let Err(e) = self.expire_candidates().await {
                log::warn!("candidate expiry failed: {e}");
            }
            if let Err(e) = self.sync_friends().await {
                log::warn!("friend sync failed: {e}");
            }
        }
    }
}

/// Stable display hue for an identity.
pub fn hue_of(did: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for b in did.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash % 360
}
