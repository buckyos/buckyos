//! Agent Runtime (§7): the execution environment of `exec` (host, file
//! system view, PATH layers, native vs tmux). Bound on the first drive and
//! never changed afterwards.

pub mod bin_overlay;
mod native;
pub mod tmux;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_tool::exec_tracking::{ExecutionRecord, ExecutionRegistrar};
use agent_tool::llm_bash::BashRunner;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::protocol::*;
use crate::session::SessionDir;

pub use bin_overlay::{BinPlan, ToolPlan};
pub use native::{local_host_id as native_host_id, NativeRuntime};
pub use tmux::TmuxRuntime;

/// What the environment of one session looks like (§7.3).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEnvCtx {
    pub session_id: String,
    pub session_dir: PathBuf,
    pub agent_did: String,
    #[serde(default)]
    pub agent_root: Option<PathBuf>,
    #[serde(default)]
    pub input_queue: Option<String>,
    #[serde(default)]
    pub trace_id: String,
    /// `session_config.runtime.env`.
    #[serde(default)]
    pub extra_env: Vec<(String, String)>,
}

/// An opened execution view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEnv {
    pub runtime_id: String,
    pub kind: String,
    pub workdir: PathBuf,
    /// PATH layers, `.runtime/bin` first.
    pub path_layers: Vec<PathBuf>,
    pub env: Vec<(String, String)>,
}

#[async_trait]
pub trait AgentRuntime: Send + Sync {
    fn descriptor(&self) -> &RuntimeDescriptor;
    /// Host id recorded in execution records.
    fn host_id(&self) -> &str;
    /// Where exec runs for this session: the workspace when one is declared,
    /// else the session directory.
    fn resolve_workdir(
        &self,
        sd: &SessionDir,
        workspace: Option<&WorkspaceRef>,
        agent_root: Option<&Path>,
    ) -> Result<PathBuf>;
    fn can_access(&self, path: &Path) -> bool;
    /// Executables available in this runtime (for `requirement.tools`).
    fn has_tool(&self, name: &str) -> bool;
    /// Idempotently repair `<sid>/.runtime/bin` (tombstones, helpers).
    async fn prepare_session_bin(&self, sd: &SessionDir, plan: &BinPlan) -> Result<()>;
    /// Verify cwd / PATH / tools / tombstones before any command runs.
    async fn verify_session_env(&self, sd: &SessionDir, binding: &Binding, plan: &BinPlan)
        -> Result<()>;
    async fn open_session_env(&self, binding: &Binding, ctx: &SessionEnvCtx) -> Result<SessionEnv>;
    /// The `exec` runner: launch handshake + execution identities persisted
    /// through `registrar` before the command runs.
    fn bash_runner(&self, env: &SessionEnv, registrar: Arc<dyn ExecutionRegistrar>)
        -> Arc<dyn BashRunner>;
    /// Confirm an old execution stopped (terminate verified leftovers and
    /// wait). Unverifiable → `RecoveryBlocked`.
    async fn reconcile_execution(&self, rec: &ExecutionRecord) -> Result<()>;
    /// For the prompt environment (`runtime.status`).
    async fn status(&self) -> Value;
    async fn close_session_env(&self, _env: SessionEnv) -> Result<()> {
        Ok(())
    }
}

/// Check `runtime.requirement` against the runtime (every drive).
pub fn check_requirement(
    req: &RuntimeRequirement,
    rt: &dyn AgentRuntime,
    app_tools: &[String],
) -> Result<()> {
    if let Some(id) = &req.runtime_id {
        if id != &rt.descriptor().runtime_id {
            return Err(OpenDanError::RuntimeMismatch {
                bound: id.clone(),
                provided: rt.descriptor().runtime_id.clone(),
            });
        }
    }
    let missing: Vec<&String> = req.tools.iter().filter(|t| !rt.has_tool(t)).collect();
    if !missing.is_empty() {
        return Err(OpenDanError::Bind(format!(
            "runtime {} lacks required tools: {}",
            rt.descriptor().runtime_id,
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let missing_app: Vec<&String> = req
        .app_tools
        .iter()
        .filter(|t| !app_tools.contains(t))
        .collect();
    if !missing_app.is_empty() {
        return Err(OpenDanError::Bind(format!(
            "runner does not provide required app tools: {}",
            missing_app
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(())
}

/// Bind on first drive, verify on every later drive (§7.1). Failures happen
/// before any inference (Q3). `binding.json` only proves the identity; the
/// environment is repaired and verified every time.
pub async fn bind_or_verify(
    sd: &SessionDir,
    lease: &crate::lock::Lease,
    rt: &dyn AgentRuntime,
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    app_tools: &[String],
    plan: &BinPlan,
    runner_id: &str,
) -> Result<Binding> {
    let existing = sd.binding_opt()?;
    let desc = rt.descriptor();
    let binding = match &existing {
        Some(b) => {
            if b.runtime_id != desc.runtime_id {
                return Err(OpenDanError::RuntimeMismatch {
                    bound: b.runtime_id.clone(),
                    provided: desc.runtime_id.clone(),
                });
            }
            b.clone()
        }
        None => {
            let workdir = rt.resolve_workdir(sd, cfg.workspace.as_ref(), agent_root)?;
            Binding {
                runtime_id: desc.runtime_id.clone(),
                kind: desc.kind.clone(),
                workdir: workdir.display().to_string(),
                bound_at_ms: crate::now_ms(),
                bound_by: runner_id.to_string(),
            }
        }
    };
    check_requirement(&cfg.runtime.requirement, rt, app_tools)?;
    if !rt.can_access(Path::new(&binding.workdir)) {
        return Err(OpenDanError::Bind(format!(
            "runtime {} cannot access workdir {}",
            desc.runtime_id, binding.workdir
        )));
    }
    if existing.is_none() {
        let published = lease.fenced(|| {
            fsutil::publish_noreplace_json(&sd.file(BINDING_FILE), &binding)
        })?;
        if !published {
            let cur = sd
                .binding_opt()?
                .ok_or_else(|| OpenDanError::Bind("binding.json vanished".into()))?;
            if !cur.same_binding(&binding) {
                return Err(OpenDanError::RuntimeMismatch {
                    bound: cur.runtime_id,
                    provided: binding.runtime_id,
                });
            }
        }
    }
    let binding = sd.binding_opt()?.unwrap_or(binding);
    rt.prepare_session_bin(sd, plan).await?;
    rt.verify_session_env(sd, &binding, plan).await?;
    Ok(binding)
}

/// Build the Session Bin plan for a session (tool plan + helper CLI).
pub fn bin_plan_for(
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    session_cli: Option<PathBuf>,
    runtime_layers: &[PathBuf],
) -> Result<BinPlan> {
    let mut plan = BinPlan {
        session_cli,
        ..Default::default()
    };
    if let Some(root) = agent_root {
        plan.universe_dirs.push(root.join("tools"));
    }
    plan.universe_dirs.extend(runtime_layers.iter().cloned());
    if let (Some(name), Some(root)) = (&cfg.runtime.tool_plan, agent_root) {
        if let Some(tp) = ToolPlan::load(root, name)? {
            plan.tool_plan_name = Some(name.clone());
            plan.tool_plan = Some(tp);
        }
    }
    Ok(plan)
}

/// The §7.3 environment contract.
pub fn session_env_vars(ctx: &SessionEnvCtx, runtime_id: &str) -> Vec<(String, String)> {
    let mut v = vec![
        ("OPENDAN_AGENT_DID".to_string(), ctx.agent_did.clone()),
        (
            "OPENDAN_SESSION_DIR".to_string(),
            ctx.session_dir.display().to_string(),
        ),
        ("OPENDAN_SESSION_ID".to_string(), ctx.session_id.clone()),
        ("OPENDAN_TRACE_ID".to_string(), ctx.trace_id.clone()),
        ("OPENDAN_RUNTIME_ID".to_string(), runtime_id.to_string()),
    ];
    if let Some(r) = &ctx.agent_root {
        v.push(("OPENDAN_AGENT_ROOT".to_string(), r.display().to_string()));
    }
    if let Some(q) = &ctx.input_queue {
        v.push(("OPENDAN_INPUT_QUEUE".to_string(), q.clone()));
    }
    if let Ok(tok) = std::env::var("BUCKYOS_APPCLIENT_SESSION_TOKEN") {
        v.push(("BUCKYOS_APPCLIENT_SESSION_TOKEN".to_string(), tok));
    }
    v.extend(ctx.extra_env.iter().cloned());
    v
}
