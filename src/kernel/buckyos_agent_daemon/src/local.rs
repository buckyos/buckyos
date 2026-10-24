use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::access::valid_executor_id;

#[cfg(windows)]
#[path = "local_windows.rs"]
pub mod windows;

#[derive(Clone, Serialize, Deserialize)]
pub struct Status {
    pub executor_id: String,
    pub pid: u32,
    pub process_start: String,
    pub uid: u32,
    #[cfg(windows)]
    pub user_sid: String,
    pub state: String,
    pub generation: Option<u64>,
    pub store_id: Option<String>,
    pub expires_at_ms: Option<u64>,
    pub updated_at_ms: u64,
    pub capabilities: Value,
}

pub fn default_data_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("buckyos-agent-daemon"))
            .ok_or_else(|| "an absolute LOCALAPPDATA or --data-dir is required".into());
    }
    #[cfg(not(windows))]
    {
        #[cfg(not(target_os = "macos"))]
        if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
            if Path::new(&path).is_absolute() {
                return Ok(PathBuf::from(path).join("buckyos-agent-daemon"));
            }
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .map(|home| {
                home.join(if cfg!(target_os = "macos") {
                    "Library/Application Support/buckyos-agent-daemon"
                } else {
                    ".local/share/buckyos-agent-daemon"
                })
            })
            .ok_or_else(|| "an absolute HOME or --data-dir is required".into())
    }
}

pub fn private_dir(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    return windows::private_dir(path);
    #[cfg(not(windows))]
    {
        fs::create_dir_all(path).map_err(|_| "cannot create daemon data directory")?;
        let metadata =
            fs::symlink_metadata(path).map_err(|_| "cannot inspect daemon data directory")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("daemon data directory must be a real directory".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err("daemon data directory belongs to another OS user".into());
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| "cannot secure daemon data directory")?;
        }
        Ok(())
    }
}

pub fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(target_os = "linux")]
fn process_start(pid: u32) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    if pid == 0 || fs::metadata(format!("/proc/{pid}")).ok()?.uid() != current_uid() {
        return None;
    }
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if matches!(fields.first(), Some(&"Z") | Some(&"X")) {
        return None;
    }
    fields.get(19).map(|value| (*value).to_owned())
}

#[cfg(target_os = "macos")]
fn process_start(pid: u32) -> Option<String> {
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    if info.pbi_uid != current_uid() || info.pbi_pid != pid || info.pbi_status == libc::SZOMB {
        return None;
    }
    Some(format!(
        "{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

#[cfg(windows)]
fn process_start(pid: u32) -> Option<String> {
    windows::process_start(pid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn process_start(_: u32) -> Option<String> {
    None
}

pub fn capabilities(shell: &Path) -> Value {
    #[cfg(unix)]
    let executable = {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::CString::new(shell.as_os_str().as_bytes())
            .ok()
            .is_some_and(|path| unsafe { libc::access(path.as_ptr(), libc::X_OK) } == 0)
    };
    #[cfg(windows)]
    let executable = shell
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    #[cfg(not(any(unix, windows)))]
    let executable = false;
    let supported = cfg!(any(target_os = "linux", target_os = "macos", windows));
    let result = json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "identity_mode": "current_user",
        "os_session": current_identity(),
        "command": {"supported": supported, "ready": supported && shell.is_file() && executable},
        "file": {"supported": supported, "ready": supported},
        "ui": {"supported": false, "ready": false, "state": "unsupported"},
        "accessibility": {"supported": false, "ready": false, "state": "unsupported"},
        "capture": {"supported": false, "ready": false, "state": "unsupported"},
        "input": {"supported": false, "ready": false, "state": "unsupported"}
    });
    #[cfg(any(target_os = "macos", windows))]
    {
        let mut result = result;
        if let Some(computer) = crate::computer::capabilities().as_object() {
            result.as_object_mut().unwrap().extend(computer.clone());
        }
        result
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    result
}

pub fn current_identity() -> String {
    #[cfg(windows)]
    {
        windows::current_user_sid()
            .map(|sid| format!("sid:{sid}"))
            .unwrap_or_else(|_| "unavailable".into())
    }
    #[cfg(not(windows))]
    {
        format!("uid:{}", current_uid())
    }
}

pub fn identity() -> Value {
    #[cfg(windows)]
    {
        json!({"sid": windows::current_user_sid().ok(), "mode": "current_user"})
    }
    #[cfg(not(windows))]
    {
        json!({"uid": current_uid(), "mode": "current_user"})
    }
}

impl Status {
    pub fn new(executor_id: &str, shell: &Path) -> Self {
        let pid = std::process::id();
        Self {
            executor_id: executor_id.into(),
            pid,
            process_start: process_start(pid).unwrap_or_default(),
            uid: current_uid(),
            #[cfg(windows)]
            user_sid: windows::current_user_sid().unwrap_or_default(),
            state: "connecting".into(),
            generation: None,
            store_id: None,
            expires_at_ms: None,
            updated_at_ms: node_executor::now_ms(),
            capabilities: capabilities(shell),
        }
    }

    pub fn write(&mut self, dir: &Path) -> Result<(), String> {
        self.updated_at_ms = node_executor::now_ms();
        let temp = dir.join(format!(".status-{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| "cannot encode local status")?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options
            .open(&temp)
            .map_err(|_| "cannot write local status")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "cannot persist local status")?;
        fs::rename(temp, dir.join("status.json")).map_err(|_| "cannot publish local status".into())
    }

    pub fn running(&self) -> bool {
        #[cfg(windows)]
        if self.user_sid.is_empty()
            || windows::current_user_sid().ok().as_deref() != Some(self.user_sid.as_str())
        {
            return false;
        }
        self.uid == current_uid()
            && !self.process_start.is_empty()
            && process_start(self.pid).as_deref() == Some(self.process_start.as_str())
    }
}

pub fn statuses(base: &Path, executor_id: Option<&str>) -> Result<Vec<Status>, String> {
    let dirs = if let Some(id) = executor_id {
        if !valid_executor_id(id) {
            return Err("invalid executor_id".into());
        }
        vec![base.join(id)]
    } else {
        match fs::read_dir(base) {
            Ok(entries) => entries.filter_map(Result::ok).map(|e| e.path()).collect(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err("cannot inspect daemon state".into()),
        }
    };
    let mut output = Vec::new();
    for dir in dirs {
        #[cfg(windows)]
        let content = windows::read_private_file(&dir.join("status.json"), 1024 * 1024);
        #[cfg(not(windows))]
        let content = fs::read(dir.join("status.json"));
        if let Ok(content) = content {
            if let Ok(mut state) = serde_json::from_slice::<Status>(&content) {
                if !state.running() {
                    state.state = "offline".into();
                }
                output.push(state);
            }
        }
    }
    Ok(output)
}

#[cfg(unix)]
pub async fn stop(base: &Path, executor_id: Option<&str>) -> Result<(), String> {
    let active: Vec<_> = statuses(base, executor_id)?
        .into_iter()
        .filter(Status::running)
        .collect();
    if active.len() > 1 {
        return Err("multiple daemons are running; select --executor".into());
    }
    for state in active {
        if unsafe { libc::kill(state.pid as libc::pid_t, libc::SIGTERM) } != 0 {
            return Err("cannot stop daemon".into());
        }
        for _ in 0..100 {
            if !state.running() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        return Err("daemon has not exited within 10 seconds".into());
    }
    Ok(())
}

#[cfg(windows)]
pub async fn stop(base: &Path, executor_id: Option<&str>) -> Result<(), String> {
    let active: Vec<_> = statuses(base, executor_id)?
        .into_iter()
        .filter(Status::running)
        .collect();
    if active.len() > 1 {
        return Err("multiple daemons are running; select --executor".into());
    }
    for state in active {
        windows::signal(&state, "stop")?;
        for _ in 0..100 {
            if !state.running() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        return Err("daemon has not exited within 10 seconds".into());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub async fn stop(_: &Path, _: Option<&str>) -> Result<(), String> {
    Err("local stop is not implemented on this platform".into())
}

pub fn set_computer_paused(
    base: &Path,
    executor_id: Option<&str>,
    paused: bool,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", windows))]
    {
        let active: Vec<_> = statuses(base, executor_id)?
            .into_iter()
            .filter(Status::running)
            .collect();
        let state = match active.as_slice() {
            [state] => state,
            [] => return Err("no local daemon is running".into()),
            _ => return Err("multiple daemons are running; select --executor".into()),
        };
        if !state.running() {
            return Err("daemon exited before the control request".into());
        }
        #[cfg(windows)]
        return windows::signal(state, if paused { "pause" } else { "resume" });
        #[cfg(target_os = "macos")]
        {
            let signal = if paused { libc::SIGUSR1 } else { libc::SIGUSR2 };
            if unsafe { libc::kill(state.pid as libc::pid_t, signal) } != 0 {
                return Err("cannot change desktop input state".into());
            }
            Ok(())
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (base, executor_id, paused);
        Err("desktop pause/resume is only supported on Windows and macOS".into())
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn local_status_is_private_and_detects_stale_pid() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        private_dir(temp.path()).unwrap();
        let mut state = Status::new("executor-test", Path::new("/bin/bash"));
        assert!(state.running());
        state.write(temp.path()).unwrap();
        assert_eq!(
            fs::metadata(temp.path().join("status.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        state.process_start = "different-process".into();
        assert!(!state.running());
        assert_eq!(state.capabilities["command"]["supported"], true);
        assert_eq!(state.capabilities["file"]["ready"], true);
        #[cfg(target_os = "linux")]
        assert_eq!(state.capabilities["ui"]["state"], "unsupported");
    }

    #[test]
    fn process_identity_rejects_missing_and_reaped_processes() {
        assert!(process_start(0).is_none());
        assert!(process_start(u32::MAX).is_none());
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let identity = process_start(child.id()).unwrap();
        assert_eq!(
            process_start(child.id()).as_deref(),
            Some(identity.as_str())
        );
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(process_start(child.id()).is_none());
    }

    #[test]
    fn command_readiness_requires_an_executable_file() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let shell = temp.path().join("shell");
        fs::write(&shell, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(capabilities(&shell)["command"]["ready"], false);
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(capabilities(&shell)["command"]["ready"], true);
    }
}
