use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::access::valid_executor_id;

#[derive(Clone, Serialize, Deserialize)]
pub struct Status {
    pub executor_id: String,
    pub pid: u32,
    pub process_start: String,
    pub uid: u32,
    pub state: String,
    pub generation: Option<u64>,
    pub store_id: Option<String>,
    pub expires_at_ms: Option<u64>,
    pub updated_at_ms: u64,
    pub capabilities: Value,
}

pub fn default_data_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        if Path::new(&path).is_absolute() {
            return Ok(PathBuf::from(path).join("buckyos-agent-daemon"));
        }
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share/buckyos-agent-daemon"))
        .ok_or_else(|| "HOME or --data-dir is required".into())
}

pub fn private_dir(path: &Path) -> Result<(), String> {
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

fn process_start(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

pub fn capabilities(shell: &Path) -> Value {
    #[cfg(unix)]
    let executable = {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::CString::new(shell.as_os_str().as_bytes())
            .ok()
            .is_some_and(|path| unsafe { libc::access(path.as_ptr(), libc::X_OK) } == 0)
    };
    #[cfg(not(unix))]
    let executable = false;
    json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "identity_mode": "current_user",
        "os_session": format!("uid:{}", current_uid()),
        "command": {"supported": cfg!(target_os = "linux"), "ready": cfg!(target_os = "linux") && shell.is_file() && executable},
        "file": {"supported": cfg!(target_os = "linux"), "ready": cfg!(target_os = "linux")},
        "ui": {"supported": false, "ready": false, "state": "unsupported"},
        "accessibility": {"supported": false, "ready": false, "state": "unsupported"},
        "capture": {"supported": false, "ready": false, "state": "unsupported"},
        "input": {"supported": false, "ready": false, "state": "unsupported"}
    })
}

impl Status {
    pub fn new(executor_id: &str, shell: &Path) -> Self {
        let pid = std::process::id();
        Self {
            executor_id: executor_id.into(),
            pid,
            process_start: process_start(pid).unwrap_or_default(),
            uid: current_uid(),
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
        if let Ok(content) = fs::read(dir.join("status.json")) {
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

#[cfg(not(unix))]
pub async fn stop(_: &Path, _: Option<&str>) -> Result<(), String> {
    Err("local stop is not implemented on this platform".into())
}

#[cfg(all(test, target_os = "linux"))]
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
        assert_eq!(state.capabilities["ui"]["state"], "unsupported");
    }
}
