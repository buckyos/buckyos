pub mod bin_overlay;
pub use agent_tool::runtime::tmux;
pub use agent_tool::runtime::{native_host_id, AgentRuntime, NativeRuntime, TmuxRuntime};
pub use bin_overlay::{BinPlan, ToolPlan};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use agent_tool::runtime::RuntimeOpenCtx;
use agent_tool::xllm::{LoopModel, ToolsConfig};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEnvCtx {
    pub session_id: String,
    pub session_dir: PathBuf,
    pub agent_did: String,
    pub agent_root: Option<PathBuf>,
    pub input_queue: Option<String>,
    pub trace_id: String,
    pub extra_env: Vec<(String, String)>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEnv {
    pub runtime_id: String,
    pub kind: String,
    pub workdir: PathBuf,
    pub path_layers: Vec<PathBuf>,
    pub env: Vec<(String, String)>,
}

pub fn resolve_workdir(
    sd: &SessionDir,
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
) -> Result<PathBuf> {
    let p = match &cfg.workspace {
        None => sd.path().to_path_buf(),
        Some(WorkspaceRef::External { path }) => {
            let p = PathBuf::from(path);
            if !p.is_absolute() {
                return Err(OpenDanError::Bind(
                    "external workspace must be absolute".into(),
                ));
            }
            p
        }
        Some(WorkspaceRef::Agent { id }) => {
            crate::ids::validate_session_id(id)?;
            let p = agent_root
                .ok_or_else(|| OpenDanError::Bind("agent workspace needs AgentRoot".into()))?
                .join("workspace")
                .join(id);
            std::fs::create_dir_all(&p).map_err(|e| OpenDanError::io(&p, e))?;
            p
        }
    };
    Ok(p.canonicalize().unwrap_or(p))
}

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
    if rt.config().kind() == "remote_ssh" {
        return Err(OpenDanError::Bind("Capability: remote Session helpers are not deployed; use SSH runtime with xllm independently".into()));
    }
    let fallback = resolve_workdir(sd, cfg, agent_root)?;
    let mut open = RuntimeOpenCtx::new(&fallback, sd.sid(), LoopModel::FunctionCall);
    let _ = rt
        .open(
            &open,
            &ToolsConfig {
                enabled: Some(false),
                ..Default::default()
            },
            &agent_tool::xllm::XllmDeps::default(),
        )
        .await
        .map_err(|e| OpenDanError::Bind(e.to_string()))?;
    let desc = rt.descriptor();
    let existing = sd.binding_opt()?;
    let binding = Binding {
        schema: "opendan.binding/3".into(),
        runtime_id: desc.runtime_id.clone(),
        kind: desc.kind.clone(),
        target: desc.target.clone(),
        workdir: desc.workdir.clone(),
        bound_at_ms: crate::now_ms(),
        bound_by: runner_id.into(),
    };
    if let Some(b) = &existing {
        if !b.same_binding(&binding) {
            return Err(OpenDanError::RuntimeMismatch {
                bound: format!("{} {} {} {}", b.runtime_id, b.kind, b.target, b.workdir),
                provided: format!(
                    "{} {} {} {}",
                    binding.runtime_id, binding.kind, binding.target, binding.workdir
                ),
            });
        }
    }
    if let Some(id) = &cfg.runtime.requirement.runtime_id {
        if id != &binding.runtime_id {
            return Err(OpenDanError::RuntimeMismatch {
                bound: id.clone(),
                provided: binding.runtime_id.clone(),
            });
        }
    }
    let missing_app = cfg
        .runtime
        .requirement
        .app_tools
        .iter()
        .filter(|n| !app_tools.contains(n))
        .collect::<Vec<_>>();
    if !missing_app.is_empty() {
        return Err(OpenDanError::Bind(format!(
            "runner lacks app tools: {missing_app:?}"
        )));
    }
    bin_overlay::prepare(&sd.runtime_bin_dir(), plan)?;
    bin_overlay::verify(&sd.runtime_bin_dir(), plan)?;
    open.path_prefix.push(sd.runtime_bin_dir());
    if let Some(root) = agent_root {
        if root.join("tools").is_dir() {
            open.path_prefix.push(root.join("tools"));
        }
    }
    let (_, sandbox) = rt
        .open(
            &open,
            &ToolsConfig {
                enabled: Some(false),
                ..Default::default()
            },
            &agent_tool::xllm::XllmDeps::default(),
        )
        .await
        .map_err(|e| OpenDanError::Bind(e.to_string()))?;
    for name in &cfg.runtime.requirement.tools {
        let result = agent_tool::runtime::Sandbox::exec(
            &sandbox,
            agent_tool::llm_bash::BashRunRequest {
                command: format!("command -v {}", agent_tool::runtime::shell_quote(name)),
                cwd: binding.workdir.clone().into(),
                timeout_ms: 5000,
                max_output_bytes: 4096,
                env: Vec::new(),
                target: agent_tool::llm_bash::BashTarget::Local,
                call_id: None,
            },
            &agent_tool::SessionRuntimeContext {
                trace_id: sd.sid().into(),
                agent_name: "libopendan".into(),
                behavior: "runtime_probe".into(),
                tool_call_index: 0,
                wakeup_id: String::new(),
                session_id: sd.sid().into(),
                read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
            },
        )
        .await
        .map_err(|e| OpenDanError::Bind(e.to_string()))?;
        if result.exit_code != 0 {
            return Err(OpenDanError::Bind(format!(
                "runtime lacks required tool {name}"
            )));
        }
    }
    if existing.is_none() {
        let published = lease
            .fenced(|| crate::fsutil::publish_noreplace_json(&sd.file(BINDING_FILE), &binding))?;
        if !published {
            let current = sd
                .binding_opt()?
                .ok_or_else(|| OpenDanError::Bind("binding vanished".into()))?;
            if !current.same_binding(&binding) {
                return Err(OpenDanError::RuntimeMismatch {
                    bound: current.runtime_id,
                    provided: binding.runtime_id,
                });
            }
        }
    }
    Ok(existing.unwrap_or(binding))
}

pub fn open_session_env(binding: &Binding, ctx: &SessionEnvCtx) -> SessionEnv {
    let mut layers = vec![ctx.session_dir.join(RUNTIME_DIR).join("bin")];
    if let Some(root) = &ctx.agent_root {
        if root.join("tools").is_dir() {
            layers.push(root.join("tools"));
        }
    }
    SessionEnv {
        runtime_id: binding.runtime_id.clone(),
        kind: binding.kind.clone(),
        workdir: binding.workdir.clone().into(),
        path_layers: layers,
        env: session_env_vars(ctx, &binding.runtime_id),
    }
}

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
