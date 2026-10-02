use super::*;
use crate::exec_tracking::MemoryRegistrar;
use buckyos_api::AiToolCall;
use llm_context::deps::ToolManager;
use llm_context::prompt_engine::{EngineConfig, PromptRenderEngine, RenderVars};
use serde_json::json;

fn context() -> SessionRuntimeContext {
    SessionRuntimeContext {
        trace_id: "test".into(),
        agent_name: "test".into(),
        behavior: "test".into(),
        tool_call_index: 0,
        wakeup_id: String::new(),
        session_id: "test".into(),
        read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
    }
}
fn request(cwd: &str, command: &str) -> BashRunRequest {
    BashRunRequest {
        command: command.into(),
        cwd: cwd.into(),
        timeout_ms: 10000,
        max_output_bytes: 1024 * 64,
        env: Vec::new(),
        target: BashTarget::Local,
    }
}
fn call(name: &str, args: Value) -> AiToolCall {
    AiToolCall {
        call_id: format!("test-{name}"),
        name: name.into(),
        args: args
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}
async fn open(
    config: RuntimeConfig,
    workdir: &Path,
    policy: crate::xllm::FilesystemPolicy,
) -> (
    Arc<dyn AgentRuntime>,
    Arc<XllmToolManager>,
    Arc<MemoryRegistrar>,
) {
    let rt = RuntimeRegistry::from_config(&config).unwrap();
    let reg = Arc::new(MemoryRegistrar::default());
    let mut ctx = RuntimeOpenCtx::new(workdir, "test", LoopModel::FunctionCall);
    ctx.registrar = reg.clone();
    let (_, manager) = rt
        .open(
            &ctx,
            &ToolsConfig {
                enabled: Some(true),
                filesystem_policy: Some(policy),
                ..Default::default()
            },
            Vec::new(),
        )
        .await
        .unwrap();
    (rt, Arc::new(manager), reg)
}

#[test]
fn config_merge_resolves_source_paths_and_resets_kind() {
    let a=crate::xllm::parse_llm_context_file(Path::new("/one/.llm_context"),"runtime:\n  kind: tmux\n  workdir: work\n  env: {ONE: a}\n  tmux: {session: test, socket: sock}\n").unwrap();
    let b = crate::xllm::parse_llm_context_file(
        Path::new("/two/.llm_context"),
        "runtime:\n  env: {TWO: b}\n  tmux: {mode: attach}\n",
    )
    .unwrap();
    let merged = crate::xllm::merge_config_layers(&[
        crate::xllm::ConfigLayer {
            path: "/one/.llm_context".into(),
            file: a,
        },
        crate::xllm::ConfigLayer {
            path: "/two/.llm_context".into(),
            file: b,
        },
    ])
    .unwrap();
    assert_eq!(merged.runtime.workdir.as_deref(), Some("/one/work"));
    assert_eq!(
        merged.runtime.tmux.as_ref().unwrap().socket.as_deref(),
        Some("/one/sock")
    );
    assert_eq!(merged.runtime.env.len(), 2);
    let mut r = merged.runtime;
    r.merge_over(&RuntimeConfig {
        kind: Some("native".into()),
        ..Default::default()
    });
    assert!(r.tmux.is_none() && r.env.is_empty() && r.workdir.is_none());
    for raw in [
        "runtime: {kind: bad}",
        "runtime: {kind: native, limits: {}}",
        "runtime: {kind: native, tmux: {session: x}}",
    ] {
        assert!(matches!(
            crate::xllm::parse_llm_context_file(Path::new(".llm_context"), raw),
            Err(XllmError::Config { .. })
        ));
    }
    for kind in [
        "container",
        "container_host",
        "remote_node",
        "http_proxy_runtime",
    ] {
        assert!(matches!(
            RuntimeRegistry::from_config(&RuntimeConfig {
                kind: Some(kind.into()),
                ..Default::default()
            }),
            Err(XllmError::Capability(_))
        ));
    }
    assert!(RuntimeRegistry::from_config(&RuntimeConfig {
        kind: Some("remote_ssh".into()),
        remote_ssh: Some(SshConfig {
            host: Some("host".into()),
            ..Default::default()
        }),
        ..Default::default()
    })
    .is_err());
}

#[tokio::test]
async fn native_default_explicit_env_files_and_prompt_share_executor() {
    let dir = tempfile::tempdir().unwrap();
    let (default, _, _) = open(
        RuntimeConfig::default(),
        dir.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    let config = RuntimeConfig {
        kind: Some("native".into()),
        env: BTreeMap::from([("LLM_RUNTIME_TEST".into(), "base".into())]),
        ..Default::default()
    };
    let (rt, manager, reg) =
        open(config, dir.path(), crate::xllm::FilesystemPolicy::Workspace).await;
    assert_eq!(rt.descriptor().runtime_id, default.descriptor().runtime_id);
    assert_eq!(rt.descriptor().target, default.descriptor().target);
    let obs = manager
        .call_tool(call(
            "write_file",
            json!({"path":"quote ' 中文.txt","content":"literal $() 中文\n"}),
        ))
        .await
        .unwrap();
    assert!(
        !matches!(obs, llm_context::observation::Observation::Error { .. }),
        "{obs:?}"
    );
    let mut req = request(
        manager.workdir(),
        "printf '%s' \"$LLM_RUNTIME_TEST\"; cat \"quote ' 中文.txt\"",
    );
    req.env.push(("LLM_RUNTIME_TEST".into(), "once".into()));
    let output = manager.exec(req, &context()).await.unwrap();
    assert_eq!(output.stdout, "onceliteral $() 中文\n");
    let hosted = crate::xllm::XllmTask::prepare_hosted(
        dir.path(),
        &json!({"runtime": rt.config(), "tools": {"enabled": true}}),
        &dir.path().join("session_config.json").display().to_string(),
        "OS={{runtime.os}} HOST={{runtime.hostname}} CWD={{runtime.cwd}} TOOLS={{runtime.tools}}",
        &crate::xllm::XllmDeps::default().with_runtime(rt.clone()),
    )
    .await
    .unwrap();
    assert!(hosted
        .prompt
        .system_prompt
        .contains(&format!("HOST={}", rt.info().await.unwrap().hostname)));
    assert!(hosted.prompt.system_prompt.contains("read_file"));
    assert!(crate::xllm::XllmTask::prepare_hosted(
        dir.path(),
        &json!({"runtime": rt.config()}),
        "session_config.json",
        "{{runtime.current_time}}",
        &crate::xllm::XllmDeps::default().with_runtime(rt.clone())
    )
    .await
    .is_err());
    let info = rt.info().await.unwrap();
    let adapter = Arc::new(SandboxPromptExec {
        sandbox: manager.clone(),
        context: context(),
    });
    let engine = PromptRenderEngine::new(EngineConfig {
        allow_exec: true,
        executor: Some(adapter),
        ..Default::default()
    });
    let rendered = engine
        .render(
            "__ENV($runtime.hostname)__|__EXEC(uname -n)__",
            &RenderVars::new(),
            &RuntimeValueLoader(info.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        rendered.rendered,
        format!("{}|{}\n", info.hostname, info.hostname)
    );
    assert!(reg.records.lock().unwrap().len() >= 2);
    assert_eq!(
        reg.records.lock().unwrap().len(),
        reg.completed.lock().unwrap().len()
    );
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        let obs = manager
            .call_tool(call(
                "write_file",
                json!({"path":"escape/new","content":"bad"}),
            ))
            .await
            .unwrap();
        assert!(matches!(
            obs,
            llm_context::observation::Observation::Error { .. }
        ));
        assert!(!outside.path().join("new").exists());
    }
}

#[tokio::test]
async fn tmux_modes_dedicated_pane_and_serial_execution() {
    if !Runtime::available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("socket").display().to_string();
    let cfg = RuntimeConfig {
        kind: Some("tmux".into()),
        tmux: Some(TmuxConfig {
            session: Some("runtime-test".into()),
            socket: Some(socket.clone()),
            mode: Some(TmuxMode::Attach),
        }),
        ..Default::default()
    };
    let r = RuntimeRegistry::from_config(&cfg).unwrap();
    let ctx = RuntimeOpenCtx::new(dir.path(), "test", LoopModel::FunctionCall);
    assert!(r
        .open(&ctx, &ToolsConfig::default(), Vec::new())
        .await
        .is_err());
    let mut cfg = cfg;
    cfg.tmux.as_mut().unwrap().mode = Some(TmuxMode::Create);
    let (rt, m, reg) = open(
        cfg.clone(),
        dir.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    let original = CommandOutput::pane(&socket, "runtime-test:0.0");
    let o = m
        .exec(
            request(m.workdir(), "echo tmux; printf same > file"),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(o.stdout, "tmux\n");
    assert_eq!(o.engine, "tmux");
    let obs = m
        .call_tool(call("read_file", json!({"path":"file"})))
        .await
        .unwrap();
    assert!(format!("{obs:?}").contains("same"));
    assert_eq!(original, CommandOutput::pane(&socket, "runtime-test:0.0"));
    let bad = RuntimeRegistry::from_config(&cfg).unwrap();
    assert!(bad
        .open(&ctx, &ToolsConfig::default(), Vec::new())
        .await
        .is_err());
    cfg.tmux.as_mut().unwrap().mode = Some(TmuxMode::CreateOrAttach);
    let (second, _, _) = open(cfg, dir.path(), crate::xllm::FilesystemPolicy::Workspace).await;
    assert_eq!(rt.descriptor(), second.descriptor());
    assert_eq!(
        reg.records.lock().unwrap().len(),
        reg.completed.lock().unwrap().len()
    );
    let _ = std::process::Command::new("tmux")
        .args(["-S", &socket, "kill-server"])
        .status();
}
struct CommandOutput;
impl CommandOutput {
    fn pane(socket: &str, pane: &str) -> Vec<u8> {
        std::process::Command::new("tmux")
            .args(["-S", socket, "capture-pane", "-p", "-t", pane])
            .output()
            .unwrap()
            .stdout
    }
}

fn ssh_config() -> RuntimeConfig {
    RuntimeConfig {
        kind: Some("remote_ssh".into()),
        workdir: Some(
            std::env::var("LLM_RUNTIME_SSH_WORKDIR").expect("use test/runtime_ssh/run.sh"),
        ),
        remote_ssh: Some(SshConfig {
            host: Some(std::env::var("LLM_RUNTIME_SSH_HOST").expect("use test/runtime_ssh/run.sh")),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore]
async fn ssh_real_transport_files_paths_timeout_and_recovery() {
    let local = tempfile::tempdir().unwrap();
    std::fs::write(local.path().join("same.txt"), "local").unwrap();
    let (rt, m, reg) = open(
        ssh_config(),
        local.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    assert_ne!(m.workdir(), local.path().to_str().unwrap());
    let path = "a space ' 中文.txt";
    let content = "line one\nquotes ' \" and $(touch SHOULD_NOT_RUN)\n中文\n";
    let obs = m
        .call_tool(call("write_file", json!({"path":path,"content":content})))
        .await
        .unwrap();
    assert!(
        !matches!(obs, llm_context::observation::Observation::Error { .. }),
        "{obs:?}"
    );
    let obs = m
        .call_tool(call(
            "edit_file",
            json!({"path":path,"old_string":"line one","new_string":"updated"}),
        ))
        .await
        .unwrap();
    assert!(
        !matches!(obs, llm_context::observation::Observation::Error { .. }),
        "{obs:?}"
    );
    let output = m
        .exec(
            request(
                m.workdir(),
                &format!(
                    "cat -- {}; printf remote > same.txt; test ! -e SHOULD_NOT_RUN",
                    shell_quote(path)
                ),
            ),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(output.stdout, content.replacen("line one", "updated", 1));
    assert_eq!(output.exit_code, 0);
    assert_eq!(
        std::fs::read_to_string(local.path().join("same.txt")).unwrap(),
        "local"
    );
    let info = rt.info().await.unwrap();
    assert_eq!(info.cwd, m.workdir());
    assert_eq!(info.kind, "remote_ssh");
    let out = m
        .exec(request(m.workdir(), "uname -n"), &context())
        .await
        .unwrap();
    assert_eq!(out.stdout.trim(), info.hostname);
    let mut timeout = request(m.workdir(), "sleep 40; printf BAD > timed-out");
    timeout.timeout_ms = 700;
    let output = m.exec(timeout, &context()).await.unwrap();
    assert!(output.timed_out);
    let rec = reg.records.lock().unwrap().last().unwrap().clone();
    rt.reconcile_execution(&rec).await.unwrap();
    let out = m
        .exec(
            request(m.workdir(), "test ! -e timed-out; ln -sfn /tmp escape"),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, 0);
    let obs = m
        .call_tool(call(
            "write_file",
            json!({"path":"escape/outside-runtime-test","content":"denied"}),
        ))
        .await
        .unwrap();
    assert!(matches!(
        obs,
        llm_context::observation::Observation::Error { .. }
    ));
    let obs = m
        .call_tool(call("exec", json!({"command":"pwd","cwd":"/tmp"})))
        .await
        .unwrap();
    assert!(matches!(
        obs,
        llm_context::observation::Observation::Error { .. }
    ));
    let (_, unrestricted, _) = open(
        ssh_config(),
        local.path(),
        crate::xllm::FilesystemPolicy::Unrestricted,
    )
    .await;
    let obs = unrestricted
        .call_tool(call("exec", json!({"command":"pwd","cwd":"/tmp"})))
        .await
        .unwrap();
    assert!(
        !matches!(obs, llm_context::observation::Observation::Error { .. }),
        "{obs:?}"
    );
    let mut wrong = rec.clone();
    wrong.host = Some("different-host".into());
    assert!(matches!(
        rt.reconcile_execution(&wrong).await,
        Err(XllmError::RecoveryBlocked(_))
    ));
}

#[tokio::test]
#[ignore]
async fn ssh_cancel_failure_and_disconnect_are_explicit() {
    let local = tempfile::tempdir().unwrap();
    let (rt, m, reg) = open(
        ssh_config(),
        local.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    let cwd = m.workdir().to_string();
    let task_m = m.clone();
    let pending = tokio::spawn(async move {
        task_m
            .exec(
                request(
                    &cwd,
                    "echo started > cancel-start; sleep 40; echo bad > cancel-end",
                ),
                &context(),
            )
            .await
    });
    let remote = PathBuf::from(std::env::var("LLM_RUNTIME_SSH_WORKDIR").unwrap());
    wait_file(&remote.join("cancel-start")).await;
    pending.abort();
    let _ = pending.await;
    let rec = reg.records.lock().unwrap().last().unwrap().clone();
    rt.reconcile_execution(&rec).await.unwrap();
    assert!(!remote.join("cancel-end").exists());
    let mut bad = ssh_config();
    bad.remote_ssh.as_mut().unwrap().port = Some(1);
    let r = RuntimeRegistry::from_config(&bad).unwrap();
    let ctx = RuntimeOpenCtx::new(local.path(), "bad", LoopModel::FunctionCall);
    assert!(matches!(
        r.open(&ctx, &ToolsConfig::default(), Vec::new()).await,
        Err(XllmError::Capability(_))
    ));
    let mut denied = ssh_config();
    denied.remote_ssh.as_mut().unwrap().host = Some("runtime-denied".into());
    let r = RuntimeRegistry::from_config(&denied).unwrap();
    assert!(matches!(
        r.open(&ctx, &ToolsConfig::default(), Vec::new()).await,
        Err(XllmError::Capability(_))
    ));
    let cwd = m.workdir().to_string();
    let task_m = m.clone();
    let pending = tokio::spawn(async move {
        task_m.call_tool(call("exec",json!({"command":"echo start > disconnect-start; sleep 40; echo bad > disconnect-end","cwd":cwd}))).await
    });
    wait_file(&remote.join("disconnect-start")).await;
    let pid_file = std::env::var("LLM_RUNTIME_SSH_PID_FILE").unwrap();
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    assert!(std::process::Command::new("kill")
        .arg(pid.trim())
        .status()
        .unwrap()
        .success());
    let result = tokio::time::timeout(Duration::from_secs(20), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(result.effect_unknown, "{result:?}");
    let rec = reg.records.lock().unwrap().last().unwrap().clone();
    assert!(matches!(
        rt.reconcile_execution(&rec).await,
        Err(XllmError::RecoveryBlocked(_))
    ));
    let status = std::process::Command::new(std::env::var("LLM_RUNTIME_SSH_SERVER").unwrap())
        .args([
            "-f",
            &std::env::var("LLM_RUNTIME_SSH_SERVER_CONFIG").unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    rt.reconcile_execution(&rec).await.unwrap();
    assert!(!remote.join("disconnect-end").exists());
}

async fn wait_file(path: &Path) {
    for _ in 0..300 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("file did not appear: {}", path.display());
}
struct DiskTestRegistrar(PathBuf);
#[async_trait]
impl ExecutionRegistrar for DiskTestRegistrar {
    async fn register(&self, rec: &ExecutionRecord) -> std::result::Result<(), String> {
        use std::io::Write;
        let mut f = std::fs::File::create(&self.0).map_err(|e| e.to_string())?;
        f.write_all(&serde_json::to_vec(rec).unwrap())
            .map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())
    }
    async fn completed(&self, _: &str) {}
}
#[tokio::test]
#[ignore]
async fn ssh_crash_child() {
    let Ok(record) = std::env::var("LLM_RUNTIME_SSH_CRASH_RECORD") else {
        return;
    };
    let r = RuntimeRegistry::from_config(&ssh_config()).unwrap();
    let mut ctx = RuntimeOpenCtx::new(Path::new("/tmp"), "child", LoopModel::FunctionCall);
    ctx.registrar = Arc::new(DiskTestRegistrar(record.into()));
    let (_, m) = r
        .open(&ctx, &ToolsConfig::default(), Vec::new())
        .await
        .unwrap();
    let _ = m
        .exec(
            request(
                m.workdir(),
                "echo start >> crash-count; sleep 60; echo end >> crash-count",
            ),
            &context(),
        )
        .await;
}
#[tokio::test]
#[ignore]
async fn ssh_runner_kill9_reconciles_persisted_remote_execution() {
    let local = tempfile::tempdir().unwrap();
    let record = local.path().join("execution.json");
    let (rt, m, _) = open(
        ssh_config(),
        local.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    m.exec(request(m.workdir(), "rm -f crash-count"), &context())
        .await
        .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "runtime::tests::ssh_crash_child"])
        .env("LLM_RUNTIME_SSH_CRASH_RECORD", &record)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let remote = PathBuf::from(std::env::var("LLM_RUNTIME_SSH_WORKDIR").unwrap());
    wait_file(&remote.join("crash-count")).await;
    let rec: ExecutionRecord = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    child.kill().unwrap();
    let _ = child.wait();
    rt.reconcile_execution(&rec).await.unwrap();
    let output = m
        .exec(request(m.workdir(), "cat crash-count"), &context())
        .await
        .unwrap();
    assert_eq!(output.stdout, "start\n");
}

#[tokio::test]
#[ignore]
async fn ssh_saved_target_rejects_alias_redirect_with_same_id() {
    let local = tempfile::tempdir().unwrap();
    let (rt, m, _) = open(
        ssh_config(),
        local.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    let mut record = crate::xllm::RunRecord::synthetic(
        "saved",
        local.path(),
        crate::xllm::RunStatus::Interrupted,
    );
    record.config.runtime = rt.config();
    record.config.runtime_descriptor = rt.descriptor().clone();
    let client = PathBuf::from(std::env::var("LLM_RUNTIME_SSH_CLIENT_CONFIG").unwrap());
    let old = std::fs::read_to_string(&client).unwrap();
    let port = std::env::var("LLM_RUNTIME_SSH_REDIRECT_PORT").unwrap();
    let changed = old
        .lines()
        .map(|l| {
            if l.trim().starts_with("Port ") {
                format!("  Port {port}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&client, changed).unwrap();
    let result = crate::xllm::rebuild_toolset(&record, &XllmDeps::default()).await;
    std::fs::write(&client, old).unwrap();
    assert!(matches!(result, Err(XllmError::RuntimeMismatch(_))));
    let out = m
        .exec(request(m.workdir(), "printf pinned"), &context())
        .await
        .unwrap();
    assert_eq!(out.stdout, "pinned");
}
