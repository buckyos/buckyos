//! All service open/recovery routines run on private copies while writers are stopped.
mod appdata;
mod config;
mod files;
mod identity_config;
mod sql;
pub use sql::configured_external_db;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct Change {
    pub target: PathBuf,
    pub staged: Option<PathBuf>,
    pub permissions_from: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Failure {
    pub module: String,
    pub area: String,
    pub reason: String,
    pub unsupported: bool,
}

impl Display for Failure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: {}", self.module, self.area, self.reason)
    }
}

impl std::error::Error for Failure {}

type RawConfig = BTreeMap<Vec<u8>, Vec<u8>>;
struct Context<'a> {
    root: &'a Path,
    security: &'a Path,
    work: &'a Path,
    module: &'static str,
    confirm: &'a dyn Fn(&str) -> anyhow::Result<bool>,
    snapshot: RawConfig,
    rdb: BTreeMap<(String, String), Vec<PathBuf>>,
    editor: appdata::TextEditorData,
    agents: Vec<PathBuf>,
    owned: BTreeSet<PathBuf>,
    changes: Vec<Change>,
    owners: BTreeMap<PathBuf, &'static str>,
    next_copy: usize,
}

impl Context<'_> {
    fn run(
        &mut self,
        id: &'static str,
        reader: fn(&mut Context<'_>) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        self.module = id;
        let first = self.changes.len();
        reader(self)?;
        for change in &self.changes[first..] {
            if let Some(other) = self.owners.insert(change.target.clone(), id) {
                return Err(self.error(
                    change.target.display(),
                    format!("data change already owned by {other}"),
                ));
            }
        }
        Ok(())
    }

    fn error(&self, area: impl Display, e: impl Display) -> Failure {
        Failure {
            module: self.module.into(),
            area: area.to_string(),
            reason: e.to_string(),
            unsupported: false,
        }
    }

    fn unsupported(&self, area: impl Display, e: impl Display) -> Failure {
        Failure {
            unsupported: true,
            ..self.error(area, e)
        }
    }

    fn own(&mut self, path: &Path) -> Result<(), Failure> {
        if self.owned.insert(path.to_owned()) {
            Ok(())
        } else {
            Err(self.unsupported(
                path.display(),
                "multiple registrations resolve to the same data area",
            ))
        }
    }

    fn copy_path(&mut self) -> PathBuf {
        self.next_copy += 1;
        self.work.join(format!("store-{}", self.next_copy))
    }

    fn exists(&self, path: &Path) -> Result<bool, Failure> {
        // Validate every component even when the leaf is absent or a broken link.
        let base = if path.starts_with(self.root) {
            self.root
        } else if path.starts_with(self.security) {
            self.security
        } else {
            if !configured_external_db(self.root, path)
                .map_err(|e| self.error(path.display(), e))?
            {
                return Err(self.error(
                    path.display(),
                    "path is outside the configured installation data areas",
                ));
            }
            return path.try_exists().map_err(|e| self.error(path.display(), e));
        };
        let mut here = base.to_path_buf();
        let components = path.strip_prefix(base).unwrap().components();
        for component in std::iter::once(Component::CurDir).chain(components) {
            match component {
                Component::Normal(name) => here.push(name),
                Component::CurDir => (),
                _ => return Err(self.error(path.display(), "invalid path component")),
            }
            match fs::symlink_metadata(&here) {
                Ok(m) if linked(&m) => {
                    return Err(self.error(here.display(), "symbolic link in data path"))
                }
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(self.error(here.display(), e)),
            }
        }
        Ok(true)
    }

    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, Failure> {
        if !self.exists(path)? {
            return Ok(Vec::new());
        }
        let mut result = fs::read_dir(path)
            .and_then(|entries| {
                entries
                    .map(|entry| entry.map(|e| e.path()))
                    .collect::<io::Result<Vec<_>>>()
            })
            .map_err(|e| self.error(path.display(), e))?;
        result.sort();
        Ok(result)
    }

    fn json(&self, path: &Path) -> Result<serde_json::Value, Failure> {
        if !self.exists(path)? {
            return Err(self.unsupported(path.display(), "required data file missing"));
        }
        let file = fs::File::open(path).map_err(|e| self.error(path.display(), e))?;
        serde_json::from_reader(file).map_err(|e| self.unsupported(path.display(), e))
    }
}

fn linked(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes directory junctions as well as symbolic links.
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.is_symlink()
    }
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if linked(&meta) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("linked data path {}", src.display()),
        ));
    }
    if meta.is_file() {
        fs::copy(src, dst)?;
    } else if meta.is_dir() {
        fs::create_dir(dst)?;
        for entry in fs::read_dir(src)? {
            let e = entry?;
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        fs::set_permissions(dst, meta.permissions())?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a regular file or directory", src.display()),
        ));
    }
    Ok(())
}

include!(concat!(env!("OUT_DIR"), "/readers.rs"));

#[derive(Debug)]
pub struct Prepared {
    pub changes: Vec<Change>,
    pub owners: BTreeMap<PathBuf, &'static str>,
    pub source_build: Option<String>,
}

pub fn prepare(
    root: &Path,
    security_root: &Path,
    work: &Path,
    confirm: &dyn Fn(&str) -> anyhow::Result<bool>,
) -> Result<Prepared, Failure> {
    let mut ctx = Context {
        root,
        security: security_root,
        confirm,
        work,
        module: "system-config",
        snapshot: Default::default(),
        rdb: Default::default(),
        editor: Default::default(),
        agents: Vec::new(),
        owned: Default::default(),
        owners: Default::default(),
        changes: Vec::new(),
        next_copy: 0,
    };
    run_registered(&mut ctx)?;
    ctx.module = "system-config";
    let source_build = ctx
        .snapshot
        .get(buckyos_api::BUCKYOS_INFO_KEY.as_bytes())
        .map(|value| -> Result<String, Failure> {
            let info: buckyos_api::BuckyOSInfo = serde_json::from_slice(value)
                .map_err(|e| ctx.unsupported(buckyos_api::BUCKYOS_INFO_KEY, e))?;
            Ok(match info.build_version {
                Some(build) => format!("{}+{build}", info.version),
                None => info.version,
            })
        })
        .transpose()?;
    Ok(Prepared {
        changes: ctx.changes,
        owners: ctx.owners,
        source_build,
    })
}
