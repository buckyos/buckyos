//! One offline transaction across independent filesystems. Each destination has a
//! private sibling on its own volume. The journal is durable before we create any
//! siblings, and every backup is durable before the first live rename.
use crate::readers::Change;
use anyhow::{bail, ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

const STATE: &str = ".node-updater";
const JOURNAL: &str = "transaction.json";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Staging,
    Applying,
    Committed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    id: Uuid,
    build: String,
    phase: Phase,
    entries: Vec<Entry>,
    created_dirs: BTreeSet<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    target: PathBuf,
    before: Option<String>,
    after: Option<String>,
}

pub struct Store {
    root: PathBuf,
    security: PathBuf,
    state: PathBuf,
    _lock: File,
}

#[derive(Debug)]
pub struct ApplyError {
    pub cause: anyhow::Error,
    pub recovery: &'static str,
}

#[derive(Debug)]
pub struct DataArea(pub PathBuf);

impl std::fmt::Display for DataArea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "migration of {}", self.0.display())
    }
}

pub fn metadata(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_dir(_: &Path) -> io::Result<()> {
    // Windows metadata barriers are issued on the renames themselves below.
    Ok(())
}

#[cfg(unix)]
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)?;
    sync_dir(to.parent().unwrap())?;
    if from.parent() != to.parent() {
        sync_dir(from.parent().unwrap())?;
    }
    Ok(())
}

#[cfg(windows)]
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn write_json(path: &Path, data: &impl Serialize) -> Result<()> {
    write_json_file(path, data, None)
}

pub fn write_install_receipt(
    path: &Path,
    data: &buckyos_api::completed_install::CompletedInstall,
) -> Result<()> {
    write_json_file(path, data, Some(0o644))
}

fn write_json_file(path: &Path, data: &impl Serialize, _unix_mode: Option<u32>) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut file, data)?;
    file.write_all(b"\n")?;
    #[cfg(unix)]
    if let Some(mode) = _unix_mode {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    file.as_file().sync_all()?;
    let source = file.into_temp_path();
    rename(&source, path)?;
    Ok(())
}

#[cfg(windows)]
use crate::windows_permissions::private_dir;

#[cfg(unix)]
fn private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    sync_dir(path.parent().unwrap())
}

fn remove(path: &Path) -> Result<()> {
    if let Some(meta) = metadata(path)? {
        if meta.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        sync_dir(path.parent().unwrap())?;
    }
    Ok(())
}

fn digest(path: &Path) -> Result<Option<String>> {
    fn visit(path: &Path, hash: &mut Sha256) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        if meta.is_file() {
            hash.update(b"F");
            hash.update(meta.len().to_le_bytes());
            io::copy(&mut File::open(path)?, hash)?;
        } else if meta.is_dir() {
            hash.update(b"D");
            let names = fs::read_dir(path)?
                .map(|e| e.map(|v| v.file_name()))
                .collect::<io::Result<BTreeSet<_>>>()?;
            hash.update((names.len() as u64).to_le_bytes());
            for name in names {
                let bytes = name.as_encoded_bytes();
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
                visit(&path.join(name), hash)?;
            }
        } else {
            bail!("unsupported file type at {}", path.display());
        }
        Ok(())
    }
    if metadata(path)?.is_none() {
        return Ok(None);
    }
    let mut hash = Sha256::new();
    visit(path, &mut hash)?;
    Ok(Some(format!("{:x}", hash.finalize())))
}

#[cfg(unix)]
#[path = "unix_permissions.rs"]
mod unix_permissions;

#[cfg(unix)]
#[cfg(unix)]
fn nearest_existing(path: &Path) -> io::Result<&Path> {
    for ancestor in path.ancestors() {
        if metadata(ancestor)?.is_some() {
            return Ok(ancestor);
        }
    }
    Err(io::ErrorKind::NotFound.into())
}

fn copy_tree(from: &Path, to: &Path, original: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(from)?;
    let existing = metadata(original)?;
    if meta.is_dir() {
        // The entire copy is inside the private slot. Set the original
        // directory's inheritable ACL before creating new storage files.
        fs::create_dir(to)?;
        if existing.is_some() {
            #[cfg(unix)]
            unix_permissions::access_metadata(original, to)?;
            #[cfg(windows)]
            crate::windows_permissions::copy_dacl(original, to)?;
        }
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &to.join(entry.file_name()),
                &original.join(entry.file_name()),
            )?;
        }
    } else if meta.is_file() {
        let mut input = File::open(from)?;
        let mut output = File::create_new(to)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
    } else {
        bail!("unsupported file type at {}", from.display());
    }
    #[cfg(unix)]
    unix_permissions::owner(nearest_existing(original)?, to)?;
    fs::set_permissions(to, existing.as_ref().unwrap_or(&meta).permissions())?;
    if existing.is_some() {
        #[cfg(unix)]
        unix_permissions::access_metadata(original, to)?;
        #[cfg(windows)]
        if !meta.is_dir() {
            crate::windows_permissions::copy_dacl(original, to)?;
        }
    }
    if to.is_dir() {
        sync_dir(to)?;
    } else {
        #[cfg(unix)]
        File::open(to)?.sync_all()?;
    }
    Ok(())
}

pub fn validate_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|c| !matches!(c, Component::ParentDir | Component::CurDir)),
        "expected an absolute installation path: {}",
        path.display()
    );
    for parent in path.ancestors() {
        if let Some(meta) = metadata(parent)? {
            ensure!(
                !meta.is_symlink(),
                "symbolic link in migration path: {}",
                parent.display()
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    meta.file_attributes() & 0x400 == 0,
                    "reparse point in migration path: {}",
                    parent.display()
                );
            }
        }
    }
    Ok(())
}

impl Store {
    pub fn open(root: &Path, security: &Path) -> Result<Self> {
        validate_path(root)?;
        validate_path(security)?;
        ensure!(root.is_dir(), "installation root must be a directory");
        let state = root.join(STATE);
        validate_path(&state)?;
        match private_dir(&state) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            other => other?,
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let owner = fs::metadata(&state)?;
            ensure!(
                owner.is_dir()
                    && owner.mode() & 0o077 == 0
                    && owner.uid() == rustix::process::geteuid().as_raw(),
                "migration state directory has unsafe ownership or permissions"
            );
        }
        #[cfg(windows)]
        crate::windows_permissions::check_private_state(&state)?;
        let path = state.join("lock");
        validate_path(&path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        lock.try_lock_exclusive()
            .context("another migration holds the installation lock")?;
        Ok(Self {
            root: root.into(),
            security: security.into(),
            state,
            _lock: lock,
        })
    }

    pub fn work(&self) -> Result<tempfile::TempDir> {
        // With the lock held, any old prepare directory was abandoned before apply.
        for entry in fs::read_dir(&self.state)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("prepare-") {
                remove(&entry.path())?;
            }
        }
        Ok(tempfile::Builder::new()
            .prefix("prepare-")
            .tempdir_in(&self.state)?)
    }

    fn validate_target(&self, path: &Path) -> Result<()> {
        validate_path(path)?;
        ensure!(
            (path.starts_with(&self.root)
                || path.starts_with(&self.security)
                || crate::readers::configured_external_db(&self.root, path)?)
                && path != self.root
                && path != self.security
                && !path.starts_with(&self.state)
                && !self.security.starts_with(path),
            "invalid migration destination {}",
            path.display()
        );
        Ok(())
    }

    fn journal(&self) -> PathBuf {
        self.state.join(JOURNAL)
    }

    fn slot(entry: &Entry, id: Uuid, i: usize) -> PathBuf {
        entry
            .target
            .parent()
            .unwrap()
            .join(format!(".node-updater-{id}-{i}"))
    }

    fn validate_journal(&self, j: &Journal, build: &str) -> Result<()> {
        ensure!(
            j.version == 1 && j.build == build,
            "pending migration requires build {}",
            j.build
        );
        for (i, entry) in j.entries.iter().enumerate() {
            self.validate_target(&entry.target)?;
            ensure!(
                j.entries[..i]
                    .iter()
                    .all(|p| !p.target.starts_with(&entry.target)
                        && !entry.target.starts_with(&p.target)),
                "overlapping migration destinations"
            );
            validate_path(&Self::slot(entry, j.id, i))?;
        }
        for path in &j.created_dirs {
            validate_path(path)?;
            ensure!(
                (path.starts_with(&self.root) || path.starts_with(&self.security))
                    && j.entries
                        .iter()
                        .any(|e| e.target.parent().unwrap().starts_with(path)),
                "invalid transaction directory"
            );
        }
        Ok(())
    }

    fn clean(&self, j: &Journal) -> Result<()> {
        for (i, entry) in j.entries.iter().enumerate() {
            remove(&Self::slot(entry, j.id, i))?;
        }
        if j.phase != Phase::Committed {
            for path in j.created_dirs.iter().rev() {
                match fs::remove_dir(path) {
                    Ok(()) => sync_dir(path.parent().unwrap())?,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        remove(&self.journal())
    }

    fn rollback(&self, j: &Journal) -> Result<()> {
        for (i, entry) in j.entries.iter().enumerate().rev() {
            let slot = Self::slot(entry, j.id, i);
            let actual = digest(&entry.target)?;
            if actual == entry.before && metadata(&slot.join("original"))?.is_none() {
                continue;
            }
            ensure!(
                actual.is_none() || actual == entry.after,
                "unexpected live contents at {}",
                entry.target.display()
            );
            if actual.is_some() {
                // Preserve the complete installed item until its original is restored.
                rename(&entry.target, &slot.join("reverted"))?;
            }
            if entry.before.is_some() {
                let prior = slot.join("original");
                let backup = slot.join("backup");
                let source = if metadata(&prior)?.is_some() {
                    &prior
                } else {
                    &backup
                };
                ensure!(
                    digest(source)? == entry.before,
                    "backup mismatch at {}",
                    entry.target.display()
                );
                rename(source, &entry.target)?;
            }
            ensure!(
                digest(&entry.target)? == entry.before,
                "restore mismatch at {}",
                entry.target.display()
            );
        }
        self.clean(j)
    }

    pub fn recover(&self, build: &str) -> Result<()> {
        if metadata(&self.journal())?.is_none() {
            return Ok(());
        }
        let j: Journal = serde_json::from_reader(File::open(self.journal())?)?;
        self.validate_journal(&j, build)?;
        match j.phase {
            Phase::Staging => self.clean(&j),
            Phase::Applying => self.rollback(&j),
            Phase::Committed => {
                for e in &j.entries {
                    ensure!(
                        digest(&e.target)? == e.after,
                        "committed data mismatch at {}",
                        e.target.display()
                    );
                }
                self.clean(&j)
            }
        }
    }

    fn plan(&self, changes: &[Change], work: &Path, build: &str) -> Result<Journal> {
        let mut j = Journal {
            version: 1,
            id: Uuid::new_v4(),
            build: build.into(),
            phase: Phase::Staging,
            entries: Vec::new(),
            created_dirs: BTreeSet::new(),
        };
        for change in changes {
            self.validate_target(&change.target)?;
            if let Some(source) = &change.permissions_from {
                self.validate_target(source)?;
                ensure!(
                    metadata(source)?.is_some(),
                    "missing permissions source {}",
                    source.display()
                );
            }
            let after = match &change.staged {
                None => None,
                Some(staged) => {
                    validate_path(staged)?;
                    ensure!(
                        staged.starts_with(work) && staged != work,
                        "staged path outside prepare directory"
                    );
                    Some(digest(staged)?.context("staged data is missing")?)
                }
            };
            let before = digest(&change.target)?;
            for parent in change.target.parent().unwrap().ancestors() {
                if metadata(parent)?.is_some() {
                    break;
                }
                j.created_dirs.insert(parent.to_owned());
            }
            j.entries.push(Entry {
                target: change.target.clone(),
                before,
                after,
            });
        }
        self.validate_journal(&j, build)?;
        Ok(j)
    }

    fn stage_entry(&self, e: &Entry, change: &Change, slot: &Path) -> Result<()> {
        private_dir(slot)?;
        if e.before.is_some() {
            copy_tree(&e.target, &slot.join("backup"), &e.target)?;
            ensure!(
                digest(&slot.join("backup"))? == e.before,
                "backup verification failed at {}",
                e.target.display()
            );
        }
        if let Some(staged) = &change.staged {
            let new = slot.join("new");
            let permissions = if e.before.is_some() {
                &e.target
            } else {
                change.permissions_from.as_ref().unwrap_or(&e.target)
            };
            copy_tree(staged, &new, permissions)?;
            #[cfg(unix)]
            if e.target.starts_with(&self.security) && new.is_file() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&new, fs::Permissions::from_mode(0o600))?;
                File::open(&new)?.sync_all()?;
            }
            ensure!(
                digest(&slot.join("new"))? == e.after,
                "staged verification failed at {}",
                e.target.display()
            );
        }
        sync_dir(slot)?;
        Ok(())
    }

    fn install_entry(e: &Entry, slot: &Path) -> Result<()> {
        if e.before.is_some() {
            rename(&e.target, &slot.join("original"))?;
        }
        if e.after.is_some() {
            rename(&slot.join("new"), &e.target)?;
        }
        ensure!(
            digest(&e.target)? == e.after,
            "apply verification failed at {}",
            e.target.display()
        );
        Ok(())
    }

    fn execute(
        &self,
        j: &mut Journal,
        changes: &[Change],
        mut checkpoint: impl FnMut(usize) -> Result<()>,
    ) -> Result<()> {
        write_json(&self.journal(), j)?;
        for parent in &j.created_dirs {
            private_dir(parent)?;
            let source = changes
                .iter()
                .find_map(|c| {
                    c.target
                        .starts_with(parent)
                        .then_some(c.permissions_from.as_deref())
                        .flatten()
                })
                .unwrap_or(parent.parent().unwrap());
            #[cfg(unix)]
            {
                unix_permissions::owner(source, parent)?;
                sync_dir(parent)?;
            }
            #[cfg(windows)]
            crate::windows_permissions::copy_dacl(source, parent)?;
        }
        for (i, (e, change)) in j.entries.iter().zip(changes).enumerate() {
            let slot = Self::slot(e, j.id, i);
            self.stage_entry(e, change, &slot)
                .with_context(|| DataArea(e.target.clone()))?;
        }
        j.phase = Phase::Applying;
        write_json(&self.journal(), j)?;
        for (i, e) in j.entries.iter().enumerate() {
            let slot = Self::slot(e, j.id, i);
            Self::install_entry(e, &slot).with_context(|| DataArea(e.target.clone()))?;
            checkpoint(i)?;
        }
        j.phase = Phase::Committed;
        write_json(&self.journal(), j)?;
        checkpoint(j.entries.len())?;
        self.clean(j)
    }

    pub fn apply(&self, changes: &[Change], work: &Path, build: &str) -> Result<(), ApplyError> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut j = self
            .plan(changes, work, build)
            .map_err(|cause| ApplyError {
                cause,
                recovery: "unchanged",
            })?;
        if let Err(cause) = self.execute(&mut j, changes, |_| Ok(())) {
            // Re-read durable intent. An I/O error can occur after a rename succeeded.
            let durable = self.journal();
            let phase = (|| -> Result<Option<Phase>> {
                if metadata(&durable)?.is_none() {
                    return Ok(None);
                }
                Ok(Some(
                    serde_json::from_reader::<_, Journal>(File::open(durable)?)?.phase,
                ))
            })();
            let recovery = match phase {
                Ok(Some(Phase::Committed)) => "incomplete",
                Ok(Some(Phase::Applying)) if self.recover(build).is_ok() => "rolled_back",
                Ok(Some(Phase::Staging)) if self.recover(build).is_ok() => "unchanged",
                Ok(None) if j.phase == Phase::Staging => "unchanged",
                _ => "incomplete",
            };
            return Err(ApplyError { cause, recovery });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
    }

    fn inputs(root: &Path, work: &Path) -> Vec<Change> {
        // One directory (sled), a file on another logical partition, and an absent destination.
        for (base, version) in [(root, "old"), (work, "new")] {
            fs::create_dir(base.join("config")).unwrap();
            fs::write(base.join("config/db"), version).unwrap();
            fs::write(base.join("tasks"), version).unwrap();
        }
        fs::write(work.join("start_config"), b"identity").unwrap();
        vec![
            Change {
                permissions_from: None,
                target: root.join("config"),
                staged: Some(work.join("config")),
            },
            Change {
                permissions_from: None,
                target: root.join("tasks"),
                staged: Some(work.join("tasks")),
            },
            Change {
                permissions_from: None,
                target: root.join("security/start_config"),
                staged: Some(work.join("start_config")),
            },
        ]
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn replacement_preserves_explicit_file_acl_and_extended_attributes() {
        use rustix::fs::{getxattr, setxattr, XattrFlags};
        let root = temp_root();
        let store = Store::open(root.path(), &root.path().join("security")).unwrap();
        let work = store.work().unwrap();
        let target = root.path().join("db");
        let new = work.path().join("converted");
        fs::write(&target, "old").unwrap();
        fs::write(&new, "new").unwrap();
        // Linux's little-endian POSIX ACL wire format. Give a named user read access.
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, permission, id) in [
            (1u16, 6u16, !0u32),
            (2, 4, 65534),
            (4, 0, !0),
            (16, 4, !0),
            (32, 0, !0),
        ] {
            acl.extend(tag.to_le_bytes());
            acl.extend(permission.to_le_bytes());
            acl.extend(id.to_le_bytes());
        }
        setxattr(
            &target,
            "system.posix_acl_access",
            &acl,
            XattrFlags::empty(),
        )
        .unwrap();
        setxattr(
            &target,
            "user.buckyos.test",
            b"retained\0bytes",
            XattrFlags::empty(),
        )
        .unwrap();
        let change = Change {
            target: target.clone(),
            staged: Some(new),
            permissions_from: None,
        };
        store.apply(&[change], work.path(), "v").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        let mut buf = [0u8; 128];
        let n = getxattr(&target, "user.buckyos.test", &mut buf[..]).unwrap();
        assert_eq!(&buf[..n], b"retained\0bytes");
        let n = getxattr(&target, "system.posix_acl_access", &mut buf[..]).unwrap();
        assert_eq!(buf[..n], acl);
    }

    #[test]
    #[cfg(unix)]
    fn new_private_directory_and_moved_file_keep_the_service_owner() {
        use std::os::unix::fs::MetadataExt;
        let root = temp_root();
        let security = root.path().join("security");
        let store = Store::open(root.path(), &security).unwrap();
        let work = store.work().unwrap();
        let source = root.path().join("old");
        let converted = work.path().join("converted");
        fs::write(&source, "old").unwrap();
        if rustix::process::geteuid().is_root() {
            std::os::unix::fs::chown(&source, Some(65534), Some(65534)).unwrap();
        }
        fs::write(&converted, "new").unwrap();
        let target = security.join("start_config.json");
        let change = Change {
            target: target.clone(),
            staged: Some(converted),
            permissions_from: Some(source.clone()),
        };
        store.apply(&[change], work.path(), "v").unwrap();
        let owner = source.metadata().unwrap();
        for (path, mode) in [(security, 0o700), (target, 0o600)] {
            let attrs = path.metadata().unwrap();
            assert_eq!((attrs.uid(), attrs.gid()), (owner.uid(), owner.gid()));
            assert_eq!(attrs.mode() & 0o777, mode);
        }
    }

    #[test]
    fn interrupted_transactions_restore_or_finish_according_to_durable_commit() {
        // Stop after each installed destination and just after the commit record.
        for interrupt in 0..=3 {
            let root = temp_root();
            let security = root.path().join("security");
            let store = Store::open(root.path(), &security).unwrap();
            let work = store.work().unwrap();
            let changes = inputs(root.path(), work.path());
            let mut j = store.plan(&changes, work.path(), "target1").unwrap();
            let outcome = store.execute(&mut j, &changes, |i| {
                ensure!(i != interrupt, "simulated power loss");
                Ok(())
            });
            assert!(outcome.is_err());
            drop(work);
            drop(store);
            let rerun = Store::open(root.path(), &security).unwrap();
            assert!(rerun.recover("target2").is_err());
            rerun.recover("target1").unwrap();
            rerun.recover("target1").unwrap();
            let expected = if interrupt == 3 { "new" } else { "old" };
            assert_eq!(
                fs::read_to_string(root.path().join("config/db")).unwrap(),
                expected
            );
            assert_eq!(
                fs::read_to_string(root.path().join("tasks")).unwrap(),
                expected
            );
            assert_eq!(security.exists(), interrupt == 3);
            assert!(!rerun.journal().exists());
        }
    }

    #[test]
    fn ambiguous_committed_contents_leave_recovery_materials_and_block() {
        let root = temp_root();
        let store = Store::open(root.path(), &root.path().join("security")).unwrap();
        let work = store.work().unwrap();
        let changes = inputs(root.path(), work.path());
        let mut j = store.plan(&changes, work.path(), "v").unwrap();
        store
            .execute(&mut j, &changes, |i| {
                ensure!(i != 3, "stopped");
                Ok(())
            })
            .unwrap_err();
        fs::write(&changes[1].target, "unrecognized").unwrap();
        assert!(store.recover("v").is_err());
        assert!(store.journal().is_file());
    }

    #[test]
    fn late_prepare_error_and_overlap_cannot_install_first_change() {
        let root = temp_root();
        let store = Store::open(root.path(), &root.path().join("security")).unwrap();
        assert!(Store::open(root.path(), &root.path().join("security")).is_err());
        let work = store.work().unwrap();
        let mut changes = inputs(root.path(), work.path());
        changes.last_mut().unwrap().staged = Some(work.path().join("missing"));
        assert_eq!(
            store
                .apply(&changes, work.path(), "v")
                .unwrap_err()
                .recovery,
            "unchanged"
        );
        changes.last_mut().unwrap().staged = None;
        changes.last_mut().unwrap().target = root.path().join("config/db");
        assert!(store.apply(&changes, work.path(), "v").is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("config/db")).unwrap(),
            "old"
        );
        assert!(!store.journal().exists());
        assert!(!root.path().join("security").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_to_a_different_store() {
        let root = temp_root();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("db")).unwrap();
        let store = Store::open(root.path(), &root.path().join("security")).unwrap();
        let work = store.work().unwrap();
        let change = Change {
            permissions_from: None,
            target: root.path().join("db/user.sqlite"),
            staged: None,
        };
        assert!(store.apply(&[change], work.path(), "v").is_err());
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}
