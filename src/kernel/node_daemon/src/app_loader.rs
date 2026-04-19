use crate::run_item::{ControlRuntItemErrors, Result};
use crate::service_pkg::new_system_package_env;
use buckyos_api::{
    get_buckyos_api_runtime, get_full_appid, get_session_token_env_key, AppDoc,
    AppServiceInstanceConfig, AppType, LocalAppInstanceConfig, ServiceInstallConfig,
    ServiceInstanceState, SubPkgDesc, VERIFY_HUB_TOKEN_EXPIRE_TIME,
};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_root_dir};
use log::{debug, info, warn};
use ndn_lib::{load_named_object_from_obj_str, ObjId};
use package_lib::{MediaInfo, PackageEnv, PackageId, PackageMeta, PkgError};
use serde::Deserialize;
use serde_json::{json, Value};
use shlex::Shlex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

const DEFAULT_OPENDAN_SERVICE_PORT: u16 = 4060;
const OPENDAN_SERVICE_PORT_FALLBACK_KEYS: [&str; 4] = ["www", "http", "https", "main"];
/// Default repo for the unified Worker Image (paios/aios). This single image
/// now hosts both Agent and HostScript apps — the legacy buckyos/script-service
/// image is gone (see notepads/paios容器需求.md §6.3, §8).
const DEFAULT_WORKER_IMAGE_REPO: &str = "paios/aios";
const AGENT_RUNTIME_HOST_GATEWAY: &str = "host.docker.internal";
/// Read-only source mount inside the container. The image entrypoint
/// file-syncs this into the instance volume's working copy, replacing the
/// old overlayfs / fuse-overlayfs bootstrap.
const WORKER_CONTAINER_PKG_SOURCE_ROOT: &str = "/mnt/buckyos/pkg";
/// Base mount point of the per-instance private Docker volume. Contains the
/// working copy, dep caches (deno/uv/npm/pip) and sync metadata.
const WORKER_CONTAINER_INSTANCE_VOLUME_ROOT: &str = "/opt/buckyos/instance";
/// Writable working copy of the app package inside the instance volume.
const WORKER_CONTAINER_PKG_ROOT: &str = "/opt/buckyos/instance/pkg";
const WORKER_CONTAINER_LOG_ROOT: &str = "/opt/buckyos/logs";
const WORKER_CONTAINER_STORAGE_ROOT: &str = "/opt/buckyos/storage";
const DEVENV_JSON_FILE_NAME: &str = "devenv.json";
const DEVENV_JSON_AIOS_KEY: &str = "aios";
pub(crate) const DOCKER_LABEL_APP_ID: &str = "buckyos.app_id";
pub(crate) const DOCKER_LABEL_OWNER_USER_ID: &str = "buckyos.owner_user_id";
pub(crate) const DOCKER_LABEL_FULL_APPID: &str = "buckyos.full_appid";
pub(crate) const DOCKER_LABEL_PKG_OBJID: &str = "buckyos.pkg_objid";
pub(crate) const DOCKER_LABEL_IMAGE_DIGEST: &str = "buckyos.image_digest";

#[derive(Clone)]
enum LoaderConfig {
    Service(AppServiceInstanceConfig),
    Local(LocalAppInstanceConfig),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeType {
    Docker,
    HostScript,
    Agent,
    Vm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlatformOs {
    Linux,
    Macos,
    Windows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlatformArch {
    Amd64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlatformTarget {
    pub os: PlatformOs,
    pub arch: PlatformArch,
}

impl PlatformTarget {
    pub(crate) const fn new(os: PlatformOs, arch: PlatformArch) -> Self {
        Self { os, arch }
    }

    pub(crate) fn current() -> Self {
        let os = match std::env::consts::OS {
            "windows" => PlatformOs::Windows,
            "macos" => PlatformOs::Macos,
            _ => PlatformOs::Linux,
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" | "arm64" => PlatformArch::Aarch64,
            _ => PlatformArch::Amd64,
        };
        Self { os, arch }
    }

    fn python_program(self) -> &'static str {
        match self.os {
            PlatformOs::Windows => "python",
            PlatformOs::Linux | PlatformOs::Macos => "python3",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ControlOperation {
    Deploy,
    Start,
    Stop,
    Status,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    fn new(program: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ControlCommandPreview {
    pub runtime: RuntimeType,
    pub commands: Vec<CommandSpec>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DockerRuntimeIdentity {
    pub image_id: Option<String>,
    pub repo_digests: Vec<String>,
    pub labels: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct DockerContainerRuntime {
    running: bool,
    identity: DockerRuntimeIdentity,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DockerInspectState {
    #[serde(rename = "Running", default)]
    pub(crate) running: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DockerInspectConfig {
    #[serde(rename = "Labels", default)]
    pub(crate) labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DockerContainerInspectDoc {
    #[serde(rename = "State", default)]
    pub(crate) state: DockerInspectState,
    #[serde(rename = "Config", default)]
    pub(crate) config: DockerInspectConfig,
    #[serde(rename = "Image", default)]
    pub(crate) image: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackageRole {
    DockerImage,
    HostApp,
    AgentPkg,
}

pub struct AppLoader {
    app_id: String,
    owner_user_id: String,
    config: LoaderConfig,
    platform: PlatformTarget,
    support_container_override: Option<bool>,
    worker_image_repo_override: Option<String>,
    safe_mode: bool,
}

impl AppLoader {
    pub fn new_for_service(app_instance_id: &str, config: AppServiceInstanceConfig) -> Self {
        let app_id = app_instance_id
            .split('@')
            .next()
            .unwrap_or(app_instance_id)
            .to_string();
        let owner_user_id = config.app_spec.user_id.clone();
        Self {
            app_id,
            owner_user_id,
            config: LoaderConfig::Service(config),
            platform: PlatformTarget::current(),
            support_container_override: None,
            worker_image_repo_override: None,
            safe_mode: false,
        }
    }

    pub fn new_for_local(app_id: &str, config: LocalAppInstanceConfig) -> Self {
        Self {
            app_id: app_id.to_string(),
            owner_user_id: config.user_id.clone(),
            config: LoaderConfig::Local(config),
            platform: PlatformTarget::current(),
            support_container_override: None,
            worker_image_repo_override: None,
            safe_mode: false,
        }
    }

    pub(crate) fn with_platform(mut self, platform: PlatformTarget) -> Self {
        self.platform = platform;
        self
    }

    pub(crate) fn with_container_support_override(mut self, support_container: bool) -> Self {
        self.support_container_override = Some(support_container);
        self
    }

    pub(crate) fn with_worker_image_repo_override(
        mut self,
        image_repo: impl Into<String>,
    ) -> Self {
        self.worker_image_repo_override = Some(image_repo.into());
        self
    }

    /// Start the app with an empty instance volume (§7.5 Safe Mode).
    /// The instance volume is isolated per safe-mode session so the real
    /// instance volume is preserved for later diagnosis.
    pub(crate) fn with_safe_mode(mut self, safe_mode: bool) -> Self {
        self.safe_mode = safe_mode;
        self
    }

    pub async fn deploy(&self) -> Result<()> {
        let runtime = self.resolve_runtime()?;
        info!(
            "app_loader deploy app={} user={} runtime={:?}",
            self.app_id, self.owner_user_id, runtime
        );

        match runtime {
            RuntimeType::Docker => {
                self.prepare_docker_image().await?;
            }
            RuntimeType::HostScript => {
                let pkg_id = self
                    .host_script_pkg_id()
                    .ok_or_else(|| self.pkg_not_found("script or host app package"))?;
                self.ensure_pkg_installed(pkg_id.as_str()).await?;
                self.prepare_worker_image().await?;
            }
            RuntimeType::Agent => {
                let pkg_id = self
                    .agent_pkg_id()
                    .ok_or_else(|| self.pkg_not_found("agent package"))?;
                self.ensure_pkg_installed(pkg_id.as_str()).await?;
                if let Some(skills_pkg_id) = self.agent_skills_pkg_id() {
                    self.ensure_pkg_installed(skills_pkg_id.as_str()).await?;
                }
                self.prepare_worker_image().await?;
            }
            RuntimeType::Vm => {
                return Err(ControlRuntItemErrors::NotSupport(
                    "vm runtime is reserved but not implemented".to_string(),
                ));
            }
        }

        Ok(())
    }

    pub async fn start(&self) -> Result<()> {
        let runtime = self.resolve_runtime()?;
        info!(
            "app_loader start app={} user={} runtime={:?}",
            self.app_id, self.owner_user_id, runtime
        );

        match runtime {
            RuntimeType::Docker => self.start_docker().await,
            RuntimeType::HostScript => self.start_host_script().await,
            RuntimeType::Agent => self.start_agent().await,
            RuntimeType::Vm => Err(ControlRuntItemErrors::NotSupport(
                "vm runtime is reserved but not implemented".to_string(),
            )),
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let runtime = self.resolve_runtime()?;
        info!(
            "app_loader stop app={} user={} runtime={:?}",
            self.app_id, self.owner_user_id, runtime
        );

        match runtime {
            RuntimeType::Docker => self.stop_docker().await,
            RuntimeType::HostScript => self.stop_host_script().await,
            RuntimeType::Agent => self.stop_agent().await,
            RuntimeType::Vm => Err(ControlRuntItemErrors::NotSupport(
                "vm runtime is reserved but not implemented".to_string(),
            )),
        }
    }

    pub async fn status(&self) -> Result<ServiceInstanceState> {
        let runtime = self.resolve_runtime()?;
        debug!(
            "app_loader status app={} user={} runtime={:?}",
            self.app_id, self.owner_user_id, runtime
        );

        match runtime {
            RuntimeType::Docker => self.status_docker().await,
            RuntimeType::HostScript => self.status_host_script().await,
            RuntimeType::Agent => self.status_agent().await,
            RuntimeType::Vm => Err(ControlRuntItemErrors::NotSupport(
                "vm runtime is reserved but not implemented".to_string(),
            )),
        }
    }

    pub(crate) fn preview_operation(
        &self,
        operation: ControlOperation,
    ) -> Result<ControlCommandPreview> {
        let runtime = self.resolve_runtime()?;
        let commands = match (runtime, operation) {
            (RuntimeType::Docker, ControlOperation::Deploy) => self.preview_docker_deploy()?,
            (RuntimeType::Docker, ControlOperation::Start) => self.preview_docker_start()?,
            (RuntimeType::Docker, ControlOperation::Stop) => {
                vec![CommandSpec::new(
                    "docker",
                    ["rm", "-f", self.full_appid().as_str()],
                )]
            }
            (RuntimeType::Docker, ControlOperation::Status) => self.preview_docker_status(),
            (RuntimeType::HostScript, ControlOperation::Deploy) => {
                self.preview_host_script_deploy()?
            }
            (RuntimeType::HostScript, ControlOperation::Start) => {
                self.preview_host_script_start()?
            }
            (RuntimeType::HostScript, ControlOperation::Stop) => {
                vec![CommandSpec::new(
                    "docker",
                    ["rm", "-f", self.full_appid().as_str()],
                )]
            }
            (RuntimeType::HostScript, ControlOperation::Status) => {
                self.preview_host_script_status()
            }
            (RuntimeType::Agent, ControlOperation::Deploy) => self.preview_agent_deploy()?,
            (RuntimeType::Agent, ControlOperation::Start) => self.preview_agent_start()?,
            (RuntimeType::Agent, ControlOperation::Stop) => vec![self.preview_agent_stop()],
            (RuntimeType::Agent, ControlOperation::Status) => self.preview_agent_status(),
            (RuntimeType::Vm, _) => {
                return Err(ControlRuntItemErrors::NotSupport(
                    "vm runtime is reserved but not implemented".to_string(),
                ));
            }
        };

        Ok(ControlCommandPreview { runtime, commands })
    }

    fn full_appid(&self) -> String {
        get_full_appid(&self.app_id, &self.owner_user_id)
    }

    fn app_doc(&self) -> &AppDoc {
        match &self.config {
            LoaderConfig::Service(config) => &config.app_spec.app_doc,
            LoaderConfig::Local(config) => &config.app_doc,
        }
    }

    fn install_config(&self) -> &ServiceInstallConfig {
        match &self.config {
            LoaderConfig::Service(config) => &config.app_spec.install_config,
            LoaderConfig::Local(config) => &config.install_config,
        }
    }

    fn service_ports_config(&self) -> HashMap<String, u16> {
        match &self.config {
            LoaderConfig::Service(config) => config.service_ports_config.clone(),
            LoaderConfig::Local(_) => HashMap::new(),
        }
    }

    fn is_local_app(&self) -> bool {
        matches!(self.config, LoaderConfig::Local(_))
    }

    fn effective_app_type(&self) -> AppType {
        let doc = self.app_doc();
        if let Some(category) = doc.categories.first() {
            if let Ok(app_type) = AppType::try_from(category.as_str()) {
                return app_type;
            }
        }

        if doc.pkg_list.agent.is_some() {
            return AppType::Agent;
        }
        if doc.pkg_list.web.is_some() {
            return AppType::Web;
        }
        if self.docker_image_desc().is_some() {
            return AppType::AppService;
        }

        AppType::Service
    }

    fn resolve_runtime(&self) -> Result<RuntimeType> {
        let app_type = self.effective_app_type();
        let has_docker = self.docker_image_desc().is_some();
        let has_host_script = self.host_script_pkg_id().is_some();
        let has_agent_pkg = self.agent_pkg_id().is_some();

        if app_type == AppType::Agent || has_agent_pkg {
            if !has_agent_pkg {
                return Err(self.pkg_not_found("agent package"));
            }
            if !self.device_supports_container() {
                return Err(ControlRuntItemErrors::NotSupport(format!(
                    "agent app {} requires container runtime but current device does not support containers",
                    self.app_id
                )));
            }
            return Ok(RuntimeType::Agent);
        }

        if self.is_local_app() {
            if has_host_script {
                return Ok(RuntimeType::HostScript);
            }
            return Err(ControlRuntItemErrors::NotSupport(format!(
                "local app {} has no script or native host package for current platform",
                self.app_id
            )));
        }

        if has_docker && self.device_supports_container() {
            return Ok(RuntimeType::Docker);
        }

        if has_host_script {
            return Ok(RuntimeType::HostScript);
        }

        if has_docker {
            return Err(ControlRuntItemErrors::NotSupport(format!(
                "app {} only provides docker runtime but current device does not support containers",
                self.app_id
            )));
        }

        Ok(RuntimeType::Vm)
    }

    fn device_supports_container(&self) -> bool {
        if let Some(support_container) = self.support_container_override {
            return support_container;
        }
        std::env::var("BUCKYOS_THIS_DEVICE_INFO")
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(raw.as_str()).ok())
            .and_then(|value| value.get("support_container").and_then(Value::as_bool))
            .unwrap_or(true)
    }

    fn docker_image_desc(&self) -> Option<&SubPkgDesc> {
        let pkg_list = &self.app_doc().pkg_list;
        if self.platform.arch == PlatformArch::Aarch64 {
            pkg_list
                .aarch64_docker_image
                .as_ref()
                .or(pkg_list.amd64_docker_image.as_ref())
        } else {
            pkg_list
                .amd64_docker_image
                .as_ref()
                .or(pkg_list.aarch64_docker_image.as_ref())
        }
    }

    fn agent_desc(&self) -> Option<&SubPkgDesc> {
        self.app_doc().pkg_list.agent.as_ref()
    }

    fn host_app_desc(&self) -> Option<&SubPkgDesc> {
        let pkg_list = &self.app_doc().pkg_list;
        match (self.platform.os, self.platform.arch) {
            (PlatformOs::Linux, PlatformArch::Aarch64) => pkg_list
                .aarch64_linux_app
                .as_ref()
                .or(pkg_list.amd64_linux_app.as_ref()),
            (PlatformOs::Linux, PlatformArch::Amd64) => pkg_list
                .amd64_linux_app
                .as_ref()
                .or(pkg_list.aarch64_linux_app.as_ref()),
            (PlatformOs::Macos, PlatformArch::Aarch64) => pkg_list
                .aarch64_apple_app
                .as_ref()
                .or(pkg_list.amd64_apple_app.as_ref()),
            (PlatformOs::Macos, PlatformArch::Amd64) => pkg_list
                .amd64_apple_app
                .as_ref()
                .or(pkg_list.aarch64_apple_app.as_ref()),
            (PlatformOs::Windows, PlatformArch::Aarch64) => pkg_list
                .aarch64_win_app
                .as_ref()
                .or(pkg_list.amd64_win_app.as_ref()),
            (PlatformOs::Windows, PlatformArch::Amd64) => pkg_list
                .amd64_win_app
                .as_ref()
                .or(pkg_list.aarch64_win_app.as_ref()),
            (PlatformOs::Linux, _) => None,
        }
    }

    fn host_app_pkg_id(&self) -> Option<String> {
        self.host_app_desc()
            .and_then(SubPkgDesc::get_pkg_id_with_objid)
    }

    fn script_desc(&self) -> Option<&SubPkgDesc> {
        self.app_doc().pkg_list.script.as_ref()
    }

    fn script_pkg_id(&self) -> Option<String> {
        self.script_desc()
            .and_then(SubPkgDesc::get_pkg_id_with_objid)
    }

    /// Returns the package ID for the HostScript runtime.
    /// Prefers the platform-independent `script` field; falls back to
    /// platform-specific host app fields for native binaries.
    fn host_script_pkg_id(&self) -> Option<String> {
        self.script_pkg_id().or_else(|| self.host_app_pkg_id())
    }

    /// Returns the SubPkgDesc used for HostScript labels.
    fn host_script_desc(&self) -> Option<&SubPkgDesc> {
        self.script_desc().or_else(|| self.host_app_desc())
    }

    fn agent_pkg_id(&self) -> Option<String> {
        self.app_doc()
            .pkg_list
            .agent
            .as_ref()
            .and_then(SubPkgDesc::get_pkg_id_with_objid)
    }

    fn agent_skills_pkg_id(&self) -> Option<String> {
        self.app_doc()
            .pkg_list
            .agent_skills
            .as_ref()
            .and_then(SubPkgDesc::get_pkg_id_with_objid)
    }

    fn pkg_not_found(&self, role: &str) -> ControlRuntItemErrors {
        ControlRuntItemErrors::PkgNotExist(format!(
            "app {} {} not found for current platform",
            self.app_id, role
        ))
    }

    async fn ensure_pkg_installed(&self, pkg_id: &str) -> Result<MediaInfo> {
        let mut env = new_system_package_env();
        self.ensure_pkg_meta_indexed(&env, pkg_id).await?;
        match env.install_pkg(pkg_id, true, false).await {
            Ok(_) => info!("installed pkg {} for app {}", pkg_id, self.app_id),
            Err(PkgError::PackageAlreadyInstalled(_)) => {
                info!("pkg {} already installed for app {}", pkg_id, self.app_id)
            }
            Err(error) => {
                warn!(
                    "install pkg {} for app {} failed, will continue to load installed media if present: {}",
                    pkg_id, self.app_id, error
                );
            }
        }

        env.load(pkg_id).await.map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "load_pkg".to_string(),
                format!("load pkg {} failed: {}", pkg_id, error),
            )
        })
    }

    async fn try_load_pkg(&self, pkg_id: &str) -> Option<MediaInfo> {
        let env = new_system_package_env();
        env.load(pkg_id).await.ok()
    }

    async fn prepare_docker_image(&self) -> Result<()> {
        let desc = self
            .docker_image_desc()
            .ok_or_else(|| self.pkg_not_found("docker image"))?;
        let image_name = desc.docker_image_name.clone().ok_or_else(|| {
            ControlRuntItemErrors::ParserConfigError(format!(
                "docker_image_name is missing for app {}",
                self.app_id
            ))
        })?;
        let digest = desc.docker_image_digest.clone();

        if self
            .check_docker_image_exists(image_name.as_str(), digest.as_deref())
            .await?
        {
            return Ok(());
        }

        if let Some(pkg_id) = desc.get_pkg_id_with_objid() {
            if let Ok(media_info) = self.ensure_pkg_installed(pkg_id.as_str()).await {
                if let Some(tar_path) = self.find_docker_image_tar(&media_info) {
                    info!(
                        "load docker image for app {} from {}",
                        self.app_id,
                        tar_path.display()
                    );
                    self.load_docker_image_from_tar(tar_path.as_path()).await?;
                    if self
                        .check_docker_image_exists(image_name.as_str(), digest.as_deref())
                        .await?
                    {
                        return Ok(());
                    }
                }
            }
        }

        self.pull_docker_image(image_name.as_str(), digest.as_deref())
            .await?;

        if !self
            .check_docker_image_exists(image_name.as_str(), digest.as_deref())
            .await?
        {
            return Err(ControlRuntItemErrors::ExecuteError(
                "deploy".to_string(),
                format!("docker image {} prepared but validation failed", image_name),
            ));
        }

        Ok(())
    }

    async fn start_docker(&self) -> Result<()> {
        self.prepare_docker_image().await?;

        let desc = self
            .docker_image_desc()
            .ok_or_else(|| self.pkg_not_found("docker image"))?;
        let image_name = desc.docker_image_name.clone().ok_or_else(|| {
            ControlRuntItemErrors::ParserConfigError(format!(
                "docker_image_name is missing for app {}",
                self.app_id
            ))
        })?;
        let container_name = self.full_appid();

        self.stop_docker().await?;

        let env_vars = self.build_runtime_env(PackageRole::DockerImage).await?;
        let mut args: Vec<String> = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            container_name.clone(),
        ];
        append_host_gateway_run_args(&mut args);

        for (service_name, instance_port) in self.service_ports_config() {
            let container_port = self
                .app_doc()
                .install_config_tips
                .service_ports
                .get(service_name.as_str())
                .copied()
                .ok_or_else(|| {
                    ControlRuntItemErrors::ParserConfigError(format!(
                        "service {} port mapping missing in app_doc for {}",
                        service_name, self.app_id
                    ))
                })?;
            args.push("-p".to_string());
            args.push(format!("{instance_port}:{container_port}"));
        }

        for (container_path, host_path, permission) in self.build_volume_mounts()? {
            args.push("-v".to_string());
            args.push(format!(
                "{}:{}:{}",
                host_path.to_string_lossy(),
                container_path,
                permission
            ));
        }

        for (key, value) in env_vars.iter() {
            args.push("-e".to_string());
            args.push(format!("{key}={value}"));
        }

        for (key, value) in self.docker_runtime_labels(desc) {
            args.push("--label".to_string());
            args.push(format!("{key}={value}"));
        }

        if let Some(container_param) = &self.install_config().container_param {
            args.extend(split_shell_words(container_param.as_str())?);
        }

        args.push(image_name.clone());
        let output = run_command("docker", &args, None, None).await?;
        ensure_success("docker run", &output)?;
        info!(
            "docker container {} started for app {} with image {}",
            container_name, self.app_id, image_name
        );
        Ok(())
    }

    async fn stop_docker(&self) -> Result<()> {
        self.remove_current_docker_container().await
    }

    async fn status_docker(&self) -> Result<ServiceInstanceState> {
        let desc = self
            .docker_image_desc()
            .ok_or_else(|| self.pkg_not_found("docker image"))?;
        let exact_match_required = docker_desc_requires_exact_match(desc);
        if let Some(runtime) = self.inspect_current_docker_container().await? {
            if exact_match_required && !docker_runtime_matches_target(&runtime.identity, desc) {
                // A stale container with the current name exists but does not match the target package.
                // Treat it as stopped and let the subsequent start path replace it.
            } else if runtime.running {
                return Ok(ServiceInstanceState::Started);
            } else {
                return Ok(ServiceInstanceState::Exited);
            }
        }

        let image_name = desc.docker_image_name.clone().ok_or_else(|| {
            ControlRuntItemErrors::ParserConfigError(format!(
                "docker_image_name is missing for app {}",
                self.app_id
            ))
        })?;

        if self
            .check_docker_image_exists(image_name.as_str(), desc.docker_image_digest.as_deref())
            .await?
        {
            Ok(ServiceInstanceState::Stopped)
        } else {
            Ok(ServiceInstanceState::NotExist)
        }
    }

    async fn start_host_script(&self) -> Result<()> {
        let pkg_id = self
            .host_script_pkg_id()
            .ok_or_else(|| self.pkg_not_found("script or host app package"))?;
        let media_info = self.ensure_pkg_installed(pkg_id.as_str()).await?;
        let package_root = media_info.full_path.clone();
        if !package_root.exists() {
            return Err(ControlRuntItemErrors::ExecuteError(
                "start".to_string(),
                format!(
                    "host script package root {} not found",
                    package_root.display()
                ),
            ));
        }

        self.stop_host_script().await?;

        self.run_worker_container(
            PackageRole::HostApp,
            "script",
            package_root.as_path(),
            None,
            self.host_script_desc(),
        )
        .await
    }

    async fn stop_host_script(&self) -> Result<()> {
        self.remove_current_docker_container().await
    }

    async fn status_host_script(&self) -> Result<ServiceInstanceState> {
        let pkg_id = match self.host_script_pkg_id() {
            Some(pkg_id) => pkg_id,
            None => return Ok(ServiceInstanceState::NotExist),
        };
        if self.try_load_pkg(pkg_id.as_str()).await.is_none() {
            return Ok(ServiceInstanceState::NotExist);
        }

        if let Some(runtime) = self.inspect_current_docker_container().await? {
            if runtime.running {
                return Ok(ServiceInstanceState::Started);
            }
            return Ok(ServiceInstanceState::Exited);
        }

        let image_name = self.worker_image_name();
        if self
            .check_docker_image_exists(image_name.as_str(), None)
            .await?
        {
            Ok(ServiceInstanceState::Stopped)
        } else {
            Ok(ServiceInstanceState::NotExist)
        }
    }

    async fn start_agent(&self) -> Result<()> {
        let _ = self.deploy().await?;

        let desc = self
            .agent_desc()
            .ok_or_else(|| self.pkg_not_found("agent package"))?;
        let media_info = self
            .package_media_info(PackageRole::AgentPkg)
            .await?
            .ok_or_else(|| self.pkg_not_found("agent package"))?;
        let agent_root = media_info.full_path.clone();
        if !agent_root.exists() {
            return Err(ControlRuntItemErrors::ExecuteError(
                "start".to_string(),
                format!("agent package root {} not found", agent_root.display()),
            ));
        }

        self.stop_agent().await?;

        let service_port = self.select_agent_service_port();
        self.run_worker_container(
            PackageRole::AgentPkg,
            "agent",
            agent_root.as_path(),
            Some(service_port),
            Some(desc),
        )
        .await
    }

    async fn stop_agent(&self) -> Result<()> {
        self.remove_current_docker_container().await
    }

    async fn status_agent(&self) -> Result<ServiceInstanceState> {
        let desc = match self.agent_desc() {
            Some(desc) => desc,
            None => return Ok(ServiceInstanceState::NotExist),
        };
        let media_info = match self.agent_pkg_id() {
            Some(pkg_id) => self.try_load_pkg(pkg_id.as_str()).await,
            None => None,
        };
        if media_info.is_none() {
            return Ok(ServiceInstanceState::NotExist);
        }

        let exact_match_required = desc.pkg_objid.is_some();
        if let Some(runtime) = self.inspect_current_docker_container().await? {
            if exact_match_required && !docker_runtime_matches_target(&runtime.identity, desc) {
                // An old generation container is still occupying the canonical name.
                // Leave it to the start path to replace it.
            } else if runtime.running {
                return Ok(ServiceInstanceState::Started);
            } else {
                return Ok(ServiceInstanceState::Exited);
            }
        }

        if self
            .check_docker_image_exists(self.worker_image_name().as_str(), None)
            .await?
        {
            Ok(ServiceInstanceState::Stopped)
        } else {
            Ok(ServiceInstanceState::NotExist)
        }
    }

    /// Build + launch a worker container (paios/aios) with the unified
    /// instance-volume layout. Called by both start_agent and start_host_script.
    async fn run_worker_container(
        &self,
        role: PackageRole,
        app_type_label: &str,
        pkg_source_root: &Path,
        service_port: Option<u16>,
        desc: Option<&SubPkgDesc>,
    ) -> Result<()> {
        let image_name = self.worker_image_name();
        let container_name = self.full_appid();
        let instance_volume = self.instance_volume_name();
        let env_vars = self
            .build_worker_runtime_env(role, app_type_label, service_port)
            .await?;
        let mounts = self.build_worker_volume_mounts(pkg_source_root, role)?;

        info!(
            "starting worker app_type={} id={} image={} pkg_source={} instance_volume={} safe_mode={}",
            app_type_label,
            container_name,
            image_name,
            pkg_source_root.display(),
            instance_volume,
            self.safe_mode,
        );

        let mut args: Vec<String> = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            container_name.clone(),
        ];
        append_host_gateway_run_args(&mut args);

        // Named instance volume (private per-app).
        args.push("-v".to_string());
        args.push(format!(
            "{}:{}:rw",
            instance_volume, WORKER_CONTAINER_INSTANCE_VOLUME_ROOT
        ));

        // Bind mounts (pkg source ro, logs, storage, data, declared mounts).
        for (container_path, host_path, permission) in mounts {
            args.push("-v".to_string());
            args.push(format!(
                "{}:{}:{}",
                host_path.to_string_lossy(),
                container_path,
                permission
            ));
        }

        // Port publishing. Agents publish their service_port; scripts
        // publish whichever ports the install config declared.
        if app_type_label == "agent" {
            if let Some(port) = service_port {
                args.push("-p".to_string());
                args.push(format!("{port}:{port}"));
            }
        } else {
            for (service_name, instance_port) in self.service_ports_config() {
                if let Some(container_port) = self
                    .app_doc()
                    .install_config_tips
                    .service_ports
                    .get(service_name.as_str())
                    .copied()
                {
                    args.push("-p".to_string());
                    args.push(format!("{instance_port}:{container_port}"));
                }
            }
        }

        for (key, value) in env_vars.iter() {
            args.push("-e".to_string());
            args.push(format!("{key}={value}"));
        }

        if let Some(desc) = desc {
            for (key, value) in self.docker_runtime_labels(desc) {
                args.push("--label".to_string());
                args.push(format!("{key}={value}"));
            }
        }

        if let Some(container_param) = &self.install_config().container_param {
            args.extend(split_shell_words(container_param.as_str())?);
        }

        args.push(image_name.clone());

        let output = run_command("docker", &args, None, None).await?;
        ensure_success("docker run (worker)", &output)?;

        info!(
            "worker container {} started (image={}, app_type={})",
            container_name, image_name, app_type_label
        );
        Ok(())
    }

    async fn package_media_info(&self, role: PackageRole) -> Result<Option<MediaInfo>> {
        let pkg_id = match role {
            PackageRole::DockerImage => self
                .docker_image_desc()
                .and_then(|desc| desc.get_pkg_id_with_objid()),
            PackageRole::HostApp => self.host_script_pkg_id(),
            PackageRole::AgentPkg => self.agent_pkg_id(),
        };

        if let Some(pkg_id) = pkg_id {
            return Ok(self.try_load_pkg(pkg_id.as_str()).await);
        }

        Ok(None)
    }

    async fn build_runtime_env(&self, role: PackageRole) -> Result<HashMap<String, String>> {
        let mut env_vars = HashMap::new();
        if let Some(zone_config) = std::env::var("BUCKYOS_ZONE_CONFIG").ok() {
            env_vars.insert("BUCKYOS_ZONE_CONFIG".to_string(), zone_config);
        }
        if let Some(device_info) = std::env::var("BUCKYOS_THIS_DEVICE_INFO").ok() {
            env_vars.insert("BUCKYOS_THIS_DEVICE_INFO".to_string(), device_info);
        }
        if let Some(device_doc) = std::env::var("BUCKYOS_THIS_DEVICE").ok() {
            env_vars.insert("BUCKYOS_THIS_DEVICE".to_string(), device_doc);
        }
        env_vars.insert(
            "BUCKYOS_HOST_GATEWAY".to_string(),
            AGENT_RUNTIME_HOST_GATEWAY.to_string(),
        );

        if let Some(media_info) = self.package_media_info(role).await? {
            env_vars.insert(
                "app_media_info".to_string(),
                json!({
                    "pkg_id": self.media_pkg_id(role),
                    "full_path": media_info.full_path.to_string_lossy(),
                })
                .to_string(),
            );
        }

        match &self.config {
            LoaderConfig::Service(config) => {
                let app_config_str = serde_json::to_string(config).map_err(|error| {
                    ControlRuntItemErrors::ParserConfigError(format!(
                        "serialize app_instance_config failed: {}",
                        error
                    ))
                })?;
                env_vars.insert("app_instance_config".to_string(), app_config_str);

                let timestamp = buckyos_get_unix_timestamp();
                let runtime = get_buckyos_api_runtime().map_err(|error| {
                    ControlRuntItemErrors::ExecuteError(
                        "build_env".to_string(),
                        format!("buckyos runtime unavailable: {}", error),
                    )
                })?;
                let device_doc = runtime.device_config.as_ref().ok_or_else(|| {
                    ControlRuntItemErrors::ExecuteError(
                        "build_env".to_string(),
                        "device_config is missing".to_string(),
                    )
                })?;
                let device_private_key = runtime.device_private_key.as_ref().ok_or_else(|| {
                    ControlRuntItemErrors::ExecuteError(
                        "build_env".to_string(),
                        "device_private_key is missing".to_string(),
                    )
                })?;

                let login_jti = timestamp.to_string();
                let session_token = kRPC::RPCSessionToken {
                    token_type: kRPC::RPCSessionTokenType::Normal,
                    appid: Some(self.app_id.clone()),
                    jti: Some(login_jti.clone()),
                    session: Some(timestamp),
                    sub: Some(config.app_spec.user_id.clone()),
                    aud: None,
                    exp: Some(timestamp + VERIFY_HUB_TOKEN_EXPIRE_TIME * 2),
                    iss: Some(device_doc.name.clone()),
                    token: None,
                    extra: HashMap::new(),
                };
                let session_token_jwt = session_token
                    .generate_jwt(Some(device_doc.name.clone()), device_private_key)
                    .map_err(|error| {
                        ControlRuntItemErrors::ExecuteError(
                            "build_env".to_string(),
                            format!("generate session token failed: {}", error),
                        )
                    })?;
                env_vars.insert(
                    get_session_token_env_key(self.full_appid().as_str(), true),
                    session_token_jwt,
                );
            }
            LoaderConfig::Local(config) => {
                let local_config_str = serde_json::to_string(config).map_err(|error| {
                    ControlRuntItemErrors::ParserConfigError(format!(
                        "serialize local_app_instance_config failed: {}",
                        error
                    ))
                })?;
                env_vars.insert(
                    "local_app_instance_config".to_string(),
                    local_config_str.clone(),
                );
                env_vars.insert("loca_app_instance_config".to_string(), local_config_str);
            }
        }

        Ok(env_vars)
    }

    fn media_pkg_id(&self, role: PackageRole) -> Option<String> {
        match role {
            PackageRole::DockerImage => self
                .docker_image_desc()
                .and_then(SubPkgDesc::get_pkg_id_with_objid),
            PackageRole::HostApp => self.host_script_pkg_id(),
            PackageRole::AgentPkg => self.agent_pkg_id(),
        }
    }

    fn find_docker_image_tar(&self, media_info: &MediaInfo) -> Option<PathBuf> {
        for candidate in docker_image_tar_candidates_for_arch(&self.app_id, self.platform.arch) {
            let path = media_info.full_path.join(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    async fn load_docker_image_from_tar(&self, tar_path: &Path) -> Result<()> {
        let output = run_command(
            "docker",
            &[
                "load".to_string(),
                "-i".to_string(),
                tar_path.to_string_lossy().to_string(),
            ],
            None,
            None,
        )
        .await?;
        ensure_success("docker load", &output)?;
        Ok(())
    }

    async fn pull_docker_image(&self, image_name: &str, digest: Option<&str>) -> Result<()> {
        let repo_base = self.zone_docker_repo_base_url();
        let image_ref = match repo_base {
            Some(base) => format!("{}/{}", base.trim_end_matches('/'), image_name),
            None => image_name.to_string(),
        };
        let pull_ref = if let Some(digest) = normalize_digest(digest) {
            format!("{image_ref}@{digest}")
        } else {
            image_ref.clone()
        };

        let output = run_command(
            "docker",
            &["pull".to_string(), pull_ref.clone()],
            None,
            None,
        )
        .await?;
        ensure_success("docker pull", &output)?;

        if pull_ref != image_name {
            let image_id_output = run_command(
                "docker",
                &["images".to_string(), "-q".to_string(), image_ref.clone()],
                None,
                None,
            )
            .await?;
            ensure_success("docker images -q", &image_id_output)?;
            let image_id = image_id_output.stdout.trim();
            if !image_id.is_empty() {
                let tag_output = run_command(
                    "docker",
                    &[
                        "tag".to_string(),
                        image_id.to_string(),
                        image_name.to_string(),
                    ],
                    None,
                    None,
                )
                .await?;
                ensure_success("docker tag", &tag_output)?;
            }
        }

        Ok(())
    }

    async fn check_docker_image_exists(
        &self,
        image_name: &str,
        digest: Option<&str>,
    ) -> Result<bool> {
        let images = run_command(
            "docker",
            &[
                "images".to_string(),
                "-q".to_string(),
                image_name.to_string(),
            ],
            None,
            None,
        )
        .await?;
        ensure_success("docker images -q", &images)?;
        if images.stdout.trim().is_empty() {
            return Ok(false);
        }

        let Some(expected_digest) = normalize_digest(digest) else {
            return Ok(true);
        };

        let repo_digest_output = run_command(
            "docker",
            &[
                "image".to_string(),
                "inspect".to_string(),
                "--format={{json .RepoDigests}}".to_string(),
                image_name.to_string(),
            ],
            None,
            None,
        )
        .await?;
        ensure_success("docker image inspect RepoDigests", &repo_digest_output)?;
        if let Ok(repo_digests) =
            serde_json::from_str::<Vec<String>>(repo_digest_output.stdout.trim())
        {
            if repo_digests.iter().any(|repo_digest| {
                repo_digest
                    .split_once('@')
                    .map(|(_, digest)| digest == expected_digest)
                    .unwrap_or(false)
            }) {
                return Ok(true);
            }
        }

        let id_output = run_command(
            "docker",
            &[
                "image".to_string(),
                "inspect".to_string(),
                "--format={{.Id}}".to_string(),
                image_name.to_string(),
            ],
            None,
            None,
        )
        .await?;
        ensure_success("docker image inspect Id", &id_output)?;
        let image_id = id_output.stdout.trim();
        if image_id == expected_digest {
            return Ok(true);
        }
        if let (Some((_, image_hash)), Some((_, expected_hash))) =
            (image_id.split_once(':'), expected_digest.split_once(':'))
        {
            return Ok(image_hash == expected_hash);
        }

        Ok(false)
    }

    async fn remove_current_docker_container(&self) -> Result<()> {
        let container_name = self.full_appid();
        let output = run_command(
            "docker",
            &["rm".to_string(), "-f".to_string(), container_name.clone()],
            None,
            None,
        )
        .await?;
        if output.status.success() {
            return Ok(());
        }

        if matches!(self.has_current_docker_container_name().await, Ok(false)) {
            return Ok(());
        }

        Err(ControlRuntItemErrors::ExecuteError(
            "docker rm".to_string(),
            format_command_failure("docker rm", &output),
        ))
    }

    fn zone_docker_repo_base_url(&self) -> Option<String> {
        std::env::var("BUCKYOS_ZONE_CONFIG")
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(raw.as_str()).ok())
            .and_then(|value| {
                value
                    .get("docker_repo_base_url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
    }

    fn build_volume_mounts(&self) -> Result<Vec<(String, PathBuf, &'static str)>> {
        let mut mounts: HashMap<String, (PathBuf, &'static str)> = HashMap::new();
        let app_data_container =
            format!("/home/{}/.local/share/{}", self.owner_user_id, self.app_id);
        mounts.insert("/tmp".to_string(), (self.app_local_cache_dir(), "rw"));
        mounts.insert(app_data_container.clone(), (self.app_data_dir(), "rw"));

        for (container_path, raw_value) in &self.install_config().data_mount_point {
            let (host_relative, permission_override) = parse_mount_value(raw_value.as_str());
            let host_path = get_buckyos_root_dir().join(host_relative.trim_start_matches('/'));
            let permission = permission_override.unwrap_or_else(|| {
                default_mount_permission(container_path.as_str(), &self.app_id, &self.owner_user_id)
            });
            mounts.insert(container_path.clone(), (host_path, permission));
        }

        let base_cache_dir = self.app_cache_dir();
        for mount_point in &self.install_config().cache_mount_point {
            mounts.insert(
                mount_point.clone(),
                (
                    base_cache_dir.join(mount_point.trim_start_matches('/')),
                    "rw",
                ),
            );
        }

        let base_local_cache_dir = self.app_local_cache_dir();
        for mount_point in &self.install_config().local_cache_mount_point {
            mounts.insert(
                mount_point.clone(),
                (
                    base_local_cache_dir.join(mount_point.trim_start_matches('/')),
                    "rw",
                ),
            );
        }

        let mut result = Vec::new();
        for (container_path, (host_path, permission)) in mounts {
            ensure_directory(&host_path, permission == "rw")?;
            let resolved_host_path = canonicalize_mount_path(host_path.as_path());
            result.push((
                trim_trailing_slash(container_path.as_str()).to_string(),
                resolved_host_path,
                permission,
            ));
        }

        result.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
        Ok(result)
    }

    fn select_agent_service_port(&self) -> u16 {
        let instance_ports = self.service_ports_config();
        if instance_ports.is_empty() {
            return DEFAULT_OPENDAN_SERVICE_PORT;
        }

        let mut seen_names = HashSet::new();
        for service_name in self.app_doc().install_config_tips.service_ports.keys() {
            if !seen_names.insert(service_name.clone()) {
                continue;
            }
            if let Some(port) = instance_ports.get(service_name).copied() {
                return port;
            }
        }
        for fallback in OPENDAN_SERVICE_PORT_FALLBACK_KEYS {
            if let Some(port) = instance_ports.get(fallback).copied() {
                return port;
            }
        }

        let mut sorted_ports = instance_ports.into_iter().collect::<Vec<_>>();
        sorted_ports.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
        sorted_ports
            .into_iter()
            .map(|(_, port)| port)
            .next()
            .unwrap_or(DEFAULT_OPENDAN_SERVICE_PORT)
    }

    fn app_data_dir(&self) -> PathBuf {
        get_buckyos_root_dir()
            .join("data")
            .join("home")
            .join(&self.owner_user_id)
            .join(".local")
            .join("share")
            .join(&self.app_id)
    }

    fn app_cache_dir(&self) -> PathBuf {
        get_buckyos_root_dir()
            .join("data")
            .join("cache")
            .join(self.full_appid())
    }

    fn app_local_cache_dir(&self) -> PathBuf {
        PathBuf::from("/tmp")
            .join("buckyos")
            .join(self.full_appid())
    }

    fn worker_log_dir(&self) -> PathBuf {
        get_buckyos_root_dir()
            .join("logs")
            .join("apps")
            .join(self.full_appid())
    }

    fn worker_storage_dir(&self) -> PathBuf {
        get_buckyos_root_dir().join("storage")
    }

    fn build_worker_volume_mounts(
        &self,
        pkg_source_root: &Path,
        role: PackageRole,
    ) -> Result<Vec<(String, PathBuf, &'static str)>> {
        // §6.1/§7.1: the worker container always has three volume groups —
        // (1) a read-only view of the upstream package at /mnt/buckyos/pkg,
        // (2) a private instance volume at /opt/buckyos/instance (for working
        //     copy, dep caches, and self-evolved state),
        // (3) external mounts for business data, logs, storage, plus the
        //     developer-declared data_mount_point / cache_mount_point pairs.
        let data_root = self.app_data_dir();
        let log_root = self.worker_log_dir();
        let storage_root = self.worker_storage_dir();
        ensure_directory(&data_root, true)?;
        ensure_directory(&log_root, true)?;
        ensure_directory(&storage_root, true)?;

        let app_data_container =
            format!("/home/{}/.local/share/{}", self.owner_user_id, self.app_id);
        let instance_volume_mount =
            format!("{}:{}:rw", self.instance_volume_name(), WORKER_CONTAINER_INSTANCE_VOLUME_ROOT);
        // Mounts are keyed by container-path so later entries can be overridden
        // by user declarations if they legitimately want to remap. The instance
        // volume mount uses a named volume — we encode that as a sentinel path
        // and the caller formats it separately.
        let mut mounts: HashMap<String, (PathBuf, &'static str)> = HashMap::new();
        mounts.insert(
            WORKER_CONTAINER_PKG_SOURCE_ROOT.to_string(),
            (canonicalize_mount_path(pkg_source_root), "ro"),
        );
        mounts.insert(
            WORKER_CONTAINER_LOG_ROOT.to_string(),
            (canonicalize_mount_path(log_root.as_path()), "rw"),
        );
        mounts.insert(
            WORKER_CONTAINER_STORAGE_ROOT.to_string(),
            (canonicalize_mount_path(storage_root.as_path()), "rw"),
        );
        mounts.insert(
            app_data_container.clone(),
            (canonicalize_mount_path(data_root.as_path()), "rw"),
        );

        // Developer-declared data mount points (e.g. zone shared data) map from
        // container paths into BUCKYOS_ROOT on the host.
        for (container_path, raw_value) in &self.install_config().data_mount_point {
            let (host_relative, permission_override) = parse_mount_value(raw_value.as_str());
            let host_path = get_buckyos_root_dir().join(host_relative.trim_start_matches('/'));
            let permission = permission_override.unwrap_or_else(|| {
                default_mount_permission(container_path.as_str(), &self.app_id, &self.owner_user_id)
            });
            ensure_directory(&host_path, permission == "rw")?;
            mounts.insert(container_path.clone(), (host_path, permission));
        }

        let base_cache_dir = self.app_cache_dir();
        for mount_point in &self.install_config().cache_mount_point {
            let host_path = base_cache_dir.join(mount_point.trim_start_matches('/'));
            ensure_directory(&host_path, true)?;
            mounts.insert(mount_point.clone(), (host_path, "rw"));
        }
        let base_local_cache_dir = self.app_local_cache_dir();
        for mount_point in &self.install_config().local_cache_mount_point {
            let host_path = base_local_cache_dir.join(mount_point.trim_start_matches('/'));
            ensure_directory(&host_path, true)?;
            mounts.insert(mount_point.clone(), (host_path, "rw"));
        }

        let _ = role; // reserved for future per-role external mounts

        let mut result: Vec<(String, PathBuf, &'static str)> = mounts
            .into_iter()
            .map(|(container_path, (host_path, permission))| {
                let resolved = canonicalize_mount_path(host_path.as_path());
                (
                    trim_trailing_slash(container_path.as_str()).to_string(),
                    resolved,
                    permission,
                )
            })
            .collect();
        result.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

        // Prepend the named-volume mount as a synthetic entry the caller can
        // recognise (empty host path => "this is a docker volume, format as
        // <name>:<container>:<perm>").
        // To keep the external API simple we instead return a sentinel string.
        // Callers already iterate tuples of (container, host_path, perm); we
        // expose a separate accessor for the instance-volume mount spec.
        let _ = instance_volume_mount;
        Ok(result)
    }

    /// Canonical name of the per-app instance volume (§6.1). In safe mode
    /// (§7.5) we use a disposable volume so the real one can be inspected by
    /// the user afterwards.
    fn instance_volume_name(&self) -> String {
        if self.safe_mode {
            let timestamp = buckyos_get_unix_timestamp();
            format!("buckyos-instance-{}-safe-{}", self.full_appid(), timestamp)
        } else {
            format!("buckyos-instance-{}", self.full_appid())
        }
    }

    fn worker_image_repo(&self) -> String {
        if let Some(repo) = &self.worker_image_repo_override {
            return repo.clone();
        }

        resolve_devenv_aios_image_repo()
            .unwrap_or_else(|| DEFAULT_WORKER_IMAGE_REPO.to_string())
    }

    fn worker_image_name(&self) -> String {
        let arch_tag = match self.platform.arch {
            PlatformArch::Aarch64 => "latest-aarch64",
            PlatformArch::Amd64 => "latest-amd64",
        };
        format!("{}:{arch_tag}", self.worker_image_repo())
    }

    async fn prepare_worker_image(&self) -> Result<()> {
        let image_name = self.worker_image_name();
        if self
            .check_docker_image_exists(image_name.as_str(), None)
            .await?
        {
            return Ok(());
        }

        info!(
            "worker image {} missing for app {}, pulling now",
            image_name, self.app_id
        );
        self.pull_docker_image(image_name.as_str(), None).await?;

        if !self
            .check_docker_image_exists(image_name.as_str(), None)
            .await?
        {
            return Err(ControlRuntItemErrors::ExecuteError(
                "deploy".to_string(),
                format!(
                    "worker image {} prepared but validation failed",
                    image_name
                ),
            ));
        }

        Ok(())
    }

    async fn build_worker_runtime_env(
        &self,
        role: PackageRole,
        app_type_label: &str,
        service_port: Option<u16>,
    ) -> Result<HashMap<String, String>> {
        let mut env_vars = self.build_runtime_env(role).await?;

        // §9 env contract.
        env_vars.insert("BUCKYOS_APP_ID".to_string(), self.app_id.clone());
        env_vars.insert(
            "BUCKYOS_APP_TYPE".to_string(),
            app_type_label.to_string(),
        );
        env_vars.insert(
            "BUCKYOS_OWNER_USER_ID".to_string(),
            self.owner_user_id.clone(),
        );
        env_vars.insert(
            "BUCKYOS_DATA_DIR".to_string(),
            format!("/home/{}/.local/share/{}", self.owner_user_id, self.app_id),
        );
        env_vars.insert(
            "BUCKYOS_LOG_DIR".to_string(),
            WORKER_CONTAINER_LOG_ROOT.to_string(),
        );
        env_vars.insert(
            "BUCKYOS_STORAGE_DIR".to_string(),
            WORKER_CONTAINER_STORAGE_ROOT.to_string(),
        );
        env_vars.insert(
            "BUCKYOS_PKG_SOURCE_DIR".to_string(),
            WORKER_CONTAINER_PKG_SOURCE_ROOT.to_string(),
        );
        env_vars.insert(
            "BUCKYOS_PKG_DIR".to_string(),
            WORKER_CONTAINER_PKG_ROOT.to_string(),
        );
        env_vars.insert(
            "BUCKYOS_INSTANCE_VOLUME".to_string(),
            WORKER_CONTAINER_INSTANCE_VOLUME_ROOT.to_string(),
        );
        env_vars.insert(
            "BUCKYOS_SAFE_MODE".to_string(),
            if self.safe_mode { "1".to_string() } else { "0".to_string() },
        );
        if let Some(port) = service_port {
            env_vars.insert("BUCKYOS_SERVICE_PORT".to_string(), port.to_string());
            // Legacy name kept for OpenDAN which still reads OPENDAN_SERVICE_PORT.
            env_vars.insert("OPENDAN_SERVICE_PORT".to_string(), port.to_string());
        }
        if app_type_label == "agent" {
            env_vars.insert("OPENDAN_AGENT_ID".to_string(), self.app_id.clone());
        }

        // The app sees itself at BUCKYOS_PKG_DIR, not at the host media path.
        if let Some(media_info) = env_vars.get("app_media_info").cloned() {
            if let Ok(mut value) = serde_json::from_str::<Value>(media_info.as_str()) {
                if let Some(object) = value.as_object_mut() {
                    object.insert(
                        "full_path".to_string(),
                        Value::String(WORKER_CONTAINER_PKG_ROOT.to_string()),
                    );
                    env_vars.insert("app_media_info".to_string(), value.to_string());
                }
            }
        }

        Ok(env_vars)
    }

    fn docker_runtime_labels(&self, desc: &SubPkgDesc) -> Vec<(String, String)> {
        let mut labels = vec![
            (DOCKER_LABEL_APP_ID.to_string(), self.app_id.clone()),
            (
                DOCKER_LABEL_OWNER_USER_ID.to_string(),
                self.owner_user_id.clone(),
            ),
            (DOCKER_LABEL_FULL_APPID.to_string(), self.full_appid()),
        ];
        if let Some(pkg_objid) = desc.pkg_objid.as_ref() {
            labels.push((DOCKER_LABEL_PKG_OBJID.to_string(), pkg_objid.to_string()));
        }
        if let Some(digest) = normalize_digest(desc.docker_image_digest.as_deref()) {
            labels.push((DOCKER_LABEL_IMAGE_DIGEST.to_string(), digest.to_string()));
        }
        labels
    }

    async fn has_current_docker_container_name(&self) -> Result<bool> {
        let container_name = self.full_appid();
        let output = run_command(
            "docker",
            &[
                "container".to_string(),
                "ls".to_string(),
                "-a".to_string(),
                "--format".to_string(),
                "{{.Names}}".to_string(),
                "--filter".to_string(),
                format!("name={container_name}"),
            ],
            None,
            None,
        )
        .await?;
        ensure_success("docker container ls -a", &output)?;
        Ok(container_list_contains_name(
            output.stdout.as_str(),
            container_name.as_str(),
        ))
    }

    async fn inspect_current_docker_container(&self) -> Result<Option<DockerContainerRuntime>> {
        let container_name = self.full_appid();
        let inspect_output = run_command(
            "docker",
            &[
                "container".to_string(),
                "inspect".to_string(),
                "--format={{json .}}".to_string(),
                container_name.clone(),
            ],
            None,
            None,
        )
        .await?;
        if !inspect_output.status.success() {
            if matches!(self.has_current_docker_container_name().await, Ok(false)) {
                return Ok(None);
            }
            return Err(ControlRuntItemErrors::ExecuteError(
                "docker container inspect".to_string(),
                format_command_failure("docker container inspect", &inspect_output),
            ));
        }

        let inspect_doc = parse_docker_container_inspect(inspect_output.stdout.trim())?;
        let image_id = inspect_doc.image.as_deref().and_then(trim_to_option);
        let repo_digests = match image_id.as_deref() {
            Some(image_id) => self.inspect_docker_image_repo_digests(image_id).await?,
            None => Vec::new(),
        };

        Ok(Some(DockerContainerRuntime {
            running: inspect_doc.state.running,
            identity: DockerRuntimeIdentity {
                image_id,
                repo_digests,
                labels: inspect_doc.config.labels.unwrap_or_default(),
            },
        }))
    }

    async fn inspect_docker_image_repo_digests(&self, image_ref: &str) -> Result<Vec<String>> {
        let output = run_command(
            "docker",
            &[
                "image".to_string(),
                "inspect".to_string(),
                "--format={{json .RepoDigests}}".to_string(),
                image_ref.to_string(),
            ],
            None,
            None,
        )
        .await?;
        if docker_object_missing(&output) {
            return Ok(Vec::new());
        }
        ensure_success("docker image inspect RepoDigests", &output)?;
        let raw = output.stdout.trim();
        if raw.is_empty() || raw == "null" {
            return Ok(Vec::new());
        }
        serde_json::from_str::<Vec<String>>(raw).map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "docker image inspect RepoDigests".to_string(),
                format!("parse docker repo digests failed: {}", error),
            )
        })
    }

    async fn ensure_pkg_meta_indexed(&self, env: &PackageEnv, pkg_id: &str) -> Result<()> {
        let package_id = PackageId::parse(pkg_id).map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!("parse pkg id {} failed: {}", pkg_id, error),
            )
        })?;
        let Some(meta_obj_id_str) = package_id.objid.as_deref() else {
            return Ok(());
        };
        if env.get_pkg_meta(pkg_id).await.is_ok() {
            return Ok(());
        }

        let runtime = get_buckyos_api_runtime().map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!(
                    "buckyos runtime unavailable when indexing {}: {}",
                    pkg_id, error
                ),
            )
        })?;
        let store_mgr = runtime.get_named_store().await.map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!("get named store for {} failed: {}", pkg_id, error),
            )
        })?;
        let meta_obj_id = ObjId::new(meta_obj_id_str).map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!("parse pkg objid {} failed: {}", meta_obj_id_str, error),
            )
        })?;
        let pkg_meta_str = store_mgr.get_object(&meta_obj_id).await.map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!(
                    "load pkg meta {} from named store failed: {}",
                    meta_obj_id, error
                ),
            )
        })?;
        let meta_obj_id_string = meta_obj_id.to_string();
        let mut pkg_meta =
            parse_package_meta_from_store(meta_obj_id_string.as_str(), &pkg_meta_str)?;
        let expected_pkg_name = expected_env_pkg_name(env, &package_id);
        if pkg_meta.name != expected_pkg_name {
            pkg_meta.name = expected_pkg_name;
        }
        env.set_pkg_meta_to_index_db(meta_obj_id_string.as_str(), &pkg_meta)
            .await
            .map_err(|error| {
                ControlRuntItemErrors::ExecuteError(
                    "index_pkg_meta".to_string(),
                    format!(
                        "insert pkg meta {} into env db failed: {}",
                        meta_obj_id, error
                    ),
                )
            })?;
        info!("indexed pkg meta {} for app {}", pkg_id, self.app_id);
        Ok(())
    }

    fn preview_docker_deploy(&self) -> Result<Vec<CommandSpec>> {
        let desc = self
            .docker_image_desc()
            .ok_or_else(|| self.pkg_not_found("docker image"))?;
        let image_name = desc.docker_image_name.clone().ok_or_else(|| {
            ControlRuntItemErrors::ParserConfigError(format!(
                "docker_image_name is missing for app {}",
                self.app_id
            ))
        })?;

        let mut commands = Vec::new();
        if let Some(pkg_id) = desc.get_pkg_id_with_objid() {
            commands.push(CommandSpec::new("pkg-install", [pkg_id]));
            commands.push(CommandSpec::new(
                "docker",
                vec![
                    "load".to_string(),
                    "-i".to_string(),
                    format!(
                        "<pkg_media>/{}",
                        docker_image_tar_candidates_for_arch(&self.app_id, self.platform.arch)[0]
                    ),
                ],
            ));
        }
        commands.push(CommandSpec::new(
            "docker",
            vec![
                "pull".to_string(),
                self.preview_docker_pull_ref(image_name.as_str(), desc),
            ],
        ));
        Ok(commands)
    }

    fn preview_docker_start(&self) -> Result<Vec<CommandSpec>> {
        let desc = self
            .docker_image_desc()
            .ok_or_else(|| self.pkg_not_found("docker image"))?;
        let image_name = desc.docker_image_name.clone().ok_or_else(|| {
            ControlRuntItemErrors::ParserConfigError(format!(
                "docker_image_name is missing for app {}",
                self.app_id
            ))
        })?;

        let mut docker_run_args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            self.full_appid(),
        ];
        append_host_gateway_run_args(&mut docker_run_args);

        let mut service_ports = self.service_ports_config().into_iter().collect::<Vec<_>>();
        service_ports.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
        for (service_name, instance_port) in service_ports {
            let container_port = self
                .app_doc()
                .install_config_tips
                .service_ports
                .get(service_name.as_str())
                .copied()
                .ok_or_else(|| {
                    ControlRuntItemErrors::ParserConfigError(format!(
                        "service {} port mapping missing in app_doc for {}",
                        service_name, self.app_id
                    ))
                })?;
            docker_run_args.push("-p".to_string());
            docker_run_args.push(format!("{instance_port}:{container_port}"));
        }

        for env_key in self.preview_env_keys(PackageRole::DockerImage) {
            docker_run_args.push("-e".to_string());
            docker_run_args.push(format!("{env_key}=<value>"));
        }

        for (key, value) in self.docker_runtime_labels(desc) {
            docker_run_args.push("--label".to_string());
            docker_run_args.push(format!("{key}={value}"));
        }

        docker_run_args.push(image_name);

        Ok(vec![
            CommandSpec::new("docker", ["rm", "-f", self.full_appid().as_str()]),
            CommandSpec::new("docker", docker_run_args),
        ])
    }

    fn preview_docker_status(&self) -> Vec<CommandSpec> {
        let filter = format!("name=^{}$", self.full_appid());
        let image_name = self
            .docker_image_desc()
            .and_then(|desc| desc.docker_image_name.clone())
            .unwrap_or_else(|| "<missing-image>".to_string());
        vec![
            CommandSpec::new("docker", ["ps", "-q", "-f", filter.as_str()]),
            CommandSpec::new("docker", ["ps", "-aq", "-f", filter.as_str()]),
            CommandSpec::new("docker", ["images", "-q", image_name.as_str()]),
        ]
    }

    fn preview_host_script_deploy(&self) -> Result<Vec<CommandSpec>> {
        let pkg_id = self
            .host_script_pkg_id()
            .ok_or_else(|| self.pkg_not_found("script or host app package"))?;
        let mut commands = vec![CommandSpec::new("pkg-install", [pkg_id])];
        commands.push(CommandSpec::new(
            "docker",
            ["pull", self.worker_image_name().as_str()],
        ));
        Ok(commands)
    }

    fn preview_host_script_start(&self) -> Result<Vec<CommandSpec>> {
        self.preview_worker_start("script", None, self.host_script_desc())
    }

    fn preview_host_script_status(&self) -> Vec<CommandSpec> {
        self.preview_worker_status()
    }

    fn preview_agent_deploy(&self) -> Result<Vec<CommandSpec>> {
        let pkg_id = self
            .agent_pkg_id()
            .ok_or_else(|| self.pkg_not_found("agent package"))?;
        let mut commands = vec![CommandSpec::new("pkg-install", [pkg_id])];
        if let Some(skills_pkg_id) = self.agent_skills_pkg_id() {
            commands.push(CommandSpec::new("pkg-install", [skills_pkg_id]));
        }
        commands.push(CommandSpec::new(
            "docker",
            ["pull", self.worker_image_name().as_str()],
        ));
        Ok(commands)
    }

    fn preview_agent_start(&self) -> Result<Vec<CommandSpec>> {
        let service_port = self.select_agent_service_port();
        self.preview_worker_start("agent", Some(service_port), self.agent_desc())
    }

    fn preview_agent_stop(&self) -> CommandSpec {
        CommandSpec::new("docker", ["rm", "-f", self.full_appid().as_str()])
    }

    fn preview_agent_status(&self) -> Vec<CommandSpec> {
        self.preview_worker_status()
    }

    fn preview_worker_start(
        &self,
        app_type_label: &str,
        service_port: Option<u16>,
        desc: Option<&SubPkgDesc>,
    ) -> Result<Vec<CommandSpec>> {
        let image_name = self.worker_image_name();
        let instance_volume = self.instance_volume_name();
        let mut docker_run_args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            self.full_appid(),
        ];
        append_host_gateway_run_args(&mut docker_run_args);

        docker_run_args.push("-v".to_string());
        docker_run_args.push(format!(
            "{}:{}:rw",
            instance_volume, WORKER_CONTAINER_INSTANCE_VOLUME_ROOT
        ));
        docker_run_args.push("-v".to_string());
        docker_run_args.push(format!(
            "<app_pkg>:{}:ro",
            WORKER_CONTAINER_PKG_SOURCE_ROOT
        ));
        docker_run_args.push("-v".to_string());
        docker_run_args.push(format!("<app_logs>:{}:rw", WORKER_CONTAINER_LOG_ROOT));
        docker_run_args.push("-v".to_string());
        docker_run_args.push(format!(
            "<app_storage>:{}:rw",
            WORKER_CONTAINER_STORAGE_ROOT
        ));
        docker_run_args.push("-v".to_string());
        docker_run_args.push(format!(
            "<app_data>:/home/{}/.local/share/{}:rw",
            self.owner_user_id, self.app_id
        ));

        if app_type_label == "agent" {
            if let Some(port) = service_port {
                docker_run_args.push("-p".to_string());
                docker_run_args.push(format!("{port}:{port}"));
            }
        }

        let package_role = if app_type_label == "agent" {
            PackageRole::AgentPkg
        } else {
            PackageRole::HostApp
        };
        for env_key in self.preview_env_keys(package_role) {
            docker_run_args.push("-e".to_string());
            docker_run_args.push(format!("{env_key}=<value>"));
        }
        docker_run_args.push("-e".to_string());
        docker_run_args.push(format!("BUCKYOS_APP_ID={}", self.app_id));
        docker_run_args.push("-e".to_string());
        docker_run_args.push(format!("BUCKYOS_APP_TYPE={}", app_type_label));
        if let Some(port) = service_port {
            docker_run_args.push("-e".to_string());
            docker_run_args.push(format!("BUCKYOS_SERVICE_PORT={port}"));
        }

        if let Some(desc) = desc {
            for (key, value) in self.docker_runtime_labels(desc) {
                docker_run_args.push("--label".to_string());
                docker_run_args.push(format!("{key}={value}"));
            }
        }

        docker_run_args.push(image_name);

        Ok(vec![
            CommandSpec::new("docker", ["rm", "-f", self.full_appid().as_str()]),
            CommandSpec::new("docker", docker_run_args),
        ])
    }

    fn preview_worker_status(&self) -> Vec<CommandSpec> {
        let filter = format!("name=^{}$", self.full_appid());
        vec![
            CommandSpec::new("docker", ["ps", "-q", "-f", filter.as_str()]),
            CommandSpec::new("docker", ["ps", "-aq", "-f", filter.as_str()]),
            CommandSpec::new("docker", ["images", "-q", self.worker_image_name().as_str()]),
        ]
    }

    fn preview_env_keys(&self, role: PackageRole) -> Vec<String> {
        let mut keys = vec![
            "BUCKYOS_ZONE_CONFIG".to_string(),
            "BUCKYOS_THIS_DEVICE_INFO".to_string(),
            "BUCKYOS_THIS_DEVICE".to_string(),
            "BUCKYOS_HOST_GATEWAY".to_string(),
        ];
        if self.media_pkg_id(role).is_some() {
            keys.push("app_media_info".to_string());
        }

        match &self.config {
            LoaderConfig::Service(_) => {
                keys.push("app_instance_config".to_string());
                keys.push(get_session_token_env_key(self.full_appid().as_str(), true));
            }
            LoaderConfig::Local(_) => {
                keys.push("local_app_instance_config".to_string());
                keys.push("loca_app_instance_config".to_string());
            }
        }
        keys
    }

    fn preview_docker_pull_ref(&self, image_name: &str, desc: &SubPkgDesc) -> String {
        let image_ref = match self.zone_docker_repo_base_url() {
            Some(base) => format!("{}/{}", base.trim_end_matches('/'), image_name),
            None => image_name.to_string(),
        };
        if let Some(digest) = normalize_digest(desc.docker_image_digest.as_deref()) {
            format!("{image_ref}@{digest}")
        } else {
            image_ref
        }
    }
}

#[cfg(test)]
impl AppLoader {
    pub(crate) fn test_worker_image_name(&self) -> String {
        self.worker_image_name()
    }

    pub(crate) fn test_instance_volume_name(&self) -> String {
        self.instance_volume_name()
    }
}

pub(crate) fn docker_image_tar_candidates_for_arch(
    app_id: &str,
    arch: PlatformArch,
) -> Vec<String> {
    let mut candidates = vec![format!("{app_id}.tar")];
    if arch == PlatformArch::Aarch64 {
        candidates.push("aarch64_docker_image.tar".to_string());
        candidates.push("amd64_docker_image.tar".to_string());
    } else {
        candidates.push("amd64_docker_image.tar".to_string());
        candidates.push("aarch64_docker_image.tar".to_string());
    }
    candidates
}

pub(crate) fn docker_image_tar_candidates(app_id: &str) -> Vec<String> {
    docker_image_tar_candidates_for_arch(app_id, PlatformTarget::current().arch)
}

pub(crate) fn normalize_digest(digest: Option<&str>) -> Option<&str> {
    digest.and_then(|value| {
        let normalized = value.trim();
        if normalized.is_empty() {
            None
        } else if let Some((_, digest)) = normalized.split_once('@') {
            Some(digest)
        } else {
            Some(normalized)
        }
    })
}

pub(crate) fn parse_package_meta_from_store(
    meta_obj_id: &str,
    pkg_meta_str: &str,
) -> Result<PackageMeta> {
    PackageMeta::from_str(pkg_meta_str).or_else(|_| {
        let pkg_meta_json = serde_json::from_str::<Value>(pkg_meta_str)
            .or_else(|_| load_named_object_from_obj_str(pkg_meta_str))
            .map_err(|error| {
                ControlRuntItemErrors::ExecuteError(
                    "index_pkg_meta".to_string(),
                    format!(
                        "parse pkg meta {} from named store failed: {}",
                        meta_obj_id, error
                    ),
                )
            })?;
        serde_json::from_value::<PackageMeta>(pkg_meta_json).map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!("decode pkg meta {} failed: {}", meta_obj_id, error),
            )
        })
    })
}

pub(crate) fn expected_env_pkg_name(env: &PackageEnv, package_id: &PackageId) -> String {
    if package_id.name.contains('.') {
        package_id.name.clone()
    } else {
        format!("{}.{}", env.get_prefix(), package_id.name)
    }
}

pub(crate) fn docker_desc_requires_exact_match(desc: &SubPkgDesc) -> bool {
    desc.pkg_objid.is_some() || normalize_digest(desc.docker_image_digest.as_deref()).is_some()
}

pub(crate) fn docker_runtime_matches_target(
    identity: &DockerRuntimeIdentity,
    desc: &SubPkgDesc,
) -> bool {
    if let Some(expected_pkg_objid) = desc.pkg_objid.as_ref() {
        let Some(actual_pkg_objid) = identity.labels.get(DOCKER_LABEL_PKG_OBJID) else {
            return false;
        };
        if actual_pkg_objid != expected_pkg_objid.to_string().as_str() {
            return false;
        }
    }

    let Some(expected_digest) = normalize_digest(desc.docker_image_digest.as_deref()) else {
        return true;
    };

    if identity
        .labels
        .get(DOCKER_LABEL_IMAGE_DIGEST)
        .map(String::as_str)
        == Some(expected_digest)
    {
        return true;
    }

    if identity.repo_digests.iter().any(|repo_digest| {
        repo_digest
            .split_once('@')
            .map(|(_, digest)| digest == expected_digest)
            .unwrap_or(false)
    }) {
        return true;
    }

    identity
        .image_id
        .as_deref()
        .map(|image_id| docker_image_id_matches_digest(image_id, expected_digest))
        .unwrap_or(false)
}

pub(crate) fn command_matches_agent_process(
    cmd: &[String],
    app_id: &str,
    agent_env_root: &Path,
) -> bool {
    if command_arg_value(cmd, "--agent-id") != Some(app_id) {
        return false;
    }

    if let Some(agent_env) = command_arg_value(cmd, "--agent-env") {
        return path_matches_value(agent_env_root, agent_env);
    }

    true
}

pub(crate) fn command_matches_exact_agent_process(
    cmd: &[String],
    app_id: &str,
    agent_env_root: &Path,
    expected_agent_root: Option<&Path>,
    expected_pkg_objid: Option<&str>,
) -> bool {
    if !command_matches_agent_process(cmd, app_id, agent_env_root) {
        return false;
    }

    let Some(agent_bin) = command_arg_value(cmd, "--agent-bin") else {
        return false;
    };

    if let Some(expected_agent_root) = expected_agent_root {
        if path_matches_value(expected_agent_root, agent_bin) {
            return true;
        }
    }

    expected_pkg_objid
        .map(|pkg_objid| normalize_path_value(agent_bin).contains(pkg_objid))
        .unwrap_or(false)
}

fn parse_mount_value(value: &str) -> (&str, Option<&'static str>) {
    if let Some(path) = value.strip_suffix(":ro") {
        return (path, Some("ro"));
    }
    if let Some(path) = value.strip_suffix(":rw") {
        return (path, Some("rw"));
    }
    (value, None)
}

fn default_mount_permission(
    container_path: &str,
    app_id: &str,
    owner_user_id: &str,
) -> &'static str {
    let path = trim_trailing_slash(container_path);
    let app_data = format!("/home/{owner_user_id}/.local/share/{app_id}");
    if path == app_data || path.starts_with(format!("{app_data}/").as_str()) {
        return "rw";
    }

    let shared = format!("/home/{owner_user_id}/shared");
    if path == "/srv/publish"
        || path.starts_with("/srv/publish/")
        || path == shared
        || path.starts_with(format!("{shared}/").as_str())
        || path == format!("/home/{owner_user_id}")
        || path.starts_with(format!("/home/{owner_user_id}/").as_str())
    {
        return "ro";
    }

    "rw"
}

fn trim_trailing_slash(value: &str) -> &str {
    value.trim_end_matches('/')
}

fn trim_path_separators(value: &str) -> &str {
    value.trim_end_matches(|ch| ch == '/' || ch == '\\')
}

fn canonicalize_mount_path(path: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        return path.to_path_buf();
    }
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn normalize_path_value(value: &str) -> String {
    let normalized = if Path::new(value).is_absolute() {
        canonicalize_mount_path(Path::new(value))
            .to_string_lossy()
            .to_string()
    } else {
        value.to_string()
    };
    let trimmed = trim_path_separators(normalized.as_str());
    if cfg!(target_os = "windows") {
        trimmed.to_ascii_lowercase()
    } else {
        trimmed.to_string()
    }
}

fn path_matches_value(expected_path: &Path, actual_value: &str) -> bool {
    normalize_path_value(expected_path.to_string_lossy().as_ref())
        == normalize_path_value(actual_value)
}

fn ensure_directory(path: &Path, make_world_writable: bool) -> Result<()> {
    fs::create_dir_all(path).map_err(|error| {
        ControlRuntItemErrors::ExecuteError(
            "mkdir".to_string(),
            format!("create directory {} failed: {}", path.display(), error),
        )
    })?;

    #[cfg(unix)]
    if make_world_writable {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o777);
        fs::set_permissions(path, permissions).map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "chmod".to_string(),
                format!("set permissions for {} failed: {}", path.display(), error),
            )
        })?;
    }

    Ok(())
}

fn split_shell_words(input: &str) -> Result<Vec<String>> {
    let words = Shlex::new(input).collect::<Vec<_>>();
    if words.is_empty() && !input.trim().is_empty() {
        return Err(ControlRuntItemErrors::ParserConfigError(format!(
            "parse shell words failed for `{}`",
            input
        )));
    }
    Ok(words)
}

fn append_host_gateway_run_args(args: &mut Vec<String>) {
    args.push("--add-host".to_string());
    args.push(format!("{AGENT_RUNTIME_HOST_GATEWAY}:host-gateway"));
}

struct CommandOutput {
    command_line: String,
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn render_command_for_log(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn run_command(
    program: &str,
    args: &[String],
    envs: Option<&HashMap<String, String>>,
    cwd: Option<&Path>,
) -> Result<CommandOutput> {
    let command_line = render_command_for_log(program, args);
    if program == "docker" && matches!(args.first().map(String::as_str), Some("run")) {
        info!("executing docker run: {}", command_line);
    }

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

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(windows_hidden_process_creation_flags());
    }

    let output = cmd.output().await.map_err(|error| {
        ControlRuntItemErrors::ExecuteError(
            program.to_string(),
            format!("spawn {} failed: {} (cmd={})", program, error, command_line),
        )
    })?;

    Ok(CommandOutput {
        command_line,
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[cfg(target_os = "windows")]
fn windows_hidden_process_creation_flags() -> u32 {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
}

fn ensure_success(step: &str, output: &CommandOutput) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    Err(ControlRuntItemErrors::ExecuteError(
        step.to_string(),
        format_command_failure(step, output),
    ))
}

fn format_command_failure(step: &str, output: &CommandOutput) -> String {
    let code = output
        .status
        .code()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    format!(
        "{} failed (code={}): cmd=`{}` stdout=`{}` stderr=`{}`",
        step,
        code,
        output.command_line,
        output.stdout.trim(),
        output.stderr.trim()
    )
}

pub(crate) fn resolve_aios_image_repo_from_paths<I, P>(paths: I) -> Option<String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut visited = HashSet::new();

    for candidate in paths {
        let candidate = candidate.as_ref();
        let path_key = candidate.to_string_lossy().to_string();
        if !visited.insert(path_key) || !candidate.is_file() {
            continue;
        }

        match parse_aios_image_repo_from_devenv(candidate) {
            Ok(Some(image_repo)) => return Some(image_repo),
            Ok(None) => {}
            Err(error) => warn!(
                "ignore invalid {} at {}: {}",
                DEVENV_JSON_FILE_NAME,
                candidate.display(),
                error
            ),
        }
    }

    None
}

fn resolve_devenv_aios_image_repo() -> Option<String> {
    resolve_aios_image_repo_from_paths([devenv_json_path()])
}

fn parse_aios_image_repo_from_devenv(path: &Path) -> std::result::Result<Option<String>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read {} failed: {}", path.display(), error))?;
    let value = serde_json::from_str::<Value>(raw.as_str())
        .map_err(|error| format!("parse {} failed: {}", path.display(), error))?;

    Ok(value
        .get(DEVENV_JSON_AIOS_KEY)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn devenv_json_path() -> PathBuf {
    get_buckyos_root_dir()
        .join("etc")
        .join(DEVENV_JSON_FILE_NAME)
}

fn trim_to_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn docker_missing_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("no such object")
        || lower.contains("no such image")
        || lower.contains("no such container")
}

fn docker_object_missing(output: &CommandOutput) -> bool {
    docker_missing_text(&output.stderr) || docker_missing_text(&output.stdout)
}

pub(crate) fn parse_docker_container_inspect(raw: &str) -> Result<DockerContainerInspectDoc> {
    serde_json::from_str::<DockerContainerInspectDoc>(raw).map_err(|error| {
        ControlRuntItemErrors::ExecuteError(
            "docker container inspect".to_string(),
            format!("parse docker inspect result failed: {}", error),
        )
    })
}

pub(crate) fn container_list_contains_name(raw: &str, expected_name: &str) -> bool {
    raw.lines().map(str::trim).any(|name| name == expected_name)
}

fn docker_image_id_matches_digest(image_id: &str, expected_digest: &str) -> bool {
    if image_id == expected_digest {
        return true;
    }
    if let (Some((_, image_hash)), Some((_, expected_hash))) =
        (image_id.split_once(':'), expected_digest.split_once(':'))
    {
        return image_hash == expected_hash;
    }
    false
}

fn command_arg_value<'a>(cmd: &'a [String], key: &str) -> Option<&'a str> {
    for (index, arg) in cmd.iter().enumerate() {
        if arg == key {
            return cmd.get(index + 1).map(String::as_str);
        }
        let prefix = format!("{key}=");
        if let Some(value) = arg.strip_prefix(prefix.as_str()) {
            return Some(value);
        }
    }
    None
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
