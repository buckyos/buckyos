//! Background work: nfs-server is the Zone's named-store GC host (§14.4: cascade
//! edges, pin expiry, LRU flush, eviction only when enabled), and runs the
//! bucket reconcile (§7.3) and lifecycle (§9) rounds.

use crate::state::SharedState;
use std::time::Duration;

const GC_HOST_EVERY: Duration = Duration::from_secs(30);
const RECONCILE_EVERY: Duration = Duration::from_secs(600);
const DEEP_RECONCILE_EVERY: Duration = Duration::from_secs(24 * 3600);
const LIFECYCLE_EVERY: Duration = Duration::from_secs(3600);

pub async fn maintenance_loop(state: SharedState) {
    let store = loop {
        match state.buckets.store(&state).await {
            Ok(s) => break s.clone(),
            Err(e) => {
                log::warn!("bucket service unavailable, retrying: {e}");
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
    };
    if let Err(e) = store.reconcile(false).await {
        log::warn!("startup bucket reconcile: {e}");
    }
    let mut gc = tokio::time::interval(GC_HOST_EVERY);
    let mut reconcile = tokio::time::interval(RECONCILE_EVERY);
    let mut deep = tokio::time::interval(DEEP_RECONCILE_EVERY);
    let mut lifecycle = tokio::time::interval(LIFECYCLE_EVERY);
    for t in [&mut gc, &mut reconcile, &mut deep, &mut lifecycle] {
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    }
    reconcile.tick().await;
    deep.tick().await;
    loop {
        tokio::select! {
            _ = gc.tick() => {
                match store.ndm().maintenance_round(state.config.gc_evict).await {
                    Ok(r) if r.edges_delivered > 0 || r.pins_expired > 0 || r.gc.freed_bytes > 0 => log::info!("named store maintenance: {r:?}"),
                    Ok(_) => {}
                    Err(e) => log::warn!("named store maintenance: {e}"),
                }
            }
            _ = reconcile.tick() => {
                if let Err(e) = store.reconcile(false).await {
                    log::warn!("bucket reconcile: {e}");
                }
            }
            _ = deep.tick() => {
                if let Err(e) = store.reconcile(true).await {
                    log::warn!("deep bucket reconcile: {e}");
                }
                super::s3::abort_stale_uploads(&state).await;
            }
            _ = lifecycle.tick() => {
                if let Err(e) = store.run_lifecycle(bucket_store::now_ms()).await {
                    log::warn!("bucket lifecycle: {e}");
                }
            }
        }
    }
}
