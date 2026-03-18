use buckyos_api::ServiceInstanceState;
use buckyos_kit::*;
use log::*;
use package_lib::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use sysinfo::{Pid, ProcessesToUpdate, System};
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
    pub pkg_env_path: PathBuf,
    pub current_dir: Option<PathBuf>,
    pub env_vars: HashMap<String, String>,
    pub media_info: Mutex<Option<MediaInfo>>,
}

struct NativeServiceSpec {
    executable: PathBuf,
    process_names: Vec<String>,
    status_port: Option<u16>,
    use_op_subcommand: bool,
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
            pkg_env_path: PathBuf::from(""),
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: Mutex::new(None),
        }
    }
}
impl ServicePkg {
    pub fn new(pkg_id: String, env_path: PathBuf) -> Self {
        Self {
            pkg_id,
            pkg_env_path: env_path,
            current_dir: None,
            env_vars: HashMap::new(),
            media_info: Mutex::new(None),
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

        self.native_stop(spec, media_root, None, env_vars).await?;
        let pid = spawn_detached(spec.executable.as_path(), &args, env_vars, cwd)?;
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

        let pids = service_process_pids(spec);
        if pids.is_empty() {
            info!("service {} is not running", spec.executable.display());
            return Ok(0);
        }

        for pid in pids {
            stop_process_tree(pid).await?;
        }
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

        if service_process_pids(spec).is_empty() {
            return Ok(1);
        }

        if let Some(port) = spec.status_port {
            if !check_local_port(port) {
                return Ok(1);
            }
        }

        Ok(0)
    }
}

fn package_unique_name(pkg_id: &str) -> String {
    let base = pkg_id.split('#').next().unwrap_or(pkg_id).trim();
    base.rsplit('.').next().unwrap_or(base).trim().to_string()
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

fn process_matches(spec: &NativeServiceSpec, process: &sysinfo::Process) -> bool {
    if let Some(exe) = process.exe() {
        if normalize_path_value(exe) == normalize_path_value(spec.executable.as_path()) {
            return true;
        }
    }

    if let Some(cmd0) = process.cmd().first() {
        let cmd0_path = Path::new(cmd0);
        if cmd0_path.is_absolute()
            && normalize_path_value(cmd0_path) == normalize_path_value(spec.executable.as_path())
        {
            return true;
        }

        let cmd0_name = cmd0.to_string_lossy();
        if process_name_candidates(cmd0_name.as_ref())
            .iter()
            .any(|candidate| spec.process_names.iter().any(|name| name == candidate))
        {
            return true;
        }
    }

    let process_name = process.name().to_string_lossy();
    process_name_candidates(process_name.as_ref())
        .iter()
        .any(|candidate| spec.process_names.iter().any(|name| name == candidate))
}

fn service_process_pids(spec: &NativeServiceSpec) -> Vec<i32> {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);

    let mut pids = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| process_matches(spec, process).then_some(pid.as_u32() as i32))
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn check_local_port(port: u16) -> bool {
    if port == 0 {
        return true;
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
}

async fn stop_process_tree(pid: i32) -> Result<()> {
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
        let group_args = vec!["-TERM".to_string(), format!("-{pid}")];
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
    if pid <= 0 {
        return false;
    }

    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.process(Pid::from_u32(pid as u32)).is_some()
}

async fn run_command(
    program: &Path,
    args: &[String],
    envs: Option<&HashMap<String, String>>,
    cwd: Option<&Path>,
) -> Result<CommandOutput> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(envs) = envs {
        cmd.envs(envs);
    }
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output().await.map_err(|error| {
        ServiceControlError::ReasonError(format!("spawn {} failed: {}", program.display(), error))
    })?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn spawn_detached(
    program: &Path,
    args: &[String],
    envs: Option<&HashMap<String, String>>,
    cwd: Option<&Path>,
) -> Result<u32> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    if let Some(envs) = envs {
        cmd.envs(envs);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

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
    Ok(child.id())
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
}
