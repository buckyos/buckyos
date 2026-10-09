//! AgentRoot layout and its sync from the agent package. Files the agent
//! (or its owner) changed locally are never overwritten by a newer package:
//! the manifest remembers what was installed, a file is replaced only while
//! it still equals what was installed.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SYNC_MANIFEST: &str = ".meta/rootfs_sync.json";
const SYNC_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    files: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    source_sha256: String,
    installed_sha256: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    pub copied: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub preserved: usize,
}

pub fn ensure_layout(agent_root: &Path) -> Result<()> {
    for rel in ["", ".meta", "behaviors", "skills", "tools", "memory", "notebook", "workspace", "sessions"] {
        let dir = agent_root.join(rel);
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            log::warn!("rootfs: skip package symlink {}", path.display());
        } else if ty.is_dir() {
            collect(&path, out)?;
        } else if ty.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn copy(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::copy(source, target)
        .with_context(|| format!("copy {} -> {}", source.display(), target.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(source)?.permissions();
        perm.set_mode(perm.mode() | 0o200);
        fs::set_permissions(target, perm)?;
    }
    Ok(())
}

/// The manifest is the Loader's only persistent state of its own: it is
/// written with a real fsync and a damaged one stops the start instead of
/// being taken for an empty one (which would overwrite local changes).
fn load_manifest(path: &Path) -> Result<Manifest> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).with_context(|| {
            format!(
                "{} is damaged; repair or remove it by hand (removing it makes the next sync keep every file that differs from the package)",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

fn save_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    libopendan::fsutil::atomic_replace_json(path, manifest)
        .with_context(|| format!("write {}", path.display()))
}

pub fn sync_from_package(package_root: &Path, agent_root: &Path) -> Result<SyncReport> {
    if !package_root.is_dir() {
        bail!("agent package directory {} does not exist", package_root.display());
    }
    let manifest_path = agent_root.join(SYNC_MANIFEST);
    let mut manifest = load_manifest(&manifest_path)?;
    manifest.version = SYNC_VERSION;
    let mut report = SyncReport::default();
    let mut files = Vec::new();
    collect(package_root, &mut files)?;
    files.sort();
    for source in files {
        let rel = source.strip_prefix(package_root)?;
        let key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        if key == SYNC_MANIFEST {
            continue;
        }
        let target = agent_root.join(rel);
        let source_hash = sha256_file(&source)?;
        let existing = target.is_file().then(|| sha256_file(&target)).transpose()?;
        let previous = manifest.files.get(&key).cloned();
        let untouched = match (existing.as_deref(), &previous) {
            (None, _) => true,
            (Some(cur), Some(p)) => cur == p.installed_sha256,
            (Some(cur), None) => cur == source_hash,
        };
        match (&existing, untouched) {
            (None, _) => {
                copy(&source, &target)?;
                report.copied += 1;
            }
            (Some(cur), true) if cur != &source_hash => {
                copy(&source, &target)?;
                report.updated += 1;
            }
            (Some(_), true) => report.unchanged += 1,
            (Some(_), false) => {
                report.preserved += 1;
                log::warn!("rootfs: keep locally modified {}", target.display());
            }
        }
        manifest.files.insert(
            key,
            Entry {
                installed_sha256: if untouched {
                    source_hash.clone()
                } else {
                    previous
                        .map(|p| p.installed_sha256)
                        .unwrap_or_else(|| existing.unwrap_or_default())
                },
                source_sha256: source_hash,
            },
        );
    }
    save_manifest(&manifest_path, &manifest)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_updates_reach_untouched_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let (pkg, root) = (dir.path().join("pkg"), dir.path().join("root"));
        fs::create_dir_all(pkg.join("behaviors")).unwrap();
        fs::write(pkg.join("role.md"), "role v1").unwrap();
        fs::write(pkg.join("behaviors/do.toml"), "v1").unwrap();
        ensure_layout(&root).unwrap();
        let r = sync_from_package(&pkg, &root).unwrap();
        assert_eq!((r.copied, r.updated, r.preserved), (2, 0, 0));

        fs::write(root.join("role.md"), "my own role").unwrap();
        fs::write(pkg.join("role.md"), "role v2").unwrap();
        fs::write(pkg.join("behaviors/do.toml"), "v2").unwrap();
        let r = sync_from_package(&pkg, &root).unwrap();
        assert_eq!((r.copied, r.updated, r.preserved), (0, 1, 1));
        assert_eq!(fs::read_to_string(root.join("role.md")).unwrap(), "my own role");
        assert_eq!(fs::read_to_string(root.join("behaviors/do.toml")).unwrap(), "v2");
        // Still preserved on the next start, and files the agent created
        // itself are never touched.
        fs::write(root.join("skills/mine.md"), "x").unwrap();
        let r = sync_from_package(&pkg, &root).unwrap();
        assert_eq!((r.unchanged, r.preserved), (1, 1));
        assert!(root.join("skills/mine.md").is_file());
    }

    #[test]
    fn a_damaged_manifest_stops_the_sync() {
        let dir = tempfile::tempdir().unwrap();
        let (pkg, root) = (dir.path().join("pkg"), dir.path().join("root"));
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("role.md"), "role").unwrap();
        ensure_layout(&root).unwrap();
        fs::write(root.join(SYNC_MANIFEST), "{ not json").unwrap();
        let err = format!("{:#}", sync_from_package(&pkg, &root).unwrap_err());
        assert!(err.contains("damaged"), "{err}");
        assert!(!root.join("role.md").exists());
    }
}
