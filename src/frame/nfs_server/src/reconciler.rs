//! Reconciler (nfs_server.md §2.1/§3.6): the one v1-only component, isolated
//! here — nothing else may depend on its internals; phase two deletes it.
//!
//! v1 uses a portable scan loop instead of platform watchers (inotify/USN/
//! FSEvents are deferred — 待决策 N6), plus the on-access checks that live in
//! namespace.rs. Detection ladder per anchor:
//!
//!   path present, ino matches, (size,mtime) unchanged → untouched
//!   path present, ino matches, (size,mtime) changed  → touched (hash cleared)
//!   path present, ino differs                        → overwrite-rebind
//!   path gone, ino found at exactly one new path     → rename (path follows)
//!   path gone, ino ambiguous or missing              → stale (诚实, 不猜)
//!
//! Bypass changes to directory contents are detected by per-dir fingerprints
//! and surface as container_changed events; mass changes collapse into a
//! single resync (watch is lossy by contract, D11).

use crate::error::NfsResult;
use crate::handle::NativeHandle;
use crate::state::{AppState, SharedState};
use crate::watch::ContainerKey;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Per-dir content fingerprints from the previous scan, keyed by (root, rel).
#[derive(Default)]
pub struct ScanState {
    fingerprints: HashMap<(String, String), u64>,
    baseline_done: bool,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ReconcileReport {
    pub dirs_changed: usize,
    pub renamed: usize,
    pub rebound: usize,
    pub touched: usize,
    pub staled: usize,
    pub resync: bool,
}

/// Dirs changed in one scan beyond this emit a single resync instead of events.
const RESYNC_THRESHOLD: usize = 50;

struct ScannedEntry {
    ino: u64,
    dev: u64,
    is_dir: bool,
    size: u64,
    mtime: i64,
}

impl AppState {
    /// Runs one full reconcile pass over every export root.
    pub fn reconcile_now(&self) -> NfsResult<ReconcileReport> {
        let mut report = ReconcileReport::default();
        let mut new_fps: HashMap<(String, String), u64> = HashMap::new();
        let mut changed_dirs: Vec<(String, String, u64)> = Vec::new(); // root, rel, ino

        for export in &self.config.exports {
            let (entries, fps, dir_inos) = scan_root(&export.path)?;
            self.reconcile_anchors(&export.id, &entries, &mut report)?;
            // Fingerprint diff → bypass changes to dir contents.
            {
                let st = self.scan_state.lock().unwrap();
                for ((rel, fp), ino) in fps.iter().zip(dir_inos.iter()) {
                    let key = (export.id.clone(), rel.clone());
                    if st.baseline_done {
                        match st.fingerprints.get(&key) {
                            Some(old) if old == fp => {}
                            _ => changed_dirs.push((export.id.clone(), rel.clone(), *ino)),
                        }
                    }
                    new_fps.insert(key, *fp);
                }
            }
        }

        report.dirs_changed = changed_dirs.len();
        if changed_dirs.len() > RESYNC_THRESHOLD {
            report.resync = true;
            self.bus.emit_resync("mass_bypass_change");
            // Still bump revisions so cached listings are invalidated by CAS.
            for (root, rel, _) in &changed_dirs {
                self.revisions.bump(&ContainerKey::Dir { root: root.clone(), rel: rel.clone() });
            }
        } else {
            for (root, rel, ino) in &changed_dirs {
                let key = ContainerKey::Dir { root: root.clone(), rel: rel.clone() };
                let revision = self.revisions.bump(&key);
                let handle = NativeHandle {
                    r: root.clone(),
                    p: rel.clone(),
                    i: *ino,
                    k: "d".to_string(),
                };
                self.bus.emit_container_changed(
                    &key,
                    json!(crate::types::WireRef::live(self.handles.encode(&handle), 0)),
                    "dir",
                    &revision,
                    "bypass_change",
                    None,
                );
            }
        }

        let mut st = self.scan_state.lock().unwrap();
        // Keep fingerprints of other roots (only replace scanned keys wholesale).
        st.fingerprints = new_fps;
        st.baseline_done = true;
        Ok(report)
    }

    fn reconcile_anchors(
        &self,
        root_id: &str,
        entries: &HashMap<String, ScannedEntry>,
        report: &mut ReconcileReport,
    ) -> NfsResult<()> {
        // ino → paths (rename lookup).
        let mut by_ino: HashMap<u64, Vec<&String>> = HashMap::new();
        for (rel, e) in entries {
            by_ino.entry(e.ino).or_default().push(rel);
        }
        let mut moved_dir_prefixes: Vec<(String, String)> = Vec::new();
        for anchor in self.db.anchors_for_root(root_id)? {
            // A parent dir already detected as moved will have rewritten this
            // anchor's path via anchors_move_prefix; re-read to stay accurate.
            let anchor = if moved_dir_prefixes
                .iter()
                .any(|(old, _)| anchor.path.starts_with(old.as_str()))
            {
                match self.db.anchor_get(anchor.node_id)? {
                    Some(a) => a,
                    None => continue,
                }
            } else {
                anchor
            };
            match entries.get(&anchor.path) {
                Some(e) if e.ino == anchor.ino => {
                    if anchor.size != Some(e.size as i64) || anchor.mtime != Some(e.mtime) {
                        self.db.anchor_touch(anchor.node_id, e.size as i64, e.mtime, true)?;
                        report.touched += 1;
                    }
                }
                Some(e) => {
                    // Same path, new native id: editor-overwrite heuristic.
                    self.db.anchor_rebind(
                        anchor.node_id,
                        e.ino,
                        e.dev,
                        e.size as i64,
                        e.mtime,
                    )?;
                    report.rebound += 1;
                }
                None => {
                    let candidates: Vec<&&String> = by_ino
                        .get(&anchor.ino)
                        .map(|v| v.iter().collect())
                        .unwrap_or_default();
                    if candidates.len() == 1 && anchor.ino != 0 {
                        let new_rel = candidates[0].as_str().to_string();
                        let was_dir = entries.get(&new_rel).map(|e| e.is_dir).unwrap_or(false);
                        self.db.anchor_update_path(anchor.node_id, root_id, &new_rel)?;
                        if was_dir {
                            // Children anchors follow the moved directory.
                            self.db.anchors_move_prefix(
                                root_id,
                                &anchor.path,
                                root_id,
                                &new_rel,
                            )?;
                            moved_dir_prefixes
                                .push((format!("{}/", anchor.path), new_rel.clone()));
                        }
                        report.renamed += 1;
                    } else {
                        // Ambiguous (hardlinks) or gone: honest stale, never guess.
                        self.db.anchor_set_state(anchor.node_id, "stale")?;
                        report.staled += 1;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Walks one export root without following symlinks. Returns
/// (rel → entry, per-dir (rel, fingerprint) list, matching dir inos).
#[allow(clippy::type_complexity)]
fn scan_root(
    root: &std::path::Path,
) -> NfsResult<(HashMap<String, ScannedEntry>, Vec<(String, u64)>, Vec<u64>)> {
    let mut entries: HashMap<String, ScannedEntry> = HashMap::new();
    let mut dir_hashers: HashMap<String, DefaultHasher> = HashMap::new();
    let mut dir_inos: HashMap<String, u64> = HashMap::new();
    dir_hashers.insert(String::new(), DefaultHasher::new());
    {
        let root_meta = std::fs::symlink_metadata(root)?;
        dir_inos.insert(String::new(), crate::fsutil::file_id(&root_meta).ino);
    }

    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .min_depth(1);
    for item in walker {
        let item = match item {
            Ok(i) => i,
            Err(_) => continue,
        };
        let rel = match item.path().strip_prefix(root) {
            Ok(p) => match p.to_str() {
                Some(s) => s.replace(std::path::MAIN_SEPARATOR, "/"),
                None => continue,
            },
            Err(_) => continue,
        };
        let meta = match item.metadata() {
            Ok(m) => crate::fsutil::node_meta(&m),
            Err(_) => continue,
        };
        let (parent, name) = crate::fsutil::rel_split(&rel).unwrap_or_default();
        if let Some(h) = dir_hashers.get_mut(&parent) {
            name.hash(h);
            meta.id.ino.hash(h);
            meta.size.hash(h);
            meta.mtime.hash(h);
            (meta.kind == "dir").hash(h);
        }
        if meta.kind == "dir" {
            dir_hashers.insert(rel.clone(), DefaultHasher::new());
            dir_inos.insert(rel.clone(), meta.id.ino);
        }
        entries.insert(
            rel,
            ScannedEntry {
                ino: meta.id.ino,
                dev: meta.id.dev,
                is_dir: meta.kind == "dir",
                size: meta.size,
                mtime: meta.mtime,
            },
        );
    }
    let mut fps: Vec<(String, u64)> = dir_hashers
        .into_iter()
        .map(|(rel, h)| (rel, h.finish()))
        .collect();
    fps.sort_by(|a, b| a.0.cmp(&b.0));
    let inos: Vec<u64> = fps.iter().map(|(rel, _)| dir_inos.get(rel).copied().unwrap_or(0)).collect();
    Ok((entries, fps, inos))
}

/// Background loop driven by --scan-interval-secs.
pub async fn reconcile_loop(state: SharedState, interval_secs: u64) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(1)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let st = state.clone();
        let result = tokio::task::spawn_blocking(move || st.reconcile_now()).await;
        match result {
            Ok(Ok(report)) => {
                if report.dirs_changed > 0
                    || report.renamed + report.rebound + report.touched + report.staled > 0
                {
                    log::info!(
                        "reconcile: dirs_changed={} renamed={} rebound={} touched={} staled={} resync={}",
                        report.dirs_changed, report.renamed, report.rebound,
                        report.touched, report.staled, report.resync
                    );
                }
            }
            Ok(Err(e)) => log::warn!("reconcile failed: {}", e),
            Err(e) => log::warn!("reconcile task panicked: {}", e),
        }
    }
}
