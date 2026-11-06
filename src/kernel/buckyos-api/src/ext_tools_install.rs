//! Installing extension packages under a tools root: `$BUCKYOS_ROOT/tools`, or the node's tools volume as a worker
//! container sees it (`opendan ext-tools`, so one implementation serves both). The only writer of the layout:
//!
//! ```text
//! bin/<launcher>                      copies from the active version of the package that provides them
//! store/<id>                          pointer (symlink, on Windows a junction) to versions/<id>/<dir>/store/<id>
//! versions/<id>/<version>-<digest12>/ one payload, unpacked; the previous one stays for rollback
//! versions/<id>/<dir>.json            what that directory holds
//! installs/<id>.json                  what is active
//! current.json                        id -> version and digest of everything active
//! staging/                            payloads being checked; nothing here is in use
//! journal.json                        the activation in progress
//! ```
//!
//! A version directory never changes once made, so a running tool keeps its files while another version is activated
//! and Windows file locks only ever hit `bin/`. Every failure before the pointer moves leaves the old version untouched;
//! a failure after it moves is rolled back, and an interrupted activation is finished or undone by the next call.

use crate::ext_tools::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const EXT_TOOLS_INSTALL_SCHEMA: &str = "buckyos.ext_tools.install/1";
/// What a release ships next to the payloads: `bin/ext-tools/ext_tools.json`.
pub const EXT_TOOLS_PIN_SCHEMA: &str = "buckyos.ext-tools/v1";
pub const EXT_TOOLS_PIN_DIR: &str = "bin/ext-tools";
pub const EXT_TOOLS_PIN_FILE: &str = "ext_tools.json";
/// Skip the smoke of a seeded package, for machines where startup time matters more than the check.
pub const EXT_TOOLS_SKIP_SMOKE_ENV: &str = "BUCKYOS_EXT_TOOLS_SKIP_SMOKE";
const SMOKE_TIMEOUT: Duration = Duration::from_secs(180);
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const JOURNAL_FILE: &str = "journal.json";
const LOCK_FILE: &str = ".lock";

pub type ExtToolsResult<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtToolsOrigin {
    /// Unpacked from the release's `bin/ext-tools`: it stays until the desired state says otherwise.
    Seed,
    /// Installed because the desired state asked for it: it goes when the desired state drops it.
    Desired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsPreviousVersion {
    pub version: String,
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsInstall {
    pub schema: String,
    pub id: String,
    pub version: String,
    /// `sha256:<hex>` of the payload tar.
    pub payload_digest: String,
    /// `sha256:<hex>` of the `.pikg` the payload belongs to, where known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
    pub platform: String,
    pub origin: ExtToolsOrigin,
    /// The version directory.
    pub dir: String,
    /// Commands of `provides`, each with what its `version_probe` printed.
    pub commands: BTreeMap<String, String>,
    /// File names in `bin/`.
    pub launchers: Vec<String>,
    pub smoke: ExtToolsSmokeResult,
    pub activated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<ExtToolsPreviousVersion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtToolsSmokeResult {
    Passed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VersionSidecar {
    version: String,
    payload_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pikg_digest: Option<String>,
    platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Journal {
    id: String,
    to_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsPinPlatform {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsPinPackage {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
    pub platforms: BTreeMap<String, ExtToolsPinPlatform>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsPinFile {
    pub schema: String,
    pub packages: BTreeMap<String, ExtToolsPinPackage>,
}

impl ExtToolsPinFile {
    pub fn load(dir: &Path) -> ExtToolsResult<Option<Self>> {
        let path = dir.join(EXT_TOOLS_PIN_FILE);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let pin: Self = serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))?;
        if pin.schema != EXT_TOOLS_PIN_SCHEMA {
            return Err(format!("{}: schema must be {EXT_TOOLS_PIN_SCHEMA}", path.display()));
        }
        Ok(Some(pin))
    }
}

pub struct ExtToolsApply<'a> {
    /// The payload `.tar.gz`.
    pub tar: &'a Path,
    pub id: &'a str,
    /// `sha256:<hex>` the tar must have.
    pub expected_digest: Option<&'a str>,
    pub pikg_digest: Option<&'a str>,
    pub platform: &'a ExtToolsPlatform,
    pub origin: ExtToolsOrigin,
    pub skip_smoke: bool,
}

pub struct ToolsRoot {
    root: PathBuf,
}

/// Held while the layout changes; the OS releases it when the process dies.
pub struct ToolsRootLock {
    _file: fs::File,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn ext_tools_sha256_file(path: &Path) -> ExtToolsResult<String> {
    let mut file = BufReader::new(fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

fn normal_relative(path: &Path) -> bool {
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Where a symlink at `entry` pointing at `link` lands, without touching the disk; `None`: outside the root.
fn resolved_inside(entry: &Path, link: &Path) -> Option<PathBuf> {
    if link.is_absolute() {
        return None;
    }
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    for component in entry.parent().unwrap_or(Path::new("")).components().chain(link.components()) {
        match component {
            Component::Normal(part) => stack.push(part.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                stack.pop()?;
            }
            _ => return None,
        }
    }
    Some(stack.iter().collect())
}

fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

/// Unpack a payload into the empty `dest`. Nothing leaves `dest`: paths are relative with no `..`, a symlink may only
/// point inside, and special files and set-id bits do not exist in a package. Extended attributes are dropped, which
/// is also what keeps `com.apple.quarantine` of a downloaded archive off every file on macOS.
pub fn ext_tools_unpack(tar_gz: &Path, dest: &Path) -> ExtToolsResult<()> {
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let file = fs::File::open(tar_gz).map_err(|e| format!("{}: {e}", tar_gz.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(file)));
    archive.set_unpack_xattrs(false);
    archive.set_preserve_permissions(false);
    let entries = archive.entries().map_err(|e| format!("{}: {e}", tar_gz.display()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("{}: {e}", tar_gz.display()))?;
        let path = entry.path().map_err(|e| format!("{}: {e}", tar_gz.display()))?.into_owned();
        if !normal_relative(&path) {
            return Err(format!("{}: `{}` leaves the package", tar_gz.display(), path.display()));
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let link = entry
                .link_name()
                .map_err(|e| format!("{}: {e}", tar_gz.display()))?
                .ok_or_else(|| format!("{}: `{}` has no link target", tar_gz.display(), path.display()))?
                .into_owned();
            let inside = if kind.is_symlink() { resolved_inside(&path, &link).is_some() } else { normal_relative(&link) };
            if !inside {
                return Err(format!("{}: `{}` links outside the package ({})", tar_gz.display(), path.display(), link.display()));
            }
        } else if !kind.is_file() && !kind.is_dir() {
            return Err(format!("{}: `{}` is a special file", tar_gz.display(), path.display()));
        }
        let mode = entry.header().mode().unwrap_or(0o644);
        let unpacked = entry.unpack_in(dest).map_err(|e| format!("{}: unpack `{}`: {e}", tar_gz.display(), path.display()))?;
        if !unpacked {
            return Err(format!("{}: `{}` leaves the package", tar_gz.display(), path.display()));
        }
        // A worker runs as another account: everything stays readable, and only what was executable stays so.
        if kind.is_dir() {
            set_mode(&dest.join(&path), 0o755);
        } else if kind.is_file() {
            set_mode(&dest.join(&path), if mode & 0o111 != 0 { 0o755 } else { 0o644 });
        }
    }
    Ok(())
}

fn tail(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let text = text.trim();
    let start = text.char_indices().rev().nth(800).map(|(i, _)| i).unwrap_or(0);
    text[start..].to_string()
}

/// Run to completion or until `timeout`, with stdout and stderr in one captured file. Not next to the tool: a worker's
/// tools root is read-only.
fn run_bounded(mut command: Command, timeout: Duration) -> Result<(bool, Vec<u8>), String> {
    let capture_path = std::env::temp_dir().join(format!(".ext-tools-capture-{}-{}", std::process::id(), now_ms()));
    let capture = fs::File::create(&capture_path).map_err(|e| format!("{}: {e}", capture_path.display()))?;
    let stderr = capture.try_clone().map_err(|e| e.to_string())?;
    command.stdin(Stdio::null()).stdout(Stdio::from(capture)).stderr(Stdio::from(stderr));
    let mut child = command.spawn().map_err(|e| format!("spawn: {e}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break Some(status),
            None if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let output = fs::read(&capture_path).unwrap_or_default();
    let _ = fs::remove_file(&capture_path);
    match status {
        Some(status) => Ok((status.success(), output)),
        None => Ok((false, format!("no result in {}s; {}", timeout.as_secs(), tail(&output)).into_bytes())),
    }
}

fn with_bin_on_path(command: &mut Command, dir: &Path, also: Option<&Path>) {
    let mut paths = vec![dir.join("bin")];
    paths.extend(also.map(Path::to_path_buf));
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        command.env("PATH", joined);
    }
}

fn shell_command(windows: bool, line: &str) -> Command {
    let (shell, flag) = if windows { ("cmd", "/c") } else { ("sh", "-c") };
    let mut command = Command::new(shell);
    command.arg(flag).arg(line);
    command
}

/// Run a payload's smoke scripts with its own `bin/` first on the PATH. The launchers know where they are, so this works
/// in staging, in a version directory and after a move.
pub fn ext_tools_run_smoke(manifest: &ExtToolsManifest, dir: &Path, windows: bool, also_on_path: Option<&Path>) -> ExtToolsResult<()> {
    let mut scripts: Vec<&str> = manifest.provides.iter().filter_map(|p| p.smoke.as_deref()).collect();
    scripts.sort();
    scripts.dedup();
    for script in scripts {
        let path = ext_tools_smoke_path(dir, script, windows);
        let mut command = if windows {
            let mut command = Command::new("powershell");
            command.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(&path);
            command
        } else {
            let mut command = Command::new("sh");
            command.arg(&path);
            command
        };
        with_bin_on_path(&mut command, dir, also_on_path);
        let (ok, output) = run_bounded(command, SMOKE_TIMEOUT).map_err(|e| format!("smoke {script}: {e}"))?;
        if !ok {
            return Err(format!("smoke {script} failed: {}", tail(&output)));
        }
    }
    Ok(())
}

impl ToolsRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    fn install_path(&self, id: &str) -> PathBuf {
        self.root.join("installs").join(format!("{id}.json"))
    }

    fn version_dir(&self, id: &str, dir: &str) -> PathBuf {
        self.root.join("versions").join(id).join(dir)
    }

    fn sidecar_path(&self, id: &str, dir: &str) -> PathBuf {
        self.root.join("versions").join(id).join(format!("{dir}.json"))
    }

    fn pointer(&self, id: &str) -> PathBuf {
        self.root.join("store").join(id)
    }

    /// Make the layout. This is where a root the node cannot write to turns into a precise error, and where nothing else
    /// happens: no sudo, no other location.
    pub fn prepare(&self) -> ExtToolsResult<()> {
        for sub in ["bin", "store", "versions", "installs", "staging"] {
            let path = self.root.join(sub);
            fs::create_dir_all(&path).map_err(|e| format!("cannot create {} ({e}); give the BuckyOS account write access to {}", path.display(), self.root.display()))?;
            set_mode(&path, 0o755);
        }
        set_mode(&self.root, 0o755);
        let probe = self.root.join(format!(".write-check-{}", std::process::id()));
        fs::write(&probe, b"").map_err(|e| format!("{} is not writable ({e}); give the BuckyOS account write access to it", self.root.display()))?;
        let _ = fs::remove_file(probe);
        Ok(())
    }

    /// One writer at a time; a second one fails at once instead of waiting.
    pub fn lock(&self) -> ExtToolsResult<ToolsRootLock> {
        self.prepare()?;
        let path = self.root.join(LOCK_FILE);
        let file = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        file.try_lock().map_err(|e| format!("{} is being changed by another reconciler: {e}", self.root.display()))?;
        Ok(ToolsRootLock { _file: file })
    }

    pub fn installed(&self) -> BTreeMap<String, ExtToolsInstall> {
        let mut out = BTreeMap::new();
        let Ok(entries) = fs::read_dir(self.root.join("installs")) else {
            return out;
        };
        for entry in entries.flatten() {
            let Ok(raw) = fs::read_to_string(entry.path()) else { continue };
            if let Ok(install) = serde_json::from_str::<ExtToolsInstall>(&raw) {
                out.insert(install.id.clone(), install);
            }
        }
        out
    }

    pub fn install_record(&self, id: &str) -> Option<ExtToolsInstall> {
        let raw = fs::read_to_string(self.install_path(id)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn write_json<T: Serialize>(path: &Path, value: &T) -> ExtToolsResult<()> {
        let raw = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, raw).map_err(|e| format!("{}: {e}", temp.display()))?;
        set_mode(&temp, 0o644);
        fs::rename(&temp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn write_current(&self) -> ExtToolsResult<()> {
        let packages: BTreeMap<String, serde_json::Value> = self
            .installed()
            .into_iter()
            .map(|(id, install)| (id, serde_json::json!({"version": install.version, "digest": install.payload_digest})))
            .collect();
        Self::write_json(&self.root.join("current.json"), &serde_json::json!({"packages": packages}))
    }

    /// A problem with an active package that a reinstall of its version directory would fix.
    pub fn health(&self, id: &str) -> ExtToolsResult<()> {
        let install = self.install_record(id).ok_or_else(|| format!("{id} is not installed"))?;
        if !self.pointer(id).is_dir() {
            return Err(format!("store/{id} does not lead to version {}", install.version));
        }
        let missing: Vec<&String> = install.launchers.iter().filter(|name| !self.bin_dir().join(name).is_file()).collect();
        if !missing.is_empty() {
            return Err(format!("{id} is missing launchers: {missing:?}"));
        }
        Ok(())
    }

    fn switch_pointer(&self, id: &str, dir: &str) -> ExtToolsResult<()> {
        let link = self.pointer(id);
        #[cfg(unix)]
        {
            let temp = self.root.join("store").join(format!(".{id}.new"));
            let _ = fs::remove_file(&temp);
            let target = PathBuf::from("..").join("versions").join(id).join(dir).join("store").join(id);
            std::os::unix::fs::symlink(&target, &temp).map_err(|e| format!("{}: {e}", temp.display()))?;
            if let Ok(meta) = fs::symlink_metadata(&link) {
                if !meta.file_type().is_symlink() {
                    let _ = fs::remove_file(&temp);
                    return Err(format!("{} is a real directory that no package owns; remove it", link.display()));
                }
            }
            fs::rename(&temp, &link).map_err(|e| format!("{}: {e}", link.display()))
        }
        #[cfg(windows)]
        {
            if fs::symlink_metadata(&link).is_ok() {
                fs::remove_dir(&link).map_err(|e| format!("{}: {e}", link.display()))?;
            }
            let target = self.version_dir(id, dir).join("store").join(id);
            let status = Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(|e| format!("mklink: {e}"))?;
            if !status.status.success() {
                return Err(format!("mklink /J {}: {}", link.display(), tail(&status.stderr)));
            }
            Ok(())
        }
    }

    fn remove_pointer(&self, id: &str) -> ExtToolsResult<()> {
        let link = self.pointer(id);
        if fs::symlink_metadata(&link).is_err() {
            return Ok(());
        }
        #[cfg(unix)]
        return fs::remove_file(&link).map_err(|e| format!("{}: {e}", link.display()));
        #[cfg(windows)]
        return fs::remove_dir(&link).map_err(|e| format!("{}: {e}", link.display()));
    }

    /// Launcher files of a version directory.
    fn launchers_of(&self, id: &str, dir: &str) -> ExtToolsResult<Vec<String>> {
        let bin = self.version_dir(id, dir).join("bin");
        let mut names: Vec<String> = fs::read_dir(&bin)
            .map_err(|e| format!("{}: {e}", bin.display()))?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        Ok(names)
    }

    /// Put a version's launchers in `bin/`, each with one atomic rename, and take away what the old version had and this
    /// one has not.
    fn sync_launchers(&self, id: &str, dir: &str, old: &[String]) -> ExtToolsResult<Vec<String>> {
        let source = self.version_dir(id, dir).join("bin");
        let bin = self.bin_dir();
        let names = self.launchers_of(id, dir)?;
        for name in &names {
            let temp = bin.join(format!(".{name}.new"));
            fs::copy(source.join(name), &temp).map_err(|e| format!("copy launcher {name}: {e}"))?;
            set_mode(&temp, 0o755);
            fs::rename(&temp, bin.join(name)).map_err(|e| {
                let _ = fs::remove_file(&temp);
                format!("replace launcher {name} (in use?): {e}")
            })?;
        }
        for name in old.iter().filter(|name| !names.contains(name)) {
            let _ = fs::remove_file(bin.join(name));
        }
        Ok(names)
    }

    /// What each command says about itself once active.
    fn probe_versions(&self, manifest: &ExtToolsManifest, windows: bool) -> BTreeMap<String, String> {
        manifest
            .provides
            .iter()
            .map(|provided| {
                let mut command = shell_command(windows, &provided.version_probe);
                with_bin_on_path(&mut command, &self.root, None);
                let version = run_bounded(command, PROBE_TIMEOUT)
                    .ok()
                    .filter(|(ok, _)| *ok)
                    .map(|(_, output)| String::from_utf8_lossy(&output).lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("WARNING")).unwrap_or("").to_string())
                    .unwrap_or_default();
                (provided.command.clone(), version)
            })
            .collect()
    }

    fn read_version_manifest(&self, id: &str, dir: &str) -> ExtToolsResult<ExtToolsManifest> {
        let path = self.version_dir(id, dir).join(EXT_TOOLS_MANIFEST_FILE);
        let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        ExtToolsManifest::parse(&raw)
    }

    fn read_sidecar(&self, id: &str, dir: &str) -> ExtToolsResult<VersionSidecar> {
        let path = self.sidecar_path(id, dir);
        let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Finish an activation that was cut short. It has either not moved anything, or moved parts that the same steps
    /// move again, so finishing means doing it once more; only a target that vanished means going back.
    pub fn recover(&self) -> ExtToolsResult<()> {
        let path = self.root.join(JOURNAL_FILE);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let journal: Journal = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        let target = if self.version_dir(&journal.id, &journal.to_dir).is_dir() { Some(journal.to_dir.clone()) } else { journal.from_dir.clone() };
        let finished = match target {
            Some(dir) => {
                let old = self.install_record(&journal.id).map(|i| i.launchers).unwrap_or_default();
                let origin = self.install_record(&journal.id).map(|i| i.origin).unwrap_or(ExtToolsOrigin::Desired);
                self.activate_dir(&journal.id, &dir, journal.from_dir.as_deref(), &old, origin).map(|_| ())
            }
            None => Ok(()),
        };
        finished?;
        fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Point everything at a version directory and record it.
    fn activate_dir(&self, id: &str, dir: &str, previous_dir: Option<&str>, old_launchers: &[String], origin: ExtToolsOrigin) -> ExtToolsResult<ExtToolsInstall> {
        let manifest = self.read_version_manifest(id, dir)?;
        let sidecar = self.read_sidecar(id, dir)?;
        let windows = sidecar.platform.starts_with("windows");
        self.switch_pointer(id, dir)?;
        let launchers = self.sync_launchers(id, dir, old_launchers)?;
        let previous = previous_dir.filter(|p| *p != dir).and_then(|p| self.read_sidecar(id, p).ok().map(|s| ExtToolsPreviousVersion { version: s.version, dir: p.to_string() }));
        let install = ExtToolsInstall {
            schema: EXT_TOOLS_INSTALL_SCHEMA.to_string(),
            id: id.to_string(),
            version: sidecar.version.clone(),
            payload_digest: sidecar.payload_digest.clone(),
            pikg_digest: sidecar.pikg_digest.clone(),
            platform: sidecar.platform.clone(),
            origin,
            dir: dir.to_string(),
            commands: self.probe_versions(&manifest, windows),
            launchers,
            smoke: ExtToolsSmokeResult::Passed,
            activated_at_ms: now_ms(),
            previous,
        };
        Self::write_json(&self.install_path(id), &install)?;
        self.write_current()?;
        Ok(install)
    }

    /// Move to a version directory, undoing whatever was done if it does not work out.
    fn activate(&self, id: &str, dir: &str, origin: ExtToolsOrigin) -> ExtToolsResult<ExtToolsInstall> {
        let current = self.install_record(id);
        let from_dir = current.as_ref().map(|i| i.dir.clone());
        let old_launchers = current.as_ref().map(|i| i.launchers.clone()).unwrap_or_default();
        let journal_path = self.root.join(JOURNAL_FILE);
        Self::write_json(&journal_path, &Journal { id: id.to_string(), to_dir: dir.to_string(), from_dir: from_dir.clone() })?;
        match self.activate_dir(id, dir, from_dir.as_deref(), &old_launchers, origin) {
            Ok(install) => {
                fs::remove_file(&journal_path).map_err(|e| format!("{}: {e}", journal_path.display()))?;
                self.collect_versions(id);
                Ok(install)
            }
            Err(error) => {
                let undone = match from_dir.as_deref() {
                    Some(from) => self.switch_pointer(id, from).and_then(|_| self.sync_launchers(id, from, &old_launchers).map(|_| ())),
                    None => self.remove_launchers_of(id, dir).and_then(|_| self.remove_pointer(id)),
                };
                let _ = fs::remove_file(&journal_path);
                match undone {
                    Ok(()) => Err(error),
                    Err(undo) => Err(format!("{error}; going back failed too: {undo}")),
                }
            }
        }
    }

    fn remove_launchers_of(&self, id: &str, dir: &str) -> ExtToolsResult<()> {
        for name in self.launchers_of(id, dir)? {
            let _ = fs::remove_file(self.bin_dir().join(name));
        }
        Ok(())
    }

    /// Only the active version and the one before it stay. What a running tool holds open on Windows stays until a later
    /// activation can delete it.
    fn collect_versions(&self, id: &str) {
        let Some(install) = self.install_record(id) else { return };
        let keep: Vec<String> = std::iter::once(install.dir.clone()).chain(install.previous.map(|p| p.dir)).collect();
        let Ok(entries) = fs::read_dir(self.root.join("versions").join(id)) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let dir = name.strip_suffix(".json").unwrap_or(&name);
            if keep.iter().any(|k| k == dir) {
                continue;
            }
            let _ = if entry.path().is_dir() { fs::remove_dir_all(entry.path()) } else { fs::remove_file(entry.path()) };
        }
    }

    /// The version directory holding this `.pikg`'s payload, kept from before: the way back needs no download.
    pub fn find_version(&self, id: &str, pikg_digest: &str) -> Option<String> {
        let entries = fs::read_dir(self.root.join("versions").join(id)).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(dir) = name.strip_suffix(".json") else { continue };
            if self.version_dir(id, dir).is_dir() && self.read_sidecar(id, dir).is_ok_and(|s| s.pikg_digest.as_deref() == Some(pikg_digest)) {
                return Some(dir.to_string());
            }
        }
        None
    }

    /// Make a version directory that is already here active again.
    pub fn activate_existing(&self, id: &str, dir: &str, origin: ExtToolsOrigin) -> ExtToolsResult<ExtToolsInstall> {
        let _lock = self.lock()?;
        self.recover()?;
        self.check_ownership(id, dir)?;
        self.activate(id, dir, origin)
    }

    /// Launchers another package owns are not ours to replace.
    fn check_ownership(&self, id: &str, dir: &str) -> ExtToolsResult<()> {
        let names = self.launchers_of(id, dir)?;
        for (other, install) in self.installed() {
            if other == id {
                continue;
            }
            let shared: Vec<&String> = names.iter().filter(|name| install.launchers.contains(name)).collect();
            if !shared.is_empty() {
                return Err(format!("{other} already provides {shared:?}"));
            }
        }
        Ok(())
    }

    /// Check a payload and make it active. `on_stage` hears each step; a failure at any of them leaves what was active in
    /// place.
    pub fn install(&self, apply: &ExtToolsApply<'_>, on_stage: &mut dyn FnMut(ExtToolsStage)) -> ExtToolsResult<ExtToolsInstall> {
        let _lock = self.lock()?;
        self.recover()?;
        on_stage(ExtToolsStage::Verifying);
        let digest = ext_tools_sha256_file(apply.tar)?;
        if let Some(expected) = apply.expected_digest {
            if expected != digest {
                return Err(format!("{}: digest is {digest}, expected {expected}", apply.tar.display()));
            }
        }
        let short: String = digest.trim_start_matches("sha256:").chars().take(12).collect();
        on_stage(ExtToolsStage::Staging);
        let staging = self.root.join("staging").join(format!("{}-{short}-{}", apply.id, now_ms()));
        let result = self.install_staged(apply, &digest, &short, &staging, on_stage);
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn install_staged(&self, apply: &ExtToolsApply<'_>, digest: &str, short: &str, staging: &Path, on_stage: &mut dyn FnMut(ExtToolsStage)) -> ExtToolsResult<ExtToolsInstall> {
        ext_tools_unpack(apply.tar, staging)?;
        let manifest_path = staging.join(EXT_TOOLS_MANIFEST_FILE);
        let raw = fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        let manifest = ExtToolsManifest::parse(&raw)?;
        if manifest.id != apply.id {
            return Err(format!("the payload is {}, expected {}", manifest.id, apply.id));
        }
        if !manifest.platforms.iter().any(|p| p == apply.platform.name) {
            return Err(format!("{} has no payload for {}", manifest.id, apply.platform.name));
        }
        let reserved = ext_tools_reserved_provides(&manifest);
        if !reserved.is_empty() {
            return Err(format!("{} provides standard commands: {}", manifest.id, reserved.join(", ")));
        }
        let errors = ext_tools_payload_errors(&manifest, apply.platform, staging);
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        let dir = format!("{}-{short}", manifest.version);
        let active = self.install_record(apply.id);
        if active.as_ref().is_some_and(|i| i.dir == dir) && self.health(apply.id).is_ok() {
            return active.ok_or_else(|| format!("{} vanished", apply.id));
        }
        on_stage(ExtToolsStage::Checking);
        let mut smoke = ExtToolsSmokeResult::Skipped;
        if !apply.skip_smoke {
            ext_tools_run_smoke(&manifest, staging, apply.platform.os == "windows", Some(&self.bin_dir()))?;
            smoke = ExtToolsSmokeResult::Passed;
        }
        let version_dir = self.version_dir(apply.id, &dir);
        fs::create_dir_all(version_dir.parent().unwrap_or(&self.root)).map_err(|e| e.to_string())?;
        if version_dir.exists() {
            fs::remove_dir_all(&version_dir).map_err(|e| format!("{}: {e}", version_dir.display()))?;
        }
        fs::rename(staging, &version_dir).map_err(|e| format!("{}: {e}", version_dir.display()))?;
        Self::write_json(
            &self.sidecar_path(apply.id, &dir),
            &VersionSidecar { version: manifest.version.clone(), payload_digest: digest.to_string(), pikg_digest: apply.pikg_digest.map(str::to_string), platform: apply.platform.name.to_string() },
        )?;
        self.check_ownership(apply.id, &dir)?;
        let mut install = self.activate(apply.id, &dir, apply.origin)?;
        install.smoke = smoke;
        Self::write_json(&self.install_path(apply.id), &install)?;
        Ok(install)
    }

    /// Run the smoke of what is active, for `ext-tools check --full`.
    pub fn check(&self, id: &str) -> ExtToolsResult<()> {
        self.health(id)?;
        let install = self.install_record(id).ok_or_else(|| format!("{id} is not installed"))?;
        let manifest = self.read_version_manifest(id, &install.dir)?;
        ext_tools_run_smoke(&manifest, &self.version_dir(id, &install.dir), install.platform.starts_with("windows"), Some(&self.bin_dir()))
    }

    /// Refresh the record's origin without touching the files: a seeded package the desired state names is the desired
    /// state's from then on.
    pub fn adopt(&self, id: &str, pikg_digest: Option<&str>) -> ExtToolsResult<()> {
        let _lock = self.lock()?;
        let mut install = self.install_record(id).ok_or_else(|| format!("{id} is not installed"))?;
        install.origin = ExtToolsOrigin::Desired;
        if let Some(digest) = pikg_digest {
            install.pikg_digest = Some(digest.to_string());
            let mut sidecar = self.read_sidecar(id, &install.dir)?;
            sidecar.pikg_digest = Some(digest.to_string());
            Self::write_json(&self.sidecar_path(id, &install.dir), &sidecar)?;
        }
        Self::write_json(&self.install_path(id), &install)
    }

    /// Take a package out. A version directory a running tool still holds stays until a later call removes it.
    pub fn remove(&self, id: &str) -> ExtToolsResult<()> {
        let _lock = self.lock()?;
        self.recover()?;
        let Some(install) = self.install_record(id) else {
            return Ok(());
        };
        for (other, record) in self.installed() {
            if other == id {
                continue;
            }
            let manifest = self.read_version_manifest(&other, &record.dir)?;
            if manifest.requires.iter().any(|r| r.id == id) {
                return Err(format!("{other} requires {id}"));
            }
        }
        for name in &install.launchers {
            let _ = fs::remove_file(self.bin_dir().join(name));
        }
        self.remove_pointer(id)?;
        fs::remove_file(self.install_path(id)).map_err(|e| format!("{}: {e}", self.install_path(id).display()))?;
        self.write_current()?;
        let _ = fs::remove_dir_all(self.root.join("versions").join(id));
        Ok(())
    }

    /// Unpack what a release shipped for packages this root has never had. Once a package has a record it belongs to the
    /// desired state, so a newer installer never overwrites a node's version.
    pub fn seed(&self, pin_dir: &Path, platform: &ExtToolsPlatform, skip_smoke: bool) -> ExtToolsResult<Vec<ExtToolsInstall>> {
        let Some(pin) = ExtToolsPinFile::load(pin_dir)? else {
            return Ok(Vec::new());
        };
        let mut seeded = Vec::new();
        for (id, package) in &pin.packages {
            if self.install_record(id).is_some() {
                continue;
            }
            let Some(payload) = package.platforms.get(platform.name) else { continue };
            let tar = pin_dir.join(&payload.file);
            if !tar.is_file() {
                continue;
            }
            let install = self.install(
                &ExtToolsApply {
                    tar: &tar,
                    id,
                    expected_digest: Some(&payload.sha256),
                    pikg_digest: package.pikg_digest.as_deref(),
                    platform,
                    origin: ExtToolsOrigin::Seed,
                    skip_smoke,
                },
                &mut |_| {},
            )?;
            seeded.push(install);
        }
        // A seed image's payload lies inside the root it fills, and is of no more use once the root has that package.
        if pin_dir.starts_with(&self.root) {
            let _ = fs::remove_dir_all(pin_dir);
        }
        Ok(seeded)
    }
}

/// `opendan ext-tools <command>`: the engine for a tools root only a container can reach, namely the node's tools volume on
/// a machine whose Docker runs in a VM. Prints one JSON line.
pub fn ext_tools_cli_main(args: &[String]) -> i32 {
    fn value(args: &[String], name: &str) -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    }
    fn required(args: &[String], name: &str) -> ExtToolsResult<String> {
        value(args, name).ok_or_else(|| format!("{name} is required"))
    }
    let run = || -> ExtToolsResult<serde_json::Value> {
        let command = args.first().map(String::as_str).unwrap_or("");
        let root = ToolsRoot::new(required(args, "--root")?);
        let origin = match value(args, "--origin").as_deref() {
            Some("seed") => ExtToolsOrigin::Seed,
            _ => ExtToolsOrigin::Desired,
        };
        let skip_smoke = args.iter().any(|a| a == "--skip-smoke");
        let platform = ext_tools_current_platform().ok_or("this platform has no extension packages")?;
        fn to_json<T: Serialize>(value: &T) -> serde_json::Value {
            serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
        }
        match command {
            "status" => Ok(to_json(&root.installed())),
            "apply" => {
                let tar = PathBuf::from(required(args, "--tar")?);
                let id = required(args, "--id")?;
                let digest = value(args, "--digest");
                let pikg_digest = value(args, "--pikg-digest");
                let install = root.install(
                    &ExtToolsApply { tar: &tar, id: &id, expected_digest: digest.as_deref(), pikg_digest: pikg_digest.as_deref(), platform, origin, skip_smoke },
                    &mut |_| {},
                )?;
                Ok(to_json(&install))
            }
            "activate" => {
                let id = required(args, "--id")?;
                let dir = root.find_version(&id, &required(args, "--pikg-digest")?).ok_or("that version is not here")?;
                Ok(to_json(&root.activate_existing(&id, &dir, origin)?))
            }
            "seed" => {
                let dir = PathBuf::from(required(args, "--dir")?);
                Ok(to_json(&root.seed(&dir, platform, skip_smoke)?))
            }
            "adopt" => {
                root.adopt(&required(args, "--id")?, value(args, "--pikg-digest").as_deref())?;
                Ok(serde_json::json!({}))
            }
            "remove" => {
                root.remove(&required(args, "--id")?)?;
                Ok(serde_json::json!({}))
            }
            "check" => {
                root.check(&required(args, "--id")?)?;
                Ok(serde_json::json!({}))
            }
            other => Err(format!("unknown command `{other}`")),
        }
    };
    match run() {
        Ok(result) => {
            println!("{}", serde_json::json!({"ok": true, "result": result}));
            0
        }
        Err(error) => {
            println!("{}", serde_json::json!({"ok": false, "error": error}));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ext-tools-install-{name}-{}-{}", std::process::id(), now_ms()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn linux() -> &'static ExtToolsPlatform {
        ext_tools_platform("linux-x86_64").unwrap()
    }

    fn manifest(id: &str, version: &str, commands: &[&str], requires: &[&str]) -> ExtToolsManifest {
        ExtToolsManifest {
            schema: EXT_TOOLS_MANIFEST_SCHEMA.to_string(),
            id: id.to_string(),
            version: version.to_string(),
            requires: requires.iter().map(|r| ExtToolsRequire { id: r.to_string(), version: "*".to_string() }).collect(),
            conflicts: vec![],
            provides: commands.iter().map(|c| ExtToolsCommand { command: c.to_string(), version_probe: format!("{c} --version"), smoke: Some("smoke/all.sh".to_string()) }).collect(),
            platforms: vec!["linux-x86_64".to_string()],
            size_bytes: BTreeMap::new(),
            license: String::new(),
            desc: String::new(),
        }
    }

    /// A payload whose commands print the version, and find it through `store/<id>` from wherever they are.
    fn payload(dir: &Path, m: &ExtToolsManifest, smoke: &str) -> PathBuf {
        let tree = dir.join(format!("tree-{}-{}", m.id, m.version));
        let _ = fs::remove_dir_all(&tree);
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::create_dir_all(tree.join("smoke")).unwrap();
        fs::create_dir_all(tree.join("store").join(&m.id)).unwrap();
        fs::write(tree.join(EXT_TOOLS_MANIFEST_FILE), serde_json::to_string(m).unwrap()).unwrap();
        fs::write(tree.join("smoke/all.sh"), smoke).unwrap();
        fs::write(tree.join("store").join(&m.id).join("VERSION"), &m.version).unwrap();
        for provided in &m.provides {
            let launcher = tree.join("bin").join(&provided.command);
            fs::write(&launcher, format!("#!/bin/sh\nhere=\"$(cd \"${{0%/*}}\" && pwd -P)\"\necho {} $(cat \"$here/../store/{}/VERSION\")\n", provided.command, m.id)).unwrap();
            #[cfg(unix)]
            fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tar_path = dir.join(format!("{}-{}.tar.gz", m.id, m.version));
        let file = fs::File::create(&tar_path).unwrap();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(file, flate2::Compression::fast()));
        builder.append_dir_all(".", &tree).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        tar_path
    }

    fn apply<'a>(tar: &'a Path, id: &'a str) -> ExtToolsApply<'a> {
        ExtToolsApply { tar, id, expected_digest: None, pikg_digest: None, platform: linux(), origin: ExtToolsOrigin::Desired, skip_smoke: false }
    }

    fn run_command(root: &Path, command: &str) -> String {
        let output = Command::new(root.join("bin").join(command)).output().unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[cfg(unix)]
    #[test]
    fn a_payload_is_checked_activated_and_replaced_with_the_old_one_kept() {
        let scratch = Scratch::new("activate");
        let root = ToolsRoot::new(scratch.0.join("tools"));
        let one = payload(&scratch.0, &manifest("ext-a", "1.0.0", &["cmd-a", "cmd-gone"], &[]), "#!/bin/sh\ncmd-a | grep -q 1.0.0\n");
        let mut stages = Vec::new();
        let install = root.install(&apply(&one, "ext-a"), &mut |s| stages.push(s)).unwrap();
        assert_eq!(stages, vec![ExtToolsStage::Verifying, ExtToolsStage::Staging, ExtToolsStage::Checking]);
        assert_eq!(install.version, "1.0.0");
        assert_eq!(install.smoke, ExtToolsSmokeResult::Passed);
        assert_eq!(install.commands["cmd-a"], "cmd-a 1.0.0");
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");
        assert_eq!(root.install_record("ext-a").unwrap().smoke, ExtToolsSmokeResult::Passed);
        let current: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.path().join("current.json")).unwrap()).unwrap();
        assert_eq!(current["packages"]["ext-a"]["version"], "1.0.0");

        let two = payload(&scratch.0, &manifest("ext-a", "2.0.0", &["cmd-a"], &[]), "#!/bin/sh\ncmd-a | grep -q 2.0.0\n");
        let install = root.install(&apply(&two, "ext-a"), &mut |_| {}).unwrap();
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 2.0.0");
        assert!(!root.bin_dir().join("cmd-gone").exists(), "what the new version does not provide leaves");
        assert_eq!(install.previous.as_ref().unwrap().version, "1.0.0");
        assert!(root.path().join("versions/ext-a").join(&install.previous.as_ref().unwrap().dir).is_dir());

        let back = install.previous.unwrap().dir;
        let install = root.activate_existing("ext-a", &back, ExtToolsOrigin::Desired).unwrap();
        assert_eq!(install.version, "1.0.0");
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");
        assert!(root.bin_dir().join("cmd-gone").exists(), "the launcher comes back with its version");
        root.health("ext-a").unwrap();

        root.remove("ext-a").unwrap();
        assert!(!root.bin_dir().join("cmd-a").exists());
        assert!(fs::symlink_metadata(root.path().join("store/ext-a")).is_err());
        assert!(root.install_record("ext-a").is_none());
        assert!(!root.path().join("versions/ext-a").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_at_any_check_leaves_the_active_version_running() {
        let scratch = Scratch::new("failures");
        let root = ToolsRoot::new(scratch.0.join("tools"));
        let good = payload(&scratch.0, &manifest("ext-a", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        root.install(&apply(&good, "ext-a"), &mut |_| {}).unwrap();
        let still_good = || {
            assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");
            assert_eq!(root.install_record("ext-a").unwrap().version, "1.0.0");
            assert_eq!(fs::read_dir(root.path().join("staging")).unwrap().count(), 0, "staging is left clean");
            assert!(!root.path().join(JOURNAL_FILE).exists());
        };

        let wrong_digest = root.install(&ExtToolsApply { expected_digest: Some("sha256:00"), ..apply(&good, "ext-a") }, &mut |_| {}).unwrap_err();
        assert!(wrong_digest.contains("digest"), "{wrong_digest}");
        still_good();

        let failing_smoke = payload(&scratch.0, &manifest("ext-a", "2.0.0", &["cmd-a"], &[]), "#!/bin/sh\necho no ffmpeg here >&2\nexit 3\n");
        let error = root.install(&apply(&failing_smoke, "ext-a"), &mut |_| {}).unwrap_err();
        assert!(error.contains("no ffmpeg here"), "{error}");
        still_good();

        let wrong_id = payload(&scratch.0, &manifest("ext-b", "2.0.0", &["cmd-b"], &[]), "#!/bin/sh\n");
        assert!(root.install(&apply(&wrong_id, "ext-a"), &mut |_| {}).is_err());
        still_good();

        let mut other_platform = manifest("ext-a", "2.0.1", &["cmd-a"], &[]);
        other_platform.platforms = vec!["darwin-arm64".to_string()];
        let other_platform = payload(&scratch.0, &other_platform, "#!/bin/sh\n");
        let error = root.install(&apply(&other_platform, "ext-a"), &mut |_| {}).unwrap_err();
        assert!(error.contains("linux-x86_64"), "{error}");
        still_good();

        let shadowing = payload(&scratch.0, &manifest("ext-a", "2.0.2", &["xagent"], &[]), "#!/bin/sh\n");
        let error = root.install(&apply(&shadowing, "ext-a"), &mut |_| {}).unwrap_err();
        assert!(error.contains("standard commands"), "{error}");
        still_good();

        let truncated = scratch.0.join("truncated.tar.gz");
        fs::write(&truncated, &fs::read(&good).unwrap()[..64]).unwrap();
        assert!(root.install(&apply(&truncated, "ext-a"), &mut |_| {}).is_err());
        still_good();
    }

    fn evil_tar(path: &Path, write: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<fs::File>>)) {
        let file = fs::File::create(path).unwrap();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(file, flate2::Compression::fast()));
        write(&mut builder);
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn file_header(kind: tar::EntryType, size: u64) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(size);
        header.set_mode(0o755);
        header
    }

    #[cfg(unix)]
    #[test]
    fn a_payload_cannot_reach_outside_its_directory() {
        let scratch = Scratch::new("escape");
        let outside = scratch.0.join("outside");
        fs::create_dir_all(&outside).unwrap();

        let dot_dot = scratch.0.join("dotdot.tar.gz");
        evil_tar(&dot_dot, |b| {
            let mut header = file_header(tar::EntryType::Regular, 4);
            header.as_old_mut().name[..15].copy_from_slice(b"../outside/evil");
            header.set_cksum();
            b.append(&header, &b"evil"[..]).unwrap();
        });
        let error = ext_tools_unpack(&dot_dot, &scratch.0.join("d1")).unwrap_err();
        assert!(error.contains("leaves the package"), "{error}");
        assert!(!outside.join("evil").exists());

        let absolute_link = scratch.0.join("abslink.tar.gz");
        evil_tar(&absolute_link, |b| {
            let mut header = file_header(tar::EntryType::Symlink, 0);
            b.append_link(&mut header, "store/x", outside.join("evil")).unwrap();
        });
        let error = ext_tools_unpack(&absolute_link, &scratch.0.join("d2")).unwrap_err();
        assert!(error.contains("links outside"), "{error}");

        let relative_link = scratch.0.join("rellink.tar.gz");
        evil_tar(&relative_link, |b| {
            let mut header = file_header(tar::EntryType::Symlink, 0);
            b.append_link(&mut header, "store/x", "../../outside").unwrap();
        });
        let error = ext_tools_unpack(&relative_link, &scratch.0.join("d3")).unwrap_err();
        assert!(error.contains("links outside"), "{error}");

        let inside_link = scratch.0.join("inside.tar.gz");
        evil_tar(&inside_link, |b| {
            let mut header = file_header(tar::EntryType::Symlink, 0);
            b.append_link(&mut header, "store/a/x", "../b").unwrap();
        });
        ext_tools_unpack(&inside_link, &scratch.0.join("d4")).unwrap();
        assert!(fs::symlink_metadata(scratch.0.join("d4/store/a/x")).unwrap().file_type().is_symlink());

        let device = scratch.0.join("device.tar.gz");
        evil_tar(&device, |b| {
            let mut header = file_header(tar::EntryType::Char, 0);
            b.append_data(&mut header, "store/null", &b""[..]).unwrap();
        });
        let error = ext_tools_unpack(&device, &scratch.0.join("d5")).unwrap_err();
        assert!(error.contains("special file"), "{error}");

        let setuid = scratch.0.join("setuid.tar.gz");
        evil_tar(&setuid, |b| {
            let mut header = file_header(tar::EntryType::Regular, 2);
            header.set_mode(0o4755);
            b.append_data(&mut header, "bin/x", &b"hi"[..]).unwrap();
        });
        ext_tools_unpack(&setuid, &scratch.0.join("d6")).unwrap();
        assert_eq!(fs::metadata(scratch.0.join("d6/bin/x")).unwrap().permissions().mode() & 0o7777, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn an_activation_cut_short_is_finished_by_the_next_writer() {
        let scratch = Scratch::new("journal");
        let root = ToolsRoot::new(scratch.0.join("tools"));
        let one = payload(&scratch.0, &manifest("ext-a", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let two = payload(&scratch.0, &manifest("ext-a", "2.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let first = root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap();
        let second = root.install(&apply(&two, "ext-a"), &mut |_| {}).unwrap();
        // The state after a crash right after the journal is written and the pointer moved, launchers still on the old one.
        root.switch_pointer("ext-a", &first.dir).unwrap();
        ToolsRoot::write_json(&root.path().join(JOURNAL_FILE), &Journal { id: "ext-a".into(), to_dir: second.dir.clone(), from_dir: Some(first.dir.clone()) }).unwrap();
        root.recover().unwrap();
        assert_eq!(root.install_record("ext-a").unwrap().dir, second.dir);
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 2.0.0");
        assert!(!root.path().join(JOURNAL_FILE).exists());

        // A target that vanished means going back to where it came from.
        ToolsRoot::write_json(&root.path().join(JOURNAL_FILE), &Journal { id: "ext-a".into(), to_dir: "9.9.9-gone".into(), from_dir: Some(first.dir.clone()) }).unwrap();
        root.recover().unwrap();
        assert_eq!(root.install_record("ext-a").unwrap().dir, first.dir);
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");
    }

    #[cfg(unix)]
    #[test]
    fn a_launcher_that_cannot_be_replaced_rolls_the_pointer_back() {
        let scratch = Scratch::new("rollback");
        let root = ToolsRoot::new(scratch.0.join("tools"));
        let one = payload(&scratch.0, &manifest("ext-a", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let first = root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap();
        let two = payload(&scratch.0, &manifest("ext-a", "2.0.0", &["cmd-a", "cmd-b"], &[]), "#!/bin/sh\n");
        // A directory where the new launcher goes makes the rename fail after the pointer already moved.
        fs::create_dir_all(root.bin_dir().join("cmd-b")).unwrap();
        fs::write(root.bin_dir().join("cmd-b").join("occupied"), "").unwrap();
        let error = root.install(&apply(&two, "ext-a"), &mut |_| {}).unwrap_err();
        assert!(error.contains("cmd-b"), "{error}");
        assert_eq!(root.install_record("ext-a").unwrap().dir, first.dir);
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");
        assert!(!root.path().join(JOURNAL_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn writers_exclude_each_other_and_packages_do_not_overwrite_each_other() {
        let scratch = Scratch::new("lock");
        let root = ToolsRoot::new(scratch.0.join("tools"));
        let held = root.lock().unwrap();
        let one = payload(&scratch.0, &manifest("ext-a", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let error = root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap_err();
        assert!(error.contains("another reconciler"), "{error}");
        assert!(root.install_record("ext-a").is_none());
        drop(held);
        root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap();

        let thief = payload(&scratch.0, &manifest("ext-b", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let error = root.install(&apply(&thief, "ext-b"), &mut |_| {}).unwrap_err();
        assert!(error.contains("ext-a already provides"), "{error}");
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");

        let dependent = payload(&scratch.0, &manifest("ext-c", "1.0.0", &["cmd-c"], &["ext-a"]), "#!/bin/sh\n");
        root.install(&apply(&dependent, "ext-c"), &mut |_| {}).unwrap();
        let error = root.remove("ext-a").unwrap_err();
        assert!(error.contains("ext-c requires"), "{error}");
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_root_is_reported_and_nothing_else_is_tried() {
        let scratch = Scratch::new("readonly");
        fs::write(scratch.0.join("file"), "").unwrap();
        let root = ToolsRoot::new(scratch.0.join("file").join("tools"));
        let one = payload(&scratch.0, &manifest("ext-a", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let error = root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap_err();
        assert!(error.contains("write access"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_seed_only_fills_what_a_root_has_never_had() {
        let scratch = Scratch::new("seed");
        let root = ToolsRoot::new(scratch.0.join("tools"));
        let pin_dir = scratch.0.join("pin");
        fs::create_dir_all(&pin_dir).unwrap();
        let tar = payload(&pin_dir, &manifest("ext-core", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let write_pin = |version: &str, tar: &Path| {
            let digest = ext_tools_sha256_file(tar).unwrap();
            let pin = ExtToolsPinFile {
                schema: EXT_TOOLS_PIN_SCHEMA.to_string(),
                packages: BTreeMap::from([(
                    "ext-core".to_string(),
                    ExtToolsPinPackage {
                        version: version.to_string(),
                        pikg_digest: Some("sha256:pikg".to_string()),
                        platforms: BTreeMap::from([("linux-x86_64".to_string(), ExtToolsPinPlatform { file: tar.file_name().unwrap().to_string_lossy().to_string(), sha256: digest, size: 1 })]),
                    },
                )]),
            };
            fs::write(pin_dir.join(EXT_TOOLS_PIN_FILE), serde_json::to_string(&pin).unwrap()).unwrap();
        };
        assert!(root.seed(&pin_dir, linux(), false).unwrap().is_empty(), "no pin, nothing to do");
        write_pin("1.0.0", &tar);
        let seeded = root.seed(&pin_dir, linux(), false).unwrap();
        assert_eq!(seeded[0].origin, ExtToolsOrigin::Seed);
        assert_eq!(seeded[0].pikg_digest.as_deref(), Some("sha256:pikg"));
        assert_eq!(root.find_version("ext-core", "sha256:pikg").unwrap(), seeded[0].dir);

        let newer = payload(&pin_dir, &manifest("ext-core", "2.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        write_pin("2.0.0", &newer);
        assert!(root.seed(&pin_dir, linux(), false).unwrap().is_empty(), "an installed node keeps its version");
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");

        // Removing the record re-seeds, which is how a developer retries a package of their own.
        root.remove("ext-core").unwrap();
        let seeded = root.seed(&pin_dir, linux(), false).unwrap();
        assert_eq!(seeded[0].version, "2.0.0");
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 2.0.0");
    }

    #[cfg(unix)]
    #[test]
    fn the_same_payload_again_changes_nothing_and_a_broken_tree_is_repaired() {
        let scratch = Scratch::new("idempotent");
        let root = ToolsRoot::new(scratch.0.join("tools"));
        let one = payload(&scratch.0, &manifest("ext-a", "1.0.0", &["cmd-a"], &[]), "#!/bin/sh\n");
        let first = root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap();
        let again = root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap();
        assert_eq!(first.activated_at_ms, again.activated_at_ms);
        fs::remove_file(root.bin_dir().join("cmd-a")).unwrap();
        assert!(root.health("ext-a").is_err());
        root.install(&apply(&one, "ext-a"), &mut |_| {}).unwrap();
        root.health("ext-a").unwrap();
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");

        // What a reconciler does when it finds a package damaged: back from its version directory, no download.
        fs::remove_file(root.bin_dir().join("cmd-a")).unwrap();
        fs::remove_file(root.path().join("store/ext-a")).unwrap();
        root.activate_existing("ext-a", &first.dir, ExtToolsOrigin::Desired).unwrap();
        root.health("ext-a").unwrap();
        assert_eq!(run_command(root.path(), "cmd-a"), "cmd-a 1.0.0");
    }
}
