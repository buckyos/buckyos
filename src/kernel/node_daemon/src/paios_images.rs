//! paios image pinning (`bin/paios-images/paios_images.json`).
//!
//! Release payloads ship a manifest that pins the worker (`aios`) and ExtTool
//! (`exttool`) images by identity, plus the matching `docker save` tarballs in
//! the same directory. Without the manifest node_daemon runs in local mode and
//! uses `<repo>:latest-<arch>` from the local Docker store. Neither mode pulls.

use crate::app_loader::{
    docker_image_absent, ensure_success, run_docker_with, CommandOutput, DockerCommandRunner,
    PlatformArch, PlatformTarget,
};
use crate::run_item::{ControlRuntItemErrors, Result as RunResult};
use buckyos_kit::get_buckyos_root_dir;
use log::{error, info, warn};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

pub(crate) const PAIOS_IMAGES_DIR: &str = "bin/paios-images";
pub(crate) const PAIOS_IMAGES_MANIFEST: &str = "paios_images.json";
pub(crate) const PAIOS_IMAGES_SCHEMA: &str = "buckyos.paios-images/v1";
pub(crate) const IMAGE_REVISION_LABEL: &str = "org.opencontainers.image.revision";
const DEVENV_JSON_RELATIVE_PATH: &str = "etc/devenv.json";
const LOADED_STATE_RELATIVE_PATH: &str = "local/node_daemon/paios_images_state.json";
pub(crate) const EXTTOOL_VOLUME_PREFIX: &str = "buckyos-exttool";
const EXTTOOL_MOUNT_ROOT: &str = "/opt/buckyos/tools";
const EXPECTATION_TTL: Duration = Duration::from_secs(30);
const PRELOAD_MAX_BACKOFF: Duration = Duration::from_secs(300);

static IMAGE_LOCKS: OnceLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();
static EXTTOOL_PREPARE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static SEEDED_VOLUMES: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();
type TarVerification = (
    u64,
    Option<std::time::SystemTime>,
    String,
    std::result::Result<(), String>,
);
static VERIFIED_TARS: OnceLock<std::sync::Mutex<HashMap<PathBuf, TarVerification>>> =
    OnceLock::new();
static EXPECTATIONS: OnceLock<std::sync::Mutex<HashMap<String, (Instant, WorkerExpectation)>>> =
    OnceLock::new();

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PaiosImagesManifest {
    pub schema: String,
    pub buckyos_revision: String,
    pub images: HashMap<String, PaiosImage>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PaiosImage {
    pub repo: String,
    pub version: String,
    pub revision: String,
    pub platforms: HashMap<String, PinnedPlatform>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct PinnedPlatform {
    #[serde(rename = "ref")]
    pub reference: String,
    pub image_id: String,
    #[serde(default)]
    pub manifest_digest: Option<String>,
    pub rootfs_layers_sha256: String,
    pub tar: PinnedTar,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct PinnedTar {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaiosImageKind {
    Aios,
    Exttool,
}

impl PaiosImageKind {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Aios => "aios",
            Self::Exttool => "exttool",
        }
    }

    pub(crate) fn default_repo(self) -> &'static str {
        match self {
            Self::Aios => "paios/aios",
            Self::Exttool => "paios/exttool",
        }
    }
}

pub(crate) fn arch_key(arch: PlatformArch) -> &'static str {
    match arch {
        PlatformArch::Amd64 => "amd64",
        PlatformArch::Aarch64 => "aarch64",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PinnedImage {
    pub platform: PinnedPlatform,
    pub revision: String,
    pub tar_path: PathBuf,
    /// BuckyOS root, for the record of images node_daemon loaded (D9).
    pub root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedImage {
    pub kind: PaiosImageKind,
    pub reference: String,
    pub pinned: Option<PinnedImage>,
}

pub(crate) fn manifest_dir(root: &Path) -> PathBuf {
    root.join(PAIOS_IMAGES_DIR)
}

pub(crate) fn load_manifest(root: &Path) -> Result<Option<PaiosImagesManifest>, String> {
    let path = manifest_dir(root).join(PAIOS_IMAGES_MANIFEST);
    let content = match std::fs::read_to_string(path.as_path()) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {} failed: {}", path.display(), error)),
    };
    let manifest: PaiosImagesManifest = serde_json::from_str(&content)
        .map_err(|error| format!("parse {} failed: {}", path.display(), error))?;
    validate_manifest(&manifest)
        .map_err(|error| format!("invalid {}: {}", path.display(), error))?;
    Ok(Some(manifest))
}

fn validate_manifest(manifest: &PaiosImagesManifest) -> Result<(), String> {
    if manifest.schema != PAIOS_IMAGES_SCHEMA {
        return Err(format!("unsupported schema {:?}", manifest.schema));
    }
    if !manifest.images.contains_key(PaiosImageKind::Aios.key()) {
        return Err("images.aios is missing".to_string());
    }
    for (name, image) in &manifest.images {
        if image.repo.trim().is_empty() || image.revision.trim().is_empty() {
            return Err(format!("images.{name} needs repo and revision"));
        }
        for (arch, platform) in &image.platforms {
            if arch != "amd64" && arch != "aarch64" {
                return Err(format!("images.{name}.platforms has unknown arch {arch:?}"));
            }
            validate_platform(platform)
                .map_err(|error| format!("images.{name}.platforms.{arch}: {error}"))?;
        }
    }
    Ok(())
}

fn validate_platform(platform: &PinnedPlatform) -> Result<(), String> {
    if platform.reference.trim().is_empty() || platform.reference.contains('@') {
        return Err("ref must be a tag reference".to_string());
    }
    for (field, value) in [
        ("image_id", Some(platform.image_id.as_str())),
        ("manifest_digest", platform.manifest_digest.as_deref()),
        (
            "rootfs_layers_sha256",
            Some(platform.rootfs_layers_sha256.as_str()),
        ),
    ] {
        if let Some(value) = value {
            if !is_sha256_digest(value) {
                return Err(format!("{field} must be sha256:<64 hex>"));
            }
        }
    }
    let file = platform.tar.file.as_str();
    if file.is_empty()
        || file == "."
        || file.contains("..")
        || file.contains('/')
        || file.contains('\\')
        || file == PAIOS_IMAGES_MANIFEST
    {
        return Err(format!("tar.file {file:?} must be a plain file name"));
    }
    if !is_sha256_hex(platform.tar.sha256.as_str()) || platform.tar.size == 0 {
        return Err("tar needs sha256 (64 hex) and a non-zero size".to_string());
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(is_sha256_hex)
}

/// Resolve which image reference node_daemon must use.
///
/// Priority: `etc/devenv.json` override (warned in pinned mode) > manifest >
/// `<default repo>:latest-<arch>`. `Ok(None)` means the pinned manifest does not
/// carry this (optional) image, which only applies to ExtTool.
pub(crate) fn resolve(
    kind: PaiosImageKind,
    arch: PlatformArch,
    root: &Path,
) -> Result<Option<ResolvedImage>, String> {
    let manifest = load_manifest(root)?;
    let devenv_repo = devenv_image_repo(root, kind);
    if let Some(repo) = devenv_repo {
        static WARNED: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();
        let first = WARNED
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(format!("{}={}", kind.key(), repo));
        if manifest.is_some() && first {
            warn!(
                "{} {}: {:?} overrides the pinned image from {}/{}",
                DEVENV_JSON_RELATIVE_PATH,
                kind.key(),
                repo,
                PAIOS_IMAGES_DIR,
                PAIOS_IMAGES_MANIFEST
            );
        }
        return Ok(Some(local_image(kind, arch, repo.as_str())));
    }
    let Some(manifest) = manifest else {
        return Ok(Some(local_image(kind, arch, kind.default_repo())));
    };
    let Some(image) = manifest.images.get(kind.key()) else {
        return Ok(None);
    };
    let platform = image.platforms.get(arch_key(arch)).ok_or_else(|| {
        format!(
            "{}/{} has no {} image for {}",
            PAIOS_IMAGES_DIR,
            PAIOS_IMAGES_MANIFEST,
            kind.key(),
            arch_key(arch)
        )
    })?;
    Ok(Some(ResolvedImage {
        kind,
        reference: platform.reference.clone(),
        pinned: Some(PinnedImage {
            platform: platform.clone(),
            revision: image.revision.clone(),
            tar_path: manifest_dir(root).join(platform.tar.file.as_str()),
            root: root.to_path_buf(),
        }),
    }))
}

pub(crate) fn local_image(kind: PaiosImageKind, arch: PlatformArch, repo: &str) -> ResolvedImage {
    ResolvedImage {
        kind,
        reference: format!("{}:latest-{}", repo, arch_key(arch)),
        pinned: None,
    }
}

fn devenv_image_repo(root: &Path, kind: PaiosImageKind) -> Option<String> {
    let path = root.join(DEVENV_JSON_RELATIVE_PATH);
    let raw = match std::fs::read_to_string(path.as_path()) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => {
            warn!("ignore unreadable {}: {}", path.display(), error);
            return None;
        }
    };
    match serde_json::from_str::<serde_json::Value>(raw.as_str()) {
        Ok(value) => value
            .get(kind.key())
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        Err(error) => {
            warn!("ignore invalid {}: {}", path.display(), error);
            None
        }
    }
}

/// Identity fields from `docker image inspect --format={{json .}}`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ImageIdentity {
    pub id: String,
    pub repo_digests: Vec<String>,
    pub layers: Vec<String>,
    pub revision: Option<String>,
}

pub(crate) fn parse_image_inspect(raw: &str) -> Result<ImageIdentity, String> {
    let value: serde_json::Value = serde_json::from_str(raw.trim())
        .map_err(|error| format!("parse docker image inspect failed: {error}"))?;
    let value = match value {
        serde_json::Value::Array(mut values) if values.len() == 1 => values.remove(0),
        value => value,
    };
    let id = value
        .get("Id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or("docker image inspect has no Id")?
        .trim()
        .to_string();
    let strings = |value: Option<&serde_json::Value>| -> Vec<String> {
        value
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(ImageIdentity {
        id,
        repo_digests: strings(value.get("RepoDigests")),
        layers: strings(value.pointer("/RootFS/Layers")),
        revision: value
            .pointer("/Config/Labels")
            .and_then(|labels| labels.get(IMAGE_REVISION_LABEL))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

pub(crate) fn rootfs_layers_sha256(layers: &[String]) -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(layers.join("\n").as_bytes()))
    )
}

/// A loaded image is the pinned one when its id is the config or registry
/// manifest digest, a repo digest names the registry manifest, or (containerd
/// image store / WSLC, where `docker load` reports another id) the layer list
/// and the source revision label both match.
pub(crate) fn matches_pinned(identity: &ImageIdentity, pinned: &PinnedImage) -> bool {
    let expected = &pinned.platform;
    let same_digest = |left: &str, right: &str| left.eq_ignore_ascii_case(right);
    if same_digest(identity.id.as_str(), expected.image_id.as_str()) {
        return true;
    }
    if let Some(manifest_digest) = expected.manifest_digest.as_deref() {
        if same_digest(identity.id.as_str(), manifest_digest)
            || identity.repo_digests.iter().any(|repo_digest| {
                repo_digest
                    .split_once('@')
                    .is_some_and(|(_, digest)| same_digest(digest, manifest_digest))
            })
        {
            return true;
        }
    }
    !identity.layers.is_empty()
        && same_digest(
            rootfs_layers_sha256(&identity.layers).as_str(),
            expected.rootfs_layers_sha256.as_str(),
        )
        && identity.revision.as_deref() == Some(pinned.revision.as_str())
}

/// Check a bundled tarball against the manifest before handing it to `docker load`.
pub(crate) fn verify_tar(path: &Path, expected: &PinnedTar) -> Result<(), String> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(format!(
                "incomplete installation: {}/{} is missing; reinstall the same BuckyOS version",
                PAIOS_IMAGES_DIR, expected.file
            ));
        }
        Err(error) => return Err(format!("open {} failed: {}", path.display(), error)),
    };
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {} failed: {}", path.display(), error))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    let actual = hex::encode(hasher.finalize());
    if size != expected.size || !actual.eq_ignore_ascii_case(expected.sha256.as_str()) {
        return Err(format!(
            "{}/{} is corrupted (size {} sha256 {}, expected size {} sha256 {}); reinstall the same BuckyOS version",
            PAIOS_IMAGES_DIR, expected.file, size, actual, expected.size, expected.sha256
        ));
    }
    Ok(())
}

/// `verify_tar` remembered per file size, mtime and expected hash, so a
/// broken installation does not re-hash a large tarball on every retry.
fn verify_tar_cached(path: &Path, expected: &PinnedTar) -> Result<(), String> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return verify_tar(path, expected);
    };
    let key = (
        metadata.len(),
        metadata.modified().ok(),
        expected.sha256.clone(),
    );
    let cache = VERIFIED_TARS.get_or_init(Default::default);
    if let Some((size, modified, sha256, result)) = cache.lock().unwrap().get(path) {
        if (*size, *modified, sha256.clone()) == key {
            return result.clone();
        }
    }
    let result = verify_tar(path, expected);
    cache
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), (key.0, key.1, key.2, result.clone()));
    result
}

pub(crate) fn local_image_missing_hint(image: &ResolvedImage) -> String {
    let rebuild = match image.kind {
        PaiosImageKind::Aios => "./build_aios --local-test",
        PaiosImageKind::Exttool => "./build_exttool --image local/exttool-test --arch <arch>",
    };
    format!(
        "{} image {} is not in the local container store and BuckyOS never pulls it. Fix with one of: `docker pull {}`; `uv run build.py` (pre-pulls the paios images); `{}` (local build)",
        image.kind.key(),
        image.reference,
        image.reference,
        rebuild
    )
}

/// Images node_daemon loaded from release tarballs; only these are ever
/// garbage collected (D9), never images a developer pulled or built.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize, Eq, PartialEq)]
pub(crate) struct LoadedImages {
    #[serde(default)]
    pub loaded: Vec<LoadedImage>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub(crate) struct LoadedImage {
    #[serde(rename = "ref")]
    pub reference: String,
    pub image_id: String,
}

fn loaded_state_path(root: &Path) -> PathBuf {
    root.join(LOADED_STATE_RELATIVE_PATH)
}

pub(crate) fn load_loaded_images(root: &Path) -> LoadedImages {
    std::fs::read_to_string(loaded_state_path(root))
        .ok()
        .and_then(|raw| serde_json::from_str(raw.as_str()).ok())
        .unwrap_or_default()
}

fn save_loaded_images(root: &Path, state: &LoadedImages) {
    let path = loaded_state_path(root);
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&temporary, &path)
    })();
    if let Err(error) = result {
        warn!(
            "record loaded paios images in {} failed: {}",
            path.display(),
            error
        );
    }
}

fn record_loaded_image(pinned: &PinnedImage) {
    let mut state = load_loaded_images(pinned.root.as_path());
    let entry = LoadedImage {
        reference: pinned.platform.reference.clone(),
        image_id: pinned.platform.image_id.clone(),
    };
    if !state.loaded.contains(&entry) {
        state.loaded.push(entry);
        save_loaded_images(pinned.root.as_path(), &state);
    }
}

/// Serialises ExtTool volume preparation across every app on this node.
pub(crate) fn exttool_prepare_lock() -> &'static tokio::sync::Mutex<()> {
    EXTTOOL_PREPARE_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn image_lock(reference: &str) -> Arc<tokio::sync::Mutex<()>> {
    IMAGE_LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(reference.to_string())
        .or_default()
        .clone()
}

pub(crate) fn short_id(image_id: &str) -> &str {
    let hex = image_id.strip_prefix("sha256:").unwrap_or(image_id);
    hex.get(..12).unwrap_or(hex)
}

/// ExtTool volumes are named after the image id they were seeded from, so a
/// new ExtTool image always gets a fresh volume.
pub(crate) fn exttool_volume_name(image_id: &str) -> String {
    format!("{}-{}", EXTTOOL_VOLUME_PREFIX, short_id(image_id))
}

fn seed_marker(image_id: &str) -> String {
    format!(".buckyos-seeded-{}", short_id(image_id))
}

/// What a correctly deployed worker container looks like right now.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerExpectation {
    /// Id of the worker image in the local store; `None` when it is missing.
    pub image_id: Option<String>,
    /// The seeded ExtTool volume a worker should mount, if any.
    pub exttool_volume: Option<String>,
}

fn invalidate_expectations() {
    if let Some(cache) = EXPECTATIONS.get() {
        cache.lock().unwrap().clear();
    }
}

/// paios image operations on the local container runtime (Docker or WSLC).
/// `runner` replaces the docker CLI in tests.
pub(crate) struct PaiosDocker<'a> {
    runner: Option<&'a DockerCommandRunner>,
}

impl<'a> PaiosDocker<'a> {
    pub(crate) fn new(runner: Option<&'a DockerCommandRunner>) -> Self {
        Self { runner }
    }

    async fn docker(&self, args: &[&str]) -> RunResult<CommandOutput> {
        let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        run_docker_with(self.runner, &args).await
    }

    pub(crate) async fn inspect(&self, reference: &str) -> RunResult<Option<ImageIdentity>> {
        let output = self
            .docker(&["image", "inspect", "--format={{json .}}", reference])
            .await?;
        if !output.status.success() && docker_image_absent(self.runner, reference).await {
            return Ok(None);
        }
        ensure_success("docker image inspect", &output)?;
        parse_image_inspect(output.stdout.as_str())
            .map(Some)
            .map_err(|error| {
                ControlRuntItemErrors::ExecuteError("docker image inspect".to_string(), error)
            })
    }

    /// Make sure `image` is in the local store and return its id. Pinned images
    /// are verified and, when missing or different, loaded from the bundled
    /// tarball; local images must already exist. Nothing is ever pulled.
    pub(crate) async fn ensure_image(&self, image: &ResolvedImage) -> RunResult<String> {
        let lock = image_lock(image.reference.as_str());
        let _guard = lock.lock().await;
        let current = self.inspect(image.reference.as_str()).await?;
        let Some(pinned) = image.pinned.as_ref() else {
            return current.map(|identity| identity.id).ok_or_else(|| {
                ControlRuntItemErrors::ExecuteError(
                    "deploy".to_string(),
                    local_image_missing_hint(image),
                )
            });
        };
        if let Some(identity) = current {
            if matches_pinned(&identity, pinned) {
                return Ok(identity.id);
            }
            warn!(
                "{} ({}) is not the pinned {} image {}; reloading it from {}",
                image.reference,
                identity.id,
                image.kind.key(),
                pinned.platform.image_id,
                pinned.tar_path.display()
            );
        }
        let started = Instant::now();
        let tar_path = pinned.tar_path.clone();
        let expected = pinned.platform.tar.clone();
        tokio::task::spawn_blocking(move || verify_tar_cached(tar_path.as_path(), &expected))
            .await
            .map_err(|error| {
                ControlRuntItemErrors::ExecuteError(
                    "verify paios image".to_string(),
                    error.to_string(),
                )
            })?
            .map_err(|error| {
                ControlRuntItemErrors::ExecuteError("verify paios image".to_string(), error)
            })?;
        info!(
            "loading {} image {} from {} ({} bytes)",
            image.kind.key(),
            image.reference,
            pinned.tar_path.display(),
            pinned.platform.tar.size
        );
        let tar_path = pinned.tar_path.to_string_lossy().to_string();
        let output = self.docker(&["load", "-i", tar_path.as_str()]).await?;
        ensure_success("docker load", &output)?;
        record_loaded_image(pinned);
        invalidate_expectations();
        let identity = self
            .inspect(image.reference.as_str())
            .await?
            .ok_or_else(|| {
                ControlRuntItemErrors::ExecuteError(
                    "docker load".to_string(),
                    format!(
                        "{} is still missing after loading {}",
                        image.reference, tar_path
                    ),
                )
            })?;
        if !matches_pinned(&identity, pinned) {
            return Err(ControlRuntItemErrors::ExecuteError(
                "docker load".to_string(),
                format!(
                    "{} loaded from {} is {}, not the pinned image {}",
                    image.reference, tar_path, identity.id, pinned.platform.image_id
                ),
            ));
        }
        info!(
            "loaded {} as {} in {:.1}s",
            image.reference,
            identity.id,
            started.elapsed().as_secs_f64()
        );
        Ok(identity.id)
    }

    pub(crate) async fn volume_exists(&self, volume: &str) -> RunResult<bool> {
        let output = self.docker(&["volume", "ls", "--format={{.Name}}"]).await?;
        ensure_success("docker volume ls", &output)?;
        Ok(output.stdout.lines().any(|name| name.trim() == volume))
    }

    /// The ExtTool volume this image seeds. Pinned images are named from the
    /// manifest (the image itself may already be gone, D8); local images need
    /// to be present.
    pub(crate) async fn exttool_volume(
        &self,
        exttool: &ResolvedImage,
    ) -> RunResult<Option<String>> {
        if let Some(pinned) = exttool.pinned.as_ref() {
            return Ok(Some(exttool_volume_name(pinned.platform.image_id.as_str())));
        }
        Ok(self
            .inspect(exttool.reference.as_str())
            .await?
            .map(|identity| exttool_volume_name(identity.id.as_str())))
    }

    async fn volume_seeded(&self, volume: &str, marker: &str, worker_ref: &str) -> RunResult<bool> {
        let cache = SEEDED_VOLUMES.get_or_init(Default::default);
        if self.runner.is_none() && cache.lock().unwrap().contains(volume) {
            return Ok(true);
        }
        let mount = format!("{volume}:/seed:ro");
        let check = format!("test -f /seed/{marker}");
        let output = self
            .docker(&[
                "run",
                "--rm",
                "-v",
                mount.as_str(),
                "--entrypoint",
                "/bin/sh",
                worker_ref,
                "-c",
                check.as_str(),
            ])
            .await?;
        let seeded = output.status.success();
        if seeded && self.runner.is_none() {
            cache.lock().unwrap().insert(volume.to_string());
        }
        Ok(seeded)
    }

    /// Make sure the ExtTool volume of `exttool` exists and is completely
    /// seeded, and return its name. `Ok(None)` means ExtTool is unavailable
    /// (D6): the reason is logged as an error and workers start without it.
    pub(crate) async fn prepare_exttool(
        &self,
        exttool: Option<&ResolvedImage>,
        worker_ref: &str,
    ) -> RunResult<Option<String>> {
        let _guard = exttool_prepare_lock().lock().await;
        let Some(exttool) = exttool else {
            error!(
                "{}/{} pins no exttool image; workers start without the ExtTool mount (incomplete release package)",
                PAIOS_IMAGES_DIR, PAIOS_IMAGES_MANIFEST
            );
            return Ok(None);
        };
        let image_id = match exttool.pinned.as_ref() {
            Some(pinned) => pinned.platform.image_id.clone(),
            None => match self.inspect(exttool.reference.as_str()).await? {
                Some(identity) => identity.id,
                None => {
                    error!(
                        "{}; workers start without the ExtTool mount",
                        local_image_missing_hint(exttool)
                    );
                    return Ok(None);
                }
            },
        };
        let volume = exttool_volume_name(image_id.as_str());
        let marker = seed_marker(image_id.as_str());
        if self.volume_exists(volume.as_str()).await? {
            if self
                .volume_seeded(volume.as_str(), marker.as_str(), worker_ref)
                .await?
            {
                return Ok(Some(volume));
            }
            warn!(
                "{} has no seed marker (interrupted seeding?); recreating it",
                volume
            );
            let output = self.docker(&["volume", "rm", volume.as_str()]).await?;
            ensure_success("docker volume rm", &output)?;
        }
        if exttool.pinned.is_some() {
            if let Err(error) = self.ensure_image(exttool).await {
                error!(
                    "exttool image {} unavailable: {}; workers start without the ExtTool mount",
                    exttool.reference, error
                );
                return Ok(None);
            }
        }
        let output = self.docker(&["volume", "create", volume.as_str()]).await?;
        ensure_success("docker volume create", &output)?;
        info!(
            "seeding {} from {} (copies the image payload into the volume)",
            volume, exttool.reference
        );
        let started = Instant::now();
        let mount = format!("{volume}:{EXTTOOL_MOUNT_ROOT}");
        let touch = format!("touch {EXTTOOL_MOUNT_ROOT}/{marker}");
        let output = self
            .docker(&[
                "run",
                "--rm",
                "-v",
                mount.as_str(),
                "--entrypoint",
                "/bin/sh",
                exttool.reference.as_str(),
                "-c",
                touch.as_str(),
            ])
            .await?;
        if let Err(error) = ensure_success("docker run (exttool seed)", &output) {
            let _ = self.docker(&["volume", "rm", volume.as_str()]).await;
            return Err(error);
        }
        if self.runner.is_none() {
            SEEDED_VOLUMES
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .insert(volume.clone());
        }
        invalidate_expectations();
        info!(
            "seeded {} in {:.1}s",
            volume,
            started.elapsed().as_secs_f64()
        );
        if exttool.pinned.is_some() {
            // D8: the volume now holds the payload and the bundled tarball can
            // recreate the image, so the (large) seed image is not kept.
            let output = self
                .docker(&["image", "rm", exttool.reference.as_str()])
                .await?;
            if !output.status.success() {
                warn!(
                    "remove seeded exttool image {} failed: {}",
                    exttool.reference,
                    output.stderr.trim()
                );
            }
        }
        Ok(Some(volume))
    }

    /// D9, pinned mode: remove images node_daemon loaded for older releases
    /// and ExtTool volumes other than `keep_volume`. `docker image rm` and
    /// `docker volume rm` refuse anything a container still uses, so those
    /// stay until a later start.
    pub(crate) async fn collect_garbage(
        &self,
        root: &Path,
        current: &[&ResolvedImage],
        keep_volume: Option<&str>,
    ) -> RunResult<()> {
        let keep = current
            .iter()
            .map(|image| image.reference.as_str())
            .collect::<HashSet<_>>();
        let mut state = load_loaded_images(root);
        let mut remaining = Vec::new();
        for entry in state.loaded.drain(..) {
            if keep.contains(entry.reference.as_str()) {
                remaining.push(entry);
                continue;
            }
            let output = self
                .docker(&["image", "rm", entry.reference.as_str()])
                .await?;
            if output.status.success() {
                info!(
                    "removed old paios image {} ({})",
                    entry.reference, entry.image_id
                );
            } else if docker_image_absent(self.runner, entry.reference.as_str()).await {
                info!("old paios image {} is already gone", entry.reference);
            } else {
                info!(
                    "keep old paios image {} for now: {}",
                    entry.reference,
                    output.stderr.trim()
                );
                remaining.push(entry);
            }
        }
        state.loaded = remaining;
        save_loaded_images(root, &state);

        let output = self.docker(&["volume", "ls", "--format={{.Name}}"]).await?;
        ensure_success("docker volume ls", &output)?;
        for volume in output.stdout.lines().map(str::trim) {
            let exttool_volume = volume == EXTTOOL_VOLUME_PREFIX
                || volume.starts_with(&format!("{EXTTOOL_VOLUME_PREFIX}-"));
            if !exttool_volume || Some(volume) == keep_volume {
                continue;
            }
            let removed = self.docker(&["volume", "rm", volume]).await?;
            if removed.status.success() {
                info!("removed unused ExtTool volume {}", volume);
            } else {
                info!(
                    "keep ExtTool volume {} for now: {}",
                    volume,
                    removed.stderr.trim()
                );
            }
        }
        Ok(())
    }

    /// The worker image id and mountable ExtTool volume a running worker must
    /// match. Only inspects; real-docker results are cached briefly because
    /// every status check asks.
    pub(crate) async fn worker_expectation(
        &self,
        worker: &ResolvedImage,
        exttool: Option<&ResolvedImage>,
    ) -> RunResult<WorkerExpectation> {
        let key = format!(
            "{}|{}",
            worker.reference,
            exttool.map(|image| image.reference.as_str()).unwrap_or("")
        );
        let cache = EXPECTATIONS.get_or_init(Default::default);
        if self.runner.is_none() {
            if let Some((observed_at, expectation)) = cache.lock().unwrap().get(&key) {
                if observed_at.elapsed() < EXPECTATION_TTL {
                    return Ok(expectation.clone());
                }
            }
        }
        let image_id = self
            .inspect(worker.reference.as_str())
            .await?
            .map(|identity| identity.id);
        let exttool_volume = match exttool {
            Some(exttool) => match self.exttool_volume(exttool).await? {
                Some(volume) if self.volume_exists(volume.as_str()).await? => Some(volume),
                _ => None,
            },
            None => None,
        };
        let expectation = WorkerExpectation {
            image_id,
            exttool_volume,
        };
        if self.runner.is_none() {
            cache
                .lock()
                .unwrap()
                .insert(key, (Instant::now(), expectation.clone()));
        }
        Ok(expectation)
    }
}

/// Pinned mode only: load the worker and ExtTool images and seed the ExtTool
/// volume in the background right after start, so the first Agent deploy does
/// not wait minutes for it. Shares the deploy locks; retries while the
/// container runtime is unavailable.
pub(crate) fn spawn_preload() {
    if !crate::product::containers_enabled() {
        info!("paios images: this product runs no containers, nothing to preload");
        return;
    }
    let root = get_buckyos_root_dir();
    let arch = PlatformTarget::current().arch;
    let worker = match resolve(PaiosImageKind::Aios, arch, root.as_path()) {
        Ok(Some(worker)) if worker.pinned.is_some() => worker,
        Ok(_) => return,
        Err(error) => {
            error!("paios image manifest unusable: {}", error);
            return;
        }
    };
    let exttool = match resolve(PaiosImageKind::Exttool, arch, root.as_path()) {
        Ok(exttool) => exttool,
        Err(error) => {
            error!("paios image manifest unusable: {}", error);
            None
        }
    };
    tokio::spawn(async move {
        let docker = PaiosDocker::new(None);
        let started = Instant::now();
        let mut backoff = Duration::from_secs(5);
        loop {
            let result = match docker.ensure_image(&worker).await {
                Ok(_) => {
                    docker
                        .prepare_exttool(exttool.as_ref(), worker.reference.as_str())
                        .await
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(volume) => {
                    info!(
                        "paios images ready in {:.1}s: {} (exttool volume {:?})",
                        started.elapsed().as_secs_f64(),
                        worker.reference,
                        volume
                    );
                    let mut current = vec![&worker];
                    current.extend(exttool.as_ref());
                    if let Err(error) = docker
                        .collect_garbage(root.as_path(), &current, volume.as_deref())
                        .await
                    {
                        warn!("paios image cleanup failed: {}", error);
                    }
                    return;
                }
                Err(error) => {
                    warn!(
                        "paios image preload failed after {:.1}s, retry in {}s: {}",
                        started.elapsed().as_secs_f64(),
                        backoff.as_secs(),
                        error
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(PRELOAD_MAX_BACKOFF);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("testdata/paios_images.v1.json");

    fn root_with(manifest: Option<&str>, devenv: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        if let Some(manifest) = manifest {
            let dir = manifest_dir(root.path());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(PAIOS_IMAGES_MANIFEST), manifest).unwrap();
        }
        if let Some(devenv) = devenv {
            std::fs::create_dir_all(root.path().join("etc")).unwrap();
            std::fs::write(root.path().join(DEVENV_JSON_RELATIVE_PATH), devenv).unwrap();
        }
        root
    }

    fn fixture_with(edit: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        edit(&mut value);
        value.to_string()
    }

    #[test]
    fn missing_manifest_is_local_mode() {
        let root = root_with(None, None);
        assert!(load_manifest(root.path()).unwrap().is_none());
        let image = resolve(PaiosImageKind::Aios, PlatformArch::Amd64, root.path())
            .unwrap()
            .unwrap();
        assert_eq!(image.reference, "paios/aios:latest-amd64");
        assert!(image.pinned.is_none());
        let image = resolve(PaiosImageKind::Exttool, PlatformArch::Aarch64, root.path())
            .unwrap()
            .unwrap();
        assert_eq!(image.reference, "paios/exttool:latest-aarch64");
    }

    #[test]
    fn fixture_resolves_pinned_images_and_ignores_unknown_fields() {
        let root = root_with(Some(FIXTURE), None);
        let manifest = load_manifest(root.path()).unwrap().unwrap();
        assert_eq!(manifest.images.len(), 2);
        let image = resolve(PaiosImageKind::Aios, PlatformArch::Amd64, root.path())
            .unwrap()
            .unwrap();
        assert_eq!(image.reference, "paios/aios:0.7.0-835b5bd44125-amd64");
        let pinned = image.pinned.unwrap();
        assert_eq!(pinned.revision, "835b5bd44125cd923444206782fe34ff9e0c3225");
        assert_eq!(
            pinned.tar_path,
            root.path()
                .join("bin/paios-images/aios-amd64-7329ae7f4108.tar.gz")
        );
        let exttool = resolve(PaiosImageKind::Exttool, PlatformArch::Aarch64, root.path())
            .unwrap()
            .unwrap();
        assert_eq!(
            exttool.reference,
            "paios/exttool:0.7.0-07787a750c1e-aarch64"
        );
    }

    #[test]
    fn broken_manifest_is_an_error_not_local_mode() {
        let root = root_with(Some("{not json"), None);
        assert!(resolve(PaiosImageKind::Aios, PlatformArch::Amd64, root.path()).is_err());

        let root = root_with(
            Some(&fixture_with(|value| {
                value["schema"] = "buckyos.paios-images/v2".into()
            })),
            None,
        );
        assert!(load_manifest(root.path())
            .unwrap_err()
            .contains("unsupported schema"));

        let root = root_with(
            Some(&fixture_with(|value| {
                value["images"].as_object_mut().unwrap().remove("aios");
            })),
            None,
        );
        assert!(load_manifest(root.path())
            .unwrap_err()
            .contains("images.aios"));
    }

    #[test]
    fn missing_arch_is_an_error() {
        let root = root_with(
            Some(&fixture_with(|value| {
                value["images"]["aios"]["platforms"]
                    .as_object_mut()
                    .unwrap()
                    .remove("aarch64");
            })),
            None,
        );
        assert!(resolve(PaiosImageKind::Aios, PlatformArch::Amd64, root.path()).is_ok());
        let error = resolve(PaiosImageKind::Aios, PlatformArch::Aarch64, root.path()).unwrap_err();
        assert!(error.contains("no aios image for aarch64"), "{error}");
    }

    #[test]
    fn tar_file_must_be_a_plain_name() {
        for file in [
            "../aios.tar.gz",
            "sub/aios.tar.gz",
            "sub\\aios.tar.gz",
            "..",
            "",
        ] {
            let root = root_with(
                Some(&fixture_with(|value| {
                    value["images"]["aios"]["platforms"]["amd64"]["tar"]["file"] = file.into();
                })),
                None,
            );
            assert!(load_manifest(root.path()).is_err(), "{file:?} accepted");
        }
    }

    #[test]
    fn exttool_absent_from_manifest_resolves_to_none() {
        let root = root_with(
            Some(&fixture_with(|value| {
                value["images"].as_object_mut().unwrap().remove("exttool");
            })),
            None,
        );
        assert!(
            resolve(PaiosImageKind::Exttool, PlatformArch::Amd64, root.path())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn devenv_override_wins_over_manifest() {
        let root = root_with(Some(FIXTURE), Some(r#"{"aios": "local/aios-test"}"#));
        let aios = resolve(PaiosImageKind::Aios, PlatformArch::Amd64, root.path())
            .unwrap()
            .unwrap();
        assert_eq!(aios.reference, "local/aios-test:latest-amd64");
        assert!(aios.pinned.is_none());
        let exttool = resolve(PaiosImageKind::Exttool, PlatformArch::Amd64, root.path())
            .unwrap()
            .unwrap();
        assert!(exttool.pinned.is_some());
    }

    #[test]
    fn identity_matching_rules() {
        let root = root_with(Some(FIXTURE), None);
        let pinned = resolve(PaiosImageKind::Aios, PlatformArch::Amd64, root.path())
            .unwrap()
            .unwrap()
            .pinned
            .unwrap();
        let classic = ImageIdentity {
            id: pinned.platform.image_id.clone(),
            ..Default::default()
        };
        assert!(matches_pinned(&classic, &pinned));
        let pulled = ImageIdentity {
            id: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            repo_digests: vec![format!(
                "paios/aios@{}",
                pinned.platform.manifest_digest.clone().unwrap()
            )],
            ..Default::default()
        };
        assert!(matches_pinned(&pulled, &pinned));

        // Real `docker load` result on Docker 29 with the containerd image store.
        let raw = r#"[{"Id":"sha256:15a94a4b45c47bcb86a6e51e786fcd1fe24f5a53c650a131cc90a9c603a41f5d","RepoDigests":["paios/aios@sha256:15a94a4b45c47bcb86a6e51e786fcd1fe24f5a53c650a131cc90a9c603a41f5d"],"RootFS":{"Type":"layers","Layers":["sha256:a"]},"Config":{"Labels":{"org.opencontainers.image.revision":"835b5bd44125cd923444206782fe34ff9e0c3225"}}}]"#;
        let mut loaded = parse_image_inspect(raw).unwrap();
        assert!(!matches_pinned(&loaded, &pinned));
        let mut layer_pinned = pinned.clone();
        layer_pinned.platform.rootfs_layers_sha256 = rootfs_layers_sha256(&loaded.layers);
        assert!(matches_pinned(&loaded, &layer_pinned));
        loaded.revision = Some("0000000000000000000000000000000000000000".to_string());
        assert!(!matches_pinned(&loaded, &layer_pinned));
    }

    #[test]
    fn rootfs_layers_hash_joins_layers_with_newlines() {
        assert_eq!(
            rootfs_layers_sha256(&["sha256:a".to_string(), "sha256:b".to_string()]),
            format!(
                "sha256:{}",
                hex::encode(Sha256::digest(b"sha256:a\nsha256:b"))
            )
        );
    }

    #[test]
    fn verify_tar_checks_presence_size_and_hash() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("aios.tar.gz");
        let expected = PinnedTar {
            file: "aios.tar.gz".to_string(),
            sha256: hex::encode(Sha256::digest(b"image")),
            size: 5,
        };
        let missing = verify_tar(&path, &expected).unwrap_err();
        assert!(
            missing.contains("bin/paios-images/aios.tar.gz"),
            "{missing}"
        );
        std::fs::write(&path, b"image").unwrap();
        verify_tar(&path, &expected).unwrap();
        std::fs::write(&path, b"imagf").unwrap();
        assert!(verify_tar(&path, &expected)
            .unwrap_err()
            .contains("corrupted"));
    }

    use crate::app_loader::{DockerCommandRunner, DockerExecOutcome};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeStore {
        images: HashMap<String, serde_json::Value>,
        volumes: Vec<String>,
        loads: HashMap<String, (String, serde_json::Value)>,
        seeded: bool,
        inspect_fails: bool,
        list_fails: bool,
        log: Vec<String>,
    }

    fn fake_runner(store: Arc<Mutex<FakeStore>>) -> DockerCommandRunner {
        Arc::new(move |args: Vec<String>| {
            let store = store.clone();
            Box::pin(async move {
                let mut store = store.lock().unwrap();
                store.log.push(args.join(" "));
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                let ok = |stdout: String| DockerExecOutcome {
                    ok: true,
                    stdout,
                    stderr: String::new(),
                };
                let fail = |stderr: &str| DockerExecOutcome {
                    ok: false,
                    stdout: String::new(),
                    stderr: stderr.to_string(),
                };
                let missing = |reference: &str| DockerExecOutcome {
                    ok: false,
                    stdout: "[]".to_string(),
                    stderr: format!("未找到映像 '{reference}'。"),
                };
                match args.as_slice() {
                    ["image", "inspect", _, reference] => match store.images.get(*reference) {
                        _ if store.inspect_fails => fail("无法连接到 WSLC 会话"),
                        Some(identity) => ok(identity.to_string()),
                        None => missing(reference),
                    },
                    ["images", "-q", reference] => {
                        if store.list_fails {
                            fail("无法连接到 WSLC 会话")
                        } else {
                            ok(store
                                .images
                                .get(*reference)
                                .map(|identity| identity["Id"].as_str().unwrap().to_string())
                                .unwrap_or_default())
                        }
                    }
                    ["load", "-i", path] => match store.loads.get(*path).cloned() {
                        Some((reference, identity)) => {
                            store.images.insert(reference, identity);
                            ok("Loaded image".to_string())
                        }
                        None => fail("invalid tar"),
                    },
                    ["image", "rm", reference] => match store.images.remove(*reference) {
                        Some(_) => ok(String::new()),
                        None => missing(reference),
                    },
                    ["volume", "ls", ..] => ok(store.volumes.join("\n")),
                    ["volume", "create", name] => {
                        store.volumes.push(name.to_string());
                        ok(String::new())
                    }
                    ["volume", "rm", name] => {
                        store.volumes.retain(|volume| volume != name);
                        store.seeded = false;
                        ok(String::new())
                    }
                    ["run", .., command] if command.starts_with("test -f") => {
                        if store.seeded {
                            ok(String::new())
                        } else {
                            fail("")
                        }
                    }
                    ["run", .., command] if command.starts_with("touch") => {
                        store.seeded = true;
                        ok(String::new())
                    }
                    _ => panic!("unexpected docker command {args:?}"),
                }
            })
        })
    }

    /// Fixture manifest whose amd64 tarballs exist under `root` with real hashes.
    fn pinned_root() -> (tempfile::TempDir, PinnedImage, PinnedImage) {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        for (name, data) in [
            ("aios", b"aios tarball".as_slice()),
            ("exttool", b"exttool tarball".as_slice()),
        ] {
            let tar = &mut value["images"][name]["platforms"]["amd64"]["tar"];
            tar["sha256"] = hex::encode(Sha256::digest(data)).into();
            tar["size"] = (data.len() as u64).into();
        }
        let root = root_with(Some(&value.to_string()), None);
        let mut pinned = Vec::new();
        for (kind, data) in [
            (PaiosImageKind::Aios, b"aios tarball".as_slice()),
            (PaiosImageKind::Exttool, b"exttool tarball".as_slice()),
        ] {
            let image = resolve(kind, PlatformArch::Amd64, root.path())
                .unwrap()
                .unwrap()
                .pinned
                .unwrap();
            std::fs::write(&image.tar_path, data).unwrap();
            pinned.push(image);
        }
        let exttool = pinned.pop().unwrap();
        let aios = pinned.pop().unwrap();
        (root, aios, exttool)
    }

    fn pinned_image(kind: PaiosImageKind, pinned: &PinnedImage) -> ResolvedImage {
        ResolvedImage {
            kind,
            reference: pinned.platform.reference.clone(),
            pinned: Some(pinned.clone()),
        }
    }

    fn identity_of(pinned: &PinnedImage) -> serde_json::Value {
        serde_json::json!({ "Id": pinned.platform.image_id })
    }

    fn log_has(store: &Arc<Mutex<FakeStore>>, needle: &str) -> usize {
        store
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }

    #[tokio::test]
    async fn pinned_image_in_store_is_used_as_is() {
        let (_root, aios, _) = pinned_root();
        let store = Arc::new(Mutex::new(FakeStore::default()));
        store
            .lock()
            .unwrap()
            .images
            .insert(aios.platform.reference.clone(), identity_of(&aios));
        let runner = fake_runner(store.clone());
        let id = PaiosDocker::new(Some(&runner))
            .ensure_image(&pinned_image(PaiosImageKind::Aios, &aios))
            .await
            .unwrap();
        assert_eq!(id, aios.platform.image_id);
        assert_eq!(store.lock().unwrap().log.len(), 1);
    }

    #[tokio::test]
    async fn missing_or_retagged_pinned_image_is_loaded_from_its_tarball() {
        let (_root, aios, _) = pinned_root();
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let tar = aios.tar_path.to_string_lossy().to_string();
        store.lock().unwrap().images.insert(
            aios.platform.reference.clone(),
            serde_json::json!({ "Id": format!("sha256:{}", "0".repeat(64)) }),
        );
        store.lock().unwrap().loads.insert(
            tar.clone(),
            (aios.platform.reference.clone(), identity_of(&aios)),
        );
        let runner = fake_runner(store.clone());
        let id = PaiosDocker::new(Some(&runner))
            .ensure_image(&pinned_image(PaiosImageKind::Aios, &aios))
            .await
            .unwrap();
        assert_eq!(id, aios.platform.image_id);
        assert_eq!(log_has(&store, &format!("load -i {tar}")), 1);
        assert_eq!(log_has(&store, "pull"), 0);
    }

    #[tokio::test]
    async fn tampered_missing_or_wrong_tarball_fails_without_pulling() {
        let (_root, aios, _) = pinned_root();
        let image = pinned_image(PaiosImageKind::Aios, &aios);
        let tar = aios.tar_path.to_string_lossy().to_string();

        std::fs::write(&aios.tar_path, b"aios tarbalL").unwrap();
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let runner = fake_runner(store.clone());
        let error = PaiosDocker::new(Some(&runner))
            .ensure_image(&image)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("corrupted"), "{error}");
        assert_eq!(log_has(&store, "load"), 0);

        std::fs::remove_file(&aios.tar_path).unwrap();
        let error = PaiosDocker::new(Some(&runner))
            .ensure_image(&image)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("aios-amd64-7329ae7f4108.tar.gz"),
            "{error}"
        );

        std::fs::write(&aios.tar_path, b"aios tarball").unwrap();
        store.lock().unwrap().loads.insert(
            tar,
            (
                aios.platform.reference.clone(),
                serde_json::json!({ "Id": format!("sha256:{}", "1".repeat(64)) }),
            ),
        );
        let error = PaiosDocker::new(Some(&runner))
            .ensure_image(&image)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("not the pinned image"),
            "{error}"
        );
        assert_eq!(log_has(&store, "pull"), 0);
    }

    #[tokio::test]
    async fn localized_missing_image_loads_but_runtime_faults_keep_inspect_error() {
        let (_root, aios, _) = pinned_root();
        let image = pinned_image(PaiosImageKind::Aios, &aios);
        let store = Arc::new(Mutex::new(FakeStore::default()));
        store.lock().unwrap().loads.insert(
            aios.tar_path.to_string_lossy().to_string(),
            (aios.platform.reference.clone(), identity_of(&aios)),
        );
        let runner = fake_runner(store.clone());
        let docker = PaiosDocker::new(Some(&runner));

        {
            let mut store = store.lock().unwrap();
            store.inspect_fails = true;
            store.list_fails = true;
        }
        let error = docker.ensure_image(&image).await.unwrap_err();
        assert!(
            matches!(&error, ControlRuntItemErrors::ExecuteError(step, _) if step == "docker image inspect"),
            "{error}"
        );
        assert!(
            error.to_string().contains("无法连接到 WSLC 会话"),
            "{error}"
        );
        assert_eq!(log_has(&store, "load"), 0);

        {
            let mut store = store.lock().unwrap();
            store.list_fails = false;
            store
                .images
                .insert(aios.platform.reference.clone(), identity_of(&aios));
        }
        let error = docker.ensure_image(&image).await.unwrap_err();
        assert!(
            matches!(&error, ControlRuntItemErrors::ExecuteError(step, _) if step == "docker image inspect"),
            "{error}"
        );
        assert_eq!(log_has(&store, "load"), 0);

        {
            let mut store = store.lock().unwrap();
            store.inspect_fails = false;
            store.images.clear();
        }
        let id = docker.ensure_image(&image).await.unwrap();
        assert_eq!(id, aios.platform.image_id);
        assert_eq!(log_has(&store, "load -i"), 1);
    }

    #[tokio::test]
    async fn missing_local_image_is_an_error_with_a_pull_hint() {
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let runner = fake_runner(store.clone());
        let image = local_image(PaiosImageKind::Aios, PlatformArch::Amd64, "paios/aios");
        let error = PaiosDocker::new(Some(&runner))
            .ensure_image(&image)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("docker pull paios/aios:latest-amd64"),
            "{error}"
        );
        assert_eq!(log_has(&store, "pull"), 0);
    }

    #[tokio::test]
    async fn pinned_exttool_seeds_once_per_image_and_drops_the_seed_image() {
        let (_root, aios, exttool) = pinned_root();
        let store = Arc::new(Mutex::new(FakeStore::default()));
        store.lock().unwrap().loads.insert(
            exttool.tar_path.to_string_lossy().to_string(),
            (exttool.platform.reference.clone(), identity_of(&exttool)),
        );
        let runner = fake_runner(store.clone());
        let docker = PaiosDocker::new(Some(&runner));
        let image = pinned_image(PaiosImageKind::Exttool, &exttool);
        let worker = aios.platform.reference.as_str();

        let volume = docker.prepare_exttool(Some(&image), worker).await.unwrap();
        assert_eq!(volume.as_deref(), Some("buckyos-exttool-444444444444"));
        assert_eq!(log_has(&store, "load -i"), 1);
        assert_eq!(
            log_has(
                &store,
                "touch /opt/buckyos/tools/.buckyos-seeded-444444444444"
            ),
            1
        );
        assert_eq!(
            log_has(&store, "image rm paios/exttool:0.7.0-07787a750c1e-amd64"),
            1,
            "D8"
        );

        // Seeded volume: no image needed, only the marker check with the worker image.
        store.lock().unwrap().log.clear();
        let volume = docker.prepare_exttool(Some(&image), worker).await.unwrap();
        assert_eq!(volume.as_deref(), Some("buckyos-exttool-444444444444"));
        let log = store.lock().unwrap().log.clone();
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(
            log[1].contains(worker)
                && log[1].contains("test -f /seed/.buckyos-seeded-444444444444")
        );

        // Interrupted seeding: the volume exists without its marker and is rebuilt.
        store.lock().unwrap().seeded = false;
        let volume = docker.prepare_exttool(Some(&image), worker).await.unwrap();
        assert!(volume.is_some());
        assert_eq!(log_has(&store, "volume rm buckyos-exttool-444444444444"), 1);
        assert_eq!(
            log_has(&store, "load -i"),
            1,
            "the dropped seed image comes back from its tarball"
        );
    }

    #[tokio::test]
    async fn unavailable_exttool_degrades_without_failing() {
        let (_root, aios, exttool) = pinned_root();
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let runner = fake_runner(store.clone());
        let docker = PaiosDocker::new(Some(&runner));
        assert_eq!(
            docker
                .prepare_exttool(None, &aios.platform.reference)
                .await
                .unwrap(),
            None
        );
        assert!(store.lock().unwrap().log.is_empty());

        std::fs::remove_file(&exttool.tar_path).unwrap();
        let image = pinned_image(PaiosImageKind::Exttool, &exttool);
        assert_eq!(
            docker
                .prepare_exttool(Some(&image), &aios.platform.reference)
                .await
                .unwrap(),
            None
        );
        assert_eq!(log_has(&store, "volume create"), 0);
    }

    #[tokio::test]
    async fn worker_expectation_reports_image_and_mountable_volume() {
        let (_root, aios, exttool) = pinned_root();
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let runner = fake_runner(store.clone());
        let docker = PaiosDocker::new(Some(&runner));
        let worker = pinned_image(PaiosImageKind::Aios, &aios);
        let exttool = pinned_image(PaiosImageKind::Exttool, &exttool);
        assert_eq!(
            docker
                .worker_expectation(&worker, Some(&exttool))
                .await
                .unwrap(),
            WorkerExpectation::default()
        );
        store
            .lock()
            .unwrap()
            .images
            .insert(aios.platform.reference.clone(), identity_of(&aios));
        store
            .lock()
            .unwrap()
            .volumes
            .push("buckyos-exttool-444444444444".to_string());
        assert_eq!(
            docker
                .worker_expectation(&worker, Some(&exttool))
                .await
                .unwrap(),
            WorkerExpectation {
                image_id: Some(aios.platform.image_id.clone()),
                exttool_volume: Some("buckyos-exttool-444444444444".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn garbage_collection_removes_only_what_node_daemon_loaded() {
        let (root, aios, exttool) = pinned_root();
        let store = Arc::new(Mutex::new(FakeStore::default()));
        store.lock().unwrap().loads.insert(
            aios.tar_path.to_string_lossy().to_string(),
            (aios.platform.reference.clone(), identity_of(&aios)),
        );
        let runner = fake_runner(store.clone());
        let docker = PaiosDocker::new(Some(&runner));
        let worker = pinned_image(PaiosImageKind::Aios, &aios);
        docker.ensure_image(&worker).await.unwrap();
        let mut state = load_loaded_images(root.path());
        assert_eq!(state.loaded.len(), 1);
        state.loaded.push(LoadedImage {
            reference: "paios/aios:0.6.9-000000000000-amd64".to_string(),
            image_id: format!("sha256:{}", "9".repeat(64)),
        });
        save_loaded_images(root.path(), &state);
        store.lock().unwrap().volumes.extend(
            [
                "buckyos-exttool",
                "buckyos-exttool-111111111111",
                "buckyos-exttool-444444444444",
                "other",
            ]
            .map(String::from),
        );
        let exttool = pinned_image(PaiosImageKind::Exttool, &exttool);
        docker
            .collect_garbage(
                root.path(),
                &[&worker, &exttool],
                Some("buckyos-exttool-444444444444"),
            )
            .await
            .unwrap();
        assert_eq!(
            log_has(&store, "image rm paios/aios:0.6.9-000000000000-amd64"),
            1
        );
        assert_eq!(
            log_has(&store, "image rm"),
            1,
            "the current and developer images stay"
        );
        assert_eq!(
            store.lock().unwrap().volumes,
            vec![
                "buckyos-exttool-444444444444".to_string(),
                "other".to_string()
            ]
        );
        assert_eq!(
            load_loaded_images(root.path()).loaded,
            vec![LoadedImage {
                reference: aios.platform.reference.clone(),
                image_id: aios.platform.image_id.clone(),
            }]
        );
    }
}
