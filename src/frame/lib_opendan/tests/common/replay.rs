//! Real-model replays: the Jarvis package of this repository with the real
//! model (the local Zone's AICC) where a case does not script the answer, and
//! what a session was given, ran and said, as text.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::AiResponse;
use libopendan::host::HostDeps;
use libopendan::notify::LocalNotify;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, RunnerDeps, StopSignal, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::SessionDir;
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::LLMComputeError;
use serde_json::{json, Value};

use super::plan::*;
use super::*;

pub const OWNER: &str = "did:bns:alice";
pub const ASK_TASK: &str = "t-0000000000000000000000000000ask1";

pub type Script = dyn Fn(&LlmInferenceRequest) -> Option<AiResponse> + Send + Sync;

/// The real model, unless the script answers the request.
pub struct Mixed {
    pub real: agent_tool::xllm::AiccLlmClient,
    pub script: Box<Script>,
    pub real_calls: AtomicUsize,
}

#[async_trait]
impl LlmClient for Mixed {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        match (self.script)(&req) {
            Some(r) => Ok(r),
            None => {
                self.real_calls.fetch_add(1, Ordering::SeqCst);
                self.real.infer(req).await
            }
        }
    }
}

pub async fn mixed(script: impl Fn(&LlmInferenceRequest) -> Option<AiResponse> + Send + Sync + 'static) -> Arc<Mixed> {
    agent_tool::xllm::ensure_buckyos_runtime().await.expect("the local Zone");
    Arc::new(Mixed {
        real: agent_tool::xllm::AiccLlmClient::new(),
        script: Box::new(script),
        real_calls: AtomicUsize::new(0),
    })
}

pub fn setup() -> Env {
    let env = match std::env::var("PLAN_REPLAY_KEEP") {
        Ok(dir) => Env::at(&PathBuf::from(dir).join(format!("{}", libopendan::now_ms()))),
        Err(_) => Env::new(),
    };
    install_jarvis(&env);
    copy_dir(&Path::new(PACKAGE).join("plan_skills"), &env.agent_root.join("plan_skills"));
    tool_env(&env);
    // `ask_user` of the cases: what a Zone would print, and no Zone asked.
    let tools = env.agent_root.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let envelope = json!({ "agent_tool_protocol": "1", "status": "pending", "task_id": ASK_TASK,
        "pending_reason": "user_input", "summary": "asked the user", "detail": {} });
    std::fs::write(
        tools.join("ask_user"),
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$OPENDAN_AGENT_ROOT/asked.txt\"\ncat <<'EOT'\n{envelope}\nEOT\n"),
    )
    .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(tools.join("ask_user"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // The shell of a replay is this machine's: what would change it, or reach
    // another one, is not there.
    for name in ["crontab", "at", "sudo", "su", "systemctl", "service", "launchctl", "docker", "apt", "apt-get", "snap", "ssh", "scp", "sftp", "rsync", "pkill", "killall"] {
        std::fs::write(tools.join(name), format!("#!/bin/sh\necho '{name}: not available here' >&2\nexit 1\n")).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(tools.join(name), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    env
}

/// The model of a replay: the Zone's `llm.chat`, or the alias named here.
pub fn real_model() -> String {
    std::env::var("REPLAY_MODEL").unwrap_or_else(|_| "llm.chat".into())
}

pub fn real_context() -> Value {
    json!({ "model": real_model(), "max_tokens": 8192, "provider": { "type": "buckyos" }, "tools": { "enabled": true } })
}

/// The deps the host drives `sd` with.
pub async fn deps(env: &Env, sd: &SessionDir, llm: &Arc<Mixed>) -> RunnerDeps {
    host_of(env, llm.clone()).runner_deps(sd, StopSignal::default()).await.unwrap()
}

pub fn host_of(env: &Env, llm: Arc<Mixed>) -> HostDeps {
    let mut h = host(env, llm);
    h.options.max_wait = Duration::from_secs(1200);
    h
}

/// Drive a session and its sub sessions to its end.
pub async fn finish(env: &Env, sd: &SessionDir, llm: &Arc<Mixed>) -> DriveResult {
    Box::pin(libopendan::host::run_session(
        &host_of(env, llm.clone()),
        sd,
        StopWhen::Finished,
        StopSignal::default(),
        false,
    ))
    .await
    .result
}

/// The shell commands of a session, in order.
pub fn commands(sd: &SessionDir) -> Vec<String> {
    read_worklog(sd)
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::AssistantMessage { tool_calls, .. } => Some(tool_calls.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|c| c.args.get("command").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// What a session was given and said, as text.
pub fn transcript(sd: &SessionDir) -> String {
    let mut out = format!("## session {}\n\n", sd.sid());
    for e in read_worklog(sd) {
        match e.body {
            WorklogBody::UserMessage { content, .. } => out.push_str(&format!("### input\n\n```\n{content}\n```\n\n")),
            WorklogBody::AssistantMessage { assistant, tool_calls, .. } => {
                if !assistant.trim().is_empty() {
                    out.push_str(&format!("### reply\n\n{assistant}\n\n"));
                }
                for c in tool_calls {
                    out.push_str(&format!(
                        "### call `{}`{}\n\n```\n{}\n```\n\n",
                        c.tool,
                        c.intent.map(|i| format!(" — {i}")).unwrap_or_default(),
                        c.args.get("command").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| serde_json::to_string_pretty(&c.args).unwrap())
                    ));
                }
            }
            WorklogBody::ActionResult { result, .. } => {
                let text: String = result.to_string().chars().take(1500).collect();
                out.push_str(&format!("### result\n\n```\n{text}\n```\n\n"));
            }
            WorklogBody::ControlApplied { command, detail, .. } if command == "perceive" => {
                out.push_str(&format!("### perceived\n\n```\n{detail}\n```\n\n"));
            }
            _ => {}
        }
    }
    out
}

/// The record of a case, to be read: `target/<group>/<case>.md`.
pub fn write_to(group: &str, case: &str, text: String) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target").join(group);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{case}.md")), &text).unwrap();
    println!("{text}");
}

pub async fn works_of(env: &Env, parent: Option<&str>) -> Vec<SessionDir> {
    let mut out = Vec::new();
    for e in env.agent().sessions().query(&RegistryQuery::default()).await.unwrap() {
        let mine = parent.is_none_or(|p| e.origin.as_ref().and_then(|o| o.parent_session.as_deref()) == Some(p));
        if e.kind == SessionKind::Work && mine {
            out.push(SessionDir::open(&e.location).unwrap());
        }
    }
    out
}

pub async fn say(env: &Env, sd: &SessionDir, llm: &Arc<Mixed>, words: &str) {
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg(words)).await.unwrap();
    let mut d = deps(env, sd, llm).await;
    d.notify = Some(Arc::new(LocalNotify::enable(&env.agent_root, AGENT).unwrap()));
    for _ in 0..2 {
        Box::pin(drive(sd, &d, StopWhen::Idle)).await;
    }
}

