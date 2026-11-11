//! Background work: nfs-server is the Zone's named-store GC host (§14.4: cascade
//! edges, pin expiry, LRU flush, eviction only when enabled), and runs the
//! bucket reconcile (§7.3) and lifecycle (§9) rounds.

use crate::state::SharedState;
use std::time::Duration;

const GC_HOST_EVERY: Duration = Duration::from_secs(30);
const RECONCILE_EVERY: Duration = Duration::from_secs(600);
const DEEP_RECONCILE_EVERY: Duration = Duration::from_secs(24 * 3600);
const LIFECYCLE_EVERY: Duration = Duration::from_secs(3600);

/// How long one `bucket.collect_garbage` call may keep evicting.
pub const MANUAL_GC_BUDGET: Duration = Duration::from_secs(30);

/// What a manual collection did; `complete` is false when the time budget ran
/// out with garbage still left (the caller may run it again).
#[derive(Debug, Default, serde::Serialize)]
pub struct GarbageReport {
    pub freed_bytes: u64,
    pub evicted_objects: u64,
    pub evicted_chunks: u64,
    pub pins_expired: u64,
    pub rounds: u32,
    pub complete: bool,
}

/// Collections kept for `bucket.gc_status`.
const GC_LOG_KEEP: usize = 3;

/// One collection that is worth showing: a background round that evicted or
/// expired something, or any manual run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GcRecord {
    pub at: i64,
    /// `background` or `manual`.
    pub trigger: &'static str,
    pub freed_bytes: u64,
    pub evicted_objects: u64,
    pub evicted_chunks: u64,
    pub pins_expired: u64,
}

/// Since this process started: when the background round last ran (most rounds
/// find nothing and leave no record), and the latest records, newest first.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GcLog {
    pub last_round_at: Option<i64>,
    pub rounds: u64,
    /// The latest background round failed with this.
    pub last_error: Option<String>,
    pub recent: std::collections::VecDeque<GcRecord>,
}

impl GcLog {
    fn push(&mut self, record: GcRecord) {
        self.recent.push_front(record);
        self.recent.truncate(GC_LOG_KEEP);
    }

    fn background(&mut self, round: &ndn_lib::NdnResult<named_store::MaintenanceReport>) {
        self.last_round_at = Some(bucket_store::now_ms());
        self.rounds += 1;
        self.last_error = round.as_ref().err().map(|e| e.to_string());
        if let Ok(r) = round {
            if r.gc.evicted_objects + r.gc.evicted_chunks + r.pins_expired > 0 {
                self.push(GcRecord {
                    at: bucket_store::now_ms(),
                    trigger: "background",
                    freed_bytes: r.gc.freed_bytes,
                    evicted_objects: r.gc.evicted_objects,
                    evicted_chunks: r.gc.evicted_chunks,
                    pins_expired: r.pins_expired,
                });
            }
        }
    }

    pub fn manual(&mut self, report: &GarbageReport) {
        self.push(GcRecord {
            at: bucket_store::now_ms(),
            trigger: "manual",
            freed_bytes: report.freed_bytes,
            evicted_objects: report.evicted_objects,
            evicted_chunks: report.evicted_chunks,
            pins_expired: report.pins_expired,
        });
    }
}

/// Manual GC: the periodic round evicts one batch per store every 30 seconds,
/// this repeats eviction rounds until one finds nothing to evict. It evicts even
/// when `named_store_gc_evict` is off (the admin asked for it), under the same
/// protection rules: pins, incoming refs, fs anchors and the 24 h chunk grace.
pub async fn collect_garbage(ndm: &named_store::NamedDataMgr, budget: Duration) -> ndn_lib::NdnResult<GarbageReport> {
    let started = std::time::Instant::now();
    let mut report = GarbageReport::default();
    loop {
        let round = ndm.maintenance_round(true).await?;
        report.rounds += 1;
        report.freed_bytes += round.gc.freed_bytes;
        report.evicted_objects += round.gc.evicted_objects;
        report.evicted_chunks += round.gc.evicted_chunks;
        report.pins_expired += round.pins_expired;
        // A round delivers pending cascade edges before it evicts, so a round
        // that evicts nothing has left nothing evictable behind.
        if round.gc.evicted_objects + round.gc.evicted_chunks == 0 {
            report.complete = true;
            break;
        }
        if started.elapsed() >= budget {
            break;
        }
    }
    log::info!("manual named store gc: {report:?}");
    Ok(report)
}

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
                let round = store.ndm().maintenance_round(state.config.gc_evict).await;
                state.buckets.gc_log.lock().unwrap().background(&round);
                match round {
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
