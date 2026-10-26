//! App processes this node runs natively, without a container: the zone
//! owner's Agent Service on its host runtime and, on BuckyOS Desktop Lite,
//! the product's preinstalled script apps (`etc/product.json`).
//!
//! A process is remembered by a record under `$BUCKYOS_ROOT/local/app_native/`
//! (pid + start time against pid reuse, what deployment it runs, its ports),
//! so a restarted node_daemon finds it again instead of starting a second
//! one. Ready means its service ports accept connections, not that the pid
//! lives. Start failures and crashes back off before the next start and stay
//! visible as the instance's deployment error until a start succeeds.

use crate::run_item::{ControlRuntItemErrors, Result};
use buckyos_api::DeploymentError;
use buckyos_kit::buckyos_get_unix_timestamp;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Directory (under `$BUCKYOS_ROOT/local`) of the records.
pub(crate) const NATIVE_APP_STATE_DIR: &str = "app_native";
/// A started process gets this long to open its ports before it is reported
/// as not listening.
const READY_GRACE_SECS: u64 = 120;
const BACKOFF_BASE_SECS: u64 = 5;
const BACKOFF_MAX_SECS: u64 = 300;
/// Logs rotate to `<name>.1` beyond this size when a process starts.
const LOG_ROTATE_BYTES: u64 = 16 * 1024 * 1024;
const LOG_TAIL_BYTES: u64 = 2048;
/// Bounded graceful stop before the process tree is killed.
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(10);

/// Stable codes of native deployment errors.
pub(crate) const ERR_PORT_CONFLICT: &str = "port_conflict";
pub(crate) const ERR_PORT_CONFIG: &str = "port_config_missing";
pub(crate) const ERR_INTERPRETER: &str = "interpreter_missing";
pub(crate) const ERR_ENTRY: &str = "entry_missing";
pub(crate) const ERR_SPAWN: &str = "spawn_failed";
pub(crate) const ERR_EXITED: &str = "exited";
pub(crate) const ERR_NOT_LISTENING: &str = "not_listening";
pub(crate) const ERR_NOT_ADMITTED: &str = "not_admitted";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NativeProcessRecord {
    pub pid: i32,
    pub start_time: u64,
    /// What it runs (`<app_doc_object_id>#<spec_generation>`): another
    /// deployment means the process is stopped and started again.
    pub deployment: String,
    pub ports: BTreeMap<String, u16>,
    pub started_at: u64,
    pub log: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NativeFailure {
    pub consecutive: u32,
    pub next_attempt_at: u64,
    pub error: DeploymentError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NativeReadiness {
    /// Every service port accepts connections.
    Ready,
    /// Still within the start grace.
    Starting,
    /// Past the grace and a port does not answer.
    NotListening(DeploymentError),
}

pub(crate) struct NativeProcess {
    state_dir: PathBuf,
    key: String,
}

impl NativeProcess {
    pub(crate) fn new(state_dir: PathBuf, key: impl Into<String>) -> Self {
        Self {
            state_dir,
            key: key.into(),
        }
    }

    fn record_path(&self) -> PathBuf {
        self.state_dir.join(format!("{}.json", self.key))
    }

    fn failure_path(&self) -> PathBuf {
        self.state_dir.join(format!("{}.failure.json", self.key))
    }

    /// The recorded process, whether or not it still runs.
    pub(crate) fn record(&self) -> Option<NativeProcessRecord> {
        let raw = fs::read_to_string(self.record_path()).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// The recorded process while it still runs.
    pub(crate) fn running(&self) -> Option<NativeProcessRecord> {
        let record = self.record()?;
        crate::service_pkg::process_identity_is_current(&crate::service_pkg::ProcessIdentity {
            pid: record.pid,
            start_time: record.start_time,
        })
        .then_some(record)
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        fs::create_dir_all(&self.state_dir).map_err(|error| {
            exec_error("start", format!("{}: {error}", self.state_dir.display()))
        })?;
        let body = serde_json::to_vec(value).unwrap_or_default();
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, body)
            .and_then(|_| fs::rename(&tmp, path))
            .map_err(|error| exec_error("start", format!("{}: {error}", path.display())))
    }

    /// Starts the process detached (its own session / process group), with
    /// stdout and stderr appended to `log`, and records it.
    pub(crate) fn spawn(
        &self,
        program: &Path,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: &Path,
        log: &Path,
        deployment: &str,
        ports: BTreeMap<String, u16>,
    ) -> Result<NativeProcessRecord> {
        rotate_log(log);
        let pid = crate::service_pkg::spawn_detached_logged(program, args, Some(env), Some(cwd), log)
            .map_err(|error| exec_error("start", error.to_string()))?;
        let identity = crate::service_pkg::read_process_identity(pid as i32).ok_or_else(|| {
            exec_error(
                "start",
                format!("{} exited right after it started (pid {pid})", program.display()),
            )
        })?;
        let record = NativeProcessRecord {
            pid: identity.pid,
            start_time: identity.start_time,
            deployment: deployment.to_string(),
            ports,
            started_at: buckyos_get_unix_timestamp(),
            log: log.to_path_buf(),
        };
        self.write_json(&self.record_path(), &record)?;
        info!(
            "native app {} started: {} {} (pid {pid}, log {})",
            self.key,
            program.display(),
            args.join(" "),
            log.display()
        );
        Ok(record)
    }

    /// Stops the recorded process tree (bounded graceful stop, then kill)
    /// and forgets it. Stopping on purpose is not a failure.
    pub(crate) async fn stop(&self) -> Result<()> {
        if let Some(record) = self.running() {
            crate::service_pkg::stop_process_tree_bounded(record.pid, STOP_GRACE)
                .await
                .map_err(|error| exec_error("stop", error.to_string()))?;
            info!("native app {} stopped (pid {})", self.key, record.pid);
        }
        let _ = fs::remove_file(self.record_path());
        Ok(())
    }

    pub(crate) fn failure(&self) -> Option<NativeFailure> {
        let raw = fs::read_to_string(self.failure_path()).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Seconds left before the next start may be tried, with the failure.
    pub(crate) fn backoff(&self, now: u64) -> Option<(u64, NativeFailure)> {
        let failure = self.failure()?;
        (failure.next_attempt_at > now).then(|| (failure.next_attempt_at - now, failure))
    }

    /// Remembers a failed start or a crash: the next start waits
    /// 5 s, 10 s, 20 s … up to 5 min.
    pub(crate) fn record_failure(&self, code: &str, message: impl Into<String>) -> NativeFailure {
        let consecutive = self.failure().map(|f| f.consecutive).unwrap_or(0) + 1;
        let delay = BACKOFF_BASE_SECS
            .saturating_mul(1u64 << (consecutive - 1).min(10))
            .min(BACKOFF_MAX_SECS);
        let failure = NativeFailure {
            consecutive,
            next_attempt_at: buckyos_get_unix_timestamp() + delay,
            error: DeploymentError {
                code: code.to_string(),
                message: message.into(),
                detail: None,
            },
        };
        warn!(
            "native app {} failed ({} in a row, next start in {delay}s): {} ({})",
            self.key, consecutive, failure.error.message, failure.error.code
        );
        if let Err(error) = self.write_json(&self.failure_path(), &failure) {
            warn!("native app {}: cannot record the failure: {error}", self.key);
        }
        failure
    }

    pub(crate) fn clear_failure(&self) {
        let _ = fs::remove_file(self.failure_path());
    }

    /// A record whose process is gone without `stop`: it exited on its own.
    /// The record is dropped and the exit counts as a failure.
    pub(crate) fn reap_crash(&self) -> Option<NativeFailure> {
        let record = self.record()?;
        if self.running().is_some() {
            return None;
        }
        let _ = fs::remove_file(self.record_path());
        let tail = log_tail(&record.log);
        Some(self.record_failure(
            ERR_EXITED,
            format!(
                "the process (pid {}) exited; last log lines: {}",
                record.pid,
                if tail.is_empty() { "<empty>".to_string() } else { tail }
            ),
        ))
    }

    pub(crate) fn readiness(&self, record: &NativeProcessRecord, now: u64) -> NativeReadiness {
        let silent: Vec<_> = record
            .ports
            .iter()
            .filter(|(_, port)| !port_accepts(**port))
            .collect();
        if silent.is_empty() {
            return NativeReadiness::Ready;
        }
        if now.saturating_sub(record.started_at) < READY_GRACE_SECS {
            return NativeReadiness::Starting;
        }
        let ports = silent
            .iter()
            .map(|(service, port)| format!("{service} port {port}"))
            .collect::<Vec<_>>()
            .join(", ");
        NativeReadiness::NotListening(DeploymentError {
            code: ERR_NOT_LISTENING.to_string(),
            message: format!(
                "pid {} runs but does not listen on {ports} after {READY_GRACE_SECS}s",
                record.pid
            ),
            detail: None,
        })
    }
}

pub(crate) fn exec_error(step: &str, message: impl Into<String>) -> ControlRuntItemErrors {
    ControlRuntItemErrors::ExecuteError(step.to_string(), message.into())
}

/// Whether something listens on `127.0.0.1:port`.
pub(crate) fn port_accepts(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

/// Whether this node can bind `0.0.0.0:port` now, where a native app
/// service listens. A port another program holds is reported, never taken
/// over.
pub(crate) fn port_free(port: u16) -> bool {
    if port_accepts(port) {
        return false;
    }
    TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).is_ok()
}

fn rotate_log(log: &Path) {
    if let Some(parent) = log.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(log).map(|m| m.len() > LOG_ROTATE_BYTES).unwrap_or(false) {
        let mut rotated = log.as_os_str().to_owned();
        rotated.push(".1");
        let _ = fs::rename(log, PathBuf::from(rotated));
    }
}

/// The last lines of a log, on one line.
pub(crate) fn log_tail(log: &Path) -> String {
    let Ok(mut file) = fs::File::open(log) else {
        return String::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL_BYTES)));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn failures_back_off_and_clear() {
        let dir = tempdir().unwrap();
        let process = NativeProcess::new(dir.path().to_path_buf(), "app@alice");
        let now = buckyos_get_unix_timestamp();
        assert!(process.backoff(now).is_none());
        let first = process.record_failure(ERR_PORT_CONFLICT, "www 127.0.0.1:10032 in use");
        assert_eq!(first.consecutive, 1);
        let (wait, failure) = process.backoff(now).unwrap();
        assert!(wait <= BACKOFF_BASE_SECS && wait > 0);
        assert_eq!(failure.error.code, ERR_PORT_CONFLICT);
        let second = process.record_failure(ERR_PORT_CONFLICT, "again");
        assert_eq!(second.consecutive, 2);
        assert!(second.next_attempt_at >= now + 2 * BACKOFF_BASE_SECS);
        for _ in 0..20 {
            process.record_failure(ERR_EXITED, "crash");
        }
        assert!(process.failure().unwrap().next_attempt_at <= buckyos_get_unix_timestamp() + BACKOFF_MAX_SECS);
        process.clear_failure();
        assert!(process.failure().is_none());
    }

    #[test]
    fn a_held_port_is_neither_free_nor_taken_over() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port_accepts(port));
        assert!(!port_free(port));
        drop(listener);
        assert!(port_free(port));
    }

    #[test]
    fn the_log_tail_is_the_last_lines() {
        let dir = tempdir().unwrap();
        let log = dir.path().join("app.log");
        fs::write(&log, "one\n\ntwo\nthree\n").unwrap();
        assert_eq!(log_tail(&log), "one | two | three");
        assert_eq!(log_tail(&dir.path().join("missing.log")), "");
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use tempfile::tempdir;

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    async fn wait_gone(pid: i32) {
        for _ in 0..100 {
            crate::service_pkg::reap_detached_children();
            if crate::service_pkg::read_process_identity(pid).is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("pid {pid} still runs");
    }

    /// Ready means the service port answers, not that the pid lives; a stop
    /// ends the process tree and forgets it.
    #[tokio::test]
    async fn a_process_is_ready_once_its_port_answers_and_stops_with_its_children() {
        let dir = tempdir().unwrap();
        let process = NativeProcess::new(dir.path().to_path_buf(), "app@alice");
        let port = free_port();
        let record = process
            .spawn(
                Path::new("/bin/sh"),
                &["-c".to_string(), "sleep 30 & sleep 30".to_string()],
                &HashMap::new(),
                dir.path(),
                &dir.path().join("logs/native.log"),
                "doc#1",
                BTreeMap::from([("www".to_string(), port)]),
            )
            .unwrap();
        let now = buckyos_get_unix_timestamp();
        assert_eq!(process.readiness(&record, now), NativeReadiness::Starting);
        assert!(matches!(
            process.readiness(&record, now + READY_GRACE_SECS + 1),
            NativeReadiness::NotListening(_)
        ));
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        assert_eq!(process.readiness(&record, now), NativeReadiness::Ready);
        drop(listener);

        let again = NativeProcess::new(dir.path().to_path_buf(), "app@alice");
        assert_eq!(again.running().map(|r| r.pid), Some(record.pid), "found from its record");
        again.stop().await.unwrap();
        wait_gone(record.pid).await;
        assert!(process.running().is_none() && process.record().is_none());
        assert!(process.reap_crash().is_none(), "a stop is not a crash");
    }

    /// A process that exits on its own is a crash: reported with its last log
    /// lines and backed off.
    #[tokio::test]
    async fn an_exit_without_stop_is_a_crash_with_its_log() {
        let dir = tempdir().unwrap();
        let process = NativeProcess::new(dir.path().to_path_buf(), "app@alice");
        let record = process
            .spawn(
                Path::new("/bin/sh"),
                &["-c".to_string(), "sleep 0.3; echo listen failed: address in use; exit 3".to_string()],
                &HashMap::new(),
                dir.path(),
                &dir.path().join("native.log"),
                "doc#1",
                BTreeMap::new(),
            )
            .unwrap();
        wait_gone(record.pid).await;
        let failure = process.reap_crash().unwrap();
        assert_eq!(failure.error.code, ERR_EXITED);
        assert!(failure.error.message.contains("address in use"), "{}", failure.error.message);
        assert!(process.backoff(buckyos_get_unix_timestamp()).is_some());
        assert!(process.reap_crash().is_none(), "reported once");
    }
}
