//! Parent side of worker processes: privilege drop at spawn, bounded I/O.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::Stdio;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::Duration;

use buckyos_api::{ExecutorError, ExecutorErrorCode, ExecutorRunAs};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::process::Command;

use crate::worker::{WorkerJob, WorkerReply};

/// Reads the script from stdin, then runs it with stdin on /dev/null, so the
/// command text never appears in argv.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub const SHELL_WRAPPER: &str = "IFS= read -r -d '' __nx_script; exec </dev/null; eval \"$__nx_script\"";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_REPLY_LINE: usize = 64 * 1024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const FILE_JOB_TIMEOUT: Duration = Duration::from_secs(120);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_FD_SWEEP: libc::rlim_t = 1 << 20;

#[derive(Debug, Clone)]
pub struct WorkerLauncher {
    pub program: PathBuf,
    pub args: Vec<String>,
}

pub fn default_shell() -> PathBuf {
    for candidate in ["/bin/bash", "/usr/bin/bash"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from("/bin/bash")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn internal(msg: impl Into<String>) -> ExecutorError {
    ExecutorError::new(ExecutorErrorCode::Internal, msg)
}

/// In the child before exec: a new process group; on Linux no_new_privs and
/// no ambient capabilities; then setgroups([gid]) + setgid + setuid, checked
/// afterwards, chdir as the target user, and every inherited descriptor
/// marked close-on-exec. The group list is always set explicitly: on macOS
/// `setgroups(0, NULL)` (what std's `Command::uid` uses) makes the egid 0.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn drop_privileges(cmd: &mut Command, run_as: &ExecutorRunAs, cwd: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let uid = run_as.uid as libc::uid_t;
    let gid = run_as.gid as libc::gid_t;
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
            if libc::setgroups(1, &gid) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut groups = [0 as libc::gid_t; 4];
            let count = libc::getgroups(groups.len() as libc::c_int, groups.as_mut_ptr());
            if libc::getuid() != uid
                || libc::geteuid() != uid
                || libc::getgid() != gid
                || libc::getegid() != gid
                || count != 1
                || groups[0] != gid
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
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
#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn spawn_shell(
    shell: &Path,
    run_as: &ExecutorRunAs,
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> std::io::Result<Child> {
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
    drop_privileges(&mut cmd, run_as, cwd)?;
    cmd.spawn()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn spawn_shell(
    _shell: &Path,
    _run_as: &ExecutorRunAs,
    _cwd: &Path,
    _env: &BTreeMap<String, String>,
) -> std::io::Result<Child> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "node executor execution requires Linux or macOS",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn signal_group(pgid: u32, signal: i32) {
    if pgid > 1 {
        unsafe {
            libc::kill(-(pgid as i32), signal);
        }
    }
}

/// Outcome of a file job: `Unknown` means the worker died without a reply,
/// so a write may or may not have happened.
pub enum FileJobOutcome {
    Reply(WorkerReply, Vec<u8>),
    SpawnFailed(ExecutorError),
    Unknown(ExecutorError),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub async fn run_file_job(
    launcher: &WorkerLauncher,
    run_as: &ExecutorRunAs,
    job: &WorkerJob,
    payload: &[u8],
    max_reply_payload: u64,
) -> FileJobOutcome {
    let mut cmd = Command::new(&launcher.program);
    cmd.args(&launcher.args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let spawned = drop_privileges(&mut cmd, run_as, Path::new("/")).and_then(|_| cmd.spawn());
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            return FileJobOutcome::SpawnFailed(internal(format!(
                "cannot start executor worker as uid {}: {e}",
                run_as.uid
            )))
        }
    };
    let pid = child.id().unwrap_or(0);
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
    let collected = tokio::time::timeout(FILE_JOB_TIMEOUT, io).await;
    let _ = writer.await;
    let (out, err) = match collected {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            signal_group(pid, libc::SIGKILL);
            let _ = child.wait().await;
            return FileJobOutcome::Unknown(internal(format!("worker io failed: {e}")));
        }
        Err(_) => {
            signal_group(pid, libc::SIGKILL);
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn run_file_job(
    _launcher: &WorkerLauncher,
    _run_as: &ExecutorRunAs,
    _job: &WorkerJob,
    _payload: &[u8],
    _max_reply_payload: u64,
) -> FileJobOutcome {
    FileJobOutcome::SpawnFailed(ExecutorError::new(
        ExecutorErrorCode::Capability,
        "node executor file operations require Linux or macOS",
    ))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
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
