//! AgentRoot layout, the identity it belongs to, and its sync from the
//! agent package. Files the agent (or its owner) changed locally are never
//! overwritten by a newer package: the manifest remembers what was
//! installed, a file is replaced only while it still equals what was
//! installed.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use libopendan::state::ROLE_SUPPLEMENT_FILE;
use ndn_lib::ObjId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SYNC_MANIFEST: &str = ".meta/rootfs_sync.json";
pub const IDENTITY_FILE: &str = ".meta/identity.json";
/// Next to the AgentRoots: AgentRoots of earlier agents of the same name.
pub const ARCHIVE_DIR: &str = ".archived";
const SYNC_VERSION: u32 = 1;

/// The agent an AgentRoot belongs to. `agent_doc_object_id` is the
/// AgentDocument of its AgentSpec (`None` outside a zone): an agent deleted
/// and created again under the same name has a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootIdentity {
    pub agent_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_doc_object_id: Option<ObjId>,
}

impl RootIdentity {
    fn same_agent(&self, other: &RootIdentity) -> bool {
        self.agent_did == other.agent_did
            && match (&self.agent_doc_object_id, &other.agent_doc_object_id) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

fn is_empty_dir(dir: &Path) -> Result<bool> {
    match fs::read_dir(dir) {
        Ok(mut rd) => Ok(rd.next().is_none()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e).with_context(|| format!("read_dir {}", dir.display())),
    }
}

/// Make `agent_root` the AgentRoot of `identity`. An AgentRoot recorded as
/// another agent's, or an earlier agent's of the same name, is moved to
/// `<parent>/.archived/<name>-<ms>` (returned) and a new one is started:
/// nothing is inherited from it. A directory nobody claimed yet (prepared
/// by hand) is taken as it is.
pub fn claim(agent_root: &Path, identity: &RootIdentity) -> Result<Option<PathBuf>> {
    let path = agent_root.join(IDENTITY_FILE);
    let mut archived = None;
    if !is_empty_dir(agent_root)? {
        let stored = match fs::read_to_string(&path) {
            Ok(raw) => Some(serde_json::from_str::<RootIdentity>(&raw).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        match stored {
            None => {}
            Some(Some(stored)) if stored.same_agent(identity) => {
                if stored.agent_doc_object_id.is_some() || identity.agent_doc_object_id.is_none() {
                    return Ok(None);
                }
            }
            Some(stored) => {
                let name = agent_root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "agent".to_string());
                let parent = agent_root.parent().unwrap_or(Path::new("."));
                let target = parent
                    .join(ARCHIVE_DIR)
                    .join(format!("{name}-{}", libopendan::now_ms()));
                fs::create_dir_all(target.parent().unwrap_or(parent))
                    .with_context(|| format!("create {}", parent.join(ARCHIVE_DIR).display()))?;
                fs::rename(agent_root, &target).with_context(|| {
                    format!("archive {} to {}", agent_root.display(), target.display())
                })?;
                log::warn!(
                    "rootfs: {} belonged to {}; archived to {}",
                    agent_root.display(),
                    stored
                        .map(|s| format!("{} ({:?})", s.agent_did, s.agent_doc_object_id.map(|o| o.to_string())))
                        .unwrap_or_else(|| "an unknown agent".to_string()),
                    target.display()
                );
                archived = Some(target);
            }
        }
    }
    ensure_layout(agent_root)?;
    libopendan::fsutil::atomic_replace_json(&path, identity)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(archived)
}

/// The AgentRoot has been synced from a package at least once.
pub fn initialized(agent_root: &Path) -> bool {
    agent_root.join(SYNC_MANIFEST).is_file()
}

/// The owner's supplement to the role, read by the behavior catalog after
/// `role.md`. Empty: no file.
pub fn write_role_supplement(agent_root: &Path, text: &str) -> Result<()> {
    let path = agent_root.join(ROLE_SUPPLEMENT_FILE);
    let text = text.trim();
    if text.is_empty() {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("remove {}", path.display()))
            }
            _ => Ok(()),
        };
    }
    if fs::read_to_string(&path).is_ok_and(|cur| cur == format!("{text}\n")) {
        return Ok(());
    }
    libopendan::fsutil::atomic_replace(&path, format!("{text}\n").as_bytes())
        .with_context(|| format!("write {}", path.display()))
}

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

    fn identity(did: &str, doc: Option<&str>) -> RootIdentity {
        RootIdentity {
            agent_did: did.to_string(),
            agent_doc_object_id: doc.map(|d| ObjId::new(&format!("agentdoc:{}", d.repeat(32))).unwrap()),
        }
    }

    #[test]
    fn an_agentroot_of_another_agent_is_archived() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("agents").join("xiaobai.example.com");
        let first = identity("did:web:xiaobai.example.com", Some("aa"));
        assert_eq!(claim(&root, &first).unwrap(), None);
        fs::write(root.join("memory/notes.md"), "kept").unwrap();
        // The same agent keeps its AgentRoot.
        assert_eq!(claim(&root, &first).unwrap(), None);
        assert!(root.join("memory/notes.md").is_file());
        // Deleted and created again under the same name: a new AgentDocument.
        let second = identity("did:web:xiaobai.example.com", Some("bb"));
        let archived = claim(&root, &second).unwrap().expect("archived");
        assert!(archived.starts_with(dir.path().join("agents").join(ARCHIVE_DIR)));
        assert!(archived.file_name().unwrap().to_string_lossy().starts_with("xiaobai.example.com-"));
        assert_eq!(fs::read_to_string(archived.join("memory/notes.md")).unwrap(), "kept");
        assert!(!root.join("memory/notes.md").exists());
        let stored: RootIdentity = serde_json::from_str(&fs::read_to_string(root.join(IDENTITY_FILE)).unwrap()).unwrap();
        assert_eq!(stored, second);
        // A damaged record names nobody: archived as well.
        fs::write(root.join(IDENTITY_FILE), "{").unwrap();
        assert!(claim(&root, &second).unwrap().is_some());
        // A directory nobody claimed yet is taken as it is.
        let prepared = dir.path().join("agents").join("prepared");
        fs::create_dir_all(&prepared).unwrap();
        fs::write(prepared.join("role.md"), "by hand").unwrap();
        assert_eq!(claim(&prepared, &identity("did:web:prepared.example.com", None)).unwrap(), None);
        assert!(prepared.join("role.md").is_file() && prepared.join(IDENTITY_FILE).is_file());
    }

    #[test]
    fn an_identity_without_a_document_learns_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        assert_eq!(claim(&root, &identity("did:bns:jarvis", None)).unwrap(), None);
        assert_eq!(claim(&root, &identity("did:bns:jarvis", Some("aa"))).unwrap(), None);
        let stored: RootIdentity = serde_json::from_str(&fs::read_to_string(root.join(IDENTITY_FILE)).unwrap()).unwrap();
        assert!(stored.agent_doc_object_id.is_some());
        // A development run without a document is the same agent.
        assert_eq!(claim(&root, &identity("did:bns:jarvis", None)).unwrap(), None);
        assert!(claim(&root, &identity("did:bns:other", None)).unwrap().is_some());
    }

    #[test]
    fn the_role_supplement_file_follows_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        ensure_layout(dir.path()).unwrap();
        let path = dir.path().join(ROLE_SUPPLEMENT_FILE);
        write_role_supplement(dir.path(), "  默认使用日语回答。\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "默认使用日语回答。\n");
        write_role_supplement(dir.path(), " ").unwrap();
        assert!(!path.exists());
        write_role_supplement(dir.path(), "").unwrap();
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
