//! Parent side of worker processes: identity switch at spawn (root Unix
//! daemon), process group / job control, bounded I/O. A Windows host runs
//! workers as its own user.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use buckyos_api::{ExecutorError, ExecutorErrorCode, ExecutorRunAs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::worker::{WorkerJob, WorkerReply};

/// Reads the script from stdin, then runs it with stdin on /dev/null, so the
/// command text never appears in argv.
#[cfg(unix)]
pub const SHELL_WRAPPER: &str = "IFS= read -r -d '' __nx_script; exec </dev/null; eval \"$__nx_script\"";

/// Reads the whole script from stdin as UTF-8 (stdin is then at EOF), so the
/// command text never appears in argv, and runs it as a script block. The
/// exit code is the script's `exit`, else the last native command's, 1 on a
/// terminating error.
#[cfg(windows)]
pub const SHELL_WRAPPER: &str = r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$utf8 = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
$__nx_buffer = [System.IO.MemoryStream]::new()
[Console]::OpenStandardInput().CopyTo($__nx_buffer)
$__nx_script = [ScriptBlock]::Create($utf8.GetString($__nx_buffer.ToArray()))
Remove-Variable __nx_buffer
$global:LASTEXITCODE = 0
try {
    & $__nx_script
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine($_.ToString())
    exit 1
}"#;

const MAX_REPLY_LINE: usize = 64 * 1024;
const FILE_JOB_TIMEOUT: Duration = Duration::from_secs(120);
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(2);
#[cfg(unix)]
const MAX_FD_SWEEP: libc::rlim_t = 1 << 20;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct WorkerLauncher {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct HostIdentity {
    pub mode: crate::ExecutorIdentityMode,
    #[cfg(unix)]
    pub uid: libc::uid_t,
    #[cfg(unix)]
    pub gid: libc::gid_t,
    #[cfg(unix)]
    groups: Vec<libc::gid_t>,
}

impl HostIdentity {
    pub fn capture(mode: crate::ExecutorIdentityMode) -> std::io::Result<Self> {
        #[cfg(unix)]
        unsafe {
            let uid = libc::getuid();
            let gid = libc::getgid();
            if mode == crate::ExecutorIdentityMode::CurrentUser
                && (uid != libc::geteuid() || gid != libc::getegid())
            {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
                    "current_user requires matching real and effective uid/gid"));
            }
            let count = libc::getgroups(0, std::ptr::null_mut());
            if count < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut groups = vec![0; count as usize];
            if libc::getgroups(count, groups.as_mut_ptr()) != count {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self { mode, uid, gid, groups })
        }
        #[cfg(not(unix))]
        Ok(Self { mode })
    }

    pub fn can_run(&self) -> bool {
        match self.mode {
            crate::ExecutorIdentityMode::CurrentUser => cfg!(any(unix, windows)),
            crate::ExecutorIdentityMode::SwitchUser => {
                #[cfg(unix)]
                { unsafe { libc::geteuid() == 0 } }
                #[cfg(not(unix))]
                { false }
            }
        }
    }
}

pub(crate) fn executable_available(path: &Path) -> bool {
    if !path.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false; };
        unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
    }
    #[cfg(not(unix))]
    { true }
}

#[cfg(unix)]
pub fn default_shell() -> PathBuf {
    for candidate in ["/bin/bash", "/usr/bin/bash"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from("/bin/bash")
}

/// PowerShell 7 when it is on PATH, else Windows PowerShell.
#[cfg(windows)]
pub fn default_shell() -> PathBuf {
    if let Some(path) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("pwsh.exe"))
            .find(|file| file.is_file())
    }) {
        return path;
    }
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32\\WindowsPowerShell\\v1.0\\powershell.exe")
}

fn internal(msg: impl Into<String>) -> ExecutorError {
    ExecutorError::new(ExecutorErrorCode::Internal, msg)
}

#[cfg(unix)]
fn drop_privileges(
    cmd: &mut Command,
    identity: &HostIdentity,
    run_as: Option<&ExecutorRunAs>,
    cwd: &Path,
) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let switching = identity.mode == crate::ExecutorIdentityMode::SwitchUser;
    let (uid, gid, groups) = match (switching, run_as) {
        (true, Some(run_as)) => (run_as.uid as libc::uid_t, run_as.gid as libc::gid_t, vec![run_as.gid as libc::gid_t]),
        (false, None) => (identity.uid, identity.gid, identity.groups.clone()),
        _ => return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, crate::RUN_AS_MISMATCH)),
    };
    let mut actual_groups = vec![0; groups.len().max(1)];
    let cwd = std::ffi::CString::new(cwd.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    cmd.process_group(0);
    unsafe {
        cmd.pre_exec(move || {
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::prctl(
                    libc::PR_CAP_AMBIENT,
                    libc::PR_CAP_AMBIENT_CLEAR_ALL as libc::c_ulong,
                    0,
                    0,
                    0,
                );
            }
            if switching && (libc::setgroups(1, &gid) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0) {
                return Err(std::io::Error::last_os_error());
            }
            let count = libc::getgroups(actual_groups.len() as libc::c_int, actual_groups.as_mut_ptr());
            if libc::getuid() != uid
                || libc::geteuid() != uid
                || libc::getgid() != gid
                || libc::getegid() != gid
                || count != groups.len() as libc::c_int
                || actual_groups[..groups.len()] != groups[..]
            {
                return Err(std::io::Error::from_raw_os_error(libc::EPERM));
            }
            if libc::chdir(cwd.as_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            mark_inherited_fds_cloexec();
            Ok(())
        });
    }
    Ok(())
}

#[cfg(unix)]
unsafe fn mark_inherited_fds_cloexec() {
    #[cfg(target_os = "linux")]
    if libc::syscall(
        libc::SYS_close_range,
        3 as libc::c_uint,
        libc::c_uint::MAX,
        libc::CLOSE_RANGE_CLOEXEC as libc::c_uint,
    ) == 0
    {
        return;
    }
    sweep_fds_cloexec();
}

/// Async-signal-safe fallback: fcntl every descriptor below RLIMIT_NOFILE.
#[cfg(unix)]
unsafe fn sweep_fds_cloexec() {
    let mut limit: libc::rlimit = std::mem::zeroed();
    let max = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 {
        limit.rlim_cur.min(MAX_FD_SWEEP)
    } else {
        MAX_FD_SWEEP
    };
    for fd in 3..max as libc::c_int {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
}

#[cfg(unix)]
fn signal_group(pgid: u32, signal: i32) {
    if pgid > 1 {
        unsafe {
            libc::kill(-(pgid as i32), signal);
        }
    }
}

/// A job object holding a shell and everything it starts. Closing the handle
/// leaves the processes running (no KILL_ON_JOB_CLOSE): what a command leaves
/// in the background after exiting normally is not managed, as on Unix.
#[cfg(windows)]
mod job {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    pub struct Job(*mut c_void);

    // A kernel handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn assign(child: &tokio::process::Child) -> std::io::Result<Job> {
            let process = child
                .raw_handle()
                .ok_or_else(|| std::io::Error::other("the shell exited before job assignment"))?;
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = Job(handle);
            if unsafe { AssignProcessToJobObject(job.0, process as *mut c_void) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(job)
        }

        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// A running shell with the handle that stops everything it started.
pub struct ShellProcess {
    pub child: Child,
    #[cfg(unix)]
    pgid: u32,
    #[cfg(windows)]
    job: job::Job,
}

impl ShellProcess {
    pub fn pid(&self) -> u32 {
        self.child.id().unwrap_or(0)
    }

    /// Unix: SIGTERM the process group, SIGKILL after a grace period.
    /// Windows: terminate the job. The caller confirms the exit by waiting.
    pub async fn terminate(&mut self) {
        #[cfg(unix)]
        {
            signal_group(self.pgid, libc::SIGTERM);
            let _ = tokio::time::timeout(TERM_GRACE, self.child.wait()).await;
            signal_group(self.pgid, libc::SIGKILL);
        }
        #[cfg(windows)]
        self.job.terminate();
    }
}

#[cfg(unix)]
pub fn spawn_shell(
    shell: &Path,
    identity: &HostIdentity,
    run_as: Option<&ExecutorRunAs>,
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> std::io::Result<ShellProcess> {
    let mut cmd = Command::new(shell);
    cmd.arg("--noprofile")
        .arg("--norc")
        .arg("-c")
        .arg(SHELL_WRAPPER)
        .env_clear()
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    drop_privileges(&mut cmd, identity, run_as, cwd)?;
    let child = cmd.spawn()?;
    let pgid = child.id().unwrap_or(0);
    Ok(ShellProcess { child, pgid })
}

/// The shell joins its job before it is given the script, so nothing the
/// command starts escapes cancellation.
#[cfg(windows)]
pub fn spawn_shell(
    shell: &Path,
    identity: &HostIdentity,
    run_as: Option<&ExecutorRunAs>,
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> std::io::Result<ShellProcess> {
    use base64::Engine;
    if identity.mode != crate::ExecutorIdentityMode::CurrentUser || run_as.is_some() {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, crate::RUN_AS_MISMATCH));
    }
    let wrapper: Vec<u8> = SHELL_WRAPPER.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = Command::new(shell);
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-OutputFormat",
        "Text",
        "-EncodedCommand",
    ])
    .arg(base64::engine::general_purpose::STANDARD.encode(wrapper))
    .env_clear()
    .envs(env)
    .current_dir(cwd)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .creation_flags(CREATE_NO_WINDOW)
    .kill_on_drop(false);
    let mut child = cmd.spawn()?;
    match job::Job::assign(&child) {
        Ok(job) => Ok(ShellProcess { child, job }),
        Err(e) => {
            let _ = child.start_kill();
            Err(e)
        }
    }
}

#[cfg(unix)]
fn prepare_worker(cmd: &mut Command, identity: &HostIdentity, run_as: Option<&ExecutorRunAs>) -> std::io::Result<()> {
    cmd.env("PATH", "/usr/bin:/bin").env("LANG", "C.UTF-8");
    drop_privileges(cmd, identity, run_as, Path::new("/"))
}

#[cfg(windows)]
fn prepare_worker(cmd: &mut Command, identity: &HostIdentity, run_as: Option<&ExecutorRunAs>) -> std::io::Result<()> {
    if identity.mode != crate::ExecutorIdentityMode::CurrentUser || run_as.is_some() {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, crate::RUN_AS_MISMATCH));
    }
    if let Some(root) = std::env::var_os("SystemRoot") {
        cmd.env("SystemRoot", root);
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    Ok(())
}

fn kill_worker(child: &mut Child) {
    #[cfg(unix)]
    signal_group(child.id().unwrap_or(0), libc::SIGKILL);
    #[cfg(windows)]
    let _ = child.start_kill();
}

/// Outcome of a file job: `Unknown` means the worker died without a reply,
/// so a write may or may not have happened.
pub enum FileJobOutcome {
    Reply(WorkerReply, Vec<u8>),
    SpawnFailed(ExecutorError),
    Unknown(ExecutorError),
}

pub async fn run_file_job(
    launcher: &WorkerLauncher,
    identity: &HostIdentity,
    run_as: Option<&ExecutorRunAs>,
    job: &WorkerJob,
    payload: &[u8],
    max_reply_payload: u64,
    mut cancel: Option<tokio::sync::watch::Receiver<Option<crate::ops::CancelReason>>>,
) -> FileJobOutcome {
    let mut cmd = Command::new(&launcher.program);
    cmd.args(&launcher.args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let spawned = prepare_worker(&mut cmd, identity, run_as).and_then(|_| cmd.spawn());
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return FileJobOutcome::SpawnFailed(internal(format!("cannot start executor worker: {e}"))),
    };
    let mut line = match serde_json::to_vec(job) {
        Ok(v) => v,
        Err(e) => return FileJobOutcome::SpawnFailed(internal(e.to_string())),
    };
    line.push(b'\n');
    let mut stdin = child.stdin.take().expect("piped stdin");
    let input = [line, payload.to_vec()].concat();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
        let _ = stdin.shutdown().await;
    });
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let limit = MAX_REPLY_LINE as u64 + max_reply_payload + 1;
    let io = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_limited = (&mut stdout).take(limit);
        let mut err_limited = (&mut stderr).take(16 * 1024);
        let (r1, _) = tokio::join!(
            out_limited.read_to_end(&mut out),
            err_limited.read_to_end(&mut err)
        );
        r1.map(|_| (out, err))
    };
    let cancelled = async {
        if let Some(rx) = cancel.as_mut() {
            loop {
                if rx.borrow().is_some() { return; }
                if rx.changed().await.is_err() { break; }
            }
        }
        std::future::pending::<()>().await;
    };
    let collected = tokio::select! {
        result = tokio::time::timeout(FILE_JOB_TIMEOUT, io) => Some(result),
        _ = cancelled => None,
    };
    let Some(collected) = collected else {
        kill_worker(&mut child);
        let _ = child.wait().await;
        writer.abort();
        return FileJobOutcome::Unknown(ExecutorError::new(ExecutorErrorCode::Unavailable, "worker cancelled"));
    };
    writer.abort();
    let _ = writer.await;
    let (out, err) = match collected {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            kill_worker(&mut child);
            let _ = child.wait().await;
            return FileJobOutcome::Unknown(internal(format!("worker io failed: {e}")));
        }
        Err(_) => {
            kill_worker(&mut child);
            let _ = child.wait().await;
            return FileJobOutcome::Unknown(internal("worker timed out"));
        }
    };
    let status = child.wait().await;
    let Some(nl) = out.iter().position(|b| *b == b'\n') else {
        return FileJobOutcome::Unknown(internal(format!(
            "worker exited without a reply ({:?}): {}",
            status.ok().and_then(|s| s.code()),
            String::from_utf8_lossy(&err).trim()
        )));
    };
    let reply: WorkerReply = match serde_json::from_slice(&out[..nl]) {
        Ok(r) => r,
        Err(e) => return FileJobOutcome::Unknown(internal(format!("bad worker reply: {e}"))),
    };
    let payload = out[nl + 1..].to_vec();
    if payload.len() as u64 != reply.payload_len || reply.payload_len > max_reply_payload {
        return FileJobOutcome::Unknown(internal("worker reply payload length mismatch"));
    }
    FileJobOutcome::Reply(reply, payload)
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn fd_sweep_marks_inherited_descriptors_close_on_exec() {
        let fd = unsafe { libc::dup(2) };
        assert!(fd > 2);
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
        unsafe { super::sweep_fds_cloexec() };
        assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
        unsafe { libc::close(fd) };
    }
}
