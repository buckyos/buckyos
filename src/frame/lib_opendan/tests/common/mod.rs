//! Shared test helpers: temp AgentRoot / app dirs, a dev kmsg queue, a
//! scripted LLM and runner deps.
#![allow(dead_code)]

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
        Self { _tmp: Some(tmp), ..e }
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
            FsAgentStateClient::open(&self.agent_root, AGENT, Some(ch.client()), Some(Arc::new(PollWaker)))
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
    pub fn new(f: impl Fn(&LlmInferenceRequest, usize) -> AiResponse + Send + Sync + 'static) -> Arc<Self> {
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
                AiContent::ToolUse { name, args, call_id } => {
                    s.push_str(&format!("<tool_use {name} {call_id} {:?}>", args))
                }
                AiContent::ToolResult { call_id, content, is_error } => s.push_str(&format!(
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
    let map = args.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
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
            if let AiContent::ToolResult { call_id: id, content, .. } = c {
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
pub fn post_blocking(queue_dir: &Path, queue: &str, input: libopendan::protocol::Input) -> u64 {
    let queue_dir = queue_dir.to_path_buf();
    let queue = queue.to_string();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let client = libopendan::channel::DirMsgQueue::client(&queue_dir).unwrap();
            libopendan::channel::kmsg::post_to_queue(&client, &queue, &input, APP)
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
