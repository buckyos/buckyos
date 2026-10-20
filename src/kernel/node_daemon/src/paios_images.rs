//! paios image pinning (`bin/paios-images/paios_images.json`).
//!
//! Release payloads ship a manifest that pins the worker (`aios`) and ExtTool
//! (`exttool`) images by identity, plus the matching `docker save` tarballs in
//! the same directory. Without the manifest node_daemon runs in local mode and
//! uses `<repo>:latest-<arch>` from the local Docker store. Neither mode pulls.

use crate::app_loader::PlatformArch;
use log::warn;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

pub(crate) const PAIOS_IMAGES_DIR: &str = "bin/paios-images";
pub(crate) const PAIOS_IMAGES_MANIFEST: &str = "paios_images.json";
pub(crate) const PAIOS_IMAGES_SCHEMA: &str = "buckyos.paios-images/v1";
pub(crate) const IMAGE_REVISION_LABEL: &str = "org.opencontainers.image.revision";
const DEVENV_JSON_RELATIVE_PATH: &str = "etc/devenv.json";

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
    validate_manifest(&manifest).map_err(|error| format!("invalid {}: {}", path.display(), error))?;
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
        ("rootfs_layers_sha256", Some(platform.rootfs_layers_sha256.as_str())),
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
        if manifest.is_some() {
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
        assert_eq!(exttool.reference, "paios/exttool:0.7.0-07787a750c1e-aarch64");
    }

    #[test]
    fn broken_manifest_is_an_error_not_local_mode() {
        let root = root_with(Some("{not json"), None);
        assert!(resolve(PaiosImageKind::Aios, PlatformArch::Amd64, root.path()).is_err());

        let root = root_with(
            Some(&fixture_with(|value| value["schema"] = "buckyos.paios-images/v2".into())),
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
        assert!(load_manifest(root.path()).unwrap_err().contains("images.aios"));
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
        for file in ["../aios.tar.gz", "sub/aios.tar.gz", "sub\\aios.tar.gz", "..", ""] {
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
        assert!(resolve(PaiosImageKind::Exttool, PlatformArch::Amd64, root.path())
            .unwrap()
            .is_none());
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
            format!("sha256:{}", hex::encode(Sha256::digest(b"sha256:a\nsha256:b")))
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
        assert!(missing.contains("bin/paios-images/aios.tar.gz"), "{missing}");
        std::fs::write(&path, b"image").unwrap();
        verify_tar(&path, &expected).unwrap();
        std::fs::write(&path, b"imagf").unwrap();
        assert!(verify_tar(&path, &expected).unwrap_err().contains("corrupted"));
    }
}
