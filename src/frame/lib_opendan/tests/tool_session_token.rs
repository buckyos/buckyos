#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use agent_tool::llm_bash::{BashRunRequest, BashTarget};
use agent_tool::runtime::{RuntimeConfig, RuntimeOpenCtx, RuntimeRegistry, Sandbox, TmuxConfig};
use agent_tool::xllm::{LoopModel, ToolsConfig, XllmDeps};
use agent_tool::{SessionRuntimeContext, BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV as TOKEN_ENV};
use buckyos_api::{
    get_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntime, BuckyOSRuntimeType,
};
use libopendan::runner::{drive, StopWhen};
use libopendan::runtime::{session_env_vars, SessionEnvCtx};
use serde_json::json;

fn token(id: &str, exp: u64) -> String {
    json!({
        "iss": "verify-hub", "sub": "alice", "appid": "jarvis.buckyos.bns.did",
        "jti": id, "exp": exp
    })
    .to_string()
}

fn request(cwd: &Path, command: &str) -> BashRunRequest {
    BashRunRequest {
        command: command.into(),
        cwd: cwd.into(),
        timeout_ms: 5000,
        max_output_bytes: 4096,
        env: vec![(TOKEN_ENV.into(), "stale-call-token".into())],
        target: BashTarget::Local,
        call_id: None,
    }
}

struct TmuxSocket(PathBuf);

impl Drop for TmuxSocket {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .arg("-S")
            .arg(&self.0)
            .arg("kill-server")
            .output();
    }
}

#[tokio::test]
async fn session_tools_use_current_runtime_token_without_owner_keys() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::remove_var(TOKEN_ENV);
    std::env::set_var(
        "BUCKYOS_APP_TOKEN",
        "bootstrap-assertion-must-not-be-forwarded",
    );
    let mut ctx = SessionEnvCtx {
        session_id: "token-test".into(),
        session_dir: tmp.path().into(),
        agent_did: "did:bns:jarvis.alice".into(),
        ..Default::default()
    };
    assert!(session_env_vars(&ctx, "test")
        .await
        .unwrap()
        .iter()
        .all(|(key, _)| key != TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "explicit-dev-token");
    assert!(session_env_vars(&ctx, "test")
        .await
        .unwrap()
        .contains(&(TOKEN_ENV.into(), "explicit-dev-token".into())));
    std::env::remove_var(TOKEN_ENV);

    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let initial = token("initial", expires);
    let runtime = BuckyOSRuntime::new(
        "jarvis.buckyos.bns.did",
        Some("alice".into()),
        BuckyOSRuntimeType::AppService,
    );
    assert!(runtime.user_private_key.is_none() && runtime.device_private_key.is_none());
    *runtime.session_token.write().await = initial.clone();
    set_buckyos_api_runtime(runtime).unwrap();
    let runtime = get_buckyos_api_runtime().unwrap();
    let env = session_env_vars(&ctx, "test").await.unwrap();
    assert!(env.contains(&(TOKEN_ENV.into(), initial.clone())));
    assert!(std::env::var_os(TOKEN_ENV).is_none());

    std::env::set_var(TOKEN_ENV, "stale-parent-token");
    ctx.extra_env
        .push((TOKEN_ENV.into(), "stale-session-token".into()));
    let env = session_env_vars(&ctx, "test").await.unwrap();
    let tokens: Vec<_> = env.iter().filter(|(key, _)| key == TOKEN_ENV).collect();
    assert_eq!(tokens, vec![&(TOKEN_ENV.into(), initial)]);
    std::env::remove_var(TOKEN_ENV);

    let context = SessionRuntimeContext {
        trace_id: "test".into(),
        agent_name: "test".into(),
        behavior: "test".into(),
        tool_call_index: 0,
        wakeup_id: String::new(),
        session_id: ctx.session_id.clone(),
        read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
    };
    let mut configs = vec![RuntimeConfig::default()];
    let socket = TmuxSocket(tmp.path().join("tmux.sock"));
    if Command::new("tmux").arg("-V").output().is_ok() {
        configs.push(RuntimeConfig {
            kind: Some("tmux".into()),
            tmux: Some(TmuxConfig {
                session: Some("token-test".into()),
                socket: Some(socket.0.display().to_string()),
                ..Default::default()
            }),
            ..Default::default()
        });
    } else {
        eprintln!("tmux unavailable; checking native token injection only");
    }
    for config in configs {
        let rt = RuntimeRegistry::from_config(&config).unwrap();
        let mut open = RuntimeOpenCtx::new(tmp.path(), "token-test", LoopModel::FunctionCall);
        open.env = env.iter().cloned().collect();
        let (_, sandbox) = rt
            .open(&open, &ToolsConfig::default(), &XllmDeps::default())
            .await
            .unwrap();
        for id in ["renewed-1", "renewed-2"] {
            let current = token(id, expires);
            *runtime.session_token.write().await = current.clone();
            let result = sandbox
                .exec(
                    request(
                        tmp.path(),
                        "printf '%s' \"$BUCKYOS_APPCLIENT_SESSION_TOKEN\"",
                    ),
                    &context,
                )
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout, current, "{}", config.kind());
            assert!(std::env::var_os(TOKEN_ENV).is_none());
        }

        for invalid in [String::new(), "malformed-token".into(), token("expired", 1)] {
            *runtime.session_token.write().await = invalid;
            assert!(session_env_vars(&ctx, "test").await.is_err());
            let err = sandbox
                .exec(request(tmp.path(), "touch must-not-run"), &context)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("appclient session token"), "{err}");
            assert!(!tmp.path().join("must-not-run").exists());
        }
        *runtime.session_token.write().await = token("restored", expires);
    }

    let session = common::Env::new();
    let sd = session
        .create_work(common::work_spec("check tool authentication"))
        .await;
    let current = token("rotated-during-inference", expires);
    let tool_token = current.clone();
    let llm = common::ScriptedLlm::new(move |req, n| match n {
        0 => {
            *runtime.session_token.try_write().unwrap() = tool_token.clone();
            common::tool_call(
                "auth",
                "shell",
                json!({"command": "test -n \"$BUCKYOS_APPCLIENT_SESSION_TOKEN\" && printf '%s' \"$BUCKYOS_APPCLIENT_SESSION_TOKEN\" > tool-token && echo authenticated"}),
            )
        }
        _ => {
            assert!(common::has_tool_result(req, "auth")
                .unwrap()
                .contains("authenticated"));
            common::text("done")
        }
    });
    let result = drive(&sd, &session.deps(llm), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(sd.path().join("tool-token")).unwrap(),
        current
    );
    let record = sd
        .runs()
        .record(&sd.state().unwrap().last_run.unwrap())
        .unwrap();
    let check = &record.host.as_ref().unwrap().env_check;
    assert!(check["env"].get(TOKEN_ENV).is_none());
    assert!(check["env_refs"]
        .as_array()
        .unwrap()
        .contains(&json!(TOKEN_ENV)));
    assert!(!record.config.runtime.env.contains_key(TOKEN_ENV));
}
