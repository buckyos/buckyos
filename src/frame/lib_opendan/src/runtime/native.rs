//! `NativeRuntime`: every command is one `/bin/bash -c` in its own process
//! group, started through the tracked launch handshake (the execution
//! identity is persisted before the command is released). xllm can take over
//! runs of native sessions.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_tool::exec_tracking::{
    stop_execution, ExecutionRecord, ExecutionRegistrar, TrackedBashRunner,
};
use agent_tool::llm_bash::BashRunner;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;

use super::{bin_overlay, session_env_vars, AgentRuntime, BinPlan, SessionEnv, SessionEnvCtx};

pub struct NativeRuntime {
    desc: RuntimeDescriptor,
    host: String,
    stop_wait: Duration,
}

/// Stable id of this machine (`host:<hostname>:<machine-id prefix>`).
pub fn local_host_id() -> String {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "localhost".to_string());
    let machine = std::fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|s| s.trim().chars().take(12).collect::<String>())
        .unwrap_or_default();
    if machine.is_empty() {
        format!("host:{hostname}")
    } else {
        format!("host:{hostname}:{machine}")
    }
}

fn which(name: &str, layers: &[PathBuf]) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    layers
        .iter()
        .cloned()
        .chain(path.split(':').filter(|s| !s.is_empty()).map(PathBuf::from))
        .any(|d| {
            let p = d.join(name);
            std::fs::metadata(&p)
                .map(|m| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        m.is_file() && m.permissions().mode() & 0o111 != 0
                    }
                    #[cfg(not(unix))]
                    {
                        m.is_file()
                    }
                })
                .unwrap_or(false)
        })
}

impl NativeRuntime {
    /// A native runtime on this host. `runtime_id` usually embeds the host
    /// (the session is then pinned to this machine, §7.1).
    pub fn local(runtime_id: &str, provider: &str) -> Self {
        let host = local_host_id();
        Self {
            desc: RuntimeDescriptor {
                runtime_id: runtime_id.to_string(),
                kind: "native".to_string(),
                host: Some(host.clone()),
                provider: provider.to_string(),
                fs_view: FsView::default(),
                path_layers: Vec::new(),
                capabilities: Capabilities {
                    tools: Default::default(),
                    network: true,
                    os: std::env::consts::OS.to_string(),
                },
            },
            host,
            stop_wait: Duration::from_secs(10),
        }
    }

    /// Extra PATH layers after `.runtime/bin` and the agent tools.
    pub fn with_path_layers(mut self, layers: Vec<String>) -> Self {
        self.desc.path_layers = layers;
        self
    }

    pub fn with_fs_view(mut self, view: FsView) -> Self {
        self.desc.fs_view = view;
        self
    }

    pub fn with_stop_wait(mut self, wait: Duration) -> Self {
        self.stop_wait = wait;
        self
    }

    fn runtime_layers(&self) -> Vec<PathBuf> {
        self.desc.path_layers.iter().map(PathBuf::from).collect()
    }
}

#[async_trait]
impl AgentRuntime for NativeRuntime {
    fn descriptor(&self) -> &RuntimeDescriptor {
        &self.desc
    }

    fn host_id(&self) -> &str {
        &self.host
    }

    fn resolve_workdir(
        &self,
        sd: &SessionDir,
        workspace: Option<&WorkspaceRef>,
        agent_root: Option<&Path>,
    ) -> Result<PathBuf> {
        let p = match workspace {
            None => sd.path().to_path_buf(),
            Some(WorkspaceRef::External { path }) => {
                let p = PathBuf::from(path);
                if !p.is_absolute() {
                    return Err(OpenDanError::Bind(format!(
                        "external workspace path must be absolute: {path}"
                    )));
                }
                p
            }
            Some(WorkspaceRef::Agent { id }) => {
                let root = agent_root.ok_or_else(|| {
                    OpenDanError::Bind("agent workspace needs a visible AgentRoot".into())
                })?;
                crate::ids::validate_session_id(id)
                    .map_err(|_| OpenDanError::Bind(format!("bad workspace id `{id}`")))?;
                let p = root.join("workspace").join(id);
                std::fs::create_dir_all(&p).map_err(|e| OpenDanError::io(&p, e))?;
                p
            }
        };
        Ok(p.canonicalize().unwrap_or(p))
    }

    fn can_access(&self, path: &Path) -> bool {
        path.is_dir() && std::fs::read_dir(path).is_ok()
    }

    fn has_tool(&self, name: &str) -> bool {
        which(name, &self.runtime_layers())
    }

    async fn prepare_session_bin(&self, sd: &SessionDir, plan: &BinPlan) -> Result<()> {
        bin_overlay::prepare(&sd.runtime_bin_dir(), plan).map(|_| ())
    }

    async fn verify_session_env(
        &self,
        sd: &SessionDir,
        binding: &Binding,
        plan: &BinPlan,
    ) -> Result<()> {
        if !self.can_access(Path::new(&binding.workdir)) {
            return Err(OpenDanError::Bind(format!(
                "workdir {} is not accessible",
                binding.workdir
            )));
        }
        bin_overlay::verify(&sd.runtime_bin_dir(), plan)
    }

    async fn open_session_env(&self, binding: &Binding, ctx: &SessionEnvCtx) -> Result<SessionEnv> {
        let mut layers = vec![ctx.session_dir.join(RUNTIME_DIR).join("bin")];
        if let Some(root) = &ctx.agent_root {
            layers.push(root.join("tools"));
        }
        layers.extend(self.runtime_layers());
        Ok(SessionEnv {
            runtime_id: self.desc.runtime_id.clone(),
            kind: "native".to_string(),
            workdir: PathBuf::from(&binding.workdir),
            path_layers: layers,
            env: session_env_vars(ctx, &self.desc.runtime_id),
        })
    }

    fn bash_runner(
        &self,
        env: &SessionEnv,
        registrar: Arc<dyn ExecutionRegistrar>,
    ) -> Arc<dyn BashRunner> {
        Arc::new(
            TrackedBashRunner::new(registrar)
                .with_runtime(Some(self.desc.runtime_id.clone()), Some(self.host.clone()))
                .with_path_layers(env.path_layers.clone())
                .with_env(env.env.clone()),
        )
    }

    async fn reconcile_execution(&self, rec: &ExecutionRecord) -> Result<()> {
        stop_execution(rec, Some(&self.host), self.stop_wait)
            .await
            .map_err(|reason| {
                OpenDanError::blocked(
                    format!(
                        "cannot confirm that execution {} (`{}`) stopped: {reason}",
                        rec.execution_id, rec.command
                    ),
                    None,
                )
            })
    }

    async fn status(&self) -> Value {
        json!({
            "runtime_id": self.desc.runtime_id,
            "kind": "native",
            "host": self.host,
            "os": std::env::consts::OS,
        })
    }
}
