//! Parent side of worker processes: privilege drop at spawn, bounded I/O.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Stdio;
#[cfg(target_os = "linux")]
use std::time::Duration;

use buckyos_api::{ExecutorError, ExecutorErrorCode, ExecutorRunAs};
#[cfg(target_os = "linux")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
#[cfg(target_os = "linux")]
use tokio::process::Command;

use crate::worker::{WorkerJob, WorkerReply};

/// Reads the script from stdin, then runs it with stdin on /dev/null, so the
/// command text never appears in argv.
#[cfg(target_os = "linux")]
pub const SHELL_WRAPPER: &str = "IFS= read -r -d '' __nx_script; exec </dev/null; eval \"$__nx_script\"";
#[cfg(target_os = "linux")]
const MAX_REPLY_LINE: usize = 64 * 1024;
#[cfg(target_os = "linux")]
const FILE_JOB_TIMEOUT: Duration = Duration::from_secs(120);

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

#[cfg(target_os = "linux")]
fn internal(msg: impl Into<String>) -> ExecutorError {
    ExecutorError::new(ExecutorErrorCode::Internal, msg)
}

/// setgroups([]) + setgid + setuid (std, before exec), a new process group,
/// no_new_privs, and every inherited descriptor marked close-on-exec.
#[cfg(target_os = "linux")]
fn drop_privileges(cmd: &mut Command, run_as: &ExecutorRunAs) {
    cmd.uid(run_as.uid).gid(run_as.gid).process_group(0);
    unsafe {
        cmd.pre_exec(|| {
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
            libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                libc::c_uint::MAX,
                libc::CLOSE_RANGE_CLOEXEC as libc::c_uint,
            );
            Ok(())
        });
    }
}

#[cfg(target_os = "linux")]
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
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    drop_privileges(&mut cmd, run_as);
    cmd.spawn()
}

#[cfg(not(target_os = "linux"))]
pub fn spawn_shell(
    _shell: &Path,
    _run_as: &ExecutorRunAs,
    _cwd: &Path,
    _env: &BTreeMap<String, String>,
) -> std::io::Result<Child> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "node executor execution requires Linux",
    ))
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    drop_privileges(&mut cmd, run_as);
    let mut child = match cmd.spawn() {
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

#[cfg(not(target_os = "linux"))]
pub async fn run_file_job(
    _launcher: &WorkerLauncher,
    _run_as: &ExecutorRunAs,
    _job: &WorkerJob,
    _payload: &[u8],
    _max_reply_payload: u64,
) -> FileJobOutcome {
    FileJobOutcome::SpawnFailed(ExecutorError::new(
        ExecutorErrorCode::Capability,
        "node executor file operations require Linux",
    ))
}
