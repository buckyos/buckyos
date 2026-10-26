use buckyos_api::{hide_background_child_console, hide_child_console_async, ServiceInstanceState};
use buckyos_kit::*;
use log::*;
use package_lib::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};
use tokio::process::Command;
use tokio::sync::Mutex;

type Result<T> = std::result::Result<T, ServiceControlError>;

pub(crate) fn new_package_env(pkg_env_path: PathBuf) -> PackageEnv {
    let mut pkg_env = PackageEnv::new(pkg_env_path);
    if pkg_env.config.named_store_config_path.is_none() {
        pkg_env.config.named_store_config_path = Some(
            get_buckyos_root_dir()
                .join("storage")
                .join("named_store.json")
                .to_string_lossy()
                .to_string(),
        );
    }
    pkg_env
}

pub(crate) fn new_system_package_env() -> PackageEnv {
    new_package_env(get_buckyos_system_bin_dir())
}

pub struct ServicePkg {
    pub pkg_id: String,
    pub service_identity: String,
    pub pkg_env_path: PathBuf,
    pub current_dir: Option<PathBuf>,
    pub env_vars: HashMap<String, String>,
    pub media_info: Mutex<Option<MediaInfo>>,
    #[cfg(test)]
    before_native_spawn: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

struct NativeServiceSpec {
    executable: PathBuf,
    process_names: Vec<String>,
    status_port: Option<u16>,
    use_op_subcommand: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessIdentity {
    pub(crate) pid: i32,
    pub(crate) start_time: u64,
}

#[derive(Clone, Debug)]
struct ManagedProcess {
    identity: ProcessIdentity,
    executable: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedServiceTarget {
    pkg_id: String,
    executable: String,
}

#[derive(Clone)]
struct ManagedService {
    target: ManagedServiceTarget,
    processes: Vec<ManagedProcess>,
}

#[derive(Clone)]
struct DiscoveredProcess {
    managed: ManagedProcess,
    executable: Option<String>,
    cmd0: Option<String>,
    cmd0_names: Vec<String>,
    names: Vec<String>,
}

type ServiceProcessKey = (PathBuf, String);

static MANAGED_PROCESSES: OnceLock<Mutex<HashMap<ServiceProcessKey, ManagedService>>> =
    OnceLock::new();
static PROCESS_DISCOVERY: OnceLock<Mutex<Option<Arc<Vec<DiscoveredProcess>>>>> = OnceLock::new();
static PROCESS_SCAN_COUNT: AtomicU64 = AtomicU64::new(0);
static PROCESS_PID_CHECK_COUNT: AtomicU64 = AtomicU64::new(0);
static PROCESS_FAST_HIT_COUNT: AtomicU64 = AtomicU64::new(0);

fn managed_processes() -> &'static Mutex<HashMap<ServiceProcessKey, ManagedService>> {
    MANAGED_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn process_discovery() -> &'static Mutex<Option<Arc<Vec<DiscoveredProcess>>>> {
    PROCESS_DISCOVERY.get_or_init(|| Mutex::new(None))
}

pub async fn begin_service_process_round(service_names: &HashSet<String>) {
    managed_processes()
        .lock()
        .await
        .retain(|(_, name), _| service_names.contains(name));
    *process_discovery().lock().await = None;
}

pub fn service_process_check_counts() -> (u64, u64, u64) {
    (
        PROCESS_SCAN_COUNT.load(Ordering::Relaxed),
        PROCESS_PID_CHECK_COUNT.load(Ordering::Relaxed),
        PROCESS_FAST_HIT_COUNT.load(Ordering::Relaxed),
    )
}

struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Deserialize)]
struct KernelPkgConfig {
    service_name: Option<String>,
}

impl Default for ServicePkg {
    fn default() -> Self {
        Self {
            pkg_id: "".to_string(),
            service_identity: String::new(),
            pkg_env_path: PathBuf::from(""),
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: Mutex::new(None),
            #[cfg(test)]
            before_native_spawn: Mutex::new(None),
        }
    }
}
impl ServicePkg {
    pub fn new(pkg_id: String, env_path: PathBuf) -> Self {
        Self {
            service_identity: package_unique_name(&pkg_id),
            pkg_id,
            pkg_env_path: env_path,
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: Mutex::new(None),
            #[cfg(test)]
            before_native_spawn: Mutex::new(None),
        }
    }

    pub async fn try_load(&self) -> bool {
        let mut media_info = self.media_info.lock().await;
        if media_info.is_none() {
            let pkg_env = new_package_env(self.pkg_env_path.clone());
            let new_media_info = pkg_env.load(&self.pkg_id).await;
            if new_media_info.is_ok() {
                debug!("load service pkg {} success", self.pkg_id);
                let new_media_info = new_media_info.unwrap();
                *media_info = Some(new_media_info);
                return true;
            }
        }
        false
    }

    pub fn set_context(
        &mut self,
        current_dir: Option<&PathBuf>,
        env_vars: Option<&HashMap<String, String>>,
    ) {
        if let Some(current_dir) = current_dir {
            self.current_dir = Some(current_dir.clone());
        }
        if let Some(env_vars) = env_vars {
            self.env_vars = env_vars.clone();
        }
    }

    pub async fn execute_operation(
        &self,
        op_name: &str,
        params: Option<&Vec<String>>,
    ) -> Result<i32> {
        self.execute_operation_with_env(op_name, params, Some(&self.env_vars))
            .await
    }

    pub async fn execute_operation_with_env(
        &self,
        op_name: &str,
        params: Option<&Vec<String>>,
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<i32> {
        let media_root = {
            let media_info = self.media_info.lock().await;
            let media_info = media_info.as_ref();
            if media_info.is_none() {
                return Err(ServiceControlError::PkgNotLoaded);
            }
            media_info.unwrap().full_path.clone()
        };
        if should_prefer_native_operation(self.pkg_id.as_str(), media_root.as_path(), op_name) {
            return self
                .execute_native_operation(media_root.as_path(), op_name, params, env_vars)
                .await;
        }

        let op_file = media_root.join(op_name);
        if !op_file.exists() {
            return self
                .execute_native_operation(media_root.as_path(), op_name, params, env_vars)
                .await;
        }

        let (result, output) = execute(&op_file, 1200, params, self.current_dir.as_ref(), env_vars)
            .await
            .map_err(|e| {
                error!("# execute {} failed! {}", op_file.display(), e);
                return ServiceControlError::ReasonError(e.to_string());
            })?;

        if result == 0 && matches!(op_name, "start" | "stop") {
            self.forget_managed_processes().await;
            *process_discovery().lock().await = None;
        }

        let params_str = params.map(|p| p.join(" ")).unwrap_or_default();
        if result == 0 {
            info!(
                "# run {} {} => {} \n\t {}",
                op_file.display(),
                params_str,
                result,
                String::from_utf8_lossy(&output)
            );
        } else {
            info!(
                "# run {} {} => {} \n\t {}",
                op_file.display(),
                params_str,
                result,
                String::from_utf8_lossy(&output)
            );
        }
        Ok(result)
    }

    pub async fn start(&self, params: Option<&Vec<String>>) -> Result<i32> {
        self.try_load().await;
        let result = self.execute_operation("start", params).await?;
        Ok(result)
    }

    pub async fn stop(&self, params: Option<&Vec<String>>) -> Result<i32> {
        self.try_load().await;
        let result = self.execute_operation("stop", params).await?;
        Ok(result)
    }

    pub async fn status(&self, params: Option<&Vec<String>>) -> Result<ServiceInstanceState> {
        self.status_with_env(params, Some(&self.env_vars)).await
    }

    pub async fn status_with_env(
        &self,
        params: Option<&Vec<String>>,
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<ServiceInstanceState> {
        let pkg_env = new_package_env(self.pkg_env_path.clone());
        let media_info = pkg_env.load(&self.pkg_id).await;
        if media_info.is_err() {
            self.forget_managed_processes().await;
            info!("pkg {} not exist", self.pkg_id);
            return Ok(ServiceInstanceState::NotExist);
        }
        let media_info = media_info.unwrap();
        let mut media_info_lock = self.media_info.lock().await;
        *media_info_lock = Some(media_info);
        drop(media_info_lock);
        let result = self
            .execute_operation_with_env("status", params, env_vars)
            .await?;
        match result {
            0 => Ok(ServiceInstanceState::Started),
            255 => Ok(ServiceInstanceState::NotExist),
            254 => Ok(ServiceInstanceState::Deploying),
            _ => Ok(ServiceInstanceState::Stopped),
        }
    }

    async fn execute_native_operation(
        &self,
        media_root: &Path,
        op_name: &str,
        params: Option<&Vec<String>>,
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<i32> {
        let spec = self.resolve_native_service_spec(media_root)?;
        info!(
            "service_pkg native fallback: pkg={} op={} executable={}",
            self.pkg_id,
            op_name,
            spec.executable.display()
        );

        match op_name {
            "start" => self.native_start(&spec, media_root, params, env_vars).await,
            "stop" => self.native_stop(&spec, media_root, params, env_vars).await,
            "status" => self.native_status(&spec, media_root, params, env_vars).await,
            _ => Err(ServiceControlError::ReasonError(format!(
                "script {} not found for pkg {} and native fallback only supports start/stop/status",
                op_name, self.pkg_id
            ))),
        }
    }

    fn resolve_native_service_spec(&self, media_root: &Path) -> Result<NativeServiceSpec> {
        let dir_name = media_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let pkg_name = package_unique_name(self.pkg_id.as_str());
        let kernel_pkg_service_name = read_kernel_pkg_service_name(media_root);

        let mut name_candidates = Vec::new();
        push_service_aliases(&mut name_candidates, kernel_pkg_service_name.as_deref());
        push_service_aliases(&mut name_candidates, Some(pkg_name.as_str()));
        push_service_aliases(&mut name_candidates, Some(dir_name.as_str()));

        for candidate in &name_candidates {
            let executable = executable_path(media_root, candidate.as_str());
            if executable.is_file() {
                let process_names = name_aliases(candidate.as_str());
                return Ok(NativeServiceSpec {
                    status_port: status_port_for_service(candidate.as_str()),
                    use_op_subcommand: uses_operation_subcommand(candidate.as_str()),
                    executable,
                    process_names,
                });
            }
        }

        let mut discovered = fs::read_dir(media_root)
            .map_err(|err| {
                ServiceControlError::ReasonError(format!(
                    "read service directory {} failed: {}",
                    media_root.display(),
                    err
                ))
            })?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| looks_like_service_executable(path))
            .collect::<Vec<_>>();
        discovered.sort();
        discovered.dedup();

        if discovered.len() == 1 {
            let executable = discovered.remove(0);
            let stem = executable
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let process_names = name_aliases(stem);
            return Ok(NativeServiceSpec {
                status_port: status_port_for_service(stem),
                use_op_subcommand: uses_operation_subcommand(stem),
                executable,
                process_names,
            });
        }

        Err(ServiceControlError::ReasonError(format!(
            "resolve executable for pkg {} failed under {}",
            self.pkg_id,
            media_root.display()
        )))
    }

    async fn native_start(
        &self,
        spec: &NativeServiceSpec,
        media_root: &Path,
        params: Option<&Vec<String>>,
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<i32> {
        let args = native_args(spec, "start", params);
        let cwd = self.current_dir.as_deref().or(Some(media_root));

        if spec.use_op_subcommand {
            let output = run_command(spec.executable.as_path(), &args, env_vars, cwd).await?;
            log_command_output(spec.executable.as_path(), &args, &output);
            return Ok(exit_code(&output.status));
        }

        if should_run_start_in_foreground(&args) {
            let output = run_command(spec.executable.as_path(), &args, env_vars, cwd).await?;
            log_command_output(spec.executable.as_path(), &args, &output);
            return Ok(exit_code(&output.status));
        }

        if !self.service_processes(spec, true).await?.is_empty() {
            if spec.status_port.map(check_local_port).unwrap_or(true) {
                return Ok(0);
            }
            self.native_stop(spec, media_root, None, env_vars).await?;
            if !self.service_processes(spec, true).await?.is_empty() {
                return Ok(1);
            }
        }
        #[cfg(test)]
        if let Some(hook) = self.before_native_spawn.lock().await.take() {
            hook();
        }
        let pid = spawn_detached(spec.executable.as_path(), &args, env_vars, cwd)?;
        if let Some(identity) = read_process_identity(pid as i32) {
            self.remember_processes(
                spec,
                vec![ManagedProcess {
                    identity,
                    executable: Some(spec.executable.clone()),
                }],
            )
            .await;
        }
        *process_discovery().lock().await = None;
        info!(
            "# run detached {} {} => pid {}",
            spec.executable.display(),
            args.join(" "),
            pid
        );
        Ok(0)
    }

    async fn native_stop(
        &self,
        spec: &NativeServiceSpec,
        media_root: &Path,
        params: Option<&Vec<String>>,
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<i32> {
        if spec.use_op_subcommand {
            let args = native_args(spec, "stop", params);
            let cwd = self.current_dir.as_deref().or(Some(media_root));
            let output = run_command(spec.executable.as_path(), &args, env_vars, cwd).await?;
            log_command_output(spec.executable.as_path(), &args, &output);
            return Ok(exit_code(&output.status));
        }

        let processes = self.service_processes(spec, true).await?;
        if processes.is_empty() {
            info!("service {} is not running", spec.executable.display());
            return Ok(0);
        }

        for process in processes {
            if process_identity_is_current(&process.identity) {
                stop_process_tree(process.identity.pid).await?;
            }
        }
        self.forget_managed_processes().await;
        *process_discovery().lock().await = None;
        Ok(0)
    }

    async fn native_status(
        &self,
        spec: &NativeServiceSpec,
        media_root: &Path,
        params: Option<&Vec<String>>,
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<i32> {
        if spec.use_op_subcommand {
            let args = native_args(spec, "status", params);
            let cwd = self.current_dir.as_deref().or(Some(media_root));
            let output = run_command(spec.executable.as_path(), &args, env_vars, cwd).await?;
            log_command_output(spec.executable.as_path(), &args, &output);
            return Ok(exit_code(&output.status));
        }

        if self.service_processes(spec, false).await?.is_empty() {
            return Ok(1);
        }

        if let Some(port) = spec.status_port {
            if !check_local_port(port) {
                return Ok(1);
            }
        }

        Ok(0)
    }

    fn process_key(&self) -> ServiceProcessKey {
        (self.pkg_env_path.clone(), self.service_identity.clone())
    }

    fn process_target(&self, spec: &NativeServiceSpec) -> ManagedServiceTarget {
        ManagedServiceTarget {
            pkg_id: self.pkg_id.clone(),
            executable: normalize_path_value(&spec.executable),
        }
    }

    async fn forget_managed_processes(&self) {
        managed_processes().lock().await.remove(&self.process_key());
    }

    async fn remember_processes(&self, spec: &NativeServiceSpec, processes: Vec<ManagedProcess>) {
        managed_processes().lock().await.insert(
            self.process_key(),
            ManagedService {
                target: self.process_target(spec),
                processes,
            },
        );
    }

    async fn service_processes(
        &self,
        spec: &NativeServiceSpec,
        fresh_discovery: bool,
    ) -> Result<Vec<ManagedProcess>> {
        let target = self.process_target(spec);
        let known = {
            let mut registry = managed_processes().lock().await;
            match registry.get(&self.process_key()) {
                Some(known) if known.target == target => Some(known.processes.clone()),
                Some(_) => {
                    registry.remove(&self.process_key());
                    None
                }
                None => None,
            }
        };

        if !fresh_discovery {
            if let Some(known) = known.as_ref() {
                if !known.is_empty()
                    && known
                        .iter()
                        .all(|process| process_identity_is_current(&process.identity))
                {
                    PROCESS_FAST_HIT_COUNT.fetch_add(1, Ordering::Relaxed);
                    return Ok(known.clone());
                }
            }
        }

        let mut processes =
            discover_service_processes(spec, &target.executable, fresh_discovery).await?;
        if let Some(known) = known {
            for process in known {
                if !processes
                    .iter()
                    .any(|found| found.identity == process.identity)
                    && process_identity_is_current(&process.identity)
                {
                    processes.push(process);
                }
            }
        }
        processes.sort_unstable_by_key(|process| process.identity.pid);
        if processes.is_empty() {
            self.forget_managed_processes().await;
        } else {
            for process in &processes {
                debug!(
                    "managed service {} pid={} start_time={} executable={:?}",
                    self.service_identity,
                    process.identity.pid,
                    process.identity.start_time,
                    process.executable
                );
            }
            managed_processes().lock().await.insert(
                self.process_key(),
                ManagedService {
                    target,
                    processes: processes.clone(),
                },
            );
        }
        Ok(processes)
    }
}

fn package_unique_name(pkg_id: &str) -> String {
    let base = pkg_id.split('#').next().unwrap_or(pkg_id).trim();
    base.rsplit('.').next().unwrap_or(base).trim().to_string()
}

fn should_prefer_native_operation(pkg_id: &str, media_root: &Path, op_name: &str) -> bool {
    if !matches!(op_name, "start" | "stop" | "status") {
        return false;
    }

    let dir_name = media_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    matches!(
        package_unique_name(pkg_id).as_str(),
        "cyfs-gateway" | "cyfs_gateway"
    ) || matches!(dir_name, "cyfs-gateway" | "cyfs_gateway")
}

fn push_service_aliases(target: &mut Vec<String>, name: Option<&str>) {
    for alias in name_aliases(name.unwrap_or_default()) {
        if !alias.is_empty() && !target.iter().any(|value| value == &alias) {
            target.push(alias);
        }
    }
}

fn name_aliases(name: &str) -> Vec<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut result = HashSet::new();
    result.insert(trimmed.to_string());
    result.insert(trimmed.replace('-', "_"));
    result.insert(trimmed.replace('_', "-"));

    let mut values = result.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn executable_path(media_root: &Path, name: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let candidate = media_root.join(name);
        if candidate.extension().is_some() {
            candidate
        } else {
            media_root.join(format!("{name}.exe"))
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        media_root.join(name)
    }
}

fn read_kernel_pkg_service_name(media_root: &Path) -> Option<String> {
    let config_path = media_root.join("kernel_pkg.toml");
    let raw = fs::read_to_string(config_path).ok()?;
    toml::from_str::<KernelPkgConfig>(&raw)
        .ok()
        .and_then(|config| config.service_name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn uses_operation_subcommand(service_name: &str) -> bool {
    matches!(service_name, "smb-service" | "smb_service")
}

fn status_port_for_service(service_name: &str) -> Option<u16> {
    match service_name {
        "repo-service" | "repo_service" => Some(4000),
        "system-config" | "system_config" => Some(3200),
        _ => None,
    }
}

fn looks_like_service_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };

    if matches!(
        name,
        "start" | "stop" | "status" | "deploy" | "kernel_pkg.toml" | "readme.txt" | "readme.md"
    ) {
        return false;
    }

    let lowercase = name.to_ascii_lowercase();
    !lowercase.ends_with(".py")
        && !lowercase.ends_with(".toml")
        && !lowercase.ends_with(".json")
        && !lowercase.ends_with(".md")
        && !lowercase.ends_with(".txt")
        && !lowercase.ends_with(".html")
        && !lowercase.ends_with(".d.ts")
}

fn native_args(
    spec: &NativeServiceSpec,
    op_name: &str,
    params: Option<&Vec<String>>,
) -> Vec<String> {
    let mut args = Vec::new();
    if spec.use_op_subcommand {
        args.push(op_name.to_string());
    }
    if let Some(params) = params {
        args.extend(params.iter().cloned());
    }
    args
}

fn should_run_start_in_foreground(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--boot" | "--reload" | "reload"))
}

fn normalize_path_value(path: &Path) -> String {
    let normalized = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let raw = normalized.to_string_lossy();
    let trimmed = raw.trim_end_matches(|ch| ch == '/' || ch == '\\');
    if cfg!(target_os = "windows") {
        trimmed.to_ascii_lowercase()
    } else {
        trimmed.to_string()
    }
}

fn process_name_candidates(name: &str) -> Vec<String> {
    let base = Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name);
    name_aliases(base)
}

fn process_matches(
    spec: &NativeServiceSpec,
    executable: &str,
    process: &DiscoveredProcess,
) -> bool {
    if process.executable.as_deref() == Some(executable)
        || process.cmd0.as_deref() == Some(executable)
    {
        return true;
    }
    process
        .cmd0_names
        .iter()
        .chain(&process.names)
        .any(|candidate| spec.process_names.contains(candidate))
}

fn is_active_process_status(status: ProcessStatus) -> bool {
    !matches!(status, ProcessStatus::Zombie | ProcessStatus::Dead)
}

pub(crate) fn read_process_identity(pid: i32) -> Option<ProcessIdentity> {
    if pid <= 0 {
        return None;
    }
    PROCESS_PID_CHECK_COUNT.fetch_add(1, Ordering::Relaxed);
    let mut system = System::new();
    let sys_pid = Pid::from_u32(pid as u32);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sys_pid]),
        ProcessRefreshKind::new(),
    );
    let process = system.process(sys_pid)?;
    is_active_process_status(process.status()).then(|| ProcessIdentity {
        pid,
        start_time: process.start_time(),
    })
}

// An established identity uses only PID and start time; replaced executables may be deleted.
pub(crate) fn process_identity_is_current(identity: &ProcessIdentity) -> bool {
    read_process_identity(identity.pid).as_ref() == Some(identity)
}

fn scan_processes() -> Vec<DiscoveredProcess> {
    let scan_count = PROCESS_SCAN_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let started = std::time::Instant::now();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    let processes = system
        .processes()
        .iter()
        .filter(|(_, process)| {
            process.thread_kind().is_none() && is_active_process_status(process.status())
        })
        .map(|(pid, process)| {
            let executable = process.exe().map(Path::to_path_buf);
            let cmd0 = process.cmd().first();
            DiscoveredProcess {
                managed: ManagedProcess {
                    identity: ProcessIdentity {
                        pid: pid.as_u32() as i32,
                        start_time: process.start_time(),
                    },
                    executable: executable.clone(),
                },
                executable: executable.as_deref().map(normalize_path_value),
                cmd0: cmd0
                    .map(Path::new)
                    .filter(|path| path.is_absolute())
                    .map(normalize_path_value),
                cmd0_names: cmd0
                    .map(|name| process_name_candidates(&name.to_string_lossy()))
                    .unwrap_or_default(),
                names: process_name_candidates(&process.name().to_string_lossy()),
            }
        })
        .collect::<Vec<_>>();
    debug!(
        "service process discovery scan={} candidates={} elapsed_ms={}",
        scan_count,
        processes.len(),
        started.elapsed().as_millis()
    );
    processes
}

async fn discover_service_processes(
    spec: &NativeServiceSpec,
    executable: &str,
    fresh: bool,
) -> Result<Vec<ManagedProcess>> {
    let mut snapshot = process_discovery().lock().await;
    let reused = !fresh && snapshot.is_some();
    if fresh || snapshot.is_none() {
        *snapshot = Some(Arc::new(
            tokio::task::spawn_blocking(scan_processes)
                .await
                .map_err(|error| ServiceControlError::ReasonError(error.to_string()))?,
        ));
    }
    let find_matches = |processes: &[DiscoveredProcess]| {
        processes
            .iter()
            .filter(|process| process_matches(spec, executable, process))
            .filter(|process| process_identity_is_current(&process.managed.identity))
            .map(|process| process.managed.clone())
            .collect::<Vec<_>>()
    };
    let processes = find_matches(snapshot.as_deref().unwrap());
    if !processes.is_empty() || !reused {
        return Ok(processes);
    }
    *snapshot = Some(Arc::new(
        tokio::task::spawn_blocking(scan_processes)
            .await
            .map_err(|error| ServiceControlError::ReasonError(error.to_string()))?,
    ));
    Ok(find_matches(snapshot.as_deref().unwrap()))
}

fn check_local_port(port: u16) -> bool {
    if port == 0 {
        return true;
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
}

pub(crate) async fn stop_process_tree(pid: i32) -> Result<()> {
    if pid <= 0 {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let args = vec![
            "/F".to_string(),
            "/T".to_string(),
            "/PID".to_string(),
            pid.to_string(),
        ];
        let output = run_command(Path::new("taskkill"), &args, None, None).await?;
        if output.status.success() || !is_pid_running(pid) {
            return Ok(());
        }
        return Err(ServiceControlError::ReasonError(format!(
            "taskkill pid {} failed: {}",
            pid,
            format_command_failure(&output)
        )));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let group_args = vec!["-TERM".to_string(), "--".to_string(), format!("-{pid}")];
        let group_output = run_command(Path::new("kill"), &group_args, None, None).await?;
        if group_output.status.success() || !is_pid_running(pid) {
            return Ok(());
        }

        let pid_args = vec!["-TERM".to_string(), pid.to_string()];
        let pid_output = run_command(Path::new("kill"), &pid_args, None, None).await?;
        if pid_output.status.success() || !is_pid_running(pid) {
            return Ok(());
        }

        return Err(ServiceControlError::ReasonError(format!(
            "kill pid {} failed: {}",
            pid,
            format_command_failure(&pid_output)
        )));
    }
}

fn is_pid_running(pid: i32) -> bool {
    read_process_identity(pid).is_some()
}

/// Stops a detached process and its children: asks first (SIGTERM to its
/// process group; `taskkill /T` on Windows), waits up to `grace`, then
/// kills the tree. Returns once the process is gone.
pub(crate) async fn stop_process_tree_bounded(pid: i32, grace: Duration) -> Result<()> {
    let Some(identity) = read_process_identity(pid) else {
        return Ok(());
    };
    let gone = || !process_identity_is_current(&identity);
    #[cfg(target_os = "windows")]
    let (ask, kill) = (
        vec!["/T".to_string(), "/PID".to_string(), pid.to_string()],
        vec!["/F".to_string(), "/T".to_string(), "/PID".to_string(), pid.to_string()],
    );
    #[cfg(not(target_os = "windows"))]
    let (ask, kill) = (
        vec!["-TERM".to_string(), "--".to_string(), format!("-{pid}")],
        vec!["-KILL".to_string(), "--".to_string(), format!("-{pid}")],
    );
    let program = Path::new(if cfg!(target_os = "windows") { "taskkill" } else { "kill" });
    // A windowless Windows process refuses a polite taskkill: kill it now.
    let asked = run_command(program, &ask, None, None)
        .await
        .is_ok_and(|output| output.status.success());
    let deadline = std::time::Instant::now() + if asked { grace } else { Duration::ZERO };
    while std::time::Instant::now() < deadline {
        reap_detached_children();
        if gone() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    warn!("pid {pid} did not stop within {}s, killing its process tree", grace.as_secs());
    let output = run_command(program, &kill, None, None).await?;
    for _ in 0..25 {
        reap_detached_children();
        if gone() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(ServiceControlError::ReasonError(format!(
        "pid {pid} is still running after kill: {}",
        format_command_failure(&output)
    )))
}

async fn run_command(
    program: &Path,
    args: &[String],
    envs: Option<&HashMap<String, String>>,
    cwd: Option<&Path>,
) -> Result<CommandOutput> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdin(Stdio::null());
    if let Some(envs) = envs {
        cmd.envs(envs);
    }
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    hide_child_console_async(&mut cmd);

    let output = cmd.output().await.map_err(|error| {
        ServiceControlError::ReasonError(format!("spawn {} failed: {}", program.display(), error))
    })?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub(crate) fn spawn_detached(
    program: &Path,
    args: &[String],
    envs: Option<&HashMap<String, String>>,
    cwd: Option<&Path>,
) -> Result<u32> {
    spawn_detached_with(program, args, envs, cwd, Stdio::null(), Stdio::null())
}

/// [`spawn_detached`] with stdout and stderr appended to `log`.
pub(crate) fn spawn_detached_logged(
    program: &Path,
    args: &[String],
    envs: Option<&HashMap<String, String>>,
    cwd: Option<&Path>,
    log: &Path,
) -> Result<u32> {
    let open = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .map_err(|error| {
                ServiceControlError::ReasonError(format!("open log {} failed: {error}", log.display()))
            })
    };
    let stdout = open()?;
    let stderr = open()?;
    spawn_detached_with(program, args, envs, cwd, Stdio::from(stdout), Stdio::from(stderr))
}

fn spawn_detached_with(
    program: &Path,
    args: &[String],
    envs: Option<&HashMap<String, String>>,
    cwd: Option<&Path>,
    stdout: Stdio,
    stderr: Stdio,
) -> Result<u32> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.stdin(Stdio::null());
    cmd.stdout(stdout);
    cmd.stderr(stderr);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    if let Some(envs) = envs {
        cmd.envs(envs);
    }

    hide_background_child_console(&mut cmd);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let child = cmd.spawn().map_err(|error| {
        ServiceControlError::ReasonError(format!("spawn {} failed: {}", program.display(), error))
    })?;
    let pid = child.id();
    DETACHED_CHILDREN.lock().unwrap().insert(pid);
    Ok(pid)
}

/// Detached service processes whose `Child` was dropped; nothing else waits
/// for them, so the main loop reaps exactly these pids.
static DETACHED_CHILDREN: std::sync::Mutex<std::collections::BTreeSet<u32>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

#[cfg(unix)]
pub(crate) fn reap_detached_children() -> usize {
    let pids: Vec<u32> = DETACHED_CHILDREN.lock().unwrap().iter().copied().collect();
    let mut reaped = 0usize;
    for pid in pids {
        let mut status = 0i32;
        let rc = loop {
            let rc = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
            if rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break rc;
        };
        if rc == 0 {
            continue;
        }
        DETACHED_CHILDREN.lock().unwrap().remove(&pid);
        if rc > 0 {
            reaped += 1;
            info!("reaped exited child process pid={} status={}", pid, status);
        }
    }
    reaped
}

#[cfg(not(unix))]
pub(crate) fn reap_detached_children() -> usize {
    0
}

fn exit_code(status: &std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn format_command_failure(output: &CommandOutput) -> String {
    let code = output
        .status
        .code()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    format!(
        "code={} stdout=`{}` stderr=`{}`",
        code,
        output.stdout.trim(),
        output.stderr.trim()
    )
}

fn log_command_output(program: &Path, args: &[String], output: &CommandOutput) {
    info!(
        "# run {} {} => {} stdout=`{}` stderr=`{}`",
        program.display(),
        args.join(" "),
        exit_code(&output.status),
        output.stdout.trim(),
        output.stderr.trim()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detached_reaper_does_not_steal_other_children() {
        let pid = spawn_detached(Path::new("/bin/true"), &[], None, None).unwrap();
        let reaper = tokio::spawn(async {
            for _ in 0..300 {
                reap_detached_children();
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });
        for i in 0..60 {
            let status = tokio::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("exit {}", i % 7))
                .status()
                .await
                .expect("child status must not be stolen by the reaper");
            assert_eq!(status.code(), Some(i % 7));
        }
        reaper.await.unwrap();
        for _ in 0..200 {
            reap_detached_children();
            if !DETACHED_CHILDREN.lock().unwrap().contains(&pid) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(!DETACHED_CHILDREN.lock().unwrap().contains(&pid));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("service-pkg-{name}-{nonce}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolves_executable_from_kernel_pkg_service_name() {
        let dir = temp_dir("system-config");
        fs::write(
            dir.join("kernel_pkg.toml"),
            "service_name = \"system_config\"\n",
        )
        .unwrap();
        fs::write(dir.join("system_config"), "").unwrap();

        let pkg = ServicePkg::new(
            "nightly-linux-amd64.system-config".to_string(),
            PathBuf::new(),
        );
        let spec = pkg.resolve_native_service_spec(dir.as_path()).unwrap();

        assert_eq!(spec.executable, dir.join("system_config"));
        assert_eq!(spec.status_port, Some(3200));
        assert!(!spec.use_op_subcommand);
    }

    #[test]
    fn resolves_smb_service_as_subcommand_mode() {
        let dir = temp_dir("smb-service");
        fs::write(
            dir.join("kernel_pkg.toml"),
            "service_name = \"smb_service\"\n",
        )
        .unwrap();
        fs::write(dir.join("smb_service"), "").unwrap();

        let pkg = ServicePkg::new("smb-service".to_string(), PathBuf::new());
        let spec = pkg.resolve_native_service_spec(dir.as_path()).unwrap();

        assert_eq!(spec.executable, dir.join("smb_service"));
        assert!(spec.use_op_subcommand);
        assert_eq!(
            native_args(&spec, "status", None),
            vec!["status".to_string()]
        );
    }

    #[test]
    fn cyfs_gateway_prefers_native_service_control_for_runtime_ops() {
        let dir = temp_dir("cyfs-gateway");

        for op_name in ["start", "stop", "status"] {
            assert!(should_prefer_native_operation(
                "nightly-windows-amd64.cyfs-gateway",
                dir.as_path(),
                op_name
            ));
        }

        assert!(!should_prefer_native_operation(
            "nightly-windows-amd64.cyfs-gateway",
            dir.as_path(),
            "deploy"
        ));
    }

    #[test]
    fn zombie_and_dead_processes_are_not_treated_as_active() {
        assert!(!is_active_process_status(ProcessStatus::Zombie));
        assert!(!is_active_process_status(ProcessStatus::Dead));
        assert!(is_active_process_status(ProcessStatus::Run));
        assert!(is_active_process_status(ProcessStatus::Sleep));
    }

    #[cfg(unix)]
    static PROCESS_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    #[cfg(unix)]
    struct TestService {
        directory: PathBuf,
        pkg: ServicePkg,
        spec: NativeServiceSpec,
        child: Option<std::process::Child>,
    }

    #[cfg(unix)]
    impl TestService {
        fn new(name: &str) -> Self {
            let directory = temp_dir(name);
            let executable_name = directory.file_name().unwrap().to_string_lossy().to_string();
            let executable = directory.join(&executable_name);
            fs::copy("/bin/sleep", &executable).unwrap();
            let pkg = ServicePkg::new(executable_name.clone(), directory.clone());
            let spec = NativeServiceSpec {
                executable,
                process_names: name_aliases(&executable_name),
                status_port: None,
                use_op_subcommand: false,
            };
            Self {
                directory,
                pkg,
                spec,
                child: None,
            }
        }

        fn spawn(&mut self) -> i32 {
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new(&self.spec.executable);
            command.arg("60");
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let child = command.spawn().unwrap();
            let pid = child.id() as i32;
            self.child = Some(child);
            pid
        }

        fn reap(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                child.wait().unwrap();
            }
        }
    }

    #[cfg(unix)]
    impl Drop for TestService {
        fn drop(&mut self) {
            self.reap();
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shares_cold_discovery_and_reuses_pids_across_service_reconstruction() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut first = TestService::new("first");
        let mut second = TestService::new("second");
        let first_pid = first.spawn();
        let second_pid = second.spawn();
        begin_service_process_round(&HashSet::new()).await;
        let scans = service_process_check_counts().0;
        let (first_processes, second_processes) = tokio::join!(
            first.pkg.service_processes(&first.spec, false),
            second.pkg.service_processes(&second.spec, false)
        );
        assert_eq!(first_processes.unwrap()[0].identity.pid, first_pid);
        assert_eq!(second_processes.unwrap()[0].identity.pid, second_pid);
        assert_eq!(service_process_check_counts().0 - scans, 1);

        let reconstructed = ServicePkg::new(first.pkg.pkg_id.clone(), first.directory.clone());
        let retained = HashSet::from([first.pkg.service_identity.clone()]);
        begin_service_process_round(&retained).await;
        let scans = service_process_check_counts().0;
        assert_eq!(
            reconstructed
                .service_processes(&first.spec, false)
                .await
                .unwrap()[0]
                .identity
                .pid,
            first_pid
        );
        assert_eq!(service_process_check_counts().0, scans);
        assert!(!managed_processes()
            .lock()
            .await
            .contains_key(&second.pkg.process_key()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovers_manual_replacement_and_daemon_restart_without_duplicate_start() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("replacement");
        let old_pid = service.spawn();
        begin_service_process_round(&HashSet::new()).await;
        assert_eq!(
            service
                .pkg
                .service_processes(&service.spec, false)
                .await
                .unwrap()[0]
                .identity
                .pid,
            old_pid
        );
        service.reap();
        let new_pid = service.spawn();
        assert_eq!(
            service
                .pkg
                .service_processes(&service.spec, false)
                .await
                .unwrap()[0]
                .identity
                .pid,
            new_pid
        );
        assert_eq!(
            service
                .pkg
                .native_start(
                    &service.spec,
                    &service.directory,
                    Some(&vec!["60".into()]),
                    None
                )
                .await
                .unwrap(),
            0
        );
        assert!(is_pid_running(new_pid));

        begin_service_process_round(&HashSet::new()).await;
        assert_eq!(
            service
                .pkg
                .service_processes(&service.spec, false)
                .await
                .unwrap()[0]
                .identity
                .pid,
            new_pid
        );
        assert_eq!(
            service
                .pkg
                .service_processes(&service.spec, true)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_refreshes_a_previous_negative_discovery() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("negative");
        begin_service_process_round(&HashSet::new()).await;
        assert!(service
            .pkg
            .service_processes(&service.spec, false)
            .await
            .unwrap()
            .is_empty());
        let pid = service.spawn();
        assert_eq!(
            service
                .pkg
                .native_start(
                    &service.spec,
                    &service.directory,
                    Some(&vec!["60".into()]),
                    None
                )
                .await
                .unwrap(),
            0
        );
        let processes = service
            .pkg
            .service_processes(&service.spec, false)
            .await
            .unwrap();
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].identity.pid, pid);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn version_and_path_changes_invalidate_managed_processes() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("target");
        begin_service_process_round(&HashSet::new()).await;
        let current = ManagedProcess {
            identity: read_process_identity(std::process::id() as i32).unwrap(),
            executable: std::env::current_exe().ok(),
        };
        service
            .pkg
            .remember_processes(&service.spec, vec![current.clone()])
            .await;
        assert_eq!(
            service
                .pkg
                .service_processes(&service.spec, false)
                .await
                .unwrap()
                .len(),
            1
        );
        service.pkg.pkg_id.push_str("#2");
        assert!(service
            .pkg
            .service_processes(&service.spec, false)
            .await
            .unwrap()
            .is_empty());

        service
            .pkg
            .remember_processes(&service.spec, vec![current])
            .await;
        service.spec.executable = service.directory.join("other-target");
        assert!(service
            .pkg
            .service_processes(&service.spec, false)
            .await
            .unwrap()
            .is_empty());
        assert!(!managed_processes()
            .lock()
            .await
            .contains_key(&service.pkg.process_key()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn single_pid_checks_reject_changed_start_time_and_exited_processes() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("identity");
        let pid = service.spawn();
        let identity = read_process_identity(pid).unwrap();
        assert!(process_identity_is_current(&identity));
        let changed = ProcessIdentity {
            start_time: identity.start_time + 1,
            ..identity.clone()
        };
        assert!(!process_identity_is_current(&changed));
        service.child.as_mut().unwrap().kill().unwrap();
        for _ in 0..100 {
            if !is_pid_running(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!process_identity_is_current(&identity));
        service.reap();
        assert!(!is_pid_running(pid));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn replacing_a_running_executable_keeps_the_pid_fast_path() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("deleted");
        let pid = service.spawn();
        begin_service_process_round(&HashSet::new()).await;
        service
            .pkg
            .service_processes(&service.spec, false)
            .await
            .unwrap();
        fs::remove_file(&service.spec.executable).unwrap();
        fs::copy("/bin/sleep", &service.spec.executable).unwrap();
        let proc_exe = fs::read_link(format!("/proc/{pid}/exe")).unwrap();
        assert!(proc_exe.to_string_lossy().ends_with(" (deleted)"));
        let scans = service_process_check_counts().0;
        for _ in 0..3 {
            assert_eq!(
                service
                    .pkg
                    .service_processes(&service.spec, false)
                    .await
                    .unwrap()[0]
                    .identity
                    .pid,
                pid
            );
        }
        assert_eq!(service_process_check_counts().0, scans);
    }

    #[cfg(unix)]
    struct DetachedTestProcess(ProcessIdentity);

    #[cfg(unix)]
    impl Drop for DetachedTestProcess {
        fn drop(&mut self) {
            unsafe {
                if process_identity_is_current(&self.0) {
                    libc::kill(self.0.pid, libc::SIGKILL);
                }
                libc::waitpid(self.0.pid, std::ptr::null_mut(), 0);
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_stop_and_status_do_not_restart_a_stopped_process() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("stop");
        let pid = service.spawn();
        begin_service_process_round(&HashSet::new()).await;
        assert_eq!(
            service
                .pkg
                .native_status(&service.spec, &service.directory, None, None)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            service
                .pkg
                .native_stop(&service.spec, &service.directory, None, None)
                .await
                .unwrap(),
            0
        );
        for _ in 0..100 {
            if !is_pid_running(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!is_pid_running(pid));
        assert_eq!(
            service
                .pkg
                .native_status(&service.spec, &service.directory, None, None)
                .await
                .unwrap(),
            1
        );
        assert!(!managed_processes()
            .lock()
            .await
            .contains_key(&service.pkg.process_key()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stopping_a_native_process_group_terminates_its_leader() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("group-stop");
        let pid = service.spawn();
        assert_eq!(unsafe { libc::getpgid(pid) }, pid);
        assert!(is_pid_running(pid));
        stop_process_tree(pid).await.unwrap();
        for _ in 0..100 {
            if !is_pid_running(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!is_pid_running(pid));
        service.reap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_unhealthy_status_port_still_restarts_the_running_process() {
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("unhealthy");
        let port_reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        service.spec.status_port = Some(port_reservation.local_addr().unwrap().port());
        drop(port_reservation);
        let old_pid = service.spawn();
        begin_service_process_round(&HashSet::new()).await;
        assert_eq!(
            service
                .pkg
                .native_status(&service.spec, &service.directory, None, None)
                .await
                .unwrap(),
            1
        );
        for _ in 0..100 {
            if service
                .pkg
                .native_start(
                    &service.spec,
                    &service.directory,
                    Some(&vec!["60".into()]),
                    None,
                )
                .await
                .unwrap()
                == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let processes = service
            .pkg
            .service_processes(&service.spec, false)
            .await
            .unwrap();
        assert_eq!(processes.len(), 1);
        assert_ne!(processes[0].identity.pid, old_pid);
        let replacement = DetachedTestProcess(processes[0].identity.clone());
        assert!(!is_pid_running(old_pid));
        assert!(is_pid_running(replacement.0.pid));
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess fixture for the native discovery/start race test"]
    fn raced_native_service_fixture() {
        let Ok(port) = std::env::var("BUCKYOS_TEST_NATIVE_RACE_PORT") else {
            return;
        };
        let port = port.parse::<u16>().unwrap();
        let attempts =
            PathBuf::from(std::env::var_os("BUCKYOS_TEST_NATIVE_RACE_ATTEMPTS").unwrap());
        fs::write(attempts.join(std::process::id().to_string()), "started").unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        let ready = PathBuf::from(std::env::var_os("BUCKYOS_TEST_NATIVE_RACE_READY").unwrap());
        let release = ready.with_extension("release");
        let worker_release = release.clone();
        let mut worker = Some(std::thread::spawn(move || {
            while !worker_release.exists() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }));
        fs::write(&ready, std::process::id().to_string()).unwrap();
        loop {
            let _ = listener.accept().unwrap();
            if release.exists() {
                if let Some(worker) = worker.take() {
                    worker.join().unwrap();
                    fs::write(ready.with_extension("exited"), "joined").unwrap();
                }
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn adopts_external_process_after_it_wins_the_discovery_spawn_race() {
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let _guard = PROCESS_TEST_LOCK.lock().await;
        let mut service = TestService::new("spawn-race");
        fs::remove_file(&service.spec.executable).unwrap();
        let test_executable = std::env::current_exe().unwrap();
        if fs::hard_link(&test_executable, &service.spec.executable).is_err() {
            fs::copy(&test_executable, &service.spec.executable).unwrap();
        }
        let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        service.spec.status_port = Some(port);
        drop(reservation);
        let attempts = service.directory.join("attempts");
        fs::create_dir(&attempts).unwrap();
        let ready = service.directory.join("ready");
        let envs = HashMap::from([
            ("BUCKYOS_TEST_NATIVE_RACE_PORT".into(), port.to_string()),
            (
                "BUCKYOS_TEST_NATIVE_RACE_ATTEMPTS".into(),
                attempts.to_string_lossy().into_owned(),
            ),
            (
                "BUCKYOS_TEST_NATIVE_RACE_READY".into(),
                ready.to_string_lossy().into_owned(),
            ),
        ]);
        let args = vec![
            "--exact".to_string(),
            "service_pkg::tests::raced_native_service_fixture".to_string(),
            "--ignored".to_string(),
            "--nocapture".to_string(),
        ];
        let external_child = Arc::new(std::sync::Mutex::new(None::<OwnedChild>));
        let hook_child = Arc::clone(&external_child);
        let hook_executable = service.spec.executable.clone();
        let hook_args = args.clone();
        let hook_envs = envs.clone();
        let hook_ready = ready.clone();
        *service.pkg.before_native_spawn.lock().await = Some(Box::new(move || {
            let child = std::process::Command::new(hook_executable)
                .args(hook_args)
                .envs(hook_envs)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id().to_string();
            *hook_child.lock().unwrap() = Some(OwnedChild(child));
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while fs::read_to_string(&hook_ready).ok().as_deref() != Some(pid.as_str()) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "external service did not bind its port"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }));

        begin_service_process_round(&HashSet::new()).await;
        assert_eq!(
            service
                .pkg
                .native_start(&service.spec, &service.directory, Some(&args), Some(&envs))
                .await
                .unwrap(),
            0
        );
        let external_pid = external_child.lock().unwrap().as_ref().unwrap().0.id() as i32;
        assert_eq!(
            fs::read_to_string(&ready).unwrap(),
            external_pid.to_string()
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let duplicate_pid = loop {
            let duplicate = fs::read_dir(&attempts)
                .unwrap()
                .map(|entry| {
                    entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .parse::<i32>()
                        .unwrap()
                })
                .find(|pid| *pid != external_pid);
            if let Some(pid) = duplicate {
                break pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "daemon did not attempt its detached start"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let mut status = 0;
        loop {
            let waited = unsafe { libc::waitpid(duplicate_pid, &mut status, libc::WNOHANG) };
            if waited == duplicate_pid {
                break;
            }
            assert_eq!(waited, 0, "failed to reap duplicate daemon child");
            assert!(
                std::time::Instant::now() < deadline,
                "duplicate service did not exit after bind failure"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(libc::WIFEXITED(status));
        assert_ne!(libc::WEXITSTATUS(status), 0);
        assert!(is_pid_running(external_pid));
        assert!(!is_pid_running(duplicate_pid));
        assert_eq!(
            service
                .pkg
                .native_status(&service.spec, &service.directory, None, None)
                .await
                .unwrap(),
            0
        );
        let processes = service
            .pkg
            .service_processes(&service.spec, false)
            .await
            .unwrap();
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].identity.pid, external_pid);
        let scans = service_process_check_counts().0;
        fs::write(ready.with_extension("release"), "release worker").unwrap();
        drop(TcpStream::connect(("127.0.0.1", port)).unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !ready.with_extension("exited").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "worker thread did not exit"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            service
                .pkg
                .native_status(&service.spec, &service.directory, None, None)
                .await
                .unwrap(),
            0
        );
        assert_eq!(service_process_check_counts().0, scans);
        assert_eq!(
            service
                .pkg
                .native_start(&service.spec, &service.directory, Some(&args), Some(&envs))
                .await
                .unwrap(),
            0
        );
        assert_eq!(fs::read_dir(&attempts).unwrap().count(), 2);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "manual CPU comparison using fourteen service checks"]
    async fn benchmark_fourteen_managed_service_checks() {
        fn cpu_ticks() -> u64 {
            let stat = fs::read_to_string("/proc/self/stat").unwrap();
            let fields = stat
                .rsplit_once(") ")
                .unwrap()
                .1
                .split_whitespace()
                .collect::<Vec<_>>();
            fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
        }
        fn legacy_matches(spec: &NativeServiceSpec, process: &sysinfo::Process) -> bool {
            if process.exe().is_some_and(|path| {
                normalize_path_value(path) == normalize_path_value(&spec.executable)
            }) {
                return true;
            }
            if let Some(cmd0) = process.cmd().first() {
                let path = Path::new(cmd0);
                if path.is_absolute()
                    && normalize_path_value(path) == normalize_path_value(&spec.executable)
                {
                    return true;
                }
                if process_name_candidates(&cmd0.to_string_lossy())
                    .iter()
                    .any(|name| spec.process_names.contains(name))
                {
                    return true;
                }
            }
            process_name_candidates(&process.name().to_string_lossy())
                .iter()
                .any(|name| spec.process_names.contains(name))
        }
        let _guard = PROCESS_TEST_LOCK.lock().await;
        let services = (0..14)
            .map(|index| {
                let mut service = TestService::new(&format!("benchmark-{index}"));
                service.spawn();
                service
            })
            .collect::<Vec<_>>();
        let rounds = 5;
        let before_cpu = cpu_ticks();
        let started = std::time::Instant::now();
        let mut process_count = 0;
        for _ in 0..rounds {
            for service in &services {
                let mut system = System::new_all();
                system.refresh_processes(ProcessesToUpdate::All);
                process_count = system.processes().len();
                let found = system
                    .processes()
                    .iter()
                    .filter(|(_, process)| {
                        is_active_process_status(process.status())
                            && legacy_matches(&service.spec, process)
                    })
                    .map(|(pid, _)| pid.as_u32())
                    .collect::<Vec<_>>();
                assert_eq!(found, vec![service.child.as_ref().unwrap().id()]);
            }
        }
        let legacy_elapsed = started.elapsed();
        let legacy_cpu = cpu_ticks() - before_cpu;
        begin_service_process_round(&HashSet::new()).await;
        for service in &services {
            service
                .pkg
                .service_processes(&service.spec, false)
                .await
                .unwrap();
        }
        let scans = service_process_check_counts().0;
        let before_cpu = cpu_ticks();
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            for service in &services {
                assert_eq!(
                    service
                        .pkg
                        .service_processes(&service.spec, false)
                        .await
                        .unwrap()[0]
                        .identity
                        .pid as u32,
                    service.child.as_ref().unwrap().id()
                );
            }
        }
        let optimized_elapsed = started.elapsed();
        let optimized_cpu = cpu_ticks() - before_cpu;
        let optimized_scans = service_process_check_counts().0 - scans;
        assert_eq!(optimized_scans, 0);
        println!("processes={process_count} rounds={rounds} checks_per_round=14 legacy_wall_ms={:.3} legacy_cpu_ticks={legacy_cpu} legacy_scans={} optimized_wall_ms={:.3} optimized_cpu_ticks={optimized_cpu} optimized_scans={optimized_scans}", legacy_elapsed.as_secs_f64() * 1000.0, rounds * 28, optimized_elapsed.as_secs_f64() * 1000.0);
    }
}
