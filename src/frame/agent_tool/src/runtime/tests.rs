use super::*;
use crate::exec_tracking::InflightAction;
use crate::llm_bash::TOOL_SHELL;
use buckyos_api::AiToolCall;
use llm_context::deps::{ToolCallCtx, ToolManager};
use std::time::Duration;
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
        call_id: None,
    }
}
fn inflight(call_id: &str, command: &str) -> InflightAction {
    InflightAction {
        call_id: call_id.into(),
        tool: TOOL_SHELL.into(),
        args: json!({ "command": command }),
        effect: "unknown".into(),
        idempotency_key: None,
        step_index: None,
        started_at_ms: crate::now_ms(),
    }
}
trait CallT {
    async fn call_tool_t(
        &self,
        call: AiToolCall,
    ) -> std::result::Result<llm_context::observation::Observation, llm_context::deps::ToolDispatchError>;
}
impl CallT for XllmToolManager {
    async fn call_tool_t(
        &self,
        call: AiToolCall,
    ) -> std::result::Result<llm_context::observation::Observation, llm_context::deps::ToolDispatchError>
    {
        self.call_tool(call, ToolCallCtx::noop()).await
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
) -> (Arc<dyn AgentRuntime>, Arc<XllmToolManager>, RunBindingSlot) {
    let rt = RuntimeRegistry::from_config(&config).unwrap();
    let ctx = RuntimeOpenCtx::new(workdir, "test", LoopModel::FunctionCall);
    let slot = ctx.run.clone();
    let (_, mut manager) = rt
        .open(
            &ctx,
            &ToolsConfig {
                enabled: Some(true),
                filesystem_policy: Some(policy),
                ..Default::default()
            },
            &XllmDeps::default(),
        )
        .await
        .unwrap();
    manager.bind_run_dir("test", Some(workdir.join("runs").join("test")));
    (rt, Arc::new(manager), slot)
}

#[test]
fn config_merge_resolves_source_paths_and_resets_kind() {
    let a = crate::xllm::parse_llm_context_file(
        Path::new("/one/.llm_context"),
        "runtime:\n  kind: remote_ssh\n  workdir: /srv/work\n  env: {ONE: a}\n  remote_ssh: {host: build, identity_file: key}\n",
    )
    .unwrap();
    let b = crate::xllm::parse_llm_context_file(
        Path::new("/two/.llm_context"),
        "runtime:\n  env: {TWO: b}\n  remote_ssh: {user: dev}\n",
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
    assert_eq!(merged.runtime.workdir.as_deref(), Some("/srv/work"));
    let ssh = merged.runtime.remote_ssh.as_ref().unwrap();
    assert_eq!(
        ssh.identity_file.as_deref().map(Path::new),
        Some(Path::new("/one/key"))
    );
    assert_eq!((ssh.host.as_deref(), ssh.user.as_deref()), (Some("build"), Some("dev")));
    assert_eq!(merged.runtime.env.len(), 2);
    let local = crate::xllm::parse_llm_context_file(
        Path::new("/one/.llm_context"),
        "runtime: {kind: native, workdir: work}",
    )
    .unwrap();
    assert_eq!(
        local.runtime.unwrap().workdir.as_deref().map(Path::new),
        Some(Path::new("/one/work"))
    );
    let mut r = merged.runtime;
    r.merge_over(&RuntimeConfig {
        kind: Some("native".into()),
        ..Default::default()
    });
    assert!(r.remote_ssh.is_none() && r.env.is_empty() && r.workdir.is_none());
    for raw in [
        "runtime: {kind: bad}",
        "runtime: {kind: tmux}",
        "runtime: {kind: native, limits: {}}",
        "runtime: {kind: native, remote_ssh: {host: x}}",
    ] {
        assert!(matches!(
            crate::xllm::parse_llm_context_file(Path::new(".llm_context"), raw),
            Err(XllmError::Config { .. })
        ));
    }
    let node = RuntimeConfig {
        kind: Some("remote_node".into()),
        workdir: Some("/home/devtest/ws".into()),
        remote_node: Some(RemoteNodeConfig {
            node_did: Some("did:web:ood1.test.buckyos.io".into()),
            profile_id: Some("dev".into()),
        }),
        ..Default::default()
    };
    assert!(matches!(
        RuntimeRegistry::from_config(&node),
        Err(XllmError::Capability(_))
    ));
    assert!(matches!(
        RuntimeRegistry::from_config_with(&node, &XllmDeps::default()),
        Err(XllmError::Capability(_))
    ));
    let mut no_profile = node.clone();
    no_profile.remote_node.as_mut().unwrap().profile_id = None;
    assert!(matches!(
        RuntimeRegistry::from_config(&no_profile),
        Err(XllmError::Config { .. })
    ));
    for (workdir, ok) in [(r"C:\Users\devtest\ws", true), ("ws", false), (r"C:ws", false)] {
        let mut windows_node = node.clone();
        windows_node.workdir = Some(workdir.into());
        let result = RuntimeRegistry::from_config(&windows_node);
        assert_eq!(!matches!(result, Err(XllmError::Config { .. })), ok, "{workdir}");
    }
    let parsed = crate::xllm::parse_llm_context_file(
        Path::new(".llm_context"),
        "runtime: {kind: remote_node, workdir: /home/devtest/ws, remote_node: {node_did: did:web:ood1.test.buckyos.io, profile_id: dev}}",
    )
    .unwrap();
    assert_eq!(parsed.runtime.unwrap(), node);
    for kind in ["container", "container_host", "http_proxy_runtime"] {
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
    let (rt, manager, slot) =
        open(config, dir.path(), crate::xllm::FilesystemPolicy::Workspace).await;
    assert_eq!(rt.descriptor().runtime_id, default.descriptor().runtime_id);
    assert_eq!(rt.descriptor().target, default.descriptor().target);
    let obs = manager
        .call_tool_t(call(
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
        if cfg!(windows) {
            "[Console]::Write($env:LLM_RUNTIME_TEST); [Console]::Write((Get-Content -LiteralPath \"quote ' 中文.txt\" -Raw -Encoding UTF8))"
        } else {
            "printf '%s' \"$LLM_RUNTIME_TEST\"; cat \"quote ' 中文.txt\""
        },
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
    if cfg!(windows) {
        assert_eq!(info.os, "windows");
        assert!(
            info.shell.ends_with("pwsh.exe") || info.shell.ends_with("powershell.exe"),
            "{}",
            info.shell
        );
        assert!(info.tools.iter().any(|tool| tool == "Get-Content"));
    }
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
            if cfg!(windows) { "__ENV($runtime.hostname)__|__EXEC([Console]::WriteLine([Environment]::MachineName))__" } else { "__ENV($runtime.hostname)__|__EXEC(uname -n)__" },
            &RenderVars::new(),
            &RuntimeValueLoader(info.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        rendered.rendered.replace("\r\n", "\n"),
        format!("{}|{}\n", info.hostname, info.hostname)
    );
    // Crash recovery wording (§3.2): a finished command's exit and output
    // are read from its execution directory; an unknown one is described
    // as possibly still running, nothing is probed or killed.
    let binding = slot.lock().unwrap().clone().unwrap();
    let done = manager
        .call_tool(
            call(TOOL_SHELL, json!({"command": "echo partial; exit 3"})),
            ToolCallCtx::noop(),
        )
        .await
        .unwrap();
    assert!(matches!(done, llm_context::observation::Observation::Error { .. }));
    let text = rt
        .describe_interrupted(&binding, &inflight(&format!("test-{TOOL_SHELL}"), "echo partial; exit 3"))
        .await;
    assert!(text.contains("exit code 3"), "{text}");
    assert!(text.contains("partial"), "{text}");
    let text = rt
        .describe_interrupted(&binding, &inflight("never-ran", "sleep 1"))
        .await;
    assert!(text.contains("may have partly executed"), "{text}");
    assert!(text.contains("Background processes it started are unaffected"), "{text}");
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        let obs = manager
            .call_tool_t(call(
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
    let (rt, m, slot) = open(
        ssh_config(),
        local.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    assert_ne!(m.workdir(), local.path().to_str().unwrap());
    let path = "a space ' 中文.txt";
    let content = "line one\nquotes ' \" and $(touch SHOULD_NOT_RUN)\n中文\n";
    let obs = m
        .call_tool_t(call("write_file", json!({"path":path,"content":content})))
        .await
        .unwrap();
    assert!(
        !matches!(obs, llm_context::observation::Observation::Error { .. }),
        "{obs:?}"
    );
    let setup = m
        .exec(
            request(
                m.workdir(),
                "printf untouched > victim; printf original > 'victim
second'; ln -sfn 'victim
second' newline-link",
            ),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(setup.exit_code, 0);
    for bad_path in ["victim\nsecond", "newline-link", "new\rname"] {
        for tool_call in [
            call("write_file", json!({"path":bad_path,"content":"corrupted"})),
            call("read_file", json!({"path":bad_path})),
            call(
                "edit_file",
                json!({"path":bad_path,"old_string":"original","new_string":"corrupted"}),
            ),
        ] {
            let obs = m.call_tool_t(tool_call).await.unwrap();
            assert!(
                matches!(obs, llm_context::observation::Observation::Error { .. }),
                "{bad_path:?}: {obs:?}"
            );
        }
    }
    let out = m
        .exec(
            request(
                m.workdir(),
                "cat victim; printf '|'; cat 'victim
second'",
            ),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(out.stdout, "untouched|original");
    let obs = m
        .call_tool_t(call(
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
    timeout.call_id = Some("timed".into());
    let output = m.exec(timeout, &context()).await.unwrap();
    assert!(output.timed_out);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let binding = slot.lock().unwrap().clone().unwrap();
    let text = rt
        .describe_interrupted(&binding, &inflight("timed", "sleep 40"))
        .await;
    assert!(text.contains("remote"), "{text}");
    let out = m
        .exec(
            request(m.workdir(), "test ! -e timed-out; ln -sfn /tmp escape"),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, 0);
    let obs = m
        .call_tool_t(call(
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
        .call_tool_t(call(TOOL_SHELL, json!({"command":"pwd","cwd":"/tmp"})))
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
        .call_tool_t(call(TOOL_SHELL, json!({"command":"pwd","cwd":"/tmp"})))
        .await
        .unwrap();
    assert!(
        !matches!(obs, llm_context::observation::Observation::Error { .. }),
        "{obs:?}"
    );
}

#[tokio::test]
#[ignore]
async fn ssh_configured_runs_do_not_acquire_local_workdir_locks() {
    use crate::xllm::{TaskInput, TaskOverrides, XllmRun, XllmTask};

    let local = tempfile::tempdir().unwrap();
    let cfg = ssh_config();
    std::fs::write(
        local.path().join(".llm_context"),
        json!({
            "runtime": cfg,
            "tools": {"enabled": true},
            "provider": {"type": "openai", "base_url": "http://127.0.0.1:1/v1", "api_key": "test"},
            "model": "test"
        })
        .to_string(),
    )
    .unwrap();
    let deps = XllmDeps::default().with_lock_dir(local.path().join("locks"));
    let overrides = TaskOverrides {
        runs_dir: Some(local.path().join("runs")),
        ..Default::default()
    };
    let first = XllmTask::prepare(
        local.path(),
        TaskInput::question("first"),
        overrides.clone(),
        &deps,
    )
    .await
    .unwrap();
    assert_eq!(first.config.runtime.kind(), "remote_ssh");
    assert!(deps.runtime.is_none());
    let first = XllmRun::start(first, deps.clone()).await.unwrap();
    let second = XllmTask::prepare(
        local.path(),
        TaskInput::question("second"),
        overrides,
        &deps,
    )
    .await
    .unwrap();
    let second = XllmRun::start(second, deps).await.unwrap();
    assert_ne!(first.run_id(), second.run_id());
    assert!(!local.path().join("locks").exists());
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

#[tokio::test]
async fn native_runtime_files_stream_guard_and_commit() {
    use files::{LocalFileBackend, PathType, RuntimeFiles, WholeWriteMode, WriteChunk};
    use buckyos_api::{executor_content_hash, ExecutorErrorCode};
    let dir = tempfile::tempdir().unwrap();
    let files = LocalFileBackend;
    let p = |name: &str| dir.path().join(name).to_string_lossy().to_string();
    let chunk = |mode, expected_size, expected_hash, data| WriteChunk { mode, expected_size, expected_hash, data };

    let first = files.write_chunk(&p("sub/a.part"), chunk(WholeWriteMode::CreateNew, None, None, b"hello ")).await.unwrap();
    assert!(first.created);
    assert_eq!(first.hash.as_deref(), Some(executor_content_hash(b"hello ").as_str()));
    let second = files.write_chunk(&p("sub/a.part"), chunk(WholeWriteMode::Append, Some(6), None, b"world")).await.unwrap();
    assert_eq!((second.size, second.hash.is_none(), second.created), (11, true, false));
    let stale = files.write_chunk(&p("sub/a.part"), chunk(WholeWriteMode::Append, Some(6), None, b"world")).await;
    assert_eq!(stale.unwrap_err().code, ExecutorErrorCode::Conflict);
    let prefix = executor_content_hash(b"hello world");
    let hashed = files
        .write_chunk(&p("sub/a.part"), chunk(WholeWriteMode::Append, None, Some(&prefix), b"!"))
        .await
        .unwrap();
    assert_eq!(hashed.hash.as_deref(), Some(executor_content_hash(b"hello world!").as_str()));
    let missing = files.write_chunk(&p("none"), chunk(WholeWriteMode::Append, Some(0), None, b"x")).await;
    assert_eq!(missing.unwrap_err().code, ExecutorErrorCode::Conflict);
    assert!(!dir.path().join("none").exists());

    let stat = files.stat(&p("sub/a.part"), true).await.unwrap();
    assert_eq!((stat.path_type, stat.size), (PathType::File, 12));
    assert_eq!(stat.hash.as_deref(), Some(executor_content_hash(b"hello world!").as_str()));
    assert_eq!(files.stat(&p("sub"), true).await.unwrap().path_type, PathType::Directory);
    assert_eq!(files.stat(&p("nothing"), false).await.unwrap().path_type, PathType::Missing);
    let range = files.read_range(&p("sub/a.part"), 6, 100).await.unwrap();
    assert_eq!((range.data.as_slice(), range.file_size), (&b"world!"[..], 12));
    let too_big = files.read_range(&p("sub/a.part"), 0, files.limits().max_chunk_bytes + 1).await;
    assert_eq!(too_big.unwrap_err().code, ExecutorErrorCode::InvalidArgument);

    files.rename(&p("sub/a.part"), &p("sub/a"), false).await.unwrap();
    assert_eq!(std::fs::read(dir.path().join("sub/a")).unwrap(), b"hello world!");
    files.write_chunk(&p("b"), chunk(WholeWriteMode::Replace, None, None, b"b")).await.unwrap();
    assert_eq!(files.rename(&p("b"), &p("sub/a"), false).await.unwrap_err().code, ExecutorErrorCode::Conflict);
    files.rename(&p("b"), &p("sub/a"), true).await.unwrap();
    assert_eq!(std::fs::read(dir.path().join("sub/a")).unwrap(), b"b");
    assert_eq!(files.rename(&p("b"), &p("c"), false).await.unwrap_err().code, ExecutorErrorCode::NotFound);
    assert_eq!(files.rename(&p("sub"), &p("d"), false).await.unwrap_err().code, ExecutorErrorCode::InvalidArgument);
    assert!(files.remove(&p("sub/a")).await.unwrap());
    assert!(!files.remove(&p("sub/a")).await.unwrap());
    assert_eq!(files.remove(&p("sub")).await.unwrap_err().code, ExecutorErrorCode::InvalidArgument);
    assert_eq!(files.stat("relative", false).await.unwrap_err().code, ExecutorErrorCode::InvalidArgument);
}

#[tokio::test]
async fn runtime_info_carries_platform_shell_kind_and_declared_baseline() {
    let probe = |os: &str, shell: &str| RuntimeInfo {
        os: os.into(),
        shell: shell.into(),
        ..Default::default()
    };
    let mac = probe("darwin", "/bin/zsh").with_declared(None);
    assert_eq!((mac.platform.as_str(), mac.shell_kind.as_str(), mac.baseline.as_str()), ("macos", "zsh", ""));
    let win = probe("windows", r"C:\Program Files\PowerShell\7\pwsh.exe").with_declared(Some(RuntimeBaseline::Extended));
    assert_eq!((win.platform.as_str(), win.shell_kind.as_str(), win.baseline.as_str()), ("windows", "pwsh", "extended"));

    let dir = tempfile::tempdir().unwrap();
    let mut config: RuntimeConfig = serde_json::from_value(json!({"baseline": "standard"})).unwrap();
    config.workdir = Some(dir.path().display().to_string());
    assert!(serde_json::from_value::<RuntimeConfig>(json!({"baseline": "full"})).is_err());
    let mut merged = RuntimeConfig::default();
    merged.merge_over(&config);
    assert_eq!(merged.baseline, Some(RuntimeBaseline::Standard));
    let rt = RuntimeRegistry::from_config(&config).unwrap();
    let info = rt.info().await.unwrap();
    assert_eq!(info.baseline, "standard");
    assert_eq!(info.platform, std::env::consts::OS);
    assert_eq!(info.template_values()["baseline"], "standard");
    let hosted = crate::xllm::XllmTask::prepare_hosted(
        dir.path(),
        &json!({"runtime": rt.config(), "tools": {"enabled": true}}),
        &dir.path().join("session_config.json").display().to_string(),
        "SHELL_KIND={{runtime.shell_kind}}",
        &crate::xllm::XllmDeps::default().with_runtime(rt.clone()),
    )
    .await
    .unwrap();
    assert!(hosted.prompt.system_prompt.contains(&format!("SHELL_KIND={}", info.shell_kind)));
    let contexts = |info: &RuntimeInfo| {
        crate::xllm::build_contexts_system_text(&crate::xllm::TemplateEnv::for_new_run(info), &BTreeMap::new()).unwrap()
    };
    assert!(contexts(&info).contains("Environment baseline: standard"));
    assert!(!contexts(&RuntimeInfo { os: "linux".into(), ..Default::default() }).contains("Environment baseline"));
}
