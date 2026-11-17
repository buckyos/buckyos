//! The Chromium process of the agent's browser: where its executable and
//! directories are, how it is started (headed on the virtual display, or
//! headless; the debugging endpoint on a loopback port given to it) and how
//! it is ended without leaving processes behind.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const ENV_CHROMIUM: &str = "OPENDAN_CHROMIUM";
pub const ENV_BROWSER_DIR: &str = "OPENDAN_BROWSER_DIR";
/// Set for the browser to a value of its own: its processes are told from
/// any other by it, wherever they went.
pub const ENV_MARK: &str = "OPENDAN_BROWSER_MARK";
const PORT_FILE: &str = "DevToolsActivePort";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const NO_SANDBOX_ERROR: &str = "No usable sandbox";
/// What the browser logs when another process took its debugging port.
const PORT_TAKEN: &str = "Cannot start http server for devtools";
/// The headless browser's screens (sizes in device pixels): the one its
/// windows open on, and beside it one of twice its pixel density, the same
/// size in CSS pixels (2560×1440, larger than any window it is given). A tab
/// whose window is moved there paints frames of twice as many pixels, its page unchanged.
pub const SCREENS: &str = "{0,0 2560x1440}{2560,0 5120x2880 devicePixelRatio=2}";
/// Where a window is sent to be on the dense screen.
pub const DENSE_SCREEN_LEFT: u32 = 2560;
/// How much higher a headless window is than its page: what the browser
/// keeps of it for a toolbar it does not draw.
pub const TOOLBAR_HEIGHT: u32 = 87;
/// Chromium keeps a socket in `<TMPDIR>/.org.chromium.Chromium.<6>/SingletonSocket`
/// and does not start when that path is too long for a socket address
/// (about 104 bytes): this much is left for `TMPDIR`.
const MAX_TMPDIR: usize = 50;

/// What this node has of the browser.
#[derive(Debug, Clone, PartialEq)]
pub enum Chromium {
    /// The executable to start.
    Ready(PathBuf),
    /// The extension package's launcher is there, the browser it starts is
    /// not: why.
    NotDownloaded { launcher: PathBuf, why: String },
    /// No `chromium` at all: why.
    NoPackage(String),
}

impl Chromium {
    /// The node's `chromium`, whether or not it has a browser to start.
    pub fn launcher(&self) -> Option<&Path> {
        match self {
            Self::Ready(launcher) | Self::NotDownloaded { launcher, .. } => Some(launcher),
            Self::NoPackage(_) => None,
        }
    }
}

/// The browser's executable: `OPENDAN_CHROMIUM`, else `chromium` of the
/// node's tool directories.
pub fn find_chromium() -> Chromium {
    match std::env::var_os(ENV_CHROMIUM).filter(|p| !p.is_empty()) {
        Some(path) => named_chromium(Path::new(&path)),
        None => chromium_in(&libopendan::baseline::managed_tool_dirs()),
    }
}

/// An executable named outright (`OPENDAN_CHROMIUM`, a test's): started as
/// it is, whatever the node has downloaded.
pub fn named_chromium(path: &Path) -> Chromium {
    if path.is_file() {
        Chromium::Ready(path.to_path_buf())
    } else {
        Chromium::NoPackage(format!("no `chromium` at {} (`{ENV_CHROMIUM}`)", path.display()))
    }
}

/// `chromium` of the tool directories `dirs`: the extension package's
/// launcher, usable once the browser it starts is downloaded.
pub fn chromium_in(dirs: &[PathBuf]) -> Chromium {
    let launcher = std::env::join_paths(dirs)
        .ok()
        .filter(|_| !dirs.is_empty())
        .and_then(|dirs| libopendan::baseline::resolve_locally(&["chromium"], &[], Some(&dirs), &[]).remove("chromium"))
        .map(PathBuf::from);
    let Some(launcher) = launcher else {
        return Chromium::NoPackage("no `chromium` on this node (it comes with the ext-core extension tools)".to_string());
    };
    match not_downloaded(&launcher) {
        Some(why) => Chromium::NotDownloaded { launcher, why },
        None => Chromium::Ready(launcher),
    }
}

/// Why the browser `launcher` starts is not on this node (`None`: it is).
/// The browser is an online item of the extension package: not part of it,
/// downloaded by the node to `online/chromium` beside the launcher's
/// directory and recorded in `online/chromium.json` (no record: not
/// downloaded). Read at every call: a download that ends is seen at once.
fn not_downloaded(launcher: &Path) -> Option<String> {
    const SETTINGS: &str = "Settings › Extension tools, or with `buckyos ext-tools download ext-core chromium`";
    let online = launcher.parent()?.parent()?.join("online");
    let (dir, file) = (online.join("chromium"), online.join("chromium.json"));
    let unreadable = |why: &dyn std::fmt::Display| format!("the record of the browser's download ({}) is not readable: {why}", file.display());
    let record: Value = match std::fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(record) => record,
            Err(e) => return Some(unreadable(&e)),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Some(format!(
                "the browser is not downloaded on this node yet (the online item `chromium` of ext-core): download it in {SETTINGS}"
            ));
        }
        Err(e) => return Some(unreadable(&e)),
    };
    match record["state"].as_str() {
        Some("ready") if dir.is_dir() => None,
        Some("ready") => Some(format!(
            "the browser's download is recorded as ready, but {} is not there: download it again in {SETTINGS}",
            dir.display()
        )),
        Some("downloading") => {
            let progress = match (record["downloaded_bytes"].as_u64(), record["size_bytes"].as_u64().filter(|size| *size > 0)) {
                (Some(done), Some(size)) => format!(" ({}% of {} MiB)", done.min(size) * 100 / size, size.div_ceil(1024 * 1024)),
                _ => String::new(),
            };
            Some(format!("the browser is being downloaded to this node{progress}"))
        }
        Some("failed") => Some(format!(
            "the browser's download to this node failed: {}. Start it again in {SETTINGS}",
            record["error"].as_str().map(str::trim).filter(|e| !e.is_empty()).unwrap_or("no reason is recorded").trim_end_matches('.')
        )),
        state => Some(format!(
            "the record of the browser's download ({}) has a state this service does not know: {}",
            file.display(),
            state.unwrap_or("none")
        )),
    }
}

/// `browser/versions.json` of the extension package the launcher belongs to
/// (`<tools>/bin/chromium` → `<tools>/store/ext-core/browser/versions.json`).
pub fn package_versions(chromium: &Path) -> Option<Value> {
    let file = chromium.parent()?.parent()?.join("store/ext-core/browser/versions.json");
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

#[derive(Debug, Clone)]
pub struct BrowserDirs {
    pub root: PathBuf,
    pub profile: PathBuf,
    pub cache: PathBuf,
    pub downloads: PathBuf,
}

impl BrowserDirs {
    /// `OPENDAN_BROWSER_DIR` (the container's instance volume), else
    /// `<AgentRoot>/.cache/browser`.
    pub fn of(agent_root: &Path) -> Self {
        let root = std::env::var_os(ENV_BROWSER_DIR)
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| agent_root.join(".cache").join("browser"));
        Self::at(root)
    }

    pub fn at(root: PathBuf) -> Self {
        Self {
            profile: root.join("profile"),
            cache: root.join("cache"),
            downloads: root.join("downloads"),
            root,
        }
    }

    fn log(&self) -> PathBuf {
        self.root.join("chromium.log")
    }
}

pub struct Launched {
    pub child: tokio::process::Child,
    pub tree: ProcessTree,
    /// The debugging endpoint: `ws://127.0.0.1:<port><path>`.
    pub port: u16,
    pub path: String,
}

/// The browser and whatever it started: a process group (Unix) or a Job
/// Object (Windows).
pub struct ProcessTree {
    pub pid: u32,
    /// What every process the browser starts carries in its environment.
    mark: String,
    /// Processes of the browser that left its process group (its crash
    /// handlers do): `(pid, start time)`.
    strays: std::sync::Mutex<Vec<(u32, u64)>>,
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl ProcessTree {
    /// Ask the tree to end (Windows has nothing gentler than ending it).
    pub fn terminate(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGTERM);
        }
        #[cfg(windows)]
        self.kill();
    }

    /// Remember the processes of the browser that are not in its process
    /// group: found by the mark in their environment while they live (a
    /// process that ended has none to read), ended and collected by
    /// [`Self::sweep`].
    pub fn note_strays(&self) {
        #[cfg(unix)]
        {
            let mark = format!("{ENV_MARK}={}", self.mark);
            let mut strays = self.strays.lock().expect("strays");
            for pid in pids() {
                let Some(stat) = ProcStat::of(pid) else { continue };
                let marked = std::fs::read(format!("/proc/{pid}/environ"))
                    .is_ok_and(|env| env.split(|b| *b == 0).any(|var| var == mark.as_bytes()));
                if marked && stat.pgrp != self.pid && !strays.contains(&(pid, stat.started)) {
                    strays.push((pid, stat.started));
                }
            }
        }
    }

    /// End what is left of the tree and collect it.
    pub async fn sweep(&self) {
        #[cfg(unix)]
        {
            let (pid, strays) = (self.pid, std::mem::take(&mut *self.strays.lock().expect("strays")));
            let _ = tokio::task::spawn_blocking(move || {
                reap_group(pid);
                reap_strays(strays);
            })
            .await;
        }
        #[cfg(windows)]
        self.kill();
    }

    #[cfg(windows)]
    fn kill(&self) {
        match &self.job {
            Some(job) => job.terminate(),
            None => {
                let mut taskkill = std::process::Command::new("taskkill");
                taskkill.args(["/PID", &self.pid.to_string(), "/T", "/F"]);
                let _ = buckyos_api::hide_child_console(&mut taskkill).status();
            }
        }
    }
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// How the browser is started.
#[derive(Debug, Clone, PartialEq)]
pub enum Shown {
    /// Headless: its frames and input go through the DevTools protocol.
    Headless,
    /// Headed on the X display `display` (`:20`): it draws at `scale` device
    /// pixels per CSS pixel, its windows open at `size` (CSS pixels).
    Display { display: String, scale: f64, size: (u32, u32) },
}

/// How the browser is started. A headless one says `HeadlessChrome` in its
/// user agent: its pages are told otherwise over DevTools ([`super::identity`]),
/// since a user agent given here would leave their client hints empty.
fn arguments(dirs: &BrowserDirs, no_sandbox: bool, shown: &Shown, port: u16) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", dirs.profile.display()),
        format!("--disk-cache-dir={}", dirs.cache.display()),
        format!("--remote-debugging-port={port}"),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-dev-shm-usage".to_string(),
        // No tab of its own: every tab belongs to a group.
        "--no-startup-window".to_string(),
        // The caret of a focused field stays lit: one that blinks paints two frames a second for the screens.
        "--disable-blink-features=CaretBlinking".to_string(),
    ];
    match shown {
        Shown::Headless => {
            args.insert(0, "--headless=new".to_string());
            args.push("--window-size=1280,800".to_string());
            args.push(format!("--screen-info={SCREENS}"));
        }
        Shown::Display { scale, size: (width, height), .. } => {
            args.push("--window-position=0,0".to_string());
            args.push(format!("--window-size={width},{height}"));
            args.push(format!("--force-device-scale-factor={scale}"));
        }
    }
    if no_sandbox {
        args.push("--no-sandbox".to_string());
    }
    args
}

/// A loopback port nobody listens on now (the browser may still lose it to another process: it is then started again).
fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| format!("no free loopback port: {e}"))
}

/// The `TMPDIR` the browser is started with when the one of this process
/// is too long for its socket (`None`: this process's is fine).
fn short_tmpdir(tmpdir: Option<&std::ffi::OsStr>) -> Option<&'static str> {
    (cfg!(unix) && tmpdir.is_some_and(|t| t.len() > MAX_TMPDIR) && Path::new("/tmp").is_dir()).then_some("/tmp")
}

fn log_tail(dirs: &BrowserDirs) -> String {
    let text = std::fs::read_to_string(dirs.log()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(6)..].join(" | ")
}

/// A browser this profile was left with by a process that ended without
/// stopping it still holds the profile: it is ended, and the profile's lock
/// (which names a host and a pid: a container gets a new host name every
/// time it is recreated) is removed.
fn release_profile(dirs: &BrowserDirs) {
    #[cfg(unix)]
    if let Ok(owner) = std::fs::read_link(dirs.profile.join("SingletonLock")) {
        let pid = owner.to_string_lossy().rsplit('-').next().and_then(|p| p.parse::<i32>().ok());
        let profile = format!("--user-data-dir={}", dirs.profile.display());
        let holds = |pid: i32| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|c| c.split(|b| *b == 0).any(|a| a == profile.as_bytes()))
        };
        if let Some(pid) = pid.filter(|p| *p > 1 && holds(*p)) {
            log::warn!("browser: profile {} is held by a browser left behind (pid {pid}); ending it", dirs.profile.display());
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::kill(pid, libc::SIGKILL);
            }
            let until = Instant::now() + Duration::from_secs(3);
            while holds(pid) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    for name in ["SingletonLock", "SingletonCookie", "SingletonSocket", PORT_FILE] {
        let _ = std::fs::remove_file(dirs.profile.join(name));
    }
}

async fn spawn(exe: &Path, dirs: &BrowserDirs, no_sandbox: bool, shown: &Shown, port: u16) -> Result<Launched, String> {
    for dir in [&dirs.profile, &dirs.cache, &dirs.downloads] {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    release_profile(dirs);
    let log = std::fs::File::create(dirs.log()).map_err(|e| format!("create {}: {e}", dirs.log().display()))?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(arguments(dirs, no_sandbox, shown, port))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .kill_on_drop(true);
    if let Some(tmpdir) = short_tmpdir(std::env::var_os("TMPDIR").as_deref()) {
        cmd.env("TMPDIR", tmpdir);
    }
    match shown {
        Shown::Display { display, .. } => cmd.env("DISPLAY", display),
        Shown::Headless => cmd.env_remove("DISPLAY"),
    };
    cmd.env_remove("WAYLAND_DISPLAY");
    let mark = format!("{}-{}", std::process::id(), libopendan::now_ms());
    cmd.env(ENV_MARK, &mark);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(buckyos_api::windows_background_creation_flags());
    let mut child = cmd.spawn().map_err(|e| format!("start {}: {e}", exe.display()))?;
    let pid = child.id().ok_or_else(|| "the browser ended as it started".to_string())?;
    let tree = ProcessTree {
        pid,
        mark,
        strays: std::sync::Mutex::new(Vec::new()),
        #[cfg(windows)]
        job: job::Job::of(&child).map_err(|e| log::warn!("browser: no job object for pid {pid}: {e}")).ok(),
    };

    let port_file = dirs.profile.join(PORT_FILE);
    let started = Instant::now();
    loop {
        // The browser says where it listens in its log; a headless one also writes `DevToolsActivePort`.
        let endpoint = std::fs::read_to_string(dirs.log()).ok().and_then(|log| listening(&log, port)).or_else(|| {
            let text = std::fs::read_to_string(&port_file).ok()?;
            let mut lines = text.lines();
            Some((lines.next()?.trim().parse::<u16>().ok()?, lines.next()?.trim().to_string()))
        });
        if let Some((port, path)) = endpoint {
            return Ok(Launched { child, tree, port, path });
        }
        if let Ok(Some(status)) = child.try_wait() {
            tree.sweep().await;
            return Err(format!("the browser ended while starting ({status}): {}", log_tail(dirs)));
        }
        if std::fs::read_to_string(dirs.log()).is_ok_and(|log| log.contains(PORT_TAKEN)) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            tree.sweep().await;
            return Err(format!("{PORT_TAKEN} on port {port}"));
        }
        if started.elapsed() > START_TIMEOUT {
            let _ = child.start_kill();
            let _ = child.wait().await;
            tree.sweep().await;
            return Err(format!("the browser did not open its debugging endpoint in {}s: {}", START_TIMEOUT.as_secs(), log_tail(dirs)));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `(port, path)` of the debugging endpoint the log says the browser listens on (`port` is the one it was given).
fn listening(log: &str, port: u16) -> Option<(u16, String)> {
    let prefix = format!("DevTools listening on ws://127.0.0.1:{port}");
    log.lines().find_map(|line| line.trim().strip_prefix(&prefix)).map(|path| (port, path.trim().to_string()))
}

/// Start the browser. As root its sandbox is off (it refuses to start
/// otherwise); as anyone else it is tried with the sandbox first and once
/// more without when the system offers none.
pub async fn launch(exe: &Path, dirs: &BrowserDirs, shown: &Shown) -> Result<Launched, String> {
    let mut no_sandbox = is_root();
    let mut tries = 0;
    loop {
        tries += 1;
        match spawn(exe, dirs, no_sandbox, shown, free_port()?).await {
            Err(e) if !no_sandbox && e.contains(NO_SANDBOX_ERROR) => {
                log::warn!("browser: this system offers Chromium no usable sandbox; starting it with --no-sandbox ({e})");
                no_sandbox = true;
            }
            Err(e) if e.contains(PORT_TAKEN) && tries < 4 => log::warn!("browser: {e}; starting it on another port"),
            other => return other,
        }
    }
}

/// What `/proc/<pid>/stat` says about a process (Linux).
#[cfg(unix)]
struct ProcStat {
    state: char,
    ppid: u32,
    pgrp: u32,
    /// Start time: with the pid it names one process for good.
    started: u64,
}

#[cfg(unix)]
impl ProcStat {
    fn of(pid: u32) -> Option<Self> {
        // `pid (comm) state ppid pgrp …`: comm may hold spaces and parentheses.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        Some(Self {
            state: fields.first()?.chars().next()?,
            ppid: fields.get(1)?.parse().ok()?,
            pgrp: fields.get(2)?.parse().ok()?,
            started: fields.get(19)?.parse().ok()?,
        })
    }
}

#[cfg(unix)]
fn pids() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries.flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()).collect()
}

/// Pids of the process group `pgid` (Linux: read from `/proc`).
#[cfg(unix)]
pub fn group_pids(pgid: u32) -> Vec<u32> {
    pids().into_iter().filter(|pid| ProcStat::of(*pid).is_some_and(|stat| stat.pgrp == pgid)).collect()
}

/// End the processes of the browser that left its group and collect those
/// that are this process's children. A pid is only touched while it still
/// names the process that was noted (same start time).
#[cfg(unix)]
fn reap_strays(mut strays: Vec<(u32, u64)>) {
    let me = std::process::id();
    let until = Instant::now() + Duration::from_secs(3);
    let mut killed = false;
    loop {
        strays.retain(|(pid, started)| {
            let Some(stat) = ProcStat::of(*pid).filter(|stat| stat.started == *started) else { return false };
            if stat.state != 'Z' && !killed {
                unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
            }
            if stat.ppid == me {
                let mut status = 0;
                unsafe { libc::waitpid(*pid as i32, &mut status, libc::WNOHANG) };
            }
            // Someone else's zombie is theirs to collect.
            stat.ppid == me || stat.state != 'Z'
        });
        killed = true;
        if strays.is_empty() {
            break;
        }
        if Instant::now() > until {
            log::warn!("browser: processes that left the browser's group are still there: {strays:?}");
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// End what is left of the browser's process group and collect the
/// processes of it that became this process's children (in a container the
/// Agent Service is pid 1: a browser child whose parent is gone is handed to
/// it and stays a zombie until it is waited for). Only pids seen in that
/// group are waited for, never "any child": a shell command's exit status
/// belongs to whoever started it. The browser itself is waited for by its
/// owner.
#[cfg(unix)]
pub fn reap_group(pgid: u32) {
    unsafe { libc::kill(-(pgid as i32), libc::SIGKILL) };
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let left: Vec<u32> = group_pids(pgid).into_iter().filter(|pid| *pid != pgid).collect();
        for pid in &left {
            let mut status = 0;
            unsafe { libc::waitpid(*pid as i32, &mut status, libc::WNOHANG) };
        }
        if left.is_empty() {
            break;
        }
        if Instant::now() > until {
            log::warn!("browser: processes of group {pgid} are still there: {left:?}");
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
pub mod job {
    //! The browser's process tree as a Job Object (the way `agent_tool`
    //! ends a shell command's tree).

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const std::ffi::c_void, name: *const u16) -> *mut std::ffi::c_void;
        fn AssignProcessToJobObject(job: *mut std::ffi::c_void, process: *mut std::ffi::c_void) -> i32;
        fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }

    pub struct Job(usize);

    impl Job {
        pub fn of(child: &tokio::process::Child) -> std::io::Result<Self> {
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let assigned = child
                .raw_handle()
                .ok_or_else(|| std::io::Error::other("the browser ended before it was assigned to a job"))
                .and_then(|process| {
                    if unsafe { AssignProcessToJobObject(job, process as _) } == 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            match assigned {
                Ok(()) => Ok(Self(job as usize)),
                Err(e) => {
                    unsafe { CloseHandle(job) };
                    Err(e)
                }
            }
        }

        pub fn terminate(&self) {
            unsafe { TerminateJobObject(self.0 as _, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0 as _) };
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_group_of_a_process_is_read_from_proc() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & sleep 30 & wait"])
            .process_group(0)
            .spawn()
            .unwrap();
        let pgid = child.id();
        let until = Instant::now() + Duration::from_secs(5);
        while group_pids(pgid).len() < 3 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(group_pids(pgid).len(), 3, "the shell and its two children");
        reap_group(pgid);
        child.wait().unwrap();
        assert!(group_pids(pgid).is_empty());
    }

    #[test]
    fn the_endpoint_is_read_from_the_log() {
        let log = "[1:1:ERROR:dbus] no bus\n\nDevTools listening on ws://127.0.0.1:9333/devtools/browser/83acad6b\n";
        assert_eq!(listening(log, 9333), Some((9333, "/devtools/browser/83acad6b".to_string())));
        assert_eq!(listening(log, 9334), None);
        assert_eq!(listening("starting", 9333), None);
    }

    #[test]
    fn directories_follow_the_environment_or_the_agent_root() {
        let dirs = BrowserDirs::at(PathBuf::from("/x/browser"));
        assert_eq!(dirs.profile, PathBuf::from("/x/browser/profile"));
        assert_eq!(dirs.downloads, PathBuf::from("/x/browser/downloads"));
        let args = arguments(&dirs, true, &Shown::Headless, 9222);
        assert!(args.contains(&"--headless=new".to_string()));
        assert!(args.contains(&"--user-data-dir=/x/browser/profile".to_string()));
        assert!(args.contains(&"--remote-debugging-port=9222".to_string()));
        assert!(args.contains(&"--no-sandbox".to_string()));
        assert!(args.contains(&"--disable-blink-features=CaretBlinking".to_string()));
        assert!(args.contains(&"--screen-info={0,0 2560x1440}{2560,0 5120x2880 devicePixelRatio=2}".to_string()));
        // Its pages are told its user agent over DevTools, with their client hints.
        assert!(!args.iter().any(|a| a.starts_with("--user-agent=")));
        assert!(!arguments(&dirs, false, &Shown::Headless, 1).contains(&"--no-sandbox".to_string()));
        // Headed: its windows have the display's size and scale.
        let headed = Shown::Display { display: ":20".into(), scale: 2.0, size: (412, 915) };
        let args = arguments(&dirs, false, &headed, 9222);
        assert!(!args.iter().any(|a| a.starts_with("--headless") || a.starts_with("--user-agent=") || a.starts_with("--screen-info=")), "{args:?}");
        for wanted in ["--window-size=412,915", "--window-position=0,0", "--force-device-scale-factor=2", "--no-startup-window"] {
            assert!(args.contains(&wanted.to_string()), "{wanted} {args:?}");
        }
    }

    /// The extension package brings the launcher; the browser itself is
    /// its online item, there once its record says `ready`.
    #[test]
    fn the_browser_of_the_tool_directories_is_there_once_its_download_is_ready() {
        use serde_json::json;
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let tools = tmp.path().join("tools");
        let dirs = [tools.join("bin"), tmp.path().join("bin/agent-tools")];
        let launcher = tools.join("bin/chromium");
        let why = |found: Chromium| match found {
            Chromium::NotDownloaded { launcher: of, why } if of == launcher => why,
            other => panic!("not waiting for its download: {other:?}"),
        };
        let record = |record: Value| std::fs::write(tools.join("online/chromium.json"), record.to_string()).unwrap();

        // No extension package.
        assert!(matches!(chromium_in(&dirs), Chromium::NoPackage(why) if why.contains("no `chromium` on this node")));
        assert!(matches!(chromium_in(&[]), Chromium::NoPackage(_)));
        assert_eq!(chromium_in(&dirs).launcher(), None);

        // The package, whose online item is not there yet.
        std::fs::create_dir_all(tools.join("bin")).unwrap();
        std::fs::write(&launcher, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(why(chromium_in(&dirs)).contains("not downloaded on this node yet"));
        assert_eq!(chromium_in(&dirs).launcher(), Some(launcher.as_path()));
        std::fs::create_dir_all(tools.join("online")).unwrap();
        record(json!({ "state": "downloading", "size_bytes": 200u64 << 20, "downloaded_bytes": 84u64 << 20 }));
        assert_eq!(why(chromium_in(&dirs)), "the browser is being downloaded to this node (42% of 200 MiB)");
        record(json!({ "state": "downloading" }));
        assert_eq!(why(chromium_in(&dirs)), "the browser is being downloaded to this node");
        record(json!({ "state": "failed", "error": "sha256 mismatch." }));
        assert!(why(chromium_in(&dirs)).contains("failed: sha256 mismatch. Start it again in Settings"));
        record(json!({ "state": "failed", "error": null }));
        assert!(why(chromium_in(&dirs)).contains("failed: no reason is recorded."));
        record(json!({ "state": "paused" }));
        assert!(why(chromium_in(&dirs)).contains("does not know: paused"));
        std::fs::write(tools.join("online/chromium.json"), "{ \"state\": \"rea").unwrap();
        assert!(why(chromium_in(&dirs)).contains("is not readable"));
        record(json!({ "state": "ready" }));
        assert!(why(chromium_in(&dirs)).contains("recorded as ready, but"), "the record without what it records");

        // Downloaded.
        std::fs::create_dir_all(tools.join("online/chromium")).unwrap();
        assert_eq!(chromium_in(&dirs), Chromium::Ready(launcher.clone()));
        // Downloaded again: not there until that ends.
        record(json!({ "state": "downloading", "size_bytes": 10, "downloaded_bytes": 0 }));
        assert!(why(chromium_in(&dirs)).contains("0% of 1 MiB"));

        // An executable named outright is started as it is.
        assert_eq!(named_chromium(&launcher), Chromium::Ready(launcher.clone()));
        assert!(matches!(named_chromium(Path::new("/nonexistent/chromium")), Chromium::NoPackage(why) if why.contains("/nonexistent/chromium")));
    }

    #[test]
    fn a_tmpdir_too_long_for_the_browsers_socket_is_replaced() {
        use std::ffi::OsStr;
        assert_eq!(short_tmpdir(None), None);
        assert_eq!(short_tmpdir(Some(OsStr::new("/tmp"))), None);
        assert_eq!(short_tmpdir(Some(OsStr::new("/opt/buckyos/data/home/alice/agents/jarvis.alice/.cache/tmp/session"))), Some("/tmp"));
    }
}
