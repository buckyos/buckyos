//! The extension package reconciler (ExtTools TODO §3.3, X3): this node's `NodeConfig.ext_tools` against what is installed.
//! `native` is `$BUCKYOS_ROOT/tools`, for whatever runs on the machine itself: host runtimes, the workflow service's media
//! functions. On Linux the workers mount that same directory. Where Docker runs in a VM they cannot run its files, so there
//! is a second root, `container`: a real volume filled with the Linux payload, through the worker image's copy of the one
//! engine in `buckyos_api::ext_tools_install`. Agents and their services never install anything: they read `tools/installs/`.
//!
//! A package's online items (`tools/online/<item>/`) are downloaded here from their upstream, beside the reconciler and
//! never in its way: once by itself when the package is there and the item never was, and after that only when the
//! administrator asks (`nodes/<node>/ext_tools_online/<item>`). A download that fails stays failed.

use crate::app_loader::{CommandOutput, PlatformTarget};
use crate::paios_images::{self, PaiosDocker, PaiosImageKind, EXTTOOL_MOUNT_ROOT};
use buckyos_api::ext_tools_install::*;
use buckyos_api::pikg::PikgReader;
use buckyos_api::{
    ext_tools_container_platform, ext_tools_current_platform, ext_tools_node_status_key, ext_tools_online_request_key,
    ext_tools_online_request_prefix, ext_tools_platform, get_buckyos_api_runtime, ExtToolsNodeStatus, ExtToolsOnlineRequest,
    ExtToolsOnlineState, ExtToolsOnlineStatus, ExtToolsPlatform, ExtToolsStage, ExtToolsTargetStatus, NodeConfig, NodeExtTools,
};
use buckyos_kit::get_buckyos_root_dir;
use log::{info, warn};
use ndn_lib::ObjId;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_secs(15);
/// How often a download's progress is looked at between rounds.
const PROGRESS_TICK: Duration = Duration::from_secs(2);
const RETRY_MIN: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(30 * 60);
const VOLUME_STATE_TTL: Duration = Duration::from_secs(300);
const DOWNLOAD_DIR: &str = "local/node_daemon/ext_tools";
/// Archives of online items on their way to the tools volume. Not under `DOWNLOAD_DIR`, which every round clears.
const ONLINE_DOWNLOAD_DIR: &str = "local/node_daemon/ext_tools_online";
const INTERRUPTED: &str = "the node stopped during the download";
const NATIVE: &str = "native";
const CONTAINER: &str = "container";

/// What an online item needs next on one tools root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OnlineStep {
    Keep,
    /// Download it, answering the request made at `request_ms`; 0: nobody asked, its package was just installed.
    Download { request_ms: u64 },
    /// The record says a download is under way and none is: the node stopped in the middle of it.
    Interrupted,
}

/// For an item with no download under way. By itself only when it is `auto` and was never tried: a failure is not
/// retried, after a restart either. A request newer than the one the record answered always downloads again.
pub(crate) fn plan_online(entry: &ExtToolsOnlineEntry, request: Option<&ExtToolsOnlineRequest>) -> OnlineStep {
    let asked = request.map(|r| r.request_ms).unwrap_or(0);
    if asked > entry.request_ms {
        return OnlineStep::Download { request_ms: asked };
    }
    match entry.status.state {
        ExtToolsOnlineState::Downloading => OnlineStep::Interrupted,
        ExtToolsOnlineState::Missing if entry.auto => OnlineStep::Download { request_ms: asked },
        _ => OnlineStep::Keep,
    }
}

/// A download under way, as the reconciler follows it.
struct OnlineDownload {
    package: String,
    job: Arc<ExtToolsOnlineJob>,
    task: tokio::task::JoinHandle<()>,
    status: ExtToolsOnlineStatus,
}

impl OnlineDownload {
    /// Progress in steps of one percent: that is how often it is worth telling the Zone.
    fn status(&mut self) -> ExtToolsOnlineStatus {
        let step = (self.status.size_bytes / 100).max(1);
        let downloaded = self.job.downloaded_bytes.load(Ordering::Relaxed) / step * step;
        if downloaded != self.status.downloaded_bytes {
            self.status.downloaded_bytes = downloaded;
            self.status.updated_at_ms = now_ms();
        }
        self.status.clone()
    }
}

/// What a tools root needs next for one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Installed and what is asked for.
    Keep,
    /// The right bytes, unpacked from the release: the desired state owns it from now on.
    Adopt,
    /// Not there, or another version.
    Install { platform: String },
    /// Installed, and asked to go.
    Remove,
    /// Not there, and asked not to be.
    Absent,
    /// This root's platform has no payload.
    Unsupported,
}

/// `wanted`: this node's target state. Whatever is enabled and has a payload gets installed; what was installed for the
/// desired state leaves with it; what a release seeded stays until the desired state says otherwise.
pub(crate) fn plan_root(
    wanted: &BTreeMap<String, NodeExtTools>,
    installed: &BTreeMap<String, ExtToolsInstall>,
    payload_of: impl Fn(&NodeExtTools) -> Option<&String>,
) -> BTreeMap<String, Step> {
    let mut steps = BTreeMap::new();
    for (id, entry) in wanted {
        let current = installed.get(id);
        let step = if !entry.enabled {
            if current.is_some() { Step::Remove } else { Step::Absent }
        } else {
            match payload_of(entry) {
                None => Step::Unsupported,
                Some(platform) => match current {
                    Some(install) if install.pikg_digest.as_deref() == Some(entry.pikg_digest.as_str()) => {
                        if install.origin == ExtToolsOrigin::Seed { Step::Adopt } else { Step::Keep }
                    }
                    _ => Step::Install { platform: platform.clone() },
                },
            }
        };
        steps.insert(id.clone(), step);
    }
    for (id, install) in installed {
        if !wanted.contains_key(id) {
            steps.insert(id.clone(), if install.origin == ExtToolsOrigin::Desired { Step::Remove } else { Step::Keep });
        }
    }
    steps
}

/// Failed installs wait, longer each time; a change of what is asked for starts over.
#[derive(Default)]
struct RetryClock {
    failures: HashMap<(String, String, String), (u32, Instant)>,
}

impl RetryClock {
    fn waiting(&self, key: &(String, String, String)) -> bool {
        self.failures.get(key).is_some_and(|(count, at)| at.elapsed() < (RETRY_MIN * 2u32.saturating_pow(*count - 1)).min(RETRY_MAX))
    }

    fn failed(&mut self, key: (String, String, String)) {
        let entry = self.failures.entry(key).or_insert((0, Instant::now()));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = Instant::now();
    }

    fn succeeded(&mut self, key: &(String, String, String)) {
        self.failures.remove(key);
    }
}

fn engine_result<T: DeserializeOwned>(output: &CommandOutput) -> Result<T, String> {
    let line = output.stdout.lines().rev().find(|line| line.trim_start().starts_with('{'));
    let Some(line) = line else {
        return Err(format!("the worker image's engine printed no result: {} {}", output.stdout.trim(), output.stderr.trim()));
    };
    let value: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("engine result: {e}"))?;
    if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(value.get("error").and_then(serde_json::Value::as_str).unwrap_or("the engine failed").to_string());
    }
    serde_json::from_value(value.get("result").cloned().unwrap_or_default()).map_err(|e| format!("engine result: {e}"))
}

/// The tools volume, reached through the worker image.
struct VolumeRoot {
    state: Option<(Instant, BTreeMap<String, ExtToolsInstall>)>,
    online: Option<(Instant, BTreeMap<String, ExtToolsOnlineEntry>)>,
    /// The seed image's payload, for a machine whose release carries no Linux tar, was looked at.
    own_seed_tried: bool,
}

/// Run the worker image's engine against the tools volume. It has no network there.
async fn volume_engine<T: DeserializeOwned>(mount: Option<(&str, &str)>, args: Vec<String>) -> Result<T, String> {
    let worker = VolumeRoot::prepare().await?;
    let mut command = args;
    command.extend(["--root".to_string(), EXTTOOL_MOUNT_ROOT.to_string()]);
    let output = PaiosDocker::new(None).run_tools_engine(worker.as_str(), mount, &command).await.map_err(|e| e.to_string())?;
    engine_result(&output)
}

/// An online item of the tools volume: the archive is downloaded here, where the network is, and the engine in the
/// worker image checks and unpacks it.
async fn volume_online(item: &str, job: &ExtToolsOnlineJob) -> Result<ExtToolsOnlineRecord, String> {
    let step = |command: &str| vec![command.to_string(), "--item".to_string(), item.to_string(), "--request-ms".to_string(), job.request_ms.load(Ordering::Relaxed).to_string()];
    let entry: ExtToolsOnlineEntry = volume_engine(None, step("online-begin")).await?;
    let archive = get_buckyos_root_dir().join(ONLINE_DOWNLOAD_DIR).join(format!("{item}-{}", entry.source.sha256));
    let fetched = ext_tools_online_fetch(&entry.source, &archive, &ExtToolsOnlineLimits::default(), &mut |received| job.downloaded_bytes.store(received, Ordering::Relaxed)).await;
    let placed = match fetched {
        Ok(()) => {
            let mut args = step("online-apply");
            args.extend(["--archive".to_string(), "/in/archive".to_string()]);
            volume_engine(Some((archive.to_string_lossy().as_ref(), "/in/archive")), args).await
        }
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_file(&archive);
    if let Err(error) = &placed {
        let mut args = step("online-fail");
        args.extend(["--error".to_string(), error.clone()]);
        let _ = volume_engine::<ExtToolsOnlineRecord>(None, args).await;
    }
    placed
}

/// Inside the volume: what the seed image copied there.
const VOLUME_SEED_DIR: &str = "/opt/buckyos/tools/seed";

impl VolumeRoot {
    async fn prepare() -> Result<String, String> {
        let root = get_buckyos_root_dir();
        let arch = PlatformTarget::current().arch;
        let worker = paios_images::resolve(PaiosImageKind::Aios, arch, root.as_path())?
            .ok_or_else(|| "this node has no worker image".to_string())?;
        let exttool = paios_images::resolve(PaiosImageKind::Exttool, arch, root.as_path())?;
        let docker = PaiosDocker::new(None);
        docker.ensure_image(&worker).await.map_err(|e| e.to_string())?;
        docker.prepare_exttool(exttool.as_ref(), worker.reference.as_str()).await.map_err(|e| e.to_string())?;
        Ok(worker.reference)
    }

    async fn engine<T: DeserializeOwned>(&mut self, mount: Option<(&str, &str)>, args: Vec<String>) -> Result<T, String> {
        let result = volume_engine(mount, args).await;
        self.forget();
        result
    }

    /// What was read from the volume no longer holds.
    fn forget(&mut self) {
        self.state = None;
        self.online = None;
    }

    /// A worker image from before online items has no answer: the volume has none until the image is replaced.
    async fn online(&mut self) -> BTreeMap<String, ExtToolsOnlineEntry> {
        if let Some((at, online)) = &self.online {
            if at.elapsed() < VOLUME_STATE_TTL {
                return online.clone();
            }
        }
        let online: BTreeMap<String, ExtToolsOnlineEntry> = volume_engine(None, vec!["online".to_string()]).await.unwrap_or_else(|error| {
            warn!("ext_tools {CONTAINER}: online items: {error}");
            BTreeMap::new()
        });
        self.online = Some((Instant::now(), online.clone()));
        online
    }

    async fn installed(&mut self) -> Result<BTreeMap<String, ExtToolsInstall>, String> {
        if let Some((at, state)) = &self.state {
            if at.elapsed() < VOLUME_STATE_TTL {
                return Ok(state.clone());
            }
        }
        let state: BTreeMap<String, ExtToolsInstall> = self.engine(None, vec!["status".to_string()]).await?;
        self.state = Some((Instant::now(), state.clone()));
        Ok(state)
    }
}

enum ToolsTarget {
    Native(ToolsRoot),
    Volume(VolumeRoot),
}

fn skip_smoke() -> bool {
    std::env::var(EXT_TOOLS_SKIP_SMOKE_ENV).is_ok_and(|value| !value.is_empty() && value != "0")
}

fn seed_dir() -> PathBuf {
    get_buckyos_root_dir().join(EXT_TOOLS_PIN_DIR)
}

impl ToolsTarget {
    fn name(&self) -> &'static str {
        match self {
            Self::Native(_) => NATIVE,
            Self::Volume(_) => CONTAINER,
        }
    }

    fn payload_of<'a>(&self, entry: &'a NodeExtTools) -> Option<&'a String> {
        match self {
            Self::Native(_) => entry.payload.as_ref(),
            Self::Volume(_) => entry.container_payload.as_ref(),
        }
    }

    fn platform(&self) -> Option<&'static ExtToolsPlatform> {
        match self {
            Self::Native(_) => ext_tools_current_platform(),
            Self::Volume(_) => ext_tools_container_platform(std::env::consts::ARCH),
        }
    }

    async fn installed(&mut self) -> Result<BTreeMap<String, ExtToolsInstall>, String> {
        match self {
            Self::Native(root) => {
                let root = ToolsRoot::new(root.path());
                tokio::task::spawn_blocking(move || {
                    root.prepare()?;
                    Ok(root.installed())
                })
                .await
                .map_err(|e| e.to_string())?
            }
            Self::Volume(volume) => volume.installed().await,
        }
    }

    /// What `installed` says is really in place, or put back from the version directory it came from. A worker's volume
    /// can only be looked at through a container, and nothing else writes it.
    async fn repair(&self, id: &str, install: Option<&ExtToolsInstall>) -> Result<(), String> {
        let (Self::Native(root), Some(install)) = (self, install) else {
            return Ok(());
        };
        if root.health(id).is_ok() {
            return Ok(());
        }
        warn!("ext_tools native: {id} is damaged; restoring it from {}", install.dir);
        let (root, id, dir, origin) = (ToolsRoot::new(root.path()), id.to_string(), install.dir.clone(), install.origin);
        tokio::task::spawn_blocking(move || root.activate_existing(&id, &dir, origin).map(|_| ())).await.map_err(|e| e.to_string())?
    }

    /// Unpack what the release shipped for packages this root never had. A package the desired state already names is
    /// not seeded: it comes from there, or has been asked to stay away.
    async fn seed(&mut self, named: &BTreeMap<String, NodeExtTools>) -> Result<Vec<ExtToolsInstall>, String> {
        let dir = seed_dir();
        let installed = self.installed().await?;
        if let Self::Native(root) = self {
            let inside = root.path().join("seed");
            if inside.is_dir() {
                let (root, platform, skip) = (ToolsRoot::new(root.path()), self.platform(), skip_smoke());
                if let Some(platform) = platform {
                    return tokio::task::spawn_blocking(move || root.seed(&inside, platform, skip)).await.map_err(|e| e.to_string())?;
                }
            }
        }
        if let Self::Volume(volume) = self {
            if !volume.own_seed_tried && installed.is_empty() && named.is_empty() {
                volume.own_seed_tried = true;
                let mut args = vec!["seed".to_string(), "--dir".to_string(), VOLUME_SEED_DIR.to_string()];
                if skip_smoke() {
                    args.push("--skip-smoke".to_string());
                }
                let seeded: Vec<ExtToolsInstall> = volume.engine(None, args).await?;
                if !seeded.is_empty() {
                    return Ok(seeded);
                }
            }
        }
        let Some(pin) = ExtToolsPinFile::load(&dir)? else {
            return Ok(Vec::new());
        };
        let Some(platform) = self.platform() else {
            return Ok(Vec::new());
        };
        let needed = pin.packages.iter().any(|(id, package)| !installed.contains_key(id) && !named.contains_key(id) && package.platforms.get(platform.name).is_some_and(|p| dir.join(&p.file).is_file()));
        if !needed {
            return Ok(Vec::new());
        }
        self.seed_from(dir).await
    }

    async fn seed_from(&mut self, dir: PathBuf) -> Result<Vec<ExtToolsInstall>, String> {
        let Some(platform) = self.platform() else {
            return Ok(Vec::new());
        };
        match self {
            Self::Native(root) => {
                let root = ToolsRoot::new(root.path());
                let skip = skip_smoke();
                tokio::task::spawn_blocking(move || root.seed(&dir, platform, skip)).await.map_err(|e| e.to_string())?
            }
            Self::Volume(volume) => {
                let host = dir.to_string_lossy().to_string();
                let mut args = vec!["seed".to_string(), "--dir".to_string(), "/in".to_string()];
                if skip_smoke() {
                    args.push("--skip-smoke".to_string());
                }
                volume.engine(Some((host.as_str(), "/in")), args).await
            }
        }
    }

    async fn activate_kept(&mut self, id: &str, entry: &NodeExtTools) -> Result<Option<ExtToolsInstall>, String> {
        match self {
            Self::Native(root) => {
                let root = ToolsRoot::new(root.path());
                let (id, digest) = (id.to_string(), entry.pikg_digest.clone());
                tokio::task::spawn_blocking(move || match root.find_version(&id, &digest) {
                    Some(dir) => root.activate_existing(&id, &dir, ExtToolsOrigin::Desired).map(Some),
                    None => Ok(None),
                })
                .await
                .map_err(|e| e.to_string())?
            }
            Self::Volume(volume) => {
                let result: Result<ExtToolsInstall, String> = volume
                    .engine(None, vec!["activate".to_string(), "--id".to_string(), id.to_string(), "--pikg-digest".to_string(), entry.pikg_digest.clone()])
                    .await;
                match result {
                    Ok(install) => Ok(Some(install)),
                    Err(error) if error.contains("not here") => Ok(None),
                    Err(error) => Err(error),
                }
            }
        }
    }

    async fn install(&mut self, tar: &Path, id: &str, entry: &NodeExtTools, platform: &'static ExtToolsPlatform) -> Result<ExtToolsInstall, String> {
        match self {
            Self::Native(root) => {
                let root = ToolsRoot::new(root.path());
                let (tar, id, digest) = (tar.to_path_buf(), id.to_string(), entry.pikg_digest.clone());
                tokio::task::spawn_blocking(move || {
                    root.install(
                        &ExtToolsApply { tar: &tar, id: &id, expected_digest: None, pikg_digest: Some(&digest), platform, origin: ExtToolsOrigin::Desired, skip_smoke: false },
                        &mut |_| {},
                    )
                })
                .await
                .map_err(|e| e.to_string())?
            }
            Self::Volume(volume) => {
                let host = tar.to_string_lossy().to_string();
                let args = vec![
                    "apply".to_string(),
                    "--tar".to_string(),
                    "/in/payload.tar.gz".to_string(),
                    "--id".to_string(),
                    id.to_string(),
                    "--pikg-digest".to_string(),
                    entry.pikg_digest.clone(),
                ];
                volume.engine(Some((host.as_str(), "/in/payload.tar.gz")), args).await
            }
        }
    }

    async fn adopt(&mut self, id: &str, digest: &str) -> Result<(), String> {
        match self {
            Self::Native(root) => root.adopt(id, Some(digest)),
            Self::Volume(volume) => volume
                .engine::<serde_json::Value>(None, vec!["adopt".to_string(), "--id".to_string(), id.to_string(), "--pikg-digest".to_string(), digest.to_string()])
                .await
                .map(|_| ()),
        }
    }

    async fn remove(&mut self, id: &str) -> Result<(), String> {
        match self {
            Self::Native(root) => {
                let root = ToolsRoot::new(root.path());
                let id = id.to_string();
                tokio::task::spawn_blocking(move || root.remove(&id)).await.map_err(|e| e.to_string())?
            }
            Self::Volume(volume) => volume.engine::<serde_json::Value>(None, vec!["remove".to_string(), "--id".to_string(), id.to_string()]).await.map(|_| ()),
        }
    }

    /// The online items of the packages active on this root.
    async fn online(&mut self) -> Result<BTreeMap<String, ExtToolsOnlineEntry>, String> {
        match self {
            Self::Native(root) => {
                let root = ToolsRoot::new(root.path());
                tokio::task::spawn_blocking(move || root.online()).await.map_err(|e| e.to_string())
            }
            Self::Volume(volume) => Ok(volume.online().await),
        }
    }

    async fn online_fail(&mut self, item: &str, error: &str) -> Result<ExtToolsOnlineRecord, String> {
        match self {
            Self::Native(root) => {
                let (root, item, error) = (ToolsRoot::new(root.path()), item.to_string(), error.to_string());
                tokio::task::spawn_blocking(move || root.online_fail(&item, &error, 0)).await.map_err(|e| e.to_string())?
            }
            Self::Volume(volume) => volume.engine(None, vec!["online-fail".to_string(), "--item".to_string(), item.to_string(), "--error".to_string(), error.to_string()]).await,
        }
    }

    /// Start a download and come back at once: it runs beside the reconciler, which only looks at how far it is.
    fn start_online(&self, item: &str, job: Arc<ExtToolsOnlineJob>) -> tokio::task::JoinHandle<()> {
        let (name, item) = (self.name(), item.to_string());
        let root = match self {
            Self::Native(root) => Some(root.path().to_path_buf()),
            Self::Volume(_) => None,
        };
        tokio::spawn(async move {
            let done = match root {
                Some(root) => ext_tools_online_install(&root, &item, &ExtToolsOnlineLimits::default(), &job).await,
                None => volume_online(&item, &job).await,
            };
            match done {
                Ok(record) => info!("ext_tools {name}: online item {item} {} is ready", record.version),
                Err(error) => warn!("ext_tools {name}: online item {item}: {error}"),
            }
        })
    }

    /// A download ended: what the volume said before no longer holds.
    fn online_ended(&mut self) {
        if let Self::Volume(volume) = self {
            volume.forget();
        }
    }
}

/// The payload of `platform` out of the `.pikg` the Repo keeps, to a file. The named store checks the bytes against the
/// object id and the reader against `pikg_digest`, so this is the same package the administrator chose, wherever it came
/// from.
async fn fetch_payload(entry: &NodeExtTools, platform: &ExtToolsPlatform, dest: &Path) -> Result<(), String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
    let store = runtime.get_local_named_store().await.map_err(|e| format!("named store: {e}"))?;
    let file_id = ObjId::new(&entry.content_id).map_err(|e| format!("{}: {e}", entry.content_id))?;
    let reader = PikgReader::open_named(store, &file_id, Some(&entry.pikg_digest)).await.map_err(|e| format!("open {}: {e}", entry.content_id))?;
    let inspection = reader.inspection();
    let desc = inspection.app_doc.pkg_list.get(platform.key).ok_or_else(|| format!("the package has no payload for {}", platform.name))?;
    let meta_id = desc.pkg_objid.as_ref().ok_or_else(|| format!("{}: no package meta", platform.key))?.to_string();
    let content = inspection
        .package_meta
        .package_objects
        .get(&meta_id)
        .and_then(|meta| meta.get("content"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{}: no content", platform.key))?
        .to_string();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    reader.copy_content_to_file(&content, dest).await.map_err(|e| format!("read {}: {e}", platform.key))
}

/// A bare Linux host has no container program and only ever the native root.
fn container_program_present() -> bool {
    if matches!(crate::wslc::program(), Ok(Some(_))) {
        return true;
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(if cfg!(windows) { "docker.exe" } else { "docker" }).is_file()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// One package across a node's roots. An upgrade in progress beats a root that is done; a failure beats everything.
fn summarize(version: String, targets: BTreeMap<String, ExtToolsTargetStatus>) -> ExtToolsNodeStatus {
    let rank = |stage: ExtToolsStage| match stage {
        ExtToolsStage::Failed => 0,
        ExtToolsStage::Queued | ExtToolsStage::Downloading | ExtToolsStage::Verifying | ExtToolsStage::Staging | ExtToolsStage::Checking | ExtToolsStage::Removing => 1,
        ExtToolsStage::Active => 2,
        ExtToolsStage::Removed => 3,
        ExtToolsStage::Unsupported => 4,
    };
    let stage = targets.values().map(|t| t.stage).min_by_key(|s| rank(*s)).unwrap_or(ExtToolsStage::Removed);
    let error = targets.iter().find_map(|(name, t)| t.error.as_ref().map(|error| format!("{name}: {error}")));
    // An online item across the roots: the one furthest from ready, a failure before everything.
    let online_rank = |state: ExtToolsOnlineState| match state {
        ExtToolsOnlineState::Failed => 0,
        ExtToolsOnlineState::Downloading => 1,
        ExtToolsOnlineState::Missing => 2,
        ExtToolsOnlineState::Ready => 3,
    };
    let mut online: BTreeMap<String, ExtToolsOnlineStatus> = BTreeMap::new();
    for (item, status) in targets.values().flat_map(|t| t.online.iter()) {
        if online.get(item).is_none_or(|known| online_rank(status.state) < online_rank(known.state)) {
            online.insert(item.clone(), status.clone());
        }
    }
    ExtToolsNodeStatus { version, stage, targets, error, online, at_ms: now_ms() }
}

fn target_status(stage: ExtToolsStage, version: Option<String>, digest: Option<String>, error: Option<String>) -> ExtToolsTargetStatus {
    ExtToolsTargetStatus { stage, version, digest, error, online: BTreeMap::new(), waiting: BTreeMap::new() }
}

struct Reconciler {
    node_id: String,
    targets: Vec<ToolsTarget>,
    retry: RetryClock,
    reported: HashMap<String, ExtToolsNodeStatus>,
    /// By root and online item.
    downloads: HashMap<(String, String), OnlineDownload>,
}

impl Reconciler {
    fn new(node_id: String) -> Self {
        Self { node_id, targets: Vec::new(), retry: RetryClock::default(), reported: HashMap::new(), downloads: HashMap::new() }
    }

    /// The machine always has its root. The containers need one of their own only when their volume is not that directory.
    fn ensure_targets(&mut self) {
        if self.targets.is_empty() {
            self.targets.push(ToolsTarget::Native(ToolsRoot::new(get_buckyos_root_dir().join("tools"))));
        }
        let device_runs_containers = get_buckyos_api_runtime().ok().and_then(|r| r.device_config.as_ref().map(|d| d.support_container)).unwrap_or(true);
        let separate = crate::product::containers_enabled() && device_runs_containers && paios_images::tools_volume_device().is_none() && container_program_present();
        if separate && !self.targets.iter().any(|t| matches!(t, ToolsTarget::Volume(_))) {
            self.targets.push(ToolsTarget::Volume(VolumeRoot { state: None, online: None, own_seed_tried: false }));
        }
    }

    /// What the administrator asked this node to download again, by online item.
    async fn load_requests(&self) -> BTreeMap<String, ExtToolsOnlineRequest> {
        let mut requests = BTreeMap::new();
        let Ok(runtime) = get_buckyos_api_runtime() else { return requests };
        let Ok(client) = runtime.get_system_config_client().await else { return requests };
        for item in client.list(ext_tools_online_request_prefix(&self.node_id).as_str()).await.unwrap_or_default() {
            if let Ok(value) = client.get(ext_tools_online_request_key(&self.node_id, &item).as_str()).await {
                if let Ok(request) = serde_json::from_str(&value.value) {
                    requests.insert(item, request);
                }
            }
        }
        requests
    }

    async fn load_config(&self) -> Option<NodeConfig> {
        let runtime = get_buckyos_api_runtime().ok()?;
        let client = runtime.get_system_config_client().await.ok()?;
        let value = client.get(buckyos_api::node_config_key(&self.node_id).as_str()).await.ok()?.value;
        serde_json::from_str(&value).ok()
    }

    async fn report(&mut self, id: &str, status: Option<ExtToolsNodeStatus>) {
        let same = match (&status, self.reported.get(id)) {
            (Some(new), Some(old)) => new.version == old.version && new.stage == old.stage && new.targets == old.targets,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        let Ok(runtime) = get_buckyos_api_runtime() else { return };
        let Ok(client) = runtime.get_system_config_client().await else { return };
        let key = ext_tools_node_status_key(&self.node_id, id);
        let written = match &status {
            Some(status) => client.set(key.as_str(), serde_json::to_string(status).unwrap_or_default().as_str()).await.map(|_| ()),
            None => client.delete(key.as_str()).await.map(|_| ()),
        };
        if let Err(error) = written {
            warn!("report {key}: {error}");
            return;
        }
        match status {
            Some(status) => {
                self.reported.insert(id.to_string(), status);
            }
            None => {
                self.reported.remove(id);
            }
        }
    }

    async fn tick(&mut self) {
        // No config is not an empty config: the Zone may just be unreachable, and only the seed goes ahead without it.
        let config = self.load_config().await;
        self.ensure_targets();
        let mut targets = std::mem::take(&mut self.targets);
        let Some(wanted) = config.map(|c| c.ext_tools) else {
            for target in targets.iter_mut() {
                self.seed_target(target, &BTreeMap::new()).await;
                self.reconcile_online(target, &BTreeMap::new(), &mut BTreeMap::new()).await;
            }
            self.targets = targets;
            first_seed_done();
            return;
        };
        let requests = self.load_requests().await;
        let mut per_package: BTreeMap<String, BTreeMap<String, ExtToolsTargetStatus>> = BTreeMap::new();
        for target in targets.iter_mut() {
            self.reconcile_target(target, &wanted, &mut per_package).await;
            self.reconcile_online(target, &requests, &mut per_package).await;
        }
        self.targets = targets;
        first_seed_done();
        let mut seen: Vec<String> = Vec::new();
        for (id, states) in per_package {
            let version = wanted
                .get(&id)
                .map(|e| e.version.clone())
                .or_else(|| states.values().find_map(|s| s.version.clone()))
                .unwrap_or_default();
            let status = summarize(version, states);
            seen.push(id.clone());
            if status.stage == ExtToolsStage::Removed {
                self.report(&id, None).await;
            } else {
                self.report(&id, Some(status)).await;
            }
        }
        let gone: Vec<String> = self.reported.keys().filter(|id| !seen.contains(id)).cloned().collect();
        for id in gone {
            self.report(&id, None).await;
        }
        let download_dir = get_buckyos_root_dir().join(DOWNLOAD_DIR);
        let _ = std::fs::remove_dir_all(download_dir);
    }

    async fn seed_target(&mut self, target: &mut ToolsTarget, wanted: &BTreeMap<String, NodeExtTools>) {
        let key = (target.name().to_string(), "seed".to_string(), String::new());
        if self.retry.waiting(&key) {
            return;
        }
        match target.seed(wanted).await {
            Ok(seeded) => {
                for install in seeded {
                    info!("ext_tools {}: seeded {} {}", target.name(), install.id, install.version);
                }
                self.retry.succeeded(&key);
            }
            Err(error) => {
                warn!("ext_tools {}: seed failed: {error}", target.name());
                self.retry.failed(key);
            }
        }
    }

    async fn reconcile_target(
        &mut self,
        target: &mut ToolsTarget,
        wanted: &BTreeMap<String, NodeExtTools>,
        per_package: &mut BTreeMap<String, BTreeMap<String, ExtToolsTargetStatus>>,
    ) {
        let name = target.name();
        let failed_root = |error: String, per_package: &mut BTreeMap<String, BTreeMap<String, ExtToolsTargetStatus>>| {
            warn!("ext_tools {name}: {error}");
            for id in wanted.keys() {
                per_package.entry(id.clone()).or_default().insert(name.to_string(), target_status(ExtToolsStage::Failed, None, None, Some(error.clone())));
            }
        };
        self.seed_target(target, wanted).await;
        let root_key = (name.to_string(), "root".to_string(), String::new());
        if self.retry.waiting(&root_key) {
            failed_root("the tools root was unavailable; retrying later".to_string(), per_package);
            return;
        }
        let mut installed = match target.installed().await {
            Ok(installed) => {
                self.retry.succeeded(&root_key);
                installed
            }
            Err(error) => {
                self.retry.failed(root_key);
                failed_root(error, per_package);
                return;
            }
        };
        let steps = plan_root(wanted, &installed, |entry| target.payload_of(entry));
        for (id, step) in steps {
            let entry = wanted.get(&id);
            let current = installed.get(&id);
            let mut state = target_status(
                ExtToolsStage::Active,
                current.map(|i| i.version.clone()),
                current.and_then(|i| i.pikg_digest.clone().or_else(|| Some(i.payload_digest.clone()))),
                None,
            );
            state.waiting = current.map(|i| i.waiting.clone()).unwrap_or_default();
            match step {
                Step::Keep => {
                    if let Err(problem) = target.repair(&id, current).await {
                        warn!("ext_tools {name}: {id} is broken and could not be repaired: {problem}");
                        state.stage = ExtToolsStage::Failed;
                        state.error = Some(problem);
                    }
                }
                Step::Adopt => {
                    let digest = entry.map(|e| e.pikg_digest.clone()).unwrap_or_default();
                    if let Err(error) = target.adopt(&id, &digest).await {
                        warn!("ext_tools {name}: adopt {id}: {error}");
                    }
                }
                Step::Install { platform } => {
                    if let Some(entry) = entry {
                        self.install_one(target, &id, entry, &platform, &mut state, &mut installed).await;
                    }
                }
                Step::Remove => {
                    let key = (name.to_string(), id.clone(), "remove".to_string());
                    if self.retry.waiting(&key) {
                        state.stage = ExtToolsStage::Failed;
                        state.error = Some("removal failed; retrying later".to_string());
                    } else {
                        match target.remove(&id).await {
                            Ok(()) => {
                                info!("ext_tools {name}: removed {id}");
                                self.retry.succeeded(&key);
                                installed.remove(&id);
                                state = target_status(ExtToolsStage::Removed, None, None, None);
                            }
                            Err(error) => {
                                warn!("ext_tools {name}: remove {id}: {error}");
                                self.retry.failed(key);
                                state.stage = ExtToolsStage::Failed;
                                state.error = Some(error);
                            }
                        }
                    }
                }
                Step::Absent => {
                    state = target_status(ExtToolsStage::Removed, None, None, None);
                }
                Step::Unsupported => {
                    state.stage = ExtToolsStage::Unsupported;
                }
            }
            per_package.entry(id).or_default().insert(name.to_string(), state);
        }
    }

    /// The online items of one root: start what is due, and say where each one is. Nothing here waits for a download.
    async fn reconcile_online(
        &mut self,
        target: &mut ToolsTarget,
        requests: &BTreeMap<String, ExtToolsOnlineRequest>,
        per_package: &mut BTreeMap<String, BTreeMap<String, ExtToolsTargetStatus>>,
    ) {
        let name = target.name();
        let ended: Vec<(String, String)> = self.downloads.iter().filter(|((root, _), download)| root == name && download.task.is_finished()).map(|(key, _)| key.clone()).collect();
        for key in ended {
            self.downloads.remove(&key);
            target.online_ended();
        }
        let entries = match target.online().await {
            Ok(entries) => entries,
            Err(error) => {
                warn!("ext_tools {name}: online items: {error}");
                return;
            }
        };
        for (item, entry) in entries {
            let key = (name.to_string(), item.clone());
            let request = requests.get(&item);
            let mut status = entry.status.clone();
            if let Some(download) = self.downloads.get_mut(&key) {
                // Asked again while it is on its way: this download is the answer.
                download.job.request_ms.fetch_max(request.map(|r| r.request_ms).unwrap_or(0), Ordering::Relaxed);
                status = download.status();
            } else {
                match plan_online(&entry, request) {
                    OnlineStep::Keep => {}
                    OnlineStep::Interrupted => match target.online_fail(&item, INTERRUPTED).await {
                        Ok(record) => status = record.status(),
                        Err(error) => warn!("ext_tools {name}: online item {item}: {error}"),
                    },
                    OnlineStep::Download { request_ms } => {
                        info!("ext_tools {name}: downloading online item {item} {} of {} from {}", entry.version, entry.package, entry.source.url);
                        status = ExtToolsOnlineStatus { state: ExtToolsOnlineState::Downloading, downloaded_bytes: 0, error: None, updated_at_ms: now_ms(), ..status };
                        let job = Arc::new(ExtToolsOnlineJob::new(request_ms));
                        let task = target.start_online(&item, job.clone());
                        self.downloads.insert(key, OnlineDownload { package: entry.package.clone(), job, task, status: status.clone() });
                    }
                }
            }
            if let Some(state) = per_package.get_mut(&entry.package).and_then(|targets| targets.get_mut(name)) {
                state.online.insert(item, status);
            }
        }
    }

    /// Between rounds: how far the downloads are, for the Zone to show. `true`: one ended, and the next round is due.
    async fn report_downloads(&mut self) -> bool {
        let ended = self.downloads.values().any(|download| download.task.is_finished());
        let running: Vec<(String, String, String, ExtToolsOnlineStatus)> = self
            .downloads
            .iter_mut()
            .filter(|(_, download)| !download.task.is_finished())
            .map(|((root, item), download)| (download.package.clone(), root.clone(), item.clone(), download.status()))
            .collect();
        for (package, root, item, status) in running {
            let Some(mut reported) = self.reported.get(&package).cloned() else { continue };
            let Some(state) = reported.targets.get_mut(&root) else { continue };
            if state.online.get(&item) != Some(&status) {
                state.online.insert(item, status);
                self.report(&package, Some(summarize(reported.version, reported.targets))).await;
            }
        }
        ended
    }

    async fn install_one(
        &mut self,
        target: &mut ToolsTarget,
        id: &str,
        entry: &NodeExtTools,
        platform_name: &str,
        state: &mut ExtToolsTargetStatus,
        installed: &mut BTreeMap<String, ExtToolsInstall>,
    ) {
        let name = target.name();
        let key = (name.to_string(), id.to_string(), entry.pikg_digest.clone());
        if self.retry.waiting(&key) {
            state.stage = ExtToolsStage::Failed;
            state.error = Some(format!("{} failed; retrying later", entry.version));
            return;
        }
        let Some(platform) = ext_tools_platform(platform_name) else {
            state.stage = ExtToolsStage::Unsupported;
            return;
        };
        let attempt = async {
            if let Some(install) = target.activate_kept(id, entry).await? {
                return Ok::<ExtToolsInstall, String>(install);
            }
            self.report_progress(id, &entry.version, name, ExtToolsStage::Downloading, state).await;
            let dest = get_buckyos_root_dir().join(DOWNLOAD_DIR).join(format!("{id}-{platform_name}-{}.tar.gz", entry.pikg_digest.trim_start_matches("sha256:")));
            if !dest.is_file() {
                fetch_payload(entry, platform, &dest).await?;
            }
            self.report_progress(id, &entry.version, name, ExtToolsStage::Staging, state).await;
            target.install(&dest, id, entry, platform).await
        };
        match attempt.await {
            Ok(install) => {
                info!("ext_tools {name}: {id} {} active", install.version);
                self.retry.succeeded(&key);
                state.stage = ExtToolsStage::Active;
                state.version = Some(install.version.clone());
                state.digest = Some(entry.pikg_digest.clone());
                state.error = None;
                state.waiting = install.waiting.clone();
                installed.insert(id.to_string(), install);
            }
            Err(error) => {
                warn!("ext_tools {name}: {id} {}: {error}", entry.version);
                self.retry.failed(key);
                state.stage = ExtToolsStage::Failed;
                state.error = Some(error);
            }
        }
    }

    async fn report_progress(&mut self, id: &str, version: &str, target: &str, stage: ExtToolsStage, state: &ExtToolsTargetStatus) {
        let mut targets = self.reported.get(id).map(|s| s.targets.clone()).unwrap_or_default();
        targets.insert(target.to_string(), ExtToolsTargetStatus { stage, ..state.clone() });
        self.report(id, Some(summarize(version.to_string(), targets))).await;
    }
}

static FIRST_SEED: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();

fn first_seed_done() {
    if let Some(sender) = FIRST_SEED.get() {
        sender.send_replace(true);
    }
}

/// Until the reconciler has been through its first round, or `limit`. No reconciler (a test, a machine in the middle of
/// activation) means there is nothing to wait for.
pub(crate) async fn wait_first_seed(limit: Duration) {
    let Some(sender) = FIRST_SEED.get() else { return };
    let mut receiver = sender.subscribe();
    if tokio::time::timeout(limit, receiver.wait_for(|done| *done)).await.is_err() {
        warn!("ext_tools: the seed is taking longer than {}s; starting without it", limit.as_secs());
    }
}

/// Runs for the life of the node. Starts with the seed, which needs nothing from the Zone.
pub(crate) fn spawn_reconciler(node_id: String) {
    if FIRST_SEED.set(tokio::sync::watch::channel(false).0).is_err() {
        return;
    }
    tokio::spawn(async move {
        let mut reconciler = Reconciler::new(node_id);
        loop {
            reconciler.tick().await;
            let round = Instant::now();
            while round.elapsed() < TICK {
                if reconciler.downloads.is_empty() {
                    tokio::time::sleep(TICK.saturating_sub(round.elapsed())).await;
                } else {
                    tokio::time::sleep(PROGRESS_TICK).await;
                    if reconciler.report_downloads().await {
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(enabled: bool, payload: Option<&str>) -> NodeExtTools {
        NodeExtTools {
            version: "1.1.0".to_string(),
            pikg_digest: "sha256:new".to_string(),
            content_id: "file:new".to_string(),
            enabled,
            payload: payload.map(str::to_string),
            container_payload: None,
        }
    }

    fn install(digest: Option<&str>, origin: ExtToolsOrigin) -> ExtToolsInstall {
        ExtToolsInstall {
            schema: EXT_TOOLS_INSTALL_SCHEMA.to_string(),
            id: "ext-core".to_string(),
            version: "1.0.0".to_string(),
            payload_digest: "sha256:payload".to_string(),
            pikg_digest: digest.map(str::to_string),
            platform: "linux-x86_64".to_string(),
            origin,
            dir: "1.0.0-abc".to_string(),
            commands: BTreeMap::new(),
            waiting: BTreeMap::new(),
            launchers: vec![],
            smoke: ExtToolsSmokeResult::Passed,
            activated_at_ms: 1,
            previous: None,
        }
    }

    fn plan(wanted: Option<NodeExtTools>, installed: Option<ExtToolsInstall>) -> Step {
        let wanted: BTreeMap<_, _> = wanted.map(|w| ("ext-core".to_string(), w)).into_iter().collect();
        let installed: BTreeMap<_, _> = installed.map(|i| ("ext-core".to_string(), i)).into_iter().collect();
        plan_root(&wanted, &installed, |e| e.payload.as_ref()).remove("ext-core").unwrap_or(Step::Absent)
    }

    #[test]
    fn a_root_converges_on_what_is_asked_and_seeds_stay_until_told_otherwise() {
        let platform = || Step::Install { platform: "linux-x86_64".to_string() };
        assert_eq!(plan(Some(wanted(true, Some("linux-x86_64"))), None), platform());
        assert_eq!(plan(Some(wanted(true, Some("linux-x86_64"))), Some(install(Some("sha256:old"), ExtToolsOrigin::Desired))), platform(), "an upgrade");
        assert_eq!(plan(Some(wanted(true, Some("linux-x86_64"))), Some(install(Some("sha256:new"), ExtToolsOrigin::Desired))), Step::Keep);
        assert_eq!(plan(Some(wanted(true, Some("linux-x86_64"))), Some(install(Some("sha256:new"), ExtToolsOrigin::Seed))), Step::Adopt, "the same package from the release");
        assert_eq!(plan(Some(wanted(true, Some("linux-x86_64"))), Some(install(Some("sha256:old"), ExtToolsOrigin::Seed))), platform(), "another version than the release's");
        assert_eq!(plan(Some(wanted(false, Some("linux-x86_64"))), Some(install(Some("sha256:new"), ExtToolsOrigin::Seed))), Step::Remove, "disabled means gone, seeded or not");
        assert_eq!(plan(Some(wanted(false, Some("linux-x86_64"))), None), Step::Absent);
        assert_eq!(plan(Some(wanted(true, None)), None), Step::Unsupported);
        assert_eq!(plan(None, Some(install(None, ExtToolsOrigin::Seed))), Step::Keep, "no opinion in the Zone: the seed stays");
        assert_eq!(plan(None, Some(install(None, ExtToolsOrigin::Desired))), Step::Remove, "dropped from the Zone");
    }

    #[test]
    fn a_failure_waits_longer_each_time_and_success_forgets_it() {
        let mut clock = RetryClock::default();
        let key = ("native".to_string(), "ext-core".to_string(), "sha256:new".to_string());
        assert!(!clock.waiting(&key));
        clock.failed(key.clone());
        assert!(clock.waiting(&key));
        clock.succeeded(&key);
        assert!(!clock.waiting(&key));
    }

    #[test]
    fn the_status_of_a_package_is_its_worst_root() {
        let state = |stage| target_status(stage, Some("1.0.0".to_string()), None, None);
        let status = |stages: &[(&str, ExtToolsStage)]| summarize("1.0.0".to_string(), stages.iter().map(|(n, s)| (n.to_string(), state(*s))).collect());
        assert_eq!(status(&[(NATIVE, ExtToolsStage::Active), (CONTAINER, ExtToolsStage::Active)]).stage, ExtToolsStage::Active);
        assert_eq!(status(&[(NATIVE, ExtToolsStage::Active), (CONTAINER, ExtToolsStage::Downloading)]).stage, ExtToolsStage::Downloading);
        assert_eq!(status(&[(NATIVE, ExtToolsStage::Failed), (CONTAINER, ExtToolsStage::Downloading)]).stage, ExtToolsStage::Failed);
        assert_eq!(status(&[(NATIVE, ExtToolsStage::Unsupported), (CONTAINER, ExtToolsStage::Active)]).stage, ExtToolsStage::Active, "a Mac with Docker");
        assert_eq!(status(&[(NATIVE, ExtToolsStage::Unsupported)]).stage, ExtToolsStage::Unsupported);
    }

    fn online_status(state: ExtToolsOnlineState) -> ExtToolsOnlineStatus {
        ExtToolsOnlineStatus { state, version: "159.0".to_string(), size_bytes: 1000, downloaded_bytes: 0, error: None, updated_at_ms: 1 }
    }

    fn online_entry(state: ExtToolsOnlineState, auto: bool, answered: u64) -> ExtToolsOnlineEntry {
        ExtToolsOnlineEntry {
            id: "chromium".to_string(),
            package: "ext-core".to_string(),
            version: "159.0".to_string(),
            auto,
            platform: "linux-x86_64".to_string(),
            source: buckyos_api::ExtToolsOnlineSource {
                url: "https://storage.googleapis.com/chrome.zip".to_string(),
                sha256: "ab".repeat(32),
                size_bytes: 1000,
                format: "zip".to_string(),
                into: None,
                rename: vec![],
                overlay: None,
            },
            status: online_status(state),
            request_ms: answered,
        }
    }

    #[test]
    fn an_online_item_downloads_once_by_itself_and_again_only_when_asked() {
        use ExtToolsOnlineState::*;
        let asked = |at| ExtToolsOnlineRequest { package: "ext-core".to_string(), request_ms: at, by: "root".to_string() };
        let download = |request_ms| OnlineStep::Download { request_ms };
        assert_eq!(plan_online(&online_entry(Missing, true, 0), None), download(0), "its package is there and it never was");
        assert_eq!(plan_online(&online_entry(Missing, false, 0), None), OnlineStep::Keep, "not automatic: it waits to be asked for");
        assert_eq!(plan_online(&online_entry(Missing, false, 0), Some(&asked(5))), download(5));
        assert_eq!(plan_online(&online_entry(Failed, true, 0), None), OnlineStep::Keep, "a failure is not retried, after a restart either");
        assert_eq!(plan_online(&online_entry(Failed, true, 0), Some(&asked(5))), download(5), "asked again");
        assert_eq!(plan_online(&online_entry(Failed, true, 5), Some(&asked(5))), OnlineStep::Keep, "that request is the one that failed");
        assert_eq!(plan_online(&online_entry(Ready, true, 5), Some(&asked(5))), OnlineStep::Keep);
        assert_eq!(plan_online(&online_entry(Ready, true, 5), Some(&asked(9))), download(9), "asked for again, though it is there");
        assert_eq!(plan_online(&online_entry(Downloading, true, 0), None), OnlineStep::Interrupted, "nothing of this process downloads it: the node stopped in the middle");
        assert_eq!(plan_online(&online_entry(Downloading, true, 0), Some(&asked(5))), download(5));
        assert_eq!(plan_online(&online_entry(Missing, true, 9), Some(&asked(5))), download(5), "another version of an item whose request was answered");
    }

    #[test]
    fn an_online_item_of_a_node_is_the_one_furthest_from_ready() {
        use ExtToolsOnlineState::*;
        let node = |states: &[(&str, ExtToolsOnlineState)]| {
            let targets = states
                .iter()
                .map(|(name, state)| {
                    let mut target = target_status(ExtToolsStage::Active, Some("1.0.0".to_string()), None, None);
                    target.online.insert("chromium".to_string(), online_status(*state));
                    (name.to_string(), target)
                })
                .collect();
            let status = summarize("1.0.0".to_string(), targets);
            assert_eq!(status.stage, ExtToolsStage::Active, "an online item that is not there does not break its package");
            status.online["chromium"].state
        };
        assert_eq!(node(&[(NATIVE, Ready), (CONTAINER, Ready)]), Ready);
        assert_eq!(node(&[(NATIVE, Ready), (CONTAINER, Missing)]), Missing);
        assert_eq!(node(&[(NATIVE, Missing), (CONTAINER, Downloading)]), Downloading);
        assert_eq!(node(&[(NATIVE, Downloading), (CONTAINER, Failed)]), Failed);
        let plain = summarize("1.0.0".to_string(), BTreeMap::from([(NATIVE.to_string(), target_status(ExtToolsStage::Active, None, None, None))]));
        assert!(serde_json::to_value(&plain).unwrap().get("online").is_none(), "a package without online items reports as before");
    }

    #[tokio::test]
    async fn a_download_tells_its_progress_in_steps_of_one_percent() {
        let job = Arc::new(ExtToolsOnlineJob::new(0));
        let mut download = OnlineDownload { package: "ext-core".to_string(), job: job.clone(), task: tokio::spawn(async {}), status: online_status(ExtToolsOnlineState::Downloading) };
        job.downloaded_bytes.store(9, Ordering::Relaxed);
        assert_eq!(download.status(), online_status(ExtToolsOnlineState::Downloading), "less than a percent: nothing new to tell");
        job.downloaded_bytes.store(25, Ordering::Relaxed);
        let status = download.status();
        assert_eq!(status.downloaded_bytes, 20);
        assert!(status.updated_at_ms > 1);
    }

    #[test]
    fn a_failed_engine_run_says_why() {
        let output = |stdout: &str| CommandOutput { command_line: String::new(), status: std::process::ExitStatus::default(), stdout: stdout.to_string(), stderr: "stderr".to_string() };
        let failed = engine_result::<serde_json::Value>(&output("noise\n{\"ok\":false,\"error\":\"smoke failed\"}\n")).unwrap_err();
        assert_eq!(failed, "smoke failed");
        assert!(engine_result::<serde_json::Value>(&output("not json at all")).unwrap_err().contains("not json at all"));
        let state: BTreeMap<String, ExtToolsInstall> = engine_result(&output("{\"ok\":true,\"result\":{}}")).unwrap();
        assert!(state.is_empty());
    }
}
