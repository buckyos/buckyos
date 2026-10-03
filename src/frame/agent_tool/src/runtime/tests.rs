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
    assert_eq!(merged.runtime.workdir.as_deref().map(Path::new), Some(Path::new("/one/work")));
    assert_eq!(
        merged.runtime.tmux.as_ref().unwrap().socket.as_deref().map(Path::new),
        Some(Path::new("/one/sock"))
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
        .open(&ctx, &ToolsConfig::default(), &XllmDeps::default())
        .await
        .is_err());
    let mut cfg = cfg;
    cfg.tmux.as_mut().unwrap().mode = Some(TmuxMode::Create);
    let (rt, m, slot) = open(
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
        .call_tool_t(call("read_file", json!({"path":"file"})))
        .await
        .unwrap();
    assert!(format!("{obs:?}").contains("same"));
    assert_eq!(original, CommandOutput::pane(&socket, "runtime-test:0.0"));
    let bad = RuntimeRegistry::from_config(&cfg).unwrap();
    assert!(bad
        .open(&ctx, &ToolsConfig::default(), &XllmDeps::default())
        .await
        .is_err());
    cfg.tmux.as_mut().unwrap().mode = Some(TmuxMode::CreateOrAttach);
    let (second, _, _) = open(cfg, dir.path(), crate::xllm::FilesystemPolicy::Workspace).await;
    assert_eq!(rt.descriptor(), second.descriptor());
    // A command the executor stops waiting for keeps running in tmux; a
    // timeout kills its window.
    let mut long = request(m.workdir(), "sleep 30; echo late");
    long.timeout_ms = 500;
    long.call_id = Some("long".into());
    let out = m.exec(long, &context()).await.unwrap();
    assert!(out.timed_out);
    let binding = slot.lock().unwrap().clone().unwrap();
    let text = rt
        .describe_interrupted(&binding, &inflight("long", "sleep 30; echo late"))
        .await;
    assert!(text.contains("tmux session `runtime-test`"), "{text}");
    let _ = std::process::Command::new("tmux")
        .args(["-S", &socket, "kill-server"])
        .status();
}

#[tokio::test]
async fn tmux_run_cancellation_keeps_the_command_running() {
    if !Runtime::available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("socket").display().to_string();
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::process::Command::new("tmux")
                .args(["-S", &self.0, "kill-server"])
                .output();
        }
    }
    let _cleanup = Cleanup(socket.clone());
    let (_, manager, slot) = open(
        RuntimeConfig {
            kind: Some("tmux".into()),
            tmux: Some(TmuxConfig {
                session: Some("run-cancellation".into()),
                socket: Some(socket),
                mode: Some(TmuxMode::Create),
            }),
            ..Default::default()
        },
        dir.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    let binding = slot.lock().unwrap().clone().unwrap();
    for cause in ["interrupt", "finish", "deadline"] {
        let handle = llm_context::LLMContextInterruptHandle::standalone();
        let call_ctx = ToolCallCtx {
            abort: handle.token(),
            deadline_ms: (cause == "deadline").then(|| crate::now_ms() + 500),
            allow_deferred: false,
        };
        let mut call = call(
            TOOL_SHELL,
            json!({"command": format!("echo started > {cause}.started; sleep 2; echo finished")}),
        );
        call.call_id = cause.into();
        let manager = manager.clone();
        let running = tokio::spawn(async move { manager.call_tool(call, call_ctx).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !dir.path().join(format!("{cause}.started")).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("tmux command started");
        match cause {
            "interrupt" => {
                handle.interrupt("test");
            }
            "finish" => {
                handle.finish("test");
            }
            _ => {}
        }
        let observation = tokio::time::timeout(Duration::from_secs(1), running)
            .await
            .expect("run cancellation is prompt")
            .unwrap()
            .unwrap();
        let llm_context::observation::Observation::Cancelled {
            reason,
            effect_unknown,
            ..
        } = observation
        else {
            panic!("expected cancelled, got {observation:?}");
        };
        assert!(!effect_unknown);
        assert!(reason.contains("still running"), "{reason}");
        assert!(
            reason.contains("tmux session `run-cancellation`"),
            "{reason}"
        );
        let exec = crate::llm_bash::exec_dir_for(binding.run_dir.as_deref(), Some(cause)).unwrap();
        assert!(reason.contains(&exec.display().to_string()), "{reason}");
        assert!(!exec.join("exit").exists());
        tokio::time::timeout(Duration::from_secs(4), async {
            while crate::llm_bash::read_exit_file(&exec).is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("detached command completes");
        assert_eq!(crate::llm_bash::read_exit_file(&exec), Some(0));
        assert!(std::fs::read_to_string(exec.join("stdout"))
            .unwrap()
            .contains("finished"));
    }
}

struct CommandOutput;
impl CommandOutput {
    fn tmux(socket: &str, args: &[&str]) -> String {
        let output = std::process::Command::new("tmux")
            .args(["-S", socket])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    fn pane(socket: &str, pane: &str) -> Vec<u8> {
        std::process::Command::new("tmux")
            .args(["-S", socket, "capture-pane", "-p", "-t", pane])
            .output()
            .unwrap()
            .stdout
    }
}

#[tokio::test]
async fn tmux_duplicate_window_names_cancel_only_the_requested_run() {
    if !Runtime::available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("socket").display().to_string();
    let target = Arc::new(
        tmux::TmuxTarget::open(
            &TmuxConfig {
                session: Some("duplicates".into()),
                socket: Some(socket.clone()),
                mode: Some(TmuxMode::Create),
            },
            dir.path(),
        )
        .await
        .unwrap(),
    );
    let mut handles = Vec::new();
    for run_id in ["first", "second"] {
        let slot = new_run_binding_slot();
        *slot.lock().unwrap() = Some(RunBinding {
            run_id: run_id.into(),
            run_dir: Some(dir.path().join(run_id)),
        });
        let runner = tmux::TmuxBashRunner::new(target.clone(), BTreeMap::new(), slot);
        let mut req = request(dir.path().to_str().unwrap(), "sleep 30");
        req.call_id = Some("duplicate".into());
        handles.push(runner.start(&context(), req).await.unwrap());
    }
    let windows = || {
        CommandOutput::tmux(
            &socket,
            &[
                "list-windows",
                "-t",
                "=duplicates",
                "-F",
                "#{window_id} #{window_name}",
            ],
        )
        .lines()
        .filter(|line| line.ends_with(" llm-duplicate"))
        .map(str::to_string)
        .collect::<Vec<_>>()
    };
    let original = windows();
    assert_eq!(original.len(), 2);
    assert!(handles[0]
        .locator()
        .contains(original[0].split_once(' ').unwrap().0));
    assert!(handles[1]
        .locator()
        .contains(original[1].split_once(' ').unwrap().0));
    assert!(handles[0].kill().await.unwrap().timed_out);
    assert_eq!(windows(), vec![original[1].clone()]);
    assert!(handles[1]
        .wait(Duration::from_millis(50))
        .await
        .unwrap()
        .is_none());
    assert!(handles[1].kill().await.unwrap().timed_out);
    assert!(windows().is_empty());
    CommandOutput::tmux(&socket, &["kill-server"]);
}

#[tokio::test]
async fn tmux_info_and_shell_use_the_session_environment() {
    if !Runtime::available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("socket").display().to_string();
    let tmux_cfg = TmuxConfig {
        session: Some("environment".into()),
        socket: Some(socket.clone()),
        mode: Some(TmuxMode::Create),
    };
    tmux::TmuxTarget::open(&tmux_cfg, dir.path()).await.unwrap();
    let path = format!(
        "{}:{}",
        dir.path().join("session-bin").display(),
        std::env::var("PATH").unwrap()
    );
    let bash_env = dir.path().join("bash-env");
    std::fs::write(
        &bash_env,
        format!("export PATH={}\nunset BASH_ENV\n", shell_quote(&path)),
    )
    .unwrap();
    CommandOutput::tmux(
        &socket,
        &[
            "set-environment",
            "-t",
            "=environment",
            "BASH_ENV",
            bash_env.to_str().unwrap(),
        ],
    );
    CommandOutput::tmux(
        &socket,
        &["set-environment", "-t", "=environment", "TZ", "HST10"],
    );
    let config = RuntimeConfig {
        kind: Some("tmux".into()),
        tmux: Some(TmuxConfig {
            mode: Some(TmuxMode::Attach),
            ..tmux_cfg
        }),
        ..Default::default()
    };
    let (rt, manager, _) = open(
        config.clone(),
        dir.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    assert_eq!(manager.runtime_info().unwrap().timezone, "HST-1000");
    let out = manager
        .exec(
            request(manager.workdir(), "date +%Z%z; printf '%s' \"$PATH\""),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(out.stdout, format!("HST-1000\n{path}"));
    CommandOutput::tmux(
        &socket,
        &["set-environment", "-t", "=environment", "TZ", "JST-9"],
    );
    assert_eq!(rt.info().await.unwrap().timezone, "JST+0900");
    let mut override_config = config;
    override_config.env.insert("TZ".into(), "UTC0".into());
    let (rt, manager, _) = open(
        override_config,
        dir.path(),
        crate::xllm::FilesystemPolicy::Workspace,
    )
    .await;
    assert_eq!(rt.info().await.unwrap().timezone, "UTC+0000");
    let out = manager
        .exec(request(manager.workdir(), "date +%Z%z"), &context())
        .await
        .unwrap();
    assert_eq!(out.stdout, "UTC+0000\n");
    CommandOutput::tmux(&socket, &["kill-server"]);
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
