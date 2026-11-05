//! Shared test helpers: temp AgentRoot / app dirs, a dev kmsg queue, a
//! scripted LLM and runner deps.
#![allow(dead_code)]

pub mod plan;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_tool::xllm::XllmDeps;
use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiUsage};
use libopendan::api::{create_session, SessionSpec};
use libopendan::channel::{KmsgChannels, PollWaker};
use libopendan::runner::{RunnerDeps, RunnerOptions};
use libopendan::runtime::NativeRuntime;
use libopendan::{FsAgentStateClient, SessionDir};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::LLMComputeError;
use serde_json::{json, Value};

pub const AGENT: &str = "did:bns:jarvis.alice";
pub const APP: &str = "app:app2@alice";

/// A node executor that answers login and describe (what registering and
/// checking a runtime ask) and refuses every operation: enough for a
/// registered node runtime to be `available` and to host sessions that are
/// never driven.
pub struct MockNode {
    pub roots: Vec<String>,
    pub home: String,
}

#[async_trait]
impl agent_tool::runtime::node::NodeExecutorConnector for MockNode {
    async fn connect(&self, _node: &str) -> std::io::Result<Box<dyn agent_tool::runtime::node::NodeIo>> {
        use tokio::io::AsyncWriteExt;
        let (roots, home) = (self.roots.clone(), self.home.clone());
        let (client, mut server) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            let mut profile = Value::Null;
            while let Ok(Some(buckyos_api::ExecutorFrame::Control(bytes))) =
                buckyos_api::read_executor_frame(&mut server).await
            {
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                let id = &request["request_id"];
                let response = match request["type"].as_str().unwrap() {
                    "login" => {
                        profile = request["profile"].clone();
                        json!({"type":"login_ok", "request_id":id, "posture":"normal",
                            "principal":AGENT, "app_instance_id":"app2@alice",
                            "caps":["open_stream","describe","exec","read","write","edit","mkdir","stat","rename","remove","query","cancel"],
                            "limits":buckyos_api::ExecutorLimits::default(), "revision":"one", "generation":1,
                            "auth_valid_until_ms":u64::MAX, "token_expires_at_ms":u64::MAX,
                            "store_id":"mock-ledger", "server_epoch":"one"})
                    }
                    "describe" => json!({"type":"describe_ok", "request_id":id,
                        "target":{"node_did":"did:dev:lab", "profile_id":profile, "uid":1000, "gid":1000,
                            "workdir":request["workdir"], "workdir_roots":roots, "store_id":"mock-ledger", "protocol_version":1},
                        "info":{"os":"linux", "arch":"x86_64", "hostname":"lab", "shell":"bash", "time_ms":1,
                            "home":home, "identity_mode":"current_user"}}),
                    other => json!({"type":"error", "request_id":id,
                        "error":{"code":"capability", "message":format!("mock node: no {other}")}}),
                };
                server.write_all(&buckyos_api::encode_executor_control(&response).unwrap()).await.unwrap();
            }
        });
        Ok(Box::new(client))
    }

    async fn session_token(&self) -> std::result::Result<String, String> {
        Ok("mock-session".into())
    }
}

/// Install [`MockNode`] as this process's node connector.
pub fn mock_node(roots: &[&str], home: &str) {
    libopendan::runtime::set_node_connector(Arc::new(MockNode {
        roots: roots.iter().map(|r| r.to_string()).collect(),
        home: home.to_string(),
    }));
}

pub struct Env {
    pub root: PathBuf,
    pub agent_root: PathBuf,
    pub app_dir: PathBuf,
    pub queue_dir: PathBuf,
    pub _tmp: Option<tempfile::TempDir>,
}

impl Env {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let e = Self::at(&root);
        Self {
            _tmp: Some(tmp),
            ..e
        }
    }

    /// Re-open an environment at an existing root (child processes).
    pub fn at(root: &Path) -> Self {
        let agent_root = root.join("agent_root");
        let app_dir = root.join("app2").join("agent_sessions");
        let queue_dir = root.join("kmsg");
        for d in [&agent_root, &app_dir, &queue_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        Self {
            root: root.to_path_buf(),
            agent_root,
            app_dir,
            queue_dir,
            _tmp: None,
        }
    }

    pub fn channels(&self) -> Arc<KmsgChannels> {
        Arc::new(KmsgChannels::dir(&self.queue_dir).unwrap())
    }

    pub fn agent(&self) -> Arc<FsAgentStateClient> {
        let ch = self.channels();
        Arc::new(
            FsAgentStateClient::open(
                &self.agent_root,
                AGENT,
                Some(ch.client()),
                Some(Arc::new(PollWaker)),
            )
            .unwrap(),
        )
    }

    pub fn runtime(&self) -> Arc<NativeRuntime> {
        Arc::new(NativeRuntime::local("rt-test-native", "app:app2"))
    }

    pub fn deps(&self, llm: Arc<dyn LlmClient>) -> RunnerDeps {
        let xllm = XllmDeps::default().with_llm(llm);
        let opts = RunnerOptions {
            poll_interval: Duration::from_millis(50),
            max_wait: Duration::from_millis(300),
            load_hints: false,
            ..Default::default()
        };
        RunnerDeps::new(APP, self.agent(), self.channels(), self.runtime(), xllm).with_options(opts)
    }

    pub async fn workspace(&self, path: &Path) -> libopendan::protocol::WorkspaceRef {
        use libopendan::state::AgentStateClient;
        let record = self
            .agent()
            .workspaces()
            .import(
                &libopendan::protocol::WorkspaceImport {
                    operation_id: path.display().to_string(),
                    location: libopendan::protocol::WorkspaceLocation {
                        runtime_id: "local".into(),
                        directory: path.to_path_buf(),
                    },
                    name: Some("test workspace".into()),
                    description: String::new(),
                    url: None,
                    usage: Default::default(),
                    expected_revision: None,
                    source_session: None,
                    policy_ref: None,
                },
                APP,
            )
            .await
            .unwrap();
        libopendan::protocol::WorkspaceRef {
            workspace_id: record.workspace_id,
            access: Default::default(),
        }
    }

    pub async fn create_work(&self, spec: SessionSpec) -> SessionDir {
        let agent = self.agent();
        let ch = self.channels();
        create_session(&self.app_dir, spec, agent.as_ref(), APP, ch.as_ref())
            .await
            .unwrap()
    }
}

/// Behaviour of the scripted LLM for one request.
pub type Script =
    dyn Fn(&LlmInferenceRequest, usize) -> Result<AiResponse, LLMComputeError> + Send + Sync;

pub struct ScriptedLlm {
    pub calls: AtomicUsize,
    pub requests: Mutex<Vec<LlmInferenceRequest>>,
    script: Box<Script>,
}

impl ScriptedLlm {
    pub fn new(
        f: impl Fn(&LlmInferenceRequest, usize) -> AiResponse + Send + Sync + 'static,
    ) -> Arc<Self> {
        Self::fallible(move |r, n| Ok(f(r, n)))
    }

    pub fn fallible(
        f: impl Fn(&LlmInferenceRequest, usize) -> Result<AiResponse, LLMComputeError>
            + Send
            + Sync
            + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            script: Box::new(f),
        })
    }

    pub fn transcript(&self, idx: usize) -> String {
        let reqs = self.requests.lock().unwrap();
        render(&reqs[idx].messages)
    }

    pub fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

pub fn render(messages: &[AiMessage]) -> String {
    let mut s = String::new();
    for m in messages {
        s.push_str(&format!("[{:?}] ", m.role));
        for c in &m.content {
            match c {
                AiContent::Text { text } => s.push_str(text),
                AiContent::ToolUse {
                    name,
                    args,
                    call_id,
                } => s.push_str(&format!("<tool_use {name} {call_id} {:?}>", args)),
                AiContent::ToolResult {
                    call_id,
                    content,
                    is_error,
                } => s.push_str(&format!(
                    "<tool_result {call_id} error={is_error} {}>",
                    content
                        .iter()
                        .filter_map(|x| x.text_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
                _ => {}
            }
        }
        s.push('\n');
    }
    s
}

#[async_trait]
impl LlmClient for ScriptedLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(req.clone());
        let mut r = (self.script)(&req, n)?;
        r.usage = Some(AiUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            total_tokens: Some(15),
            ..Default::default()
        });
        Ok(r)
    }
}

pub fn text(t: &str) -> AiResponse {
    AiResponse::new(AiMessage::text(AiRole::Assistant, t))
}

pub fn tool_call(call_id: &str, name: &str, args: Value) -> AiResponse {
    let map = args
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    AiResponse::new(AiMessage::new(
        AiRole::Assistant,
        vec![AiContent::tool_use(call_id, name, map)],
    ))
}

pub fn last_user_text(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == AiRole::User)
        .map(|m| m.text_content())
        .unwrap_or_default()
}

pub fn has_tool_result(req: &LlmInferenceRequest, call_id: &str) -> Option<String> {
    for m in &req.messages {
        for c in &m.content {
            if let AiContent::ToolResult {
                call_id: id,
                content,
                ..
            } = c
            {
                if id == call_id {
                    return Some(
                        content
                            .iter()
                            .filter_map(|x| x.text_str())
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
            }
        }
    }
    None
}

/// DID the test user speaks as (the owner of [`APP`]).
pub const USER: &str = "did:bns:alice";

/// A text message from the test user to the agent (`key` = its ObjId).
pub fn msg(text: impl Into<String>) -> libopendan::protocol::PostedInput {
    libopendan::protocol::PostedInput::text(APP, AGENT, text).unwrap()
}

/// An event record of `source_kind:source_id`.
pub fn event(
    key: &str,
    subscription: Option<&str>,
    source_kind: &str,
    source_id: &str,
    seq: Option<u64>,
    summary: &str,
) -> libopendan::protocol::PostedInput {
    libopendan::protocol::PostedInput::event(
        APP,
        key,
        libopendan::protocol::AgentEvent {
            subscription_id: subscription.map(str::to_string),
            source: libopendan::protocol::EventSource::new(source_kind, source_id),
            event: "changed".into(),
            seq,
            summary: summary.into(),
            data_ref: None,
            terminal: false,
            call_id: None,
        },
    )
}

/// Text of every user message of a request, in order.
pub fn user_texts(req: &LlmInferenceRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::User)
        .map(|m| m.text_content())
        .collect()
}

pub fn work_spec(objective: &str) -> SessionSpec {
    let mut s = SessionSpec::work(objective);
    s.prompt.llm_context = json!({ "tools": { "enabled": true } });
    s
}

pub fn read_worklog(sd: &SessionDir) -> Vec<libopendan::protocol::WorklogEntry> {
    sd.worklog().read_all_for_audit().unwrap()
}

pub fn kinds(entries: &[libopendan::protocol::WorklogEntry]) -> Vec<&'static str> {
    entries.iter().map(|e| e.body.kind()).collect()
}

/// Post an input from synchronous code (e.g. inside an LLM script) through a
/// dedicated thread + runtime.
pub fn post_blocking(
    queue_dir: &Path,
    queue: &str,
    input: libopendan::protocol::PostedInput,
) -> u64 {
    let queue_dir = queue_dir.to_path_buf();
    let queue = queue.to_string();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let client = libopendan::channel::DirMsgQueue::client(&queue_dir).unwrap();
            libopendan::channel::kmsg::post_to_queue(&client, &queue, &input)
                .await
                .unwrap()
        })
    })
    .join()
    .unwrap()
}

pub fn queue_of(sd: &SessionDir) -> String {
    sd.config().unwrap().channels.kmsg().unwrap().1.to_string()
}
