#![allow(dead_code)]

use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::storage::{
    LogQueryRequest, LogStorage, LogStorageRef, LogStorageType, SqlitePartitionedConfig,
    create_log_storage_with_dir,
};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, Instant};

pub fn new_temp_root(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "buckyos/slog_process_e2e/{}_{}_{}",
        prefix,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../src/kernel/slog_daemon
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn target_debug_bin(bin_name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(bin_name)
}

fn cargo_bin() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn env_flag_true(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn update_latest_mtime(path: &Path, latest: &mut SystemTime) {
    if let Some(mtime) = file_mtime(path) {
        if mtime > *latest {
            *latest = mtime;
        }
    }
}

fn scan_dir_latest_mtime(dir: &Path, latest: &mut SystemTime) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("failed to read dir {} for mtime scan: {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read dir entry: {}", e))?;
        let path = entry.path();
        if path.is_dir() {
            scan_dir_latest_mtime(&path, latest)?;
        } else {
            update_latest_mtime(&path, latest);
        }
    }
    Ok(())
}

fn latest_source_mtime() -> Result<SystemTime, String> {
    let ws = workspace_root();
    let mut latest = UNIX_EPOCH;

    // Workspace-level change points.
    update_latest_mtime(&ws.join("Cargo.toml"), &mut latest);
    update_latest_mtime(&ws.join("Cargo.lock"), &mut latest);

    // Crates participating in the process e2e flow.
    let crate_paths = [
        ws.join("frame/slog_server/Cargo.toml"),
        ws.join("frame/slog_server/src"),
        ws.join("kernel/slog_daemon/Cargo.toml"),
        ws.join("kernel/slog_daemon/src"),
        ws.join("kernel/slog/Cargo.toml"),
        ws.join("kernel/slog/src"),
    ];

    for path in crate_paths {
        if path.is_dir() {
            scan_dir_latest_mtime(&path, &mut latest)?;
        } else {
            update_latest_mtime(&path, &mut latest);
        }
    }

    Ok(latest)
}

fn e2e_bins_up_to_date() -> Result<bool, String> {
    let server_bin = target_debug_bin("slog_server");
    let daemon_bin = target_debug_bin("slog_daemon");
    if !server_bin.exists() || !daemon_bin.exists() {
        return Ok(false);
    }

    let server_mtime = file_mtime(&server_bin).unwrap_or(UNIX_EPOCH);
    let daemon_mtime = file_mtime(&daemon_bin).unwrap_or(UNIX_EPOCH);
    let oldest_bin_mtime = std::cmp::min(server_mtime, daemon_mtime);

    let latest_src = latest_source_mtime()?;
    Ok(oldest_bin_mtime >= latest_src)
}

pub fn build_binaries_for_e2e() -> Result<(), String> {
    static BUILD_ONCE: OnceLock<Result<(), String>> = OnceLock::new();
    BUILD_ONCE
        .get_or_init(|| {
            if env_flag_true("SLOG_E2E_SKIP_BUILD") {
                return Ok(());
            }

            let force_build = env_flag_true("SLOG_E2E_FORCE_BUILD");
            if !force_build && e2e_bins_up_to_date()? {
                return Ok(());
            }

            let status = Command::new(cargo_bin())
                .arg("build")
                .arg("-p")
                .arg("slog_server")
                .arg("-p")
                .arg("slog_daemon")
                .current_dir(workspace_root())
                .status()
                .map_err(|e| format!("failed to run cargo build for e2e binaries: {}", e))?;

            if status.success() {
                Ok(())
            } else {
                Err(format!("cargo build failed with status: {}", status))
            }
        })
        .clone()
}

pub fn allocate_bind_addr() -> Result<String, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("failed to bind loopback listener: {}", e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("failed to read local addr: {}", e))?;
    Ok(format!("127.0.0.1:{}", addr.port()))
}

pub fn open_process_e2e_storage(storage_dir: &Path) -> Result<LogStorageRef, String> {
    create_log_storage_with_dir(
        LogStorageType::SqlitePartitioned(SqlitePartitionedConfig::default()),
        storage_dir,
    )
}

pub fn make_record(service: &str, time: u64, content: &str) -> SystemLogRecord {
    SystemLogRecord {
        level: LogLevel::Info,
        target: service.to_string(),
        time,
        file: Some("process_e2e.rs".to_string()),
        line: Some(1),
        content: content.to_string(),
    }
}

pub fn prepare_service_logs(
    log_root: &Path,
    service: &str,
    records: &[SystemLogRecord],
) -> Result<(), String> {
    let service_dir = log_root.join(service);
    std::fs::create_dir_all(&service_dir).map_err(|e| {
        format!(
            "failed to create service log dir {}: {}",
            service_dir.display(),
            e
        )
    })?;

    let meta = LogMeta::open(&service_dir)?;
    let file_name = format!("{}.1.log", service);
    meta.append_new_file(&file_name)
        .map_err(|e| format!("append_new_file failed: {}", e))?;

    let mut content = String::new();
    for record in records {
        content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }

    let log_file = service_dir.join(&file_name);
    std::fs::write(&log_file, &content)
        .map_err(|e| format!("failed to write {}: {}", log_file.display(), e))?;
    meta.update_current_write_index(content.len() as u64)
        .map_err(|e| format!("update_current_write_index failed: {}", e))?;
    Ok(())
}

pub fn append_service_logs(
    log_root: &Path,
    service: &str,
    records: &[SystemLogRecord],
) -> Result<(), String> {
    let service_dir = log_root.join(service);
    let meta = LogMeta::open(&service_dir)?;
    let write_file = meta
        .get_active_write_file()
        .map_err(|e| format!("get_active_write_file failed: {}", e))?
        .ok_or_else(|| "no active write file".to_string())?;

    let mut content = String::new();
    for record in records {
        content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }

    let log_file = service_dir.join(write_file.name);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_file)
        .map_err(|e| format!("failed to open {} for append: {}", log_file.display(), e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("failed to append {}: {}", log_file.display(), e))?;
    meta.increase_current_write_index(content.len() as i64)
        .map_err(|e| format!("increase_current_write_index failed: {}", e))?;

    Ok(())
}

pub struct ChildGuard {
    pub child: Child,
    name: String,
    stderr_path: Option<PathBuf>,
}

impl ChildGuard {
    fn new(child: Child, name: &str, stderr_path: Option<PathBuf>) -> Self {
        Self {
            child,
            name: name.to_string(),
            stderr_path,
        }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn read_stderr_tail(&self, max_bytes: usize) -> String {
        let Some(path) = &self.stderr_path else {
            return "stderr capture unavailable".to_string();
        };

        match std::fs::read(path) {
            Ok(data) => {
                if data.is_empty() {
                    return "stderr empty".to_string();
                }
                let start = data.len().saturating_sub(max_bytes);
                String::from_utf8_lossy(&data[start..]).into_owned()
            }
            Err(e) => format!("failed to read stderr log {}: {}", path.display(), e),
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        eprintln!("killed lingering child process: {}", self.name);
    }
}

fn new_stderr_capture_file(root: &Path, process_name: &str) -> Result<(Stdio, PathBuf), String> {
    let capture_dir = root.join("process_stderr");
    std::fs::create_dir_all(&capture_dir).map_err(|e| {
        format!(
            "failed to create stderr capture dir {}: {}",
            capture_dir.display(),
            e
        )
    })?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("failed to get system time for stderr capture: {}", e))?
        .as_nanos();

    let capture_path = capture_dir.join(format!(
        "{}_{}_{}.stderr.log",
        process_name,
        std::process::id(),
        nanos
    ));

    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&capture_path)
        .map_err(|e| {
            format!(
                "failed to open stderr capture file {}: {}",
                capture_path.display(),
                e
            )
        })?;

    Ok((Stdio::from(file), capture_path))
}

pub fn spawn_server_process(bind_addr: &str, storage_dir: &Path) -> Result<ChildGuard, String> {
    let bin_path = target_debug_bin("slog_server");
    if !bin_path.exists() {
        return Err(format!(
            "slog_server binary not found: {}",
            bin_path.display()
        ));
    }

    let buckyos_root = storage_dir.parent().unwrap_or(storage_dir);
    let (stderr_stdio, stderr_path) = new_stderr_capture_file(buckyos_root, "slog_server")?;

    let child = Command::new(&bin_path)
        .env("SLOG_SERVER_BIND", bind_addr)
        .env("SLOG_STORAGE_DIR", storage_dir)
        .env("BUCKYOS_ROOT", buckyos_root)
        .stdout(Stdio::null())
        .stderr(stderr_stdio)
        .spawn()
        .map_err(|e| format!("failed to spawn slog_server process: {}", e))?;

    Ok(ChildGuard::new(child, "slog_server", Some(stderr_path)))
}

pub fn spawn_daemon_process(
    node: &str,
    endpoint: &str,
    log_root: &Path,
    timeout_secs: u64,
) -> Result<ChildGuard, String> {
    spawn_daemon_process_with_concurrency(node, endpoint, log_root, timeout_secs, None)
}

pub fn spawn_daemon_process_with_concurrency(
    node: &str,
    endpoint: &str,
    log_root: &Path,
    timeout_secs: u64,
    global_concurrency: Option<usize>,
) -> Result<ChildGuard, String> {
    let bin_path = target_debug_bin("slog_daemon");
    if !bin_path.exists() {
        return Err(format!(
            "slog_daemon binary not found: {}",
            bin_path.display()
        ));
    }

    let buckyos_root = log_root.parent().unwrap_or(log_root);
    let (stderr_stdio, stderr_path) = new_stderr_capture_file(buckyos_root, "slog_daemon")?;

    let mut cmd = Command::new(&bin_path);
    cmd.env("SLOG_NODE_ID", node)
        .env("SLOG_SERVER_ENDPOINT", endpoint)
        .env("SLOG_LOG_DIR", log_root)
        .env("SLOG_UPLOAD_TIMEOUT_SECS", timeout_secs.to_string())
        .env("BUCKYOS_ROOT", buckyos_root)
        .stdout(Stdio::null())
        .stderr(stderr_stdio);
    if let Some(v) = global_concurrency
        && v > 0
    {
        cmd.env("SLOG_UPLOAD_GLOBAL_CONCURRENCY", v.to_string());
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn slog_daemon process: {}", e))?;

    Ok(ChildGuard::new(child, "slog_daemon", Some(stderr_path)))
}

pub async fn wait_for_tcp_ready(addr: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => return Ok(()),
            Err(_) => {
                if Instant::now() >= deadline {
                    return Err(format!("timeout waiting TCP ready at {}", addr));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

pub async fn wait_for_tcp_ready_or_process_exit(
    addr: &str,
    process: &mut ChildGuard,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = process
            .child
            .try_wait()
            .map_err(|e| format!("failed to poll child process status: {}", e))?
        {
            let stderr_tail = process.read_stderr_tail(8192);
            return Err(format!(
                "child process exited before TCP ready at {}: status={}, stderr_tail={}",
                addr, status, stderr_tail
            ));
        }

        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => return Ok(()),
            Err(_) => {
                if Instant::now() >= deadline {
                    let stderr_tail = process.read_stderr_tail(8192);
                    return Err(format!(
                        "timeout waiting TCP ready at {}, stderr_tail={}",
                        addr, stderr_tail
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

pub async fn wait_for_tcp_not_ready(addr: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => {
                if Instant::now() >= deadline {
                    return Err(format!("timeout waiting TCP down at {}", addr));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(_) => return Ok(()),
        }
    }
}

pub fn send_sigint(process: &ChildGuard) -> Result<(), String> {
    let pid = process.pid().to_string();
    let status = Command::new("kill")
        .arg("-INT")
        .arg(&pid)
        .status()
        .map_err(|e| format!("failed to send SIGINT to pid {}: {}", pid, e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "kill -INT failed for pid {} with status {}",
            pid, status
        ))
    }
}

pub async fn wait_for_process_exit(
    process: &mut ChildGuard,
    timeout: Duration,
) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = process
            .child
            .try_wait()
            .map_err(|e| format!("failed to poll child process status: {}", e))?
        {
            return Ok(status);
        }

        if Instant::now() >= deadline {
            let stderr_tail = process.read_stderr_tail(8192);
            return Err(format!(
                "timeout waiting process '{}' exit, stderr_tail={}",
                process.name, stderr_tail
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn wait_for_uploaded_count(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let result = storage
            .query_logs(LogQueryRequest {
                node: Some(node.to_string()),
                service: Some(service.to_string()),
                level: None,
                start_time: None,
                end_time: None,
                limit: Some(1000),
            })
            .await?;
        let count: usize = result.iter().map(|r| r.logs.len()).sum();
        if count >= expected_count {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting uploaded count for node={}, service={}, expected={}, got={}",
                node, service, expected_count, count
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub async fn query_uploaded_count(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
) -> Result<usize, String> {
    let result = storage
        .query_logs(LogQueryRequest {
            node: Some(node.to_string()),
            service: Some(service.to_string()),
            level: None,
            start_time: None,
            end_time: None,
            limit: Some(5000),
        })
        .await?;
    Ok(result.iter().map(|r| r.logs.len()).sum())
}

pub async fn query_uploaded_contents(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
) -> Result<Vec<String>, String> {
    let result = storage
        .query_logs(LogQueryRequest {
            node: Some(node.to_string()),
            service: Some(service.to_string()),
            level: None,
            start_time: None,
            end_time: None,
            limit: Some(5000),
        })
        .await?;

    let mut contents = Vec::new();
    for row in result {
        for log in row.logs {
            contents.push(log.content);
        }
    }
    Ok(contents)
}

pub async fn query_uploaded_counts_by_service(
    storage: &dyn LogStorage,
    node: &str,
) -> Result<HashMap<String, usize>, String> {
    let result = storage
        .query_logs(LogQueryRequest {
            node: Some(node.to_string()),
            service: None,
            level: None,
            start_time: None,
            end_time: None,
            limit: None,
        })
        .await?;

    let mut counts = HashMap::new();
    for row in result {
        *counts.entry(row.service).or_insert(0) += row.logs.len();
    }
    Ok(counts)
}
