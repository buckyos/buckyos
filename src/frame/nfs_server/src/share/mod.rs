//! ShareContentMgr inside nfs-server (notepads/share-content-mgr-tech-design-todo.md):
//! snapshot shares of files/folders, `share.<zone>/s/{token}` links, app
//! handoffs (MessageHub, HomeStation), audit, access statistics and the Public
//! (`srv/publish`) projection.
//!
//! - `db`: share tables in filedb; `snapshot`: source → immutable named object,
//!   retained while the share is active by key `<share_id>` of the bucket
//!   `zone/nfs-server/_/shares` (nfs-server hosts it, so in process).
//! - `rpc`: NFSP extension methods `share.*` (same envelope/session as files).
//! - `serving`: the visitor data plane and page; `public`: Public observations.
//!
//! Link tokens are `HMAC(secret, cap_id)`: filedb keeps only their sha256 in
//! `grants`, and the server can show the link to its owner again.

pub mod db;
pub mod public;
pub mod rpc;
pub mod serving;
pub mod snapshot;

use crate::config::{NamedStoreSource, ServerConfig};
use crate::error::*;
use crate::filedb::FileDb;
use crate::fsutil::unix_now;
use crate::state::SharedState;
use base64::Engine;
use bucket_store::{BucketError, BucketStore, Caller, Conditions, ListRequest, PutRequest};
use db::{AccessEvent, AuditEntry};
use hmac::{Hmac, Mac};
use named_store::NamedDataMgr;
use sha2::{Digest, Sha256};
use snapshot::Progress;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type HmacSha256 = Hmac<Sha256>;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
const PBKDF2_ROUNDS: u32 = 600_000;
/// Password unlock cookies live at most this long (design §3.5).
pub const UNLOCK_TTL_SECS: i64 = 12 * 3600;
pub const UNLOCK_PER_MINUTE: u32 = 10;
pub const ACCESS_LOG_DAYS: i64 = 90;

pub struct ShareMgr {
    store: tokio::sync::OnceCell<NamedDataMgr>,
    secret: [u8; 32],
    /// Per process: a restart asks visitors for the password again.
    cookie_key: [u8; 32],
    events: Mutex<Vec<AccessEvent>>,
    unlock_attempts: Mutex<HashMap<String, (i64, u32)>>,
    running: Mutex<HashMap<String, Arc<Progress>>>,
    public_scan: Mutex<Option<(i64, bool)>>,
}

impl ShareMgr {
    pub fn new(db: &FileDb) -> NfsResult<ShareMgr> {
        let secret = match db.kv_get("share_token_secret")? {
            Some(hexed) => {
                let raw = hex::decode(hexed).map_err(|_| internal("bad share_token_secret"))?;
                raw.try_into().map_err(|_| internal("bad share_token_secret length"))?
            }
            None => {
                let fresh: [u8; 32] = rand::random();
                db.kv_set("share_token_secret", &hex::encode(fresh))?;
                fresh
            }
        };
        Ok(ShareMgr {
            store: tokio::sync::OnceCell::new(),
            secret,
            cookie_key: rand::random(),
            events: Mutex::new(Vec::new()),
            unlock_attempts: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            public_scan: Mutex::new(None),
        })
    }

    pub fn store_configured(config: &ServerConfig) -> bool {
        !matches!(config.named_store, NamedStoreSource::None)
    }

    pub async fn store(&self, config: &ServerConfig) -> NfsResult<&NamedDataMgr> {
        let unavailable = |msg: String| {
            NfsError::new(ErrorCode::Unsupported, msg).with("reason", serde_json::json!("named_store_unavailable"))
        };
        self.store
            .get_or_try_init(|| async {
                match &config.named_store {
                    NamedStoreSource::None => Err(unavailable("this server has no named store".into())),
                    NamedStoreSource::Runtime => buckyos_api::get_buckyos_api_runtime()
                        .map_err(|e| unavailable(e.to_string()))?
                        .get_local_named_store()
                        .await
                        .map_err(|e| unavailable(e.to_string())),
                    NamedStoreSource::Dir(dir) => open_local_store(dir).await.map_err(unavailable),
                }
            })
            .await
    }

    // ---------- link tokens ----------

    fn mac(&self, key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
        for p in parts {
            mac.update(p);
            mac.update(&[0]);
        }
        mac.finalize().into_bytes().into()
    }

    pub fn token_for(&self, cap_id: &str) -> String {
        B64.encode(self.mac(&self.secret, &[b"share-token", cap_id.as_bytes()]))
    }

    pub fn token_hash(token: &str) -> String {
        hex::encode(Sha256::digest(token.as_bytes()))
    }

    /// Per-day salted visitor id; no raw address is stored.
    pub fn visitor_id(&self, ip: &str, ua: &str, at: i64) -> String {
        let day = (at.div_euclid(86400)).to_string();
        hex::encode(&self.mac(&self.secret, &[b"visitor", day.as_bytes(), ip.as_bytes(), ua.as_bytes()])[..8])
    }

    // ---------- passwords ----------

    pub fn hash_password(password: &str) -> String {
        let salt: [u8; 16] = rand::random();
        let mut out = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ROUNDS, &mut out);
        format!("$pbkdf2-sha256$i={},l=32${}${}", PBKDF2_ROUNDS, B64.encode(salt), B64.encode(out))
    }

    pub fn verify_password(password: &str, phc: &str) -> bool {
        let parts: Vec<&str> = phc.split('$').collect();
        let [_, "pbkdf2-sha256", params, salt, hash] = parts.as_slice() else { return false };
        let Some(rounds) = params.split(',').find_map(|p| p.strip_prefix("i=")).and_then(|v| v.parse::<u32>().ok()) else {
            return false;
        };
        let (Ok(salt), Ok(hash)) = (B64.decode(salt), B64.decode(hash)) else { return false };
        let mut out = vec![0u8; hash.len()];
        pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, rounds, &mut out);
        constant_eq(&out, &hash)
    }

    // ---------- unlock cookies ----------

    pub fn sign_unlock(&self, share_id: &str, exp: i64, password_rev: i64) -> String {
        let payload = format!("{share_id}.{exp}.{password_rev}");
        let sig = self.mac(&self.cookie_key, &[b"share-unlock", payload.as_bytes()]);
        format!("{}.{}", B64.encode(payload.as_bytes()), B64.encode(sig))
    }

    /// True when the cookie unlocks `share_id` at `password_rev` right now.
    pub fn verify_unlock(&self, cookie: &str, share_id: &str, password_rev: i64) -> bool {
        let Some((payload, sig)) = cookie.split_once('.') else { return false };
        let (Ok(payload), Ok(sig)) = (B64.decode(payload), B64.decode(sig)) else { return false };
        if !constant_eq(&self.mac(&self.cookie_key, &[b"share-unlock", &payload]), &sig) {
            return false;
        }
        let Ok(payload) = String::from_utf8(payload) else { return false };
        let mut it = payload.rsplitn(3, '.');
        let (Some(rev), Some(exp), Some(id)) = (it.next(), it.next(), it.next()) else { return false };
        id == share_id && rev.parse::<i64>().ok() == Some(password_rev) && exp.parse::<i64>().is_ok_and(|e| e > unix_now())
    }

    /// Unlock attempts per token per minute (design §3.5: 10).
    pub fn unlock_allowed(&self, token_hash: &str) -> bool {
        let minute = unix_now() / 60;
        let mut map = self.unlock_attempts.lock().unwrap();
        if map.len() > 10_000 {
            map.retain(|_, (m, _)| *m == minute);
        }
        let slot = map.entry(token_hash.to_string()).or_insert((minute, 0));
        if slot.0 != minute {
            *slot = (minute, 0);
        }
        slot.1 += 1;
        slot.1 <= UNLOCK_PER_MINUTE
    }

    // ---------- running operations ----------

    pub fn progress_for(&self, operation_id: &str) -> Arc<Progress> {
        self.running.lock().unwrap().entry(operation_id.to_string()).or_default().clone()
    }

    pub fn running(&self, operation_id: &str) -> Option<Arc<Progress>> {
        self.running.lock().unwrap().get(operation_id).cloned()
    }

    pub fn finish_running(&self, operation_id: &str) {
        self.running.lock().unwrap().remove(operation_id);
    }

    // ---------- statistics ----------

    pub fn record(&self, event: AccessEvent) {
        let mut events = self.events.lock().unwrap();
        events.push(event);
    }

    fn take_events(&self, at_least: usize) -> Vec<AccessEvent> {
        let mut events = self.events.lock().unwrap();
        if events.len() < at_least {
            return Vec::new();
        }
        std::mem::take(&mut *events)
    }

    pub fn flush_stats(&self, db: &FileDb) {
        let events = self.take_events(1);
        if let Err(e) = db.share_access_flush(&events) {
            log::warn!("share stats flush failed ({} events dropped): {}", events.len(), e);
        }
    }

    pub fn public_scan_state(&self) -> Option<(i64, bool)> {
        *self.public_scan.lock().unwrap()
    }

    pub fn set_public_scan_state(&self, at: i64, ok: bool) {
        *self.public_scan.lock().unwrap() = Some((at, ok));
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn open_local_store(dir: &std::path::Path) -> Result<NamedDataMgr, String> {
    use named_store::{NamedLocalStore, StoreLayout, StoreTarget};
    let store = NamedLocalStore::get_named_store_by_path(dir.to_path_buf()).await.map_err(|e| e.to_string())?;
    let store_id = store.store_id().to_string();
    let mgr = NamedDataMgr::new();
    mgr.register_store(Arc::new(tokio::sync::Mutex::new(store))).await;
    mgr.add_layout(StoreLayout::new(
        1,
        vec![StoreTarget {
            store_id,
            device_did: String::new(),
            capacity: None,
            used: None,
            readonly: false,
            enabled: true,
            weight: 1,
        }],
        0,
        0,
    ))
    .await;
    Ok(mgr)
}

/// Standalone servers declare the shares bucket themselves; in service mode it
/// comes from `services/nfs-server/spec` (scheduler `add_nfs_server`).
pub fn declare_bucket(config: &mut ServerConfig) {
    let own = config.bucket_specs.entry(buckyos_api::NFS_SERVER_SERVICE_NAME.to_string()).or_default();
    for (name, decl) in buckyos_api::nfs_server_bucket_decls() {
        own.buckets.entry(name).or_insert(decl);
    }
    own.storage_quota_mib.get_or_insert(buckyos_api::NFS_SERVER_STORAGE_QUOTA_MIB);
}

fn bucket_error(e: BucketError) -> NfsError {
    match e {
        BucketError::QuotaExceeded(_) => NfsError::new(ErrorCode::QuotaExceeded, e.to_string()).with("reason", serde_json::json!("quota_exceeded")),
        e => internal(format!("share bucket: {e}")),
    }
}

async fn share_bucket(state: &SharedState) -> NfsResult<&Arc<BucketStore>> {
    crate::bucket::spec::ensure(state, &buckyos_api::nfs_share_bucket()).await.map_err(bucket_error)?;
    state.buckets.store(state).await.map_err(bucket_error)
}

/// Retains a share's snapshot under its share id; success means the bucket holds it.
pub async fn retain(state: &SharedState, share_id: &str, obj_id: &str) -> NfsResult<()> {
    let id = buckyos_api::nfs_share_bucket();
    let req = PutRequest { key: share_id.into(), obj_id: obj_id.into(), conditions: Conditions::create_only(), ..Default::default() };
    let put = share_bucket(state).await?.put(&id, &Caller::system(), req).await.map_err(bucket_error)?;
    log::info!("bucket bind {id} key={share_id} obj_id={obj_id} version={}", put.version);
    Ok(())
}

/// Stops retaining a share's snapshot. The share row is already final; a key
/// left by a failed release is removed by `reconcile_retention`.
pub async fn release(state: &SharedState, share_id: &str) {
    let id = buckyos_api::nfs_share_bucket();
    let deleted = match share_bucket(state).await {
        Ok(store) => store.delete(&id, &Caller::system(), share_id, None, None).await.map_err(bucket_error),
        Err(e) => Err(e),
    };
    match deleted {
        Ok(r) => log::info!("bucket unbind {id} key={share_id} deleted={} version={:?}", r.deleted, r.version),
        Err(e) => log::warn!("release snapshot of share {share_id} failed, left to reconciliation: {e}"),
    }
}

/// The binding retaining a share's snapshot, if any.
pub async fn retention(state: &SharedState, share_id: &str) -> NfsResult<Option<bucket_store::Binding>> {
    match share_bucket(state).await?.get(&buckyos_api::nfs_share_bucket(), &Caller::system(), share_id, None).await {
        Ok(b) => Ok(Some(b)),
        Err(BucketError::NoSuchKey(_)) => Ok(None),
        Err(e) => Err(bucket_error(e)),
    }
}

/// Keys of the shares bucket equal the active shares (bind before the row,
/// release after it): stray keys are deleted unless bound within `grace_ms`
/// (a share still being created), an active share missing its key is bound again.
pub async fn reconcile_retention(state: &SharedState, grace_ms: i64) -> NfsResult<()> {
    let id = buckyos_api::nfs_share_bucket();
    let store = share_bucket(state).await?;
    let mut active: HashMap<String, String> =
        state.db.shares_in_status("active")?.into_iter().map(|s| (s.share_id, s.obj_id)).collect();
    let cutoff = bucket_store::now_ms() - grace_ms;
    let mut req = ListRequest { limit: Some(1000), ..Default::default() };
    loop {
        let page = store.list_objects(&id, &Caller::system(), &req).await.map_err(bucket_error)?;
        for item in page.items {
            if active.remove(&item.key).is_none() && item.created_at <= cutoff {
                release(state, &item.key).await;
            }
        }
        match page.next_start_after {
            Some(next) => req.start_after = Some(next),
            None => break,
        }
    }
    for (share_id, obj_id) in active {
        if let Err(e) = retain(state, &share_id, &obj_id).await {
            log::warn!("active share {share_id} has no retained snapshot: {}", e.message);
        }
    }
    Ok(())
}

/// Marks a due link expired, then releases its snapshot.
async fn expire(state: &SharedState, share: &db::ShareRow) {
    let audit = AuditEntry {
        share_id: &share.share_id,
        action: "expire",
        revision_before: Some(share.revision),
        revision_after: Some(share.revision + 1),
        actor_user_id: "system",
        actor_app_id: None,
        source: "system",
        reason: None,
        outcome: "ok",
        at: unix_now(),
    };
    match state.db.share_set_status(&share.share_id, "expired", None, true, Some(audit)) {
        Ok(()) => release(state, &share.share_id).await,
        Err(e) => log::warn!("expire share {} failed: {}", share.share_id, e),
    }
}

/// Once a minute: expire due links, flush statistics, prune the access log,
/// and every 5 minutes rescan Public. Snapshot retention is reconciled once at start.
pub async fn sweeper_loop(state: SharedState) {
    if ShareMgr::store_configured(&state.config) {
        if let Err(e) = reconcile_retention(&state, 10 * 60 * 1000).await {
            log::warn!("share retention reconciliation: {}", e.message);
        }
    }
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut tick: u64 = 0;
    loop {
        ticker.tick().await;
        state.share.flush_stats(&state.db);
        tick += 1;
        if tick % 12 != 1 {
            continue;
        }
        sweep_once(&state).await;
        if tick % 60 == 1 {
            let st = state.clone();
            let _ = tokio::task::spawn_blocking(move || st.share_public_scan()).await;
        }
    }
}

pub async fn sweep_once(state: &SharedState) {
    let now = unix_now();
    match state.db.shares_expired_active(now) {
        Ok(rows) => {
            for share in rows {
                expire(state, &share).await;
            }
        }
        Err(e) => log::warn!("share sweeper: {}", e),
    }
    if let Err(e) = state.db.share_access_prune(now - ACCESS_LOG_DAYS * 86400) {
        log::warn!("share access log prune: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let phc = ShareMgr::hash_password("hunter2");
        assert!(phc.starts_with("$pbkdf2-sha256$i=600000,l=32$"));
        assert!(ShareMgr::verify_password("hunter2", &phc));
        assert!(!ShareMgr::verify_password("hunter3", &phc));
        assert!(!ShareMgr::verify_password("hunter2", "garbage"));
    }

    #[test]
    fn tokens_and_cookies() {
        let db = FileDb::open_memory().unwrap();
        let mgr = ShareMgr::new(&db).unwrap();
        let again = ShareMgr::new(&db).unwrap();
        assert_eq!(mgr.token_for("cap_1"), again.token_for("cap_1"));
        assert_ne!(mgr.token_for("cap_1"), mgr.token_for("cap_2"));
        assert_eq!(B64.decode(mgr.token_for("cap_1")).unwrap().len(), 32);
        let c = mgr.sign_unlock("sh_1", unix_now() + 60, 2);
        assert!(mgr.verify_unlock(&c, "sh_1", 2));
        assert!(!mgr.verify_unlock(&c, "sh_1", 3));
        assert!(!mgr.verify_unlock(&c, "sh_2", 2));
        assert!(!again.verify_unlock(&c, "sh_1", 2), "cookie keys are per process");
        assert!(!mgr.verify_unlock(&mgr.sign_unlock("sh_1", unix_now() - 1, 2), "sh_1", 2));
        for _ in 0..UNLOCK_PER_MINUTE {
            assert!(mgr.unlock_allowed("t"));
        }
        assert!(!mgr.unlock_allowed("t"));
    }
}
