//! Archive caches (archive TODO T1–T3): parsed indexes by outer version,
//! object outers landed on disk with their sha256 (D16), materialized members
//! (D9), one disk budget for the last two with least-recently-used eviction,
//! and single-flight work that stops when its last waiter leaves.

use super::names::Naming;
use super::zip::{self, Member, PosReader, Source, Stamp};
use super::ArchiveConfig;
use crate::error::*;
use crate::state::AppState;
use ndn_lib::ObjId;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TMP_PREFIX: &str = ".tmp-";

/// A parsed archive, shared by every request on the same outer version.
#[derive(Debug)]
pub struct Index {
    /// `n:<root>/<rel>:<ino>:<dev>:<birth>:<size>:<mtime>` or `o:<obj_id>`.
    pub key: String,
    pub src: Arc<Source>,
    pub zip: ::zip::ZipArchive<PosReader>,
    pub members: Vec<Member>,
    pub naming: Naming,
    /// Outer file name (the archive root's display name).
    pub name: String,
    /// Whole-file sha256 (hex) when known without hashing (object outers).
    pub sha256: Option<String>,
}

impl Index {
    pub fn build(key: String, path: PathBuf, stamp: Option<Stamp>, name: String, sha256: Option<String>, limits: zip::Limits) -> NfsResult<Index> {
        let src = Source::open(path, stamp).map_err(zip::io_error)?;
        let parsed = zip::parse(&src, limits)?;
        Ok(Index { key, src, zip: parsed.zip, members: parsed.members, naming: parsed.naming, name, sha256 })
    }

    pub fn member_key(&self, raw: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(self.key.as_bytes());
        h.update([0]);
        h.update(raw);
        hex::encode(h.finalize())
    }

    pub fn encrypted_entries(&self) -> usize {
        self.members.iter().filter(|m| m.encrypted).count()
    }
}

struct Flight<T> {
    rx: tokio::sync::watch::Receiver<Option<Result<T, NfsError>>>,
    waiters: AtomicUsize,
    cancel: Arc<AtomicBool>,
}

/// Concurrent requests for one key share one run; it is cancelled when the
/// last waiter leaves before it finished.
pub struct Flights<T> {
    map: Arc<Mutex<HashMap<String, Arc<Flight<T>>>>>,
}

impl<T> Default for Flights<T> {
    fn default() -> Self {
        Flights { map: Default::default() }
    }
}

struct Waiting<T> {
    flight: Arc<Flight<T>>,
}

impl<T> Drop for Waiting<T> {
    fn drop(&mut self) {
        if self.flight.waiters.fetch_sub(1, Ordering::SeqCst) == 1 && self.flight.rx.borrow().is_none() {
            self.flight.cancel.store(true, Ordering::SeqCst);
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Flights<T> {
    pub async fn run<F, Fut>(&self, key: &str, make: F) -> NfsResult<T>
    where
        F: FnOnce(Arc<AtomicBool>) -> Fut,
        Fut: Future<Output = NfsResult<T>> + Send + 'static,
    {
        let flight = {
            let mut map = self.map.lock().unwrap();
            match map.get(key) {
                Some(f) if !f.cancel.load(Ordering::SeqCst) => f.clone(),
                _ => {
                    let (tx, rx) = tokio::sync::watch::channel(None);
                    let cancel = Arc::new(AtomicBool::new(false));
                    let flight = Arc::new(Flight { rx, waiters: AtomicUsize::new(0), cancel: cancel.clone() });
                    map.insert(key.to_string(), flight.clone());
                    let fut = make(cancel);
                    let (map_ref, key, mine) = (self.map.clone(), key.to_string(), flight.clone());
                    tokio::spawn(async move {
                        let result = fut.await;
                        let _ = tx.send(Some(result));
                        let mut map = map_ref.lock().unwrap();
                        if map.get(&key).is_some_and(|f| Arc::ptr_eq(f, &mine)) {
                            map.remove(&key);
                        }
                    });
                    flight
                }
            }
        };
        flight.waiters.fetch_add(1, Ordering::SeqCst);
        let _waiting = Waiting { flight: flight.clone() };
        let mut rx = flight.rx.clone();
        loop {
            if let Some(result) = rx.borrow().clone() {
                return result;
            }
            if rx.changed().await.is_err() {
                return Err(internal("archive work ended without a result"));
            }
        }
    }
}

#[derive(Debug, Default)]
struct Budget {
    files: HashMap<PathBuf, (u64, Instant)>,
    total: u64,
}

/// What the archive feature keeps in memory and under `<data_dir>/archive`.
pub struct ArchiveMgr {
    pub config: ArchiveConfig,
    pub dir: PathBuf,
    indexes: Mutex<HashMap<String, (Arc<Index>, Instant)>>,
    budget: Mutex<Budget>,
    pub objects: Flights<(PathBuf, String, String)>,
    pub members: Flights<PathBuf>,
    pub indexing: Flights<Arc<Index>>,
    pub workers: Arc<tokio::sync::Semaphore>,
    pub passwords: super::password::Sessions,
}

impl ArchiveMgr {
    pub fn new(data_dir: &Path, config: ArchiveConfig) -> NfsResult<ArchiveMgr> {
        let dir = data_dir.join("archive");
        let mut budget = Budget::default();
        for sub in ["objects", "members"] {
            let d = dir.join(sub);
            std::fs::create_dir_all(&d)?;
            for e in std::fs::read_dir(&d)?.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(TMP_PREFIX) {
                    let _ = std::fs::remove_file(e.path());
                    continue;
                }
                if name.ends_with(".json") {
                    continue;
                }
                if let Ok(m) = e.metadata() {
                    budget.total += m.len();
                    budget.files.insert(e.path(), (m.len(), Instant::now()));
                }
            }
        }
        let workers = Arc::new(tokio::sync::Semaphore::new(config.workers.max(1)));
        Ok(ArchiveMgr {
            config,
            dir,
            indexes: Default::default(),
            budget: Mutex::new(budget),
            objects: Default::default(),
            members: Default::default(),
            indexing: Default::default(),
            workers,
            passwords: Default::default(),
        })
    }

    pub fn limits(&self) -> zip::Limits {
        zip::Limits { max_central_directory_bytes: self.config.max_central_directory_bytes, max_entries: self.config.max_entries }
    }

    pub fn cached(&self, key: &str) -> Option<Arc<Index>> {
        let mut map = self.indexes.lock().unwrap();
        let hit = map.get_mut(key)?;
        hit.1 = Instant::now();
        Some(hit.0.clone())
    }

    pub fn insert(&self, index: Arc<Index>) {
        let mut map = self.indexes.lock().unwrap();
        map.insert(index.key.clone(), (index, Instant::now()));
        while map.len() > self.config.max_indexes.max(1) {
            let Some(oldest) = map.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else { break };
            map.remove(&oldest);
        }
    }

    /// Indexes of native outers at `prefix` other than `keep` are of older versions.
    pub fn forget_versions(&self, prefix: &str, keep: &str) {
        self.indexes.lock().unwrap().retain(|k, _| !k.starts_with(prefix) || k == keep);
    }

    /// Idle descriptors close (Windows keeps users from replacing open files).
    pub fn sweep(&self) {
        let idle = Duration::from_secs(self.config.idle_close_secs);
        for (index, _) in self.indexes.lock().unwrap().values() {
            index.src.close_idle(idle);
        }
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.dir.join("objects")
    }

    pub fn members_dir(&self) -> PathBuf {
        self.dir.join("members")
    }

    pub fn tmp_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{TMP_PREFIX}{}", uuid::Uuid::new_v4().simple()))
    }

    /// Room for `size` more bytes: least recently used files go first.
    pub fn reserve(&self, size: u64) -> NfsResult<()> {
        if size > self.config.max_materialize_bytes.min(self.config.disk_budget_bytes) {
            return Err(NfsError::new(ErrorCode::QuotaExceeded, "the content is larger than the archive cache allows").with("limit", json!(self.config.max_materialize_bytes)));
        }
        let mut b = self.budget.lock().unwrap();
        let mut order: Vec<(PathBuf, u64, Instant)> = b.files.iter().map(|(p, (s, t))| (p.clone(), *s, *t)).collect();
        order.sort_by_key(|(_, _, t)| *t);
        for (path, len, _) in order {
            if b.total + size <= self.config.disk_budget_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() || !path.exists() {
                let _ = std::fs::remove_file(path.with_extension("json"));
                b.files.remove(&path);
                b.total = b.total.saturating_sub(len);
                if path.parent() == Some(self.objects_dir().as_path()) {
                    let obj_key = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.indexes.lock().unwrap().retain(|_, (i, _)| !i.src_path_ends_with(&obj_key));
                }
            }
        }
        if b.total + size > self.config.disk_budget_bytes {
            return Err(NfsError::new(ErrorCode::QuotaExceeded, "the archive cache is full").with("limit", json!(self.config.disk_budget_bytes)));
        }
        Ok(())
    }

    pub fn record(&self, path: &Path, size: u64) {
        let mut b = self.budget.lock().unwrap();
        if let Some((old, _)) = b.files.insert(path.to_path_buf(), (size, Instant::now())) {
            b.total = b.total.saturating_sub(old);
        }
        b.total += size;
    }

    pub fn touch(&self, path: &Path) -> bool {
        let mut b = self.budget.lock().unwrap();
        match b.files.get_mut(path) {
            Some(entry) if path.exists() => {
                entry.1 = Instant::now();
                true
            }
            Some(_) => {
                let (len, _) = b.files.remove(path).unwrap();
                b.total = b.total.saturating_sub(len);
                false
            }
            None => false,
        }
    }

    pub fn member_path(&self, index: &Index, raw: &[u8]) -> PathBuf {
        self.members_dir().join(index.member_key(raw))
    }
}

impl Index {
    fn src_path_ends_with(&self, file_name: &str) -> bool {
        self.key.starts_with("o:") && hex::encode(Sha256::digest(&self.key.as_bytes()[2..])) == file_name
    }
}

/// `(path, sha256 hex, file name)` of an object outer landed on disk.
pub async fn object_outer(state: &Arc<AppState>, obj_id: &str) -> NfsResult<(PathBuf, String, String)> {
    let id = ObjId::new(obj_id).map_err(|e| invalid(format!("bad obj_id: {e}")))?;
    if !id.is_file_object() {
        return Err(NfsError::new(ErrorCode::Unsupported, "only FileObjects can be opened as archives").with("reason", json!("not_file_object")));
    }
    let mgr = &state.archive;
    let file_name = hex::encode(Sha256::digest(obj_id.as_bytes()));
    let path = mgr.objects_dir().join(&file_name);
    let side = path.with_extension("json");
    if mgr.touch(&path) {
        if let Some(meta) = std::fs::read(&side).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            if let (Some(sha), Some(name)) = (meta["sha256"].as_str(), meta["name"].as_str()) {
                return Ok((path, sha.to_string(), name.to_string()));
            }
        }
    }
    let st = state.clone();
    let key = obj_id.to_string();
    mgr.objects.run(&key, move |cancel| async move { land_object(st, id, path, cancel).await }).await
}

async fn land_object(state: Arc<AppState>, id: ObjId, path: PathBuf, cancel: Arc<AtomicBool>) -> NfsResult<(PathBuf, String, String)> {
    let ndm = state.share.store(&state.config).await?;
    let file = bucket_store::content::load_file(ndm, &id).await.map_err(|e| not_found(format!("the object is not available: {e}")))?;
    let mgr = &state.archive;
    mgr.reserve(file.size)?;
    let content = crate::preview::content_of(ndm, &id).await.map_err(|e| not_found(e.message))?;
    let (mut reader, _) = ndm.open_reader(&content, None).await.map_err(|e| not_found(format!("the object content is not available: {e}")))?;
    let tmp = mgr.tmp_path(&mgr.objects_dir());
    let result = async {
        let mut out = create_private(&tmp).await?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(NfsError::new(ErrorCode::Internal, "cancelled"));
            }
            let n = reader.read(&mut buf).await.map_err(|e| not_found(format!("reading the object: {e}")))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > file.size {
                return Err(zip::corrupt("the object is longer than its FileObject says"));
            }
            hash.update(&buf[..n]);
            out.write_all(&buf[..n]).await?;
        }
        if total != file.size {
            return Err(zip::corrupt("the object is shorter than its FileObject says"));
        }
        out.sync_all().await?;
        Ok(hex::encode(hash.finalize()))
    }
    .await;
    let sha = match result {
        Ok(sha) => sha,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    };
    tokio::fs::rename(&tmp, &path).await?;
    tokio::fs::write(path.with_extension("json"), json!({"sha256": sha, "name": file.name, "size": file.size}).to_string()).await?;
    mgr.record(&path, file.size);
    Ok((path, sha, file.name.clone()))
}

pub async fn create_private(path: &Path) -> NfsResult<tokio::fs::File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    Ok(options.open(path).await?)
}

pub fn create_private_sync(path: &Path) -> NfsResult<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

/// Decodes a member completely into the cache (blocking); the file becomes
/// visible only after the whole member checked out.
pub fn materialize_blocking(index: &Index, m: &Member, password: Option<&[u8]>, tmp: &Path, dest: &Path, cancel: &AtomicBool) -> NfsResult<bool> {
    use std::io::Write;
    let mut zip = index.zip.clone();
    let reader = zip::open(&mut zip, m, password)?;
    let mut out = std::io::BufWriter::new(create_private_sync(tmp)?);
    let done = zip::pump(reader, m.size, None, cancel, &mut |chunk| out.write_all(chunk).map(|_| true));
    let finished = match done {
        Ok(true) => {
            let file = out.into_inner().map_err(|e| NfsError::from(e.into_error()))?;
            file.sync_all()?;
            std::fs::rename(tmp, dest)?;
            true
        }
        Ok(false) => false,
        Err(e) => {
            drop(out);
            let _ = std::fs::remove_file(tmp);
            return Err(e);
        }
    };
    if !finished {
        let _ = std::fs::remove_file(tmp);
    }
    Ok(finished)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[tokio::test]
    async fn concurrent_callers_share_one_run() {
        let flights: Arc<Flights<u32>> = Arc::new(Flights::default());
        let runs = Arc::new(AtomicU32::new(0));
        let mut joins = Vec::new();
        for _ in 0..8 {
            let (f, r) = (flights.clone(), runs.clone());
            joins.push(tokio::spawn(async move {
                f.run("k", move |_| async move {
                    r.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(7)
                })
                .await
            }));
        }
        for j in joins {
            assert_eq!(j.await.unwrap().unwrap(), 7);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_last_waiter_leaving_cancels() {
        let flights: Arc<Flights<u32>> = Arc::new(Flights::default());
        let seen = Arc::new(AtomicBool::new(false));
        let s = seen.clone();
        let waiter = {
            let f = flights.clone();
            tokio::spawn(async move {
                f.run("k", move |cancel| async move {
                    for _ in 0..100 {
                        if cancel.load(Ordering::SeqCst) {
                            s.store(true, Ordering::SeqCst);
                            return Err(internal("cancelled"));
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Ok(1)
                })
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        waiter.abort();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(seen.load(Ordering::SeqCst));
        assert_eq!(flights.run("k", |_| async { Ok(2) }).await.unwrap(), 2, "a new caller starts afresh");
    }

    #[test]
    fn the_budget_evicts_least_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        let config = ArchiveConfig { disk_budget_bytes: 100, max_materialize_bytes: 100, ..Default::default() };
        let mgr = ArchiveMgr::new(dir.path(), config).unwrap();
        let (a, b) = (mgr.members_dir().join("a"), mgr.members_dir().join("b"));
        for p in [&a, &b] {
            std::fs::write(p, [0u8; 40]).unwrap();
            mgr.record(p, 40);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(mgr.touch(&a));
        mgr.reserve(40).unwrap();
        assert!(a.exists() && !b.exists());
        assert_eq!(mgr.reserve(101).unwrap_err().code, ErrorCode::QuotaExceeded);
        std::fs::write(mgr.members_dir().join(".tmp-left"), b"x").unwrap();
        drop(mgr);
        let again = ArchiveMgr::new(dir.path(), ArchiveConfig::default()).unwrap();
        assert!(!again.members_dir().join(".tmp-left").exists(), "startup clears unfinished files");
    }
}
