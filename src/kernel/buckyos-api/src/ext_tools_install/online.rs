//! Online items (doc/app_service/paios容器需求.md §6.4): what a package only declares. Its `ext-manifest.json` names the
//! upstream's archive for each platform, and a node downloads that after the package is installed. The bytes never pass
//! through a package, a seed or the Repo. An item lives at `online/<id>/`, its record at `online/<id>.json`, both outside
//! the version directories.
//!
//! This is one attempt, from the record saying `downloading` to it saying `ready` or `failed`. When to make one is the
//! caller's: once when the package is installed, and after that only when someone asks. A failure stays a failure.
//!
//! Whoever uses an item reads its record and takes `state: ready` with the directory there as "usable". So what is in
//! place stays `ready` while another version, or the same one again, is downloaded: that attempt is the record's `next`,
//! and the item is only out of use for the moment the two directories swap. Everything an item holds can be read and run
//! by any account, whatever the umask of the node: a worker and the system's page renderer run as other users.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub const EXT_TOOLS_ONLINE_SCHEMA: &str = "buckyos.ext_tools.online/1";
/// How long putting an item in place waits for a package that is being installed.
const LOCK_WAIT: Duration = Duration::from_secs(900);
const RECORD_INTERVAL: Duration = Duration::from_secs(1);
const MAX_REDIRECTS: usize = 5;

/// `online/<id>.json`. No record: the item was never downloaded here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsOnlineRecord {
    pub schema: String,
    pub id: String,
    pub version: String,
    /// The package that declared it.
    pub package: String,
    pub platform: String,
    pub sha256: String,
    /// `downloading`, `ready` or `failed`.
    pub state: ExtToolsOnlineState,
    pub size_bytes: u64,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
    pub updated_at_ms: u64,
    /// The request this download answered (`ExtToolsOnlineRequest::request_ms`); 0: none, it was the automatic one.
    #[serde(default)]
    pub request_ms: u64,
    /// A download made while the item was `ready`, which it stays until that one takes its place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<ExtToolsOnlineAttempt>,
}

/// A download of another version than the one in place, or of that one again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsOnlineAttempt {
    pub version: String,
    pub sha256: String,
    /// `downloading`; `failed` only when it was to fetch again the version in place, which is still good. A version that
    /// is not in place and failed is what the whole record says.
    pub state: ExtToolsOnlineState,
    pub size_bytes: u64,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
    pub updated_at_ms: u64,
}

impl ExtToolsOnlineAttempt {
    fn status(&self) -> ExtToolsOnlineStatus {
        ExtToolsOnlineStatus {
            state: self.state,
            version: self.version.clone(),
            size_bytes: self.size_bytes,
            downloaded_bytes: self.downloaded_bytes,
            error: self.error.clone(),
            updated_at_ms: self.updated_at_ms,
        }
    }
}

impl ExtToolsOnlineRecord {
    fn in_place(&self) -> ExtToolsOnlineStatus {
        ExtToolsOnlineStatus {
            state: self.state,
            version: self.version.clone(),
            size_bytes: self.size_bytes,
            downloaded_bytes: self.downloaded_bytes,
            error: self.error.clone(),
            updated_at_ms: self.updated_at_ms,
        }
    }

    /// Where the item of the record's last attempt stands: on its way, or as it is in place, with the reason when
    /// fetching it again did not work.
    pub fn status(&self) -> ExtToolsOnlineStatus {
        match &self.next {
            Some(next) if next.state == ExtToolsOnlineState::Downloading || !(next.version == self.version && same_digest(&next.sha256, &self.sha256)) => next.status(),
            Some(next) => ExtToolsOnlineStatus { error: next.error.clone(), ..self.in_place() },
            None => self.in_place(),
        }
    }
}

/// An online item as a tools root has it: who declares it, where it comes from and how far it got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsOnlineEntry {
    pub id: String,
    pub package: String,
    pub version: String,
    pub auto: bool,
    pub platform: String,
    pub source: ExtToolsOnlineSource,
    /// Of the declared version: a record of another version counts as `missing`.
    pub status: ExtToolsOnlineStatus,
    /// The request the last download answered, whatever its version.
    pub request_ms: u64,
}

impl ExtToolsOnlineEntry {
    fn is(&self, version: &str, sha256: &str) -> bool {
        self.version == version && same_digest(sha256, &self.source.sha256)
    }

    fn attempt(&self, state: ExtToolsOnlineState, downloaded_bytes: u64, error: Option<String>) -> ExtToolsOnlineAttempt {
        ExtToolsOnlineAttempt {
            version: self.version.clone(),
            sha256: self.source.sha256.clone(),
            state,
            size_bytes: self.source.size_bytes,
            downloaded_bytes,
            error,
            updated_at_ms: now_ms(),
        }
    }

    /// A record that is all about `attempt`: nothing else is in use.
    fn record(&self, attempt: ExtToolsOnlineAttempt, request_ms: u64) -> ExtToolsOnlineRecord {
        ExtToolsOnlineRecord {
            schema: EXT_TOOLS_ONLINE_SCHEMA.to_string(),
            id: self.id.clone(),
            version: attempt.version,
            package: self.package.clone(),
            platform: self.platform.clone(),
            sha256: attempt.sha256,
            state: attempt.state,
            size_bytes: attempt.size_bytes,
            downloaded_bytes: attempt.downloaded_bytes,
            error: attempt.error,
            updated_at_ms: attempt.updated_at_ms,
            request_ms,
            next: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtToolsOnlineLimits {
    pub connect: Duration,
    /// The longest the upstream may send nothing.
    pub stall: Duration,
    pub total: Duration,
}

impl Default for ExtToolsOnlineLimits {
    fn default() -> Self {
        Self { connect: Duration::from_secs(30), stall: Duration::from_secs(60), total: Duration::from_secs(2 * 60 * 60) }
    }
}

/// A download as whoever started it sees it.
#[derive(Debug, Default)]
pub struct ExtToolsOnlineJob {
    /// The request it answers. Raised while it runs when another request comes in: this download answers that one too.
    pub request_ms: AtomicU64,
    pub downloaded_bytes: AtomicU64,
}

impl ExtToolsOnlineJob {
    pub fn new(request_ms: u64) -> Self {
        Self { request_ms: AtomicU64::new(request_ms), downloaded_bytes: AtomicU64::new(0) }
    }
}

struct Declared {
    package: String,
    /// The version directory of the package.
    dir: String,
    platform: String,
    item: ExtToolsOnlineItem,
}

fn same_digest(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl ToolsRoot {
    fn online_path(&self, item: &str) -> PathBuf {
        self.root.join(ONLINE_DIR).join(item)
    }

    fn online_record_path(&self, item: &str) -> PathBuf {
        self.root.join(ONLINE_DIR).join(format!("{item}.json"))
    }

    pub fn online_record(&self, item: &str) -> Option<ExtToolsOnlineRecord> {
        let raw = fs::read_to_string(self.online_record_path(item)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// What the active packages declare for their platform. One item id is one thing on a node: the first package to
    /// declare it has it.
    fn online_declared(&self) -> BTreeMap<String, Declared> {
        let mut out = BTreeMap::new();
        for (package, install) in self.installed() {
            let Ok(manifest) = self.read_version_manifest(&package, &install.dir) else { continue };
            for item in manifest.online {
                if item.platforms.contains_key(&install.platform) && !out.contains_key(&item.id) {
                    out.insert(item.id.clone(), Declared { package: package.clone(), dir: install.dir.clone(), platform: install.platform.clone(), item });
                }
            }
        }
        out
    }

    /// The record, when what it says is in place is this very archive. A `ready` one whose directory is gone says nothing.
    fn online_current(&self, item: &str, version: &str, platform: &str, source: &ExtToolsOnlineSource) -> Option<ExtToolsOnlineRecord> {
        self.online_record(item)
            .filter(|r| r.version == version && r.platform == platform && same_digest(&r.sha256, &source.sha256))
            .filter(|r| r.state != ExtToolsOnlineState::Ready || self.online_path(item).is_dir())
    }

    /// What is in place and in use, whatever its version.
    fn online_in_place(&self, item: &str) -> Option<ExtToolsOnlineRecord> {
        self.online_record(item).filter(|r| r.state == ExtToolsOnlineState::Ready && self.online_path(item).is_dir())
    }

    fn online_entry(&self, declared: &Declared) -> ExtToolsOnlineEntry {
        let item = &declared.item;
        let source = item.platforms[&declared.platform].clone();
        let record = self.online_record(&item.id);
        let request_ms = record.as_ref().map(|r| r.request_ms).unwrap_or(0);
        let next = record.and_then(|r| r.next).filter(|next| next.version == item.version && same_digest(&next.sha256, &source.sha256));
        let status = match (self.online_current(&item.id, &item.version, &declared.platform, &source), next) {
            // The declared version is in place and is fetched again, or was and that did not work.
            (Some(current), Some(next)) if current.state == ExtToolsOnlineState::Ready => match next.state {
                ExtToolsOnlineState::Downloading => next.status(),
                _ => ExtToolsOnlineStatus { error: next.error, ..current.in_place() },
            },
            (Some(current), _) => current.in_place(),
            // Another version is in place, and in use, while this one is on its way.
            (None, Some(next)) => next.status(),
            (None, None) => ExtToolsOnlineStatus {
                state: ExtToolsOnlineState::Missing,
                version: item.version.clone(),
                size_bytes: source.size_bytes,
                downloaded_bytes: 0,
                error: None,
                updated_at_ms: 0,
            },
        };
        ExtToolsOnlineEntry {
            id: item.id.clone(),
            package: declared.package.clone(),
            version: item.version.clone(),
            auto: item.auto,
            platform: declared.platform.clone(),
            source,
            status,
            request_ms,
        }
    }

    /// The online items of the packages active here, by id.
    pub fn online(&self) -> BTreeMap<String, ExtToolsOnlineEntry> {
        self.online_declared().values().map(|declared| (declared.item.id.clone(), self.online_entry(declared))).collect()
    }

    fn online_wanted(&self, item: &str) -> ExtToolsResult<ExtToolsOnlineEntry> {
        self.online().remove(item).ok_or_else(|| format!("no installed package declares the online item `{item}` for this platform"))
    }

    /// The commands of `manifest` that cannot run yet, each with the online items it is short of. An item the platform
    /// has no entry for never comes.
    pub(super) fn waiting_of(&self, manifest: &ExtToolsManifest, platform: &str) -> BTreeMap<String, Vec<String>> {
        let ready = |need: &String| {
            manifest
                .online
                .iter()
                .find(|item| &item.id == need)
                .and_then(|item| item.platforms.get(platform).map(|source| (item, source)))
                .and_then(|(item, source)| self.online_current(need, &item.version, platform, source))
                .is_some_and(|record| record.state == ExtToolsOnlineState::Ready)
        };
        manifest
            .provides
            .iter()
            .filter_map(|provided| {
                let short: Vec<String> = provided.needs.iter().filter(|need| !ready(need)).cloned().collect();
                (!short.is_empty()).then(|| (provided.command.clone(), short))
            })
            .collect()
    }

    /// Bring the record of every active package in line with what is ready: a command whose items arrived is probed, one
    /// whose item went away waits again. The caller holds the lock.
    fn refresh_waiting(&self) -> ExtToolsResult<()> {
        for (id, mut install) in self.installed() {
            let Ok(manifest) = self.read_version_manifest(&id, &install.dir) else { continue };
            let waiting = self.waiting_of(&manifest, &install.platform);
            if waiting == install.waiting {
                continue;
            }
            let windows = install.platform.starts_with("windows");
            for provided in &manifest.provides {
                match (install.waiting.contains_key(&provided.command), waiting.contains_key(&provided.command)) {
                    (true, false) => install.commands.insert(provided.command.clone(), self.probe_version(provided, windows)),
                    (false, true) => install.commands.insert(provided.command.clone(), String::new()),
                    _ => None,
                };
            }
            install.waiting = waiting;
            Self::write_json(&self.install_path(&id), &install)?;
        }
        Ok(())
    }

    fn refresh_waiting_if_stale(&self) {
        let stale = self
            .installed()
            .iter()
            .any(|(id, install)| self.read_version_manifest(id, &install.dir).is_ok_and(|manifest| self.waiting_of(&manifest, &install.platform) != install.waiting));
        if stale {
            if let Ok(_lock) = self.lock_wait(LOCK_WAIT) {
                let _ = self.refresh_waiting();
            }
        }
    }

    /// Lay the overlays of a package that just became active over its online items that are in place. An overlay is the
    /// package's and changes with it: a new launcher script must reach a node whose archive stays the same, and that
    /// archive is not fetched again. The caller holds the lock.
    pub(super) fn relay_online_overlays(&self, package: &str, dir: &str, manifest: &ExtToolsManifest, platform: &str) {
        for item in &manifest.online {
            let Some(source) = item.platforms.get(platform) else { continue };
            let Some(overlay) = &source.overlay else { continue };
            if !self.online_current(&item.id, &item.version, platform, source).is_some_and(|r| r.state == ExtToolsOnlineState::Ready) {
                continue;
            }
            if let Err(error) = lay_over(&self.version_dir(package, dir).join(overlay), &self.online_path(&item.id)) {
                log::warn!("ext_tools: the overlay of {package} for online item {}: {error}", item.id);
            }
        }
    }

    /// Drop what no active package declares any more. The caller holds the lock.
    pub(super) fn collect_online(&self) {
        let declared = self.online_declared();
        let Ok(entries) = fs::read_dir(self.root.join(ONLINE_DIR)) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if declared.contains_key(name.split('.').next().unwrap_or(&name)) {
                continue;
            }
            let _ = if entry.path().is_dir() { fs::remove_dir_all(entry.path()) } else { fs::remove_file(entry.path()) };
        }
    }

    /// In one rename, so that whoever reads the record never finds half of one.
    fn save_online_record(&self, record: &ExtToolsOnlineRecord) -> ExtToolsResult<()> {
        let dir = self.root.join(ONLINE_DIR);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        set_mode(&dir, 0o755);
        Self::write_json(&self.online_record_path(&record.id), record)
    }

    /// The item as declared, unless the download the record speaks of was for something the package has since replaced:
    /// then there is nothing to finish, and the item is `missing` again for its package to ask anew.
    fn online_attempt(&self, item: &str) -> ExtToolsResult<Option<ExtToolsOnlineEntry>> {
        let entry = self.online_wanted(item)?;
        let under_way = self.online_record(item).and_then(|r| match r.next {
            Some(next) if next.state == ExtToolsOnlineState::Downloading => Some((next.version, next.sha256)),
            _ if r.state == ExtToolsOnlineState::Downloading => Some((r.version, r.sha256)),
            _ => None,
        });
        let other = under_way.is_some_and(|(version, sha256)| !entry.is(&version, &sha256));
        Ok((!other).then_some(entry))
    }

    /// Where an attempt keeps its files until they are in place: `staging/online-<item>.<what>`. An id has no dot.
    fn online_staging(&self, item: &str, what: &str) -> PathBuf {
        self.root.join("staging").join(format!("online-{item}.{what}"))
    }

    /// What an attempt cut short left in `staging/`.
    fn clear_online_staging(&self, item: &str) {
        let prefix = format!("online-{item}.");
        let Ok(entries) = fs::read_dir(self.root.join("staging")) else { return };
        for entry in entries.flatten().filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix)) {
            let _ = if entry.path().is_dir() { fs::remove_dir_all(entry.path()) } else { fs::remove_file(entry.path()) };
        }
    }

    /// The first step of a download: the record says `downloading` from here until `online_apply` or `online_fail`. An
    /// item that is in place keeps saying `ready`, with the download as its `next`.
    pub fn online_begin(&self, item: &str, request_ms: u64) -> ExtToolsResult<ExtToolsOnlineEntry> {
        self.prepare()?;
        let entry = self.online_wanted(item)?;
        self.clear_online_staging(item);
        let attempt = entry.attempt(ExtToolsOnlineState::Downloading, 0, None);
        let request_ms = request_ms.max(entry.request_ms);
        let record = match self.online_in_place(item) {
            Some(kept) => ExtToolsOnlineRecord { next: Some(attempt), request_ms, ..kept },
            None => entry.record(attempt, request_ms),
        };
        self.save_online_record(&record)?;
        self.refresh_waiting_if_stale();
        self.online_wanted(item)
    }

    /// How far a download got, for whoever reads the record.
    pub fn online_progress(&self, item: &str, downloaded_bytes: u64) {
        let Some(mut record) = self.online_record(item) else { return };
        match &mut record.next {
            Some(next) if next.state == ExtToolsOnlineState::Downloading => {
                next.downloaded_bytes = downloaded_bytes;
                next.updated_at_ms = now_ms();
            }
            _ if record.state == ExtToolsOnlineState::Downloading => {
                record.downloaded_bytes = downloaded_bytes;
                record.updated_at_ms = now_ms();
            }
            _ => return,
        }
        let _ = Self::write_json(&self.online_record_path(item), &record);
    }

    fn online_failed(&self, entry: &ExtToolsOnlineEntry, error: &str, request_ms: u64) -> ExtToolsResult<ExtToolsOnlineRecord> {
        let downloaded = self.online_record(&entry.id).map_or(0, |r| match r.next {
            Some(next) => next.downloaded_bytes,
            None if r.state == ExtToolsOnlineState::Downloading => r.downloaded_bytes,
            None => 0,
        });
        let attempt = entry.attempt(ExtToolsOnlineState::Failed, downloaded, Some(error.to_string()));
        let request_ms = request_ms.max(entry.request_ms);
        // The declared version in place is as good as before: it stays in use, and only tells why it was not replaced.
        // Another version in place is not what the package needs: the item has failed.
        let record = match self.online_in_place(&entry.id).filter(|kept| kept.platform == entry.platform && entry.is(&kept.version, &kept.sha256)) {
            Some(kept) => ExtToolsOnlineRecord { next: Some(attempt), request_ms, ..kept },
            None => entry.record(attempt, request_ms),
        };
        self.save_online_record(&record)?;
        self.refresh_waiting_if_stale();
        Ok(record)
    }

    /// The end of a download that did not work. It stays that way until someone asks again.
    pub fn online_fail(&self, item: &str, error: &str, request_ms: u64) -> ExtToolsResult<ExtToolsOnlineRecord> {
        match self.online_attempt(item)? {
            Some(entry) => self.online_failed(&entry, error, request_ms),
            None => Err(format!("the download of `{item}` was for a version its package no longer declares")),
        }
    }

    /// Check the archive, unpack it beside what is in use and swap the two. It ends with the record: `ready`, or `failed`
    /// with nothing of the attempt left behind and what was in place before still there.
    pub fn online_apply(&self, item: &str, archive: &Path, request_ms: u64) -> ExtToolsResult<ExtToolsOnlineRecord> {
        let superseded = || format!("the download of `{item}` was for a version its package no longer declares");
        let entry = self.online_attempt(item)?.ok_or_else(superseded)?;
        let stage = self.online_staging(item, &now_ms().to_string());
        let placed = self.online_place(&entry, archive, &stage, request_ms);
        if fs::symlink_metadata(&stage).is_ok() {
            let _ = fs::remove_dir_all(&stage);
        }
        match placed {
            Ok(Some(record)) => Ok(record),
            Ok(None) => Err(superseded()),
            Err(error) => {
                self.online_failed(&entry, &error, request_ms)?;
                Err(error)
            }
        }
    }

    /// `None`: the package changed its mind while the archive was unpacked.
    fn online_place(&self, entry: &ExtToolsOnlineEntry, archive: &Path, stage: &Path, request_ms: u64) -> ExtToolsResult<Option<ExtToolsOnlineRecord>> {
        let digest = ext_tools_sha256_file(archive)?;
        let digest = digest.trim_start_matches("sha256:");
        if !same_digest(digest, &entry.source.sha256) {
            return Err(format!("{}: sha256 is {digest}, the package declares {}", entry.source.url, entry.source.sha256));
        }
        let into = match &entry.source.into {
            Some(into) => stage.join(into),
            None => stage.to_path_buf(),
        };
        match entry.source.format.as_str() {
            "zip" => ext_tools_unzip(archive, &into)?,
            _ => ext_tools_unpack(archive, &into)?,
        }
        for (from, to) in &entry.source.rename {
            rename_inside(stage, from, to)?;
        }
        let lock = self.lock_wait(LOCK_WAIT)?;
        let declared = self.online_declared();
        let Some(declared) = declared.get(&entry.id).filter(|d| d.item.platforms.get(&d.platform) == Some(&entry.source) && d.item.version == entry.version && d.package == entry.package) else {
            return Ok(None);
        };
        if let Some(overlay) = &entry.source.overlay {
            let from = self.version_dir(&declared.package, &declared.dir).join(overlay);
            lay_over(&from, stage)?;
        }
        let whole = fs::canonicalize(stage).map_err(|e| format!("{}: {e}", stage.display()))?;
        seal(&whole, &whole, archive)?;
        let place = self.online_path(&entry.id);
        let old = self.online_staging(&entry.id, &format!("old-{}", now_ms()));
        let had = fs::symlink_metadata(&place).is_ok();
        if had {
            fs::rename(&place, &old).map_err(|e| format!("{} cannot be replaced (in use?): {e}", place.display()))?;
        }
        if let Err(error) = fs::rename(stage, &place) {
            if had {
                let _ = fs::rename(&old, &place);
            }
            return Err(format!("{}: {error}", place.display()));
        }
        let record = entry.record(entry.attempt(ExtToolsOnlineState::Ready, entry.source.size_bytes, None), request_ms.max(entry.request_ms));
        self.save_online_record(&record)?;
        let _ = self.refresh_waiting();
        drop(lock);
        if had {
            let _ = fs::remove_dir_all(&old);
        }
        Ok(Some(record))
    }
}

fn rename_inside(stage: &Path, from: &str, to: &str) -> ExtToolsResult<()> {
    let (source, target) = (stage.join(from), stage.join(to));
    if fs::symlink_metadata(&source).is_err() {
        return Err(format!("rename: `{from}` is not in the archive"));
    }
    if fs::symlink_metadata(&target).is_ok() {
        return Err(format!("rename: `{to}` is already in the archive"));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::rename(&source, &target).map_err(|e| format!("rename `{from}` to `{to}`: {e}"))
}

/// Lay the overlay `from` over `to`: a file takes the place of what is there under its name, in one rename, and the
/// rest stays. An overlay is a package's own few files; it holds no links.
fn lay_over(from: &Path, to: &Path) -> ExtToolsResult<()> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    set_mode(to, 0o755);
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", from.display()))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", source.display()))?;
        let existing = fs::symlink_metadata(&target).ok();
        if kind.is_dir() {
            if existing.is_some_and(|m| !m.is_dir()) {
                fs::remove_file(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            lay_over(&source, &target)?;
        } else if kind.is_file() {
            let fresh = to.join(format!(".{}.overlay", entry.file_name().to_string_lossy()));
            fs::copy(&source, &fresh).map_err(|e| format!("{}: {e}", fresh.display()))?;
            set_mode(&fresh, if is_executable(&source) { 0o755 } else { 0o644 });
            if existing.is_some_and(|m| m.is_dir()) {
                fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            fs::rename(&fresh, &target).map_err(|e| {
                let _ = fs::remove_file(&fresh);
                format!("{}: {e}", target.display())
            })?;
        } else {
            return Err(format!("{}: an overlay holds files and directories only", source.display()));
        }
    }
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// The tree as any account may use it, whatever umask it was made under: directories anyone enters, files anyone reads,
/// and runs when they were executable. And no symlink that leads out: a link's target is checked as written when it is
/// made, and here as the disk resolves it.
fn seal(dir: &Path, base: &Path, archive: &Path) -> ExtToolsResult<()> {
    set_mode(dir, 0o755);
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", entry.path().display()))?;
        if kind.is_dir() {
            seal(&entry.path(), base, archive)?;
        } else if kind.is_symlink() {
            if fs::canonicalize(entry.path()).is_ok_and(|real| !real.starts_with(base)) {
                return Err(format!("{}: `{}` links outside the item", archive.display(), entry.path().strip_prefix(base).unwrap_or(&entry.path()).display()));
            }
        } else {
            set_mode(&entry.path(), if is_executable(&entry.path()) { 0o755 } else { 0o644 });
        }
    }
    Ok(())
}

/// Unpack a zip into `dest` the way `ext_tools_unpack` does a tar: nothing leaves `dest`, what was executable stays so
/// and everything is readable, symlinks stay symlinks (a macOS app bundle is made of them) and may only point inside.
pub fn ext_tools_unzip(zip_path: &Path, dest: &Path) -> ExtToolsResult<()> {
    let name = zip_path.display();
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let file = fs::File::open(zip_path).map_err(|e| format!("{name}: {e}"))?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| format!("{name}: {e}"))?;
    let mut links: Vec<(PathBuf, PathBuf)> = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| format!("{name}: {e}"))?;
        let path = entry.enclosed_name().filter(|path| normal_relative(path)).ok_or_else(|| format!("{name}: `{}` leaves the item", entry.name()))?;
        let mode = entry.unix_mode();
        if entry.is_symlink() {
            let mut target = String::new();
            entry.by_ref().take(4096).read_to_string(&mut target).map_err(|e| format!("{name}: `{}`: {e}", path.display()))?;
            let target = PathBuf::from(target);
            if resolved_inside(&path, &target).is_none() {
                return Err(format!("{name}: `{}` links outside the item ({})", path.display(), target.display()));
            }
            links.push((path, target));
            continue;
        }
        let out = dest.join(&path);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| format!("{name}: `{}`: {e}", path.display()))?;
            continue;
        }
        if mode.is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000)) {
            return Err(format!("{name}: `{}` is a special file", path.display()));
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{name}: `{}`: {e}", path.display()))?;
        }
        let mut target = fs::File::create(&out).map_err(|e| format!("{name}: `{}`: {e}", path.display()))?;
        std::io::copy(&mut entry, &mut target).map_err(|e| format!("{name}: unpack `{}`: {e}", path.display()))?;
        set_mode(&out, if mode.is_some_and(|m| m & 0o111 != 0) { 0o755 } else { 0o644 });
    }
    // Links come last, so that no file was written through one.
    for (path, target) in &links {
        let mut parent = dest.to_path_buf();
        for part in path.parent().unwrap_or(Path::new("")).components() {
            parent.push(part);
            if fs::symlink_metadata(&parent).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!("{name}: `{}` lies behind a symlink", path.display()));
            }
        }
        fs::create_dir_all(&parent).map_err(|e| format!("{name}: `{}`: {e}", path.display()))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, dest.join(path)).map_err(|e| format!("{name}: `{}`: {e}", path.display()))?;
        #[cfg(not(unix))]
        return Err(format!("{name}: `{}` is a symlink to {}, which this platform cannot unpack", path.display(), target.display()));
    }
    let base = fs::canonicalize(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    seal(&base, &base, zip_path)
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text = format!("{text}: {cause_text}");
        }
        source = cause.source();
    }
    text
}

async fn fetch_to(source: &ExtToolsOnlineSource, dest: &Path, limits: &ExtToolsOnlineLimits, on_progress: &mut (dyn FnMut(u64) + Send)) -> ExtToolsResult<()> {
    use tokio::io::AsyncWriteExt;
    let url = source.url.as_str();
    if !online_url_allowed(url) {
        return Err(format!("{url}: an online item only comes from an https address"));
    }
    let client = reqwest::Client::builder()
        .connect_timeout(limits.connect)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if online_url_allowed(attempt.url().as_str()) {
                attempt.follow()
            } else {
                attempt.error("redirected away from https")
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;
    let stalled = || format!("{url}: nothing received for {}s", limits.stall.as_secs_f32());
    let mut response = tokio::time::timeout(limits.stall.max(limits.connect), client.get(url).send())
        .await
        .map_err(|_| format!("{url}: no answer in {}s", limits.stall.max(limits.connect).as_secs_f32()))?
        .map_err(|e| format!("{url}: {}", error_chain(&e)))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status().as_u16()));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    loop {
        let chunk = tokio::time::timeout(limits.stall, response.chunk()).await.map_err(|_| stalled())?.map_err(|e| format!("{url}: {}", error_chain(&e)))?;
        let Some(chunk) = chunk else { break };
        received += chunk.len() as u64;
        if received > source.size_bytes {
            return Err(format!("{url}: more than the {} bytes the package declares", source.size_bytes));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| format!("{}: {e}", dest.display()))?;
        on_progress(received);
    }
    file.flush().await.map_err(|e| format!("{}: {e}", dest.display()))?;
    let digest = hex::encode(hasher.finalize());
    if !same_digest(&digest, &source.sha256) {
        return Err(format!("{url}: sha256 is {digest} ({received} bytes), the package declares {} ({} bytes)", source.sha256, source.size_bytes));
    }
    Ok(())
}

/// The upstream's archive to `dest`, streamed, and only kept when it is the bytes the package declares.
pub async fn ext_tools_online_fetch(source: &ExtToolsOnlineSource, dest: &Path, limits: &ExtToolsOnlineLimits, on_progress: &mut (dyn FnMut(u64) + Send)) -> ExtToolsResult<()> {
    let fetched = match tokio::time::timeout(limits.total, fetch_to(source, dest, limits, on_progress)).await {
        Ok(fetched) => fetched,
        Err(_) => Err(format!("{}: not finished in {}s", source.url, limits.total.as_secs())),
    };
    if fetched.is_err() {
        let _ = fs::remove_file(dest);
    }
    fetched
}

/// One attempt at an online item of the tools root at `root`: download, check, unpack, put in place. Whatever happens
/// the record says so when this returns.
pub async fn ext_tools_online_install(root: &Path, item: &str, limits: &ExtToolsOnlineLimits, job: &ExtToolsOnlineJob) -> ExtToolsResult<ExtToolsOnlineRecord> {
    let (tools, name, asked) = (ToolsRoot::new(root), item.to_string(), job.request_ms.load(Ordering::Relaxed));
    let entry = tokio::task::spawn_blocking(move || tools.online_begin(&name, asked)).await.map_err(|e| e.to_string())??;
    let tools = ToolsRoot::new(root);
    let archive = tools.online_staging(item, "download");
    let mut written = Instant::now();
    let fetched = ext_tools_online_fetch(&entry.source, &archive, limits, &mut |received| {
        job.downloaded_bytes.store(received, Ordering::Relaxed);
        if written.elapsed() >= RECORD_INTERVAL {
            written = Instant::now();
            tools.online_progress(item, received);
        }
    })
    .await;
    let (tools, name, asked, file) = (ToolsRoot::new(root), item.to_string(), job.request_ms.load(Ordering::Relaxed), archive.clone());
    let done = tokio::task::spawn_blocking(move || match fetched {
        Ok(()) => tools.online_apply(&name, &file, asked),
        Err(error) => {
            tools.online_fail(&name, &error, asked)?;
            Err(error)
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&archive);
    done
}
